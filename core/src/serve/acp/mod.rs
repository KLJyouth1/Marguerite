//! ACP（Agent Client Protocol）薄壳。
//!
//! 设计对齐 nexustar-api `app/acp` 的差距②薄壳方案：
//! - 协议层（`protocol`）只做 JSON-RPC 2.0 解析/构造，不涉及网络/LLM/agent 调用。
//! - 会话层（`session`）把方法路由到注入的 `AgentInvoker`，真实调用由上层 sidecar 提供。
//! - 回环优先：ACP 仅经本地 sidecar 暴露，禁止突破 127.0.0.1（由上层网络层保证）。

pub mod listener;
pub mod protocol;
pub mod session;

pub use listener::{start_listener, ServeHandle};
pub use protocol::{
    make_error, make_result, parse, parse_bytes, parse_value, AcpError, AcpRequest, INTERNAL_ERROR,
    INVALID_PARAMS, INVALID_REQUEST, JSONRPC_VERSION, METHOD_NOT_FOUND, PARSE_ERROR,
};
pub use session::{AcpSession, AgentInvoker};
