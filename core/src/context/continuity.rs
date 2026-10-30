//! 开场简报：新会话自动载入最近 1–3 份 [`CompactState`] + 现场 git 状态。
//!
//! # 设计红线（与 `compact_state` 同源）
//!
//! 导不出的字段一律 `None`，并在 [`RecentBrief::notes`] 里写**具体**原因；
//! 绝不用空串 / 0 / 「main」冒充。`dirty_files` 的三态语义必须分清：
//!
//! * `Some(非空)` = 现场确认有未提交改动；
//! * `Some(空)` = 现场确认工作区干净（**干净不注入**，干净不是信息）；
//! * `None` = 读不到（未绑定项目 / 非 git 仓库 / 超时），由 `notes` 点名。
//!
//! # 注入纪律
//!
//! 整段简报上限 [`BRIEF_CHAR_BUDGET`] 字符；超出按 **branch → git → 目标** 的优先级
//! 保留，后段截断并置 [`RecentBrief::truncated`]。git 探测只跑 `status --short` 与
//! `diff --stat`（均为只读动词），单条命令 **5s 超时**，失败只降级、不 panic。
//!
//! # 只读边界
//!
//! 本模块只读台账 JSON 与 git 输出，**不写任何账本**，也不改工作区。

use std::path::Path;
use std::time::Duration;

use crate::context::compact_state::CompactState;
use crate::proc::SilentCommand;

/// 简报标题（可观测锚点：请求体里出现它 = 这一路真的注入了）。
pub const CONTINUITY_HEADER: &str = "## 开场简报（最近压缩交接 + 工作区状态，仅供参考）";

/// 整段简报的字符预算（与 `role_context::MEMORY_CHAR_BUDGET` 同量级：
/// 简报只是增强项，不得吃掉本轮任务本身的上下文）。
pub const BRIEF_CHAR_BUDGET: usize = 1_200;

/// 默认载入的最近交接状态份数（规范：1–3 份）。
pub const RECENT_STATE_LIMIT: usize = 3;

/// 单条 git 命令的超时（超时按「读不到」降级，不 panic）。
const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// 简报里最多逐条列出的未提交文件数（其余折叠成一行计数）。
const DIRTY_LIST_MAX: usize = 8;

/// 最近一次压缩交接 + 现场工作区的**结构化简报**。
///
/// 字段缺失是**可解释**的（见 [`RecentBrief::notes`]），不是空串。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecentBrief {
    /// 最近已知分支；`None` = 非 git 仓库 / 必绑定项目 / 无交接状态。
    pub branch: Option<String>,
    /// 未提交路径（三态语义见模块头）。
    pub dirty_files: Option<Vec<String>>,
    /// `git diff --stat` 的最后一行（变更规模摘要）；无改动 / 未执行 ⇒ `None`。
    pub diff_stat: Option<String>,
    /// 最近一次真跑过的测试（`命令 → 退出码`）；本引擎通常为 `None`。
    pub test_status: Option<String>,
    /// 上一次的用户目标（取自活动计划标题）；无则 `None`。
    pub last_user_goal: Option<String>,
    /// 「哪些字段为什么是空的」——把缺失变成可显示的状态。
    pub notes: Vec<String>,
    /// 是否因超出字符预算而截断（被截断的内容不再保证完整）。
    pub truncated: bool,
}

impl RecentBrief {
    /// 渲染成可直接注入角色上下文的文本块。
    ///
    /// 返回 `None` 表示**没有可注入内容**：工作区干净且既无测试结果也无历史目标
    /// （只有 `notes` 里的「缺什么」不算可注入内容）。
    pub fn render(&mut self) -> Option<String> {
        let has_dirty = matches!(&self.dirty_files, Some(d) if !d.is_empty());
        if !has_dirty && self.test_status.is_none() && self.last_user_goal.is_none() {
            return None;
        }

        let mut sections: Vec<String> = Vec::new();
        if has_dirty {
            if let Some(branch) = self.branch.as_deref() {
                sections.push(format!("- 分支：{branch}"));
            }
            let dirty = self.dirty_files.clone().unwrap_or_default();
            let mut lines = vec![format!("- 未提交改动 {} 项：", dirty.len())];
            for p in dirty.iter().take(DIRTY_LIST_MAX) {
                lines.push(format!("  · {p}"));
            }
            if dirty.len() > DIRTY_LIST_MAX {
                lines.push(format!(
                    "  · …（其余 {} 项省略）",
                    dirty.len() - DIRTY_LIST_MAX
                ));
            }
            if let Some(stat) = self.diff_stat.as_deref() {
                lines.push(format!("- 变更规模：{stat}"));
            }
            sections.push(lines.join("\n"));
        }
        if let Some(status) = self.test_status.as_deref() {
            sections.push(format!("- 最近测试：{status}"));
        }
        if let Some(goal) = self.last_user_goal.as_deref() {
            sections.push(format!("- 上次目标：{goal}"));
        }
        // 说明只在「已经要注入」时随行出现，避免干净工作区被一条「缺测试结果」噪声触发注入。
        for note in &self.notes {
            sections.push(format!("- 说明：{note}"));
        }

        let mut out = String::from(CONTINUITY_HEADER);
        let mut used = out.chars().count();
        for section in sections {
            let cost = section.chars().count() + 1; // '\n'
            if used + cost > BRIEF_CHAR_BUDGET {
                // 尽量塞进一段截断版，保住高优先级内容；剩余一律放弃。
                let room = BRIEF_CHAR_BUDGET.saturating_sub(used + 2);
                if room > 8 {
                    let clipped: String = section.chars().take(room).collect();
                    out.push('\n');
                    out.push_str(&clipped);
                    out.push('…');
                }
                self.truncated = true;
                return Some(out);
            }
            used += cost;
            out.push('\n');
            out.push_str(&section);
        }
        Some(out)
    }
}

/// 纯函数装配：最近交接状态 + 项目目录 → 结构化简报。
///
/// `states` 应是**最新优先**（[`recent_compact_states`] 已排序）；`project_path`
/// 为 `None`/空 ⇒ 不跑 git，`dirty_files` 回退交接状态里的历史值并点名原因。
pub fn recent_brief(states: &[CompactState], project_path: Option<&str>) -> RecentBrief {
    let branch = states
        .iter()
        .find_map(|s| s.branch.clone().filter(|b| !b.trim().is_empty()));
    let test_status = states.iter().find_map(|s| {
        s.test_status
            .as_ref()
            .map(|t| format!("{} → 退出码 {}", t.command, t.exit_code))
    });
    let last_user_goal = states.iter().find_map(|s| {
        s.active_plan
            .as_ref()
            .map(|p| p.title.trim().to_string())
            .filter(|t| !t.is_empty())
    });
    let history_dirty = states
        .iter()
        .find_map(|s| (!s.dirty_files.is_empty()).then(|| s.dirty_files.clone()));

    let mut notes: Vec<String> = Vec::new();
    if states.is_empty() {
        notes.push("recent: 未找到任何 CompactState（最近会话从未压缩过）".to_string());
    }

    let (dirty_files, diff_stat) = match project_path.map(str::trim).filter(|p| !p.is_empty()) {
        Some(dir) => {
            let probe = probe_git(Path::new(dir));
            match probe.dirty {
                Some(dirty) => (Some(dirty), probe.diff_stat),
                None => {
                    notes.push(format!(
                        "dirty_files: 现场 git 状态不可读（{}）",
                        probe.problem.unwrap_or_else(|| "未知原因".to_string())
                    ));
                    (history_dirty, None)
                }
            }
        }
        None => {
            notes.push("dirty_files: 会话未绑定项目目录，无法运行 git status".to_string());
            (history_dirty, None)
        }
    };

    let mut brief = RecentBrief {
        branch,
        dirty_files,
        diff_stat,
        test_status,
        last_user_goal,
        notes,
        ..Default::default()
    };

    if brief.branch.is_none() {
        brief.notes.push(
            "branch: 最近 CompactState 与现场 git 都没有分支信息（非 git 仓库 / 未绑定项目）"
                .to_string(),
        );
    }
    if brief.test_status.is_none() {
        brief
            .notes
            .push("test_status: 最近 CompactState 均无结构化测试结果（本引擎不记录）".to_string());
    }
    if brief.last_user_goal.is_none() {
        brief.notes.push(
            "last_user_goal: 最近 CompactState 均无活动计划标题（计划库不按会话归属）".to_string(),
        );
    }
    brief
}

/// 生产入口：读最近 [`RECENT_STATE_LIMIT`] 份交接状态 + 现场 git → 注入文本。
///
/// 工作区干净且无其他内容 ⇒ `None`（不注入空块）。
pub fn inject_block(project_path: Option<&str>) -> Option<String> {
    let states = recent_compact_states(RECENT_STATE_LIMIT);
    let mut brief = recent_brief(&states, project_path);
    brief.render()
}

/// 从会话台账目录扫描出最近 `limit` 份 `CompactState`（按 `generated_at_ms` 最新优先）。
///
/// 只读；损坏 / 无此键 / 时间戳为 0 的台账一律跳过（不臆造）。
pub fn recent_compact_states(limit: usize) -> Vec<CompactState> {
    let dir = crate::context::session::context_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut states: Vec<CompactState> = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(ctx) = serde_json::from_str::<crate::context::session::SessionContext>(&raw) else {
            continue;
        };
        if let Some(state) = ctx.compact_state {
            if state.generated_at_ms > 0 {
                states.push(state);
            }
        }
    }
    states.sort_by_key(|s| std::cmp::Reverse(s.generated_at_ms));
    states.truncate(limit.max(1));
    states
}

/// 现场 git 探测结果（`dirty` 的三态语义同 [`RecentBrief::dirty_files`]）。
struct GitProbe {
    dirty: Option<Vec<String>>,
    diff_stat: Option<String>,
    problem: Option<String>,
}

fn probe_git(dir: &Path) -> GitProbe {
    if !dir.is_dir() {
        return GitProbe {
            dirty: None,
            diff_stat: None,
            problem: Some(format!("项目目录不存在：{}", dir.display())),
        };
    }
    let status = match run_git(dir, &["status", "--short"]) {
        Some((0, out)) => out,
        Some((code, _)) => {
            return GitProbe {
                dirty: None,
                diff_stat: None,
                problem: Some(format!("git status 退出码 {code}（非 git 仓库？）")),
            }
        }
        None => {
            return GitProbe {
                dirty: None,
                diff_stat: None,
                problem: Some("git status 超时（5s）或 git 不可用".to_string()),
            }
        }
    };
    let dirty: Vec<String> = status
        .lines()
        .filter_map(|line| {
            // `git status --short` 的格式是 "XY PATH"：前 3 字节固定为 ASCII 状态码 + 空格。
            let rest = line.get(3..)?.trim();
            (!rest.is_empty()).then(|| rest.to_string())
        })
        .take(20)
        .collect();
    let diff_stat = if dirty.is_empty() {
        None
    } else {
        run_git(dir, &["diff", "--stat"]).and_then(|(code, out)| {
            if code != 0 {
                return None;
            }
            out.lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map(|l| l.trim().to_string())
        })
    };
    GitProbe {
        dirty: Some(dirty),
        diff_stat,
        problem: None,
    }
}

/// 带 5s 超时地跑一条 git 命令；超时 / 启动失败 / 非零退出都返回 `None` 或退出码，不 panic。
///
/// 超时后后台线程自行结束（其结果投递到已关闭的 channel 被忽略），**不阻塞调用方**。
fn run_git(dir: &Path, args: &[&str]) -> Option<(i32, String)> {
    let dir = dir.to_path_buf();
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let (tx, rx) = std::sync::mpsc::channel();
    let _handle = std::thread::spawn(move || {
        let mut cmd = std::process::Command::new("git");
        cmd.args(&args).current_dir(&dir).silent();
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let result = cmd.output().map(|o| {
            (
                o.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&o.stdout).to_string(),
            )
        });
        let _ = tx.send(result);
    });
    match rx.recv_timeout(GIT_TIMEOUT) {
        Ok(Ok(v)) => Some(v),
        Ok(Err(_)) | Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::compact_state::ActivePlan;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mgt-continuity-{}-{}-{}",
            std::process::id(),
            tag,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("临时目录必须可创建");
        for args in [
            vec!["init"],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "user.name", "t"],
        ] {
            run_git(&dir, args.as_slice()).expect("git 初始化命令必须成功（测试机需有 git）");
        }
        std::fs::write(dir.join("base.txt"), "base").expect("写入基线文件");
        run_git(&dir, &["add", "base.txt"]).expect("git add 必须成功");
        run_git(&dir, &["commit", "-m", "init"]).expect("git commit 必须成功");
        dir
    }

    /// 脏工作区：必须有注入，且带上真实未提交路径。
    #[test]
    fn dirty_workspace_injects_git_brief() {
        let dir = temp_repo("dirty");
        std::fs::write(dir.join("dirty.txt"), "x").expect("写入脏文件");
        let state = CompactState {
            branch: Some("main".to_string()),
            ..Default::default()
        };
        let mut brief = recent_brief(&[state], dir.to_str());
        assert!(
            matches!(&brief.dirty_files, Some(d) if d.iter().any(|p| p == "dirty.txt")),
            "脏文件必须被读到：{:?}",
            brief.dirty_files
        );
        let block = brief.render().expect("脏工作区必须注入简报");
        assert!(block.starts_with(CONTINUITY_HEADER), "{block}");
        assert!(block.contains("dirty.txt"), "未提交路径必须在块里：{block}");
        assert!(block.contains("分支：main"), "已知分支应随行：{block}");
        assert!(
            block.chars().count() <= BRIEF_CHAR_BUDGET,
            "简报 {} 字符超过预算 {BRIEF_CHAR_BUDGET}",
            block.chars().count()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 干净工作区：不注入（干净不是信息）。
    #[test]
    fn clean_workspace_injects_nothing() {
        let dir = temp_repo("clean");
        let mut brief = recent_brief(&[], dir.to_str());
        assert!(
            matches!(&brief.dirty_files, Some(d) if d.is_empty()),
            "干净必须是「确认的空列表」而不是 None：{:?}",
            brief.dirty_files
        );
        assert!(brief.render().is_none(), "干净工作区不得注入任何文本");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 无 CompactState：notes 必须逐字段点名为什么是空的。
    #[test]
    fn missing_compact_states_are_explained_in_notes() {
        let brief = recent_brief(&[], None);
        assert!(brief.branch.is_none());
        assert!(brief.dirty_files.is_none(), "读不到 ≠ 干净");
        assert!(brief.test_status.is_none());
        assert!(brief.last_user_goal.is_none());
        let notes = brief.notes.join(" | ");
        assert!(notes.contains("CompactState"), "{notes}");
        assert!(notes.contains("test_status"), "{notes}");
        assert!(notes.contains("last_user_goal"), "{notes}");
        assert!(notes.contains("未绑定项目目录"), "{notes}");
    }

    /// 超预算：置 truncated 且总长不超上限。
    #[test]
    fn over_budget_brief_is_truncated_with_flag() {
        let state = CompactState {
            branch: Some("feature/long-branch".to_string()),
            dirty_files: (0..50)
                .map(|i| format!("core/src/some/really/long/path/number-{i}.rs"))
                .collect(),
            active_plan: Some(ActivePlan {
                plan_id: "p-1".to_string(),
                title: "目标".repeat(400),
                current_step: None,
            }),
            ..Default::default()
        };
        let mut brief = recent_brief(&[state], None);
        let block = brief.render().expect("有内容应注入");
        assert!(brief.truncated, "超长必须置 truncated");
        assert!(
            block.chars().count() <= BRIEF_CHAR_BUDGET,
            "截断后 {} 字符仍超预算 {BRIEF_CHAR_BUDGET}",
            block.chars().count()
        );
    }
}
