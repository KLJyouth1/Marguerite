//! WP2 MCP·A2A 双向协议底座 —— MCP client 双向原语。
//!
//! 本模块只定义**抽象边界**：
//! - [`McpTransport`] trait 把「连某个 MCP server」这件事完全抽走；
//! - [`McpEndpoint`] 是一个已连接 endpoint 的句柄（`name` + `transport`）；
//! - [`ToolSpec`] 是单个工具的静态描述，危险等级**复用** [`crate::devices::HazardLevel`]
//!   （L0–L3 语义与 WP1 设备总线一致，便于同一份人在回路闸门复用）。
//!
//! 纪律：
//! - 本 crate **不真连任何外部 MCP server**；真实 stdio / Streamable HTTP transport
//!   由后续阶段在本 trait 上接线。单测全部走 Stub transport，零出向。
//! - L2 / L3 的审批抬升**不在本模块内**做——这里只透传 hazard 元数据，
//!   由外层 PlanGate / 审批闸门（`gateway/methods.rs` 串行收口）统一决策。
//! - 不修改 `gateway/protocol.rs` 的 SUPPORTED_METHODS，也不碰 `methods.rs` 的
//!   大 dispatch match——那是 WP2 owner 的串行收口点。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::devices::HazardLevel;

/// MCP 原语层错误（transport 失败 / 工具未找到 / 参数形状非法）。
///
/// 不伪装成功：transport 错误原样带字符串透出，调用方（RPC 层）负责映射成
/// `RpcError`。
#[derive(Debug, Error)]
pub enum McpError {
    #[error("MCP transport 失败: {0}")]
    Transport(String),
    #[error("MCP 工具未找到: {0}")]
    ToolNotFound(String),
    #[error("MCP 工具参数非法: {0}")]
    InvalidArgs(String),
}

/// 单个 MCP 工具的静态描述。
///
/// `hazard` 复用 WP1 的 [`HazardLevel`]，保证「设备总线」与「MCP 工具」在同一套
/// 人在回路分级下被治理：L0 只读自动放行、L1 安全写自动执行、L2 需审批、
/// L3 物理致动 fail-closed。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolSpec {
    /// 工具名（endpoint 内唯一）。
    pub name: String,
    /// 人类可读描述。
    pub description: String,
    /// JSON Schema 子集；`None` 表示无参数。
    pub input_schema: Option<Value>,
    /// 危险等级（复用 `crate::devices::HazardLevel`）。
    pub hazard: HazardLevel,
}

/// 可注入的 MCP transport 抽象。
///
/// 所有真实 I/O（stdio 子进程、WebSocket、Streamable HTTP）都必须落在这个 trait
/// 的某个实现里；本模块自身不发起任何网络 / 进程连接。单测用 Stub 实现，
/// 零出向即可覆盖 [`McpEndpoint`] 的全部行为。
#[async_trait]
pub trait McpTransport: Send + Sync {
    /// 列出该 endpoint 暴露的全部工具（MCP `tools/list` 语义）。
    async fn list_tools(&self) -> Result<Vec<ToolSpec>, McpError>;

    /// 按 name 调用工具（MCP `tools/call` 语义）。
    ///
    /// `args` 必须是 JSON object（MCP 契约）；非 object 由 [`McpEndpoint::call_tool`]
    /// 在本地前置拒绝，不进 transport。未知工具名由 transport 自己回
    /// [`McpError::ToolNotFound`]（真实 server 会在 JSON-RPC 层报 -32601）。
    async fn call_tool(&self, name: &str, args: Value) -> Result<Value, McpError>;
}

/// 一个已连接的 MCP endpoint 句柄。
///
/// 字段字面就是 `{ name, transport }`：`name` 是日志 / 审计 / 前端徽标用的
/// 端点别名，`transport` 是被注入的具体连接（可换 Stub / 真 stdio）。
pub struct McpEndpoint<T: McpTransport> {
    /// endpoint 别名（配置里登记的名字，例如 "filesystem" / "github"）。
    pub name: String,
    /// 被注入的 transport；泛型而非 `Box<dyn>`，便于零开销单测。
    pub transport: T,
}

impl<T: McpTransport> McpEndpoint<T> {
    /// 构造一个 endpoint 句柄；不做任何连接握手（transport 在传入时应已就绪，
    /// 或按其自身懒连接策略工作——本层不关心）。
    pub fn new(name: impl Into<String>, transport: T) -> Self {
        Self {
            name: name.into(),
            transport,
        }
    }

    /// 列出工具：直接透传 transport。
    pub async fn list_tools(&self) -> Result<Vec<ToolSpec>, McpError> {
        self.transport.list_tools().await
    }

    /// 调用工具。
    ///
    /// 本地前置校验（fail-closed）：
    /// - `name` 空串 → [`McpError::InvalidArgs`]（不打 transport）；
    /// - `args` 非 JSON object → [`McpError::InvalidArgs`]（MCP `tools/call` 契约要求 object）。
    ///
    /// 通过前置后**直接透传** transport——未知工具名 / 执行失败由 transport 映射成
    /// [`McpError::ToolNotFound`] / [`McpError::Transport`]，本层不重试、不缓存、
    /// 不擅自降级 L2/L3。hazard 分级见 [`list_tools()`](Self::list_tools) 返回的
    /// [`ToolSpec`]，由外层闸门决策。
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<Value, McpError> {
        if name.is_empty() {
            return Err(McpError::InvalidArgs(
                "tool name 不可为空".to_string(),
            ));
        }
        if !args.is_object() {
            let shape = if args.is_null() {
                "null".to_string()
            } else if args.is_array() {
                "array".to_string()
            } else if args.is_string() {
                "string".to_string()
            } else if args.is_number() {
                "number".to_string()
            } else if args.is_boolean() {
                "boolean".to_string()
            } else {
                "unknown".to_string()
            };
            return Err(McpError::InvalidArgs(format!(
                "MCP tools/call 的 args 必须是 JSON object，收到 {shape}"
            )));
        }
        tracing::debug!(endpoint = %self.name, tool = %name, "mcp_router::call_tool dispatch");
        self.transport.call_tool(name, args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Stub transport：工具清单预置，调用记录进 Mutex 日志，零出向。
    struct StubTransport {
        tools: Vec<ToolSpec>,
        /// 记录每次 (name, args)，供测试断言调用路径。
        calls: Mutex<Vec<(String, Value)>>,
        /// 预置的成功返回值（按工具名）；未命中 → ToolNotFound。
        canned: Vec<(String, Value)>,
    }

    impl StubTransport {
        fn new(tools: Vec<ToolSpec>, canned: Vec<(String, Value)>) -> Self {
            Self {
                tools,
                calls: Mutex::new(Vec::new()),
                canned,
            }
        }

        fn recorded(&self) -> Vec<(String, Value)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl McpTransport for StubTransport {
        async fn list_tools(&self) -> Result<Vec<ToolSpec>, McpError> {
            Ok(self.tools.clone())
        }

        async fn call_tool(&self, name: &str, args: Value) -> Result<Value, McpError> {
            self.calls.lock().unwrap().push((name.to_string(), args.clone()));
            match self.canned.iter().find(|(n, _)| n == name) {
                Some((_, v)) => Ok(v.clone()),
                None => Err(McpError::ToolNotFound(name.to_string())),
            }
        }
    }

    fn sample_tools() -> Vec<ToolSpec> {
        vec![
            ToolSpec {
                name: "read_file".to_string(),
                description: "读文件（只读）".to_string(),
                input_schema: Some(serde_json::json!({"type": "object"})),
                hazard: HazardLevel::ReadOnly,
            },
            ToolSpec {
                name: "write_file".to_string(),
                description: "写文件（安全写）".to_string(),
                input_schema: None,
                hazard: HazardLevel::SafeAuto,
            },
            ToolSpec {
                name: "unlock_door".to_string(),
                description: "开锁（物理致动）".to_string(),
                input_schema: None,
                hazard: HazardLevel::PhysicalActuation,
            },
        ]
    }

    #[tokio::test]
    async fn list_tools_passes_through_transport() {
        let ep = McpEndpoint::new(
            "stub",
            StubTransport::new(sample_tools(), vec![]),
        );
        let tools = ep.list_tools().await.unwrap();
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0].name, "read_file");
        // hazard 原样保留：L0/L1/L3 三档都在。
        assert_eq!(tools[0].hazard, HazardLevel::ReadOnly);
        assert_eq!(tools[1].hazard, HazardLevel::SafeAuto);
        assert_eq!(tools[2].hazard, HazardLevel::PhysicalActuation);
    }

    #[tokio::test]
    async fn call_tool_delegates_with_args_object() {
        let ep = McpEndpoint::new(
            "stub",
            StubTransport::new(
                sample_tools(),
                vec![("read_file".to_string(), serde_json::json!({"content": "hi"}))],
            ),
        );
        let out = ep
            .call_tool("read_file", serde_json::json!({"path": "/x"}))
            .await
            .unwrap();
        assert_eq!(out, serde_json::json!({"content": "hi"}));
        // stub 真的被调到一次，args 原样透传。
        let rec = ep.transport.recorded();
        assert_eq!(rec.len(), 1);
        assert_eq!(rec[0].0, "read_file");
        assert_eq!(rec[0].1, serde_json::json!({"path": "/x"}));
    }

    #[tokio::test]
    async fn call_tool_rejects_empty_name_before_transport() {
        let ep = McpEndpoint::new(
            "stub",
            StubTransport::new(sample_tools(), vec![]),
        );
        let err = ep.call_tool("", serde_json::json!({})).await.unwrap_err();
        match err {
            McpError::InvalidArgs(_) => {}
            other => panic!("期望 InvalidArgs，得到 {other:?}"),
        }
        // transport 一次都没被调到。
        assert!(ep.transport.recorded().is_empty());
    }

    #[tokio::test]
    async fn call_tool_rejects_non_object_args() {
        let ep = McpEndpoint::new(
            "stub",
            StubTransport::new(sample_tools(), vec![]),
        );
        for bad in [
            serde_json::json!(null),
            serde_json::json!([1, 2, 3]),
            serde_json::json!("str"),
            serde_json::json!(42),
            serde_json::json!(true),
        ] {
            let err = ep.call_tool("read_file", bad).await.unwrap_err();
            assert!(
                matches!(err, McpError::InvalidArgs(_)),
                "未拒绝非法 args: {err:?}"
            );
        }
        // 所有非法形状都在本地被拒，零出向。
        assert!(ep.transport.recorded().is_empty());
    }

    #[tokio::test]
    async fn call_tool_unknown_name_propagates_tool_not_found() {
        let ep = McpEndpoint::new(
            "stub",
            StubTransport::new(sample_tools(), vec![]),
        );
        let err = ep
            .call_tool("does_not_exist", serde_json::json!({}))
            .await
            .unwrap_err();
        match err {
            McpError::ToolNotFound(n) => assert_eq!(n, "does_not_exist"),
            other => panic!("期望 ToolNotFound，得到 {other:?}"),
        }
        // 即便工具不存在，transport 仍被咨询了一次（MCP server 自己判 -32601）。
        assert_eq!(ep.transport.recorded().len(), 1);
    }

    #[test]
    fn tool_spec_serde_roundtrip_preserves_hazard() {
        // ToolSpec 要能跨 RPC / 缓存序列化；hazard 枚举的 L0–L3 必须往返不变。
        let spec = ToolSpec {
            name: "x".into(),
            description: "d".into(),
            input_schema: None,
            hazard: HazardLevel::RequiresApproval,
        };
        let v = serde_json::to_value(&spec).unwrap();
        assert_eq!(v["hazard"], "RequiresApproval");
        let back: ToolSpec = serde_json::from_value(v).unwrap();
        assert_eq!(back, spec);
    }
}
