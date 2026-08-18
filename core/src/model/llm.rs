//! LLM 调用封装
//!
//! 统一封装 LLM API 调用，支持 OpenAI 兼容接口。
//! 提供非流式和流式两种完成方式，以及 token 估算和错误重试能力。

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::types::Message;

use crate::cache::{CacheEntry, PrefixCache};

/// LLM 提供商
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LLMProvider {
    /// OpenAI 兼容 API
    OpenAI,
    /// DeepSeek API
    DeepSeek,
    /// 自定义 API
    #[serde(rename = "custom")]
    Custom {
        /// 自定义 base URL
        base_url: String,
    },
}

impl Default for LLMProvider {
    fn default() -> Self {
        Self::DeepSeek
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

fn default_timeout_secs() -> u64 {
    60
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
        }
    }
}

impl LLMConfig {
    /// 从主应用配置的模型段构建 LLM 客户端配置（Bloom 并行编排等复用同一后端）。
    /// provider 解析与 ModelRouter 保持一致（openai/deepseek/其余视为 custom）。
    pub fn from_model_config(config: &crate::persistence::config::Config) -> Self {
        let m = &config.model;
        Self {
            provider: match m.provider.as_str() {
                "openai" => LLMProvider::OpenAI,
                "deepseek" => LLMProvider::DeepSeek,
                _ => LLMProvider::Custom {
                    base_url: m.base_url.clone(),
                },
            },
            model: m.model_name.clone(),
            api_key: m.api_key.clone(),
            base_url: m.base_url.clone(),
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            timeout_secs: default_timeout_secs(),
            thinking: crate::model::thinking::ThinkingEffort::Max,
            context_window: 256 * 1024,
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
            provider: match provider.as_str() {
                "openai" => LLMProvider::OpenAI,
                "deepseek" => LLMProvider::DeepSeek,
                _ => LLMProvider::Custom {
                    base_url: base_url.clone(),
                },
            },
            model: cop.model_name.clone(),
            api_key,
            base_url,
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            timeout_secs: default_timeout_secs(),
            thinking: crate::model::thinking::ThinkingEffort::Off,
            context_window: default_context_window(),
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
}

/// 流式接收器
pub type StreamReceiver = mpsc::Receiver<Result<LLMChunk>>;

/// 重试配置
const MAX_RETRIES: u32 = 3;
const BASE_DELAY_MS: u64 = 1000;

/// 判断错误是否可重试
fn is_retryable_error(status: Option<u16>) -> bool {
    matches!(status, Some(429) | Some(502) | Some(503) | Some(504) | None)
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

impl LLMClient {
    /// 创建新的 LLM 客户端
    pub fn new(config: LLMConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .unwrap_or_default();
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

    /// 构建请求 URL
    fn chat_url(&self) -> String {
        format!(
            "{}/v1/chat/completions",
            self.config.base_url.trim_end_matches('/')
        )
    }

    /// 构建请求头
    fn headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", self.config.api_key).parse().unwrap(),
        );
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
                            && prev["content"].as_str().unwrap_or("") == ""
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

    /// 构建请求体（统一入口：是否流式 + 是否带工具定义）
    fn build_body(
        &self,
        messages: &[Message],
        stream: bool,
        tools: Option<&[ToolDef]>,
    ) -> serde_json::Value {
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
        if let Some(effort) = self.config.thinking.reasoning_effort() {
            if crate::model::thinking::supports_reasoning_effort(&self.config.model) {
                body["reasoning_effort"] = serde_json::Value::String(effort.to_string());
            }
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

    /// 构建请求体（非流式，无工具）
    fn build_request_body(&self, messages: &[Message]) -> serde_json::Value {
        self.build_body(messages, false, None)
    }

    /// 构建流式请求体（无工具）
    fn build_stream_request_body(&self, messages: &[Message]) -> serde_json::Value {
        self.build_body(messages, true, None)
    }

    /// 非流式聊天完成
    pub async fn chat_complete(&self, messages: &[Message]) -> Result<LLMResponse> {
        let url = self.chat_url();
        let headers = self.headers();
        let body = self.build_request_body(messages);

        let mut last_error = None;
        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                let delay = BASE_DELAY_MS * 2u64.pow(attempt - 1);
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }

            let response = self
                .client
                .post(&url)
                .headers(headers.clone())
                .json(&body)
                .send()
                .await;

            match response {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if resp.status().is_success() {
                        let parsed = parse_chat_response(resp.text().await?, &self.config.model)?;
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
                        last_error = Some(anyhow!("API error ({}): {}", status, error_text));
                        continue;
                    } else if status == 404 && self.config.model != "deepseek-v4-pro" {
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
                        return Err(anyhow!("API error ({}): {}", status, error_text));
                    }
                }
                Err(e) => {
                    if e.is_timeout() && is_retryable_error(None) {
                        last_error = Some(anyhow!("Request timeout: {}", e));
                        continue;
                    }
                    return Err(anyhow!("Request failed: {}", e));
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
            None => return self.chat_complete(messages).await,
        };
        // KV-Cache 极简实践 P1：稳定前缀（system/persona/KB）前置，放大命中率；
        // 同时让发送给服务端的请求也是稳定前缀在前，配合服务端 prompt caching。
        let ordered = crate::context::optimize::reorder_stable_prefix(messages);
        // 观测稳定前缀复用（服务端 KV 块复用，如实计量，不虚报客户端命中）
        let prefix_hash =
            crate::cache::hash_text(&crate::context::optimize::stable_prefix_text(&ordered));
        let prefix_tokens = crate::context::optimize::stable_prefix_tokens(
            &ordered,
            model_family(&self.config.model),
        ) as u32;
        cache.note_prefix(prefix_hash, prefix_tokens);

        let key = crate::cache::hash_text(&self.cache_key_text(&ordered));
        if let Some(e) = cache.get(key) {
            return Ok(LLMResponse {
                content: e.content,
                thinking: None,
                model: self.config.model.clone(),
                usage: TokenUsage {
                    prompt_tokens: 0,
                    completion_tokens: e.completion_tokens,
                    total_tokens: e.completion_tokens,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                },
                tool_calls: vec![],
            });
        }
        let resp = self.chat_complete(&ordered).await?;
        cache.put(
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
        let prefix_hash =
            crate::cache::hash_text(&crate::context::optimize::stable_prefix_text(&ordered));
        let prefix_tokens = crate::context::optimize::stable_prefix_tokens(
            &ordered,
            model_family(&self.config.model),
        ) as u32;
        cache.note_prefix(prefix_hash, prefix_tokens);
        let key = crate::cache::hash_text(&format!(
            "{}{}",
            self.cache_key_text(&ordered),
            serde_json::to_string(tools).unwrap_or_default()
        ));
        if let Some(e) = cache.get(key) {
            return Ok(LLMResponse {
                content: e.content,
                thinking: None,
                model: self.config.model.clone(),
                usage: TokenUsage {
                    prompt_tokens: 0,
                    completion_tokens: e.completion_tokens,
                    total_tokens: e.completion_tokens,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                },
                tool_calls: vec![],
            });
        }
        let resp = self.chat_complete_with_tools(&ordered, tools).await?;
        cache.put(
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

    /// 服务端 KV 缓存预热（命中率优化）：只发含稳定前缀（system/persona/KB）的空调用。
    ///
    /// 配合服务端 prompt caching：先让服务端为该前缀预填充 KV 块，随后真实请求命中，
    /// 省去重复预填充。`max_tokens` 压到 1，仅触发预填充、不产生有意义输出，成本可忽略；
    /// 无稳定前缀（无 system 消息）时直接返回、不发请求；失败静默忽略（预热是尽力而为）。
    pub async fn warmup(&self, messages: &[Message]) -> Result<()> {
        let ordered = crate::context::optimize::reorder_stable_prefix(messages);
        let stable: Vec<Message> = ordered
            .into_iter()
            .filter(|m| matches!(m, Message::System { .. }))
            .collect();
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

        tokio::spawn(async move {
            let response = match client.post(&url).headers(headers).json(&body).send().await {
                Ok(resp) => resp,
                Err(e) => {
                    let _ = tx.send(Err(anyhow!("Stream request failed: {}", e))).await;
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let error_text = response.text().await.unwrap_or_default();
                let _ = tx
                    .send(Err(anyhow!("API error ({}): {}", status, error_text)))
                    .await;
                return;
            }

            let mut current_content = String::new();
            let mut current_thinking = String::new();

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
                                        }))
                                        .await;
                                    return;
                                }

                                match parse_stream_chunk(data, &model) {
                                    Ok(parsed) => {
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
                        let _ = tx.send(Err(anyhow!("Stream read error: {}", e))).await;
                        return;
                    }
                }
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
        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                let delay = BASE_DELAY_MS * 2u64.pow(attempt - 1);
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }

            let response = self
                .client
                .post(&url)
                .headers(headers.clone())
                .json(&body)
                .send()
                .await;

            match response {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if resp.status().is_success() {
                        let parsed = parse_chat_response(resp.text().await?, &self.config.model)?;
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
                        last_error = Some(anyhow!("API error ({}): {}", status, error_text));
                        continue;
                    } else {
                        let error_text = resp.text().await.unwrap_or_default();
                        return Err(anyhow!("API error ({}): {}", status, error_text));
                    }
                }
                Err(e) => {
                    if e.is_timeout() && is_retryable_error(None) {
                        last_error = Some(anyhow!("Request timeout: {}", e));
                        continue;
                    }
                    return Err(anyhow!("Request failed: {}", e));
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

        tokio::spawn(async move {
            let response = match client.post(&url).headers(headers).json(&body).send().await {
                Ok(resp) => resp,
                Err(e) => {
                    let _ = tx.send(Err(anyhow!("Stream request failed: {}", e))).await;
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let error_text = response.text().await.unwrap_or_default();
                let _ = tx
                    .send(Err(anyhow!("API error ({}): {}", status, error_text)))
                    .await;
                return;
            }

            let mut tool_accum: Vec<PartialToolCall> = Vec::new();
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
                                        }))
                                        .await;
                                    return;
                                }

                                match parse_stream_chunk(data, &model) {
                                    Ok(parsed) => {
                                        for tc in parsed.tool_calls {
                                            accumulate_tool_call(&mut tool_accum, tc);
                                        }
                                        let chunk = LLMChunk {
                                            content: parsed.content,
                                            thinking: parsed.thinking,
                                            finish_reason: parsed.finish_reason,
                                            tool_calls: None,
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
                        let _ = tx.send(Err(anyhow!("Stream read error: {}", e))).await;
                        return;
                    }
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
    struct PromptTokensDetails {
        #[serde(rename = "cached_tokens", default)]
        cached_tokens: u32,
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
                // 或嵌套 `prompt_tokens_details.cached_tokens`；三者归一为「服务端缓存命中 token」用于统一计量，
                // 并上报进程级前缀缓存追踪器（呼应 GUI_Agent PrefixCacheTracker，使缓存命中率可观测）。
                let cache_read = u.cache_read_input_tokens
                    + u.prompt_cache_hit_tokens
                    + u.prompt_tokens_details.map_or(0, |d| d.cached_tokens);
                let cache_creation = u.cache_creation_input_tokens;
                crate::cache::report_prefix_cache_usage(
                    u.prompt_tokens as u64,
                    cache_read as u64,
                    cache_creation as u64,
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
struct ParsedChunk {
    content: Option<String>,
    thinking: Option<String>,
    finish_reason: Option<String>,
    tool_calls: Vec<RawToolCallDelta>,
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
    struct StreamUsage {
        prompt_tokens: Option<u32>,
        completion_tokens: Option<u32>,
        total_tokens: Option<u32>,
        #[serde(default)]
        cache_read_input_tokens: Option<u32>,
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

    // 最终 usage 块（choices 为空，含 token 用量）
    if let Some(u) = &chunk.usage {
        if chunk.choices.is_empty() {
            let usage = TokenUsage {
                prompt_tokens: u.prompt_tokens.unwrap_or(0),
                completion_tokens: u.completion_tokens.unwrap_or(0),
                total_tokens: u.total_tokens.unwrap_or(0),
                cache_read_input_tokens: u.cache_read_input_tokens.unwrap_or(0),
                cache_creation_input_tokens: 0,
            };
            if usage.total_tokens > 0 {
                crate::usage::record(_model, &usage);
            }
            // usage-only chunk：不产生内容，返回空 ParsedChunk（不是错误）
            return Ok(ParsedChunk {
                content: None,
                thinking: None,
                finish_reason: None,
                tool_calls: Vec::new(),
            });
        }
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;
    use std::sync::Arc;

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
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = body.to_string();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 16384];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                *captured.lock().unwrap() = Some(String::from_utf8_lossy(&buf[..n]).to_string());
                let reason = if status >= 500 { "Server Error" } else { "OK" };
                let header = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(body.as_bytes()).await;
                let _ = socket.flush().await;
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

    /// 简易 mock HTTP 服务器：依次返回给定响应
    async fn mock_server(responses: Vec<(u16, String)>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for (status, body) in responses {
                if let Ok((mut socket, _)) = listener.accept().await {
                    let mut buf = [0u8; 4096];
                    let _ = socket.read(&mut buf).await;
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
        let s = cache.stats();
        assert!(
            s.prefix_hits >= 1,
            "共享 system 前缀应被记为复用, got {}",
            s.prefix_hits
        );
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
        assert_eq!(resp.usage.cache_read_input_tokens, 80, "应归一 prompt_cache_hit_tokens");
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
        assert!((s.server_cache_hit_rate() - (240.0 / 270.0)).abs() < 1e-9, "240 read / (240 read + 30 creation) ≈ 0.889");
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
        let system: String = std::iter::repeat(unit).take(60).collect();
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
