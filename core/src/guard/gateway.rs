//! 审批门控（v2 工作台）
//!
//! 危险操作（写文件/执行/出网/桌面控制）的统一审批闸口：
//! - 同一时刻仅一个挂起审批
//! - 超时未响应按 DENY（fail-closed）
//! - 非交互路径（CLI 脚本/无确认通道）默认拒绝

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 风险等级
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    /// 低风险（只读/无害）
    #[default]
    Low,
    /// 中风险（写操作）
    Medium,
    /// 高风险（破坏性/出网）
    High,
}

/// 审批/权限预设的来源（对齐上游 PR #2608 `permission/preset` 的
/// `origin: default | selection | inferred` 溯源语义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalOrigin {
    /// 内置默认预设（未经用户选择）
    #[default]
    Default,
    /// 用户显式选择（模式切换 / 目录覆盖规格）
    Selection,
    /// 启发式推断
    Inferred,
}

impl ApprovalOrigin {
    /// 小写名（wire 值）。
    pub fn as_str(&self) -> &'static str {
        match self {
            ApprovalOrigin::Default => "default",
            ApprovalOrigin::Selection => "selection",
            ApprovalOrigin::Inferred => "inferred",
        }
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
    /// 权限预设来源（default / selection / inferred）
    pub origin: ApprovalOrigin,
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

    /// 挂起一个审批请求（权限预设来源 = [`ApprovalOrigin::Default`]）。
    ///
    /// 若已有挂起审批则返回 `None`（一次仅一个）。
    pub fn request(&self, tool: &str, args: String, risk: RiskLevel) -> Option<PendingApproval> {
        self.request_with_origin(tool, args, risk, ApprovalOrigin::default())
    }

    /// 挂起一个审批请求，显式标注权限预设来源（上游 PR #2608 溯源语义）。
    ///
    /// 若已有挂起审批则返回 `None`（一次仅一个）。
    pub fn request_with_origin(
        &self,
        tool: &str,
        args: String,
        risk: RiskLevel,
        origin: ApprovalOrigin,
    ) -> Option<PendingApproval> {
        let mut guard = self.pending.lock().unwrap();
        if guard.is_some() {
            return None;
        }
        let approval = PendingApproval {
            id: format!("ap_{}", uuid::Uuid::new_v4()),
            tool: tool.to_string(),
            args,
            risk,
            origin,
            created_at: Instant::now(),
        };
        *guard = Some(approval.clone());
        Some(approval)
    }

    /// 当前挂起的审批
    pub fn pending(&self) -> Option<PendingApproval> {
        self.pending.lock().unwrap().clone()
    }

    /// 当前挂起且**未超时**的审批（只读，不改变门控状态）。
    ///
    /// 与 [`ApprovalGate::outcome`] 的分工：`outcome` 是**消费式**查询 —— 非交互模式直接
    /// 判 `Denied`、超时推进为 `Timeout`，都会改写状态；清单类只读接口（WebUI 快照）不能
    /// 走它，否则「看一眼列表」就把待裁决项判掉了。
    pub fn live_pending(&self) -> Option<PendingApproval> {
        self.pending
            .lock()
            .unwrap()
            .as_ref()
            .filter(|p| p.created_at.elapsed() <= self.timeout)
            .cloned()
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
        // v3.3.1：`guard.approval_timeout_secs` 非 0 时覆盖默认 60s
        let secs = crate::mode::approval_timeout_secs();
        let timeout = if secs > 0 {
            Duration::from_secs(secs)
        } else {
            Duration::from_secs(60)
        };
        Self::new(timeout)
    }
}

// ───────────────────────── agent 工具回路专用审批门 ─────────────────────────
//
// 缺陷形态（P0-4）：链式执行器（`chain::executor`）在步骤分发前走
// [`crate::mode::ModeManager::gate`] + `ApprovalGate`，但**智能体工具回路**
// （`agent::tool_cycle::dispatch_cached` → `execute_tool_internal`）此前只有
// 「Plan 模式硬拒」一道门，没有任何审批 —— 同一条聊天回合里，模型调 `shell_exec`
// 删目录、调 `delete_file_permanent` 永久删文件、调桌面控制驱动本机 UI，
// 都不会弹确认卡，直接执行。这不是「能力子集」，是同一产品内两条执行路径的安全
// 口径不一致。
//
// 为什么用进程级单门而不是把 gate 塞进 `ToolCycle`：`ToolCycle` 是通用基础设施，
// 有 6+ 处纯注册表构造点（runtime / stream_loop / subagent / multi_strategy），
// 全都要传 gate 会改动面过大；而审批门本身就该是进程级「同一时刻一个挂起项」的语义。

static AGENT_GATE: OnceLock<Arc<ApprovalGate>> = OnceLock::new();
/// 宿主是否已领养 agent 回路审批门（见 [`install_agent_gate`]）。
static AGENT_GATE_ADOPTED: AtomicBool = AtomicBool::new(false);

/// agent 工具回路的进程级审批门（懒构造；宿主可用 [`install_agent_gate`] 替换）。
pub fn agent_gate() -> Arc<ApprovalGate> {
    AGENT_GATE
        .get_or_init(|| {
            // 测试进程内若真等到 60s 审批超时，会让同进程的其它用例被拖慢；
            // 生产路径仍走配置的审批超时（`guard.approval_timeout_secs`，默认 60s）。
            #[cfg(test)]
            let gate = Arc::new(ApprovalGate::new(Duration::from_millis(10)));
            #[cfg(not(test))]
            let gate = Arc::new(ApprovalGate::default());
            gate
        })
        .clone()
}

/// 宿主领养：把自己的 gate 交给 agent 回路使用，使挂起的审批与宿主的裁决通道
/// 是**同一个门**（否则宿主拿到 approval_id 也裁不掉，只能等到超时）。
///
/// 首次调用生效（幂等）；后到者被忽略（返回 `false`），避免多宿主互相覆盖。
/// TUI 的 `UiEffect::ResolveApproval` 走 `state.gate.resolve(id, …)`，故 TUI 必须在
/// 启动时领养，否则智能体主链抬起的审批在 TUI 里无法裁决。
pub fn install_agent_gate(gate: Arc<ApprovalGate>) -> bool {
    let installed = AGENT_GATE.set(gate).is_ok();
    // 测试进程里没有真实宿主终端（不会有人按「允许」），若跟着点亮「宿主在场」，
    // 同进程内后续用例的高危工具调用就会白等一个审批超时（默认 60s）。
    if installed && !cfg!(test) {
        AGENT_GATE_ADOPTED.store(true, Ordering::SeqCst);
    }
    installed
}

/// agent 回路此刻是否存在**可裁决**该挂起项的宿主。
///
/// - 网关（WebUI / GUI 控制台）启动时经 `events::set_global_bus` 注册全局事件总线，
///   控制台每 2s 轮询 `/api/approvals`（同一 `approval_registry`）→ 有；
/// - TUI 经 [`install_agent_gate`] 领养同一门，内联审批框可裁决 → 有；
/// - CLI 脚本 / 未启动宿主的库调用 / 单元测试：**没有人能裁决** → 没有。
///
/// 没有宿主时必须立即按拒绝处理（[`ApprovalGate::outcome`] 的非交互语义），
/// 而不是让工具循环白等一个 60s 超时 —— 那对命令行用户就是「卡住一分钟」。
/// 与 CLI 既有的「非交互 → 写命令 fail-closed」约定同口径。
pub fn agent_approval_host_present() -> bool {
    AGENT_GATE_ADOPTED.load(Ordering::SeqCst) || crate::events::global_bus().is_some()
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
    fn test_request_defaults_to_default_origin() {
        let gate = ApprovalGate::new(Duration::from_secs(60));
        let p = gate
            .request("edit_file", "{}".into(), RiskLevel::Medium)
            .unwrap();
        assert_eq!(p.origin, ApprovalOrigin::Default);
    }

    #[test]
    fn test_request_with_origin_carries_selection() {
        let gate = ApprovalGate::new(Duration::from_secs(60));
        let p = gate
            .request_with_origin(
                "exec",
                "[]".into(),
                RiskLevel::High,
                ApprovalOrigin::Selection,
            )
            .unwrap();
        assert_eq!(p.origin, ApprovalOrigin::Selection);
        assert!(gate.approve(&p.id));
    }

    #[test]
    fn test_origin_serializes_lowercase() {
        assert_eq!(
            serde_json::to_value(ApprovalOrigin::Selection).unwrap(),
            serde_json::json!("selection")
        );
        assert_eq!(
            serde_json::to_value(ApprovalOrigin::Inferred).unwrap(),
            serde_json::json!("inferred")
        );
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
