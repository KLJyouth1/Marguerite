//! 生成式 UI 交互契约（Track C，对齐上游 `omdsh-dev/dsh-genui`） + DSH GenUI 二创
//!
//! 本模块分两层：
//! 1. **Track C / C2** 原有的版本化文档（`GenUiDocument` + Layout/Chart/... 强类型）
//!    见 [`widget`] / [`validate`] / [`action`]。
//! 2. **FR-B2 / T-2.2 DSH GenUI 二创的轻量交互卡片**：Widget/Card/Button/
//!    ProgressBar/Form 枚举，见 [`contract`]；对应 Ratatui 渲染见 [`render`]。
//!
//! 安全纪律（P0）：任何进入渲染层的文本必须先过 [`validate`]。未 validate 的文档
//! 视为不可信。

use serde::{Deserialize, Serialize};

// ── 原有子模块（Track C） ──
pub mod action;
pub mod validate;
pub mod widget;

// ── T-2.2 DSH GenUI 二创模块（按任务要求保留 + 新增） ──
pub mod contract;
pub mod render;

// ── 重新导出：DSH GenUI 二创在顶层可见 ──
// （任务要求：必须存在 pub use contract::*）
pub use contract::*;
// render 顶层常用工具函数导出
pub use render::{click_dispatch, render_button_hover, render_widget, SteeringEvent};

// ── Track C 原有导出（无冲突部分保留） ──
pub use action::{route_action, ActionEvent, ActionSink, VecActionSink};
pub use validate::validate;
// 注意：`widget::Widget`、`widget::FormField` 与 `contract::Widget/FormField` 同名，
// 此处不 re-export widget 枚举，需要使用时显式写 `widget::Widget` 前缀。
pub use widget::{
    Block, ChartWidget, FormWidget, LayoutWidget, MermaidWidget, QuizWidget, Scene3DWidget, Series,
};

/// 当前生成式 UI 契约版本（Track C2 顶层文档 schema）。
pub const GENUI_SCHEMA_VERSION: u32 = 1;

/// 版本化 GenUI 文档：一组 `widget::Widget` + schema version。
///
/// validate.rs 消费此结构；未定义会导致 `use super::GenUiDocument` 失败。
/// 这里与 Track C2 注释对齐、避免同名冲突（不 re-export widget::Widget 顶层），
/// 文档使用时显式写 `widget::Widget` 枚举而非 contract::Widget。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenUiDocument {
    /// schema version（validate 时若超过 [`GENUI_SCHEMA_VERSION`] 即拒）。
    pub version: u32,
    /// 按绘制顺序的 widget 列表（Layout 首元素通常作根）。
    pub widgets: Vec<crate::genui::widget::Widget>,
}

impl GenUiDocument {
    /// 便捷构造：写入当前引擎支持的 schema 版本。
    pub fn new(widgets: Vec<crate::genui::widget::Widget>) -> Self {
        Self {
            version: GENUI_SCHEMA_VERSION,
            widgets,
        }
    }
}

/// 生成式 UI 契约错误（解析 / 版本 / 沙箱校验）。
#[derive(Debug, thiserror::Error)]
pub enum GenUiError {
    /// 文档版本高于本引擎支持的契约版本 —— 拒绝渲染，不做降级猜测。
    #[error("生成式 UI 文档版本 {found} 超出引擎支持版本 {supported}")]
    UnsupportedVersion {
        /// 文档声明的版本
        found: u32,
        /// 引擎支持的最高版本（[`GENUI_SCHEMA_VERSION`]）
        supported: u32,
    },
    /// 沙箱校验拒绝：字段内容含脚本注入面。
    #[error("沙箱校验拒绝 widget `{widget_id}` 的字段 `{field}`：{reason}")]
    Unsafe {
        /// 命中的 widget id
        widget_id: String,
        /// 命中的字段名
        field: String,
        /// 拒绝原因（人可读，可直接进决策日志）
        reason: String,
    },
    /// JSON 反序列化失败（含未知 widget 类型 —— 返回错误而非 panic）。
    #[error("生成式 UI JSON 解析失败: {0}")]
    Json(#[from] serde_json::Error),
}
