//! Track A 单元测试：正常路由 / 弱路由 / 降级 / 映射 / 决策日志 / catalog 不变量（T1–T12）。

#![cfg(test)]

use std::io::Read;

use super::*;
use crate::cache::hash_text;
use crate::model::llm::{LLMClient, LLMConfig, LLMProvider};
use crate::model::thinking::ThinkingEffort;

/// 构造一个普通路由输入（无历史、runtime 可用）。
fn input(msg: &str) -> ClassifyInput {
    ClassifyInput {
        message: msg.to_string(),
        routing_history: vec![],
        prev_route_decisions: vec![],
        context_tokens_est: 0,
        runtime_available: true,
    }
}

/// 构造一个带 tier/thinking/policy 的路由决策（用于弱路由 prev 回退）。
fn decision(tier: PersonaTier, tm: ThinkingMode, pp: PromptPolicy) -> RouteDecision {
    RouteDecision {
        tier,
        confidence: 0.9,
        thinking_mode: tm,
        prompt_policy: pp,
        source: DecisionSource::Rule,
        fallback_reason: None,
        steps: vec![],
    }
}

// T1：代码提示 → coder 档
#[test]
fn t1_code_routes_to_coder() {
    let d = classify(&input("请帮我写一个函数 def foo() 并修复 bug"));
    assert_eq!(d.tier, PersonaTier::Coder);
    assert!(d.confidence >= 0.5, "confidence 应 >= 0.5，实际 {}", d.confidence);
    assert_eq!(d.source, DecisionSource::Rule);
    assert_eq!(d.thinking_mode, ThinkingMode::T1);
    assert_eq!(d.prompt_policy, PromptPolicy::P1);
}

// T2：数学证明提示 → reasoner 档
#[test]
fn t2_proof_routes_to_reasoner() {
    let d = classify(&input("证明这个定理 theorem proof"));
    assert_eq!(d.tier, PersonaTier::Reasoner);
    assert_eq!(d.thinking_mode, ThinkingMode::T1);
    assert_eq!(d.prompt_policy, PromptPolicy::P1);
}

// T3：闲聊提示 → general 档
#[test]
fn t3_chat_routes_to_general() {
    let d = classify(&input("你好，今天天气怎么样"));
    assert_eq!(d.tier, PersonaTier::General);
    assert_eq!(d.thinking_mode, ThinkingMode::T0);
    assert_eq!(d.prompt_policy, PromptPolicy::P0);
}

// T4：模糊提示（无强信号）→ 弱路由，无历史时 General + History
#[test]
fn t4_weak_routing_without_history() {
    let d = classify(&input("随便聊聊今天的午饭吃什么"));
    assert!(d.confidence < 0.5, "弱路由应 confidence < 0.5");
    assert_eq!(d.source, DecisionSource::History);
    assert_eq!(d.tier, PersonaTier::General);
    assert_eq!(d.fallback_reason.as_deref(), Some("weak_confidence_stick_default"));
}

// T5：弱路由 + 有 prev 决策 → 采用 prev.tier
#[test]
fn t5_weak_routing_with_prev() {
    let prev = decision(PersonaTier::Coder, ThinkingMode::T1, PromptPolicy::P1);
    let mut inp = input("随便聊聊");
    inp.prev_route_decisions.push(prev);
    let d = classify(&inp);
    assert!(d.confidence < 0.5);
    assert_eq!(d.source, DecisionSource::History);
    assert_eq!(d.tier, PersonaTier::Coder);
    assert_eq!(d.fallback_reason.as_deref(), Some("weak_confidence_stick_prev"));
}

// T6：runtime 不可用 → RuntimeUnavailable 降级
#[test]
fn t6_runtime_unavailable_fallback() {
    let mut inp = input("任何消息");
    inp.runtime_available = false;
    let d = classify(&inp);
    assert_eq!(d.source, DecisionSource::RuntimeUnavailable);
    assert_eq!(d.fallback_reason.as_deref(), Some("v4_unavailable"));
    assert_eq!(d.tier, PersonaTier::General);
}

// T7：名册空 → 降级 Default，不 panic
#[test]
fn t7_empty_catalog_falls_back() {
    let d = classify_with(&input("任何消息"), &PersonaCatalog::default());
    assert_eq!(d.source, DecisionSource::Default);
    assert_eq!(d.fallback_reason.as_deref(), Some("classify_error"));
    assert_eq!(d.tier, PersonaTier::General);
}

// T8：thinking 映射
#[test]
fn t8_thinking_mode_mapping() {
    assert_eq!(ThinkingMode::T0.to_thinking_effort(), ThinkingEffort::Off);
    assert_eq!(ThinkingMode::T1.to_thinking_effort(), ThinkingEffort::Deep);
    let d = decision(PersonaTier::General, ThinkingMode::T1, PromptPolicy::P0);
    assert_eq!(d.thinking_effort(), ThinkingEffort::Deep);
}

// T9：prompt 映射
#[test]
fn t9_prompt_policy_mapping() {
    assert_eq!(PromptPolicy::P0.system_fragment(), "");
    assert!(!PromptPolicy::P1.system_fragment().is_empty());
}

// T10：集成级（无 live LLM）—— 决策派生 thinking_effort + with_thinking 生效 + P1 片段非空
#[test]
fn t10_injection_wiring_without_llm() {
    let d = classify(&input("实现一个 Rust trait impl 并写测试"));
    assert_eq!(d.tier, PersonaTier::Coder);
    assert_eq!(d.thinking_mode, ThinkingMode::T1);
    assert_eq!(d.prompt_policy, PromptPolicy::P1);
    // 思考预算档位派生
    assert_eq!(d.thinking_effort(), ThinkingEffort::Deep);
    // with_thinking 把档位落到 LLMConfig.thinking（不发起网络请求）
    let mut cfg = LLMConfig::default();
    cfg.model = "deepseek-v4-pro".to_string();
    cfg.provider = LLMProvider::DeepSeek;
    let client = LLMClient::new(cfg).with_thinking(d.thinking_effort());
    assert_eq!(client.config.thinking, ThinkingEffort::Deep);
    // P1 策略片段非空（运行时装配点会前置为 system 消息）
    assert!(!d.prompt_policy.system_fragment().is_empty());
}

// T11：决策日志——每轮一行、含 schema_version、model_hash 为 hash（无明文模型名）、可独立解析
#[test]
fn t11_decision_log_format_and_hashing() {
    let dir = std::env::temp_dir().join(format!("marg_router_log_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时日志目录");

    let model = "deepseek-v4-pro";
    let expected_hash = format!("{:x}", hash_text(model));

    // 多轮：Rule / 弱路由 / runtime 不可用
    let d1 = classify(&input("用 cargo 重构这段代码"));
    DecisionLog::record_to_dir(&d1, "sess-1", "turn-1", model, &dir).unwrap();
    let mut weak = input("随便聊聊");
    weak.prev_route_decisions
        .push(decision(PersonaTier::Reasoner, ThinkingMode::T1, PromptPolicy::P1));
    let d2 = classify(&weak);
    DecisionLog::record_to_dir(&d2, "sess-1", "turn-2", model, &dir).unwrap();
    let mut ru = input("任何消息");
    ru.runtime_available = false;
    let d3 = classify(&ru);
    DecisionLog::record_to_dir(&d3, "sess-1", "turn-3", model, &dir).unwrap();

    let path = dir.join("router_decision_log.jsonl");
    let mut content = String::new();
    std::fs::File::open(&path)
        .expect("日志文件应存在")
        .read_to_string(&mut content)
        .unwrap();
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 3, "应恰好追加 3 行");

    for line in &lines {
        // 每行可独立 serde_json::from_str
        let v: serde_json::Value =
            serde_json::from_str(line).expect("每行应可被 serde_json 解析");
        assert_eq!(v["schema_version"], 1, "应含 schema_version=1");
        let model_hash = v["model_hash"].as_str().expect("应含 model_hash");
        assert_eq!(model_hash, expected_hash, "model_hash 应等于 hash_text 的十六进制");
        assert!(model_hash.chars().all(|c| c.is_ascii_hexdigit()), "model_hash 应为十六进制");
        assert_ne!(model_hash, model, "model_hash 不得是明文模型名");
        assert!(!content.contains(model), "日志不得包含明文模型名");
        // 原始 message 不得入日志（此处消息未写入，仅校验字段存在）
        assert!(v.get("tier").is_some());
        assert!(v.get("source").is_some());
    }

    let _ = std::fs::remove_dir_all(&dir);
}

// T12：catalog 不变量——非空、tier 唯一、五档全覆盖
#[test]
fn t12_catalog_invariants() {
    let cat = PersonaCatalog::default_catalog();
    assert!(!cat.personas.is_empty(), "默认 catalog 应非空");

    // tier 唯一
    let mut seen = std::collections::HashSet::new();
    for p in &cat.personas {
        assert!(seen.insert(p.tier), "tier {:?} 在默认 catalog 中重复", p.tier);
    }

    // 五档全覆盖
    let all = [
        PersonaTier::General,
        PersonaTier::Coder,
        PersonaTier::Reasoner,
        PersonaTier::Tooler,
        PersonaTier::Explorer,
    ];
    for t in all {
        assert!(cat.lookup(t).is_some(), "默认 catalog 应覆盖 tier {:?}", t);
    }
}
