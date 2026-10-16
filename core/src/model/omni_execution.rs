//! Credential-aware execution seam for deterministic Omni routing.
//! This module performs no provider or network I/O; callers provide the executor.

use crate::model::bloom_failover::CandidateCredential;
use crate::model::omni::{OmniCandidateScore, OmniScorer, OmniWeights, OmniWeightsSource};
use crate::model::routing_contract::{CandidateProfile, RouteContractError, RouteMode, RouteRequest};
use crate::usage::attribution::{AttributionError, UsageAttribution};
use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OmniExecutionDecision {
    pub request_id: String,
    pub mode: RouteMode,
    pub selected_model: Option<String>,
    pub candidates_considered: Vec<String>,
    pub scores: Vec<OmniCandidateScore>,
    pub weights_source: OmniWeightsSource,
    pub skipped_missing_credentials: Vec<String>,
    pub execution_failures: Vec<String>,
    pub fallback: bool,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OmniExecutionResult {
    pub request_id: String,
    pub mode: RouteMode,
    pub selected_model: Option<String>,
    pub attempts: usize,
    pub fallback: bool,
    pub explanation: String,
    pub decision: OmniExecutionDecision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageAttribution>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OmniExecutionError {
    InvalidRequest(Vec<RouteContractError>),
    NoCredentials { unavailable_count: usize },
    ExecutionFailed { attempts: usize },
    Attribution(AttributionError),
}

impl OmniExecutionError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest(_) => "OMNI_INVALID_REQUEST",
            Self::NoCredentials { .. } => "OMNI_NO_CREDENTIALS",
            Self::ExecutionFailed { .. } => "OMNI_EXECUTION_FAILED",
            Self::Attribution(_) => "OMNI_INVALID_ATTRIBUTION",
        }
    }

    pub fn attempts(&self) -> usize {
        match self {
            Self::ExecutionFailed { attempts } => *attempts,
            _ => 0,
        }
    }
}

impl fmt::Display for OmniExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(errors) => write!(f, "{}: {} validation errors", self.code(), errors.len()),
            Self::NoCredentials { unavailable_count } => write!(f, "{}: {unavailable_count} candidates have no credential", self.code()),
            Self::ExecutionFailed { attempts } => write!(f, "{}: all candidates failed after {attempts} attempts", self.code()),
            Self::Attribution(error) => write!(f, "{}: {error}", self.code()),
        }
    }
}
impl std::error::Error for OmniExecutionError {}

#[derive(Debug, Clone, Copy)]
pub struct OmniExecution {
    scorer: OmniScorer,
}

impl Default for OmniExecution {
    fn default() -> Self { Self::new() }
}

impl OmniExecution {
    pub fn new() -> Self { Self { scorer: OmniScorer::default() } }
    pub fn with_weights(weights: OmniWeights) -> Self { Self { scorer: OmniScorer::with_weights(weights) } }
    pub fn weights(&self) -> OmniWeights { self.scorer.weights() }

    pub fn execute<F, E>(
        &self,
        request: &RouteRequest,
        profiles: &[CandidateProfile],
        credentials: &[CandidateCredential],
        mut executor: F,
    ) -> Result<OmniExecutionResult, OmniExecutionError>
    where
        F: FnMut(&CandidateCredential) -> Result<(), E>,
    {
        self.execute_internal(request, profiles, credentials, |candidate| {
            executor(candidate).map(|_| None)
        })
    }

    pub fn execute_with_attribution<F, E>(
        &self,
        request: &RouteRequest,
        profiles: &[CandidateProfile],
        credentials: &[CandidateCredential],
        executor: F,
    ) -> Result<OmniExecutionResult, OmniExecutionError>
    where
        F: FnMut(&CandidateCredential) -> Result<Option<UsageAttribution>, E>,
    {
        self.execute_internal(request, profiles, credentials, executor)
    }

    fn execute_internal<F, E>(
        &self,
        request: &RouteRequest,
        profiles: &[CandidateProfile],
        credentials: &[CandidateCredential],
        mut executor: F,
    ) -> Result<OmniExecutionResult, OmniExecutionError>
    where
        F: FnMut(&CandidateCredential) -> Result<Option<UsageAttribution>, E>,
    {
        let decision = self.scorer.decide(request, profiles).map_err(OmniExecutionError::InvalidRequest)?;
        let mut skipped_missing_credentials = Vec::new();
        let mut ordered = Vec::new();
        for score in &decision.scores {
            match credentials.iter().find(|candidate| candidate.model == score.model) {
                Some(candidate) if candidate.has_key => ordered.push(candidate.clone()),
                Some(_) | None => skipped_missing_credentials.push(score.model.clone()),
            }
        }
        if ordered.is_empty() {
            return Err(OmniExecutionError::NoCredentials { unavailable_count: skipped_missing_credentials.len() });
        }

        let mut execution_failures = Vec::new();
        for (index, candidate) in ordered.iter().enumerate() {
            let usage = match executor(candidate) {
                Ok(usage) => usage,
                Err(_) => {
                    execution_failures.push(candidate.model.clone());
                    continue;
                }
            };
            if let Some(ref attribution) = usage {
                if attribution.run_id != request.request_id || attribution.model != candidate.model {
                    return Err(OmniExecutionError::Attribution(AttributionError::ScopeMismatch {
                        run_id: attribution.run_id.clone(), session_id: attribution.session_id.clone(), model: attribution.model.clone(),
                    }));
                }
            }
            let fallback = index > 0;
            let selected_model = Some(candidate.model.clone());
            let explanation = if fallback { "Primary Omni candidate failed; execution used deterministic fallback" } else { "Executed highest deterministic Omni candidate" };
            let execution_decision = OmniExecutionDecision {
                request_id: request.request_id.clone(), mode: RouteMode::Omni, selected_model: selected_model.clone(),
                candidates_considered: request.candidates.clone(), scores: decision.scores.clone(), weights_source: decision.weights_source,
                skipped_missing_credentials, execution_failures, fallback, explanation: explanation.into(),
            };
            return Ok(OmniExecutionResult { request_id: request.request_id.clone(), mode: RouteMode::Omni, selected_model, attempts: index + 1, fallback, explanation: explanation.into(), decision: execution_decision, usage });
        }
        Err(OmniExecutionError::ExecutionFailed { attempts: ordered.len() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::bloom_failover::CandidateCredential;
    use crate::model::omni::OmniWeights;
    use crate::model::routing_contract::{CandidateProfile, RouteMode, RouteRequest, RouteWeights};
    use crate::usage::attribution::UsageAttribution;
    use std::collections::BTreeMap;

    fn request(candidates: &[&str]) -> RouteRequest {
        RouteRequest { request_id: "run-1".into(), intent: "chat".into(), complexity: 0.5, mode: RouteMode::Omni, candidates: candidates.iter().map(|x| (*x).into()).collect(), weights: RouteWeights::default(), metadata: BTreeMap::new() }
    }
    fn profile(model: &str, capability: f64, cost: f64, latency: f64) -> CandidateProfile { CandidateProfile { model: model.into(), capability, cost, latency } }
    fn credential(model: &str, has_key: bool) -> CandidateCredential { CandidateCredential { model: model.into(), has_key, capability: 0.5 } }

    #[test]
    fn default_omni_selection_is_deterministic() {
        let result = OmniExecution::new().execute(&request(&["cheap", "capable"]), &[profile("cheap", 0.4, 0.1, 0.2), profile("capable", 0.9, 0.2, 0.2)], &[credential("cheap", true), credential("capable", true)], |_| Ok::<(), ()>(())).unwrap();
        assert_eq!(result.selected_model.as_deref(), Some("capable")); assert_eq!(result.mode, RouteMode::Omni); assert_eq!(result.attempts, 1); assert!(!result.fallback);
    }
    #[test]
    fn user_weights_change_selection() {
        let result = OmniExecution::with_weights(OmniWeights::new(0.0, 1.0, 0.0).unwrap()).execute(&request(&["expensive", "cheap"]), &[profile("expensive", 1.0, 0.9, 0.0), profile("cheap", 0.0, 0.1, 1.0)], &[credential("expensive", true), credential("cheap", true)], |_| Ok::<(), ()>(())).unwrap();
        assert_eq!(result.selected_model.as_deref(), Some("cheap")); assert_eq!(result.decision.weights_source, crate::model::omni::OmniWeightsSource::User);
    }
    #[test]
    fn missing_keys_are_filtered_before_execution() {
        let result = OmniExecution::new().execute(&request(&["missing", "ready"]), &[profile("missing", 1.0, 0.0, 0.0), profile("ready", 0.5, 0.5, 0.5)], &[credential("missing", false), credential("ready", true)], |c| { assert_eq!(c.model, "ready"); Ok::<(), ()>(()) }).unwrap();
        assert_eq!(result.selected_model.as_deref(), Some("ready")); assert_eq!(result.decision.skipped_missing_credentials, vec!["missing"]);
    }
    #[test]
    fn failed_execution_falls_back_to_next_model() {
        let mut calls = 0; let result = OmniExecution::new().execute(&request(&["primary", "backup"]), &[profile("primary", 1.0, 0.0, 0.0), profile("backup", 0.8, 0.2, 0.2)], &[credential("primary", true), credential("backup", true)], |c| { calls += 1; if c.model == "primary" { Err("down") } else { Ok(()) } }).unwrap();
        assert_eq!(calls, 2); assert_eq!(result.selected_model.as_deref(), Some("backup")); assert!(result.fallback); assert_eq!(result.attempts, 2); assert_eq!(result.decision.execution_failures, vec!["primary"]);
    }
    #[test]
    fn all_missing_keys_return_stable_error() {
        let result = OmniExecution::new().execute(&request(&["a", "b"]), &[profile("a", 1.0, 0.0, 0.0), profile("b", 0.5, 0.0, 0.0)], &[credential("a", false), credential("b", false)], |_| Ok::<(), ()>(())); assert_eq!(result.unwrap_err().code(), "OMNI_NO_CREDENTIALS");
    }
    #[test]
    fn all_failures_return_stable_error() {
        let error = OmniExecution::new().execute(&request(&["a", "b"]), &[profile("a", 1.0, 0.0, 0.0), profile("b", 0.5, 0.0, 0.0)], &[credential("a", true), credential("b", true)], |_| Err::<(), _>("down")).unwrap_err(); assert_eq!(error.code(), "OMNI_EXECUTION_FAILED"); assert_eq!(error.attempts(), 2);
    }
    #[test]
    fn decision_fields_are_echoed_and_secrets_do_not_leak() {
        let mut req = request(&["m"]); req.metadata.insert("api_key".into(), "secret-key".into()); let result = OmniExecution::new().execute(&req, &[profile("m", 1.0, 0.0, 0.0)], &[credential("m", true)], |_| Ok::<(), ()>(())).unwrap(); let json = serde_json::to_string(&result).unwrap(); let debug = format!("{result:?}"); assert_eq!(result.request_id, "run-1"); assert_eq!(result.decision.candidates_considered, vec!["m"]); assert!(!json.contains("secret-key")); assert!(!debug.contains("secret-key"));
    }
    #[test]
    fn valid_and_invalid_token_attribution_are_handled() {
        let valid = UsageAttribution::new("run-1", "session-1", "m", 2, 3, 5, None).unwrap(); let result = OmniExecution::new().execute_with_attribution(&request(&["m"]), &[profile("m", 1.0, 0.0, 0.0)], &[credential("m", true)], |_| Ok::<_, ()>(Some(valid.clone()))).unwrap(); assert_eq!(result.usage.unwrap().total_tokens, 5); let invalid = UsageAttribution::new("run-1", "session-1", "m", 2, 3, 4, None); assert!(invalid.is_err());
    }
}
