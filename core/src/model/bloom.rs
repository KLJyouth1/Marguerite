//! Bloom 编排内核（Phase G）
//!
//! 吸收 MAX 模式 双模型 + 双领队 + 蜂群，Rust 化改进（不搬迁）：
//! - 并行双模型（Captain 主 + Copilot 审校），默认 `tokio::join!` 预取降低延迟（D-04 决议）
//!
//! 已移除的伪实现 / 死代码（诚实清理 2026-08-13）：
//! - `bloom:auto` 自主闭环（`BloomOrchestrator::run_autonomous`）：与 `plan::runner::run_autonomous`
//!   真闭环重复，且其 `execute()` 仅让 LLM 生成文本、未真正执行工具/代码，属伪执行，已删除。
//! - `bloom:deep` 四阶段推理（`ReasoningRunner`）：仅有单测可达、生产从不调用，已删除。
//!
//! 安全：Bloom 不直接放宽沙箱；并行/多轮仅是推理层优化，工具调用仍过 sandbox 白名单。

use std::sync::Arc;
use std::time::Instant;

#[cfg(test)]
use anyhow::anyhow;
use anyhow::Result;
use chrono::Utc;

use crate::agent::{BloomDecisionCache, SemanticIntentCache};
use crate::events::{EnsembleProgress, Envelope, EventBus};
use crate::model::llm::{LLMClient, LLMResponse};
use crate::types::Message;

/// Captain（主模型）在并行成员条中的序号与展示名
const CAPTAIN_IDX: u32 = 0;
/// Captain 展示名
const CAPTAIN_LABEL: &str = "Captain";
/// Copilot（审校模型）序号
const COPILOT_IDX: u32 = 1;
/// Copilot 展示名
const COPILOT_LABEL: &str = "Copilot";

/// 经过时间（毫秒，饱和转换避免截断）
fn elapsed_ms(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// 由一次模型调用的结果构造「结束帧」：成功回填模型名与用量，失败回填错误摘要。
///
/// 失败也发结束帧（而非静默），前端才能把该并行成员收敛为「已失败」而不是永远转圈。
fn finish_frame(
    index: u32,
    label: &str,
    result: &Result<LLMResponse>,
    elapsed: u64,
) -> EnsembleProgress {
    match result {
        Ok(r) => EnsembleProgress::proposer_finish(index, label, &r.model, elapsed)
            .with_usage(r.usage.prompt_tokens, r.usage.completion_tokens),
        Err(e) => EnsembleProgress::proposer_error(index, label, &e.to_string(), elapsed),
    }
}

/// 聊天后端抽象（便于注入 FakeBackend 做单测）
pub trait ChatBackend {
    fn chat(
        &self,
        messages: &[Message],
    ) -> impl std::future::Future<Output = Result<LLMResponse>> + Send;
}

impl ChatBackend for LLMClient {
    async fn chat(&self, messages: &[Message]) -> Result<LLMResponse> {
        self.chat_complete(messages).await
    }
}

/// 引用后端也可作为后端（便于四阶段管线以 `&B` 持有，避免要求 `B: Clone` 移动所有权）。
impl<B: ChatBackend + Sync> ChatBackend for &B {
    async fn chat(&self, messages: &[Message]) -> Result<LLMResponse> {
        (**self).chat(messages).await
    }
}

fn build_messages(system: &str, user: &str) -> Vec<Message> {
    vec![
        Message::System {
            content: system.to_string(),
            timestamp: Utc::now(),
        },
        Message::User {
            content: user.to_string(),
            timestamp: Utc::now(),
        },
    ]
}

/// 合并主答案与审校（Copilot 非空时追加补充）
fn merge(primary: &LLMResponse, critique: &LLMResponse) -> String {
    let c = critique.content.trim();
    if c.is_empty() {
        return primary.content.clone();
    }
    format!("{}\n\n---\n审校补充（Copilot）：\n{}", primary.content, c)
}

/// Bloom 单次运行结果
pub struct BloomResult {
    /// 主模型输出
    pub primary: LLMResponse,
    /// 副模型审校（无 copilot 时为 None）
    pub critique: Option<LLMResponse>,
    /// 合并后的最终文本
    pub merged: String,
}

/// Bloom 编排器（泛型后端，零成本抽象）
pub struct BloomOrchestrator<B: ChatBackend> {
    /// 主模型（Captain）
    pub captain: B,
    /// 副模型（Copilot / Critic），可空
    pub copilot: Option<B>,
    /// 并行预取（默认 true，D-04 决议）
    pub prefetch: bool,
    /// 四阶段深度推理管线（EvoMaster Solve→Critique→Rewrite→Select）。
    /// 开启后 `run` 走 [`crate::model::bloom_deep::DeepReasoning`]，以 Captain 作
    /// solver、Copilot 作 critic 打磨答案；默认关（既有单测/调用方行为不变）。
    pub deep: bool,
    /// 并行进度事件下发通道（事件总线 + 会话键）。
    ///
    /// 未注入时静默不发（CLI / 单测默认形态）；注入后 Captain / Copilot 的
    /// 起止与聚合阶段经 `session.event.ensemble_progress` 回流对话流，
    /// 使「双模型并行」在 UI 上可见而不只是后台事实。
    events: Option<(Arc<EventBus>, String)>,
    /// 决策缓存（KV-Cache「重算变复用」在调度层）：注入后相同意图命中即跳过双模型编排。
    decision_cache: Option<Arc<BloomDecisionCache>>,
    /// 语义意图缓存（W-7a）：精确决策缓存 miss 后，高置信语义候选可复用决策。
    semantic_cache: Option<Arc<SemanticIntentCache>>,
    /// Captain 模型名（用于 ensemble_progress 事件，前端显示）
    captain_model: String,
    /// Copilot 模型名
    copilot_model: String,
}

impl<B: ChatBackend + Sync> BloomOrchestrator<B> {
    /// 单模型编排（无 copilot）
    pub fn new(captain: B) -> Self {
        Self {
            captain,
            copilot: None,
            prefetch: true,
            deep: false,
            events: None,
            decision_cache: None,
            semantic_cache: None,
            captain_model: String::new(),
            copilot_model: String::new(),
        }
    }

    /// 双模型编排（Captain + Copilot）
    pub fn with_copilot(captain: B, copilot: B) -> Self {
        Self {
            captain,
            copilot: Some(copilot),
            prefetch: true,
            deep: false,
            events: None,
            decision_cache: None,
            semantic_cache: None,
            captain_model: String::new(),
            copilot_model: String::new(),
        }
    }

    /// 设置模型名（用于 ensemble_progress 事件）
    pub fn with_model_names(mut self, captain: &str, copilot: &str) -> Self {
        self.captain_model = captain.to_string();
        self.copilot_model = copilot.to_string();
        self
    }

    /// 设置预取开关（默认开）
    pub fn set_prefetch(mut self, on: bool) -> Self {
        self.prefetch = on;
        self
    }

    /// 开启四阶段深度推理管线（Solve→Critique→Rewrite→Select）。
    ///
    /// 默认关；`dispatch_bloom` 显式开启以获得更高质量答案。
    pub fn with_deep(mut self) -> Self {
        self.deep = true;
        self
    }

    /// 注入事件总线，让并行过程经 `session.event.ensemble_progress` 回流前端。
    ///
    /// 不注入时行为与旧版完全一致（不发任何事件），向后兼容。
    pub fn with_events(mut self, bus: Arc<EventBus>, session: impl Into<String>) -> Self {
        self.events = Some((bus, session.into()));
        self
    }

    /// 注入决策缓存：相同意图命中即复用既有决策，跳过昂贵的双模型编排。
    ///
    /// 命中语义由 [`BloomDecisionCache`] 保证：意图逐字节一致且未过期才命中，
    /// 绝不冒充语义命中。未注入时行为与旧版一致（每次都真编排），向后兼容。
    pub fn with_decision_cache(mut self, cache: Arc<BloomDecisionCache>) -> Self {
        self.decision_cache = Some(cache);
        self
    }

    /// 注入语义意图缓存（W-7a）：精确决策 miss 后，高置信语义候选复用既有决策。
    ///
    /// 语义命中严格受阈值约束（默认 0.8），绝不把近似意图冒充精确命中；
    /// 未注入时行为与旧版一致（仅精确决策缓存），向后兼容。
    pub fn with_semantic_cache(mut self, cache: Arc<SemanticIntentCache>) -> Self {
        self.semantic_cache = Some(cache);
        self
    }

    /// 发布一帧并行进度（未注入总线时为空操作）
    fn emit(&self, progress: EnsembleProgress) {
        if let Some((bus, session)) = &self.events {
            bus.publish(Envelope::ensemble_progress(session, &progress));
        }
    }

    /// 单次双模型运行
    ///
    /// 预取开（默认）：captain 与 copilot 经 `tokio::join!` 并行，延迟 ≈ max。
    /// 预取关：captain 先出主答案，copilot 基于主答案再审校，延迟 ≈ sum。
    pub async fn run(&self, prompt: &str) -> Result<BloomResult> {
        // 决策缓存短路：相同意图命中即复用既有决策，跳过双模型编排（KV-Cache「重算变复用」）。
        if let Some(cache) = &self.decision_cache {
            if let Some(decision) = cache.get(prompt) {
                return Ok(BloomResult {
                    primary: LLMResponse {
                        content: decision.clone(),
                        thinking: None,
                        model: "decision-cache".into(),
                        usage: crate::model::llm::TokenUsage::default(),
                        tool_calls: Vec::new(),
                    },
                    critique: None,
                    merged: decision,
                });
            }
        }
        // 语义候选回退（W-7a）：精确 miss 后，高置信相似意图复用既有决策。
        // 阈值由 SemanticIntentCache 裁决（默认 0.8）；命中标记为 semantic-cache，不冒充精确命中。
        if let Some(cache) = &self.semantic_cache {
            if let Some(hit) = cache.get_above_threshold(prompt) {
                return Ok(BloomResult {
                    primary: LLMResponse {
                        content: hit.decision.clone(),
                        thinking: None,
                        model: "semantic-cache".into(),
                        usage: crate::model::llm::TokenUsage::default(),
                        tool_calls: Vec::new(),
                    },
                    critique: None,
                    merged: hit.decision,
                });
            }
        }
        let cap_msgs = build_messages(
            "你是主模型 Captain，直接、准确地回答用户问题，无需解释思考过程。",
            prompt,
        );
        let result = self.run_uncached(&cap_msgs, prompt).await;
        // 真实编排完成后回填精确决策缓存与语义候选缓存（命中省 token 由真实值上报）。
        if let (Ok(r), Some(cache)) = (&result, &self.decision_cache) {
            cache.put(
                prompt,
                r.merged.clone(),
                r.primary.usage.prompt_tokens as u64,
            );
        }
        if let (Ok(r), Some(cache)) = (&result, &self.semantic_cache) {
            cache.put(prompt, r.merged.clone());
        }
        result
    }

    /// 双模型编排的真实执行体（不含决策缓存短路）。
    async fn run_uncached(&self, cap_msgs: &[Message], prompt: &str) -> Result<BloomResult> {
        match &self.copilot {
            None => {
                let t0 = Instant::now();
                self.emit(EnsembleProgress::proposer_start_with_model(
                    CAPTAIN_IDX,
                    CAPTAIN_LABEL,
                    &self.captain_model,
                    "deepseek",
                ));
                let res = self.captain.chat(cap_msgs).await;
                self.emit(finish_frame(
                    CAPTAIN_IDX,
                    CAPTAIN_LABEL,
                    &res,
                    elapsed_ms(t0),
                ));
                let primary = res?;
                let merged = primary.content.clone();
                Ok(BloomResult {
                    primary,
                    critique: None,
                    merged,
                })
            }
            Some(cop) => {
                // 四阶段深度推理管线：Captain 求解/重写，Copilot 批判/择优。
                // 以引用持有后端（`&B: ChatBackend`），无需移动所有权。
                if self.deep {
                    let t0 = Instant::now();
                    self.emit(EnsembleProgress::proposer_start_with_model(
                        CAPTAIN_IDX,
                        CAPTAIN_LABEL,
                        &self.captain_model,
                        "deepseek",
                    ));
                    self.emit(EnsembleProgress::proposer_start_with_model(
                        COPILOT_IDX,
                        COPILOT_LABEL,
                        &self.copilot_model,
                        "deepseek",
                    ));
                    let dr = crate::model::bloom_deep::DeepReasoning::dual(&self.captain, cop);
                    match dr.run(prompt).await {
                        Ok(deep) => {
                            let spent = elapsed_ms(t0);
                            let merged = deep.final_answer;
                            let primary = LLMResponse {
                                content: merged.clone(),
                                thinking: None,
                                model: "deep-reason".into(),
                                usage: crate::model::llm::TokenUsage::default(),
                                tool_calls: Vec::new(),
                            };
                            let critique = Some(LLMResponse {
                                content: deep.critique,
                                thinking: None,
                                model: "deep-critic".into(),
                                usage: crate::model::llm::TokenUsage::default(),
                                tool_calls: Vec::new(),
                            });
                            self.emit(finish_frame(
                                CAPTAIN_IDX,
                                CAPTAIN_LABEL,
                                &Ok(primary.clone()),
                                spent,
                            ));
                            self.emit(finish_frame(
                                COPILOT_IDX,
                                COPILOT_LABEL,
                                // 失败分支不可达，仅用于借出 result 提取模型/用量
                                &Ok(critique.as_ref().unwrap().clone()),
                                spent,
                            ));
                            self.emit(EnsembleProgress::aggregator("start", None));
                            self.emit(EnsembleProgress::aggregator("finish", Some(spent)));
                            Ok(BloomResult {
                                primary,
                                critique,
                                merged,
                            })
                        }
                        Err(e) => {
                            let spent = elapsed_ms(t0);
                            let err: Result<LLMResponse> = Err(e);
                            self.emit(finish_frame(CAPTAIN_IDX, CAPTAIN_LABEL, &err, spent));
                            self.emit(finish_frame(COPILOT_IDX, COPILOT_LABEL, &err, spent));
                            err.map(|_| {
                                unreachable!(
                                    "deep pipeline 失败时 finish_frame 已上报，此处不应产出结果"
                                )
                            })
                        }
                    }
                } else if self.prefetch {
                    let cop_msgs = build_messages(
                        "你是审校模型 Copilot，针对同一问题给出独立改进建议与潜在疏漏（简明）。",
                        prompt,
                    );
                    // 两帧 start 都在 join 之前发出：事件顺序本身即「并行」证据
                    // （串行分支下 Captain 的 finish 必然早于 Copilot 的 start）。
                    self.emit(EnsembleProgress::proposer_start_with_model(
                        CAPTAIN_IDX,
                        CAPTAIN_LABEL,
                        &self.captain_model,
                        "deepseek",
                    ));
                    self.emit(EnsembleProgress::proposer_start_with_model(
                        COPILOT_IDX,
                        COPILOT_LABEL,
                        &self.copilot_model,
                        "deepseek",
                    ));
                    let t0 = Instant::now();
                    let (p, c) = tokio::join!(self.captain.chat(cap_msgs), cop.chat(&cop_msgs));
                    let spent = elapsed_ms(t0);
                    self.emit(finish_frame(CAPTAIN_IDX, CAPTAIN_LABEL, &p, spent));
                    self.emit(finish_frame(COPILOT_IDX, COPILOT_LABEL, &c, spent));
                    let primary = p?;
                    let critique = c?;
                    self.emit(EnsembleProgress::aggregator("start", None));
                    let merged = merge(&primary, &critique);
                    self.emit(EnsembleProgress::aggregator("finish", Some(elapsed_ms(t0))));
                    Ok(BloomResult {
                        primary,
                        critique: Some(critique),
                        merged,
                    })
                } else {
                    let t0 = Instant::now();
                    self.emit(EnsembleProgress::proposer_start_with_model(
                        CAPTAIN_IDX,
                        CAPTAIN_LABEL,
                        &self.captain_model,
                        "deepseek",
                    ));
                    let cap_res = self.captain.chat(cap_msgs).await;
                    self.emit(finish_frame(
                        CAPTAIN_IDX,
                        CAPTAIN_LABEL,
                        &cap_res,
                        elapsed_ms(t0),
                    ));
                    let primary = cap_res?;
                    let cop_msgs = build_messages(
                        "你是审校模型 Copilot。",
                        &format!(
                            "问题：{}\n主答案：{}\n请指出疏漏并给出改进。",
                            prompt, primary.content
                        ),
                    );
                    let t1 = Instant::now();
                    self.emit(EnsembleProgress::proposer_start_with_model(
                        COPILOT_IDX,
                        COPILOT_LABEL,
                        &self.copilot_model,
                        "deepseek",
                    ));
                    let cop_res = cop.chat(&cop_msgs).await;
                    self.emit(finish_frame(
                        COPILOT_IDX,
                        COPILOT_LABEL,
                        &cop_res,
                        elapsed_ms(t1),
                    ));
                    let critique = cop_res?;
                    self.emit(EnsembleProgress::aggregator("start", None));
                    let merged = merge(&primary, &critique);
                    self.emit(EnsembleProgress::aggregator("finish", Some(elapsed_ms(t0))));
                    Ok(BloomResult {
                        primary,
                        critique: Some(critique),
                        merged,
                    })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用假后端：固定回复
    struct FakeBackend {
        reply: String,
        /// 非空时 chat 返回该错误（用于测试副模型失败传播）
        fail: Option<String>,
    }

    impl ChatBackend for FakeBackend {
        async fn chat(&self, _messages: &[Message]) -> Result<LLMResponse> {
            if let Some(e) = &self.fail {
                return Err(anyhow!("{e}"));
            }
            Ok(LLMResponse {
                content: self.reply.clone(),
                thinking: None,
                model: "fake".into(),
                usage: crate::model::llm::TokenUsage::default(),
                tool_calls: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn test_run_no_copilot() {
        let orch = BloomOrchestrator::new(FakeBackend {
            reply: "主答案".into(),
            fail: None,
        });
        let r = orch.run("问题").await.unwrap();
        assert_eq!(r.primary.content, "主答案");
        assert!(r.critique.is_none());
        assert_eq!(r.merged, "主答案");
    }

    #[tokio::test]
    async fn test_run_prefetch_on_merges_critique() {
        let orch = BloomOrchestrator::with_copilot(
            FakeBackend {
                reply: "主答案".into(),
                fail: None,
            },
            FakeBackend {
                reply: "补充建议".into(),
                fail: None,
            },
        );
        assert!(orch.prefetch);
        let r = orch.run("问题").await.unwrap();
        assert!(r.critique.is_some());
        assert!(r.merged.contains("主答案"));
        assert!(r.merged.contains("补充建议"));
    }

    #[tokio::test]
    async fn test_run_prefetch_off_merges_critique() {
        let orch = BloomOrchestrator::with_copilot(
            FakeBackend {
                reply: "主答案".into(),
                fail: None,
            },
            FakeBackend {
                reply: "补充建议".into(),
                fail: None,
            },
        )
        .set_prefetch(false);
        assert!(!orch.prefetch);
        let r = orch.run("问题").await.unwrap();
        assert!(r.merged.contains("主答案"));
        assert!(r.merged.contains("补充建议"));
    }

    // ===== 并行可见性：ensemble_progress 事件下发（对话流呈现并行证据） =====

    /// 在栅栏上会合的后端：只有两侧**真正并发**时才能同时到达栅栏并放行。
    /// 若实现退化为串行，第二侧永远等不到对方 → 测试超时失败。
    struct BarrierBackend {
        barrier: Arc<tokio::sync::Barrier>,
        reply: String,
    }

    impl ChatBackend for BarrierBackend {
        async fn chat(&self, _messages: &[Message]) -> Result<LLMResponse> {
            self.barrier.wait().await;
            Ok(LLMResponse {
                content: self.reply.clone(),
                thinking: None,
                model: "barrier-model".into(),
                usage: crate::model::llm::TokenUsage {
                    prompt_tokens: 7,
                    completion_tokens: 9,
                    total_tokens: 16,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                },
                tool_calls: Vec::new(),
            })
        }
    }

    /// 收取总线上的全部事件（按发布顺序）
    fn drain(rx: &mut tokio::sync::mpsc::Receiver<Envelope>) -> Vec<Envelope> {
        let mut out = Vec::new();
        while let Ok(env) = rx.try_recv() {
            out.push(env);
        }
        out
    }

    /// 取出 ensemble_progress 事件的 (event_type, proposer_label) 序列
    fn progress_seq(events: &[Envelope]) -> Vec<(String, String)> {
        events
            .iter()
            .filter(|e| e.r#type == crate::events::EventType::EnsembleProgress)
            .map(|e| {
                (
                    e.payload["event_type"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    e.payload["proposer_label"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn test_parallel_run_emits_ensemble_progress_events() {
        let bus = Arc::new(EventBus::default());
        let mut rx = bus.subscribe("**");
        let orch = BloomOrchestrator::with_copilot(
            FakeBackend {
                reply: "主答案".into(),
                fail: None,
            },
            FakeBackend {
                reply: "审校意见".into(),
                fail: None,
            },
        )
        .with_events(bus.clone(), "ws.s1");

        let r = orch.run("问题").await.unwrap();
        assert!(r.critique.is_some(), "双模型应产出审校");

        let events = drain(&mut rx);
        let seq = progress_seq(&events);
        assert_eq!(
            seq,
            vec![
                ("proposer_start".to_string(), "Captain".to_string()),
                ("proposer_start".to_string(), "Copilot".to_string()),
                ("proposer_finish".to_string(), "Captain".to_string()),
                ("proposer_finish".to_string(), "Copilot".to_string()),
                ("aggregator_start".to_string(), String::new()),
                ("aggregator_finish".to_string(), String::new()),
            ],
            "并行路径下两个 start 必须都早于任一 finish —— 事件顺序即并行证据"
        );

        // 事件必须落在前端契约的路由键上，否则 UI 收不到
        let first = events
            .iter()
            .find(|e| e.r#type == crate::events::EventType::EnsembleProgress)
            .expect("应发出 ensemble_progress 事件");
        assert_eq!(first.route_key(), "ws.s1.ensemble_progress");
    }

    #[tokio::test]
    async fn test_parallel_run_truly_overlaps() {
        // 栅栏容量 2：唯有 captain 与 copilot 同时在途才能通过。
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let orch = BloomOrchestrator::with_copilot(
            BarrierBackend {
                barrier: barrier.clone(),
                reply: "主答案".into(),
            },
            BarrierBackend {
                barrier,
                reply: "审校".into(),
            },
        );
        let out = tokio::time::timeout(std::time::Duration::from_secs(3), orch.run("问题")).await;
        let r = out
            .expect("双模型未真正并行：栅栏未会合导致超时")
            .expect("run 不应失败");
        assert!(r.merged.contains("主答案") && r.merged.contains("审校"));
    }

    #[tokio::test]
    async fn test_serial_mode_event_order_differs_from_parallel() {
        // prefetch=false 时事件顺序如实反映串行：Captain 结束帧早于 Copilot 起始帧。
        let bus = Arc::new(EventBus::default());
        let mut rx = bus.subscribe("**");
        let orch = BloomOrchestrator::with_copilot(
            FakeBackend {
                reply: "主答案".into(),
                fail: None,
            },
            FakeBackend {
                reply: "审校意见".into(),
                fail: None,
            },
        )
        .set_prefetch(false)
        .with_events(bus.clone(), "ws.s2");

        orch.run("问题").await.unwrap();

        let seq = progress_seq(&drain(&mut rx));
        assert_eq!(
            seq[0],
            ("proposer_start".to_string(), "Captain".to_string())
        );
        assert_eq!(
            seq[1],
            ("proposer_finish".to_string(), "Captain".to_string()),
            "串行路径下 Captain 必须先结束，Copilot 才开始"
        );
        assert_eq!(
            seq[2],
            ("proposer_start".to_string(), "Copilot".to_string())
        );
    }

    #[tokio::test]
    async fn test_finish_frame_carries_model_and_usage() {
        let bus = Arc::new(EventBus::default());
        let mut rx = bus.subscribe("**");
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let orch = BloomOrchestrator::with_copilot(
            BarrierBackend {
                barrier: barrier.clone(),
                reply: "a".into(),
            },
            BarrierBackend {
                barrier,
                reply: "b".into(),
            },
        )
        .with_events(bus.clone(), "ws.s3");
        orch.run("问题").await.unwrap();

        let events = drain(&mut rx);
        let finish = events
            .iter()
            .find(|e| e.payload["event_type"] == "proposer_finish")
            .expect("应有结束帧");
        assert_eq!(finish.payload["proposer_model"], "barrier-model");
        assert_eq!(finish.payload["input_tokens"], 7);
        assert_eq!(finish.payload["output_tokens"], 9);
        assert!(finish.payload["elapsed_ms"].is_u64(), "结束帧应带耗时");
    }

    #[tokio::test]
    async fn test_failed_proposer_still_emits_finish_frame_with_error() {
        let bus = Arc::new(EventBus::default());
        let mut rx = bus.subscribe("**");
        let orch = BloomOrchestrator::with_copilot(
            FakeBackend {
                reply: "主答案".into(),
                fail: None,
            },
            FakeBackend {
                reply: "审校".into(),
                fail: Some("connection refused".into()),
            },
        )
        .with_events(bus.clone(), "ws.s4");

        let err = orch.run("问题").await;
        assert!(err.is_err(), "副模型失败应向上传播");

        let events = drain(&mut rx);
        let failed = events
            .iter()
            .find(|e| e.payload.get("error").is_some())
            .expect("失败侧必须发结束帧，否则前端该成员永远转圈");
        assert_eq!(failed.payload["event_type"], "proposer_finish");
        assert_eq!(failed.payload["proposer_label"], "Copilot");
        assert!(failed.payload["error"]
            .as_str()
            .unwrap_or_default()
            .contains("connection refused"));
    }

    #[tokio::test]
    async fn test_without_event_bus_stays_silent() {
        // 向后兼容：未注入总线时不得发任何事件（CLI / 既有调用方行为不变）。
        let bus = Arc::new(EventBus::default());
        let mut rx = bus.subscribe("**");
        let orch = BloomOrchestrator::with_copilot(
            FakeBackend {
                reply: "主答案".into(),
                fail: None,
            },
            FakeBackend {
                reply: "审校".into(),
                fail: None,
            },
        );
        orch.run("问题").await.unwrap();
        assert!(drain(&mut rx).is_empty(), "未注入总线不应产生事件");
    }

    #[tokio::test]
    async fn test_deep_enabled_runs_four_stage_pipeline() {
        // 开启四阶段管线后：Solve(Captain)→Critique(Copilot)→Rewrite→Select，
        // 以 Copilot 的 Select 输出为 final_answer，且 copilot 参与批判。
        let orch = BloomOrchestrator::with_copilot(
            FakeBackend {
                reply: "初步答案".into(),
                fail: None,
            },
            FakeBackend {
                reply: "发现边界缺陷".into(),
                fail: None,
            },
        )
        .with_deep();
        assert!(orch.deep);
        let r = orch.run("写一个函数").await.unwrap();
        // FakeBackend 恒返各自 reply：Select 由 critic(copilot) 产出 → final_answer 为批判句。
        assert_eq!(
            r.primary.model, "deep-reason",
            "deep 路径 primary 应标记来源"
        );
        assert!(r.critique.is_some(), "deep 路径应产出批判");
        assert_eq!(r.critique.as_ref().unwrap().model, "deep-critic");
        assert_eq!(
            r.merged, "发现边界缺陷",
            "final_answer 由 Critic Select 阶段产出"
        );
    }

    #[tokio::test]
    async fn test_deep_off_by_default_keeps_parallel_path() {
        // 默认 deep=false：不启用四阶段，走既有 tokio::join! 并行路径，
        // merged 包含主答案与审校，primary.model 为真实后端。
        let orch = BloomOrchestrator::with_copilot(
            FakeBackend {
                reply: "主答案".into(),
                fail: None,
            },
            FakeBackend {
                reply: "补充".into(),
                fail: None,
            },
        );
        assert!(!orch.deep, "默认不应开启四阶段，保持向后兼容");
        let r = orch.run("问题").await.unwrap();
        assert_eq!(r.primary.model, "fake");
        assert!(r.merged.contains("主答案") && r.merged.contains("补充"));
    }

    // ===== 决策缓存接线：相同意图命中短路，跳过双模型编排 =====

    /// 计数后端：记录调用次数，用于证明命中短路确实跳过了模型调用。
    struct CountingBackend {
        reply: String,
        calls: Arc<std::sync::atomic::AtomicU64>,
    }

    impl ChatBackend for CountingBackend {
        async fn chat(&self, _messages: &[Message]) -> Result<LLMResponse> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(LLMResponse {
                content: self.reply.clone(),
                thinking: None,
                model: "counting".into(),
                usage: crate::model::llm::TokenUsage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                },
                tool_calls: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn test_decision_cache_hit_short_circuits_model_call() {
        let calls = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let cache = Arc::new(BloomDecisionCache::with_default_policy());
        let orch = BloomOrchestrator::with_copilot(
            CountingBackend {
                reply: "主答案".into(),
                calls: calls.clone(),
            },
            CountingBackend {
                reply: "审校".into(),
                calls: calls.clone(),
            },
        )
        .with_decision_cache(cache.clone());

        // 首次：真实编排，双模型各调用一次。
        let r1 = orch.run("同一意图").await.unwrap();
        assert!(r1.merged.contains("主答案"));
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::Relaxed),
            2,
            "首次应真编排"
        );

        // 第二次：命中缓存，跳过双模型调用。
        let r2 = orch.run("同一意图").await.unwrap();
        assert_eq!(r2.primary.model, "decision-cache", "命中应标记为缓存来源");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::Relaxed),
            2,
            "命中应短路模型调用"
        );

        // 统计如实：1 命中 1 未命中。
        let s = cache.stats();
        assert_eq!(s.hits, 1);
        assert_eq!(s.misses, 1);
    }

    #[tokio::test]
    async fn test_semantic_cache_reuses_near_duplicate_intent() {
        let calls = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let semantic = Arc::new(SemanticIntentCache::new(16, 0.5));
        let orch = BloomOrchestrator::with_copilot(
            CountingBackend {
                reply: "语义答案".into(),
                calls: calls.clone(),
            },
            CountingBackend {
                reply: "审校".into(),
                calls: calls.clone(),
            },
        )
        .with_semantic_cache(semantic.clone());

        // 首次：真实编排，并回填语义候选缓存。
        let r1 = orch.run("rust async runtime tokio").await.unwrap();
        assert!(r1.merged.contains("语义答案"));
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::Relaxed),
            2,
            "首次应真编排"
        );

        // 近同意图（精确不同、语义相似）：精确决策缓存 miss，语义候选命中。
        let r2 = orch.run("tokio async runtime rust").await.unwrap();
        assert_eq!(r2.primary.model, "semantic-cache", "语义命中应标记来源");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::Relaxed),
            2,
            "语义命中应短路模型调用"
        );
        assert_eq!(semantic.semantic_hits(), 1, "语义命中应单独计量");
    }
}
