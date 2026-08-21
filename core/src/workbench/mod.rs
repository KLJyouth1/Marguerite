//! 工作台领域/数据层（Track D，对齐上游 `DSH-better-sidebar`）。
//!
//! 这是协同链路里「web 任务/状态跟踪」的 Rust 原生数据主干，与 `tui::workbench`
//! （TUI 视图层）职责分离：本模块只产出可被消费的类型与注册表，不做任何渲染。
//!
//! 职责边界（P0 视觉纪律全程生效：禁 emoji 图标、禁紫粉渐变、禁硬编码色）：
//! - [`panel`]：`PanelRegistry` 面板注册表（37 内置槽位 + ComingSoon 占位 + 第三方 tab）。
//! - [`agent`]：`AgentMeta` 单一真相源（收敛 `agent_catalog.json` 88 条）。
//! - [`theme`]：主题 token 引用表（皮肤切到 token 名，绝不写死 hex）。
//! - [`usage`]：用量仪表（面板打开 / Agent 调用聚合）。
//! - [`pet`]：鲸鱼娘桌宠原生状态机（无渲染）。
//!
//! 安全纪律：渲染层（Vue/TUI）须消费本 domain 暴露的类型，且任何进入渲染层的文本
//! 必须先过 `genui::validate` 类闸门防注入。本模块本身只产出纯数据。

pub mod agent;
pub mod panel;
pub mod pet;
/// PetV2 6 帧状态机（T-2.3 / FR-B3 增强版：affinity/pets/feeds + DshAgentEvent + ASCII 精灵）。
pub mod pet_v2;
pub mod theme;
pub mod usage;

pub use agent::{catalog_by_id, load_catalog, load_catalog_path, AgentMeta};
pub use panel::{default_panels, PanelDef, PanelKind, PanelRegistry};
pub use pet::{feed, pet, tick, PetMood, PetState};
pub use theme::{tokens_for, Skin, ThemeTokens, TokenRef};
pub use usage::UsageStats;

/// 工作台领域层错误（解析 / 目录加载）。
#[derive(Debug, thiserror::Error)]
pub enum WorkbenchError {
    /// `agent_catalog.json` 反序列化失败（字段错配在反序列化即暴露，非裸 `Value`）。
    #[error("AgentMeta 目录解析失败: {0}")]
    CatalogJson(#[from] serde_json::Error),
    /// 目录文件读取失败（IO，如 `load_catalog_path` 打开文件）。
    #[error("AgentMeta 目录读取失败: {0}")]
    Io(#[from] std::io::Error),
}
