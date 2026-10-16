use serde::Serialize;
use crate::model::routing_contract::{CandidateProfile, RouteContractError, RouteMode, RouteRequest};

/// Weight source exposed by the scoring adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OmniWeightsSource {
    Default,
    User,
}

/// Normalized weights used by Omni scoring.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct OmniWeights {
    pub capability: f64,
    pub cost: f64,
    pub latency: f64,
    pub source: OmniWeightsSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OmniWeightsError {
    Invalid,
}

impl Default for OmniWeights {
    fn default() -> Self {
        Self { capability: 0.5, cost: 0.3, latency: 0.2, source: OmniWeightsSource::Default }
    }
}

impl OmniWeights {
    /// Build normalized user weights. Values must be finite, non-negative,
    /// and have a strictly positive sum.
    pub fn new(capability: f64, cost: f64, latency: f64) -> Result<Self, OmniWeightsError> {
        let values = [capability, cost, latency];
        if values.iter().any(|value| !value.is_finite() || *value < 0.0) {
            return Err(OmniWeightsError::Invalid);
        }
        let sum = values.iter().sum::<f64>();
        if sum <= 0.0 {
            return Err(OmniWeightsError::Invalid);
        }
        Ok(Self {
            capability: capability / sum,
            cost: cost / sum,
            latency: latency / sum,
            source: OmniWeightsSource::User,
        })
    }

    fn validate(&self) -> Result<(), OmniWeightsError> {
        let values = [self.capability, self.cost, self.latency];
        if values.iter().any(|value| !value.is_finite() || *value < 0.0)
            || values.iter().sum::<f64>() <= 0.0
        {
            Err(OmniWeightsError::Invalid)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OmniCandidateScore {
    pub model: String,
    pub capability: f64,
    pub cost: f64,
    pub latency: f64,
    pub total: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OmniDecision {
    pub request_id: String,
    pub mode: RouteMode,
    pub selected_model: Option<String>,
    pub candidates: Vec<String>,
    pub scores: Vec<OmniCandidateScore>,
    pub weights_source: OmniWeightsSource,
    pub explanation: String,
    pub fallback: bool,
}

/// Deterministic, local Omni scoring adapter. It never performs provider I/O.
#[derive(Debug, Clone, Copy, Default)]
pub struct OmniScorer {
    weights: OmniWeights,
}

impl OmniScorer {
    pub fn with_weights(weights: OmniWeights) -> Self {
        Self { weights }
    }

    pub fn weights(&self) -> OmniWeights {
        self.weights
    }

    pub fn decide(
        &self,
        request: &RouteRequest,
        profiles: &[CandidateProfile],
    ) -> Result<OmniDecision, Vec<RouteContractError>> {
        request.validate()?;
        if self.weights.validate().is_err() {
            return Err(vec![RouteContractError::InvalidWeights]);
        }

        let mut scores: Vec<_> = profiles
            .iter()
            .filter(|profile| request.candidates.iter().any(|candidate| candidate == &profile.model))
            .map(|profile| self.score(profile))
            .collect();
        scores.sort_by(|left, right| {
            right.total.partial_cmp(&left.total)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.model.cmp(&right.model))
        });
        let selected_model = scores.first().map(|score| score.model.clone());
        let fallback = selected_model.is_none();
        Ok(OmniDecision {
            request_id: request.request_id.clone(),
            mode: RouteMode::Omni,
            selected_model,
            candidates: request.candidates.clone(),
            scores,
            weights_source: self.weights.source,
            explanation: if fallback {
                "No eligible candidate; Omni fallback required".into()
            } else {
                "Selected highest deterministic Omni weighted score".into()
            },
            fallback,
        })
    }

    fn score(&self, profile: &CandidateProfile) -> OmniCandidateScore {
        // Cost and latency are inverted so lower values are better.
        OmniCandidateScore {
            model: profile.model.clone(),
            capability: profile.capability,
            cost: profile.cost,
            latency: profile.latency,
            total: profile.capability * self.weights.capability
                + (1.0 - profile.cost) * self.weights.cost
                + (1.0 - profile.latency) * self.weights.latency,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::routing_contract::{CandidateProfile, RouteMode, RouteRequest, RouteWeights};
    use std::collections::BTreeMap;

    fn request(candidates: Vec<&str>) -> RouteRequest {
        RouteRequest { request_id: "req-1".into(), intent: "chat".into(), complexity: 0.5, mode: RouteMode::Omni, candidates: candidates.into_iter().map(str::to_owned).collect(), weights: RouteWeights::default(), metadata: BTreeMap::new() }
    }
    fn profile(model: &str, capability: f64, cost: f64, latency: f64) -> CandidateProfile {
        CandidateProfile { model: model.into(), capability, cost, latency }
    }

    #[test]
    fn default_weights_are_inherited() {
        let weights = OmniWeights::default();
        assert_eq!((weights.capability, weights.cost, weights.latency), (0.5, 0.3, 0.2));
        assert_eq!(weights.source, OmniWeightsSource::Default);
    }
    #[test]
    fn custom_weights_are_normalized_and_marked_user() {
        let weights = OmniWeights::new(2.0, 1.0, 1.0).unwrap();
        assert_eq!(weights.source, OmniWeightsSource::User);
        assert!((weights.capability - 0.5).abs() < 1e-9);
        assert!((weights.cost - 0.25).abs() < 1e-9);
    }
    #[test]
    fn invalid_weights_are_rejected() {
        assert!(OmniWeights::new(-1.0, 1.0, 1.0).is_err());
        assert!(OmniWeights::new(0.0, 0.0, 0.0).is_err());
        assert!(OmniWeights::new(f64::NAN, 1.0, 1.0).is_err());
    }
    #[test]
    fn capability_priority_selects_capable_model() {
        let scorer = OmniScorer::with_weights(OmniWeights::new(1.0, 0.0, 0.0).unwrap());
        let result = scorer.decide(&request(vec!["a", "b"]), &[profile("a", 0.9, 1.0, 1.0), profile("b", 0.5, 0.0, 0.0)]).unwrap();
        assert_eq!(result.selected_model.as_deref(), Some("a"));
    }
    #[test]
    fn cost_priority_selects_lower_cost() {
        let scorer = OmniScorer::with_weights(OmniWeights::new(0.0, 1.0, 0.0).unwrap());
        let result = scorer.decide(&request(vec!["expensive", "cheap"]), &[profile("expensive", 1.0, 0.9, 0.0), profile("cheap", 0.0, 0.1, 1.0)]).unwrap();
        assert_eq!(result.selected_model.as_deref(), Some("cheap"));
    }
    #[test]
    fn latency_priority_selects_lower_latency() {
        let scorer = OmniScorer::with_weights(OmniWeights::new(0.0, 0.0, 1.0).unwrap());
        let result = scorer.decide(&request(vec!["slow", "fast"]), &[profile("slow", 1.0, 0.0, 0.9), profile("fast", 0.0, 1.0, 0.1)]).unwrap();
        assert_eq!(result.selected_model.as_deref(), Some("fast"));
    }
    #[test]
    fn ties_break_by_stable_model_id() {
        let scorer = OmniScorer::default();
        let result = scorer.decide(&request(vec!["zeta", "alpha"]), &[profile("zeta", 0.5, 0.5, 0.5), profile("alpha", 0.5, 0.5, 0.5)]).unwrap();
        assert_eq!(result.selected_model.as_deref(), Some("alpha"));
        assert_eq!(result.scores[0].model, "alpha");
    }
    #[test]
    fn empty_candidates_return_stable_fallback() {
        let result = OmniScorer::default().decide(&request(vec![]), &[]).unwrap();
        assert!(result.fallback);
        assert_eq!(result.selected_model, None);
        assert_eq!(result.explanation, "No eligible candidate; Omni fallback required");
    }
    #[test]
    fn sensitive_metadata_never_appears_in_json_contract() {
        let mut req = request(vec!["m"]);
        req.metadata.insert("api_key".into(), "secret-value".into());
        req.metadata.insert("token".into(), "secret-token".into());
        let result = OmniScorer::default().decide(&req, &[profile("m", 1.0, 0.0, 0.0)]).unwrap();
        let json = serde_json::to_string(&result).unwrap();
        assert!(!json.contains("secret-value"));
        assert!(!json.contains("secret-token"));
    }
    #[test]
    fn json_contract_contains_required_fields() {
        let result = OmniScorer::default().decide(&request(vec!["m"]), &[profile("m", 1.0, 0.0, 0.0)]).unwrap();
        let value: serde_json::Value = serde_json::to_value(result).unwrap();
        for field in ["request_id", "mode", "selected_model", "candidates", "scores", "weights_source", "explanation", "fallback"] { assert!(value.get(field).is_some(), "missing {field}"); }
        assert_eq!(value["mode"], "omni");
    }
}
