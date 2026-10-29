//! 审批 REST 面：`GET /api/approvals` 与 `POST /api/approvals/resolve`。
//!
//! # 为什么必须由 **Rust** 网关提供
//!
//! WebUI 的审批卡片有两条独立传输：新审批的**推送**走 WS `session.event.approval`，
//! 而**快照与裁决只走 REST**（`webui/src/composables/chat/useChatApprovals.ts` 的
//! `fetchSnapshot` / `resolveApproval` 都是 `fetch('/api/approvals…')`）。
//! 抬升审批的 `chat.send` 由 Rust 实现（`ASYNC_ONLY_METHODS`），挂起项注册进
//! [`crate::guard::approval_registry`]。而在此之前 Rust 网关对 `/api/approvals*`
//! 一律回 501 —— 于是**在控制台点「允许」永远不会生效**：链式执行器只能等审批超时，
//! 按 fail-closed 判失败。这不是「能力子集」，是功能坏掉（`guard/approval_registry.rs`
//! 的头注释也把 `/api/approvals` 列为 WebUI 的三条裁决链路之一，等于文档承诺了、
//! 代码没兑现）。
//!
//! # 契约（与 pyengine `app.py::api_approvals / api_approvals_resolve` 对齐）
//!
//! - `GET  /api/approvals`         → 200 `{pending, mode, allowPatterns, denyPatterns,
//!   sidecarOffline, pending_sidecar_offline}`（后两项是 P2-4 的双引擎可见性）
//! - `POST /api/approvals/resolve` → 200 `{resolved, approved}`；缺 `id` → 400
//!
//! 写接口按 pyengine `_same_origin` 的口径做同源校验（见 [`origin_allowed`]）。
//!
//! # 两侧队列（双引擎）
//!
//! Rust 引擎与侧车**各有一份审批队列**：Rust 的挂起项在
//! [`crate::guard::approval_registry`]，侧车的在它的 `ApprovalQueue`。控制台只连 Rust
//! 网关，所以本模块负责把两侧缝起来：快照**合并**两侧（[`merge_pending`]），裁决时先查
//! 本进程、未命中再**转发**给侧车的 `/api/approvals/resolve`。
//!
//! 缺了转发就会是「卡片推得到、点『允许』却什么也不发生」——侧车抬起的审批
//! （渠道/自动化流程）实测就是这种情况：REST 面回 404，而侧车队列里那条仍然
//! `pending: true`（R57 实测）。

use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

use crate::gateway::server::GatewayState;
use crate::guard::approval_registry::{global_registry, ApprovalRegistry, DecisionResult};
use crate::guard::gateway::{PendingApproval, RiskLevel};
use crate::observability::redact;

/// 拉侧车快照的超时：控制台的 hydration 会被频繁触发（进入会话、可选的 2s 轮询），
/// 侧车启动期（Python 网关 boot 可长达 ~90s）不能把它拖住 —— 宁可只报本进程那半。
const SIDECAR_LIST_TIMEOUT: Duration = Duration::from_secs(2);

/// 转发裁决的超时：这是用户点「允许/拒绝」后的等待，比快照多给一点耐心，
/// 但仍必须有上限 —— 侧车卡死时界面要能落到明确错误，而不是无限转圈。
const SIDECAR_RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);

/// 审批 REST 未授权响应（fail-closed）。
///
/// 为什么必须在这里也校验 token：审批裁决是**能放行高危操作**的写接口，
/// 仅靠 Origin 校验挡不住本机进程（无 Origin 头会被 `origin_allowed` 放行）。
pub(crate) fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(json!({
            "error": "UNAUTHORIZED",
            "code": "UNAUTHORIZED",
            "message": "缺少或错误的本地网关 token",
        })),
    )
        .into_response()
}

/// GET `/api/approvals`：当前挂起审批的快照。
///
/// `mode` / `allowPatterns` / `denyPatterns` 是 pyengine 快照的必备字段；Rust 引擎
/// **没有**审批模式存储（审批门恒为「逐个等人工裁决」），故如实回 `prompt` + 空模式表，
/// 而不是伪造一个看起来更像 Python 的策略。
///
/// 侧车在线时**合并**它的队列（见 `merge_pending`）：两边是两套独立队列，只报自己那半
/// 会让控制台漏掉侧车侧挂起的审批 —— 那些卡片推得到、裁决却打不通（R57 实测）。
pub(crate) async fn approvals_list(
    State(state): State<std::sync::Arc<GatewayState>>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Response {
    if !crate::gateway::auth::rest_authorized(&headers, Some(&uri), &state.token) {
        return unauthorized();
    }
    let mut pending: Vec<Value> = global_registry()
        .pending_snapshots()
        .iter()
        .map(pending_item)
        .collect();
    let mut mode = "prompt".to_string();
    let mut allow_patterns = json!([]);
    let mut deny_patterns = json!([]);

    let snapshot = sidecar_snapshot(&state).await;
    let sidecar_offline = snapshot.is_none();
    if let Some(snapshot) = snapshot {
        if let Some(items) = snapshot.get("pending").and_then(Value::as_array) {
            pending = merge_pending(pending, items);
        }
        if let Some(sidecar_mode) = snapshot.get("mode").and_then(Value::as_str) {
            mode = sidecar_mode.to_string();
        }
        allow_patterns = snapshot
            .get("allowPatterns")
            .cloned()
            .unwrap_or_else(|| json!([]));
        deny_patterns = snapshot
            .get("denyPatterns")
            .cloned()
            .unwrap_or_else(|| json!([]));
    }
    // P2-4：侧车离线时它的挂起审批既不会出现在 Rust 队列，也不会推事件 ——
    // 用户只看到「没有待审批」，重连后那几条才突然冒出来。这里直接从侧车
    // 持久化的审批队列（SQLite，`state/approval_queue.sqlite`）读出未裁决数量，
    // 让前端能显示「侧车离线，N 条审批待同步」的黄条，而不是假装队列是空的。
    let pending_sidecar_offline = if sidecar_offline {
        offline_sidecar_pending().unwrap_or(0)
    } else {
        0
    };

    let body = json!({
        "pending": pending,
        "mode": mode,
        "allowPatterns": allow_patterns,
        "denyPatterns": deny_patterns,
        // 双口径：布尔给样式（黄条显隐），计数给文案（N 条待同步）。
        "sidecarOffline": sidecar_offline,
        "pending_sidecar_offline": pending_sidecar_offline,
    });
    (StatusCode::OK, axum::Json(body)).into_response()
}

/// 侧车持久化审批队列的默认路径。
///
/// 侧车由本进程以 `OPENSQUILLA_HOME=<data_dir>/pyengine-home` 且未设置
/// `OPENSQUILLA_PROFILE` 启动，于是 pyengine `paths.state_dir()` 解析成
/// `<home>/default/state`（见 `pyengine/src/opensquilla/paths.py`）。
/// 段名写死是有意为之：这条路径必须与 spawn 侧的约定一致，漂移会让离线计数
/// 永远读到不存在的文件（表现为「静默 0」）。
fn sidecar_queue_path() -> std::path::PathBuf {
    crate::persistence::config::Config::data_dir()
        .join("pyengine-home")
        .join("default")
        .join("state")
        .join("approval_queue.sqlite")
}

/// 侧车离线时，读它落盘的队列统计「仍未裁决」的条数。
///
/// 返回 `None` = 队列文件不存在 / 表结构不符 / 打不开（从未跑过侧车，或侧车
/// 降级到临时 home）——调用方按 0 处理；绝不因为读不到就编一个非零数字。
fn offline_sidecar_pending() -> Option<u64> {
    count_pending_in_queue(&sidecar_queue_path())
}

fn count_pending_in_queue(path: &std::path::Path) -> Option<u64> {
    if !path.is_file() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM approval_queue WHERE resolved = 0 AND consumed = 0",
            [],
            |row| row.get(0),
        )
        .ok()?;
    u64::try_from(count).ok()
}

/// POST `/api/approvals/resolve`：裁决一个挂起审批。
///
/// 请求体：`{id, approved, namespace?, choice?}`（与 pyengine 同形；`namespace`/`choice`
/// 在 Rust 侧不参与定位 —— 注册表按 `id` 全局唯一索引，`choice` 的沙箱语义只有
/// pyengine 的队列实现有）。
///
/// 本进程注册表里找不到 id 时**转发给侧车**：侧车抬起的审批（渠道/自动化流程）在两侧
/// 各有一份队列，不转发的话控制台只会回 404，用户点了「允许」而审批仍然挂在那里（R57 实测）。
pub(crate) async fn approvals_resolve(
    State(state): State<std::sync::Arc<GatewayState>>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    body: Bytes,
) -> Response {
    if !crate::gateway::auth::rest_authorized(&headers, Some(&uri), &state.token) {
        return unauthorized();
    }
    if !origin_allowed(
        headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()),
        headers.get(header::HOST).and_then(|v| v.to_str().ok()),
    ) {
        // 与 pyengine `forbidden_origin_response()` 同一载荷（跨源写请求的 CSRF 兜底）
        return (
            StatusCode::FORBIDDEN,
            axum::Json(json!({
                "error": "cross-origin requests are not allowed",
                "code": "FORBIDDEN_ORIGIN",
            })),
        )
            .into_response();
    }

    let Ok(parsed) = serde_json::from_slice::<Value>(&body) else {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({ "error": "Invalid JSON body" })),
        )
            .into_response();
    };
    let Some(id) = parsed
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({ "error": "id is required" })),
        )
            .into_response();
    };
    let approved = parsed
        .get("approved")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    match global_registry().decide(id, approved) {
        DecisionResult::Decided(_) => (
            StatusCode::OK,
            axum::Json(json!({ "resolved": true, "approved": approved })),
        )
            .into_response(),
        // 本进程没有 → 可能是侧车抬起的审批：转发给它（幂等：侧车自己会判「已裁决」）
        DecisionResult::NotFound => {
            if let Some(resp) = forward_resolve(&state, &body).await {
                return resp;
            }
            // 与 Rust 侧同义（`security_seam.rs` 用的是同一句话）：id 不存在、不匹配，
            // 或已被其它宿主裁决过。
            (
                StatusCode::NOT_FOUND,
                axum::Json(json!({ "error": "approval is missing or does not match" })),
            )
                .into_response()
        }
    }
}

/// 侧车在线时的审批快照（离线/超时/非 200 一律 `None`：绝不因为侧车慢而让快照变空）。
async fn sidecar_snapshot(state: &GatewayState) -> Option<Value> {
    let base = crate::gateway::server::py_sidecar()?.base_url();
    let resp = sidecar_client()
        .get(format!("{base}/api/approvals"))
        .bearer_auth(&state.token)
        .timeout(SIDECAR_LIST_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<Value>().await.ok()
}

/// 把裁决请求原样转发给侧车；成功时把侧车的状态码与响应体如实回给控制台。
///
/// 返回 `None` = 侧车不可用（离线/超时/连接失败），调用方据此回自己的 404。
async fn forward_resolve(state: &GatewayState, body: &Bytes) -> Option<Response> {
    let base = crate::gateway::server::py_sidecar()?.base_url();
    let resp = sidecar_client()
        .post(format!("{base}/api/approvals/resolve"))
        .bearer_auth(&state.token)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body.clone())
        .timeout(SIDECAR_RESOLVE_TIMEOUT)
        .send()
        .await
        .ok()?;
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let payload = resp.bytes().await.ok()?;
    Some(
        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(payload))
            .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response()),
    )
}

/// 侧车 HTTP 客户端（进程级复用连接池；R61 起 `pub(crate)` 供通用侧车 REST 代理复用）。
///
/// **`.no_proxy()` 是必需项，不是优化**：侧车永远在回环（`127.0.0.1:<随机端口>`），而
/// reqwest 默认会采用系统代理（Windows 上读注册表的 `ProxyEnable/ProxyServer`，
/// 本机实测 `127.0.0.1:7890` 的 iKuuuVPN/clash）。代理会**拒绝转发回环目标**并回
/// `502`（空体），于是「点允许无效 / 附件传不上去」在装了代理的机器上必然复现，
/// 且现象与「侧车离线」难以区分。回环流量一律直连。
pub(crate) fn sidecar_client() -> &'static reqwest::Client {
    static C: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// 合并两侧队列，按条目 `id` 去重（本进程在前）。
///
/// 两条队列的 id 空间不同（Rust `ap_<uuid>` / 侧车十六进制），正常不会撞；去重是为了
/// 万一将来某条审批同时出现在两侧时，**同一 id 不要在快照里出现两次**（前端按 id 折叠中断，
/// 重复条目会把同一张卡片渲染两次）。
fn merge_pending(local: Vec<Value>, remote: &[Value]) -> Vec<Value> {
    let mut seen: std::collections::HashSet<String> = local
        .iter()
        .filter_map(|v| v.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let mut merged = local;
    for item in remote {
        let id = item.get("id").and_then(Value::as_str).unwrap_or("");
        if !id.is_empty() && !seen.insert(id.to_string()) {
            continue;
        }
        merged.push(item.clone());
    }
    merged
}


/// 挂起审批 → WebUI 快照条目。
///
/// 只映射 UI 明确定义过的字段（`ApprovalsSnapshotItem`），且**只映射真实存在的信息**：
/// Rust 的 [`PendingApproval`] 没有会话键、显示提示（displayKind/displayTarget）、
/// 备份状态与截止时间，就不编造 —— 缺哪项，卡片就按「通用敏感操作」渲染。
/// 把挂起审批投影成**可以在浏览器里显示**的条目。
///
/// `args` 是审批队列的内部策略记录，可能含工具凭据（`curl -H "Authorization: Bearer …"`）、
/// 复核指纹等 —— 整个对象原样塞进 HTTP 响应，就等于把凭据送到任何能读本机回环的进程
/// 与浏览器开发者工具里。pyengine 的 `approval_events.py::_json_safe_display_value`
/// 对同一份数据做了「递归脱敏 + 丢弃 fingerprint/review* 键」，这里按同一口径对齐
/// （REST 快照是**跨实现**契约，两侧形状必须一致，见本模块头注释）。
fn pending_item(p: &PendingApproval) -> Value {
    let args = display_value_opt(serde_json::from_str::<Value>(&p.args).ok());
    let command = args
        .as_ref()
        .and_then(|v| v.get("command"))
        .and_then(Value::as_str)
        .map(str::to_string);
    json!({
        "id": p.id,
        "namespace": "exec",
        "toolName": p.tool,
        "command": command,
        // 解析不出来时也不能原样透传：`raw` 走同一套脱敏
        "args": args.unwrap_or_else(|| json!({ "raw": redact::scrub_text(&p.args) })),
        "destructive": p.risk == RiskLevel::High,
    })
}

fn display_value_opt(value: Option<Value>) -> Option<Value> {
    value.map(|v| display_value(&v))
}

/// 递归脱敏（与 pyengine `_json_safe_display_value` 的判定顺序一致）：
/// 先按键名丢弃复核指纹类字段，再按键名抹掉敏感值，其余字符串过文本脱敏。
fn display_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| !is_review_metadata(key))
                .map(|(key, item)| {
                    let projected = if redact::is_secret_key(key) {
                        Value::String(redact::REDACTED.to_string())
                    } else {
                        display_value(item)
                    };
                    (key.clone(), projected)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(display_value).collect()),
        Value::String(text) => Value::String(redact::scrub_text(text)),
        other => other.clone(),
    }
}

/// 复核指纹与 review 元数据不进展示面（pyengine 同款：`fingerprint` 子串 / `review` 前缀）。
fn is_review_metadata(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    lowered.contains("fingerprint") || lowered.starts_with("review")
}

/// 状态查询的命名空间（与 pyengine `approval_lookup_status_rpc_payload` 同词表）。
fn status_namespace(method: &str) -> &'static str {
    if method.starts_with("plugin.") {
        "plugin"
    } else {
        "exec"
    }
}

/// RPC `exec.approval.status` / `plugin.approval.status` 的本进程应答
/// （`Some` = 本条审批确实由 Rust 引擎持有且仍挂起；`None` = 不在本进程，调用方转发侧车）。
///
/// # 为什么状态查询也必须先问本进程
///
/// `exec.approval.` 在 `PY_ENGINE_PREFIXES` 里，于是状态探测会被代理到侧车。但两侧是
/// **两套独立队列**（见模块头）：侧车对 Rust 抬起的审批一无所知，会如实回 `found:false` ——
/// 而前端把 `found:false` 解读为**终态** `unavailable`，重连恢复时挂起卡片自己折叠成
/// 「审批已不可用」，用户再也点不了「允许」。这与 R54（裁决 REST）/ R57（跨队列裁决）/
/// R61（REST 面兜底）同族，是「某一侧沉默地被当成另一侧的答案」。
pub(crate) fn registry_status(
    registry: &ApprovalRegistry,
    method: &str,
    params: &Value,
) -> Option<Value> {
    // Rust 注册表只有 exec 语义的挂起项（`pending_item` 同样固定 `namespace: "exec"`）。
    // `plugin.approval.status` 问进来时交还调用方转发，绝不跨命名空间作答。
    if status_namespace(method) != "exec" {
        return None;
    }
    let id = params.get("id").and_then(Value::as_str)?.trim();
    if id.is_empty() {
        return None;
    }
    // `pending_snapshots` 已过滤超时项：超时即 fail-closed 拒绝，不该再答「仍挂起」。
    let pending = registry
        .pending_snapshots()
        .into_iter()
        .find(|p| p.id == id)?;
    Some(json!({
        "found": true,
        "id": pending.id,
        "namespace": "exec",
        "pending": true,
        "resolutionInProgress": false,
        "resolved": false,
        "approved": false,
        "resolution": "",
        "consumed": false,
        // `PendingApproval` 只有单调的 `Instant`，没有墙上时钟截止时间 ——
        // 如实回 null（前端 `applyApprovalDeadline` 遇非有限值直接跳过），不编造。
        "deadline": Value::Null,
    }))
}

/// 两侧队列都没有这条审批时的查询结果（与 pyengine
/// `approval_lookup_status_rpc_payload` 的缺省分支逐字段同形）。
pub(crate) fn missing_status(method: &str, params: &Value) -> Value {
    json!({
        "found": false,
        "id": params.get("id").and_then(Value::as_str).unwrap_or(""),
        "namespace": status_namespace(method),
        "pending": false,
        "resolutionInProgress": false,
        "resolved": false,
    })
}

/// 同源校验（照 pyengine `origin_guard._origin_allowed` 的口径，去掉 `cors.allowed_origins`
/// 白名单 —— Rust 网关没有 CORS 配置项）。
///
/// 规则：
/// 1. 无 `Origin`（curl / 桌面 node 客户端）→ 放行：不是浏览器发起的跨源请求；
/// 2. `Origin: null`（沙箱 iframe / data: 页面）→ 拒绝；
/// 3. 其它：`Origin` 必须与请求自身 authority 同源（scheme=http、主机名、有效端口都要相等）；
/// 4. 请求 authority 必须是**回环**（IP 字面量回环或 `localhost` / `*.localhost`）。
///
/// 第 4 条是 DNS rebinding 防线：网关只绑 `127.0.0.1`，但如果有个恶意域名解析到回环，
/// 浏览器会把 `Host` 与 `Origin` **同时**写成那个域名，第 3 条的同源比较就会通过 ——
/// 所以还要先证明 authority 是网关本该服务的那些名字。pyengine 的等价实现是
/// `_request_authority_matches_config`。
///
/// R61 起 `pub(crate)`：通用侧车 REST 代理（[`crate::gateway::sidecar_proxy`]）复用同一口径，
/// 避免两套同源判定各自漂移。
pub(crate) fn origin_allowed(origin: Option<&str>, request_host: Option<&str>) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    if origin == "null" {
        return false;
    }
    let Some(origin_authority) = origin.strip_prefix("http://") else {
        return false; // 本网关只有明文回环 http，https/其它 scheme 一律不同源
    };
    if origin_authority.contains('/') || origin_authority.contains('?') || origin_authority.contains('#') {
        return false; // 带路径/查询/片段的不是合法的浏览器 Origin
    }
    let Some((origin_host, origin_port)) = split_authority(origin_authority) else {
        return false;
    };
    let Some((request_host_name, request_port)) = request_host.and_then(split_authority) else {
        return false;
    };
    is_loopback_hostname(request_host_name)
        && normalized_hostname(origin_host) == normalized_hostname(request_host_name)
        && effective_port(origin_port) == effective_port(request_port)
}

/// 拆 `host[:port]`；IPv6 字面量（`[::1]:8080`）按括号剥离。
fn split_authority(authority: &str) -> Option<(&str, Option<u16>)> {
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        let port = match tail.strip_prefix(':') {
            Some(p) => Some(p.parse().ok()?),
            None if tail.is_empty() => None,
            None => return None,
        };
        return Some((host, port));
    }
    match authority.split_once(':') {
        Some((host, port)) => {
            if host.is_empty() {
                return None;
            }
            Some((host, Some(port.parse().ok()?)))
        }
        None => Some((authority, None)),
    }
}

fn effective_port(port: Option<u16>) -> u16 {
    port.unwrap_or(80) // 网关只有 http
}

fn normalized_hostname(host: &str) -> String {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

/// 回环判定：`localhost` / `*.localhost` / 回环 IP 字面量（含 `::1`）。
fn is_loopback_hostname(host: &str) -> bool {
    let normalized = normalized_hostname(host);
    if normalized == "localhost" || normalized.ends_with(".localhost") {
        return true;
    }
    normalized
        .parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P2-4：离线计数只统计「未裁决且未消费」的真实队列行，文件缺失按未知处理。
    #[test]
    fn offline_count_reads_unresolved_rows_from_sidecar_queue() {
        let dir = std::env::temp_dir().join(format!(
            "marguerite-approval-count-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("approval_queue.sqlite");

        // 文件不存在 → None（不是 0，调用方自己决定回退）
        assert_eq!(count_pending_in_queue(&db), None);

        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE approval_queue (
                approval_id TEXT PRIMARY KEY,
                namespace TEXT NOT NULL,
                params TEXT NOT NULL,
                created_at REAL NOT NULL,
                resolved INTEGER NOT NULL DEFAULT 0,
                approved INTEGER NOT NULL DEFAULT 0,
                consumed INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO approval_queue (approval_id, namespace, params, created_at, resolved, consumed)
            VALUES ('a', 'exec', '{}', 1, 0, 0),
                   ('b', 'exec', '{}', 2, 0, 0),
                   ('c', 'exec', '{}', 3, 1, 1);",
        )
        .unwrap();
        drop(conn);

        assert_eq!(count_pending_in_queue(&db), Some(2));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn origin_guard_accepts_loopback_same_origin_only() {
        // 控制台自身的同源请求（webui 在 http://127.0.0.1:<port>/control/）
        assert!(origin_allowed(
            Some("http://127.0.0.1:18801"),
            Some("127.0.0.1:18801")
        ));
        assert!(origin_allowed(
            Some("http://localhost:18801"),
            Some("localhost:18801")
        ));
        // 无 Origin：curl / 桌面客户端
        assert!(origin_allowed(None, Some("127.0.0.1:18801")));

        // 跨源（恶意页面）→ 拒
        assert!(!origin_allowed(
            Some("http://evil.example"),
            Some("127.0.0.1:18801")
        ));
        // 端口不同 → 不同源
        assert!(!origin_allowed(
            Some("http://127.0.0.1:5173"),
            Some("127.0.0.1:18801")
        ));
        // 不透明源
        assert!(!origin_allowed(Some("null"), Some("127.0.0.1:18801")));
        // https 页面向明文回环发写请求 → 拒
        assert!(!origin_allowed(
            Some("https://127.0.0.1:18801"),
            Some("127.0.0.1:18801")
        ));
        // DNS rebinding：Host 与 Origin 同为攻击者域名 → authority 非回环，拒
        assert!(!origin_allowed(
            Some("http://rebind.example:18801"),
            Some("rebind.example:18801")
        ));
        // 畸形 Origin（带路径/用户信息）→ 拒
        assert!(!origin_allowed(
            Some("http://127.0.0.1:18801/control/"),
            Some("127.0.0.1:18801")
        ));
        assert!(!origin_allowed(
            Some("http://user@127.0.0.1:18801"),
            Some("127.0.0.1:18801")
        ));
    }

    /// 状态查询必须由**持有方**作答：本进程注册表里的挂起项回 found/pending，
    /// 其余一律 `None`（调用方去问侧车），越界命名空间也不作答。
    #[test]
    fn registry_status_answers_only_for_live_local_exec_approvals() {
        let reg = ApprovalRegistry::new();
        let gate = std::sync::Arc::new(crate::guard::gateway::ApprovalGate::new(
            Duration::from_secs(60),
        ));
        let pending = gate
            .request("fs_delete", "{\"path\":\"x\"}".into(), RiskLevel::High)
            .expect("首次挂起应成功");
        reg.register(gate, &pending);

        let status = registry_status(&reg, "exec.approval.status", &json!({ "id": pending.id }))
            .expect("本进程持有的挂起审批必须作答");
        assert_eq!(status["found"], true, "{status}");
        assert_eq!(status["id"], pending.id, "{status}");
        assert_eq!(status["namespace"], "exec", "{status}");
        assert_eq!(status["pending"], true, "{status}");
        assert_eq!(status["resolved"], false, "{status}");
        assert_eq!(status["resolutionInProgress"], false, "{status}");
        assert!(status["deadline"].is_null(), "没有墙上时钟截止时间就不要编：{status}");

        // 不在本进程 / 空 id / 缺 id / 跨命名空间 → None（交给调用方转发侧车）
        assert!(registry_status(&reg, "exec.approval.status", &json!({ "id": "ap_missing" })).is_none());
        assert!(registry_status(&reg, "exec.approval.status", &json!({ "id": "  " })).is_none());
        assert!(registry_status(&reg, "exec.approval.status", &json!({})).is_none());
        assert!(
            registry_status(&reg, "plugin.approval.status", &json!({ "id": pending.id })).is_none(),
            "Rust 注册表没有 plugin 队列，不得跨命名空间作答"
        );
    }

    /// 已超时的挂起项不得再被答成「仍挂起」：gate 的语义是 fail-closed 拒绝，卡片该收起。
    #[test]
    fn registry_status_ignores_timed_out_pending() {
        let reg = ApprovalRegistry::new();
        let gate = std::sync::Arc::new(crate::guard::gateway::ApprovalGate::new(
            Duration::from_millis(1),
        ));
        let pending = gate
            .request("exec", "{}".into(), RiskLevel::High)
            .expect("首次挂起应成功");
        reg.register(gate, &pending);
        std::thread::sleep(Duration::from_millis(20));
        assert!(registry_status(&reg, "exec.approval.status", &json!({ "id": pending.id })).is_none());
    }

    /// 缺省结果与 pyengine `approval_lookup_status_rpc_payload` 的 missing 分支逐字段同形。
    #[test]
    fn missing_status_matches_the_pyengine_lookup_shape() {
        assert_eq!(
            missing_status("plugin.approval.status", &json!({ "id": "x1" })),
            json!({
                "found": false,
                "id": "x1",
                "namespace": "plugin",
                "pending": false,
                "resolutionInProgress": false,
                "resolved": false,
            })
        );
        assert_eq!(missing_status("exec.approval.status", &json!({}))["id"], "");
    }

    /// 路线级回归（含 methods.rs 的拦截注册点）：Rust 抬起的审批，状态查询必须由本进程作答。
    ///
    /// 修复前 `exec.approval.status` 被 `PY_ENGINE_PREFIXES` 代理到侧车：离线回
    /// `NOT_IMPLEMENTED`、在线回 `found:false` —— 前端把 `found:false` 判成终态
    /// `unavailable`，重连恢复时仍挂起的卡片自己折叠，用户再也点不了「允许」。
    #[tokio::test]
    async fn dispatch_answers_status_for_rust_raised_approval() {
        use crate::guard::gateway::ApprovalGate;
        use crate::gateway::methods;

        let gate = std::sync::Arc::new(ApprovalGate::new(Duration::from_secs(60)));
        let pending = gate
            .request("fs_delete", "{\"path\":\"x\"}".into(), RiskLevel::High)
            .expect("首次挂起应成功");
        global_registry().register(gate, &pending);

        let ctx = methods::GatewayContext {
            config: crate::persistence::config::Config::default(),
            token: "t".into(),
            started_at: std::time::Instant::now(),
            bus: std::sync::Arc::new(crate::events::EventBus::default()),
        };
        let status = methods::dispatch(&ctx, "exec.approval.status", json!({ "id": pending.id }))
            .await
            .expect("状态查询必须被 Rust 网关服务（侧车在单测里未注册）");
        assert_eq!(status["found"], true, "{status}");
        assert_eq!(status["pending"], true, "{status}");
        assert_eq!(status["namespace"], "exec", "{status}");

        // 两侧都无 → 如实回缺省，而不是 NOT_IMPLEMENTED
        let unknown = methods::dispatch(&ctx, "exec.approval.status", json!({ "id": "ap_missing-x" }))
            .await
            .expect("两侧皆无时必须如实回缺省");
        assert_eq!(unknown["found"], false, "{unknown}");
        // plugin 命名空间不得越界作答
        let foreign = methods::dispatch(&ctx, "plugin.approval.status", json!({ "id": pending.id }))
            .await
            .expect("plugin 状态查询不得报 NOT_IMPLEMENTED");
        assert_eq!(foreign["found"], false, "{foreign}");
        assert_eq!(foreign["namespace"], "plugin", "{foreign}");

        global_registry().unregister(&pending.id);
    }

    #[test]
    fn pending_item_maps_only_real_fields() {
        let p = PendingApproval {
            id: "ap_test".to_string(),
            tool: "shell".to_string(),
            args: r#"{"command":"rm -rf build"}"#.to_string(),
            risk: RiskLevel::High,
            origin: Default::default(),
            created_at: std::time::Instant::now(),
        };
        let item = pending_item(&p);
        assert_eq!(item["id"], "ap_test");
        assert_eq!(item["namespace"], "exec");
        assert_eq!(item["toolName"], "shell");
        assert_eq!(item["command"], "rm -rf build");
        assert_eq!(item["args"]["command"], "rm -rf build");
        assert_eq!(item["destructive"], true);
        // 没把握的字段不许凭空出现（UI 会照单渲染）
        assert!(item.get("sessionKey").is_none());
        assert!(item.get("warning").is_none());
        assert!(item.get("backupState").is_none());
    }

    /// 审批参数进入浏览器之前必须脱敏：凭据、复核指纹都不许出网关。
    ///
    /// 变异实测：删掉 `display_value`（恢复整包透传）→ 本用例在
    /// `assert!(!serialized.contains("sk-live"))` 处判红。
    #[test]
    fn pending_item_never_leaks_secrets_or_review_metadata() {
        let p = PendingApproval {
            id: "ap_leak".to_string(),
            tool: "shell".to_string(),
            args: serde_json::json!({
                "command": "curl -H \"Authorization: Bearer sk-live-ABCDEFGHIJKLMNOPQRSTUV\" https://api.example/x",
                "apiKey": "sk-proj-SHOULD-NOT-ESCAPE-0123456789",
                "accessToken": "plain-opaque-token-value",
                "path": "/tmp/report.txt",
                "fingerprint": "9f2c",
                "reviewNotes": "internal reviewer said ok",
                "nested": { "password": "hunter2", "keep": 7 },
            })
            .to_string(),
            risk: RiskLevel::High,
            origin: Default::default(),
            created_at: std::time::Instant::now(),
        };
        let item = pending_item(&p);
        let serialized = item.to_string();

        assert!(!serialized.contains("sk-live"), "Authorization 明文泄漏: {serialized}");
        assert!(!serialized.contains("SHOULD-NOT-ESCAPE"), "敏感键未抹: {serialized}");
        assert!(!serialized.contains("plain-opaque-token-value"), "驼峰敏感键未抹: {serialized}");
        assert!(!serialized.contains("hunter2"), "嵌套敏感键未抹: {serialized}");
        assert!(!serialized.contains("internal reviewer"), "review 元数据不应进展示面");
        assert!(!serialized.contains("9f2c"), "复核指纹不应进展示面");

        // 展示需要的非敏感内容必须原样保留（否则卡片退化成空白）
        assert_eq!(item["args"]["path"], "/tmp/report.txt");
        assert_eq!(item["args"]["nested"]["keep"], 7);
        assert!(item["command"].as_str().unwrap_or("").contains("curl"));
    }

    /// 无法解析的 `args` 走 `raw` 分支，同样要脱敏。
    #[test]
    fn pending_item_redacts_unparsable_args() {
        let p = PendingApproval {
            id: "ap_raw".to_string(),
            tool: "shell".to_string(),
            args: "opaque sk-ant-abcdefghijklmnopqrstuvwxyz012345 trailing".to_string(),
            risk: RiskLevel::Low,
            origin: Default::default(),
            created_at: std::time::Instant::now(),
        };
        let item = pending_item(&p);
        let raw = item["args"]["raw"].as_str().unwrap_or("");
        assert!(!raw.contains("sk-ant-"), "raw 分支未脱敏: {raw}");
        assert!(raw.contains("[redacted]"), "应保留脱敏占位: {raw}");
    }

    #[test]
    fn merge_pending_keeps_both_queues_without_duplicating_ids() {
        let local = vec![json!({ "id": "ap_1", "toolName": "shell" })];
        let remote = vec![
            json!({ "id": "83cad65fa891", "toolName": "shell" }),
            json!({ "id": "ap_1", "toolName": "shell" }),
        ];
        let merged = merge_pending(local, &remote);
        let ids: Vec<&str> = merged
            .iter()
            .filter_map(|v| v.get("id").and_then(Value::as_str))
            .collect();
        assert_eq!(ids, vec!["ap_1", "83cad65fa891"], "两侧都要在，且同 id 不得重复");
    }

    /// 侧车不可用时：转发失败 → 回本进程的 404（不能假装成功，也不能把界面挂住）。
    #[tokio::test]
    async fn forward_resolve_returns_none_when_sidecar_is_unreachable() {
        // 没有侧车注册（`py_sidecar()` 未初始化）→ 直接 None
        let state = crate::gateway::server::GatewayState {
            config: crate::persistence::config::Config::default(),
            bus: crate::events::EventBus::default(),
            token: "t".to_string(),
            started_at: std::time::Instant::now(),
            webui_dir: std::env::temp_dir(),
            rate_limiter: std::sync::Arc::new(crate::gateway::rate_limit::RateLimiter::new(
                crate::gateway::rate_limit::BucketConfig::new(240, 40.0),
                crate::gateway::rate_limit::BucketConfig::new(240, 40.0),
            )),
            rate_limit_enabled: false,
        };
        assert!(
            forward_resolve(&state, &Bytes::from_static(b"{}")).await.is_none(),
            "侧车不在线时必须回 None，由调用方给 404"
        );
        assert!(sidecar_snapshot(&state).await.is_none());
    }
}
