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
        has_image: false,
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
        effort_override: None,
        steps: vec![],
    }
}

/// T3：会话级/请求级 effort 覆盖——thinking_mode 对齐 + 精确档位经 effort_override 落模型层。
#[test]
fn effort_override_aligns_mode_and_carries_exact_effort() {
    use crate::model::thinking::ThinkingEffort;
    let base = decision(PersonaTier::General, ThinkingMode::T0, PromptPolicy::P0);

    // None：行为与旧版完全一致
    let d = base.clone().with_effort_override(None);
    assert_eq!(d.effort_override, None);
    assert_eq!(d.thinking_effort(), ThinkingEffort::Off);

    // Off/Normal → T0；Deep/Max → T1（前端消费的 thinking_mode 语义不回退）
    let d = base.clone().with_effort_override(Some(ThinkingEffort::Off));
    assert_eq!(d.thinking_mode, ThinkingMode::T0);
    assert_eq!(d.thinking_effort(), ThinkingEffort::Off);

    let d = base.clone().with_effort_override(Some(ThinkingEffort::Normal));
    assert_eq!(d.thinking_mode, ThinkingMode::T0);
    assert_eq!(d.thinking_effort(), ThinkingEffort::Normal);

    let d = base.clone().with_effort_override(Some(ThinkingEffort::Deep));
    assert_eq!(d.thinking_mode, ThinkingMode::T1);
    assert_eq!(d.thinking_effort(), ThinkingEffort::Deep);

    let d = base.with_effort_override(Some(ThinkingEffort::Max));
    assert_eq!(d.thinking_mode, ThinkingMode::T1);
    assert_eq!(d.thinking_effort(), ThinkingEffort::Max);

    // 序列化：无覆盖时不带新键（旧前端 / 旧决策日志零影响）
    let v = serde_json::to_value(decision(PersonaTier::General, ThinkingMode::T0, PromptPolicy::P0)).unwrap();
    assert!(v.get("effort_override").is_none(), "None 覆盖不得出现在序列化输出");
}

// T1：代码提示 → coder 档
#[test]
fn t1_code_routes_to_coder() {
    let d = classify(&input("请帮我写一个函数 def foo() 并修复 bug"));
    assert_eq!(d.tier, PersonaTier::Coder);
    assert!(
        d.confidence >= 0.5,
        "confidence 应 >= 0.5，实际 {}",
        d.confidence
    );
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
    assert_eq!(
        d.fallback_reason.as_deref(),
        Some("weak_confidence_stick_default")
    );
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
    assert_eq!(
        d.fallback_reason.as_deref(),
        Some("weak_confidence_stick_prev")
    );
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
    let cfg = LLMConfig {
        model: "deepseek-v4-pro".to_string(),
        provider: LLMProvider::DeepSeek,
        ..Default::default()
    };
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
    weak.prev_route_decisions.push(decision(
        PersonaTier::Reasoner,
        ThinkingMode::T1,
        PromptPolicy::P1,
    ));
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
        let v: serde_json::Value = serde_json::from_str(line).expect("每行应可被 serde_json 解析");
        assert_eq!(v["schema_version"], 1, "应含 schema_version=1");
        let model_hash = v["model_hash"].as_str().expect("应含 model_hash");
        assert_eq!(
            model_hash, expected_hash,
            "model_hash 应等于 hash_text 的十六进制"
        );
        assert!(
            model_hash.chars().all(|c| c.is_ascii_hexdigit()),
            "model_hash 应为十六进制"
        );
        assert_ne!(model_hash, model, "model_hash 不得是明文模型名");
        assert!(!content.contains(model), "日志不得包含明文模型名");
        // 原始 message 不得入日志（此处消息未写入，仅校验字段存在）
        assert!(v.get("tier").is_some());
        assert!(v.get("source").is_some());
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// W4③：决策日志必须**有上限**，而且轮转在 Windows 上真的跑得起来。
///
/// 这条同时钉住两个坑：
/// 1. 它原本是纯 append、永不轮转 —— 每行还带完整 `steps` 轨迹，长任务下只增不减；
/// 2. 写句柄 `f` 若没在轮转之前释放，`fs::rename` 在 Windows 上会因共享冲突失败。
///    而失败是被 `warn` 吞掉的，所以断言看的是「`.1` 真的出现了」，
///    不是「调用没报错」—— 后者在句柄泄漏时照样成立，正是假绿。
#[test]
fn t13_decision_log_rotation_caps_generations() {
    let dir = std::env::temp_dir().join(format!(
        "marg_router_rot_{}_{}",
        std::process::id(),
        line!()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时日志目录");

    let model = "deepseek-v4-pro";
    // 阈值给小：真实门槛是 10 MB，让单测先写满 10 MB 是在制造体积而不是验证语义。
    const ROTATE_AT: u64 = 2048;
    let mut d = classify(&input("用 cargo 重构这段代码"));
    d.steps = vec![
        PipelineStepRecord {
            step_name: "rule_match".to_string(),
            applied: true,
            routing_source: "Rule".to_string(),
            fallback_reason: None,
        };
        6
    ];
    let turns = 30usize;
    for i in 0..turns {
        DecisionLog::record_to_dir_with_rotation(
            &d,
            "sess-1",
            &format!("turn-{i}"),
            model,
            &dir,
            ROTATE_AT,
        )
        .unwrap();
    }

    let path = dir.join("router_decision_log.jsonl");
    let rotated = dir.join("router_decision_log.jsonl.1");
    assert!(
        rotated.exists(),
        "超过阈值必须切出 .1 一代（未切出=轮转没生效，多为写句柄未在 rename 前释放）"
    );

    // 单代封顶：写入返回后当前代必须回到阈值以下。
    // 最后一次 append 若正好触发轮转，当前代此刻是被切走的状态（下一次写才新建）⇒ 视作 0。
    let cur_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    assert!(
        cur_bytes < ROTATE_AT,
        "当前代必须小于阈值，实测 {cur_bytes} ≥ {ROTATE_AT}"
    );

    // 代数封顶：只允许 .1 / .2 两代历史，不得出现 .3。
    assert!(
        !dir.join("router_decision_log.jsonl.3").exists(),
        "历史代数必须封顶（保留 {} 代含当前代）",
        crate::observability::rotation::KEEP_GENERATIONS
    );

    // 存活内容仍是合法 JSONL，且明文模型名红线跨代同样成立。
    let mut surviving: Vec<String> = vec![];
    let mut total_bytes = 0u64;
    for name in [
        "router_decision_log.jsonl",
        "router_decision_log.jsonl.1",
        "router_decision_log.jsonl.2",
    ] {
        let p = dir.join(name);
        let Ok(mut f) = std::fs::File::open(&p) else { continue };
        let mut content = String::new();
        f.read_to_string(&mut content).unwrap();
        total_bytes += content.len() as u64;
        assert!(!content.contains(model), "{name} 不得含明文模型名");
        for line in content.lines().filter(|l| !l.trim().is_empty()) {
            let v: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("{name} 轮转后应逐行可解析: {e}"));
            assert_eq!(v["schema_version"], 1, "{name} 应含 schema_version=1");
            surviving.push(v["turn_id"].as_str().unwrap_or_default().to_string());
        }
    }

    assert!(
        surviving.contains(&"turn-29".to_string()),
        "轮转后最新一次写入仍须落盘（append 语义不能因轮转而断掉）"
    );
    assert!(
        !surviving.contains(&"turn-0".to_string()),
        "有界保留必须真的丢最老的一代，否则只是把无限增长换了个名字"
    );
    assert!(
        (surviving.len() as u64) < turns as u64,
        "总量必须小于未轮转的 {} 行，实测存活 {} 行",
        turns,
        surviving.len()
    );
    assert!(
        total_bytes < ROTATE_AT * crate::observability::rotation::KEEP_GENERATIONS as u64 + 4096,
        "合起来的体积必须有界，实测 {total_bytes}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ── P1-4：复杂任务不应被压到 General/T0 ──

#[test]
fn estimate_complexity_low_for_chat_high_for_multistep() {
    assert_eq!(
        estimate_complexity(&input("你好，今天天气怎么样")),
        TaskComplexity::Low
    );
    assert_eq!(estimate_complexity(&input("随便聊聊今天的午饭吃什么")), TaskComplexity::Low);
    let long = "请设计一个高可用的分布式订单系统，拆解为若干服务，然后说明每个服务的职责，\
                并且评估一致性、可用性与性能之间的权衡，最后给出分阶段的迁移方案与回滚策略。";
    assert!(
        estimate_complexity(&input(long)) >= TaskComplexity::Medium,
        "长且多步骤的任务应为 Medium 及以上"
    );
}

#[test]
fn p14_complex_generic_task_escalates_out_of_general_t0() {
    // 无任何强 persona 关键词的复杂设计任务：此前会落 General/T0；现在应提升到更高档 + T1。
    let msg = "请设计一个高可用的分布式订单系统，拆解为若干服务，然后说明每个服务的职责，\
               并且评估一致性与性能权衡，最后给出迁移方案。";
    let d = classify(&input(msg));
    assert_ne!(d.tier, PersonaTier::General, "复杂任务不应留在 General: {d:?}");
    assert_eq!(d.thinking_mode, ThinkingMode::T1, "复杂任务应开启思考: {d:?}");
    assert_eq!(
        d.fallback_reason.as_deref(),
        Some("complexity_escalated"),
        "应以 complexity_escalated 标记提升来源: {d:?}"
    );
}

#[test]
fn p14_complex_code_task_escalates_to_coder_t1() {
    let msg = "请重构这个模块的架构，修复并发 bug，然后补充测试并优化性能。";
    let d = classify(&input(msg));
    assert_eq!(d.tier, PersonaTier::Coder, "{d:?}");
    assert_eq!(d.thinking_mode, ThinkingMode::T1, "{d:?}");
}

#[test]
fn p14_simple_chat_still_general_t0() {
    // 反向守卫：简单闲聊不得被复杂度提升误伤。
    let d = classify(&input("你好，今天天气怎么样"));
    assert_eq!(d.tier, PersonaTier::General);
    assert_eq!(d.thinking_mode, ThinkingMode::T0);
    assert_ne!(d.fallback_reason.as_deref(), Some("complexity_escalated"));
}

// T12：catalog 不变量——非空、tier 唯一、五档全覆盖
#[test]
fn t12_catalog_invariants() {
    let cat = PersonaCatalog::default_catalog();
    assert!(!cat.personas.is_empty(), "默认 catalog 应非空");

    // tier 唯一
    let mut seen = std::collections::HashSet::new();
    for p in &cat.personas {
        assert!(
            seen.insert(p.tier),
            "tier {:?} 在默认 catalog 中重复",
            p.tier
        );
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
