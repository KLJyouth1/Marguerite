//! 服务态与协议层（E 期 serve/）。
//!
//! 提供 Agent Client Protocol（ACP）薄壳：JSON-RPC 2.0 信封 + 会话路由，
//! 用于把本地 agent 能力以标准化协议暴露给桌面 sidecar（仅回环 127.0.0.1）。

pub mod acp;
