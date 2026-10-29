//! LLM 调用封装
//!
//! 统一封装 LLM API 调用，支持 OpenAI 兼容接口。
//! 提供非流式和流式两种完成方式，以及 token 估算和错误重试能力。

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::types::Message;

use crate::cache::{CacheEntry, PrefixCache};

/// 探针/观测输出开关的环境变量名。
///
/// 这些 `[PROBE-cache]` / `[CACHE-OBS]` / `[PROBE-cc]` 行是**开发期诊断信息**，
/// 不是产品输出：公开产品默认必须完全静默，否则首屏体验会被调试噪音污染
/// （P0-B 离线首启体验）。需要排障时显式设置 `MARGUERITE_DEBUG_PROBE=1` 恢复。
pub const DEBUG_PROBE_ENV: &str = "MARGUERITE_DEBUG_PROBE";

/// 解析探针开关取值（纯函数，便于单测）：`1` / `true` / `yes` / `on`（大小写不敏感、
/// 允许首尾空白）为开；缺失、空串及其它取值一律为关（**默认静默**）。
fn probe_flag_from_value(raw: Option<&str>) -> bool {
    let Some(v) = raw else {
        return false;
    };
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// 探针是否启用（进程内缓存一次；默认 false = 完全不输出探针行）。
pub fn probe_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        probe_flag_from_value(std::env::var(DEBUG_PROBE_ENV).ok().as_deref())
    })
}

/// 已输出的探针行数（仅用于单测断言「默认零输出」）。
static PROBE_LINE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// 探针输出唯一出口：未启用时**不写任何字节**（连格式化都不做）。
pub fn probe_write(args: std::fmt::Arguments<'_>) {
    if probe_enabled() {
        PROBE_LINE_COUNT.fetch_add(1, Ordering::Relaxed);
        eprintln!("{args}");
    }
}

/// 探针行计数（单测用）。
#[cfg(test)]
fn probe_line_count() -> usize {
    PROBE_LINE_COUNT.load(Ordering::Relaxed)
}

/// 写一行探针输出（默认静默，`MARGUERITE_DEBUG_PROBE=1` 时输出到 stderr）。
macro_rules! probe_log {
    ($($arg:tt)*) => {
        $crate::model::llm::probe_write(format_args!($($arg)*))
    };
}

/// LLM 提供商
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum LLMProvider {
    /// OpenAI 兼容 API
    OpenAI,
    /// DeepSeek API
    #[default]
    DeepSeek,
    /// Anthropic Messages API（x-api-key/Anthropic-Version；原生 content blocks/tool_use）
    Anthropic,
    /// Anthropic Messages **兼容网关**：协议与 `Anthropic` 完全相同
    /// （`/v1/messages` + content blocks + tool_use），差别只在**鉴权头**用
    /// `Authorization: Bearer` 而不是 `x-api-key`。
    ///
    /// MiniMax / Kimi / 腾讯 Token 计划 等第三方网关把 Anthropic Messages 协议
    /// 暴露在自建路径上，却沿用 OpenAI 风格的密钥头。分成两个变体，才能只换鉴权头、
    /// 不换 URL 与请求体 —— 把它们合并成一个会让其中一半必然 401。
    AnthropicCompatible,
    /// 自定义 API
    #[serde(rename = "custom")]
    Custom {
        /// 自定义 base URL
        base_url: String,
    },
}

/// Anthropic Messages 端点的鉴权头形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnthropicAuth {
    /// `x-api-key: <key>`（Anthropic 官方、腾讯 TokenHub 按量付费端点）。
    #[default]
    XApiKey,
    /// `Authorization: Bearer <key>`（MiniMax / Kimi / 腾讯 Token 计划 等兼容网关）。
    Bearer,
}

impl LLMProvider {
    /// 是否走 Anthropic Messages 协议（`/v1/messages` + content blocks + tool_use）。
    pub fn speaks_anthropic_messages(&self) -> bool {
        matches!(
            self,
            LLMProvider::Anthropic | LLMProvider::AnthropicCompatible
        )
    }

    /// Anthropic Messages 端点的鉴权头形态。
    ///
    /// 非 Anthropic 端点返回 `XApiKey` 但**不会被用到** —— 调用方必须先判
    /// [`LLMProvider::speaks_anthropic_messages`]。
    pub fn anthropic_auth(&self) -> AnthropicAuth {
        match self {
            LLMProvider::AnthropicCompatible => AnthropicAuth::Bearer,
            _ => AnthropicAuth::XApiKey,
        }
    }
}

/// LLM 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMConfig {
    /// 提供商
    #[serde(default)]
    pub provider: LLMProvider,
    /// 模型名称
    #[serde(default = "default_model")]
    pub model: String,
    /// API 密钥
    #[serde(default)]
    pub api_key: String,
    /// API 基础 URL
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// 最大生成 token 数
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// 采样温度
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// 请求超时时间（秒）
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// 思考深度档位（DeepSeek-V4-Pro 专项）：Off 不注入；其余映射为 `reasoning_effort`。
    #[serde(default)]
    pub thinking: crate::model::thinking::ThinkingEffort,
    /// 上下文窗口（1M 长任务模式用；供压缩/压缩率决策参考）
    #[serde(default = "default_context_window")]
    pub context_window: u32,
    /// 请求协议：`chat`（Chat Completions，默认）或 `responses`（Responses API）。
    ///
    /// 与 Codex 的 `wire_api` 同语义。**缺省必须是 `Chat`**：全部内置档案与本地
    /// 端点（Ollama / vLLM / LM Studio）只有 `/chat/completions`，默认值一变就会
    /// 让存量用户全部 404。`#[serde(default)]` 保证旧 `models.json` / `config.yaml`
    /// 缺字段时行为与改动前逐字节一致。
    #[serde(default)]
    pub wire_api: crate::model::wire::WireApi,
}

fn default_context_window() -> u32 {
    crate::model::thinking::CONTEXT_WINDOW_DEFAULT
}

fn default_model() -> String {
    "deepseek-v4-pro".to_string()
}

fn default_base_url() -> String {
    "https://api.deepseek.com".to_string()
}

fn default_max_tokens() -> u32 {
    4096
}

fn default_temperature() -> f32 {
    0.7
}

/// provider 读超时的默认值。
///
/// 旧默认 60s 是 `read_timeout`（连续 60s 没有任何字节才判死）——对长思考模型
/// 偏紧：模型在 reasoning 阶段可能长时间不吐 token，60s 就被判超时，用户看到的是
/// 「请求超时」而不是慢。这里跟自适应预算的 Standard 档对齐（180s）；
/// 用户显式配置 `model.timeout_secs` 时仍以配置为准。
pub(crate) fn default_timeout_secs() -> u64 {
    crate::agent::budget_policy::resolve("[[budget:standard]]", 1, 0).llm_read_timeout_secs
}

impl Default for LLMConfig {
    fn default() -> Self {
        Self {
            provider: LLMProvider::default(),
            model: default_model(),
            api_key: String::new(),
            base_url: default_base_url(),
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            timeout_secs: default_timeout_secs(),
            thinking: crate::model::thinking::ThinkingEffort::Off,
            context_window: default_context_window(),
            wire_api: crate::model::wire::WireApi::default(),
        }
    }
}

impl LLMConfig {
    /// 从主应用配置的模型段构建 LLM 客户端配置（Bloom 并行编排等复用同一后端）。
    /// provider 解析与 ModelRouter 保持一致（openai/deepseek/其余视为 custom）。
    pub fn from_model_config(config: &crate::persistence::config::Config) -> Self {
        let m = &config.model;
        Self {
            // provider id → 协议走唯一事实源（`model::provider_protocol`）。
            provider: crate::model::provider_protocol::provider_for_id(&m.provider, &m.base_url),
            model: m.model_name.clone(),
            api_key: m.api_key.clone(),
            base_url: m.base_url.clone(),
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            timeout_secs: if config.model.timeout_secs > 0 {
                config.model.timeout_secs
            } else {
                default_timeout_secs()
            },
            thinking: crate::model::thinking::ThinkingEffort::Max,
            context_window: 256 * 1024,
            // 协议来自 `config.yaml` 的 `[model] wire_api`；缺省 Chat。
            wire_api: m.wire_api,
        }
    }

    /// 从主配置的副模型段构建 Copilot 客户端配置。
    ///
    /// 副模型未配置（`copilot_model` 为空）或未给出模型名时返回 `None`，
    /// 调用方据此退化为「同模型审校」或单模型编排。副模型缺省字段继承主模型。
    pub fn copilot_from_config(config: &crate::persistence::config::Config) -> Option<Self> {
        let m = &config.model;
        let cop = m.copilot_model.as_ref()?;
        if cop.model_name.trim().is_empty() {
            return None;
        }
        let provider = cop.provider.clone().unwrap_or_else(|| m.provider.clone());
        let base_url = cop.base_url.clone().unwrap_or_else(|| m.base_url.clone());
        let api_key = cop.api_key.clone().unwrap_or_else(|| m.api_key.clone());
        Some(Self {
            provider: crate::model::provider_protocol::provider_for_id(&provider, &base_url),
            model: cop.model_name.clone(),
            api_key,
            base_url,
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            timeout_secs: if config.model.timeout_secs > 0 {
                config.model.timeout_secs
            } else {
                default_timeout_secs()
            },
            thinking: crate::model::thinking::ThinkingEffort::Off,
            context_window: default_context_window(),
            // 副模型没有独立的协议字段，继承主模型：副模型通常是同一家的另一个
            // 型号，协议一致；刻意不猜"副模型走别的协议"（那需要独立配置项）。
            wire_api: m.wire_api,
        })
    }
}

/// Token 用量
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    /// 提示 token 数
    pub prompt_tokens: u32,
    /// 补全 token 数
    pub completion_tokens: u32,
    /// 总 token 数
    pub total_tokens: u32,
    /// 服务端前缀缓存命中 token（DeepSeek `prompt_cache_hit_tokens` / OpenAI `cache_read_input_tokens`）。
    /// 仅当服务商返回该字段时非零；用于如实计量服务端 KV 缓存命中率。
    #[serde(default)]
    pub cache_read_input_tokens: u32,
    /// 服务端前缀缓存写入 token（首次为该前缀建立缓存时计费的部分）。
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
}

/// 工具定义（发送给 LLM 的 function calling 描述，OpenAI 兼容格式）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    /// 工具名称
    pub name: String,
    /// 工具描述
    pub description: String,
    /// JSON Schema 参数描述
    pub parameters: serde_json::Value,
}

/// LLM 返回的工具调用（已解析；`arguments` 为 JSON 字符串，调用方需自行解析为结构化参数）
#[derive(Debug, Clone)]
pub struct ToolCall {
    /// 调用 id（部分 provider 提供，用于多轮工具消息关联）
    pub id: Option<String>,
    /// 工具名称
    pub name: String,
    /// 参数（JSON 字符串）
    pub arguments: String,
}

/// 兼容层（P2-B）：把"把动作写在 content 里"的模型输出，桥接成标准 `tool_calls`。
///
/// 为什么需要：有些本地权重（例如本项目自训的 OmniPilot LoRA）不走 OpenAI 的
/// `tool_calls` 字段，而是把动作以 JSON 写在 content 里，例如
/// `{"thought":"…","tool":"web_search","args":{"query":"…"}}`。
/// Ollama 的原生 tool 模板对这类权重不生效 → `tool_calls` 恒为空 →
/// 工作台会把模型当成"只是在聊天"，一个工具都不调用（实测就是这么失效的）。
///
/// 识别两种常见写法：`{"tool":…,"args":…}` 与 `{"name":…,"arguments":…}`；
/// 含 `final` 的视为正常答复，不合成（调用方按最终答案处理）。
pub fn bridge_content_action(content: &str) -> Option<ToolCall> {
    let raw = content.trim();
    // 1) 标准 Qwen / Ollama 工具调用块：
    //      <tool_call>{"name": "…", "arguments": {…}}</tool_call>
    //    自训权重按官方 qwen2.5 模板输出这种形式；当 Ollama 没有把它解析成原生
    //    `tool_calls`（例如请求未带 tools、或模板未被识别）时，它会以**纯文本**到达
    //    这里，必须由本函数代解析，否则工作台会把动作当成聊天，一个工具都不调。
    //    块前允许有一句自然语言理由（Qwen 模板即如此）。
    if let Some(start) = raw.find("<tool_call>") {
        let after = &raw[start + "<tool_call>".len()..];
        let inner = after.split("</tool_call>").next().unwrap_or(after).trim();
        if let Some(tc) = parse_action_json(inner) {
            return Some(tc);
        }
    }
    // 2) 纯 JSON / ```json 包裹（历史协议与其它模型）
    let body = if let Some(rest) = raw.strip_prefix("```") {
        let rest = rest
            .strip_prefix("json")
            .or_else(|| rest.strip_prefix("JSON"))
            .unwrap_or(rest);
        rest.trim().strip_suffix("```").unwrap_or(rest).trim()
    } else {
        raw
    };
    parse_action_json(body)
}

/// 把一段 JSON 文本解析成工具调用；含 `final` 或没有工具名时返回 None（视为答复）。
fn parse_action_json(text: &str) -> Option<ToolCall> {
    let value: serde_json::Value = serde_json::from_str(text.trim()).ok()?;
    let obj = value.as_object()?;
    if obj.contains_key("final") {
        return None; // 正常答复，不是动作
    }
    let name = obj
        .get("tool")
        .or_else(|| obj.get("name"))
        .or_else(|| obj.get("tool_name"))
        .and_then(|v| v.as_str())?
        .trim()
        .to_string();
    if name.is_empty() {
        return None;
    }
    let args = obj
        .get("args")
        .or_else(|| obj.get("arguments"))
        .or_else(|| obj.get("parameters"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    Some(ToolCall {
        id: Some("omnipilot-bridged".to_string()),
        name,
        arguments: args.to_string(),
    })
}

#[cfg(test)]
mod bridge_tests {
    use super::bridge_content_action;

    /// 标准 Qwen 工具调用块（块前可有一句理由）——自训权重的主路径
    #[test]
    fn bridges_qwen_tool_call_block() {
        let s = "先打开目标网页\n<tool_call>\n\
                 {\"name\": \"browser_goto\", \"arguments\": {\"url\": \"https://example.com\"}}\n\
                 </tool_call>";
        let tc = bridge_content_action(s).expect("应能解析 <tool_call> 块");
        assert_eq!(tc.name, "browser_goto");
        assert!(tc.arguments.contains("example.com"));
    }

    /// 历史协议：直接输出 {"tool":…,"args":…}
    #[test]
    fn bridges_legacy_json() {
        let tc = bridge_content_action(
            "{\"tool\": \"file_write\", \"args\": {\"path\": \"a.txt\", \"content\": \"x\"}}",
        )
        .expect("应能解析旧协议 JSON");
        assert_eq!(tc.name, "file_write");
    }

    /// 纯自然语言答复 / {"final":…} 都不该被当成动作
    #[test]
    fn ignores_plain_reply_and_final() {
        assert!(bridge_content_action("已完成，三条要点已写入文件。").is_none());
        assert!(bridge_content_action("{\"final\": \"done\"}").is_none());
    }
}

/// LLM 响应
#[derive(Debug, Clone)]
pub struct LLMResponse {
    /// 回复内容
    pub content: String,
    /// 思考过程（如果有）
    pub thinking: Option<String>,
    /// 使用的模型
    pub model: String,
    /// Token 用量
    pub usage: TokenUsage,
    /// 模型请求的工具调用（function calling）；空表示无
    pub tool_calls: Vec<ToolCall>,
}

/// 流式响应块
#[derive(Debug, Clone)]
pub struct LLMChunk {
    /// 增量内容
    pub content: Option<String>,
    /// 增量思考过程
    pub thinking: Option<String>,
    /// 结束原因
    pub finish_reason: Option<String>,
    /// 本块携带（或流末一次性给出）的工具调用；流式场景下在流末完整给出
    pub tool_calls: Option<Vec<ToolCall>>,
    /// P2-9：流末（或本块）一次性给出的 token 用量，供 turn 预算熔断记账。
    pub usage: Option<TokenUsage>,
}

/// 流式接收器
pub type StreamReceiver = mpsc::Receiver<Result<LLMChunk>>;

/// 重试配置
///
/// 次数不再写死：取自适应预算的 Standard 档（3 次，与历史行为一致），
/// `MARGUERITE_LLM_MAX_RETRIES` 可覆盖，硬上限受 `budget_policy::MAX_RETRIES` 约束。
fn max_retries() -> u32 {
    crate::agent::budget_policy::default_max_retries()
}
const BASE_DELAY_MS: u64 = 1000;

/// 判断错误是否可重试
fn is_retryable_error(status: Option<u16>) -> bool {
    matches!(status, Some(429) | Some(502) | Some(503) | Some(504) | None)
}

/// 归类构造 provider 错误（P0-4）。
///
/// 上游原始响应正文（如 `Your api key: ****gEni is invalid`）只进
/// [`ProviderError::detail`]（并写入日志供诊断），**不**作为用户可见文案；
/// 用户文案由 `ProviderError` 的中性 `safe_message` 提供，前端再按错误码本地化。
fn provider_http_error(status: u16, error_text: String) -> anyhow::Error {
    let class = crate::model::error_taxonomy::ProviderErrorClass::from_status(status);
    // 原始正文仅入日志（warn 级），便于排障；不进入返回错误的 Display。
    tracing::warn!(
        status,
        class = class.code(),
        detail = %error_text,
        "model provider request failed"
    );
    anyhow::Error::new(crate::model::error_taxonomy::ProviderError::from_status(
        status, error_text,
    ))
}

/// 归类构造网络层失败（连接失败 / 超时 / 中途断开，无 HTTP 状态码）。
fn provider_network_error(detail: String) -> anyhow::Error {
    tracing::warn!(class = "NETWORK", detail = %detail, "model provider network failure");
    anyhow::Error::new(crate::model::error_taxonomy::ProviderError::new(
        crate::model::error_taxonomy::ProviderErrorClass::Network,
        detail,
    ))
}

/// 归类构造**超时**失败（区别于普通网络失败，给出稳定的 `TIMEOUT` 码，
/// 前端可本地化为「请求超时」而非笼统的网络错误）。
fn provider_timeout_error(detail: String) -> anyhow::Error {
    tracing::warn!(class = "TIMEOUT", detail = %detail, "model provider request timed out");
    anyhow::Error::new(crate::model::error_taxonomy::ProviderError::new(
        crate::model::error_taxonomy::ProviderErrorClass::Timeout,
        detail,
    ))
}

/// LLM 客户端
#[derive(Debug, Clone)]
pub struct LLMClient {
    /// 客户端配置
    pub config: LLMConfig,
    /// HTTP 客户端
    client: reqwest::Client,
    /// 可选客户端缓存（KV-cache 极简实践：精确前缀命中 + 命中率计量）
    cache: Option<std::sync::Arc<PrefixCache>>,
}

/// 进程级 `reqwest::Client` 连接池缓存：按 `(base_url, timeout_secs)` 复用。
///
/// 关键收益：(1) 大幅减少高频 LLM 调用下的 socket 创建/握手开销；(2) 复用已建立的
/// TCP 连接，规避 Windows loopback 上「关闭连接后再 connect 同一端口」偶发卡死
/// （每次新建 client 必然触发一次新 connect，对 mock/本地后端尤为致命）。
/// 进程级 `reqwest::Client` 连接池缓存：按 `(base_url, timeout_secs)` 复用。
fn shared_client_cache() -> &'static RwLock<HashMap<String, reqwest::Client>> {
    static CACHE: OnceLock<RwLock<HashMap<String, reqwest::Client>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// 建立 TCP 连接的上限（秒）。与「整包耗时」解耦：远端后端偶发慢握手时，
/// 不再被整包超时一并判死。
const CONNECT_TIMEOUT_SECS: u64 = 10;

fn build_shared_client(config: &LLMConfig) -> reqwest::Client {
    // 复用键 = `(base_url, timeout_secs)`。
    //
    // ⚠️ 历史缺陷（真机踩过，是最高频失败的主因）：这里曾**只**用 `base_url`
    // 作键，而文档注释却写着按 `(base_url, timeout_secs)` 复用 —— 注释与实现矛盾，
    // 后果是「同一 base_url 下，不同 timeout 取**首个**创建者为准」，于是：
    //   1. 用户在设置里把超时从 60s 调到 300s，**在进程存活期内永不生效**；
    //   2. 测试/单测里对 `config.model.timeout_secs` 的临时覆盖会**泄漏**进
    //      生产流量（同一 base_url 共用同一 client）。
    // 原注释给出的理由「对本地/同源后端可接受」不成立 —— 主后端是**远端**
    // `https://api.deepseek.com`。
    let key = format!("{}|{}", config.base_url, config.timeout_secs);
    let cache = shared_client_cache();
    if let Some(c) = cache.read().ok().and_then(|g| g.get(&key).cloned()) {
        probe_log!("[PROBE-cache] HIT base={}", key);
        return c;
    }
    probe_log!("[PROBE-cache] MISS base={}", key);
    let built = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        // 用 **读空闲**上限（每次成功读到数据即重置），而不是整包总超时。
        //
        // 此前用 `.timeout(timeout_secs)`（reqwest 的**整包**超时，含读体）：
        // 对「持续在出 token 的长推理」是致命的 —— 200 已返回、读体中途被掐，
        // 上层看到的就是 `response decode error (200): error decoding response body`
        // （实测最高频失败，142 次 plan_step_failed 的主因；且重试 3 次同样撞墙）。
        // `read_timeout` 只在**真正卡住**（N 秒内一个字节都没来）时才判死，
        // 既救回长推理，又不放松对僵死连接的兜底。
        .read_timeout(Duration::from_secs(config.timeout_secs))
        .build()
        .unwrap_or_default();
    if let Ok(mut g) = cache.write() {
        g.entry(key).or_insert_with(|| built.clone());
    }
    built
}

/// 归一化 `base_url` 并拼接 chat/completions 路径（纯函数，便于表驱动单测）。
///
/// 旧 `chat_url` 恒拼 `{base}/v1/chat/completions`，于是 base 若已含 `/v1`（Ollama
/// 默认 `http://localhost:11434/v1`、OpenRouter `https://openrouter.ai/api/v1`）会拼出
/// `/v1/v1/...` 或 `/api/v1/v1/...` 的 404 —— 用户选中本地模型后真实发对话 404 即此因。
///
/// 规则：先 trim 空白与末尾 `/`；base 已以 `/v1` 结尾（含 `/api/v1`，它同样以 `/v1`
/// 结尾）时直接追加 chat 路径，否则补 `/v1`。Anthropic 走 `/messages`，OpenAI 兼容走
/// `/chat/completions`。
pub fn build_chat_url(base_url: &str, is_anthropic: bool) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let suffix = if is_anthropic {
        "messages"
    } else {
        "chat/completions"
    };
    if base.ends_with("/v1") {
        format!("{base}/{suffix}")
    } else {
        format!("{base}/v1/{suffix}")
    }
}

impl LLMClient {
    /// 创建新的 LLM 客户端
    pub fn new(config: LLMConfig) -> Self {
        let client = build_shared_client(&config);
        Self {
            config,
            client,
            cache: None,
        }
    }

    /// 挂载客户端缓存（KV-cache 极简实践：同前缀请求精确命中，省去重复预填充）
    pub fn with_cache(mut self, cache: std::sync::Arc<PrefixCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    /// 依据任务类型做模型层自适应（DeepSeek-V4-Pro 专项）：
    /// 用 [`ThinkingProfile`](crate::model::thinking::ThinkingProfile) 覆盖 `max_tokens`、
    /// `thinking` 与 `context_window`。对 `Long` 任务自动开启 1M 上下文档。
    pub fn with_adaptive_profile(mut self, kind: crate::model::thinking::TaskKind) -> Self {
        let p = crate::model::thinking::profile_for(kind);
        self.config.max_tokens = p.max_tokens;
        self.config.thinking = p.effort;
        self.config.context_window = p.context_window;
        self
    }

    /// 设置思考预算档位（Track A 路由：thinking_mode T0/T1 → ThinkingEffort）。
    /// 镜像 `with_adaptive_profile`，但仅覆盖思考预算，不改动 max_tokens / context_window。
    pub fn with_thinking(mut self, effort: crate::model::thinking::ThinkingEffort) -> Self {
        self.config.thinking = effort;
        self
    }

    /// 覆盖采样温度（模式规格接线：`BloomSpec::temperature`，MAX 档 0.05~0.1 收敛更稳）。
    ///
    /// 历史缺陷：`BloomSpec.temperature` 是纯展示字段，Bloom 实际以默认 0.7 采样，
    /// 与「MAX 收敛更稳」的声明相反。此处补上覆盖入口。
    /// 入参被钳到 `[0.0, 2.0]`（OpenAI 兼容区间），非法值（NaN）回落为不改动。
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        if temperature.is_finite() {
            self.config.temperature = temperature.clamp(0.0, 2.0);
        }
        self
    }

    /// 覆盖输出预算上限（锚定 bootstrap 通道：首个请求注入、之后剥离）。
    /// 仅改 `max_tokens`，其余配置（thinking/context_window）保持原样。
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.config.max_tokens = max_tokens;
        self
    }

    /// 极限思考（HLE / Agents' Last Exam 等难推理）：开 Max 思考 + 大思考预算。
    pub fn with_max_thinking(mut self) -> Self {
        self.config.thinking = crate::model::thinking::ThinkingEffort::Max;
        self.config.max_tokens = crate::model::thinking::MAX_TOKENS_BIG;
        self
    }

    /// 1M 超长上下文长任务模式：大上下文 + 大输出，配合精细压缩处理长任务。
    pub fn with_long_context(mut self) -> Self {
        self.config.thinking = crate::model::thinking::ThinkingEffort::Deep;
        self.config.max_tokens = crate::model::thinking::MAX_TOKENS_LARGE;
        self.config.context_window = crate::model::thinking::CONTEXT_WINDOW_1M;
        self
    }

    /// 落盘客户端缓存（W-7b）：若挂载了 `PrefixCache` 则序列化到 JSON 文件（原子写），
    /// 使冷上下文跨进程重启持久化；无缓存时静默成功。
    pub fn save_cache(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(cache) = &self.cache {
            cache.save(path)?;
        }
        Ok(())
    }

    /// 从磁盘并入冷上下文缓存（W-7b 加载端接线）：条目与已观测前缀合并进既有缓存
    /// （缺失/损坏静默忽略）；无缓存时静默成功。
    pub fn load_cache(&self, path: &std::path::Path) {
        if let Some(cache) = &self.cache {
            cache.absorb(path);
        }
    }

    /// 是否为 Anthropic Messages API（x-api-key / content blocks / tool_use）
    fn is_anthropic(&self) -> bool {
        self.config.provider.speaks_anthropic_messages()
    }

    /// 构建请求 URL（委托给纯函数 [`build_chat_url`]，便于表驱动单测）。
    ///
    /// 按 `wire_api` 分派：
    /// - `Chat`（默认）→ `/chat/completions`（Anthropic 走 `/messages`），**行为与引入该字段前逐字节一致**；
    /// - `Responses` → `/v1/responses`。
    fn chat_url(&self) -> String {
        match self.config.wire_api {
            crate::model::wire::WireApi::Chat => {
                build_chat_url(&self.config.base_url, self.is_anthropic())
            }
            crate::model::wire::WireApi::Responses => {
                crate::model::wire::responses_url(&self.config.base_url)
            }
        }
    }

    /// 构建请求头
    fn headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        if self.is_anthropic() {
            // 同一套 Anthropic Messages 协议，鉴权头按服务商分两种：
            // 官方与腾讯 TokenHub 用 `x-api-key`；MiniMax / Kimi 等兼容网关用 Bearer。
            match self.config.provider.anthropic_auth() {
                AnthropicAuth::XApiKey => {
                    if let Ok(v) = self.config.api_key.parse() {
                        headers.insert("x-api-key", v);
                    }
                }
                AnthropicAuth::Bearer => {
                    headers.insert(
                        reqwest::header::AUTHORIZATION,
                        format!("Bearer {}", self.config.api_key).parse().unwrap(),
                    );
                }
            }
            if let Ok(v) = "2023-06-01".parse() {
                headers.insert("anthropic-version", v);
            }
        } else {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.config.api_key).parse().unwrap(),
            );
        }
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        headers
    }

    /// 将 Message 转换为 API 请求格式
    ///
    /// OpenAI function-calling 契约要求：工具结果消息（role=tool）前必须有一条
    /// assistant 消息携带 `tool_calls` 数组，且每条 tool 消息须有匹配的 `tool_call_id`。
    /// 内部 Message 模型不显式存储 assistant 的 tool_calls，此处按连续 Tool 段
    /// 合成一条 assistant tool_calls 消息，保证 API 400 不会因缺字段而失败。
    fn messages_to_api(&self, messages: &[Message]) -> Vec<serde_json::Value> {
        let mut out: Vec<serde_json::Value> = Vec::with_capacity(messages.len() + 4);
        let mut i = 0;
        let mut tool_seq: u64 = 0;
        while i < messages.len() {
            match &messages[i] {
                Message::Tool { .. } => {
                    // 检查前一条是否是带 reasoning_content 的空 Assistant 消息（工具回路中保存的思考）
                    let pending_reasoning = if let Some(prev) = out.last() {
                        if prev["role"] == "assistant"
                            && prev["content"].as_str().unwrap_or("").is_empty()
                        {
                            prev.get("reasoning_content").cloned()
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if pending_reasoning.is_some() {
                        out.pop(); // 移除空 assistant 消息，reasoning_content 合并到 tool_calls 消息
                    }
                    // 收集连续 Tool 段
                    let mut segment: Vec<(String, String, String)> = Vec::new();
                    while i < messages.len() {
                        if let Message::Tool {
                            name,
                            input,
                            output,
                            ..
                        } = &messages[i]
                        {
                            segment.push((name.clone(), input.clone(), output.clone()));
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    // 合成 assistant.tool_calls 消息（含 reasoning_content）
                    let mut calls = Vec::with_capacity(segment.len());
                    for (tname, tinput, _) in &segment {
                        tool_seq += 1;
                        let safe_name: String = tname
                            .chars()
                            .map(|c| {
                                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                                    c
                                } else {
                                    '_'
                                }
                            })
                            .collect();
                        let call_id = format!("call_{tool_seq:04x}_{safe_name}");
                        calls.push(serde_json::json!({
                            "id": call_id,
                            "type": "function",
                            "function": {
                                "name": safe_name,
                                "arguments": if tinput.is_empty() { "{}".to_string() } else { tinput.clone() }
                            }
                        }));
                    }
                    let mut assistant_msg = serde_json::json!({
                        "role": "assistant",
                        "content": null,
                        "tool_calls": calls
                    });
                    if let Some(rc) = pending_reasoning {
                        assistant_msg["reasoning_content"] = rc;
                    }
                    out.push(assistant_msg);
                    // 每条 tool 结果消息
                    for (idx, (tname, tinput, toutput)) in segment.iter().enumerate() {
                        let safe_name: String = tname
                            .chars()
                            .map(|c| {
                                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                                    c
                                } else {
                                    '_'
                                }
                            })
                            .collect();
                        let call_id = format!(
                            "call_{:04x}_{}",
                            tool_seq - segment.len() as u64 + idx as u64 + 1,
                            safe_name
                        );
                        out.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "name": safe_name,
                            "content": format!("Input: {}\nOutput: {}", tinput, toutput)
                        }));
                    }
                }
                Message::User { content, .. } => {
                    out.push(serde_json::json!({"role": "user", "content": content}));
                    i += 1;
                }
                Message::Assistant {
                    content, thinking, ..
                } => {
                    let mut m = serde_json::json!({"role": "assistant", "content": content});
                    if let Some(t) = thinking {
                        m["reasoning_content"] = serde_json::Value::String(t.clone());
                    }
                    out.push(m);
                    i += 1;
                }
                Message::System { content, .. } => {
                    // DeepSeek / OpenAI 兼容端点均**自动**前缀缓存：相同前缀（≥1024 token、
                    // 逐字节一致）的重复请求由服务端复用 KV 块并按约 10% 单价计费，无需
                    // `cache_control` 提示。命中杠杆是「稳定且足够长的系统前缀」——
                    // 由 `context::optimize::reorder_stable_prefix` 与 `PrefixCache::note_prefix`
                    // 保证前缀前置且不变；此处只产出干净 payload，不注入非标准字段。
                    out.push(serde_json::json!({"role": "system", "content": content}));
                    i += 1;
                }
            }
        }
        out
    }

    /// 将 Message 转为 Anthropic Messages API 形态：返回 (system blocks, messages)。
    ///
    /// - System 独立出顶层 `system` 字段，且**返回 block 数组**：多条 System 用 `\n`
    ///   合并为**一个** text block（多个 block 会改变语义，也浪费断点配额）；
    /// - W5.4：该 block 挂 `cache_control: {"type":"ephemeral"}` —— Anthropic 的前缀
    ///   缓存是**显式断点**，没有它 system 前缀结构性不可缓存；
    /// - 工具回路中的 Assistant + Tool 段按 Anthropic 的 `tool_use` / `tool_result`
    ///   content blocks 合成（内部 Message 不存 tool_calls，按连续 Tool 段反向合成）；
    /// - 最后一条 user 消息的 content 改为 block 数组并挂断点（第 3 个断点；上限 4）。
    ///   断点**只**落在最后一条 user 上，绝不放在 history 中间。
    fn messages_to_anthropic(
        &self,
        messages: &[Message],
    ) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
        let mut system = String::new();
        let mut out: Vec<serde_json::Value> = Vec::with_capacity(messages.len() + 4);
        let mut i = 0;
        let mut tool_seq: u64 = 0;
        while i < messages.len() {
            match &messages[i] {
                Message::System { content, .. } => {
                    if !system.is_empty() {
                        system.push('\n');
                    }
                    system.push_str(content);
                    i += 1;
                }
                Message::User { content, .. } => {
                    out.push(serde_json::json!({ "role": "user", "content": content }));
                    i += 1;
                }
                Message::Assistant {
                    content, thinking, ..
                } => {
                    let mut blocks = Vec::new();
                    if let Some(t) = thinking {
                        if !t.trim().is_empty() {
                            blocks.push(serde_json::json!({ "type": "thinking", "thinking": t }));
                        }
                    }
                    if !content.trim().is_empty() {
                        blocks.push(serde_json::json!({ "type": "text", "text": content }));
                    }
                    out.push(serde_json::json!({ "role": "assistant", "content": blocks }));
                    i += 1;
                }
                Message::Tool { .. } => {
                    let mut segment: Vec<(String, String, String)> = Vec::new();
                    while i < messages.len() {
                        if let Message::Tool {
                            name,
                            input,
                            output,
                            ..
                        } = &messages[i]
                        {
                            segment.push((name.clone(), input.clone(), output.clone()));
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    let mut use_blocks = Vec::new();
                    let mut result_blocks = Vec::new();
                    for (tname, tinput, toutput) in &segment {
                        tool_seq += 1;
                        let safe_name: String = tname
                            .chars()
                            .map(|c| {
                                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                                    c
                                } else {
                                    '_'
                                }
                            })
                            .collect();
                        let id = format!("toolu_{tool_seq:04x}_{safe_name}");
                        let input_value = serde_json::from_str::<serde_json::Value>(tinput)
                            .unwrap_or_else(|_| serde_json::Value::String(tinput.clone()));
                        use_blocks.push(serde_json::json!({
                            "type": "tool_use",
                            "id": id,
                            "name": safe_name,
                            "input": input_value,
                        }));
                        result_blocks.push(serde_json::json!({
                            "type": "tool_result",
                            "tool_use_id": id,
                            "content": format!("Input: {}\nOutput: {}", tinput, toutput),
                        }));
                    }
                    // Anthropic 要求 tool_use 放在 assistant 消息内：优先附加到上一条 assistant。
                    let mut attached = false;
                    if let Some(last) = out.last_mut() {
                        if last["role"] == "assistant" {
                            if let Some(arr) = last
                                .get_mut("content")
                                .and_then(serde_json::Value::as_array_mut)
                            {
                                arr.extend(use_blocks.clone());
                                attached = true;
                            }
                        }
                    }
                    if !attached {
                        out.push(serde_json::json!({ "role": "assistant", "content": use_blocks }));
                    }
                    out.push(serde_json::json!({ "role": "user", "content": result_blocks }));
                }
            }
        }
        // W5.4 断点 3：最后一条 user 消息。Anthropic 最多 4 个断点，此处只用 3 个
        // （system / 最后一个 tool / 最后一条 user），且断点永远在**最后一条 user**，
        // 不放 history 中间 —— 中途断点会让后续 token 无法复用前面的缓存。
        if let Some(last_user) = out.iter_mut().rev().find(|m| m["role"] == "user") {
            if let Some(text) = last_user["content"].as_str() {
                let text = text.to_string();
                last_user["content"] = serde_json::json!([{
                    "type": "text",
                    "text": text,
                    "cache_control": { "type": "ephemeral" },
                }]);
            } else if let Some(blocks) = last_user["content"].as_array_mut() {
                // 工具回路里最后一条 user 是 tool_result block 数组：断点挂在最后一个
                // text block；若整段没有 text block（纯 tool_result），则挂在末块
                // （Anthropic 允许在 tool_result 上放断点，覆盖到该工具结果为止）。
                if let Some(last_text) = blocks
                    .iter_mut()
                    .rev()
                    .find(|block| block["type"] == "text")
                {
                    last_text["cache_control"] = serde_json::json!({ "type": "ephemeral" });
                } else if let Some(last_block) = blocks.last_mut() {
                    last_block["cache_control"] = serde_json::json!({ "type": "ephemeral" });
                }
            }
        }
        let system_blocks = if system.is_empty() {
            Vec::new()
        } else {
            vec![serde_json::json!({
                "type": "text",
                "text": system,
                "cache_control": { "type": "ephemeral" },
            })]
        };
        (system_blocks, out)
    }

    /// 构建请求体（统一入口：是否流式 + 是否带工具定义）
    fn build_body(
        &self,
        messages: &[Message],
        stream: bool,
        tools: Option<&[ToolDef]>,
    ) -> serde_json::Value {
        // Responses 协议：字段名与结构都与 Chat Completions 不同（不是别名）。
        // 先于 anthropic 分支判断：二者互斥，且 `validate()` 已在 URL 构建处拒绝
        // anthropic+responses 的组合（此处仍按 responses 处理，避免静默走错协议）。
        if matches!(self.config.wire_api, crate::model::wire::WireApi::Responses) {
            let (instructions, input) = self.messages_to_responses_input(messages);
            let mut body = serde_json::json!({
                "model": self.config.model,
                "input": input,
                // 上限字段名不同：Chat 用 max_tokens，Responses 用 max_output_tokens
                "max_output_tokens": self.config.max_tokens,
                "temperature": self.config.temperature,
                "stream": stream,
            });
            if !instructions.is_empty() {
                body["instructions"] = serde_json::Value::String(instructions);
            }
            // 思考档位：Responses 用 `reasoning.effort`（Chat 用顶层 reasoning_effort）
            // 本轮显式档位（UI 滑块）必须覆盖客户端配置：否则 `with_adaptive_profile`
            // 会把用户选的 deep 按任务分类改回 off，请求体里永远没有该字段。
            let effective = crate::model::thinking::current_turn_effort()
                .unwrap_or(self.config.thinking);
            if let Some(effort) = effective.reasoning_effort() {
                let supported = crate::model::thinking::supports_reasoning_effort(&self.config.model);
                if supported {
                    body["reasoning"] = serde_json::json!({ "effort": effort });
                }
                note_effort_wire(
                    &self.config.model,
                    effective,
                    supported,
                    "reasoning.effort",
                    supported.then_some(effort),
                );
            }
            if let Some(tools) = tools {
                if !tools.is_empty() {
                    // Responses 的工具是**扁平**结构（name 在顶层），
                    // 与 Chat 的 `{"type":"function","function":{...}}` 不同。
                    body["tools"] = serde_json::json!(tools
                        .iter()
                        .map(|t| serde_json::json!({
                            "type": "function",
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                        }))
                        .collect::<Vec<_>>());
                }
            }
            return body;
        }
        if self.is_anthropic() {
            let (system_blocks, api_messages) = self.messages_to_anthropic(messages);
            // W5.4：Anthropic 的前缀缓存是**显式断点**（cache_control），不是自动的。
            // 旧实现 system 是纯字符串、tools 无断点 → 结构性不可用（命中恒 0）。
            // 三个断点：system / 最后一个 tool / 最后一条 user（Anthropic 上限 4）。
            // system 与最后一条 user 的断点由 `messages_to_anthropic` 负责，
            // 这里只补 tools 上的那个。
            //
            // Anthropic 的扩展思考与 OpenAI 的 `reasoning_effort` **不是同一个字段**：
            // 请求体里是 `thinking: {type, budget_tokens}`，且开启后 temperature 必须缺省。
            let effective_thinking =
                crate::model::thinking::current_turn_effort().unwrap_or(self.config.thinking);
            let thinking_payload = anthropic_thinking_payload(
                &self.config.model,
                effective_thinking,
                self.config.max_tokens,
            );
            let max_tokens = thinking_payload
                .as_ref()
                .map(|(_, m)| *m)
                .unwrap_or(self.config.max_tokens);
            let mut body = serde_json::json!({
                "model": self.config.model,
                "messages": api_messages,
                "max_tokens": max_tokens,
                "stream": stream,
            });
            // Anthropic 开启扩展思考后不接受 temperature（必须缺省或为 1，否则 400）。
            if thinking_payload.is_none() {
                body["temperature"] = serde_json::json!(self.config.temperature);
            }
            if let Some((payload, _)) = thinking_payload {
                body["thinking"] = payload;
                note_effort_wire(
                    &self.config.model,
                    effective_thinking,
                    true,
                    "thinking.budget_tokens",
                    Some("enabled"),
                );
            } else {
                note_effort_wire(
                    &self.config.model,
                    effective_thinking,
                    supports_anthropic_thinking(&self.config.model),
                    "thinking.budget_tokens",
                    None,
                );
            }
            if !system_blocks.is_empty() {
                body["system"] = serde_json::Value::Array(system_blocks);
            }
            if let Some(tools) = tools {
                if !tools.is_empty() {
                    let mut api_tools: Vec<serde_json::Value> = tools
                        .iter()
                        .map(|t| {
                            serde_json::json!({
                                "name": t.name,
                                "description": t.description,
                                "input_schema": t.parameters,
                            })
                        })
                        .collect();
                    // 断点覆盖到最后一个工具：Anthropic 会缓存到该位置为止的全部前缀。
                    if let Some(last) = api_tools.last_mut() {
                        last["cache_control"] = serde_json::json!({ "type": "ephemeral" });
                    }
                    body["tools"] = serde_json::json!(api_tools);
                }
            }
            return body;
        }
        let mut body = serde_json::json!({
            "model": self.config.model,
            "messages": self.messages_to_api(messages),
            "max_tokens": self.config.max_tokens,
            "temperature": self.config.temperature,
            "stream": stream
        });
        // 流式请求时请求 API 返回 token 用量（OpenAI/DeepSeek 兼容）
        if stream {
            body["stream_options"] = serde_json::json!({ "include_usage": true });
        }
        // DeepSeek-V4-Pro 极限思考：思考档位开启且模型支持时注入 reasoning_effort
        // （与 mode/spec.rs 语义一致：low/medium/high）。不认识/不支持时仅本地 max_tokens 自适应，安全降级。
        // 本轮显式档位（UI 滑块）优先于客户端/自适应配置：所有请求路径最终都汇聚到
        // 这里，因此这是「档位一定生效」的唯一可靠单点（工具回路、纯文本、子代理同源）。
        let effective_thinking = crate::model::thinking::current_turn_effort()
            .unwrap_or(self.config.thinking);
        if let Some(effort) = effective_thinking.reasoning_effort() {
            let supported = crate::model::thinking::supports_reasoning_effort(&self.config.model);
            if supported {
                body["reasoning_effort"] = serde_json::Value::String(effort.to_string());
            }
            note_effort_wire(
                &self.config.model,
                effective_thinking,
                supported,
                "reasoning_effort",
                supported.then_some(effort),
            );
        } else {
            // Off 档：也必须留痕，否则「档位没效果」和「档位本来就是关」无法区分。
            note_effort_wire(&self.config.model, effective_thinking, true, "-", None);
        }
        if let Some(tools) = tools {
            if !tools.is_empty() {
                body["tools"] = serde_json::json!(tools
                    .iter()
                    .map(|t| serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters
                        }
                    }))
                    .collect::<Vec<_>>());
            }
        }
        body
    }

    /// 将内部 [`Message`] 映射为 Responses API 的 `input` 数组 + `instructions`。
    ///
    /// 与 `messages_to_api` 的关键差异（不是"换个字段名"那么简单）：
    ///
    /// | 内部 | Chat Completions | Responses |
    /// |---|---|---|
    /// | `System` | `messages[role=system]` | 顶层 `instructions` 字符串（多条按 `\n\n` 合并） |
    /// | `User` | `content: string` | `content: [{type:"input_text", text}]` |
    /// | `Assistant` | `content: string` | `content: [{type:"output_text", text}]` |
    /// | `Tool` | `role=tool` + 前置 assistant.tool_calls | `function_call` + `function_call_output` **成对** item |
    ///
    /// 内部 `Message::Tool` 不保存 `call_id`，因此这里**合成**稳定的
    /// `call_<序号>` 并把 `function_call` 与 `function_call_output` 配对写入 ——
    /// 与 `messages_to_api` 合成 assistant tool_calls 的做法同源。缺任一半都会
    /// 被服务端拒绝（"No tool call found for function call output"）。
    fn messages_to_responses_input(
        &self,
        messages: &[Message],
    ) -> (String, Vec<serde_json::Value>) {
        let mut instructions: Vec<String> = Vec::new();
        let mut input: Vec<serde_json::Value> = Vec::with_capacity(messages.len() + 4);
        let mut call_seq: u64 = 0;
        for m in messages {
            match m {
                Message::System { content, .. } => {
                    if !content.trim().is_empty() {
                        instructions.push(content.clone());
                    }
                }
                Message::User { content, .. } => {
                    input.push(serde_json::json!({
                        "role": "user",
                        "content": [{ "type": "input_text", "text": content }]
                    }));
                }
                Message::Assistant { content, .. } => {
                    // 空正文不写 item：Responses 对空 output_text 会报错，
                    // 而工具回路里的空 assistant 是正常的（只有 thinking）。
                    if !content.trim().is_empty() {
                        input.push(serde_json::json!({
                            "role": "assistant",
                            "content": [{ "type": "output_text", "text": content }]
                        }));
                    }
                }
                Message::Tool {
                    name, input: args, output, ..
                } => {
                    call_seq += 1;
                    let call_id = format!("call_{call_seq}");
                    input.push(serde_json::json!({
                        "type": "function_call",
                        "call_id": call_id,
                        "name": name,
                        "arguments": args,
                    }));
                    input.push(serde_json::json!({
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": output,
                    }));
                }
            }
        }
        (instructions.join("\n\n"), input)
    }

    /// 构建请求体（非流式，无工具）
    fn build_request_body(&self, messages: &[Message]) -> serde_json::Value {
        self.build_body(messages, false, None)
    }

    /// 构建流式请求体（无工具）
    fn build_stream_request_body(&self, messages: &[Message]) -> serde_json::Value {
        self.build_body(messages, true, None)
    }

    /// 按提供商解析非流式响应（Anthropic content blocks 形态 vs OpenAI/DeepSeek choices 形态）。
    ///
    /// # 为什么按**响应体形态**分派，而不是只按请求协议
    ///
    /// 线上实证（2026-10-21，api.apigoto.com）：同一个 `/v1/responses` 端点，
    /// 流式返回真 Responses（`object:"response"`），**非流式却返回 Chat Completions**
    /// （`object:"chat.completion"` + `choices[]`）——它只实现了一半，非流式走了
    /// chat 兼容层。
    ///
    /// 旧实现只认 `output[]`，遇到这种响应会产出**空 content + 全零 usage**，
    /// 且不报错：用户看到"模型什么都没说"，用量台账记 0。这是最坏的失败形态——
    /// 看起来成功，实际什么都没拿到。
    ///
    /// 判据：先看响应体有没有 `choices[]`（chat 形态）/ `output[]`（Responses 形态），
    /// 再决定解析器。请求协议是我们发的，响应形态是上游决定的，两者可能不一致。
    fn parse_response(&self, text: String) -> Result<LLMResponse> {
        if matches!(self.config.wire_api, crate::model::wire::WireApi::Responses) {
            // 兼容层探测：只认结构特征，不猜。
            let looks_like_chat = serde_json::from_str::<serde_json::Value>(&text)
                .map(|v| v.get("choices").and_then(|c| c.as_array()).is_some())
                .unwrap_or(false);
            if looks_like_chat {
                return parse_chat_response(text, &self.config.model);
            }
            parse_responses_response(text, &self.config.model)
        } else if self.is_anthropic() {
            parse_anthropic_response(text, &self.config.model)
        } else {
            parse_chat_response(text, &self.config.model)
        }
    }

    /// 非流式聊天完成
    pub async fn chat_complete(&self, messages: &[Message]) -> Result<LLMResponse> {
        let url = self.chat_url();
        let headers = self.headers();
        let body = self.build_request_body(messages);
        probe_log!(
            "[CACHE-OBS] chat_complete(no-cache passthrough) msg_count={} model={}",
            messages.len(),
            self.config.model
        );

        let mut last_error = None;
        let attempts = max_retries();
        for attempt in 0..attempts {
            if attempt > 0 {
                let delay = BASE_DELAY_MS * 2u64.pow(attempt - 1);
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }

            let t0 = std::time::Instant::now();
            probe_log!("[PROBE-cc] attempt={} url={} send start", attempt, url);
            let response = self
                .client
                .post(&url)
                .headers(headers.clone())
                .json(&body)
                .send()
                .await;
            probe_log!(
                "[PROBE-cc] attempt={} elapsed={:?} outcome={}",
                attempt,
                t0.elapsed(),
                if response.is_ok() { "ok" } else { "err" }
            );

            match response {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if resp.status().is_success() {
                        // v3.2：响应体读取/解码失败（decode）视为**可重试瞬态错误**。
                        // 真实 LLM 长推理时偶发连接中断/分块损坏，此前 `resp.text().await?`
                        // 的 `?` 直接把 decode 错误抛为最终错误，浪费了外层重试机会，
                        // 表现为「all failover candidates failed: error decoding response body」。
                        let body_text = match resp.text().await {
                            Ok(t) => t,
                            Err(e) => {
                                last_error =
                                    Some(anyhow!("response decode error ({}): {}", status, e));
                                continue;
                            }
                        };
                        let parsed = match self.parse_response(body_text) {
                            Ok(p) => p,
                            Err(e) => {
                                last_error = Some(anyhow!("response parse error: {e}"));
                                continue;
                            }
                        };
                        // 真实用量记账（进程级台账；未初始化时 no-op）
                        crate::usage::record(&self.config.model, &parsed.usage);
                        // 服务端前缀缓存命中计量（诚实：仅当 DeepSeek 返回 cache 字段时计入）
                        if let Some(c) = &self.cache {
                            c.record_server_cache(
                                parsed.usage.cache_read_input_tokens,
                                parsed.usage.cache_creation_input_tokens,
                            );
                        }
                        return Ok(parsed);
                    } else if is_retryable_error(Some(status)) {
                        let error_text = resp.text().await.unwrap_or_default();
                        last_error = Some(provider_http_error(status, error_text));
                        continue;
                    } else if status == 404
                        && !self.is_anthropic()
                        && self.config.model != "deepseek-v4-pro"
                    {
                        // 模型不存在（如用户误填 slug）：回退到稳定可用的 deepseek-v4-pro 再试一次，
                        // 保证「接真实 LLM 后端」始终能出真回复，不被模型名绊死。
                        let mut fb = self.config.clone();
                        fb.model = "deepseek-v4-pro".to_string();
                        tracing::warn!(
                            "模型 {} 返回 404，回退到 deepseek-v4-pro 重试",
                            self.config.model
                        );
                        let fb_client = LLMClient::new(fb);
                        return match Box::pin(fb_client.chat_complete(messages)).await {
                            Ok(r) => Ok(r),
                            Err(e) => Err(anyhow!(
                                "API error (404 模型不存在) 且 deepseek-v4-pro 回退失败: {}",
                                e
                            )),
                        };
                    } else {
                        let error_text = resp.text().await.unwrap_or_default();
                        return Err(provider_http_error(status, error_text));
                    }
                }
                Err(e) => {
                    if e.is_timeout() && is_retryable_error(None) {
                        last_error = Some(provider_timeout_error(format!("Request timeout: {e}")));
                        continue;
                    }
                    return Err(provider_network_error(format!("Request failed: {e}")));
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("Max retries exceeded")))
    }

    /// 带客户端缓存的对话完成（KV-cache 极简实践）。
    ///
    /// - 命中：直接复用缓存结果，`prompt_tokens` 记 0（无新增预填充），`completion_tokens` 照记；
    ///   用量台账如实反映"省下的预填充"，不虚报、不夸大。
    /// - 未命中：走真实 `chat_complete`，结果写入缓存。
    /// - 未挂载缓存时（默认 `None`）退化为普通 `chat_complete`，行为完全一致。
    ///
    /// 缓存键 = 真实 API 请求体的精确序列化哈希 → 只有逐字节一致的请求才命中（零语义假阳性）。
    pub async fn chat_complete_cached(&self, messages: &[Message]) -> Result<LLMResponse> {
        let cache = match &self.cache {
            Some(c) => c,
            None => {
                probe_log!("[CACHE-OBS] chat_complete_cached: cache=None -> passthrough chat_complete");
                return self.chat_complete(messages).await;
            }
        };
        // KV-Cache 极简实践 P1：稳定前缀（system/persona/KB）前置，放大命中率；
        // 同时让发送给服务端的请求也是稳定前缀在前，配合服务端 prompt caching。
        let ordered = crate::context::optimize::reorder_stable_prefix(messages);
        // 缓存实例收敛（M2 降耗根因修复）：进程内若已安装三层语义缓存（网关路径），
        // 其 Tier 0 精确层即网关共享 PrefixCache —— 无论本 client 自带的 `cache` 是否为
        // 同一对象，读写一律收敛到该共享层。
        //
        // 背景：此前仅在 `Arc::ptr_eq(layer.exact_arc(), cache)` 成立时才走三层接驳，
        // 而 `AgentRuntime` 等组件经 `ModelRouter::new` 各自新建了独立 PrefixCache，
        // 导致键虽稳定、查找却落在不同的私有表上，跨轮 Tier 0 命中率恒为 0。
        let layer = crate::cache::semantic_layer();
        let effective: Arc<PrefixCache> = match layer {
            Some(l) => l.exact_arc().clone(),
            None => cache.clone(),
        };
        // 观测稳定前缀复用（服务端 KV 块复用，如实计量，不虚报客户端命中）
        let prefix_hash =
            crate::cache::hash_text(&crate::context::optimize::stable_prefix_text(&ordered));
        let prefix_tokens = crate::context::optimize::stable_prefix_tokens(
            &ordered,
            model_family(&self.config.model),
        ) as u32;
        effective.note_prefix(prefix_hash, prefix_tokens);

        let key = crate::cache::hash_text(&self.cache_key_text(&ordered));
        let kt = self.cache_key_text(&ordered);
        let kt_head: String = kt.chars().take(80).collect();
        probe_log!(
            "[CACHE-OBS] enter key={:016x} len={} head={:?} layer_installed={} ptr_eq={}",
            key,
            kt.len(),
            kt_head,
            layer.is_some(),
            layer
                .map(|l| Arc::ptr_eq(l.exact_arc(), cache))
                .unwrap_or(false)
        );
        // 三层语义缓存接驳（Task #5）：进程级多层缓存存在时，统一用 `get_with_policy`
        // 逐层短路补命中——Tier 0 由共享精确层计量（零重复计数）。
        // 策略：默认 NormalizedOnly（Tier 0 + Tier 1 规范化精确，零近似假阳性）；
        // 显式选入 Tier 2（`MARGUERITE_SEMANTIC_CACHE=on`）时改用 Full，让语义层真正参与
        // ——否则「接了真实嵌入后端」在生产里永远走不到（这是文档 1 缺口的第二半）。
        // 工具路径不受影响：它走 ExactOnly（下方 `get_with_policy(ExactOnly)`），
        // 近似命中永远不会丢弃工具调用。
        if let Some(layer) = layer {
            if let Some(hit) = layer.get_with_policy(
                &self.cache_key_text(&ordered),
                crate::cache::semantic::text_path_policy(
                    crate::cache::semantic::tier2_opted_in_from_env(),
                ),
            ) {
                probe_log!("[CACHE-OBS] semantic-layer HIT tier={:?}", hit.tier);
                return Ok(Self::cached_response(&hit, &self.config.model));
            }
            probe_log!("[CACHE-OBS] semantic-layer MISS -> upstream then layer.put");
            let resp = self.chat_complete(&ordered).await?;
            layer.put(
                &self.cache_key_text(&ordered),
                &resp.content,
                resp.usage.prompt_tokens,
                resp.usage.completion_tokens,
            );
            return Ok(resp);
        }
        if let Some(e) = effective.get(key) {
            probe_log!("[CACHE-OBS] prefix-cache HIT");
            return Ok(Self::cached_response(
                &crate::cache::semantic::CachedResponse {
                    content: e.content,
                    completion_tokens: e.completion_tokens,
                    tier: crate::cache::semantic::HitTier::Exact,
                },
                &self.config.model,
            ));
        }
        probe_log!("[CACHE-OBS] prefix-cache MISS -> upstream then cache.put");
        let resp = self.chat_complete(&ordered).await?;
        effective.put(
            key,
            CacheEntry {
                content: resp.content.clone(),
                prompt_tokens: resp.usage.prompt_tokens,
                completion_tokens: resp.usage.completion_tokens,
                last_used_ms: crate::cache::now_ms(),
            },
        );
        Ok(resp)
    }

    /// 带工具定义的对话完成（客户端缓存版，KV-cache 极简实践）。
    ///
    /// 与 [`chat_complete_cached`] 同语义，但走 `chat_complete_with_tools`：
    /// 稳定前缀前置 + 前缀复用观测 + 精确命中复用，命中时 `prompt_tokens` 记 0；
    /// 缓存键额外纳入工具定义，避免不同工具集相互污染。未挂载缓存时退化为普通
    /// `chat_complete_with_tools`。
    pub async fn chat_complete_with_tools_cached(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<LLMResponse> {
        let cache = match &self.cache {
            Some(c) => c,
            None => return self.chat_complete_with_tools(messages, tools).await,
        };
        let ordered = crate::context::optimize::reorder_stable_prefix(messages);
        // 与 `chat_complete_cached` 同口径：收敛到进程级共享精确层（网关 Tier 0），
        // 避免私有缓存实例导致跨轮命中率恒为 0。
        let effective: Arc<PrefixCache> = match crate::cache::semantic_layer() {
            Some(l) => l.exact_arc().clone(),
            None => cache.clone(),
        };
        let prefix_hash =
            crate::cache::hash_text(&crate::context::optimize::stable_prefix_text(&ordered));
        let prefix_tokens = crate::context::optimize::stable_prefix_tokens(
            &ordered,
            model_family(&self.config.model),
        ) as u32;
        effective.note_prefix(prefix_hash, prefix_tokens);
        let key_text = format!(
            "{}{}",
            self.cache_key_text(&ordered),
            serde_json::to_string(tools).unwrap_or_default()
        );
        let key = crate::cache::hash_text(&key_text);
        // 工具路径永远走 ExactOnly（近似命中会静默丢弃工具调用）；进程级多层缓存存在时
        // 经其 ExactOnly 策略查询并计入「工具路径」真实计数，否则退化为共享精确层。
        let hit = match crate::cache::semantic_layer() {
            Some(l) => l.get_with_policy(&key_text, crate::cache::semantic::TierPolicy::ExactOnly),
            None => effective.get(key).map(|e| crate::cache::semantic::CachedResponse {
                content: e.content,
                completion_tokens: e.completion_tokens,
                tier: crate::cache::semantic::HitTier::Exact,
            }),
        };
        if let Some(hit) = hit {
            return Ok(LLMResponse {
                content: hit.content,
                thinking: None,
                model: self.config.model.clone(),
                usage: TokenUsage {
                    prompt_tokens: 0,
                    completion_tokens: hit.completion_tokens,
                    total_tokens: hit.completion_tokens,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                },
                tool_calls: vec![],
            });
        }
        let resp = self.chat_complete_with_tools(&ordered, tools).await?;
        effective.put(
            key,
            CacheEntry {
                content: resp.content.clone(),
                prompt_tokens: resp.usage.prompt_tokens,
                completion_tokens: resp.usage.completion_tokens,
                last_used_ms: crate::cache::now_ms(),
            },
        );
        Ok(resp)
    }

    /// 缓存键文本：与真实请求逐字节一致（用 API 形态序列化），保证精确命中。
    fn cache_key_text(&self, messages: &[Message]) -> String {
        serde_json::to_string(&self.messages_to_api(messages)).unwrap_or_default()
    }

    /// 把三层缓存命中转化为 `LLMResponse`（命中时 `prompt_tokens` 记 0，如实反映省下的预填充）。
    fn cached_response(hit: &crate::cache::semantic::CachedResponse, model: &str) -> LLMResponse {
        LLMResponse {
            content: hit.content.clone(),
            thinking: None,
            model: model.to_string(),
            usage: TokenUsage {
                prompt_tokens: 0,
                completion_tokens: hit.completion_tokens,
                total_tokens: hit.completion_tokens,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            },
            tool_calls: vec![],
        }
    }

    /// 服务端 KV 缓存预热（命中率优化）：只发含稳定前缀（system/persona/KB）的空调用。
    ///
    /// 配合服务端 prompt caching：先让服务端为该前缀预填充 KV 块，随后真实请求命中，
    /// 省去重复预填充。`max_tokens` 压到 1，仅触发预填充、不产生有意义输出，成本可忽略；
    /// 无稳定前缀（无 system 消息）时直接返回、不发请求；失败静默忽略（预热是尽力而为）。
    pub async fn warmup(&self, messages: &[Message]) -> Result<()> {
        let ordered = crate::context::optimize::reorder_stable_prefix(messages);
        // 只预热 **leading run**（不变前缀）——必须与 `chat_complete_cached` 的稳定前缀口径一致：
        // 那里用 `stable_prefix_text`（只取 leading run）算指纹喂给 `note_prefix`，
        // 若这里把中段 volatile 的 System（记忆召回/经验提示）也算进来，两边指纹不同，
        // 预热登记的那条前缀永远匹配不上，首个真实请求会被判成「新前缀」——预热收益计不上；
        // 顺带也避免把逐轮变化的召回段塞进预热请求白烧 prefill。
        let stable: Vec<Message> = crate::context::optimize::leading_stable_prefix(&ordered).to_vec();
        if stable.is_empty() {
            return Ok(());
        }
        let mut cfg = self.config.clone();
        cfg.max_tokens = 1;
        let client = Self::new(cfg);
        let _ = client.chat_complete(&stable).await?;
        // 记录稳定前缀已预热：使随后首个真实请求计为「前缀复用」，如实反映预热收益。
        if let Some(c) = &self.cache {
            let prefix_hash =
                crate::cache::hash_text(&crate::context::optimize::stable_prefix_text(&stable));
            let prefix_tokens = crate::context::optimize::stable_prefix_tokens(
                &stable,
                model_family(&self.config.model),
            ) as u32;
            c.note_prefix(prefix_hash, prefix_tokens);
        }
        Ok(())
    }

    /// 流式聊天完成
    pub async fn chat_complete_stream(&self, messages: &[Message]) -> Result<StreamReceiver> {
        let url = self.chat_url();
        let headers = self.headers();
        let body = self.build_stream_request_body(messages);

        let (tx, rx) = mpsc::channel(64);

        let client = self.client.clone();
        let model = self.config.model.clone();
        let anthropic = self.is_anthropic();
        // Responses 的流事件名与 Chat 完全不同，必须走独立解析器。
        let responses = matches!(
            self.config.wire_api,
            crate::model::wire::WireApi::Responses
        );
        let cache = self.cache.clone();
        // task-local 不跨 `tokio::spawn`：在 spawn 之前把本 turn 的用量归属取出来
        // 带进生产者任务，否则流式用量的 session 恒为空（真机缺陷：按会话统计全错）。
        let usage_session = crate::usage::current_session();
        let usage_run = crate::usage::current_run();

        tokio::spawn(async move {
            let response = match client.post(&url).headers(headers).json(&body).send().await {
                Ok(resp) => resp,
                Err(e) => {
                    let _ = tx
                        .send(Err(provider_network_error(format!("Stream request failed: {e}"))))
                        .await;
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let error_text = response.text().await.unwrap_or_default();
                let _ = tx
                    .send(Err(provider_http_error(status, error_text)))
                    .await;
                return;
            }

            let mut current_content = String::new();
            let mut current_thinking = String::new();
            // T7：流式 usage 记账旗标（first-wins）；见下方 Ok(parsed) 记账块。
            let mut usage_recorded = false;
            // Anthropic 把 usage 拆到 `message_start`（输入侧）与 `message_delta`
            // （输出侧）两个事件，必须跨事件合并后才能交下游记账。
            let mut anthropic_state = AnthropicStreamState::default();

            // SSE 流式解析
            let mut stream = response.bytes_stream();
            use futures_util::StreamExt;
            let mut buffer = String::new();

            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        let chunk_str = String::from_utf8_lossy(&bytes);
                        buffer.push_str(&chunk_str);

                        // 按行处理 SSE 事件
                        while let Some(line_end) = buffer.find('\n') {
                            let line = buffer[..line_end].trim().to_string();
                            buffer = buffer[line_end + 1..].to_string();

                            if line.is_empty() || line.starts_with(':') {
                                continue;
                            }

                            if let Some(data) = line.strip_prefix("data: ") {
                                if data == "[DONE]" {
                                    let _ = tx
                                        .send(Ok(LLMChunk {
                                            content: None,
                                            thinking: None,
                                            finish_reason: Some("stop".to_string()),
                                            tool_calls: None,
                                            usage: None,
                                        }))
                                        .await;
                                    return;
                                }

                                let parsed_result = if responses {
                                    parse_responses_stream_chunk(data, &model)
                                } else if anthropic {
                                    parse_anthropic_stream_event(data, &model, &mut anthropic_state)
                                } else {
                                    parse_stream_chunk(data, &model)
                                };
                                match parsed_result {
                                    Ok(parsed) => {
                                        // T7：流式 usage 记账。
                                        // first-wins：流内首个携带有效 usage 的 chunk 立即
                                        // 落账；布尔旗标保证同一份 usage（含个别 include_usage
                                        // 网关每 chunk 重复回显）只记一次，绝不多记。
                                        // 选 first-wins 而非 last-wins 的理由：
                                        // ① 生产两种形态（DeepSeek v4 把 usage 挂在最后一个
                                        //    content chunk；OpenAI include_usage 发 usage-only
                                        //    尾 chunk）里 usage 都是流的最终权威值，首个携带者
                                        //    即最终值，first-wins 与 last-wins 结果一致；
                                        // ② 即时落账：断流 / 消费者提前 drop 时已花费 token
                                        //    不丢（last-wins 要等流末，early-return 路径易漏
                                        //    记，正是本次缺陷的教训）。
                                        if !usage_recorded {
                                            if let Some(u) = parsed.usage.clone() {
                                                crate::usage::record_attributed(
                                                    usage_run.as_deref(),
                                                    usage_session.as_deref().unwrap_or_default(),
                                                    &model,
                                                    &u,
                                                );
                                                // 进程级前缀缓存追踪 + 实例级服务端缓存计量，
                                                // 与非流式路径对齐（修复前流式路径永不调用，
                                                // cache.status.server_calls 恒为 0）。
                                                crate::cache::report_prefix_cache_usage(
                                                    crate::cache::total_input_tokens(
                                                        u.prompt_tokens as u64,
                                                        u.cache_read_input_tokens as u64,
                                                        u.cache_creation_input_tokens as u64,
                                                        anthropic,
                                                    ),
                                                    u.cache_read_input_tokens as u64,
                                                    u.cache_creation_input_tokens as u64,
                                                    crate::cache::min_cache_tokens(anthropic),
                                                );
                                                if let Some(c) = &cache {
                                                    c.record_server_cache(
                                                        u.cache_read_input_tokens,
                                                        u.cache_creation_input_tokens,
                                                    );
                                                }
                                                usage_recorded = true;
                                            }
                                        }
                                        if let Some(ref c) = parsed.content {
                                            current_content.push_str(c);
                                        }
                                        if let Some(ref t) = parsed.thinking {
                                            current_thinking.push_str(t);
                                        }
                                        let chunk = LLMChunk {
                                            content: parsed.content,
                                            thinking: parsed.thinking,
                                            finish_reason: parsed.finish_reason,
                                            tool_calls: None,
                                            usage: parsed.usage.clone(),
                                        };
                                        if tx.send(Ok(chunk)).await.is_err() {
                                            return;
                                        }
                                    }
                                    Err(e) => {
                                        let _ = tx.send(Err(e)).await;
                                        return;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(provider_network_error(format!("Stream read error: {e}"))))
                            .await;
                        return;
                    }
                }
            }

            // 流末兜底：Anthropic 的流若中途断开（没走到 message_delta / message_stop），
            // 已知的输入计数仍要落账 —— 否则「断流不丢已花费 token」这条契约
            // 在 Anthropic 协议上不成立。
            if let Some(u) = anthropic_state.take_usage() {
                let _ = tx
                    .send(Ok(LLMChunk {
                        content: None,
                        thinking: None,
                        finish_reason: None,
                        tool_calls: None,
                        usage: Some(u),
                    }))
                    .await;
            }
        });

        Ok(rx)
    }

    /// 非流式聊天完成（带工具定义，支持 function calling）
    ///
    /// 把 `tools` 作为 OpenAI 兼容的 `tools` 字段注入请求体；模型可在响应中返回
    /// `tool_calls`，由调用方（如 agent 工具回路）解析并执行。
    pub async fn chat_complete_with_tools(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<LLMResponse> {
        let url = self.chat_url();
        let headers = self.headers();
        let body = self.build_body(messages, false, Some(tools));

        let mut last_error = None;
        let attempts = max_retries();
        for attempt in 0..attempts {
            if attempt > 0 {
                let delay = BASE_DELAY_MS * 2u64.pow(attempt - 1);
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }

            let t0 = std::time::Instant::now();
            probe_log!("[PROBE-cc] attempt={} url={} send start", attempt, url);
            let response = self
                .client
                .post(&url)
                .headers(headers.clone())
                .json(&body)
                .send()
                .await;
            probe_log!(
                "[PROBE-cc] attempt={} elapsed={:?} outcome={}",
                attempt,
                t0.elapsed(),
                if response.is_ok() { "ok" } else { "err" }
            );

            match response {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if resp.status().is_success() {
                        // v3.2：decode/parse 失败视为可重试瞬态（与 chat_complete 同策略）
                        let body_text = match resp.text().await {
                            Ok(t) => t,
                            Err(e) => {
                                last_error =
                                    Some(anyhow!("response decode error ({}): {}", status, e));
                                continue;
                            }
                        };
                        let parsed = match self.parse_response(body_text) {
                            Ok(p) => p,
                            Err(e) => {
                                last_error = Some(anyhow!("response parse error: {e}"));
                                continue;
                            }
                        };
                        // 真实用量记账（进程级台账；未初始化时 no-op）
                        crate::usage::record(&self.config.model, &parsed.usage);
                        // 服务端前缀缓存命中计量（诚实：仅当 DeepSeek 返回 cache 字段时计入）
                        if let Some(c) = &self.cache {
                            c.record_server_cache(
                                parsed.usage.cache_read_input_tokens,
                                parsed.usage.cache_creation_input_tokens,
                            );
                        }
                        return Ok(parsed);
                    } else if is_retryable_error(Some(status)) {
                        let error_text = resp.text().await.unwrap_or_default();
                        last_error = Some(provider_http_error(status, error_text));
                        continue;
                    } else {
                        let error_text = resp.text().await.unwrap_or_default();
                        return Err(provider_http_error(status, error_text));
                    }
                }
                Err(e) => {
                    if e.is_timeout() && is_retryable_error(None) {
                        last_error = Some(provider_timeout_error(format!("Request timeout: {e}")));
                        continue;
                    }
                    return Err(provider_network_error(format!("Request failed: {e}")));
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("Max retries exceeded")))
    }

    /// 流式聊天完成（带工具定义）
    ///
    /// 与 `chat_complete_with_tools` 同语义，但逐块吐出；流式工具调用在流末以
    /// 一个携带 `tool_calls` 的 `LLMChunk`（`finish_reason = "tool_calls"`）一次性给出完整调用。
    pub async fn chat_stream_with_tools(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<StreamReceiver> {
        let url = self.chat_url();
        let headers = self.headers();
        let body = self.build_body(messages, true, Some(tools));

        let (tx, rx) = mpsc::channel(64);
        let client = self.client.clone();
        let model = self.config.model.clone();
        let anthropic = self.is_anthropic();
        // Responses 的流事件名与 Chat 完全不同，必须走独立解析器。
        let responses = matches!(
            self.config.wire_api,
            crate::model::wire::WireApi::Responses
        );
        let cache = self.cache.clone();
        // task-local 不跨 `tokio::spawn`：在 spawn 之前把本 turn 的用量归属取出来
        // 带进生产者任务，否则流式用量的 session 恒为空（真机缺陷：按会话统计全错）。
        let usage_session = crate::usage::current_session();
        let usage_run = crate::usage::current_run();

        tokio::spawn(async move {
            let response = match client.post(&url).headers(headers).json(&body).send().await {
                Ok(resp) => resp,
                Err(e) => {
                    let _ = tx
                        .send(Err(provider_network_error(format!("Stream request failed: {e}"))))
                        .await;
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let error_text = response.text().await.unwrap_or_default();
                let _ = tx
                    .send(Err(provider_http_error(status, error_text)))
                    .await;
                return;
            }

            let mut tool_accum: Vec<PartialToolCall> = Vec::new();
            // T7：流式 usage 记账旗标（first-wins）；见下方 Ok(parsed) 记账块。
            let mut usage_recorded = false;
            // Anthropic 把 usage 拆到 `message_start`（输入侧）与 `message_delta`
            // （输出侧）两个事件，必须跨事件合并后才能交下游记账。
            let mut anthropic_state = AnthropicStreamState::default();
            let mut stream = response.bytes_stream();
            use futures_util::StreamExt;
            let mut buffer = String::new();

            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        let chunk_str = String::from_utf8_lossy(&bytes);
                        buffer.push_str(&chunk_str);

                        while let Some(line_end) = buffer.find('\n') {
                            let line = buffer[..line_end].trim().to_string();
                            buffer = buffer[line_end + 1..].to_string();

                            if line.is_empty() || line.starts_with(':') {
                                continue;
                            }

                            if let Some(data) = line.strip_prefix("data: ") {
                                if data == "[DONE]" {
                                    if !tool_accum.is_empty() {
                                        let calls: Vec<ToolCall> = tool_accum
                                            .into_iter()
                                            .filter_map(|p| {
                                                p.name.map(|name| ToolCall {
                                                    id: p.id,
                                                    name,
                                                    arguments: p.arguments,
                                                })
                                            })
                                            .collect();
                                        if !calls.is_empty() {
                                            let _ = tx
                                                .send(Ok(LLMChunk {
                                                    content: None,
                                                    thinking: None,
                                                    finish_reason: Some("tool_calls".to_string()),
                                                    tool_calls: Some(calls),
                                                    usage: None,
                                                }))
                                                .await;
                                        }
                                    }
                                    let _ = tx
                                        .send(Ok(LLMChunk {
                                            content: None,
                                            thinking: None,
                                            finish_reason: Some("stop".to_string()),
                                            tool_calls: None,
                                            usage: None,
                                        }))
                                        .await;
                                    return;
                                }

                                let parsed_result = if responses {
                                    parse_responses_stream_chunk(data, &model)
                                } else if anthropic {
                                    parse_anthropic_stream_event(data, &model, &mut anthropic_state)
                                } else {
                                    parse_stream_chunk(data, &model)
                                };
                                match parsed_result {
                                    Ok(parsed) => {
                                        // T7：流式 usage 记账（first-wins，理由同
                                        // `chat_complete_stream` 记账块）：流内首个携带有效
                                        // usage 的 chunk 立即落账，布尔旗标防重复回显导致多记。
                                        if !usage_recorded {
                                            if let Some(u) = parsed.usage.clone() {
                                                crate::usage::record_attributed(
                                                    usage_run.as_deref(),
                                                    usage_session.as_deref().unwrap_or_default(),
                                                    &model,
                                                    &u,
                                                );
                                                crate::cache::report_prefix_cache_usage(
                                                    crate::cache::total_input_tokens(
                                                        u.prompt_tokens as u64,
                                                        u.cache_read_input_tokens as u64,
                                                        u.cache_creation_input_tokens as u64,
                                                        anthropic,
                                                    ),
                                                    u.cache_read_input_tokens as u64,
                                                    u.cache_creation_input_tokens as u64,
                                                    crate::cache::min_cache_tokens(anthropic),
                                                );
                                                if let Some(c) = &cache {
                                                    c.record_server_cache(
                                                        u.cache_read_input_tokens,
                                                        u.cache_creation_input_tokens,
                                                    );
                                                }
                                                usage_recorded = true;
                                            }
                                        }
                                        for tc in parsed.tool_calls {
                                            accumulate_tool_call(&mut tool_accum, tc);
                                        }
                                        // 终态（OpenAI finish_reason / Anthropic message_delta/message_stop）：
                                        // 立即收敛 tool_accum，避免依赖 [DONE]（Anthropic 无 [DONE]）导致工具调用丢失。
                                        if parsed.finish_reason.is_some() && !tool_accum.is_empty()
                                        {
                                            let calls: Vec<ToolCall> = tool_accum
                                                .drain(..)
                                                .filter_map(|p| {
                                                    p.name.map(|name| ToolCall {
                                                        id: p.id,
                                                        name,
                                                        arguments: p.arguments,
                                                    })
                                                })
                                                .collect();
                                            if !calls.is_empty() {
                                                let _ = tx
                                                    .send(Ok(LLMChunk {
                                                        content: None,
                                                        thinking: None,
                                                        finish_reason: Some(
                                                            "tool_calls".to_string(),
                                                        ),
                                                        tool_calls: Some(calls),
                                                        usage: None,
                                                    }))
                                                    .await;
                                            }
                                        }
                                        let chunk = LLMChunk {
                                            content: parsed.content,
                                            thinking: parsed.thinking,
                                            finish_reason: parsed.finish_reason,
                                            tool_calls: None,
                                            usage: parsed.usage.clone(),
                                        };
                                        if tx.send(Ok(chunk)).await.is_err() {
                                            return;
                                        }
                                    }
                                    Err(e) => {
                                        let _ = tx.send(Err(e)).await;
                                        return;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(provider_network_error(format!("Stream read error: {e}"))))
                            .await;
                        return;
                    }
                }
            }

            // 流末兜底：Anthropic 的流若中途断开（没走到 message_delta / message_stop），
            // 已知的输入计数仍要落账 —— 否则「断流不丢已花费 token」这条契约
            // 在 Anthropic 协议上不成立。
            if let Some(u) = anthropic_state.take_usage() {
                let _ = tx
                    .send(Ok(LLMChunk {
                        content: None,
                        thinking: None,
                        finish_reason: None,
                        tool_calls: None,
                        usage: Some(u),
                    }))
                    .await;
            }

            // 非 SSE 提供方兜底：部分兼容端点忽略 `stream: true`，一次性返回整份 JSON。
            // 此前这类响应被逐行扫描后直接丢弃（剩余 buffer 从未消费），表现为
            // 「模型明明答了，客户端却拿到空文本」，从而把整个回合误判为空回复。
            // 这里在流末识别「整份 JSON」并翻译成与流式同形的 chunk 序列。
            let leftover = buffer.trim().to_string();
            if leftover.is_empty() {
                return;
            }
            if leftover.starts_with('{') {
                let parsed = if responses {
                    parse_responses_response(leftover, &model)
                } else if anthropic {
                    parse_anthropic_response(leftover, &model)
                } else {
                    parse_chat_response(leftover, &model)
                };
                match parsed {
                    Ok(resp) => {
                        if let Some(thought) = resp.thinking.filter(|t| !t.trim().is_empty()) {
                            let _ = tx
                                .send(Ok(LLMChunk {
                                    content: None,
                                    thinking: Some(thought),
                                    finish_reason: None,
                                    tool_calls: None,
                                    usage: None,
                                }))
                                .await;
                        }
                        if !resp.content.is_empty() {
                            let _ = tx
                                .send(Ok(LLMChunk {
                                    content: Some(resp.content),
                                    thinking: None,
                                    finish_reason: None,
                                    tool_calls: None,
                                    usage: None,
                                }))
                                .await;
                        }
                        if !resp.tool_calls.is_empty() {
                            let _ = tx
                                .send(Ok(LLMChunk {
                                    content: None,
                                    thinking: None,
                                    finish_reason: Some("tool_calls".to_string()),
                                    tool_calls: Some(resp.tool_calls),
                                    usage: None,
                                }))
                                .await;
                        }
                        // 用量记账：`parse_chat_response` 已上报前缀缓存，这里只补
                        // 「按会话/运行归属」的用量，避免缓存命中被重复计入。
                        if !usage_recorded {
                            crate::usage::record_attributed(
                                usage_run.as_deref(),
                                usage_session.as_deref().unwrap_or_default(),
                                &model,
                                &resp.usage,
                            );
                        }
                        let _ = tx
                            .send(Ok(LLMChunk {
                                content: None,
                                thinking: None,
                                finish_reason: Some("stop".to_string()),
                                tool_calls: None,
                                usage: Some(resp.usage),
                            }))
                            .await;
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(provider_network_error(format!(
                                "Response was neither SSE nor a parseable completion body: {e}"
                            ))))
                            .await;
                    }
                }
                return;
            }
            // 半行 SSE：流末最后一条 `data:` 行没有换行结尾时也要消费；
            // 解析失败只可能是截断片段，静默丢弃（已流出的正文不受影响）。
            if let Some(data) = leftover.strip_prefix("data: ") {
                let data = data.trim();
                if data.is_empty() || data == "[DONE]" {
                    return;
                }
                let parsed = if responses {
                    parse_responses_stream_chunk(data, &model)
                } else if anthropic {
                    parse_anthropic_stream_event(data, &model, &mut anthropic_state)
                } else {
                    parse_stream_chunk(data, &model)
                };
                if let Ok(parsed) = parsed {
                    for tc in parsed.tool_calls {
                        accumulate_tool_call(&mut tool_accum, tc);
                    }
                    if !tool_accum.is_empty() {
                        let calls: Vec<ToolCall> = tool_accum
                            .drain(..)
                            .filter_map(|p| {
                                p.name.map(|name| ToolCall {
                                    id: p.id,
                                    name,
                                    arguments: p.arguments,
                                })
                            })
                            .collect();
                        if !calls.is_empty() {
                            let _ = tx
                                .send(Ok(LLMChunk {
                                    content: None,
                                    thinking: None,
                                    finish_reason: Some("tool_calls".to_string()),
                                    tool_calls: Some(calls),
                                    usage: None,
                                }))
                                .await;
                        }
                    }
                    let _ = tx
                        .send(Ok(LLMChunk {
                            content: parsed.content,
                            thinking: parsed.thinking,
                            finish_reason: parsed.finish_reason,
                            tool_calls: None,
                            usage: parsed.usage,
                        }))
                        .await;
                }
            }
        });

        Ok(rx)
    }

    /// 高精度 token 估算（取代朴素 `len()/4`）。
    ///
    /// CJK 字符约 1.3 token/字，ASCII 约 0.25 token/字符（≈4 字符/token），其余 Unicode 约 0.5；
    /// 再乘模型家族微调系数。中文 / 混合文本误差远低于字节/4。
    pub fn count_tokens(&self, text: &str) -> usize {
        token_estimate(text, model_family(&self.config.model))
    }
}

/// 模型家族微调系数（默认 1.0；可按家族细分得更细）
fn model_family(model: &str) -> f64 {
    let m = model.to_ascii_lowercase();
    if m.contains("claude") {
        0.95
    } else {
        1.0
    }
}

/// 最近若干次请求的思考参数环形缓冲（最新在尾部）。
///
/// 为什么不是单槽：同一回合里除了主回复，还可能有预热/摘要/经验抽取等辅助请求，
/// 它们会覆盖单槽快照 —— 单槽观测会把「主请求已注入档位」误读成「档位没生效」。
fn effort_wire_ring() -> &'static std::sync::Mutex<std::collections::VecDeque<serde_json::Value>>
{
    const MAX: usize = 8;
    static RING: std::sync::OnceLock<std::sync::Mutex<std::collections::VecDeque<serde_json::Value>>> =
        std::sync::OnceLock::new();
    RING.get_or_init(|| std::sync::Mutex::new(std::collections::VecDeque::with_capacity(MAX)))
}

/// 记录本次请求的思考参数（供 `model.effort.last` 查询）。
///
/// 这是「推理档位到底有没有发出去」的唯一可自查证据：模型名是否在支持名单、
/// 实际注入到哪个 wire 字段、值是什么、以及**构造请求时是否处在回合档位作用域内**
/// （`turnScoped`）。此前不支持时**静默跳过**，用户切档完全无反馈 ——
/// 体验上就是「档位是个玩具」。
fn note_effort_wire(
    model: &str,
    effort: crate::model::thinking::ThinkingEffort,
    supported: bool,
    field: &'static str,
    value: Option<&'static str>,
) {
    // 作用域内 = 请求体确实是由「本轮」构造的；作用域外只能说明这是后台/辅助请求。
    let scoped = crate::model::thinking::current_turn_effort();
    let payload = serde_json::json!({
        "available": true,
        "model": model,
        "effort": effort.as_key(),
        "supported": supported,
        "injected": value.is_some(),
        "field": field,
        "value": value,
        "turnScoped": scoped.is_some(),
        "scopedEffort": scoped.map(|e| e.as_key()),
        "at_ms": crate::cache::now_ms(),
        "reason": if value.is_some() {
            "已注入请求体"
        } else if !supported {
            "该模型不在思考参数支持名单：档位不会影响请求体，只会改变本地思考提示词"
        } else {
            "档位为 off：按设计不注入思考参数"
        },
    });
    let mut ring = effort_wire_ring()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if ring.len() >= 8 {
        ring.pop_front();
    }
    ring.push_back(payload);
}

/// `model.effort.last`：最近一次请求实际发出的思考参数（附最近 8 次历史）。
pub fn last_effort_wire() -> serde_json::Value {
    let ring = effort_wire_ring()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match ring.back() {
        Some(last) => {
            let mut out = last.clone();
            out["recent"] = serde_json::Value::Array(ring.iter().cloned().collect());
            out
        }
        None => serde_json::json!({
            "available": false,
            "reason": "本进程还没有发出过模型请求",
        }),
    }
}

/// 高精度 token 估算（与模型无关，family = 1.0 时）。
///
/// 中文"一字一 token 到两 token"的直觉用 CJK 系数 1.3 逼近；英文按 4 字符/token。
pub fn token_estimate(text: &str, family: f64) -> usize {
    if text.is_empty() {
        return 0;
    }
    let mut t = 0.0;
    for c in text.chars() {
        if c.is_ascii() {
            t += 0.25;
        } else if is_cjk(c) {
            t += 1.3;
        } else {
            t += 0.5;
        }
    }
    ((t * family).ceil() as usize).max(1)
}

/// 是否为 CJK / 假名 / 谚文等"一字多 token"字符
fn is_cjk(c: char) -> bool {
    let u = c as u32;
    (0x3040..=0x30FF).contains(&u) // 平片假名
        || (0x3400..=0x4DBF).contains(&u) // CJK 扩展 A
        || (0x4E00..=0x9FFF).contains(&u) // CJK 统一表意
        || (0xAC00..=0xD7AF).contains(&u) // 谚文音节
        || (0xF900..=0xFAFF).contains(&u) // CJK 兼容
}

/// 解析 Anthropic Messages API 非流式响应（content blocks 形态）。
fn parse_anthropic_response(response_text: String, model: &str) -> Result<LLMResponse> {
    #[derive(Deserialize)]
    struct AnthropicResponse {
        #[serde(default)]
        content: Vec<AnthropicBlock>,
        #[serde(default, rename = "stop_reason")]
        _stop_reason: Option<String>,
        #[serde(default)]
        usage: Option<AnthropicUsage>,
    }

    #[derive(Deserialize)]
    struct AnthropicBlock {
        #[serde(rename = "type")]
        kind: String,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        thinking: Option<String>,
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        input: Option<serde_json::Value>,
    }

    #[derive(Deserialize)]
    struct AnthropicUsage {
        #[serde(rename = "input_tokens", default)]
        input_tokens: u32,
        #[serde(rename = "output_tokens", default)]
        output_tokens: u32,
        #[serde(rename = "cache_read_input_tokens", default)]
        cache_read: u32,
        #[serde(rename = "cache_creation_input_tokens", default)]
        cache_creation: u32,
    }

    let response: AnthropicResponse =
        serde_json::from_str(&response_text).context("Failed to parse Anthropic response")?;

    let mut content = String::new();
    let mut thinking = None;
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    for block in response.content {
        match block.kind.as_str() {
            "text" => content.push_str(&block.text.unwrap_or_default()),
            "thinking" if thinking.is_none() => {
                thinking = block.thinking;
            }
            "tool_use" => {
                if let (Some(id), Some(name), Some(input)) = (block.id, block.name, block.input) {
                    tool_calls.push(ToolCall {
                        id: Some(id),
                        name,
                        arguments: input.to_string(),
                    });
                }
            }
            _ => {}
        }
    }

    let usage = response.usage.map_or(TokenUsage::default(), |u| {
        crate::cache::report_prefix_cache_usage(
            crate::cache::total_input_tokens(
                u.input_tokens as u64,
                u.cache_read as u64,
                u.cache_creation as u64,
                true,
            ),
            u.cache_read as u64,
            u.cache_creation as u64,
            crate::cache::min_cache_tokens(true),
        );
        TokenUsage {
            prompt_tokens: u.input_tokens,
            completion_tokens: u.output_tokens,
            total_tokens: u.input_tokens + u.output_tokens,
            cache_read_input_tokens: u.cache_read,
            cache_creation_input_tokens: u.cache_creation,
        }
    });

    Ok(LLMResponse {
        content,
        thinking,
        model: model.to_string(),
        usage,
        tool_calls,
    })
}

/// 解析非流式 API 响应
fn parse_chat_response(response_text: String, model: &str) -> Result<LLMResponse> {
    #[derive(Deserialize)]
    struct ChatResponse {
        choices: Vec<Choice>,
        usage: Option<Usage>,
    }

    #[derive(Deserialize)]
    struct Choice {
        message: ResponseMessage,
        #[serde(rename = "finish_reason")]
        _finish_reason: Option<String>,
    }

    #[derive(Deserialize)]
    struct ResponseMessage {
        content: Option<String>,
        #[serde(rename = "reasoning_content")]
        reasoning_content: Option<String>,
        #[serde(default)]
        tool_calls: Vec<RawToolCall>,
    }

    #[derive(Deserialize)]
    struct RawToolCall {
        #[serde(default)]
        id: Option<String>,
        #[serde(default, rename = "type")]
        _call_type: Option<String>,
        function: RawFunction,
    }

    #[derive(Deserialize)]
    struct RawFunction {
        name: String,
        #[serde(default)]
        arguments: String,
    }

    #[derive(Deserialize)]
    struct Usage {
        #[serde(rename = "prompt_tokens")]
        prompt_tokens: u32,
        #[serde(rename = "completion_tokens")]
        completion_tokens: u32,
        #[serde(rename = "total_tokens")]
        total_tokens: u32,
        #[serde(rename = "cache_read_input_tokens", default)]
        cache_read_input_tokens: u32,
        #[serde(rename = "cache_creation_input_tokens", default)]
        cache_creation_input_tokens: u32,
        #[serde(rename = "prompt_cache_hit_tokens", default)]
        prompt_cache_hit_tokens: u32,
        #[serde(rename = "prompt_tokens_details", default)]
        prompt_tokens_details: Option<PromptTokensDetails>,
    }

    let response: ChatResponse =
        serde_json::from_str(&response_text).context("Failed to parse chat response")?;

    let choice = response
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("No choices in response"))?;

    let tool_calls = choice
        .message
        .tool_calls
        .into_iter()
        .map(|tc| ToolCall {
            id: tc.id,
            name: tc.function.name,
            arguments: tc.function.arguments,
        })
        .collect();

    Ok(LLMResponse {
        content: choice.message.content.unwrap_or_default(),
        thinking: choice.message.reasoning_content,
        model: model.to_string(),
        usage: response.usage.map_or(TokenUsage::default(), |u| {
            // DeepSeek 返回 `prompt_cache_hit_tokens`，OpenAI 系返回 `cache_read_input_tokens`
            // 或嵌套 `prompt_tokens_details.cached_tokens`；三者是**同一数值的三个别名**
            // （DeepSeek 文档："Same as prompt_cache_hit_tokens"），必须归一而非相加，
            // 否则命中量翻倍 → 覆盖率恒被 clamp 到 100%（假 100%）。
            let cache_read = normalize_cache_read([
                Some(u.cache_read_input_tokens),
                Some(u.prompt_cache_hit_tokens),
                u.prompt_tokens_details.map(|d| d.cached_tokens),
            ]);
            let cache_creation = u.cache_creation_input_tokens;
            crate::cache::report_prefix_cache_usage(
                crate::cache::total_input_tokens(
                    u.prompt_tokens as u64,
                    cache_read as u64,
                    cache_creation as u64,
                    false,
                ),
                cache_read as u64,
                cache_creation as u64,
                crate::cache::min_cache_tokens(false),
            );
            TokenUsage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                total_tokens: u.total_tokens,
                cache_read_input_tokens: cache_read,
                cache_creation_input_tokens: cache_creation,
            }
        }),
        tool_calls,
    })
}

/// 流式块解析结果（含可能的工具调用增量）
#[derive(Debug)]
struct ParsedChunk {
    content: Option<String>,
    thinking: Option<String>,
    finish_reason: Option<String>,
    tool_calls: Vec<RawToolCallDelta>,
    /// 本 chunk 携带的有效 usage（T7：不再要求 choices 为空；见 `parse_stream_chunk`）。
    /// `None` = 该 chunk 未携带有效用量（无 usage 字段或全零用量）。
    usage: Option<TokenUsage>,
}

/// OpenAI 兼容 usage 的嵌套缓存命中字段（非流式 `Usage` 与流式 `StreamUsage` 共用）。
#[derive(Deserialize)]
struct PromptTokensDetails {
    #[serde(rename = "cached_tokens", default)]
    cached_tokens: u32,
}

/// 把「缓存命中 token」的三个别名归一成**一个**值。
///
/// DeepSeek 官方文档写明：`prompt_tokens_details.cached_tokens` 与
/// `prompt_cache_hit_tokens` 是同一个数值（原文 "Same as prompt_cache_hit_tokens"），
/// 且同一次响应里会**同时出现**。三者属于同一量纲的不同名字，因此
/// **相加会把命中量翻倍**：`cache_read` 会超过 `prompt_tokens`（而 `prompt_tokens`
/// 本就 = 命中 + 未命中，包含命中部分），覆盖率算出来 >100% 再被 clamp 成 100% ——
/// 这正是「假 100%」的成因（实测 157952 命中 vs 102877 总输入）。
///
/// 取 `max` 而非「取第一个」：任意子集下 max 都等于那个唯一真实存在的值。
fn normalize_cache_read(aliases: [Option<u32>; 3]) -> u32 {
    aliases.into_iter().flatten().max().unwrap_or(0)
}

/// 流式工具调用 delta 片段（按 index 累积；name/arguments 嵌套在 function 下，与 OpenAI 一致）
#[derive(Debug, Clone, Deserialize)]
struct RawToolCallDelta {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<RawFunctionDelta>,
}

/// 流式工具调用的 function 片段
#[derive(Debug, Clone, Deserialize)]
struct RawFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// 流式工具调用累积态
struct PartialToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

/// 把流式 delta 中的单个工具调用片段合并进累积列表（按 index）
fn accumulate_tool_call(accum: &mut Vec<PartialToolCall>, delta: RawToolCallDelta) {
    let index = delta.index.unwrap_or(accum.len());
    if index >= accum.len() {
        accum.resize_with(index + 1, || PartialToolCall {
            id: None,
            name: None,
            arguments: String::new(),
        });
    }
    let slot = &mut accum[index];
    if let Some(id) = delta.id {
        slot.id = Some(id);
    }
    if let Some(func) = delta.function {
        if let Some(name) = func.name {
            slot.name = Some(name);
        }
        if let Some(args) = func.arguments {
            slot.arguments.push_str(&args);
        }
    }
}

/// 解析 Responses API 的 `usage` 对象。
///
/// 字段名与 Chat Completions 完全不同：`input_tokens` / `output_tokens`，
/// 而不是 `prompt_tokens` / `completion_tokens`。缓存命中在
/// `input_tokens_details.cached_tokens`。
///
/// 为什么单独写而不复用 `stream_usage_to_token_usage`：后者按 Chat 字段名反序列化，
/// 对 Responses 载荷会得到**全零**——用量台账会静默记 0，是典型的"看起来正常其实错"。
fn responses_usage_to_token_usage(u: &serde_json::Value) -> Option<TokenUsage> {
    let prompt = u.get("input_tokens").and_then(|v| v.as_u64())? as u32;
    let completion = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let total = u
        .get("total_tokens")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(prompt + completion);
    if total == 0 {
        return None;
    }
    let cache_read = u
        .pointer("/input_tokens_details/cached_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    Some(TokenUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        cache_read_input_tokens: cache_read,
        cache_creation_input_tokens: 0,
    })
}

/// 解析 Responses API 的非流式响应体。
///
/// 与 Chat 的结构差异：没有 `choices[]`，而是 `output[]` 数组；
/// 文本在 `output[].content[].text`（`type=output_text`），
/// 工具调用是独立的 `output[]` item（`type=function_call`）。
fn parse_responses_response(text: String, model: &str) -> Result<LLMResponse> {
    let v: serde_json::Value =
        serde_json::from_str(&text).context("Failed to parse Responses payload")?;

    if let Some(err) = v.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .or_else(|| err.as_str())
            .unwrap_or("Responses API returned an error");
        return Err(anyhow!("{msg}"));
    }

    let mut content = String::new();
    let mut thinking = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for item in v.get("output").and_then(|o| o.as_array()).cloned().unwrap_or_default() {
        match item.get("type").and_then(|t| t.as_str()) {
            Some("message") => {
                for part in item
                    .get("content")
                    .and_then(|c| c.as_array())
                    .cloned()
                    .unwrap_or_default()
                {
                    match part.get("type").and_then(|t| t.as_str()) {
                        Some("output_text") => {
                            if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                                content.push_str(t);
                            }
                        }
                        // 拒答不是空答复：原样带回，让上层能看到"模型拒绝"而不是空白
                        Some("refusal") => {
                            if let Some(r) = part.get("refusal").and_then(|r| r.as_str()) {
                                content.push_str(r);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("function_call") => {
                if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                    tool_calls.push(ToolCall {
                        id: item
                            .get("call_id")
                            .and_then(|c| c.as_str())
                            .map(|s| s.to_string()),
                        name: name.to_string(),
                        arguments: item
                            .get("arguments")
                            .and_then(|a| a.as_str())
                            .unwrap_or("{}")
                            .to_string(),
                    });
                }
            }
            Some("reasoning") => {
                for part in item
                    .get("summary")
                    .and_then(|s| s.as_array())
                    .cloned()
                    .unwrap_or_default()
                {
                    if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                        thinking.push_str(t);
                    }
                }
            }
            _ => {}
        }
    }

    // 便捷字段：部分实现只在顶层给 `output_text`，output[] 里没有 message item。
    if content.is_empty() {
        if let Some(t) = v.get("output_text").and_then(|t| t.as_str()) {
            content.push_str(t);
        }
    }

    let usage = v
        .get("usage")
        .and_then(responses_usage_to_token_usage)
        .unwrap_or_default();

    Ok(LLMResponse {
        content,
        thinking: if thinking.is_empty() {
            None
        } else {
            Some(thinking)
        },
        model: model.to_string(),
        usage,
        tool_calls,
    })
}

/// 解析 Responses API 的流式事件（SSE data 行的 JSON）。
///
/// 事件名是**扁平字符串**（`response.output_text.delta`），不是 Chat 的 `choices[].delta`。
/// 映射到与 Chat 相同的 [`ParsedChunk`]，从而复用既有的累积/记账逻辑：
///
/// | Responses 事件 | ParsedChunk |
/// |---|---|
/// | `response.output_text.delta` | `content` |
/// | `response.reasoning_summary_text.delta` | `thinking` |
/// | `response.output_item.added`（function_call） | `tool_calls`（起始：id + name） |
/// | `response.function_call_arguments.delta` | `tool_calls`（参数增量） |
/// | `response.completed` | `finish_reason` + `usage` |
/// | `response.failed` / `error` | `Err`（保留原始 message） |
///
/// 未知事件一律忽略：协议在演进，新增事件类型不该让整条流失败。
fn parse_responses_stream_chunk(data: &str, _model: &str) -> Result<ParsedChunk> {
    let v: serde_json::Value = serde_json::from_str(data)
        .context("Failed to parse Responses stream event")?;

    // 兼容层探测（线上实证：apigoto 的非流式走 chat 兼容层，流式是真 Responses；
    // 但某天某端点可能把流式也降级）。若 chunk 带 `choices[]`，它是 Chat 形态的
    // 流分片 —— 交给 chat 解析器，而不是被当成"未知事件"静默丢掉。
    // 丢掉的后果是：整条流解析出零内容，前端显示"思考完但没回答"，且不报错。
    if v.get("choices").and_then(|c| c.as_array()).is_some() {
        return parse_stream_chunk(data, _model);
    }

    let event_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let mut out = ParsedChunk {
        content: None,
        thinking: None,
        finish_reason: None,
        tool_calls: Vec::new(),
        usage: None,
    };

    match event_type {
        "response.output_text.delta" => {
            out.content = v
                .get("delta")
                .and_then(|d| d.as_str())
                .map(|s| s.to_string());
        }
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
            out.thinking = v
                .get("delta")
                .and_then(|d| d.as_str())
                .map(|s| s.to_string());
        }
        "response.output_item.added" => {
            let item = v.get("item");
            let is_call = item
                .and_then(|i| i.get("type"))
                .and_then(|t| t.as_str())
                == Some("function_call");
            if is_call {
                let item = item.expect("checked above");
                // `output_index` 是 Responses 的稳定序号，正好当累积 index 用
                let index = v
                    .get("output_index")
                    .and_then(|i| i.as_u64())
                    .unwrap_or(0) as usize;
                out.tool_calls.push(RawToolCallDelta {
                    index: Some(index),
                    // call_id 用于后续 function_call_output 配对，必须带下去
                    id: item
                        .get("call_id")
                        .and_then(|c| c.as_str())
                        .map(|s| s.to_string()),
                    function: Some(RawFunctionDelta {
                        name: item
                            .get("name")
                            .and_then(|n| n.as_str())
                            .map(|s| s.to_string()),
                        arguments: None,
                    }),
                });
            }
        }
        "response.function_call_arguments.delta" => {
            let index = v
                .get("output_index")
                .and_then(|i| i.as_u64())
                .unwrap_or(0) as usize;
            let delta = v
                .get("delta")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string();
            out.tool_calls.push(RawToolCallDelta {
                index: Some(index),
                id: None,
                function: Some(RawFunctionDelta {
                    name: None,
                    arguments: Some(delta),
                }),
            });
        }
        "response.completed" => {
            out.finish_reason = Some("stop".to_string());
            if let Some(usage) = v
                .get("response")
                .and_then(|r| r.get("usage"))
                .and_then(responses_usage_to_token_usage)
            {
                out.usage = Some(usage);
            }
        }
        "response.failed" => {
            let msg = v
                .pointer("/response/error/message")
                .and_then(|m| m.as_str())
                .unwrap_or("Responses API reported a failed response");
            return Err(anyhow!("{msg}"));
        }
        "error" => {
            let msg = v
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Responses API stream error");
            return Err(anyhow!("{msg}"));
        }
        _ => {}
    }
    Ok(out)
}

/// 解析 Anthropic Messages API 流式事件（SSE data 行的 JSON）。
///
/// 返回与 OpenAI 一致的 ParsedChunk：content_delta → content，
/// thinking_delta → thinking，tool_use 起始/input_json_delta → 工具调用
/// delta 片段（复用 accumulate_tool_call 按 index 合并），
/// message_delta/message_stop → finish_reason。
/// Anthropic SSE 事件里的 usage 对象。
///
/// `message_start.message.usage` 与 `message_delta.usage` 同形但语义不同：
/// 前者给**输入**侧（`input_tokens` 与缓存读写），后者给**输出**侧（`output_tokens`）。
/// `message_start` 里的 `output_tokens` 是占位值（恒为 1），不是真实输出量。
#[derive(Deserialize, Debug, Default)]
struct AnthropicUsageFields {
    #[serde(rename = "input_tokens", default)]
    input_tokens: u32,
    #[serde(rename = "output_tokens", default)]
    output_tokens: u32,
    #[serde(rename = "cache_read_input_tokens", default)]
    cache_read_input_tokens: u32,
    #[serde(rename = "cache_creation_input_tokens", default)]
    cache_creation_input_tokens: u32,
}

/// Anthropic 流的跨事件状态：usage 合并 + 停止原因去重。
///
/// 为什么需要状态：下游对流式 usage 的记账是 **first-wins**（流内首个携带
/// usage 的 chunk 立即落账，同一份 usage 只记一次）。而 Anthropic 把 usage
/// 拆到两个事件里 —— `message_start` 给输入侧（`output_tokens` 是占位 1），
/// `message_delta` 才给真实输出量。若「谁带 usage 就原样返回」，第一个
/// `message_start` 会用占位输出量把账锁死，真实输出量永久丢失
/// （token 台账与前缀缓存命中率恒为 0）。
///
/// 所以这里跨事件合并，等输出量到位后才吐出一份**完整**的 usage。
#[derive(Debug, Default)]
struct AnthropicStreamState {
    usage: AnthropicUsageFields,
    /// 是否见过 usage 字段（没有 usage 的流不该被记成 0）。
    usage_seen: bool,
    /// 是否已吐出过 usage（保证下游 first-wins：只吐一次）。
    usage_emitted: bool,
    /// 是否已吐出过停止原因（`message_stop` 不得覆盖 `message_delta` 的原因）。
    stop_reason_sent: bool,
}

impl AnthropicStreamState {
    /// 吸收 `message_start.message.usage`（输入侧）。
    fn ingest_start_usage(&mut self, u: Option<&AnthropicUsageFields>) {
        let Some(u) = u else { return };
        self.usage_seen = true;
        self.usage.input_tokens = u.input_tokens;
        self.usage.cache_read_input_tokens = u.cache_read_input_tokens;
        self.usage.cache_creation_input_tokens = u.cache_creation_input_tokens;
    }

    /// 吸收 `message_delta.usage`（输出侧）。
    fn ingest_delta_usage(&mut self, u: Option<&AnthropicUsageFields>) {
        let Some(u) = u else { return };
        self.usage_seen = true;
        if u.output_tokens > 0 {
            self.usage.output_tokens = u.output_tokens;
        }
    }

    /// 取出合并后的 usage（只吐一次；下游 first-wins 依赖这一点）。
    fn take_usage(&mut self) -> Option<TokenUsage> {
        if !self.usage_seen || self.usage_emitted {
            return None;
        }
        self.usage_emitted = true;
        Some(TokenUsage {
            prompt_tokens: self.usage.input_tokens,
            completion_tokens: self.usage.output_tokens,
            total_tokens: self.usage.input_tokens + self.usage.output_tokens,
            cache_read_input_tokens: self.usage.cache_read_input_tokens,
            cache_creation_input_tokens: self.usage.cache_creation_input_tokens,
        })
    }
}

/// Anthropic 的 `stop_reason` → OpenAI 风格的 `finish_reason`。
///
/// 两个词表同义不同名：上游只认 `stop` / `length` / `tool_calls`，
/// Anthropic 发的是 `end_turn` / `max_tokens` / `tool_use` / `stop_sequence`。
/// 不归一的后果：`tool_use` 这种非标准值一路流到 `AgentEvent::StepFinish.reason`，
/// 按 `tool_calls` 判终态的调用方永远匹配不上。
fn normalize_anthropic_stop_reason(reason: &str) -> &'static str {
    match reason {
        "end_turn" | "stop_sequence" => "stop",
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        "refusal" => "content_filter",
        _ => "stop",
    }
}

/// Anthropic 扩展思考：返回 (`thinking` 片段, 生效的 `max_tokens`)。
///
/// Anthropic 的扩展思考与 OpenAI 的 `reasoning_effort` **不是同一个字段**：
/// 请求体里是 `thinking: {type, budget_tokens}`，且
/// ① `budget_tokens` 必须小于 `max_tokens`（否则 400）；
/// ② 开启后 `temperature` 必须缺省或为 1（否则 400）。
fn anthropic_thinking_payload(
    model: &str,
    effort: crate::model::thinking::ThinkingEffort,
    max_tokens: u32,
) -> Option<(serde_json::Value, u32)> {
    use crate::model::thinking::ThinkingEffort;
    let budget = match effort {
        ThinkingEffort::Off => return None,
        ThinkingEffort::Normal => 1024,
        ThinkingEffort::Deep => 4096,
        ThinkingEffort::Max => 8192,
    };
    if !supports_anthropic_thinking(model) {
        return None;
    }
    // Anthropic 要求 budget_tokens < max_tokens；不够就抬高 max_tokens（与侧车同策略）。
    let max_tokens = if max_tokens <= budget {
        budget + 4096
    } else {
        max_tokens
    };
    Some((
        serde_json::json!({ "type": "enabled", "budget_tokens": budget }),
        max_tokens,
    ))
}

/// 模型是否支持 Anthropic 扩展思考（3.7 / 4 系）。
///
/// 只认**已知支持**的型号：Claude 3.5 / 3.0 收到 `thinking` 会直接 400，
/// 所以宁可少发也不发错（自定义网关的模型名千奇百怪，认不出就按不支持处理）。
fn supports_anthropic_thinking(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    if !m.contains("claude") {
        return false;
    }
    m.contains("3-7")
        || m.contains("3.7")
        || m.contains("claude-4")
        || m.contains("-4-")
        || m.contains("-4.")
        || m.contains("-4@")
}

#[cfg(test)]
fn parse_anthropic_stream_chunk(data: &str, model: &str) -> Result<ParsedChunk> {
    let mut state = AnthropicStreamState::default();
    parse_anthropic_stream_event(data, model, &mut state)
}

fn parse_anthropic_stream_event(
    data: &str,
    _model: &str,
    state: &mut AnthropicStreamState,
) -> Result<ParsedChunk> {
    #[derive(Deserialize)]
    struct Event {
        #[serde(rename = "type")]
        kind: String,
        #[serde(default)]
        index: Option<usize>,
        #[serde(default)]
        delta: Option<serde_json::Value>,
        #[serde(default)]
        content_block: Option<serde_json::Value>,
        #[serde(default)]
        message: Option<MessageEnvelope>,
        #[serde(default)]
        usage: Option<AnthropicUsageFields>,
    }

    #[derive(Deserialize)]
    struct MessageEnvelope {
        #[serde(default)]
        usage: Option<AnthropicUsageFields>,
    }

    let event: Event =
        serde_json::from_str(data).context("Failed to parse Anthropic stream event")?;

    let empty = || ParsedChunk {
        content: None,
        thinking: None,
        finish_reason: None,
        tool_calls: Vec::new(),
        usage: None,
    };

    match event.kind.as_str() {
        "message_start" => {
            // 输入侧 usage 在这里；输出侧要等 message_delta，所以**不**在此处吐 usage
            // （否则会用过期的占位 output_tokens 抢掉 first-wins 的记账机会）。
            state.ingest_start_usage(event.message.as_ref().and_then(|m| m.usage.as_ref()));
            Ok(empty())
        }
        "content_block_start" => {
            let block = event.content_block.unwrap_or_default();
            match block.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                "tool_use" => {
                    let index = event.index.unwrap_or(0);
                    let id = block.get("id").and_then(|v| v.as_str()).map(String::from);
                    let name = block.get("name").and_then(|v| v.as_str()).map(String::from);
                    Ok(ParsedChunk {
                        content: None,
                        thinking: None,
                        finish_reason: None,
                        tool_calls: vec![RawToolCallDelta {
                            index: Some(index),
                            id,
                            function: Some(RawFunctionDelta {
                                name,
                                arguments: None,
                            }),
                        }],
                        usage: None,
                    })
                }
                _ => Ok(empty()),
            }
        }
        "content_block_delta" => {
            let delta = event.delta.unwrap_or_default();
            match delta.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                "text_delta" => Ok(ParsedChunk {
                    content: delta.get("text").and_then(|v| v.as_str()).map(String::from),
                    thinking: None,
                    finish_reason: None,
                    tool_calls: Vec::new(),
                    usage: None,
                }),
                "thinking_delta" => Ok(ParsedChunk {
                    content: None,
                    thinking: delta
                        .get("thinking")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    finish_reason: None,
                    tool_calls: Vec::new(),
                    usage: None,
                }),
                "input_json_delta" => {
                    let index = event.index.unwrap_or(0);
                    Ok(ParsedChunk {
                        content: None,
                        thinking: None,
                        finish_reason: None,
                        tool_calls: vec![RawToolCallDelta {
                            index: Some(index),
                            id: None,
                            function: Some(RawFunctionDelta {
                                name: None,
                                arguments: delta
                                    .get("partial_json")
                                    .and_then(|v| v.as_str())
                                    .map(String::from),
                            }),
                        }],
                        usage: None,
                    })
                }
                _ => Ok(empty()),
            }
        }
        "message_delta" => {
            state.ingest_delta_usage(event.usage.as_ref());
            let stop_reason = event
                .delta
                .as_ref()
                .and_then(|d| d.get("stop_reason"))
                .and_then(|v| v.as_str())
                .map(normalize_anthropic_stop_reason);
            if stop_reason.is_some() {
                state.stop_reason_sent = true;
            }
            Ok(ParsedChunk {
                content: None,
                thinking: None,
                finish_reason: stop_reason.map(String::from),
                tool_calls: Vec::new(),
                usage: state.take_usage(),
            })
        }
        "message_stop" => Ok(ParsedChunk {
            content: None,
            thinking: None,
            // message_stop 本身不带停止原因，且不得覆盖 message_delta 已给出的那个
            // （否则 `tool_use` 会被抹成 `stop`，工具回路的终态判据就失效了）。
            finish_reason: if state.stop_reason_sent {
                None
            } else {
                Some("stop".to_string())
            },
            tool_calls: Vec::new(),
            usage: state.take_usage(),
        }),
        _ => Ok(empty()),
    }
}

/// 流式 chunk 携带的 usage 对象（OpenAI `stream_options.include_usage` /
/// DeepSeek v4 实测形态）。缓存命名字段与非流式 `Usage` 对齐：三种别名
/// （`cache_read_input_tokens` / `prompt_cache_hit_tokens` /
/// `prompt_tokens_details.cached_tokens`）由 [`stream_usage_to_token_usage`] 归一。
#[derive(Deserialize)]
struct StreamUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
    #[serde(rename = "prompt_cache_hit_tokens", default)]
    prompt_cache_hit_tokens: Option<u32>,
    #[serde(rename = "prompt_tokens_details", default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
}

/// 流式 usage → [`TokenUsage`] 归一化（纯函数，单测直接覆盖）。
///
/// 返回 `None` 表示该 chunk 未携带**有效**用量：门槛与
/// `usage::record_attributed` 的 no-op 条件一致（total / prompt / completion
/// 全零视为未携带），避免把空 usage 对象记进台账。
fn stream_usage_to_token_usage(u: &StreamUsage) -> Option<TokenUsage> {
    // 三个别名是同一个数值（DeepSeek 文档："Same as prompt_cache_hit_tokens"），
    // 取 max 归一；相加会让命中量翻倍（详见 `normalize_cache_read`）。
    let cache_read = normalize_cache_read([
        u.cache_read_input_tokens,
        u.prompt_cache_hit_tokens,
        u.prompt_tokens_details.as_ref().map(|d| d.cached_tokens),
    ]);
    let usage = TokenUsage {
        prompt_tokens: u.prompt_tokens.unwrap_or(0),
        completion_tokens: u.completion_tokens.unwrap_or(0),
        total_tokens: u.total_tokens.unwrap_or(0),
        cache_read_input_tokens: cache_read,
        cache_creation_input_tokens: 0,
    };
    (usage.total_tokens > 0 || usage.prompt_tokens > 0 || usage.completion_tokens > 0)
        .then_some(usage)
}

/// 解析流式响应块
fn parse_stream_chunk(data: &str, _model: &str) -> Result<ParsedChunk> {
    #[derive(Deserialize)]
    struct StreamChunk {
        #[serde(default)]
        choices: Vec<StreamChoice>,
        #[serde(default)]
        usage: Option<StreamUsage>,
    }

    #[derive(Deserialize)]
    struct StreamChoice {
        delta: Delta,
        #[serde(rename = "finish_reason")]
        finish_reason: Option<String>,
    }

    #[derive(Deserialize)]
    struct Delta {
        content: Option<String>,
        #[serde(rename = "reasoning_content")]
        reasoning_content: Option<String>,
        #[serde(default)]
        tool_calls: Vec<RawToolCallDelta>,
    }

    let chunk: StreamChunk = serde_json::from_str(data).context("Failed to parse stream chunk")?;

    // T7 修复：usage 记账**不以 choices 为空为前提**。
    //
    // 旧实现只在「usage-only 尾 chunk（choices 为空，OpenAI
    // stream_options.include_usage 风格）」时记账；而 DeepSeek v4 API 实测
    // （QA curl 直验）把 usage 挂在**最后一个 content chunk**（choices 非空），
    // 于是生产默认流式路径的用量全部丢失。
    // 现在只要 chunk 携带有效 usage 就随 ParsedChunk 上传，由流循环按
    // first-wins 策略记账一次（见 `chat_complete_stream` /
    // `chat_stream_with_tools` 中的记账块）。
    let usage = chunk.usage.as_ref().and_then(stream_usage_to_token_usage);

    // usage-only chunk（choices 为空且携带 usage 字段）：不产生内容，返回空
    // ParsedChunk（不是错误）。choices 为空且**无** usage 字段仍是协议异常，
    // 保持旧行为落到下面的 `No choices in stream chunk` 错误。
    if chunk.choices.is_empty() && chunk.usage.is_some() {
        return Ok(ParsedChunk {
            content: None,
            thinking: None,
            finish_reason: None,
            tool_calls: Vec::new(),
            usage,
        });
    }

    let choice = chunk
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("No choices in stream chunk"))?;

    // tool_calls 已按 OpenAI 格式嵌套在 function 下，直接透传给 ParsedChunk 即可
    let tool_calls = choice.delta.tool_calls;

    Ok(ParsedChunk {
        content: choice.delta.content,
        thinking: choice.delta.reasoning_content,
        finish_reason: choice.finish_reason,
        tool_calls,
        usage,
    })
}

#[cfg(test)]
#[path = "wire_api_tests.rs"]
mod wire_api_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;
    use std::sync::Arc;

    /* ── 客户端连接池复用键（回归钉子） ─────────────────────────────────── */

    /// **核心回归**：不同 `timeout_secs` 必须拿到**不同**的 client。
    ///
    /// 旧实现只按 `base_url` 作键 → 第二个配置直接复用第一个的 client，
    /// 于是「设置里调超时」在进程存活期内永不生效。本用例把它钉死。
    #[test]
    fn shared_client_distinguishes_timeout() {
        let a = LLMConfig {
            base_url: "https://cache-key-test.invalid".to_string(),
            timeout_secs: 60,
            ..LLMConfig::default()
        };
        let b = LLMConfig {
            timeout_secs: 300,
            ..a.clone()
        };

        // 先建 60s 的，再要 300s 的 —— 旧实现会错误地把它当命中。
        let _ = build_shared_client(&a);
        let key_a = format!("{}|{}", a.base_url, a.timeout_secs);
        let key_b = format!("{}|{}", b.base_url, b.timeout_secs);
        assert_ne!(key_a, key_b, "复用键必须把 timeout 纳入");

        let _ = build_shared_client(&b);
        let cache = shared_client_cache();
        let guard = cache.read().expect("cache readable");
        assert!(guard.contains_key(&key_a), "60s client 应已入池");
        assert!(guard.contains_key(&key_b), "300s client 应单独入池（而非复用 60s）");
    }

    /// 同 `(base_url, timeout_secs)` 必须**复用**同一 client（连接池语义不变）。
    ///
    /// 断言只统计**本用例专属 base_url** 的条目数，而不是全局 `len()`：
    /// 池是进程级共享的，并行跑其他用例时会在两次读数之间插入无关条目，
    /// 旧写法会把「别人入池」误判成「本键重复入池」（已实测偶发假失败）。
    #[test]
    fn shared_client_reuses_same_key() {
        let c = LLMConfig {
            base_url: "https://cache-reuse-test.invalid".to_string(),
            timeout_secs: 42,
            ..LLMConfig::default()
        };
        let prefix = format!("{}|", c.base_url);
        let count_for_this_key = || {
            shared_client_cache()
                .read()
                .expect("readable")
                .keys()
                .filter(|k| k.starts_with(&prefix))
                .count()
        };
        let _ = build_shared_client(&c);
        assert_eq!(count_for_this_key(), 1, "首次构建应恰好入池一条");
        let _ = build_shared_client(&c);
        assert_eq!(count_for_this_key(), 1, "同键不应重复入池");
    }

    /// 回归：base_url 已含 `/v1` 时不得再拼出 `/v1/v1/...`（Ollama / OpenRouter 404 根因）。
    #[test]
    fn build_chat_url_normalizes_existing_version_prefix() {
        let cases: &[(&str, bool, &str)] = &[
            // (base_url, is_anthropic, expected)
            ("http://localhost:11434", false, "http://localhost:11434/v1/chat/completions"),
            ("http://localhost:11434/", false, "http://localhost:11434/v1/chat/completions"),
            // 已带 /v1：不得双写
            ("http://localhost:11434/v1", false, "http://localhost:11434/v1/chat/completions"),
            ("http://localhost:11434/v1/", false, "http://localhost:11434/v1/chat/completions"),
            // /api/v1（OpenRouter）：同样以 /v1 结尾，直接追加
            ("https://openrouter.ai/api/v1", false, "https://openrouter.ai/api/v1/chat/completions"),
            // OpenAI 标准端点
            ("https://api.openai.com/v1", false, "https://api.openai.com/v1/chat/completions"),
            // Anthropic：不带 /v1 的根地址补 /v1/messages
            ("https://api.anthropic.com", true, "https://api.anthropic.com/v1/messages"),
            ("https://api.anthropic.com/", true, "https://api.anthropic.com/v1/messages"),
            ("https://api.anthropic.com/v1", true, "https://api.anthropic.com/v1/messages"),
            // 空白容错
            ("  http://localhost:11434/v1  ", false, "http://localhost:11434/v1/chat/completions"),
        ];
        for (base, is_anthropic, want) in cases {
            assert_eq!(build_chat_url(base, *is_anthropic), *want, "base={base}");
        }
    }

    #[test]
    fn test_count_tokens() {
        let config = LLMConfig::default();
        let client = LLMClient::new(config);
        assert_eq!(client.count_tokens("hello"), 2);
        assert_eq!(client.count_tokens(""), 0);
        assert!(client.count_tokens("a longer text message here") > 0);
    }

    #[test]
    fn test_messages_to_api() {
        let config = LLMConfig::default();
        let client = LLMClient::new(config);

        let messages = vec![
            Message::User {
                content: "hello".to_string(),
                timestamp: chrono::Utc::now(),
            },
            Message::Assistant {
                content: "hi".to_string(),
                timestamp: chrono::Utc::now(),
                thinking: None,
            },
            Message::System {
                content: "be helpful".to_string(),
                timestamp: chrono::Utc::now(),
            },
        ];

        let api_messages = client.messages_to_api(&messages);
        assert_eq!(api_messages.len(), 3);
        assert_eq!(api_messages[0]["role"], "user");
        assert_eq!(api_messages[0]["content"], "hello");
        assert_eq!(api_messages[1]["role"], "assistant");
        assert_eq!(api_messages[2]["role"], "system");
    }

    #[test]
    fn test_llm_config_default() {
        let config = LLMConfig::default();
        assert_eq!(config.model, "deepseek-v4-pro");
        assert_eq!(config.max_tokens, 4096);
        assert_eq!(config.temperature, 0.7);
    }

    // ===== P0-B：探针静默化（默认零输出，仅 MARGUERITE_DEBUG_PROBE 开启） =====

    #[test]
    fn test_probe_flag_parsing() {
        // 开启：1 / true / yes / on（大小写不敏感、允许首尾空白）
        for on in ["1", "true", "TRUE", " True ", "yes", "on", "ON"] {
            assert!(probe_flag_from_value(Some(on)), "{on:?} 应视为开启探针");
        }
        // 关闭：缺失 / 空 / 其它取值（默认静默，绝不"宽松解析"成开启）
        for off in ["", "  ", "0", "false", "no", "off", "2", "enabled", "debug", "y"] {
            assert!(!probe_flag_from_value(Some(off)), "{off:?} 不应开启探针");
        }
        assert!(!probe_flag_from_value(None), "未设置环境变量时必须默认静默");
    }

    #[test]
    fn test_probe_silent_by_default() {
        // 用户显式开启探针（有意排障）时不参与断言。
        if probe_flag_from_value(std::env::var(DEBUG_PROBE_ENV).ok().as_deref()) {
            return;
        }
        assert!(!probe_enabled(), "默认必须关闭探针输出");
        let before = probe_line_count();
        // 走与产品代码完全相同的出口：默认连一次格式化/写入都不允许发生。
        probe_log!("[PROBE-cache] MISS base={}", "https://example.invalid");
        probe_log!("[CACHE-OBS] prefix-cache MISS -> upstream then cache.put");
        probe_log!("[PROBE-cc] attempt=0 elapsed=0ms outcome=ok");
        assert_eq!(
            probe_line_count(),
            before,
            "默认（未设置 {DEBUG_PROBE_ENV}）不得输出任何探针行"
        );
    }

    // ===== Bloom 副模型（Copilot）配置解析与主模型继承 =====

    fn cfg_with_copilot() -> crate::persistence::config::Config {
        use crate::persistence::config::SecondaryModelConfig;
        let mut c = crate::persistence::config::Config::default();
        c.model.copilot_model = Some(SecondaryModelConfig {
            provider: None,                                // 未指定 → 继承主模型 provider
            model_name: "copilot-x".into(),                // 指定模型名
            api_key: None,                                 // 未指定 → 继承主模型 key
            base_url: Some("http://copilot.local".into()), // 指定 base_url
        });
        c
    }

    #[test]
    fn test_copilot_from_config_none_when_unset() {
        // 未配置 copilot_model → None → Bloom 退化为同模型双调用
        let c = crate::persistence::config::Config::default();
        assert!(LLMConfig::copilot_from_config(&c).is_none());
    }

    #[test]
    fn test_copilot_from_config_none_when_empty_model_name() {
        // 配置了副模型段但模型名为空 → None（不产生无效客户端）
        use crate::persistence::config::SecondaryModelConfig;
        let mut c = crate::persistence::config::Config::default();
        c.model.copilot_model = Some(SecondaryModelConfig {
            model_name: "  ".into(),
            ..Default::default()
        });
        assert!(LLMConfig::copilot_from_config(&c).is_none());
    }

    #[test]
    fn test_copilot_from_config_resolves_and_inherits() {
        // 主模型默认 deepseek + deepseek-chat + 空 key；副模型缺省字段应继承主模型，
        // 明确指定的 base_url 应独立生效。
        let c = cfg_with_copilot();
        let cop = LLMConfig::copilot_from_config(&c).expect("应解析出副模型客户端");
        assert_eq!(cop.model, "copilot-x");
        assert_eq!(
            cop.provider,
            LLMProvider::DeepSeek,
            "副模型 provider 应继承主模型"
        );
        assert_eq!(cop.api_key, "", "副模型 api_key 应继承主模型（空值）");
        assert_eq!(
            cop.base_url, "http://copilot.local",
            "明确指定的 base_url 应独立生效"
        );
    }

    #[test]
    fn test_copilot_from_config_custom_provider_overrides() {
        // 副模型显式覆盖 provider=openai 时，应独立于主模型
        use crate::persistence::config::SecondaryModelConfig;
        let mut c = crate::persistence::config::Config::default();
        c.model.copilot_model = Some(SecondaryModelConfig {
            provider: Some("openai".into()),
            model_name: "gpt-4o-mini".into(),
            api_key: Some("sk-copilot".into()),
            base_url: None, // 未指定 → 继承主模型 deepseek 的 base_url
        });
        let cop = LLMConfig::copilot_from_config(&c).expect("应解析出副模型客户端");
        assert_eq!(
            cop.provider,
            LLMProvider::OpenAI,
            "副模型 provider 显式覆盖应生效"
        );
        assert_eq!(cop.api_key, "sk-copilot");
        assert_eq!(cop.base_url, config_default_base_url());
    }

    fn config_default_base_url() -> String {
        "https://api.deepseek.com".to_string()
    }

    #[test]
    fn test_is_retryable_error() {
        assert!(is_retryable_error(Some(429)));
        assert!(is_retryable_error(Some(502)));
        assert!(is_retryable_error(Some(503)));
        assert!(is_retryable_error(Some(504)));
        assert!(is_retryable_error(None));
        assert!(!is_retryable_error(Some(400)));
        assert!(!is_retryable_error(Some(401)));
        assert!(!is_retryable_error(Some(200)));
    }

    #[test]
    fn test_parse_stream_chunk() {
        let data = r#"{"choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}"#;
        let chunk = parse_stream_chunk(data, "test-model").unwrap();
        assert_eq!(chunk.content, Some("Hello".to_string()));
        assert!(chunk.finish_reason.is_none());

        let data = r#"{"choices":[{"delta":{"content":""},"finish_reason":"stop"}]}"#;
        let chunk = parse_stream_chunk(data, "test-model").unwrap();
        assert_eq!(chunk.finish_reason, Some("stop".to_string()));
    }

    /* ── T7：流式 usage 记账修复回归钉子 ────────────────────────────────── */

    /// 形态①（DeepSeek v4 实测，QA curl 直验）：usage 挂在最后一个 content chunk，
    /// choices **非空**。旧实现在此分支直接丢弃 usage → 生产默认流式路径零记账。
    #[test]
    fn test_parse_stream_chunk_usage_on_last_content_chunk() {
        let data = r#"{"choices":[{"delta":{"content":"done"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#;
        let chunk = parse_stream_chunk(data, "test-model").unwrap();
        assert_eq!(chunk.content.as_deref(), Some("done"));
        assert_eq!(chunk.finish_reason.as_deref(), Some("stop"));
        let u = chunk
            .usage
            .expect("content chunk 携带的 usage 必须被提取（T7 根因修复点）");
        assert_eq!(
            (u.prompt_tokens, u.completion_tokens, u.total_tokens),
            (10, 5, 15)
        );
    }

    /// 形态②（OpenAI `stream_options.include_usage` 风格）：usage-only 尾 chunk，
    /// choices 为空。此形态旧实现已支持，修复后必须保持（不回退）。
    #[test]
    fn test_parse_stream_chunk_usage_only_tail_chunk() {
        let data = r#"{"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#;
        let chunk = parse_stream_chunk(data, "test-model").unwrap();
        assert!(chunk.content.is_none());
        assert!(chunk.finish_reason.is_none());
        let u = chunk.usage.expect("usage-only 尾 chunk 必须被提取");
        assert_eq!(u.total_tokens, 15);
    }

    /// 形态③：usage 出现在中间 chunk（choices 非空）。同样必须提取、不得丢弃。
    #[test]
    fn test_parse_stream_chunk_usage_middle_chunk() {
        let data = r#"{"choices":[{"delta":{"content":"mid"},"finish_reason":null}],
            "usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}"#;
        let chunk = parse_stream_chunk(data, "test-model").unwrap();
        assert_eq!(chunk.content.as_deref(), Some("mid"));
        assert_eq!(
            chunk.usage.expect("中间 chunk 的 usage 必须被提取").total_tokens,
            10
        );
    }

    /// 无 usage 的普通 content chunk：`usage` 必须为 None（记账块据此跳过）。
    #[test]
    fn test_parse_stream_chunk_without_usage_is_none() {
        let data = r#"{"choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}"#;
        let chunk = parse_stream_chunk(data, "test-model").unwrap();
        assert!(chunk.usage.is_none());
    }

    /// 全零 usage 视为「未携带」：与 `usage::record_attributed` 的 no-op 门槛一致，
    /// 不把空 usage 对象上传给记账块。
    #[test]
    fn test_parse_stream_chunk_zero_usage_is_none() {
        let data = r#"{"choices":[{"delta":{"content":"x"},"finish_reason":null}],
            "usage":{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0}}"#;
        let chunk = parse_stream_chunk(data, "test-model").unwrap();
        assert!(chunk.usage.is_none());
    }

    /// choices 为空且无 usage 的 chunk 仍是解析错误（保持旧行为）。
    #[test]
    fn test_parse_stream_chunk_empty_choices_without_usage_errors() {
        let err = parse_stream_chunk(r#"{"choices":[]}"#, "test-model").unwrap_err();
        assert!(
            err.to_string().contains("No choices in stream chunk"),
            "空 choices 且无 usage 应保留解析错误，实际: {err}"
        );
    }

    /// 缓存命名字段三别名归一（与非流式口径一致）：DeepSeek `prompt_cache_hit_tokens`
    /// 与 OpenAI `cache_read_input_tokens` / `prompt_tokens_details.cached_tokens` 求和。
    #[test]
    fn test_parse_stream_chunk_usage_cache_aliases_normalized() {
        let deepseek = r#"{"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5,
            "total_tokens":15,"prompt_cache_hit_tokens":4}}"#;
        let u = parse_stream_chunk(deepseek, "m").unwrap().usage.unwrap();
        assert_eq!(u.cache_read_input_tokens, 4);

        let openai = r#"{"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5,
            "total_tokens":15,"cache_read_input_tokens":6,
            "prompt_tokens_details":{"cached_tokens":2}}}"#;
        let u2 = parse_stream_chunk(openai, "m").unwrap().usage.unwrap();
        // 两个字段是同一量的别名，不是两笔命中：修复前 `6 + 2 = 8` 把命中量算多了，
        // 这会让 `cache_read` 超过 `prompt_tokens`、覆盖率被 clamp 成 100%。
        assert_eq!(u2.cache_read_input_tokens, 6);
    }

    /// 真实 DeepSeek 形态：`prompt_cache_hit_tokens` 与
    /// `prompt_tokens_details.cached_tokens` **同时出现且数值相同**（官方文档
    /// "Same as prompt_cache_hit_tokens"）。修复前二者相加 → 命中量翻倍。
    #[test]
    fn test_deepseek_dual_alias_not_double_counted() {
        let body = r#"{"choices":[],"usage":{"prompt_tokens":1000,"completion_tokens":5,
            "total_tokens":1005,"prompt_cache_hit_tokens":768,"prompt_cache_miss_tokens":232,
            "prompt_tokens_details":{"cached_tokens":768}}}"#;
        let u = parse_stream_chunk(body, "deepseek-chat")
            .unwrap()
            .usage
            .unwrap();
        assert_eq!(u.cache_read_input_tokens, 768, "两个别名不得相加");
        assert!(
            u.cache_read_input_tokens <= u.prompt_tokens,
            "命中量必须 ≤ 总输入（prompt_tokens 已包含命中部分），否则覆盖率会超 100%"
        );
    }

    #[test]
    fn test_chat_url_format() {
        let config = LLMConfig {
            base_url: "https://api.deepseek.com".to_string(),
            ..Default::default()
        };
        let client = LLMClient::new(config);
        assert_eq!(
            client.chat_url(),
            "https://api.deepseek.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_chat_url_trailing_slash() {
        let config = LLMConfig {
            base_url: "https://api.openai.com/".to_string(),
            ..Default::default()
        };
        let client = LLMClient::new(config);
        assert_eq!(
            client.chat_url(),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_headers() {
        let config = LLMConfig {
            api_key: "sk-test".to_string(),
            ..Default::default()
        };
        let client = LLMClient::new(config);
        let headers = client.headers();
        assert_eq!(headers.get("authorization").unwrap(), "Bearer sk-test");
        assert_eq!(headers.get("content-type").unwrap(), "application/json");
    }

    #[test]
    fn test_build_request_body() {
        let client = LLMClient::new(LLMConfig::default());
        let messages = vec![Message::User {
            content: "hi".to_string(),
            timestamp: chrono::Utc::now(),
        }];
        let body = client.build_request_body(&messages);
        assert_eq!(body["model"], "deepseek-v4-pro");
        assert_eq!(body["stream"], serde_json::Value::Bool(false));
        assert_eq!(body["messages"][0]["role"], "user");
        let stream_body = client.build_stream_request_body(&messages);
        assert_eq!(stream_body["stream"], serde_json::Value::Bool(true));
    }

    #[test]
    fn test_build_body_injects_reasoning_effort_for_deepseek() {
        // DeepSeek 模型 + 思考档位开启 → 注入 reasoning_effort
        let msg = vec![Message::User {
            content: "x".into(),
            timestamp: chrono::Utc::now(),
        }];
        let client = LLMClient::new(LLMConfig {
            model: "deepseek-v4-pro-0813".into(),
            thinking: crate::model::thinking::ThinkingEffort::Max,
            ..Default::default()
        });
        let body = client.build_body(&msg, false, None);
        assert_eq!(body["reasoning_effort"], "high");

        // 默认 Off → 不注入
        let off = LLMClient::new(LLMConfig::default());
        let body_off = off.build_body(&msg, false, None);
        assert!(body_off.get("reasoning_effort").is_none());
    }

    #[test]
    fn test_build_body_no_reasoning_effort_for_unknown_provider() {
        // 非 DeepSeek 模型即使开了思考也不注入（安全降级，仅本地 max_tokens 自适应）
        let msg = vec![Message::User {
            content: "x".into(),
            timestamp: chrono::Utc::now(),
        }];
        let client = LLMClient::new(LLMConfig {
            model: "gpt-4o".into(),
            thinking: crate::model::thinking::ThinkingEffort::Deep,
            ..Default::default()
        });
        let body = client.build_body(&msg, false, None);
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn test_adaptive_profile_and_builders() {
        // 代码任务自适应 → 大输出防截断
        let c = LLMClient::new(LLMConfig::default())
            .with_adaptive_profile(crate::model::thinking::TaskKind::Code);
        assert!(c.config.max_tokens >= 32 * 1024);
        assert_eq!(c.config.thinking.reasoning_effort(), Some("medium"));

        // 极限思考
        let m = LLMClient::new(LLMConfig::default()).with_max_thinking();
        assert_eq!(m.config.thinking.reasoning_effort(), Some("high"));

        // 1M 长任务
        let l = LLMClient::new(LLMConfig::default()).with_long_context();
        assert_eq!(
            l.config.context_window,
            crate::model::thinking::CONTEXT_WINDOW_1M
        );
    }

    #[test]
    fn test_with_temperature_lands_in_request_body() {
        // 模式规格接线：BloomSpec.temperature（MAX 档 0.1）必须真的落到请求体，
        // 而不是停留在默认 0.7。
        let base = LLMClient::new(LLMConfig::default());
        assert_eq!(base.config.temperature, 0.7, "默认温度基线");

        let cold = LLMClient::new(LLMConfig::default()).with_temperature(0.1);
        assert!((cold.config.temperature - 0.1).abs() < f32::EPSILON);
        let msg = vec![Message::User {
            content: "hi".into(),
            timestamp: chrono::Utc::now(),
        }];
        let body = cold.build_body(&msg, false, None);
        assert!(
            (body["temperature"].as_f64().unwrap() - 0.1).abs() < 1e-6,
            "温度必须进请求体，实际 {}",
            body["temperature"]
        );

        // 越界钳制与非法值保护
        let hi = LLMClient::new(LLMConfig::default()).with_temperature(9.9);
        assert!(
            (hi.config.temperature - 2.0).abs() < f32::EPSILON,
            "上限钳到 2.0"
        );
        let lo = LLMClient::new(LLMConfig::default()).with_temperature(-1.0);
        assert!(lo.config.temperature.abs() < f32::EPSILON, "下限钳到 0.0");
        let nan = LLMClient::new(LLMConfig::default()).with_temperature(f32::NAN);
        assert_eq!(nan.config.temperature, 0.7, "NaN 不应破坏配置");
    }

    #[test]
    fn test_parse_chat_response_success() {
        let body = r#"{"choices":[{"message":{"content":"Hello!","reasoning_content":"think"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#;
        let resp = parse_chat_response(body.to_string(), "m").unwrap();
        assert_eq!(resp.content, "Hello!");
        assert_eq!(resp.thinking, Some("think".to_string()));
        assert_eq!(resp.usage.total_tokens, 15);
    }

    #[test]
    fn test_parse_chat_response_no_usage() {
        let body = r#"{"choices":[{"message":{"content":"Hi"},"finish_reason":"stop"}]}"#;
        let resp = parse_chat_response(body.to_string(), "m").unwrap();
        assert_eq!(resp.content, "Hi");
        assert_eq!(resp.usage.total_tokens, 0);
    }

    #[test]
    fn test_parse_chat_response_invalid_json() {
        assert!(parse_chat_response("not json".to_string(), "m").is_err());
    }

    #[test]
    fn test_parse_chat_response_no_choices() {
        let body = r#"{"choices":[]}"#;
        assert!(parse_chat_response(body.to_string(), "m").is_err());
    }

    #[test]
    fn test_parse_chat_response_with_tool_calls() {
        let body = r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Beijing\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#;
        let resp = parse_chat_response(body.to_string(), "m").unwrap();
        assert!(resp.content.is_empty());
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "get_weather");
        assert_eq!(resp.tool_calls[0].id.as_deref(), Some("call_1"));
        assert_eq!(resp.tool_calls[0].arguments, "{\"city\":\"Beijing\"}");
    }

    #[test]
    fn test_anthropic_chat_url_and_headers() {
        let client = LLMClient::new(LLMConfig {
            provider: LLMProvider::Anthropic,
            base_url: "https://api.anthropic.com".into(),
            api_key: "sk-ant-test".into(),
            ..Default::default()
        });
        assert_eq!(client.chat_url(), "https://api.anthropic.com/v1/messages");
        let headers = client.headers();
        assert_eq!(headers.get("x-api-key").unwrap(), "sk-ant-test");
        assert_eq!(headers.get("anthropic-version").unwrap(), "2023-06-01");
        assert!(headers.get("authorization").is_none());
    }

    #[test]
    fn test_anthropic_build_body_system_and_tools() {
        let client = LLMClient::new(LLMConfig {
            provider: LLMProvider::Anthropic,
            model: "claude-sonnet-4-20250514".into(),
            base_url: "https://api.anthropic.com".into(),
            ..Default::default()
        });
        let messages = vec![
            Message::System {
                content: "be helpful".into(),
                timestamp: chrono::Utc::now(),
            },
            Message::User {
                content: "hi".into(),
                timestamp: chrono::Utc::now(),
            },
            Message::Assistant {
                content: String::new(),
                timestamp: chrono::Utc::now(),
                thinking: None,
            },
            Message::Tool {
                name: "echo".into(),
                input: "{\"message\":\"x\"}".into(),
                output: "ok".into(),
                timestamp: chrono::Utc::now(),
                elapsed_ms: None,
            },
        ];
        let body = client.build_body(&messages, false, Some(&sample_tool_defs()));
        assert_eq!(body["model"], "claude-sonnet-4-20250514");
        // W5.4：system 改为带 cache_control 的 block 数组（Anthropic 前缀缓存是显式断点）。
        assert_eq!(body["system"][0]["type"], "text");
        assert_eq!(body["system"][0]["text"], "be helpful");
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        let tools = body["tools"].as_array().expect("tools");
        assert_eq!(tools.last().unwrap()["cache_control"]["type"], "ephemeral");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "hi");
        assert_eq!(body["messages"][1]["role"], "assistant");
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(body["messages"][1]["content"][0]["name"], "echo");
        assert_eq!(body["messages"][2]["role"], "user");
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(body["tools"][0]["name"], "echo");
        assert!(body.get("stream_options").is_none());
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn test_anthropic_last_user_message_gets_a_cache_breakpoint() {
        let client = LLMClient::new(LLMConfig {
            provider: LLMProvider::Anthropic,
            model: "claude-sonnet-4-20250514".into(),
            base_url: "https://api.anthropic.com".into(),
            ..Default::default()
        });
        let messages = vec![
            Message::System {
                content: "be helpful".into(),
                timestamp: chrono::Utc::now(),
            },
            Message::User {
                content: "first".into(),
                timestamp: chrono::Utc::now(),
            },
            Message::Assistant {
                content: "answer".into(),
                timestamp: chrono::Utc::now(),
                thinking: None,
            },
            Message::User {
                content: "second".into(),
                timestamp: chrono::Utc::now(),
            },
        ];
        let body = client.build_body(&messages, false, None);
        // 只有最后一条 user 被改成 block array 并带断点；更早的 user 保持字符串。
        assert_eq!(body["messages"][0]["content"], "first");
        assert_eq!(body["messages"][2]["content"][0]["type"], "text");
        assert_eq!(body["messages"][2]["content"][0]["text"], "second");
        assert_eq!(
            body["messages"][2]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn test_anthropic_system_messages_merge_into_one_cached_block() {
        let client = LLMClient::new(LLMConfig {
            provider: LLMProvider::Anthropic,
            model: "claude-sonnet-4-20250514".into(),
            base_url: "https://api.anthropic.com".into(),
            ..Default::default()
        });
        let messages = vec![
            Message::System {
                content: "first".into(),
                timestamp: chrono::Utc::now(),
            },
            Message::System {
                content: "second".into(),
                timestamp: chrono::Utc::now(),
            },
            Message::User {
                content: "hi".into(),
                timestamp: chrono::Utc::now(),
            },
        ];
        let (system, api_messages) = client.messages_to_anthropic(&messages);
        // W5.4：多个 System 合并为**一个** text block，断点就挂在这个 block 上。
        assert_eq!(system.len(), 1, "多个 System 必须合并成一个 text block");
        assert_eq!(system[0]["type"], "text");
        assert_eq!(system[0]["text"], "first\nsecond");
        assert_eq!(system[0]["cache_control"]["type"], "ephemeral");
        assert_eq!(api_messages.len(), 1);
        assert!(
            api_messages[0]["content"].is_array(),
            "最后一条 user 必须转成 block 数组"
        );
        assert_eq!(
            api_messages[0]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn test_anthropic_body_has_exactly_three_breakpoints_and_none_mid_history() {
        let client = LLMClient::new(LLMConfig {
            provider: LLMProvider::Anthropic,
            model: "claude-sonnet-4-20250514".into(),
            base_url: "https://api.anthropic.com".into(),
            ..Default::default()
        });
        let messages = vec![
            Message::System {
                content: "stable".into(),
                timestamp: chrono::Utc::now(),
            },
            Message::User {
                content: "first".into(),
                timestamp: chrono::Utc::now(),
            },
            Message::Assistant {
                content: "answer".into(),
                timestamp: chrono::Utc::now(),
                thinking: None,
            },
            Message::User {
                content: "second".into(),
                timestamp: chrono::Utc::now(),
            },
        ];
        let body = client.build_body(&messages, false, Some(&sample_tool_defs()));
        // 断点恰好 3 个：system + 最后一个 tool + 最后一条 user（Anthropic 上限 4）。
        assert_eq!(
            body.to_string().matches("\"cache_control\"").count(),
            3,
            "断点数量必须是 3，且不重复注入：{body}"
        );
        // 更早的 user 保持字符串 —— 断点不在 history 中间。
        assert!(body["messages"][0]["content"].is_string());
        assert!(body["messages"][2]["content"].is_array());
        assert_eq!(
            body["messages"][2]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn test_non_anthropic_bodies_never_contain_cache_control() {
        // OpenAI / DeepSeek 是**自动**前缀缓存：注入非标准字段会被 400 或静默忽略。
        let messages = vec![
            Message::System {
                content: "stable".into(),
                timestamp: chrono::Utc::now(),
            },
            Message::User {
                content: "hi".into(),
                timestamp: chrono::Utc::now(),
            },
        ];
        for provider in [LLMProvider::OpenAI, LLMProvider::DeepSeek] {
            let client = LLMClient::new(LLMConfig {
                provider: provider.clone(),
                ..Default::default()
            });
            let body = client.build_body(&messages, false, Some(&sample_tool_defs()));
            assert!(
                !body.to_string().contains("cache_control"),
                "{provider:?} 的请求体不应含 cache_control：{body}"
            );
        }
    }

    #[test]
    fn test_parse_anthropic_response_text_and_tool() {
        let body = r#"{"content":[{"type":"text","text":"Hello!"},{"type":"tool_use","id":"toolu_1","name":"echo","input":{"message":"x"}}],"usage":{"input_tokens":12,"output_tokens":8}}"#;
        let resp = parse_anthropic_response(body.to_string(), "claude").unwrap();
        assert_eq!(resp.content, "Hello!");
        assert_eq!(resp.usage.prompt_tokens, 12);
        assert_eq!(resp.usage.total_tokens, 20);
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "echo");
    }

    #[test]
    fn test_parse_anthropic_stream_text_and_tool_delta() {
        let text =
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#;
        let c = parse_anthropic_stream_chunk(text, "claude").unwrap();
        assert_eq!(c.content.as_deref(), Some("Hi"));

        let start = r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"echo"}}"#;
        let s = parse_anthropic_stream_chunk(start, "claude").unwrap();
        assert_eq!(s.tool_calls.len(), 1);
        assert_eq!(s.tool_calls[0].index, Some(1));
        assert_eq!(s.tool_calls[0].id.as_deref(), Some("toolu_1"));
        assert_eq!(
            s.tool_calls[0].function.as_ref().unwrap().name.as_deref(),
            Some("echo")
        );

        let args = r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"message\":\"x\"}"}}"#;
        let a = parse_anthropic_stream_chunk(args, "claude").unwrap();
        assert_eq!(
            a.tool_calls[0]
                .function
                .as_ref()
                .unwrap()
                .arguments
                .as_deref(),
            Some("{\"message\":\"x\"}")
        );

        let done = r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#;
        let d = parse_anthropic_stream_chunk(done, "claude").unwrap();
        assert_eq!(d.finish_reason.as_deref(), Some("tool_calls"));
    }

    /// Anthropic 流式：usage 必须跨事件合并，`stop_reason` 必须归一。
    ///
    /// 真机缺陷（两处，都是「自定义 Anthropic 协议不完整」的一部分）：
    /// ① 解析器此前**完全不返回 usage** —— Anthropic 把 usage 拆到
    ///    `message_start`（输入侧）与 `message_delta`（输出侧）两个事件，
    ///    而流式记账是 first-wins（首个携带 usage 的 chunk 立即落账）。
    ///    原实现两头都没有 usage，于是 Anthropic 的 token 台账与前缀缓存
    ///    命中率**恒为 0**。
    /// ② `stop_reason` 原样透传（`tool_use`），而下游按 OpenAI 词表
    ///    （`stop` / `length` / `tool_calls`）判终态，非标准值匹配不上。
    #[tokio::test]
    async fn anthropic_stream_merges_usage_and_normalizes_stop_reason() {
        let sse = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":120,\"output_tokens\":1,\"cache_read_input_tokens\":40,\"cache_creation_input_tokens\":10}}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"你好\"}}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":55}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        let url = mock_server(vec![(200, sse.to_string())]).await;
        let config = LLMConfig {
            provider: LLMProvider::Anthropic,
            base_url: url,
            api_key: "k".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            timeout_secs: 5,
            ..Default::default()
        };
        let client = LLMClient::new(config);
        let mut rx = client
            .chat_stream_with_tools(&user_msg("hi"), &[])
            .await
            .unwrap();
        let mut usage: Option<TokenUsage> = None;
        let mut finish: Option<String> = None;
        while let Some(item) = rx.recv().await {
            let chunk = item.expect("流式帧不应出错");
            if let Some(u) = chunk.usage {
                usage = Some(u);
            }
            if chunk.finish_reason.is_some() {
                finish = chunk.finish_reason;
            }
        }
        assert!(
            usage.is_some() && finish.as_deref() == Some("tool_calls"),
            "Anthropic 流式应给出合并后的 usage，且把 tool_use 归一为 tool_calls；\
             实得 usage={usage:?} finish={finish:?}"
        );
        let u = usage.unwrap();
        assert_eq!(u.prompt_tokens, 120, "输入侧 token 来自 message_start");
        assert_eq!(u.completion_tokens, 55, "输出侧 token 来自 message_delta");
        assert_eq!(u.total_tokens, 175);
        assert_eq!(u.cache_read_input_tokens, 40);
        assert_eq!(u.cache_creation_input_tokens, 10);
    }

    #[test]
    fn token_estimate_empty_is_zero() {
        assert_eq!(token_estimate("", 1.0), 0);
    }

    #[test]
    fn token_estimate_ascii_approx_four_chars_per_token() {
        // "hello world" = 11 ASCII chars → 11*0.25 = 2.75 → ceil 3
        assert_eq!(token_estimate("hello world", 1.0), 3);
    }

    #[test]
    fn token_estimate_cjk_counts_per_char() {
        // "你好世界" = 4 CJK chars → 4*1.3 = 5.2 → ceil 6（远高于朴素 len()/4 = 3）
        assert_eq!(token_estimate("你好世界", 1.0), 6);
    }

    #[test]
    fn token_estimate_mixed_text() {
        // "Hi 你好" → ascii 3*0.25=0.75 + CJK 2*1.3=2.6 = 3.35 → ceil 4
        assert_eq!(token_estimate("Hi 你好", 1.0), 4);
    }

    #[test]
    fn effort_wire_snapshot_distinguishes_unsupported_and_off() {
        use crate::model::thinking::ThinkingEffort;
        // 模型不在支持名单：必须记录「未注入 + 原因」，而不是静默跳过
        note_effort_wire("gpt-5-mini", ThinkingEffort::Deep, false, "reasoning_effort", None);
        let snap = last_effort_wire();
        assert_eq!(snap["injected"], serde_json::json!(false));
        assert_eq!(snap["supported"], serde_json::json!(false));
        assert_eq!(snap["effort"], serde_json::json!("deep"));
        assert!(
            snap["reason"].as_str().unwrap_or_default().contains("支持名单"),
            "必须说明为何没注入：{snap}"
        );

        // 支持名单内：记录真实注入的字段与值
        note_effort_wire("deepseek-v4-pro", ThinkingEffort::Max, true, "reasoning_effort", Some("high"));
        let snap = last_effort_wire();
        assert_eq!(snap["injected"], serde_json::json!(true));
        assert_eq!(snap["value"], serde_json::json!("high"));
        assert_eq!(snap["field"], serde_json::json!("reasoning_effort"));
    }

    #[test]
    fn count_tokens_delegates_to_estimate_with_family() {
        let client = LLMClient::new(LLMConfig::default());
        // 默认模型 family=1.0；中文应远高于朴素字节/4
        assert_eq!(client.count_tokens("你好世界"), 6);
        assert_eq!(client.count_tokens("hello world"), 3);
        assert_eq!(client.count_tokens(""), 0);
    }

    fn sample_tool_defs() -> Vec<ToolDef> {
        vec![ToolDef {
            name: "echo".to_string(),
            description: "Echo a message".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "message": { "type": "string" } }
            }),
        }]
    }

    /// 捕获请求体的 mock 服务器
    async fn capturing_mock_llm_server(
        status: u16,
        body: &str,
        captured: Arc<std::sync::Mutex<Option<String>>>,
    ) -> String {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = body.to_string();
        tokio::spawn(async move {
            let mut first = true;
            loop {
                if let Ok((mut socket, _)) = listener.accept().await {
                    let data = crate::tests::mock_http::read_request_full(&mut socket).await;
                    // 只捕获首请求（真实对话请求；后续经验抽取等次要请求不覆盖断言目标）
                    if first {
                        first = false;
                        *captured.lock().unwrap() =
                            Some(String::from_utf8_lossy(&data).to_string());
                    }
                    let reason = if status >= 500 { "Server Error" } else { "OK" };
                    let header = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(body.as_bytes()).await;
                    let _ = socket.flush().await;
                }
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn test_chat_complete_with_tools_parses_tool_calls() {
        let body = r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"echo","arguments":"{\"message\":\"hi\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
        let url = mock_server(vec![(200, body.to_string())]).await;
        let config = LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            ..Default::default()
        };
        let client = LLMClient::new(config);
        let resp = client
            .chat_complete_with_tools(&user_msg("use echo"), &sample_tool_defs())
            .await
            .unwrap();
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "echo");
        assert_eq!(resp.tool_calls[0].arguments, "{\"message\":\"hi\"}");
    }

    #[tokio::test]
    async fn test_chat_complete_with_tools_sends_tools() {
        let captured = Arc::new(std::sync::Mutex::new(None));
        let url = capturing_mock_llm_server(
            200,
            r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}"#,
            captured.clone(),
        )
        .await;
        let config = LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            ..Default::default()
        };
        let client = LLMClient::new(config);
        let _ = client
            .chat_complete_with_tools(&user_msg("hi"), &sample_tool_defs())
            .await;
        let req = captured.lock().unwrap().clone().unwrap();
        assert!(req.contains("\"tools\""), "请求体应携带 tools 字段");
        assert!(req.contains("\"echo\""), "请求体应包含工具名");
        assert!(
            req.contains("\"function\""),
            "请求体应使用 OpenAI function 格式"
        );
    }

    #[tokio::test]
    async fn test_stream_with_tools_accumulates_tool_calls() {
        let c1 =
            r#"{"index":0,"id":"c1","type":"function","function":{"name":"echo","arguments":""}}"#;
        let c2 = r#"{"index":0,"function":{"arguments":"{\"message\":\""}}"#;
        let c3 = r#"{"index":0,"function":{"arguments":"hi\"}"}}"#;
        let sse = format!(
            "data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{c1}]}},\"finish_reason\":null}}]}}\n\n\
             data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{c2}]}},\"finish_reason\":null}}]}}\n\n\
             data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{c3}]}},\"finish_reason\":null}}]}}\n\n\
             data: [DONE]\n\n"
        );
        let url = mock_server(vec![(200, sse)]).await;
        let config = LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            ..Default::default()
        };
        let client = LLMClient::new(config);
        let mut rx = client
            .chat_stream_with_tools(&user_msg("use echo"), &sample_tool_defs())
            .await
            .unwrap();
        let mut final_tool_calls: Option<Vec<ToolCall>> = None;
        while let Some(item) = rx.recv().await {
            let chunk = item.unwrap();
            if let Some(tc) = chunk.tool_calls {
                final_tool_calls = Some(tc);
            }
        }
        let calls = final_tool_calls.expect("应累积出工具调用");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "echo");
        assert_eq!(calls[0].arguments, "{\"message\":\"hi\"}");
    }

    /// 非 SSE 提供方（忽略 `stream: true`，一次性返回整份 JSON）：流式客户端
    /// 必须把整份响应翻译成 chunk，而不是静默返回空文本。
    ///
    /// 真机缺陷：这类提供方下 `run_tool_loop_stream` 收到的是空流，界面上
    /// 「模型明明答了却什么都没有」，回合被误判为空回复。
    #[tokio::test]
    async fn stream_with_tools_parses_non_sse_completion_body() {
        let body = r#"{"choices":[{"message":{"content":"答案正文","reasoning_content":"先想再答"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":4,"total_tokens":7}}"#;
        let url = mock_server(vec![(200, body.to_string())]).await;
        let config = LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            ..Default::default()
        };
        let client = LLMClient::new(config);
        let mut rx = client
            .chat_stream_with_tools(&user_msg("hi"), &sample_tool_defs())
            .await
            .unwrap();
        let mut content = String::new();
        let mut thinking = String::new();
        let mut stopped = false;
        while let Some(item) = rx.recv().await {
            let chunk = item.expect("非 SSE 兜底不应产生错误帧");
            if let Some(t) = chunk.thinking {
                thinking.push_str(&t);
            }
            if let Some(c) = chunk.content {
                content.push_str(&c);
            }
            if chunk.finish_reason.as_deref() == Some("stop") {
                stopped = true;
            }
        }
        assert_eq!(thinking, "先想再答");
        assert_eq!(content, "答案正文");
        assert!(stopped, "非 SSE 兜底必须给出终止帧");
    }

    /* ── T7：流式 usage 落账端到端（台账行数断言） ───────────────────────── */

    /// 唯一 model 名：测试构建的 ledger 是进程级 temp DB（见 `usage::ledger_path`），
    /// 用唯一 model 名隔离同进程其他测试的写入，使行数断言确定。
    fn unique_test_model(tag: &str) -> String {
        format!("t7-{tag}-{}", uuid::Uuid::new_v4())
    }

    /// 序列化测试内的台账初始化（双保险；生产侧 `UsageLedger::new` 已设
    /// busy_timeout，见 usage/mod.rs）。消除并发 init 竞态导致的记录丢失。
    static USAGE_INIT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn ensure_usage_ledger() {
        let _guard = USAGE_INIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::usage::init();
    }

    /// 查询台账中指定 model 的 `(记账次数, total_tokens)`；从未记账返回 None。
    ///
    /// ⚠️ 调用方必须**先** `crate::usage::init()` 再发起流式请求：
    /// `usage::record` 在台账未初始化时是静默 no-op（见 `usage::record_attributed`），
    /// 事后 init 补不回已丢失的记录。init 幂等且测试构建固定落在进程级 temp DB
    /// （见 `usage::ledger_path`），唯一 model 名隔离同进程其他测试的写入。
    fn usage_ledger_entry(model: &str) -> Option<(i64, i64)> {
        ensure_usage_ledger();
        let status = crate::usage::status(&serde_json::json!({}));
        status["models"].as_array()?.iter().find_map(|m| {
            if m["model"].as_str() == Some(model) {
                Some((m["calls"].as_i64()?, m["total_tokens"].as_i64()?))
            } else {
                None
            }
        })
    }

    async fn drain_stream_chunks(mut rx: StreamReceiver) -> (String, bool) {
        let mut got = String::new();
        let mut done = false;
        while let Some(item) = rx.recv().await {
            let chunk = item.unwrap();
            if let Some(c) = chunk.content {
                got.push_str(&c);
            }
            if chunk.finish_reason.is_some() {
                done = true;
                break;
            }
        }
        (got, done)
    }

    /// 形态①端到端（DeepSeek v4 实测）：usage 挂在最后一个 content chunk，
    /// 且随后还有 usage-only 尾 chunk 重复回显同一份 usage——
    /// first-wins 必须只记一次（不多记）。
    #[tokio::test]
    async fn test_stream_usage_recorded_once_deepseek_v4_shape() {
        ensure_usage_ledger();
        let model = unique_test_model("ds-v4");
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}\n\n",
            "data: [DONE]\n\n"
        );
        let url = mock_server(vec![(200, sse.to_string())]).await;
        let config = LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            model: model.clone(),
            ..Default::default()
        };
        let client = LLMClient::new(config);
        let (got, done) = drain_stream_chunks(
            client
                .chat_complete_stream(&user_msg("ping"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(got, "Hello");
        assert!(done);
        let (calls, total) = usage_ledger_entry(&model).unwrap_or_else(|| {
            panic!(
                "content chunk 携带的 usage 必须落账（T7 根因修复点）；status={} items={}",
                crate::usage::status(&serde_json::json!({})),
                crate::usage::query(&serde_json::json!({"limit": 100}))
            )
        });
        assert_eq!(calls, 1, "usage 挂 content chunk + 尾 chunk 重复回显只许记账一次");
        assert_eq!(total, 15);
    }

    /// 形态②端到端（OpenAI include_usage 风格）：usage-only 尾 chunk 恰好记账一次。
    #[tokio::test]
    async fn test_stream_usage_recorded_once_usage_only_tail() {
        ensure_usage_ledger();
        let model = unique_test_model("openai-tail");
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}\n\n",
            "data: [DONE]\n\n"
        );
        let url = mock_server(vec![(200, sse.to_string())]).await;
        let config = LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            model: model.clone(),
            ..Default::default()
        };
        let client = LLMClient::new(config);
        drain_stream_chunks(
            client
                .chat_complete_stream(&user_msg("ping"))
                .await
                .unwrap(),
        )
        .await;
        let (calls, total) = usage_ledger_entry(&model).unwrap_or_else(|| {
            panic!(
                "usage-only 尾 chunk 必须落账；status={}",
                crate::usage::status(&serde_json::json!({}))
            )
        });
        assert_eq!((calls, total), (1, 15));
    }

    /// 形态③端到端：usage 出现在中间 chunk 必须落账；first-wins 锁定首个值，
    /// 流末不同 usage 不得覆盖、不得重复记账。
    #[tokio::test]
    async fn test_stream_usage_middle_chunk_first_wins() {
        ensure_usage_ledger();
        let model = unique_test_model("mid");
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"a\"},\"finish_reason\":null}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3,\"total_tokens\":10}}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"b\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":99,\"completion_tokens\":99,\"total_tokens\":198}}\n\n",
            "data: [DONE]\n\n"
        );
        let url = mock_server(vec![(200, sse.to_string())]).await;
        let config = LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            model: model.clone(),
            ..Default::default()
        };
        let client = LLMClient::new(config);
        let (got, done) = drain_stream_chunks(
            client
                .chat_complete_stream(&user_msg("ping"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(got, "ab");
        assert!(done);
        let (calls, total) =
            usage_ledger_entry(&model).expect("中间 chunk 的 usage 必须落账");
        assert_eq!(calls, 1);
        assert_eq!(total, 10, "first-wins：后续 chunk 的 usage 不得覆盖或重复记账");
    }

    /// 无 usage 的流不得在台账留下记录（no-op 门槛）。
    #[tokio::test]
    async fn test_stream_without_usage_records_nothing() {
        ensure_usage_ledger();
        let model = unique_test_model("none");
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let url = mock_server(vec![(200, sse.to_string())]).await;
        let config = LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            model: model.clone(),
            ..Default::default()
        };
        let client = LLMClient::new(config);
        drain_stream_chunks(
            client
                .chat_complete_stream(&user_msg("ping"))
                .await
                .unwrap(),
        )
        .await;
        assert!(
            usage_ledger_entry(&model).is_none(),
            "无 usage 的流不得记账"
        );
    }

    /// 生产默认路径（Bloom 并行 = 流式工具回路 `chat_stream_with_tools`）：
    /// usage 挂在携带 finish_reason 的工具 chunk 上，必须恰好落账一次。
    #[tokio::test]
    async fn test_stream_with_tools_usage_recorded_once() {
        ensure_usage_ledger();
        let model = unique_test_model("tools");
        let c1 =
            r#"{"index":0,"id":"c1","type":"function","function":{"name":"echo","arguments":""}}"#;
        let c2 = r#"{"index":0,"function":{"arguments":"{\"message\":\"hi\"}"}}"#;
        let sse = format!(
            "data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{c1}]}},\"finish_reason\":null}}]}}\n\n\
             data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{c2}]}},\"finish_reason\":\"tool_calls\"}}],\"usage\":{{\"prompt_tokens\":12,\"completion_tokens\":8,\"total_tokens\":20}}}}\n\n\
             data: [DONE]\n\n"
        );
        let url = mock_server(vec![(200, sse)]).await;
        let config = LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            model: model.clone(),
            ..Default::default()
        };
        let client = LLMClient::new(config);
        drain_stream_chunks(
            client
                .chat_stream_with_tools(&user_msg("use echo"), &sample_tool_defs())
                .await
                .unwrap(),
        )
        .await;
        let (calls, total) = usage_ledger_entry(&model)
            .expect("流式工具回路（Bloom 生产路径）的 usage 必须落账");
        assert_eq!((calls, total), (1, 20));
    }

    /// 简易 mock HTTP 服务器：依次返回给定响应
    async fn mock_server(responses: Vec<(u16, String)>) -> String {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            // 响应耗尽后复用末条（流程可能追加经验抽取等次要请求，绝不挂死）
            let last = responses.last().cloned();
            for (status, body) in responses {
                if let Ok((mut socket, _)) = listener.accept().await {
                    let _ = crate::tests::mock_http::read_request_full(&mut socket).await;
                    let reason = if status >= 500 { "Server Error" } else { "OK" };
                    let header = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(body.as_bytes()).await;
                    let _ = socket.flush().await;
                }
            }
            if let Some((status, body)) = last {
                loop {
                    if let Ok((mut socket, _)) = listener.accept().await {
                        let _ = crate::tests::mock_http::read_request_full(&mut socket).await;
                        let reason = if status >= 500 { "Server Error" } else { "OK" };
                        let header = format!(
                            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = socket.write_all(header.as_bytes()).await;
                        let _ = socket.write_all(body.as_bytes()).await;
                        let _ = socket.flush().await;
                    }
                }
            }
        });
        format!("http://{addr}")
    }

    fn user_msg(content: &str) -> Vec<Message> {
        vec![Message::User {
            content: content.to_string(),
            timestamp: chrono::Utc::now(),
        }]
    }

    #[tokio::test]
    async fn test_chat_complete_success() {
        let body = r#"{"choices":[{"message":{"content":"pong"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
        let url = mock_server(vec![(200, body.to_string())]).await;
        let config = LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            ..Default::default()
        };
        let client = LLMClient::new(config);
        let resp = client.chat_complete(&user_msg("ping")).await.unwrap();
        assert_eq!(resp.content, "pong");
    }

    #[tokio::test]
    async fn test_chat_complete_retries_on_429() {
        let err_body = r#"{"error":{"message":"rate limited"}}"#;
        let ok_body = r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}"#;
        let url = mock_server(vec![
            (429, err_body.to_string()),
            (200, ok_body.to_string()),
        ])
        .await;
        let config = LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            ..Default::default()
        };
        let client = LLMClient::new(config);
        let resp = client.chat_complete(&user_msg("ping")).await.unwrap();
        assert_eq!(resp.content, "ok");
    }

    #[tokio::test]
    async fn test_chat_complete_non_retryable_error() {
        let err_body = r#"{"error":"bad request"}"#;
        let url = mock_server(vec![(400, err_body.to_string())]).await;
        let config = LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            ..Default::default()
        };
        let client = LLMClient::new(config);
        let err = client.chat_complete(&user_msg("ping")).await.unwrap_err();
        assert!(err.to_string().contains("400"));
    }

    #[tokio::test]
    async fn test_chat_complete_stream_sse() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":null}]}\n\n",
            "data: [DONE]\n\n"
        );
        let url = mock_server(vec![(200, sse.to_string())]).await;
        let config = LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            ..Default::default()
        };
        let client = LLMClient::new(config);
        let mut rx = client
            .chat_complete_stream(&user_msg("ping"))
            .await
            .unwrap();
        let mut got = String::new();
        let mut done = false;
        while let Some(chunk) = rx.recv().await {
            let chunk = chunk.unwrap();
            if let Some(c) = chunk.content {
                got.push_str(&c);
            }
            if chunk.finish_reason.is_some() {
                done = true;
                break;
            }
        }
        assert_eq!(got, "Hello");
        assert!(done);
    }

    #[tokio::test]
    async fn test_chat_complete_stream_error() {
        let err_body = r#"{"error":"bad"}"#;
        let url = mock_server(vec![(500, err_body.to_string())]).await;
        let config = LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            ..Default::default()
        };
        let client = LLMClient::new(config);
        let mut rx = client
            .chat_complete_stream(&user_msg("ping"))
            .await
            .unwrap();
        let item = rx.recv().await.unwrap();
        assert!(item.is_err());
    }

    #[tokio::test]
    async fn test_chat_complete_cached_records_prefix_reuse() {
        // 清除全局缓存，防止先行测试污染语义层导致 HIT 跳过真实调用
        if let Some(layer) = crate::cache::semantic_layer() {
            layer.clear();
        }
        crate::gateway::chat::gateway_prefix_cache().clear();
        // 两次调用共享同一 system 前缀、不同 user 输入 → 稳定前缀应被记为复用（prefix_hits>=1）。
        // 这正是 P-7「稳定前缀重组」放大缓存命中率的观测证据。
        let body = r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":1,"total_tokens":6}}"#;
        let url = mock_server(vec![(200, body.to_string()), (200, body.to_string())]).await;
        let config = LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            ..Default::default()
        };
        let cache = Arc::new(PrefixCache::new(64));
        let client = LLMClient::new(config).with_cache(cache.clone());
        let sys = Message::System {
            content: "You are a helpful assistant.".into(),
            timestamp: chrono::Utc::now(),
        };
        let msgs1 = vec![
            sys.clone(),
            Message::User {
                content: "first".into(),
                timestamp: chrono::Utc::now(),
            },
        ];
        let msgs2 = vec![
            sys.clone(),
            Message::User {
                content: "second".into(),
                timestamp: chrono::Utc::now(),
            },
        ];
        let _ = client.chat_complete_cached(&msgs1).await.unwrap();
        let _ = client.chat_complete_cached(&msgs2).await.unwrap();
        // 语义层安装后，note_prefix 走全局 exact_arc（即 gateway_prefix_cache），
        // 不走测试本地 cache——断言须指向实际计数的全局实例
        let stats_source = crate::cache::semantic_layer()
            .map(|l| l.exact_arc().clone())
            .unwrap_or_else(|| cache.clone());
        let s = stats_source.stats();
        assert!(
            s.prefix_hits >= 1,
            "共享 system 前缀应被记为复用, got {}",
            s.prefix_hits
        );
    }

    #[tokio::test]
    async fn test_cached_replay_short_circuits_via_shared_semantic_layer() {
        // Task #5 接驳守卫：当进程级多层缓存的 Tier0 与 client 共享同一 PrefixCache（Arc 同源），
        // 精确未命中后应通过 NormalizedOnly 策略短路补命中，无需二次 HTTP 请求。
        // 注意：`semantic_layer()` 为进程级单例（首次安装者生效），本测试顺序无关——
        // 若本测试之前已有其他路径安装过语义层（如网关测试），则仅验证「同源时复用、异源时回退精确」，
        // 不依赖具体命中分层计数，避免跨测试全局状态耦合。
        let body = r#"{"choices":[{"message":{"content":"Paris"},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":1,"total_tokens":6}}"#;
        let url = mock_server(vec![(200, body.to_string()), (200, body.to_string())]).await;
        let config = LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            ..Default::default()
        };
        let cache = Arc::new(PrefixCache::new(64));
        crate::cache::install_semantic_layer(cache.clone(), None);
        let client = LLMClient::new(config).with_cache(cache.clone());

        let sys = Message::System {
            content: "You are a geography assistant.".into(),
            timestamp: chrono::Utc::now(),
        };
        let msgs1 = vec![
            sys.clone(),
            Message::User {
                content:
                    "What is the capital of France? trace=123e4567-e89b-12d3-a456-426614174000"
                        .into(),
                timestamp: chrono::Utc::now(),
            },
        ];
        // 同一规范化键、仅 UUID 易变后缀不同 → 同源时应被 Tier1 命中复用
        let msgs2 = vec![
            sys.clone(),
            Message::User {
                content:
                    "What is the capital of France? trace=999e9999-e89b-12d3-a456-426614174000"
                        .into(),
                timestamp: chrono::Utc::now(),
            },
        ];
        let r1 = client.chat_complete_cached(&msgs1).await.unwrap();
        assert_eq!(r1.content, "Paris");
        let r2 = client.chat_complete_cached(&msgs2).await.unwrap();
        assert_eq!(r2.content, "Paris");
        // 若语义层与 client 同源（本测试安装生效），则孪生请求应走 Tier1 短路无需二次 HTTP；
        // 同源时 layer 必记录了一次规范化命中。异源时退化为精确（仍返回正确内容，不报错）。
        if let Some(layer) = crate::cache::semantic_layer() {
            if Arc::ptr_eq(layer.exact_arc(), &cache) {
                let st = layer.stats();
                assert!(st.normalized_hits >= 1, "同源时应至少有一层规范化命中");
            }
        }
    }

    #[tokio::test]
    async fn test_warmup_sends_only_stable_prefix() {
        // warmup 只应送达稳定前缀（system）相关请求。这里用 mock 服务器验证它能成功完成
        // 且不因含 volatile user 消息而失败；无 system 前缀时则不应发任何请求。
        let body = r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
        let url = mock_server(vec![(200, body.to_string())]).await;
        let config = LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            ..Default::default()
        };
        let client = LLMClient::new(config);
        let msgs = vec![
            Message::System {
                content: "stable".into(),
                timestamp: chrono::Utc::now(),
            },
            Message::User {
                content: "volatile".into(),
                timestamp: chrono::Utc::now(),
            },
        ];
        client.warmup(&msgs).await.expect("预热应成功");
    }

    #[tokio::test]
    async fn test_warmup_noop_without_system() {
        // 无稳定前缀：warmup 不应发请求、直接返回 Ok。
        let client = LLMClient::new(LLMConfig {
            base_url: "http://127.0.0.1:1".to_string(), // 不可达地址——若触发请求会失败
            api_key: "k".to_string(),
            timeout_secs: 1,
            ..Default::default()
        });
        client
            .warmup(&user_msg("ping"))
            .await
            .expect("无 system 前缀应直接返回");
    }

    #[test]
    fn test_messages_to_api_no_cache_control_for_deepseek() {
        // DeepSeek V4 前缀缓存是**自动**的：相同前缀（≥1024 token、逐字节一致）的重复请求
        // 由服务端复用 KV 块，无需、也不应在 payload 中注入非标准 `cache_control` 字段
        // （错误形态会导致 400 或被忽略）。命中杠杆是稳定且足够长的系统前缀。
        let client = LLMClient::new(LLMConfig::default()); // DeepSeek
        let messages = vec![Message::System {
            content: "be helpful".to_string(),
            timestamp: chrono::Utc::now(),
        }];
        let api = client.messages_to_api(&messages);
        let sys = api
            .iter()
            .find(|m| m["role"] == "system")
            .expect("应有 system 消息");
        assert!(
            sys.get("cache_control").is_none(),
            "DeepSeek 不应注入非标准 cache_control（自动前缀缓存）"
        );
        assert_eq!(sys["content"], "be helpful", "内容不应被改变");
    }

    #[test]
    fn test_messages_to_api_no_cache_control_for_openai() {
        let client = LLMClient::new(LLMConfig {
            provider: LLMProvider::OpenAI,
            ..Default::default()
        });
        let messages = vec![Message::System {
            content: "be helpful".to_string(),
            timestamp: chrono::Utc::now(),
        }];
        let api = client.messages_to_api(&messages);
        let sys = api
            .iter()
            .find(|m| m["role"] == "system")
            .expect("应有 system 消息");
        assert!(
            sys.get("cache_control").is_none(),
            "OpenAI 不应注入 cache_control"
        );
    }

    #[test]
    fn test_token_usage_parses_deepseek_cache_tokens() {
        // DeepSeek 返回 prompt_cache_hit_tokens；OpenAI 系返回 cache_read_input_tokens。
        // 二者应归一为 cache_read_input_tokens。
        let body = r#"{
            "choices": [{"message": {"content": "hi", "role": "assistant"}}],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 10,
                "total_tokens": 110,
                "prompt_cache_hit_tokens": 80,
                "cache_creation_input_tokens": 20
            }
        }"#;
        let resp = parse_chat_response(body.to_string(), "deepseek-v4-pro").unwrap();
        assert_eq!(resp.usage.prompt_tokens, 100);
        assert_eq!(
            resp.usage.cache_read_input_tokens, 80,
            "应归一 prompt_cache_hit_tokens"
        );
        assert_eq!(resp.usage.cache_creation_input_tokens, 20);

        // OpenAI 风格 usage
        let body2 = r#"{
            "choices": [{"message": {"content": "hi", "role": "assistant"}}],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 10,
                "total_tokens": 110,
                "cache_read_input_tokens": 70
            }
        }"#;
        let resp2 = parse_chat_response(body2.to_string(), "gpt-4o").unwrap();
        assert_eq!(resp2.usage.cache_read_input_tokens, 70);
    }

    #[test]
    fn test_token_usage_parses_openai_nested_cached_tokens() {
        // OpenAI 兼容端点把缓存命中放在 `usage.prompt_tokens_details.cached_tokens`，
        // 与 DeepSeek 的顶层 `prompt_cache_hit_tokens` 不同；两者都要归一为
        // `cache_read_input_tokens`。
        let body = r#"{
            "choices": [{"message": {"content": "hi", "role": "assistant"}}],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 10,
                "total_tokens": 110,
                "prompt_tokens_details": {"cached_tokens": 60}
            }
        }"#;
        let resp = parse_chat_response(body.to_string(), "gpt-4o").unwrap();
        assert_eq!(
            resp.usage.cache_read_input_tokens, 60,
            "应归一 prompt_tokens_details.cached_tokens"
        );
    }

    #[test]
    fn test_prefix_cache_record_server_cache() {
        let c = crate::cache::PrefixCache::new(8);
        c.record_server_cache(120, 30);
        c.record_server_cache(120, 0); // 第二次命中：仅 read，无 creation
        let s = c.stats();
        assert_eq!(s.server_cache_read_tokens, 240);
        assert_eq!(s.server_cache_creation_tokens, 30);
        assert!(
            (s.server_cache_hit_rate() - (240.0 / 270.0)).abs() < 1e-9,
            "240 read / (240 read + 30 creation) ≈ 0.889"
        );
    }

    // ===== 实打实端到端：DeepSeek V4 原生前缀缓存命中（需真实 key，默认忽略） =====
    // 运行：
    //   MARGUERITE_API_KEY=sk-xxx cargo test -p marguerite-core --lib \
    //     test_deepseek_cache_hit_e2e_real -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn test_deepseek_cache_hit_e2e_real() {
        let api_key = match std::env::var("MARGUERITE_API_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => {
                eprintln!("跳过：未设置 MARGUERITE_API_KEY（真实 DeepSeek 端到端测试需要）");
                return;
            }
        };
        let model =
            std::env::var("MARGUERITE_MODEL").unwrap_or_else(|_| "deepseek-v4-pro".to_string());
        let base_url = std::env::var("MARGUERITE_BASE_URL")
            .unwrap_or_else(|_| "https://api.deepseek.com".to_string());

        let config = LLMConfig {
            api_key,
            base_url,
            model,
            provider: LLMProvider::DeepSeek,
            timeout_secs: 60,
            max_tokens: 64,
            temperature: 0.0,
            ..Default::default()
        };
        let cache = Arc::new(crate::cache::PrefixCache::new(2048));
        let client = LLMClient::new(config).with_cache(cache.clone());

        // 稳定且足够长的系统前缀（≥1024 token、逐字节一致）——DeepSeek 自动前缀缓存的命中杠杆。
        let unit = "You are Marguerite, an autonomous coding agent. Operate by these stable \
            principles: prefer minimal reviewable diffs; run the project test suite before \
            claiming done; never invent APIs; read the source when uncertain; keep this system \
            prefix byte-identical across requests so the provider reuses its KV cache. ";
        let system: String = std::iter::repeat_n(unit, 60).collect();
        eprintln!(
            "[e2e] system prefix = {} chars (~{} tokens)",
            system.chars().count(),
            system.chars().count() / 4
        );

        // 先用稳定前缀预热（warmup）：向 DeepSeek 发送一次仅含 system 前缀的请求（max_tokens=1），
        // 触发服务端为该前缀预填充 KV 块；随后真实请求即可命中，消除冷启动（研究指出 DeepSeek
        // 前缀缓存有「2+ 请求冷启动延迟」）。这是 marguerite 缓存策略的关键一环，需端到端验证。
        let warm_msg = vec![
            Message::System {
                content: system.clone(),
                timestamp: chrono::Utc::now(),
            },
            Message::User {
                content: "warmup".to_string(),
                timestamp: chrono::Utc::now(),
            },
        ];
        let mut warmup_ok = false;
        match client.warmup(&warm_msg).await {
            Ok(()) => {
                eprintln!("[e2e] warmup 成功（已触发服务端前缀预填充）");
                warmup_ok = true;
            }
            Err(e) => eprintln!("[e2e] warmup 失败（尽力而为，忽略）: {:?}", e),
        }

        // 三次真实请求：若 warmup 成功，call1 应直接命中（消除冷启动）；call2 同后缀（必命中）；
        // call3 仅后缀不同（前缀仍应命中，验证 PREFIX 缓存而非整请求去重）。
        let suffixes = ["compute 2+2", "compute 2+2", "compute 3+3"];
        let mut saw_hit = false;
        let mut first_call_hit = false;
        for (i, suf) in suffixes.iter().enumerate() {
            let n = i + 1;
            let messages = vec![
                Message::System {
                    content: system.clone(),
                    timestamp: chrono::Utc::now(),
                },
                Message::User {
                    content: format!("Please {}.", suf),
                    timestamp: chrono::Utc::now(),
                },
            ];
            match client.chat_complete(&messages).await {
                Ok(resp) => {
                    let u = resp.usage;
                    eprintln!(
                        "[e2e] call {}: prompt={} completion={} cache_read={} cache_creation={} content_len={}",
                        n, u.prompt_tokens, u.completion_tokens, u.cache_read_input_tokens,
                        u.cache_creation_input_tokens, resp.content.len()
                    );
                    if u.cache_read_input_tokens > 0 {
                        saw_hit = true;
                        if n == 1 {
                            first_call_hit = true;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[e2e] call {} FAILED: {:?}", n, e);
                    panic!("DeepSeek 端到端请求失败（call {}）: {:?}", n, e);
                }
            }
        }

        let stats = cache.stats();
        eprintln!(
            "[e2e] server_cache_read={} server_cache_creation={} hit_rate={:.3}",
            stats.server_cache_read_tokens,
            stats.server_cache_creation_tokens,
            stats.server_cache_hit_rate()
        );
        assert!(
            saw_hit,
            "DeepSeek 应在重复稳定前缀上命中自动前缀缓存（cache_read_input_tokens > 0）"
        );
        if warmup_ok {
            assert!(
                first_call_hit,
                "warmup 后首个真实请求应直接命中（消除冷启动），但 call1 未命中"
            );
        }
    }
}
