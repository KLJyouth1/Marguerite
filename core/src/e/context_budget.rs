//! Track E — Context budget 四态策略（对译 opensquilla `context_budget.py` 的治理意图）。
//!
//! 纯逻辑、零依赖、可单测。将"已用 / 上限"比例映射为四态阶梯，供缓存/上下文窗口
//! 驱逐决策消费。阈值常量可配（见下方 `OK_FRACTION` / `NEAR_LIMIT_FRACTION`）。

/// 低于该比例视为健康（Ok）。对译 opensquilla `context_overflow_threshold` 的低端。
pub const OK_FRACTION: f64 = 0.80;
/// 处于 [OK_FRACTION, NEAR_LIMIT_FRACTION) 视为临近上限（NearLimit）。
pub const NEAR_LIMIT_FRACTION: f64 = 0.95;
// (NEAR_LIMIT_FRACTION, 1.0] => OverLimit；> 1.0 => MustEvict。

/// 上下文预算四态阶梯。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetState {
    /// 充足（used < 80%）。
    Ok,
    /// 临近上限（80% ≤ used < 95%）。
    NearLimit,
    /// 超过软上限（95% ≤ used ≤ 100%）。
    OverLimit,
    /// 硬溢出，必须驱逐（used > 100%）。
    MustEvict,
}

/// 一个带上限的上下文预算快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudget {
    pub max_tokens: u64,
    pub used: u64,
}

impl ContextBudget {
    #[must_use]
    pub fn new(max_tokens: u64) -> Self {
        Self {
            max_tokens,
            used: 0,
        }
    }

    /// 按四态阶梯评估当前填充比例。
    ///
    /// 退化窗口（`max_tokens == 0`）视为已满 -> `MustEvict`（无法再接受任何 token）。
    #[must_use]
    pub fn assess(&self) -> BudgetState {
        if self.max_tokens == 0 {
            return BudgetState::MustEvict;
        }
        let ratio = self.used as f64 / self.max_tokens as f64;
        if ratio < OK_FRACTION {
            BudgetState::Ok
        } else if ratio < NEAR_LIMIT_FRACTION {
            BudgetState::NearLimit
        } else if ratio <= 1.0 {
            BudgetState::OverLimit
        } else {
            BudgetState::MustEvict
        }
    }

    /// 加入 `extra` token 后是否仍不超出窗口（`saturating_add` 防恶意溢出）。
    #[must_use]
    pub fn can_accept(&self, extra: u64) -> bool {
        self.used.saturating_add(extra) <= self.max_tokens
    }

    /// 返回加入 `extra` token 后的新预算，夹紧在 `max_tokens`（绝不溢出）。
    #[must_use]
    pub fn after_add(&self, extra: u64) -> ContextBudget {
        let used = self.used.saturating_add(extra).min(self.max_tokens);
        ContextBudget {
            max_tokens: self.max_tokens,
            used,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assess_four_state_boundaries() {
        // 以 max_tokens = 1000 验证 79/80/94/95/100/101% 的精确边界。
        // 直接构造（不经 after_add 夹紧）以覆盖 >100% 的硬溢出态。
        let mk = |used: u64| ContextBudget {
            max_tokens: 1000,
            used,
        };
        assert_eq!(mk(799).assess(), BudgetState::Ok); // 79.9%
        assert_eq!(mk(800).assess(), BudgetState::NearLimit); // 80.0%
        assert_eq!(mk(949).assess(), BudgetState::NearLimit); // 94.9%
        assert_eq!(mk(950).assess(), BudgetState::OverLimit); // 95.0%
        assert_eq!(mk(1000).assess(), BudgetState::OverLimit); // 100.0%
        assert_eq!(mk(1010).assess(), BudgetState::MustEvict); // 101.0%
    }

    #[test]
    fn assess_zero_window_is_must_evict() {
        let b = ContextBudget {
            max_tokens: 0,
            used: 0,
        };
        assert_eq!(b.assess(), BudgetState::MustEvict);
    }

    #[test]
    fn can_accept_respects_window_and_overflow() {
        let b = ContextBudget {
            max_tokens: 1000,
            used: 900,
        };
        assert!(b.can_accept(100)); // 恰好到上限
        assert!(!b.can_accept(101)); // 超出
                                     // 即便 extra 极大也不溢出判据
        assert!(!b.can_accept(u64::MAX));
    }

    #[test]
    fn after_add_clamps_no_overflow() {
        let b = ContextBudget {
            max_tokens: 1000,
            used: 950,
        };
        let next = b.after_add(100); // 1050 应夹紧到 1000
        assert_eq!(next.used, 1000);
        assert!(next.used <= next.max_tokens);

        // 极大 extra 触发 saturating，仍夹紧到 max
        let overflow = b.after_add(u64::MAX);
        assert_eq!(overflow.used, 1000);
    }
}
