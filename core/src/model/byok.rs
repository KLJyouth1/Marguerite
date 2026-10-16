//! BYOK 密钥管理（Phase H3）— AES-256-GCM 加密 + 环境变量回退链 + 0600 落盘
//!
//! 设计原则（用户 2026-08-05 决议 D-03：本地模型仅 custom 接入点，不预内置）：
//! - API Key 默认**不落盘明文**；优先走环境变量 `MARGUERITE_API_KEY`（按档案 ID 优先）。
//! - 确需持久化时，用 AES-256-GCM 令牌（Fernet 风格：随机 12 字节 nonce + 密文，base64url）加密后存 `models.json`；
//!   主密钥单独落 `model.key`（0600）。
//! - 采用纯 Rust `aes-gcm`（RustCrypto），无 openssl/C 依赖，Windows MSVC 可直接编译。
//! - 解密失败（密钥不匹配 / token 损坏）明确报错，绝不静默回退为空。

use std::path::Path;

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::rngs::OsRng;
use rand::RngCore;

use aes_gcm::aead::Aead;
use aes_gcm::aead::KeyInit;
use aes_gcm::{Aes256Gcm, Key, Nonce};

/// 令牌前缀（用于判定一个字符串是否已是加密令牌）
const TOKEN_PREFIX: &str = "mg1:";
/// nonce 长度（AES-GCM 标准 96-bit）
const NONCE_LEN: usize = 12;
/// 主密钥长度（AES-256）
const KEY_LEN: usize = 32;

/// 主密钥文件路径（`~/.marguerite/model.key`，0600）
pub fn key_path() -> std::path::PathBuf {
    crate::persistence::config::Config::config_dir().join("model.key")
}

/// 生成新的主密钥（base64url 编码的 32 字节）
pub fn generate_key() -> String {
    let mut raw = [0u8; KEY_LEN];
    OsRng.fill_bytes(&mut raw);
    URL_SAFE_NO_PAD.encode(raw)
}

/// 用显式密钥加密（供测试与显式调用）。输出 `mg1:<base64url(nonce||ciphertext)>`
pub fn encrypt_with_key(plain: &str, key_b64: &str) -> Result<String> {
    if plain.is_empty() {
        return Ok(String::new());
    }
    let raw = decode_key(key_b64)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&raw));
    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plain.as_bytes())
        .map_err(|_| anyhow!("AES-GCM 加密失败"))?;
    let mut blob = Vec::with_capacity(NONCE_LEN + ct.len());
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ct);
    Ok(format!("{TOKEN_PREFIX}{}", URL_SAFE_NO_PAD.encode(blob)))
}

/// 用显式密钥解密（供测试与显式调用）
pub fn decrypt_with_key(token: &str, key_b64: &str) -> Result<String> {
    if token.is_empty() {
        return Ok(String::new());
    }
    if !is_encrypted(token) {
        // 明文直通（兼容未加密的存量字段）
        return Ok(token.to_string());
    }
    let raw = decode_key(key_b64)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&raw));
    let blob = URL_SAFE_NO_PAD
        .decode(token.strip_prefix(TOKEN_PREFIX).unwrap())
        .map_err(|_| anyhow!("api_key token 解码失败（格式损坏）"))?;
    if blob.len() < NONCE_LEN {
        return Err(anyhow!("api_key token 长度不足"));
    }
    let (nonce_b, ct) = blob.split_at(NONCE_LEN);
    let nonce = Nonce::from_slice(nonce_b);
    let plain = cipher
        .decrypt(nonce, ct)
        .map_err(|_| anyhow!("api_key 解密失败（密钥不匹配或 token 损坏）"))?;
    String::from_utf8(plain).map_err(|_| anyhow!("api_key 解密后非 UTF-8"))
}

/// 加载主密钥（P0-2 起为「用户主口令 + PBKDF2 派生的 32 字节密钥」，经 [`crate::vault`]）。
///
/// 兼容旧签名保留函数名，但语义从「加载或随机生成」改为「**仅加载**」：
/// Vault 未初始化（无 `vault.json`）时返回明确可诊断错误——**绝不静默回退为空密钥**。
/// 首启须先经 `vault::set_master_passphrase` 显式设置主口令。
pub fn load_or_create_master_key() -> Result<String> {
    crate::vault::load_master_key()
}

/// 加密 API Key（使用主密钥，结果可直接存 `models.json`）
pub fn encrypt_api_key(plain: &str) -> Result<String> {
    if plain.is_empty() {
        return Ok(String::new());
    }
    let key = load_or_create_master_key()?;
    encrypt_with_key(plain, &key)
}

/// 解密 API Key（使用主密钥；明文直通）
pub fn decrypt_api_key(token: &str) -> Result<String> {
    if token.is_empty() || !is_encrypted(token) {
        return Ok(token.to_string());
    }
    let key = load_or_create_master_key()?;
    decrypt_with_key(token, &key)
}

/// 判定字符串是否为加密令牌
pub fn is_encrypted(s: &str) -> bool {
    s.starts_with(TOKEN_PREFIX)
}

/// 文件权限收紧为 0600（仅属主读写）。
/// Unix 直接 chmod；Windows 无等价 owner-only ACL，尽力保留文件存在态。
pub fn chmod_0600(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perm = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perm)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// 解析 API Key 的最终值：
/// 显式传入 > 环境变量（按档案 ID 优先 `MARGUERITE_API_KEY_<ID>`）> 环境变量 `MARGUERITE_API_KEY`
/// > 解密 `models.json` 中的令牌 > 明文回退 > 空
pub fn resolve_api_key(profile_id: &str, stored: &str, explicit: Option<&str>) -> String {
    if let Some(e) = explicit {
        if !e.is_empty() {
            return e.to_string();
        }
    }
    let id_upper = profile_id.to_uppercase().replace(['-', '.'], "_");
    for var in [
        format!("MARGUERITE_API_KEY_{id_upper}"),
        "MARGUERITE_API_KEY".to_string(),
    ] {
        if let Ok(v) = std::env::var(&var) {
            if !v.is_empty() {
                return v;
            }
        }
    }
    if is_encrypted(stored) {
        if let Ok(p) = decrypt_api_key(stored) {
            return p;
        }
    }
    stored.to_string()
}

/// 解码 base64url 主密钥并校验长度
fn decode_key(key_b64: &str) -> Result<Vec<u8>> {
    let raw = URL_SAFE_NO_PAD
        .decode(key_b64.trim())
        .map_err(|_| anyhow!("主密钥格式无效（非 base64url）"))?;
    if raw.len() != KEY_LEN {
        return Err(anyhow!("主密钥长度无效（应为 32 字节）"));
    }
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 串行化所有会读写进程级环境变量（`MARGUERITE_API_KEY*`）的测试，
    /// 避免并行执行时相互污染（预存的测试隔离缺陷：全局 env 被并发写入）。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = generate_key();
        let token = encrypt_with_key("sk-secret-123", &key).unwrap();
        assert!(is_encrypted(&token));
        let plain = decrypt_with_key(&token, &key).unwrap();
        assert_eq!(plain, "sk-secret-123");
    }

    #[test]
    fn test_decrypt_wrong_key_fails() {
        let k1 = generate_key();
        let k2 = generate_key();
        let token = encrypt_with_key("topsecret", &k1).unwrap();
        assert!(decrypt_with_key(&token, &k2).is_err());
    }

    #[test]
    fn test_empty_passthrough() {
        let k = generate_key();
        assert_eq!(encrypt_with_key("", &k).unwrap(), "");
        assert_eq!(decrypt_with_key("", &k).unwrap(), "");
    }

    #[test]
    fn test_plaintext_decrypt_passthrough() {
        // 非令牌字符串解密时直通，不报错
        let k = generate_key();
        assert_eq!(decrypt_with_key("plain-k", &k).unwrap(), "plain-k");
    }

    #[test]
    fn test_resolve_prefers_explicit() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("MARGUERITE_API_KEY", "env-global");
        let r = resolve_api_key("deepseek-chat", "stored", Some("explicit-k"));
        assert_eq!(r, "explicit-k");
        std::env::remove_var("MARGUERITE_API_KEY");
    }

    #[test]
    fn test_resolve_env_fallback() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("MARGUERITE_API_KEY_MY_MODEL", "env-per-profile");
        let r = resolve_api_key("my-model", "stored", None);
        assert_eq!(r, "env-per-profile");
        std::env::remove_var("MARGUERITE_API_KEY_MY_MODEL");
    }

    #[test]
    fn test_resolve_stored_plaintext() {
        let _guard = ENV_LOCK.lock().unwrap();
        let r = resolve_api_key("x", "stored-plain", None);
        assert_eq!(r, "stored-plain");
    }
}
