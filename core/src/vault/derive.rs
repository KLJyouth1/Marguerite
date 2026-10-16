//! Vault 密钥派生层——纯 PBKDF2-HMAC-SHA256（无 IO、无持久化）。
//!
//! 负责：生成随机 salt、从用户主口令派生 32 字节主密钥、base64url 编解码与长度校验。
//! 上层（`mod.rs`）负责持久化状态与首启门禁，本层保持纯函数、可独立单测。
//!
//! 设计（2026 P0-2 主口令闭环）：
//! - 算法：PBKDF2-HMAC-SHA256（RustCrypto 纯 Rust，无 C 工具链）。
//! - 迭代次数：默认 210000（OWASP 2023 对 PBKDF2-HMAC-SHA256 推荐 ≥ 60 万，
//!   此处取 21 万兼顾交互延迟与当代消费级 CPU，交互式设置主口令 < 1s 体验优先；
//!   可配置，见 `DEFAULT_ITERATIONS`）。
//! - salt 长度：16 字节（128-bit，随机，足够抵御预计算彩虹表）。
//! - 派生密钥长度：32 字节（AES-256-GCM 硬性要求）。

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use pbkdf2::pbkdf2_hmac;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::Sha256;

/// 派生主密钥长度（AES-256）
pub const KEY_LEN: usize = 32;
/// salt 长度（128-bit）
pub const SALT_LEN: usize = 16;
/// 默认 PBKDF2 迭代次数
pub const DEFAULT_ITERATIONS: u32 = 210_000;

/// 生成随机 salt（16 字节，OsRng）
pub fn generate_salt() -> Vec<u8> {
    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    salt.to_vec()
}

/// salt → base64url
pub fn salt_b64(salt: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(salt)
}

/// 从主口令派生 32 字节密钥（PBKDF2-HMAC-SHA256）。
///
/// 同一口令 + 同一 salt + 同迭代次数 → 确定性输出；口令不同则密钥不同。
pub fn derive_key(passphrase: &str, salt_b64: &str, iterations: u32) -> Result<Vec<u8>> {
    let salt = URL_SAFE_NO_PAD
        .decode(salt_b64.trim())
        .map_err(|_| anyhow!("vault salt 编码无效（非 base64url）"))?;
    if salt.is_empty() {
        return Err(anyhow!("vault salt 为空"));
    }
    let mut output = vec![0u8; KEY_LEN];
    pbkdf2_hmac::<Sha256>(passphrase.as_bytes(), &salt, iterations, &mut output);
    Ok(output)
}

/// 密钥字节 → base64url
pub fn key_b64(key: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(key)
}

/// 解码 base64url 密钥并校验长度为 `KEY_LEN` 字节。
pub fn decode_key_b64(key_b64: &str) -> Result<Vec<u8>> {
    let raw = URL_SAFE_NO_PAD
        .decode(key_b64.trim())
        .map_err(|_| anyhow!("主密钥格式无效（非 base64url）"))?;
    if raw.len() != KEY_LEN {
        return Err(anyhow!("主密钥长度无效（应为 {KEY_LEN} 字节，实际 {}）", raw.len()));
    }
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_is_deterministic_for_same_passphrase_and_salt() {
        let salt = generate_salt();
        let salt_b64 = salt_b64(&salt);
        let k1 = derive_key("hunter2", &salt_b64, DEFAULT_ITERATIONS).unwrap();
        let k2 = derive_key("hunter2", &salt_b64, DEFAULT_ITERATIONS).unwrap();
        assert_eq!(k1, k2);
    }

    #[test]
    fn derive_differs_for_different_passphrase() {
        let salt = generate_salt();
        let salt_b64 = salt_b64(&salt);
        let k1 = derive_key("pass-a", &salt_b64, DEFAULT_ITERATIONS).unwrap();
        let k2 = derive_key("pass-b", &salt_b64, DEFAULT_ITERATIONS).unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn derived_key_is_32_bytes_and_base64url_roundtrips() {
        let salt = generate_salt();
        let salt_b64 = salt_b64(&salt);
        let key = derive_key("pw", &salt_b64, DEFAULT_ITERATIONS).unwrap();
        assert_eq!(key.len(), KEY_LEN);
        let b64 = key_b64(&key);
        assert_eq!(decode_key_b64(&b64).unwrap(), key);
    }

    #[test]
    fn bad_salt_is_rejected() {
        assert!(derive_key("pw", "not base64!!", DEFAULT_ITERATIONS).is_err());
        assert!(derive_key("pw", "", DEFAULT_ITERATIONS).is_err());
    }
}
