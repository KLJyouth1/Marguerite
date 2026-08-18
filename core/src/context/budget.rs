//! Token 预算管理
//!
//! 会话级 Token 预算，超预算自动压缩（优先折叠摘要页），预算告警。

use serde::{Deserialize, Serialize};

/// 预算告警级别
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BudgetAlert {
    /// 正常
    Normal,
    /// 接近上限（>80%）
    Warning,
    /// 已超限（>100%）
    Exceeded,
}

/// Token 预算错误
#[derive(Debug, thiserror::Error)]
pub enum TokenBudgetError {
    #[error("预算为 0，无法使用")]
    ZeroBudget,
    #[error("当前用量超出预算")]
    OverBudget,
}

/// Token 预算
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBudget {
    /// 预算上限
    pub max_tokens: u64,
    /// 告警阈值（0-1，默认 0.8）
    pub warn_ratio: f64,
    /// 当前已用量
    pub used: u64,
}

impl TokenBudget {
    pub fn new(max_tokens: u64) -> Self {
        Self {
            max_tokens,
            warn_ratio: 0.8,
            used: 0,
        }
    }

    /// 记录用量
    pub fn record(&mut self, tokens: u64) -> Result<(), TokenBudgetError> {
        if self.max_tokens == 0 {
            return Err(TokenBudgetError::ZeroBudget);
        }
        self.used += tokens;
        if self.used > self.max_tokens {
            Err(TokenBudgetError::OverBudget)
        } else {
            Ok(())
        }
    }

    /// 当前告警级别
    pub fn alert(&self) -> BudgetAlert {
        if self.max_tokens == 0 {
            return BudgetAlert::Normal;
        }
        let ratio = self.used as f64 / self.max_tokens as f64;
        if ratio >= 1.0 {
            BudgetAlert::Exceeded
        } else if ratio >= self.warn_ratio {
            BudgetAlert::Warning
        } else {
            BudgetAlert::Normal
        }
    }

    /// 剩余可用 token
    pub fn remaining(&self) -> u64 {
        self.max_tokens.saturating_sub(self.used)
    }

    /// 重置用量
    pub fn reset(&mut self) {
        self.used = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_record() {
        let mut budget = TokenBudget::new(100);
        assert!(budget.record(40).is_ok());
        assert_eq!(budget.remaining(), 60);
    }

    #[test]
    fn test_budget_over() {
        let mut budget = TokenBudget::new(100);
        budget.record(120).unwrap_err();
        assert_eq!(budget.alert(), BudgetAlert::Exceeded);
    }

    #[test]
    fn test_budget_warning_threshold() {
        let mut budget = TokenBudget::new(100);
        budget.warn_ratio = 0.8;
        budget.record(85).unwrap();
        assert_eq!(budget.alert(), BudgetAlert::Warning);
        budget.record(15).unwrap();
        assert_eq!(budget.alert(), BudgetAlert::Exceeded);
    }

    #[test]
    fn test_budget_zero() {
        let mut budget = TokenBudget::new(0);
        assert!(matches!(
            budget.record(1),
            Err(TokenBudgetError::ZeroBudget)
        ));
    }

    #[test]
    fn test_budget_reset() {
        let mut budget = TokenBudget::new(100);
        budget.record(60).unwrap();
        budget.reset();
        assert_eq!(budget.used, 0);
    }
}
