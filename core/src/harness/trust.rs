//! Trust 锁 + 编辑预算 —— 自我改进的「合并前强制链」（路线图 §5.3 机制 1/3）。
//!
//! 文献里反复出现的结论：自评门失效（智能体给自己的改动打分，几乎总是放行）。
//! 本项目已有的外生门是 [`crate::governance::CompletionGate`] 与沙箱回放
//! （[`super::meta::evaluate_candidate`]）；本模块补的是**审计链**：
//!
//! ```text
//! 提案(ImprovementPlan) → 变更集(可 diff) → 回滚点 → 外生验证 → 才允许合并
//! ```
//!
//! 四环缺一不可，由 [`trust_gate`] 逐环裁决。这与 `ChangeTarget` 的窄枚举
//! 叠加成两条红线：
//!
//! 1. **不为 RSI 加 core 源变体**（类型层已封死），变更集里出现 `core/src`
//!    路径同样拒 —— 走变更集后门等同于改 core 源码；
//! 2. **编辑预算**（DGM 系）：每轮改动半径设上限（[`budget_gate`]），
//!    防止「智能体改智能体提示词」悄悄扩大。
//!
//! 接入点：[`super::meta::MetaAgent::run_cycle_with_trust`] —— 门插在
//! 「沙箱裁决通过之后、快照登记之前」，被拒的候选**不会**在台账里留下
//! 「已灰度」的快照（否则台账会谎报一个没有证据的 active 版本）。

use serde::{Deserialize, Serialize};

use super::meta::ImprovementPlan;

// ───────────────────────────────────────────────────────────────────────────
// Trust 锁（机制 1）
// ───────────────────────────────────────────────────────────────────────────

/// 变更集里的一条改动 —— 必须可 diff，不接受「我改过了」一句话。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangeRecord {
    /// 相对仓库根的路径（`/` 分隔；`\` 在比较时归一化）
    pub path: String,
    pub kind: ChangeKind,
    /// 可 diff 的证据：统一 diff 片段或前后摘要。**空白 = 没有留痕**，直接拒。
    pub diff: String,
}

/// 改动类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

/// 外生验证记录：证据必须来自**实际执行的命令及其退出码**，
/// 不接受「我认为没问题」式的自评（文献结论：自评门失效）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationRecord {
    /// 实际执行的验证命令（如 `cargo test -p marguerite-core --lib harness::trust`）
    pub command: String,
    /// 命令退出码：`0` = 通过；非 0 = 验证失败，提案被拒
    pub exit_code: i32,
    /// 输出 / 日志摘录（如 `test result: ok. N passed; 0 failed`）
    pub evidence: String,
}

/// 一份带 Trust 锁的合并提案：把第 3 层的 [`ImprovementPlan`] 落成可审计的变更集。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangeProposal {
    /// 对应的改进提案（必须与周期内生成的 plan 一致，防止张冠李戴）
    pub plan: ImprovementPlan,
    /// 变更集：**至少一条**、每条必须有 diff 留痕
    pub changes: Vec<ChangeRecord>,
    /// 回滚点（`checkpoint.rs` 的产物 id）。`None`/空白 = 没有 Trust 锁，拒。
    pub rollback: Option<String>,
    /// 外生验证记录。`None` = 未验证，拒。
    pub verification: Option<VerificationRecord>,
}

/// Trust 锁裁决
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "verdict", content = "detail")]
pub enum TrustVerdict {
    /// 四环齐全，允许合并
    Merge,
    /// 提案与周期内生成的 plan 不一致（变更集是给别的提案准备的）
    RejectPlanMismatch,
    /// 变更集为空 —— 无 diff 就无可审计
    RejectEmptyChangeset,
    /// 某条改动没有 diff 留痕
    RejectUndiffable { path: String },
    /// 变更集试图改 core 源码 —— 走后门也红线
    RejectCoreSource { path: String },
    /// 没有回滚点 —— 合并后不可回退 = 没有 Trust 锁
    RejectNoRollback,
    /// 未做外生验证
    RejectNoVerification,
    /// 外生验证实际失败了（不是没做，是做了没过）
    RejectVerificationFailed { command: String, exit_code: i32 },
}

impl TrustVerdict {
    pub fn is_merge(&self) -> bool {
        matches!(self, Self::Merge)
    }

    /// 稳定的英文码（本地化由前端做）
    pub fn code(&self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::RejectPlanMismatch => "reject_plan_mismatch",
            Self::RejectEmptyChangeset => "reject_empty_changeset",
            Self::RejectUndiffable { .. } => "reject_undiffable",
            Self::RejectCoreSource { .. } => "reject_core_source",
            Self::RejectNoRollback => "reject_no_rollback",
            Self::RejectNoVerification => "reject_no_verification",
            Self::RejectVerificationFailed { .. } => "reject_verification_failed",
        }
    }
}

/// Trust 锁裁决（§5.3 机制 1）：合并前的最后一道闸。
///
/// 判据顺序有意为：**身份 → 红线 → 留痕 → 可回退 → 外生验证**。
/// 身份错配最先查（变更集张冠李戴时，其余检查都在检查错误的对象）；
/// core 源码红线其次（无论证据多齐全，后门一律不开）。
pub fn trust_gate(plan: &ImprovementPlan, proposal: &ChangeProposal) -> TrustVerdict {
    // 1) 身份：变更集必须属于这份 plan
    if proposal.plan != *plan {
        return TrustVerdict::RejectPlanMismatch;
    }

    // 2) 红线 + 3) 留痕：逐条检查变更集
    if proposal.changes.is_empty() {
        return TrustVerdict::RejectEmptyChangeset;
    }
    for c in &proposal.changes {
        let p = normalize_path(&c.path);
        if p.starts_with("core/src") {
            return TrustVerdict::RejectCoreSource { path: c.path.clone() };
        }
        if c.diff.trim().is_empty() {
            return TrustVerdict::RejectUndiffable { path: c.path.clone() };
        }
    }

    // 4) 可回退：没有回滚点就不许合并
    match &proposal.rollback {
        Some(id) if !id.trim().is_empty() => {}
        _ => return TrustVerdict::RejectNoRollback,
    }

    // 5) 外生验证：做了、且通过
    match &proposal.verification {
        None => TrustVerdict::RejectNoVerification,
        Some(v) if v.command.trim().is_empty() => TrustVerdict::RejectNoVerification,
        Some(v) if v.exit_code != 0 => TrustVerdict::RejectVerificationFailed {
            command: v.command.clone(),
            exit_code: v.exit_code,
        },
        Some(_) => TrustVerdict::Merge,
    }
}

/// 路径归一化：`\` → `/`，去掉 `./` 前缀（比较用）
fn normalize_path(p: &str) -> String {
    p.replace('\\', "/").trim_start_matches("./").to_string()
}

// ───────────────────────────────────────────────────────────────────────────
// 编辑预算（机制 3，DGM 系）
// ───────────────────────────────────────────────────────────────────────────

/// 每轮自改进的**改动半径上限**。
///
/// 与 `ChangeTarget` 窄枚举叠加：枚举管「能改什么类」，预算管「一轮改多大」。
/// 防的是同一种失败：改进循环里某轮突然膨胀（改 40 个文件、塞 20 条提案），
/// 事前没人拦，事后审计才发现。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditBudget {
    /// 单轮允许触碰的最大**去重**文件数
    pub max_files: usize,
    /// 单轮允许的提案数
    pub max_proposals: usize,
}

impl Default for EditBudget {
    fn default() -> Self {
        // 缺省是「小步」：一轮最多 10 个文件、3 条提案。
        // 真需要大改时由策略显式放宽 —— 放宽本身可审计。
        Self {
            max_files: 10,
            max_proposals: 3,
        }
    }
}

/// 预算裁决
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "verdict", content = "detail")]
pub enum BudgetVerdict {
    /// 半径之内，放行
    Within,
    /// 触碰文件数超限（跨提案去重后计）
    RejectTooManyFiles { used: usize, limit: usize },
    /// 提案数超限
    RejectTooManyProposals { used: usize, limit: usize },
}

impl BudgetVerdict {
    pub fn is_within(&self) -> bool {
        matches!(self, Self::Within)
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Within => "within",
            Self::RejectTooManyFiles { .. } => "reject_too_many_files",
            Self::RejectTooManyProposals { .. } => "reject_too_many_proposals",
        }
    }
}

/// 编辑预算裁决（§5.3 机制 3）。
///
/// 文件数按**去重后**计：两条提案碰同一个文件只算 1 个 ——
/// 预算约束的是改动半径，不是记录条数。
pub fn budget_gate(proposals: &[ChangeProposal], budget: &EditBudget) -> BudgetVerdict {
    if proposals.len() > budget.max_proposals {
        return BudgetVerdict::RejectTooManyProposals {
            used: proposals.len(),
            limit: budget.max_proposals,
        };
    }
    let mut files = std::collections::HashSet::new();
    for p in proposals {
        for c in &p.changes {
            files.insert(normalize_path(&c.path));
        }
    }
    if files.len() > budget.max_files {
        return BudgetVerdict::RejectTooManyFiles {
            used: files.len(),
            limit: budget.max_files,
        };
    }
    BudgetVerdict::Within
}

// ───────────────────────────────────────────────────────────────────────────
// 测试
// ───────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::meta::{ChangeTarget, RootCause, RiskLevel};

    fn valid_plan() -> ImprovementPlan {
        ImprovementPlan {
            target: ChangeTarget::SystemPrompt,
            cause: RootCause::PlanLogicWeak,
            risk: RiskLevel::Low,
            mutation: "在系统提示词补入失败驱动的拆解与自检规则".to_string(),
            rationale: "信号集中在规划/产出侧".to_string(),
        }
    }

    /// 四环齐全的提案：可 diff 的变更集 + 回滚点 + exit 0 的外生验证
    fn valid_proposal(plan: &ImprovementPlan) -> ChangeProposal {
        ChangeProposal {
            plan: plan.clone(),
            changes: vec![ChangeRecord {
                path: "skills/deck_craft/intake-gate.md".to_string(),
                kind: ChangeKind::Modified,
                diff: "- 旧规则：直接产出\n+ 新规则：先弹问答窗确认大纲与素材源".to_string(),
            }],
            rollback: Some("ckpt-20261027-abc123".to_string()),
            verification: Some(VerificationRecord {
                command: "cargo test -p marguerite-core --lib harness::trust".to_string(),
                exit_code: 0,
                evidence: "test result: ok. 12 passed; 0 failed".to_string(),
            }),
        }
    }

    #[test]
    fn trust_gate_passes_full_chain() {
        let plan = valid_plan();
        assert_eq!(trust_gate(&plan, &valid_proposal(&plan)), TrustVerdict::Merge);
    }

    #[test]
    fn trust_gate_rejects_plan_mismatch() {
        let plan = valid_plan();
        let mut p = valid_proposal(&plan);
        p.plan.mutation = "改的是另一件事".to_string();
        assert_eq!(
            trust_gate(&plan, &p),
            TrustVerdict::RejectPlanMismatch,
            "变更集张冠李戴必须最先被拦"
        );
    }

    #[test]
    fn trust_gate_rejects_core_source_backdoor() {
        let plan = valid_plan();
        let mut p = valid_proposal(&plan);
        p.changes[0].path = "core/src/harness/meta.rs".to_string();
        p.changes[0].diff = "- fn a()\n+ fn b()".to_string();
        assert_eq!(
            trust_gate(&plan, &p),
            TrustVerdict::RejectCoreSource {
                path: "core/src/harness/meta.rs".to_string()
            },
            "ChangeTarget 没有 core 变体，变更集后门同样不许开"
        );
    }

    #[test]
    fn trust_gate_rejects_empty_changeset() {
        let plan = valid_plan();
        let mut p = valid_proposal(&plan);
        p.changes.clear();
        assert_eq!(trust_gate(&plan, &p), TrustVerdict::RejectEmptyChangeset);
    }

    #[test]
    fn trust_gate_rejects_undiffable_change() {
        let plan = valid_plan();
        let mut p = valid_proposal(&plan);
        p.changes[0].diff = "   ".to_string();
        assert_eq!(
            trust_gate(&plan, &p),
            TrustVerdict::RejectUndiffable {
                path: "skills/deck_craft/intake-gate.md".to_string()
            },
            "空白 diff = 没有留痕，不能只说「我改过了」"
        );
    }

    #[test]
    fn trust_gate_rejects_missing_rollback() {
        let plan = valid_plan();
        for rollback in [None, Some("  ".to_string())] {
            let mut p = valid_proposal(&plan);
            p.rollback = rollback.clone();
            assert_eq!(
                trust_gate(&plan, &p),
                TrustVerdict::RejectNoRollback,
                "不可回退的合并 = 没有 Trust 锁（rollback = {rollback:?}）"
            );
        }
    }

    #[test]
    fn trust_gate_rejects_missing_or_failed_verification() {
        let plan = valid_plan();

        let mut none = valid_proposal(&plan);
        none.verification = None;
        assert_eq!(trust_gate(&plan, &none), TrustVerdict::RejectNoVerification);

        let mut blank_cmd = valid_proposal(&plan);
        blank_cmd.verification = Some(VerificationRecord {
            command: " ".to_string(),
            exit_code: 0,
            evidence: String::new(),
        });
        assert_eq!(
            trust_gate(&plan, &blank_cmd),
            TrustVerdict::RejectNoVerification,
            "没有命令 = 没有验证，evidence 写得再满也不算"
        );

        let mut failed = valid_proposal(&plan);
        failed.verification = Some(VerificationRecord {
            command: "cargo test".to_string(),
            exit_code: 101,
            evidence: "error[E0425]".to_string(),
        });
        assert_eq!(
            trust_gate(&plan, &failed),
            TrustVerdict::RejectVerificationFailed {
                command: "cargo test".to_string(),
                exit_code: 101
            },
            "验证做了但没过 ≠ 通过"
        );
    }

    #[test]
    fn budget_gate_dedupes_files_across_proposals() {
        let plan = valid_plan();
        let record = || ChangeRecord {
            path: "skills/deck_craft/intake-gate.md".to_string(),
            kind: ChangeKind::Modified,
            diff: "- a\n+ b".to_string(),
        };
        let proposals = vec![
            ChangeProposal {
                plan: plan.clone(),
                changes: vec![record()],
                rollback: Some("ckpt-1".to_string()),
                verification: None,
            },
            ChangeProposal {
                plan: plan.clone(),
                changes: vec![record()],
                rollback: Some("ckpt-2".to_string()),
                verification: None,
            },
        ];
        // 两条提案碰同一个文件 → 去重后 1 个文件，within
        assert_eq!(
            budget_gate(&proposals, &EditBudget::default()),
            BudgetVerdict::Within,
            "预算约束的是改动半径，不是记录条数"
        );
    }

    #[test]
    fn budget_gate_rejects_too_many_files() {
        let plan = valid_plan();
        let changes: Vec<ChangeRecord> = (0..11)
            .map(|i| ChangeRecord {
                path: format!("skills/skill-{i}.md"),
                kind: ChangeKind::Added,
                diff: "+ new".to_string(),
            })
            .collect();
        let p = ChangeProposal {
            plan: plan.clone(),
            changes,
            rollback: Some("ckpt-1".to_string()),
            verification: None,
        };
        assert_eq!(
            budget_gate(std::slice::from_ref(&p), &EditBudget::default()),
            BudgetVerdict::RejectTooManyFiles {
                used: 11,
                limit: 10
            }
        );
    }

    #[test]
    fn budget_gate_rejects_too_many_proposals() {
        let plan = valid_plan();
        let proposals: Vec<ChangeProposal> = (0..4)
            .map(|_| ChangeProposal {
                plan: plan.clone(),
                changes: vec![ChangeRecord {
                    path: "skills/x.md".to_string(),
                    kind: ChangeKind::Modified,
                    diff: "- a\n+ b".to_string(),
                }],
                rollback: Some("ckpt".to_string()),
                verification: None,
            })
            .collect();
        assert_eq!(
            budget_gate(&proposals, &EditBudget::default()),
            BudgetVerdict::RejectTooManyProposals {
                used: 4,
                limit: 3
            }
        );
    }

    #[test]
    fn trust_verdict_codes_are_stable() {
        // 前端依赖这些码做本地化；改名 = 前端静默退化
        let plan = valid_plan();
        assert_eq!(trust_gate(&plan, &valid_proposal(&plan)).code(), "merge");
        assert_eq!(
            TrustVerdict::RejectCoreSource {
                path: String::new()
            }
            .code(),
            "reject_core_source"
        );
        assert_eq!(BudgetVerdict::Within.code(), "within");
    }
}
