//! 请求协议（`wire_api`）—— 与 Codex 的同名字段同义。
//!
//! # 为什么需要它
//!
//! Codex 在 `~/.codex/config.toml` 用 `wire_api = "responses"` 把请求打到 OpenAI
//! 的 **Responses API**（`/v1/responses`）；它也有 `"chat"` 形态。本仓库此前只有
//! 一条路径：`build_chat_url()` 恒拼 `/chat/completions`，`LLMConfig` 里也没有
//! 任何"协议选择"字段。
//!
//! 后果不是"少一个功能"，而是**接不进来**：只提供 Responses 端点的服务商无法使用；
//! 反过来把 responses 端点当 chat 打，会得到 404 —— 错误信息却指向"模型名不对"，
//! 极难排查（这类"下游文案掩盖真因"是本仓库反复吃过的亏）。
//!
//! # 为什么默认 `Chat`
//!
//! 现有 37 个内置档案、全部本地端点（Ollama / vLLM / LM Studio）以及绝大多数
//! OpenAI 兼容网关都只有 `/chat/completions`。默认值必须让**存量零迁移**：
//! `#[serde(default)]` → `Chat`，缺字段的旧 `models.json` / `config.yaml` 行为逐字节不变。
//!
//! # 为什么不用"自动探测"
//!
//! 探测要额外发一次请求：既增加延迟，也会在用户不知情时把载荷发给一个可能
//! 不该收到的地址。协议由用户/档案模板显式声明，错了就**响亮报错**（见
//! [`validate`]），不静默降级。

use serde::{Deserialize, Serialize};

/// 请求协议形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireApi {
    /// OpenAI Chat Completions（`/v1/chat/completions`）。默认值。
    #[default]
    Chat,
    /// OpenAI Responses（`/v1/responses`）。
    Responses,
}

impl WireApi {
    /// 稳定字符串键（写配置 / 回传前端 / CLI 参数都用它）。
    pub fn as_key(&self) -> &'static str {
        match self {
            WireApi::Chat => "chat",
            WireApi::Responses => "responses",
        }
    }

    /// 从字符串解析。**只接受明确写法**，并把常见别名收进来。
    ///
    /// 返回 `None` 时调用方必须报错而不是回落 —— 让 "wire_api = respones" 这类
    /// 拼写错误变成立即失败，而不是悄悄走 chat 然后 404。
    pub fn from_key(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "chat" | "chat_completions" | "chat-completions" | "completions" => {
                Some(WireApi::Chat)
            }
            "responses" | "response" | "responses_api" | "responses-api" => Some(WireApi::Responses),
            _ => None,
        }
    }

    /// 人类可读标签（错误消息与 UI 用）。
    pub fn label(&self) -> &'static str {
        match self {
            WireApi::Chat => "Chat Completions",
            WireApi::Responses => "Responses",
        }
    }
}

/// 校验 `wire_api` 与 provider 的组合。
///
/// 目前只有一条硬冲突：**Responses + Anthropic**。
/// Anthropic 走 `/messages` 与 content blocks，不支持 Responses 的 `input` /
/// `output_text` 形态；这个组合不可能成功，必须在发请求前就拒绝，并同时点出
/// 两个字段名，否则用户只会看到 404。
pub fn validate(wire: WireApi, provider_is_anthropic: bool) -> Result<(), String> {
    if wire == WireApi::Responses && provider_is_anthropic {
        return Err(format!(
            "wire_api=\"{}\" 与 provider=\"anthropic\" 不兼容：Anthropic 使用 /messages 协议。\
             请把 wire_api 改为 \"chat\"（当前 provider 只能走 Chat Completions 形态）。",
            wire.as_key()
        ));
    }
    Ok(())
}

/// 归一化 base_url 并拼接 Responses 路径。
///
/// 与 `build_chat_url` 共用同一套归一化规则（trim 空白与末尾 `/`；已以 `/v1`
/// 结尾则直接追加，否则补 `/v1`），避免出现 `/v1/v1/responses` 这类 404 ——
/// 那个坑在 chat 路径上已经踩过一次（Ollama `:11434/v1`、OpenRouter `/api/v1`）。
pub fn responses_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/responses")
    } else {
        format!("{base}/v1/responses")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_chat_so_existing_configs_keep_working() {
        // 存量零迁移的硬要求：缺字段必须等于 Chat，不能等于 Responses。
        assert_eq!(WireApi::default(), WireApi::Chat);
        assert_eq!(
            serde_json::from_str::<WireApi>("\"chat\"").unwrap(),
            WireApi::Chat
        );
    }

    #[test]
    fn key_round_trips_through_serde() {
        for api in [WireApi::Chat, WireApi::Responses] {
            let json = serde_json::to_string(&api).unwrap();
            assert_eq!(json, format!("\"{}\"", api.as_key()));
            assert_eq!(WireApi::from_key(api.as_key()), Some(api));
        }
    }

    #[test]
    fn from_key_accepts_aliases_but_rejects_typos() {
        for alias in ["chat", "CHAT", " Chat ", "chat_completions", "completions"] {
            assert_eq!(WireApi::from_key(alias), Some(WireApi::Chat), "{alias}");
        }
        for alias in ["responses", "Responses", "responses_api", "response"] {
            assert_eq!(WireApi::from_key(alias), Some(WireApi::Responses), "{alias}");
        }
        // 拼写错误必须是 None（让调用方报错），不能静默回落到 chat。
        for typo in ["respones", "respons", "", "gpt", "chat-completion!"]
        {
            assert_eq!(WireApi::from_key(typo), None, "{typo}");
        }
    }

    #[test]
    fn responses_url_normalises_like_chat_url() {
        // 与 build_chat_url 同规则：已含 /v1 不重复拼，末尾斜杠归一
        assert_eq!(
            responses_url("https://api.openai.com"),
            "https://api.openai.com/v1/responses"
        );
        assert_eq!(
            responses_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/responses"
        );
        assert_eq!(
            responses_url("https://api.openai.com/v1/"),
            "https://api.openai.com/v1/responses"
        );
        assert_eq!(
            responses_url("  https://openrouter.ai/api/v1  "),
            "https://openrouter.ai/api/v1/responses"
        );
        // 反例守卫：绝不出现 /v1/v1/
        assert!(!responses_url("https://x/v1").contains("/v1/v1"));
    }

    #[test]
    fn anthropic_plus_responses_is_rejected_with_both_field_names() {
        let err = validate(WireApi::Responses, true).unwrap_err();
        assert!(err.contains("wire_api"), "错误消息必须点出 wire_api: {err}");
        assert!(
            err.contains("anthropic"),
            "错误消息必须点出 anthropic: {err}"
        );
        assert!(
            err.contains("chat"),
            "错误消息必须给出修复示例（改成 chat）: {err}"
        );

        // 其余组合放行
        assert!(validate(WireApi::Responses, false).is_ok());
        assert!(validate(WireApi::Chat, true).is_ok());
        assert!(validate(WireApi::Chat, false).is_ok());
    }
}
