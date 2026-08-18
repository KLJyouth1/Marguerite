//! ACP 回环 listener（差距② 薄壳的真实网络落点）。
//!
//! 把 `AcpSession` 暴露为本地 127.0.0.1 的 JSON-RPC 帧服务（逐 `\n` 分隔）。
//! 仅回环：绝不绑定非本地地址（由 `bind(("127.0.0.1", port))` 强制约束）。
//! 真实 sidecar（桌面 egui / TUI）通过回环端口与引擎通信，绝不暴露到公网。

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use super::session::AcpSession;

/// 一个已启动的 listener 句柄：持有监听地址与后台 task。
///
/// Drop 时后台 accept 循环随 `JoinHandle` 被 abort（Rust 的 `JoinHandle` Drop 即取消 task）。
pub struct ServeHandle {
    /// 实际监听的回环地址（OS 分配随机端口时为真实端口）。
    pub addr: SocketAddr,
    /// 后台 accept 循环 task。
    pub handle: JoinHandle<()>,
}

/// 起一个仅回环（127.0.0.1）的 ACP TCP listener，逐行（JSON-RPC 帧）处理入站消息。
///
/// - `port_hint = 0` 时由 OS 分配空闲端口（推荐，避免端口冲突 / 权限问题）。
/// - `session` 被 `Arc` 包裹，所有连接共享同一会话路由逻辑。
/// - 返回监听地址（供 sidecar 连接）+ task handle（drop 即停）。
pub async fn start_listener(session: Arc<AcpSession>, port_hint: u16) -> io::Result<ServeHandle> {
    let listener = TcpListener::bind(("127.0.0.1", port_hint)).await?;
    let addr = listener.local_addr()?;
    let handle = tokio::spawn(async move {
        while let Ok((stream, _peer)) = listener.accept().await {
            let s = Arc::clone(&session);
            tokio::spawn(async move {
                let _ = handle_conn(&s, stream).await;
            });
        }
    });
    Ok(ServeHandle { addr, handle })
}

/// 处理单个连接：逐行读取 JSON-RPC 帧，经 `session.handle` 路由，写回响应。
async fn handle_conn(session: &AcpSession, stream: TcpStream) -> io::Result<()> {
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut lines = BufReader::new(read_half).lines();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(resp) = session.handle(line).await {
            // JSON-RPC 响应以 `\n` 帧分隔，便于 sidecar 逐行解析。
            let mut payload = serde_json::to_string(&resp)
                .unwrap_or_else(|_| "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32603,\"message\":\"serialize failed\"}}".to_string());
            payload.push('\n');
            write_half.write_all(payload.as_bytes()).await?;
            write_half.flush().await?;
        }
        // notification（无 id）→ resp 为 None → 不写回（网关侧回 204）
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::serve::acp::{AcpSession, AgentInvoker};

    struct StubInvoker;
    #[async_trait::async_trait]
    impl AgentInvoker for StubInvoker {
        async fn invoke(&self, _p: &str) -> Result<String, String> {
            Ok("stub".to_string())
        }
    }

    #[tokio::test]
    async fn loopback_listener_echoes_ping() {
        // 真实回环 listener 端到端：bind 随机端口 → 客户端发 ping → 收 pong。
        let invoker: Arc<dyn AgentInvoker> = Arc::new(StubInvoker);
        let session = Arc::new(AcpSession::new("srv", invoker));
        let handle = start_listener(session, 0).await.unwrap();
        let addr = handle.addr;

        // 客户端：异步连回环端口，发一行 JSON-RPC，读一行响应。
        // 用 tokio 异步流，避免阻塞当前 runtime worker 导致 accept 循环饿死（单核环境尤甚）。
        let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
        client
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n")
            .await
            .unwrap();
        let mut reader = BufReader::new(client);
        let mut buf = String::new();
        reader.read_line(&mut buf).await.unwrap();

        assert!(
            buf.contains("\"pong\":true"),
            "listener 应回 pong，实际: {buf}"
        );
        assert!(buf.contains("\"session_id\":\"srv\""));
        // 回环约束：地址必须是 127.0.0.1
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
    }
}
