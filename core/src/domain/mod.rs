//! Marguerite 统一领域模型（Single Source of Truth）
//!
//! 取代 nexustar-api (Python) 与 nexustar-ui (TypeScript) 中平行的领域定义。
//! CLI、TUI（Ratatui）、原生桌面 GUI（egui）三前端共用此处的类型，
//! 从根上消除 Rust / Python / TS 三套实现的类型漂移。

use serde::{Deserialize, Serialize};
use std::fmt;

pub mod capabilities;

/// 对话角色。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    System,
    Tool,
    Orchestrator,
}

/// Agent 编排形态。
///
/// `Solo` 等价于 标准 AI 助手 的单 Agent 编码循环（基线）；
/// 其余三种为 Marguerite 的超越项——也是「比它们强数倍」的核心来源。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentMode {
    /// 单 Agent 编码循环（标准 AI 助手 基线等价）
    Solo,
    /// MAX 双带队：Flash 规划 + Pro 执行，原生的规划/执行分离
    DualLeader,
    /// 专家团队：角色专精的子 Agent，统一走 spawn 模型
    ExpertTeam,
    /// 集群：分配器调度的多 Worker 并行
    Cluster,
    /// 交付流水线：一句话含多个交付物（检索 → 网页 → PPT …）时，按契约 DAG 逐步产出产物。
    /// 这是复合任务的**可观测编排形态**（对应 `orchestration::deliverable` 检测结果）。
    Pipeline,
}

impl AgentMode {
    pub fn all() -> &'static [AgentMode] {
        &[
            AgentMode::Solo,
            AgentMode::DualLeader,
            AgentMode::ExpertTeam,
            AgentMode::Cluster,
            AgentMode::Pipeline,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            AgentMode::Solo => "Solo Agent",
            AgentMode::DualLeader => "MAX Dual-Leader",
            AgentMode::ExpertTeam => "Expert Team",
            AgentMode::Cluster => "Cluster",
            AgentMode::Pipeline => "Delivery Pipeline",
        }
    }

    /// 稳定机器标识（与 `#[serde(rename_all = "kebab-case")]` 逐字一致）。
    ///
    /// A5：计划 payload 只下发这个机器键，**展示名由前端按 i18n 渲染**——
    /// 引擎不得把中文标签硬编码进 Markdown 正文（会让 en/de/fr/es/ja 界面冒中文）。
    pub fn as_key(&self) -> &'static str {
        match self {
            AgentMode::Solo => "solo",
            AgentMode::DualLeader => "dual-leader",
            AgentMode::ExpertTeam => "expert-team",
            AgentMode::Cluster => "cluster",
            AgentMode::Pipeline => "pipeline",
        }
    }

    /// 是否超出 标准 AI 助手 基线能力。
    pub fn exceeds_baseline(&self) -> bool {
        !matches!(self, AgentMode::Solo)
    }

    /// 由「3×3 用户模式」安全派生编排形态（拓扑）。
    ///
    /// 设计约束（见 [`UserMode`]）：用户只选 强度 × 工作模式 的组合，
    /// 编排形态是**派生结果**而非用户直选。映射原则——
    /// - 咨询（Ask）：永远单 Agent 顾问式（Solo），不执行；
    /// - 方案（Plan）：默认只读规划（Solo），仅「极致·方案」拉起专家团队共绘；
    /// - 执行（DoIt）：随强度升级 Solo→DualLeader→ExpertTeam→Cluster。
    pub fn derive(mode: UserMode) -> AgentMode {
        match mode {
            UserMode::MinimalAsk | UserMode::StandardAsk | UserMode::ExtremeAsk => AgentMode::Solo,
            UserMode::MinimalPlan | UserMode::StandardPlan => AgentMode::Solo,
            UserMode::ExtremePlan => AgentMode::ExpertTeam,
            UserMode::MinimalDoIt => AgentMode::DualLeader,
            UserMode::StandardDoIt => AgentMode::ExpertTeam,
            UserMode::ExtremeDoIt => AgentMode::Cluster,
        }
    }
}

/// 强度档位（3×3 用户模式的第一维）。
///
/// 取代历史上 bloom 等模式自带的「强度」隐含语义——现在强度是显式、可枚举的第一轴。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModeIntensity {
    /// 极简：一句话结论 + 关键建议，最少产出
    Minimal,
    /// 标准：结构化分析，含依据与边界
    Standard,
    /// 极致：深度根因、多视角交叉、全工具面
    Extreme,
}

impl ModeIntensity {
    pub fn all() -> &'static [ModeIntensity] {
        &[ModeIntensity::Minimal, ModeIntensity::Standard, ModeIntensity::Extreme]
    }
    pub fn as_key(&self) -> &'static str {
        match self {
            ModeIntensity::Minimal => "minimal",
            ModeIntensity::Standard => "standard",
            ModeIntensity::Extreme => "extreme",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            ModeIntensity::Minimal => "极简",
            ModeIntensity::Standard => "标准",
            ModeIntensity::Extreme => "极致",
        }
    }
}

/// 工作模式（3×3 用户模式的第二维）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkingMode {
    /// 咨询：仅给出建议，不执行
    Ask,
    /// 方案：输出分步计划与里程碑
    Plan,
    /// 执行：带校验地实际执行（结果可追溯）
    DoIt,
}

impl WorkingMode {
    pub fn all() -> &'static [WorkingMode] {
        &[WorkingMode::Ask, WorkingMode::Plan, WorkingMode::DoIt]
    }
    pub fn as_key(&self) -> &'static str {
        match self {
            WorkingMode::Ask => "ask",
            WorkingMode::Plan => "plan",
            WorkingMode::DoIt => "doit",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            WorkingMode::Ask => "咨询",
            WorkingMode::Plan => "方案",
            WorkingMode::DoIt => "执行",
        }
    }
}

/// 唯一的合法用户模式：强度 × 工作模式 = 9 种组合。
///
/// 取代历史上散乱的旧模式（bloom / research / automation / safe / work 及其变体）。
/// 旧模式的能力被**重新规划升级**进这三个强度档位：
/// - 旧 `plan`（只读探索）→ **Ask**
/// - 旧 `agent`（交互审批）→ **Plan**
/// - 旧 `bloom` / `ensemble` / `omni-transcend`（强编排执行）→ **DoIt**（强度越高越激进）
///
/// 编排形态 [`AgentMode`] 与运行时执行模式 [`crate::mode::ModeKind`] 均由本枚举**派生**，
/// 不再由用户直选，消除「同一意图多种模式名」的漂移。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserMode {
    MinimalAsk, MinimalPlan, MinimalDoIt,
    StandardAsk, StandardPlan, StandardDoIt,
    ExtremeAsk, ExtremePlan, ExtremeDoIt,
}

impl UserMode {
    /// 九种组合的规范顺序（强度优先）。
    pub fn all() -> &'static [UserMode] {
        &[
            UserMode::MinimalAsk, UserMode::MinimalPlan, UserMode::MinimalDoIt,
            UserMode::StandardAsk, UserMode::StandardPlan, UserMode::StandardDoIt,
            UserMode::ExtremeAsk, UserMode::ExtremePlan, UserMode::ExtremeDoIt,
        ]
    }

    /// 稳定机器标识（与 `#[serde(rename_all = "snake_case")]` 逐字一致），如 `minimal_ask`。
    pub fn as_key(&self) -> &'static str {
        match self {
            UserMode::MinimalAsk => "minimal_ask",
            UserMode::MinimalPlan => "minimal_plan",
            UserMode::MinimalDoIt => "minimal_doit",
            UserMode::StandardAsk => "standard_ask",
            UserMode::StandardPlan => "standard_plan",
            UserMode::StandardDoIt => "standard_doit",
            UserMode::ExtremeAsk => "extreme_ask",
            UserMode::ExtremePlan => "extreme_plan",
            UserMode::ExtremeDoIt => "extreme_doit",
        }
    }

    /// 从名称解析（大小写不敏感，下划线与连字符等价，对齐前端 `mode.switch` 历史漂移修复）。
    pub fn from_key(s: &str) -> Option<Self> {
        let norm = s.trim().to_ascii_lowercase().replace('-', "_");
        match norm.as_str() {
            "minimal_ask" => Some(UserMode::MinimalAsk),
            "minimal_plan" => Some(UserMode::MinimalPlan),
            "minimal_doit" => Some(UserMode::MinimalDoIt),
            "standard_ask" => Some(UserMode::StandardAsk),
            "standard_plan" => Some(UserMode::StandardPlan),
            "standard_doit" => Some(UserMode::StandardDoIt),
            "extreme_ask" => Some(UserMode::ExtremeAsk),
            "extreme_plan" => Some(UserMode::ExtremePlan),
            "extreme_doit" => Some(UserMode::ExtremeDoIt),
            _ => None,
        }
    }

    pub fn intensity(&self) -> ModeIntensity {
        match self {
            UserMode::MinimalAsk | UserMode::MinimalPlan | UserMode::MinimalDoIt => ModeIntensity::Minimal,
            UserMode::StandardAsk | UserMode::StandardPlan | UserMode::StandardDoIt => ModeIntensity::Standard,
            UserMode::ExtremeAsk | UserMode::ExtremePlan | UserMode::ExtremeDoIt => ModeIntensity::Extreme,
        }
    }

    pub fn working(&self) -> WorkingMode {
        match self {
            UserMode::MinimalAsk | UserMode::StandardAsk | UserMode::ExtremeAsk => WorkingMode::Ask,
            UserMode::MinimalPlan | UserMode::StandardPlan | UserMode::ExtremePlan => WorkingMode::Plan,
            UserMode::MinimalDoIt | UserMode::StandardDoIt | UserMode::ExtremeDoIt => WorkingMode::DoIt,
        }
    }

    /// 人类可读标签，如「极简·咨询」。
    pub fn label(&self) -> String {
        format!("{}·{}", self.intensity().label(), self.working().label())
    }

    /// 派生的编排形态（拓扑）。
    pub fn agent_mode(&self) -> AgentMode {
        AgentMode::derive(*self)
    }
}

#[cfg(test)]
mod user_mode_tests {
    use super::*;

    #[test]
    fn user_mode_is_nine_canonical_combos() {
        assert_eq!(UserMode::all().len(), 9);
        for m in UserMode::all() {
            assert!(UserMode::from_key(m.as_key()) == Some(*m), "{} 往返失败", m.as_key());
            assert_eq!(*m, UserMode::from_key(m.as_key()).unwrap());
        }
    }

    #[test]
    fn user_mode_key_accepts_hyphen_and_case() {
        assert_eq!(UserMode::from_key("minimal-ask"), Some(UserMode::MinimalAsk));
        assert_eq!(UserMode::from_key("EXTREME_DOIT"), Some(UserMode::ExtremeDoIt));
        assert_eq!(UserMode::from_key("bogus"), None);
    }

    #[test]
    fn intensity_and_working_axes() {
        assert_eq!(UserMode::StandardPlan.intensity(), ModeIntensity::Standard);
        assert_eq!(UserMode::StandardPlan.working(), WorkingMode::Plan);
        assert_eq!(UserMode::ExtremeDoIt.working(), WorkingMode::DoIt);
        assert_eq!(UserMode::MinimalAsk.label(), "极简·咨询");
    }

    #[test]
    fn derive_escalates_with_intensity() {
        // Ask is always advisory Solo.
        assert_eq!(AgentMode::derive(UserMode::ExtremeAsk), AgentMode::Solo);
        // DoIt escalates by intensity.
        assert_eq!(AgentMode::derive(UserMode::MinimalDoIt), AgentMode::DualLeader);
        assert_eq!(AgentMode::derive(UserMode::StandardDoIt), AgentMode::ExpertTeam);
        assert_eq!(AgentMode::derive(UserMode::ExtremeDoIt), AgentMode::Cluster);
        // Extreme Plan pulls in the expert team; lesser plans stay Solo.
        assert_eq!(AgentMode::derive(UserMode::ExtremePlan), AgentMode::ExpertTeam);
        assert_eq!(AgentMode::derive(UserMode::StandardPlan), AgentMode::Solo);
    }

    #[test]
    fn legacy_mode_capabilities_remapped() {
        // 旧 plan → Ask, 旧 agent → Plan, 旧 bloom/ensemble/omni → DoIt
        assert_eq!(UserMode::MinimalAsk.working(), WorkingMode::Ask);
        assert_eq!(UserMode::StandardPlan.working(), WorkingMode::Plan);
        assert_eq!(UserMode::StandardDoIt.intensity(), ModeIntensity::Standard);
    }
}

/// 记忆层级（5 层，含自进化闭环）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTier {
    Working,
    Episodic,
    Semantic,
    Project,
    /// 自进化闭环层：从踩坑中沉淀、验证、升格为决策记录
    Evolutionary,
}

impl MemoryTier {
    pub fn all() -> &'static [MemoryTier] {
        &[
            MemoryTier::Working,
            MemoryTier::Episodic,
            MemoryTier::Semantic,
            MemoryTier::Project,
            MemoryTier::Evolutionary,
        ]
    }
}

/// 模型后端种类：云端 API 或本地量化模型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Cloud,
    Local,
}

/// 强类型 ID 包装，避免字符串在系统中乱飞。
macro_rules! id_newtype {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                $name(s)
            }
        }
    };
}

id_newtype!(SessionId);
id_newtype!(AgentId);
id_newtype!(ToolId);
id_newtype!(ModelId);
id_newtype!(CapabilityId);
id_newtype!(TaskId);

/// 一条对话消息。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// 可选的结构化载荷（工具调用结果、编排快照等）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Message {
            role: Role::User,
            content: content.into(),
            meta: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Message {
            role: Role::Assistant,
            content: content.into(),
            meta: None,
        }
    }
}

/// 一次会话：可承载任意编排形态与多模型。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub mode: AgentMode,
    pub title: String,
    #[serde(default)]
    pub messages: Vec<Message>,
}
