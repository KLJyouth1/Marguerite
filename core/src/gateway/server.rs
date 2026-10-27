//! 统一网关 HTTP/WebSocket 服务（仅回环 127.0.0.1）
//!
//! 路由：
//! - `/ws`                     → WebSocket JSON-RPC（Marguerite 协议）
//! - `/control/{*path}`        → Web UI 静态产物（SPA history fallback + index 注入）
//! - `/control/static/{*path}` → 后端静态资源（品牌图、分享图）
//! - `/api/health`             → REST 健康探测
//! - `/`                       → 重定向到 `/control/`
//!
//! 安全：只绑定 `127.0.0.1`；`connect` 握手在 token 已配置时**强制**校验
//! （缺失 token 与错误 token 同路拒绝，F-T4-01）；WS 升级阶段做 Origin
//! 白名单校验（无 Origin 头的原生客户端放行，由 token 兜底）；静态路径做
//! `..` / 反斜杠穿越防护。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{any, get};
use axum::Router;
use futures_util::StreamExt;
use serde_json::Value;
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

/// 侧车启动阶段。
///
/// # 为什么需要它（2026-10-22 实测的「一启动就报错、点重试才连上」）
///
/// 启动顺序是：Tauri 拉起 Rust 网关（**立刻**监听 18790）→ Rust 网关**异步**启动
/// Python 侧车（冷启动约 6 秒，日志 `startup_elapsed_ms=5991`）→ 侧车就绪。
///
/// 而 WebView 在网关一监听上就加载并连上 WS，于是那 6 秒内前端发起的每个
/// Python 方法都落到 `RpcError::not_implemented` —— 前端把它当**错误**展示，
/// 用户看到"报错"，点重试时侧车已就绪，于是"重试就好了"。
///
/// 有了这个阶段标记，dispatch 就能区分：
/// - `Starting` → 回 `SIDECAR_STARTING`（前端据此显示"初始化中"并自动重试，**不弹错误**）
/// - `Ready` / `Failed` → 仍回 `NOT_IMPLEMENTED`（此时它是真的不可用，必须如实报错）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarPhase {
    /// 正在启动（进程已拉起或尚未拉起，健康探测未完成）
    Starting,
    /// 已就绪，WS 客户端可转发
    Ready,
    /// 启动失败 / 握手失败 —— 已降级纯 Rust 模式
    Failed,
}

static SIDECAR_PHASE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub fn sidecar_phase() -> SidecarPhase {
    match SIDECAR_PHASE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => SidecarPhase::Ready,
        2 => SidecarPhase::Failed,
        _ => SidecarPhase::Starting,
    }
}

fn set_sidecar_phase(phase: SidecarPhase) {
    let v = match phase {
        SidecarPhase::Starting => 0,
        SidecarPhase::Ready => 1,
        SidecarPhase::Failed => 2,
    };
    SIDECAR_PHASE.store(v, std::sync::atomic::Ordering::Relaxed);
}

async fn set_py_registry(sidecar: Arc<PyEngineSidecar>, client: Arc<PyGatewayClient>) {
    let _ = PY_SIDECAR_REG.set(sidecar);
    let _ = PY_CLIENT_REG.set(client);
    // 注册即就绪：此后 Python 方法可正常转发。
    set_sidecar_phase(SidecarPhase::Ready);
}

/// 网关共享状态
pub struct GatewayState {
    pub config: Config,
    pub bus: EventBus,
    pub token: String,
    pub started_at: Instant,
    pub webui_dir: PathBuf,
    /// 生产限流令牌桶（W3.4）。只作用于 `/api/*` REST；`/api/health` 与 WS 豁免。
    pub rate_limiter: Arc<crate::gateway::rate_limit::RateLimiter>,
    pub rate_limit_enabled: bool,
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
        let bucket = crate::gateway::rate_limit::BucketConfig::new(
            config.gateway.rate_limit_capacity(),
            config.gateway.rate_limit_refill(),
        );
        let rate_limiter = Arc::new(crate::gateway::rate_limit::RateLimiter::new(bucket, bucket));
        let rate_limit_enabled = config.gateway.rate_limit_is_enabled();
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
            rate_limiter,
            rate_limit_enabled,
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
            tracing::error!(err = %e, "侧车自动探测失败：已降级纯 Rust 模式，全部 Python 方法（logs.tail / config.get / onboarding.* …）将回 NOT_IMPLEMENTED");
            set_sidecar_phase(SidecarPhase::Failed);
            return;
        }
    };
    // 先尝试**复用**一个已在运行的侧车。Python 侧车对同一 profile 是单实例：
    // 上一次 Marguerite 被强杀时 `kill_on_drop` 不会执行，侧车会变成孤儿继续占
    // profile 锁，此后「再拉一个」必然秒退（OPENSQUILLA_PROFILE_IN_USE），于是
    // py_client() 恒为 None —— 日志页/设置页等所有侧车方法静默失效。复用是治本路径。
    //
    // 判定只能用**真实的网关 WS 握手**：同一侧车进程还会监听别的内部端口（实测
    // 32484 同时听 54069 与 61750，54069 对 /health 回 404），而 `check_health`
    // 把 404 也算「活着」——用探活挑端口会挑错。
    let mut reused: Option<(Arc<PyEngineSidecar>, Arc<PyGatewayClient>)> = None;
    for cand in crate::pyengine::sidecar::candidate_sidecars(&cfg) {
        // P1-7 修复（实测双侧车根因）：先做**轻量可达性探测**（只握手，不建
        // 监督任务），**只有**真实可连的候选才 `attach`（attach 会立刻 spawn
        // `run_sidecar_supervisor`）。旧写法对每个候选先 `attach` 再 connect，
        // connect 失败时 `attached` 被 drop 但 tokio `JoinHandle` 不会被 abort，
        // 那个 Attached 监督任务继续跑：发现附着 pid 已死就 `spawn_once` 接管，
        // 在候选端口上偷偷拉起第二个侧车——随后 `start()` 又在新端口拉起自有侧车，
        // 于是出现「两个 python 进程 / 一个孤儿不在注册表」。
        let probe_url = format!("http://127.0.0.1:{}", cand.port);
        match PyGatewayClient::connect(&probe_url, &state.token).await {
            Ok(c) => {
                // 可达：此刻才 attach（spawn 监督任务，传入 cand.pid 靠它判活并
                // 采集退出证据）。拿不到 pid 时 `attach` 内部不启守护——降级而不
                // 是猜测，见 `SidecarCandidate::pid`。
                let attached = Arc::new(PyEngineSidecar::attach(
                    cfg.clone().with_port(cand.port),
                    cand.pid,
                ));
                tracing::info!(
                    port = cand.port,
                    pid = ?cand.pid,
                    "复用已在运行的 Python 侧车（避免 profile 单实例锁冲突）"
                );
                reused = Some((attached, Arc::new(c)));
                break;
            }
            Err(e) => {
                tracing::debug!(err = %e, port = cand.port, "候选端口不是可用网关，继续尝试下一个（未建监督任务）");
            }
        }
    }
    let (sidecar, client) = match reused {
        Some(pair) => pair,
        None => {
            let s = match PyEngineSidecar::start(cfg).await {
                Ok(s) => Arc::new(s),
                Err(e) => {
                    // 不再静默：这条是「日志页/设置页整片失效」的唯一根因，必须显式可见。
                    tracing::error!(
                        err = %e,
                        "侧车启动失败：已降级纯 Rust 模式；全部 Python 方法（logs.tail / logs.status / config.get / onboarding.* / channels.* …）会返回 NOT_IMPLEMENTED"
                    );
                    set_sidecar_phase(SidecarPhase::Failed);
                    return;
                }
            };
            match PyGatewayClient::connect(&s.base_url(), &state.token).await {
                Ok(c) => (s, Arc::new(c)),
                Err(e) => {
                    tracing::error!(err = %e, "Python WS 握手失败：已降级纯 Rust 模式");
                    set_sidecar_phase(SidecarPhase::Failed);
                    return;
                }
            }
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
        // 审批 REST 面：WebUI 的审批卡片**只走 REST** 做快照与裁决
        // （`useChatApprovals.ts`），而抬升审批的 chat.send 是 Rust 实现 ——
        // 少了这两条，控制台点「允许」会拿到 501，链式执行器只能等超时 fail-closed。
        .route(
            "/api/approvals",
            get(crate::gateway::approvals::approvals_list),
        )
        .route(
            "/api/approvals/resolve",
            axum::routing::post(crate::gateway::approvals::approvals_resolve),
        )
        // 产物取字节：控制台的下载按钮 / 内联文本编辑**只走这条 REST**
        // （`artifacts.ts::artifactDownloadUrl` 对「有 id 无 download_url」的产物拼这个路径）。
        // 缺它 → 501 → 前端弹「下载失败」；`open_in_system`/`edit_inline` 另有 RPC 通道，不受影响。
        .route(
            "/api/v1/artifacts/{id}",
            get(crate::gateway::artifacts::artifact_download),
        )
        // 更新横幅状态探测（webui UpdateBanner.vue 的 `GET /api/system/update`）：
        // 侧车在线走真值，离线时如实回「无更新通道」，**不再落到通配代理回 501**
        // （真机实测：每次页面加载都在浏览器控制台留一条 501 Not Implemented）。
        .route("/api/system/update", get(api_system_update))
        // 其它 /api/* → 侧车 REST 代理（侧车离线才回 501 JSON，见 sidecar_proxy 模块头）。
        // 必须**明确**回 JSON，不能落进 SPA 静态回退（否则前端 JSON.parse 抛错、失败被静默吞掉）。
        // 用 any() 覆盖所有方法；上面按方法注册的具体端点（如 /api/health、/api/approvals*、
        // /api/v1/artifacts/{id}）在 axum 里优先匹配，仍由 Rust 直接服务。
        .route("/api/{*path}", any(crate::gateway::sidecar_proxy::api_proxy))
        // axum 通配符 `{*path}` 不匹配空路径，根路径单独注册
        .route("/control", get(control_root))
        .route("/control/", get(control_root))
        .route("/control/{*path}", get(control_spa_assets))
        .route("/control/static/{*path}", get(control_static_assets))
        .fallback(fallback)
        .with_state(state.clone())
        // W3.4 生产限流：只限 /api/* REST（/api/health 与 WS/静态资源豁免）。
        // 超限回 HTTP 429 + JSON-RPC 错误体；缺这一层时占位实现的 rate_limit
        // 模块有测试、有实现，却对真实流量零作用（"模块完整但零引用"）。
        .layer(axum::middleware::from_fn_with_state(
            state,
            api_rate_limit_layer,
        ))
        // Control UI 安全响应头（CSP / X-Frame-Options / nosniff / Referrer-Policy）：
        // 挂在最外层，覆盖静态托管**与** SPA fallback 两条产出路径。
        .layer(axum::middleware::from_fn(control_ui_security_headers_layer))
        // 路径穿越入口闸（F-T4-02）：在路由匹配前按**解码后**路径拒绝。
        // 必须在路由前：`/control/static/../x` 可能被路由归一成 `/control/x`，
        // 到了 handler 已看不出原始 `..`；更不能用「SPA 回退 200」掩盖。
        .layer(axum::middleware::from_fn(control_traversal_guard_layer))
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
    // v3.3.1：审批门超时注入（guard.approval_timeout_secs，0 = 保持默认 60s）。
    // 必须在任何 ApprovalGate 构造前执行——ModeManager/ChainExecutor 均在
    // 首次 chat.send 时才惰性建门。
    crate::mode::set_approval_timeout_secs(state.config.guard.approval_timeout_secs);
    // 侧车在后台异步启动（不阻塞网关 HTTP/WS 服务；失败自动降级纯 Rust 模式）。
    // 之前用 timeout(3s) 在 detect 阶段就被取消导致 channels.* 恒不可用，现改为
    // 常驻后台任务，Python 3.12+ 探测与启动完成后自动注册 py_client() 供方法代理。
    tokio::spawn(maybe_bootstrap_pyengine(state.clone()));

    // P0 S4b：启动即装配插件（patch → PluginSlot），结果经 OnceLock 复用，重启生效。
    let asm = crate::gateway::plugin_status::warm_assembly();
    let assembled = asm.manifest_status.iter().filter(|m| m.assembled).count();
    tracing::info!(
        manifests = asm.manifests.len(),
        patches = asm.patches.len(),
        assembled = assembled,
        applied = asm.report.applied.len(),
        skipped = asm.report.skipped.len(),
        errors = asm.report.errors.len(),
        "插件装配完成（patch → PluginSlot；重启生效）"
    );

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
        // R14：桌面壳在「端口已被占用」时要判断占用者是**本产品的网关**还是别的程序
        // （决定「复用现有网关」还是「报错退出」）。只靠 `engine` 字段无法判断版本是否
        // 兼容，故这里如实上报构建版本，让附着方能把「版本不一致」告知用户，
        // 而不是静默连上一个旧版网关、再让界面出现无法解释的契约错误。
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_ms": state.started_at.elapsed().as_millis(),
    }))
}

/// `GET /api/system/update`：控制台更新横幅的状态探测。
///
/// 控制台（`webui/src/components/UpdateBanner.vue`）在**浏览器**表面会轮询这条
/// REST；此前它没有 Rust 路由，落到 `/api/{*path}` 通配代理 —— 侧车在线时转发真值，
/// 侧车离线（纯 Rust 托管，即打包环境）时回 501，于是**每次页面加载都在浏览器控制台
/// 留下一条 501**（真机实测 25 次导航 = 25 条）。
///
/// 修法：本路由**先问侧车**（保持双引擎下更新检查的真值不回退），侧车不可用/不支持时
/// 如实回一份「本部署无更新通道」的契约（`available:false`），前端据此不显示横幅、
/// 也不再有 501 噪声。这不是伪造「无更新」，而是把「Rust 网关不承担更新检查」这一
/// 事实显式化 —— 更新由桌面壳或侧车负责。
async fn api_system_update(
    State(state): State<Arc<GatewayState>>,
    uri: axum::http::Uri,
    headers: HeaderMap,
) -> Response {
    if let Some(resp) = crate::gateway::sidecar_proxy::forward_get(&state, &uri, &headers).await {
        if resp.status().is_success() {
            return resp;
        }
    }
    (
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "current": env!("CARGO_PKG_VERSION"),
            "latest": Value::Null,
            "available": false,
            "url": Value::Null,
            "checkedAt": Value::Null,
        })),
    )
        .into_response()
}

/// 未实现的 `/api/*` 端点：**明确**回 501 + JSON，而不是落进 SPA 静态回退。
///
/// 为什么需要它：前端有几处按 REST 约定调的端点，网关从未实现过
/// （实测：`POST /api/elevated-mode`、`/api/approvals`、`/api/approvals/resolve`、
/// `/api/system/update`、`/api/v1/*`、`/api/audio/transcribe` 全部是 405/404）。
/// 没有这条路由时它们会命中 `fallback` → 返回 `<!doctype html>`（静态回退），
/// 于是前端 `JSON.parse` 直接抛错，而调用处多半只 `console.warn`——
/// **失败被静默吞掉，缺陷不可见**（真实案例见 `webui/src/composables/chat/useChatElevatedMode.ts`）。
///
/// 回 501 + 结构化 JSON 后，客户端能干净地识别「本部署不支持该能力」并降级。
/// 注意：真实业务全部走 `/ws` 上的 JSON-RPC，这条只服务「误用 REST 约定」的调用方。
///
/// R61 起这条不再是路由入口：`/api/{*path}` 交给
/// [`crate::gateway::sidecar_proxy::api_proxy`] —— 侧车在线就转发（否则附件上传、
/// 诊断包等只有 REST 一条路的功能在 Rust 托管下是坏的），侧车离线才回落到这里。
pub(crate) async fn api_unimplemented() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        axum::Json(serde_json::json!({
            "ok": false,
            "error": "not_implemented",
            "message": "该 REST 端点未实现；Marguerite 的业务方法走 /ws 上的 JSON-RPC。",
            "transport": "/ws",
        })),
    )
}

// ---------------------------------------------------------------------------
// WebSocket JSON-RPC 会话
// ---------------------------------------------------------------------------

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
) -> Response {
    // Origin 白名单校验（F-T4-01 第二半）：非浏览器客户端无 Origin 头，
    // 按 DEFAULT_WS_ORIGIN_ALLOWLIST 的取舍说明放行；带 Origin 但不在
    // 白名单（恶意网页）的直接在升级前 403 拒绝，根本不进入 connect 握手。
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    if !ws_origin_allowed(origin, &effective_origin_allowlist(&state)) {
        tracing::warn!(
            origin = origin.unwrap_or(""),
            "WS 握手被拒绝：Origin 不在白名单（默认白名单 + gateway.origin_allowlist 配置扩展）"
        );
        // 审计只记 Origin 字符串（非敏感），便于追溯恶意页面来源。
        audit_ws_rejection("ws.handshake", origin.unwrap_or("(no-origin)"), "origin_not_allowed");
        return (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({
                "ok": false,
                "error": "ORIGIN_NOT_ALLOWED",
                "message": "WebSocket 握手 Origin 不在白名单，已拒绝；\
                             合法桌面来源见 core/src/gateway/server.rs DEFAULT_WS_ORIGIN_ALLOWLIST，\
                             自定义来源可用 gateway.origin_allowlist 配置扩展",
            })),
        )
            .into_response();
    }
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

/// WS 事件循环「单轮耗时」告警阈值。
///
/// 事件循环正常一轮只是「等一个帧 + 写一帧」，毫秒级；即使被对端读窗口逼到
/// `send_text` 的 10s 写超时上界，也有 10s 这个天然天花板。因此 5s 足以判定
/// 「这一轮不正常」，又不会被单次 RPC 往返（约 1s）误伤。
const SLOW_LOOP_SPIN_WARN: Duration = Duration::from_secs(5);

async fn handle_ws(mut socket: WebSocket, state: Arc<GatewayState>) {
    // 1) 握手第一步：connect.challenge
    if let Err(e) = send_text(&mut socket, &challenge_frame().to_string()).await {
        tracing::debug!(err = %e, "连接在 challenge 阶段断开");
        return;
    }

    let mut seq: u64 = 0;
    let mut tick = tokio::time::interval(Duration::from_millis(TICK_INTERVAL_MS));
    // Burst 而非 Skip：事件循环被短暂阻塞（对端写停滞、长同步段）后，恢复时
    // 立即补齐错过的 tick，而不是把几十秒的心跳真空一次性吞掉——前端
    // `_pongWatchTimer` 以 20s 无帧判死，任何心跳真空都会触发被动重连。
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);

    // 慢方法的响应回流通道。dispatch 在独立任务里跑，响应经这里回到事件循环
    // 发送；循环本身不等待 dispatch，因此始终能服务 ping / tick / 事件推送。
    let (resp_tx, mut resp_rx) = mpsc::unbounded_channel::<String>();
    // 单连接在途 dispatch 上限：防止异常客户端用海量请求把进程撑爆。
    // 正常 UI 并发远低于此（关键读 + 若干元数据读）。
    const MAX_INFLIGHT_PER_CONN: usize = 64;
    let inflight = Arc::new(std::sync::atomic::AtomicUsize::new(0));

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
        let loop_entered = std::time::Instant::now();
        // 入站帧优先：ping/pong 与请求读取必须优先于事件/响应推送被 poll。
        // tokio::select! 默认在多个 ready 分支间随机选择，长任务期间事件与
        // 在途响应持续 ready 会稀释入站分支，导致客户端 WS 协议层 keepalive
        // ping 得不到 Pong 而被单方面判死（websockets 库 20s ping_timeout）。
        tokio::select! {
            biased;
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
                            Err(e) => {
                                tracing::warn!("ws: inbound frame parse failed: {e}");
                                continue;
                            }
                        };
                        if frame.r#type != "req" {
                            tracing::debug!("ws: non-req frame type '{}' ignored", frame.r#type);
                            continue;
                        }
                        let id = frame.id.unwrap_or_default();
                        let method = frame.method.unwrap_or_default();
                        if id.is_empty() || method.is_empty() {
                            tracing::warn!("ws: frame missing id or method (id={}, method={})", id, method);
                            continue;
                        }
                        // 慢方法（chat.send / 计划运行 / 上下文压缩 / skills.install）
                        // 绝不 inline await：那会把本连接的整个事件循环堵住数十秒，
                        // 期间 ping、tick、事件推送全部停摆。客户端心跳看门狗
                        // （ping 55s 周期 + 20s 无任何帧即判死）随即触发重连，
                        // 而重连会把在途的长 dispatch 连同计划运行一起掐死——
                        // 实测表现为步骤被置 blocked、run 被取消、用户看到「和后端断连」。
                        // 这里把 dispatch 丢到独立任务，循环继续服务其它帧；
                        // 响应按 JSON-RPC id 匹配，乱序返回无碍。
                        let current = inflight.load(std::sync::atomic::Ordering::Relaxed);
                        if current >= MAX_INFLIGHT_PER_CONN {
                            let err = RpcError::new(
                                "OVERLOADED",
                                format!("网关在途请求已达上限（{MAX_INFLIGHT_PER_CONN}），请稍后重试"),
                            );
                            let resp = err_frame(&id, &err).to_string();
                            if send_text(&mut socket, &resp).await.is_err() {
                                break;
                            }
                            continue;
                        }
                        inflight.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        spawn_slow_dispatch(
                            ctx.clone(),
                            method,
                            frame.params,
                            id,
                            resp_tx.clone(),
                            Arc::clone(&inflight),
                        );
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
            // 在途 dispatch 的响应回流（与事件、心跳同loop，互不阻塞）
            Some(resp) = resp_rx.recv() => {
                if send_text(&mut socket, &resp).await.is_err() {
                    break;
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
        let spin = loop_entered.elapsed();
        // 阈值 200ms → 5s：区分「压测读日志噪音」与「真事故」。
        //
        // 依据（实测数据）：
        // - 噪音侧：连续灌 RPC 时本循环单轮 ≈ 一次 RPC 往返（实测 478~994ms），
        //   200ms 阈值下几乎每轮都告警，会把真正的卡死信号彻底淹没。
        // - 事故侧：真实事故是 **56 秒** 量级（长任务与 WS 循环共享 runtime）；
        //   客户端心跳看门狗为 ping 55s + 20s 无帧判死，故 5s 仍比「用户可感知
        //   的断连」早一个数量级，留足了取证窗口。
        // - 5s 远大于最坏噪音（~1s）5 倍，又远小于事故量级（56s）一个数量级，
        //   是二者之间唯一的干净分界。
        // 埋点保留（含原始数值），它仍是这类事故唯一的现场证据。
        if spin > SLOW_LOOP_SPIN_WARN {
            tracing::warn!(
                spin_ms = spin.as_millis(),
                "ws: select 循环单轮耗时异常（阈值 {}ms；正常 RPC 往返约 1s 以内，事故为秒级）",
                SLOW_LOOP_SPIN_WARN.as_millis()
            );
        }
        let now = std::time::Instant::now();
        if now.duration_since(last_inbound) > std::time::Duration::from_secs(10) {
            tracing::warn!(
                since_inbound_ms = now.duration_since(last_inbound).as_millis(),
                "ws: 超过 10s 无入站帧（可能客户端已不发心跳）"
            );
        }
    }
}

/// 慢方法专用 runtime（惰性启动，进程级单例）。
///
/// 为什么必须独立：`chat.send` 会进入计划驱动的长执行（`run_autonomous`），
/// 其内部有同步磁盘 IO、rusqlite、以及可能的长同步段。实测（裸 socket 探针 +
/// 服务端循环迭代时间戳）证明：长任务在途时，与 WS 循环**共享同一个 tokio
/// runtime** 会把 WS 事件循环一起卡住 **56 秒**——tick 停发、ping 无 pong，
/// 前端心跳看门狗 20s 判死后被动重连，用户看到的就是「一跑任务就和后端断连」，
/// 而 plans.json 里留下 step blocked + run cancelled 的残迹。
///
/// 把慢方法的 dispatch 放到独立 runtime 后，两者互不影响：WS 循环的 tick /
/// ping / 事件推送始终按节奏走，慢方法爱跑多久跑多久。
#[cfg(not(test))]
static SLOW_RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();

#[cfg(not(test))]
fn slow_runtime() -> &'static tokio::runtime::Runtime {
    SLOW_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("marguerite-slow-dispatch")
            .enable_all()
            .build()
            .expect("慢方法 runtime 应可构建")
    })
}

/// 把慢方法分派到独立 runtime 执行，响应经 channel 回流原连接的事件循环。
///
/// 与 `tokio::spawn` 的区别：目标任务跑在另一个 runtime 上，它内部的同步阻塞
/// 无法拖住承载 WebSocket 的那个 runtime（详见 [`SLOW_RUNTIME`] 的说明）。
fn spawn_slow_dispatch(
    ctx: GatewayContext,
    method: String,
    params: Value,
    id: String,
    resp_tx: mpsc::UnboundedSender<String>,
    inflight: Arc<std::sync::atomic::AtomicUsize>,
) {
    let task = async move {
        // 守卫必须活在整个 dispatch 期间：任务结束（含 panic）时才递减。
        let _guard = InflightGuard(inflight);
        let resp = match dispatch(&ctx, &method, params).await {
            Ok(payload) => ok_frame(&id, payload).to_string(),
            Err(e) => err_frame(&id, &e).to_string(),
        };
        // 通道随连接存活；发送失败说明循环已退出，无需处理。
        let _ = resp_tx.send(resp);
    };
    // **单测不切独立 runtime**：测试的配置目录隔离是 `thread_local`
    // （`persistence::OVERRIDE_CONFIG_DIR`），只在当前线程可见。若把 dispatch
    // 丢到 `SLOW_RUNTIME` 的 worker 线程，隔离目录立刻丢失 —— 会话入口
    // `apply_effective_model` 会读开发者本机 `~/.marguerite/models.json`，
    // 把用例注入的本地 mock 端点覆盖成真实云端端点（e2e_fusion_methods_over_ws
    // 的真实现场：mock 从未被访问、真实请求成功、等不到 `.error` 事件）。
    // 生产仍走独立 runtime：避免长任务卡住 WS 事件循环（见 `SLOW_RUNTIME` 注释）。
    #[cfg(test)]
    tokio::spawn(task);
    #[cfg(not(test))]
    slow_runtime().spawn(task);
}

/// 发送文本帧（统一错误处理 + 写超时保护）。
///
/// 对端读窗口满时 `socket.send()` 可能长时间不返回；WS 事件循环的所有分支
/// 共享同一个 socket，任一处的无界写等待都会把整条连接（含 tick、入站读取、
/// 其它请求的响应）一起挂起——表现为「运行任务期间前端完全收不到帧、
/// 心跳超时后被动重连」。这里有界等待，超时即让调用方断开连接。
async fn send_text(socket: &mut WebSocket, text: &str) -> Result<(), axum::Error> {
    let fut = socket.send(Message::Text(text.into()));
    match tokio::time::timeout(Duration::from_secs(10), fut).await {
        Ok(res) => res,
        Err(_) => Err(axum::Error::new("ws send timeout")),
    }
}

/// 在途 dispatch 计数守卫：任务结束（含 panic）时必定递减，避免上限被缓慢泄漏占满。
struct InflightGuard(Arc<std::sync::atomic::AtomicUsize>);

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// 开箱即用放行（未配置 token）的进程级一次性告警。
///
/// `swap(true)` 保证多连接并发下也只打一条；后续静默（否则每个无 token
/// 连接都刷一条 warn，日志被同质噪声淹没反而看不见真事件）。
static OPEN_BOX_WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn warn_no_token_open_box_once() {
    use std::sync::atomic::Ordering;
    if !OPEN_BOX_WARNED.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            "网关未配置本地 token：按开箱即用策略放行本次无 token 连接。\
             为其数据目录生成 gateway-token 后，本类连接将被强制校验（UNAUTHORIZED）。"
        );
    }
}

/// WS 握手 Origin 白名单（默认项）。
///
/// 覆盖桌面端全部内置合法来源：
/// - `http(s)://(localhost|127.0.0.1|[::1])(:<port>)?`：Tauri 主窗口以
///   `http://localhost:18790/control/` 加载网关自家托管的控制台，浏览器 /
///   WebView 对该页面发起的 WS 连接，其 Origin 就是这两个主机 + 实际端口；
/// - `tauri://localhost`（macOS/Linux 自定义协议壳）、`http(s)://tauri.localhost`
///   （Windows/Android WebView 域）、`app://localhost`：其余桌面壳形态。
///
/// 恶意网页（`https://evil.example`）不在列表内 → 拒绝：WS **不受浏览器同源
/// 策略约束**，没有这层校验，任意网页 JS 都能无凭据驱动本机网关。
///
/// **「无 Origin 头放行」的取舍说明**：Origin 头由浏览器强制发送、页面 JS
/// 无法伪造；原生客户端（CLI / Python 探针 / pyengine 侧车 / 集成测试）根本
/// 不发 Origin，且对它们而言 Origin 没有信息量（可任意伪造）。本白名单的
/// 威胁模型是「浏览器里的第三方页面」——无 Origin 意味着不是浏览器页面，
/// 故放行并由 token 校验兜底。非浏览器客户端的准入完全靠 F-T4-01 的
/// 强制 token（见 [`await_connect`]）。
const DEFAULT_WS_ORIGIN_ALLOWLIST: &[&str] = &[
    "http://localhost",
    "https://localhost",
    "http://127.0.0.1",
    "https://127.0.0.1",
    "http://[::1]",
    "https://[::1]",
    "tauri://localhost",
    "http://tauri.localhost",
    "https://tauri.localhost",
    "app://localhost",
];

/// Origin 白名单扩展项环境变量（逗号分隔；与 `gateway.origin_allowlist`
/// 配置同义，供不便改配置文件的部署场景快速追加合法来源）。
const ORIGIN_ALLOWLIST_ENV: &str = "MARGUERITE_GATEWAY_ORIGIN_ALLOWLIST";

/// `scheme://host[:port]` 的解析结果（Origin 头 / 白名单条目通用）。
#[derive(Debug, PartialEq, Eq)]
struct OriginParts {
    scheme: String,
    host: String,
    port: Option<u16>,
}

/// 解析 `scheme://host[:port]`；非法形态（无 scheme、空 host、端口非数字、
/// 带 userinfo、IPv6 括号不闭合）返回 `None`（调用方应按拒绝处理）。
fn parse_origin(value: &str) -> Option<OriginParts> {
    let (scheme, rest) = value.trim().split_once("://")?;
    if scheme.is_empty() {
        return None;
    }
    // 只取 authority 段：浏览器 Origin 永不含 path/query/fragment。
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("").trim();
    if authority.is_empty() || authority.contains('@') {
        // userinfo（`http://user@host`）不是合法 WS Origin 形态，直接拒绝。
        return None;
    }
    let (host, port) = if let Some(rest_ipv6) = authority.strip_prefix('[') {
        // IPv6 字面量：`[::1]` 或 `[::1]:8080`。
        let end = rest_ipv6.find(']')?;
        let host = format!("[{}]", &rest_ipv6[..end]);
        let port = rest_ipv6[end + 1..]
            .strip_prefix(':')
            .and_then(|p| p.parse().ok());
        (host, port)
    } else if let Some((host, port)) = authority.rsplit_once(':') {
        if host.is_empty() {
            return None;
        }
        (host.to_string(), Some(port.parse().ok()?))
    } else {
        (authority.to_string(), None)
    };
    Some(OriginParts {
        scheme: scheme.to_ascii_lowercase(),
        host: host.to_ascii_lowercase(),
        port,
    })
}

/// 单个白名单条目是否匹配：scheme / host 大小写不敏感；**条目带端口则须
/// 精确相等，条目不带端口则接受任意端口**（浏览器对默认端口会省略，桌面壳
/// 与测试实例又常用随机端口，两种形态都要覆盖）。
fn origin_entry_matches(entry: &str, want: &OriginParts) -> bool {
    let Some(parsed) = parse_origin(entry) else {
        return false;
    };
    parsed.scheme == want.scheme
        && parsed.host == want.host
        && parsed.port.is_none_or(|p| Some(p) == want.port)
}

/// WS 握手 Origin 是否放行。
///
/// - `origin` 为 `None` / 空串（**没有 Origin 头**）→ 放行，理由见
///   [`DEFAULT_WS_ORIGIN_ALLOWLIST`] 的取舍说明；
/// - 否则必须命中默认白名单或调用方给的扩展白名单（配置 + 环境变量合并）。
fn ws_origin_allowed(origin: Option<&str>, allowlist: &[String]) -> bool {
    let Some(origin) = origin.map(str::trim).filter(|o| !o.is_empty()) else {
        return true;
    };
    let Some(parts) = parse_origin(origin) else {
        return false;
    };
    DEFAULT_WS_ORIGIN_ALLOWLIST
        .iter()
        .copied()
        .chain(allowlist.iter().map(String::as_str))
        .any(|entry| origin_entry_matches(entry, &parts))
}

/// 本次连接实际生效的 Origin 扩展白名单（配置项 + 环境变量追加项合并）。
fn effective_origin_allowlist(state: &GatewayState) -> Vec<String> {
    let mut list = state.config.gateway.origin_allowlist.clone();
    if let Ok(extra) = std::env::var(ORIGIN_ALLOWLIST_ENV) {
        list.extend(
            extra
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        );
    }
    list
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
                // F-T4-01（P0）修复：token **已配置**（`state.token` 非空）时强制校验——
                // 缺失 token 与错误 token 走同一条 UNAUTHORIZED 拒绝路径。
                //
                // 修复前这里是 `params.token.map(verify).unwrap_or(true)`：
                // `None`（没带 token）被无条件放行，于是本机任意进程/网页可以
                // **零凭据**驱动网关全部 RPC（QA 三态对照实测：token 已配置，
                // 无 token 连接仍 `ok=True`，chat.send/usage.query 等全部可打）。
                //
                // token **未配置**（空串：仅测试构造或 token 落盘失败的降级
                // 路径）时保持开箱即用放行——否则老部署升级后会立刻失联。
                // 该路径只应出现在非常态场景，故只告警一次，不逐连接刷日志。
                let token_ok = match params.token.as_deref() {
                    Some(t) => auth::verify(t, &state.token),
                    None if state.token.trim().is_empty() => {
                        warn_no_token_open_box_once();
                        true
                    }
                    None => false,
                };
                if !token_ok {
                    let err = RpcError::unauthorized();
                    let frame = err_frame(&id, &err).to_string();
                    let _ = send_text(socket, &frame).await;
                    tracing::warn!("网关连接被拒绝：token 校验失败");
                    audit_ws_rejection("ws.connect", "loopback", "unauthorized");
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
async fn control_root(State(state): State<Arc<GatewayState>>, headers: HeaderMap) -> Response {
    serve_rel(
        &state,
        "",
        request_accept_language(&headers),
        false,
        request_range(&headers),
    )
    .await
}

/// `/control/{*path}` 通配处理器（SPA 页面与产物；未知路径走 index 回退）
async fn control_spa_assets(
    AxumPath(path): AxumPath<String>,
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
) -> Response {
    serve_rel(
        &state,
        &path,
        request_accept_language(&headers),
        false,
        request_range(&headers),
    )
    .await
}

/// `/control/static/{*path}` 通配处理器（后端资产面）。
///
/// **必须单独注册 handler**：axum 的 `{*path}` 不包含路由前缀，若与 SPA 路由
/// 共用一个 handler，就无法区分「资产」与「SPA 深链」，资产缺失会被 index
/// 回退成 200 + text/html（前端把 HTML 当 GLB 解析 → 恒报「资产校验失败」）。
/// 资产面缺失/越界一律 404，绝不回 SPA。
async fn control_static_assets(
    AxumPath(path): AxumPath<String>,
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
) -> Response {
    serve_rel(
        &state,
        &path,
        request_accept_language(&headers),
        true,
        request_range(&headers),
    )
    .await
}

/// 取请求的 `Accept-Language`（首屏语言解析的输入；非 UTF-8 头按「没给」处理）
fn request_accept_language(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
}

/// 取请求的 `Range` 头（仅媒体资源会用到）。
///
/// 非 UTF-8 / 非法头按「没给」处理：让请求退回完整 200 响应，
/// 而不是因为一个畸形头就 416 —— 播放器能播总比报错好。
fn request_range(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
}

/// 宽松百分号解码（只处理合法 `%XX`；非法序列原样保留）。
///
/// 为什么不直接用 String::from_utf8_lossy(percent_decode)：
/// 攻击串可能是非 UTF-8 字节序列，**必须在字节层判断** `..`；先解码成
/// lossy 字符串会把无效字节替换成 U+FFFD，从而丢失真实分隔符。
fn percent_decode_bytes(s: &str) -> Vec<u8> {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// 字节级路径穿越判定：按 `/` 与 `\` 分段，任一段为 `..` 即拒绝。
fn decoded_has_traversal(path: &str) -> bool {
    let decoded = percent_decode_bytes(path);
    decoded
        .split(|b| *b == b'/' || *b == b'\\')
        .any(|seg| seg == b"..")
}

/// `/control*` 的路径穿越入口闸（F-T4-02）。
///
/// 覆盖：明文 `..`、百分号编码（`%2e%2e%2f`）、反斜杠（Windows 分隔符）。
/// 仅作用于控制台路径；`/api/*` 与 WS 不受影响。
async fn control_traversal_guard_layer(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let path = req.uri().path();
    if path.starts_with("/control") && decoded_has_traversal(path) {
        return StatusCode::NOT_FOUND.into_response();
    }
    next.run(req).await
}

/// 认证/准入失败落审计（F-T4-03：入侵必须可追溯）。
///
/// 只记录**判定结果**与来源，不记录 token/密钥；审计库独立 append-only
/// （`Config::data_dir()/audit.db`，SHA-256 链式校验）。写入失败仅告警，
/// 绝不阻塞拒绝路径（安全优先：日志失败不能变成放行）。
///
/// 测试构建下为空实现：网关单测会故意制造大量拒绝，若写真实用户审计库会
/// 污染 `~/.marguerite/audit.db`（破坏测试隔离）；append-only 语义本身由
/// `audit_store` 的专项测试覆盖。
#[cfg(not(test))]
fn audit_ws_rejection(method: &str, target: &str, reason: &str) {
    let ts = chrono::Utc::now().to_rfc3339();
    match crate::audit_store::AuditStore::open_default() {
        Ok(store) => {
            if let Err(e) = store.append(crate::audit_store::AuditRecord::new(
                ts,
                "gateway",
                method,
                target,
                reason,
            )) {
                tracing::warn!(err = %e, "安全审计落盘失败（拒绝仍生效）");
            }
        }
        Err(e) => tracing::warn!(err = %e, "安全审计库打开失败（拒绝仍生效）"),
    }
}

#[cfg(test)]
fn audit_ws_rejection(_method: &str, _target: &str, _reason: &str) {}

/// 把 `rel` 解析成 `webui_dir` 内**真实存在**的文件路径（F-T4-02 修复）。
///
/// 为什么不能只做字符串检测：`Path::join` 在 Windows 上遇到绝对路径
/// （如 `C:/Windows/win.ini`）或 rooted path 会**替换**基路径；符号链接也能
/// 指到根目录之外。字符串层面能挡 `..`/`\`，挡不住这两类。这里统一用
/// canonicalize 后的前缀校验，返回 `None` 表示「越界或不存在」。
async fn resolve_under_root(state: &GatewayState, rel: &str) -> Option<std::path::PathBuf> {
    if has_traversal(rel) {
        return None;
    }
    let root = tokio::fs::canonicalize(&state.webui_dir).await.ok()?;
    let candidate = if rel.is_empty() {
        root.join("index.html")
    } else {
        root.join(rel)
    };
    let resolved = tokio::fs::canonicalize(&candidate).await.ok()?;
    resolved.starts_with(&root).then_some(resolved)
}

/// 相对 `webui_dir` 的静态文件服务 + SPA history fallback + index 注入。
///
/// `static_only=true`（`/control/static/*` 资产面）时，缺失/越界一律 404，
/// 不做 SPA 回退——避免「404 资产返回 index.html + 200」污染前端解析。
async fn serve_rel(
    state: &GatewayState,
    rel: &str,
    accept_language: Option<&str>,
    static_only: bool,
    range: Option<&str>,
) -> Response {
    if let Some(file) = resolve_under_root(state, rel).await {
        let Ok(bytes) = tokio::fs::read(&file).await else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let mime = content_type(&file);
        // Range 只对媒体生效：视频播放器会先用 `bytes=0-` 探测，
        // 不支持 206 时部分 WebView 会拒绝进入播放（只显示首帧或直接 error）。
        // 对 html/js/css 走 Range 没有意义，也会让 index 注入逻辑复杂化。
        let is_media = mime.starts_with("video/") || mime.starts_with("audio/");
        if is_media {
            if let Some(raw) = range {
                match parse_byte_range(raw, bytes.len()) {
                    Some((start, end)) => {
                        let slice = bytes[start..=end].to_vec();
                        let content_range = format!("bytes {start}-{end}/{}", bytes.len());
                        let Ok(cr) = HeaderValue::from_str(&content_range) else {
                            return StatusCode::RANGE_NOT_SATISFIABLE.into_response();
                        };
                        let mut resp = (
                            StatusCode::PARTIAL_CONTENT,
                            [
                                (header::CONTENT_TYPE, HeaderValue::from_static(mime)),
                                (header::ACCEPT_RANGES, HeaderValue::from_static("bytes")),
                                (header::CONTENT_RANGE, cr),
                            ],
                            slice,
                        )
                            .into_response();
                        set_cache_policy(&mut resp, &file);
                        return resp;
                    }
                    None => {
                        // 无法满足：按 RFC 7233 回 416 + `Content-Range: bytes */len`
                        let cr = format!("bytes */{}", bytes.len());
                        let Ok(cr) = HeaderValue::from_str(&cr) else {
                            return StatusCode::RANGE_NOT_SATISFIABLE.into_response();
                        };
                        return (
                            StatusCode::RANGE_NOT_SATISFIABLE,
                            [(header::CONTENT_RANGE, cr)],
                        )
                            .into_response();
                    }
                }
            }
        }
        let mut resp = (
            [
                (header::CONTENT_TYPE, HeaderValue::from_static(mime)),
                // 声明可分段：播放器据此决定是否走渐进式播放。
                (header::ACCEPT_RANGES, HeaderValue::from_static("bytes")),
            ],
            bytes,
        )
            .into_response();
        set_cache_policy(&mut resp, &file);
        maybe_inject_index(&mut resp, state, accept_language);
        return resp;
    }

    if static_only {
        return StatusCode::NOT_FOUND.into_response();
    }

    // SPA history fallback：未知路由回退到 index.html（注入 data）。
    // index.html 同样走 canonicalize + 前缀校验（防 dist 内符号链接外逃）。
    match resolve_under_root(state, "index.html").await {
        Some(index) => match tokio::fs::read(&index).await {
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
                maybe_inject_index(&mut resp, state, accept_language);
                resp
            }
            // WebUI 产物整体缺失（index.html 都读不到）：**不再回空白 404**，
            // 而是回自包含诊断页 + 503，让桌面窗口显示可执行的修复指引。
            Err(_) => webui_missing_page(state),
        },
        None => webui_missing_page(state),
    }
}

/// WebUI 产物缺失兜底诊断页的响应头值 / 页内标记。
///
/// 桌面端（`gui/src-tauri/src/lib.rs`）的就绪探测据此区分
/// 「真实控制台入口」与「兜底诊断页」——两者都是 HTML，只有标记能说明真伪。
pub const WEBUI_MISSING_HEADER: &str = "x-marguerite-webui";
/// 兜底页标记值（HTTP 头 `x-marguerite-webui: missing`）
pub const WEBUI_MISSING_VALUE: &str = "missing";
/// 兜底页内联标记（HTML 属性），供只读 body 的探测方识别
pub const WEBUI_MISSING_MARKER: &str = "data-marguerite-webui=\"missing\"";

/// WebUI 产物缺失时的自包含诊断页（HTTP 503 + 内联 HTML，无外部资源）。
///
/// 场景（P0-D 缺陷 1 实测）：把裸 `marguerite-gui.exe` 复制到仓库外空目录双击，
/// 网关起来了、`/api/health` 200，但 `/control/` 曾返回 `404` 且 `content-length: 0`
/// → 窗口全白且 stderr 谎报「就绪」。现在改为返回可渲染的诊断页：
/// - 状态码 503（诚实：控制台**不可用**，不是 200 OK）；
/// - 页内列出探测到的 `webui_dir` 与修复动作（放产物 / npm run build / 环境变量）。
pub fn webui_missing_page(state: &GatewayState) -> Response {
    let dir = state.webui_dir.display();
    let body = format!(
        r#"<!doctype html>
<html lang="zh-Hans">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Marguerite 控制台不可用：未找到 webui/dist</title>
</head>
<body {WEBUI_MISSING_MARKER} style="margin:0;padding:2.5rem;font:14px/1.7 -apple-system,'Segoe UI',system-ui,sans-serif;background:#0f1115;color:#e6e8eb">
<h1 style="font-size:1.25rem;margin:0 0 .75rem">控制台产物缺失（webui/dist）</h1>
<p style="margin:0 0 1rem;color:#f0b429">内嵌网关已在 127.0.0.1 监听，但找不到控制台静态产物，因此本页代替界面显示。</p>
<p style="margin:0 0 .5rem">已探测的产物目录：</p>
<pre style="margin:0 0 1rem;padding:.75rem;background:#171a21;border-radius:6px;overflow:auto">{dir}</pre>
<p style="margin:0 0 .5rem">修复（任选其一）：</p>
<ol style="margin:0 0 1rem;padding-left:1.25rem">
<li>把 <code>webui/dist</code> 目录放到 <code>marguerite-gui.exe</code> 同级目录（或仓库根目录下），重启本程序；</li>
<li>在源码目录执行 <code>npm run build</code>（<code>webui/</code>）生成 <code>webui/dist</code>；</li>
<li>用环境变量 <code>MARGUERITE_WEBUI_DIST=&lt;绝对路径&gt;</code> 显式指定产物目录。</li>
</ol>
<p style="margin:0;color:#8b949e">诊断：<code>curl http://127.0.0.1:{DEFAULT_PORT}/api/health</code> 应为 200；本页为 503 说明网关正常、产物缺失。</p>
</body>
</html>
"#,
        DEFAULT_PORT = crate::gateway::DEFAULT_PORT
    );
    let length = body.len().to_string();
    let mut resp = Response::new(axum::body::Body::from(body));
    *resp.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
    let headers = resp.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(
        HeaderName::from_static(WEBUI_MISSING_HEADER),
        HeaderValue::from_static(WEBUI_MISSING_VALUE),
    );
    if let Ok(v) = HeaderValue::from_str(&length) {
        headers.insert(header::CONTENT_LENGTH, v);
    }
    resp
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

/// 控制台支持的界面语言（与前端 `SUPPORTED_LOCALES`、pyengine `_SUPPORTED_LOCALES` 一致）
pub const SUPPORTED_UI_LOCALES: [&str; 6] = ["en", "zh-Hans", "ja", "fr", "de", "es"];
/// 基线语言：`control_ui.default_locale` 等于它时视为「未被钉死」，交 `Accept-Language` 决定
const BASELINE_UI_LOCALE: &str = "en";

/// BCP-47 标签 → 受支持语言（`None` = 不支持）；`zh*` 一律收敛到 `zh-Hans`
fn locale_from_tag(tag: &str) -> Option<&'static str> {
    let t = tag.trim().to_ascii_lowercase();
    if t.is_empty() {
        return None;
    }
    if t.starts_with("zh") {
        return Some("zh-Hans");
    }
    SUPPORTED_UI_LOCALES
        .iter()
        .copied()
        .find(|c| *c != BASELINE_UI_LOCALE && (t == *c || t.starts_with(&format!("{c}-"))))
        .or((t == BASELINE_UI_LOCALE || t.starts_with("en-")).then_some(BASELINE_UI_LOCALE))
}

/// 首屏语言解析（与 pyengine `gateway/control_ui.py::_resolve_locale` 同契约）：
/// ① 配置被显式钉成非 `en` 的受支持语言 → 用它（运维意图不被浏览器覆盖）；
/// ② 否则取请求 `Accept-Language` 中第一个受支持标签；
/// ③ 都没有 → `en`。
///
/// 客户端 localStorage 的已保存偏好与手动切换在**前端**层面始终优先于本结果，
/// 本函数只决定「全新浏览器首访」渲染哪种语言。
pub fn resolve_ui_locale(configured: &str, accept_language: Option<&str>) -> &'static str {
    if let Some(code) = locale_from_tag(configured) {
        if code != BASELINE_UI_LOCALE {
            return code;
        }
    }
    for part in accept_language.unwrap_or("").split(',') {
        if let Some(code) = locale_from_tag(part.split(';').next().unwrap_or("")) {
            return code;
        }
    }
    BASELINE_UI_LOCALE
}

/// 把首屏 `<html lang="…">` 改写成解析出的语言（只动第一个 `<html …>` 标签）。
///
/// 与 pyengine 模板（`gateway/templates/index.html` 的 `lang="{{ locale }}"`）同契约：
/// 让 `lang` 与 `#Marguerite-data[data-locale]` 一致，读 `<html lang>` 的
/// 无障碍/预绘制守卫不会拿到一个与界面不符的声明。
fn replace_html_lang(html: &str, locale: &str) -> String {
    let Some(start) = html.find("<html") else {
        return html.to_string();
    };
    let Some(rel_end) = html[start..].find('>') else {
        return html.to_string();
    };
    let end = start + rel_end;
    let tag = &html[start..end];
    let new_tag = match tag.find("lang=\"") {
        Some(ls) => {
            let attr_start = start + ls; // `lang="` 的起点
            let after_quote = attr_start + "lang=\"".len();
            match html[after_quote..].find('"') {
                Some(qe) => format!(
                    "{prefix}lang=\"{locale}\"{suffix}",
                    prefix = &html[start..attr_start],
                    suffix = &html[after_quote + qe + 1..end],
                ),
                None => return html.to_string(),
            }
        }
        None => format!("{tag} lang=\"{locale}\""),
    };
    format!("{}{new_tag}{}", &html[..start], &html[end..])
}

/// HTML 属性值转义（`&` `<` `>` `"` `'`）——注入值与配置路径同源进 DOM 属性，
/// 一律转义（token 是 base64url 字母表，转义对它是无操作的，但保持通用）。
fn escape_html_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// 若响应体是 index.html（未被注入过），注入 `#Marguerite-data` 元数据元素
///
/// 首屏语言按 [`resolve_ui_locale`] 解析（配置钉死 > Accept-Language > en）并同时写进
/// `data-locale` 与 `<html lang>`。**不再硬编码 `zh-Hans`**：旧实现让英语浏览器首访
/// 也被强制中文，与 pyengine 网关契约不一致（也是 webui e2e 语言不确定的根因）。
fn maybe_inject_index(resp: &mut Response, state: &GatewayState, accept_language: Option<&str>) {
    if resp
        .headers()
        .get(header::CONTENT_TYPE)
        .is_none_or(|v| !v.to_str().unwrap_or("").contains("text/html"))
    {
        return;
    }
    let path = state.webui_dir.join("index.html");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return;
    };
    // ⚠️ The id MUST be `Marguerite-data`: that is what every frontend consumer
    // reads (`router/index.ts`, `i18n/index.ts`, `App.vue`, `UpdateBanner.vue`,
    // `useBgm.ts`, `useChatShareExport.ts`, `OverviewView.vue`) and what the
    // frontend's own tests assert. The injected element used to carry the
    // pre-rebrand id `opensquilla-data`, so `data-locale` / `data-config-path` /
    // `data-base-path` were silently unreadable and the fallbacks (`|| '/control'`,
    // browser-language detection) masked it. Keep both names in sync if this ever
    // has to change again.
    if raw.contains("Marguerite-data") || !raw.contains(r#"<div id="app">"#) {
        return;
    }
    let config_path = Config::config_path()
        .to_string_lossy()
        .replace('&', "&amp;");
    let locale = resolve_ui_locale(&state.config.control_ui.default_locale, accept_language);
    // F-T4-01 兼容性关键：token 强制校验后，浏览器端（Tauri 主窗口加载的
    // `http://localhost:18790/control/`）也必须带 token 握手。把 token 随
    // 同源页面下发给前端（`data-ws-token`，前端 rpc store 读取后用于 connect
    // 握手）——它只对本机回环上的同源页面可见：远端网页既读不到（跨源），
    // 其 WS 连接也会被 Origin 白名单提前拒绝。`?token=` 深链/sessionStorage
    // 仍优先（`webui/src/stores/rpc.ts`）。
    let ws_token = escape_html_attr(&state.token);
    let data = format!(
        r#"<div id="Marguerite-data" data-base-path="{CONTROL_BASE_PATH}" data-config-path="{config_path}" data-locale="{locale}" data-ws-token="{ws_token}"></div>"#
    );
    let injected = raw.replace(
        r#"<div id="app">"#,
        &format!("{data}\n    <div id=\"app\">"),
    );
    // 深链修复：dist 由 Vite 以 `base: './'` 构建（资源路径全是相对的），浏览器按**文档
    // 路径**解析 `./assets/…`。于是 `/control/settings/connection` 这类 ≥2 段的路径会去请求
    // `/control/settings/assets/…` → 命中 SPA 回退、拿到 text/html → 模块脚本被 MIME 校验
    // 拒绝（`Expected a JavaScript-or-Wasm module script … "text/html"`）→ **整页白屏**。
    // 注入 `<base href="/control/">` 后，相对资源在任意深度都解析到 /control/assets/…（实测白屏消失）。
    let injected = inject_base_href(&injected, CONTROL_BASE_PATH);
    let injected = replace_html_lang(&injected, locale);
    let length = injected.len().to_string();
    *resp.body_mut() = axum::body::Body::from(injected);
    let _ = resp.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&length).unwrap_or(HeaderValue::from_static("0")),
    );
}

/// 控制台基路径（SPA 路由前缀 + 静态资源前缀的**单一事实源**）。
///
/// 三处必须一致：①`data-base-path`（前端 router 基路径）②`<base href>`（相对资源解析）
/// ③`build_router` 里 `/control/{*path}` 的路由前缀。
pub const CONTROL_BASE_PATH: &str = "/control";

// ---------------------------------------------------------------------------
// Control UI 安全响应头（契约与 pyengine `SecurityHeadersMiddleware` **逐字一致**）
// ---------------------------------------------------------------------------

/// Control UI 的 CSP。
///
/// **必须与** `pyengine/src/opensquilla/gateway/middleware.py` 中
/// `response.headers["content-security-policy"]` 拼接出的字符串逐字相同
/// （同一份控制台可能由任一引擎托管，两边下发不同策略 = 同一个 UI 两套安全边界）。
/// 由源码守卫 `test_csp_matches_pyengine_middleware` 强制比对，改一边必须改另一边。
///
/// `http://*.localhost:*` 两个入口是刻意放开的：产物预览（artifact preview）用
/// `http://p-<token>.localhost:<port>` 这种**独立源**隔离渲染用户 HTML，并通过
/// `/.opensquilla/clear-site-data` 清理其缓存/Cookie —— 这正是
/// `webui/e2e/preview-origin-csp.spec.ts` 钉住的契约。
const CONTROL_UI_CSP: &str = "default-src 'self'; \
script-src 'self' 'unsafe-inline'; \
style-src 'self' 'unsafe-inline'; \
img-src 'self' data: blob:; \
connect-src 'self' ws: wss: http://*.localhost:*; \
media-src 'self' blob: https:; \
font-src 'self' data:; \
frame-src 'self' blob: http://*.localhost:*; \
object-src 'none'; \
frame-ancestors 'self';";

/// `/control` 文档与静态资源（GET/HEAD）才下发安全头。
///
/// 语义与 pyengine `_is_control_ui_request` 对齐：只覆盖只读的控制台文档/资源；
/// 控制面（`/api`、`/ws`、`/health*`）绝不能被套上 UI 行为。pyengine 侧还有一个
/// `control_ui.enabled` 开关，Rust 侧没有该字段（网关始终托管控制台），故按
/// `enabled = true` 处理。
fn is_control_ui_request(path: &str, method: &axum::http::Method) -> bool {
    if method != axum::http::Method::GET && method != axum::http::Method::HEAD {
        return false;
    }
    if is_control_plane_path(path) {
        return false;
    }
    is_at_or_below(path, CONTROL_BASE_PATH)
}

/// 控制面路径（`pyengine::_CONTROL_PLANE_PATHS` + `/api`/`/ws` 子树）
fn is_control_plane_path(path: &str) -> bool {
    matches!(path, "/health" | "/healthz" | "/ready" | "/readyz")
        || is_at_or_below(path, "/api")
        || is_at_or_below(path, "/ws")
}

fn is_at_or_below(path: &str, prefix: &str) -> bool {
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

/// 注入 4 个安全响应头（CSP / X-Frame-Options / nosniff / Referrer-Policy）
fn apply_control_ui_security_headers(resp: &mut Response) {
    let headers = resp.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTROL_UI_CSP),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
}

/// axum 中间件：按**请求**路径/方法判定（而不是看响应 URL），与 pyengine 的
/// `BaseHTTPMiddleware` 判定口径一致。
async fn control_ui_security_headers_layer(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let mut resp = next.run(req).await;
    if is_control_ui_request(&path, &method) {
        apply_control_ui_security_headers(&mut resp);
    }
    resp
}

/// 限流作用面（纯函数，便于单测）：只限 `/api/*` REST。
///
/// - `/api/health` 永远豁免：限流不得影响存活探测（生存性优先，见 R61 的看门狗教训）；
/// - WS 升级（`/ws`）与 `/control/*` 静态资源根本不走该中间件的路径集合。
fn rate_limit_applies(path: &str) -> bool {
    path.starts_with("/api") && path != "/api/health"
}

/// W3.4：生产限流。超限回 HTTP 429 + `rate_limit::RateLimitError` 的 JSON-RPC 错误体。
async fn api_rate_limit_layer(
    State(state): State<Arc<GatewayState>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if state.rate_limit_enabled && rate_limit_applies(req.uri().path()) {
        if let Err(err) = state.rate_limiter.check("loopback") {
            return (StatusCode::TOO_MANY_REQUESTS, axum::Json(err.json_rpc_error()))
                .into_response();
        }
    }
    next.run(req).await
}

/// 注入 `<base href="{base_path}/">`，让 `base: './'` 构建出的**相对资源路径**在任意
/// 深度的文档路径下都解析到同一处（见 `maybe_inject_index` 的深链白屏说明）。
///
/// 规则（幂等 + 保守）：
/// - 已存在 `<base …>`（构建方显式声明）→ 原样返回，绝不覆盖别人的显式决策；
/// - 有 `<head>` → 紧随其后插入（必须早于 `<link>`/`<script>` 等携带 URL 的元素）；
/// - 无 `<head>` → 原样返回（不是可注入的 HTML 文档，宁可不改）。
fn inject_base_href(html: &str, base_path: &str) -> String {
    if html.contains("<base") {
        return html.to_string();
    }
    let lower = html.to_ascii_lowercase();
    let Some(head_at) = lower.find("<head") else {
        return html.to_string();
    };
    let Some(gt) = lower[head_at..].find('>') else {
        return html.to_string();
    };
    let insert_at = head_at + gt + 1;
    let tag = format!("\n    <base href=\"{base_path}/\" />");
    let mut out = String::with_capacity(html.len() + tag.len());
    out.push_str(&html[..insert_at]);
    out.push_str(&tag);
    out.push_str(&html[insert_at..]);
    out
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
        // 视频：启动动画（splash）与桌面端媒体预览都靠这几条。
        //
        // 【2026-10-22 修】此前只有音频、没有视频类型，`.mp4` / `.webm` 一律落到
        // `application/octet-stream`；再叠加本文件统一注入的
        // `X-Content-Type-Options: nosniff`，Chromium **拒绝**把该响应当作视频解码
        // （nosniff 的语义就是"不许猜"）。实测：`/control/splash/bg-v2-h264.mp4`
        // 返回 200 + application/octet-stream + nosniff ⇒ `<video>` 立刻 error
        // ⇒ 启动动画只剩极光与字幕，用户"没看见视频"。
        //
        // 注意这是同一条链上的**第三层**问题：编码（HEVC→H.264）→ MIME → Range。
        // 只修前两层仍然不播，所以三者必须一起成立。
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        "mov" => "video/quicktime",
        "m4a" => "audio/mp4",
        _ => "application/octet-stream",
    }
}

/// 解析 `Range: bytes=start-end`（单区间）。
///
/// 只支持单区间：浏览器对 `<video>` 的探测就是单区间请求；多区间（`bytes=0-1,5-6`）
/// 需要 multipart/byteranges 响应，收益为零、复杂度不低，故不支持（返回 None → 416）。
///
/// 支持三种形态：
/// - `bytes=100-200` 闭区间
/// - `bytes=100-`    到结尾
/// - `bytes=-100`    末尾 100 字节（suffix）
fn parse_byte_range(raw: &str, len: usize) -> Option<(usize, usize)> {
    let spec = raw.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (s, e) = spec.split_once('-')?;
    let s = s.trim();
    let e = e.trim();
    if len == 0 {
        return None;
    }
    let (start, end) = if s.is_empty() {
        // suffix：最后 N 字节
        let n: usize = e.parse().ok()?;
        if n == 0 {
            return None;
        }
        (len.saturating_sub(n), len - 1)
    } else {
        let start: usize = s.parse().ok()?;
        let end = if e.is_empty() {
            len - 1
        } else {
            e.parse::<usize>().ok()?.min(len - 1)
        };
        (start, end)
    };
    if start > end || start >= len {
        return None;
    }
    Some((start, end))
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

    /// F-T4-02 回归：入口闸必须在**解码后**判断，覆盖编码与反斜杠变体。
    #[test]
    fn test_decoded_traversal_guard_variants() {
        assert!(decoded_has_traversal("/control/static/../secret"));
        assert!(decoded_has_traversal("/control/static/%2e%2e%2fsecret"));
        assert!(decoded_has_traversal("/control/static/%2E%2E%5Csecret"));
        assert!(decoded_has_traversal(r"/control/static\..\secret"));
        assert!(decoded_has_traversal("/control/static/%2e%2e/secret"));
        assert!(!decoded_has_traversal("/control/assets/app.js"));
        assert!(!decoded_has_traversal("/control/static/island3d/code_crystal.glb"));
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

    /// 视频必须映射到 `video/*`，否则叠加 `X-Content-Type-Options: nosniff` 后
    /// Chromium 拒绝解码 —— 实测导致启动动画的视频一帧都不播（2026-10-22）。
    #[test]
    fn video_assets_map_to_video_mime_types() {
        assert_eq!(content_type(Path::new("bg-v2-h264.mp4")), "video/mp4");
        assert_eq!(content_type(Path::new("bg-v2.webm")), "video/webm");
        assert_eq!(content_type(Path::new("clip.mov")), "video/quicktime");
        // 大小写不敏感（Windows 产物可能带大写扩展名）
        assert_eq!(content_type(Path::new("A.MP4")), "video/mp4");
    }

    #[test]
    fn parse_byte_range_handles_the_three_forms_players_use() {
        // 闭区间
        assert_eq!(parse_byte_range("bytes=0-99", 1000), Some((0, 99)));
        // 开区间到结尾
        assert_eq!(parse_byte_range("bytes=900-", 1000), Some((900, 999)));
        // suffix：最后 100 字节
        assert_eq!(parse_byte_range("bytes=-100", 1000), Some((900, 999)));
        // 末端越界被夹到 len-1（RFC 允许）
        assert_eq!(parse_byte_range("bytes=990-5000", 1000), Some((990, 999)));
    }

    #[test]
    fn parse_byte_range_rejects_unsatisfiable_and_multi_range() {
        // 起点越界
        assert_eq!(parse_byte_range("bytes=1000-", 1000), None);
        // 起点大于终点
        assert_eq!(parse_byte_range("bytes=500-100", 1000), None);
        // 多区间：不支持（需 multipart/byteranges），退回整文件
        assert_eq!(parse_byte_range("bytes=0-1,5-6", 1000), None);
        // 非法/空文件
        assert_eq!(parse_byte_range("items=0-1", 1000), None);
        assert_eq!(parse_byte_range("bytes=0-1", 0), None);
        assert_eq!(parse_byte_range("bytes=-0", 1000), None);
    }

    #[test]
    fn test_index_injection_marker_absent_then_present() {
        // 注入逻辑判定依据（不被重复注入）：第二次注入因含 marker 直接跳过。
        // marker 必须是前端消费方实际读取的那个 id。
        let raw = r#"<html><head></head><body><div id="app"></div></body></html>"#;
        assert!(raw.contains(r#"<div id="app">"#));
        assert!(!raw.contains("Marguerite-data"));
    }

    /// 未实现的 `/api/*` 必须回 **501 + JSON**，而不是落进 SPA 静态回退返回 HTML。
    ///
    /// 背景：前端有几处按 REST 约定调用，而网关从未实现这些端点
    /// （`/api/elevated-mode`、`/api/approvals`、`/api/approvals/resolve`、
    /// `/api/system/update`、`/api/v1/*` …）。没有这条路由时它们命中 `fallback`，
    /// 拿到 `<!doctype html>`，前端 `JSON.parse` 抛错后被 `console.warn` 吞掉 ——
    /// 失败完全不可见（`webui/.../useChatElevatedMode.ts` 就是这么烂了很久）。
    ///
    /// 同时锁住路由优先级：`/api/health` 必须仍是 200 JSON，不能被通配路由吃掉。
    #[tokio::test]
    async fn test_unimplemented_api_returns_501_json_and_health_still_200() {
        let state = Arc::new(GatewayState {
            config: Config::default(),
            bus: EventBus::default(),
            token: "api-probe-token".to_string(),
            started_at: Instant::now(),
            webui_dir: test_webui_dir(),
            rate_limiter: std::sync::Arc::new(crate::gateway::rate_limit::RateLimiter::new(
                crate::gateway::rate_limit::BucketConfig::new(240, 40.0),
                crate::gateway::rate_limit::BucketConfig::new(240, 40.0),
            )),
            rate_limit_enabled: false,
        });
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let router = build_router(Arc::clone(&state));
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        // 未带 token 的代理调用必须 401（否则本机进程可借侧车通道零凭据操作）
        let unauth = http_request(addr, "POST", "/api/elevated-mode", Some("{}")).await;
        assert!(unauth.starts_with("HTTP/1.1 401"), "代理无 token 应 401: {unauth}");

        // POST 到未实现端点 -> 501 且响应体是 JSON（不是 HTML）
        let raw = http_request_auth(addr, "POST", "/api/elevated-mode", Some("{}"), "api-probe-token").await;
        assert!(raw.starts_with("HTTP/1.1 501"), "应为 501，实际: {raw}");
        assert!(
            raw.contains("\"error\":\"not_implemented\""),
            "响应体应为结构化 JSON，实际: {raw}"
        );
        assert!(
            !raw.contains("<!doctype html>"),
            "绝不能回落到 SPA 的 HTML fallback: {raw}"
        );
        // Content-Type 必须是 JSON，客户端才会用 JSON.parse 而不是当页面处理
        assert!(
            raw.to_ascii_lowercase().contains("content-type: application/json"),
            "Content-Type 应为 application/json: {raw}"
        );

        // /api/health 仍然 200 JSON（通配路由不得遮蔽更具体的注册）
        let health = http_request(addr, "GET", "/api/health", None).await;
        assert!(health.starts_with("HTTP/1.1 200"), "health 应 200: {health}");
        assert!(health.contains("\"status\":\"ok\""), "health 体: {health}");
    }

    // ------------------------------------------------------------------
    // Control UI 安全响应头（契约与 pyengine 逐字一致）
    // ------------------------------------------------------------------

    /// 判定口径必须与 pyengine `_is_control_ui_request` 一致。
    ///
    /// 关键是否定面：控制面（`/api`、`/ws`、`/health*`）与写方法**绝不能**被套上
    /// UI 行为 —— 否则 API 响应会带上只对文档有意义的安全头。
    #[test]
    fn test_is_control_ui_request_matches_pyengine_semantics() {
        let get = axum::http::Method::GET;
        let head = axum::http::Method::HEAD;
        let post = axum::http::Method::POST;

        // 控制台文档与资源（含 SPA 深链）→ 是
        for path in [
            "/control",
            "/control/",
            "/control/settings",
            "/control/a/b/c",
            "/control/static/assets/index-abc.js",
        ] {
            assert!(is_control_ui_request(path, &get), "GET {path} 应下发安全头");
        }
        assert!(is_control_ui_request("/control/", &head), "HEAD 与 GET 同权");

        // 控制面 → 否
        for path in [
            "/api/health",
            "/api",
            "/ws",
            "/ws/chat",
            "/health",
            "/healthz",
            "/ready",
            "/readyz",
        ] {
            assert!(!is_control_ui_request(path, &get), "GET {path} 不得下发安全头");
        }

        // 非控制台路径 / 写方法 → 否
        assert!(!is_control_ui_request("/", &get), "根路径不是控制台文档");
        assert!(!is_control_ui_request("/favicon.ico", &get));
        assert!(!is_control_ui_request("/controling", &get), "前缀必须按段匹配");
        assert!(!is_control_ui_request("/control/", &post), "写方法不下发");
    }

    /// 真实 HTTP 层：`/control` 文档带 4 个安全头，控制面与写方法不带。
    #[tokio::test]
    async fn test_control_ui_security_headers_are_served() {
        let state = e2e_state();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, build_router(state)).await.unwrap();
        });

        for path in ["/control/", "/control/settings/deep"] {
            let raw = http_request(addr, "GET", path, None).await;
            assert!(raw.starts_with("HTTP/1.1 200"), "{path} 应 200: {raw}");
            let lower = raw.to_ascii_lowercase();
            assert!(
                lower.contains(&format!("content-security-policy: {}", CONTROL_UI_CSP.to_ascii_lowercase())),
                "{path} 缺 CSP 或值不符: {raw}"
            );
            assert!(lower.contains("x-frame-options: deny"), "{path} 缺 X-Frame-Options: {raw}");
            assert!(
                lower.contains("x-content-type-options: nosniff"),
                "{path} 缺 nosniff: {raw}"
            );
            assert!(
                lower.contains("referrer-policy: strict-origin-when-cross-origin"),
                "{path} 缺 Referrer-Policy: {raw}"
            );
        }

        // 控制面：不得带上 UI 的安全头
        let health = http_request(addr, "GET", "/api/health", None).await;
        assert!(health.starts_with("HTTP/1.1 200"), "health 应 200: {health}");
        assert!(
            !health.to_ascii_lowercase().contains("content-security-policy"),
            "控制面响应不该带 CSP: {health}"
        );
    }

    /// 跨实现契约守卫：Rust 与 pyengine 的 4 个安全头必须**逐字一致**。
    ///
    /// 同一份控制台可能由任一引擎托管（双引擎在线时 Python 出 `/control`，
    /// 侧车离线时 Rust 出）—— 两侧策略不一致 = 同一个 UI 两套安全边界。
    /// 实测缺口：Rust 侧曾**一个安全头都不发**（`preview-origin-csp.spec.ts` 收到
    /// 空 CSP），而 Python 侧一直有；本守卫防再犯。
    #[test]
    fn test_control_ui_security_headers_match_pyengine_middleware() {
        const PY: &str = include_str!("../../../pyengine/src/opensquilla/gateway/middleware.py");

        fn quoted(line: &str) -> Option<String> {
            let start = line.find('"')?;
            let end = line.rfind('"')?;
            (end > start).then(|| line[start + 1..end].to_string())
        }

        // CSP 在 Python 侧是多行括号拼接
        let mut py_csp = String::new();
        let mut lines = PY.lines();
        let mut found = false;
        while let Some(line) = lines.next() {
            if line.contains(r#"response.headers["content-security-policy"] = ("#) {
                found = true;
                for inner in lines.by_ref() {
                    let trimmed = inner.trim();
                    if trimmed == ")" {
                        break;
                    }
                    if let Some(piece) = quoted(trimmed) {
                        py_csp.push_str(&piece);
                    }
                }
                break;
            }
        }
        assert!(
            found,
            "pyengine 中间件里找不到 CSP 赋值 —— 契约形态已变，请同步本守卫"
        );
        assert_eq!(
            py_csp, CONTROL_UI_CSP,
            "两侧 Control UI CSP 不一致：改一边必须改另一边"
        );

        for (marker, expected) in [
            (r#"response.headers["x-frame-options"]"#, "DENY"),
            (r#"response.headers["x-content-type-options"]"#, "nosniff"),
            (
                r#"response.headers["referrer-policy"]"#,
                "strict-origin-when-cross-origin",
            ),
        ] {
            let line = PY
                .lines()
                .find(|l| l.contains(marker))
                .unwrap_or_else(|| panic!("pyengine 中间件缺少 {marker}"));
            // 取 `=` 右侧的字符串字面量（左侧方括号里也含引号，不能整行取首尾引号）
            let value_part = line.rsplit('=').next().unwrap_or_default();
            assert_eq!(
                quoted(value_part.trim()).as_deref(),
                Some(expected),
                "{marker} 的值两侧不一致"
            );
        }
    }

    // ------------------------------------------------------------------
    // 审批 REST 面（WebUI 的裁决通道）
    // ------------------------------------------------------------------

    /// 控制台必须能列出并裁决**本进程**挂起的审批。
    ///
    /// 缺陷形态（实测）：`chat.send` 由 Rust 抬升审批并注册进 `approval_registry`，而
    /// WebUI 的快照/裁决**只走 REST**；Rust 网关此前对 `/api/approvals*` 一律回 501，
    /// 于是「允许」按钮永远打不通，链式执行器只能等审批超时按 fail-closed 失败。
    #[tokio::test]
    async fn test_approvals_rest_lists_and_resolves_pending() {
        use crate::guard::approval_registry::global_registry;
        use crate::guard::gateway::{ApprovalGate, RiskLevel};
        use std::sync::Arc;
        use std::time::Duration;

        let state = e2e_state();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, build_router(state)).await.unwrap();
        });

        // 未带 token 必须 401（Phase 0：审批 REST 不再接受本机无凭据调用）
        let unauth = http_request(addr, "GET", "/api/approvals", None).await;
        assert!(unauth.starts_with("HTTP/1.1 401"), "无 token 应 401: {unauth}");
        assert!(!unauth.contains("\"id\":\"ap_"), "未授权不得泄露挂起项: {unauth}");

        // 基线：必须 200 且是结构化的 `{pending: [...]}`（此前是 501）。
        //
        // 注：注册表是**进程级**的，并行跑的链式执行器用例此刻可能正持有挂起审批 ——
        // 所以这里**不能**断言「整个数组为空」（那是对全局状态的无根据断言，
        // 实测已因此误报）。只锚定契约形状，本用例自己的 id 在场/缺席在下面断言。
        let before = http_request_auth(addr, "GET", "/api/approvals", None, "e2e-test-token").await;
        assert!(before.starts_with("HTTP/1.1 200"), "快照应 200: {before}");
        assert!(
            before.contains("\"pending\":["),
            "快照必须是结构化的 pending 数组: {before}"
        );

        // 注册一个真实挂起审批（与 chain/executor.rs 抬升路径同一注册表）
        let gate = Arc::new(ApprovalGate::new(Duration::from_secs(60)));
        let pending = gate
            .request("shell", r#"{"command":"rm -rf build"}"#.to_string(), RiskLevel::High)
            .expect("挂起审批");
        global_registry().register(gate.clone(), &pending);

        let listed = http_request_auth(addr, "GET", "/api/approvals", None, "e2e-test-token").await;
        assert!(listed.starts_with("HTTP/1.1 200"), "{listed}");
        assert!(listed.contains(&format!("\"id\":\"{}\"", pending.id)), "{listed}");
        assert!(listed.contains("\"toolName\":\"shell\""), "{listed}");
        assert!(listed.contains("\"destructive\":true"), "{listed}");
        assert!(listed.contains("rm -rf build"), "{listed}");

        // 裁决：批准 → 200 {resolved:true,approved:true}，且门控真的放行、注册表清空
        let resolved = http_request_auth(
            addr,
            "POST",
            "/api/approvals/resolve",
            Some(&format!(r#"{{"id":"{}","approved":true}}"#, pending.id)),
            "e2e-test-token",
        )
        .await;
        assert!(resolved.starts_with("HTTP/1.1 200"), "{resolved}");
        assert!(resolved.contains("\"resolved\":true"), "{resolved}");
        assert!(resolved.contains("\"approved\":true"), "{resolved}");
        assert_eq!(
            gate.outcome(&pending.id),
            crate::guard::gateway::ApprovalOutcome::Approved,
            "门控必须真的被裁定为批准（否则执行器仍在等）"
        );
        assert!(!global_registry().pending_ids().contains(&pending.id));

        // 复查：本用例的挂起项已从快照消失
        let after = http_request_auth(addr, "GET", "/api/approvals", None, "e2e-test-token").await;
        assert!(
            !after.contains(&format!("\"id\":\"{}\"", pending.id)),
            "裁决后应从快照消失: {after}"
        );
    }

    /// 裁决接口的否定面：缺 id → 400、未知 id → 404、坏 JSON → 400。
    ///
    /// 跨源 403 由 `gateway::approvals` 的单测覆盖（`http_request` 不带自定义头，
    /// 这里造不出带 `Origin` 的请求）。
    #[tokio::test]
    async fn test_approvals_resolve_rejects_bad_input() {
        let state = e2e_state();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, build_router(state)).await.unwrap();
        });

        let no_id = http_request_auth(addr, "POST", "/api/approvals/resolve", Some("{}"), "e2e-test-token").await;
        assert!(no_id.starts_with("HTTP/1.1 400"), "{no_id}");
        assert!(no_id.contains("id is required"), "{no_id}");

        let unknown = http_request_auth(
            addr,
            "POST",
            "/api/approvals/resolve",
            Some(r#"{"id":"ap_missing","approved":true}"#),
            "e2e-test-token",
        )
        .await;
        assert!(unknown.starts_with("HTTP/1.1 404"), "{unknown}");
        assert!(unknown.contains("approval is missing or does not match"), "{unknown}");

        let bad_json = http_request_auth(addr, "POST", "/api/approvals/resolve", Some("{oops"), "e2e-test-token").await;
        assert!(bad_json.starts_with("HTTP/1.1 400"), "{bad_json}");
    }

    // ------------------------------------------------------------------
    // 产物取字节 REST 面（控制台的下载 / 内联编辑通道）
    // ------------------------------------------------------------------

    /// 已登记产物必须能取到字节；未登记的 id 必须 404（**不得**按 URL 读任意文件）。
    #[tokio::test]
    async fn test_artifact_download_serves_registered_file_only() {
        let state = e2e_state();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, build_router(state)).await.unwrap();
        });

        let dir = std::env::temp_dir().join(format!("mgt_artifact_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let artifact_path = dir.join("report.txt");
        std::fs::write(&artifact_path, b"artifact payload").unwrap();
        let secret_path = dir.join("secret.txt");
        std::fs::write(&secret_path, b"top secret").unwrap();

        crate::tools::file_change::record_artifact_with_session(
            "agent:main:webchat:artifact-test",
            artifact_path.to_str().unwrap(),
            "created",
        );
        let entry = crate::tools::file_change::list_artifacts("agent:main:webchat:artifact-test", None, 10)
            .0
            .into_iter()
            .find(|a| a.path == artifact_path.to_str().unwrap())
            .expect("产物已登记");
        let encoded = urlencoding(&entry.id);

        // 未带 token 必须 401（产物字节不允许本机无凭据读取）
        let unauth = http_request(addr, "GET", &format!("/api/v1/artifacts/{encoded}"), None).await;
        assert!(unauth.starts_with("HTTP/1.1 401"), "无 token 应 401: {unauth}");
        assert!(!unauth.contains("artifact payload"), "401 绝不能回文件本体: {unauth}");

        let served = http_request_auth(addr, "GET", &format!("/api/v1/artifacts/{encoded}"), None, "e2e-test-token").await;
        assert!(served.starts_with("HTTP/1.1 200"), "已登记产物应 200: {served}");
        assert!(served.contains("artifact payload"), "应回文件本体: {served}");
        assert!(
            served.to_ascii_lowercase().contains("content-length: 16"),
            "应给出真实长度（前端据此拦超大文本）: {served}"
        );
        assert!(
            served.contains("content-disposition: attachment; filename=\"report.txt\""),
            "应带安全文件名: {served}"
        );

        // 关键否定面：同目录下的**未登记**文件不能靠猜 id 读到（否则就是任意文件读取）
        let unregistered = urlencoding(&format!("artifact:any:{}", secret_path.display()));
        let guess = http_request_auth(addr, "GET", &format!("/api/v1/artifacts/{unregistered}"), None, "e2e-test-token").await;
        assert!(guess.starts_with("HTTP/1.1 404"), "未登记路径必须 404: {guess}");
        assert!(!guess.contains("top secret"), "绝不能回未登记文件的内容: {guess}");

        // 不存在的 id → 404（不是 501、不是 500）
        let missing = http_request_auth(addr, "GET", "/api/v1/artifacts/artifact:nope:nope", None, "e2e-test-token").await;
        assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 百分号编码（产物 id 形如 `artifact:<session>:<path>`，含冒号与反斜杠）
    fn urlencoding(raw: &str) -> String {
        raw.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect()
    }

    // ------------------------------------------------------------------
    // 端到端（真实回环 127.0.0.1，无需外部网络；跑全量前需 sc stop nfwfp）
    // ------------------------------------------------------------------

    /// 发一个原始 HTTP 请求并返回完整原始响应（状态行 + 头 + 体）。
    ///
    /// 用**异步** `tokio::net::TcpStream`：同步 `std::net::TcpStream` 会阻塞 runtime worker，
    /// 单核环境下 accept 饿死、连接超时（错误码 10060）。见 AGENTS.md §4。
    ///
    /// `body = Some(..)` 时补上 `Content-Length`，这样 axum 才会把它当作请求体；
    /// 缺了它，POST 会被当成空体（回 405/400），断言就会测到别的东西。
    async fn http_request(
        addr: std::net::SocketAddr,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let body = body.unwrap_or("");
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf).to_string()
    }

    /// 带本地 token 的 REST 请求（Phase 0 收口后 `/api/*` 全部要求认证）。
    async fn http_request_auth(
        addr: std::net::SocketAddr,
        method: &str,
        path: &str,
        body: Option<&str>,
        token: &str,
    ) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let body = body.unwrap_or("");
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\
             Content-Type: application/json\r\nAuthorization: Bearer {token}\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf).to_string()
    }

    fn test_webui_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("marguerite-gw-e2e-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        // 夹具刻意**模仿真实 dist 的形状**（`base: './'` → 相对资源路径 + 有 <head>）：
        // 「深链白屏」这类缺陷只在相对资源 + 多段路径下出现，没有这两点的夹具测不出来。
        std::fs::write(
            dir.join("index.html"),
            r#"<!doctype html><html><head><meta charset="UTF-8" /><script type="module" src="./assets/index.js"></script></head><body><div id="app"></div></body></html>"#,
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
            rate_limiter: std::sync::Arc::new(crate::gateway::rate_limit::RateLimiter::new(
                crate::gateway::rate_limit::BucketConfig::new(240, 40.0),
                crate::gateway::rate_limit::BucketConfig::new(240, 40.0),
            )),
            rate_limit_enabled: false,
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

    /// 本地慢速 LLM mock：收到请求后先睡 `delay` 再回合法 chat completion。
    ///
    /// 用于复现「长任务期间 WS 循环是否仍能服务其它帧」——修复前 dispatch 被
    /// inline await，慢方法会把整个连接的事件循环（含 ping/tick）堵住数十秒，
    /// 客户端心跳看门狗随即判死重连，把在途的计划运行一起掐死。
    async fn slow_llm_server(delay: std::time::Duration) -> String {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    continue;
                };
                tokio::spawn(async move {
                    let _ = crate::tests::mock_http::read_request_full(&mut socket).await;
                    tokio::time::sleep(delay).await;
                    let body = r#"{"id":"cmpl-slow","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"slow-but-fine"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(body.as_bytes()).await;
                    let _ = socket.flush().await;
                });
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

    /// 回归（P0 断连）：慢方法在途时，同一条 WS 连接必须继续服务 ping 与其它请求。
    ///
    /// 修复前 `dispatch(...).await` 被 inline 在 select 循环里，一次慢 LLM 调用
    /// 就能把整条连接堵住：客户端 ping 得不到 pong，其心跳看门狗（20s 无任何帧）
    /// 判死并重连，重连又将在途的计划运行连同步骤一起掐掉。用户在聊天里看到的
    /// 就是「运行任务直接和后端断连」，plans.json 里则留下 step blocked + run cancelled。
    #[tokio::test]
    async fn e2e_slow_method_does_not_block_ping_and_other_requests() {
        use tokio_tungstenite::tungstenite::Message;

        // 慢 LLM：每次调用睡 5s。chat.send 一次回合会多次往返 LLM（意图/路由 +
        // 主完成），累计远超客户端 20s pong 超时的一半，足以暴露修复前的事件循环阻塞。
        let slow_base = slow_llm_server(std::time::Duration::from_secs(5)).await;
        let (addr, _state) = spawn_gateway_with(move |state| {
            state.config.model.base_url = slow_base;
            state.config.model.api_key = "e2e-key".to_string();
            state.config.model.timeout_secs = 90;
        })
        .await;

        let (mut ws, _challenge) = connect_ws(addr).await;
        ws.send(Message::Text(
            format!(
                r#"{{"type":"req","id":"1","method":"connect","params":{{"minProtocol":3,"maxProtocol":3,"auth":{{"token":"{}"}}}}}}"#,
                _state.token
            ),
        ))
        .await
        .unwrap();
        // 读到 HelloOk
        loop {
            let raw = ws.next().await.unwrap().unwrap();
            let v: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
            if v["type"] == "res" && v["id"] == "1" {
                assert_eq!(v["ok"], true);
                break;
            }
        }

        // 1) 发起一个必然走慢 LLM 的请求（chat.send），不等待它的响应
        ws.send(Message::Text(
            r#"{"type":"req","id":"slow","method":"chat.send","params":{"sessionKey":"e2e-slow","message":"hi"}}"#.into(),
        ))
        .await
        .unwrap();

        // 2) 慢请求在途期间，ping 必须立刻得到 pong
        ws.send(Message::Text(r#"{"type":"ping"}"#.into()))
            .await
            .unwrap();
        let pong_deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut got_pong = false;
        let mut got_slow = false;
        while std::time::Instant::now() < pong_deadline {
            let raw = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
                .await
                .expect("慢方法在途时 ping 必须仍在 3s 内得到响应（事件循环未被堵住）")
                .unwrap()
                .unwrap();
            let v: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
            if v["type"] == "pong" {
                got_pong = true;
                break;
            }
            if v["type"] == "res" && v["id"] == "slow" {
                got_slow = true;
            }
        }
        assert!(got_pong, "慢方法在途时未收到 pong —— WS 事件循环被 dispatch 阻塞");

        // 3) 同一条连接上的普通请求也必须能立即拿到响应。
        //    注意：慢请求的 res 可能在任何时刻到达（甚至在 step 2 的等待里），
        //    因此每个等待循环都必须顺手记录它，不能只在自己的阶段里找。
        ws.send(Message::Text(
            r#"{"type":"req","id":"fast","method":"health","params":{}}"#.into(),
        ))
        .await
        .unwrap();
        let mut got_fast = false;
        while let Ok(Some(Ok(raw))) =
            tokio::time::timeout(std::time::Duration::from_secs(3), ws.next()).await
        {
            let v: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
            if v["type"] == "res" && v["id"] == "slow" {
                got_slow = true;
            }
            if v["type"] == "res" && v["id"] == "fast" {
                assert_eq!(v["ok"], true);
                got_fast = true;
                break;
            }
        }
        assert!(
            got_fast,
            "慢方法在途时 health 未得到响应 —— 事件循环仍被阻塞"
        );

        // 4) 慢请求最终也必须正常收尾（不能因为改成异步分发就丢响应）
        while !got_slow {
            let Ok(Some(Ok(raw))) =
                tokio::time::timeout(std::time::Duration::from_secs(120), ws.next()).await
            else {
                break;
            };
            let v: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
            if v["type"] == "res" && v["id"] == "slow" {
                got_slow = true;
            }
        }
        assert!(got_slow, "慢方法的响应丢失 —— 异步分发路径有缺陷");
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

    /// F-T4-01（P0）四态矩阵：token 已配置时的三种连接形态 + token 未配置的
    /// 开箱即用放行。修复前「已配置 + 无 token」被 `unwrap_or(true)` 放行
    /// （QA 实测 `ok=True`），本用例把该态钉死在拒绝上。
    #[tokio::test]
    async fn e2e_ws_token_matrix_four_states() {
        use tokio_tungstenite::tungstenite::Message;

        // ① 已配置 token + 无 token → 拒（UNAUTHORIZED，与错误 token 同路）
        {
            let (addr, state) = spawn_gateway().await;
            assert!(!state.token.is_empty(), "本用例前提：e2e 网关已配置 token");
            let (mut ws, challenge) = connect_ws(addr).await;
            assert_eq!(challenge["event"], "connect.challenge");
            ws.send(Message::Text(
                r#"{"type":"req","id":"1","method":"connect","params":{"minProtocol":3,"maxProtocol":3}}"#
                    .into(),
            ))
            .await
            .unwrap();
            let raw = ws.next().await.unwrap().unwrap();
            let v: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
            assert_eq!(v["ok"], false, "已配置 token 时缺失 token 必须拒绝: {v}");
            assert_eq!(
                v["error"]["code"], "UNAUTHORIZED",
                "缺失 token 必须与错误 token 同一拒绝码: {v}"
            );
            if let Some(Ok(m)) = ws.next().await {
                assert!(m.is_close(), "拒绝后应关闭连接，实际: {m:?}");
            }
        }

        // ② 已配置 + 错误 token → 拒（与既有 e2e_ws_rejects_bad_token 对齐）
        {
            let (addr, _state) = spawn_gateway().await;
            let (mut ws, _challenge) = connect_ws(addr).await;
            ws.send(Message::Text(
                r#"{"type":"req","id":"1","method":"connect","params":{"auth":{"token":"wrong-token"}}}"#
                    .into(),
            ))
            .await
            .unwrap();
            let raw = ws.next().await.unwrap().unwrap();
            let v: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
            assert_eq!(v["ok"], false);
            assert_eq!(v["error"]["code"], "UNAUTHORIZED");
        }

        // ③ 已配置 + 正确 token → 放行（HelloOk，且后续方法可正常驱动）
        {
            let (addr, state) = spawn_gateway().await;
            let (mut ws, _challenge) = connect_ws(addr).await;
            ws.send(Message::Text(
                format!(
                    r#"{{"type":"req","id":"1","method":"connect","params":{{"minProtocol":3,"maxProtocol":3,"auth":{{"token":"{}"}}}}}}"#,
                    state.token
                )
            ))
            .await
            .unwrap();
            let raw = ws.next().await.unwrap().unwrap();
            let hello: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
            assert_eq!(hello["ok"], true, "正确 token 应放行: {hello}");
            assert_eq!(hello["auth"]["principal"]["isOwner"], true);
        }

        // ④ 未配置 token（空串，构造态/降级路径）+ 无 token → 开箱即用放行
        {
            let (addr, state) = spawn_gateway_with(|s| s.token = String::new()).await;
            assert!(state.token.is_empty(), "本用例前提：token 未配置");
            let (mut ws, _challenge) = connect_ws(addr).await;
            ws.send(Message::Text(
                r#"{"type":"req","id":"1","method":"connect","params":{"minProtocol":3,"maxProtocol":3}}"#
                    .into(),
            ))
            .await
            .unwrap();
            let raw = ws.next().await.unwrap().unwrap();
            let hello: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
            assert_eq!(
                hello["ok"], true,
                "token 未配置时应保持开箱即用放行（老部署升级不失联）: {hello}"
            );
        }
    }

    /// 带自定义 Origin 的 WS 连接助手（`connect_async` 不带 Origin，需手工
    /// 构造握手请求）。拒绝时 `client_async` 以 `Error::Http(403)` 失败。
    async fn connect_ws_with_origin(
        addr: SocketAddr,
        origin: Option<&str>,
    ) -> Result<
        (
            tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
            serde_json::Value,
        ),
        tokio_tungstenite::tungstenite::Error,
    > {
        // tungstenite 0.24 的 `client_async(Request)` 要求调用方自带全部 WS
        // 必需头（它只做校验不再补全）；这里按 RFC 6455 手工装配，key 取固定
        // 16 字节 base64（客户端请求 key 无唯一性要求）。
        use base64::Engine;
        let key = base64::engine::general_purpose::STANDARD.encode([7u8; 16]);
        let mut builder = tokio_tungstenite::tungstenite::http::Request::builder()
            .uri(format!("ws://{addr}/ws"))
            .header("Host", format!("{addr}"))
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", key);
        if let Some(origin) = origin {
            builder = builder.header("Origin", origin);
        }
        let request = builder.body(()).unwrap();
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (mut ws, _resp) = tokio_tungstenite::client_async(request, stream).await?;
        let first = ws.next().await.unwrap().unwrap();
        let value: serde_json::Value = serde_json::from_str(first.to_text().unwrap()).unwrap();
        Ok((ws, value))
    }

    /// F-T4-01 第二半：非白名单 Origin（模拟恶意网页 JS 直连）必须在
    /// **WS 升级前**被 403 拒绝，连接根本进不到 connect 握手。
    #[tokio::test]
    async fn e2e_ws_rejects_non_whitelisted_origin() {
        let (addr, _state) = spawn_gateway().await;
        for evil in [
            "https://evil.attacker.example",
            "http://evil.attacker.example:18790",
            "http://127.0.0.1:18790.evil.example",
            "http://localhost.evil.example",
            "ftp://localhost",
            "null",
        ] {
            let err = connect_ws_with_origin(addr, Some(evil))
                .await
                .expect_err(&format!("Origin={evil} 应被拒绝"));
            match err {
                tokio_tungstenite::tungstenite::Error::Http(resp) => {
                    assert_eq!(
                        resp.status(),
                        403,
                        "Origin={evil} 应以 HTTP 403 拒绝，实际: {:?}",
                        resp.status()
                    );
                }
                other => panic!("Origin={evil} 应得到 HTTP 403，实际错误: {other:?}"),
            }
        }
    }

    /// F-T4-01 第二半：白名单内 Origin（含 Tauri 桌面壳来源）完成握手；
    /// 无 Origin 头的原生客户端（CLI/探针）不被误杀。
    #[tokio::test]
    async fn e2e_ws_allows_whitelisted_and_absent_origin() {
        use tokio_tungstenite::tungstenite::Message;
        let (addr, state) = spawn_gateway().await;
        let port = addr.port();
        let origins: [Option<&str>; 7] = [
            None, // 原生客户端无 Origin 头
            Some("http://127.0.0.1"),
            Some(&format!("http://127.0.0.1:{port}")),
            Some(&format!("http://localhost:{port}")),
            Some("https://localhost:8443"),
            Some("tauri://localhost"),
            Some("http://tauri.localhost"),
        ];
        for origin in origins {
            let (mut ws, challenge) = connect_ws_with_origin(addr, origin)
                .await
                .unwrap_or_else(|_| panic!("Origin={origin:?} 应被放行"));
            assert_eq!(
                challenge["event"], "connect.challenge",
                "Origin={origin:?} 应进入握手"
            );
            ws.send(Message::Text(
                format!(
                    r#"{{"type":"req","id":"1","method":"connect","params":{{"minProtocol":3,"maxProtocol":3,"auth":{{"token":"{}"}}}}}}"#,
                    state.token
                )
            ))
            .await
            .unwrap();
            let raw = ws.next().await.unwrap().unwrap();
            let hello: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
            assert_eq!(
                hello["ok"], true,
                "Origin={origin:?} + 正确 token 应完成握手: {hello}"
            );
        }
    }

    /// Origin 白名单匹配规则纯函数矩阵（含 `gateway.origin_allowlist` 扩展）。
    #[test]
    fn ws_origin_allowlist_matching_rules() {
        let none: Vec<String> = Vec::new();
        // 放行：默认白名单（任意端口）+ 无 Origin
        for ok in [
            None,
            Some(""),
            Some("http://localhost"),
            Some("http://127.0.0.1:18790"),
            Some("http://127.0.0.1:1"),
            Some("http://LOCALHOST:18790"), // scheme/host 大小写不敏感
            Some("https://localhost"),
            Some("http://[::1]:18790"),
            Some("tauri://localhost"),
            Some("http://tauri.localhost"),
            Some("https://tauri.localhost"),
            Some("app://localhost"),
        ] {
            assert!(
                ws_origin_allowed(ok, &none),
                "应放行 Origin={ok:?}（默认白名单/无 Origin）"
            );
        }
        // 拒绝：白名单外/畸形
        for bad in [
            Some("https://evil.attacker.example"),
            Some("http://evil.attacker.example:18790"),
            Some("http://127.0.0.1:18790.evil.example"),
            Some("http://localhost.evil.example"),
            Some("http://user@localhost"),
            Some("ftp://localhost"),
            Some("null"),
            Some("localhost"), // 无 scheme
        ] {
            assert!(
                !ws_origin_allowed(bad, &none),
                "应拒绝 Origin={bad:?}（白名单外）"
            );
        }
        // 扩展：配置/环境变量追加项生效（仅加法，不动默认项）
        let extra = vec!["https://console.internal.example".to_string()];
        assert!(ws_origin_allowed(
            Some("https://console.internal.example"),
            &extra
        ));
        assert!(
            ws_origin_allowed(Some("https://console.internal.example:8443"), &extra),
            "扩展项不带端口 → 任意端口放行"
        );
        let extra = vec!["https://console.internal.example:8443".to_string()];
        assert!(ws_origin_allowed(
            Some("https://console.internal.example:8443"),
            &extra
        ));
        assert!(
            !ws_origin_allowed(Some("https://console.internal.example"), &extra),
            "扩展项带端口 → 不带端口的来源不匹配"
        );
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
            index.contains(r#"<div id="Marguerite-data""#),
            "应注入 data 元素，且 id 必须是前端消费方读取的 Marguerite-data"
        );
        assert!(index.contains(r#"data-base-path="/control""#));
        // 未带 Accept-Language + 配置为基线 en → 首屏 en（不再硬编码 zh-Hans）
        assert!(index.contains(r#"data-locale="en""#), "index 应注入 en: {index}");

        // SPA fallback（未知路由回退 index）
        let fallback = read_http("/control/chat").await;
        assert!(fallback.contains("200 OK"));
        assert!(fallback.contains(r#"<div id="app">"#));

        // 路径穿越防护 → 404
        let traversal = read_http("/control/static/../secret").await;
        assert!(traversal.contains("404"), "穿越应 404: {traversal}");
        // 编码变体也要挡（axum 解码 %2e%2e%2f → ../）
        let encoded = read_http("/control/static/%2e%2e%2fsecret").await;
        assert!(encoded.contains("404"), "编码穿越应 404: {encoded}");

        // 资产面缺失必须 404，不能回 SPA 的 index.html + 200
        // （回归：曾因回落 HTML 让前端把 .glb 当模型解析，恒显示「资产校验失败」）
        let missing_asset = read_http("/control/static/island3d/not-there.glb").await;
        assert!(
            missing_asset.contains("404"),
            "缺失资产必须 404 而非 index fallback: {missing_asset}"
        );
        assert!(
            !missing_asset.contains(r#"<div id="app">"#),
            "资产 404 不应包含 SPA 页面: {missing_asset}"
        );
    }

    /// 首屏语言契约（与 pyengine `control_ui._resolve_locale` 逐条对齐）
    #[test]
    fn ui_locale_resolution_matches_python_contract() {
        // ① 基线 en（默认）→ 交给 Accept-Language（首个受支持标签胜出）
        assert_eq!(
            resolve_ui_locale("en", Some("zh-CN,zh;q=0.9,en;q=0.8")),
            "zh-Hans"
        );
        assert_eq!(resolve_ui_locale("en", Some("ja-JP,ja;q=0.9")), "ja");
        assert_eq!(resolve_ui_locale("en", Some("fr-FR")), "fr");
        // 不支持的标签继续往后找
        assert_eq!(resolve_ui_locale("en", Some("ko-KR,de-DE;q=0.8")), "de");
        // 都不支持 / 没给头 → en（**绝不再硬编码 zh-Hans**）
        assert_eq!(resolve_ui_locale("en", Some("ko-KR")), "en");
        assert_eq!(resolve_ui_locale("en", None), "en");
        // ② 显式钉死非 en → 运维意图不被浏览器覆盖
        assert_eq!(
            resolve_ui_locale("zh-Hans", Some("en-US,en;q=0.9")),
            "zh-Hans"
        );
        // zh 的任意拼写收敛到 zh-Hans（与 Python 的 zh* → zh-Hans 一致）
        assert_eq!(resolve_ui_locale("zh-CN", Some("en-US")), "zh-Hans");
        assert_eq!(resolve_ui_locale("ja", None), "ja");
        // ③ 非法配置 → 视为未设置，回落到浏览器
        assert_eq!(resolve_ui_locale("klingon", Some("es-ES")), "es");
    }

    /// `<html lang>` 与注入语言同步：只改首个 `<html …>` 标签，别处不动
    #[test]
    fn html_lang_rewrite_is_scoped_to_first_html_tag() {
        let out = replace_html_lang(
            r#"<!doctype html><html lang="en" class="dark"><body>x</body></html>"#,
            "zh-Hans",
        );
        assert!(
            out.contains(r#"<html lang="zh-Hans" class="dark">"#),
            "{out}"
        );
        assert!(out.contains("<body>x</body>"));
        // 无 lang 属性 → 补一个
        let out2 = replace_html_lang("<!doctype html><html><body>x</body></html>", "ja");
        assert!(out2.contains(r#"<html lang="ja">"#), "{out2}");
        // 无 <html> → 原样返回，不 panic
        assert_eq!(replace_html_lang("plain", "ja"), "plain");
    }

    /// 真 HTTP 层：网关托管的 index.html 首屏语言随 `Accept-Language` 变化
    /// （此前硬编码 zh-Hans，英语浏览器首访也被强制中文）
    #[tokio::test]
    async fn e2e_index_language_follows_accept_language() {
        let (addr, _state) = spawn_gateway().await;
        let get = |accept: &'static str| {
            let addr: SocketAddr = addr;
            async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
                let req = format!(
                    "GET /control/ HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept-Language: {accept}\r\nConnection: close\r\n\r\n"
                );
                stream.write_all(req.as_bytes()).await.unwrap();
                let mut buf = Vec::new();
                stream.read_to_end(&mut buf).await.unwrap();
                String::from_utf8_lossy(&buf).to_string()
            }
        };
        let zh = get("zh-CN,zh;q=0.9").await;
        assert!(zh.contains(r#"data-locale="zh-Hans""#), "{zh}");
        assert!(zh.contains(r#"<html lang="zh-Hans""#), "{zh}");
        let ja = get("ja-JP,ja;q=0.9").await;
        assert!(ja.contains(r#"data-locale="ja""#), "{ja}");
        let ko = get("ko-KR").await;
        assert!(ko.contains(r#"data-locale="en""#), "{ko}");
    }

    /// F-T4-01 兼容性契约：index.html 注入的 `#Marguerite-data` 必须带
    /// `data-ws-token`（= 本网关 token）——token 强制校验后，桌面 WebView
    /// 加载的同源页面靠它完成 connect 握手（`webui/src/stores/rpc.ts`
    /// 的 `injectedWsToken`）。没有它，桌面壳会直接失联。
    #[tokio::test]
    async fn e2e_index_injects_ws_bootstrap_token() {
        let (addr, state) = spawn_gateway().await;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let req = "GET /control/ HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let body = String::from_utf8_lossy(&buf).to_string();
        assert!(
            body.contains(&format!(r#"data-ws-token="{}""#, state.token)),
            "index 应注入与本网关一致的 data-ws-token: {body}"
        );
    }

    /// `<base href>` 注入：深链白屏的修复（顺序、幂等、保守）
    #[test]
    fn base_href_injection_precedes_url_bearing_tags() {
        let html = r#"<!doctype html><html><head><meta charset="UTF-8" /><link rel="icon" href="./mark.svg" /><script type="module" src="./assets/index.js"></script></head><body><div id="app"></div></body></html>"#;
        let out = inject_base_href(html, CONTROL_BASE_PATH);
        assert!(out.contains(r#"<base href="/control/" />"#), "{out}");
        // 必须早于任何携带相对 URL 的元素，否则它们仍按文档路径解析
        let base_at = out.find("<base").unwrap();
        assert!(base_at < out.find("./mark.svg").unwrap(), "{out}");
        assert!(base_at < out.find("./assets/index.js").unwrap(), "{out}");
        // 幂等：已注入过不再重复插入
        assert_eq!(inject_base_href(&out, CONTROL_BASE_PATH).matches("<base").count(), 1);
        // 已有显式 <base> → 尊重构建方决策，原样返回
        let explicit = r#"<html><head><base href="/x/" /></head><body></body></html>"#;
        assert_eq!(inject_base_href(explicit, CONTROL_BASE_PATH), explicit);
        // 无 <head> 的片段 → 原样返回（不猜）
        assert_eq!(inject_base_href("<div>x</div>", CONTROL_BASE_PATH), "<div>x</div>");
    }

    /// 真 HTTP 层：**深链**（≥2 段的 SPA 路径）回退的 index.html 必须带 `<base href="/control/">`
    /// —— 否则浏览器会把 `./assets/…` 解析到 `/control/settings/assets/…`，拿到 text/html，
    /// 模块脚本被 MIME 校验拒绝，整页白屏（实测：`/control/settings/connection` 冷启全白）。
    #[tokio::test]
    async fn e2e_deep_link_index_carries_base_href() {
        let (addr, _state) = spawn_gateway().await;
        let body = http_get_raw(addr, "/control/settings/connection").await;
        assert!(body.contains(r#"<base href="/control/" />"#), "{body}");
        assert!(body.contains(r#"data-base-path="/control""#), "{body}");
        // 相对资源路径仍在（Vite base:'./'）——靠 <base> 才能解析正确，故顺序必须早于它
        let base_at = body.find("<base").expect("必须注入 base");
        let asset_at = body.find("./assets/").expect("产物应有相对资源路径");
        assert!(base_at < asset_at, "{body}");
    }

    /// 原始 HTTP GET（测试辅助）：返回含状态行与响应头的完整文本。
    async fn http_get_raw(addr: SocketAddr, path: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf).to_string()
    }

    /// P0-D 缺陷 1 回归：WebUI 产物缺失时 `/control/` 必须回**可渲染的诊断页**
    /// （503 + `x-marguerite-webui: missing` + 页内标记 + 修复指引），
    /// 而不是 `404` 且 `content-length: 0` 的空白页；同时 `/api/health` 仍 200
    /// （网关活着 ≠ 控制台可用，这正是就绪判定必须看 `/control/` 的原因）。
    #[tokio::test]
    async fn e2e_control_missing_webui_returns_diagnostic_page_not_blank_404() {
        let missing = std::env::temp_dir().join(format!(
            "marguerite-gw-missing-{}",
            uuid::Uuid::new_v4()
        ));
        assert!(!missing.exists(), "前置：产物目录不存在");
        let (addr, _state) = spawn_gateway_with(|s| s.webui_dir = missing.clone()).await;

        let page = http_get_raw(addr, "/control/").await;
        assert!(
            page.contains("503 Service Unavailable"),
            "缺失产物必须 503（非 200 假就绪、非空白 404）: {page}"
        );
        assert!(
            page.to_ascii_lowercase()
                .contains(&format!("{WEBUI_MISSING_HEADER}: {WEBUI_MISSING_VALUE}")),
            "缺识别标记响应头: {page}"
        );
        assert!(
            page.contains(WEBUI_MISSING_MARKER),
            "缺页内标记（桌面就绪探测据此区分真伪页）: {page}"
        );
        assert!(page.contains("webui/dist"), "须给出可执行修复指引: {page}");
        assert!(page.contains("npm run build"), "须给出构建指引: {page}");
        assert!(
            !page.contains("content-length: 0"),
            "不得再回空白响应: {page}"
        );

        // SPA 深链同样给诊断页（而不是 404 白屏）
        let deep = http_get_raw(addr, "/control/settings").await;
        assert!(deep.contains("503 Service Unavailable"), "深链: {deep}");

        // 网关本身仍健康：HTTP 200 —— 健康检查不能替代界面就绪判定
        let health = http_get_raw(addr, "/api/health").await;
        assert!(health.contains("200 OK"), "health 应 200: {health}");
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
    /// 配置注入「本地 400 mock」的 LLM 端点（400 不可重试），
    /// 使 chat.send 后台引擎快速失败 → Error 事件确定回流，不依赖外部网络。
    /// （MSI P0 修复后：空 API Key 在 chat.send 前置拦截返回 API_KEY_MISSING，
    /// 不再进入引擎层；本测试注入了有效 key + 400 mock 端点测引擎失败路径。）
    #[tokio::test]
    async fn e2e_fusion_methods_over_ws() {
        use std::time::Duration;
        // **隔离配置目录**（与 chat.rs::ctx_with_bus 同因同治，不可省）：
        // `run_turn` 在会话入口调用 `apply_effective_model`，会读 `config_dir/models.json`
        // 的 active 档案并覆盖 provider/model/base_url/key。若读到开发者本机档案库，
        // 下面就指向本地 400 mock 的 base_url 会被换回真实云端端点 —— 用例不仅会红
        // （真实请求可能成功，永远不会发 `.error`），还会真的把测试请求发到云端。
        // 空隔离目录里没有 `models.json`，`apply_effective_model` 原样返回注入配置。
        // 注：`OVERRIDE_CONFIG_DIR` 是 thread_local，只在当前线程可见。单测下
        // `spawn_slow_dispatch` 固定用 `tokio::spawn`（不切 `SLOW_RUNTIME`），
        // 故 dispatch 与 `run_turn` 都留在本测试线程，覆盖可见（详见其注释）。
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let isolated = std::env::temp_dir().join(format!(
            "mgt-e2e-fusion-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::create_dir_all(&isolated);
        let _ = std::fs::remove_file(isolated.join("models.json")); // 目录名复用时的防御
        Config::set_config_dir_override(isolated);

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
            rate_limiter: std::sync::Arc::new(crate::gateway::rate_limit::RateLimiter::new(
                crate::gateway::rate_limit::BucketConfig::new(240, 40.0),
                crate::gateway::rate_limit::BucketConfig::new(240, 40.0),
            )),
            rate_limit_enabled: false,
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
        //
        // 注意：慢方法的 dispatch 现在跑在独立任务里，事件可能在 res 之前到达
        // （前端本就按此设计：useChatSend.ts 的「Event frames can beat the
        // chat.send response」+ pending 缓冲回放）。因此必须按 id 匹配响应，
        // 不能假设「下一帧就是我的 res」。
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"type":"req","id":"13","method":"chat.send","params":{"message":"hi","sessionKey":"ws.e2e1","clientRequestId":"cr-e2e"}}"#.into(),
        ))
        .await
        .unwrap();
        let mut send_res: Option<serde_json::Value> = None;
        let mut got_error = false;
        let res_deadline = std::time::Instant::now() + Duration::from_secs(15);
        while send_res.is_none() && std::time::Instant::now() < res_deadline {
            let Ok(Some(Ok(raw))) =
                tokio::time::timeout(Duration::from_millis(500), ws.next()).await
            else {
                continue;
            };
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw.to_text().unwrap()) {
                // 异步分发后事件可能先于 res 到达，两个目标都要在同一循环里收。
                if v["type"] == "res" && v["id"] == "13" {
                    send_res = Some(v);
                } else if v["type"] == "event"
                    && v["event"].as_str().is_some_and(|e| e.ends_with(".error"))
                {
                    got_error = true;
                }
            }
        }
        let v = send_res.expect("应收到 id=13 的 chat.send 响应");
        assert_eq!(v["ok"], true);
        assert_eq!(v["payload"]["accepted"], true);
        assert_eq!(v["payload"]["sessionKey"], "ws.e2e1");

        // 事件桥：应收到 session.event.error（引擎快速失败）。若它比 res 更早
        // 到达，上面的循环已经收下了。
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while !got_error && std::time::Instant::now() < deadline {
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
        for (expected_seq, _) in (1_u64..).zip(0..2) {
            let raw = ws.next().await.unwrap().unwrap();
            let ev: serde_json::Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
            assert_eq!(ev["type"], "event");
            assert_eq!(ev["seq"], expected_seq, "事件 seq 应连续递增: {ev}");
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

#[cfg(test)]
mod rate_limit_surface_tests {
    use super::rate_limit_applies;

    #[test]
    fn limiter_covers_api_rest_but_never_liveness_or_static() {
        assert!(rate_limit_applies("/api/config"));
        assert!(rate_limit_applies("/api/v1/files/upload"));
        assert!(rate_limit_applies("/api/approvals"));
        // 生存性优先：健康检查永不被限流（R61 的看门狗教训）。
        assert!(!rate_limit_applies("/api/health"));
        // WS 升级与静态资源不在作用面内。
        assert!(!rate_limit_applies("/ws"));
        assert!(!rate_limit_applies("/control/"));
        assert!(!rate_limit_applies("/control/static/app.js"));
    }
}
