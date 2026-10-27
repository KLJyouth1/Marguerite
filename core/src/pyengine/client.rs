//! Python 网关 WS 客户端：做 challenge→connect 握手 + 单请求 dispatch
//!
//! 协议与 Rust 网关实现完全一致（Marguerite v3）：
//! ```text
//! ws.recv: challenge  → ws.send: {type:"connect",auth:{token}}
//!           └──────► ws.recv: HelloOk（确认协议、鉴权）
//! ws.send:  req {id, method, params} → ws.recv: {type:"res", ok, payload}
//! ws.recv:  event 帧（独立于请求/响应，由 event_bridge.rs 消费）
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::time::{timeout, Instant};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, trace, warn};

use crate::gateway::protocol::{RpcError, PROTOCOL_VERSION};

/// 方法分派响应：对应 Rust 网关的 `dispatch` 返回形态
pub type PyDispatchResult = std::result::Result<Value, RpcError>;

/// Python WS 客户端（1:N 调用：一个 client 一个长连接；方法调用串行）
pub struct PyGatewayClient {
    tx: Arc<Mutex<mpsc::Sender<ClientCmd>>>,
}

enum ClientCmd {
    Call {
        id: String,
        method: String,
        params: Value,
        reply: tokio::sync::oneshot::Sender<PyDispatchResult>,
    },
    /// 取消某个未完成的调用：驱动循环据此丢弃对应 pending，让等待方立即感知通道关闭
    Cancel { id: String },
}

/// WS 半部的具体类型（`connect_async` 的产物，与 `browser/mod.rs` 同形）。
type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;
type WsWriter = futures_util::stream::SplitSink<WsStream, Message>;
type WsReader = futures_util::stream::SplitStream<WsStream>;

impl PyGatewayClient {
    /// 建立 WS 连接并完成 challenge→connect 握手。
    ///
    /// 返回的句柄是**跨代稳定**的：内部有监督任务负责断开后重连，
    /// 因此 `PY_CLIENT_REG` 这个 `OnceLock` 单例不会因为
    /// 「Python 侧车被守护任务进程内重启」而变成死连接（S7）。
    pub async fn connect(base_url: &str, _token: &str) -> Result<Self> {
        let ws_url = build_ws_url(base_url);

        // 事件扇出与命令入口在**重连之间保持不变**：重连只换 WS 半部，
        // 不换消费者，否则 event_bridge 会订阅到一个再也不产出的信道。
        //
        // P1-8：事件信道用 broadcast 而非 mpsc——event_bridge 是慢消费者
        // （每个 event 都要 emit 到 EventBus + 推给 WS 订阅者），mpsc 满时
        // `event_tx.send().await` 会**阻塞整个 WS 驱动循环**（req/res 都走
        // 同一个 select!），侧车一洪峰事件，RPC 全部停摆。broadcast::Sender::send
        // 是同步非阻塞的：消费者落后超过容量即收到 `Lagged` 自愈——语义正是
        // 「丢最旧事件帧、保控制/响应帧」。
        let (event_tx, event_rx) = broadcast::channel::<Value>(EVENT_CHANNEL_CAP);
        let (cmd_tx, cmd_rx) = mpsc::channel::<ClientCmd>(32);
        let cmd_tx_arc = Arc::new(Mutex::new(cmd_tx));
        // P2-10：跨重连跟踪已转发的最大事件 seq。断线重连后按它向新会话发
        // `events.resync` 补回缺口、去重，杜绝重复执行与丢事件。
        let last_seq = Arc::new(std::sync::atomic::AtomicU64::new(0));

        let (writer, reader) = Self::open_session(&ws_url).await?;
        tokio::spawn(async move {
            Self::supervise_ws(ws_url, writer, reader, event_tx, cmd_rx, last_seq).await;
        });
        // event_rx 暴露到全局静态注册表：通过 EVENTS_BUS 转发在 event_bridge.rs
        set_event_sink(event_rx);

        Ok(Self { tx: cmd_tx_arc })
    }

    /// 建连 + 完成 challenge→connect 握手，返回拆分的读写半部。
    async fn open_session(ws_url: &str) -> Result<(WsWriter, WsReader)> {
    let req = ws_url
        .into_client_request()
        .with_context(|| format!("构造 WS 请求失败: {ws_url}"))?;
    let (ws, _resp) = connect_async(req)
        .await
        .with_context(|| format!("连接 Python 网关 WS 失败: {ws_url}"))?;
    let (mut writer, mut reader) = ws.split();

        // 1) 等待 challenge
        let challenge_msg: String = timeout(Duration::from_secs(10), async {
            loop {
                match reader.next().await {
                    Some(Ok(Message::Text(t))) => return Ok(t.to_string()),
                    Some(Ok(Message::Close(_))) | None => {
                        return Err(anyhow!("WS 在 challenge 阶段关闭"));
                    }
                    Some(Err(e)) => return Err(anyhow!("WS 错误: {e}")),
                    _ => continue,
                }
            }
        })
        .await
        .with_context(|| "Python WS challenge 超时")??;

        let parsed: Value =
            serde_json::from_str(&challenge_msg).with_context(|| "challenge 帧非 JSON")?;
        if parsed.get("type").and_then(Value::as_str) != Some("event")
            || parsed.get("event").and_then(Value::as_str) != Some("connect.challenge")
        {
            return Err(anyhow!("首个帧非 connect.challenge"));
        }

        // 2) 发送标准 RPC v3 connect req（Python 网关 websocket.py 校验
        //    `type == "req" && method == "connect"`，非标准 `{"type":"connect"}` 会被拒绝）。
        //    auth 留空：Python resolve_auth 对空 token 返回 guest principal（放行）；
        //    若带与 config.auth.token 不匹配的 token 反而被判 invalid 拒绝。
        let connect = json!({
            "type": "req",
            "method": "connect",
            "id": "pybridge-handshake",
            "params": {
                "auth": {},
                "role": "operator",
            },
        });
        writer
            .send(Message::Text(connect.to_string()))
            .await
            .context("发送 connect 帧失败")?;

        // 3) 等待 HelloOk 或 error
        let hello_msg: String = timeout(Duration::from_secs(15), async {
            loop {
                match reader.next().await {
                    Some(Ok(Message::Text(t))) => return Ok(t.to_string()),
                    Some(Ok(Message::Ping(b))) => {
                        let _ = writer.send(Message::Pong(b)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        return Err(anyhow!("WS 在 hello 阶段关闭"));
                    }
                    Some(Err(e)) => return Err(anyhow!("WS 错误: {e}")),
                    _ => continue,
                }
            }
        })
        .await
        .with_context(|| "Python WS HelloOk 超时")??;

        let hello: Value = serde_json::from_str(&hello_msg).with_context(|| "hello 帧非 JSON")?;
        // Python 网关的握手成功帧是 `{"type":"hello-ok",...}`（无 `ok` 字段），
        // 与 Rust 网关自身的 `{"ok":true}` res 帧不同；两种都接受。
        let is_hello_ok = hello.get("type").and_then(Value::as_str) == Some("hello-ok");
        let is_ok_res = hello.get("ok").and_then(Value::as_bool) == Some(true);
        if !is_hello_ok && !is_ok_res {
            return Err(anyhow!("握手被拒: {hello:?}"));
        }
        let proto = hello
            .get("protocol")
            .and_then(Value::as_u64)
            .unwrap_or_default() as u32;
        debug!(proto, "Python 网关握手完成");

        // 保存协议版本（debug 用）
        let _ = proto; // 避免未使用；可按需提升字段
        let _ = PROTOCOL_VERSION;

        Ok((writer, reader))
}

    /// 连接监督：断开后按退避重连，直到命令信道关闭。
    ///
    /// # 为什么必须重连
    ///
    /// Python 侧车子进程会被守护任务在**进程内**重启（`sidecar.rs` 的退避重启）。
    /// 若客户端在 WS 断开时永久退出，`py_client()` 这个 `OnceLock` 单例就会永远
    /// 指向**上一代侧车**的死连接 —— 此后所有经 Python 代理的方法
    /// （`logs.tail` / `pyengine.*` / 侧车网关方法…）全部失败且无从自愈。
    /// 这正是 S7「禁跨代缓存句柄」要消灭的缺陷类型：与其让句柄带上代际标记，
    /// 不如让客户端自己重连 —— 句柄于是与「第几代」无关。
    ///
    /// # 断连窗口内的调用
    ///
    /// fail-fast：重连期间到达的调用立即回一个明确错误，而不是让调用方
    /// 白等自己的 60s 超时。
    async fn supervise_ws(
        ws_url: String,
        mut writer: WsWriter,
        mut reader: WsReader,
        event_tx: broadcast::Sender<Value>,
        mut cmd_rx: mpsc::Receiver<ClientCmd>,
        last_seq: Arc<std::sync::atomic::AtomicU64>,
    ) {
    const INITIAL_BACKOFF: Duration = Duration::from_millis(500);
    const MAX_BACKOFF: Duration = Duration::from_secs(30);

    let mut backoff = INITIAL_BACKOFF;
    loop {
        drive_ws_loop(writer, reader, event_tx.clone(), &mut cmd_rx, last_seq.clone()).await;

        // 断连 → 重连，直到成功。期间保持对 cmd_rx 的服务（fail-fast）。
        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    match cmd {
                        // 断连窗口内到达的调用：立即回错，别让调用方等满超时
                        Some(ClientCmd::Call { reply, .. }) => {
                            let _ = reply.send(Err(RpcError::internal(
                                "Python 侧车连接已断开，正在重连；请稍后重试",
                            )));
                        }
                        Some(ClientCmd::Cancel { .. }) => {}
                        // 所有发送端已释放 → 外层句柄被丢弃，监督结束
                        None => return,
                    }
                }
                res = Self::open_session(&ws_url) => {
                    match res {
                        Ok((w, r)) => {
                            writer = w;
                            reader = r;
                            backoff = INITIAL_BACKOFF;
                            // P2-10：重连成功后、进入驱动循环前，按 last_seq 对账：
                            // 向新会话发 events.resync 补回断线期间的事件缺口（去重），
                            // 杜绝重复执行与丢事件。失败（侧车不支持/超时）则安全跳过，
                            // 后续 live 事件照常处理。
                            resync_after_reconnect(
                                &mut writer,
                                &mut reader,
                                &event_tx,
                                &last_seq,
                            )
                            .await;
                            // info 而非 debug：重连成功是**生命周期事件**，
                            // 默认 `warn` 过滤下若不可见，运维就无法区分
                            // 「已自愈」与「一直没连上」。
                            tracing::info!("Python WS 已重连，侧车方法代理恢复");
                            break;
                        }
                        Err(e) => {
                            warn!(
                                err = %e,
                                backoff_ms = backoff.as_millis(),
                                "Python WS 重连失败，退避后重试"
                            );
                            // 退避期间仍然 fail-fast 服务 cmd_rx
                            let deadline = Instant::now() + backoff;
                            loop {
                                tokio::select! {
                                    _ = tokio::time::sleep_until(deadline) => break,
                                    cmd = cmd_rx.recv() => {
                                        match cmd {
                                            Some(ClientCmd::Call { reply, .. }) => {
                                                let _ = reply.send(Err(RpcError::internal(
                                                    "Python 侧车未就绪（重连中）；请稍后重试",
                                                )));
                                            }
                                            Some(ClientCmd::Cancel { .. }) => {}
                                            None => return,
                                        }
                                    }
                                }
                            }
                            backoff = (backoff * 2).min(MAX_BACKOFF);
                        }
                    }
                }
            }
        }
    }
}

    /// 同步调用：发送 req{id,method,params}，等待对应 res。
    /// 调用方等待包在超时内（默认 60s，可由 `MARGUERITE_PYENGINE_CALL_TIMEOUT_MS` 覆盖）：
    /// 超时则 best-effort 通知驱动循环取消该 pending，再返回 `TIMEOUT` 错误，避免永久挂起。
    pub async fn call(&self, method: &str, params: Value) -> PyDispatchResult {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let id = format!("py-{}", uuid::Uuid::new_v4());
        {
            let guard = self.tx.lock().await;
            guard
                .send(ClientCmd::Call {
                    id: id.clone(),
                    method: method.to_string(),
                    params,
                    reply: tx,
                })
                .await
                .map_err(|_| RpcError::internal("Python 侧车命令信道已关闭"))?;
        }
        let dur = call_timeout();
        match timeout(dur, rx).await {
            Ok(Ok(v)) => v,
            Ok(Err(_)) => Err(RpcError::internal("Python 侧车调用响应通道关闭")),
            Err(_) => {
                // 超时：best-effort 通知驱动循环取消该 pending（发失败不影响返回）
                let waited_ms = dur.as_millis() as u64;
                let _ = self.tx.lock().await.send(ClientCmd::Cancel { id }).await;
                Err(RpcError::timeout(method, waited_ms))
            }
        }
    }
}

fn build_ws_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    format!("{}/ws", trimmed.replacen("http://", "ws://", 1))
}

// ---------------------------------------------------------------------------
// 事件转发桥的 sink（全局共享，event_bridge.rs 拿出来 consume）
// ---------------------------------------------------------------------------

/// P1-8：事件信道容量。洪峰时消费者落后超过此数即丢最旧（broadcast Lagged）。
pub(crate) const EVENT_CHANNEL_CAP: usize = 256;

static EVENT_SINK: std::sync::OnceLock<Mutex<Option<broadcast::Receiver<Value>>>> =
    std::sync::OnceLock::new();

fn set_event_sink(rx: broadcast::Receiver<Value>) {
    EVENT_SINK
        .set(Mutex::new(Some(rx)))
        .unwrap_or_else(|_| panic!("EVENT_SINK 已初始化"));
}

pub async fn take_event_sink() -> Option<broadcast::Receiver<Value>> {
    EVENT_SINK.get()?.lock().await.take()
}

// ---------------------------------------------------------------------------
// 长连接驱动循环
// ---------------------------------------------------------------------------

async fn drive_ws_loop<S, R>(
    mut writer: futures_util::stream::SplitSink<S, Message>,
    mut reader: futures_util::stream::SplitStream<R>,
    event_tx: broadcast::Sender<Value>,
    cmd_rx: &mut mpsc::Receiver<ClientCmd>,
    last_seq: Arc<std::sync::atomic::AtomicU64>,
) where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
    R: futures_util::Stream<
            Item = std::result::Result<Message, tokio_tungstenite::tungstenite::Error>,
        > + Unpin,
{
    let mut pending: HashMap<String, PendingCall> = HashMap::new();
    let call_timeout = call_timeout();
    let mut tick = tokio::time::interval(Duration::from_secs(25)); // <30s tick 间隔

    loop {
        tokio::select! {
            maybe_msg = reader.next() => {
                match maybe_msg {
                    Some(Ok(Message::Text(t))) => {
                        let text = t.to_string();
                        if let Ok(frame) = serde_json::from_str::<Value>(&text) {
                            match frame.get("type").and_then(Value::as_str) {
                                Some("res") | Some("result") => {
                                    if let Some(id) = frame.get("id").and_then(Value::as_str).map(String::from) {
                                        if let Some(reply) = pending.remove(&id) {
                                            let ok = frame.get("ok").and_then(Value::as_bool).unwrap_or(false);
                                            let result = if ok {
                                                Ok(frame.get("payload").cloned().unwrap_or(Value::Null))
                                            } else {
                                                let err = frame.get("error");
                                                let code = err.and_then(|e| e.get("code").and_then(Value::as_str)).unwrap_or("UNKNOWN");
                                                let msg = err.and_then(|e| e.get("message").and_then(Value::as_str)).unwrap_or("Python 侧返回错误");
                                                Err(RpcError {
                                                    code: code.to_string(),
                                                    message: msg.to_string(),
                                                    details: err.cloned().map(Box::new),
                                                    retryable: Some(err.and_then(|e| e.get("retryable").and_then(Value::as_bool)).unwrap_or(false)),
                                                    retry_after_ms: None,
                                                    accepted: Some(false),
                                                })
                                            };
                                            let _ = reply.reply.send(result);
                                        }
                                    }
                                }
                                Some("event") => {
                                    // P1-8：broadcast::Sender::send 同步非阻塞——
                                    // 慢消费者落后时丢最旧（Lagged），绝不 await 堵驱动循环。
                                    // 没有接收者（event_bridge 未启动）时 Err 直接忽略。
                                    // P2-10：按 seq 去重（resync 已补过的事件不再重发）。
                                    forward_event_dedup(frame, &event_tx, &last_seq);
                                }
                                Some("tick") => { /* tick 帧不处理 */ }
                                Some("ping") => {
                                    let _ = writer.send(Message::Text(r#"{"type":"pong"}"#.into())).await;
                                }
                                _ => {
                                    trace!(raw = %text, "未知帧类型");
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Ping(b))) => {
                        let _ = writer.send(Message::Pong(b)).await;
                    }
                    Some(Ok(Message::Binary(b))) => {
                        // 预留二进制信道：忽略
                        let _ = b;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(e)) => {
                        warn!(err = %e, "Python WS 错误，驱动退出");
                        break;
                    }
                    _ => {}
                }
            }
            maybe_cmd = cmd_rx.recv() => {
                match maybe_cmd {
                    Some(ClientCmd::Call { id, method, params, reply }) => {
                        let req = json!({
                            "type": "req",
                            "id": id,
                            "method": method,
                            "params": params,
                        });
                        match writer.send(Message::Text(req.to_string())).await {
                            Ok(()) => {
                                pending.insert(id, PendingCall { reply, inserted_at: Instant::now() });
                            }
                            Err(e) => {
                                let _ = reply.send(Err(RpcError::internal(&format!(
                                    "写入 Python WS 失败: {e}"
                                ))));
                                break;
                            }
                        }
                    }
                    Some(ClientCmd::Cancel { id }) => {
                        // 取消：丢弃对应 pending 的 sender，让等待方立即感知通道关闭
                        let _ = apply_cancel(&mut pending, &id);
                    }
                    None => break,
                }
            }
            _ = tick.tick() => {
                // ping 保活：避免中间设备断 TCP
                let _ = writer.send(Message::Text(r#"{"type":"ping"}"#.into())).await;
                // 兜底清扫：移除超过调用超时上限的 pending（Cancel 若丢失则此处回收，防内存滞留）
                let evicted = evict_stale_pending(&mut pending, Instant::now(), call_timeout);
                if evicted > 0 {
                    debug!(evicted, "清扫超期未完成的 Python 调用 pending");
                }
            }
        }
    }
    // 清理：把所有 pending 标失败（调用方能感知断连）
    for (_, pc) in pending {
        let _ = pc
            .reply
            .send(Err(RpcError::internal("Python WS 已断开，调用未完成")));
    }
}

// ---------------------------------------------------------------------------
// P2-10：断线重连 seq 对账（resync 补帧 + 去重）
// ---------------------------------------------------------------------------

/// 重连后对账：向新会话发 `events.resync(last_seq)`，把断线期间缺口事件按 seq
/// 去重后重新经 `event_tx` 转发。任何失败（侧车不支持 / 超时 / 写失败）都安全
/// 跳过，不阻塞后续 live 事件——绝不为了补帧把界面挂住。
async fn resync_after_reconnect(
    writer: &mut WsWriter,
    reader: &mut WsReader,
    event_tx: &broadcast::Sender<Value>,
    last_seq: &Arc<std::sync::atomic::AtomicU64>,
) {
    let current = last_seq.load(std::sync::atomic::Ordering::Acquire);
    let req = json!({
        "type": "req",
        "id": RECONNECT_RESYNC_ID,
        "method": "events.resync",
        "params": { "last_seq": current },
    });
    if writer.send(Message::Text(req.to_string())).await.is_err() {
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match timeout(remaining, reader.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                let Ok(frame) = serde_json::from_str::<Value>(&t.to_string()) else {
                    continue;
                };
                match frame.get("type").and_then(Value::as_str) {
                    Some("res") | Some("result")
                        if frame.get("id").and_then(Value::as_str) == Some(RECONNECT_RESYNC_ID) =>
                    {
                        if frame.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                            let payload = frame.get("payload").cloned().unwrap_or(Value::Null);
                            forward_resync_payload(&payload, event_tx, last_seq);
                        }
                        // 无论 ok 与否，对账结束
                        return;
                    }
                    Some("event") => {
                        // 对账期间到达的 live 事件：去重后转发（可能与 replay 重叠）。
                        forward_event_dedup(frame, event_tx, last_seq);
                    }
                    Some("ping") => {
                        let _ = writer.send(Message::Text(r#"{"type":"pong"}"#.into())).await;
                    }
                    _ => {}
                }
            }
            Ok(Some(Ok(Message::Ping(b)))) => {
                let _ = writer.send(Message::Pong(b)).await;
            }
            // 关闭 / 错误 / 超时：放弃对账（交给下一轮重连）。
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) | Ok(Some(Err(_))) => return,
            Err(_) => return,
            Ok(Some(Ok(_))) => {}
        }
    }
}

const RECONNECT_RESYNC_ID: &str = "reconnect-resync";

/// 转发 events.resync 回包里的事件帧；`to_seq` 用于在快照模式下把水位推进到快照点。
fn forward_resync_payload(
    payload: &Value,
    event_tx: &broadcast::Sender<Value>,
    last_seq: &Arc<std::sync::atomic::AtomicU64>,
) {
    if let Some(events) = payload.get("events").and_then(Value::as_array) {
        for ev in events {
            forward_event_dedup(ev.clone(), event_tx, last_seq);
        }
    }
    if let Some(to_seq) = payload.get("to_seq").and_then(Value::as_u64) {
        last_seq.fetch_max(to_seq, std::sync::atomic::Ordering::AcqRel);
    }
}

/// 按 seq 去重后转发单个事件帧：seq 已转发过（≤ last_seq）则跳过；否则推进
/// 水位并经 broadcast 同步发送（无接收者时 Err 忽略）。
fn forward_event_dedup(
    frame: Value,
    event_tx: &broadcast::Sender<Value>,
    last_seq: &Arc<std::sync::atomic::AtomicU64>,
) {
    if let Some(seq) = frame.get("seq").and_then(Value::as_u64) {
        let prev = last_seq.load(std::sync::atomic::Ordering::Acquire);
        if seq <= prev {
            return; // 已在 resync 或上一轮转发过
        }
        last_seq.fetch_max(seq, std::sync::atomic::Ordering::AcqRel);
    }
    let _ = event_tx.send(frame);
}

/// 纯函数：从已排序事件中挑出 seq 严格大于 `last_seq` 的（保持原顺序）。
/// 供单测钉住去重语义；无 seq 的帧（控制帧）一律保留。
#[cfg(test)]
fn filter_new_events(events: &[Value], last_seq: u64) -> Vec<Value> {
    events
        .iter()
        .filter(|ev| {
            ev.get("seq")
                .and_then(Value::as_u64)
                .map(|s| s > last_seq)
                .unwrap_or(true)
        })
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// 超时 / 取消 / 防泄漏：纯函数 + 薄封装（离线可测，无需侧车）
// ---------------------------------------------------------------------------

/// Python 侧车单调用超时（毫秒），可通过环境变量覆盖
const PYENGINE_CALL_TIMEOUT_ENV: &str = "MARGUERITE_PYENGINE_CALL_TIMEOUT_MS";

/// 单次调用 pending 记录：携带插入时间，供兜底清扫判断过期
struct PendingCall {
    reply: tokio::sync::oneshot::Sender<PyDispatchResult>,
    inserted_at: Instant,
}

/// 默认调用超时（与握手/心跳节奏协调：> 25s tick，小于常规 gateway 超时预算）
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// 纯解析超时配置：不读真实 env，便于单测。
/// `None`/空/解析失败/非正数 → 回落默认 60s；合法正整数(毫秒) → 对应 Duration。
fn call_timeout_from_env(raw: Option<&str>) -> Duration {
    let s = match raw {
        Some(s) => s.trim(),
        None => return DEFAULT_CALL_TIMEOUT,
    };
    match s.parse::<u64>() {
        Ok(ms) if ms > 0 => Duration::from_millis(ms),
        _ => DEFAULT_CALL_TIMEOUT,
    }
}

/// 读取并解析调用超时（薄封装：读真实 env 后交给 [`call_timeout_from_env`]）
fn call_timeout() -> Duration {
    call_timeout_from_env(std::env::var(PYENGINE_CALL_TIMEOUT_ENV).ok().as_deref())
}

/// 处理取消命令：从 pending 中移除对应调用并丢弃 sender，
/// 让等待方立即感知通道关闭（而不是继续悬挂直到超时）。
fn apply_cancel(pending: &mut HashMap<String, PendingCall>, id: &str) -> bool {
    pending.remove(id).is_some()
}

/// 兜底清扫：移除插入时间距今超过 `ttl` 的 pending，返回清扫掉的条数。
/// 调用方超时后会 best-effort 发 Cancel；此处回收 Cancel 丢失导致滞留的条目，防内存泄漏。
fn evict_stale_pending(
    pending: &mut HashMap<String, PendingCall>,
    now: Instant,
    ttl: Duration,
) -> usize {
    let mut removed = 0;
    pending.retain(|_, pc| {
        let stale = now.saturating_duration_since(pc.inserted_at) > ttl;
        if stale {
            removed += 1;
        }
        !stale
    });
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_sender() -> tokio::sync::oneshot::Sender<PyDispatchResult> {
        let (tx, _rx) = tokio::sync::oneshot::channel::<PyDispatchResult>();
        tx
    }

    #[test]
    fn test_call_timeout_from_env_default_and_override() {
        assert_eq!(call_timeout_from_env(None), Duration::from_secs(60));
        assert_eq!(
            call_timeout_from_env(Some("120000")),
            Duration::from_secs(120)
        );
        assert_eq!(
            call_timeout_from_env(Some("not-a-number")),
            Duration::from_secs(60)
        );
        assert_eq!(call_timeout_from_env(Some("0")), Duration::from_secs(60));
        assert_eq!(call_timeout_from_env(Some("")), Duration::from_secs(60));
        assert_eq!(call_timeout_from_env(Some("   ")), Duration::from_secs(60));
        assert_eq!(
            call_timeout_from_env(Some("60000")),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn test_evict_stale_pending_removes_only_expired() {
        let now = Instant::now();
        let old = now - Duration::from_secs(120);
        let fresh = now - Duration::from_secs(5);
        let mut pending: HashMap<String, PendingCall> = HashMap::new();
        pending.insert(
            "expired1".to_string(),
            PendingCall {
                reply: make_sender(),
                inserted_at: old,
            },
        );
        pending.insert(
            "expired2".to_string(),
            PendingCall {
                reply: make_sender(),
                inserted_at: old,
            },
        );
        pending.insert(
            "fresh1".to_string(),
            PendingCall {
                reply: make_sender(),
                inserted_at: fresh,
            },
        );
        let removed = evict_stale_pending(&mut pending, now, Duration::from_secs(60));
        assert_eq!(removed, 2);
        assert!(pending.contains_key("fresh1"));
        assert!(!pending.contains_key("expired1"));
        assert!(!pending.contains_key("expired2"));
    }

    #[test]
    fn test_cancel_variant_removes_pending() {
        let mut pending: HashMap<String, PendingCall> = HashMap::new();
        pending.insert(
            "c1".to_string(),
            PendingCall {
                reply: make_sender(),
                inserted_at: Instant::now(),
            },
        );
        assert!(apply_cancel(&mut pending, "c1"));
        assert!(!pending.contains_key("c1"));
        assert!(!apply_cancel(&mut pending, "missing"));
    }

    /// P1-8：事件洪峰 + 慢消费者不得阻塞发送方（broadcast 同步 send；落后丢最旧）。
    ///
    /// 旧实现 `event_tx.send(frame).await` 是 mpsc——event_bridge 一旦慢，
    /// 整个 WS 驱动循环（req/res 都在同一个 select!）跟着停摆。这里钉住
    /// 「洪峰瞬时入队 + 消费者恢复时 Lagged 自愈 + 拿到的是尾部新帧」三条。
    #[tokio::test]
    async fn event_flood_never_blocks_sender_and_drops_oldest() {
        let (tx, mut rx) = broadcast::channel::<Value>(4);
        // 消费者先取走一条，然后停住（模拟 event_bridge 慢）。
        assert_eq!(tx.send(json!({"seq": 1})).unwrap(), 1);
        // 洪峰：连续 send 必须全部同步返回（不 await、不阻塞驱动循环）。
        let started = Instant::now();
        for i in 2..=200 {
            assert!(tx.send(json!({"seq": i})).is_ok(), "有接收者时 send 不应失败");
        }
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "洪峰 200 条必须瞬时入队（broadcast 不阻塞发送方），实际 {:?}",
            started.elapsed()
        );
        // 消费者恢复：第一条 recv 必须拿到 Lagged（最旧事件被丢），随后拿到新帧。
        let lagged = rx.recv().await;
        assert!(
            matches!(lagged, Err(broadcast::error::RecvError::Lagged(_))),
            "慢消费者恢复后第一条必须是 Lagged（丢最旧），实际 {lagged:?}"
        );
        let next = rx.recv().await.expect("恢复后应能继续收到新帧");
        let seq = next["seq"].as_u64().unwrap();
        assert!(seq > 100, "丢的是最旧，应拿到洪峰尾部的新帧，实际 seq={seq}");
    }

    // ----- P2-10：seq 去重纯函数 -----
    #[test]
    fn filter_new_events_keeps_only_seq_above_watermark() {
        let events = vec![
            json!({"seq": 1}),
            json!({"seq": 2}),
            json!({"seq": 3}),
            json!({"seq": 4}),
            json!({"control": "no-seq"}),
        ];
        let kept = filter_new_events(&events, 2);
        let seqs: Vec<u64> = kept
            .iter()
            .filter_map(|e| e.get("seq").and_then(Value::as_u64))
            .collect();
        // seq>2 的是 3、4；无 seq 的控制帧保留。
        assert_eq!(seqs, vec![3, 4]);
        assert_eq!(kept.len(), 3);
    }

    // ----- P2-10：forward_event_dedup 去重 + 水位推进 -----
    #[tokio::test]
    async fn forward_event_dedup_skips_already_delivered_seq() {
        let (tx, mut rx) = broadcast::channel::<Value>(16);
        let last_seq = Arc::new(std::sync::atomic::AtomicU64::new(5));
        // seq=5 已转发 → 跳过
        forward_event_dedup(json!({"seq": 5, "type": "event"}), &tx, &last_seq);
        // seq=8 新 → 转发，水位推进到 8
        forward_event_dedup(json!({"seq": 8, "type": "event"}), &tx, &last_seq);
        // seq=6 落后 → 跳过
        forward_event_dedup(json!({"seq": 6, "type": "event"}), &tx, &last_seq);
        let got = rx.recv().await.expect("应收到 seq=8");
        assert_eq!(got["seq"].as_u64(), Some(8));
        assert_eq!(
            last_seq.load(std::sync::atomic::Ordering::Acquire),
            8,
            "水位应推进到 8"
        );
        // 无更多事件
        assert!(timeout(Duration::from_millis(100), rx.recv()).await.is_err());
    }

    // ----- P2-10：forward_resync_payload 重放 + to_seq 推进 -----
    #[tokio::test]
    async fn forward_resync_payload_replays_and_advances_watermark() {
        let (tx, mut rx) = broadcast::channel::<Value>(32);
        let last_seq = Arc::new(std::sync::atomic::AtomicU64::new(10));
        let payload = json!({
            "resync_mode": "replay",
            "to_seq": 13,
            "events": [
                {"seq": 11, "type": "event"},
                {"seq": 12, "type": "event"},
                {"seq": 10, "type": "event"} // 重复（≤水位），应去重
            ],
        });
        forward_resync_payload(&payload, &tx, &last_seq);
        let mut got_seqs = Vec::new();
        for _ in 0..2 {
            let ev = timeout(Duration::from_millis(200), rx.recv())
                .await
                .expect("应收到两条去重后的事件")
                .expect("event");
            got_seqs.push(ev["seq"].as_u64().unwrap());
        }
        assert_eq!(got_seqs, vec![11, 12]);
        assert_eq!(
            last_seq.load(std::sync::atomic::Ordering::Acquire),
            13,
            "to_seq 应把水位推进到 13"
        );
    }
}
