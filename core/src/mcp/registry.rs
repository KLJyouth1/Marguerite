//! 工具注册表
//!
//! 管理所有可用工具的注册中心，支持本地工具和通过 MCP 客户端发现的远程工具。

use std::collections::{HashMap, HashSet};
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

/// 把工具名规范化为 OpenAI/DeepSeek function name 形式（`^[a-zA-Z0-9_-]+$`）。
///
/// 不合法字符（点、空格、斜杠等）替换为下划线，避免 API 400 导致全部工具不可用。
///
/// **根因约定**：内置工具名必须在**调用方**就写合法（见 `tools/baota.rs`），
/// 本函数只是兜底，不应是内置工具的常规路径——否则每次启动都会走「注册后修正」。
/// 抽成纯函数供注册路径与回归测试共用，保证「测试口径 == 生产口径」。
pub(crate) fn sanitize_tool_name(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// 工具注册表
///
/// 管理所有可用工具的注册中心，支持本地工具和 MCP 工具。
#[derive(Default)]
pub struct ToolRegistry {
    /// 本地工具映射
    local_tools: HashMap<String, Box<dyn ToolExecutor>>,
    /// 历史别名表：注册时被规范化过的原始名 → 实际派发键。
    ///
    /// 修复前内置 baota 工具用点号名注册（`baota.system_info`），被静默改成
    /// `baota_system_info` 后**原名不可派发**；这里显式记住映射，保证旧名仍能调用，
    /// 同时把「修正」从 WARN 降为 debug（见 `register_local`）。
    legacy_aliases: HashMap<String, String>,
    /// MCP 客户端列表
    mcp_clients: Vec<Arc<McpClient>>,
}

impl ToolRegistry {
    /// 创建新的工具注册表
    pub fn new() -> Self {
        Self {
            local_tools: HashMap::new(),
            legacy_aliases: HashMap::new(),
            mcp_clients: Vec::new(),
        }
    }

    /// 注册本地工具
    pub fn register_local(&mut self, tool: Box<dyn ToolExecutor>) {
        let raw_name = tool.name().to_string();
        let name = sanitize_tool_name(&raw_name);
        if name != raw_name {
            // 降噪（P1-E）：这里曾经是 `warn!`，而内置 baota 工具名自带点号，
            // 于是**每次首启**（含无配置的离线回显路径）都向 stderr 刷 13+ 行告警。
            // 根因已在 `tools/baota.rs` 消除（内置名一步到位合法），此分支只应被
            // 第三方/插件注册触发，故降为 debug；并登记历史别名保证兼容。
            tracing::debug!(
                original = %raw_name,
                sanitized = %name,
                "工具名包含非法字符，已自动修正（原名仍可通过历史别名调用）"
            );
            self.legacy_aliases
                .entry(raw_name)
                .or_insert_with(|| name.clone());
        }
        self.local_tools.insert(name, tool);
    }

    /// 把（可能是历史别名的）工具名解析为实际派发键。
    fn resolve_alias<'a>(&'a self, name: &'a str) -> &'a str {
        match self.legacy_aliases.get(name) {
            Some(canonical) => canonical.as_str(),
            None => name,
        }
    }

    /// 历史别名表快照（原始非法名 → 规范名）。
    ///
    /// 空表 = 所有已注册工具名「首次注册即合法」，没有任何工具走过自动修正路径。
    /// 回归测试据此断言内置工具名不得退化为「先注册非法名再修正」。
    pub fn legacy_aliases(&self) -> &HashMap<String, String> {
        &self.legacy_aliases
    }

    /// 添加 MCP 客户端
    pub fn add_mcp_client(&mut self, client: Arc<McpClient>) {
        self.mcp_clients.push(client);
    }

    /// 移除一个本地工具（工具面 patch 层热禁用用）。
    ///
    /// 名字按注册时的规范化规则解析（历史别名同样可命中）；同时清理指向它的别名，
    /// 否则被禁用的工具会**通过别名继续可达**——禁用就成了摆设。
    /// 返回是否真的移除了一个工具。
    pub fn remove_local(&mut self, name: &str) -> bool {
        let key = self.resolve_alias(name).to_string();
        let removed = self.local_tools.remove(&key).is_some();
        if removed {
            self.legacy_aliases.retain(|_, canonical| canonical != &key);
        }
        removed
    }

    /// 获取工具
    ///
    /// 优先查找本地工具，未找到时遍历 MCP 客户端。
    pub fn get_tool(&self, name: &str) -> Option<Box<dyn ToolExecutor>> {
        // 先查本地工具（含历史别名解析）
        if self.local_tools.contains_key(self.resolve_alias(name)) {
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
        if self.local_tools.contains_key(self.resolve_alias(name)) {
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
    /// 仅移除本地注册的工具，不影响 MCP 工具。指向被移除工具的历史别名一并清理，
    /// 避免留下指向空目标的悬垂别名。
    pub fn remove_tool(&mut self, name: &str) {
        let key = self.resolve_alias(name).to_string();
        self.local_tools.remove(&key);
        self.legacy_aliases.retain(|_, canonical| canonical != &key);
    }

    /// 执行本地工具
    pub async fn execute_local_tool(
        &self,
        name: &str,
        args: &std::collections::HashMap<String, String>,
    ) -> Result<ToolResult> {
        let tool = self
            .local_tools
            .get(self.resolve_alias(name))
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
        // 稳定性优先：`local_tools` 是 HashMap，迭代顺序每个进程随机。工具列表既会
        // 写进 system 的「本轮可调用工具」段，也决定 provider 的 tools 数组顺序；
        // 顺序不稳定会让前缀缓存在重启后整段失效。按名字排序是确定性的。
        let mut items: Vec<(String, String, serde_json::Value)> = self
            .local_tools
            .iter()
            .map(|(name, tool)| {
                (
                    name.clone(),
                    tool.description().to_string(),
                    tool.input_schema(),
                )
            })
            .collect();
        items.sort_by(|a, b| a.0.cmp(&b.0));
        items
    }

    /// 返回已接入 MCP 客户端的工具描述（名称、描述、输入 schema），
    /// 供 LLM function calling 构建 `ToolDef`。
    pub fn describe_mcp_tools(&self) -> Vec<(String, String, serde_json::Value)> {
        let mut items = Vec::new();
        for client in &self.mcp_clients {
            for tool in &client.tools_cache {
                // 与派发侧（`McpClient::get_tool`）共用同一清洗函数，二者不得分叉。
                let name = crate::mcp::client::sanitize_mcp_tool_name(&tool.name);
                items.push((name, tool.description.clone(), tool.input_schema.clone()));
            }
        }
        items
    }

    /// 合并本地 + MCP 工具描述（本地/先注册者优先），并返回被遮蔽的工具名。
    ///
    /// 单一事实源：`describe_tools_for_llm` 与 `detect_tool_name_collisions`
    /// 共用本函数，保证「实际广告的集合」与「报告的冲突集合」不分叉。
    fn merge_tool_descriptions(&self) -> (Vec<(String, String, serde_json::Value)>, Vec<String>) {
        let mut merged = self.describe_local_tools();
        let mut seen: HashSet<String> = merged.iter().map(|(n, _, _)| n.clone()).collect();
        let mut shadowed = Vec::new();
        for (name, desc, schema) in self.describe_mcp_tools() {
            if seen.insert(name.clone()) {
                merged.push((name, desc, schema));
            } else {
                // 同名已被本地工具（或先注册的 MCP 工具）占用：确定性「先到先得」，
                // 但必须留下信号，否则被遮蔽的工具会静默消失（D3）。
                shadowed.push(name);
            }
        }
        // MCP 工具顺序取决于客户端/缓存顺序：统一按名字排序，保证同一工具集在
        // 任意进程启动后都产出逐字节一致的工具列表（前缀缓存的前提）。
        merged.sort_by(|a, b| a.0.cmp(&b.0));
        (merged, shadowed)
    }

    /// 返回 LLM function calling 所需的全部工具描述（本地 + 已接入 MCP）。
    /// 名称冲突时本地优先（MCP 同名工具被忽略），并对每个被遮蔽的名字告警。
    pub fn describe_tools_for_llm(&self) -> Vec<(String, String, serde_json::Value)> {
        let (merged, shadowed) = self.merge_tool_descriptions();
        for name in &shadowed {
            tracing::warn!(
                tool = %name,
                "MCP 工具名与已注册工具冲突，已被遮蔽（本地/先注册者优先）"
            );
        }
        merged
    }

    /// 返回 `describe_tools_for_llm` 中被遮蔽（shadowed）的工具名列表。
    ///
    /// 供运维与单测断言冲突确实被检测到——无需捕获日志即可验证。
    pub fn detect_tool_name_collisions(&self) -> Vec<String> {
        self.merge_tool_descriptions().1
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

    /// D3：本地工具与 MCP 工具同名时，冲突必须被检测到（可单测，无需捕获日志）。
    #[test]
    fn test_describe_tools_detects_local_mcp_collision() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(MockTool::new("dup", false)));
        let mut client = McpClient::new("http://localhost:8080");
        client.tools_cache = vec![
            McpTool {
                name: "dup".to_string(),
                description: "remote desc".to_string(),
                input_schema: serde_json::Value::Null,
            },
            McpTool {
                name: "unique".to_string(),
                description: "remote unique".to_string(),
                input_schema: serde_json::Value::Null,
            },
        ];
        registry.add_mcp_client(Arc::new(client));

        assert_eq!(
            registry.detect_tool_name_collisions(),
            vec!["dup".to_string()]
        );

        // 解析仍确定：本地胜出、唯一的 MCP 工具保留。
        let all = registry.describe_tools_for_llm();
        assert_eq!(all.len(), 2);
        assert!(all
            .iter()
            .any(|(n, d, _)| n == "dup" && d == "Mock tool: dup"));
        assert!(all.iter().any(|(n, _, _)| n == "unique"));
    }

    /// D3：两个 MCP server 暴露同名工具时，第二个被遮蔽并上报冲突。
    #[test]
    fn test_describe_tools_detects_cross_server_collision() {
        let mut registry = ToolRegistry::new();
        for url in ["http://a:1", "http://b:1"] {
            let mut client = McpClient::new(url);
            client.tools_cache = vec![McpTool {
                name: "shared".to_string(),
                description: format!("from {url}"),
                input_schema: serde_json::Value::Null,
            }];
            registry.add_mcp_client(Arc::new(client));
        }

        assert_eq!(
            registry.detect_tool_name_collisions(),
            vec!["shared".to_string()]
        );
        assert_eq!(registry.describe_tools_for_llm().len(), 1);
    }

    /// D3：无冲突时冲突列表为空（回归守卫，防止误报）。
    #[test]
    fn test_describe_tools_no_collision() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(MockTool::new("local_a", false)));
        let mut client = McpClient::new("http://localhost:8080");
        client.tools_cache = vec![McpTool {
            name: "remote_b".to_string(),
            description: "remote desc".to_string(),
            input_schema: serde_json::json!({}),
        }];
        registry.add_mcp_client(Arc::new(client));

        assert!(registry.detect_tool_name_collisions().is_empty());
    }
}
