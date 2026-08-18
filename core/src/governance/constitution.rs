//! 宪法（Constitution）：不可变规则集，用于对 ProposedPlan 做离线裁决。
//!
//! 规则分三级：
//! - `Critical`：一旦违反即阻断计划，且要求人工放行（requires_human_gate）。
//! - `Required`：违反即阻断计划（非人工门，属硬约束）。
//! - `Advisory`：仅记录，不阻断（提示性）。

use serde::{Deserialize, Serialize};

use crate::governance::ProposedPlan;

/// 规则严重级别
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleSeverity {
    Advisory,
    Required,
    Critical,
}

impl RuleSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            RuleSeverity::Advisory => "advisory",
            RuleSeverity::Required => "required",
            RuleSeverity::Critical => "critical",
        }
    }
}

/// 规则类型（决定裁决谓语）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleKind {
    /// 任何破坏性动作必须显式获批
    NoUnapprovedDestructive,
    /// 密钥读取仅限用户侧发起
    SecretReadByNonUser,
    /// 所有工具须已在注册表登记
    ToolMustBeRegistered,
    /// 单次计划动作数上限
    MaxActions(usize),
}

/// 单条宪法规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub description: String,
    pub severity: RuleSeverity,
    pub kind: RuleKind,
}

/// 违规记录
#[derive(Debug, Clone)]
pub struct Violation {
    pub rule_id: String,
    pub severity: RuleSeverity,
    pub detail: String,
}

/// 宪法
#[derive(Debug, Clone)]
pub struct Constitution {
    pub version: String,
    pub rules: Vec<Rule>,
}

impl Constitution {
    /// 默认宪法 v1：覆盖破坏性操作、密钥外发、工具登记、计划规模四条基线
    pub fn default_v1() -> Self {
        Self {
            version: "1.0.0".into(),
            rules: vec![
                Rule {
                    id: "C1".into(),
                    description: "破坏性操作必须显式获批".into(),
                    severity: RuleSeverity::Critical,
                    kind: RuleKind::NoUnapprovedDestructive,
                },
                Rule {
                    id: "C2".into(),
                    description: "密钥读取仅限用户侧发起".into(),
                    severity: RuleSeverity::Critical,
                    kind: RuleKind::SecretReadByNonUser,
                },
                Rule {
                    id: "C3".into(),
                    description: "所有工具须已在注册表登记".into(),
                    severity: RuleSeverity::Required,
                    kind: RuleKind::ToolMustBeRegistered,
                },
                Rule {
                    id: "C4".into(),
                    description: "单次计划动作数不超过 32".into(),
                    severity: RuleSeverity::Advisory,
                    kind: RuleKind::MaxActions(32),
                },
            ],
        }
    }

    /// 裁决计划，返回违规清单（空 = 无违规）
    pub fn evaluate(&self, plan: &ProposedPlan, registered: &[String]) -> Vec<Violation> {
        let mut v = Vec::new();
        for rule in &self.rules {
            match rule.kind {
                RuleKind::NoUnapprovedDestructive => {
                    for a in &plan.actions {
                        if a.destructive && !a.approved {
                            v.push(Violation {
                                rule_id: rule.id.clone(),
                                severity: rule.severity,
                                detail: format!(
                                    "destructive action '{}' not approved",
                                    a.operation
                                ),
                            });
                        }
                    }
                }
                RuleKind::SecretReadByNonUser => {
                    for a in &plan.actions {
                        if a.tool == "read_env" && plan.actor != "user" {
                            v.push(Violation {
                                rule_id: rule.id.clone(),
                                severity: rule.severity,
                                detail: format!("secret read by actor '{}'", plan.actor),
                            });
                        }
                    }
                }
                RuleKind::ToolMustBeRegistered => {
                    for a in &plan.actions {
                        if !registered.iter().any(|t| t == &a.tool) {
                            v.push(Violation {
                                rule_id: rule.id.clone(),
                                severity: rule.severity,
                                detail: format!("tool '{}' not registered", a.tool),
                            });
                        }
                    }
                }
                RuleKind::MaxActions(n) => {
                    if plan.actions.len() > n {
                        v.push(Violation {
                            rule_id: rule.id.clone(),
                            severity: rule.severity,
                            detail: format!(
                                "plan has {} actions > limit {}",
                                plan.actions.len(),
                                n
                            ),
                        });
                    }
                }
            }
        }
        v
    }
}
