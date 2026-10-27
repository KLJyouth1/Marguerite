//! Plan-mode 审批闭环（上游 plan-mode 二开落地，Track 4）。
//!
//! 对齐 DeepSeek Harness 的 plan-mode 交互闭环：
//! 拟定计划（Draft）→ exit_plan_mode 提交审批（PendingApproval）→
//! 用户批准（Approved，离开计划模式、开始执行）或驳回（Rejected，返回修订）。
//!
//! 纯逻辑状态机，离线可测；与 plan/mod.rs 的 PlanRun 状态机（queued→running）互补——
//! 本模块是「执行前的人工审批闸门」，PlanRun 是「审批通过后的执行推进」。
//!
//! # 接线点（读这段再改代码）
//!
//! 本状态机由 [`crate::plan::runner::run_autonomous`] 的**计划审批闸门**驱动，
//! 且仅在 `plan.require_approval = true` 时生效（默认 false：复杂任务拆解后直接执行，
//! 即「长续航自主执行」这一产品主张）。闸门走的是**通用裁决面**
//! （`guard::approval_registry` + 控制台轮询的 `/api/approvals`），因此控制台的审批
//! 卡片无需任何新 UI 即可裁决计划审批；未批准时 runner **不启动运行实例**（fail-closed）。
//!
//! ⚠️ 在此之前本状态机**零调用点**：模块文档写着「审批闭环」，而执行链路上根本没有
//! 审批。新增/调整计划执行路径时，务必确认这里仍被真实调用点驱动。

use anyhow::{anyhow, Result};

/// 审批状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanApprovalState {
    /// 拟定中（计划模式内）
    Draft,
    /// 已提交，等待用户审批
    PendingApproval,
    /// 已批准（离开计划模式，可执行）
    Approved,
    /// 已驳回（需修订）
    Rejected,
}

impl PlanApprovalState {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanApprovalState::Draft => "draft",
            PlanApprovalState::PendingApproval => "pending_approval",
            PlanApprovalState::Approved => "approved",
            PlanApprovalState::Rejected => "rejected",
        }
    }
}

/// 计划审批状态机
#[derive(Debug, Clone)]
pub struct PlanApproval {
    state: PlanApprovalState,
    feedback: Option<String>,
}

impl Default for PlanApproval {
    fn default() -> Self {
        Self::new()
    }
}

impl PlanApproval {
    pub fn new() -> Self {
        Self {
            state: PlanApprovalState::Draft,
            feedback: None,
        }
    }

    pub fn state(&self) -> PlanApprovalState {
        self.state
    }

    pub fn is_approved(&self) -> bool {
        self.state == PlanApprovalState::Approved
    }

    pub fn feedback(&self) -> Option<&str> {
        self.feedback.as_deref()
    }

    /// 提交审批（Draft → PendingApproval）；即 exit_plan_mode 的落点。
    pub fn submit(&mut self) -> Result<()> {
        if self.state != PlanApprovalState::Draft {
            return Err(anyhow!(
                "仅 draft 状态可提交审批，当前为 {}",
                self.state.as_str()
            ));
        }
        self.state = PlanApprovalState::PendingApproval;
        self.feedback = None;
        // M3-D1 产品事件：审批请求
        crate::events::product::emit(
            crate::events::product::APPROVAL_REQUESTED,
            "core",
            serde_json::json!({ "approval": "plan", "state": "pending_approval" }),
        );
        Ok(())
    }

    /// 批准（PendingApproval → Approved）；批准后离开计划模式。
    pub fn approve(&mut self) -> Result<()> {
        if self.state != PlanApprovalState::PendingApproval {
            return Err(anyhow!(
                "仅 pending_approval 状态可批准，当前为 {}",
                self.state.as_str()
            ));
        }
        self.state = PlanApprovalState::Approved;
        self.feedback = None;
        // M3-D1 产品事件：审批放行
        crate::events::product::emit(
            crate::events::product::APPROVAL_GRANTED,
            "core",
            serde_json::json!({ "approval": "plan", "state": "approved" }),
        );
        Ok(())
    }

    /// 驳回（PendingApproval → Rejected），记录反馈理由。
    pub fn reject(&mut self, feedback: &str) -> Result<()> {
        if self.state != PlanApprovalState::PendingApproval {
            return Err(anyhow!(
                "仅 pending_approval 状态可驳回，当前为 {}",
                self.state.as_str()
            ));
        }
        let f = feedback.trim();
        if f.is_empty() {
            return Err(anyhow!("驳回理由不可为空"));
        }
        self.state = PlanApprovalState::Rejected;
        self.feedback = Some(f.to_string());
        // M3-D1 产品事件：审批驳回
        crate::events::product::emit(
            crate::events::product::APPROVAL_DENIED,
            "core",
            serde_json::json!({ "approval": "plan", "state": "rejected", "feedback": f }),
        );
        Ok(())
    }

    /// 修订（Rejected → Draft），清空反馈、返回计划模式继续拟定。
    pub fn revise(&mut self) -> Result<()> {
        if self.state != PlanApprovalState::Rejected {
            return Err(anyhow!(
                "仅 rejected 状态可修订，当前为 {}",
                self.state.as_str()
            ));
        }
        self.state = PlanApprovalState::Draft;
        self.feedback = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_submit_approve() {
        let mut a = PlanApproval::new();
        assert_eq!(a.state(), PlanApprovalState::Draft);
        a.submit().unwrap();
        assert_eq!(a.state(), PlanApprovalState::PendingApproval);
        a.approve().unwrap();
        assert_eq!(a.state(), PlanApprovalState::Approved);
        assert!(a.is_approved());
    }

    #[test]
    fn reject_then_revise_loop() {
        let mut a = PlanApproval::new();
        a.submit().unwrap();
        a.reject("范围太大").unwrap();
        assert_eq!(a.state(), PlanApprovalState::Rejected);
        assert_eq!(a.feedback(), Some("范围太大"));
        a.revise().unwrap();
        assert_eq!(a.state(), PlanApprovalState::Draft);
        assert_eq!(a.feedback(), None);
        // 重新提交并批准
        a.submit().unwrap();
        a.approve().unwrap();
        assert!(a.is_approved());
    }

    #[test]
    fn submit_only_from_draft() {
        let mut a = PlanApproval::new();
        a.submit().unwrap();
        assert!(a.submit().is_err(), "已提交不能重复提交");
    }

    #[test]
    fn approve_only_from_pending() {
        let mut a = PlanApproval::new();
        assert!(a.approve().is_err(), "draft 不能直接批准");
    }

    #[test]
    fn reject_requires_feedback() {
        let mut a = PlanApproval::new();
        a.submit().unwrap();
        assert!(a.reject("  ").is_err());
        assert_eq!(a.state(), PlanApprovalState::PendingApproval);
    }

    #[test]
    fn revise_only_from_rejected() {
        let mut a = PlanApproval::new();
        assert!(a.revise().is_err(), "draft 不能修订");
    }
}
