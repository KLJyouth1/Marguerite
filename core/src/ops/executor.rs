//! Real SSH-backed read-only operations executor.
//!
//! [`SshOpsExecutor`] implements [`OpsExecutor`] by turning a validated
//! [`ReadOnlyRequest`] into a read-only remote shell command and running it
//! over an injected [`SshTransport`]. It never performs writes, never accepts
//! an unsafe path, and never echoes credentials.
//!
//! The transport is injected so unit tests use a recording/stub transport (no
//! real network) while production injects a system SSH transport. The pure
//! command building and output parsing live in [`crate::ops::command`].

use crate::deploy::ssh::SshTransport;
use crate::ops::command::{command_for, is_status, parse_response, sanitize};
use crate::ops::{HostRef, HostStatus, OpsError, OpsExecutor, OpsResponse, ReadOnlyRequest};

/// An executor that runs read-only operations over an SSH transport.
///
/// Carries a host registry (the configured `ops.hosts`) so `list_hosts` returns
/// the real configured hosts instead of an empty vector; per-host operations
/// (`status/list_files/tail_logs/inspect_process`) still run through the single
/// injected transport. The registry is the single source of truth for which
/// hosts are reachable — it is never populated from transport output.
pub struct SshOpsExecutor<T: SshTransport> {
    transport: T,
    hosts: Vec<HostRef>,
}

impl<T: SshTransport> SshOpsExecutor<T> {
    /// Build an executor with an empty host registry (backward compatible: all
    /// call sites that only need per-host ops keep working; `list_hosts`
    /// returns an empty list until a registry is injected).
    pub fn new(transport: T) -> Self {
        Self { transport, hosts: Vec::new() }
    }

    /// Build an executor backed by a configured host registry.
    ///
    /// Production wiring uses this from `Config::ops.hosts` so `list_hosts`
    /// reports real configured hosts rather than an empty vector.
    pub fn with_hosts(transport: T, hosts: Vec<HostRef>) -> Self {
        Self { transport, hosts }
    }

    fn execute_each(&self, request: &ReadOnlyRequest) -> Result<OpsResponse, OpsError> {
        let host = request.host.as_ref().ok_or_else(|| {
            OpsError::invalid("host is required for this operation")
        })?;
        let shell = command_for(request)?;
        match self.transport.exec(&shell.cmd) {
            Ok(output) => parse_response(&shell.op, &output, host, request),
            Err(e) => {
                // A status probe failure is not a hard error: it reports the
                // host as offline with a redacted detail. All other read-only
                // operations propagate the failure as a structured error.
                if is_status(&shell.op) {
                    Ok(OpsResponse::Status(HostStatus {
                        host_id: host.id.clone(),
                        online: false,
                        detail: sanitize(&e.to_string()),
                    }))
                } else {
                    Err(OpsError::invalid(sanitize(&e.to_string())))
                }
            }
        }
    }
}

impl<T: SshTransport> OpsExecutor for SshOpsExecutor<T> {
    fn execute(&self, request: &ReadOnlyRequest) -> Result<OpsResponse, OpsError> {
        request.validate()?;
        match request.operation.trim().to_ascii_lowercase().as_str() {
            "list_hosts" => Ok(OpsResponse::Hosts(self.hosts.clone())),
            "status" => self.execute_each(request),
            "list_files" => self.execute_each(request),
            "tail_logs" => self.execute_each(request),
            "inspect_process" => self.execute_each(request),
            // validate() already rejects these; kept for exhaustiveness.
            "delete_file" | "write_file" | "execute" | "restart" | "shutdown" => {
                Err(OpsError::forbidden_write())
            }
            _ => Err(OpsError::invalid("unknown operation")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::ssh::RecordingTransport;
    use crate::ops::{HostRef, ReadOnlyRequest};

    fn host(id: &str, address: &str, cred: &str) -> HostRef {
        HostRef::new(id, address, cred).unwrap()
    }

    #[test]
    fn executor_forbids_write_ops() {
        let ex = SshOpsExecutor::new(RecordingTransport::new().with_echo());
        for op in ["delete_file", "write_file", "execute", "restart", "shutdown"] {
            let req = ReadOnlyRequest::new(op).with_host(host("h", "node-1", "c"));
            assert_eq!(ex.execute(&req).unwrap_err().code(), "OPS_FORBIDDEN_WRITE", "forbid {op}");
        }
    }

    #[test]
    fn executor_status_online() {
        let ex = SshOpsExecutor::new(RecordingTransport::new().with_echo());
        let req = ReadOnlyRequest::new("status").with_host(host("h1", "user@host:22", "cred-ref"));
        match ex.execute(&req).unwrap() {
            OpsResponse::Status(s) => {
                assert!(s.online);
                assert_eq!(s.host_id, "h1");
                assert!(!s.detail.contains("cred-ref"));
            }
            other => panic!("expected Status, got {other:?}"),
        }
    }

    #[test]
    fn executor_status_offline_no_leak() {
        let ex = SshOpsExecutor::new(RecordingTransport::new().with_fail("uname"));
        let req = ReadOnlyRequest::new("status").with_host(host("h1", "node-1", "super-secret"));
        match ex.execute(&req).unwrap() {
            OpsResponse::Status(s) => {
                assert!(!s.online);
                assert!(!s.detail.contains("super-secret"));
                assert!(!s.detail.contains("credential_ref"));
            }
            other => panic!("expected Status, got {other:?}"),
        }
    }

    #[test]
    fn executor_tail_logs_rejects_unsafe_path() {
        let ex = SshOpsExecutor::new(RecordingTransport::new().with_echo());
        for bad in ["../../etc/passwd", "/etc/passwd"] {
            let req = ReadOnlyRequest::new("tail_logs").with_host(host("h", "node-1", "c")).with_path(bad);
            assert_eq!(ex.execute(&req).unwrap_err().code(), "OPS_UNSAFE_PATH", "reject {bad}");
        }
    }

    #[test]
    fn executor_unknown_op_rejected() {
        let ex = SshOpsExecutor::new(RecordingTransport::new().with_echo());
        let req = ReadOnlyRequest::new("format_disk").with_host(host("h", "node-1", "c"));
        assert_eq!(ex.execute(&req).unwrap_err().code(), "OPS_INVALID_REQUEST");
    }

    #[test]
    fn executor_list_hosts_empty() {
        let ex = SshOpsExecutor::new(RecordingTransport::new().with_echo());
        let req = ReadOnlyRequest::new("list_hosts");
        assert!(matches!(ex.execute(&req).unwrap(), OpsResponse::Hosts(v) if v.is_empty()));
    }

    #[test]
    fn executor_rejects_unsafe_address_before_command() {
        let ex = SshOpsExecutor::new(RecordingTransport::new().with_echo());
        let req = ReadOnlyRequest::new("status").with_host(host("h", "node-1;id", "c"));
        assert_eq!(ex.execute(&req).unwrap_err().code(), "OPS_UNSAFE_PATH");
    }
}
