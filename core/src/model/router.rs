//! 模型路由
//!
//! 根据任务类型和复杂度选择合适的模型，支持路由规则配置和回退机制。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};

use super::error_taxonomy::ProviderErrorClass;
use super::llm::{
    default_timeout_secs, LLMClient, LLMConfig, LLMResponse, StreamReceiver, ToolDef,
};
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
    /// 网页检索 / 联网搜索
    WebSearch,
    /// 视觉理解（图片 / 截图）
    Vision,
    /// 长上下文 / 长任务（1M 上下文）
    LongContext,
    /// 通用（未知意图的兜底）
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
    /// 档位治理状态（M3）。`None` = 未启用，行为与历史完全一致（零回归）。
    tier_governance: Option<TierGovernance>,
}

/// 档位治理状态（M3）：把档位准入真正接到**实际调用链**上所需的最小状态。
///
/// 为什么做成 `Option` 而非必填：既有的 265 项测试与全部历史调用路径都不传档位。
/// 若设为必填，就必须一次性改掉所有构造点与调用点，回归面过大且难以逐项验证。
/// 设为 `None` 时 [`ModelRouter::chain_for_call`] 精确退化为原 `candidate_chain`，
/// 从而**默认零行为变更**——档位治理只在使用方显式开启后才生效。
#[derive(Clone)]
pub struct TierGovernance {
    /// 本次会话生效的目标档位（由上层据模式 / TaskKind / 预算决定）
    pub tier: crate::model::tier::ModelTier,
    /// 档案表：既是准入判定的输入，也是候选接入配置的唯一来源
    pub registry: Arc<crate::model::registry::ModelRegistry>,
}

/// M3：把档位准入结果组装为候选客户端（**不含首选**，由调用方置于链首）。
///
/// 为什么抽成自由函数而不是 Router 方法：构造完整 `ModelRouter` 需要 `Config`
/// 与 vault 主口令（见同模块凭证缝测试），对一个"候选过滤"语义而言过重。
/// 抽出来后可只依赖档案表与缓存独立单测，从而精确验证
/// 「被准入线拒绝的模型绝不进入候选链」这一关键行为——这是 M3 的核心不变量。
pub(crate) fn tier_clients(
    registry: &crate::model::registry::ModelRegistry,
    tier: crate::model::tier::ModelTier,
    cache: &Arc<PrefixCache>,
) -> (Vec<Arc<LLMClient>>, crate::model::tier::TierChain) {
    let chain = registry.tier_chain(tier);
    let mut out = Vec::new();
    for id in &chain.candidates {
        // `candidates` 已由 filter_chain 保证命中档案表；这里再取一次以拿到接入配置。
        if let Some(p) = registry.profiles.get(id) {
            out.push(Arc::new(LLMClient::new(p.to_llm_config()).with_cache(cache.clone())));
        }
    }
    (out, chain)
}

impl ModelRouter {
    /// 从配置创建模型路由器。
    ///
    /// M2 降耗修复：默认缓存改为 [`crate::cache::default_prefix_cache`]——进程内若已
    /// 安装语义层（网关路径），则复用其 Tier 0 共享实例，使 `AgentRuntime` 等经此路径
    /// 构造的组件与网关共享同一份缓存，跨轮重复请求才能命中；否则退化为独立实例。
    pub fn new(config: &Config) -> Self {
        Self::new_with_cache(config, crate::cache::default_prefix_cache())
    }

    /// 从配置 + **共享前缀缓存**创建模型路由器（v9 缓存 100% 接线）：
    /// 默认客户端、故障转移链、懒创建客户端全部复用同一 `cache` 实例——
    /// 网关/长任务跨轮共享同一缓存时，可重复请求无需重新预填充即可命中；
    /// 冷上下文经 [`PrefixCache::open`] 构造或 [`ModelRouter::load_cache_from`]
    /// 并入后，跨进程重启同样命中。
    pub fn new_with_cache(config: &Config, cache: Arc<PrefixCache>) -> Self {
        let template_config = Self::build_template_config(config);
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
            tier_governance: None,
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
        Self::new_with_credentials_and_cache(config, credentials, None)
    }

    /// 凭证缝 + 共享缓存联合构造：`cache` 为 Some 时复用共享实例，None 时新建。
    pub fn new_with_credentials_and_cache(
        config: &Config,
        credentials: Option<Arc<crate::credentials::CredentialsStore>>,
        cache: Option<Arc<PrefixCache>>,
    ) -> Self {
        let resolved_cfg;
        let effective: &Config = match &credentials {
            Some(store) => {
                let resolved = crate::credentials::resolve_llm_key(
                    store,
                    &config.model.provider,
                    &config.model.api_key,
                );
                let mut overridden = config.clone();
                overridden.model.api_key = resolved;
                resolved_cfg = overridden;
                &resolved_cfg
            }
            None => config,
        };
        match cache {
            Some(c) => Self::new_with_cache(effective, c),
            None => Self::new(effective),
        }
    }

    /// 把磁盘上的冷上下文缓存并入共享缓存（跨进程重启加载端接线）。
    pub fn load_cache_from(&self, path: &std::path::Path) {
        self.cache.absorb(path);
    }

    /// 设置回退客户端
    pub fn with_fallback(mut self, config: LLMConfig) -> Self {
        self.fallback_client = Some(Arc::new(
            LLMClient::new(config).with_cache(self.cache.clone()),
        ));
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
                "anthropic" => super::llm::LLMProvider::Anthropic,
                _ => super::llm::LLMProvider::Custom {
                    base_url: config.model.base_url.clone(),
                },
            },
            model: config.model.model_name.clone(),
            api_key: config.model.api_key.clone(),
            base_url: config.model.base_url.clone(),
            timeout_secs: if config.model.timeout_secs > 0 {
                config.model.timeout_secs
            } else {
                default_timeout_secs()
            },
            ..Default::default()
        }
    }

    /// 默认故障转移链：为已知 provider 派生「同提供商全档位备用模型」作为有序候选。
    ///
    /// 蒸馏 外部参考 provider-plane 有序候选语义，但来源诚实——仅复用用户已配置的
    /// provider / base_url / api_key，不编造任何凭据或新端点。
    ///
    /// **跨档冗余修正（W-4 后续 / PRD G1 可靠性）**：旧实现仅派生「单一 sibling」，
    /// 当路由主模型恰好等于该 sibling 时，`candidate_chain` 去重后只剩一个候选，
    /// 导致**高价值复杂任务（主模型=deepseek-v4-pro）的故障转移实际失效**——
    /// 例如线上 `config.yaml` 主模型=deepseek-chat 时 sibling=deepseek-v4-pro，
    /// 与路由选中的主模型重合，回退链塌缩成单候选。新实现把 provider 的**全部已知
    /// 档位**都纳入候选链（sibling 优先、其余档位随后）；`candidate_chain` 按
    /// (base_url, model) 去重，选中的主模型自然被折叠，始终留下一个**跨档**备用
    /// 后端，确保 fallback 真正生效（PRD G1 要求 fallback < 15%）。
    ///
    /// 未知 / 自定义模型无可靠档位映射，返回空（不伪造备用后端）。
    fn default_failover_chain(config: &Config, cache: &Arc<PrefixCache>) -> Vec<Arc<LLMClient>> {
        let primary = config.model.model_name.as_str();
        let tiers = provider_tiers(primary);
        if tiers.is_empty() {
            return Vec::new();
        }
        // 顺序：sibling 优先（最可能的备用档），随后补其余档位。
        // 包含 primary 自身无害——candidate_chain 会按 (base_url, model) 去重，
        // 并在「选中主模型 ≠ primary」时充当真实回退。
        let mut ordered: Vec<String> = Vec::new();
        if let Some(sib) = provider_sibling_model(primary) {
            ordered.push(sib);
        }
        for t in tiers {
            if !ordered.contains(&t) {
                ordered.push(t);
            }
        }
        ordered
            .into_iter()
            .map(|m| {
                let mut cfg = Self::build_template_config(config);
                cfg.model = m.to_string();
                Arc::new(LLMClient::new(cfg).with_cache(cache.clone()))
            })
            .collect()
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

    /// 按**档位**组装候选链（M3：把 M2 的档位准入真正接到路由的候选链上）。
    ///
    /// 与 [`Self::candidate_chain`] 的区别：后者只按 provider 静态档位表派生候选，
    /// 而本方法从**注册表**取该档的准入结果 —— 声明能力达不到该档准入线的档案
    /// （见 `tier.rs::filter_chain`）**不会进入候选链**。这正是"准入校验生效"的地方：
    /// 若只算不接，`accepts()` 就只是一段无人调用的装饰代码。
    ///
    /// 保持既有"主模型优先"语义：`primary` 置顶，档内准入候选作为其后备，
    /// 并沿用 `candidate_chain` 的 `(base_url, model)` 去重口径。
    ///
    /// 返回值第二项是完整的准入明细（含 `rejected`/`unverified`/`missing`），
    /// 供控制中心回显"为何没选它"（对齐设计稿 §3.1 的 `RouteDecision.why`）。
    pub fn tier_candidate_chain(
        &self,
        primary: Arc<LLMClient>,
        tier: crate::model::tier::ModelTier,
        registry: &crate::model::registry::ModelRegistry,
    ) -> (Vec<Arc<LLMClient>>, crate::model::tier::TierChain) {
        let (backends, chain) = tier_clients(registry, tier, &self.cache);
        let mut out = Vec::with_capacity(backends.len() + 1);
        out.push(primary);
        out.extend(backends);
        let mut seen = std::collections::HashSet::new();
        let deduped = out
            .into_iter()
            .filter(|c| seen.insert((c.config.base_url.clone(), c.config.model.clone())))
            .collect();
        (deduped, chain)
    }

    /// 开启档位治理（M3）：此后 `chat_complete*` 的候选链由该档的**准入结果**决定。
    ///
    /// 传入的档位即"本次会话的生效档"（由上层据模式 / TaskKind / 预算决定）；
    /// 不调用本方法则 [`ModelRouter::chain_for_call`] 精确退化为历史行为。
    pub fn with_tier_governance(
        mut self,
        tier: crate::model::tier::ModelTier,
        registry: Arc<crate::model::registry::ModelRegistry>,
    ) -> Self {
        self.tier_governance = Some(TierGovernance { tier, registry });
        self
    }

    /// 由请求画像**自动判定**生效档，并据此开启档位治理（M3 闭环入口）。
    ///
    /// 这是"生效档自动判定"的落地入口：调用方只需给出模式 / 任务类型 / 延迟 / 预算，
    /// **无需自己知道该选哪一档**。判定结果与理由链一并返回，供控制中心 / TUI 回显
    /// "为何选这一档"（设计稿 §3.1 ⑥）。
    ///
    /// 返回 `(router, decision)`：`router` 已带好治理状态，可直接用于后续 `chat_complete*`。
    pub fn with_auto_tier(
        self,
        req: &crate::model::tier::TierRequest,
        registry: Arc<crate::model::registry::ModelRegistry>,
    ) -> (Self, crate::model::tier::TierDecision) {
        let decision = crate::model::tier::resolve_tier(req);
        (self.with_tier_governance(decision.tier, registry), decision)
    }

    /// 一次调用**实际**使用的候选链：开了档位治理走档位准入，否则走历史链。
    ///
    /// 这是 M3 的接线点：所有 `chat_complete*` 都经它取链，因此"准入校验生效"是
    /// **一处改动、全局受益**，而不是散落在各方法里的条件分支（后者极易漏改，
    /// 且漏改的那条路径会静默绕过准入校验）。
    fn chain_for_call(&self, primary: Arc<LLMClient>) -> Vec<Arc<LLMClient>> {
        match &self.tier_governance {
            // 未开启 → 与历史逐字节等价（默认零行为变更）
            None => self.candidate_chain(primary),
            Some(g) => self.tier_candidate_chain(primary, g.tier, &g.registry).0,
        }
    }

    /// 添加路由规则
    pub fn add_rule(&mut self, rule: ModelRoutingRule) {
        self.routing_rules.push(rule);
    }

    /// 解析命中规则的模型名
    ///
    /// 按规则表顺序逐条扫描，命中时仅在「priority 严格更高」时替换当前最优；
    /// 同 priority 的平局保留**表首**（先出现者），使选择结果确定、可复现，
    /// 不再依赖 [`Iterator::max_by_key`] 在平局时返回「末位」的迭代序语义。
    fn resolve_model(&self, intent: &IntentType, complexity: &Complexity) -> String {
        let mut best: Option<&ModelRoutingRule> = None;
        for rule in &self.routing_rules {
            let intent_matches = match &rule.intent_pattern {
                Some(pattern) => pattern == intent,
                None => true,
            };
            let complexity_matches = match &rule.complexity {
                Some(c) => c == complexity,
                None => true,
            };
            if !(intent_matches && complexity_matches) {
                continue;
            }
            let replace = match best {
                None => true,
                Some(cur) => rule.priority > cur.priority,
            };
            if replace {
                best = Some(rule);
            }
        }
        best.map(|rule| rule.model.clone())
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

    /// 构建「思考档」主客户端（Track A 非工具 / 流式 / 带工具共用）：
    /// 选中模型 + 仅覆盖思考预算（[`LLMClient::with_thinking`] 不改 max_tokens /
    /// context_window，语义正交，见 [`crate::model::llm::LLMClient`]）。
    fn primary_with_thinking(
        &self,
        intent: &IntentType,
        complexity: Complexity,
        effort: crate::model::thinking::ThinkingEffort,
    ) -> Arc<LLMClient> {
        Arc::new(
            self.select_model(intent, complexity)
                .as_ref()
                .clone()
                .with_thinking(effort),
        )
    }

    /// 构建「输出上限」主客户端（锚定 bootstrap 通道）：`max_tokens` 为 Some 时
    /// 仅在选中主模型上覆盖输出上限（[`LLMClient::with_max_tokens`] 只改
    /// max_tokens），None 时原样返回选中客户端。
    fn primary_with_cap(
        &self,
        intent: &IntentType,
        complexity: Complexity,
        max_tokens: Option<u32>,
    ) -> Arc<LLMClient> {
        match max_tokens {
            Some(cap) => Arc::new(
                self.select_model(intent, complexity)
                    .as_ref()
                    .clone()
                    .with_max_tokens(cap),
            ),
            None => self.select_model(intent, complexity),
        }
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
        let chain = self.chain_for_call(primary);
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
        let primary = self.primary_with_cap(intent, complexity, max_tokens);
        let chain = self.chain_for_call(primary);
        try_failover_chain(&chain, |c| Box::pin(c.chat_complete_cached(messages))).await
    }

    /// 非流式聊天
    ///
    /// 默认客户端失败后，按故障转移链（`candidate_chain`）逐级切下一级。
    pub async fn chat(&self, messages: &[Message]) -> Result<LLMResponse> {
        let chain = self.chain_for_call(self.default_client.clone());
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
        client
            .chat_complete_with_tools_cached(messages, tools)
            .await
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
        client
            .chat_complete_with_tools_cached(messages, tools)
            .await
    }

    /// 流式聊天
    ///
    /// 默认客户端失败后，按故障转移链（`candidate_chain`）逐级切下一级。
    pub async fn chat_stream(&self, messages: &[Message]) -> Result<StreamReceiver> {
        let chain = self.chain_for_call(self.default_client.clone());
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
        let primary = self.primary_with_thinking(intent, complexity, effort);
        let chain = self.chain_for_call(primary);
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
        let primary = self.primary_with_thinking(intent, complexity, effort);
        let chain = self.chain_for_call(primary);
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
        let primary = self.primary_with_thinking(intent, complexity, effort);
        let chain = self.chain_for_call(primary);
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
            tier_governance: self.tier_governance.clone(),
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
    // 记录每级失败：模型名 + 已归类错误的安全文案 + 归类（用于「全候选同类失败」判定）。
    let mut attempts: Vec<(String, String, Option<ProviderErrorClass>)> =
        Vec::with_capacity(candidates.len());
    for c in candidates {
        match runner(c).await {
            Ok(response) => return Ok(response),
            Err(e) => {
                let class = crate::model::error_taxonomy::provider_error_of(&e).map(|p| p.class());
                // `e.to_string()`：对 ProviderError 只含 `code: safe_message`（**不含**上游正文），
                // 故即便走下方通用兜底分支也不会把 provider 原始正文回显给用户。
                attempt_errors_capture(&mut attempts, c.config.model.clone(), e.to_string(), class);
            }
        }
    }
    let detail = attempts
        .iter()
        .map(|(model, err, _)| format!("{model}: {err}"))
        .collect::<Vec<_>>()
        .join(" | ");

    // P0-1：若**全部候选**因同一类别失败（如全部 401/403 鉴权失败、全部 429 限流），
    // 返回该稳定错误码，而非把它降级成一坨泛化文案——前端据此给出可操作的本地化引导。
    if let Some(class) = common_failure_class(&attempts) {
        tracing::warn!(
            class = class.code(),
            candidates = attempts.len(),
            detail = %detail,
            "all failover candidates failed with the same class"
        );
        return Err(anyhow::Error::new(
            crate::model::error_taxonomy::ProviderError::new(class, detail),
        ));
    }

    Err(anyhow!(
        "all failover candidates failed ({}/{}): {detail}",
        attempts.len(),
        candidates.len()
    ))
}

/// 追加一级失败明细（把 push 收敛为函数：便于判定「全候选同类失败」）。
fn attempt_errors_capture(
    attempts: &mut Vec<(String, String, Option<ProviderErrorClass>)>,
    model: String,
    err: String,
    class: Option<ProviderErrorClass>,
) {
    attempts.push((model, err, class));
}

/// 全体候选失败的归类：仅当**至少一个**候选、且每个候选都带归类、且归类完全一致时返回。
///
/// 任一候选无归类（如非 provider 类错误）或归类不一致 → 返回 `None`（保持泛化文案）。
fn common_failure_class(
    attempts: &[(String, String, Option<ProviderErrorClass>)],
) -> Option<ProviderErrorClass> {
    let first = attempts.first()?.2?;
    attempts
        .iter()
        .all(|(_, _, c)| *c == Some(first))
        .then_some(first)
}

/// 获取默认路由规则（供 CLI 展示）
pub fn default_rules_public() -> Vec<ModelRoutingRule> {
    default_routing_rules()
}

/// 已知主模型 → 同提供商备用模型（故障转移默认候选）。
///
/// 数据驱动：从 `routing_rules.json` 的 `failover.siblings` 读取，与路由规则表同源，
/// 新增/调整档位只改数据文件、不碰路由代码（对齐「禁硬编码旧版」纪律）。
/// 仅含本仓明确支持、且与主模型不同的模型；其余返回 `None`（不编造备用后端）。
fn provider_sibling_model(primary: &str) -> Option<String> {
    routing_config().failover.siblings.get(primary).cloned()
}

/// 已知模型所属 provider 的全部档位（含跨档），用于构造「全档位故障转移链」。
///
/// 数据驱动：从 `routing_rules.json` 的 `failover.tiers` 读取。仅覆盖本仓明确支持、
/// 复用用户真实凭据的 provider；其余返回空（不伪造备用后端）。返回的档位作为
/// `default_failover_chain` 的候选池，`candidate_chain` 按 (base_url, model) 去重。
fn provider_tiers(model: &str) -> Vec<String> {
    routing_config()
        .failover
        .tiers
        .get(model)
        .cloned()
        .unwrap_or_default()
}

/// 路由规则数据文件（编译期内嵌）。新增/调整路由表与故障转移拓扑只改这里，
/// 不碰路由/调用代码（对齐「禁硬编码旧版」纪律；与 `seed_models.json` 同源思路）。
const ROUTING_RULES_JSON: &str = include_str!("routing_rules.json");

/// 路由规则 DTO（从 JSON 解析；intent/complexity 缺省为 None = 通配）。
#[derive(Debug, serde::Deserialize)]
struct RoutingRuleDto {
    #[serde(default)]
    intent: Option<IntentType>,
    #[serde(default)]
    complexity: Option<Complexity>,
    model: String,
    priority: u8,
}

/// 故障转移拓扑 DTO（sibling + 全档位 tiers）。
#[derive(Debug, serde::Deserialize, Default)]
struct FailoverDto {
    #[serde(default)]
    siblings: std::collections::HashMap<String, String>,
    #[serde(default)]
    tiers: std::collections::HashMap<String, Vec<String>>,
}

/// 路由配置根（rules + failover）。
#[derive(Debug, serde::Deserialize)]
struct RoutingConfigFile {
    rules: Vec<RoutingRuleDto>,
    #[serde(default)]
    failover: FailoverDto,
}

/// 解析并缓存路由配置（parse 一次，全进程复用）。
fn routing_config() -> &'static RoutingConfigFile {
    use std::sync::OnceLock;
    static CFG: OnceLock<RoutingConfigFile> = OnceLock::new();
    CFG.get_or_init(|| {
        serde_json::from_str(ROUTING_RULES_JSON)
            .expect("routing_rules.json 解析失败（数据驱动路由表损坏）")
    })
}

/// 默认路由规则
///
/// 档位分层（沿用既有 priority 风格，定义见 `routing_rules.json`）：
/// - **强档**（priority 100）：代码 / 推理 / 长上下文 / 视觉 / 复杂问答的中高复杂度
///   → 强推理模型 `deepseek-v4-pro`；
/// - **中轻档**（priority 80）：各意图的简单任务、以及代码 / 对话 / 检索的中等复杂度
///   → 轻量模型 `deepseek-v4-flash`（「简单任务 flash 档」）；
/// - **恒定轻档**（priority 70）：翻译 / 总结无论复杂度都用轻量模型；
/// - **兜底**（priority 50）：通用 / 未知意图 → 默认强模型 `deepseek-v4-pro`。
fn default_routing_rules() -> Vec<ModelRoutingRule> {
    routing_config()
        .rules
        .iter()
        .map(|r| ModelRoutingRule {
            intent_pattern: r.intent.clone(),
            complexity: r.complexity.clone(),
            model: r.model.clone(),
            priority: r.priority,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::llm::TokenUsage;
    use crate::model::thinking::{TaskKind, ThinkingEffort};
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
        // 测试隔离：config_dir 指向本用例临时目录 + 初始化 vault，
        // 避免写真实 `~/.marguerite/`，且满足 byok 加解密所需主口令派生密钥。
        crate::persistence::config::Config::set_config_dir_override(dir.clone());
        crate::vault::set_master_passphrase("test-vault-pass").unwrap();
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

    // ---- 路由规则审计与补全：全矩阵 + 平局确定性 + 边界回退 ----

    const PRO: &str = "deepseek-v4-pro";
    const FLASH: &str = "deepseek-v4-flash";

    fn all_intents() -> Vec<IntentType> {
        vec![
            IntentType::Code,
            IntentType::Chat,
            IntentType::Reasoning,
            IntentType::Translation,
            IntentType::Summarization,
            IntentType::WebSearch,
            IntentType::Vision,
            IntentType::LongContext,
            IntentType::General,
        ]
    }

    fn all_complexities() -> Vec<Complexity> {
        vec![Complexity::Simple, Complexity::Medium, Complexity::Complex]
    }

    /// 全矩阵期望：intent × complexity → 期望模型名（与 `default_routing_rules` 严格对齐）。
    fn expected_model(intent: &IntentType, complexity: &Complexity) -> &'static str {
        use Complexity::*;
        match (intent, complexity) {
            (IntentType::Code, Complex) => PRO,
            (IntentType::Code, Medium) | (IntentType::Code, Simple) => FLASH,
            (IntentType::Reasoning, Complex) | (IntentType::Reasoning, Medium) => PRO,
            (IntentType::Reasoning, Simple) => FLASH,
            (IntentType::Translation, _) => FLASH,
            (IntentType::Summarization, _) => FLASH,
            (IntentType::WebSearch, Complex) => PRO,
            (IntentType::WebSearch, Medium) | (IntentType::WebSearch, Simple) => FLASH,
            (IntentType::Vision, Complex) | (IntentType::Vision, Medium) => PRO,
            (IntentType::Vision, Simple) => FLASH,
            (IntentType::LongContext, Complex) | (IntentType::LongContext, Medium) => PRO,
            (IntentType::LongContext, Simple) => FLASH,
            (IntentType::Chat, Complex) => PRO,
            (IntentType::Chat, Medium) | (IntentType::Chat, Simple) => FLASH,
            // General（未知 / 兜底意图）走基础档 → 默认强模型
            (IntentType::General, _) => PRO,
        }
    }

    #[test]
    fn test_routing_matrix_full_coverage() {
        // 全档遍历：Simple/Medium/Complex × 全部 IntentType 变体必须返回合法模型
        //（不 panic、不空串），且与期望矩阵严格一致。
        let config = Config::default();
        let router = ModelRouter::new(&config);
        for intent in all_intents() {
            for complexity in all_complexities() {
                let model = router.resolve_model(&intent, &complexity);
                assert!(!model.is_empty(), "{intent:?}×{complexity:?} 不得返回空串");
                assert_eq!(
                    model,
                    expected_model(&intent, &complexity),
                    "{intent:?}×{complexity:?} 模型不符"
                );
            }
        }
    }

    #[test]
    fn test_resolve_model_tie_breaks_to_first_rule() {
        // 同 priority 多条规则命中同一请求时，选择必须确定：取规则表最前者（先追加者）。
        // 用最高优先级压过所有内建规则，验证平局语义与具体意图/复杂度无关。
        let config = Config::default();
        let mut router = ModelRouter::new(&config);
        router.add_rule(ModelRoutingRule {
            intent_pattern: None,
            complexity: None,
            model: "first-added".to_string(),
            priority: 250,
        });
        router.add_rule(ModelRoutingRule {
            intent_pattern: None,
            complexity: None,
            model: "second-added".to_string(),
            priority: 250,
        });
        assert_eq!(
            router.resolve_model(&IntentType::General, &Complexity::Simple),
            "first-added",
            "同 priority 平局应取规则表最前者（确定顺序）"
        );
        assert_eq!(
            router.resolve_model(&IntentType::Code, &Complexity::Complex),
            "first-added",
            "平局确定性应与具体意图/复杂度无关"
        );
    }

    #[test]
    fn test_resolve_model_falls_back_to_default_when_no_rule_matches() {
        let config = Config::default();
        let mut router = ModelRouter::new(&config);
        router.routing_rules.clear();
        let model = router.resolve_model(&IntentType::Code, &Complexity::Complex);
        assert_eq!(model, router.default_client.config.model);
        assert_eq!(model, "deepseek-v4-pro");
    }

    #[test]
    fn test_classify_task_empty_message_defaults_to_chat() {
        // 空消息 / 空 User 文本 → Chat 档，不得 panic、不得返回空档位。
        assert_eq!(classify_task(&[]), TaskKind::Chat);
        let empty_user = Message::User {
            content: String::new(),
            timestamp: chrono::Utc::now(),
        };
        assert_eq!(classify_task(&[empty_user]), TaskKind::Chat);
    }

    // ---- 联合调用就绪性：with_thinking / with_max_tokens 叠加语义 ----

    #[test]
    fn test_primary_with_thinking_preserves_max_tokens() {
        let config = Config::default();
        let router = ModelRouter::new(&config);
        let client = router.primary_with_thinking(
            &IntentType::Code,
            Complexity::Complex,
            ThinkingEffort::Deep,
        );
        assert_eq!(client.config.model, PRO);
        assert_eq!(client.config.thinking, ThinkingEffort::Deep);
        // with_thinking 仅覆盖思考档，不得改动 max_tokens（保持模板默认值）。
        assert_eq!(client.config.max_tokens, LLMConfig::default().max_tokens);
    }

    #[test]
    fn test_primary_with_cap_overrides_max_tokens_only() {
        let config = Config::default();
        let router = ModelRouter::new(&config);
        let client = router.primary_with_cap(&IntentType::Code, Complexity::Medium, Some(1024));
        assert_eq!(client.config.model, FLASH);
        assert_eq!(client.config.max_tokens, 1024);
        // with_max_tokens 不改 thinking（保持模板默认 Off）。
        assert_eq!(client.config.thinking, ThinkingEffort::Off);
        // None → 原样返回选中客户端，max_tokens 不被覆盖。
        let uncapped = router.primary_with_cap(&IntentType::Code, Complexity::Medium, None);
        assert_eq!(uncapped.config.max_tokens, LLMConfig::default().max_tokens);
        assert_eq!(uncapped.config.model, FLASH);
    }

    #[test]
    fn test_thinking_and_cap_compose_orthogonally() {
        let base = LLMClient::new(LLMConfig::default());
        // 思考档 + 输出上限叠加：各自字段独立生效，max_tokens 取上限值（更小者）。
        let both = base
            .clone()
            .with_thinking(ThinkingEffort::Max)
            .with_max_tokens(2048);
        assert_eq!(both.config.thinking, ThinkingEffort::Max);
        assert_eq!(both.config.max_tokens, 2048);
        // 反向叠加顺序结果一致（确定性）。
        let rev = base
            .clone()
            .with_max_tokens(2048)
            .with_thinking(ThinkingEffort::Max);
        assert_eq!(rev.config.thinking, ThinkingEffort::Max);
        assert_eq!(rev.config.max_tokens, 2048);
        // 自适应大输出档 + 输出上限 → 上限（min）生效，思考档保留。
        let adaptive = base.clone().with_adaptive_profile(TaskKind::Code);
        assert!(adaptive.config.max_tokens > 2048);
        let capped = adaptive.with_max_tokens(2048);
        assert_eq!(capped.config.max_tokens, 2048);
        assert_eq!(capped.config.thinking, ThinkingEffort::Deep);
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

    // P0-1：全部候选均因鉴权失败（401/403）→ 返回稳定错误码 AUTH_FAILED，
    // 且用户可见文案不得回显上游原始正文（如 DeepSeek 回的 `****gEni`）。
    #[tokio::test]
    async fn test_try_failover_chain_all_auth_failed_yields_stable_code() {
        let a = Arc::new(LLMClient::new(LLMConfig {
            model: "a".to_string(),
            ..Default::default()
        }));
        let b = Arc::new(LLMClient::new(LLMConfig {
            model: "b".to_string(),
            ..Default::default()
        }));
        let upstream_body = "Your api key: ****gEni is invalid";
        let out: Result<LLMResponse> = try_failover_chain(&[a, b], |_c| {
            let body = upstream_body.to_string();
            Box::pin(async move {
                Err(anyhow::Error::new(
                    crate::model::error_taxonomy::ProviderError::from_status(401, body),
                ))
            })
        })
        .await;
        assert!(out.is_err(), "全候选鉴权失败应报错");
        let err = out.unwrap_err();
        assert_eq!(
            crate::model::error_taxonomy::code_of(&err),
            Some("AUTH_FAILED"),
            "全候选鉴权失败应归类为 AUTH_FAILED"
        );
        let visible = err.to_string();
        assert!(
            !visible.contains("****gEni"),
            "用户可见文案不得回显上游正文片段: {visible}"
        );
    }

    // P0-1 反向：失败类别不一致时**不**强行归类，保持泛化文案（可诊断明细仍在）。
    #[tokio::test]
    async fn test_try_failover_chain_mixed_classes_stay_generic() {
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
            Box::pin(async move {
                let status = if m == "a" { 401 } else { 503 };
                Err(anyhow::Error::new(
                    crate::model::error_taxonomy::ProviderError::from_status(status, "x"),
                ))
            })
        })
        .await;
        let err = out.unwrap_err();
        assert_eq!(
            crate::model::error_taxonomy::code_of(&err),
            None,
            "类别不一致不应归类为单一稳定码"
        );
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

    // ---- W-4 后续：跨档故障转移链不再塌缩（PRD G1 fallback < 15%） ----

    #[test]
    fn test_default_failover_chain_includes_all_tiers() {
        // deepseek provider 应把两个档位都纳入默认故障转移链，
        // 确保无论路由选中 pro 还是 flash，都有跨档备用后端。
        let config = Config::default(); // model = deepseek-v4-pro
        let router = ModelRouter::new(&config);
        let models: Vec<&str> = router
            .failover_chain()
            .iter()
            .map(|c| c.model.as_str())
            .collect();
        assert!(
            models.contains(&"deepseek-v4-pro"),
            "链应含 pro 档（跨档回退），得 {models:?}"
        );
        assert!(
            models.contains(&"deepseek-v4-flash"),
            "链应含 flash 档（首选 sibling），得 {models:?}"
        );
    }

    #[test]
    fn test_routed_complex_task_has_cross_tier_fallback() {
        // 复杂任务路由到 deepseek-v4-pro 时，candidate_chain 必须仍有一个 flash 备用，
        // 否则 PRD G1 的 fallback<15% 无法达成（旧实现会塌缩成单候选）。
        let config = Config::default();
        let router = ModelRouter::new(&config);
        let primary = router.select_model(&IntentType::Code, Complexity::Complex);
        let chain = router.candidate_chain(primary);
        let models: Vec<&str> = chain.iter().map(|c| c.config.model.as_str()).collect();
        assert!(
            models.contains(&"deepseek-v4-flash"),
            "pro 主模型须有 flash 跨档回退，得 {models:?}"
        );
    }

    #[test]
    fn test_routed_simple_task_has_cross_tier_fallback() {
        // 对称守护：简单任务路由到 flash 时，candidate_chain 须有 pro 备用。
        let config = Config::default();
        let router = ModelRouter::new(&config);
        let primary = router.select_model(&IntentType::Code, Complexity::Simple);
        let chain = router.candidate_chain(primary);
        let models: Vec<&str> = chain.iter().map(|c| c.config.model.as_str()).collect();
        assert!(
            models.contains(&"deepseek-v4-pro"),
            "flash 主模型须有 pro 跨档回退，得 {models:?}"
        );
    }

    // ── A 延伸：路由规则数据化（禁硬编码旧版）──

    #[test]
    fn test_routing_rules_load_from_json() {
        // 路由表已外置为数据文件；条目数须与 routing_rules.json 严格对齐，
        // 防止「改了 Rust 代码却忘了同步 JSON」之类的漂移。
        let rules = default_routing_rules();
        assert_eq!(rules.len(), 21, "路由表条目数须与 routing_rules.json 对齐");
        // 兜底规则（intent/complexity 均空）存在且 priority 50
        let catch = rules
            .iter()
            .find(|r| r.intent_pattern.is_none() && r.complexity.is_none());
        assert!(catch.is_some(), "须有兜底规则（intent/complexity 均空）");
        assert_eq!(catch.unwrap().priority, 50);
        // 强档：Code×Complex → deepseek-v4-pro @ priority 100
        assert!(rules.iter().any(|r| {
            r.intent_pattern == Some(IntentType::Code)
                && r.complexity == Some(Complexity::Complex)
                && r.model == "deepseek-v4-pro"
                && r.priority == 100
        }));
        // 全矩阵仍与数据文件一致（重定向到既有全矩阵守护）
        let config = Config::default();
        let router = ModelRouter::new(&config);
        for intent in all_intents() {
            for complexity in all_complexities() {
                assert_eq!(
                    router.resolve_model(&intent, &complexity),
                    expected_model(&intent, &complexity),
                    "{intent:?}×{complexity:?} 矩阵须与 routing_rules.json 对齐"
                );
            }
        }
    }

    #[test]
    fn test_failover_topology_loads_from_json() {
        // 故障转移拓扑（sibling/档位）已数据驱动；须与 routing_rules.json 的 failover 一致。
        assert_eq!(
            provider_sibling_model("deepseek-v4-pro").as_deref(),
            Some("deepseek-v4-flash")
        );
        assert_eq!(
            provider_sibling_model("gpt-4o-mini").as_deref(),
            Some("gpt-4o")
        );
        let tiers = provider_tiers("deepseek-v4-pro");
        assert!(tiers.contains(&"deepseek-v4-pro".to_string()));
        assert!(tiers.contains(&"deepseek-v4-flash".to_string()));
        // 未知模型无档位映射（不伪造备用后端）
        assert!(provider_tiers("some-unknown-model").is_empty());
        assert_eq!(provider_sibling_model("some-unknown-model"), None);
    }

    #[test]
    fn tier_clients_never_fabricate_backends_for_missing_profiles() {
        // M3 数据漂移守卫：tier_rules.json 里列了 id、但档案表里并不存在时，
        // 必须把它记入 missing 且**不构造任何客户端** —— 若照链序硬造一个指向
        // 不存在档案的客户端，故障会推迟到真正发请求时才炸，且现场信息丢失。
        let reg = crate::model::registry::ModelRegistry::default();
        let cache = crate::cache::default_prefix_cache();
        let (clients, chain) =
            tier_clients(&reg, crate::model::tier::ModelTier::Flagship, &cache);
        assert!(clients.is_empty(), "空档案表不得产出任何候选客户端");
        assert!(chain.candidates.is_empty());
        assert!(
            !chain.missing.is_empty(),
            "链中所有 id 都应被记为 missing（数据漂移必须可见）"
        );
    }
}
