//! provider id → 请求协议映射的**唯一事实源**。
//!
//! # 为什么必须收口到一处
//!
//! 这段映射此前在 6 个地方各写一份（`model/registry.rs` · `model/router.rs` ·
//! `model/llm.rs` ×2 · `gateway/fleet.rs` · `tui/workbench/mod.rs`），而**每一份
//! 只认字面量 `"anthropic"`**。后果不是「某个 id 少个分支」，而是**一整族服务商
//! 静默走错协议**：
//!
//! - `custom_anthropic`（用户自建 Anthropic 兼容端点）
//! - `qwen_token_plan_anthropic` / `kimi_coding_anthropic` / `minimax_coding_anthropic`
//! - `mimo_anthropic` / `tencent_tokenhub_anthropic` / `tencent_token_plan_anthropic`
//! - `volcengine_coding_plan_anthropic` / `byteplus_coding_plan_anthropic`
//!
//! 这些 id 全部落进 `_ => LLMProvider::Custom`，于是请求打到 `/v1/chat/completions`
//! 且带 `Authorization: Bearer` —— 而它们要的是 `/v1/messages` + Anthropic 的
//! content blocks / tool_use。**用户看到的是 404 或「模型没按我配的方式工作」，
//! 而不是「协议选错了」**，这正是本仓库反复吃过的「下游文案掩盖真因」。
//!
//! 收口之后：新增 provider id 只改这一处，且两侧实现（Rust 引擎 / Python 侧车）
//! 的判定口径可以对账。
//!
//! # 鉴权头为什么还分两种
//!
//! Anthropic Messages 协议本身只有一套，但密钥头有两种：
//! Anthropic 官方与腾讯 TokenHub 按量付费端点要 `x-api-key`；MiniMax / Kimi /
//! 腾讯 Token 计划 等兼容网关沿用 OpenAI 风格的 `Authorization: Bearer`。
//! 二者与 `pyengine/src/opensquilla/provider/registry.py` 的 `auth_header_style`
//! **逐条对齐**（见 [`X_API_KEY_PROVIDER_IDS`]）。

use crate::model::llm::LLMProvider;

/// Anthropic Messages 端点用 `x-api-key` 鉴权的 provider id。
///
/// 与 `pyengine/src/opensquilla/provider/registry.py` 的 `auth_header_style="x-api-key"`
/// 逐条对齐；**不在此表内的 Anthropic 家族 id 一律用 `Authorization: Bearer`**。
const X_API_KEY_PROVIDER_IDS: &[&str] = &["anthropic", "tencent_tokenhub_anthropic"];

/// provider id 是否属于 Anthropic Messages 家族。
///
/// 家族规则：`anthropic` 本体，或以 `_anthropic` 结尾的兼容网关 id。
pub fn is_anthropic_family(provider_id: &str) -> bool {
    let id = provider_id.trim().to_ascii_lowercase();
    id == "anthropic" || id.ends_with("_anthropic")
}

/// provider id → 协议映射。
///
/// 这是全仓**唯一**的映射入口：`models.json` 档案、`config.yaml` 主模型与副模型、
/// 故障转移链、Fleet 成员、TUI 经验抽取、Tauri 壳全部走它，避免同一 id 在不同
/// 路径上解析出不同协议。
///
/// `base_url` 只在落进 `Custom` 时使用（Anthropic 家族的 base_url 走
/// `LLMConfig.base_url`，与其它协议同一套归一化规则）。
pub fn provider_for_id(provider_id: &str, base_url: &str) -> LLMProvider {
    let id = provider_id.trim().to_ascii_lowercase();
    match id.as_str() {
        "openai" => LLMProvider::OpenAI,
        "deepseek" => LLMProvider::DeepSeek,
        _ if is_anthropic_family(&id) => {
            if X_API_KEY_PROVIDER_IDS.contains(&id.as_str()) {
                LLMProvider::Anthropic
            } else {
                LLMProvider::AnthropicCompatible
            }
        }
        _ => LLMProvider::Custom {
            base_url: base_url.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::llm::AnthropicAuth;

    /// Anthropic 家族的全部 provider id。
    ///
    /// 名单来自 `pyengine/src/opensquilla/provider/registry.py` 的 `backend="anthropic"`
    /// 条目；少列一个就是「那个服务商静默走错协议」，多列一个则是「凭空捏造」。
    const ANTHROPIC_FAMILY: &[&str] = &[
        "anthropic",
        "custom_anthropic",
        "qwen_token_plan_anthropic",
        "kimi_coding_anthropic",
        "minimax_coding_anthropic",
        "mimo_anthropic",
        "tencent_tokenhub_anthropic",
        "tencent_token_plan_anthropic",
        "volcengine_coding_plan_anthropic",
        "byteplus_coding_plan_anthropic",
    ];

    #[test]
    fn every_anthropic_family_id_speaks_anthropic_messages() {
        for id in ANTHROPIC_FAMILY {
            let provider = provider_for_id(id, "https://example.test");
            assert!(
                provider.speaks_anthropic_messages(),
                "{id} 必须走 Anthropic Messages 协议（/v1/messages），实得 {provider:?}：\
                 落进 Custom 意味着请求会打到 /v1/chat/completions 且带错鉴权头"
            );
        }
    }

    #[test]
    fn anthropic_family_detection_is_case_and_space_insensitive() {
        assert!(is_anthropic_family("ANTHROPIC"));
        assert!(is_anthropic_family("  custom_anthropic  "));
        assert!(is_anthropic_family("MiniMax_Coding_Anthropic"));
        // 不得误伤：只以 anthropic **开头**的不是家族成员
        assert!(!is_anthropic_family("anthropic_compatible_v2"));
        assert!(!is_anthropic_family("openai"));
        assert!(!is_anthropic_family("custom"));
    }

    #[test]
    fn non_anthropic_ids_keep_their_previous_mapping() {
        // 存量行为必须逐字不变：这三个 id 各自落到同一分支
        assert!(matches!(provider_for_id("openai", "x"), LLMProvider::OpenAI));
        assert!(matches!(
            provider_for_id("deepseek", "x"),
            LLMProvider::DeepSeek
        ));
        match provider_for_id("ollama", "http://localhost:11434") {
            LLMProvider::Custom { base_url } => {
                assert_eq!(base_url, "http://localhost:11434");
            }
            other => panic!("ollama 必须落进 Custom，实得 {other:?}"),
        }
    }

    #[test]
    fn anthropic_proper_and_tencent_tokenhub_sign_with_x_api_key() {
        // 官方与腾讯 TokenHub 按量付费端点用 x-api-key（与 Python 侧逐条对齐）
        for id in ["anthropic", "tencent_tokenhub_anthropic"] {
            let provider = provider_for_id(id, "https://example.test");
            assert_eq!(
                provider.anthropic_auth(),
                AnthropicAuth::XApiKey,
                "{id} 的鉴权头错了"
            );
        }
    }

    #[test]
    fn compatible_gateways_sign_with_bearer() {
        // 其余兼容网关沿用 OpenAI 风格密钥头
        for id in ANTHROPIC_FAMILY {
            if X_API_KEY_PROVIDER_IDS.contains(id) {
                continue;
            }
            let provider = provider_for_id(id, "https://example.test");
            assert_eq!(
                provider.anthropic_auth(),
                AnthropicAuth::Bearer,
                "{id} 的鉴权头错了"
            );
        }
    }

    #[test]
    fn non_anthropic_providers_have_no_anthropic_auth() {
        // 非 Anthropic 端点走 Bearer（与既有行为一致），绝不误发 x-api-key
        for id in ["openai", "deepseek", "ollama", "custom"] {
            let provider = provider_for_id(id, "https://example.test");
            assert!(!provider.speaks_anthropic_messages(), "{id}");
        }
    }
}
