//! Transport-neutral, read-only operations contract for host administration.
//! This layer validates requests and exposes an injected executor; it never runs commands.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

pub mod command;
pub mod executor;
pub mod resolver;
mod files;
mod host;
mod logs;
mod terminal;
mod rollback_policy;

pub use command::{command_for, OpKind, OpsShellCommand};
pub use executor::SshOpsExecutor;
pub use resolver::{build_system_transport, reject_shell_metachar, resolve_address};
pub use files::{limit_files, FileEntry};
pub use host::{HostRef, HostStatus};
pub use logs::tail_content;
pub use terminal::inspect_process_name;
pub use rollback_policy::{classify_operation, guard_before_execute, guard_before_execute_with_evidence, RollbackClass, RollbackDecision, RollbackError, RollbackPlan, SnapshotEvidence};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod executor_tests;

pub const OPS_INVALID_REQUEST: &str = "OPS_INVALID_REQUEST";
pub const OPS_FORBIDDEN_WRITE: &str = "OPS_FORBIDDEN_WRITE";
pub const OPS_UNSAFE_PATH: &str = "OPS_UNSAFE_PATH";
pub const OPS_CREDENTIAL_REQUIRED: &str = "OPS_CREDENTIAL_REQUIRED";
pub const OPS_SNAPSHOT_REQUIRED: &str = "OPS_SNAPSHOT_REQUIRED";
pub const OPS_SNAPSHOT_FAILED: &str = "OPS_SNAPSHOT_FAILED";
pub const OPS_CONFIRMATION_REQUIRED: &str = "OPS_CONFIRMATION_REQUIRED";
pub const OPS_ROLLBACK_UNSUPPORTED: &str = "OPS_ROLLBACK_UNSUPPORTED";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpsError {
    pub code: String,
    pub message: String,
}
impl OpsError {
    pub fn invalid(message: impl Into<String>) -> Self { Self { code: OPS_INVALID_REQUEST.into(), message: message.into() } }
    pub fn forbidden_write() -> Self { Self { code: OPS_FORBIDDEN_WRITE.into(), message: "write operations are not permitted".into() } }
    pub fn unsafe_path(path: impl Into<String>) -> Self { Self { code: OPS_UNSAFE_PATH.into(), message: format!("unsafe path: {}", path.into()) } }
    pub fn credential_required() -> Self { Self { code: OPS_CREDENTIAL_REQUIRED.into(), message: "credential_ref is required".into() } }
    pub fn code(&self) -> &str { &self.code }
}
impl fmt::Display for OpsError { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}: {}", self.code, self.message) } }
impl std::error::Error for OpsError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadOnlyRequest {
    pub operation: String,
    pub host: Option<HostRef>,
    pub path: Option<String>,
    pub max_lines: Option<usize>,
    pub max_bytes: Option<usize>,
}
impl ReadOnlyRequest {
    pub fn new(operation: impl Into<String>) -> Self { Self { operation: operation.into(), host: None, path: None, max_lines: None, max_bytes: None } }
    pub fn with_host(mut self, host: HostRef) -> Self { self.host = Some(host); self }
    pub fn with_path(mut self, path: impl Into<String>) -> Self { self.path = Some(path.into()); self }
    pub fn with_limits(mut self, lines: usize, bytes: usize) -> Self { self.max_lines = Some(lines); self.max_bytes = Some(bytes); self }
    pub fn validate(&self) -> Result<(), OpsError> {
        let op = self.operation.trim().to_ascii_lowercase();
        if op.is_empty() { return Err(OpsError::invalid("operation is required")); }
        if let Some(path) = &self.path {
            let p = Path::new(path);
            if p.is_absolute() || path.starts_with('/') || path.starts_with("\\\\") || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) { return Err(OpsError::unsafe_path(path)); }
        }
        if ["delete_file", "write_file", "execute", "restart", "shutdown"].contains(&op.as_str()) { return Err(OpsError::forbidden_write()); }
        if !["list_hosts", "status", "list_files", "tail_logs", "inspect_process"].contains(&op.as_str()) { return Err(OpsError::invalid("unknown operation")); }
        if self.max_lines == Some(0) || self.max_bytes == Some(0) { return Err(OpsError::invalid("limits must be positive")); }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum OpsResponse { Hosts(Vec<HostRef>), Status(HostStatus), Files(Vec<FileEntry>), Logs(String), Process(String) }

pub trait OpsExecutor { fn execute(&self, request: &ReadOnlyRequest) -> Result<OpsResponse, OpsError>; }

pub struct NullExecutor;
impl OpsExecutor for NullExecutor {
    fn execute(&self, request: &ReadOnlyRequest) -> Result<OpsResponse, OpsError> {
        request.validate()?;
        Err(OpsError::credential_required())
    }
}
