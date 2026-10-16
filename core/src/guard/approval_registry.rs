//! 审批注册表（M2-C1 双端裁决桥）
//!
//! 单一挂起审批由 [`ApprovalGate`] 持有，但裁决入口在多个宿主内：
//! - WebUI：经三条独立裁决链路（`/api/approvals`、`exec.approval.*` RPC、`resolve_approval` 命令）
//! - GUI：新增 Tauri 命令 `approve_approval` / `deny_approval`
//! - TUI / 未来宿主：内联审批框
//!
//! 本注册表提供 `id → Arc<ApprovalGate>` 的全局索引，使任意宿主都能在拿到
//! approval_id 后定位到持有所属挂起审批的 gate 并完成裁决，避免各宿主各自
//! 复制门控状态。index 只保留**挂起中**的审批，裁决成功或超时消费后即移除，
//! 防止陈旧 ID 被重复裁决。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use super::gateway::{ApprovalGate, ApprovalOutcome, PendingApproval};

/// 裁决结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionResult {
    /// 已成功裁定为 Approved/Denied/Timeout（含超时 fail-closed 语义）
    Decided(ApprovalOutcome),
    /// 未找到挂起审批（可能已消费或 ID 不存在）
    NotFound,
}

/// 全局审批注册表
pub struct ApprovalRegistry {
    slots: Mutex<HashMap<String, Arc<ApprovalGate>>>,
}

impl ApprovalRegistry {
    pub fn new() -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
        }
    }

    /// 注册一个持有挂起审批的 gate（executor 挂起后调用）
    pub fn register(&self, gate: Arc<ApprovalGate>, pending: &PendingApproval) {
        self.slots
            .lock()
            .unwrap()
            .insert(pending.id.clone(), gate);
    }

    /// 移除已消费的审批
    pub fn unregister(&self, id: &str) {
        self.slots.lock().unwrap().remove(id);
    }

    /// 定位某个挂起审批所属的 gate
    pub fn gate_for(&self, id: &str) -> Option<Arc<ApprovalGate>> {
        self.slots.lock().unwrap().get(id).cloned()
    }

    /// 当前所有挂起审批 ID（供宿主拉取清单 / badge 计数）
    pub fn pending_ids(&self) -> Vec<String> {
        self.slots.lock().unwrap().keys().cloned().collect()
    }

    /// 全部**仍挂起且未超时**的审批本体（供 WebUI `/api/approvals` 快照消费）。
    ///
    /// 与 [`ApprovalRegistry::pending_ids`] 的区别：这里带出工具名/参数/风险，因为控制台
    /// 要渲染裁决卡片；且用 `live_pending()` 过滤已超时项 —— 超时即 fail-closed 拒绝，
    /// 不该继续显示成「待你裁决」。顺序按挂起时间（最早在前），同一快照的前后顺序稳定。
    pub fn pending_snapshots(&self) -> Vec<PendingApproval> {
        let slots = self.slots.lock().unwrap();
        let mut out: Vec<PendingApproval> =
            slots.values().filter_map(|gate| gate.live_pending()).collect();
        out.sort_by_key(|p| p.created_at);
        out
    }

    /// 挂起审批数量
    pub fn len(&self) -> usize {
        self.slots.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 外部裁决：批准（true）或拒绝（false）。
    ///
    /// 走 gate 官方裁决通道，超时/ID 不匹配等 fail-closed 语义由 gate 保证；
    /// 消费成功后自动从注册表移除，防止重复裁决。
    pub fn decide(&self, id: &str, approve: bool) -> DecisionResult {
        let Some(gate) = self.gate_for(id) else {
            return DecisionResult::NotFound;
        };
        let ok = if approve {
            gate.approve(id)
        } else {
            gate.deny(id)
        };
        // 无论 gate 是否「消费成功」，只要它已给出终态结论就注销索引。
        // 历史缺陷：`!ok`（多为已超时）分支直接返回、不注销 —— 超时项于是永久
        // 滞留在 `slots` 里持续泄漏（`pending_snapshots` 靠 `live_pending` 过滤才
        // 没把它显示成「待裁决」，但注册表本身在涨）。注销是幂等的，对
        // 「ID 本就不存在」也无害。
        self.unregister(id);
        if ok {
            DecisionResult::Decided(if approve {
                ApprovalOutcome::Approved
            } else {
                ApprovalOutcome::Denied
            })
        } else {
            // 已超时/已消费：以 gate 实际结论为准（fail-closed）
            DecisionResult::Decided(gate.outcome(id))
        }
    }

    /// 显式 outcome 裁决（供 TUI 内联审批等通道复用 gate::resolve 超时语义）
    pub fn resolve_with(&self, id: &str, outcome: ApprovalOutcome) -> DecisionResult {
        let Some(gate) = self.gate_for(id) else {
            return DecisionResult::NotFound;
        };
        // `gate.resolve` 自带超时/重复裁决的 fail-closed 语义；无论其返回值如何，
        // gate 都已处于终态，注销索引即可（注销幂等，对「ID 本就不存在」也无害）。
        gate.resolve(id, outcome);
        self.unregister(id);
        DecisionResult::Decided(gate.outcome(id))
    }
}

impl Default for ApprovalRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// 进程级单例（供 executor / GUI / WebUI RPC 层共用同一索引）
static REGISTRY: OnceLock<ApprovalRegistry> = OnceLock::new();

pub fn global_registry() -> &'static ApprovalRegistry {
    REGISTRY.get_or_init(ApprovalRegistry::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn register_dummy(registry: &ApprovalRegistry) -> (Arc<ApprovalGate>, String) {
        let gate = Arc::new(ApprovalGate::new(Duration::from_secs(60)));
        let pending = gate
            .request("fs_delete", "{\"path\":\"x\"}".into(), crate::guard::gateway::RiskLevel::High)
            .expect("首次挂起应成功");
        let id = pending.id.clone();
        registry.register(gate.clone(), &pending);
        (gate, id)
    }

    #[test]
    fn register_and_pending_listing() {
        let reg = ApprovalRegistry::new();
        let (_, id) = register_dummy(&reg);
        assert_eq!(reg.len(), 1);
        assert!(reg.pending_ids().contains(&id));
    }

    #[test]
    fn decide_approve_consumes() {
        let reg = ApprovalRegistry::new();
        let (gate, id) = register_dummy(&reg);
        let r = reg.decide(&id, true);
        assert_eq!(r, DecisionResult::Decided(ApprovalOutcome::Approved));
        assert_eq!(gate.outcome(&id), ApprovalOutcome::Approved);
        assert!(reg.is_empty(), "消费后应从注册表移除");
        // 重复裁决 → NotFound
        assert_eq!(reg.decide(&id, true), DecisionResult::NotFound);
    }

    #[test]
    fn decide_deny() {
        let reg = ApprovalRegistry::new();
        let (gate, id) = register_dummy(&reg);
        assert_eq!(reg.decide(&id, false), DecisionResult::Decided(ApprovalOutcome::Denied));
        assert_eq!(gate.outcome(&id), ApprovalOutcome::Denied);
    }

    #[test]
    fn gate_for_unknown_id() {
        let reg = ApprovalRegistry::new();
        assert!(reg.gate_for("missing").is_none());
        assert_eq!(reg.decide("missing", true), DecisionResult::NotFound);
    }

    #[test]
    fn resolve_with_timeout_semantics() {
        let reg = ApprovalRegistry::new();
        // 超时 gate：裁决时门控自身将超时强制为 Timeout（fail-closed）
        let gate = Arc::new(ApprovalGate::new(Duration::from_millis(1)));
        let pending = gate
            .request("exec", "{}".into(), crate::guard::gateway::RiskLevel::High)
            .unwrap();
        let id = pending.id.clone();
        reg.register(gate.clone(), &pending);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(
            reg.decide(&id, true),
            DecisionResult::Decided(ApprovalOutcome::Timeout)
        );
        // 回归守卫：超时裁决后必须注销索引。历史缺陷：`!ok` 分支直接返回、
        // 不注销，超时项永久滞留在 slots 里持续泄漏。
        assert!(reg.is_empty(), "超时裁决后应从注册表移除（不得泄漏）");
    }

    /// 回归守卫：`resolve_with` 走到超时/非消费路径时同样要注销索引。
    #[test]
    fn resolve_with_unregisters_on_timeout() {
        let reg = ApprovalRegistry::new();
        let gate = Arc::new(ApprovalGate::new(Duration::from_millis(1)));
        let pending = gate
            .request("exec", "{}".into(), crate::guard::gateway::RiskLevel::High)
            .unwrap();
        let id = pending.id.clone();
        reg.register(gate.clone(), &pending);
        assert_eq!(reg.len(), 1);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(
            reg.resolve_with(&id, ApprovalOutcome::Approved),
            DecisionResult::Decided(ApprovalOutcome::Timeout)
        );
        assert!(reg.is_empty(), "超时裁决后应从注册表移除（不得泄漏）");
    }

    #[test]
    fn registry_is_process_singleton() {
        let a = global_registry();
        let b = global_registry();
        assert!(std::ptr::eq(a, b));
    }
}
