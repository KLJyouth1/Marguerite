//! 规则版 classify()：runtime 闸门 → 打分 → 选档 → 弱路由 → 降级（端口 opensquilla classify）。
//!
//! 纯函数、零 LLM 开销、可单测。复用 [`crate::intent::Classifier`] 的本地意图分类结果
//! 作为 `classify_input_from_turn` 的输入信号之一（不直接参与打分，Phase 2 接入）。

use super::*;

/// 弱路由阈值：低于此置信度走历史粘性（与 opensquilla confidence_threshold 一致）。
const CONFIDENCE_THRESHOLD: f64 = 0.5;

/// 分类入口：使用内置默认 catalog。
pub fn classify(input: &ClassifyInput) -> RouteDecision {
    classify_with(input, &PersonaCatalog::default_catalog())
}

/// 可注入 catalog 的分类（便于测试空名册 / 自定义名册）。
pub fn classify_with(input: &ClassifyInput, catalog: &PersonaCatalog) -> RouteDecision {
    // —— 步骤 1：runtime 可用性闸门（opensquilla v4_unavailable 语义）——
    let mut decision = if !input.runtime_available {
        default_decision(
            catalog,
            DecisionSource::RuntimeUnavailable,
            "v4_unavailable",
        )
    } else if catalog.personas.iter().filter(|p| p.enabled).count() == 0 {
        // 启用中的 persona 为空 → 降级（classify 异常保护，不 panic）
        default_decision(catalog, DecisionSource::Default, "classify_error")
    } else {
        // —— 步骤 2：对名册每个 persona 规则打分 ——
        let mut scored: Vec<(PersonaTier, f64)> = catalog
            .personas
            .iter()
            .filter(|p| p.enabled)
            .map(|p| (p.tier, score_persona(p, input)))
            .collect();

        // —— 步骤 3：选最高分档 ——
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let (best_tier, best_conf) = scored
            .first()
            .cloned()
            .unwrap_or((PersonaTier::General, 0.0));

        // —— 步骤 4：弱路由（confidence < 阈值，历史粘性）——
        if best_conf < CONFIDENCE_THRESHOLD {
            if let Some(prev) = input.prev_route_decisions.last() {
                decision_for(
                    catalog,
                    prev.tier,
                    best_conf,
                    DecisionSource::History,
                    "weak_confidence_stick_prev",
                )
            } else {
                default_decision(
                    catalog,
                    DecisionSource::History,
                    "weak_confidence_stick_default",
                )
            }
        } else {
            // —— 步骤 5：由档派生 thinking_mode / prompt_policy ——
            let p = catalog
                .lookup(best_tier)
                .expect("best_tier 必在启用名册中（步骤 2 已保证）");
            RouteDecision {
                tier: best_tier,
                confidence: best_conf,
                thinking_mode: p.default_thinking_mode,
                prompt_policy: p.default_prompt_policy,
                source: DecisionSource::Rule,
                fallback_reason: None,
                effort_override: None,
                steps: vec![PipelineStepRecord {
                    step_name: "classify".to_string(),
                    applied: true,
                    routing_source: "rule".to_string(),
                    fallback_reason: None,
                }],
            }
        }
    };

    // —— P1-4：复杂度提升（复杂任务不再被压到 General/T0）——
    // 规则路由此前对「无 persona 命中」或「只命中弱关键词」的输入一律落 General/T0
    // （实测决策日志 2140 条中 2138 条为 General、2115 条为 T0）。这里用可测的复杂度
    // 信号把**复杂任务**提升到更高档 + T1；简单闲聊保持 General/T0（阈值与依据见下方常量）。
    if decision.tier == PersonaTier::General
        && decision.thinking_mode == ThinkingMode::T0
        && estimate_complexity(input) >= TaskComplexity::Medium
    {
        let tier = complexity_tier(input);
        if let Some(p) = catalog.lookup(tier) {
            decision.tier = tier;
            decision.thinking_mode = p.default_thinking_mode;
            decision.prompt_policy = p.default_prompt_policy;
            decision.source = DecisionSource::Rule;
            decision.fallback_reason = Some("complexity_escalated".to_string());
            decision.steps.push(PipelineStepRecord {
                step_name: "complexity_escalate".to_string(),
                applied: true,
                routing_source: "rule".to_string(),
                fallback_reason: Some("complexity_escalated".to_string()),
            });
        }
    }

    // —— Track B 切片 1：视觉取证门（additive，不改非图片轮的路由）——
    // 仅当本轮带图（has_image）时，追加一条 vision_gate 可观测步骤。决策逻辑留在
    // `vision_gate` 模块；此处只记录其决策（routing_source），不影响 tier/thinking/policy。
    if input.has_image {
        let gate = crate::router::vision_gate::evaluate_vision_need(
            &crate::router::vision_gate::VisionGateContext {
                message: input.message.clone(),
                // 本轮带图即视为存在可复用的近期图片上下文
                has_recent_image: true,
                recent_turn_id: None,
                recent_turns: 0,
            },
        );
        decision.steps.push(PipelineStepRecord {
            step_name: "vision_gate".to_string(),
            applied: true,
            routing_source: gate.as_str().to_string(),
            fallback_reason: None,
        });
    }

    decision
}

/// 打分：消息命中 persona 的 match_rules 累加权重，归一化到 [0,1]；
/// 超大上下文额外加权 Explorer 档。
fn score_persona(p: &Persona, input: &ClassifyInput) -> f64 {
    let mut score = 0.0;
    for rule in &p.match_rules {
        if rule.matches(&input.message) {
            score += rule.weight;
        }
    }
    if input.context_tokens_est > 64_000 && p.tier == PersonaTier::Explorer {
        score += 0.3;
    }
    score.clamp(0.0, 1.0)
}

/// 默认档决策（tier = General，思考/策略取 General 默认值）。
fn default_decision(
    catalog: &PersonaCatalog,
    source: DecisionSource,
    reason: &str,
) -> RouteDecision {
    let (tm, pp) = catalog
        .lookup(PersonaTier::General)
        .map(|p| (p.default_thinking_mode, p.default_prompt_policy))
        .unwrap_or((ThinkingMode::T0, PromptPolicy::P0));
    RouteDecision {
        tier: PersonaTier::General,
        confidence: 0.0,
        thinking_mode: tm,
        prompt_policy: pp,
        source,
        fallback_reason: Some(reason.to_string()),
        effort_override: None,
        steps: vec![PipelineStepRecord {
            step_name: "classify".to_string(),
            applied: false,
            routing_source: source.as_str().to_string(),
            fallback_reason: Some(reason.to_string()),
        }],
    }
}

/// 任务复杂度档（P1-4）。仅用于「弱信号 / General 结果」是否应被提升的判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskComplexity {
    Low,
    Medium,
    High,
}

/// 复杂度阈值（可测、有依据）：
/// - 字符数：`>= COMPLEXITY_LONG_CHARS` → +2；`>= COMPLEXITY_MEDIUM_CHARS` → +1；
/// - 结构化/多步骤标记命中数：`>= 2` → +2；`== 1` → +1；
/// - 复杂度关键词命中 → +1；
/// - 上下文 token 估算 `>= COMPLEXITY_CONTEXT_TOKENS` → +1。
///
/// 总分 `0..=1` → Low；`2..=3` → Medium；`>= 4` → High。
///
/// 阈值取「一句话闲聊 / 单问单答必然为 Low」为准：闲聊消息通常 < 80 字、无多步骤标记、
/// 无复杂度关键词 → 0 分 → Low，因而维持 General/T0（不误伤简单交互）。
const COMPLEXITY_LONG_CHARS: usize = 200;
const COMPLEXITY_MEDIUM_CHARS: usize = 80;
const COMPLEXITY_CONTEXT_TOKENS: usize = 16_000;

/// 复杂度关键词（跨中英；命中即 +1）。
const COMPLEXITY_KEYWORDS: &[&str] = &[
    "重构",
    "架构",
    "设计",
    "实现",
    "优化",
    "分布式",
    "并发",
    "性能",
    "系统",
    "算法",
    "迁移",
    "方案",
    "algorithm",
    "refactor",
    "architecture",
    "design",
    "optimize",
    "implement",
    "migrate",
    "concurrent",
    "system",
];

/// 多步骤 / 结构化标记（命中数 >= 2 → +2）。
const MULTI_STEP_MARKERS: &[&str] = &[
    "然后", "接着", "首先", "其次", "最后", "并且", "同时", "1.", "2.", "3.", "\n-", "\n*",
    "and then", "finally",
];

/// 估算任务复杂度（纯函数、零 LLM 开销、可单测）。
pub fn estimate_complexity(input: &ClassifyInput) -> TaskComplexity {
    let text = &input.message;
    let chars = text.chars().count();
    let lower = text.to_ascii_lowercase();

    let mut score: u32 = 0;
    if chars >= COMPLEXITY_LONG_CHARS {
        score += 2;
    } else if chars >= COMPLEXITY_MEDIUM_CHARS {
        score += 1;
    }
    let markers = MULTI_STEP_MARKERS
        .iter()
        .filter(|k| lower.contains(&k.to_ascii_lowercase()))
        .count();
    if markers >= 2 {
        score += 2;
    } else if markers == 1 {
        score += 1;
    }
    if COMPLEXITY_KEYWORDS
        .iter()
        .any(|k| lower.contains(&k.to_ascii_lowercase()))
    {
        score += 1;
    }
    if input.context_tokens_est >= COMPLEXITY_CONTEXT_TOKENS {
        score += 1;
    }

    match score {
        0..=1 => TaskComplexity::Low,
        2..=3 => TaskComplexity::Medium,
        _ => TaskComplexity::High,
    }
}

/// 复杂任务的目标档（按领域信号择档；未知领域归 Reasoner——深度推理）。
fn complexity_tier(input: &ClassifyInput) -> PersonaTier {
    let lower = input.message.to_ascii_lowercase();
    let has = |kws: &[&str]| kws.iter().any(|k| lower.contains(&k.to_ascii_lowercase()));

    if has(&[
        "代码", "函数", "重构", "bug", "修复", "cargo", "rust", "python", "impl ", "fn ",
        "code", "refactor", "api", "数据库", "sql",
    ]) {
        PersonaTier::Coder
    } else if has(&[
        "工具", "终端", "shell", "浏览器", "桌面", "脚本", "部署", "运维", "deploy", "terminal",
        "browser", "自动化",
    ]) {
        PersonaTier::Tooler
    } else if input.context_tokens_est >= COMPLEXITY_CONTEXT_TOKENS {
        PersonaTier::Explorer
    } else {
        PersonaTier::Reasoner
    }
}

/// 弱路由回退到指定 tier 的决策（保持该档的 thinking/policy 连续）。
fn decision_for(
    catalog: &PersonaCatalog,
    tier: PersonaTier,
    conf: f64,
    source: DecisionSource,
    reason: &str,
) -> RouteDecision {
    let (tm, pp) = catalog
        .lookup(tier)
        .map(|p| (p.default_thinking_mode, p.default_prompt_policy))
        .unwrap_or((ThinkingMode::T0, PromptPolicy::P0));
    RouteDecision {
        tier,
        confidence: conf,
        thinking_mode: tm,
        prompt_policy: pp,
        source,
        fallback_reason: Some(reason.to_string()),
        effort_override: None,
        steps: vec![PipelineStepRecord {
            step_name: "classify".to_string(),
            applied: false,
            routing_source: source.as_str().to_string(),
            fallback_reason: Some(reason.to_string()),
        }],
    }
}
