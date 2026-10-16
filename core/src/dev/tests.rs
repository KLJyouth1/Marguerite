use super::*;

#[test]
fn intent_routing_is_deterministic_and_unknown_is_general() {
    assert_eq!(classify_intent("修复这个 Rust bug"), DevIntent::Coding);
    assert_eq!(classify_intent("分析这个方案的利弊"), DevIntent::Reasoning);
    assert_eq!(classify_intent("你好，今天怎么样"), DevIntent::Chat);
    assert_eq!(classify_intent("一段没有规则的输入"), DevIntent::General);
    assert_eq!(classify_intent("修复这个 Rust bug"), classify_intent("修复这个 Rust bug"));
}

#[test]
fn budget_allocation_stays_within_total_and_is_auditable() {
    let allocation = BudgetAllocation::allocate(1000, [100, 200, 300, 400]).unwrap();
    assert_eq!(allocation.total, 1000);
    assert_eq!(allocation.layers.iter().map(|x| x.allocated).sum::<u64>(), 1000);
    assert!(allocation.layers.iter().all(|x| x.allocated <= x.requested));
    assert!(allocation.audit().contains("L0"));
}

#[test]
fn budget_rejects_zero_negative_or_overflowing_requests() {
    assert!(matches!(BudgetAllocation::allocate(0, [1, 1, 1, 1]), Err(DevPipelineError::ZeroBudget)));
    assert!(matches!(BudgetAllocation::allocate(3, [1, 1, 1, 1]), Err(DevPipelineError::InsufficientBudget { .. })));
    assert!(matches!(BudgetAllocation::allocate(u64::MAX, [u64::MAX, 1, 1, 1]), Err(DevPipelineError::Overflow)));
}

#[test]
fn pipeline_returns_structured_template() {
    let pipeline = DevPipeline::new(1000);
    let result = pipeline.run("修复 API 超时", [200, 200, 200, 200]).unwrap();
    assert_eq!(result.intent, DevIntent::Coding);
    assert!(!result.template.summary.is_empty());
    assert!(!result.template.steps.is_empty());
    assert_eq!(result.template.schema_version, "dev.v1");
}

#[test]
fn template_rejects_empty_goal_and_unknown_fields() {
    assert!(matches!(render_template(TemplateInput::new("")), Err(TemplateError::EmptyGoal)));
    assert!(matches!(TemplateInput::new("goal").with_unknown_field("x"), Err(TemplateError::InvalidField { .. })));
}
