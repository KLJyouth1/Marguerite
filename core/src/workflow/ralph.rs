//! Ralph 全新子代理循环（对齐 DSH `dsh-tool-ralph`）。
//!
//! 每轮开启一个**全新**子代理（不继承上一轮对话种子），工作区共享为持久记忆，
//! 轮间只传递有界结构化报告。语义对齐：
//! - 首轮提示词 = 目标 + 指令「把工作区当持久记忆；完成回报 `completed`，受阻回报
//!   `blocker: <原因>`」；
//! - 后续轮 = 目标 + 上一轮报告 + 同样指令；
//! - 每轮经 [`parse_report`] 判定结局：`blocker:` → 受阻（取冒号后文本，空则默认
//!   `unspecified blocker`）、`completed` → 完成、否则继续；到 `max_rounds` 上限仍未
//!   终结 → [`RalphOutcome::RoundLimit`]。（先判 blocker，见 [`parse_report`] 的说明。）
//!
//! 执行体把「每轮 spawn 子代理」做成同步闭包注入（[`RalphRunner::run_with`]），
//! 测试注入确定性假回复、生产接真实 `SubAgentManager`；纯同步逻辑、离线可测。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 缺省最大轮数（未显式指定时使用）。
pub const DEFAULT_MAX_ROUNDS: u32 = 8;
/// 最大轮数硬上限（构造时 clamp 到 `[1, MAX_ROUNDS]`）。
pub const MAX_ROUNDS: u32 = 16;

/// 单轮结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RalphRound {
    /// 轮次（从 1 开始）。
    pub round: u32,
    /// 该轮子代理回报的原文（有界报告）。
    pub report: String,
    /// 是否终结（completed / blocked）。
    pub terminal: bool,
}

/// Ralph 循环结局。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RalphOutcome {
    /// 完成（子代理回报 `completed`）。
    Completed,
    /// 受阻（子代理回报 `blocker: <原因>`）。
    Blocked { reason: String },
    /// 达到轮数上限仍未终结。
    RoundLimit,
}

/// Ralph 循环报告（结构化、有界）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RalphReport {
    /// 目标。
    pub objective: String,
    /// 实际已运行轮数。
    pub rounds_run: u32,
    /// 各轮明细。
    pub rounds: Vec<RalphRound>,
    /// 结局。
    pub outcome: RalphOutcome,
}

/// Ralph 运行器：每轮全新子代理、工作区共享为持久记忆、只传有界结构化报告。
#[derive(Debug, Clone)]
pub struct RalphRunner {
    objective: String,
    max_rounds: u32,
}

impl RalphRunner {
    /// 构造运行器；`max_rounds` 会被 clamp 到 `[1, 16]`。
    pub fn new(objective: String, max_rounds: u32) -> Self {
        Self {
            objective,
            max_rounds: max_rounds.clamp(1, MAX_ROUNDS),
        }
    }

    /// 缺省上限（[`DEFAULT_MAX_ROUNDS`] = 8 轮）构造。
    pub fn with_default_rounds(objective: String) -> Self {
        Self::new(objective, DEFAULT_MAX_ROUNDS)
    }

    /// 目标文本。
    pub fn objective(&self) -> &str {
        &self.objective
    }

    /// 轮数上限（已 clamp）。
    pub fn max_rounds(&self) -> u32 {
        self.max_rounds
    }

    /// 生成下一轮提示词（纯函数）。
    ///
    /// 首轮（`last = None`）= 目标 + 指令；后续轮 = 目标 + 上一轮报告 + 同样指令。
    pub fn next_prompt(&self, last: Option<&RalphRound>) -> String {
        const INSTRUCTION: &str =
            "把工作区当作持久记忆；完成则回报 completed，受阻则回报 blocker: <原因>。";
        match last {
            None => format!("目标：{}\n\n{}", self.objective, INSTRUCTION),
            Some(r) => format!(
                "目标：{}\n\n上一轮报告：\n{}\n\n{}",
                self.objective, r.report, INSTRUCTION
            ),
        }
    }

    /// 执行 Ralph 循环（同步、可注入 spawn）。
    ///
    /// `spawn(round, prompt) -> report_text` 由调用方提供：测试注入确定性假回复，
    /// 生产侧接真实 `SubAgentManager`（每轮 spawn 一个全新子代理并回收结果）。
    ///
    /// 逐轮：调 spawn → [`parse_report`] → 写工作区记忆文件（失败只影响记忆，不中断循环）
    /// → 终结或到上限即停。
    pub fn run_with<F>(&self, workspace: &str, mut spawn: F) -> RalphReport
    where
        F: FnMut(u32, &str) -> String,
    {
        let name = memory_name(&self.objective);
        let mut rounds: Vec<RalphRound> = Vec::new();
        let mut outcome = RalphOutcome::RoundLimit;

        for round in 1..=self.max_rounds {
            let prompt = self.next_prompt(rounds.last());
            let report_text = spawn(round, &prompt);
            let (terminal, reason) = parse_report(&report_text);
            let r = RalphRound {
                round,
                report: report_text,
                terminal,
            };
            // 写记忆文件：失败只影响记忆，不中断循环。
            let _ = write_memory(workspace, &name, &self.objective, &r, &reason);
            rounds.push(r);
            if terminal {
                outcome = if reason.is_empty() {
                    RalphOutcome::Completed
                } else {
                    RalphOutcome::Blocked { reason }
                };
                break;
            }
        }

        let rounds_run = rounds.len() as u32;
        RalphReport {
            objective: self.objective.clone(),
            rounds_run,
            rounds,
            outcome,
        }
    }
}

/// 解析子代理报告文本为 `(terminal, reason)`。
///
/// 判定顺序：
/// - 含 `blocker:`（大小写不敏感）→ 受阻（`(true, 冒号后文本)`，空则默认
///   `"unspecified blocker"`）；
/// - 否则含 `completed`（大小写不敏感）→ 完成（`(true, "")`）；
/// - 否则继续（`(false, "")`）。
///
/// ⚠️ **必须先判 `blocker:` 再判 `completed`**。历史缺陷：原实现先查 `completed`，
/// 于是 `"blocker: task not completed"` / `"blocker: 尚未 completed"` 这类**否定式**
/// 报告被误判为「已完成」，Ralph 会在任务实际受阻时提前收工。显式声明的 blocker
/// 应压过任何 `completed` 字样。
pub fn parse_report(text: &str) -> (bool, String) {
    let lower = text.to_ascii_lowercase();
    if let Some(pos) = lower.find("blocker:") {
        let after = text[pos + "blocker:".len()..].trim();
        let reason = if after.is_empty() {
            "unspecified blocker".to_string()
        } else {
            after.to_string()
        };
        return (true, reason);
    }
    if lower.contains("completed") {
        return (true, String::new());
    }
    (false, String::new())
}

/// 清理名称：非法字符（非 ascii 字母数字、非 `-`/`_`/`.`）替换为 `_`。
pub fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// 工作区记忆文件路径：`workspace/ralph/<sanitized_name>.json`。
pub fn memory_path(workspace: &str, name: &str) -> PathBuf {
    let safe = sanitize_name(name);
    PathBuf::from(workspace)
        .join("ralph")
        .join(format!("{safe}.json"))
}

/// 由目标派生记忆文件名（清理 + 去边界下划线 + 截断；空则回退 `ralph-run`）。
fn memory_name(objective: &str) -> String {
    let cleaned: String = sanitize_name(objective).trim_matches('_').to_string();
    if cleaned.is_empty() {
        "ralph-run".to_string()
    } else {
        cleaned.chars().take(48).collect()
    }
}

/// 写一轮记忆文件（JSON：objective + round + report + terminal + reason）。
///
/// 目录不存在则先 `create_dir_all`；写失败返回 `Err`（调用方忽略，只影响记忆不中断循环）。
fn write_memory(
    workspace: &str,
    name: &str,
    objective: &str,
    round: &RalphRound,
    reason: &str,
) -> std::io::Result<()> {
    let path = memory_path(workspace, name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let payload = serde_json::json!({
        "objective": objective,
        "round": round.round,
        "report": round.report,
        "terminal": round.terminal,
        "reason": reason,
    });
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&payload).unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 在系统临时目录下生成唯一子目录（避免并行测试互相踩踏）。
    fn unique_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "marguerite_ralph_{tag}_{}_{}",
            std::process::id(),
            nanos
        ))
    }

    #[test]
    fn parse_report_three_outcomes() {
        // completed（大小写不敏感）。
        assert_eq!(parse_report("任务已完成 completed"), (true, String::new()));
        assert_eq!(parse_report("COMPLETED done"), (true, String::new()));
        // blocker: 取冒号后文本。
        let (term, reason) = parse_report("blocker: 缺少网络权限");
        assert!(term);
        assert_eq!(reason, "缺少网络权限");
        // blocker: 大小写不敏感。
        let (term, reason) = parse_report("BLOCKER: no access");
        assert!(term);
        assert_eq!(reason, "no access");
        // blocker: 后为空 → 默认原因。
        assert_eq!(
            parse_report("blocker:"),
            (true, "unspecified blocker".to_string())
        );
        // 否则继续。
        assert_eq!(parse_report("继续思考中..."), (false, String::new()));
    }

    /// 回归守卫：否定式 `completed` 不得盖过显式 blocker。
    #[test]
    fn parse_report_blocker_beats_negative_completed() {
        // 历史缺陷：先判 `completed` → 这两条会被误判成「已完成」。
        let (term, reason) = parse_report("blocker: task not completed");
        assert!(term);
        assert_eq!(reason, "task not completed");

        let (term, reason) = parse_report("blocker: 尚未 completed");
        assert!(term);
        assert_eq!(reason, "尚未 completed");

        // 没有 blocker 时，`completed` 仍判完成。
        assert_eq!(parse_report("all done, completed"), (true, String::new()));
    }

    #[test]
    fn run_with_completed_path() {
        let runner = RalphRunner::new("写一个 hello world".to_string(), 5);
        let ws = unique_dir("completed").to_string_lossy().to_string();
        let mut calls = 0;
        let report = runner.run_with(&ws, |_round, prompt| {
            calls += 1;
            assert!(prompt.contains("写一个 hello world"));
            "completed".to_string()
        });
        assert_eq!(report.outcome, RalphOutcome::Completed);
        assert_eq!(report.rounds_run, 1);
        assert_eq!(report.rounds.len(), 1);
        assert!(report.rounds[0].terminal);
        assert_eq!(calls, 1, "完成路径应在首轮终结");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn run_with_blocked_path() {
        let runner = RalphRunner::new("目标".to_string(), 5);
        let ws = unique_dir("blocked").to_string_lossy().to_string();
        let report = runner.run_with(&ws, |_round, _prompt| "blocker: 数据缺失".to_string());
        assert_eq!(
            report.outcome,
            RalphOutcome::Blocked {
                reason: "数据缺失".to_string()
            }
        );
        assert_eq!(report.rounds_run, 1);
        assert!(report.rounds[0].terminal);
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn run_with_round_limit_path() {
        let runner = RalphRunner::new("目标".to_string(), 3);
        let ws = unique_dir("limit").to_string_lossy().to_string();
        let responses = ["仍在进行", "继续", "还在想"];
        let mut idx = 0usize;
        let report = runner.run_with(&ws, |_round, _prompt| {
            let r = responses[idx].to_string();
            idx += 1;
            r
        });
        assert_eq!(report.outcome, RalphOutcome::RoundLimit);
        assert_eq!(report.rounds_run, 3);
        assert_eq!(report.rounds.len(), 3);
        assert!(report.rounds.iter().all(|r| !r.terminal));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn run_with_writes_memory_file() {
        let dir = unique_dir("mem");
        let ws = dir.to_string_lossy().to_string();
        let runner = RalphRunner::new("build a demo app".to_string(), 3);
        let report = runner.run_with(&ws, |_round, _prompt| "blocker: 需要更多信息".to_string());
        assert_eq!(report.rounds_run, 1);

        let path = memory_path(&ws, &memory_name("build a demo app"));
        assert!(path.exists(), "记忆文件应写入: {}", path.display());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("build a demo app"));
        assert!(content.contains("blocker"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn max_rounds_clamped() {
        assert_eq!(RalphRunner::new("o".into(), 0).max_rounds(), 1);
        assert_eq!(RalphRunner::new("o".into(), 100).max_rounds(), 16);
        assert_eq!(RalphRunner::new("o".into(), 5).max_rounds(), 5);
        assert_eq!(RalphRunner::with_default_rounds("o".into()).max_rounds(), 8);
    }

    #[test]
    fn sanitize_name_replaces_illegal_chars() {
        assert_eq!(sanitize_name("a/b\\c:d"), "a_b_c_d");
        assert_eq!(sanitize_name("ok-name.123"), "ok-name.123");
        assert_eq!(sanitize_name("中文"), "__");
    }

    #[test]
    fn memory_path_shape() {
        let p = memory_path("/tmp/ws", "my/name");
        assert_eq!(
            p,
            PathBuf::from("/tmp/ws").join("ralph").join("my_name.json")
        );
    }
}
