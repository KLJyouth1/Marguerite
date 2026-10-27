//! 工作区解析与「可执行提示」（P0-1b）
//!
//! # 为什么需要这个模块
//!
//! 实测的失败链第 1 步是「会话 `project_path = null` —— 工作区未绑定」：
//! 智能体不知道文件该写到哪，于是**猜**相对路径，`str_replace_editor` 1 ms 即失败，
//! 随后进入每 2–4 秒一轮的死循环重试。真正缺的不是"再一个工作区变量"，而是两件事：
//!
//! 1. **一个权威解析器**：把「显式绑定 → 应用数据目录 → 进程 cwd」这三级回落
//!    收敛到一处。此前 `orchestration::default_workspace()` 与
//!    `gateway::sandbox_paths::sandbox_path_root()` 各写一半，谁都不完整。
//! 2. **可执行提示**：未绑定时必须告诉模型/用户「文件现在会落到哪、怎么改」。
//!    旧文案只有一句 `path escapes workspace: <rel>` —— 模型既不知道根在哪，
//!    也不知道怎么绑定，于是同一个错误重复到触发重复失败检测为止。
//!
//! # 设计约束
//!
//! - 纯解析 + 文案，**不做权限判定**（逃逸判定仍在 `tools::fs_path::safe_resolve`）。
//! - `guidance()` 的文案是给**模型**看的系统提示片段，必须自包含：根目录、当前
//!   绑定状态、以及绑定动作，缺一项就会重演「猜路径」。
//! - 未绑定**不是错误**：回落到应用数据目录是合法的默认行为，只是必须可见。

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// 工作区来源：决定提示口吻（已绑定 = 陈述事实；未绑定 = 告知落地位置 + 绑定方法）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceSource {
    /// 会话/项目显式绑定（用户在 UI 选择了项目目录）。
    Bound,
    /// 未绑定 → 应用数据目录（`Config::data_dir()`）。不是用户项目。
    AppData,
    /// 数据目录不可用 → 进程当前工作目录。
    ProcessCwd,
    /// 显式指定了路径，但它此刻**不是目录**（已被移动/删除/从未创建）。
    /// 刻意不静默改指别处：注册表里所有工具都以这个根在跑，指引必须与工具
    /// 行为一致，否则模型按指引写的路径照样 1ms 失败。
    Missing,
}

impl WorkspaceSource {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkspaceSource::Bound => "bound",
            WorkspaceSource::AppData => "app_data",
            WorkspaceSource::ProcessCwd => "process_cwd",
            WorkspaceSource::Missing => "missing",
        }
    }
}

/// 解析后的工作区（根目录 + 来源）。
///
/// P2-9：`roots` 为「一个会话可绑定多个项目目录」预留的根集合。当前工具侧仍只
/// 支持单根，故 `root` 保持为权威主根（= `roots[0]`），其余根仅供数据层/解析层
/// 记录与后续扩展；`guidance()` 也仍按单根叙述（见函数注释）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub root: PathBuf,
    pub source: WorkspaceSource,
    /// 本会话的全部项目根（首项恒等于 `root`）。
    pub roots: Vec<PathBuf>,
}

impl Workspace {
    /// 单根构造助手：`root` 同时作为根集合的唯一元素。
    pub fn new_single(root: PathBuf, source: WorkspaceSource) -> Self {
        Self {
            roots: vec![root.clone()],
            root,
            source,
        }
    }

    /// 多根构造：`primary` 恒为首项，`extra` 去重（保持给定顺序）后追加。
    ///
    /// 工具侧仍只消费 `root`；本构造只为数据层保留完整根集合，避免将来加多根
    /// 支持时再改一遍所有构造点。
    pub fn with_roots(primary: PathBuf, extra: Vec<PathBuf>, source: WorkspaceSource) -> Self {
        let mut roots = Vec::with_capacity(extra.len() + 1);
        roots.push(primary.clone());
        for p in extra {
            if p != primary && !roots.contains(&p) {
                roots.push(p);
            }
        }
        Self {
            root: primary,
            roots,
            source,
        }
    }

    /// 本会话绑定的全部根（首项恒为主根）。
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// 解析工作区根目录。
    ///
    /// 优先级：`explicit`（会话绑定的项目目录）> 应用数据目录 > 进程 cwd。
    /// `explicit` 指向一个**存在且是目录**的路径时才视为已绑定；若它此刻不是
    /// 目录（被删除/移动/拼错），仍以它作为根并标记 [`WorkspaceSource::Missing`]
    /// ——因为工具注册表已经按这个根在跑，改指别处只会让指引与工具行为不一致。
    pub fn resolve(explicit: Option<&Path>) -> Self {
        if let Some(p) = explicit {
            let trimmed = p.to_path_buf();
            if trimmed.is_dir() {
                // 刻意**不** canonicalize：Windows 上会带上 `\\?\` 前缀，
                // 那会让 guidance / 工具报错里出现一串用户认不出来的路径。
                // 逃逸判定由 `tools::fs_path::safe_resolve` 自己做（它内部会 canonicalize）。
                return Self::new_single(trimmed, WorkspaceSource::Bound);
            }
            // 显式路径此刻不可用：保留它作为根（首次写入会自动创建），
            // 并由 guidance 说明「这不是默认目录、必要时请重新绑定」。
            return Self::new_single(trimmed, WorkspaceSource::Missing);
        }
        let (root, source) = fallback();
        Self::new_single(root, source)
    }

    /// 是否已绑定真实项目目录。
    pub fn is_bound(&self) -> bool {
        matches!(self.source, WorkspaceSource::Bound)
    }

    /// 面向模型的系统提示片段（自包含：根目录 + 状态 + 绑定方法）。
    ///
    /// 这不是装饰文案：它是「猜路径 → 1 ms 失败 → 死循环」这条链的第一道防线。
    ///
    /// P2-9：文案**刻意保持单根叙述**。工具侧（多根读写）尚未支持，若在这里
    /// 告诉模型「可以访问 roots 里的其他目录」，它就会去写那些路径并 1ms 失败。
    /// 多根引导等工具侧支持后再加。
    pub fn guidance(&self) -> String {
        let root = self.root.display().to_string();
        match self.source {
            WorkspaceSource::Bound => format!(
                "【工作区】{root}（已绑定项目目录）。\n\
                 - 所有文件工具的工作区根就是这个目录：相对路径相对它解析；\
                 绝对路径必须位于它内部，否则会被拒绝。\n\
                 - 交付产物请写到这个目录下，并用工作区内相对路径回读。",
            ),
            WorkspaceSource::AppData | WorkspaceSource::ProcessCwd => format!(
                "【工作区】{root}（未绑定项目目录，当前使用{desc}）。\n\
                 - 文件会写入这个目录；这不是用户的项目目录，只是默认落点。\n\
                 - 如果任务面向某个真实项目，请先让用户在界面上把这个会话绑定到\
                 项目目录（会话的 projectPath 指向它），再按绑定后的根读写；\
                 在绑定前，请使用上面的绝对路径，**不要猜测**其它目录。",
                desc = match self.source {
                    WorkspaceSource::AppData => "应用数据目录",
                    _ => "进程当前工作目录",
                },
            ),
            WorkspaceSource::Missing => format!(
                "【工作区】{root}（已指定，但该目录当前不存在：可能被移动、删除或从未创建）。\n\
                 - 所有文件工具仍以这个目录为工作区根：首次写入时会自动创建它。\n\
                 - 请只在这个目录内读写；如果它对应的项目已经不在原处，\
                 请让用户重新绑定项目目录后再继续，**不要猜测**其它目录顶替。",
            ),
        }
    }

    /// 工具失败时附带的路径提示：把「根在哪 + 该怎么写」说清楚。
    pub fn path_hint(&self) -> String {
        format!(
            "工作区根：{}；请改用该目录内的相对路径",
            self.root.display()
        )
    }

    /// 状态视图（供 `workspace.status` RPC 与 doctor 项使用）。
    pub fn status_json(&self) -> Value {
        json!({
            "ok": true,
            "root": self.root.display().to_string(),
            "source": self.source.as_str(),
            "bound": self.is_bound(),
            "guidance": self.guidance(),
            "git": git_summary(&self.root),
        })
    }
}

/// 只读的 Git 摘要：分支名、是否 detached、本地分支列表。
///
/// 刻意**不执行 `git` 进程**：`status_json` 会在对话/诊断面板频繁调用，拉起外部
/// 进程既慢又依赖用户 PATH。这里只读 `.git/HEAD` 与 `refs`，并处理三种真实布局：
/// 普通仓库（`.git` 目录）、worktree（`.git` 文件 + `gitdir:`）、以及 `commondir`
/// 指向主仓库的 refs。读不到就返回 `null`（不是 Git 仓库），UI 据此不显示分支胶囊。
fn git_summary(root: &Path) -> Option<Value> {
    let dot = root.join(".git");
    let git_dir = if dot.is_dir() {
        dot
    } else if dot.is_file() {
        let text = std::fs::read_to_string(&dot).ok()?;
        let raw = text
            .lines()
            .find_map(|line| line.strip_prefix("gitdir:"))?
            .trim();
        let path = PathBuf::from(raw);
        if path.is_absolute() {
            path
        } else {
            root.join(path)
        }
    } else {
        return None;
    };
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    let (branch, detached) = match head.strip_prefix("ref: refs/heads/") {
        Some(name) => (Some(name.to_string()), false),
        None => {
            let looks_like_sha = head.len() >= 7 && head.chars().all(|c| c.is_ascii_hexdigit());
            (None, looks_like_sha)
        }
    };
    // worktree 的 refs 在 commondir 里；普通仓库没有 commondir 文件。
    let common = std::fs::read_to_string(git_dir.join("commondir"))
        .ok()
        .map(|raw| {
            let path = PathBuf::from(raw.trim());
            if path.is_absolute() {
                path
            } else {
                git_dir.join(path)
            }
        })
        .unwrap_or_else(|| git_dir.clone());
    let mut branches: Vec<String> = Vec::new();
    collect_branch_refs(&common.join("refs").join("heads"), "", 0, &mut branches);
    // 已被 `git pack-refs` 打包的分支不在 refs/ 目录里，必须一并读出。
    if let Ok(packed) = std::fs::read_to_string(common.join("packed-refs")) {
        for line in packed.lines() {
            if line.starts_with('#') || line.starts_with('^') {
                continue;
            }
            if let Some((_, name)) = line.split_once(' ') {
                if let Some(short) = name.trim().strip_prefix("refs/heads/") {
                    branches.push(short.to_string());
                }
            }
        }
    }
    branches.sort();
    branches.dedup();
    branches.truncate(200);
    Some(json!({
        "branch": branch,
        "detached": detached,
        "branches": branches,
        "gitDir": git_dir.display().to_string(),
    }))
}

fn collect_branch_refs(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<String>) {
    // 分支名允许 `feature/x` 这种层级，但目录深度必须有上限（防止异常仓库拖慢 UI）。
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let path = entry.path();
        if path.is_dir() {
            collect_branch_refs(&path, &format!("{prefix}{name}/"), depth + 1, out);
        } else {
            out.push(format!("{prefix}{name}"));
        }
    }
}

/// 默认根与来源（无显式绑定时）：应用数据目录优先，进程 cwd 兜底。
///
/// 一次算出「根 + 来源」两件事：分成两个函数会让 `Config::data_dir()` 被
/// 求值两次，且存在两次之间环境变化的极小窗口（根与来源不一致）。
fn fallback() -> (PathBuf, WorkspaceSource) {
    let data = crate::persistence::config::Config::data_dir();
    if data.is_dir() {
        (data, WorkspaceSource::AppData)
    } else {
        (
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            WorkspaceSource::ProcessCwd,
        )
    }
}

/// 当前会话绑定的项目目录 → 解析成工作区；None 表示未绑定。
///
/// 统一的入口：网关的 chat / team / plan 三条 dispatch 路径都应经此函数取根，
/// 避免各处重复实现「有绑定用绑定、没有用 data_dir」。
pub fn resolve_for_session(project_path: Option<&str>) -> Workspace {
    let explicit = project_path
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(Path::new);
    Workspace::resolve(explicit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("mgt_ws_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn with_roots_dedups_and_keeps_primary_first() {
        let primary = PathBuf::from("/proj/main");
        let extra = vec![
            PathBuf::from("/proj/extra"),
            PathBuf::from("/proj/main"),   // 与主根重复 → 丢弃
            PathBuf::from("/proj/extra"),  // 自身重复 → 丢弃
            PathBuf::from("/proj/third"),
        ];
        let ws = Workspace::with_roots(primary.clone(), extra, WorkspaceSource::Bound);
        assert_eq!(
            ws.roots().to_vec(),
            vec![
                PathBuf::from("/proj/main"),
                PathBuf::from("/proj/extra"),
                PathBuf::from("/proj/third"),
            ],
            "去重后保持给定顺序，且 primary 恒在首位"
        );
        assert_eq!(ws.root, primary, "root 必须仍是主根");
        assert_eq!(ws.roots[0], ws.root, "roots 首项必须等于 root");
        assert_eq!(ws.roots().len(), 3);
    }

    #[test]
    fn explicit_existing_dir_is_bound() {
        let d = temp_dir("bound");
        let ws = Workspace::resolve(Some(&d));
        assert!(ws.is_bound(), "显式绑定且目录存在必须视为已绑定");
        assert_eq!(ws.source, WorkspaceSource::Bound);
        assert!(ws.root.is_dir());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_explicit_dir_keeps_the_declared_root_and_says_so() {
        let ghost = std::env::temp_dir().join("mgt_ws_definitely_missing_dir");
        let _ = std::fs::remove_dir_all(&ghost);
        let ws = Workspace::resolve(Some(&ghost));
        assert!(!ws.is_bound(), "目录不存在不得谎称已绑定");
        assert_eq!(
            ws.root, ghost,
            "必须保留用户指定的根——工具注册表正是按它跑的，改指别处会让指引与工具行为不一致"
        );
        assert_eq!(ws.source, WorkspaceSource::Missing);
        let g = ws.guidance();
        assert!(g.contains("不存在"), "必须说明目录当前不存在");
        assert!(g.contains("重新绑定"), "必须给出可执行的下一步");
        assert!(g.contains("不要猜测"), "必须堵住'猜一个目录顶替'这条路");
    }

    #[test]
    fn none_resolves_to_an_existing_root() {
        let ws = Workspace::resolve(None);
        assert!(ws.root.is_dir(), "默认工作区根必须存在");
        assert!(!ws.is_bound());
    }

    #[test]
    fn guidance_is_self_contained_for_both_states() {
        let d = temp_dir("guidance");
        let bound = Workspace::resolve(Some(&d));
        let g = bound.guidance();
        assert!(g.contains(&d.display().to_string()), "必须写出根目录");
        assert!(g.contains("已绑定"), "已绑定状态要说明");
        assert!(!g.contains("不要猜测"), "已绑定时不要求绑定动作");

        let unbound = Workspace::resolve(None);
        let g2 = unbound.guidance();
        assert!(g2.contains(&unbound.root.display().to_string()));
        assert!(g2.contains("未绑定"), "未绑定状态要说明");
        assert!(
            g2.contains("不要猜测"),
            "未绑定时必须给出可执行动作，否则模型继续猜路径"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 长会话里同一装配必须产出同一段指引（KV-cache 前缀稳定性的前提）。
    #[test]
    fn guidance_is_byte_stable_for_the_same_root() {
        let d = temp_dir("stable");
        let a = Workspace::resolve(Some(&d));
        let b = Workspace::resolve(Some(&d));
        assert_eq!(a.guidance(), b.guidance());
        assert_eq!(a.path_hint(), b.path_hint());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 护栏：指引里反引号包住的 RPC 名必须**真实存在**。
    ///
    /// 本仓已因 `tools.catalog` 宣称 `file_read` / `terminal_exec` / `kb_search`
    /// 等编造工具名吃过教训（前端徽标恒不显示、模型一调就失败）。给模型的系统
    /// 提示同理：写一个不存在的方法名，等于教模型去调一个必然失败的接口。
    /// 该用例即挡住"提示词漂移到不存在的 API"。
    #[test]
    fn guidance_names_no_fabricated_rpc() {
        fn phantom_rpcs(g: &str) -> Vec<String> {
            let mut out = Vec::new();
            let mut rest = g;
            while let Some(open) = rest.find('`') {
                let after = &rest[open + 1..];
                let Some(close) = after.find('`') else { break };
                let name = &after[..close];
                // 只校验形如 `a.b` 的 RPC 名；纯代码片段（如 `x`）不在协议清单里，跳过。
                if name.contains('.')
                    && !crate::gateway::protocol::SUPPORTED_METHODS.contains(&name)
                {
                    out.push(name.to_string());
                }
                rest = &after[close + 1..];
            }
            out
        }

        let d = temp_dir("phantom");
        for ws in [
            Workspace::resolve(Some(&d)),
            Workspace::resolve(None),
            Workspace::resolve(Some(&std::env::temp_dir().join("mgt_ws_phantom_missing"))),
        ] {
            let bad = phantom_rpcs(&ws.guidance());
            assert!(
                bad.is_empty(),
                "指引引用了不存在的 RPC：{bad:?}（完整指引：{}）",
                ws.guidance()
            );
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn path_hint_names_the_root() {
        let d = temp_dir("hint");
        let ws = Workspace::resolve(Some(&d));
        assert!(ws.path_hint().contains(&d.display().to_string()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn status_json_reports_git_branch_without_spawning_git() {
        let d = temp_dir("git");
        let git = d.join(".git");
        std::fs::create_dir_all(git.join("refs").join("heads").join("feature")).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git.join("refs").join("heads").join("main"), "deadbeef\n").unwrap();
        std::fs::write(
            git.join("refs").join("heads").join("feature").join("chat"),
            "feedface\n",
        )
        .unwrap();
        std::fs::write(
            git.join("packed-refs"),
            "# pack-refs with: peeled fully-peeled sorted \n\
             c0ffee00 refs/heads/release\n",
        )
        .unwrap();

        let v = Workspace::resolve(Some(&d)).status_json();
        assert_eq!(v["git"]["branch"], json!("main"));
        assert_eq!(v["git"]["detached"], json!(false));
        let branches = v["git"]["branches"].as_array().cloned().unwrap_or_default();
        let names: Vec<&str> = branches.iter().filter_map(Value::as_str).collect();
        assert!(names.contains(&"main"), "实际分支：{names:?}");
        assert!(names.contains(&"feature/chat"), "嵌套分支必须保留全名：{names:?}");
        assert!(names.contains(&"release"), "packed-refs 里的分支必须读出：{names:?}");

        // detached HEAD：没有 ref，只有提交号。
        std::fs::write(git.join("HEAD"), "0123456789abcdef\n").unwrap();
        let v2 = Workspace::resolve(Some(&d)).status_json();
        assert_eq!(v2["git"]["branch"], Value::Null);
        assert_eq!(v2["git"]["detached"], json!(true));

        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn status_json_git_is_null_outside_a_repository() {
        let d = temp_dir("nogit");
        let v = Workspace::resolve(Some(&d)).status_json();
        assert_eq!(v["git"], Value::Null);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn status_json_is_honest_about_binding() {
        let d = temp_dir("status");
        let v = Workspace::resolve(Some(&d)).status_json();
        assert_eq!(v["bound"], json!(true));
        assert_eq!(v["source"], json!("bound"));
        assert_eq!(v["ok"], json!(true));
        assert!(v["guidance"].as_str().is_some_and(|s| !s.is_empty()));

        let ghost = std::env::temp_dir().join("mgt_ws_status_missing");
        let _ = std::fs::remove_dir_all(&ghost);
        let v2 = Workspace::resolve(Some(&ghost)).status_json();
        assert_eq!(v2["bound"], json!(false));
        assert_eq!(v2["source"], json!("missing"));
        assert_eq!(v2["root"], json!(ghost.display().to_string()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn resolve_for_session_ignores_blank_paths() {
        let ws = resolve_for_session(Some("   "));
        assert!(!ws.is_bound(), "空白字符串不得当成绑定");
        let ws2 = resolve_for_session(None);
        assert_eq!(ws.source, ws2.source);
    }
}
