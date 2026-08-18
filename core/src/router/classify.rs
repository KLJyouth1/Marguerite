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
        default_decision(catalog, DecisionSource::RuntimeUnavailable, "v4_unavailable")
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
                steps: vec![PipelineStepRecord {
                    step_name: "classify".to_string(),
                    applied: true,
                    routing_source: "rule".to_string(),
                    fallback_reason: None,
                }],
            }
        }
    };

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
        steps: vec![PipelineStepRecord {
            step_name: "classify".to_string(),
            applied: false,
            routing_source: source.as_str().to_string(),
            fallback_reason: Some(reason.to_string()),
        }],
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
        steps: vec![PipelineStepRecord {
            step_name: "classify".to_string(),
            applied: false,
            routing_source: source.as_str().to_string(),
            fallback_reason: Some(reason.to_string()),
        }],
    }
}
