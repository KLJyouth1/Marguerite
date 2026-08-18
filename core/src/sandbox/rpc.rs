//! 沙箱 JSON-RPC 双通道（v2 工作台）
//!
//! - stdio JSON-RPC 2.0（`ping` / `exec`）——供 sidecar / 子进程调用；
//! - 回环 HTTP `127.0.0.1:18183`——供 GUI / 远程开发容器调用。
//!
//! 错误码：`-32700` 解析错误 / `-32601` 方法不存在 / `-32005` 未授权 / `-32001` 命令被拒。

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::mode::ModeKind;

use super::Sandbox;

/// 处理单条 JSON-RPC 请求（纯函数，stdio / HTTP 双通道共用）
pub async fn handle_jsonrpc(
    sandbox: &Sandbox,
    request: &str,
    mode: ModeKind,
    token: Option<&str>,
) -> String {
    let parsed: Result<Value, _> = serde_json::from_str(request);
    let Ok(msg) = parsed else {
        return json_error(None, -32700, "解析错误: JSON 无效");
    };
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");

    match method {
        "ping" => json_ok(id, json!("pong")),
        "exec" => handle_exec(sandbox, &msg, id, mode, token).await,
        _ => json_error(Some(id), -32601, "方法不存在"),
    }
}

/// exec 方法：{argv, cwd, token?} → 沙箱执行
async fn handle_exec(
    sandbox: &Sandbox,
    msg: &Value,
    id: Value,
    mode: ModeKind,
    token: Option<&str>,
) -> String {
    let Some(params) = msg.get("params") else {
        return json_error(Some(id), -32602, "缺少 params");
    };

    // 令牌校验
    if let Some(provided) = params.get("token").and_then(|t| t.as_str()) {
        if token.is_none_or(|expected| expected != provided) {
            return json_error(Some(id), -32005, "未授权: 令牌不符");
        }
    } else if token.is_some() {
        return json_error(Some(id), -32005, "未授权: 缺少令牌");
    }

    let argv: Vec<String> = params
        .get("argv")
        .and_then(|a| serde_json::from_value::<Vec<String>>(a.clone()).ok())
        .unwrap_or_default();
    if argv.is_empty() {
        return json_error(Some(id), -32602, "argv 不能为空");
    }
    let cwd = params
        .get("cwd")
        .and_then(|c| c.as_str())
        .unwrap_or(".")
        .to_string();

    // 沙箱执行
    let result = sandbox
        .execute(&argv, std::path::Path::new(&cwd), mode)
        .await;
    let Ok(result) = result else {
        return json_error(Some(id), -32603, "沙箱内部错误");
    };

    if result.decision != "allow" {
        return json_error(
            Some(id),
            -32001,
            format!("命令被拒[{}]: {}", result.decision, result.stderr),
        );
    }
    json_ok(
        id,
        json!({
            "decision": result.decision,
            "stdout": result.stdout,
            "stderr": result.stderr,
            "exit_code": result.exit_code,
            "duration_ms": result.duration_ms,
            "truncated": result.truncated,
        }),
    )
}

fn json_ok(id: Value, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

fn json_error(id: Option<Value>, code: i64, message: impl Into<String>) -> String {
    json!({"jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "error": {"code": code, "message": message.into()}})
        .to_string()
}

/// stdio 通道：逐行读 stdin，逐行响应 stdout（`exec` / `ping`）
pub async fn serve_stdio(
    sandbox: Arc<Sandbox>,
    mode: ModeKind,
    token: Option<String>,
) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin).lines();
    let mut writer = stdout;

    while let Some(line) = reader.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let resp = handle_jsonrpc(&sandbox, &line, mode, token.as_deref()).await;
        writer.write_all(resp.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
    }
    Ok(())
}

/// 回环 HTTP 通道：`127.0.0.1:18183`，仅支持 POST JSON-RPC
pub async fn serve_http(
    sandbox: Arc<Sandbox>,
    mode: ModeKind,
    token: Option<String>,
    addr: &str,
) -> Result<()> {
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow!("沙箱 RPC 绑定 {addr} 失败: {e}"))?;

    loop {
        let (stream, _) = listener.accept().await?;
        let sandbox = sandbox.clone();
        let token = token.clone();
        tokio::spawn(async move {
            let _ = handle_http_conn(stream, &sandbox, mode, token).await;
        });
    }
}

/// 处理单个 HTTP 连接（最小实现：解析 Content-Length，POST body → JSON-RPC）
async fn handle_http_conn(
    stream: tokio::net::TcpStream,
    sandbox: &Sandbox,
    mode: ModeKind,
    token: Option<String>,
) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    let mut reader = BufReader::new(stream);
    // 请求行（忽略，仅接受 POST 语义）
    let mut request_line = String::new();
    reader.read_line(&mut request_line).await?;
    if !request_line.starts_with("POST") {
        return Ok(());
    }
    // 头部
    let mut headers: HashMap<String, String> = HashMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        if line == "\r\n" || line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    // 内容
    let content_length = headers
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).await?;
    }
    let request = String::from_utf8_lossy(&body).into_owned();
    let response = handle_jsonrpc(sandbox, &request, mode, token.as_deref()).await;

    let mut writer = reader.into_inner();
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.len()
    );
    writer.write_all(head.as_bytes()).await?;
    writer.write_all(response.as_bytes()).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::sandbox::audit::AuditSink;
    use crate::sandbox::{CommandRunner, ResourceLimit, RunnerOutput};
    use std::path::Path;
    use std::time::Duration;

    /// 只执行 argv[0] 前缀匹配的白名单命令（避免真实进程）
    struct FakeRunner;
    #[async_trait::async_trait]
    impl CommandRunner for FakeRunner {
        async fn run(&self, argv: &[String], _cwd: &Path, _timeout: Duration) -> RunnerOutput {
            RunnerOutput {
                stdout: format!("ran: {}", argv.join(" ")).into_bytes(),
                stderr: Vec::new(),
                exit_code: 0,
                timed_out: false,
            }
        }
    }

    fn sandbox() -> (Arc<Sandbox>, String) {
        let ws = std::env::temp_dir();
        let cwd = ws.to_string_lossy().replace('\\', "\\\\");
        (
            Arc::new(Sandbox::with_runner(&ws, Box::new(FakeRunner))),
            cwd,
        )
    }

    /// 构造 exec 请求（cwd 已做 JSON 转义）
    fn exec_req(id: i64, argv: &[&str], cwd: &str, token: Option<&str>) -> String {
        let mut params = serde_json::json!({"argv": argv, "cwd": cwd});
        if let Some(t) = token {
            params["token"] = serde_json::json!(t);
        }
        serde_json::json!({"jsonrpc": "2.0", "id": id, "method": "exec", "params": params})
            .to_string()
    }

    #[tokio::test]
    async fn test_ping() {
        let (sb, _) = sandbox();
        let resp = handle_jsonrpc(
            &sb,
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            ModeKind::Bloom,
            None,
        )
        .await;
        let v: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["result"], "pong");
        assert_eq!(v["id"], 1);
    }

    #[tokio::test]
    async fn test_parse_error() {
        let (sb, _) = sandbox();
        let resp = handle_jsonrpc(&sb, "not json", ModeKind::Bloom, None).await;
        let v: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["error"]["code"], -32700);
    }

    #[tokio::test]
    async fn test_unknown_method() {
        let (sb, _) = sandbox();
        let resp = handle_jsonrpc(
            &sb,
            r#"{"jsonrpc":"2.0","id":2,"method":"foo"}"#,
            ModeKind::Bloom,
            None,
        )
        .await;
        let v: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn test_exec_allowed() {
        let (sb, cwd) = sandbox();
        let req = exec_req(3, &["echo", "hi"], &cwd, None);
        let resp = handle_jsonrpc(&sb, &req, ModeKind::Bloom, None).await;
        let v: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["result"]["stdout"], "ran: echo hi");
        assert_eq!(v["result"]["exit_code"], 0);
    }

    #[tokio::test]
    async fn test_exec_denied() {
        let (sb, _) = sandbox();
        // rm 不在白名单 → 命令被拒
        let req = r#"{"jsonrpc":"2.0","id":4,"method":"exec","params":{"argv":["rm","-rf","/"],"cwd":"."}}"#;
        let resp = handle_jsonrpc(&sb, req, ModeKind::Bloom, None).await;
        let v: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["error"]["code"], -32001);
        assert!(v["error"]["message"].as_str().unwrap().contains("被拒"));
    }

    #[tokio::test]
    async fn test_exec_token_required() {
        let (sb, cwd) = sandbox();
        let req = exec_req(5, &["ls"], &cwd, None);
        // 服务器要求令牌但请求未带
        let resp = handle_jsonrpc(&sb, &req, ModeKind::Bloom, Some("secret")).await;
        let v: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["error"]["code"], -32005);
    }

    #[tokio::test]
    async fn test_exec_token_mismatch() {
        let (sb, cwd) = sandbox();
        let req = exec_req(6, &["ls"], &cwd, Some("bad"));
        let resp = handle_jsonrpc(&sb, &req, ModeKind::Bloom, Some("secret")).await;
        let v: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["error"]["code"], -32005);
    }

    #[tokio::test]
    async fn test_exec_token_match() {
        let (sb, cwd) = sandbox();
        let req = exec_req(7, &["ls"], &cwd, Some("secret"));
        let resp = handle_jsonrpc(&sb, &req, ModeKind::Bloom, Some("secret")).await;
        let v: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["result"]["stdout"], "ran: ls");
    }

    #[tokio::test]
    async fn test_exec_empty_argv() {
        let (sb, cwd) = sandbox();
        let req = exec_req(8, &[], &cwd, None);
        let resp = handle_jsonrpc(&sb, &req, ModeKind::Bloom, None).await;
        let v: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["error"]["code"], -32602);
    }

    /// 审计仍应落盘（deny 路径）
    #[tokio::test]
    async fn test_exec_denied_still_audited() {
        let ws =
            std::env::temp_dir().join(format!("marguerite_rpc_audit_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&ws).unwrap();
        let sb = Arc::new(Sandbox::with_runner(&ws, Box::new(FakeRunner)));
        let cwd = ws.to_string_lossy().replace('\\', "\\\\");
        let req = exec_req(9, &["rm", "x"], &cwd, None);
        let _ = handle_jsonrpc(&sb, &req, ModeKind::Bloom, None).await;
        let sink = AuditSink::new(ws.join(".marguerite-audit"));
        let records = sink.read_all();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].decision, "deny");
        let _ = std::fs::remove_dir_all(&ws);
    }

    // 资源限制字段可供 RPC 通道读取（编译期保证无未用字段警告）
    #[tokio::test]
    async fn test_limits_accessible() {
        let (sb, _) = sandbox();
        let limits: ResourceLimit = sb.limits();
        assert!(limits.timeout_sec >= 1);
        let _ = limits;
    }
}
