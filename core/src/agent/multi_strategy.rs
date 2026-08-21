//! 多策略工具并行（TUMIX 式）：同一任务多策略并行求解 + 结果聚合。
//!
//! 借鉴 TUMIX 的多 agent 并行 + 多策略（text/code/search）迭代共享细化的思想，但保持极简：
//! - 多个策略凭据（各有自己的提示词模板 + 后端）**并行**执行（`tokio::join!`）；
//! - 结果**去重聚合**（stable 内容去重，去重率可观测）；
//! - 可选「择优」：用一个 selector 后端在候选结果间挑最佳（提升质量问题）。
//!
//! 服务基准：DSBench / Toolathlon / AutomationBench 等需"多解择优 + 工具结果聚合"的场景。
//! 零成本抽象（泛型后端），可注入 FakeBackend 单测。

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;

use crate::agent::runtime::parse_tool_args;
use crate::agent::tool_cycle::{ToolCall as CycleToolCall, ToolCycle, ToolResult};
use crate::mcp::registry::ToolRegistry;
use crate::model::ensemble::ChatBackend;
use crate::model::llm::{LLMResponse, TokenUsage, ToolDef};
use crate::model::router::ModelRouter;
use crate::plugins::manifest::PluginManifest;
use crate::types::Message;

/// 单一求解策略：名字 + 系统提示 + 后端。
pub struct Strategy<B: ChatBackend> {
    /// 策略名（用于去重/择优提示）
    pub name: String,
    /// 系统提示（决定该策略的"看问题角度"）
    pub system: String,
    /// 执行该策略的后端（可与主后端同一实例）
    pub backend: B,
    /// fleet 模型路由标签：该策略目标模型（如 "opus"/"sonnet"），由 `FleetRouter` 解析注入；
    /// `None` 表示沿用默认路由。仅作路由元数据，调用方据其对后端选路。
    pub model: Option<String>,
}

/// 多策略并行的运行统计（线程安全原子，可观测）
#[derive(Debug, Default)]
pub struct MultiStrategyStats {
    /// 运行过的策略数
    pub strategies_ran: AtomicU64,
    /// 去重后保留的唯一结果数
    pub unique_results: AtomicU64,
    /// 被去重丢弃的重复结果数
    pub deduped: AtomicU64,
}

/// 一次多策略并行运行的结果
#[derive(Debug)]
pub struct MultiResult {
    /// 去重后的聚合结果（按策略顺序）
    pub aggregated: Vec<String>,
    /// 择优后的最终答案（注入 selector 时）
    pub final_answer: Option<String>,
    /// 统计
    pub stats: MultiStrategyStats,
}

/// 简单去重：首行（策略投出）作为稳定指纹，内容完全一致视为重复。
fn dedupe(results: Vec<String>) -> (Vec<String>, u64) {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut dup = 0u64;
    for r in results {
        if seen.insert(r.clone()) {
            out.push(r);
        } else {
            dup += 1;
        }
    }
    (out, dup)
}

/// 运行多策略并行并聚合成最终结果。
///
/// `prompt` 为任务；`strategies` 为各策略凭据；`selector` 为 `Some` 时在聚合结果间
/// 择优（None 时直接拼接聚合结果，省一次调用）。
pub async fn run_multi_strategy<B: ChatBackend>(
    prompt: &str,
    strategies: Vec<Strategy<B>>,
    selector: Option<B>,
) -> Result<MultiResult> {
    let n = strategies.len();
    // 先在函数作用域收集各策略的消息，保证其生命周期覆盖后续持有的 futures。
    let mut all_msgs = Vec::with_capacity(n);
    for s in &strategies {
        all_msgs.push(vec![
            Message::System {
                content: s.system.clone(),
                timestamp: Utc::now(),
            },
            Message::User {
                content: prompt.to_string(),
                timestamp: Utc::now(),
            },
        ]);
    }

    let mut futures = Vec::with_capacity(n);
    for (i, msgs) in all_msgs.iter().enumerate() {
        futures.push(Box::pin(strategies[i].backend.chat(msgs)));
    }

    // 真正并行：join_all 同时驱动全部策略（修正「标称并行、实则顺序 await」的旧实现）。
    let raws = futures_util::future::join_all(futures).await;

    let mut contents = Vec::with_capacity(n);
    for r in raws {
        contents.push(r?.content);
    }
    let (aggregated, dup) = dedupe(contents);

    let stats = MultiStrategyStats {
        strategies_ran: AtomicU64::new(n as u64),
        unique_results: AtomicU64::new(aggregated.len() as u64),
        deduped: AtomicU64::new(dup),
    };

    let final_answer = match selector {
        Some(sel) => {
            if aggregated.len() <= 1 {
                Some(aggregated.first().cloned().unwrap_or_default())
            } else {
                let body = aggregated
                    .iter()
                    .enumerate()
                    .map(|(i, r)| format!("候选 {}：\n{}", i + 1, r))
                    .collect::<Vec<_>>()
                    .join("\n\n---\n\n");
                let sel = sel
                    .chat(&[
                        Message::System {
                            content: "你是择优者。从下列候选结果中选出质量最高、最符合任务要求的一个。只输出该候选的内容，不要解释。".into(),
                            timestamp: Utc::now(),
                        },
                        Message::User {
                            content: format!("任务：{prompt}\n\n候选结果：\n{body}"),
                            timestamp: Utc::now(),
                        },
                    ])
                    .await?;
                Some(sel.content)
            }
        }
        None => {
            if aggregated.is_empty() {
                None
            } else {
                Some(aggregated.join("\n\n---\n"))
            }
        }
    };

    Ok(MultiResult {
        aggregated,
        final_answer,
        stats,
    })
}

/// 便捷构造单个策略。
pub fn strategy<B: ChatBackend>(name: &str, system: &str, backend: B) -> Strategy<B> {
    Strategy {
        name: name.to_string(),
        system: system.to_string(),
        backend,
        model: None,
    }
}

/// 构造带 fleet 模型路由标签的策略（target model 由 `FleetRouter` 解析后注入）。
pub fn strategy_with_model<B: ChatBackend>(
    name: &str,
    system: &str,
    backend: B,
    model: &str,
) -> Strategy<B> {
    Strategy {
        name: name.to_string(),
        system: system.to_string(),
        backend,
        model: Some(model.to_string()),
    }
}

/// 工具回路后端：把「带工具的多轮执行」包装为 [`ChatBackend`]，使 [`run_multi_strategy`]
/// 的每个策略都能真实调用工具（read_file / code_exec / browser / system_control / vision 等），
/// 实现 TUMIX 式多策略工具并行（text/code/search 多角度并行 + 择优）。
///
/// 一次 `chat` 即一轮完整的 plan→act→observe 工具回路（最多 8 轮），返回最终收敛文本。
/// 传入的消息里的 `System` 内容作为该轮的系统提示（策略角度或择优提示），`User` 内容作为任务。
pub struct ToolLoopBackend {
    router: ModelRouter,
    registry: Arc<ToolRegistry>,
    tool_defs: Vec<ToolDef>,
}

impl ToolLoopBackend {
    /// 构造工具回路后端。`tool_defs` 为空时退化为纯文本单轮（可作择优器）。
    pub fn new(router: ModelRouter, registry: Arc<ToolRegistry>, tool_defs: Vec<ToolDef>) -> Self {
        Self {
            router,
            registry,
            tool_defs,
        }
    }
}

impl ChatBackend for ToolLoopBackend {
    async fn chat(&self, messages: &[Message]) -> Result<LLMResponse> {
        // run_multi_strategy 会构造 [System(策略/择优提示), User(任务)]，
        // 这里原样取回，使每个策略/择优器按传入角度执行。
        let system = messages
            .iter()
            .find_map(|m| match m {
                Message::System { content, .. } => Some(content.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let prompt = messages
            .iter()
            .rev()
            .find_map(|m| match m {
                Message::User { content, .. } => Some(content.clone()),
                _ => None,
            })
            .unwrap_or_default();

        let final_text = run_tool_loop_for_strategy(
            &self.router,
            &self.registry,
            &self.tool_defs,
            &system,
            &prompt,
        )
        .await?;

        Ok(LLMResponse {
            content: final_text,
            thinking: None,
            model: "tool-loop".into(),
            usage: TokenUsage::default(),
            tool_calls: Vec::new(),
        })
    }
}

/// 便捷构造一个「工具回路策略」（供 [`run_multi_strategy`] 做多策略工具并行）。
pub fn tool_strategy(
    name: &str,
    system: &str,
    router: ModelRouter,
    registry: Arc<ToolRegistry>,
    tool_defs: Vec<ToolDef>,
) -> Strategy<ToolLoopBackend> {
    Strategy {
        name: name.to_string(),
        system: system.to_string(),
        backend: ToolLoopBackend::new(router, registry, tool_defs),
        model: None,
    }
}

/// 单个策略的带工具多轮回路（plan → act → observe），返回最终收敛文本。
async fn run_tool_loop_for_strategy(
    router: &ModelRouter,
    registry: &Arc<ToolRegistry>,
    tool_defs: &[ToolDef],
    system: &str,
    prompt: &str,
) -> Result<String> {
    let mut messages = vec![
        Message::System {
            content: system.to_string(),
            timestamp: Utc::now(),
        },
        Message::User {
            content: prompt.to_string(),
            timestamp: Utc::now(),
        },
    ];
    let cycle = ToolCycle::new(registry.clone());
    for _ in 0..8 {
        let resp = router.chat_with_tools(&messages, tool_defs).await?;
        if resp.tool_calls.is_empty() {
            return Ok(resp.content);
        }
        let calls: Vec<CycleToolCall> = resp
            .tool_calls
            .iter()
            .map(|tc| CycleToolCall::new(&tc.name, parse_tool_args(&tc.arguments)))
            .collect();
        let results = cycle.execute_sequential(calls).await?;
        for (tc, res) in resp.tool_calls.iter().zip(results.into_iter()) {
            let output = match &res {
                ToolResult::Success { data, .. } => data.clone(),
                ToolResult::Error { message, code } => format!("[工具错误 {code}] {message}"),
                ToolResult::Timeout { tool_name, .. } => format!("[工具超时] {tool_name}"),
            };
            messages.push(Message::Tool {
                name: tc.name.clone(),
                input: tc.arguments.clone(),
                output,
                timestamp: Utc::now(),
            });
        }
    }
    Err(anyhow::anyhow!("多策略工具回路超过最大轮数，未能收敛"))
}

/// fleet 路由条目：能力名 → 目标模型。
#[derive(Debug, Clone)]
pub struct FleetRoute {
    /// 能力名（与 `PluginManifest::capability_models` 的键一致）
    pub capability: String,
    /// 目标模型偏好（如 "opus"/"sonnet"/"inherit"）
    pub model: String,
}

/// 能力级 → 模型 的 fleet 路由表。
///
/// 数据源：`PluginManifest::capability_models`（声明式）。`multi_strategy` 在构建
/// 每能力/角色的策略时调用 [`FleetRouter::resolve`] 取得目标模型，实现「每角色/
/// 每能力指定模型」的 fleet 路由（对齐 CodeWhale `fleets`）。
#[derive(Debug, Clone, Default)]
pub struct FleetRouter {
    routes: HashMap<String, String>,
}

impl FleetRouter {
    /// 空路由表。
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
        }
    }

    /// 从 `PluginManifest` 的能力级模型映射构建路由表。
    pub fn from_manifest(m: &PluginManifest) -> Self {
        Self {
            routes: m.capability_models.clone(),
        }
    }

    /// 注册一条能力 → 模型路由。
    pub fn add(&mut self, capability: &str, model: &str) {
        self.routes
            .insert(capability.to_string(), model.to_string());
    }

    /// 解析某能力应使用哪个模型；无登记时返回 `None`（调用方回退默认模型）。
    pub fn resolve(&self, capability: &str) -> Option<&str> {
        self.routes.get(capability).map(|s| s.as_str())
    }

    /// 路由表条目数（可观测）。
    pub fn len(&self) -> usize {
        self.routes.len()
    }

    /// 路由表是否为空。
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::llm::LLMResponse;
    use anyhow::anyhow;
    use std::sync::atomic::Ordering;

    #[derive(Clone)]
    struct Fake {
        reply: String,
    }

    impl ChatBackend for Fake {
        async fn chat(&self, _m: &[Message]) -> Result<LLMResponse> {
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
    async fn dedupes_duplicate_strategy_outputs() {
        // 两个策略各返回不同结果 → 唯一 2，去重 0
        let strategies = vec![
            strategy(
                "text",
                "text 策略",
                Fake {
                    reply: "结果A".into(),
                },
            ),
            strategy(
                "code",
                "code 策略",
                Fake {
                    reply: "结果B".into(),
                },
            ),
        ];
        let r = run_multi_strategy("任务", strategies, None::<Fake>)
            .await
            .unwrap();
        assert_eq!(r.aggregated.len(), 2);
        assert_eq!(r.stats.unique_results.load(Ordering::Relaxed), 2);
        assert_eq!(r.stats.deduped.load(Ordering::Relaxed), 0);
        assert_eq!(r.stats.strategies_ran.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn dedupes_identical_outputs() {
        let strategies = vec![
            strategy(
                "a",
                "s",
                Fake {
                    reply: "同".into()
                },
            ),
            strategy(
                "b",
                "s",
                Fake {
                    reply: "同".into()
                },
            ),
        ];
        let r = run_multi_strategy("任务", strategies, None::<Fake>)
            .await
            .unwrap();
        assert_eq!(r.aggregated.len(), 1, "相同结果应去重");
        assert_eq!(r.stats.deduped.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn selector_picks_candidate_when_multiple() {
        let strategies = vec![
            strategy(
                "a",
                "s",
                Fake {
                    reply: "候选1".into(),
                },
            ),
            strategy(
                "b",
                "s",
                Fake {
                    reply: "候选2".into(),
                },
            ),
        ];
        // selector 后端恒返"最优答案"
        let r = run_multi_strategy(
            "任务",
            strategies,
            Some(Fake {
                reply: "最优答案".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(r.final_answer.as_deref(), Some("最优答案"));
    }

    #[tokio::test]
    async fn selector_skipped_when_single_unique_result() {
        let strategies = vec![strategy(
            "a",
            "s",
            Fake {
                reply: "唯一".into(),
            },
        )];
        // selector 即使注入，唯一结果也直接返回（不额外调用）
        let r = run_multi_strategy(
            "任务",
            strategies,
            Some(Fake {
                reply: "不应被调用".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(r.final_answer.as_deref(), Some("唯一"));
    }

    #[tokio::test]
    async fn no_selector_joins_aggregated() {
        let strategies = vec![strategy("a", "s", Fake { reply: "X".into() })];
        let r = run_multi_strategy("任务", strategies, None::<Fake>)
            .await
            .unwrap();
        assert_eq!(r.final_answer.as_deref(), Some("X"));
    }

    #[tokio::test]
    async fn strategy_failure_propagates() {
        struct Fail;
        impl ChatBackend for Fail {
            async fn chat(&self, _m: &[Message]) -> Result<LLMResponse> {
                Err(anyhow!("strategy down"))
            }
        }
        let strategies = vec![strategy("a", "s", Fail)];
        let err = run_multi_strategy("任务", strategies, None::<Fail>).await;
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("strategy down"));
    }

    #[test]
    fn dedupe_identity() {
        let (out, dup) = dedupe(vec!["a".into(), "a".into(), "b".into()]);
        assert_eq!(out, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(dup, 1);
    }

    // ---- FleetRouter（fleet 模型路由消费点）----

    #[test]
    fn fleet_router_resolves_capability_model() {
        let mut r = FleetRouter::new();
        r.add("review.run", "opus");
        assert_eq!(r.resolve("review.run"), Some("opus"));
        assert_eq!(r.resolve("missing.cap"), None);
        assert!(!r.is_empty());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn fleet_router_from_manifest() {
        let m = PluginManifest::from_json(
            r#"{"name":"x","version":"0.1.0","description":"x","capability_models":{"a":"sonnet"}}"#,
        )
        .unwrap();
        let r = FleetRouter::from_manifest(&m);
        assert_eq!(r.resolve("a"), Some("sonnet"));
        assert_eq!(r.resolve("b"), None);
    }

    #[test]
    fn strategy_with_model_tags_target() {
        let s = strategy_with_model("rev", "你是审查者", Fake { reply: "R".into() }, "opus");
        assert_eq!(s.model.as_deref(), Some("opus"));
    }
}
