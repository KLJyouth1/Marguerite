//! 模式系统（v2 工作台）
//!
//! 三档模式（plan/agent/ensemble）控制工具可用性与审批行为：
//! - plan：只读探索，修改类操作一律拒绝
//! - agent：写操作请求确认（交互审批）
//! - ensemble：受控强编排（原 bloom 升级，仍受沙箱白名单约束）；`bloom` 为已弃用别名
//!
//! 规格声明式（YAML，见 `spec.rs` 与 `modes/*.yaml`），新增模式零代码。

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::events::{Envelope, EventBus};
use crate::guard::gateway::ApprovalGate;

pub mod anchoring;
pub mod ask;
pub mod privileges;
pub mod spec;

pub use privileges::EnsemblePrivileges;
pub use spec::{ApprovalAction, ApprovalPolicy, BloomSpec, ModeSpec, ModelParams};

/// 进程级运行时模式（`mode.switch` RPC 可改；优先于静态 `config.bloom.parallel_enabled`）。
///
/// 让「Bloom 模式开启」成为**运行时开关**而非仅启动时配置：前端 `mode.switch`
/// 立即生效（本次会话即走 Bloom 并行编排），无需重启网关。默认 `Agent`。
static RUNTIME_MODE: OnceLock<Mutex<ModeKind>> = OnceLock::new();

fn runtime_mode_slot() -> &'static Mutex<ModeKind> {
    RUNTIME_MODE.get_or_init(|| Mutex::new(ModeKind::Agent))
}

/// 读取当前运行时模式（默认 `Agent`）。
pub fn runtime_mode() -> ModeKind {
    *runtime_mode_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// 设置运行时模式（幂等），返回实际生效的模式。
pub fn set_runtime_mode(kind: ModeKind) -> ModeKind {
    let mut guard = runtime_mode_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *guard = kind;
    kind
}

/// 模式种类
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModeKind {
    /// 只读探索，先计划后执行
    Plan,
    /// 交互审批，写操作需确认
    Agent,
    /// 受控强编排（原 bloom 升级）：全自动执行，仍受沙箱白名单约束
    Ensemble,
    /// Bloom 模式：统一整合锚定/PTC/YOLO/MAX/AUTO/SWARM/DEEP（策划 §二升级）。
    /// 与 Ensemble 并列：Bloom = 带 anchoring+ptc 三阶段晋升 + 统一配置的
    /// 声明式强编排；Ensemble 保留为兼容老配置的扁平编排。
    Bloom,
}

impl ModeKind {
    /// 模式名（YAML 文件名）
    pub fn as_str(&self) -> &'static str {
        match self {
            ModeKind::Plan => "plan",
            ModeKind::Agent => "agent",
            ModeKind::Ensemble => "ensemble",
            ModeKind::Bloom => "bloom",
        }
    }

    /// 从名称解析（`/mode plan` 等）
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "plan" => Some(ModeKind::Plan),
            "agent" => Some(ModeKind::Agent),
            "ensemble" => Some(ModeKind::Ensemble),
            "bloom" => Some(ModeKind::Bloom),
            // 已弃用别名：映射到 Bloom（原 bloom 升级后不再走 Ensemble 别名）
            "yolo" | "max" | "auto" | "swarm" | "deep" | "ptc" => Some(ModeKind::Bloom),
            _ => None,
        }
    }
}

impl std::fmt::Display for ModeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 门控决策
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// 允许执行
    Allow,
    /// 需用户确认（agent 模式写操作）
    Confirm { reason: String },
    /// 拒绝执行
    Deny { reason: String },
}

/// 模式变更记录（事件总线预留）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeChange {
    /// 原模式
    pub from: ModeKind,
    /// 新模式
    pub to: ModeKind,
}

/// 模式切换错误
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeSwitchError {
    /// 目标模式无效
    InvalidTarget(ModeKind),
    /// 规格缺失
    SpecNotFound,
}

impl std::fmt::Display for ModeSwitchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModeSwitchError::InvalidTarget(m) => write!(f, "无效模式: {}", m.as_str()),
            ModeSwitchError::SpecNotFound => write!(f, "模式规格缺失"),
        }
    }
}

/// 模式管理器
pub struct ModeManager {
    current: ModeKind,
    specs: HashMap<ModeKind, ModeSpec>,
    gate: Arc<ApprovalGate>,
    /// 模式变更回调（事件总线接入点，A4 事件流上线后替换）
    mode_change_cb: Option<Arc<dyn Fn(ModeChange) + Send + Sync>>,
    /// 事件总线（A4 事件流埋点；None 时无事件发布）
    events: Option<Arc<EventBus>>,
}

impl ModeManager {
    /// 使用内置规格创建（默认 agent 模式）
    pub fn new() -> Self {
        let mut specs = HashMap::new();
        specs.insert(ModeKind::Plan, spec::builtin::plan());
        specs.insert(ModeKind::Agent, spec::builtin::agent());
        specs.insert(ModeKind::Ensemble, spec::builtin::bloom());
        specs.insert(ModeKind::Bloom, spec::builtin::bloom());
        Self {
            current: ModeKind::Agent,
            specs,
            gate: Arc::new(ApprovalGate::default()),
            mode_change_cb: None,
            events: None,
        }
    }

    /// 从目录加载覆盖规格（`~/.marguerite/modes/*.yaml`）
    ///
    /// 目录不存在或文件缺失时回退内置规格。
    pub fn from_dir(dir: &Path) -> Self {
        let mut mgr = Self::new();
        if !dir.exists() {
            return mgr;
        }
        for kind in [
            ModeKind::Plan,
            ModeKind::Agent,
            ModeKind::Ensemble,
            ModeKind::Bloom,
        ] {
            let path = dir.join(format!("{}.yaml", kind.as_str()));
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(spec) = ModeSpec::from_yaml(&content) {
                    mgr.specs.insert(kind, spec);
                }
            }
        }
        // 兼容过渡期：legacy yolo.yaml 覆盖规格映射到 Bloom
        let legacy = dir.join("yolo.yaml");
        if let Ok(content) = std::fs::read_to_string(&legacy) {
            if let Ok(spec) = ModeSpec::from_yaml(&content) {
                mgr.specs.insert(ModeKind::Bloom, spec);
            }
        }
        mgr
    }

    /// 当前模式
    pub fn current(&self) -> ModeKind {
        self.current
    }

    /// 切换模式
    ///
    /// 副作用：作废挂起审批；触发 `mode_change_cb`（若已注册）。
    pub fn switch(&mut self, to: ModeKind) -> Result<(), ModeSwitchError> {
        if !self.specs.contains_key(&to) {
            return Err(ModeSwitchError::SpecNotFound);
        }
        if self.current == to {
            return Ok(()); // 幂等
        }
        let change = ModeChange {
            from: self.current,
            to,
        };
        self.current = to;
        // 作废挂起审批（切换模式后原审批上下文失效）
        self.gate.cancel_pending();
        // A4 事件流埋点：发布 mode.changed（若已注册事件总线）
        if let Some(bus) = &self.events {
            bus.publish(Envelope::mode_changed(
                "core",
                change.from.as_str(),
                self.current.as_str(),
            ));
        }
        if let Some(cb) = &self.mode_change_cb {
            cb(change);
        }
        Ok(())
    }

    /// 注册模式变更回调（事件总线预留接口）
    pub fn set_mode_change_callback(&mut self, cb: Arc<dyn Fn(ModeChange) + Send + Sync>) {
        self.mode_change_cb = Some(cb);
    }

    /// 注册事件总线（A4 事件流埋点接口）
    pub fn set_event_bus(&mut self, bus: Arc<EventBus>) {
        self.events = Some(bus);
    }

    /// 获取模式规格
    pub fn spec(&self, mode: ModeKind) -> &ModeSpec {
        self.specs.get(&mode).expect("模式规格必须存在")
    }

    /// 当前模式的 Bloom 增强规格（非 Bloom 模式返回默认 BloomSpec）
    pub fn bloom_spec(&self) -> BloomSpec {
        self.spec(self.current).bloom.clone().unwrap_or_default()
    }

    /// 当前模式的沙箱策略（对齐 TUI 工作台 分层模型）
    pub fn sandbox_policy(&self) -> crate::sandbox::policy::SandboxPolicy {
        match self.current {
            ModeKind::Plan => crate::sandbox::policy::SandboxPolicy::ReadOnly,
            ModeKind::Agent => crate::sandbox::policy::SandboxPolicy::WorkspaceWrite {
                writable_roots: vec![],
                network_access: false,
                exclude_tmpdir: false,
            },
            ModeKind::Ensemble | ModeKind::Bloom => self.bloom_spec().sandbox,
        }
    }

    /// 审批门控引用
    pub fn approval_gate(&self) -> &Arc<ApprovalGate> {
        &self.gate
    }

    /// 门控决策：按当前模式 × 工具类别判定
    pub fn gate(&self, tool: &str) -> GateDecision {
        let spec = self.spec(self.current);

        // 1) 显式黑名单
        if spec.denied_tools.iter().any(|t| t == tool) {
            return GateDecision::Deny {
                reason: format!("模式 {} 禁止 {tool}", self.current.as_str()),
            };
        }
        // 2) 显式白名单（非空时不在名单内一律拒绝）
        if !spec.allowed_tools.is_empty() && !spec.allowed_tools.contains(&tool.to_string()) {
            return GateDecision::Deny {
                reason: format!("{tool} 不在模式 {} 的白名单", self.current.as_str()),
            };
        }

        let class = classify_tool(tool);
        match self.current {
            ModeKind::Ensemble | ModeKind::Bloom => {
                // MAX+YOLO 融合语义：特权驱动审批。
                // - 系统级/桌面控制（system_control/control_desktop 等）属 Bloom/Ensemble
                //   内含能力：skip_permissions（dangerously-skip-permissions，MAX 全开）
                //   → 免确认放行；否则人工确认（尊重沙箱白名单之上的边界）。
                // - auto_approve（enable-auto-mode）+ allowed_tools（--allowedTools）
                //   决定其余工具是否免确认放行；auto_approve 关闭时退化为 agent 式审批
                let privs = self
                    .spec(self.current)
                    .bloom
                    .clone()
                    .unwrap_or_default();
                match class {
                    ToolClass::Desktop => {
                        if privs.privileges.skip_permissions && privs.privileges.tool_allowed(tool)
                        {
                            GateDecision::Allow
                        } else {
                            GateDecision::Confirm {
                                reason: "系统级鼠标键盘/桌面控制需要人工确认（Bloom 默认；开启 skip_permissions 后免确认）"
                                    .into(),
                            }
                        }
                    }
                    _ if !privs.privileges.tool_allowed(tool) => GateDecision::Deny {
                        reason: format!("{tool} 不在 Bloom 特权白名单（allowed_tools）"),
                    },
                    _ if privs.privileges.is_auto_approve() => GateDecision::Allow,
                    _ => {
                        let action = match class {
                            ToolClass::Write => spec.approval_policy.mutating,
                            ToolClass::ExecWrite => spec.approval_policy.exec_write,
                            ToolClass::Network => spec.approval_policy.network,
                            ToolClass::Desktop => spec.approval_policy.desktop,
                            _ => ApprovalAction::Allow,
                        };
                        if action == ApprovalAction::Confirm {
                            GateDecision::Confirm {
                                reason: format!(
                                    "{tool} 将修改工作区或访问外部（bloom auto_approve 已关闭）"
                                ),
                            }
                        } else {
                            GateDecision::Allow
                        }
                    }
                }
            }
            ModeKind::Agent => {
                let action = match class {
                    ToolClass::Write => spec.approval_policy.mutating,
                    ToolClass::ExecWrite => spec.approval_policy.exec_write,
                    ToolClass::Network => spec.approval_policy.network,
                    ToolClass::Desktop => spec.approval_policy.desktop,
                    _ => ApprovalAction::Allow,
                };
                if action == ApprovalAction::Confirm {
                    GateDecision::Confirm {
                        reason: format!("{tool} 将修改工作区或访问外部",),
                    }
                } else {
                    GateDecision::Allow
                }
            }
            ModeKind::Plan => match class {
                ToolClass::Write | ToolClass::ExecWrite | ToolClass::Desktop => {
                    GateDecision::Deny {
                        reason: "plan 模式禁止修改".into(),
                    }
                }
                _ => GateDecision::Allow,
            },
        }
    }
}

impl Default for ModeManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 工具类别（门控矩阵的行）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolClass {
    /// 只读操作
    Read,
    /// 写操作（文件修改）
    Write,
    /// 写类命令执行
    ExecWrite,
    /// 只读命令执行
    ExecRead,
    /// 出网操作
    Network,
    /// 桌面控制
    Desktop,
    /// 护栏检查
    Guard,
}

/// 工具类别判定
///
/// 默认归为 `Read`（安全）。写/网络/桌面/执行类需显式声明。
fn classify_tool(tool: &str) -> ToolClass {
    match tool {
        // 桌面/系统级控制
        "control_desktop" | "browser_control" | "system_control" => ToolClass::Desktop,
        // 出网
        "web_fetch" | "web_search" | "browser" | "browser_a11y" | "see" => ToolClass::Network,
        // 写类命令
        "exec" | "terminal" | "install_dependency" => ToolClass::ExecWrite,
        // 只读命令
        "run_tests" | "git_status" | "git_diff" | "git_log" | "doctor" => ToolClass::ExecRead,
        // 写操作
        "edit_file" | "write_code" | "create_file" | "delete_file" | "rename_file"
        | "apply_patch" | "rollback" | "git_commit" => ToolClass::Write,
        // 护栏
        "syntax" | "semantic" | "fact" | "contract" | "guard" => ToolClass::Guard,
        // 默认只读
        _ => ToolClass::Read,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    #[test]
    fn test_m1_plan_blocks_write() {
        let mut mgr = ModeManager::new();
        mgr.switch(ModeKind::Plan).unwrap();
        let d = mgr.gate("edit_file");
        assert!(matches!(d, GateDecision::Deny { .. }));
        if let GateDecision::Deny { reason } = d {
            assert!(reason.contains("plan"));
        }
    }

    #[test]
    fn test_m2_agent_confirm_write() {
        let mgr = ModeManager::new();
        let d = mgr.gate("edit_file");
        assert!(matches!(d, GateDecision::Confirm { .. }));
    }

    #[test]
    fn test_m3_agent_allow_read() {
        let mgr = ModeManager::new();
        assert_eq!(mgr.gate("read_file"), GateDecision::Allow);
    }

    #[test]
    fn test_m4_yolo_allow_write() {
        let mut mgr = ModeManager::new();
        mgr.switch(ModeKind::Bloom).unwrap();
        assert_eq!(mgr.gate("edit_file"), GateDecision::Allow);
    }

    #[test]
    fn test_m5_bloom_desktop_confirm_by_default() {
        // Bloom 默认：系统级鼠标键盘/桌面控制需人工确认（Bloom 内含能力，不再恒禁）
        let mut mgr = ModeManager::new();
        mgr.switch(ModeKind::Bloom).unwrap();
        let d = mgr.gate("control_desktop");
        assert!(matches!(d, GateDecision::Confirm { .. }));
        let d = mgr.gate("system_control");
        assert!(matches!(d, GateDecision::Confirm { .. }));
    }

    #[test]
    fn test_m5b_bloom_skip_permissions_allows_desktop() {
        // Bloom + skip_permissions（MAX 全开）：系统级控制免确认放行
        let mut mgr = ModeManager::new();
        mgr.switch(ModeKind::Bloom).unwrap();
        // 直接以全开特权规格覆盖 bloom 规格
        let mut spec = mgr.spec(ModeKind::Bloom).clone();
        if let Some(b) = spec.bloom.as_mut() {
            b.privileges.skip_permissions = true;
        }
        mgr.specs.insert(ModeKind::Bloom, spec);
        assert_eq!(mgr.gate("system_control"), GateDecision::Allow);
        assert_eq!(mgr.gate("control_desktop"), GateDecision::Allow);
    }

    #[test]
    fn test_m6_switch_cancels_pending_and_fires_callback() {
        let mut mgr = ModeManager::new();
        // 挂起一个审批
        let p = mgr
            .approval_gate()
            .request(
                "edit_file",
                "{}".into(),
                crate::guard::gateway::RiskLevel::Medium,
            )
            .unwrap();
        assert!(mgr.approval_gate().pending().is_some());

        // 注册回调
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_cb = calls.clone();
        mgr.set_mode_change_callback(Arc::new(move |_c: ModeChange| {
            calls_cb.fetch_add(1, Ordering::SeqCst);
        }));

        // 切换 → 审批作废 + 回调触发
        mgr.switch(ModeKind::Plan).unwrap();
        assert!(mgr.approval_gate().pending().is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(mgr.current(), ModeKind::Plan);
        // 旧审批 ID 已失效
        assert_eq!(
            mgr.approval_gate().outcome(&p.id),
            crate::guard::gateway::ApprovalOutcome::Denied
        );
    }

    #[test]
    fn test_m7_switch_idempotent() {
        let mut mgr = ModeManager::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_cb = calls.clone();
        mgr.set_mode_change_callback(Arc::new(move |_c: ModeChange| {
            calls_cb.fetch_add(1, Ordering::SeqCst);
        }));
        mgr.switch(ModeKind::Agent).unwrap(); // 已是 Agent
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        mgr.switch(ModeKind::Bloom).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        mgr.switch(ModeKind::Bloom).unwrap(); // 幂等
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_m8_from_dir_missing_falls_back() {
        let mgr = ModeManager::from_dir(Path::new("__nonexistent_modes_dir__"));
        assert_eq!(mgr.spec(ModeKind::Plan).name, "plan");
        assert_eq!(mgr.spec(ModeKind::Bloom).name, "bloom");
    }

    #[test]
    fn test_m9_non_interactive_fail_closed() {
        let mgr = ModeManager::new();
        mgr.approval_gate().set_interactive(false);
        let p = mgr
            .approval_gate()
            .request(
                "edit_file",
                "{}".into(),
                crate::guard::gateway::RiskLevel::Medium,
            )
            .unwrap();
        assert_eq!(
            mgr.approval_gate().outcome(&p.id),
            crate::guard::gateway::ApprovalOutcome::Denied
        );
    }

    #[test]
    fn test_plan_allows_read_and_guard() {
        let mut mgr = ModeManager::new();
        mgr.switch(ModeKind::Plan).unwrap();
        assert_eq!(mgr.gate("read_file"), GateDecision::Allow);
        assert_eq!(mgr.gate("syntax"), GateDecision::Allow);
        assert!(matches!(mgr.gate("exec"), GateDecision::Deny { .. }));
        assert!(matches!(
            mgr.gate("control_desktop"),
            GateDecision::Deny { .. }
        ));
    }

    #[test]
    fn test_agent_network_confirm() {
        let mgr = ModeManager::new();
        assert!(matches!(
            mgr.gate("web_fetch"),
            GateDecision::Confirm { .. }
        ));
    }

    #[test]
    fn test_mode_kind_from_name() {
        assert_eq!(ModeKind::from_name("plan"), Some(ModeKind::Plan));
        assert_eq!(ModeKind::from_name("agent"), Some(ModeKind::Agent));
        assert_eq!(ModeKind::from_name("yolo"), Some(ModeKind::Bloom));
        assert_eq!(ModeKind::from_name("bogus"), None);
        assert_eq!(ModeKind::Plan.as_str(), "plan");
    }

    #[test]
    fn test_mode_switch_error_display() {
        let mut mgr = ModeManager::new();
        // 从目录加载不存在 → 内置；切换无效模式不适用，直接验证 SpecNotFound 路径
        let e = ModeSwitchError::InvalidTarget(ModeKind::Plan);
        assert!(e.to_string().contains("plan"));
        let _ = &mut mgr;
    }

    #[test]
    fn test_mode_publishes_event_on_switch() {
        let mut mgr = ModeManager::new();
        let bus = Arc::new(EventBus::default());
        mgr.set_event_bus(bus.clone());
        let since = Instant::now();
        mgr.switch(ModeKind::Bloom).unwrap();
        let replayed = bus.history(since);
        assert_eq!(replayed.len(), 1, "切换模式应发布 1 条 mode.changed 事件");
        assert_eq!(replayed[0].payload["kind"], "mode.changed");
        assert_eq!(replayed[0].payload["from"], "agent");
        assert_eq!(replayed[0].payload["to"], "bloom");
    }
}
