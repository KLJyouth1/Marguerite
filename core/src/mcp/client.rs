//! MCP 客户端
//!
//! 实现 Model Context Protocol 客户端，用于连接 MCP Server 发现和调用工具。
//! 通过 HTTP JSON-RPC 与 MCP Server 通信。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// MCP 工具描述
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    /// 工具名称
    pub name: String,
    /// 工具描述
    pub description: String,
    /// 输入参数 Schema (JSON Schema)
    pub input_schema: serde_json::Value,
}

/// MCP 工具调用结果
#[derive(Debug, Clone)]
pub struct McpToolResult {
    /// 结果内容列表
    pub content: Vec<McpContent>,
    /// 是否错误
    pub is_error: bool,
}

/// MCP 内容类型
#[derive(Debug, Clone)]
pub enum McpContent {
    /// 文本内容
    Text { text: String },
    /// 图片内容
    Image { data: String, mime_type: String },
    /// 资源内容
    Resource { uri: String, text: String },
}

/// MCP 客户端
///
/// 通过 HTTP JSON-RPC 与 MCP Server 通信，支持工具列表缓存。
#[derive(Debug, Clone)]
pub struct McpClient {
    /// MCP Server URL
    server_url: String,
    /// HTTP 客户端
    client: reqwest::Client,
    /// 工具列表缓存
    pub(crate) tools_cache: Vec<McpTool>,
    /// 缓存 TTL
    cache_ttl: Duration,
    /// 上次缓存时间
    last_cache_time: Instant,
    /// 连接状态
    connected: bool,
}

impl McpClient {
    /// 创建新的 MCP 客户端
    pub fn new(server_url: &str) -> Self {
        Self {
            server_url: server_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
            tools_cache: Vec::new(),
            cache_ttl: Duration::from_secs(300), // 5 分钟
            last_cache_time: Instant::now(),
            connected: false,
        }
    }

    /// 服务器展示名（供工具目录标注 MCP 来源）。
    ///
    /// 优先取 URL 最后一段路径（如 `…/mcp` → "mcp"）；纯 host:port 形式则
    /// 原样返回（如 "127.0.0.1:8888"），保证同一面板的多个工具归一到同一标签。
    pub fn label(&self) -> String {
        let url = self.server_url.trim_end_matches('/');
        match url.rsplit('/').next() {
            Some(last) if !last.is_empty() && !last.contains('.') && !last.contains(':') => {
                last.to_string()
            }
            _ => url
                .trim_start_matches("http://")
                .trim_start_matches("https://")
                .to_string(),
        }
    }

    /// 连接 MCP Server
    ///
    /// 向服务器发送健康检查请求，验证服务器是否可达。
    pub async fn connect(&mut self) -> Result<()> {
        // 尝试访问服务器根路径检查连通性
        let resp = self
            .client
            .get(&self.server_url)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| {
                anyhow!(
                    "Failed to connect to MCP server at {}: {}",
                    self.server_url,
                    e
                )
            })?;

        if resp.status().is_success() || resp.status().is_server_error() {
            // 服务器可达（即使返回 5xx 也说明服务器在运行）
            self.connected = true;
            Ok(())
        } else {
            Err(anyhow!(
                "MCP server returned unexpected status: {}",
                resp.status()
            ))
        }
    }

    /// 断开连接
    pub fn disconnect(&mut self) {
        self.connected = false;
        self.tools_cache.clear();
    }

    /// 获取工具列表（带缓存）
    ///
    /// 缓存有效期内直接返回缓存，否则重新请求服务器。
    pub async fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        // 检查缓存是否有效
        if !self.tools_cache.is_empty() && self.last_cache_time.elapsed() < self.cache_ttl {
            return Ok(self.tools_cache.clone());
        }

        let tools = self.fetch_tools().await?;
        self.tools_cache = tools.clone();
        self.last_cache_time = Instant::now();
        Ok(tools)
    }

    /// 从服务器获取工具列表
    async fn fetch_tools(&self) -> Result<Vec<McpTool>> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/list",
            "params": {},
            "id": 1
        });

        let resp = self
            .client
            .post(&self.server_url)
            .json(&request)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| anyhow!("Failed to list tools: {}", e))?;

        if !resp.status().is_success() {
            return Err(anyhow!("MCP server error: {}", resp.status()));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse MCP response: {}", e))?;

        // 检查 JSON-RPC 错误
        if let Some(error) = json.get("error") {
            return Err(anyhow!(
                "MCP error ({}): {}",
                error["code"].as_i64().unwrap_or(0),
                error["message"].as_str().unwrap_or("unknown")
            ));
        }

        let tools: Vec<McpTool> = serde_json::from_value(json["result"]["tools"].clone())
            .map_err(|e| anyhow!("Failed to parse tools list: {}", e))?;

        Ok(tools)
    }

    /// 调用工具
    pub async fn call_tool(
        &self,
        name: &str,
        args: &HashMap<String, String>,
    ) -> Result<McpToolResult> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": args
            },
            "id": 1
        });

        let resp = self
            .client
            .post(&self.server_url)
            .json(&request)
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| anyhow!("Failed to call tool '{}': {}", name, e))?;

        if !resp.status().is_success() {
            return Err(anyhow!(
                "MCP server error when calling '{}': {}",
                name,
                resp.status()
            ));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse tool call response: {}", e))?;

        // 检查 JSON-RPC 错误
        if let Some(error) = json.get("error") {
            return Err(anyhow!(
                "MCP error calling '{}' ({}): {}",
                name,
                error["code"].as_i64().unwrap_or(0),
                error["message"].as_str().unwrap_or("unknown")
            ));
        }

        let result = &json["result"];
        let is_error = result["isError"].as_bool().unwrap_or(false);

        let content: Vec<McpContent> = result["content"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|c| {
                        let type_str = c["type"].as_str().unwrap_or("text");
                        match type_str {
                            "image" => McpContent::Image {
                                data: c["data"].as_str().unwrap_or_default().to_string(),
                                mime_type: c["mimeType"].as_str().unwrap_or_default().to_string(),
                            },
                            "resource" => McpContent::Resource {
                                uri: c["uri"].as_str().unwrap_or_default().to_string(),
                                text: c["text"].as_str().unwrap_or_default().to_string(),
                            },
                            _ => McpContent::Text {
                                text: c["text"].as_str().unwrap_or_default().to_string(),
                            },
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(McpToolResult { content, is_error })
    }

    /// 获取特定工具信息（从缓存中查找）
    pub fn get_tool(&self, name: &str) -> Option<McpTool> {
        self.tools_cache.iter().find(|t| t.name == name).cloned()
    }

    /// 清除缓存
    pub fn clear_cache(&mut self) {
        self.tools_cache.clear();
        self.last_cache_time = Instant::now();
    }

    /// 检查是否已连接
    pub fn is_connected(&self) -> bool {
        self.connected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_client_new() {
        let client = McpClient::new("http://localhost:8080");
        assert_eq!(client.server_url, "http://localhost:8080");
        assert!(!client.is_connected());
        assert!(client.tools_cache.is_empty());
    }

    #[test]
    fn test_mcp_client_new_trailing_slash() {
        let client = McpClient::new("http://localhost:8080/");
        assert_eq!(client.server_url, "http://localhost:8080");
    }

    #[test]
    fn test_mcp_client_get_tool_not_found() {
        let client = McpClient::new("http://localhost:8080");
        assert!(client.get_tool("nonexistent").is_none());
    }

    #[test]
    fn test_mcp_client_clear_cache() {
        let mut client = McpClient::new("http://localhost:8080");
        client.tools_cache = vec![McpTool {
            name: "test".to_string(),
            description: "test tool".to_string(),
            input_schema: serde_json::Value::Null,
        }];
        assert!(!client.tools_cache.is_empty());
        client.clear_cache();
        assert!(client.tools_cache.is_empty());
    }

    #[test]
    fn test_mcp_content_types() {
        let text = McpContent::Text {
            text: "hello".to_string(),
        };
        let image = McpContent::Image {
            data: "base64data".to_string(),
            mime_type: "image/png".to_string(),
        };
        let resource = McpContent::Resource {
            uri: "file:///test".to_string(),
            text: "content".to_string(),
        };

        match text {
            McpContent::Text { ref text } => assert_eq!(text, "hello"),
            _ => panic!("Expected Text"),
        }
        match image {
            McpContent::Image {
                ref data,
                ref mime_type,
            } => {
                assert_eq!(data, "base64data");
                assert_eq!(mime_type, "image/png");
            }
            _ => panic!("Expected Image"),
        }
        match resource {
            McpContent::Resource { ref uri, ref text } => {
                assert_eq!(uri, "file:///test");
                assert_eq!(text, "content");
            }
            _ => panic!("Expected Resource"),
        }
    }

    #[test]
    fn test_mcp_tool_serde() {
        let tool = McpTool {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "arg1": {"type": "string"}
                }
            }),
        };

        let json = serde_json::to_value(&tool).unwrap();
        assert_eq!(json["name"], "test_tool");
        assert_eq!(json["description"], "A test tool");
        assert_eq!(json["input_schema"]["properties"]["arg1"]["type"], "string");

        let deserialized: McpTool = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.name, "test_tool");
    }

    #[tokio::test]
    async fn test_mcp_client_connect_invalid_host() {
        // 用「本机监听器回 404」构造确定性的连接失败场景：connect() 只把
        // 2xx/5xx 视为可达，404 必须报错。不用「关闭端口必被拒」——宿主若开
        // 系统代理（如 Clash），任意 loopback 端口都可能被代答 502 而误判可达。
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                if let Ok((mut socket, _)) = listener.accept().await {
                    let _ = crate::tests::mock_http::read_request_full(&mut socket).await;
                    use tokio::io::AsyncWriteExt;
                    let body = "not found";
                    let header = format!(
                        "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(body.as_bytes()).await;
                    let _ = socket.flush().await;
                }
            }
        });

        let mut client = McpClient::new(&format!("http://{addr}"));
        let result = client.connect().await;
        assert!(result.is_err(), "非 2xx/5xx 状态应确定性失败: {:?}", result);
        assert!(!client.is_connected());
    }

    #[test]
    fn test_mcp_client_disconnect() {
        let mut client = McpClient::new("http://localhost:8080");
        client.connected = true;
        client.tools_cache = vec![McpTool {
            name: "test".to_string(),
            description: "".to_string(),
            input_schema: serde_json::Value::Null,
        }];

        client.disconnect();
        assert!(!client.is_connected());
        assert!(client.tools_cache.is_empty());
    }

    #[test]
    fn test_mcp_tool_result() {
        let result = McpToolResult {
            content: vec![McpContent::Text {
                text: "result".to_string(),
            }],
            is_error: false,
        };
        assert!(!result.is_error);
        assert_eq!(result.content.len(), 1);

        let error_result = McpToolResult {
            content: vec![],
            is_error: true,
        };
        assert!(error_result.is_error);
    }

    /// 简易 mock HTTP 服务器：依次返回给定响应
    async fn mock_server(responses: Vec<(u16, String)>) -> String {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let last = responses.last().cloned();
            for (status, body) in responses {
                if let Ok((mut socket, _)) = listener.accept().await {
                    let _ = crate::tests::mock_http::read_request_full(&mut socket).await;
                    let reason = if status >= 500 { "Server Error" } else { "OK" };
                    let header = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(body.as_bytes()).await;
                    let _ = socket.flush().await;
                }
            }
            if let Some((status, body)) = last {
                loop {
                    if let Ok((mut socket, _)) = listener.accept().await {
                        let _ = crate::tests::mock_http::read_request_full(&mut socket).await;
                        let reason = if status >= 500 { "Server Error" } else { "OK" };
                        let header = format!(
                            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = socket.write_all(header.as_bytes()).await;
                        let _ = socket.write_all(body.as_bytes()).await;
                        let _ = socket.flush().await;
                    }
                }
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn test_mcp_connect_success() {
        let url = mock_server(vec![(200, "{}".to_string())]).await;
        let mut client = McpClient::new(&url);
        client.connect().await.unwrap();
        assert!(client.is_connected());
    }

    #[tokio::test]
    async fn test_mcp_connect_unexpected_status() {
        let url = mock_server(vec![(404, "{}".to_string())]).await;
        let mut client = McpClient::new(&url);
        assert!(client.connect().await.is_err());
        assert!(!client.is_connected());
    }

    #[tokio::test]
    async fn test_mcp_list_tools_and_cache() {
        let body = r#"{"jsonrpc":"2.0","result":{"tools":[{"name":"echo","description":"Echo","input_schema":{"type":"object"}}]},"id":1}"#;
        let url = mock_server(vec![(200, body.to_string())]).await;
        let mut client = McpClient::new(&url);
        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
        assert_eq!(client.get_tool("echo").unwrap().description, "Echo");

        // 缓存命中：再次调用不发起新请求
        let tools2 = client.list_tools().await.unwrap();
        assert_eq!(tools2.len(), 1);
    }

    #[tokio::test]
    async fn test_mcp_list_tools_error_response() {
        let body = r#"{"jsonrpc":"2.0","error":{"code":-32001,"message":"server error"},"id":1}"#;
        let url = mock_server(vec![(200, body.to_string())]).await;
        let mut client = McpClient::new(&url);
        let err = client.list_tools().await.unwrap_err();
        assert!(err.to_string().contains("server error"));
    }

    #[tokio::test]
    async fn test_mcp_list_tools_http_error() {
        let url = mock_server(vec![(500, "{}".to_string())]).await;
        let mut client = McpClient::new(&url);
        assert!(client.list_tools().await.is_err());
    }

    #[tokio::test]
    async fn test_mcp_call_tool_text() {
        let body = r#"{"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"hello"}],"isError":false},"id":1}"#;
        let url = mock_server(vec![(200, body.to_string())]).await;
        let client = McpClient::new(&url);
        let result = client
            .call_tool(
                "echo",
                &HashMap::from([("msg".to_string(), "hi".to_string())]),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        match &result.content[0] {
            McpContent::Text { text } => assert_eq!(text, "hello"),
            _ => panic!("Expected Text"),
        }
    }

    #[tokio::test]
    async fn test_mcp_call_tool_all_content_types() {
        let body = r#"{"jsonrpc":"2.0","result":{"content":[
            {"type":"text","text":"t"},
            {"type":"image","data":"base64","mimeType":"image/png"},
            {"type":"resource","uri":"file:///x","text":"r"}
        ],"isError":true},"id":1}"#;
        let url = mock_server(vec![(200, body.to_string())]).await;
        let client = McpClient::new(&url);
        let result = client.call_tool("multi", &HashMap::new()).await.unwrap();
        assert!(result.is_error);
        assert_eq!(result.content.len(), 3);
        assert!(matches!(result.content[1], McpContent::Image { .. }));
        assert!(matches!(result.content[2], McpContent::Resource { .. }));
    }

    #[tokio::test]
    async fn test_mcp_call_tool_rpc_error() {
        let body =
            r#"{"jsonrpc":"2.0","error":{"code":-32601,"message":"method not found"},"id":1}"#;
        let url = mock_server(vec![(200, body.to_string())]).await;
        let client = McpClient::new(&url);
        let err = client.call_tool("nope", &HashMap::new()).await.unwrap_err();
        assert!(err.to_string().contains("method not found"));
    }

    #[tokio::test]
    async fn test_mcp_clear_cache_after_list() {
        let body = r#"{"jsonrpc":"2.0","result":{"tools":[{"name":"a","description":"A","input_schema":{}}]},"id":1}"#;
        let url = mock_server(vec![(200, body.to_string())]).await;
        let mut client = McpClient::new(&url);
        let _ = client.list_tools().await.unwrap();
        assert!(!client.tools_cache.is_empty());
        client.clear_cache();
        assert!(client.tools_cache.is_empty());
        assert!(client.get_tool("a").is_none());
    }
}
