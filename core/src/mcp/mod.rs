//! MCP (Model Context Protocol) 模块
//!
//! 提供 MCP 客户端实现，用于连接 MCP Server 发现和调用工具。
//! 包含工具注册表，管理本地工具和 MCP 工具的注册与发现。

pub mod client;
pub mod registry;
pub mod server;

pub use client::{McpClient, McpContent, McpTool, McpToolResult};
pub use registry::{ToolExecutor, ToolRegistry};
pub use server::McpServer;
