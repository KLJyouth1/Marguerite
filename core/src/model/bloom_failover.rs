use serde::Serialize;
use std::fmt;

/// A Bloom candidate and whether its provider credential is available.
/// Credential material is deliberately not stored in this value.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CandidateCredential {
    pub model: String,
    pub has_key: bool,
    pub capability: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum SkipReason {
    MissingCredential,
    ExecutionFailed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SkippedCandidate {
    pub model: String,
    pub reason: SkipReason,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FailoverDecision {
    pub selected: CandidateCredential,
    pub candidates: Vec<CandidateCredential>,
    pub skipped: Vec<SkippedCandidate>,
    pub fallback: bool,
    pub attempts: usize,
}

impl FailoverDecision {
    pub fn skipped_count(&self) -> usize { self.skipped.len() }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExecutionResult {
    pub selected: CandidateCredential,
    pub fallback: bool,
    pub attempts: usize,
    pub failures: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FailoverError {
    NoCredentials { unavailable_count: usize },
    ExecutionExhausted { attempts: usize },
}

impl FailoverError {
    pub const NO_CREDENTIALS_CODE: &'static str = "BLOOM_NO_CREDENTIALS";

    pub fn code(&self) -> &'static str {
        match self {
            Self::NoCredentials { .. } => Self::NO_CREDENTIALS_CODE,
            Self::ExecutionExhausted { .. } => "BLOOM_EXECUTION_FAILED",
        }
    }

    /// 409 communicates a valid request with no usable provider; this is not a 404.
    pub fn http_status(&self) -> u16 { 409 }

    pub fn unavailable_count(&self) -> usize {
        match self {
            Self::NoCredentials { unavailable_count } => *unavailable_count,
            Self::ExecutionExhausted { .. } => 0,
        }
    }
}

impl fmt::Display for FailoverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCredentials { unavailable_count } => write!(f, "{NO_CREDENTIALS_CODE}: {unavailable_count} candidates have no credential", NO_CREDENTIALS_CODE = Self::NO_CREDENTIALS_CODE),
            Self::ExecutionExhausted { attempts } => write!(f, "BLOOM_EXECUTION_FAILED: all candidates failed after {attempts} attempts"),
        }
    }
}

impl std::error::Error for FailoverError {}

/// Local-only credential-aware failover. It never performs network I/O.
#[derive(Clone, Debug, Default)]
pub struct FailoverPolicy {
    candidates: Vec<CandidateCredential>,
}

impl FailoverPolicy {
    pub fn new(candidates: Vec<CandidateCredential>) -> Self { Self { candidates } }

    /// Filter missing credentials, then stably order by capability and model id.
    pub fn resolve(&self) -> Result<FailoverDecision, FailoverError> {
        let skipped: Vec<_> = self.candidates.iter().filter(|c| !c.has_key).map(|c| SkippedCandidate {
            model: c.model.clone(),
            reason: SkipReason::MissingCredential,
        }).collect();
        let candidates = self.eligible_sorted();
        let selected = candidates.first().cloned().ok_or(FailoverError::NoCredentials {
            unavailable_count: skipped.len(),
        })?;
        Ok(FailoverDecision { selected, candidates, skipped, fallback: false, attempts: 0 })
    }

    /// Execute candidates in deterministic order; failed providers are skipped locally.
    pub fn execute<F, E>(&self, mut executor: F) -> Result<ExecutionResult, FailoverError>
    where
        F: FnMut(&CandidateCredential) -> Result<(), E>,
    {
        let candidates = self.eligible_sorted();
        if candidates.is_empty() {
            return Err(FailoverError::NoCredentials {
                unavailable_count: self.candidates.iter().filter(|c| !c.has_key).count(),
            });
        }
        let mut attempts = 0;
        for candidate in &candidates {
            attempts += 1;
            if executor(candidate).is_ok() {
                return Ok(ExecutionResult {
                    selected: candidate.clone(),
                    fallback: attempts > 1,
                    attempts,
                    failures: attempts - 1,
                });
            }
        }
        Err(FailoverError::ExecutionExhausted { attempts })
    }

    fn eligible_sorted(&self) -> Vec<CandidateCredential> {
        let mut candidates: Vec<_> = self.candidates.iter().filter(|c| c.has_key).cloned().collect();
        candidates.sort_by(|left, right| right.capability.total_cmp(&left.capability).then_with(|| left.model.cmp(&right.model)));
        candidates
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(model: &str, has_key: bool, capability: f64) -> CandidateCredential {
        CandidateCredential { model: model.into(), has_key, capability }
    }

    #[test]
    fn filters_candidates_without_credentials_and_retains_reason_count() {
        let policy = FailoverPolicy::new(vec![candidate("missing", false, 1.0), candidate("ready", true, 0.5)]);
        let decision = policy.resolve().expect("ready candidate");
        assert_eq!(decision.selected.model, "ready");
        assert_eq!(decision.skipped_count(), 1);
        assert_eq!(decision.skipped[0].model, "missing");
        assert_eq!(decision.skipped[0].reason, SkipReason::MissingCredential);
    }

    #[test]
    fn selects_available_candidate() {
        let policy = FailoverPolicy::new(vec![candidate("first", true, 0.8), candidate("second", true, 0.2)]);
        let decision = policy.resolve().unwrap();
        assert_eq!(decision.selected.model, "first");
        assert!(!decision.fallback);
        assert_eq!(decision.attempts, 0);
    }

    #[test]
    fn all_missing_credentials_return_stable_error() {
        let policy = FailoverPolicy::new(vec![candidate("a", false, 0.8), candidate("b", false, 0.2)]);
        let error = policy.resolve().unwrap_err();
        assert_eq!(error.code(), "BLOOM_NO_CREDENTIALS");
        assert_ne!(error.http_status(), 404);
        assert_eq!(error.unavailable_count(), 2);
    }

    #[test]
    fn ordering_is_deterministic_by_capability_then_model() {
        let policy = FailoverPolicy::new(vec![candidate("zeta", true, 0.7), candidate("alpha", true, 0.7), candidate("low", true, 0.1)]);
        let decision = policy.resolve().unwrap();
        let models: Vec<_> = decision.candidates.iter().map(|c| c.model.as_str()).collect();
        assert_eq!(models, vec!["alpha", "zeta", "low"]);
    }

    #[test]
    fn failed_execution_falls_back_to_next_candidate() {
        let policy = FailoverPolicy::new(vec![candidate("primary", true, 0.9), candidate("backup", true, 0.8)]);
        let result = policy.execute(|candidate| {
            if candidate.model == "primary" { Err("provider unavailable") } else { Ok(()) }
        }).unwrap();
        assert_eq!(result.selected.model, "backup");
        assert!(result.fallback);
        assert_eq!(result.attempts, 2);
        assert_eq!(result.failures, 1);
    }

    #[test]
    fn sensitive_key_material_is_not_serialized_or_debugged() {
        let candidate = candidate("safe-model", true, 1.0);
        let json = serde_json::to_string(&candidate).unwrap();
        let debug = format!("{candidate:?}");
        assert!(!json.contains("api_key"));
        assert!(!json.contains("secret"));
        assert!(!debug.contains("api_key"));
        assert!(!debug.contains("secret"));
    }
}
