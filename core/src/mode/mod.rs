//! 模式系统（v2 工作台）
//!
//! **唯一合法的用户模式是 3×3：** 强度（极简 / 标准 / 极致）× 工作模式（咨询 / 方案 / 执行）
//! = 9 种组合，定义见 `crate::domain::UserMode`。旧模式（bloom / research / automation /
//! safe / work 等）的能力已被重新规划升级进这三个强度档位，不再作为独立用户模式暴露。
//!
//! `ModeKind`（plan / agent / ensemble / bloom / omni-transcend）是**派生的运行时执行档位**，
//! 由 `from_user_mode` 从 3×3 用户模式推导，控制工具可用性与审批行为：
//! - plan：只读探索，先出书面计划，修改类操作一律拒绝（← 咨询 Ask）
//! - agent：写操作请求确认（交互审批）（← 方案 Plan）
//! - ensemble：受控强编排（极简执行）
//! - bloom：深度打磨，锚定/PTC/YOLO/MAX/DEEP 声明式强编排（标准执行）
//! - omni-transcend：全知超验，带全谱超模增强的强编排（极致执行）
//!
//! 规格声明式（YAML，见 `spec.rs` 与 `modes/*.yaml`），新增模式零代码。

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::events::{Envelope, EventBus};
use crate::guard::gateway::{ApprovalGate, ApprovalOrigin};

pub mod anchoring;
pub mod ask;
pub mod privileges;
pub mod spec;
pub mod swarm;

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

/// 测试专用：串行化所有会改写全局 runtime_mode 的测试，避免并行竞态
/// （如 mode_stage 读取全局模式时被其他测试临时改成 Bloom 导致断言抖动）。
#[cfg(test)]
pub(crate) fn test_runtime_mode_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// v3.3.1 审批门超时的进程级配置（秒；0 = 未配置，用 ApprovalGate 默认 60s）。
static APPROVAL_TIMEOUT: OnceLock<Mutex<u64>> = OnceLock::new();

fn approval_timeout_slot() -> &'static Mutex<u64> {
    APPROVAL_TIMEOUT.get_or_init(|| Mutex::new(0))
}

/// 读取进程级审批门超时（秒；0 = 未配置）。
pub fn approval_timeout_secs() -> u64 {
    *approval_timeout_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// 设置进程级审批门超时（秒）。网关启动时从 `guard.approval_timeout_secs` 注入；
/// `ApprovalGate::default()` 构造时读取，非 0 则覆盖默认 60s。
pub fn set_approval_timeout_secs(secs: u64) {
    *approval_timeout_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = secs;
}

/// 模式种类
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModeKind {
    /// 只读探索，先计划后执行
    Plan,
    /// 交互审批，写操作需确认
    Agent,
    /// 受控强编排：兼容老配置的扁平编排（bloom 的简化前身，现与 Bloom 并列保留）
    Ensemble,
    /// Bloom 模式：统一整合锚定/PTC/YOLO/MAX/AUTO/SWARM/DEEP（策划 §二升级）。
    /// 与 Ensemble 并列：Bloom = 带 anchoring+ptc 三阶段晋升 + 统一配置的
    /// 声明式强编排；Ensemble 保留为兼容老配置的扁平编排。
    Bloom,
    /// 超星 x · 全谱超模（Omni-Transcend）：梁神两阶段锚定 + Plan 探索 +
    /// MAX 深想(4 层/4 验) + YOLO 免确认快照执行 + DEEP 双模型/SWARM/AUTO 工作台全域增强。
    OmniTranscend,
}

impl ModeKind {
    /// 模式名（YAML 文件名）
    pub fn as_str(&self) -> &'static str {
        match self {
            ModeKind::Plan => "plan",
            ModeKind::Agent => "agent",
            ModeKind::Ensemble => "ensemble",
            ModeKind::Bloom => "bloom",
            ModeKind::OmniTranscend => "omni-transcend",
        }
    }

    /// 从名称解析（`/mode plan`、`mode.switch` RPC 等）。
    ///
    /// 归一化后再匹配：大小写不敏感，且**下划线与连字符等价**。
    /// 历史上前端 `activeMode.ts` 发送下划线形态 `omni_transcend`，而规格名是连字符
    /// `omni-transcend`，导致 `mode.switch` 返回 `INVALID_REQUEST`、切换静默失败。
    pub fn from_name(s: &str) -> Option<Self> {
        let norm = s.trim().to_ascii_lowercase().replace('_', "-");
        match norm.as_str() {
            "plan" => Some(ModeKind::Plan),
            "agent" => Some(ModeKind::Agent),
            "ensemble" => Some(ModeKind::Ensemble),
            "bloom" => Some(ModeKind::Bloom),
            "omni-transcend" | "omni" | "omnix" | "ostar" => Some(ModeKind::OmniTranscend),
            // 已弃用别名：映射到 Bloom（原 bloom 升级后不再走 Ensemble 别名）
            "yolo" | "max" | "auto" | "swarm" | "deep" | "ptc" => Some(ModeKind::Bloom),
            _ => None,
        }
    }

    /// 全部可选模式（单一事实源）
    ///
    /// 网关 `mode.get` 的 `available`、CLI `/mode` 补全、WebUI 模式选择器
    /// 一律从此处派生，禁止再各自硬编码字符串数组——历史上 `mode.get` 硬编码
    /// `["plan","agent","bloom"]`，导致 `ensemble` 与 `omni-transcend` 虽已实装
    /// 却在前端永久不可达。
    pub const ALL: &'static [ModeKind] = &[
        ModeKind::Plan,
        ModeKind::Agent,
        ModeKind::Ensemble,
        ModeKind::Bloom,
        ModeKind::OmniTranscend,
    ];

    /// 全部模式名（`as_str()` 顺序与 [`ModeKind::ALL`] 一致）
    pub fn all_names() -> Vec<&'static str> {
        Self::ALL.iter().map(|m| m.as_str()).collect()
    }

    /// 人类可读标签（WebUI 模式选择器 / 模式面板展示用）
    pub fn label(&self) -> &'static str {
        match self {
            ModeKind::Plan => "Plan 只读探索",
            ModeKind::Agent => "Agent 交互审批",
            ModeKind::Ensemble => "Ensemble 受控强编排",
            ModeKind::Bloom => "Bloom 锚定强编排",
            ModeKind::OmniTranscend => "OmniTranscend 全谱超模",
        }
    }
}

impl std::fmt::Display for ModeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 由 3×3 用户模式派生运行时执行模式（门控 / 沙箱 / 审批）。
///
/// 旧模式的能力被重新规划升级进三个强度档位（见 [`crate::domain::UserMode`]）：
/// - **咨询 Ask** → `Plan`（只读探索，写类操作一律拒绝）
/// - **方案 Plan** → `Agent`（写操作交互审批，不执行）
/// - **执行 DoIt** → 随强度升级 `Ensemble`(极简) → `Bloom`(标准) → `OmniTranscend`(极致)
///
/// 这是「旧模式能力升格」的集中落点：用户只选 3×3，运行时执行档位由其派生，
/// 不暴露在 `mode.switch` 的公开候选里（仍是 `ModeKind::ALL` 内部可达，供 `from_user_mode` 复用）。
pub fn from_user_mode(mode: crate::domain::UserMode) -> ModeKind {
    use crate::domain::{ModeIntensity, WorkingMode};
    match mode.working() {
        WorkingMode::Ask => ModeKind::Plan,
        WorkingMode::Plan => ModeKind::Agent,
        WorkingMode::DoIt => match mode.intensity() {
            ModeIntensity::Minimal => ModeKind::Ensemble,
            ModeIntensity::Standard => ModeKind::Bloom,
            ModeIntensity::Extreme => ModeKind::OmniTranscend,
        },
    }
}

/// 反向映射：运行时执行档 → 规范 3×3 用户模式（`mode.get` 用）。
///
/// 非一对一（咨询/方案两态不含强度语义，取其规范中档 `Standard`），执行态则保留强度：
/// `Plan→标准·咨询`、`Agent→标准·方案`、`Ensemble→极简·执行`、`Bloom→标准·执行`、
/// `OmniTranscend→极致·执行`。用户在 3×3 网格上的精确选择经 `mode.switch` 的
/// `userMode` 键下发，落到运行时执行档；`mode.get` 由此回显当前格位。
pub fn default_user_mode_for(kind: ModeKind) -> crate::domain::UserMode {
    use crate::domain::UserMode;
    match kind {
        ModeKind::Plan => UserMode::StandardAsk,
        ModeKind::Agent => UserMode::StandardPlan,
        ModeKind::Ensemble => UserMode::MinimalDoIt,
        ModeKind::Bloom => UserMode::StandardDoIt,
        ModeKind::OmniTranscend => UserMode::ExtremeDoIt,
    }
}

#[cfg(test)]
mod user_mode_x3_tests {
    use super::*;
    use crate::domain::{ModeIntensity, UserMode, WorkingMode};

    #[test]
    fn three_by_three_roundtrips_to_runtime_kind() {
        // Ask/Plan 不含强度语义 → 反向取中档；DoIt 保留强度。
        for m in UserMode::all() {
            let kind = from_user_mode(*m);
            let derived_intensity = match m.working() {
                WorkingMode::Ask | WorkingMode::Plan => ModeIntensity::Standard,
                WorkingMode::DoIt => m.intensity(),
            };
            let back = UserMode::from_key(default_user_mode_for(kind).as_key()).unwrap();
            assert_eq!(back.intensity(), derived_intensity, "{} 强度回环失败", m.as_key());
            assert_eq!(back.working(), m.working(), "{} 工作态回环失败", m.as_key());
        }
    }

    #[test]
    fn default_user_mode_maps_doit_precisely() {
        assert_eq!(default_user_mode_for(ModeKind::Ensemble).intensity(), ModeIntensity::Minimal);
        assert_eq!(default_user_mode_for(ModeKind::Bloom).intensity(), ModeIntensity::Standard);
        assert_eq!(default_user_mode_for(ModeKind::OmniTranscend).intensity(), ModeIntensity::Extreme);
        assert_eq!(default_user_mode_for(ModeKind::Plan).working(), WorkingMode::Ask);
        assert_eq!(default_user_mode_for(ModeKind::Agent).working(), WorkingMode::Plan);
    }
}

/// 运行时模式是否**拒绝**执行该工具（工具执行链路上的硬拦截）。
///
/// 目前仅 `Plan`（只读探索）模式生效：写类工具（Write / ExecWrite / Desktop）以及
/// plan.yaml 显式 `denied_tools` 中的工具一律拒绝，使 `mode.switch` 切到 Plan 后
/// **真实**拒绝修改类动作，而非仅注入提示段。与 [`ModeManager::gate`] 的 Plan 分支同口径。
pub fn runtime_denies_tool(tool: &str) -> bool {
    mode_denies_tool(runtime_mode(), tool)
}

/// 指定模式下是否拒绝该工具（与 [`ModeManager::gate`] 的 Plan 分支同口径）。
pub fn mode_denies_tool(kind: ModeKind, tool: &str) -> bool {
    matches!(kind, ModeKind::Plan) && (is_write_class(tool) || plan_denied_tools().contains(tool))
}

/// plan.yaml 显式 `denied_tools` 集合（惰性缓存一次，避免热路径重复解析 YAML）。
fn plan_denied_tools() -> &'static std::collections::HashSet<String> {
    static SET: OnceLock<std::collections::HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| {
        crate::mode::spec::builtin::plan()
            .denied_tools
            .into_iter()
            .collect()
    })
}

/// 工具是否为写类（含写命令执行 / 桌面控制），与门控矩阵 Plan 分支一致。
fn is_write_class(tool: &str) -> bool {
    matches!(
        classify_tool(tool),
        ToolClass::Write | ToolClass::ExecWrite | ToolClass::Desktop
    )
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
    /// 各模式规格的权限预设来源（内置 = Default；目录覆盖/显式选择 = Selection；
    /// 对齐上游 PR #2608 permission/preset origin 溯源）
    spec_origins: HashMap<ModeKind, ApprovalOrigin>,
    gate: Arc<ApprovalGate>,
    /// 模式变更回调（事件总线接入点，A4 事件流上线后替换）
    mode_change_cb: Option<Arc<dyn Fn(ModeChange) + Send + Sync>>,
    /// 事件总线（A4 事件流埋点；None 时无事件发布）
    events: Option<Arc<EventBus>>,
}

impl ModeManager {
    /// 使用内置规格创建（默认 agent 模式）
    ///
    /// 规格一律经 [`spec::builtin::for_kind`] 派生 —— 它是「模式 → 内置规格」的
    /// 单一事实源。历史上这里曾逐个硬编码，把 `Ensemble` 写成 `bloom()`，与
    /// `for_kind` 的 `ensemble()` 分叉，造成「Ensemble 与 Bloom 参数完全一致」的
    /// 假象。改为派生后，任何新模式的规格都不会在此处漏配或配错。
    pub fn new() -> Self {
        let specs: HashMap<ModeKind, ModeSpec> = ModeKind::ALL
            .iter()
            .copied()
            .map(|k| (k, spec::builtin::for_kind(k)))
            .collect();
        let spec_origins = ModeKind::ALL
            .iter()
            .copied()
            .map(|k| (k, ApprovalOrigin::Default))
            .collect();
        Self {
            current: ModeKind::Agent,
            specs,
            spec_origins,
            gate: Arc::new(ApprovalGate::default()),
            mode_change_cb: None,
            events: None,
        }
    }

    /// 从目录加载覆盖规格（`~/.marguerite/modes/*.yaml`）
    ///
    /// 目录不存在或文件缺失时回退内置规格。覆盖成功的模式其权限预设来源
    /// 标记为 `Selection`（用户显式提供）。
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
            ModeKind::OmniTranscend,
        ] {
            let path = dir.join(format!("{}.yaml", kind.as_str()));
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(spec) = ModeSpec::from_yaml(&content) {
                    mgr.specs.insert(kind, spec);
                    mgr.spec_origins.insert(kind, ApprovalOrigin::Selection);
                }
            }
        }
        // 兼容过渡期：legacy yolo.yaml 覆盖规格映射到 Bloom
        let legacy = dir.join("yolo.yaml");
        if let Ok(content) = std::fs::read_to_string(&legacy) {
            if let Ok(spec) = ModeSpec::from_yaml(&content) {
                mgr.specs.insert(ModeKind::Bloom, spec);
                mgr.spec_origins
                    .insert(ModeKind::Bloom, ApprovalOrigin::Selection);
            }
        }
        mgr
    }

    /// 某模式规格的权限预设来源（default / selection / inferred）。
    pub fn spec_origin(&self, mode: ModeKind) -> ApprovalOrigin {
        self.spec_origins
            .get(&mode)
            .copied()
            .unwrap_or(ApprovalOrigin::Default)
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
        // A4 事件流埋点：发布 mode.changed + 权限预设溯源（若已注册事件总线）。
        // origin 取**新模式**规格的来源（Selection = 用户显式选择/目录覆盖）。
        if let Some(bus) = &self.events {
            bus.publish(Envelope::mode_changed_with_origin(
                "core",
                change.from.as_str(),
                self.current.as_str(),
                self.spec_origin(self.current).as_str(),
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

    /// 以 3×3 用户模式切换运行时。
    ///
    /// 单一事实源：公开入口只暴露 3×3（`crate::domain::UserMode`），内部执行档位
    /// 经 [`from_user_mode`] 派生，避免再维护一套并行的旧模式候选。
    pub fn switch_user(&mut self, mode: crate::domain::UserMode) -> Result<(), ModeSwitchError> {
        self.switch(from_user_mode(mode))
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
            ModeKind::Ensemble | ModeKind::Bloom | ModeKind::OmniTranscend => {
                self.bloom_spec().sandbox
            }
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
            ModeKind::Ensemble | ModeKind::Bloom | ModeKind::OmniTranscend => {
                // MAX+YOLO 融合语义：特权驱动审批。
                // - 系统级/桌面控制（system_control/control_desktop 等）属 Bloom/Ensemble
                //   内含能力：skip_permissions（dangerously-skip-permissions，MAX 全开）
                //   → 免确认放行；否则人工确认（尊重沙箱白名单之上的边界）。
                // - auto_approve（enable-auto-mode）+ allowed_tools（--allowedTools）
                //   决定其余工具是否免确认放行；auto_approve 关闭时退化为 agent 式审批
                let privs = self.spec(self.current).bloom.clone().unwrap_or_default();
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
///
/// ⚠️ 这里必须同时覆盖**真实注册名**（`tools/builtin` 里 `boxed("…")` /
/// `make_local_tool("…")` 的字符串）与历史**别名**。曾因只认 `exec`/`edit_file`
/// 这类别名、漏掉 `shell_exec`/`code_exec`/`str_replace_editor` 等真实名，
/// 导致它们在 Plan 模式下落到 `Read` 分支 —— Plan 声称只读，却能执行 shell、
/// 跑代码、改文件。新增工具时务必同步本表。
fn classify_tool(tool: &str) -> ToolClass {
    match tool {
        // 桌面/系统级控制（真实名 + 别名）
        "system_control" | "window_manage" | "clipboard" | "office_automation"
        | "control_desktop" | "browser_control" => ToolClass::Desktop,
        // 出网（真实名 + 别名）
        "web_fetch" | "web_search" | "web_research" | "http_request" | "research" | "vision"
        | "browser" | "browser_a11y" | "see" => ToolClass::Network,
        // 写类命令执行（真实名 + 别名）
        "shell_exec" | "code_exec" | "process_manage" | "automation" | "exec" | "terminal"
        | "install_dependency" => ToolClass::ExecWrite,
        // 只读命令执行（真实名 + 别名）
        "run_tests" | "git_status" | "git_diff" | "git_log" | "diff" | "doctor" => {
            ToolClass::ExecRead
        }
        // 写操作（真实名 + 别名）
        "write_file" | "append_file" | "edit_file" | "write_code" | "create_file"
        | "delete_file" | "rename_file" | "copy_file" | "move_file" | "make_dir"
        | "apply_patch" | "rollback" | "git_commit" | "str_replace_editor" | "todo_write"
        | "memory_save" => ToolClass::Write,
        // 护栏
        "syntax" | "semantic" | "fact" | "contract" | "guard" => ToolClass::Guard,
        // 默认只读（read_file / list_dir / grep / find_files / codebase_search /
        // count_lines / file_info / json_query / read_env / memory_list / memory_search …）
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

    /// 回归守卫：Plan 模式必须拦住**真实注册名**，而不只是历史别名。
    ///
    /// 历史缺陷：`classify_tool` 只认 `exec`/`edit_file` 等别名，漏掉了
    /// `shell_exec`/`code_exec`/`str_replace_editor`/`todo_write`/`make_dir`
    /// 等真实工具名，于是它们在 Plan 模式下落到 `Read` 分支 —— 声称只读，
    /// 却能执行 shell、跑代码、改文件。此测试钉住这条边界。
    #[test]
    fn test_m1b_plan_blocks_real_tool_names() {
        // 写/执行/桌面/出网类真实名，Plan 一律拒绝。
        for tool in [
            "shell_exec",
            "code_exec",
            "str_replace_editor",
            "write_file",
            "make_dir",
            "todo_write",
            "memory_save",
            "apply_patch",
            "system_control",
            "office_automation",
            "process_manage",
            "automation",
        ] {
            assert!(
                mode_denies_tool(ModeKind::Plan, tool),
                "Plan 模式应拒绝真实工具 `{tool}`"
            );
        }
        // 纯只读工具仍放行，避免把 Plan 变成「什么都干不了」。
        for tool in [
            "read_file",
            "list_dir",
            "grep",
            "find_files",
            "codebase_search",
            "count_lines",
            "file_info",
            "web_fetch",
        ] {
            // web_fetch 属 Network 类，Plan 不拦（只读探索允许查阅资料）。
            assert!(
                !mode_denies_tool(ModeKind::Plan, tool),
                "Plan 模式不应拒绝只读工具 `{tool}`"
            );
        }
    }

    /// 回归守卫：Ensemble 与 Bloom 的规格必须**不同**（曾在 ModeManager 里被写死成同一份）。
    #[test]
    fn test_m1c_ensemble_spec_not_bloom() {
        let mgr = ModeManager::new();
        let ens = mgr.spec(ModeKind::Ensemble);
        let bloom = mgr.spec(ModeKind::Bloom);
        assert_eq!(ens.name, "ensemble");
        assert_eq!(bloom.name, "bloom");
        let eb = ens.bloom.as_ref().expect("ensemble 应配置 bloom 块");
        assert!(
            !eb.anchoring.enabled,
            "Ensemble 是扁平强编排，不应启用锚定冷启动（与 Bloom 的实质差异）"
        );
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
    fn test_mode_kind_from_name_accepts_underscore_and_hyphen() {
        // 命名漂移修复：前端发送下划线 `omni_transcend`，规格名是连字符 `omni-transcend`。
        assert_eq!(
            ModeKind::from_name("omni_transcend"),
            Some(ModeKind::OmniTranscend)
        );
        assert_eq!(
            ModeKind::from_name("omni-transcend"),
            Some(ModeKind::OmniTranscend)
        );
        // 两者必须等价（验收 1 的核心断言）。
        assert_eq!(
            ModeKind::from_name("omni_transcend"),
            ModeKind::from_name("omni-transcend")
        );
        // 归一化：大小写 + 前后空白不敏感
        assert_eq!(
            ModeKind::from_name("  Omni_Transcend  "),
            Some(ModeKind::OmniTranscend)
        );
        assert_eq!(ModeKind::from_name("PLAN"), Some(ModeKind::Plan));
        // 全模式往返：as_str() 必须能被 from_name 解析回自身（无遗漏别名）。
        for k in ModeKind::ALL {
            assert_eq!(
                ModeKind::from_name(k.as_str()),
                Some(*k),
                "{} 往返解析失败",
                k.as_str()
            );
        }
    }

    #[test]
    fn test_runtime_denies_write_tools_in_plan_only() {
        let _g = test_runtime_mode_guard();
        let prev = runtime_mode();
        // Plan（只读）：拒绝写 / 写命令 / 桌面类工具
        set_runtime_mode(ModeKind::Plan);
        assert!(runtime_denies_tool("edit_file"));
        assert!(runtime_denies_tool("write_code"));
        assert!(runtime_denies_tool("create_file"));
        assert!(runtime_denies_tool("exec"));
        assert!(runtime_denies_tool("terminal"));
        assert!(runtime_denies_tool("control_desktop"));
        // plan.yaml 显式 denied（ExecRead 类但也拒绝）：证明与规格清单对齐而非仅按类
        assert!(runtime_denies_tool("run_tests"));
        // 只读 / 出网 / 护栏工具放行
        assert!(!runtime_denies_tool("read_file"));
        assert!(!runtime_denies_tool("web_search"));
        assert!(!runtime_denies_tool("syntax"));
        // 非 Plan 模式不拦截（回归保护：Agent/Bloom 写工具不受影响）
        set_runtime_mode(ModeKind::Agent);
        assert!(!runtime_denies_tool("edit_file"));
        set_runtime_mode(ModeKind::Bloom);
        assert!(!runtime_denies_tool("edit_file"));
        set_runtime_mode(prev);
    }

    #[test]
    fn test_mode_switch_error_display() {
        let mut mgr = ModeManager::new();
        // 从目录加载不存在 → 内置；切换无效模式不适用，直接验证 SpecNotFound 路径
        let e = ModeSwitchError::InvalidTarget(ModeKind::Plan);
        assert!(e.to_string().contains("plan"));
        let _ = &mut mgr;
    }

    /// F-07 护栏：模块头注释必须如实描述 5 档模式，不得再写「三档模式」。
    #[test]
    fn module_header_describes_all_five_modes() {
        let src = include_str!("mod.rs");
        let header: String = src.lines().take(12).collect::<Vec<_>>().join("\n");
        assert!(
            !header.contains("三档模式"),
            "模块注释不得再宣称「三档模式」: {header}"
        );
        for kind in ["plan", "agent", "ensemble", "bloom", "omni-transcend"] {
            assert!(header.contains(kind), "模块注释应覆盖模式 {kind}: {header}");
        }
        assert!(
            !header.contains("已弃用别名"),
            "bloom 是正式模式，模块注释不得称其为已弃用别名"
        );
    }

    /// D-04 护栏（配置终点）：agent.yaml 必须携带「先问再做」规则段，
    /// 且覆盖三项高影响歧义触发词 + ≤3 问 + 可跳过默认值 + 提问工具名。
    #[test]
    fn agent_mode_spec_carries_ask_first_section() {
        let mgr = ModeManager::new();
        let spec = mgr.spec(ModeKind::Agent);
        let section = spec.prompt_section.as_deref().unwrap_or("");
        assert!(
            !section.trim().is_empty(),
            "agent.yaml 必须含 prompt_section（F-02：默认模式此前不消费先问再做）"
        );
        for key in ["平台", "技术栈", "交付形态"] {
            assert!(
                section.contains(key),
                "规则段须覆盖高影响歧义触发词 {key}: {section}"
            );
        }
        assert!(section.contains("ask_user_question"), "须指明提问工具名");
        assert!(section.contains("3 问"), "须含「最多 3 问」约束");
        assert!(
            section.contains("跳过") && section.contains("默认值"),
            "每问须带可跳过的默认值"
        );
        assert!(
            section.contains("不触发提问") || section.contains("不追问"),
            "须声明明确单轮指令不触发提问"
        );
    }

    /// D-04 行为终点（真实解析链）：运行时模式为 Agent 时，
    /// `execute_turn` 走的 `ModeManager::spec(runtime_mode()).prompt_section`
    /// 必须解析出非空的「先问再做」段（否则规则形同虚设）。
    #[test]
    fn runtime_agent_mode_resolves_injected_ask_first_section() {
        let prev = crate::mode::runtime_mode();
        set_runtime_mode(ModeKind::Agent);
        let resolved = ModeManager::new()
            .spec(crate::mode::runtime_mode())
            .prompt_section
            .clone();
        set_runtime_mode(prev);
        let section = resolved.unwrap_or_default();
        assert!(
            section.contains("先问再做"),
            "Agent 运行时模式必须解析出「先问再做」规则段，实际: {section}"
        );
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
        // 权限预设溯源（上游 PR #2608）：内置规格 → default
        assert_eq!(replayed[0].payload["permission_origin"], "default");
    }

    #[test]
    fn test_spec_origin_builtin_default_and_dir_override_selection() {
        let mgr = ModeManager::new();
        assert_eq!(mgr.spec_origin(ModeKind::Agent), ApprovalOrigin::Default);
        assert_eq!(mgr.spec_origin(ModeKind::Bloom), ApprovalOrigin::Default);

        // 目录覆盖 → Selection
        let dir = std::env::temp_dir().join(format!("mgt_modes_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("bloom.yaml"),
            "name: bloom\nbloom:\n  privileges:\n    auto_approve: true\n",
        )
        .unwrap();
        let overridden = ModeManager::from_dir(&dir);
        assert_eq!(
            overridden.spec_origin(ModeKind::Bloom),
            ApprovalOrigin::Selection,
            "目录覆盖的规格来源应为 selection"
        );
        // 未覆盖的模式仍是默认
        assert_eq!(
            overridden.spec_origin(ModeKind::Plan),
            ApprovalOrigin::Default
        );

        // 切换事件携带 selection 溯源
        let bus = Arc::new(EventBus::default());
        let mut overridden = overridden;
        overridden.set_event_bus(bus.clone());
        let since = Instant::now();
        overridden.switch(ModeKind::Bloom).unwrap();
        let replayed = bus.history(since);
        assert_eq!(replayed[0].payload["permission_origin"], "selection");
    }

    #[test]
    fn test_approval_origin_as_str_wire_values() {
        assert_eq!(ApprovalOrigin::Default.as_str(), "default");
        assert_eq!(ApprovalOrigin::Selection.as_str(), "selection");
        assert_eq!(ApprovalOrigin::Inferred.as_str(), "inferred");
    }
}
