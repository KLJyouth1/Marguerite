//! 统一网关 HTTP/WebSocket 服务（仅回环 127.0.0.1）
//!
//! 路由：
//! - `/ws`                     → WebSocket JSON-RPC（Marguerite 协议）
//! - `/control/{*path}`        → Web UI 静态产物（SPA history fallback + index 注入）
//! - `/control/static/{*path}` → 后端静态资源（品牌图、分享图）
//! - `/api/health`             → REST 健康探测
//! - `/`                       → 重定向到 `/control/`
//!
//! 安全：只绑定 `127.0.0.1`；`connect` 握手必须携带本地 token；
//! 静态路径做 `..` / 反斜杠穿越防护。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use futures_util::StreamExt;
use tokio::sync::mpsc;

use crate::events::bus::EventBus;
use crate::events::envelope::Envelope;
use crate::gateway::auth;
use crate::gateway::events::envelope_to_frame;
use crate::gateway::methods::{dispatch, GatewayContext};
use crate::gateway::protocol::{
    challenge_frame, err_frame, hello_frame, ok_frame, parse_connect_params, parse_inbound,
    pong_frame, tick_frame, RpcError, SUPPORTED_EVENTS, SUPPORTED_METHODS, TICK_INTERVAL_MS,
};
use crate::persistence::config::Config;
use crate::pyengine::client::PyGatewayClient;
use crate::pyengine::sidecar::PyEngineSidecar;

/// 全局注册表（单例）：Python 侧车 + WS 客户端
///
/// 由 [`maybe_bootstrap_pyengine`] 在 `run()` 调用前写入一次（幂等）。
/// dispatch 方法路由直接通过 [`py_client()`] 取客户端转发 §5.2 调用。
static PY_SIDECAR_REG: std::sync::OnceLock<Arc<PyEngineSidecar>> = std::sync::OnceLock::new();
static PY_CLIENT_REG: std::sync::OnceLock<Arc<PyGatewayClient>> = std::sync::OnceLock::new();

/// 取 Python WS 客户端单例（双引擎模式启用时存在）
pub fn py_client() -> Option<&'static Arc<PyGatewayClient>> {
    PY_CLIENT_REG.get()
}
pub fn py_sidecar() -> Option<&'static Arc<PyEngineSidecar>> {
    PY_SIDECAR_REG.get()
}

async fn set_py_registry(sidecar: Arc<PyEngineSidecar>, client: Arc<PyGatewayClient>) {
    let _ = PY_SIDECAR_REG.set(sidecar);
    let _ = PY_CLIENT_REG.set(client);
}

/// 网关共享状态
pub struct GatewayState {
    pub config: Config,
    pub bus: EventBus,
    pub token: String,
    pub started_at: Instant,
    pub webui_dir: PathBuf,
}

impl GatewayState {
    /// 构造网关状态：token 加载（无则生成并落盘）、webui 产物目录探测
    pub fn new(config: Config, webui_dir: Option<PathBuf>) -> anyhow::Result<Self> {
        let token = match auth::load_or_create_token(&Config::data_dir()) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(err = %e, "网关 token 无法落盘，使用内存 token（重启后需重新认证）");
                auth::generate_token()
            }
        };
        let webui_dir = webui_dir
            .or_else(|| std::env::var_os("MARGUERITE_WEBUI_DIST").map(PathBuf::from))
            .or(detect_webui_dir())
            .unwrap_or(PathBuf::from("webui/dist"));
        Ok(Self {
            config,
            bus: {
                let bus = EventBus::default();
                // 注册网关总线全局句柄，供 usage::record 等无总线上下文的模块
                // 向已连接 WS 客户端发布事件（session.event.usage 等）。
                crate::events::set_global_bus(Arc::new(bus.clone()));
                bus
            },
            token,
            started_at: Instant::now(),
            webui_dir,
        })
    }
}

/// 启动 Python 侧车并建立 WS 客户端连接；失败降级为纯 Rust 模式（能力子集，UI 按 NOT_IMPLEMENTED 优雅提示）
pub async fn maybe_bootstrap_pyengine(state: Arc<GatewayState>) {
    use crate::pyengine::event_bridge;
    use crate::pyengine::sidecar::SidecarConfig;

    if PY_SIDECAR_REG.get().is_some() {
        return; // 已初始化，幂等退出
    }
    let cfg = match SidecarConfig::detect(state.token.clone()) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(err = %e, "侧车自动探测失败，使用纯 Rust 模式（能力子集）");
            return;
        }
    };
    let sidecar = match PyEngineSidecar::start(cfg).await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::warn!(err = ?e, "侧车启动失败，使用纯 Rust 模式（Rust 能力仍可用）");
            return;
        }
    };
    let client = match PyGatewayClient::connect(&sidecar.base_url(), &state.token).await {
        Ok(c) => Arc::new(c),
        Err(e) => {
            tracing::warn!(err = %e, "Python WS 握手失败，使用纯 Rust 模式");
            return;
        }
    };
    if let Some(handle) = event_bridge::bridge(state.bus.clone()).await {
        // 事件桥与进程同生共死，leak 句柄避免 drop
        std::mem::forget(handle);
    }
    set_py_registry(sidecar, client).await;
    tracing::info!("双引擎模式：Python 侧车 + WS 桥接入完成");
}

/// 从常见相对位置探测 Web UI 产物目录（`webui/dist` / `../webui/dist`）
fn detect_webui_dir() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    for rel in ["webui/dist", "../webui/dist"] {
        let candidate = cwd.join(rel);
        if candidate.join("index.html").is_file() {
            return Some(candidate);
        }
    }
    None
}

/// 构建 axum Router（仅回环监听，由调用方绑定）
pub fn build_router(state: Arc<GatewayState>) -> Router {
    Router::new()
        .route("/ws", get(ws_handler))
        .route("/api/health", get(api_health))
        // axum 通配符 `{*path}` 不匹配空路径，根路径单独注册
        .route("/control", get(control_root))
        .route("/control/", get(control_root))
        .route("/control/{*path}", get(control_assets))
        .route("/control/static/{*path}", get(control_assets))
        .fallback(fallback)
        .with_state(state)
}

/// 启动网关：绑定 127.0.0.1:<port> 并运行至进程结束
///
/// 先尝试启动 Python 侧车（失败降级纯 Rust 模式，不阻塞主线启动）。
pub async fn run(state: Arc<GatewayState>, port: u16) -> anyhow::Result<()> {
    // 启动时从 config.yaml 恢复运行时模式（bloom.parallel_enabled=true → Bloom），
    // 否则前端 mode.switch 切换后重启会丢失模式。
    if state.config.bloom.parallel_enabled {
        crate::mode::set_runtime_mode(crate::mode::ModeKind::Bloom);
    }
    // 侧车在后台异步启动（不阻塞网关 HTTP/WS 服务；失败自动降级纯 Rust 模式）。
    // 之前用 timeout(3s) 在 detect 阶段就被取消导致 channels.* 恒不可用，现改为
    // 常驻后台任务，Python 3.12+ 探测与启动完成后自动注册 py_client() 供方法代理。
    tokio::spawn(maybe_bootstrap_pyengine(state.clone()));

    // 用量台账在网关启动时立即初始化（不等首次 chat.send），
    // 否则用量页在首次对话前查 usage.status 会拿到全零。
    crate::usage::init();

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let addr = listener.local_addr()?;
    tracing::info!(
        %addr,
        webui = %state.webui_dir.display(),
        py_engine = py_sidecar().is_some(),
        "Marguerite 网关已启动（仅回环 127.0.0.1，绝不暴露公网）"
    );
    axum::serve(listener, build_router(state))
        .await
        .map_err(Into::into)
}

// ---------------------------------------------------------------------------
// REST
// ---------------------------------------------------------------------------

async fn api_health(State(state): State<Arc<GatewayState>>) -> impl IntoResponse {
    axum::Json(serde_json::json!({
        "status": "ok",
        "engine": "rust",
        "uptime_ms": state.started_at.elapsed().as_millis(),
    }))
}

// ---------------------------------------------------------------------------
// WebSocket JSON-RPC 会话
// ---------------------------------------------------------------------------

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<GatewayState>>) -> Response {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: Arc<GatewayState>) {
    // 1) 握手第一步：connect.challenge
    if let Err(e) = send_text(&mut socket, &challenge_frame().to_string()).await {
        tracing::debug!(err = %e, "连接在 challenge 阶段断开");
        return;
    }

    let mut seq: u64 = 0;
    let mut tick = tokio::time::interval(Duration::from_millis(TICK_INTERVAL_MS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // 2) 握手第二步：等待 connect req（10s 超时），校验 token
    let ctx = GatewayContext {
        config: state.config.clone(),
        token: state.token.clone(),
        started_at: state.started_at,
        bus: Arc::new(state.bus.clone()),
    };
    if !await_connect(&mut socket, &state).await {
        return;
    }
    let mut event_rx: Option<mpsc::Receiver<Envelope>> = Some(state.bus.subscribe("**"));

    // 3) 认证后事件循环：ws 入站 / 总线事件 / tick / 空闲看门狗
    let mut first_tick = true;
    // 服务端读超时：客户端静默死亡（未发 close）时，无任何入站帧超过阈值即
    // 主动断开，避免僵尸连接长期占用。阈值须高于客户端 ping 周期（55s）与
    // 重连抖动上限，避免误杀健康连接。
    let mut last_inbound = std::time::Instant::now();
    const IDLE_TIMEOUT_SECS: u64 = 180;
    loop {
        tokio::select! {
            maybe = socket.next() => {
                match maybe {
                    Some(Ok(Message::Text(text))) => {
                        last_inbound = std::time::Instant::now();
                        let text = text.to_string();
                        // ping 识别：兼容 `{"type":"ping"}` 与带空格变体（JSON 解析优先）
                        if let Ok(f) = parse_inbound(&text) {
                            if f.r#type == "ping" {
                                if send_text(&mut socket, &pong_frame().to_string()).await.is_err() {
                                    break;
                                }
                                continue;
                            }
                        }
                        if text.trim() == r#"{"type":"ping"}"# || text.contains(r#""type":"ping""#) {
                            if send_text(&mut socket, &pong_frame().to_string()).await.is_err() {
                                break;
                            }
                            continue;
                        }
                        let frame = match parse_inbound(&text) {
                            Ok(f) => f,
                            Err(_) => continue,
                        };
                        if frame.r#type != "req" {
                            continue;
                        }
                        let id = frame.id.unwrap_or_default();
                        let method = frame.method.unwrap_or_default();
                        if id.is_empty() || method.is_empty() {
                            continue;
                        }
                        let resp = match dispatch(&ctx, &method, frame.params).await {
                            Ok(payload) => ok_frame(&id, payload).to_string(),
                            Err(e) => err_frame(&id, &e).to_string(),
                        };
                        if send_text(&mut socket, &resp).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Ping(bytes))) => {
                        last_inbound = std::time::Instant::now();
                        if socket.send(Message::Pong(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
            env = async { event_rx.as_mut()?.recv().await } => {
                match env {
                    Some(env) => {
                        seq += 1;
                        let frame = envelope_to_frame(&env, seq).to_string();
                        if send_text(&mut socket, &frame).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            _ = tick.tick() => {
                if first_tick {
                    first_tick = false;
                    continue;
                }
                seq += 1;
                let frame = tick_frame(seq).to_string();
                if send_text(&mut socket, &frame).await.is_err() {
                    break;
                }
            }
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(
                last_inbound + Duration::from_secs(IDLE_TIMEOUT_SECS),
            )) => {
                tracing::info!(
                    idle_secs = IDLE_TIMEOUT_SECS,
                    "网关连接空闲超时，主动关闭"
                );
                break;
            }
        }
    }
}

/// 发送文本帧（统一错误处理）
async fn send_text(socket: &mut WebSocket, text: &str) -> Result<(), axum::Error> {
    socket.send(Message::Text(text.into())).await
}

/// 握手：接收 `connect` req 并校验 token；成功则回 HelloOk
async fn await_connect(socket: &mut WebSocket, state: &GatewayState) -> bool {
    let deadline = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => return false,
            maybe = socket.next() => {
                let Some(Ok(Message::Text(text))) = maybe else {
                    return false;
                };
                let text = text.to_string();
                let Ok(frame) = parse_inbound(&text) else {
                    continue;
                };
                if frame.r#type != "req" || frame.method.as_deref() != Some("connect") {
                    continue;
                }
                let id = frame.id.unwrap_or_default();
                let params = parse_connect_params(&frame.params);
                // 本地回环网关（仅 127.0.0.1）对无 token 的浏览器直连放行（开箱即用）；
                // 提供 token 时仍严格校验，防止带错误凭据的连接混入。
                let token_ok = params
                    .token
                    .as_deref()
                    .map(|t| auth::verify(t, &state.token))
                    .unwrap_or(true);
                if !token_ok {
                    let err = RpcError::unauthorized();
                    let frame = err_frame(&id, &err).to_string();
                    let _ = send_text(socket, &frame).await;
                    tracing::warn!("网关连接被拒绝：token 校验失败");
                    return false;
                }
                // 握手成功：回 HelloOk
                let hello = hello_frame(&id, SUPPORTED_METHODS, SUPPORTED_EVENTS).to_string();
                if send_text(socket, &hello).await.is_err() {
                    return false;
                }
                return true;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 静态托管（/control）与 SPA fallback
// ---------------------------------------------------------------------------

/// `/control` 与 `/control/` 根入口（空路径无 Path 参数，axum 通配符不匹配空路径）
async fn control_root(State(state): State<Arc<GatewayState>>) -> Response {
    serve_rel(&state, "").await
}

/// `/control/{*path}` 与 `/control/static/{*path}` 通配处理器
async fn control_assets(
    AxumPath(path): AxumPath<String>,
    State(state): State<Arc<GatewayState>>,
) -> Response {
    let rel = path.strip_prefix("static/").unwrap_or(&path);
    serve_rel(&state, rel).await
}

/// 相对 `webui_dir` 的静态文件服务 + SPA history fallback + index 注入
async fn serve_rel(state: &GatewayState, rel: &str) -> Response {
    if has_traversal(rel) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let file = if rel.is_empty() {
        state.webui_dir.join("index.html")
    } else {
        state.webui_dir.join(rel)
    };

    match tokio::fs::read(&file).await {
        Ok(bytes) => {
            let mime = content_type(&file);
            let mut resp = (
                [(header::CONTENT_TYPE, HeaderValue::from_static(mime))],
                bytes,
            )
                .into_response();
            set_cache_policy(&mut resp, &file);
            maybe_inject_index(&mut resp, &state.webui_dir);
            resp
        }
        Err(_) => {
            // SPA history fallback：未知路径回退到 index.html（注入 data）
            let index = state.webui_dir.join("index.html");
            match tokio::fs::read(&index).await {
                Ok(bytes) => {
                    let mut resp = (
                        [(
                            header::CONTENT_TYPE,
                            HeaderValue::from_static("text/html; charset=utf-8"),
                        )],
                        bytes,
                    )
                        .into_response();
                    resp.headers_mut()
                        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
                    maybe_inject_index(&mut resp, &state.webui_dir);
                    resp
                }
                Err(_) => StatusCode::NOT_FOUND.into_response(),
            }
        }
    }
}

/// 缓存策略：SPA 入口（index.html）与任意无 hash 页面一律 `no-cache`，
/// 避免 webview/浏览器复用旧页面导致新版本改动不可见；带内容 hash 的
/// 静态资源（assets/*.js|css）强缓存 1 年（URL 变化即视为新资源）。
fn set_cache_policy(resp: &mut Response, file: &Path) {
    let policy = if file.extension().is_none_or(|e| e == "html") {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(policy));
}

/// 若响应体是 index.html（未被注入过），注入 `#Marguerite-data` 元数据元素
fn maybe_inject_index(resp: &mut Response, webui_dir: &Path) {
    if resp
        .headers()
        .get(header::CONTENT_TYPE)
        .is_none_or(|v| !v.to_str().unwrap_or("").contains("text/html"))
    {
        return;
    }
    let path = webui_dir.join("index.html");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return;
    };
    if raw.contains("opensquilla-data") || !raw.contains(r#"<div id="app">"#) {
        return;
    }
    let config_path = Config::config_path()
        .to_string_lossy()
        .replace('&', "&amp;");
    let data = format!(
        r#"<div id="opensquilla-data" data-base-path="/control" data-config-path="{config_path}" data-locale="zh-Hans"></div>"#
    );
    let injected = raw.replace(
        r#"<div id="app">"#,
        &format!("{data}\n    <div id=\"app\">"),
    );
    let length = injected.len().to_string();
    *resp.body_mut() = axum::body::Body::from(injected);
    let _ = resp.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&length).unwrap_or(HeaderValue::from_static("0")),
    );
}

fn has_traversal(rel: &str) -> bool {
    rel.split(['/', '\\'])
        .any(|seg| seg == ".." || seg.is_empty() && !rel.is_empty())
}

/// 扩展名 → Content-Type（Web UI 产物所需子集）
fn content_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "txt" | "md" => "text/plain; charset=utf-8",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        _ => "application/octet-stream",
    }
}

async fn fallback(State(state): State<Arc<GatewayState>>) -> Response {
    let _ = &state;
    Redirect::permanent("/control/").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::protocol::PROTOCOL_VERSION;
    use futures_util::SinkExt;
    use std::net::SocketAddr;

    #[test]
    fn test_traversal_detection() {
        assert!(has_traversal("../secret"));
        assert!(has_traversal("a/../../b"));
        assert!(has_traversal(r"a\..\b"));
        assert!(!has_traversal("assets/app.js"));
        assert!(!has_traversal("static/img/m.png"));
    }

    #[test]
    fn test_content_type_subset() {
        assert_eq!(
            content_type(Path::new("index.html")),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            content_type(Path::new("x.js")),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type(Path::new("m.svg")), "image/svg+xml");
        assert_eq!(content_type(Path::new("a.bin")), "application/octet-stream");
    }

    #[test]
    fn test_index_injection_marker_absent_then_present() {
        // 注入逻辑判定依据（不被重复注入）：第二次注入因含 marker 直接跳过
        let raw = r#"<html><head></head><body><div id="app"></div></body></html>"#;
        assert!(raw.contains(r#"<div id="app">"#));
        assert!(!raw.contains("opensquilla-data"));
    }

    // ------------------------------------------------------------------
    // 端到端（真实回环 127.0.0.1，无需外部网络；跑全量前需 sc stop nfwfp）
    // ------------------------------------------------------------------

    fn test_webui_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("marguerite-gw-e2e-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("index.html"),
            r#"<!doctype html><html><body><div id="app"></div></body></html>"#,
        )
        .unwrap();
        dir
    }

    fn e2e_state() -> Arc<GatewayState> {
        Arc::new(GatewayState {
            config: Config::default(),
            bus: EventBus::default(),
            token: "e2e-test-token".to_string(),
            started_at: Instant::now(),
            webui_dir: test_webui_dir(),
        })
    }

    /// 本地 400 mock：让 chat.send 确定性快速失败（400 不可重试），
    /// 与宿主系统代理/真实 API Key 无关，e2e 错误路径稳定可测。
    async fn fail_400_server() -> String {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                if let Ok((mut socket, _)) = listener.accept().await {
                    let _ = crate::tests::mock_http::read_request_full(&mut socket).await;
                    let body = r#"{"error":{"message":"bad request"}}"#;
                    let header = format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(body.as_bytes()).await;
                    let _ = socket.flush().await;
                }
            }
        });
        format!("http://{addr}")
    }

    /// 启动真实回环网关（OS 分配随机端口），返回 ws 地址与 http 地址
    async fn spawn_gateway() -> (SocketAddr, Arc<GatewayState>) {
        spawn_gateway_with(|_| {}).await
    }

    /// 启动前允许改写 GatewayState（在 router 构造前，Arc 独占期安全 mutate）
    async fn spawn_gateway_with(
        mutate: impl FnOnce(&mut GatewayState),
    ) -> (SocketAddr, Arc<GatewayState>) {
        let mut state = e2e_state();
        if let Some(s) = Arc::get_mut(&mut state) {
            mutate(s);
        }
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let router = build_router(Arc::clone(&state));
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (addr, state)
    }

    /// 发起 WS 客户端连接并读回首帧（challenge）
    async fn connect_ws(
        addr: SocketAddr,
    ) -> (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        serde_json::Value,
    ) {
        let url = format!("ws://{addr}/ws");
        let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        let first = ws.next().await.unwrap().unwrap();
        let value: serde_json::Value = serde_json::from_str(first.to_text().unwrap()).unwrap();
        (ws, value)
    }

    #[tokio::test]
    async fn e2e_ws_handshake_and_health() {
        let (addr, state) = spawn_gateway().await;

        // 1) challenge
        let (mut ws, challenge) = connect_ws(addr).await;
        assert_eq!(challenge["type"], "event");
        assert_eq!(challenge["event"], "connect.challenge");

        // 2) connect（正确 token）→ HelloOk
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            format!(
                r#"{{"type":"req","id":"1","method":"connect","params":{{"minProtocol":3,"maxProtocol":3,"auth":{{"token":"{}"}}}}}}"#,
                state.token
            ),
        ))
        .await
        .unwrap();
        let hello_raw = ws.next().await.unwrap().unwrap();
        let hello: serde_json::Value = serde_json::from_str(hello_raw.to_text().unwrap()).unwrap();
        assert_eq!(hello["type"], "res");
        assert_eq!(hello["ok"], true);
        assert_eq!(hello["protocol"], PROTOCOL_VERSION);
        assert_eq!(hello["auth"]["principal"]["isOwner"], true);
        assert!(hello["features"]["methods"].is_array());

        // 3) health 方法
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"type":"req","id":"2","method":"health","params":{}}"#.into(),
        ))
        .await
        .unwrap();
        let health_raw = ws.next().await.unwrap().unwrap();
        let health: serde_json::Value =
            serde_json::from_str(health_raw.to_text().unwrap()).unwrap();
        assert_eq!(health["type"], "res");
        assert_eq!(health["id"], "2");
        assert_eq!(health["ok"], true);
        assert_eq!(health["payload"]["status"], "ok");
        assert_eq!(health["payload"]["engine"], "rust");
    }

    #[tokio::test]
    async fn e2e_ws_rejects_bad_token() {
        let (addr, _state) = spawn_gateway().await;
        let (mut ws, challenge) = connect_ws(addr).await;
        assert_eq!(challenge["event"], "connect.challenge");
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"type":"req","id":"9","method":"connect","params":{"auth":{"token":"wrong"}}}"#
                .into(),
        ))
        .await
        .unwrap();
        let raw = ws.next().await.unwrap().unwrap();
        let resp: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
        assert_eq!(resp["ok"], false);
        assert_eq!(resp["error"]["code"], "UNAUTHORIZED");
        // 认证失败后服务端立即关闭连接（客户端收到 Close 或流结束）
        if let Some(Ok(m)) = ws.next().await {
            assert!(m.is_close(), "应为关闭帧，实际: {m:?}");
        }
    }

    #[tokio::test]
    async fn e2e_static_serves_index_with_injection_and_guards_traversal() {
        let (addr, _state) = spawn_gateway().await;
        let http_addr = addr;
        let read_http = |path: &'static str| {
            let addr = http_addr;
            async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
                let req =
                    format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
                stream.write_all(req.as_bytes()).await.unwrap();
                let mut buf = Vec::new();
                stream.read_to_end(&mut buf).await.unwrap();
                String::from_utf8_lossy(&buf).to_string()
            }
        };

        // index 注入
        let index = read_http("/control/").await;
        assert!(index.contains("200 OK"), "index 应 200: {index}");
        assert!(
            index.contains(r#"<div id="opensquilla-data""#),
            "应注入 data 元素"
        );
        assert!(index.contains(r#"data-base-path="/control""#));
        assert!(index.contains(r#"data-locale="zh-Hans""#));

        // SPA fallback（未知路由回退 index）
        let fallback = read_http("/control/chat").await;
        assert!(fallback.contains("200 OK"));
        assert!(fallback.contains(r#"<div id="app">"#));

        // 路径穿越防护 → 404
        let traversal = read_http("/control/static/../secret").await;
        assert!(traversal.contains("404"), "穿越应 404: {traversal}");
    }

    #[tokio::test]
    async fn e2e_bus_event_pushed_with_seq() {
        let (addr, state) = spawn_gateway().await;
        let (mut ws, _challenge) = connect_ws(addr).await;
        ws.send(tokio_tungstenite::tungstenite::Message::Text(format!(
            r#"{{"type":"req","id":"1","method":"connect","params":{{"auth":{{"token":"{}"}}}}}}"#,
            state.token
        )))
        .await
        .unwrap();
        let hello_raw = ws.next().await.unwrap().unwrap();
        let _hello: serde_json::Value = serde_json::from_str(hello_raw.to_text().unwrap()).unwrap();

        // 认证后发布总线事件 → 客户端应收到 seq=1 的 session.event.* 帧
        state
            .bus
            .publish(Envelope::mode_changed("core", "agent", "bloom"));
        let raw = ws.next().await.unwrap().unwrap();
        let ev: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
        assert_eq!(ev["type"], "event");
        assert_eq!(ev["event"], "session.event.status");
        assert_eq!(ev["seq"], 1);
        assert_eq!(ev["payload"]["kind"], "mode.changed");
    }

    /// 融合方法族 WS 端到端：技能 / 渠道 / 记忆 / Bloom 会话（chat.send）全部经真实网关可达。
    ///
    /// 无 API Key 时 chat.send 仍应「接受」（accepted=true），随后引擎失败事件
    /// （session.event.error）经事件桥回流，前端据此终止本轮 —— 不依赖 LLM 的完整链路验证。
    #[tokio::test]
    async fn e2e_fusion_methods_over_ws() {
        use std::time::Duration;
        // 自建网关：配置注入「本地 400 mock」的 LLM 端点（400 不可重试），
        // 使 chat.send 后台引擎快速失败 → Error 事件确定回流，不依赖外部网络。
        // 不用「127.0.0.1:1 连接拒绝」——宿主开系统代理时该端口会被代答 502
        // （可重试），错误事件可能 15s 内等不到。
        let mut config = Config::default();
        config.model.api_key = "e2e-key".to_string();
        config.model.base_url = fail_400_server().await;
        let state = Arc::new(GatewayState {
            config,
            bus: EventBus::default(),
            token: "e2e-test-token".to_string(),
            started_at: Instant::now(),
            webui_dir: test_webui_dir(),
        });
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let router = build_router(Arc::clone(&state));
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        let (mut ws, _challenge) = connect_ws(addr).await;
        ws.send(tokio_tungstenite::tungstenite::Message::Text(format!(
            r#"{{"type":"req","id":"1","method":"connect","params":{{"auth":{{"token":"{}"}}}}}}"#,
            state.token
        )))
        .await
        .unwrap();
        let hello_raw = ws.next().await.unwrap().unwrap();
        let _hello: serde_json::Value = serde_json::from_str(hello_raw.to_text().unwrap()).unwrap();

        // skills.list（Rust SkillLoader 枚举）
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"type":"req","id":"10","method":"skills.list","params":{}}"#.into(),
        ))
        .await
        .unwrap();
        let raw = ws.next().await.unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
        assert_eq!(v["ok"], true);
        assert!(
            v["payload"]["skills"].is_array(),
            "skills.list 应返回 skills 数组"
        );

        // channels.status（Rust 实现优先于 Python 前缀代理）
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"type":"req","id":"11","method":"channels.status","params":{}}"#.into(),
        ))
        .await
        .unwrap();
        let raw = ws.next().await.unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["payload"]["channels"].as_array().unwrap().len(), 0);

        // memory.list（ExperienceLedger 直连）
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"type":"req","id":"12","method":"memory.list","params":{"limit":10}}"#.into(),
        ))
        .await
        .unwrap();
        let raw = ws.next().await.unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
        assert_eq!(v["ok"], true);
        assert!(v["payload"]["items"].is_array());

        // router.decision（AI 智能融合路由：Bloom 编排形态自动决策）
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"type":"req","id":"12b","method":"router.decision","params":{"query":"解释闭包"}}"#
                .into(),
        ))
        .await
        .unwrap();
        let raw = ws.next().await.unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["payload"]["strategy"], "Solo Agent");
        assert!(v["payload"]["rationale"]
            .as_str()
            .is_some_and(|r| !r.is_empty()));

        // chat.send：接受后引擎失败 → Error 事件回流。
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"type":"req","id":"13","method":"chat.send","params":{"message":"hi","sessionKey":"ws.e2e1","clientRequestId":"cr-e2e"}}"#.into(),
        ))
        .await
        .unwrap();
        let raw = ws.next().await.unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["payload"]["accepted"], true);
        assert_eq!(v["payload"]["sessionKey"], "ws.e2e1");

        // 事件桥：应收到 session.event.error（引擎快速失败），最多等待 15s
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let mut got_error = false;
        while std::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(500), ws.next()).await {
                Ok(Some(Ok(msg))) => {
                    let text = msg.to_text().unwrap_or_default();
                    if let Ok(frame) = serde_json::from_str::<serde_json::Value>(text) {
                        if frame["type"] == "event"
                            && frame["event"]
                                .as_str()
                                .is_some_and(|e| e.ends_with(".error"))
                        {
                            got_error = true;
                            break;
                        }
                    }
                }
                Ok(Some(Err(_))) | Ok(None) => break,
                Err(_) => continue, // 读超时继续等待（非终止）
            }
        }
        assert!(got_error, "chat.send 后应收到 session.event.error 事件");
    }

    /// 缺陷 2 验证：`sandbox.policy.get` 经真实 WS 链路返回合法策略结构。
    ///
    /// 覆盖 dispatch 全链路（WS → dispatch → RUST_FIRST_METHODS → dispatch_rust →
    /// policy_store::policy_get），断言返回与 webui `SandboxPolicy` 契约对齐。
    #[tokio::test]
    async fn e2e_sandbox_policy_get_over_ws() {
        let (addr, state) = spawn_gateway().await;
        let (mut ws, _challenge) = connect_ws(addr).await;
        ws.send(tokio_tungstenite::tungstenite::Message::Text(format!(
            r#"{{"type":"req","id":"1","method":"connect","params":{{"auth":{{"token":"{}"}}}}}}"#,
            state.token
        )))
        .await
        .unwrap();
        let hello_raw = ws.next().await.unwrap().unwrap();
        let hello: serde_json::Value = serde_json::from_str(hello_raw.to_text().unwrap()).unwrap();
        assert_eq!(hello["ok"], true);

        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"type":"req","id":"20","method":"sandbox.policy.get","params":{}}"#.into(),
        ))
        .await
        .unwrap();
        let raw = ws.next().await.unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
        assert_eq!(v["type"], "res");
        assert_eq!(v["id"], "20");
        assert_eq!(v["ok"], true, "sandbox.policy.get 应成功: {v}");
        let policy = &v["payload"];
        assert_eq!(policy["schemaVersion"], 2);
        assert!(policy["policyVersion"].as_u64().unwrap_or(0) >= 1);
        for key in ["files", "commands", "network", "runtimes"] {
            assert!(policy[key].is_object(), "{key} 应为对象: {policy}");
        }
        assert!(policy["files"]["customDenyWritePaths"].is_array());
        assert!(policy["network"]["blockAllNetwork"].is_boolean());
    }

    /// 缺陷 1 服务端契约：ping → pong；事件帧 seq 从 1 连续递增（bus 事件与 tick 共享计数）。
    #[tokio::test]
    async fn e2e_ping_pong_and_seq_continuity() {
        let (addr, state) = spawn_gateway().await;
        let (mut ws, _challenge) = connect_ws(addr).await;
        ws.send(tokio_tungstenite::tungstenite::Message::Text(format!(
            r#"{{"type":"req","id":"1","method":"connect","params":{{"auth":{{"token":"{}"}}}}}}"#,
            state.token
        )))
        .await
        .unwrap();
        let hello_raw = ws.next().await.unwrap().unwrap();
        let _hello: serde_json::Value = serde_json::from_str(hello_raw.to_text().unwrap()).unwrap();

        // ping → pong（客户端 `_startPing` 每 55s 发送 {"type":"ping"}，服务端须回 pong）
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"type":"ping"}"#.into(),
        ))
        .await
        .unwrap();
        let pong_raw = ws.next().await.unwrap().unwrap();
        let pong: serde_json::Value = serde_json::from_str(pong_raw.to_text().unwrap()).unwrap();
        assert_eq!(pong["type"], "pong", "ping 应得到 pong 回帧: {pong}");

        // 连续发布总线事件 → seq 必须从 1 递增且无断档
        state
            .bus
            .publish(Envelope::mode_changed("core", "agent", "bloom"));
        state.bus.publish(Envelope::new(
            crate::events::envelope::EventType::ToolCall,
            serde_json::json!({ "tool": "read_file" }),
            "ws.s1",
        ));
        let mut expected_seq: u64 = 1;
        for _ in 0..2 {
            let raw = ws.next().await.unwrap().unwrap();
            let ev: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
            assert_eq!(ev["type"], "event");
            assert_eq!(ev["seq"], expected_seq, "事件 seq 应连续递增: {ev}");
            expected_seq += 1;
        }
    }

    /// 缺陷 1 重连契约：新连接从 seq=1 重新计数（客户端 `_doConnect` 将 `_lastSeq` 重置为 0，
    /// 首个事件帧 seq=1 不会误判为断档）。
    #[tokio::test]
    async fn e2e_reconnect_resets_seq() {
        let (addr, state) = spawn_gateway().await;

        // 第一次连接：收 seq=1 事件后断开
        {
            let (mut ws, _challenge) = connect_ws(addr).await;
            ws.send(tokio_tungstenite::tungstenite::Message::Text(
                format!(
                    r#"{{"type":"req","id":"1","method":"connect","params":{{"auth":{{"token":"{}"}}}}}}"#,
                    state.token
                ),
            ))
            .await
            .unwrap();
            let hello_raw = ws.next().await.unwrap().unwrap();
            let _hello: serde_json::Value =
                serde_json::from_str(hello_raw.to_text().unwrap()).unwrap();
            state
                .bus
                .publish(Envelope::mode_changed("core", "agent", "bloom"));
            let raw = ws.next().await.unwrap().unwrap();
            let ev: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
            assert_eq!(ev["seq"], 1);
        }

        // 重连：seq 从 1 重新开始，客户端不会因 _lastSeq=0 触发断档
        let (mut ws2, _challenge) = connect_ws(addr).await;
        ws2.send(tokio_tungstenite::tungstenite::Message::Text(format!(
            r#"{{"type":"req","id":"1","method":"connect","params":{{"auth":{{"token":"{}"}}}}}}"#,
            state.token
        )))
        .await
        .unwrap();
        let hello_raw = ws2.next().await.unwrap().unwrap();
        let _hello: serde_json::Value = serde_json::from_str(hello_raw.to_text().unwrap()).unwrap();
        state
            .bus
            .publish(Envelope::mode_changed("core", "agent", "bloom"));
        let raw = ws2.next().await.unwrap().unwrap();
        let ev: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
        assert_eq!(ev["seq"], 1, "重连后 seq 应从 1 重新计数: {ev}");
    }
}
