//! Tests for the real SSH read-only executor.
//!
//! Isolation rule: tests inject [`RecordingTransport`] (or a hand-rolled stub),
//! never a [`SystemSshTransport`], so nothing ever contacts a real host or
//! writes to `~/.marguerite`. The pure command builder and address resolver are
//! exercised offline.

use anyhow::{bail, Result};
use std::path::Path;

use crate::deploy::ssh::recording::RecordingTransport;
use crate::deploy::ssh::transport::SshTransport;

use super::command::{command_for, OpKind};
use super::executor::SshOpsExecutor;
use super::resolver::resolve_address;
use super::{HostRef, OpsExecutor, OpsResponse, ReadOnlyRequest};

/// Hand-rolled stub transport: returns a canned stdout, or a canned failure.
struct StubTransport {
    output: String,
    fail: Option<String>,
}

impl StubTransport {
    fn ok(output: &str) -> Self {
        Self { output: output.to_string(), fail: None }
    }
    fn fail(error: &str) -> Self {
        Self { output: String::new(), fail: Some(error.to_string()) }
    }
}

impl SshTransport for StubTransport {
    fn exec(&self, _cmd: &str) -> Result<String> {
        if let Some(e) = &self.fail {
            bail!("{e}");
        }
        Ok(self.output.clone())
    }
    fn upload(&self, _local: &Path, _remote: &Path) -> Result<()> {
        Ok(())
    }
    fn download(&self, _remote: &Path, _local: &Path) -> Result<()> {
        Ok(())
    }
}

fn host(id: &str, address: &str, cred: &str) -> HostRef {
    HostRef::new(id, address, cred).unwrap()
}

// ---- address resolution -----------------------------------------------------

#[test]
fn test_resolve_address_valid() {
    let t = resolve_address("user@host:2222").unwrap();
    assert_eq!(t.user, "user");
    assert_eq!(t.host, "host");
    assert_eq!(t.port, 2222);
    assert_eq!(t.remote_dir, "/");
    assert!(t.identity.is_none());
}

#[test]
fn test_resolve_address_rejects_shell_metachar() {
    for bad in [
        "user@host;id", "user@$(id)", "user@host|id", "host`id`",
        "host>out", "host<in", "user@host run", "user@host\nwhoami",
    ] {
        let err = resolve_address(bad).unwrap_err();
        assert_eq!(err.code(), "OPS_UNSAFE_PATH", "address {bad:?} must be rejected");
    }
}

// ---- executor: status -------------------------------------------------------

#[test]
fn test_executor_status_online() {
    let exec = SshOpsExecutor::new(RecordingTransport::new().with_echo());
    let req = ReadOnlyRequest::new("status").with_host(host("h1", "node-1", "cred-ref"));
    match exec.execute(&req).unwrap() {
        OpsResponse::Status(s) => {
            assert!(s.online);
            assert_eq!(s.host_id, "h1");
            assert!(!s.detail.contains("cred-ref"), "detail must not leak credential_ref");
        }
        other => panic!("expected Status, got {other:?}"),
    }
}

#[test]
fn test_executor_status_offline() {
    let exec = SshOpsExecutor::new(RecordingTransport::new().with_fail("uname"));
    let req = ReadOnlyRequest::new("status").with_host(host("h1", "node-1", "super-secret-ref"));
    match exec.execute(&req).unwrap() {
        OpsResponse::Status(s) => {
            assert!(!s.online);
            assert!(
                !s.detail.contains("super-secret-ref"),
                "offline detail must not leak credentials: {}",
                s.detail
            );
        }
        other => panic!("expected Status, got {other:?}"),
    }
}

// ---- executor: limits & unsafe paths ----------------------------------------

const LISTING: &str = "total 12\n\
-rw-r--r-- 1 root root 100 Jan 1 00:00 app.log\n\
-rw-r--r-- 1 root root 200 Jan 1 00:00 data.txt\n";

#[test]
fn test_executor_list_files_limit() {
    let exec = SshOpsExecutor::new(StubTransport::ok(LISTING));
    let req = ReadOnlyRequest::new("list_files")
        .with_host(host("h1", "node-1", "c"))
        .with_path("logs")
        .with_limits(1, 4096);
    let err = exec.execute(&req).unwrap_err();
    assert_eq!(err.code(), "OPS_INVALID_REQUEST");
}

#[test]
fn test_executor_tail_logs_rejects_unsafe_path() {
    let exec = SshOpsExecutor::new(RecordingTransport::new().with_echo());
    for bad in ["../../etc/passwd", "/etc/passwd"] {
        let req = ReadOnlyRequest::new("tail_logs")
            .with_host(host("h1", "node-1", "c"))
            .with_path(bad);
        let err = exec.execute(&req).unwrap_err();
        assert_eq!(err.code(), "OPS_UNSAFE_PATH", "path {bad:?} must be rejected");
    }
}

// ---- executor: write / unknown ops ------------------------------------------

#[test]
fn test_executor_forbids_write_ops() {
    let exec = SshOpsExecutor::new(RecordingTransport::new().with_echo());
    for op in ["delete_file", "write_file", "execute", "restart", "shutdown"] {
        let req = ReadOnlyRequest::new(op).with_host(host("h1", "node-1", "c"));
        let err = exec.execute(&req).unwrap_err();
        assert_eq!(err.code(), "OPS_FORBIDDEN_WRITE", "op {op} must be forbidden");
    }
}

#[test]
fn test_executor_unknown_op_rejected() {
    let exec = SshOpsExecutor::new(RecordingTransport::new().with_echo());
    let req = ReadOnlyRequest::new("format_disk").with_host(host("h1", "node-1", "c"));
    let err = exec.execute(&req).unwrap_err();
    assert_eq!(err.code(), "OPS_INVALID_REQUEST");
}

// ---- credential / secret redaction ------------------------------------------

#[test]
fn test_executor_error_redacts_credentials() {
    let cred = "super-secret-ref";
    let exec = SshOpsExecutor::new(StubTransport::fail(&format!(
        "auth denied for {cred} with token=abc123secret"
    )));
    let req = ReadOnlyRequest::new("list_files")
        .with_host(host("h1", "node-1", cred))
        .with_path("logs");
    let err = exec.execute(&req).unwrap_err();
    assert!(
        !err.message.contains(cred),
        "error message must not leak credential_ref value: {}",
        err.message
    );
    assert!(!err.message.contains("abc123secret"), "token value must be redacted: {}", err.message);
}

// ---- command builder: injection defense & mapping ---------------------------

#[test]
fn test_command_for_status_maps_to_probe() {
    let req = ReadOnlyRequest::new("status").with_host(host("h1", "node-1", "c"));
    let shell = command_for(&req).unwrap();
    assert_eq!(shell.op, OpKind::Status);
    assert_eq!(shell.cmd, "uname -a && id && whoami");
}

#[test]
fn test_command_for_rejects_unsafe_address() {
    let req = ReadOnlyRequest::new("status").with_host(host("h1", "node-1;id", "c"));
    let err = command_for(&req).unwrap_err();
    assert_eq!(err.code(), "OPS_UNSAFE_PATH");
}

#[test]
fn test_command_for_rejects_unsafe_path() {
    let req = ReadOnlyRequest::new("list_files")
        .with_host(host("h1", "node-1", "c"))
        .with_path("logs;rm -rf /");
    let err = command_for(&req).unwrap_err();
    assert_eq!(err.code(), "OPS_UNSAFE_PATH");
}

#[test]
fn test_command_for_list_hosts_is_local_only() {
    let req = ReadOnlyRequest::new("list_hosts");
    let shell = command_for(&req).unwrap();
    assert_eq!(shell.op, OpKind::ListHosts);
    assert!(shell.cmd.is_empty(), "list_hosts must not build a remote command");
}

#[test]
fn test_executor_list_hosts_returns_empty_without_ssh() {
    let exec = SshOpsExecutor::new(RecordingTransport::new().with_echo());
    let req = ReadOnlyRequest::new("list_hosts");
    match exec.execute(&req).unwrap() {
        OpsResponse::Hosts(v) => assert!(v.is_empty()),
        other => panic!("expected Hosts, got {other:?}"),
    }
}
