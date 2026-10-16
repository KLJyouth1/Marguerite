//! M1.4 security seam: RBAC policy, approval gate, audit store, and event bus.
//!
//! This module is intentionally independent from gateway dispatch. Callers provide
//! the operation callback, so authorization and side effects can be tested in
//! isolation.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::Mutex;

use chrono::Utc;
use serde_json::{json, Value};

use crate::audit_store::{AuditRecord, AuditStore};
use crate::events::{envelope::Envelope, EventBus, EventType};
use crate::guard::gateway::{ApprovalGate, RiskLevel};
use crate::rbac::{
    policy::{Policy, RBAC_FORBIDDEN},
    roles::Actor,
};

pub const OPS_CONFIRM_REQUIRED: &str = "OPS_CONFIRM_REQUIRED";
pub const OPS_CONFIRM_DENIED: &str = "OPS_CONFIRM_DENIED";
pub const OPS_CONFIRM_BUSY: &str = "OPS_CONFIRM_BUSY";
pub const CALLBACK_FAILED: &str = "CALLBACK_FAILED";

/// A stable, structured error returned by the seam. `approval_id` is populated
/// only for `OPS_CONFIRM_REQUIRED` responses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityError {
    code: String,
    message: String,
    approval_id: Option<String>,
}

impl SecurityError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            approval_id: None,
        }
    }

    fn required(method: &str, id: &str) -> Self {
        Self {
            code: OPS_CONFIRM_REQUIRED.to_string(),
            message: format!("method {method} requires approval"),
            approval_id: Some(id.to_string()),
        }
    }

    pub fn code(&self) -> &str {
        &self.code
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn approval_id(&self) -> Option<&str> {
        self.approval_id.as_deref()
    }
}

impl std::fmt::Display for SecurityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for SecurityError {}

#[derive(Clone)]
struct PendingRequest {
    actor: Actor,
    method: String,
    target: String,
}

/// Independent authorization/approval seam. No request/response objects or
/// gateway dispatch details cross this boundary.
pub struct SecuritySeam {
    policy: Policy,
    gate: ApprovalGate,
    audit: AuditStore,
    bus: EventBus,
    pending: Mutex<HashMap<String, PendingRequest>>,
}

impl SecuritySeam {
    pub fn new(policy: Policy, gate: ApprovalGate, audit: AuditStore, bus: EventBus) -> Self {
        Self {
            policy,
            gate,
            audit,
            bus,
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub fn audit_store(&self) -> &AuditStore {
        &self.audit
    }
    pub fn event_bus(&self) -> &EventBus {
        &self.bus
    }
    pub fn approval_gate(&self) -> &ApprovalGate {
        &self.gate
    }

    /// Check RBAC, then either queue a dangerous operation for approval or run
    /// the injected callback. Callback errors are intentionally not persisted in
    /// detail, preventing accidental leakage of keys/secrets into the audit log.
    pub fn request<F, E>(
        &self,
        actor: Actor,
        method: &str,
        target: &str,
        _params: Value,
        callback: F,
    ) -> std::result::Result<Value, SecurityError>
    where
        F: FnOnce() -> std::result::Result<Value, E>,
        E: Display,
    {
        if let Err(forbidden) = self.policy.check(method, actor.role) {
            self.append_audit(&actor, method, target, RBAC_FORBIDDEN);
            return Err(SecurityError::new(RBAC_FORBIDDEN, forbidden.message));
        }

        if is_dangerous(method) {
            let pending = self
                .gate
                .request(method, "{}".to_string(), RiskLevel::High)
                .ok_or_else(|| {
                    SecurityError::new(OPS_CONFIRM_BUSY, "another approval is pending")
                })?;
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(
                    pending.id.clone(),
                    PendingRequest {
                        actor: actor.clone(),
                        method: method.to_string(),
                        target: target.to_string(),
                    },
                );
            self.bus.publish(Envelope::new(
                EventType::Approval,
                json!({
                    "event": "ops.confirm.required",
                    "approval_id": pending.id,
                    "method": method,
                    "target": target,
                    "risk": "high"
                }),
                "gateway",
            ));
            self.append_audit(&actor, method, target, OPS_CONFIRM_REQUIRED);
            return Err(SecurityError::required(method, &pending.id));
        }

        match callback() {
            Ok(value) => {
                self.append_audit(&actor, method, target, "ok");
                Ok(value)
            }
            Err(_) => {
                self.append_audit(&actor, method, target, CALLBACK_FAILED);
                Err(SecurityError::new(CALLBACK_FAILED, "operation failed"))
            }
        }
    }

    /// Verify the exact pending approval and actor/method/target tuple before
    /// consuming the gate. A successful or failed callback is always audited.
    pub fn approve_and_execute<F, E>(
        &self,
        approval_id: &str,
        actor: Actor,
        method: &str,
        target: &str,
        _params: Value,
        callback: F,
    ) -> std::result::Result<Value, SecurityError>
    where
        F: FnOnce() -> std::result::Result<Value, E>,
        E: Display,
    {
        let pending = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(approval_id)
            .cloned();
        let Some(pending) = pending else {
            self.append_audit(&actor, method, target, OPS_CONFIRM_DENIED);
            return Err(SecurityError::new(
                OPS_CONFIRM_DENIED,
                "approval is missing or does not match",
            ));
        };
        if pending.actor != actor
            || pending.method != method
            || pending.target != target
            || self.policy.check(method, actor.role).is_err()
        {
            self.append_audit(&actor, method, target, OPS_CONFIRM_DENIED);
            return Err(SecurityError::new(
                OPS_CONFIRM_DENIED,
                "approval is missing or does not match",
            ));
        }
        if !self.gate.approve(approval_id) {
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(approval_id);
            self.append_audit(&actor, method, target, OPS_CONFIRM_DENIED);
            return Err(SecurityError::new(
                OPS_CONFIRM_DENIED,
                "approval is missing, denied, or expired",
            ));
        }
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(approval_id);

        match callback() {
            Ok(value) => {
                self.append_audit(&actor, method, target, "ok");
                Ok(value)
            }
            Err(_) => {
                self.append_audit(&actor, method, target, CALLBACK_FAILED);
                Err(SecurityError::new(CALLBACK_FAILED, "operation failed"))
            }
        }
    }

    fn append_audit(&self, actor: &Actor, method: &str, target: &str, result: &str) {
        let record = AuditRecord::new(Utc::now().to_rfc3339(), &actor.id, method, target, result);
        if let Err(err) = self.audit.append(record) {
            tracing::error!(error = %err, "security seam audit append failed");
        }
    }
}

fn is_dangerous(method: &str) -> bool {
    matches!(
        method,
        "ops.file.write" | "ops.process.kill" | "team.users.create" | "credentials.write"
    )
}

#[cfg(test)]
#[path = "security_seam_tests.rs"]
mod tests;
