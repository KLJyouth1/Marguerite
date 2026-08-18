//! 审批门控（v2 工作台）
//!
//! 危险操作（写文件/执行/出网/桌面控制）的统一审批闸口：
//! - 同一时刻仅一个挂起审批
//! - 超时未响应按 DENY（fail-closed）
//! - 非交互路径（CLI 脚本/无确认通道）默认拒绝

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 风险等级
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    /// 低风险（只读/无害）
    Low,
    /// 中风险（写操作）
    Medium,
    /// 高风险（破坏性/出网）
    High,
}

impl Default for RiskLevel {
    fn default() -> Self {
        Self::Low
    }
}

/// 挂起中的审批
#[derive(Debug, Clone)]
pub struct PendingApproval {
    /// 审批 ID
    pub id: String,
    /// 目标工具名
    pub tool: String,
    /// 参数（JSON 字符串，简化传递）
    pub args: String,
    /// 风险等级
    pub risk: RiskLevel,
    /// 挂起时间
    pub created_at: Instant,
}

/// 审批结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    /// 已批准
    Approved,
    /// 已拒绝
    Denied,
    /// 仍在等待
    Pending,
    /// 超时（按拒绝处理）
    Timeout,
}

/// 审批门控
///
/// 管理单一挂起审批，提供批准/拒绝/取消/超时判定。
/// `interactive` 标记控制非交互路径的行为：非交互时任何请求直接按拒绝处理。
pub struct ApprovalGate {
    pending: Mutex<Option<PendingApproval>>,
    /// 最近一次消费结果（id → outcome），供等待方查询（D2.4 wait_approval）
    result: Mutex<Option<(String, ApprovalOutcome)>>,
    timeout: Duration,
    interactive: AtomicBool,
}

impl ApprovalGate {
    /// 创建审批门控
    pub fn new(timeout: Duration) -> Self {
        Self {
            pending: Mutex::new(None),
            result: Mutex::new(None),
            timeout,
            interactive: AtomicBool::new(true),
        }
    }

    /// 记录消费结果
    fn record(&self, id: &str, o: ApprovalOutcome) {
        *self.result.lock().unwrap() = Some((id.to_string(), o));
    }

    /// 设置是否处于交互模式（非交互路径 fail-closed）
    pub fn set_interactive(&self, interactive: bool) {
        self.interactive.store(interactive, Ordering::SeqCst);
    }

    /// 是否处于交互模式
    pub fn is_interactive(&self) -> bool {
        self.interactive.load(Ordering::SeqCst)
    }

    /// 挂起一个审批请求
    ///
    /// 若已有挂起审批则返回 `None`（一次仅一个）。
    pub fn request(&self, tool: &str, args: String, risk: RiskLevel) -> Option<PendingApproval> {
        let mut guard = self.pending.lock().unwrap();
        if guard.is_some() {
            return None;
        }
        let approval = PendingApproval {
            id: format!("ap_{}", uuid::Uuid::new_v4()),
            tool: tool.to_string(),
            args,
            risk,
            created_at: Instant::now(),
        };
        *guard = Some(approval.clone());
        Some(approval)
    }

    /// 当前挂起的审批
    pub fn pending(&self) -> Option<PendingApproval> {
        self.pending.lock().unwrap().clone()
    }

    /// 批准指定审批（仅当 ID 匹配且未超时）
    pub fn approve(&self, id: &str) -> bool {
        let mut guard = self.pending.lock().unwrap();
        if let Some(p) = guard.as_ref() {
            if p.id == id && p.created_at.elapsed() <= self.timeout {
                *guard = None;
                self.record(id, ApprovalOutcome::Approved);
                return true;
            }
        }
        false
    }

    /// 拒绝指定审批（仅当 ID 匹配）
    pub fn deny(&self, id: &str) -> bool {
        let mut guard = self.pending.lock().unwrap();
        if let Some(p) = guard.as_ref() {
            if p.id == id {
                *guard = None;
                self.record(id, ApprovalOutcome::Denied);
                return true;
            }
        }
        false
    }

    /// 直接裁定指定审批（供 TUI 内联审批框等外部裁决通道使用）
    ///
    /// 与 [`ApprovalGate::approve`] / [`ApprovalGate::deny`] 不同，本方法允许显式传入
    /// 任意 [`ApprovalOutcome`]。当 ID 匹配且挂起审批未超时时生效；已超时的挂起审批
    /// 一律按 [`ApprovalOutcome::Timeout`] 处理（忽略外部传入的非超时结果），保持 fail-closed。
    /// 返回是否成功裁定（ID 不匹配或不存在挂起审批时为 `false`）。
    pub fn resolve(&self, id: &str, outcome: ApprovalOutcome) -> bool {
        let mut guard = self.pending.lock().unwrap();
        match guard.as_ref() {
            None => false,
            Some(p) if p.id != id => false,
            Some(p) => {
                let effective = if p.created_at.elapsed() > self.timeout {
                    ApprovalOutcome::Timeout
                } else {
                    outcome
                };
                *guard = None;
                self.record(id, effective);
                true
            }
        }
    }

    /// 作废当前挂起审批（模式切换时调用）
    pub fn cancel_pending(&self) {
        *self.pending.lock().unwrap() = None;
    }

    /// 查询审批结果（消费式：Approved/Denied/Timeout 后清除 pending；结果可复查）
    ///
    /// 非交互模式：任何待决请求直接按 `Denied` 处理（fail-closed）。
    pub fn outcome(&self, id: &str) -> ApprovalOutcome {
        // 已记录的结果优先（等待方可跨轮询复查）
        if let Some((rid, res)) = self.result.lock().unwrap().as_ref() {
            if rid == id {
                return *res;
            }
        }
        let mut guard = self.pending.lock().unwrap();
        match guard.as_ref() {
            None => {
                self.record(id, ApprovalOutcome::Denied);
                ApprovalOutcome::Denied
            }
            Some(p) if p.id != id => ApprovalOutcome::Denied,
            Some(p) => {
                if !self.is_interactive() {
                    *guard = None;
                    self.record(id, ApprovalOutcome::Denied);
                    return ApprovalOutcome::Denied;
                }
                if p.created_at.elapsed() > self.timeout {
                    *guard = None;
                    self.record(id, ApprovalOutcome::Timeout);
                    return ApprovalOutcome::Timeout;
                }
                ApprovalOutcome::Pending
            }
        }
    }
}

impl Default for ApprovalGate {
    fn default() -> Self {
        Self::new(Duration::from_secs(60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_and_approve() {
        let gate = ApprovalGate::new(Duration::from_secs(60));
        let p = gate
            .request("edit_file", "{}".into(), RiskLevel::Medium)
            .unwrap();
        assert_eq!(gate.outcome(&p.id), ApprovalOutcome::Pending);
        assert!(gate.approve(&p.id));
        assert_eq!(gate.outcome(&p.id), ApprovalOutcome::Approved); // 结果可复查
        assert!(gate.pending().is_none());
    }

    #[test]
    fn test_request_and_deny() {
        let gate = ApprovalGate::new(Duration::from_secs(60));
        let p = gate.request("exec", "[]".into(), RiskLevel::High).unwrap();
        assert!(gate.deny(&p.id));
        assert_eq!(gate.outcome(&p.id), ApprovalOutcome::Denied);
    }

    #[test]
    fn test_wrong_id_denied() {
        let gate = ApprovalGate::new(Duration::from_secs(60));
        let _p = gate
            .request("edit_file", "{}".into(), RiskLevel::Medium)
            .unwrap();
        assert_eq!(gate.outcome("ap_wrong"), ApprovalOutcome::Denied);
        assert!(!gate.approve("ap_wrong"));
        assert!(gate.pending().is_some()); // 原审批未受影响
    }

    #[test]
    fn test_single_pending_only() {
        let gate = ApprovalGate::new(Duration::from_secs(60));
        gate.request("edit_file", "{}".into(), RiskLevel::Medium)
            .unwrap();
        assert!(gate.request("exec", "[]".into(), RiskLevel::High).is_none());
    }

    #[test]
    fn test_cancel_pending() {
        let gate = ApprovalGate::new(Duration::from_secs(60));
        gate.request("edit_file", "{}".into(), RiskLevel::Medium)
            .unwrap();
        gate.cancel_pending();
        assert!(gate.pending().is_none());
        let p = gate
            .request("edit_file", "{}".into(), RiskLevel::Medium)
            .unwrap();
        assert_eq!(gate.outcome(&p.id), ApprovalOutcome::Pending);
    }

    #[test]
    fn test_timeout_denied() {
        let gate = ApprovalGate::new(Duration::from_millis(10));
        let p = gate
            .request("edit_file", "{}".into(), RiskLevel::Medium)
            .unwrap();
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(gate.outcome(&p.id), ApprovalOutcome::Timeout);
        assert!(gate.pending().is_none());
    }

    #[test]
    fn test_non_interactive_fail_closed() {
        let gate = ApprovalGate::new(Duration::from_secs(60));
        gate.set_interactive(false);
        let p = gate
            .request("edit_file", "{}".into(), RiskLevel::Medium)
            .unwrap();
        assert_eq!(gate.outcome(&p.id), ApprovalOutcome::Denied);
        assert!(gate.pending().is_none());
    }

    #[test]
    fn test_resolve_explicit_denied() {
        let gate = ApprovalGate::new(Duration::from_secs(60));
        let p = gate
            .request("edit_file", "{}".into(), RiskLevel::Medium)
            .unwrap();
        assert!(gate.resolve(&p.id, ApprovalOutcome::Denied));
        assert_eq!(gate.outcome(&p.id), ApprovalOutcome::Denied);
        assert!(gate.pending().is_none());
    }

    #[test]
    fn test_resolve_explicit_approved() {
        let gate = ApprovalGate::new(Duration::from_secs(60));
        let p = gate.request("exec", "[]".into(), RiskLevel::High).unwrap();
        assert!(gate.resolve(&p.id, ApprovalOutcome::Approved));
        assert_eq!(gate.outcome(&p.id), ApprovalOutcome::Approved);
    }

    #[test]
    fn test_resolve_unknown_id_returns_false() {
        let gate = ApprovalGate::new(Duration::from_secs(60));
        let _p = gate
            .request("edit_file", "{}".into(), RiskLevel::Medium)
            .unwrap();
        // 未知 ID 不应裁定，且不破坏原挂起审批
        assert!(!gate.resolve("ap_unknown", ApprovalOutcome::Approved));
        assert!(gate.pending().is_some());
    }

    #[test]
    fn test_resolve_timeout_overrides_outcome() {
        let gate = ApprovalGate::new(Duration::from_millis(10));
        let p = gate
            .request("edit_file", "{}".into(), RiskLevel::Medium)
            .unwrap();
        std::thread::sleep(Duration::from_millis(30));
        // 已超时：即使显式传入 Approved，也按 Timeout 处理
        assert!(gate.resolve(&p.id, ApprovalOutcome::Approved));
        assert_eq!(gate.outcome(&p.id), ApprovalOutcome::Timeout);
    }
}
