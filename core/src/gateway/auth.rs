//! 网关本地认证：token 生成 / 持久化 / 校验
//!
//! 网关仅监听回环 127.0.0.1；token 用于防止本机其他进程/网页意外驱动
//! 引擎。token 存于 `~/.marguerite/gateway-token`，首次启动生成。

use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use rand::RngCore;

/// token 文件相对数据目录的路径
const TOKEN_FILE: &str = "gateway-token";

/// 生成新的本地 token（32 字节随机，base64url 编码）
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn token_path(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join(TOKEN_FILE)
}

/// 加载或创建 token（数据目录不存在则先创建）
pub fn load_or_create_token(data_dir: &std::path::Path) -> Result<String> {
    fs::create_dir_all(data_dir)?;
    let path = token_path(data_dir);
    if let Ok(raw) = fs::read_to_string(&path) {
        let t = raw.trim();
        if !t.is_empty() {
            return Ok(t.to_string());
        }
    }
    let token = generate_token();
    // R13：原子写。此处虽是短文本，但 `fs::write` 先截断——若在截断与写入之间
    // 崩溃，下次启动读到空 token 会**重新生成**，所有已连接的客户端 token 集体失效
    // （用户看到的是「刚打开就要求重新配对」）。
    crate::persistence::write_json_atomic(&path, &token)?;
    Ok(token)
}

/// 校验提供的 token 是否与服务端一致（恒定时间比较，避免时序侧信道）
pub fn verify(token: &str, expected: &str) -> bool {
    if token.len() != expected.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in token.bytes().zip(expected.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// REST 面的本地 token 请求头（与 WS 的 `connect.params.token` 同源）。
pub const TOKEN_HEADER: &str = "x-marguerite-token";

/// 从 REST 请求中提取 token：优先 `x-marguerite-token`，其次 `Authorization: Bearer`，
/// 最后允许 `?token=`（仅用于无法自定义请求头的同源下载/iframe 场景）。
///
/// 为什么不接受 Cookie：前端已统一用 Authorization 头；引入 Cookie 会扩大 CSRF 面，
/// 且 WebView 与浏览器对 SameSite 的行为不一致，难以保证「四端一致」。
pub fn token_from_request(
    headers: &axum::http::HeaderMap,
    uri: Option<&axum::http::Uri>,
) -> Option<String> {
    if let Some(raw) = headers.get(TOKEN_HEADER).and_then(|v| v.to_str().ok()) {
        let t = raw.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    if let Some(raw) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        let raw = raw.trim();
        if let Some(t) = raw.strip_prefix("Bearer ").or_else(|| raw.strip_prefix("bearer ")) {
            let t = t.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    if let Some(query) = uri.and_then(|u| u.query()) {
        for pair in query.split('&') {
            if let Some(v) = pair.strip_prefix("token=") {
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// REST 是否已授权（token 缺失/错误一律拒绝，fail-closed）。
pub fn rest_authorized(
    headers: &axum::http::HeaderMap,
    uri: Option<&axum::http::Uri>,
    expected: &str,
) -> bool {
    token_from_request(headers, uri)
        .map(|t| verify(&t, expected))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_token_is_urlsafe_and_unique() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 43, "32 字节 base64url 应为 43 字符");
        assert_ne!(a, b);
        assert!(!a.contains('+') && !a.contains('/') && !a.contains('='));
    }

    #[test]
    fn test_verify_constant_time_equality() {
        assert!(verify("abc", "abc"));
        assert!(!verify("abc", "abd"));
        assert!(!verify("abc", "abcd"));
        assert!(!verify("", "x"));
    }

    #[test]
    fn test_load_or_create_token_roundtrip() {
        let dir = std::env::temp_dir().join(format!("marguerite-gw-test-{}", uuid::Uuid::new_v4()));
        let t1 = load_or_create_token(&dir).unwrap();
        let t2 = load_or_create_token(&dir).unwrap();
        assert_eq!(t1, t2, "二次加载应复用已落盘 token");
        assert!(verify(&t1, &t2));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_rest_token_sources_and_fail_closed() {
        use axum::http::{HeaderMap, HeaderValue, Uri};

        let mut h = HeaderMap::new();
        h.insert(TOKEN_HEADER, HeaderValue::from_static("abc"));
        assert_eq!(token_from_request(&h, None).as_deref(), Some("abc"));
        assert!(rest_authorized(&h, None, "abc"));
        assert!(!rest_authorized(&h, None, "xyz"));

        let mut h2 = HeaderMap::new();
        h2.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer e2e-test-token"),
        );
        assert!(rest_authorized(&h2, None, "e2e-test-token"));

        let uri: Uri = "/api/approvals?x=1&token=e2e-test-token".parse().unwrap();
        let empty = HeaderMap::new();
        assert!(rest_authorized(&empty, Some(&uri), "e2e-test-token"));

        // 缺失/错误一律拒绝（fail-closed）
        assert!(!rest_authorized(&HeaderMap::new(), None, "e2e-test-token"));
        assert!(!rest_authorized(
            &HeaderMap::new(),
            Some(&"/api/approvals?token=wrong".parse::<Uri>().unwrap()),
            "e2e-test-token"
        ));
    }
}
