use super::attribution::{AttributionError, UsageAttribution};
use crate::model::llm::TokenUsage;
use serde_json::json;

fn attribution(
    run_id: &str,
    session_id: &str,
    model: &str,
    prompt_tokens: u64,
    completion_tokens: u64,
    cache_read_tokens: Option<u64>,
) -> UsageAttribution {
    UsageAttribution::new(
        run_id,
        session_id,
        model,
        prompt_tokens,
        completion_tokens,
        prompt_tokens + completion_tokens,
        cache_read_tokens,
    )
    .expect("valid usage attribution")
}

#[test]
fn attribution_exposes_all_token_fields_and_serializes_contract() {
    let value = attribution("run-1", "session-1", "model-a", 12, 8, Some(3));
    assert_eq!(value.run_id, "run-1");
    assert_eq!(value.session_id, "session-1");
    assert_eq!(value.model, "model-a");
    assert_eq!(value.prompt_tokens, 12);
    assert_eq!(value.completion_tokens, 8);
    assert_eq!(value.total_tokens, 20);
    assert_eq!(value.cache_read_tokens, Some(3));

    let encoded = serde_json::to_value(&value).expect("serialize attribution");
    assert_eq!(
        encoded,
        json!({
            "run_id": "run-1",
            "session_id": "session-1",
            "model": "model-a",
            "prompt_tokens": 12,
            "completion_tokens": 8,
            "total_tokens": 20,
            "cache_read_tokens": 3
        })
    );
    let decoded: UsageAttribution = serde_json::from_value(encoded).expect("deserialize attribution");
    assert_eq!(decoded, value);
}

#[test]
fn attribution_can_be_created_from_existing_token_usage() {
    let usage = TokenUsage {
        prompt_tokens: 9,
        completion_tokens: 4,
        total_tokens: 13,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
    };
    let value = UsageAttribution::from_token_usage("run", "session", "model", &usage)
        .expect("existing usage maps to attribution");
    assert_eq!(value.total_tokens, 13);
    assert_eq!(value.cache_read_tokens, Some(0));
}

#[test]
fn validation_rejects_inconsistent_totals() {
    let error = UsageAttribution::new("run", "session", "model", 4, 3, 99, None)
        .expect_err("inconsistent total must fail");
    assert!(matches!(error, AttributionError::TotalMismatch { .. }));
}

#[test]
fn deserialization_rejects_inconsistent_totals() {
    let result: Result<UsageAttribution, _> = serde_json::from_value(json!({
        "run_id": "r", "session_id": "s", "model": "m",
        "prompt_tokens": 4, "completion_tokens": 3, "total_tokens": 99
    }));
    assert!(result.is_err());
}

#[test]
fn cache_read_accepts_missing_null_and_zero() {
    let missing: UsageAttribution = serde_json::from_value(json!({
        "run_id": "r",
        "session_id": "s",
        "model": "m",
        "prompt_tokens": 1,
        "completion_tokens": 2,
        "total_tokens": 3
    }))
    .expect("missing cache field defaults to none");
    assert_eq!(missing.cache_read_tokens, None);

    let null_value: UsageAttribution = serde_json::from_value(json!({
        "run_id": "r", "session_id": "s", "model": "m",
        "prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3,
        "cache_read_tokens": null
    }))
    .expect("null cache field is accepted");
    assert_eq!(null_value.cache_read_tokens, None);
    assert_eq!(serde_json::to_value(&null_value).unwrap()["cache_read_tokens"], json!(null));

    let zero = attribution("r", "s", "m", 1, 2, Some(0));
    assert_eq!(zero.cache_read_tokens, Some(0));
}

#[test]
fn accumulation_keeps_run_session_model_isolated() {
    let mut first = attribution("run-1", "session-1", "model-a", 10, 2, Some(4));
    let second = attribution("run-1", "session-1", "model-a", 5, 3, None);
    first.accumulate(&second).expect("same run can accumulate");
    assert_eq!(first.prompt_tokens, 15);
    assert_eq!(first.completion_tokens, 5);
    assert_eq!(first.total_tokens, 20);
    assert_eq!(first.cache_read_tokens, Some(4));

    let foreign_run = attribution("run-2", "session-1", "model-a", 1, 1, Some(1));
    let error = first.accumulate(&foreign_run).expect_err("different run must be isolated");
    assert!(matches!(error, AttributionError::ScopeMismatch { .. }));
    assert_eq!(first.total_tokens, 20);
}

#[test]
fn negative_values_are_rejected_by_unsigned_contract() {
    let result: Result<UsageAttribution, _> = serde_json::from_value(json!({
        "run_id": "r", "session_id": "s", "model": "m",
        "prompt_tokens": -1, "completion_tokens": 0, "total_tokens": 0
    }));
    assert!(result.is_err());
}

#[test]
fn accumulation_reports_overflow_without_mutating_attribution() {
    let mut value = attribution("r", "s", "m", u64::MAX, 0, Some(u64::MAX));
    let increment = attribution("r", "s", "m", 0, 1, Some(1));
    let error = value.accumulate(&increment).expect_err("overflow must fail");
    assert!(matches!(error, AttributionError::Overflow { field: "total_tokens" }));
    assert_eq!(value.prompt_tokens, u64::MAX);
    assert_eq!(value.completion_tokens, 0);
    assert_eq!(value.total_tokens, u64::MAX);
    assert_eq!(value.cache_read_tokens, Some(u64::MAX));
}
