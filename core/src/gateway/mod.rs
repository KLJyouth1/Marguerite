//! 统一网关（Phase 2：Rust 统一网关）
//!
//! 在 marguerite-core 内扩展出回环 HTTP/WebSocket 网关，实现 Marguerite
//! RPC 协议（PROTOCOL_VERSION 3，hello/req/res/event/ping/tick + seq 断连
//! 重连语义），同源托管 Web UI 静态产物并注入 `#Marguerite-data` 元数据。
//!
//! - 仅监听 `127.0.0.1`（延续 ACP「仅回环」纪律，不变量 #2）。
//! - 认证：本地 token（`~/.marguerite/gateway-token`，connect 握手校验）。
//! - 事件推送：订阅 [`EventBus`] `**`，映射为 `session.event.*` 事件帧。
//! - 方法路由：见 [`methods::dispatch`]（Phase 2 首批 = 状态/只读方法族）。

pub mod auth;
pub mod chat;
pub mod events;
pub mod memory_import;
pub mod methods;
pub mod protocol;
pub mod server;
pub mod workspaces;

pub use server::{run, GatewayState};

/// 网关默认端口（与 Marguerite 网关一致，仅回环）
pub const DEFAULT_PORT: u16 = 18_790;
