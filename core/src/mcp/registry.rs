//! 工具注册表
//!
//! 管理所有可用工具的注册中心，支持本地工具和通过 MCP 客户端发现的远程工具。

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use super::client::McpClient;
use crate::agent::tool_cycle::ToolResult;

/// 工具执行器 trait
///
/// 所有本地工具需实现此 trait 以注册到 ToolRegistry。
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// 工具名称
    fn name(&self) -> &str;
    /// 工具描述
    fn description(&self) -> &str;
    /// 执行工具
    async fn execute(&self, args: &std::collections::HashMap<String, String>)
        -> Result<ToolResult>;
    /// 输入参数 Schema (JSON Schema)
    fn input_schema(&self) -> serde_json::Value;
}

/// MCP 工具执行器包装
///
/// 将 MCP 客户端发现的工具包装为 ToolExecutor，使其可通过注册表统一调用。
struct McpToolExecutor {
    client: Arc<McpClient>,
    tool_name: String,
    description: String,
    input_schema: serde_json::Value,
}

impl McpToolExecutor {
    fn new(client: Arc<McpClient>, tool: &super::client::McpTool) -> Self {
        Self {
            client,
            tool_name: tool.name.clone(),
            description: tool.description.clone(),
            input_schema: tool.input_schema.clone(),
        }
    }
}

#[async_trait]
impl ToolExecutor for McpToolExecutor {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    async fn execute(
        &self,
        args: &std::collections::HashMap<String, String>,
    ) -> Result<ToolResult> {
        let result = self.client.call_tool(&self.tool_name, args).await?;

        if result.is_error {
            let message = result
                .content
                .iter()
                .filter_map(|c| match c {
                    super::client::McpContent::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");

            return Ok(ToolResult::Error {
                message,
                code: "MCP_ERROR".to_string(),
            });
        }

        let data = result
            .content
            .iter()
            .filter_map(|c| match c {
                super::client::McpContent::Text { text } => Some(text.clone()),
                super::client::McpContent::Resource { text, .. } => Some(text.clone()),
                super::client::McpContent::Image { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(ToolResult::Success {
            data,
            metadata: HashMap::new(),
        })
    }

    fn input_schema(&self) -> serde_json::Value {
        self.input_schema.clone()
    }
}

/// 工具目录条目（网关 `tools.catalog` 输出）
#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    /// 来源："local"（内置工具）或 "mcp"（MCP 服务器）
    pub source: String,
    /// MCP 服务器展示名（内置工具为空串）
    pub server: String,
}

/// 工具注册表
///
/// 管理所有可用工具的注册中心，支持本地工具和 MCP 工具。
#[derive(Default)]
pub struct ToolRegistry {
    /// 本地工具映射
    local_tools: HashMap<String, Box<dyn ToolExecutor>>,
    /// MCP 客户端列表
    mcp_clients: Vec<Arc<McpClient>>,
}

impl ToolRegistry {
    /// 创建新的工具注册表
    pub fn new() -> Self {
        Self {
            local_tools: HashMap::new(),
            mcp_clients: Vec::new(),
        }
    }

    /// 注册本地工具
    pub fn register_local(&mut self, tool: Box<dyn ToolExecutor>) {
        let raw_name = tool.name().to_string();
        // OpenAI/DeepSeek function name 规范：^[a-zA-Z0-9_-]+$
        // 不合法字符（点、空格、斜杠等）替换为下划线，避免 API 400 导致全部工具不可用
        let name: String = raw_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if name != raw_name {
            tracing::warn!(original = %raw_name, sanitized = %name, "工具名包含非法字符，已自动修正");
        }
        self.local_tools.insert(name, tool);
    }

    /// 添加 MCP 客户端
    pub fn add_mcp_client(&mut self, client: Arc<McpClient>) {
        self.mcp_clients.push(client);
    }

    /// 获取工具
    ///
    /// 优先查找本地工具，未找到时遍历 MCP 客户端。
    pub fn get_tool(&self, name: &str) -> Option<Box<dyn ToolExecutor>> {
        // 先查本地工具
        if self.local_tools.contains_key(name) {
            // 本地工具通过 execute_local_tool 直接调用，此处不返回 Box 包装
            return None;
        }

        // 查 MCP 客户端
        for client in &self.mcp_clients {
            if let Some(tool) = client.get_tool(name) {
                return Some(Box::new(McpToolExecutor::new(client.clone(), &tool)));
            }
        }

        None
    }

    /// 列出所有工具名称
    ///
    /// 返回本地工具和 MCP 工具的名称列表。
    pub fn list_tools(&self) -> Vec<String> {
        let mut names: Vec<String> = self.local_tools.keys().cloned().collect();

        for client in &self.mcp_clients {
            // 从缓存中获取工具名称
            // 注意：这里只返回已缓存的工具，不会触发网络请求
            // 完整的工具列表需要通过 list_tools_async 获取
            for tool in &client.tools_cache {
                names.push(tool.name.clone());
            }
        }

        names.sort();
        names.dedup();
        names
    }

    /// 工具目录：名称 + 描述（供网关 `tools.catalog` 面板）
    pub fn catalog(&self) -> Vec<ToolInfo> {
        let mut items: Vec<ToolInfo> = self
            .local_tools
            .values()
            .map(|t| ToolInfo {
                name: t.name().to_string(),
                description: t.description().to_string(),
                source: "local".to_string(),
                server: String::new(),
            })
            .collect();
        for client in &self.mcp_clients {
            let server = client.label();
            for tool in &client.tools_cache {
                items.push(ToolInfo {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    source: "mcp".to_string(),
                    server: server.clone(),
                });
            }
        }
        items.sort_by(|a, b| a.name.cmp(&b.name));
        items.dedup_by(|a, b| a.name == b.name);
        items
    }

    /// 异步列出所有工具名称（包括从 MCP 服务器获取）
    ///
    /// 注意：此方法会尝试从所有已连接的 MCP 客户端获取最新工具列表。
    /// 由于 MCP 客户端通过 Arc 共享，实际刷新缓存需由客户端自行完成。
    pub async fn list_tools_async(&mut self) -> Result<Vec<String>> {
        let mut names: Vec<String> = self.local_tools.keys().cloned().collect();

        // 从 MCP 客户端缓存中获取工具名称（不会触发网络请求）
        for client in &self.mcp_clients {
            for tool in &client.tools_cache {
                names.push(tool.name.clone());
            }
        }

        names.sort();
        names.dedup();
        Ok(names)
    }

    /// 检查工具是否存在
    pub fn has_tool(&self, name: &str) -> bool {
        if self.local_tools.contains_key(name) {
            return true;
        }

        for client in &self.mcp_clients {
            if client.get_tool(name).is_some() {
                return true;
            }
        }

        false
    }

    /// 移除本地工具
    ///
    /// 仅移除本地注册的工具，不影响 MCP 工具。
    pub fn remove_tool(&mut self, name: &str) {
        self.local_tools.remove(name);
    }

    /// 执行本地工具
    pub async fn execute_local_tool(
        &self,
        name: &str,
        args: &std::collections::HashMap<String, String>,
    ) -> Result<ToolResult> {
        let tool = self
            .local_tools
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Tool '{}' not found in local registry", name))?;
        tool.execute(args).await
    }

    /// 获取本地工具数量
    pub fn local_tool_count(&self) -> usize {
        self.local_tools.len()
    }

    /// 返回本地工具的描述（名称、描述、输入 schema），供 LLM function calling 构建 `ToolDef`。
    ///
    /// 本地工具通过 `execute_local_tool` 直接调用；此处仅枚举元数据，不触发执行。
    pub fn describe_local_tools(&self) -> Vec<(String, String, serde_json::Value)> {
        self.local_tools
            .iter()
            .map(|(name, tool)| {
                (
                    name.clone(),
                    tool.description().to_string(),
                    tool.input_schema(),
                )
            })
            .collect()
    }

    /// 返回已接入 MCP 客户端的工具描述（名称、描述、输入 schema），
    /// 供 LLM function calling 构建 `ToolDef`。
    pub fn describe_mcp_tools(&self) -> Vec<(String, String, serde_json::Value)> {
        let mut items = Vec::new();
        for client in &self.mcp_clients {
            for tool in &client.tools_cache {
                let name: String = tool
                    .name
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                items.push((name, tool.description.clone(), tool.input_schema.clone()));
            }
        }
        items
    }

    /// 返回 LLM function calling 所需的全部工具描述（本地 + 已接入 MCP）。
    /// 名称冲突时本地优先（MCP 同名工具被忽略）。
    pub fn describe_tools_for_llm(&self) -> Vec<(String, String, serde_json::Value)> {
        let mut local = self.describe_local_tools();
        let mut seen: std::collections::HashSet<String> =
            local.iter().map(|(n, _, _)| n.clone()).collect();
        for (name, desc, schema) in self.describe_mcp_tools() {
            if seen.insert(name.clone()) {
                local.push((name, desc, schema));
            }
        }
        local
    }

    /// 获取 MCP 客户端数量
    pub fn mcp_client_count(&self) -> usize {
        self.mcp_clients.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::client::McpTool;
    use std::collections::HashMap;

    struct MockTool {
        name: String,
        description: String,
        should_fail: bool,
    }

    impl MockTool {
        fn new(name: &str, should_fail: bool) -> Self {
            Self {
                name: name.to_string(),
                description: format!("Mock tool: {}", name),
                should_fail,
            }
        }
    }

    #[async_trait]
    impl ToolExecutor for MockTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            &self.description
        }

        async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
            if self.should_fail {
                return Ok(ToolResult::Error {
                    message: "Mock error".to_string(),
                    code: "MOCK_ERROR".to_string(),
                });
            }

            let input = args.get("input").cloned().unwrap_or_default();
            Ok(ToolResult::Success {
                data: format!("{} executed: {}", self.name, input),
                metadata: HashMap::new(),
            })
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "input": {"type": "string"}
                }
            })
        }
    }

    #[test]
    fn test_register_and_list_tools() {
        let mut registry = ToolRegistry::new();

        registry.register_local(Box::new(MockTool::new("tool_a", false)));
        registry.register_local(Box::new(MockTool::new("tool_b", false)));

        let tools = registry.list_tools();
        assert_eq!(tools.len(), 2);
        assert!(tools.contains(&"tool_a".to_string()));
        assert!(tools.contains(&"tool_b".to_string()));
    }

    #[test]
    fn test_has_tool() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(MockTool::new("my_tool", false)));

        assert!(registry.has_tool("my_tool"));
        assert!(!registry.has_tool("nonexistent"));
    }

    #[test]
    fn test_remove_tool() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(MockTool::new("tool_to_remove", false)));
        assert!(registry.has_tool("tool_to_remove"));

        registry.remove_tool("tool_to_remove");
        assert!(!registry.has_tool("tool_to_remove"));
    }

    #[test]
    fn test_remove_nonexistent_tool() {
        let mut registry = ToolRegistry::new();
        registry.remove_tool("nonexistent"); // 不应 panic
        assert_eq!(registry.local_tool_count(), 0);
    }

    #[tokio::test]
    async fn test_execute_local_tool_success() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(MockTool::new("greeter", false)));

        let mut args = HashMap::new();
        args.insert("input".to_string(), "Hello".to_string());

        let result = registry.execute_local_tool("greeter", &args).await.unwrap();
        match result {
            ToolResult::Success { data, .. } => {
                assert_eq!(data, "greeter executed: Hello");
            }
            _ => panic!("Expected Success"),
        }
    }

    #[tokio::test]
    async fn test_execute_local_tool_not_found() {
        let registry = ToolRegistry::new();
        let mut args = HashMap::new();
        args.insert("input".to_string(), "test".to_string());

        let result = registry.execute_local_tool("nonexistent", &args).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_local_tool_error() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(MockTool::new("failing_tool", true)));

        let args = HashMap::new();
        let result = registry
            .execute_local_tool("failing_tool", &args)
            .await
            .unwrap();
        match result {
            ToolResult::Error { message, code } => {
                assert_eq!(message, "Mock error");
                assert_eq!(code, "MOCK_ERROR");
            }
            _ => panic!("Expected Error"),
        }
    }

    #[test]
    fn test_empty_registry() {
        let registry = ToolRegistry::new();
        assert_eq!(registry.local_tool_count(), 0);
        assert_eq!(registry.mcp_client_count(), 0);
        assert!(registry.list_tools().is_empty());
        assert!(!registry.has_tool("anything"));
    }

    #[test]
    fn test_describe_local_tools() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(MockTool::new("tool_a", false)));
        let descs = registry.describe_local_tools();
        assert_eq!(descs.len(), 1);
        assert_eq!(descs[0].0, "tool_a");
        assert!(descs[0].1.contains("Mock tool"));
        assert_eq!(descs[0].2["type"], "object");
    }

    #[test]
    fn test_add_mcp_client() {
        let mut registry = ToolRegistry::new();
        let client = Arc::new(McpClient::new("http://localhost:8080"));
        registry.add_mcp_client(client);
        assert_eq!(registry.mcp_client_count(), 1);
    }

    #[test]
    fn test_tool_executor_input_schema() {
        let tool = MockTool::new("schema_test", false);
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["input"].is_object());
    }

    #[test]
    fn test_get_tool_local_returns_none() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(MockTool::new("local", false)));
        // 本地工具通过 execute_local_tool 调用，get_tool 返回 None
        assert!(registry.get_tool("local").is_none());
    }

    #[test]
    fn test_list_tools_with_mcp_cache() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(MockTool::new("local", false)));
        let mut client = McpClient::new("http://localhost:8080");
        client.tools_cache = vec![McpTool {
            name: "remote".to_string(),
            description: "remote desc".to_string(),
            input_schema: serde_json::json!({}),
        }];
        registry.add_mcp_client(Arc::new(client));

        let tools = registry.list_tools();
        assert_eq!(tools, vec!["local".to_string(), "remote".to_string()]);

        let tools2 = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(registry.list_tools_async())
            .unwrap();
        assert_eq!(tools2, vec!["local".to_string(), "remote".to_string()]);
    }

    #[tokio::test]
    async fn test_mcp_tool_executor_via_cache() {
        let mut registry = ToolRegistry::new();
        let mut client = McpClient::new("http://localhost:8080");
        client.tools_cache = vec![McpTool {
            name: "remote_tool".to_string(),
            description: "remote desc".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        registry.add_mcp_client(Arc::new(client));

        assert!(registry.has_tool("remote_tool"));
        let tool = registry.get_tool("remote_tool").unwrap();
        assert_eq!(tool.name(), "remote_tool");
        assert_eq!(tool.description(), "remote desc");
        assert_eq!(tool.input_schema()["type"], "object");

        // 调用远端工具：本地无服务 → 连接失败
        let result = tool.execute(&HashMap::new()).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_list_tools_dedup() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(MockTool::new("dup", false)));
        let mut client = McpClient::new("http://localhost:8080");
        client.tools_cache = vec![McpTool {
            name: "dup".to_string(),
            description: "".to_string(),
            input_schema: serde_json::Value::Null,
        }];
        registry.add_mcp_client(Arc::new(client));
        let tools = registry.list_tools();
        assert_eq!(tools.len(), 1);
    }

    #[test]
    fn test_describe_tools_for_llm_merges_mcp() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(MockTool::new("local_a", false)));
        let mut client = McpClient::new("http://localhost:8080");
        client.tools_cache = vec![McpTool {
            name: "remote_b".to_string(),
            description: "remote desc".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        registry.add_mcp_client(Arc::new(client));

        let all = registry.describe_tools_for_llm();
        let names: Vec<&String> = all.iter().map(|(n, _, _)| n).collect();
        assert!(names.contains(&&"local_a".to_string()));
        assert!(names.contains(&&"remote_b".to_string()));
    }

    #[test]
    fn test_describe_tools_for_llm_local_wins_on_collision() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(MockTool::new("dup", false)));
        let mut client = McpClient::new("http://localhost:8080");
        client.tools_cache = vec![McpTool {
            name: "dup".to_string(),
            description: "remote desc".to_string(),
            input_schema: serde_json::Value::Null,
        }];
        registry.add_mcp_client(Arc::new(client));

        let all = registry.describe_tools_for_llm();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].1, "Mock tool: dup"); // 本地描述优先
    }
}
