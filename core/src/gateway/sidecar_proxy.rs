//! 通用侧车 REST 代理：Rust 网关未实现的 `/api/*` 原样转给 Python 侧车。
//!
//! # 为什么必须做（R61）
//!
//! 侧车（`pyengine/`）的 REST 面（`/api/v1/files/upload`、`/api/v1/attachments/*`、
//! `/api/config`、`/api/sessions`、`/api/usage`、`/api/audio/transcribe`、
//! `/api/v1/diagnostics/bundle`、`/api/v1/artifact-preview-leases/*` …）在**纯 Rust 托管**
//! 的控制台下**整体不可达**（R56 实测：一律 501），此前只按「UI 真用哪条」逐个补齐
//! （R54 补 `/api/approvals*`、R56 补 `/api/v1/artifacts/{id}`）。漏网的后果是**功能坏掉**：
//!
//! - 附件上传是 **REST-only**（`webui/src/composables/chat/useChatAttachments.ts`
//!   → `POST /api/v1/files/upload`）→ 拖文件进输入框必然失败，`chat.send` 拿不到 `file_uuid`；
//! - Workbench 产物预览租约（`artifactPreviewLease.ts`）、支持诊断包
//!   （`SupportDiagnosticsMenu.vue`）、语音输入（`useVoiceInput.ts`）同样只有 REST 一条路。
//!
//! 逐端点补的实现方式必然继续漏（每新增一个侧车端点就再坏一次），所以这里改成**按面**兜底：
//! 凡是 Rust 没有更具体路由的 `/api/*`，侧车在线就转发，离线维持 501 的既有契约。
//!
//! # 契约与安全
//!
//! - **诚实**：侧车不在（未启动/探测失败）→ 沿用 [`crate::gateway::server::api_unimplemented`]
//!   的 501 JSON，前端照旧能识别「本部署不支持」并降级；侧车在线但连不上/超时 → 502
//!   `sidecar_unreachable`，**不假装成功**。
//! - **同源**：口径与 `/api/approvals` 完全一致（[`crate::gateway::approvals::origin_allowed`]），
//!   挡浏览器跨源 CSRF 与 DNS rebinding；无 `Origin`（curl / 桌面 node 客户端）放行。
//! - **凭据**：客户端的 `authorization` **不转发**（只带网关自己的共享 token），`cookie` 原样转发
//!   （同源凭据，控制台的产物预览租约依赖它）。
//! - **逐跳头**（`connection` / `transfer-encoding` / `upgrade` …）两个方向都不转发，
//!   其余自定义头（`x-Marguerite-session-key` 等）原样透传 —— 侧车的会话键契约见
//!   `pyengine/src/opensquilla/gateway/session_key.py`。
//! - **流式**：请求体与响应体都是 `Stream`（大附件上传/下载不整块进内存）。
//! - 路由优先级：更具体的路由（`/api/health`、`/api/approvals*`、`/api/v1/artifacts/{id}`）
//!   在 axum 里优先于本模块的通配 `/api/{*path}`，所以它们仍由 Rust 直接服务。

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderName, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::gateway::approvals::{origin_allowed, sidecar_client};
use crate::gateway::server::{api_unimplemented, py_sidecar, GatewayState};

/// 不转发的请求头：逐跳头 + 由转发方自己决定/承载的头。
///
/// - `host`：目标（侧车）自己的 authority，照搬会破坏侧车的同源判定；
/// - `authorization`：客户端的凭据与侧车无关，网关改用共享 token 认证（[`forward`]）；
/// - `content-length`：请求体是流，长度由 reqwest 自己算（照搬会与流不一致）。
pub(crate) fn forward_request_header(name: &HeaderName) -> bool {
    !matches!(
        name.as_str(),
        "host"
            | "authorization"
            | "content-length"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "expect"
    )
}

/// 不转发的响应头：逐跳头（帧定界由本网关的 hyper 自己决定）。
///
/// `content-type` / `content-encoding` / `content-disposition` / `set-cookie` / `x-*`
/// 一律**如实透传** —— 少了 `content-encoding` 会把压缩体当明文给浏览器，少了
/// `content-disposition` 会让「下载」变成「预览」。
pub(crate) fn forward_response_header(name: &HeaderName) -> bool {
    !matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

/// `/api/{*path}` 的处理器：侧车在线 → 转发；离线 → 501（既有契约）。
pub(crate) async fn api_proxy(State(state): State<Arc<GatewayState>>, req: Request) -> Response {
    if !origin_allowed(
        req.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok()),
        req.headers().get(header::HOST).and_then(|v| v.to_str().ok()),
    ) {
        return forbidden_origin();
    }
    let Some(base) = sidecar_base() else {
        return api_unimplemented().await.into_response();
    };
    forward(&base, &state.token, req).await
}

/// `GET` 兜底（供已被 Rust 路由抢占的端点用，如 `/api/v1/artifacts/{id}` 的台账未命中）。
///
/// 返回 `None` = 侧车离线（调用方保持自己的 404/501 语义，不假装成功）。
pub(crate) async fn forward_get(
    state: &GatewayState,
    uri: &Uri,
    headers: &HeaderMap,
) -> Option<Response> {
    let base = sidecar_base()?;
    let mut builder = sidecar_client()
        .get(target_url(&base, uri))
        .bearer_auth(&state.token);
    for (name, value) in headers.iter() {
        if forward_request_header(name) {
            builder = builder.header(name, value);
        }
    }
    let resp = builder.send().await.ok()?;
    Some(relay(resp))
}

/// 侧车基址（未启动 = `None`）。
fn sidecar_base() -> Option<String> {
    Some(py_sidecar()?.base_url())
}

/// 侧车在线时把请求原样转发（方法 / 路径 / 查询 / 体 / 头），并把响应如实回给调用方。
pub(crate) async fn forward(base: &str, token: &str, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let mut builder = sidecar_client()
        .request(parts.method.clone(), target_url(base, &parts.uri))
        .bearer_auth(token);
    for (name, value) in parts.headers.iter() {
        if forward_request_header(name) {
            builder = builder.header(name, value);
        }
    }
    match builder
        .body(reqwest::Body::wrap_stream(body.into_data_stream()))
        .send()
        .await
    {
        Ok(resp) => relay(resp),
        Err(err) => sidecar_unreachable(&error_chain(&err)),
    }
}

/// 把错误整条 source 链拼成一行（`Display` 只给顶层，真实原因被吞掉后无从排查）。
fn error_chain(err: &dyn std::error::Error) -> String {
    let mut parts = vec![err.to_string()];
    let mut cursor = err.source();
    while let Some(source) = cursor {
        parts.push(source.to_string());
        cursor = source.source();
    }
    parts.join(" <- ")
}

/// `base` + `path_and_query`（基址尾部斜杠归一，查询串逐字保留）。
fn target_url(base: &str, uri: &Uri) -> String {
    let base = base.trim_end_matches('/');
    match uri.path_and_query().map(|pq| pq.as_str()) {
        Some(pq) => format!("{base}{pq}"),
        None => format!("{base}{}", uri.path()),
    }
}

/// 把侧车响应转成本网关响应：状态码与允许的头逐条照搬，体是流（不整块读）。
fn relay(resp: reqwest::Response) -> Response {
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    for (name, value) in resp.headers().iter() {
        if forward_response_header(name) {
            builder = builder.header(name, value);
        }
    }
    builder
        .body(Body::from_stream(resp.bytes_stream()))
        .unwrap_or_else(|_| sidecar_unreachable("上游响应头非法"))
}

/// 侧车在线但请求失败（连接被拒 / 超时 / 协议错）：502 + 可识别错误码，绝不静默降级成成功。
fn sidecar_unreachable(reason: &str) -> Response {
    (
        StatusCode::BAD_GATEWAY,
        axum::Json(json!({
            "ok": false,
            "error": "sidecar_unreachable",
            "message": format!("Python 侧车不可用，该 REST 端点暂时无法服务：{reason}"),
            "retryable": true,
        })),
    )
        .into_response()
}

/// 跨源（浏览器 CSRF / DNS rebinding）：载荷与 pyengine `forbidden_origin_response()` 同形。
fn forbidden_origin() -> Response {
    (
        StatusCode::FORBIDDEN,
        axum::Json(json!({
            "error": "cross-origin requests are not allowed",
            "code": "FORBIDDEN_ORIGIN",
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::any;
    use std::time::Instant;

    use crate::events::bus::EventBus;
    use crate::gateway::server::build_router;
    use crate::persistence::config::Config;

    // ------------------------------------------------------------------
    // 头过滤（逐跳头两个方向都不转发；业务头必须透传）
    // ------------------------------------------------------------------

    #[test]
    fn request_headers_drop_hop_by_hop_and_credentials() {
        for dropped in [
            "host",
            "authorization",
            "content-length",
            "connection",
            "transfer-encoding",
            "upgrade",
            "te",
            "trailer",
        ] {
            let name = HeaderName::from_bytes(dropped.as_bytes()).unwrap();
            assert!(
                !forward_request_header(&name),
                "`{dropped}` 不得转发给侧车（否则破坏侧车 authority / 凭据 / 帧定界）"
            );
        }
        for kept in [
            "content-type",
            "accept",
            "origin",
            "referer",
            "cookie",
            "x-marguerite-session-key",
            "x-opensquilla-session-key",
            "user-agent",
            "accept-language",
        ] {
            let name = HeaderName::from_bytes(kept.as_bytes()).unwrap();
            assert!(
                forward_request_header(&name),
                "`{kept}` 必须透传（业务头 / 会话键 / 内容类型）"
            );
        }
    }

    #[test]
    fn response_headers_keep_encoding_and_disposition() {
        for kept in [
            "content-type",
            "content-encoding",
            "content-disposition",
            "content-length",
            "cache-control",
            "etag",
            "last-modified",
            "accept-ranges",
            "set-cookie",
            "x-custom",
        ] {
            let name = HeaderName::from_bytes(kept.as_bytes()).unwrap();
            assert!(
                forward_response_header(&name),
                "`{kept}` 必须如实透传（压缩体 / 下载语义 / 缓存协商）"
            );
        }
        for dropped in ["transfer-encoding", "connection", "upgrade", "trailer"] {
            let name = HeaderName::from_bytes(dropped.as_bytes()).unwrap();
            assert!(!forward_response_header(&name), "`{dropped}` 是逐跳头");
        }
    }

    #[test]
    fn target_url_preserves_query_and_normalizes_base() {
        let uri: Uri = "/api/v1/attachments/a?sessionKey=k&variant=download"
            .parse()
            .unwrap();
        assert_eq!(
            target_url("http://127.0.0.1:9999/", &uri),
            "http://127.0.0.1:9999/api/v1/attachments/a?sessionKey=k&variant=download"
        );
        assert_eq!(
            target_url("http://127.0.0.1:9999", &uri),
            "http://127.0.0.1:9999/api/v1/attachments/a?sessionKey=k&variant=download"
        );
    }

    // ------------------------------------------------------------------
    // 真实 HTTP 往返：stub 侧车回声（方法 / 路径 / 查询 / 头 / 体）
    // ------------------------------------------------------------------

    /// stub 侧车：把观测到的请求如实回给调用方（状态 201 + `x-echo-*` 头 + 逐字节回显体）。
    async fn spawn_stub_sidecar() -> std::net::SocketAddr {
        async fn echo(req: Request) -> Response {
            let (parts, body) = req.into_parts();
            let auth = parts
                .headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("<none>")
                .to_string();
            let cookie = parts
                .headers
                .get(header::COOKIE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("<none>")
                .to_string();
            let host = parts
                .headers
                .get(header::HOST)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("<none>")
                .to_string();
            let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap_or_default();
            let body_len = bytes.len();
            Response::builder()
                .status(StatusCode::CREATED)
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .header("x-echo-method", parts.method.as_str())
                .header("x-echo-uri", parts.uri.to_string())
                .header("x-echo-auth", auth)
                .header("x-echo-cookie", cookie)
                .header("x-echo-host", host)
                // 逐跳头必须被网关拦掉：故意在响应里加一个，测试断言客户端看不到。
                // （用 `keep-alive` 而不是 `transfer-encoding`：后者会被 hyper 当成帧定界声明，
                //  人为制造非法响应，测的就不是转发了。）
                .header("keep-alive", "timeout=5")
                .body(Body::from(bytes))
                .unwrap()
                .into_response()
                .tap_len(body_len)
        }
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, axum::Router::new().route("/{*path}", any(echo)))
                .await
                .unwrap();
        });
        addr
    }

    /// 给响应补一个可断言的 `content-length`（stub 观测到的体长）。
    trait TapLen {
        fn tap_len(self, len: usize) -> Response;
    }

    impl TapLen for Response {
        fn tap_len(mut self, len: usize) -> Response {
            self.headers_mut().insert(
                HeaderName::from_static("x-echo-body-len"),
                axum::http::HeaderValue::from_str(&len.to_string()).unwrap(),
            );
            self
        }
    }

    /// 用 reqwest 打真实 `forward()`（不经路由），验证：
    /// ① 方法/路径/查询逐字保留 ② 客户端的 `authorization` 被替换成网关共享 token
    /// ③ 业务头（cookie / session key）透传 ④ 大体积请求体逐字节往返（流式不截断）
    /// ⑤ 侧车的逐跳响应头被拦掉、业务响应头保留 ⑥ 状态码如实
    #[tokio::test]
    async fn forward_relays_method_path_query_headers_and_streams_bodies() {
        let addr = spawn_stub_sidecar().await;
        let base = format!("http://{addr}");
        let payload = vec![b'x'; 300 * 1024];

        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/files/upload?sessionKey=sess-1&variant=download")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer client-secret-must-not-leak")
            .header(header::COOKIE, "lease=abc")
            .header("x-marguerite-session-key", "sess-1")
            .body(Body::from(payload.clone()))
            .unwrap();

        let resp = forward(&base, "shared-gateway-token", req).await;
        if resp.status() != StatusCode::CREATED {
            let status = resp.status();
            let headers = resp.headers().clone();
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap_or_default();
            panic!(
                "上游状态码必须如实透传，实际 {status}；headers={headers:?}；body={}",
                String::from_utf8_lossy(&body)
            );
        }

        let headers = resp.headers().clone();
        assert_eq!(headers.get("x-echo-method").unwrap(), "POST");
        assert_eq!(
            headers.get("x-echo-uri").unwrap(),
            "/api/v1/files/upload?sessionKey=sess-1&variant=download",
            "路径与查询串必须逐字保留"
        );
        assert_eq!(
            headers.get("x-echo-auth").unwrap(),
            "Bearer shared-gateway-token",
            "必须改带网关共享 token"
        );
        assert!(
            !headers
                .get("x-echo-auth")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("client-secret"),
            "客户端凭据绝不能漏给侧车"
        );
        assert_eq!(headers.get("x-echo-cookie").unwrap(), "lease=abc");
        assert_eq!(headers.get("x-echo-host").unwrap().to_str().unwrap(), addr.to_string());
        assert!(
            headers.get("keep-alive").is_none(),
            "逐跳响应头不得转发给调用方"
        );
        assert_eq!(
            headers.get(header::CONTENT_TYPE).unwrap(),
            "application/octet-stream",
            "业务响应头必须保留"
        );
        assert_eq!(
            headers.get("x-echo-body-len").unwrap().to_str().unwrap(),
            payload.len().to_string(),
            "300KB 请求体必须完整到达侧车（流式不截断）"
        );

        let echoed = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(echoed.len(), payload.len(), "响应体长度必须逐字节一致");
        assert_eq!(echoed.first(), Some(&b'x'));
        assert_eq!(echoed.last(), Some(&b'x'));
    }

    /// 侧车不可达（连接被拒）→ 502 `sidecar_unreachable`（可重试），**不假装成功**。
    ///
    /// 这条同时是**系统代理劫持的守卫**：本机 `ProxyEnable=1 / ProxyServer=127.0.0.1:7890`
    /// （iKuuuVPN/clash）会替回环目标回一个 `502` + 空体，于是「侧车离线」与
    /// 「代理拒绝了转发」在响应上无法区分。`sidecar_client()` 的 `.no_proxy()` 一旦被摘掉，
    /// 本用例立刻判红（实测：摘掉后响应体变成空串，而不是本模块的结构化 JSON），
    /// 因为那时回答请求的是代理而不是侧车。
    #[tokio::test]
    async fn forward_reports_unreachable_sidecar_as_502() {
        // 先占一个端口再释放，得到一个「几乎必然没人监听」的地址（连接被拒）
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let req = Request::builder()
            .method("GET")
            .uri("/api/config")
            .body(Body::empty())
            .unwrap();
        let resp = forward(&format!("http://{addr}"), "t", req).await;
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("\"error\":\"sidecar_unreachable\""),
            "必须给可识别错误码，实际: {text}"
        );
    }

    // ------------------------------------------------------------------
    // 路由层（真实 HTTP）：侧车离线 → 501 契约不变；跨源 → 403
    // ------------------------------------------------------------------

    fn proxy_test_state() -> Arc<GatewayState> {
        Arc::new(GatewayState {
            config: Config::default(),
            bus: EventBus::default(),
            token: "proxy-probe-token".to_string(),
            started_at: Instant::now(),
            webui_dir: std::env::temp_dir(),
        })
    }

    /// 侧车离线（`py_sidecar()` 未注册）时行为**必须**与代理上线前一致：
    /// 501 + `not_implemented` JSON，未被通配路由改写、也没落进 SPA HTML 回退。
    #[tokio::test]
    async fn router_keeps_501_contract_when_sidecar_is_offline() {
        let state = proxy_test_state();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, build_router(state)).await.unwrap();
        });

        for (method, path) in [
            ("GET", "/api/config"),
            ("GET", "/api/v1/attachments/deadbeef"),
            ("POST", "/api/v1/files/upload"),
        ] {
            let raw = raw_http(addr, method, path, "").await;
            assert!(raw.starts_with("HTTP/1.1 501"), "{method} {path} 应 501: {raw}");
            assert!(
                raw.contains("\"error\":\"not_implemented\""),
                "{method} {path} 载荷应为 not_implemented: {raw}"
            );
            assert!(
                !raw.contains("<!doctype html>"),
                "{method} {path} 不得落进 SPA 回退: {raw}"
            );
        }
    }

    /// 带跨源 `Origin` 的请求在代理前就被拒（403 FORBIDDEN_ORIGIN）：浏览器 CSRF 兜底，
    /// 且**不能**因为「转发给侧车」而绕过。
    #[tokio::test]
    async fn router_rejects_cross_origin_api_requests() {
        let state = proxy_test_state();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, build_router(state)).await.unwrap();
        });

        let raw = raw_http_with_headers(
            addr,
            "GET",
            "/api/config",
            &[("Origin", "http://evil.example")],
        )
        .await;
        assert!(raw.starts_with("HTTP/1.1 403"), "跨源应 403: {raw}");
        assert!(raw.contains("FORBIDDEN_ORIGIN"), "载荷应含错误码: {raw}");
    }

    async fn raw_http(
        addr: std::net::SocketAddr,
        method: &str,
        path: &str,
        _body: &str,
    ) -> String {
        raw_http_with_headers(addr, method, path, &[]).await
    }

    async fn raw_http_with_headers(
        addr: std::net::SocketAddr,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
    ) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut conn = tokio::net::TcpStream::connect(addr).await.unwrap();
        let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
        for (name, value) in headers {
            req.push_str(&format!("{name}: {value}\r\n"));
        }
        req.push_str("Content-Length: 0\r\n\r\n");
        conn.write_all(req.as_bytes()).await.unwrap();
        let mut raw = Vec::new();
        conn.read_to_end(&mut raw).await.unwrap();
        String::from_utf8_lossy(&raw).to_string()
    }
}