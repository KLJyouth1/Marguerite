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
    /// 执行失败的策略数（其结果被丢弃，但其余策略的产出仍保留）
    pub failed: AtomicU64,
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

    // 保留成功策略的产出，只丢弃失败的那一个。
    //
    // 历史缺陷：原实现用 `contents.push(r?.content)`，任一策略出错就 `?` 提前返回，
    // 把**所有已经算完的成功结果**一起丢掉 —— 一个策略网络抖动，整轮多策略白跑。
    // 现在改为：收集全部成功产出；仅当**没有任何**策略成功时，才返回首个错误（保持
    // fail-fast 语义，单策略失败仍会 propagate）。
    let mut contents = Vec::with_capacity(n);
    let mut first_err: Option<anyhow::Error> = None;
    let mut failed = 0u64;
    for r in raws {
        match r {
            Ok(resp) => contents.push(resp.content),
            Err(e) => {
                failed += 1;
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
    }
    if contents.is_empty() {
        return Err(first_err
            .unwrap_or_else(|| anyhow::anyhow!("多策略并行没有可用策略（strategies 为空）")));
    }
    let (aggregated, dup) = dedupe(contents);

    let stats = MultiStrategyStats {
        strategies_ran: AtomicU64::new(n as u64),
        unique_results: AtomicU64::new(aggregated.len() as u64),
        deduped: AtomicU64::new(dup),
        failed: AtomicU64::new(failed),
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
        for (tc, res) in resp.tool_calls.iter().zip(results) {
            let output = match &res {
                ToolResult::Success { data, .. } => data.clone(),
                ToolResult::Error { message, code } => format!("[工具错误 {code}] {message}"),
                ToolResult::Timeout { tool_name, .. } => format!("[工具超时] {tool_name}"),
            };
            // E12：工具输出**真正进入模型上下文**的点就在这里（下面的 `Message::Tool`）。
            // 本路径不发 ToolCall/ToolResult 事件（网关事件桥看不到），只能在入历史处
            // 自记：按「入参 + 完整输出」计入上下文台账。正因不发事件，不存在与事件桥
            // 重复计数的可能。
            let ledger_session = crate::usage::SESSION
                .try_with(|s| s.clone())
                .ok()
                .filter(|s| !s.is_empty());
            if let Some(sess) = &ledger_session {
                crate::context::session::record_tool_output_at_context(
                    sess,
                    tc.id.as_deref(),
                    &tc.name,
                    &output,
                );
                crate::context::session::record_tool_output(sess, &tc.name, &tc.arguments);
            }
            // P2-10/T14：该路径未按单工具计时（多策略聚合），诚实置 None。
            messages.push(Message::Tool {
                name: tc.name.clone(),
                input: tc.arguments.clone(),
                output,
                timestamp: Utc::now(),
                elapsed_ms: None,
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

/// 角色 → 模型偏好解析（联合调用强化：供 Team / 子代理在 intent/complexity 路由之外叠加角色专属模型）。
///
/// 审计结论：`Team`/`SubAgentManager` 目前只按 `intent`/`complexity` 走 [`ModelRouter`]，
/// 角色专属模型偏好未被消费（`TeamMember` 仅用 `role` 做聚合标签/依赖/事件，不参与选路）。
/// 本函数提供「角色 → 模型名」的确定性映射，供上层在角色级选路时调用：
/// 命中内置角色表返回其偏好模型，未命中（或空角色）回退 `default`。
///
/// 内置常见角色表（键大小写/分隔符不敏感，见 [`normalize_role`]）：
/// - `architect` / `architecture` → `deepseek-v4-pro`（架构设计需强推理）
/// - `reviewer` / `review` / `critic` → `deepseek-v4-pro`（审查需严谨强推理）
/// - `qa` / `tester` / `test` → `deepseek-v4-flash`（测试迭代轻量快模型）
/// - `researcher` / `research` → `deepseek-v4-pro`（深度研究）
/// - `coder` / `engineer` / `developer` / `code` → `deepseek-v4-flash`（编码实现轻量快模型）
pub fn resolve_role_model(role: &str, default: &str) -> String {
    let normalized = normalize_role(role);
    if normalized.is_empty() {
        return default.to_string();
    }
    for (key, model) in builtin_role_models() {
        if normalized == *key {
            return (*model).to_string();
        }
    }
    default.to_string()
}

/// 规范化角色名：转小写并去掉分隔符（`-`/`_`/空白等），使 `Code-Review` / `code_review`
/// 归一到同一键。只保留字母/数字（含中日韩文字），便于与内置表精确匹配。
fn normalize_role(role: &str) -> String {
    role.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// 内置角色 → 模型偏好表（键已归一化）。
fn builtin_role_models() -> &'static [(&'static str, &'static str)] {
    &[
        // 架构/规划：强推理模型
        ("architect", "deepseek-v4-pro"),
        ("architecture", "deepseek-v4-pro"),
        // 审查/评审：强推理模型
        ("reviewer", "deepseek-v4-pro"),
        ("review", "deepseek-v4-pro"),
        ("critic", "deepseek-v4-pro"),
        // QA/测试：轻量快模型
        ("qa", "deepseek-v4-flash"),
        ("tester", "deepseek-v4-flash"),
        ("test", "deepseek-v4-flash"),
        // 研究/调研：强推理模型
        ("researcher", "deepseek-v4-pro"),
        ("research", "deepseek-v4-pro"),
        // 编码/实现：轻量快模型
        ("coder", "deepseek-v4-flash"),
        ("engineer", "deepseek-v4-flash"),
        ("developer", "deepseek-v4-flash"),
        ("code", "deepseek-v4-flash"),
    ]
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

    /// 回归守卫：部分策略失败时，成功策略的产出**必须保留**。
    #[tokio::test]
    async fn partial_failure_keeps_successes() {
        struct Flaky {
            fail: bool,
            reply: String,
        }
        impl ChatBackend for Flaky {
            async fn chat(&self, _m: &[Message]) -> Result<LLMResponse> {
                if self.fail {
                    return Err(anyhow!("strategy down"));
                }
                Ok(LLMResponse {
                    content: self.reply.clone(),
                    thinking: None,
                    model: "flaky".into(),
                    usage: crate::model::llm::TokenUsage::default(),
                    tool_calls: Vec::new(),
                })
            }
        }
        let strategies = vec![
            strategy(
                "ok",
                "s",
                Flaky {
                    fail: false,
                    reply: "好结果".into(),
                },
            ),
            strategy(
                "bad",
                "s",
                Flaky {
                    fail: true,
                    reply: String::new(),
                },
            ),
        ];
        let r = run_multi_strategy("任务", strategies, None::<Flaky>)
            .await
            .expect("有一个策略成功就不应整轮失败");
        assert_eq!(r.aggregated, vec!["好结果".to_string()]);
        assert_eq!(r.final_answer.as_deref(), Some("好结果"));
        assert_eq!(
            r.stats.failed.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "失败策略数应被计入统计"
        );
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

    // ---- resolve_role_model（角色 → 模型解析，联合调用消费点）----

    #[test]
    fn resolve_role_model_maps_core_roles() {
        assert_eq!(resolve_role_model("architect", "fb"), "deepseek-v4-pro");
        assert_eq!(resolve_role_model("reviewer", "fb"), "deepseek-v4-pro");
        assert_eq!(resolve_role_model("qa", "fb"), "deepseek-v4-flash");
        assert_eq!(resolve_role_model("researcher", "fb"), "deepseek-v4-pro");
        assert_eq!(resolve_role_model("coder", "fb"), "deepseek-v4-flash");
    }

    #[test]
    fn resolve_role_model_normalizes_case_and_separators() {
        assert_eq!(resolve_role_model("Architect", "fb"), "deepseek-v4-pro");
        assert_eq!(resolve_role_model("REVIEWER", "fb"), "deepseek-v4-pro");
        assert_eq!(resolve_role_model("re-viewer", "fb"), "deepseek-v4-pro");
        assert_eq!(resolve_role_model("Q_A", "fb"), "deepseek-v4-flash");
        assert_eq!(resolve_role_model("Engineer", "fb"), "deepseek-v4-flash");
    }

    #[test]
    fn resolve_role_model_falls_back_to_default() {
        assert_eq!(resolve_role_model("unknown-role", "fallback"), "fallback");
        assert_eq!(resolve_role_model("", "fallback"), "fallback");
        assert_eq!(resolve_role_model("   ", "fallback"), "fallback");
    }
}
