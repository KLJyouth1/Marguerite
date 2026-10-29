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

use crate::persistence::config::Config;

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
    // 唯一实现放在 `persistence`：令牌 / `.env` / 凭据台账与之共用同一口径，
    // 免得两处各自维护一份「什么算 0600」而在某个平台悄悄漂移。
    crate::persistence::harden_private_file(path)
        .map_err(|e| anyhow!("chmod 0600 {}: {}", path.display(), e))
}

/// 解析 API Key 的最终值（多 provider 并存）：
///
/// 显式传入 > 档案声明的环境变量名（`stored_env`）> 档案分槽
/// `MARGUERITE_API_KEY_<PROFILE_ID>`（进程环境变量 → `.env`）
/// > 全局 `MARGUERITE_API_KEY`（进程环境变量 → `.env`，受归属标记约束）
/// > 解密 `models.json` 中的令牌 > 明文回退 > 空
///
/// # 为什么必须读 `.env`
///
/// 旧实现只读**进程环境变量**，而 `Config::write_env_key*` 写的是 `.env` 文件 ——
/// 于是「设置页里明明填了密钥」在非 `Config::load` 路径上永远解析不出来。
/// 现在两处共用同一个读取入口（[`Config::read_env_vars`]），写入即可见。
///
/// # 归属标记（跨域泄漏护栏）
///
/// 全局槽只有一个，若 B 家密钥覆盖了 A 家、又切回 A 家的端点，就会把 B 的钥匙
/// 发给 A。写入全局槽时记录 `MARGUERITE_API_KEY_PROVIDER`；解析时标记与 `provider`
/// 不符则**拒绝**使用该全局密钥。标记缺省（升级前用户）放行以保持兼容。
/// 该端点是否**不需要 API Key**（本地推理服务：Ollama / LM Studio / vLLM 回环）。
///
/// # 为什么必须是一个共享判据（2026-10-21 修）
///
/// 「没有密钥」曾被四处当成「没配置」的代理判据，于是本地模型被系统性误判：
/// · `doctor::connectivity` → `model_api` 恒 DEGRADED；
/// · `onboarding_status` → `llmConfigured=false` → 设置页横幅「需要配置」；
/// · `chat::api_key_gate` → 拒发消息（这一处早已豁免，其余三处没跟上）；
/// · `llm_profile` 建档案时 → kind 被记成 `cloud`（需要密钥的云服务）。
/// 用户视角就是「本地模型明明就绪，却一直让我配置服务商」。
///
/// 判据（2026-10-22 修正后的窄口径）：
///
/// 1. 本地推理运行时（`ollama` / `lmstudio` / `llama.cpp` …）→ 免密钥，与地址无关；
/// 2. **已知云厂商**（`deepseek` / `openai` …，**不含 `custom`**）→ 一律需要密钥，
///    即使 `base_url` 指向回环 —— 那通常是企业出口代理、本地反向代理、缓存代理
///    或测试 mock，它们后面仍然要鉴权；
/// 3. 其余（含 `custom` 与空 provider）→ 回环即视为本地服务器（免密钥）。
///
/// **新增任何「是否需要凭据」的判断都必须调用本函数**，不得再抄一份 host 列表。
///
/// # 为什么不是直接复用 `router::is_local_model_endpoint`
///
/// 试过，且**被测试抓回来了**：`keyless_endpoint_table` 断言
/// `custom + http://127.0.0.1:1234/v1` 必须免密钥 —— 那是 LM Studio / llama.cpp
/// 的典型配置。而 `is_local_model_endpoint` 把 `custom` 归为云端（路由侧刻意如此，
/// 避免把用户自定义端点钉住不被静态路由改写）。两个判据**语义本就不同**，
/// 强行合并会一边修好、另一边回归。
///
/// # 旧实现的问题（本次修正的对象）
///
/// 旧实现是 `provider == "ollama" || is_loopback_endpoint(base_url)` ——
/// 它把「回环」直接等同于「免密钥」，于是 `deepseek` 指向 `127.0.0.1`（代理 / mock）
/// 也被判成免密钥，深度检查被跳过。实测后果：`doctor::tests` 里
/// `check_model_api_probe_on_valid_key_is_ok` 与
/// `check_model_api_probe_on_rejects_bad_key_without_leaking_it` 两条**一直失败**
/// （mock 只能绑回环），而 clippy 长期是红的、测试套件没人跑完，失败长期无人发现。
pub fn is_keyless_endpoint(provider: &str, base_url: &str) -> bool {
    let p = provider.trim().to_ascii_lowercase();
    // 1) 本地推理运行时：与地址无关
    if matches!(
        p.as_str(),
        "ollama" | "llamacpp" | "llama.cpp" | "llama_cpp" | "lmstudio" | "koboldcpp" | "local"
            | "ggml" | "opencode" | "tabby"
    ) {
        return true;
    }
    // 2) 已知云厂商（custom 除外）：即使在回环也要鉴权
    if p != "custom" && crate::model::router::is_known_cloud_provider(&p) {
        return false;
    }
    // 3) 其余：回环即本地服务器
    crate::model::router::is_loopback_endpoint(base_url)
}

pub fn resolve_api_key(
    profile_id: &str,
    provider: &str,
    stored: &str,
    stored_env: &str,
    explicit: Option<&str>,
) -> String {
    resolve_api_key_with_source(profile_id, provider, stored, stored_env, explicit).0
}

/// [`resolve_api_key`] 的带来源版本：返回 `(密钥, 来源)`。
///
/// 来源取值：`"explicit"`（调用方显式传入）、实际命中的**环境变量名**
/// （档案声明名 / 分槽名 / `MARGUERITE_API_KEY`）、`"stored"`（密文或明文回退）、
/// 空串（完全无凭据）。
///
/// 设置页需要如实告诉用户「读的是哪个变量」。若在调用方另写一份判定逻辑，
/// 迟早会与解析链分叉（UI 说读 A、引擎实际读 B）；因此来源信息必须由
/// **同一条解析链**产出。
pub fn resolve_api_key_with_source(
    profile_id: &str,
    provider: &str,
    stored: &str,
    stored_env: &str,
    explicit: Option<&str>,
) -> (String, String) {
    if let Some(e) = explicit {
        if !e.is_empty() {
            return (e.to_string(), "explicit".to_string());
        }
    }
    let dotenv = Config::read_env_vars();
    let lookup = |name: &str| -> Option<String> {
        if let Ok(v) = std::env::var(name) {
            if !v.is_empty() {
                return Some(v);
            }
        }
        dotenv.get(name).filter(|v| !v.is_empty()).cloned()
    };

    // 0) 档案声明的环境变量名：用户显式选择的来源，必须压过残留的分槽值，
    //    否则「从手填密钥切到环境变量引用」后旧分槽会继续生效（切了等于没切）。
    let declared_env = stored_env.trim();
    if !declared_env.is_empty() {
        if let Some(v) = lookup(declared_env) {
            return (v, declared_env.to_string());
        }
    }

    // 1) 档案专属槽：多 provider 共存的关键，永远优先于全局槽。
    //    无档案 id 时不拼槽名（否则会撞上 `MARGUERITE_API_KEY_` 这种空后缀键）。
    if !profile_id.trim().is_empty() {
        let slot = Config::env_slot_name(profile_id);
        if let Some(v) = lookup(&slot) {
            return (v, slot);
        }
    }

    // 2) 全局槽：仅在归属标记为空或与当前 provider 相符时可用。
    let global_allowed = lookup(Config::ENV_KEY_PROVIDER_MARKER)
        .map(|m| m.trim().is_empty() || m.eq_ignore_ascii_case(provider.trim()))
        .unwrap_or(true);
    if global_allowed {
        if let Some(v) = lookup("MARGUERITE_API_KEY") {
            return (v, "MARGUERITE_API_KEY".to_string());
        }
    }

    if is_encrypted(stored) {
        if let Ok(p) = decrypt_api_key(stored) {
            return (p, "stored".to_string());
        }
    }
    if stored.is_empty() {
        (String::new(), String::new())
    } else {
        (stored.to_string(), "stored".to_string())
    }
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
        let r = resolve_api_key("deepseek-chat", "deepseek", "stored", "", Some("explicit-k"));
        assert_eq!(r, "explicit-k");
        std::env::remove_var("MARGUERITE_API_KEY");
    }

    #[test]
    fn test_resolve_env_fallback() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("MARGUERITE_API_KEY_MY_MODEL", "env-per-profile");
        let r = resolve_api_key("my-model", "custom", "stored", "", None);
        assert_eq!(r, "env-per-profile");
        std::env::remove_var("MARGUERITE_API_KEY_MY_MODEL");
    }

    /// 每个用例独立的配置目录（thread-local override），避免读写真实 `~/.marguerite`。
    fn isolated_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mgt_byok_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Config::set_config_dir_override(dir.clone());
        dir
    }

    /// 清空可能存在的进程级密钥键，确保断言只反映 `.env` 的内容。
    fn clear_process_env() {
        for key in [
            "MARGUERITE_API_KEY",
            "MARGUERITE_API_KEY_PROVIDER",
            "MARGUERITE_API_KEY_X",
            "MARGUERITE_API_KEY_DEEPSEEK_PRIMARY",
            "MARGUERITE_API_KEY_OPENAI_PRIMARY",
            "MARGUERITE_TEST_DECLARED_KEY",
        ] {
            std::env::remove_var(key);
        }
    }

    #[test]
    fn test_resolve_stored_plaintext() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = isolated_dir();
        clear_process_env();
        // cwd 下的 `core/.env` 可能带真实全局密钥：写入不匹配的归属标记，
        // 明确验证「无槽可用时回退 stored 明文」而不是被环境污染。
        Config::write_env_key_for("__other__", "sk-from-other-provider");
        let r = resolve_api_key("x", "x", "stored-plain", "", None);
        assert_eq!(r, "stored-plain");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 根因回归：写入 `.env` 的档案分槽必须能被解析出来（旧实现只读进程环境变量）。
    #[test]
    fn test_resolve_reads_slot_written_to_dotenv() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = isolated_dir();
        clear_process_env();
        assert!(Config::write_env_key_slot("deepseek-primary", "sk-deepseek-file").is_some());
        assert_eq!(
            resolve_api_key("deepseek-primary", "deepseek", "", "", None),
            "sk-deepseek-file"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 多 provider 并存：两家密钥互不覆盖，删一家不影响另一家。
    #[test]
    fn test_two_providers_keep_their_own_keys() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = isolated_dir();
        clear_process_env();

        assert!(Config::write_env_key_slot("deepseek-primary", "sk-ds").is_some());
        assert!(Config::write_env_key_slot("openai-primary", "sk-oa").is_some());
        // 全局槽属于 openai（写入归属标记），用于验证删除 DeepSeek 分槽后不会串用别家密钥。
        assert!(Config::write_env_key_for("openai", "sk-global-oa").is_some());

        assert_eq!(
            resolve_api_key("deepseek-primary", "deepseek", "", "", None),
            "sk-ds"
        );
        assert_eq!(
            resolve_api_key("openai-primary", "openai", "", "", None),
            "sk-oa"
        );

        // 删掉 DeepSeek 的分槽：OpenAI 的分槽必须原样保留
        assert!(Config::remove_env_key_slot("deepseek-primary"));
        assert_eq!(
            resolve_api_key("openai-primary", "openai", "", "", None),
            "sk-oa"
        );
        // DeepSeek 此时无可用密钥（全局槽归属 openai，必须拒绝跨 provider 复用）
        assert_eq!(resolve_api_key("deepseek-primary", "deepseek", "", "", None), "");

        let content = std::fs::read_to_string(dir.join(".env")).unwrap();
        assert!(content.contains("MARGUERITE_API_KEY_OPENAI_PRIMARY=sk-oa"), "{content}");
        assert!(content.contains("MARGUERITE_API_KEY_PROVIDER=openai"), "{content}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 跨域泄漏护栏：全局密钥带归属标记时，绝不能被别家 provider 解析到。
    #[test]
    fn test_marked_global_key_never_crosses_providers() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = isolated_dir();
        clear_process_env();

        assert!(Config::write_env_key_for("deepseek", "sk-global-ds").is_some());
        assert_eq!(
            resolve_api_key("openai-primary", "openai", "", "", None),
            "",
            "A 家的全局密钥不得发给 B 家"
        );
        assert_eq!(
            resolve_api_key("deepseek-primary", "deepseek", "", "", None),
            "sk-global-ds"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    /// 档案声明 `api_key_env`（设置页「使用环境变量」）时：
    /// 1) 该变量必须能被解析出来（`.env` 写进去也算）；
    /// 2) 它必须压过残留的档案分槽，否则「从手填切到环境变量」等于没切；
    /// 3) 变量缺失时才回落到分槽。
    #[test]
    fn test_declared_env_beats_stale_slot() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = isolated_dir();
        clear_process_env();
        assert!(Config::write_env_key_slot("deepseek-primary", "sk-stale-slot").is_some());
        assert!(Config::write_env_var_to(
            &dir.join(".env"),
            "MARGUERITE_TEST_DECLARED_KEY",
            "sk-declared-env"
        )
        .is_some());

        assert_eq!(
            resolve_api_key(
                "deepseek-primary",
                "deepseek",
                "",
                "MARGUERITE_TEST_DECLARED_KEY",
                None
            ),
            "sk-declared-env",
            "档案声明的环境变量必须优先于残留分槽"
        );
        // 变量不存在 → 回落到分槽（明确、可解释）
        assert_eq!(
            resolve_api_key("deepseek-primary", "deepseek", "", "MARGUERITE_UNSET_KEY", None),
            "sk-stale-slot"
        );

        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn keyless_endpoint_table() {
        // 本地推理：provider 或回环端点任一成立即免密钥
        assert!(is_keyless_endpoint("ollama", "http://localhost:11434"));
        assert!(is_keyless_endpoint("ollama", ""));
        assert!(is_keyless_endpoint("custom", "http://127.0.0.1:1234/v1"));
        assert!(is_keyless_endpoint("custom", "http://[::1]:8080"));
        assert!(is_keyless_endpoint("lmstudio", "http://0.0.0.0:1234"));
        // 云端：必须配置密钥
        assert!(!is_keyless_endpoint("deepseek", "https://api.deepseek.com"));
        assert!(!is_keyless_endpoint("openai", "https://api.openai.com/v1"));
        assert!(!is_keyless_endpoint("custom", ""));
    }

}
