//! 单 turn 预算硬熔断（P2-9）。
//!
//! 在工具回路（`run_tool_loop` / `run_tool_loop_stream` / `stream_loop`）每轮开头
//! 检查三类预算：**步数 / 时长 / token**。任一到达上限（`used >= limit`）即返回
//! 可识别的 [`BudgetBreach`]，回路据此中止并产出 `budget_exhausted` 结果（而不是
//! 静默截断或假装完成）。
//!
//! 与 Python 侧（pyengine `engine/turn_budget.py`）对齐：
//! - env：`MARGUERITE_TURN_MAX_STEPS` / `MARGUERITE_TURN_MAX_SECS` /
//!   `MARGUERITE_TURN_MAX_TOKENS`，均默认 `0` = 关闭；
//! - 触发优先级：steps → time → tokens（同一时刻只报最先命中的一类）；
//! - 结果体含 `budget:{kind,limit,used,remaining}`。
//!
//! 设计依据：docs/sidecar-stability-research.md §3.2 P2-9。

use std::time::Instant;

/// 预算类别（与 Python 侧序列化字符串一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetKind {
    Steps,
    Time,
    Tokens,
}

impl BudgetKind {
    /// 序列化名（进入 `budget.kind` / error code）。
    pub fn as_str(self) -> &'static str {
        match self {
            BudgetKind::Steps => "steps",
            BudgetKind::Time => "time",
            BudgetKind::Tokens => "tokens",
        }
    }
}

/// 一组 turn 预算上限（0 = 该项关闭）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TurnBudgetLimits {
    pub max_steps: u32,
    pub max_secs: u32,
    pub max_tokens: u32,
}

impl TurnBudgetLimits {
    /// 是否所有项都关闭（无任何熔断）。
    pub fn is_disabled(&self) -> bool {
        self.max_steps == 0 && self.max_secs == 0 && self.max_tokens == 0
    }
}

/// 一次预算越界（可识别的熔断结果）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetBreach {
    pub kind: BudgetKind,
    pub limit: u64,
    pub used: u64,
    pub remaining: u64,
}

impl BudgetBreach {
    /// 可识别错误码 / 终态串：`budget_exhausted`。
    pub fn code(&self) -> &'static str {
        "budget_exhausted"
    }

    /// 预算耗尽结构化回体（与 Python 侧字段对齐）：
    /// `{status:"budget_exhausted", budget:{kind,limit,used,remaining}, partial_text, task_id}`。
    pub fn to_json(&self, partial_text: &str, task_id: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "status": self.code(),
            "budget": {
                "kind": self.kind.as_str(),
                "limit": self.limit,
                "used": self.used,
                "remaining": self.remaining,
            },
            "partial_text": partial_text,
            "task_id": task_id,
        })
    }

    /// 越界的具体类别。
    pub fn kind_str(&self) -> &'static str {
        self.kind.as_str()
    }
}

/// 解析一个非负整数 env（0 = 关闭；非法/缺失 → 0，即不启用该项，
/// 避免一个拼写错误把整个回路熔断）。
fn parse_zero_default(raw: Option<&str>) -> u32 {
    raw.and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or_default()
}

/// 从当前进程 env 读取预算上限。
pub fn limits_from_env() -> TurnBudgetLimits {
    TurnBudgetLimits {
        max_steps: parse_zero_default(std::env::var("MARGUERITE_TURN_MAX_STEPS").ok().as_deref()),
        max_secs: parse_zero_default(std::env::var("MARGUERITE_TURN_MAX_SECS").ok().as_deref()),
        max_tokens: parse_zero_default(std::env::var("MARGUERITE_TURN_MAX_TOKENS").ok().as_deref()),
    }
}

fn breach(kind: BudgetKind, limit: u64, used: u64) -> BudgetBreach {
    BudgetBreach {
        kind,
        limit,
        used,
        remaining: limit.saturating_sub(used),
    }
}

/// 检查预算是否越界。
///
/// 参数：
/// - `limits`：上限（`is_disabled` 时直接返回 `None`）；
/// - `steps_used`：已完成的回路步数（轮数）；
/// - `turn_started`：本 turn 起点时刻（用于时长）；
/// - `tokens_used`：本 turn 累计 token（prompt+completion 估算，由调用方记账）。
///
/// 优先级 steps → time → tokens。`used >= limit` 判定（达到即触发，边界含等号）。
pub fn check_turn_budget(
    limits: &TurnBudgetLimits,
    steps_used: u32,
    turn_started: &Instant,
    tokens_used: u64,
) -> Option<BudgetBreach> {
    if limits.is_disabled() {
        return None;
    }
    // 1. 步数
    if limits.max_steps > 0 && steps_used >= limits.max_steps {
        return Some(breach(BudgetKind::Steps, limits.max_steps as u64, steps_used as u64));
    }
    // 2. 时长
    if limits.max_secs > 0 {
        let used_secs = turn_started.elapsed().as_secs();
        if used_secs >= limits.max_secs as u64 {
            return Some(breach(BudgetKind::Time, limits.max_secs as u64, used_secs));
        }
    }
    // 3. token
    if limits.max_tokens > 0 && tokens_used >= limits.max_tokens as u64 {
        return Some(breach(
            BudgetKind::Tokens,
            limits.max_tokens as u64,
            tokens_used,
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn parse_zero_default_table() {
        assert_eq!(parse_zero_default(None), 0);
        assert_eq!(parse_zero_default(Some("")), 0);
        assert_eq!(parse_zero_default(Some("abc")), 0);
        assert_eq!(parse_zero_default(Some("5")), 5);
        assert_eq!(parse_zero_default(Some(" 7 ")), 7);
    }

    #[test]
    fn disabled_limits_never_breach() {
        let lim = TurnBudgetLimits::default();
        assert!(lim.is_disabled());
        let start = Instant::now();
        assert_eq!(check_turn_budget(&lim, 999, &start, 9_999_999), None);
    }

    #[test]
    fn steps_breach_when_used_reaches_limit() {
        let lim = TurnBudgetLimits {
            max_steps: 3,
            ..Default::default()
        };
        let start = Instant::now();
        // 2 < 3 不触发
        assert_eq!(check_turn_budget(&lim, 2, &start, 0), None);
        // 3 >= 3 触发（含等号）
        let b = check_turn_budget(&lim, 3, &start, 0).expect("步数到顶应熔断");
        assert_eq!(b.kind, BudgetKind::Steps);
        assert_eq!(b.code(), "budget_exhausted");
        assert_eq!(b.limit, 3);
        assert_eq!(b.used, 3);
        assert_eq!(b.remaining, 0);
    }

    #[test]
    fn time_breach_after_elapsed() {
        let lim = TurnBudgetLimits {
            max_secs: 1,
            ..Default::default()
        };
        let start = Instant::now() - Duration::from_secs(2);
        let b = check_turn_budget(&lim, 0, &start, 0).expect("超时应熔断");
        assert_eq!(b.kind, BudgetKind::Time);
        assert_eq!(b.kind_str(), "time");
    }

    #[test]
    fn tokens_breach_priority_after_steps() {
        // 同时设 steps 和 tokens；steps 未到、tokens 到 → tokens
        let lim = TurnBudgetLimits {
            max_steps: 10,
            max_tokens: 100,
            ..Default::default()
        };
        let start = Instant::now();
        let b = check_turn_budget(&lim, 1, &start, 100).expect("token 到顶应熔断");
        assert_eq!(b.kind, BudgetKind::Tokens);
        assert_eq!(b.remaining, 0);
    }

    #[test]
    fn steps_has_priority_over_tokens() {
        let lim = TurnBudgetLimits {
            max_steps: 2,
            max_tokens: 10,
            ..Default::default()
        };
        let start = Instant::now();
        let b = check_turn_budget(&lim, 2, &start, 10_000).expect("应熔断");
        assert_eq!(b.kind, BudgetKind::Steps);
    }

    #[test]
    fn remaining_reports_headroom() {
        let lim = TurnBudgetLimits {
            max_tokens: 100,
            ..Default::default()
        };
        let start = Instant::now();
        let b = check_turn_budget(&lim, 0, &start, 100).unwrap();
        assert_eq!(b.remaining, 0);
    }
}
