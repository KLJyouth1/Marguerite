//! 治理内核：计划门（Plan Gate）/ 宪法（Constitution）/ 决策协议（Decision Protocol）
//!
//! 这是对标 标准 AI 助手 的「Exclusive」差异化能力：在执行任何自动编排计划前，
//! 先经宪法裁决，所有重大决策写入防篡改（sha256 链式）决策台账。
//!
//! 设计原则：纯逻辑、零外部依赖、可离线裁决；宪法与决策台账均可独立单元测试。

pub mod completion_gate;
pub mod constitution;
pub mod decisions;
pub mod review;
pub mod verifier;

pub use completion_gate::{
    BlockReason, CompletionClaim, CompletionGate, CompletionVerdict, Criterion, ShellRunner,
    VerificationCtx, VerificationOutcome, VerificationStatus, VerificationSummary,
};
pub use constitution::{Constitution, Rule, RuleKind, RuleSeverity, Violation};
pub use decisions::{DecisionLog, DecisionOutcome, DecisionRecord};
pub use review::{AutoReview, ReviewIssue, ReviewSeverity, ReviewVerdict};
pub use verifier::{OsShellRunner, Verdict, Verifier, VerifyRun, VerifyRunner, VerifySpec};

use std::fmt;

/// 一次意图执行的动作
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ActionIntent {
    /// 意图使用的工具名（须与 ToolRegistry 注册名一致，如 "shell_exec"）
    pub tool: String,
    /// 具体操作描述（用于审计与展示）
    pub operation: String,
    /// 是否破坏性（删除 / 覆盖 / 网络外发等）
    pub destructive: bool,
    /// 是否已在用户侧显式获批
    pub approved: bool,
}

impl ActionIntent {
    /// 便捷构造（默认非破坏性、未获批）
    pub fn new(tool: &str, operation: &str) -> Self {
        Self {
            tool: tool.to_string(),
            operation: operation.to_string(),
            destructive: false,
            approved: false,
        }
    }

    /// 标记为破坏性
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    /// 标记为已获批
    pub fn approved(mut self) -> Self {
        self.approved = true;
        self
    }
}

/// 待裁决的计划
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ProposedPlan {
    pub id: String,
    pub summary: String,
    /// 发起者："orchestrator" | "user" | ...
    pub actor: String,
    pub actions: Vec<ActionIntent>,
}

impl ProposedPlan {
    pub fn new(id: &str, summary: &str, actor: &str) -> Self {
        Self {
            id: id.to_string(),
            summary: summary.to_string(),
            actor: actor.to_string(),
            actions: Vec::new(),
        }
    }

    /// 链式追加一个动作
    pub fn with_action(mut self, a: ActionIntent) -> Self {
        self.actions.push(a);
        self
    }
}

/// 裁决结果
#[derive(Debug, Clone)]
pub struct GateDecision {
    pub approved: bool,
    pub violations: Vec<Violation>,
    /// 是否存在需人工放行的关键级违规
    pub requires_human_gate: bool,
}

impl GateDecision {
    /// 全绿裁决（无违规）
    pub fn approved() -> Self {
        Self {
            approved: true,
            violations: Vec::new(),
            requires_human_gate: false,
        }
    }
}

impl fmt::Display for GateDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.approved {
            write!(f, "APPROVED")
        } else {
            write!(f, "BLOCKED ({} violation(s))", self.violations.len())
        }
    }
}

/// 计划门：把宪法 + 已注册工具集合组合起来裁决计划
pub struct PlanGate {
    constitution: Constitution,
    registered_tools: Vec<String>,
}

impl PlanGate {
    pub fn new(constitution: Constitution, registered_tools: Vec<String>) -> Self {
        Self {
            constitution,
            registered_tools,
        }
    }

    /// 用默认宪法 + 已注册工具构造
    pub fn default_with_tools(registered_tools: Vec<String>) -> Self {
        Self::new(Constitution::default_v1(), registered_tools)
    }

    /// 裁决计划：返回是否批准 + 违规清单
    pub fn evaluate(&self, plan: &ProposedPlan) -> GateDecision {
        let violations = self.constitution.evaluate(plan, &self.registered_tools);
        let requires_human_gate = violations
            .iter()
            .any(|v| v.severity == RuleSeverity::Critical);
        let approved = !requires_human_gate
            && violations
                .iter()
                .all(|v| v.severity != RuleSeverity::Required);
        GateDecision {
            approved,
            violations,
            requires_human_gate,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_with(actions: Vec<ActionIntent>, actor: &str) -> ProposedPlan {
        let mut p = ProposedPlan::new("p1", "test plan", actor);
        for a in actions {
            p = p.with_action(a);
        }
        p
    }

    #[test]
    fn default_constitution_has_four_rules() {
        let c = Constitution::default_v1();
        assert_eq!(c.rules.len(), 4);
    }

    #[test]
    fn unapproved_destructive_is_blocked() {
        let gate = PlanGate::default_with_tools(vec!["shell_exec".into()]);
        let p = plan_with(
            vec![ActionIntent::new("shell_exec", "rm -rf /tmp/x").destructive()],
            "orchestrator",
        );
        let d = gate.evaluate(&p);
        assert!(!d.approved);
        assert!(d.requires_human_gate);
        assert!(d.violations.iter().any(|v| v.rule_id == "C1"));
    }

    #[test]
    fn approved_destructive_by_user_passes_c1() {
        let gate = PlanGate::default_with_tools(vec!["shell_exec".into()]);
        let p = plan_with(
            vec![ActionIntent::new("shell_exec", "rm -rf /tmp/x")
                .destructive()
                .approved()],
            "user",
        );
        let d = gate.evaluate(&p);
        assert!(d.approved, "user-approved destructive should pass C1");
        assert!(!d.requires_human_gate);
    }

    #[test]
    fn unknown_tool_is_required_violation_blocks() {
        let gate = PlanGate::default_with_tools(vec!["read_file".into()]);
        let p = plan_with(
            vec![ActionIntent::new("unknown_tool", "do thing")],
            "orchestrator",
        );
        let d = gate.evaluate(&p);
        assert!(!d.approved);
        assert!(d.violations.iter().any(|v| v.rule_id == "C3"));
    }

    #[test]
    fn advisory_only_does_not_block() {
        let mut actions = Vec::new();
        for i in 0..40 {
            actions.push(ActionIntent::new("read_file", &format!("op {i}")));
        }
        let gate = PlanGate::default_with_tools(vec!["read_file".into()]);
        let p = plan_with(actions, "orchestrator");
        let d = gate.evaluate(&p);
        assert!(d.approved, "advisory MaxActions should not block");
        assert!(d.violations.iter().any(|v| v.rule_id == "C4"));
    }

    #[test]
    fn secret_read_by_non_user_blocked() {
        let gate = PlanGate::default_with_tools(vec!["read_env".into()]);
        let p = plan_with(
            vec![ActionIntent::new("read_env", "read API key")],
            "orchestrator",
        );
        let d = gate.evaluate(&p);
        assert!(!d.approved);
        assert!(d.violations.iter().any(|v| v.rule_id == "C2"));
    }
}
