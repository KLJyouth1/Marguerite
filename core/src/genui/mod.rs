//! 生成式 UI 交互契约（Track C，对齐上游 `omdsh-dev/dsh-genui`）。
//!
//! 把「模型产出可交互界面」这件事收敛为**版本化、强类型、可校验**的 Rust 原生契约：
//! - [`widget`]：`Widget` 枚举（Layout / Chart / Form / Quiz / Mermaid / Scene3D），
//!   每个变体持有具名结构体而非裸 `serde_json::Value`，字段错配在反序列化即暴露；
//! - [`contract`]：[`GenUiDocument`] 顶层版本化文档 + JSON 编解码；
//! - [`action`]：action event loop —— 界面交互事件经 [`ActionSink`] 回传模型；
//! - [`validate`]：沙箱校验（防 XSS），拒绝模型手写的 `<script>` / `<iframe>` /
//!   事件处理器属性 / `javascript:` 伪协议。
//!
//! 安全纪律（P0）：上游 dsh-genui 的已知缺口是「模型手写 HTML/JS = XSS 面」。
//! 本模块的对策是**结构化优先**——渲染层只消费类型化 widget，且任何进入渲染层的
//! 文本必须先过 [`validate`]。渲染前未过 `validate` 的文档视为不可信。

pub mod action;
pub mod contract;
pub mod validate;
pub mod widget;

pub use action::{route_action, ActionEvent, ActionSink, VecActionSink};
pub use contract::GenUiDocument;
pub use validate::validate;
pub use widget::{
    Block, ChartWidget, FormField, FormWidget, LayoutWidget, MermaidWidget, QuizWidget,
    Scene3DWidget, Series, Widget,
};

/// 当前生成式 UI 契约版本。
///
/// 消费方遇到 `version` 高于此常量的文档必须拒绝（而非尽力解析），
/// 避免新语义被旧引擎静默降级误渲染。
pub const GENUI_SCHEMA_VERSION: u32 = 1;

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
