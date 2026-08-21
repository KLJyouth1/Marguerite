//! 模型路由
//!
//! 根据任务类型和复杂度选择合适的模型，支持路由规则配置和回退机制。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};

use super::llm::{LLMClient, LLMConfig, LLMResponse, StreamReceiver, ToolDef};
use crate::cache::PrefixCache;
use crate::persistence::config::Config;
use crate::types::Message;

/// 任务意图类型
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum IntentType {
    /// 代码相关
    Code,
    /// 问答
    Chat,
    /// 推理
    Reasoning,
    /// 翻译
    Translation,
    /// 总结
    Summarization,
    /// 通用
    General,
}

/// 任务复杂度
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Complexity {
    /// 简单任务，用轻量模型
    Simple,
    /// 中等任务
    Medium,
    /// 复杂任务，用强推理模型
    Complex,
}

/// 模型路由规则
#[derive(Debug, Clone)]
pub struct ModelRoutingRule {
    /// 匹配的意图类型
    pub intent_pattern: Option<IntentType>,
    /// 匹配的复杂度
    pub complexity: Option<Complexity>,
    /// 使用的模型
    pub model: String,
    /// 优先级（数字越大越优先）
    pub priority: u8,
}

/// 模型路由器
pub struct ModelRouter {
    /// 默认客户端
    default_client: Arc<LLMClient>,
    /// 回退客户端（单个，向后兼容 `with_fallback`）
    fallback_client: Option<Arc<LLMClient>>,
    /// 故障转移链（KV-Cache 极简工程：蒸馏 外部参考 provider-plane 的「有序候选链」）。
    /// 主客户端失败（含重试耗尽）后，按此顺序自动切下一级，直到成功或全部失败。
    /// 所有候选与 `default_client` 同级（独立后端），不构成缓存共享。
    failover_chain: Vec<Arc<LLMClient>>,
    /// 路由规则列表
    routing_rules: Vec<ModelRoutingRule>,
    /// 模型名 → 客户端缓存（按需懒创建）
    clients: Mutex<HashMap<String, Arc<LLMClient>>>,
    /// 创建新客户端用的配置模板
    template_config: LLMConfig,
    /// 跨所有候选客户端共享的客户端缓存（精确前缀命中 + 服务端前缀复用观测 + 预热）
    cache: Arc<PrefixCache>,
}

impl ModelRouter {
    /// 从配置创建模型路由器
    pub fn new(config: &Config) -> Self {
        let template_config = Self::build_template_config(config);
        // 跨所有客户端共享一份缓存：精确前缀命中 + 服务端前缀复用观测 + 预热，全部路径受益。
        let cache = Arc::new(PrefixCache::new(2048));

        let default_client =
            Arc::new(LLMClient::new(template_config.clone()).with_cache(cache.clone()));

        Self {
            default_client,
            fallback_client: None,
            // W-4 生产接线：默认即构造故障转移链（派生同提供商备用模型），
            // 使 provider 故障转移在默认构造下激活，无需调用方显式 with_fallback_chain。
            failover_chain: Self::default_failover_chain(config, &cache),
            routing_rules: default_routing_rules(),
            clients: Mutex::new(HashMap::new()),
            template_config,
            cache,
        }
    }

    /// 从配置创建模型路由器，并经凭证缝解析 API Key（凭证台账优先，config 回退）。
    ///
    /// 凭证缝接入（DSH 融合 v5）：`credentials` 为 Some 时以
    /// [`crate::credentials::resolve_llm_key`] 覆盖 `config.model.api_key`
    /// （provider 路由键 → 常见引用 → 回退），None 时等价 [`ModelRouter::new`]。
    pub fn new_with_credentials(
        config: &Config,
        credentials: Option<Arc<crate::credentials::CredentialsStore>>,
    ) -> Self {
        let Some(store) = credentials else {
            return Self::new(config);
        };
        let resolved = crate::credentials::resolve_llm_key(
            &store,
            &config.model.provider,
            &config.model.api_key,
        );
        let mut overridden = config.clone();
        overridden.model.api_key = resolved;
        Self::new(&overridden)
    }

    /// 设置回退客户端
    pub fn with_fallback(mut self, config: LLMConfig) -> Self {
        self.fallback_client =
            Some(Arc::new(LLMClient::new(config).with_cache(self.cache.clone())));
        self
    }

    /// 设置故障转移链（多个候选后端，按序尝试）。
    ///
    /// 蒸馏 外部参考 `runWithModelFallback` 的有序候选语义：第一个为主后端，
    /// 其后为逐级回退。主后端（含 `default_client` 重试耗尽、单 `fallback_client`）
    /// 失败即自动切下一级，直到成功或全部失败。
    pub fn with_fallback_chain(mut self, configs: Vec<LLMConfig>) -> Self {
        self.failover_chain = configs
            .into_iter()
            .map(|cfg| Arc::new(LLMClient::new(cfg).with_cache(self.cache.clone())))
            .collect();
        self
    }

    /// 故障转移链配置（可观测：CLI/调试可展示当前备份后端；与 `default_client` 同级）
    pub fn failover_chain(&self) -> Vec<&LLMConfig> {
        self.failover_chain.iter().map(|c| &c.config).collect()
    }

    /// 据配置构建模板 `LLMConfig`（主客户端与故障转移候选共用 provider/base_url/key）。
    fn build_template_config(config: &Config) -> LLMConfig {
        LLMConfig {
            provider: match config.model.provider.as_str() {
                "openai" => super::llm::LLMProvider::OpenAI,
                "deepseek" => super::llm::LLMProvider::DeepSeek,
                _ => super::llm::LLMProvider::Custom {
                    base_url: config.model.base_url.clone(),
                },
            },
            model: config.model.model_name.clone(),
            api_key: config.model.api_key.clone(),
            base_url: config.model.base_url.clone(),
            ..Default::default()
        }
    }

    /// 默认故障转移链：据主模型派生「同提供商备用模型」作为候选。
    ///
    /// 蒸馏 外部参考 provider-plane 有序候选语义，但来源诚实——仅复用用户已配置的
    /// provider / base_url / api_key，不编造任何凭据或新端点。已知主模型映射到其
    /// 同提供商 sibling（如 deepseek-chat ↔ deepseek-reasoner）；未知/自定义模型
    /// 无可靠 sibling，返回空（不伪造备用后端）。返回 `Vec<Arc<LLMClient>>`，
    /// 与主字段类型一致（仿 `with_fallback_chain` 把 config 包成 client）。
    fn default_failover_chain(config: &Config, cache: &Arc<PrefixCache>) -> Vec<Arc<LLMClient>> {
        match provider_sibling_model(&config.model.model_name) {
            Some(sibling) if sibling != config.model.model_name.as_str() => {
                let mut cfg = Self::build_template_config(config);
                cfg.model = sibling.to_string();
                vec![Arc::new(LLMClient::new(cfg).with_cache(cache.clone()))]
            }
            _ => Vec::new(),
        }
    }

    /// 组装一次调用的有序候选链：`[primary, fallback_client?, ...failover_chain]`，
    /// 并按 `(base_url, model)` 去重，避免对完全相同的后端重复请求。
    fn candidate_chain(&self, primary: Arc<LLMClient>) -> Vec<Arc<LLMClient>> {
        let mut out = vec![primary];
        if let Some(fb) = &self.fallback_client {
            out.push(fb.clone());
        }
        for c in &self.failover_chain {
            out.push(c.clone());
        }
        let mut seen = std::collections::HashSet::new();
        out.into_iter()
            .filter(|c| seen.insert((c.config.base_url.clone(), c.config.model.clone())))
            .collect()
    }

    /// 添加路由规则
    pub fn add_rule(&mut self, rule: ModelRoutingRule) {
        self.routing_rules.push(rule);
    }

    /// 解析命中规则的模型名
    fn resolve_model(&self, intent: &IntentType, complexity: &Complexity) -> String {
        self.routing_rules
            .iter()
            .filter(|rule| {
                let intent_matches = match &rule.intent_pattern {
                    Some(pattern) => pattern == intent,
                    None => true,
                };
                let complexity_matches = match &rule.complexity {
                    Some(c) => c == complexity,
                    None => true,
                };
                intent_matches && complexity_matches
            })
            .max_by_key(|rule| rule.priority)
            .map(|rule| rule.model.clone())
            .unwrap_or_else(|| self.default_client.config.model.clone())
    }

    /// 获取指定模型名的客户端（懒创建并缓存）
    fn client_for(&self, model: &str) -> Arc<LLMClient> {
        if model == self.default_client.config.model {
            return self.default_client.clone();
        }
        let mut map = self.clients.lock().unwrap();
        map.entry(model.to_string())
            .or_insert_with(|| {
                let mut cfg = self.template_config.clone();
                cfg.model = model.to_string();
                Arc::new(LLMClient::new(cfg).with_cache(self.cache.clone()))
            })
            .clone()
    }

    /// 根据意图和复杂度选择模型客户端
    ///
    /// 命中路由规则时返回对应模型的客户端，否则返回默认客户端。
    pub fn select_model(&self, intent: &IntentType, complexity: Complexity) -> Arc<LLMClient> {
        self.client_for(&self.resolve_model(intent, &complexity))
    }

    /// 按意图和复杂度路由的聊天调用
    ///
    /// 选中的主模型失败后，按故障转移链（`candidate_chain`）逐级切下一级，
    /// 直到成功或全部失败（蒸馏 外部参考 有序候选语义）。
    pub async fn chat_for(
        &self,
        intent: &IntentType,
        complexity: Complexity,
        messages: &[Message],
    ) -> Result<LLMResponse> {
        let primary = self.select_model(intent, complexity);
        let chain = self.candidate_chain(primary);
        try_failover_chain(&chain, |c| Box::pin(c.chat_complete_cached(messages))).await
    }

    /// 带输出预算上限的按意图/复杂度路由聊天（锚定 bootstrap 通道）：
    /// `max_tokens` 为 Some 时仅在**选中主模型**上覆盖输出上限（故障转移链
    /// 成员保持各自配置），None 时等价 [`ModelRouter::chat_for`]。
    pub async fn chat_for_capped(
        &self,
        intent: &IntentType,
        complexity: Complexity,
        messages: &[Message],
        max_tokens: Option<u32>,
    ) -> Result<LLMResponse> {
        let primary = match max_tokens {
            Some(cap) => Arc::new(
                self.select_model(intent, complexity)
                    .as_ref()
                    .clone()
                    .with_max_tokens(cap),
            ),
            None => self.select_model(intent, complexity),
        };
        let chain = self.candidate_chain(primary);
        try_failover_chain(&chain, |c| Box::pin(c.chat_complete_cached(messages))).await
    }

    /// 非流式聊天
    ///
    /// 默认客户端失败后，按故障转移链（`candidate_chain`）逐级切下一级。
    pub async fn chat(&self, messages: &[Message]) -> Result<LLMResponse> {
        let chain = self.candidate_chain(self.default_client.clone());
        try_failover_chain(&chain, |c| Box::pin(c.chat_complete_cached(messages))).await
    }

    /// 带工具定义的聊天（function calling）
    ///
    /// 通过默认客户端发送 `tools`，模型可在响应中返回 `tool_calls`，
    /// 由 agent 工具回路解析并执行。无回退（工具调用须对齐当前主模型）。
    pub async fn chat_with_tools(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<LLMResponse> {
        // thinking 自适应（DeepSeek-V4-Pro 专项）：按任务类型（代码/推理/长任务/工具/对话）
        // 动态调整 max_tokens、reasoning_effort 与上下文档，让专项优化真正落到请求体。
        let kind = classify_task(messages);
        let client = self
            .default_client
            .as_ref()
            .clone()
            .with_adaptive_profile(kind);
        client.chat_complete_with_tools_cached(messages, tools).await
    }

    /// 带输出预算上限的工具聊天（锚定 bootstrap 通道）：`max_tokens` 为 Some 时
    /// 在自适应档之上再覆盖输出上限（首个锚定请求用），None 时等价
    /// [`ModelRouter::chat_with_tools`]。
    pub async fn chat_with_tools_capped(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
        max_tokens: Option<u32>,
    ) -> Result<LLMResponse> {
        let kind = classify_task(messages);
        let base = self
            .default_client
            .as_ref()
            .clone()
            .with_adaptive_profile(kind);
        let client = match max_tokens {
            Some(cap) => base.with_max_tokens(cap),
            None => base,
        };
        client.chat_complete_with_tools_cached(messages, tools).await
    }

    /// 流式聊天
    ///
    /// 默认客户端失败后，按故障转移链（`candidate_chain`）逐级切下一级。
    pub async fn chat_stream(&self, messages: &[Message]) -> Result<StreamReceiver> {
        let chain = self.candidate_chain(self.default_client.clone());
        try_failover_chain(&chain, |c| Box::pin(c.chat_complete_stream(messages))).await
    }

    /// 路由版聊天（Track A）：按决策档的 thinking_mode 注入思考预算后走故障转移链。
    ///
    /// 选中主模型失败按 `candidate_chain` 逐级切下一级；思考预算由 `decision.thinking_effort()`
    /// 经 `LLMClient::with_thinking` 落到选中客户端（非工具路径此前恒为 Off，本方法修正）。
    pub async fn chat_for_route(
        &self,
        intent: &IntentType,
        complexity: Complexity,
        decision: &crate::router::RouteDecision,
        messages: &[Message],
    ) -> Result<LLMResponse> {
        let effort = decision.thinking_effort();
        let primary = Arc::new(
            self.select_model(intent, complexity)
                .as_ref()
                .clone()
                .with_thinking(effort),
        );
        let chain = self.candidate_chain(primary);
        try_failover_chain(&chain, |c| Box::pin(c.chat_complete_cached(messages))).await
    }

    /// 路由版流式聊天（Track A）。
    pub async fn chat_stream_route(
        &self,
        intent: &IntentType,
        complexity: Complexity,
        decision: &crate::router::RouteDecision,
        messages: &[Message],
    ) -> Result<StreamReceiver> {
        let effort = decision.thinking_effort();
        let primary = Arc::new(
            self.select_model(intent, complexity)
                .as_ref()
                .clone()
                .with_thinking(effort),
        );
        let chain = self.candidate_chain(primary);
        try_failover_chain(&chain, |c| Box::pin(c.chat_complete_stream(messages))).await
    }

    /// 路由版带工具聊天（Track A）：以 route 决策覆盖自适应思考预算。
    pub async fn chat_with_tools_route(
        &self,
        intent: &IntentType,
        complexity: Complexity,
        decision: &crate::router::RouteDecision,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<LLMResponse> {
        let effort = decision.thinking_effort();
        let primary = Arc::new(
            self.select_model(intent, complexity)
                .as_ref()
                .clone()
                .with_thinking(effort),
        );
        let chain = self.candidate_chain(primary);
        try_failover_chain(&chain, |c| {
            Box::pin(c.chat_complete_with_tools_cached(messages, tools))
        })
        .await
    }

    /// 带工具定义的**流式**聊天（P-10 真流式回路的上游）
    ///
    /// 与 `chat_with_tools` 同策略：走默认客户端、**不经故障转移链** ——
    /// 工具调用须对齐当前主模型的 function-calling 契约，中途换后端会导致
    /// tool_call id / schema 语义漂移。流式工具调用在流末以一个携带
    /// `tool_calls` 的块一次性给出。
    pub async fn chat_stream_with_tools(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<StreamReceiver> {
        self.default_client
            .chat_stream_with_tools(messages, tools)
            .await
    }

    /// 获取默认客户端引用
    pub fn default_client(&self) -> &LLMClient {
        self.default_client.as_ref()
    }

    /// 获取回退客户端引用
    pub fn fallback_client(&self) -> Option<&LLMClient> {
        self.fallback_client.as_ref().map(Arc::as_ref)
    }
}

impl Clone for ModelRouter {
    fn clone(&self) -> Self {
        Self {
            default_client: self.default_client.clone(),
            fallback_client: self.fallback_client.clone(),
            failover_chain: self.failover_chain.clone(),
            routing_rules: self.routing_rules.clone(),
            clients: Mutex::new(self.clients.lock().unwrap().clone()),
            template_config: self.template_config.clone(),
            cache: self.cache.clone(),
        }
    }
}

impl std::fmt::Debug for ModelRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelRouter")
            .field("model", &self.default_client.config.model)
            .field("routing_rules", &self.routing_rules.len())
            .field("failover_chain", &self.failover_chain.len())
            .finish()
    }
}

/// 从消息序列中取最后一条 User 消息文本，并分类任务类型（供 thinking 自适应）。
fn classify_task(messages: &[Message]) -> crate::model::thinking::TaskKind {
    let text = messages
        .iter()
        .rev()
        .find_map(|m| match m {
            Message::User { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .unwrap_or_default();
    crate::model::thinking::TaskKind::classify(text)
}

/// 故障转移核心（KV-Cache 极简工程：蒸馏 外部参考 `runWithModelFallback`）。
///
/// 按 `candidates` 顺序逐个用 `runner` 尝试；第一个 `Ok` 立即返回；
/// 若候选全部失败，返回携带各级失败明细的错误（provider/model + 原因），
/// 供上层给出可诊断信息（对应 外部参考 `FallbackSummaryError`）。
///
/// 故障转移核心（KV-Cache 极简工程：蒸馏 外部参考 `runWithModelFallback`）。
///
/// 按 `candidates` 顺序逐个用 `runner` 尝试；第一个 `Ok` 立即返回；
/// 若候选全部失败，返回携带各级失败明细的错误（provider/model + 原因），
/// 供上层给出可诊断信息（对应 外部参考 `FallbackSummaryError` 的可观测性）。
///
/// `runner` 返回的未来借用候选客户端 `'cand` 生命周期（与候选切片同寿），
/// 因此真实调用处 `|c| Box::pin(c.chat_complete(messages))` 能直接复用客户端
/// 而无需克隆；`messages` 的存活期不短于 `'cand`，故可一并借用。
/// 未来标注 `+ Send` 以便故障转移链可在 `tokio::spawn` 的任务内使用。
pub(crate) async fn try_failover_chain<'cand, T>(
    candidates: &'cand [Arc<LLMClient>],
    runner: impl Fn(&'cand LLMClient) -> Pin<Box<dyn Future<Output = Result<T>> + Send + 'cand>>,
) -> Result<T> {
    if candidates.is_empty() {
        return Err(anyhow!("failover chain is empty (no candidate client)"));
    }
    let mut attempts: Vec<(String, String)> = Vec::with_capacity(candidates.len());
    for c in candidates {
        match runner(c).await {
            Ok(response) => return Ok(response),
            Err(e) => attempts.push((c.config.model.clone(), e.to_string())),
        }
    }
    let detail = attempts
        .iter()
        .map(|(model, err)| format!("{model}: {err}"))
        .collect::<Vec<_>>()
        .join(" | ");
    Err(anyhow!(
        "all failover candidates failed ({}/{}): {detail}",
        attempts.len(),
        candidates.len()
    ))
}

/// 获取默认路由规则（供 CLI 展示）
pub fn default_rules_public() -> Vec<ModelRoutingRule> {
    default_routing_rules()
}

/// 已知主模型 → 同提供商备用模型（故障转移默认候选）。
///
/// 仅含本仓明确支持、且与主模型不同的模型；其余返回 `None`（不编造备用后端）。
/// 这是 W-4 默认故障转移链的唯一来源，确保派生候选复用用户真实凭据。
fn provider_sibling_model(primary: &str) -> Option<&'static str> {
    match primary {
        "deepseek-v4-pro" => Some("deepseek-v4-flash"),
        "deepseek-v4-flash" => Some("deepseek-v4-pro"),
        "deepseek-chat" => Some("deepseek-v4-pro"),
        "deepseek-reasoner" => Some("deepseek-v4-pro"),
        "gpt-4o" => Some("gpt-4o-mini"),
        "gpt-4o-mini" => Some("gpt-4o"),
        "claude-sonnet-4" => Some("claude-haiku-4"),
        "claude-opus-4" => Some("claude-haiku-4"),
        "claude-haiku-4" => Some("claude-sonnet-4"),
        _ => None,
    }
}

/// 默认路由规则
fn default_routing_rules() -> Vec<ModelRoutingRule> {
    vec![
        ModelRoutingRule {
            intent_pattern: Some(IntentType::Code),
            complexity: Some(Complexity::Complex),
            model: "deepseek-v4-pro".to_string(),
            priority: 100,
        },
        ModelRoutingRule {
            intent_pattern: Some(IntentType::Reasoning),
            complexity: Some(Complexity::Complex),
            model: "deepseek-v4-pro".to_string(),
            priority: 100,
        },
        ModelRoutingRule {
            intent_pattern: Some(IntentType::Code),
            complexity: Some(Complexity::Medium),
            model: "deepseek-v4-flash".to_string(),
            priority: 80,
        },
        ModelRoutingRule {
            intent_pattern: Some(IntentType::Translation),
            complexity: None,
            model: "deepseek-v4-flash".to_string(),
            priority: 70,
        },
        ModelRoutingRule {
            intent_pattern: Some(IntentType::Summarization),
            complexity: None,
            model: "deepseek-v4-flash".to_string(),
            priority: 70,
        },
        ModelRoutingRule {
            intent_pattern: None,
            complexity: None,
            model: "deepseek-v4-pro".to_string(),
            priority: 50,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::llm::TokenUsage;
    use anyhow::anyhow;
    // Message is imported via super::* from the module

    #[test]
    fn test_router_select_model() {
        let config = Config::default();
        let router = ModelRouter::new(&config);

        // 通用简单任务 → 默认模型
        let client = router.select_model(&IntentType::General, Complexity::Simple);
        assert_eq!(client.config.model, "deepseek-v4-pro");

        // 复杂代码任务 → 命中 priority 100 规则，切换到强推理模型
        let client = router.select_model(&IntentType::Code, Complexity::Complex);
        assert_eq!(client.config.model, "deepseek-v4-pro");

        // 中等代码任务 → 命中 priority 80 规则（deepseek-v4-flash）
        let client = router.select_model(&IntentType::Code, Complexity::Medium);
        assert_eq!(client.config.model, "deepseek-v4-flash");

        // 翻译任务 → 命中 priority 70 规则
        let client = router.select_model(&IntentType::Translation, Complexity::Simple);
        assert_eq!(client.config.model, "deepseek-v4-flash");
    }

    #[test]
    fn test_router_select_model_caches_clients() {
        let config = Config::default();
        let router = ModelRouter::new(&config);

        // 复杂推理任务触发 deepseek-v4-pro 客户端懒创建
        let client1 = router.select_model(&IntentType::Reasoning, Complexity::Complex);
        let client2 = router.select_model(&IntentType::Reasoning, Complexity::Complex);
        assert_eq!(client1.config.model, "deepseek-v4-pro");
        // 同一模型名应复用缓存客户端
        assert!(Arc::ptr_eq(&client1, &client2));
    }

    #[test]
    fn test_router_new_with_credentials_overrides_api_key() {
        // 凭证缝（DSH 融合 v5）：台账中的 provider 路由键优先于 config 直读值
        let dir = std::env::temp_dir().join(format!("mgt_router_cred_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Arc::new(crate::credentials::CredentialsStore::load(
            &dir.join("credentials.json"),
        ));
        store
            .set_record(
                &crate::credentials::CredentialKey::new("llm", "deepseek"),
                crate::credentials::CredentialRecord::api_key("sk-from-store"),
            )
            .unwrap();
        let mut config = Config::default();
        config.model.api_key = "sk-from-config".to_string();
        let router = ModelRouter::new_with_credentials(&config, Some(store));
        assert_eq!(router.default_client.config.api_key, "sk-from-store");
        // 未提供台账 → 等价 new（config 直读）
        let router2 = ModelRouter::new_with_credentials(&config, None);
        assert_eq!(router2.default_client.config.api_key, "sk-from-config");
    }

    #[test]
    fn test_default_routing_rules() {
        let rules = default_routing_rules();
        assert!(!rules.is_empty());

        // 验证默认规则存在
        let default_rule = rules
            .iter()
            .find(|r| r.intent_pattern.is_none() && r.complexity.is_none());
        assert!(default_rule.is_some());
        assert_eq!(default_rule.unwrap().priority, 50);
    }

    #[test]
    fn test_routing_rule_priority() {
        let rules = default_routing_rules();

        // 复杂代码任务的优先级应该高于通用规则
        let code_complex = rules
            .iter()
            .find(|r| {
                r.intent_pattern == Some(IntentType::Code)
                    && r.complexity == Some(Complexity::Complex)
            })
            .unwrap();
        assert_eq!(code_complex.priority, 100);
    }

    #[test]
    fn test_router_with_fallback() {
        let config = Config::default();
        let fallback_config = LLMConfig {
            model: "gpt-4o-mini".to_string(),
            base_url: "https://api.openai.com".to_string(),
            ..Default::default()
        };

        let router = ModelRouter::new(&config).with_fallback(fallback_config);
        assert!(router.fallback_client().is_some());
        assert_eq!(
            router.fallback_client().unwrap().config.model,
            "gpt-4o-mini"
        );
    }

    #[test]
    fn test_add_rule() {
        let config = Config::default();
        let mut router = ModelRouter::new(&config);

        let rule = ModelRoutingRule {
            intent_pattern: Some(IntentType::Chat),
            complexity: Some(Complexity::Simple),
            model: "fast-model".to_string(),
            priority: 200,
        };
        router.add_rule(rule);

        // 验证新规则被应用：Chat + Simple → fast-model（priority 200 最高）
        let client = router.select_model(&IntentType::Chat, Complexity::Simple);
        assert_eq!(client.config.model, "fast-model");
    }

    #[test]
    fn test_intent_type_equality() {
        assert_eq!(IntentType::Code, IntentType::Code);
        assert_ne!(IntentType::Code, IntentType::Chat);
    }

    #[test]
    fn test_complexity_ordering() {
        assert!(Complexity::Simple != Complexity::Complex);
        assert!(Complexity::Medium != Complexity::Simple);
    }

    // ---- P-8 故障转移链（蒸馏 外部参考 provider-plane） ----

    #[test]
    fn test_router_with_fallback_chain_builds() {
        let config = Config::default();
        let chain = vec![
            LLMConfig {
                model: "gpt-4o-mini".to_string(),
                base_url: "https://api.openai.com".to_string(),
                ..Default::default()
            },
            LLMConfig {
                model: "claude-sonnet".to_string(),
                base_url: "https://api.anthropic.com".to_string(),
                ..Default::default()
            },
        ];
        let router = ModelRouter::new(&config).with_fallback_chain(chain);
        assert_eq!(router.failover_chain().len(), 2);
        assert_eq!(router.failover_chain()[0].model, "gpt-4o-mini");
        assert_eq!(router.failover_chain()[1].model, "claude-sonnet");
    }

    #[test]
    fn test_candidate_chain_ordering_and_dedupe() {
        let config = Config::default();
        let fb = LLMConfig {
            model: "gpt-4o-mini".to_string(),
            base_url: "https://api.openai.com".to_string(),
            ..Default::default()
        };
        let chain_cfg = LLMConfig {
            model: "claude-sonnet".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            ..Default::default()
        };
        let router = ModelRouter::new(&config)
            .with_fallback(fb.clone())
            .with_fallback_chain(vec![chain_cfg.clone()]);
        // P-8 候选顺序：[default(deepseek-v4-pro), fallback(gpt-4o-mini), chain(claude-sonnet)]
        let chain = router.candidate_chain(Arc::new(router.default_client().clone()));
        let models: Vec<&str> = chain.iter().map(|c| c.config.model.as_str()).collect();
        assert_eq!(
            models,
            vec!["deepseek-v4-pro", "gpt-4o-mini", "claude-sonnet"]
        );
        // 去重：插入一个与 default 相同的候选应被折叠
        let dup_router = ModelRouter::new(&config).with_fallback_chain(vec![LLMConfig {
            model: "deepseek-v4-pro".to_string(),
            base_url: "https://api.deepseek.com".to_string(),
            ..Default::default()
        }]);
        let dup_chain = dup_router.candidate_chain(Arc::new(dup_router.default_client().clone()));
        assert_eq!(dup_chain.len(), 1, "相同 (base_url,model) 候选应被去重");
    }

    #[tokio::test]
    async fn test_try_failover_chain_first_succeeds() {
        let a = Arc::new(LLMClient::new(LLMConfig {
            model: "a".to_string(),
            ..Default::default()
        }));
        let b = Arc::new(LLMClient::new(LLMConfig {
            model: "b".to_string(),
            ..Default::default()
        }));
        let out = try_failover_chain(&[a, b], |_c| {
            Box::pin(async move {
                Ok(LLMResponse {
                    content: "ok".to_string(),
                    thinking: None,
                    model: "a".to_string(),
                    usage: TokenUsage::default(),
                    tool_calls: vec![],
                })
            })
        })
        .await;
        assert!(out.is_ok());
        assert_eq!(out.unwrap().model, "a");
    }

    #[tokio::test]
    async fn test_try_failover_chain_falls_to_second() {
        let a = Arc::new(LLMClient::new(LLMConfig {
            model: "a".to_string(),
            ..Default::default()
        }));
        let b = Arc::new(LLMClient::new(LLMConfig {
            model: "b".to_string(),
            ..Default::default()
        }));
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let calls2 = calls.clone();
        let out = try_failover_chain(&[a, b], move |c| {
            let calls = calls2.clone();
            Box::pin(async move {
                calls.lock().unwrap().push(c.config.model.clone());
                if c.config.model == "a" {
                    Err(anyhow!("primary down"))
                } else {
                    Ok(LLMResponse {
                        content: "from-b".to_string(),
                        thinking: None,
                        model: "b".to_string(),
                        usage: TokenUsage::default(),
                        tool_calls: vec![],
                    })
                }
            })
        })
        .await;
        assert!(out.is_ok(), "应切到第二级候选 b");
        assert_eq!(out.unwrap().model, "b");
        // 两级都应被尝试（按顺序）
        assert_eq!(
            *calls.lock().unwrap(),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[tokio::test]
    async fn test_try_failover_chain_all_fail() {
        let a = Arc::new(LLMClient::new(LLMConfig {
            model: "a".to_string(),
            ..Default::default()
        }));
        let b = Arc::new(LLMClient::new(LLMConfig {
            model: "b".to_string(),
            ..Default::default()
        }));
        let out: Result<LLMResponse> = try_failover_chain(&[a, b], |c| {
            let m = c.config.model.clone();
            Box::pin(async move { Err(anyhow!("{} down", m)) })
        })
        .await;
        assert!(out.is_err());
        let msg = out.unwrap_err().to_string();
        // 失败摘要须携带各级明细（蒸馏 外部参考 FallbackSummaryError 的可诊断性）
        assert!(msg.contains("a down"), "摘要应含 a 的失败原因");
        assert!(msg.contains("b down"), "摘要应含 b 的失败原因");
    }

    // ---- W-4 生产接线：ModelRouter::new 默认构造故障转移链 ----

    #[test]
    fn test_router_new_builds_default_failover_chain() {
        // 默认配置（deepseek-v4-pro）应派生出同提供商备用模型（deepseek-v4-flash）
        // 作为默认故障转移候选，使 provider 故障转移在默认构造下即激活。
        let config = Config::default();
        let router = ModelRouter::new(&config);
        let chain = router.failover_chain();
        assert!(
            !chain.is_empty(),
            "默认构造应已激活故障转移链（派生同提供商备用模型）"
        );
        // 主模型 deepseek-v4-pro 的已知 sibling 是 deepseek-v4-flash（同 base_url/key，仅 model 不同）
        assert_eq!(chain[0].model, "deepseek-v4-flash");
        assert_eq!(chain[0].base_url, config.model.base_url);
    }

    #[test]
    fn test_router_new_candidate_chain_includes_fallback() {
        // 默认构造下，chat/chat_for/chat_stream 经 candidate_chain 应得到
        // [primary, sibling] 有序候选，即故障转移在生产路径默认生效。
        let config = Config::default();
        let router = ModelRouter::new(&config);
        let primary = Arc::new(router.default_client().clone());
        let chain = router.candidate_chain(primary);
        let models: Vec<&str> = chain.iter().map(|c| c.config.model.as_str()).collect();
        assert_eq!(
            models,
            vec!["deepseek-v4-pro", "deepseek-v4-flash"],
            "默认 candidate_chain 应含主模型与其派生备用模型"
        );
    }
}
