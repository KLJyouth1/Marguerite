//! `POST /api/elevated-mode`（P2-3）：会话级提权的 REST 入口。
//!
//! 与 pyengine `gateway/app.py::api_elevated_mode` 同形：
//! 请求 `{sessionKey, mode}`，`mode ∈ {off, on, bypass, full}`。
//!
//! 为什么 Rust 也要实现：聊天工具回路（`agent::tool_cycle`）跑在 Rust 里，侧车离线
//! （纯 Rust 托管）时若没有这条路由，请求会落到 `/api/{*path}` 代理并回 501，前端
//! 只能把模式回退成「不可用」——开关点了也白点（审计 P1-3 / P2-3）。
//!
//! 安全：这是**能关闭人工审批**的写接口，故与 `/api/approvals/resolve` 同口径：
//! 必须带本地网关 token，且拒绝跨源写请求（本机进程无 Origin 头，靠 token 兜底）。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

use crate::gateway::approvals::{origin_allowed, unauthorized};
use crate::gateway::server::GatewayState;

/// POST `/api/elevated-mode`
pub(crate) async fn elevated_mode(
    State(state): State<Arc<GatewayState>>,
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
        // 与 `/api/approvals/resolve` 同一载荷（跨源写请求的 CSRF 兜底）。
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
    let Some(session_key) = parsed
        .get("sessionKey")
        .or_else(|| parsed.get("session_key"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({ "error": "sessionKey is required" })),
        )
            .into_response();
    };
    let raw_mode = parsed.get("mode").and_then(Value::as_str).unwrap_or("off");
    let mode = match crate::guard::elevated::parse_mode(raw_mode) {
        Ok(mode) => mode,
        Err(message) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({ "error": message })),
            )
                .into_response();
        }
    };

    crate::guard::elevated::global().set(session_key, mode);

    (
        StatusCode::OK,
        axum::Json(json!({
            "sessionKey": session_key,
            "mode": mode.unwrap_or("off"),
        })),
    )
        .into_response()
}
