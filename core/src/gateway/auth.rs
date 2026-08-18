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
    fs::write(&path, &token)?;
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
}
