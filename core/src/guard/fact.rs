//! 事实检查（护栏第三层）
//!
//! 针对模型最易幻觉的 API 用法进行校验：
//! 1. 内置规则库（常见 API 误用模式，借鉴 NexusStar code_contract 思路）
//! 2. 可选 MCP 工具查询（动态获取 API 文档）

use std::collections::HashMap;

use regex::Regex;

use super::{GuardIssue, GuardLayer, Severity};
use crate::guard::syntax::Language;
use crate::mcp::client::McpClient;

/// 事实规则：正则模式 → 修复建议
struct FactRule {
    /// 触发模式（检测到即提示）
    pattern: Regex,
    /// 问题描述
    message: &'static str,
    /// 修复建议
    suggestion: &'static str,
    /// 若代码包含任一子串则豁免（避免误报）
    ignore_if: &'static [&'static str],
}

impl FactRule {
    fn new(
        pattern: &str,
        message: &'static str,
        suggestion: &'static str,
        ignore_if: &'static [&'static str],
    ) -> Self {
        Self {
            pattern: Regex::new(pattern).unwrap(),
            message,
            suggestion,
            ignore_if,
        }
    }
}

/// 事实检查器
pub struct FactChecker {
    rules: Vec<FactRule>,
    /// 可选 MCP 客户端（用于查询在线文档）
    mcp: Option<McpClient>,
}

impl FactChecker {
    pub fn new() -> Self {
        Self {
            rules: Self::builtin_rules(),
            mcp: None,
        }
    }

    /// 绑定 MCP 客户端
    pub fn with_mcp(mut self, mcp: McpClient) -> Self {
        self.mcp = Some(mcp);
        self
    }

    /// 事实检查（仅本地规则，零延迟）
    pub fn check(&self, code: &str, _lang: Language) -> Vec<GuardIssue> {
        self.rules
            .iter()
            .filter_map(|rule| {
                let exempt = rule.ignore_if.iter().any(|s| code.contains(s));
                if exempt {
                    return None;
                }
                rule.pattern.find(code).map(|_| {
                    GuardIssue::new(
                        GuardLayer::Fact,
                        Severity::Warning,
                        rule.message,
                        None,
                        Some(rule.suggestion.to_string()),
                    )
                })
            })
            .collect()
    }

    /// 通过 MCP 工具查询 API 文档（异步，可选启用）
    ///
    /// 调用 MCP 服务器提供的 `lookup_api` 工具校验用法。
    pub async fn check_with_mcp(&self, code: &str) -> Vec<GuardIssue> {
        let Some(mcp) = &self.mcp else {
            return vec![];
        };
        // 克隆以获取可变引用（McpClient 为轻量共享结构）
        let mut mcp = mcp.clone();
        if !mcp.is_connected() {
            return vec![];
        }
        let Ok(tools) = mcp.list_tools().await else {
            return vec![];
        };
        if !tools.iter().any(|t| t.name == "lookup_api") {
            return vec![];
        }
        let mut args = HashMap::new();
        args.insert("code".to_string(), code.to_string());
        let Ok(result) = mcp.call_tool("lookup_api", &args).await else {
            return vec![];
        };
        let mut issues = Vec::new();
        for content in result.content {
            if let crate::mcp::client::McpContent::Text { text } = content {
                issues.push(GuardIssue::new(
                    GuardLayer::Fact,
                    Severity::Warning,
                    text,
                    None,
                    None,
                ));
            }
        }
        issues
    }

    /// 内置规则库
    fn builtin_rules() -> Vec<FactRule> {
        vec![
            FactRule::new(
                r"readFileSync\([^)]*\)",
                "readFileSync 默认返回 Buffer，读文本文件应传 encoding",
                "fs.readFileSync(path, 'utf8')",
                &["'utf8'", "\"utf8\"", "encoding"],
            ),
            FactRule::new(
                r"writeFileSync\([^)]*\)",
                "writeFileSync 写文本建议显式指定 encoding",
                "fs.writeFileSync(path, data, 'utf8')",
                &["'utf8'", "\"utf8\"", "encoding"],
            ),
            FactRule::new(
                r"JSON\.parse\([^)]*\)",
                "JSON.parse 可能抛异常，建议 try/catch 包裹",
                "try { JSON.parse(s) } catch { ... }",
                &["try", "catch"],
            ),
            FactRule::new(
                r"setTimeout\(([^,)]+),\s*\d*\s*\)",
                "setTimeout 回调参数顺序为 (fn, delay)，注意不要传反",
                "setTimeout(fn, 1000)",
                &[],
            ),
            FactRule::new(
                r"\.map\([^)]*\)\.forEach",
                "map 后再 forEach 会创建中间数组，直接 forEach 即可",
                "arr.forEach(...)",
                &[],
            ),
            FactRule::new(
                r"fetch\([^)]*\)",
                "fetch 返回 Promise，需 await 或 .then 处理",
                "await fetch(url) 或 fetch(url).then(...)",
                &["await", ".then("],
            ),
        ]
    }
}

impl Default for FactChecker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_file_encoding() {
        let checker = FactChecker::new();
        let issues = checker.check(
            "const data = fs.readFileSync('./a.txt');",
            Language::JavaScript,
        );
        assert!(!issues.is_empty());
        assert!(issues[0].message.contains("encoding"));
    }

    #[test]
    fn test_fetch_without_await() {
        let checker = FactChecker::new();
        let issues = checker.check("fetch('/api/user');", Language::JavaScript);
        assert!(!issues.is_empty());
    }

    #[test]
    fn test_fetch_with_await_no_issue() {
        let checker = FactChecker::new();
        let issues = checker.check("const r = await fetch('/api/user');", Language::JavaScript);
        assert!(issues.is_empty(), "expected no issues, got {issues:?}");
    }

    #[test]
    fn test_clean_code_no_issue() {
        let checker = FactChecker::new();
        let issues = checker.check("const x = 1 + 2;", Language::JavaScript);
        assert!(issues.is_empty());
    }

    #[test]
    fn test_ignore_if_exemption() {
        let checker = FactChecker::new();
        // readFileSync 带 'utf8' 豁免
        let issues = checker.check("fs.readFileSync('./a.txt', 'utf8');", Language::JavaScript);
        assert!(issues.is_empty());
        // writeFileSync 带 encoding 豁免
        let issues = checker.check(
            "fs.writeFileSync(p, d, { encoding: 'utf8' });",
            Language::JavaScript,
        );
        assert!(issues.is_empty());
        // JSON.parse 在 try/catch 内豁免
        let issues = checker.check("try { JSON.parse(s) } catch (e) {}", Language::JavaScript);
        assert!(issues.is_empty());
    }

    #[test]
    fn test_timeout_pattern() {
        let checker = FactChecker::new();
        let issues = checker.check("setTimeout(fn, 1000);", Language::JavaScript);
        assert!(!issues.is_empty());
        assert!(issues[0].message.contains("setTimeout"));
    }

    #[test]
    fn test_map_for_each_pattern() {
        let checker = FactChecker::new();
        let issues = checker.check(
            "arr.map(x => x * 2).forEach(console.log);",
            Language::JavaScript,
        );
        assert!(!issues.is_empty());
    }

    /// 启动一个按请求方法返回不同 JSON-RPC 响应的 mock MCP 服务器
    async fn mock_mcp_server(list_has_lookup: bool, call_error: bool) -> String {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let data = crate::tests::mock_http::read_request_full(&mut socket).await;
                let req = String::from_utf8_lossy(&data);
                let body = if req.contains("tools/list") {
                    let tools = if list_has_lookup {
                        serde_json::json!([{"name": "lookup_api", "description": "d", "input_schema": {}}])
                    } else {
                        serde_json::json!([{"name": "other_tool", "description": "d", "input_schema": {}}])
                    };
                    serde_json::json!({"jsonrpc": "2.0", "result": {"tools": tools}, "id": 1})
                        .to_string()
                } else if req.contains("tools/call") {
                    if call_error {
                        serde_json::json!({"jsonrpc": "2.0", "error": {"code": -32000, "message": "boom"}, "id": 1}).to_string()
                    } else {
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "result": {"content": [{"type": "text", "text": "lookup_api 提示：readFileSync 应传 encoding"}], "isError": false},
                            "id": 1
                        })
                        .to_string()
                    }
                } else {
                    serde_json::json!({"jsonrpc": "2.0", "result": {}, "id": 1}).to_string()
                };
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(body.as_bytes()).await;
                let _ = socket.flush().await;
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn test_check_with_mcp_no_client() {
        let checker = FactChecker::new();
        let issues = checker.check_with_mcp("fs.readFileSync('./a.txt');").await;
        assert!(issues.is_empty());
    }

    #[tokio::test]
    async fn test_check_with_mcp_not_connected() {
        let checker = FactChecker::new().with_mcp(McpClient::new("http://127.0.0.1:1"));
        let issues = checker.check_with_mcp("fs.readFileSync('./a.txt');").await;
        assert!(issues.is_empty());
    }

    #[tokio::test]
    async fn test_check_with_mcp_no_lookup_tool() {
        let url = mock_mcp_server(false, false).await;
        let mut client = McpClient::new(&url);
        client.connect().await.unwrap();
        let checker = FactChecker::new().with_mcp(client);
        let issues = checker.check_with_mcp("fs.readFileSync('./a.txt');").await;
        assert!(issues.is_empty());
    }

    #[tokio::test]
    async fn test_check_with_mcp_full() {
        let url = mock_mcp_server(true, false).await;
        let mut client = McpClient::new(&url);
        client.connect().await.unwrap();
        let checker = FactChecker::new().with_mcp(client);
        let issues = checker.check_with_mcp("fs.readFileSync('./a.txt');").await;
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("lookup_api"));
    }

    #[tokio::test]
    async fn test_check_with_mcp_call_error() {
        let url = mock_mcp_server(true, true).await;
        let mut client = McpClient::new(&url);
        client.connect().await.unwrap();
        let checker = FactChecker::new().with_mcp(client);
        let issues = checker.check_with_mcp("fs.readFileSync('./a.txt');").await;
        assert!(issues.is_empty());
    }
}
