//! MCP 客户端
//!
//! 实现 Model Context Protocol 客户端，用于连接 MCP Server 发现和调用工具。
//! 支持 HTTP JSON-RPC 与 stdio（子进程 JSON-RPC over stdin/stdout）两种传输。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command as TokioCommand};

use std::process::Stdio;

use crate::proc::SilentCommand;

/// stdio MCP 单次读取的超时。一个挂死的 MCP server 不得让握手 / tools/list /
/// tools/call 无限阻塞（HTTP 路径早已有超时，stdio 此前没有）。
pub const STDIO_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// 把 MCP 工具名清洗为 LLM function-calling 允许的字符集（字母数字 / `_` / `-`）。
///
/// ⚠️ 这是**单一事实源**：`describe_mcp_tools`（对外广告的名字）与
/// `McpClient::get_tool`（派发时的查名）必须用同一个函数。历史缺陷：前者清洗、
/// 后者按原始名查 —— 任何含 `.` `/` 的 MCP 工具（如 `fs.read`）会被广告成
/// `fs_read` 却永远派发失败（静默 "Tool not found"）。
pub fn sanitize_mcp_tool_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

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

/// stdio 型 MCP 服务器规格（子进程命令 + 参数 + 环境）。
#[derive(Debug, Clone)]
pub struct McpStdioSpec {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

/// 单次 MCP 调用失败的分类：决定是否值得「重连一次」。
///
/// - `Transport`：连接 / 发送 / 接收 / 响应体读取失败，或 HTTP 非 2xx——
///   服务器可能只是死了，重连一次有意义。
/// - `Protocol`：服务器正常应答但内容不合法（JSON-RPC `error` 对象、畸形 result）——
///   重连无益，直接上报（D2/D4）。
#[derive(Debug)]
enum McpCallError {
    Transport(anyhow::Error),
    Protocol(anyhow::Error),
}

impl McpCallError {
    fn is_transport(&self) -> bool {
        matches!(self, McpCallError::Transport(_))
    }

    fn into_error(self) -> anyhow::Error {
        match self {
            McpCallError::Transport(e) | McpCallError::Protocol(e) => e,
        }
    }
}

/// MCP 客户端
///
/// 通过 HTTP JSON-RPC 或 stdio（子进程）与 MCP Server 通信，支持工具列表缓存。
#[derive(Debug, Clone)]
pub struct McpClient {
    /// MCP Server URL（HTTP 模式）
    server_url: String,
    /// HTTP 客户端
    client: reqwest::Client,
    /// stdio 模式规格（Some = 子进程传输）
    pub(crate) stdio: Option<McpStdioSpec>,
    /// 工具列表缓存
    pub(crate) tools_cache: Vec<McpTool>,
    /// 缓存 TTL
    cache_ttl: Duration,
    /// 上次缓存时间
    last_cache_time: Instant,
    /// 连接状态（`Arc<AtomicBool>` 允许 `&self` 调用路径在传输失败后如实更新状态）
    connected: Arc<AtomicBool>,
}

impl McpClient {
    /// 创建 HTTP MCP 客户端
    pub fn new(server_url: &str) -> Self {
        Self {
            server_url: server_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
            stdio: None,
            tools_cache: Vec::new(),
            cache_ttl: Duration::from_secs(300), // 5 分钟
            last_cache_time: Instant::now(),
            connected: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 创建 stdio MCP 客户端（子进程 JSON-RPC over stdin/stdout）
    pub fn new_stdio(command: &str, args: &[String], env: HashMap<String, String>) -> Self {
        Self {
            server_url: format!("stdio:{}", command),
            client: reqwest::Client::new(),
            stdio: Some(McpStdioSpec {
                command: command.to_string(),
                args: args.to_vec(),
                env,
            }),
            tools_cache: Vec::new(),
            cache_ttl: Duration::from_secs(300),
            last_cache_time: Instant::now(),
            connected: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_stdio(&self) -> bool {
        self.stdio.is_some()
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

    /// stdio 握手：启动子进程 → initialize(id=0) → notifications/initialized。
    async fn stdio_handshake(&self) -> Result<(Child, ChildStdin, BufReader<ChildStdout>)> {
        let spec = self
            .stdio
            .as_ref()
            .ok_or_else(|| anyhow!("非 stdio 客户端"))?;
        let mut cmd = TokioCommand::new(&spec.command);
        cmd.silent();
        cmd.args(&spec.args);
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("启动 stdio MCP 失败({}): {}", spec.command, e))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("stdio MCP 无 stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("stdio MCP 无 stdout"))?;
        let mut reader = BufReader::new(stdout);

        let init = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "marguerite", "version": "0.1.5" },
            },
        });
        Self::stdio_send(&mut stdin, &init).await?;
        let _resp = Self::stdio_read(&mut reader, 0).await?;
        let notif = serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        Self::stdio_send(&mut stdin, &notif).await?;
        Ok((child, stdin, reader))
    }

    async fn stdio_send(stdin: &mut ChildStdin, req: &serde_json::Value) -> Result<()> {
        let mut buf = serde_json::to_vec(req).unwrap_or_default();
        buf.push(b'\n');
        stdin.write_all(&buf).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn stdio_read<R: tokio::io::AsyncBufRead + Unpin>(
        reader: &mut R,
        expected_id: u64,
    ) -> Result<serde_json::Value> {
        let mut line = String::new();
        loop {
            line.clear();
            // ⚠️ stdio 读必须有超时：一个挂死/不回包的 MCP server 会让 `read_line`
            // 永久阻塞，进而卡死握手、tools/list 与每一次 tools/call（HTTP 路径早有
            // 超时，stdio 此前没有）。超时后返回错误，让上层能放弃该 server。
            let n = match tokio::time::timeout(
                STDIO_READ_TIMEOUT,
                reader.read_line(&mut line),
            )
            .await
            {
                Ok(res) => res?,
                Err(_) => {
                    return Err(anyhow!(
                        "stdio MCP 读取超时（{}s，等待 id={expected_id}）",
                        STDIO_READ_TIMEOUT.as_secs()
                    ))
                }
            };
            if n == 0 {
                return Err(anyhow!("stdio MCP 进程关闭，未返回 id={expected_id}"));
            }
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            let v: serde_json::Value =
                serde_json::from_str(t).map_err(|e| anyhow!("stdio MCP 非 JSON 行: {e}"))?;
            if v["id"].as_u64() == Some(expected_id) {
                return Ok(v);
            }
        }
    }

    /// 连接 MCP Server（HTTP=根路径探活 / stdio=握手后退出）
    pub async fn connect(&mut self) -> Result<()> {
        self.handshake().await
    }

    /// 握手（不依赖 `&mut self`，供 `&self` 重连路径复用）。
    async fn handshake(&self) -> Result<()> {
        if self.is_stdio() {
            let (mut child, _, _) = self.stdio_handshake().await?;
            let _ = child.kill().await;
            self.connected.store(true, Ordering::SeqCst);
            return Ok(());
        }
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
            self.connected.store(true, Ordering::SeqCst);
            Ok(())
        } else {
            Err(anyhow!(
                "MCP server returned unexpected status: {}",
                resp.status()
            ))
        }
    }

    /// 断开连接：清除连接状态与工具缓存。
    ///
    /// 生产调用者：`list_tools` 在「传输失败 → 一次重连后仍失败」时调用本方法，
    /// 使对外报告的连接状态与真实状态一致（D2）。`call_tool` 走 `&self`，
    /// 只能清连接标志（见 `mark_disconnected`）。
    pub fn disconnect(&mut self) {
        self.mark_disconnected();
        self.tools_cache.clear();
    }

    /// 仅清除连接状态（不动工具缓存，可在 `&self` 重连路径中调用）。
    fn mark_disconnected(&self) {
        self.connected.store(false, Ordering::SeqCst);
    }

    /// 获取工具列表（带缓存）
    ///
    /// 缓存有效期内直接返回缓存，否则重新请求服务器。
    /// 传输层失败且当前为「已连接」时，清连接状态并**重连一次**再重试（有界）。
    pub async fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        // 检查缓存是否有效
        if !self.tools_cache.is_empty() && self.last_cache_time.elapsed() < self.cache_ttl {
            return Ok(self.tools_cache.clone());
        }

        match self.fetch_tools().await {
            Ok(tools) => {
                self.tools_cache = tools.clone();
                self.last_cache_time = Instant::now();
                Ok(tools)
            }
            Err(first) if first.is_transport() && self.is_connected() => {
                tracing::warn!(error = ?first, "MCP list_tools 传输失败，尝试一次重连");
                // 清连接状态 + 缓存（disconnect 的生产调用者），再做一次重握手。
                self.disconnect();
                match self.handshake().await {
                    Ok(()) => match self.fetch_tools().await {
                        Ok(tools) => {
                            self.tools_cache = tools.clone();
                            self.last_cache_time = Instant::now();
                            Ok(tools)
                        }
                        Err(second) => {
                            // 重连后仍失败 → 状态置为未连接（真话）。
                            self.disconnect();
                            Err(second.into_error())
                        }
                    },
                    Err(re) => {
                        self.disconnect();
                        Err(anyhow!(
                            "MCP list_tools 重连失败: {re}; 原始错误: {}",
                            first.into_error()
                        ))
                    }
                }
            }
            Err(e) => Err(e.into_error()),
        }
    }

    /// 从服务器获取工具列表（HTTP=post JSON-RPC / stdio=子进程 JSON-RPC，单次无重连）
    async fn fetch_tools(&self) -> Result<Vec<McpTool>, McpCallError> {
        if self.is_stdio() {
            let (mut child, mut stdin, mut reader) = self
                .stdio_handshake()
                .await
                .map_err(McpCallError::Transport)?;
            let request = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "tools/list",
                "params": {},
                "id": 1,
            });
            Self::stdio_send(&mut stdin, &request)
                .await
                .map_err(McpCallError::Transport)?;
            let json = Self::stdio_read(&mut reader, 1)
                .await
                .map_err(McpCallError::Transport)?;
            let _ = child.kill().await;
            if let Some(error) = json.get("error") {
                return Err(McpCallError::Protocol(anyhow!(
                    "MCP error ({}): {}",
                    error["code"].as_i64().unwrap_or(0),
                    error["message"].as_str().unwrap_or("unknown")
                )));
            }
            let tools: Vec<McpTool> = serde_json::from_value(json["result"]["tools"].clone())
                .map_err(|e| {
                    McpCallError::Protocol(anyhow!("Failed to parse stdio tools list: {}", e))
                })?;
            return Ok(tools);
        }

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
            .map_err(|e| McpCallError::Transport(anyhow!("Failed to list tools: {}", e)))?;

        if !resp.status().is_success() {
            return Err(McpCallError::Transport(anyhow!(
                "MCP server error: {}",
                resp.status()
            )));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| McpCallError::Transport(anyhow!("Failed to parse MCP response: {}", e)))?;

        // 检查 JSON-RPC 错误
        if let Some(error) = json.get("error") {
            return Err(McpCallError::Protocol(anyhow!(
                "MCP error ({}): {}",
                error["code"].as_i64().unwrap_or(0),
                error["message"].as_str().unwrap_or("unknown")
            )));
        }

        let tools: Vec<McpTool> = serde_json::from_value(json["result"]["tools"].clone())
            .map_err(|e| McpCallError::Protocol(anyhow!("Failed to parse tools list: {}", e)))?;

        Ok(tools)
    }

    /// 调用工具（HTTP=post JSON-RPC / stdio=子进程 JSON-RPC）
    ///
    /// 传输层失败且当前为「已连接」时，清连接状态并**重连一次**再重试（有界，绝不无限重试）。
    pub async fn call_tool(
        &self,
        name: &str,
        args: &HashMap<String, String>,
    ) -> Result<McpToolResult> {
        match self.call_tool_once(name, args).await {
            Ok(r) => Ok(r),
            Err(first) if first.is_transport() && self.is_connected() => {
                tracing::warn!(
                    tool = name,
                    error = ?first,
                    "MCP 工具调用传输失败，尝试一次重连"
                );
                // 清连接状态 + 一次重握手。
                self.mark_disconnected();
                match self.handshake().await {
                    Ok(()) => match self.call_tool_once(name, args).await {
                        Ok(r) => Ok(r),
                        Err(second) => {
                            // 重连后仍失败 → 连接状态不可信，标记为未连接（真话）。
                            self.mark_disconnected();
                            Err(second.into_error())
                        }
                    },
                    Err(re) => {
                        self.mark_disconnected();
                        Err(anyhow!(
                            "MCP 工具 '{}' 重连失败: {re}; 原始错误: {}",
                            name,
                            first.into_error()
                        ))
                    }
                }
            }
            Err(e) => Err(e.into_error()),
        }
    }

    /// 单次工具调用（无重连）。
    async fn call_tool_once(
        &self,
        name: &str,
        args: &HashMap<String, String>,
    ) -> Result<McpToolResult, McpCallError> {
        if self.is_stdio() {
            let (mut child, mut stdin, mut reader) = self
                .stdio_handshake()
                .await
                .map_err(McpCallError::Transport)?;
            let request = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "tools/call",
                "params": { "name": name, "arguments": args },
                "id": 1,
            });
            Self::stdio_send(&mut stdin, &request)
                .await
                .map_err(McpCallError::Transport)?;
            let json = Self::stdio_read(&mut reader, 1)
                .await
                .map_err(McpCallError::Transport)?;
            let _ = child.kill().await;
            if let Some(error) = json.get("error") {
                return Err(McpCallError::Protocol(anyhow!(
                    "MCP error calling '{}' ({}): {}",
                    name,
                    error["code"].as_i64().unwrap_or(0),
                    error["message"].as_str().unwrap_or("unknown")
                )));
            }
            return Self::parse_mcp_result(&json["result"]);
        }

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
            .map_err(|e| {
                McpCallError::Transport(anyhow!("Failed to call tool '{}': {}", name, e))
            })?;

        if !resp.status().is_success() {
            return Err(McpCallError::Transport(anyhow!(
                "MCP server error when calling '{}': {}",
                name,
                resp.status()
            )));
        }

        let json: serde_json::Value = resp.json().await.map_err(|e| {
            McpCallError::Transport(anyhow!("Failed to parse tool call response: {}", e))
        })?;

        // 检查 JSON-RPC 错误
        if let Some(error) = json.get("error") {
            return Err(McpCallError::Protocol(anyhow!(
                "MCP error calling '{}' ({}): {}",
                name,
                error["code"].as_i64().unwrap_or(0),
                error["message"].as_str().unwrap_or("unknown")
            )));
        }

        Self::parse_mcp_result(&json["result"])
    }

    /// 解析 MCP tools/call 结果内容（HTTP/stdio 共用）。
    ///
    /// D4：缺失 / 畸形的 `result` 必须报错，而不是静默当成「空成功」。
    /// `content` 字段必须存在且为数组；合法的空数组 `[]` 仍是有效空结果。
    fn parse_mcp_result(result: &serde_json::Value) -> Result<McpToolResult, McpCallError> {
        if result.is_null() {
            return Err(McpCallError::Protocol(anyhow!(
                "MCP 结果缺失 result 对象（畸形响应）"
            )));
        }
        let content_arr = result
            .get("content")
            .and_then(|c| c.as_array())
            .ok_or_else(|| {
                McpCallError::Protocol(anyhow!("MCP 结果缺少 'content' 数组（畸形响应）"))
            })?;
        let is_error = result["isError"].as_bool().unwrap_or(false);
        let content: Vec<McpContent> = content_arr
            .iter()
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
            .collect();
        Ok(McpToolResult { content, is_error })
    }

    /// 获取特定工具信息（从缓存中查找）
    pub fn get_tool(&self, name: &str) -> Option<McpTool> {
        // 先按原始名精确匹配；再按**清洗后的名**匹配 —— LLM 看到的是清洗名
        // （见 `describe_mcp_tools`），只按原始名查会让含 `.` `/` 的工具
        // 「被广告却无法派发」。两种都支持，覆盖两种调用来源。
        if let Some(t) = self.tools_cache.iter().find(|t| t.name == name) {
            return Some(t.clone());
        }
        self.tools_cache
            .iter()
            .find(|t| sanitize_mcp_tool_name(&t.name) == name)
            .cloned()
    }

    /// 清除缓存
    pub fn clear_cache(&mut self) {
        self.tools_cache.clear();
        self.last_cache_time = Instant::now();
    }

    /// 检查是否已连接
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
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
    fn test_stdio_client_ctor() {
        let client = McpClient::new_stdio(
            "npx",
            &["-y".to_string(), "mcp-server".to_string()],
            HashMap::new(),
        );
        assert!(client.is_stdio());
        assert!(client.server_url.starts_with("stdio:"));
        assert!(client.stdio.is_some());
    }

    /// 回归守卫：对外广告的名字（清洗名）与派发时的查名必须一致。
    /// 历史缺陷：`describe_mcp_tools` 清洗、`get_tool` 按原始名查 → 含 `.` 的
    /// MCP 工具（如 `fs.read`）被广告成 `fs_read` 却永远派发失败。
    #[test]
    fn sanitized_name_is_dispatchable() {
        assert_eq!(sanitize_mcp_tool_name("fs.read"), "fs_read");
        assert_eq!(sanitize_mcp_tool_name("a/b-c_1"), "a_b-c_1");

        let mut client = McpClient::new("http://localhost:8080");
        client.tools_cache = vec![McpTool {
            name: "fs.read".to_string(),
            description: "read a file".to_string(),
            input_schema: serde_json::json!({}),
        }];
        assert!(client.get_tool("fs.read").is_some(), "原始名仍应可命中");
        assert!(
            client.get_tool("fs_read").is_some(),
            "清洗名必须可命中（否则被广告却无法派发）"
        );
        assert!(client.get_tool("nope").is_none());
    }

    #[test]
    fn test_parse_mcp_result_text_and_error() {
        let v = serde_json::json!({
            "content": [{"type": "text", "text": "hello"}],
            "isError": false,
        });
        let r = McpClient::parse_mcp_result(&v).unwrap();
        assert_eq!(r.content.len(), 1);
        assert!(!r.is_error);
        let e = serde_json::json!({ "content": [], "isError": true });
        let r2 = McpClient::parse_mcp_result(&e).unwrap();
        assert!(r2.is_error);
    }

    /// D4：缺失 `content` 字段必须报错，而不是静默的空成功。
    #[test]
    fn test_parse_mcp_result_missing_content_is_error() {
        let v = serde_json::json!({ "isError": false });
        assert!(
            McpClient::parse_mcp_result(&v).is_err(),
            "缺少 content 字段应报错"
        );
    }

    /// D4：`result` 整体缺失时传入的是 `Value::Null`，也必须报错。
    #[test]
    fn test_parse_mcp_result_null_is_error() {
        assert!(McpClient::parse_mcp_result(&serde_json::Value::Null).is_err());
    }

    /// D4：合法的空 `content` 数组仍是有效空结果（不得误判为错误）。
    #[test]
    fn test_parse_mcp_result_empty_content_is_valid() {
        let v = serde_json::json!({ "content": [], "isError": false });
        let r = McpClient::parse_mcp_result(&v).unwrap();
        assert!(r.content.is_empty());
        assert!(!r.is_error);
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
        client.connected.store(true, Ordering::SeqCst);
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

    /// D4：HTTP 200 但 result 缺少 `content` → 必须报错，而不是「空成功」。
    #[tokio::test]
    async fn test_mcp_call_tool_malformed_result_is_error() {
        let body = r#"{"jsonrpc":"2.0","result":{"isError":false},"id":1}"#;
        let url = mock_server(vec![(200, body.to_string())]).await;
        let client = McpClient::new(&url);
        let err = client.call_tool("echo", &HashMap::new()).await.unwrap_err();
        assert!(
            err.to_string().contains("content"),
            "畸形结果应报错并提及 content: {err}"
        );
    }

    /// D2：首次 tools/call 传输失败、第二次成功 → 客户端重连一次并恢复。
    #[tokio::test]
    async fn test_call_tool_reconnects_once_and_recovers() {
        let result_body = r#"{"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"recovered"}],"isError":false},"id":1}"#;
        // 顺序：connect 探活(200) → 首次 call_tool(500) → 重连探活(200) → 重试 call_tool(200 结果)
        let url = mock_server(vec![
            (200, "{}".to_string()),
            (500, "{}".to_string()),
            (200, "{}".to_string()),
            (200, result_body.to_string()),
        ])
        .await;
        let mut client = McpClient::new(&url);
        client.connect().await.unwrap();
        assert!(client.is_connected());

        let result = client.call_tool("echo", &HashMap::new()).await.unwrap();
        match &result.content[0] {
            McpContent::Text { text } => assert_eq!(text, "recovered"),
            _ => panic!("Expected Text"),
        }
        assert!(client.is_connected(), "重连成功后应保持已连接");
    }

    /// D2：服务器持续失败 → 只重试一次即放弃，并报告「未连接」。
    #[tokio::test]
    async fn test_call_tool_gives_up_after_one_retry_when_dead() {
        // connect 探活成功(200)；此后所有 tools/call 均 500。
        let url = mock_server(vec![(200, "{}".to_string()), (500, "{}".to_string())]).await;
        let mut client = McpClient::new(&url);
        client.connect().await.unwrap();
        assert!(client.is_connected());

        let err = client.call_tool("echo", &HashMap::new()).await.unwrap_err();
        assert!(
            err.to_string().contains("MCP server error"),
            "err: {err}"
        );
        assert!(
            !client.is_connected(),
            "放弃后必须报告未连接（状态要真话）"
        );
    }

    /// D2：`list_tools` 同样有一次重连（并借此为 `disconnect()` 提供生产调用者）。
    #[tokio::test]
    async fn test_list_tools_reconnects_once_and_recovers() {
        let tools_body = r#"{"jsonrpc":"2.0","result":{"tools":[{"name":"echo","description":"Echo","input_schema":{}}]},"id":1}"#;
        let url = mock_server(vec![
            (200, "{}".to_string()),              // connect 探活
            (500, "{}".to_string()),              // 首次 list_tools 失败
            (200, "{}".to_string()),              // 重连探活
            (200, tools_body.to_string()),        // 重试 list_tools 成功
        ])
        .await;
        let mut client = McpClient::new(&url);
        client.connect().await.unwrap();

        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
        assert!(client.is_connected());
    }
}
