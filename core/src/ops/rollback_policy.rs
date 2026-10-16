use serde::{Deserialize, Serialize};
use std::fmt;

use super::{
    OPS_CONFIRMATION_REQUIRED, OPS_ROLLBACK_UNSUPPORTED, OPS_SNAPSHOT_FAILED, OPS_SNAPSHOT_REQUIRED,
};

/// Rollback strategy selected before a mutating operation is executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RollbackClass {
    Compensation,
    Snapshot,
    Irreversible,
}

/// Evidence produced by a snapshot attempt. Failed evidence is intentionally not sufficient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotEvidence {
    Succeeded { snapshot_id: String },
    Failed(String),
}

/// A rollback plan. `warning` is a human-readable compensation or irreversible warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackPlan {
    pub class: RollbackClass,
    #[serde(serialize_with = "serialize_warning")]
    pub warning: String,
    pub snapshot_id: Option<String>,
}

impl RollbackPlan {
    pub fn compensation(description: impl Into<String>) -> Self {
        Self {
            class: RollbackClass::Compensation,
            warning: description.into(),
            snapshot_id: None,
        }
    }

    pub fn snapshot(snapshot_id: impl Into<String>) -> Self {
        Self {
            class: RollbackClass::Snapshot,
            warning: "operation can be restored from the pre-operation snapshot".into(),
            snapshot_id: Some(snapshot_id.into()),
        }
    }

    pub fn irreversible(warning: impl Into<String>) -> Self {
        Self {
            class: RollbackClass::Irreversible,
            warning: warning.into(),
            snapshot_id: None,
        }
    }
}

/// Outcome of the pre-execution guard. No operation is performed by this module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackDecision {
    pub allowed: bool,
    pub class: RollbackClass,
    pub warning: String,
    pub snapshot_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackError {
    code: &'static str,
    message: String,
}

impl RollbackError {
    pub fn code(&self) -> &'static str {
        self.code
    }
    pub fn message(&self) -> &str {
        &self.message
    }
}
impl fmt::Display for RollbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for RollbackError {}

/// Classify a known operation without executing it. Unknown operations are conservative.
pub fn classify_operation(operation: &str) -> RollbackClass {
    let op = operation.trim().to_ascii_lowercase();
    if op.contains("drop")
        || op.contains("destroy")
        || op.contains("purge")
        || op.contains("delete")
    {
        RollbackClass::Irreversible
    } else if op.contains("snapshot") || op.contains("restore") {
        RollbackClass::Snapshot
    } else {
        RollbackClass::Compensation
    }
}

pub fn guard_before_execute(
    plan: &RollbackPlan,
    snapshot_succeeded: bool,
    confirmed: bool,
) -> Result<RollbackDecision, RollbackError> {
    let evidence = match (&plan.class, plan.snapshot_id.as_ref(), snapshot_succeeded) {
        (RollbackClass::Snapshot, None, _) => {
            return Err(error(
                OPS_SNAPSHOT_REQUIRED,
                "snapshot handle is required before execution",
            ))
        }
        (RollbackClass::Snapshot, Some(id), true) => SnapshotEvidence::Succeeded {
            snapshot_id: id.clone(),
        },
        (RollbackClass::Snapshot, Some(_), false) => {
            SnapshotEvidence::Failed("snapshot was not successful".into())
        }
        _ => SnapshotEvidence::Failed(String::new()),
    };
    guard_before_execute_with_evidence(plan, evidence, confirmed)
}

pub fn guard_before_execute_with_evidence(
    plan: &RollbackPlan,
    evidence: SnapshotEvidence,
    confirmed: bool,
) -> Result<RollbackDecision, RollbackError> {
    if plan.warning.trim().is_empty() {
        return Err(error(
            OPS_ROLLBACK_UNSUPPORTED,
            "rollback description is required",
        ));
    }
    match plan.class {
        RollbackClass::Compensation => Ok(decision(plan)),
        RollbackClass::Snapshot => match evidence {
            SnapshotEvidence::Succeeded { snapshot_id }
                if !snapshot_id.trim().is_empty()
                    && plan.snapshot_id.as_deref() == Some(snapshot_id.as_str()) =>
            {
                Ok(decision_with_snapshot(plan, snapshot_id))
            }
            SnapshotEvidence::Succeeded { .. } => Err(error(
                OPS_SNAPSHOT_REQUIRED,
                "snapshot handle is required before execution",
            )),
            SnapshotEvidence::Failed(reason) => Err(error(
                OPS_SNAPSHOT_FAILED,
                format!("snapshot failed: {}", sanitize(reason)),
            )),
        },
        RollbackClass::Irreversible if confirmed => Ok(decision(plan)),
        RollbackClass::Irreversible => Err(error(
            OPS_CONFIRMATION_REQUIRED,
            "explicit confirmation is required for irreversible operation",
        )),
    }
}

fn decision(plan: &RollbackPlan) -> RollbackDecision {
    RollbackDecision {
        allowed: true,
        class: plan.class,
        warning: sanitize(plan.warning.clone()),
        snapshot_id: plan.snapshot_id.clone(),
    }
}
fn decision_with_snapshot(plan: &RollbackPlan, snapshot_id: String) -> RollbackDecision {
    RollbackDecision {
        allowed: true,
        class: plan.class,
        warning: sanitize(plan.warning.clone()),
        snapshot_id: Some(snapshot_id),
    }
}
fn error(code: &'static str, message: impl Into<String>) -> RollbackError {
    RollbackError {
        code,
        message: sanitize(message.into()),
    }
}

fn sanitize(input: String) -> String {
    let lower = input.to_ascii_lowercase();
    if lower.contains("token=") || lower.contains("password=") || lower.contains("secret=") {
        "[redacted]".into()
    } else {
        input
    }
}
fn serialize_warning<S>(warning: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&sanitize(warning.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_all_three_rollback_strategies() {
        assert_eq!(
            classify_operation("update_config"),
            RollbackClass::Compensation
        );
        assert_eq!(
            classify_operation("restore_snapshot"),
            RollbackClass::Snapshot
        );
        assert_eq!(
            classify_operation("drop_database"),
            RollbackClass::Irreversible
        );
    }
    #[test]
    fn snapshot_with_success_evidence_is_allowed() {
        let decision =
            guard_before_execute(&RollbackPlan::snapshot("snap-123"), true, false).unwrap();
        assert!(decision.allowed);
        assert_eq!(decision.class, RollbackClass::Snapshot);
    }
    #[test]
    fn snapshot_without_handle_is_rejected() {
        let plan = RollbackPlan {
            class: RollbackClass::Snapshot,
            warning: "restore state".into(),
            snapshot_id: None,
        };
        assert_eq!(
            guard_before_execute(&plan, true, false).unwrap_err().code(),
            OPS_SNAPSHOT_REQUIRED
        );
    }
    #[test]
    fn snapshot_failure_is_rejected_structurally() {
        assert_eq!(
            guard_before_execute(&RollbackPlan::snapshot("snap-123"), false, false)
                .unwrap_err()
                .code(),
            OPS_SNAPSHOT_FAILED
        );
    }
    #[test]
    fn irreversible_requires_explicit_confirmation() {
        let plan = RollbackPlan::irreversible("cannot be undone");
        assert_eq!(
            guard_before_execute(&plan, false, false)
                .unwrap_err()
                .code(),
            OPS_CONFIRMATION_REQUIRED
        );
        assert!(guard_before_execute(&plan, false, true).unwrap().allowed);
    }
    #[test]
    fn compensation_description_is_required() {
        let plan = RollbackPlan {
            class: RollbackClass::Compensation,
            warning: " ".into(),
            snapshot_id: None,
        };
        assert_eq!(
            guard_before_execute(&plan, false, false)
                .unwrap_err()
                .code(),
            OPS_ROLLBACK_UNSUPPORTED
        );
    }
    #[test]
    fn failed_evidence_is_rejected() {
        let plan = RollbackPlan::snapshot("snap-123");
        assert_eq!(
            guard_before_execute_with_evidence(
                &plan,
                SnapshotEvidence::Failed("disk full".into()),
                false
            )
            .unwrap_err()
            .code(),
            OPS_SNAPSHOT_FAILED
        );
    }
    #[test]
    fn sensitive_values_are_redacted_from_serialization() {
        let plan = RollbackPlan::compensation("token=super-secret password=hunter2");
        let json = serde_json::to_string(&plan).unwrap();
        assert!(!json.contains("super-secret"));
        assert!(!json.contains("hunter2"));
        assert!(json.contains("[redacted]"));
    }
}
