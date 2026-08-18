//! 事件流模块（v2 工作台）
//!
//! 结构化事件信封（版本化 + SSE 序列化）+ 主题事件总线 + 历史回放。
//! 三形态（TUI / GUI / CLI）消费同一事件流：
//! - TUI：主循环 `bus.subscribe("ws.<session>.*")` → 事件分派到右/中栏组件
//! - GUI：core 暴露 HTTP SSE `/events/stream?session=` → 前端 `consumeSSEStream`
//! - CLI：脚本模式打印 `[type] summary` 到 stderr；`--json` 输出原始 Envelope
//!
//! 路由键（route_key）= `{session}.{type}`，订阅方用 `.` 分段的主题模式匹配，
//! 支持单段通配 `*` 与多段通配 `**`（见 [`bus::EventBus::match_topic`]）。

pub mod bus;
pub mod envelope;

pub use bus::EventBus;
pub use envelope::{EnsembleProgress, Envelope, EventError, EventType, CURRENT_EVENT_VERSION};

use std::sync::{Arc, OnceLock};

/// 网关事件总线全局句柄（网关启动时由 [`crate::gateway::server::GatewayState::new`]
/// 注册一次）。供 `usage::record` 等「无总线上下文」的模块向已连接的 WS 客户端发布
/// 事件（如 `session.event.usage`）。未启动网关（CLI / 测试）时为 `None`，调用方应 no-op。
static GATEWAY_BUS: OnceLock<Arc<EventBus>> = OnceLock::new();

/// 注册网关事件总线（幂等；重复设置被忽略）。
pub fn set_global_bus(bus: Arc<EventBus>) {
    let _ = GATEWAY_BUS.set(bus);
}

/// 取网关事件总线；未注册时返回 `None`。
pub fn global_bus() -> Option<Arc<EventBus>> {
    GATEWAY_BUS.get().cloned()
}

/// 事件主题命名空间（埋点预留，供 mode / chain / sandbox 发布）
///
/// 订阅方常用模式：`ws.<session>.*`（单会话全事件）、`agent.*`（agent 模式全事件）、
/// `**`（全部事件）、`ws.**`（所有会话全事件）。
pub mod topics {
    /// 模式切换事件 payload 的 kind 标识
    pub const MODE_CHANGED: &str = "mode.changed";
    /// 工具调用
    pub const TOOL_CALL: &str = "tool.call";
    /// 工具结果
    pub const TOOL_RESULT: &str = "tool.result";
    /// 调用链完成
    pub const CHAIN_DONE: &str = "chain.done";
    /// 审批请求
    pub const APPROVAL: &str = "approval.request";
    /// 进度播报
    pub const PROGRESS: &str = "progress";
}
