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

pub mod _helpers;
pub mod a2a;
pub mod approvals;
pub mod artifacts;
pub mod auth;
pub mod baota;
pub mod cache;
pub mod capability_scan;
pub mod chat;
pub mod collab;
pub mod config;
pub mod content_moderation;
pub mod context_refs;
pub mod desktop;
pub mod devices;
pub mod escalation_rpc;
pub mod events;
pub mod fleet;
pub mod goal;
pub mod harness_rpc;
pub mod kb;
pub mod llm_profile;
pub mod local_models;
pub mod long_running;
pub mod checkpoint_rpc;
pub mod queue_rpc;
pub mod trust_rpc;
#[cfg(test)]
mod loopback;
pub mod plugins_mcp_rpc;
pub mod plugins_rpc;
pub mod plugins_runtime;
pub mod market;
pub mod market_remote;
pub mod mcp;
pub mod mcp_router;
pub mod memory;
pub mod memory_import;
pub mod methods;
pub mod pet;
pub mod plugin_status;
pub mod mode;
pub mod mode_route;
pub mod models;
pub mod native;
pub mod onboarding;
pub mod open_path;
pub mod ops_dispatch;
pub mod orchestrate;
pub mod protocol;
pub mod rate_limit;
pub mod router;
pub mod sandbox;
pub mod sandbox_paths;
pub mod security_seam;
pub mod selfevolve;
pub mod server;
pub mod sessions;
pub mod sessions_insights;
pub mod shipped_gate;
pub mod sidecar_proxy;
pub mod session_changes;
pub mod sessions_portability;
pub mod skills;
pub mod system;
pub mod task_receipt;
pub mod team;
pub mod tools;
pub mod update;
pub mod vision;
pub mod workspace;
pub mod workspaces;

pub use server::{run, GatewayState};

/// 网关默认端口（与 Marguerite 网关一致，仅回环）。
///
/// **单一事实源 / 编译期常量不变量**：桌面端不做运行时端口参数化
/// （CLI `--port` 仍保留给 ad-hoc 实例）。改动此值必须同步：
/// - `gui/src-tauri/tauri.conf.json` 的 `app.windows[0].url`（由 `gui/src-tauri/build.rs`
///   的端口一致性校验在构建期强制，不一致即 panic）；
/// - `webui/vite.config.ts` 的 `CONVENTIONAL_GATEWAY_PORT`（dev 代理）；
/// - `scripts/verify_first_run.ps1` 的 `-Port`；
/// - `.tmp/` 下的探针脚本（未跟踪的 ad-hoc QA 层，全为字面量 18790）。
pub const DEFAULT_PORT: u16 = 18_790;
