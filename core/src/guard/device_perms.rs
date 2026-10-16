//! WP8 设备权限模型 + 跨引擎审批收口（纯数据 / 纯内存，无网络）。
//!
//! 两件事：
//! 1. [`DevicePermissionPolicy`]：按 `entity_id` 维护每设备授权档（L0 只读 / L1 自动 /
//!    L2 需审批 / L3 人在回路）。`authorize` 是 fail-closed 的纯函数判定 —— 策略缺失、
//!    L0 写越权一律拒绝；L2/L3 抬审批；L3 强制人在回路（绝不自动放行）。
//! 2. [`PendingMerge`]：合并 Rust 本进程审批队列与侧车（Python）队列的待裁决项，按 `id`
//!    去重、本进程在前。**只做纯数据合并，不做任何网络转发** —— 跨进程转发由
//!    `gateway/approvals.rs` 在运行时完成，本模块不触碰。
//!
//! 诚实边界：本模块是判定层，不持有 [`crate::guard::gateway::ApprovalGate`]、不发起
//! 审批、不轮询裁决；它只回答「这次调用该放行 / 抬审批 / 拒绝」。

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// 每设备授权档（操作员/管理员为某实体**显式授予**的信任级别）。
///
/// 与 [`crate::devices::registry::HazardLevel`]（域的固有危险度）是两个维度：
/// HazardLevel 回答「这个域天生多危险」，本档回答「操作员已经替这台设备预先放行了多宽」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionTier {
    /// L0 只读：仅允许查询，任何写操作一律拒绝。
    ReadOnly,
    /// L1 自动：安全写可自动执行，无需审批。
    Auto,
    /// L2 需审批：写操作必须抬升审批，人批准才执行。
    ApprovalRequired,
    /// L3 人在回路：物理致动级，逐次显式批准，**禁止任何自动批准**（fail-closed）。
    HumanInLoop,
}

impl PermissionTier {
    pub fn as_u8(self) -> u8 {
        match self {
            PermissionTier::ReadOnly => 0,
            PermissionTier::Auto => 1,
            PermissionTier::ApprovalRequired => 2,
            PermissionTier::HumanInLoop => 3,
        }
    }
}

/// 拒绝原因（fail-closed 时如实区分，便于上层映射 RpcError 文案）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DenyReason {
    /// 策略中没有该实体的授权档 → 按最严拒绝（fail-closed）。
    PolicyMissing,
    /// L0 只读档却请求写操作（越权写）。
    ReadOnlyViolation,
    /// L3 物理致动在无人显式批准前的默认拒绝。
    ForbiddenNoHumanInLoop,
}

/// 授权判定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthDecision {
    /// 允许执行（读，或 L1 安全写自动放行）。
    Allow,
    /// 必须抬升审批；`human_in_loop=true` 表示 L3 物理致动，逐次人在回路，
    /// 禁止任何自动批准（含超时 fail-closed 拒绝）。
    RequireApproval { human_in_loop: bool },
    /// 拒绝（fail-closed）。
    Deny { reason: DenyReason },
}

impl AuthDecision {
    /// 是否可不经审批自动执行（仅 `Allow` 为真）。
    pub fn is_auto_allowed(self) -> bool {
        matches!(self, AuthDecision::Allow)
    }

    /// 是否要求逐次人在回路（L3）。
    pub fn requires_human_in_loop(self) -> bool {
        matches!(
            self,
            AuthDecision::RequireApproval {
                human_in_loop: true
            }
        )
    }
}

/// 设备授权策略：`entity_id -> PermissionTier`，纯内存、可序列化。
///
/// fail-closed 不变量：`authorize` 对**任何**查不到授权档的实体一律返回拒绝，
/// 绝不因为「没配过」就默认放行。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DevicePermissionPolicy {
    grants: HashMap<String, PermissionTier>,
}

impl DevicePermissionPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    /// 授予/更新某实体的授权档。
    pub fn grant(&mut self, entity_id: impl Into<String>, tier: PermissionTier) {
        self.grants.insert(entity_id.into(), tier);
    }

    /// 撤销授权（撤销后回到「策略缺失」→ fail-closed 拒绝）。
    pub fn revoke(&mut self, entity_id: &str) {
        self.grants.remove(entity_id);
    }

    /// 查询某实体的授权档（None = 未配置）。
    pub fn tier_of(&self, entity_id: &str) -> Option<PermissionTier> {
        self.grants.get(entity_id).copied()
    }

    pub fn len(&self) -> usize {
        self.grants.len()
    }

    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }

    /// 授权判定（纯函数，fail-closed）。
    ///
    /// - `requested_level` 是这次动作的固有危险档（来自
    ///   [`crate::devices::registry::HazardLevel`]）：`ReadOnly` 视作读操作，其余视作写。
    /// - 策略缺失 → [`DenyReason::PolicyMissing`]。
    /// - L0 档上的写 → [`DenyReason::ReadOnlyViolation`]。
    /// - L1 档写 → [`AuthDecision::Allow`]（自动）。
    /// - L2 档写 → [`AuthDecision::RequireApproval`]；若动作本身是 L3 物理致动则
    ///   `human_in_loop=true`。
    /// - L3 档写 → [`AuthDecision::RequireApproval{ human_in_loop: true }`]，绝不自动放行。
    pub fn authorize(
        &self,
        entity_id: &str,
        _action: &str,
        requested_level: crate::devices::registry::HazardLevel,
    ) -> AuthDecision {
        // fail-closed：策略缺失按最严处理。
        let Some(tier) = self.tier_of(entity_id) else {
            return AuthDecision::Deny {
                reason: DenyReason::PolicyMissing,
            };
        };

        // 读操作：有授权档即放行（L0 档本身就是只读，读天经地义）。
        if requested_level == crate::devices::registry::HazardLevel::ReadOnly {
            return AuthDecision::Allow;
        }

        // 写操作：按授予档判定。
        match tier {
            PermissionTier::ReadOnly => AuthDecision::Deny {
                reason: DenyReason::ReadOnlyViolation,
            },
            PermissionTier::Auto => AuthDecision::Allow,
            PermissionTier::ApprovalRequired => AuthDecision::RequireApproval {
                human_in_loop: requested_level
                    == crate::devices::registry::HazardLevel::PhysicalActuation,
            },
            PermissionTier::HumanInLoop => AuthDecision::RequireApproval {
                human_in_loop: true,
            },
        }
    }
}

/// 跨引擎待裁决项的中立快照（可序列化，不绑定 Rust `Instant`）。
///
/// Rust 本进程队列项（`PendingApproval`，带 `Instant`）与侧车队列项都投影成这个形状，
/// 供 [`PendingMerge`] 做无类型依赖的纯数据合并。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedPending {
    /// 审批 ID（跨引擎去重键）。
    pub id: String,
    /// 目标工具名。
    pub tool: String,
    /// 参数（JSON 字符串）。
    pub args: String,
    /// 来源引擎。
    pub origin: PendingOrigin,
    /// 排序键（挂起先后的单调序号，越小越早；纯数据，不用 `Instant`）。
    pub seq: u64,
}

/// 待裁决项来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingOrigin {
    /// 本进程 Rust 引擎队列。
    Local,
    /// 侧车（Python）队列。
    Sidecar,
}

/// 跨引擎审批合并器（纯数据结构，无网络）。
///
/// 对应 `gateway/approvals.rs::merge_pending` 的纯函数内核：合并本进程队列与侧车队列，
/// 按 `id` 去重，**本进程在前**（同 id 冲突时保留本进程项）。这里只做合并，
/// 不做跨进程转发——转发由运行时在拿到裁决结果后另行完成。
#[derive(Debug, Clone, Default)]
pub struct PendingMerge {
    /// 合并结果（构造后即不可变快照）。
    pub items: Vec<MergedPending>,
}

impl PendingMerge {
    /// 合并两侧队列：本进程项在前，侧车项在后；同 `id` 去重，保留先出现者（本进程优先）。
    pub fn merge(local: Vec<MergedPending>, sidecar: Vec<MergedPending>) -> Self {
        let mut seen: HashSet<String> = HashSet::new();
        let mut items = Vec::with_capacity(local.len() + sidecar.len());
        for item in local {
            if seen.insert(item.id.clone()) {
                items.push(item);
            }
        }
        for item in sidecar {
            if seen.insert(item.id.clone()) {
                items.push(item);
            }
        }
        Self { items }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 合并结果里本进程项的数量。
    pub fn local_count(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.origin == PendingOrigin::Local)
            .count()
    }

    /// 合并结果里侧车项的数量。
    pub fn sidecar_count(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.origin == PendingOrigin::Sidecar)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::registry::HazardLevel;

    fn sample_policy() -> DevicePermissionPolicy {
        let mut p = DevicePermissionPolicy::new();
        p.grant("light.living_room", PermissionTier::Auto); // L1
        p.grant("sensor.temp", PermissionTier::ReadOnly); // L0
        p.grant("climate.bedroom", PermissionTier::ApprovalRequired); // L2
        p.grant("lock.front_door", PermissionTier::HumanInLoop); // L3
        p
    }

    #[test]
    fn l0_read_only_writes_are_denied() {
        let p = sample_policy();
        // L0 档（sensor）上请求写 → 越权写拒绝。
        let d = p.authorize("sensor.temp", "turn_on", HazardLevel::SafeAuto);
        assert_eq!(
            d,
            AuthDecision::Deny {
                reason: DenyReason::ReadOnlyViolation
            }
        );
        assert!(!d.is_auto_allowed());
    }

    #[test]
    fn l1_auto_writes_are_allowed() {
        let p = sample_policy();
        let d = p.authorize("light.living_room", "turn_on", HazardLevel::SafeAuto);
        assert_eq!(d, AuthDecision::Allow);
        assert!(d.is_auto_allowed());
    }

    #[test]
    fn l2_writes_require_approval() {
        let p = sample_policy();
        let d = p.authorize("climate.bedroom", "set_temperature", HazardLevel::RequiresApproval);
        assert_eq!(
            d,
            AuthDecision::RequireApproval {
                human_in_loop: false
            }
        );
        assert!(!d.is_auto_allowed());
        assert!(!d.requires_human_in_loop());
    }

    #[test]
    fn l3_is_fail_closed_human_in_loop_never_auto() {
        let p = sample_policy();
        let d = p.authorize("lock.front_door", "lock", HazardLevel::PhysicalActuation);
        // L3：强制人在回路，绝不自动放行（fail-closed）。
        assert_eq!(
            d,
            AuthDecision::RequireApproval {
                human_in_loop: true
            }
        );
        assert!(!d.is_auto_allowed(), "L3 绝不自动放行");
        assert!(d.requires_human_in_loop());
    }

    #[test]
    fn l2_tier_but_l3_action_escalates_to_human_loop() {
        let p = sample_policy();
        // 档是 L2，但动作本身是 L3 物理致动 → 升级为 human_in_loop。
        let d = p.authorize("climate.bedroom", "unlock", HazardLevel::PhysicalActuation);
        assert_eq!(
            d,
            AuthDecision::RequireApproval {
                human_in_loop: true
            }
        );
    }

    #[test]
    fn missing_policy_is_fail_closed_deny() {
        let p = sample_policy();
        let d = p.authorize("switch.never_seen", "toggle", HazardLevel::SafeAuto);
        assert_eq!(
            d,
            AuthDecision::Deny {
                reason: DenyReason::PolicyMissing
            }
        );
        assert!(!d.is_auto_allowed());
    }

    #[test]
    fn reads_are_allowed_when_policy_exists() {
        let p = sample_policy();
        // L0 档上的读操作天经地义放行。
        assert_eq!(
            p.authorize("sensor.temp", "get", HazardLevel::ReadOnly),
            AuthDecision::Allow
        );
        // 任意档上的读都放行。
        assert_eq!(
            p.authorize("lock.front_door", "get", HazardLevel::ReadOnly),
            AuthDecision::Allow
        );
    }

    #[test]
    fn revoke_returns_to_fail_closed() {
        let mut p = sample_policy();
        p.revoke("light.living_room");
        assert_eq!(
            p.authorize("light.living_room", "turn_on", HazardLevel::SafeAuto),
            AuthDecision::Deny {
                reason: DenyReason::PolicyMissing
            }
        );
    }

    #[test]
    fn policy_is_serializable_round_trip() {
        let p = sample_policy();
        let json = serde_json::to_string(&p).expect("序列化");
        let back: DevicePermissionPolicy = serde_json::from_str(&json).expect("反序列化");
        assert_eq!(
            back.tier_of("lock.front_door"),
            Some(PermissionTier::HumanInLoop)
        );
        assert_eq!(back.len(), p.len());
    }

    fn pending(id: &str, origin: PendingOrigin, seq: u64) -> MergedPending {
        MergedPending {
            id: id.to_string(),
            tool: "devices.call".into(),
            args: "{}".into(),
            origin,
            seq,
        }
    }

    #[test]
    fn pending_merge_dedups_ids_and_local_first() {
        let local = vec![
            pending("a", PendingOrigin::Local, 1),
            pending("shared", PendingOrigin::Local, 2),
        ];
        let sidecar = vec![
            pending("shared", PendingOrigin::Sidecar, 9), // 同 id：本进程优先
            pending("b", PendingOrigin::Sidecar, 3),
        ];
        let merged = PendingMerge::merge(local, sidecar);
        // 去重后 3 项：a, shared(local), b。
        assert_eq!(merged.len(), 3);
        // 本进程在前：前两项 local，第三项 sidecar。
        assert_eq!(merged.items[0].origin, PendingOrigin::Local);
        assert_eq!(merged.items[1].origin, PendingOrigin::Local);
        assert_eq!(merged.items[2].origin, PendingOrigin::Sidecar);
        // shared 保留本进程版本（seq=2 而非侧车的 seq=9）。
        let shared = merged.items.iter().find(|i| i.id == "shared").unwrap();
        assert_eq!(shared.origin, PendingOrigin::Local);
        assert_eq!(shared.seq, 2);
        assert_eq!(merged.local_count(), 2);
        assert_eq!(merged.sidecar_count(), 1);
    }

    #[test]
    fn pending_merge_empty_sides() {
        let empty: Vec<MergedPending> = vec![];
        let m = PendingMerge::merge(empty.clone(), empty);
        assert!(m.is_empty());
        let local_only = vec![pending("x", PendingOrigin::Local, 0)];
        let m2 = PendingMerge::merge(local_only, vec![]);
        assert_eq!(m2.len(), 1);
        assert_eq!(m2.local_count(), 1);
    }
}
