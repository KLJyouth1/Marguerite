//! Vault 主口令闭环（P0-2）——把主密钥从「本地随机 `model.key`」升级为
//! 「用户主口令 + PBKDF2 派生的 32 字节密钥」。
//!
//! ## 分层
//! - `derive.rs`：纯 PBKDF2-HMAC-SHA256 派生（无 IO）。
//! - `mod.rs`：Vault 状态持久化（`<config_dir>/vault.json`，0600）+ 首启门禁。
//!   `model::byok` 经 [`load_master_key`] 无缝切换到派生密钥，signature 保持兼容。
//!
//! ## 首启门禁
//! Vault 未初始化（无 `vault.json`）时，[`load_master_key`]/[`load_or_create_master_key`]
//! 返回**明确可诊断错误**，绝不静默回退为空密钥。须先经 [`set_master_passphrase`]
//! 显式设置主口令，派生密钥才落盘启用。
//!
//! ## 安全
//! - 派生密钥 `0600` 落盘（`.key` 字段），salt/iterations/kdf 元数据同文件保存。
//! - `verify` 字段存 `SHA-256(派生密钥)`，用于检测密钥字段被篡改或用不同主口令重新派生。
//! - **不存主口令明文**；口令只在派生瞬间参与计算。

pub mod derive;

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::persistence::config::Config;

pub use derive::{DEFAULT_ITERATIONS, KEY_LEN, SALT_LEN};

/// 状态文件 schema 版本
const STATE_VERSION: u32 = 1;
/// KDF 标识（用于防误读其它算法生成的密钥）
const KDF_NAME: &str = "pbkdf2-hmac-sha256";

/// Vault 状态文件路径：`<config_dir>/vault.json`
pub fn state_path() -> PathBuf {
    Config::config_dir().join("vault.json")
}

/// 落盘状态（自描述：版本 + KDF + salt + 迭代次数 + 校验标签 + 派生密钥）。
#[derive(Serialize, Deserialize)]
struct VaultState {
    version: u32,
    kdf: String,
    salt: String,
    iterations: u32,
    verify: String,
    key: String,
}

/// Vault 是否已初始化（存在 `vault.json`）。
pub fn is_initialized() -> bool {
    state_path().exists()
}

/// 设置主口令（首次初始化 / 重置派生密钥）。
///
/// 生成随机 salt 并用 PBKDF2-HMAC-SHA256 派生 32 字节密钥，连同元数据原子落盘（0600）。
/// 返回 base64url 派生密钥副本（供调用方直接使用）。空口令被拒绝。
pub fn set_master_passphrase(passphrase: &str) -> Result<String> {
    if passphrase.trim().is_empty() {
        return Err(anyhow!("主口令不能为空，请设置非空白主口令"));
    }
    let salt_b64 = derive::salt_b64(&derive::generate_salt());
    let key = derive::derive_key(passphrase, &salt_b64, DEFAULT_ITERATIONS)?;
    let key_b64 = derive::key_b64(&key);
    let verify = derive::key_b64(&Sha256::digest(&key)[..]);
    let state = VaultState {
        version: STATE_VERSION,
        kdf: KDF_NAME.to_string(),
        salt: salt_b64,
        iterations: DEFAULT_ITERATIONS,
        verify,
        key: key_b64.clone(),
    };
    persist(&state)?;
    Ok(key_b64)
}

/// 加载派生主密钥。
///
/// - Vault 未初始化 → 返回可诊断错误（绝不静默回退）。
/// - 状态文件损坏 / KDF 不支持 / 密钥长度非法 / 校验失败 → 均返回人话错误。
pub fn load_master_key() -> Result<String> {
    let p = state_path();
    if !p.exists() {
        return Err(anyhow!(
            "Vault 尚未初始化：未找到主口令派生密钥（{}）。请先设置主口令，例如通过 vault init / 设置主口令入口。",
            p.display()
        ));
    }
    let text = std::fs::read_to_string(&p)
        .map_err(|e| anyhow!("读取 vault 状态文件失败（{}）: {e}", p.display()))?;
    let state: VaultState = serde_json::from_str(&text)
        .map_err(|e| anyhow!("vault 状态文件损坏（{}）: {e}", p.display()))?;
    if state.kdf != KDF_NAME {
        return Err(anyhow!("vault KDF 不受支持: {}（期望 {KDF_NAME}）", state.kdf));
    }
    if state.key.is_empty() {
        return Err(anyhow!("vault 主密钥字段为空（状态文件未正确写入）"));
    }
    let key = derive::decode_key_b64(&state.key)?;
    let expect = derive::key_b64(&Sha256::digest(&key)[..]);
    if expect != state.verify {
        return Err(anyhow!(
            "vault 主密钥校验失败：密钥字段可能被篡改，或被不同的主口令重新派生。请重新设置主口令。"
        ));
    }
    Ok(state.key.clone())
}

/// 原子落盘：写临时文件 → 0600 → rename → 0600。
fn persist(state: &VaultState) -> Result<()> {
    let p = state_path();
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(state)?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    set_0600(&tmp)?;
    std::fs::rename(&tmp, &p)?;
    set_0600(&p)?;
    Ok(())
}

/// 文件权限收紧为 0600（仅属主读写）。Unix chmod；Windows 尽力保留存在态。
fn set_0600(path: &std::path::Path) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::byok;

    /// 为每个用例注入独立临时目录作为 config_dir（线程局部），互不串扰，
    /// 且**绝**不写入真实 `~/.marguerite/`。
    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mgt_vault_{name}_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Config::set_config_dir_override(dir.clone());
        dir
    }

    #[test]
    fn derived_key_roundtrip_encrypt_decrypt() {
        let salt = derive::generate_salt();
        let salt_b64 = derive::salt_b64(&salt);
        let key_b64 =
            derive::key_b64(&derive::derive_key("pw", &salt_b64, DEFAULT_ITERATIONS).unwrap());
        let token = byok::encrypt_with_key("sk-roundtrip", &key_b64).unwrap();
        assert!(byok::is_encrypted(&token));
        assert_eq!(byok::decrypt_with_key(&token, &key_b64).unwrap(), "sk-roundtrip");
    }

    #[test]
    fn wrong_passphrase_key_fails_with_diagnostic() {
        let salt = derive::generate_salt();
        let salt_b64 = derive::salt_b64(&salt);
        let k_a = derive::key_b64(&derive::derive_key("right-pw", &salt_b64, DEFAULT_ITERATIONS).unwrap());
        let k_b = derive::key_b64(&derive::derive_key("wrong-pw", &salt_b64, DEFAULT_ITERATIONS).unwrap());
        let token = byok::encrypt_with_key("topsecret", &k_a).unwrap();
        let err = byok::decrypt_with_key(&token, &k_b).unwrap_err();
        assert!(
            err.to_string().contains("解密失败") || err.to_string().contains("密钥不匹配"),
            "错误信息应为人话可读，实际: {err}"
        );
    }

    #[test]
    fn uninitialized_vault_returns_diagnostic_error() {
        let _ = tmp_dir("uninit");
        let err = byok::load_or_create_master_key().unwrap_err();
        assert!(
            err.to_string().contains("尚未初始化"),
            "未初始化必须返回明确可诊断错误，实际: {err}"
        );
    }

    #[test]
    fn set_master_passphrase_then_encrypt_decrypt_works() {
        let _dir = tmp_dir("firstboot");
        let key_b64 = set_master_passphrase("my-secret-pass").unwrap();
        assert!(state_path().exists());
        assert!(is_initialized());
        let token = byok::encrypt_api_key("sk-firstboot").unwrap();
        assert_eq!(byok::decrypt_api_key(&token).unwrap(), "sk-firstboot");
        // 重载得到同一派生密钥
        let loaded = load_master_key().unwrap();
        assert_eq!(loaded, key_b64);
        assert_eq!(loaded.len(), 43); // 32 字节 base64url
    }

    #[test]
    fn empty_passphrase_is_rejected() {
        let _ = tmp_dir("empty");
        assert!(set_master_passphrase("   ").is_err());
    }

    #[test]
    fn vault_state_never_stores_passphrase_plaintext() {
        let _dir = tmp_dir("no_plain");
        set_master_passphrase("super-secret-passphrase").unwrap();
        let raw = std::fs::read_to_string(state_path()).unwrap();
        assert!(
            !raw.contains("super-secret-passphrase"),
            "vault.json 不得包含主口令明文"
        );
        // 校验标签必须存在
        assert!(raw.contains("\"iterations\""));
        assert!(raw.contains("\"salt\""));
        assert!(raw.contains("\"verify\""));
    }

    #[test]
    fn derived_key_metadata_is_consistent() {
        let _dir = tmp_dir("meta");
        set_master_passphrase("p1").unwrap();
        let raw = std::fs::read_to_string(state_path()).unwrap();
        assert!(raw.contains("pbkdf2-hmac-sha256"));
        assert!(raw.contains("210000"));
    }
}
