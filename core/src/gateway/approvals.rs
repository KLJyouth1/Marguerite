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
//! - `GET  /api/approvals`         → 200 `{pending, mode, allowPatterns, denyPatterns}`
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
use crate::guard::approval_registry::{global_registry, DecisionResult};
use crate::guard::gateway::{PendingApproval, RiskLevel};

/// 拉侧车快照的超时：控制台的 hydration 会被频繁触发（进入会话、可选的 2s 轮询），
/// 侧车启动期（Python 网关 boot 可长达 ~90s）不能把它拖住 —— 宁可只报本进程那半。
const SIDECAR_LIST_TIMEOUT: Duration = Duration::from_secs(2);

/// 转发裁决的超时：这是用户点「允许/拒绝」后的等待，比快照多给一点耐心，
/// 但仍必须有上限 —— 侧车卡死时界面要能落到明确错误，而不是无限转圈。
const SIDECAR_RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);

/// GET `/api/approvals`：当前挂起审批的快照。
///
/// `mode` / `allowPatterns` / `denyPatterns` 是 pyengine 快照的必备字段；Rust 引擎
/// **没有**审批模式存储（审批门恒为「逐个等人工裁决」），故如实回 `prompt` + 空模式表，
/// 而不是伪造一个看起来更像 Python 的策略。
///
/// 侧车在线时**合并**它的队列（见 `merge_pending`）：两边是两套独立队列，只报自己那半
/// 会让控制台漏掉侧车侧挂起的审批 —— 那些卡片推得到、裁决却打不通（R57 实测）。
pub(crate) async fn approvals_list(State(state): State<std::sync::Arc<GatewayState>>) -> Response {
    let mut pending: Vec<Value> = global_registry()
        .pending_snapshots()
        .iter()
        .map(pending_item)
        .collect();
    let mut mode = "prompt".to_string();
    let mut allow_patterns = json!([]);
    let mut deny_patterns = json!([]);

    if let Some(snapshot) = sidecar_snapshot(&state).await {
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

    let body = json!({
        "pending": pending,
        "mode": mode,
        "allowPatterns": allow_patterns,
        "denyPatterns": deny_patterns,
    });
    (StatusCode::OK, axum::Json(body)).into_response()
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
    body: Bytes,
) -> Response {
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
fn pending_item(p: &PendingApproval) -> Value {
    let args = serde_json::from_str::<Value>(&p.args).ok();
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
        "args": args.unwrap_or_else(|| json!({ "raw": p.args })),
        "destructive": p.risk == RiskLevel::High,
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
        };
        assert!(
            forward_resolve(&state, &Bytes::from_static(b"{}")).await.is_none(),
            "侧车不在线时必须回 None，由调用方给 404"
        );
        assert!(sidecar_snapshot(&state).await.is_none());
    }
}
