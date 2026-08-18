//! MCP 服务器
//!
//! 实现 MCP 标准协议的服务器端，通过 HTTP JSON-RPC 暴露工具给其他 MCP 客户端。
//! 支持 tools/list、tools/call、resources/list、resources/read 等方法。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use super::client::McpTool;
use super::registry::ToolRegistry;

/// MCP 服务器
///
/// 通过 HTTP JSON-RPC 暴露工具，支持标准的 MCP 协议方法。
pub struct McpServer {
    /// 绑定主机
    host: String,
    /// 绑定端口
    port: u16,
    /// 工具注册表
    registry: Arc<ToolRegistry>,
    /// 运行标志
    running: Arc<AtomicBool>,
    /// 服务器任务句柄
    server_handle: Option<tokio::task::JoinHandle<()>>,
    /// 停止信号发送端
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl McpServer {
    /// 创建新的 MCP 服务器
    pub fn new(host: &str, port: u16, registry: Arc<ToolRegistry>) -> Self {
        Self {
            host: host.to_string(),
            port,
            registry,
            running: Arc::new(AtomicBool::new(false)),
            server_handle: None,
            shutdown_tx: None,
        }
    }

    /// 启动服务器
    ///
    /// 绑定地址并开始监听 HTTP 请求。
    pub async fn start(&mut self) -> Result<()> {
        if self.running.load(Ordering::SeqCst) {
            return Err(anyhow!("Server is already running"));
        }

        let addr = format!("{}:{}", self.host, self.port);
        let listener = TcpListener::bind(&addr)
            .await
            .map_err(|e| anyhow!("Failed to bind to {}: {}", addr, e))?;

        self.running.store(true, Ordering::SeqCst);
        let running = self.running.clone();
        let registry = self.registry.clone();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        self.shutdown_tx = Some(shutdown_tx);

        let handle = tokio::spawn(async move {
            run_server(listener, registry, running, shutdown_rx).await;
        });

        self.server_handle = Some(handle);
        Ok(())
    }

    /// 停止服务器
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }

    /// 检查服务器是否在运行
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// 获取服务器 URL
    pub fn url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

/// 服务器主循环
async fn run_server(
    listener: TcpListener,
    registry: Arc<ToolRegistry>,
    running: Arc<AtomicBool>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    tracing::info!("MCP server listening on {}", listener.local_addr().unwrap());

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, addr)) => {
                        tracing::debug!("Accepted connection from {}", addr);
                        let registry = registry.clone();
                        tokio::spawn(async move {
                            handle_connection(stream, registry).await;
                        });
                    }
                    Err(e) => {
                        tracing::error!("Failed to accept connection: {}", e);
                        if !running.load(Ordering::SeqCst) {
                            break;
                        }
                    }
                }
            }
            _ = &mut shutdown_rx => {
                tracing::info!("MCP server shutting down");
                break;
            }
        }

        if !running.load(Ordering::SeqCst) {
            break;
        }
    }
}

/// 处理 HTTP 连接
async fn handle_connection(mut stream: tokio::net::TcpStream, registry: Arc<ToolRegistry>) {
    let (reader, mut writer) = stream.split();
    let mut buf_reader = BufReader::new(reader);
    let mut request_line = String::new();

    // 读取请求行
    if buf_reader.read_line(&mut request_line).await.is_err() {
        return;
    }

    let request_line = request_line.trim();
    if request_line.is_empty() {
        return;
    }

    // 只处理 POST 请求
    if !request_line.starts_with("POST") {
        let response = "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n";
        let _ = writer.write_all(response.as_bytes()).await;
        return;
    }

    // 读取请求头
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        if buf_reader.read_line(&mut line).await.is_err() {
            return;
        }
        let line = line.trim().to_string();
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            if key.trim().to_lowercase() == "content-length" {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }

    // 读取请求体
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        if let Err(e) = buf_reader.read_exact(&mut body).await {
            tracing::error!("Failed to read request body: {}", e);
            let response = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n";
            if let Err(e) = writer.write_all(response.as_bytes()).await {
                tracing::warn!(error = %e, "MCP 响应写入客户端失败");
            }
            return;
        }
    }

    let body_str = String::from_utf8_lossy(&body);
    let response = handle_jsonrpc_request(&body_str, &registry).await;

    // 发送响应
    let response_bytes = response.as_bytes();
    let response_header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        response_bytes.len()
    );

    let _ = writer.write_all(response_header.as_bytes()).await;
    let _ = writer.write_all(response_bytes).await;
    let _ = writer.flush().await;
}

/// 处理 JSON-RPC 请求
async fn handle_jsonrpc_request(body: &str, registry: &ToolRegistry) -> String {
    let json: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return jsonrpc_error(None, -32700, &format!("Parse error: {}", e));
        }
    };

    let id = json.get("id").cloned();
    let method = match json.get("method").and_then(|m| m.as_str()) {
        Some(m) => m,
        None => {
            return jsonrpc_error(id, -32600, "Invalid Request: missing method");
        }
    };

    let params = json.get("params").unwrap_or(&serde_json::Value::Null);

    match method {
        "tools/list" => handle_tools_list(id, registry),
        "tools/call" => handle_tools_call(id, params, registry).await,
        "resources/list" => handle_resources_list(id),
        "resources/read" => handle_resources_read(id, params),
        _ => jsonrpc_error(id, -32601, &format!("Method not found: {}", method)),
    }
}

/// 处理 tools/list 方法
fn handle_tools_list(id: Option<serde_json::Value>, registry: &ToolRegistry) -> String {
    let tool_names = registry.list_tools();
    let tools: Vec<McpTool> = tool_names
        .iter()
        .map(|name| McpTool {
            name: name.clone(),
            description: String::new(),
            input_schema: serde_json::Value::Null,
        })
        .collect();

    let result = serde_json::json!({
        "tools": tools,
    });

    jsonrpc_response(id, result)
}

/// 处理 tools/call 方法
async fn handle_tools_call(
    id: Option<serde_json::Value>,
    params: &serde_json::Value,
    registry: &ToolRegistry,
) -> String {
    let name = match params.get("name").and_then(|n| n.as_str()) {
        Some(n) => n,
        None => {
            return jsonrpc_error(id, -32602, "Invalid params: missing 'name'");
        }
    };

    let arguments = params.get("arguments").unwrap_or(&serde_json::Value::Null);

    // 将 JSON 参数转换为 HashMap<String, String>
    let mut args = HashMap::new();
    if let Some(obj) = arguments.as_object() {
        for (key, value) in obj {
            let val = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            args.insert(key.clone(), val);
        }
    }

    // 执行工具
    let result = match registry.execute_local_tool(name, &args).await {
        Ok(tool_result) => {
            let (text, is_error) = match tool_result {
                crate::agent::tool_cycle::ToolResult::Success { data, .. } => (data, false),
                crate::agent::tool_cycle::ToolResult::Error { message, .. } => (message, true),
                crate::agent::tool_cycle::ToolResult::Timeout { tool_name, .. } => {
                    (format!("Tool '{}' timed out", tool_name), true)
                }
            };

            serde_json::json!({
                "content": [{"type": "text", "text": text}],
                "isError": is_error,
            })
        }
        Err(e) => {
            return jsonrpc_error(id, -32603, &format!("Tool execution error: {}", e));
        }
    };

    jsonrpc_response(id, result)
}

/// 处理 resources/list 方法
fn handle_resources_list(id: Option<serde_json::Value>) -> String {
    let result = serde_json::json!({
        "resources": [],
    });
    jsonrpc_response(id, result)
}

/// 处理 resources/read 方法
fn handle_resources_read(id: Option<serde_json::Value>, _params: &serde_json::Value) -> String {
    jsonrpc_error(id, -32601, "resources/read not implemented")
}

/// 构造 JSON-RPC 成功响应
fn jsonrpc_response(id: Option<serde_json::Value>, result: serde_json::Value) -> String {
    let mut response = serde_json::json!({
        "jsonrpc": "2.0",
        "result": result,
    });
    if let Some(id_val) = id {
        response["id"] = id_val;
    } else {
        response["id"] = serde_json::Value::Null;
    }
    serde_json::to_string(&response).unwrap_or_default()
}

/// 构造 JSON-RPC 错误响应
fn jsonrpc_error(id: Option<serde_json::Value>, code: i32, message: &str) -> String {
    let mut response = serde_json::json!({
        "jsonrpc": "2.0",
        "error": {
            "code": code,
            "message": message,
        },
    });
    if let Some(id_val) = id {
        response["id"] = id_val;
    } else {
        response["id"] = serde_json::Value::Null;
    }
    serde_json::to_string(&response).unwrap_or_default()
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tool_cycle::ToolResult;
    use crate::mcp::registry::ToolExecutor;
    use async_trait::async_trait;
    use std::collections::HashMap;

    struct EchoTool;

    #[async_trait]
    impl ToolExecutor for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "Echo tool for testing"
        }

        async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
            let msg = args.get("message").cloned().unwrap_or_default();
            Ok(ToolResult::Success {
                data: format!("echo: {}", msg),
                metadata: HashMap::new(),
            })
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {"type": "string"}
                }
            })
        }
    }

    fn setup_registry() -> Arc<ToolRegistry> {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(EchoTool));
        Arc::new(registry)
    }

    #[test]
    fn test_jsonrpc_parse_error() {
        let response = jsonrpc_error(None, -32700, "Parse error");
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["error"]["code"], -32700);
        assert_eq!(json["error"]["message"], "Parse error");
        assert!(json["id"].is_null());
    }

    #[test]
    fn test_jsonrpc_error_with_id() {
        let response = jsonrpc_error(Some(serde_json::json!(1)), -32601, "Method not found");
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["id"], 1);
        assert_eq!(json["error"]["code"], -32601);
    }

    #[test]
    fn test_jsonrpc_response() {
        let result = serde_json::json!({"tools": []});
        let response = jsonrpc_response(Some(serde_json::json!(1)), result);
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 1);
        assert!(json["result"]["tools"].is_array());
    }

    #[test]
    fn test_handle_tools_list() {
        let registry = setup_registry();
        let response = handle_tools_list(Some(serde_json::json!(1)), &registry);
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["id"], 1);
        assert!(json["result"]["tools"].is_array());
        let tools = json["result"]["tools"].as_array().unwrap();
        assert!(!tools.is_empty());
        assert!(tools.iter().any(|t| t["name"] == "echo"));
    }

    #[test]
    fn test_handle_tools_list_empty() {
        let registry = Arc::new(ToolRegistry::new());
        let response = handle_tools_list(Some(serde_json::json!(1)), &registry);
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        let tools = json["result"]["tools"].as_array().unwrap();
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn test_handle_tools_call() {
        let registry = setup_registry();
        let params = serde_json::json!({
            "name": "echo",
            "arguments": {"message": "hello"}
        });
        let response = handle_tools_call(Some(serde_json::json!(1)), &params, &registry).await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["id"], 1);
        assert!(!json["result"]["isError"].as_bool().unwrap());
        assert_eq!(json["result"]["content"][0]["text"], "echo: hello");
    }

    #[tokio::test]
    async fn test_handle_tools_call_missing_name() {
        let registry = setup_registry();
        let params = serde_json::json!({});
        let response = handle_tools_call(Some(serde_json::json!(1)), &params, &registry).await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(json.get("error").is_some());
        assert_eq!(json["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn test_handle_tools_call_not_found() {
        let registry = setup_registry();
        let params = serde_json::json!({
            "name": "nonexistent",
            "arguments": {}
        });
        let response = handle_tools_call(Some(serde_json::json!(1)), &params, &registry).await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(json.get("error").is_some());
    }

    #[test]
    fn test_handle_resources_list() {
        let response = handle_resources_list(Some(serde_json::json!(1)));
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(json["result"]["resources"].is_array());
        assert!(json["result"]["resources"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_handle_resources_read() {
        let params = serde_json::json!({"uri": "test://resource"});
        let response = handle_resources_read(Some(serde_json::json!(1)), &params);
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(json.get("error").is_some());
        assert_eq!(json["error"]["code"], -32601);
    }

    #[test]
    fn test_handle_method_not_found() {
        let registry = Arc::new(ToolRegistry::new());
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"unknown/method","params":{}}"#;
        let response = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handle_jsonrpc_request(body, &registry));
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(json.get("error").is_some());
        assert_eq!(json["error"]["code"], -32601);
    }

    #[test]
    fn test_handle_invalid_json() {
        let registry = Arc::new(ToolRegistry::new());
        let response = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handle_jsonrpc_request("not json", &registry));
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(json.get("error").is_some());
        assert_eq!(json["error"]["code"], -32700);
    }

    #[test]
    fn test_mcp_server_new() {
        let registry = Arc::new(ToolRegistry::new());
        let server = McpServer::new("127.0.0.1", 9999, registry);
        assert!(!server.is_running());
        assert_eq!(server.url(), "http://127.0.0.1:9999");
    }

    #[tokio::test]
    async fn test_mcp_server_start_stop() {
        // 使用已知端口测试 start/stop 逻辑
        let mut server = McpServer::new("127.0.0.1", 19999, setup_registry());
        let result = server.start().await;
        // 可能端口被占用，但至少测试了逻辑
        if result.is_ok() {
            assert!(server.is_running());
            server.stop();
            assert!(!server.is_running());
        }
    }

    #[test]
    fn test_mcp_server_url() {
        let registry = Arc::new(ToolRegistry::new());
        let server = McpServer::new("0.0.0.0", 8080, registry);
        assert_eq!(server.url(), "http://0.0.0.0:8080");
    }

    /// 获取一个空闲端口
    async fn free_port() -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    #[tokio::test]
    async fn test_server_end_to_end_http() {
        let port = free_port().await;
        let mut server = McpServer::new("127.0.0.1", port, setup_registry());
        server.start().await.unwrap();
        assert!(server.is_running());

        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{port}");

        // tools/list
        let body = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}});
        let resp = client.post(&url).json(&body).send().await.unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        let json: serde_json::Value = resp.json().await.unwrap();
        assert!(json["result"]["tools"].is_array());

        // tools/call
        let body = serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"echo","arguments":{"message":"hi"}}});
        let resp = client.post(&url).json(&body).send().await.unwrap();
        let json: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(json["result"]["content"][0]["text"], "echo: hi");

        // resources/list
        let body =
            serde_json::json!({"jsonrpc":"2.0","id":3,"method":"resources/list","params":{}});
        let resp = client.post(&url).json(&body).send().await.unwrap();
        let json: serde_json::Value = resp.json().await.unwrap();
        assert!(json["result"]["resources"].is_array());

        // 未知方法
        let body = serde_json::json!({"jsonrpc":"2.0","id":4,"method":"nope","params":{}});
        let resp = client.post(&url).json(&body).send().await.unwrap();
        let json: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(json["error"]["code"], -32601);

        // 非法 JSON → 解析错误
        let resp = client
            .post(&url)
            .body("not json")
            .header("content-type", "application/json")
            .send()
            .await
            .unwrap();
        let json: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(json["error"]["code"], -32700);

        // GET → 405
        let resp = client.get(&url).send().await.unwrap();
        assert_eq!(resp.status().as_u16(), 405);

        server.stop();
        assert!(!server.is_running());
    }

    #[tokio::test]
    async fn test_server_double_start_fails() {
        let port = free_port().await;
        let mut server = McpServer::new("127.0.0.1", port, setup_registry());
        server.start().await.unwrap();
        assert!(server.start().await.is_err());
        server.stop();
    }

    #[test]
    fn test_jsonrpc_response_null_id() {
        let result = serde_json::json!({"ok": true});
        let response = jsonrpc_response(None, result);
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(json["id"].is_null());
    }

    #[test]
    fn test_handle_jsonrpc_request_missing_method() {
        let registry = Arc::new(ToolRegistry::new());
        let body = r#"{"jsonrpc":"2.0","id":1,"params":{}}"#;
        let response = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handle_jsonrpc_request(body, &registry));
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(json.get("error").is_some());
        assert_eq!(json["error"]["code"], -32600);
    }

    #[test]
    fn test_tools_call_without_arguments() {
        let registry = setup_registry();
        let params = serde_json::json!({
            "name": "echo"
        });
        let response = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handle_tools_call(
                Some(serde_json::json!(1)),
                &params,
                &registry,
            ));
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(!json["result"]["isError"].as_bool().unwrap());
        assert_eq!(json["result"]["content"][0]["text"], "echo: ");
    }
}
