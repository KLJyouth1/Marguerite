//! Pure command-building and output-parsing for the read-only SSH executor.
//!
//! This layer is transport-free: it turns a validated [`ReadOnlyRequest`] into
//! a remote shell command and parses the resulting output, with injection
//! defence and credential redaction applied. It never performs I/O, so it is
//! fully unit-testable offline.

use crate::ops::{
    limit_files, tail_content, HostRef, HostStatus, OpsError, OpsResponse, ReadOnlyRequest,
};
use crate::ops::resolver::reject_shell_metachar;

/// The kind of operation a shell command will service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpKind {
    ListHosts,
    Status,
    Files,
    Logs,
    Process,
}

/// A validated, ready-to-run remote shell command for a read-only operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpsShellCommand {
    pub op: OpKind,
    pub cmd: String,
}

/// Translate a validated request into a remote command line.
///
/// Pure (no transport I/O): the command shape and injection defence are
/// unit-tested offline. Defence in depth: the host address is user-controlled
/// and (in the general case) forwarded to a shell, so metachars are rejected.
pub fn command_for(request: &ReadOnlyRequest) -> Result<OpsShellCommand, OpsError> {
    if let Some(host) = &request.host {
        if reject_shell_metachar(host.address.as_str()) {
            return Err(OpsError::unsafe_path(
                "host address contains unsafe shell metacharacter",
            ));
        }
    }
    let op = request.operation.trim().to_ascii_lowercase();
    match op.as_str() {
        "status" => Ok(OpsShellCommand {
            op: OpKind::Status,
            cmd: "uname -a && id && whoami".to_string(),
        }),
        "list_hosts" => Ok(OpsShellCommand {
            op: OpKind::ListHosts,
            cmd: String::new(),
        }),
        "list_files" => {
            let path = request
                .path
                .as_deref()
                .ok_or_else(|| OpsError::invalid("path is required"))?;
            reject_unsafe_path(path)?;
            Ok(OpsShellCommand {
                op: OpKind::Files,
                cmd: format!("ls -l -- {}", shell_quote(path)),
            })
        }
        "tail_logs" => {
            let path = request
                .path
                .as_deref()
                .ok_or_else(|| OpsError::invalid("path is required"))?;
            reject_unsafe_path(path)?;
            let n = request.max_lines.unwrap_or(200);
            Ok(OpsShellCommand {
                op: OpKind::Logs,
                cmd: format!("tail -n {} -- {}", n, shell_quote(path)),
            })
        }
        "inspect_process" => {
            let name = request
                .path
                .as_deref()
                .ok_or_else(|| OpsError::invalid("process name is required"))?;
            reject_unsafe_arg(name)?;
            Ok(OpsShellCommand {
                op: OpKind::Process,
                cmd: format!("pgrep -af {}", shell_quote(name)),
            })
        }
        _ => Err(OpsError::invalid("unknown operation")),
    }
}

/// Parse a command's stdout into a [`OpsResponse`]. Limit violations surface as
/// structured errors, never silent truncation.
pub fn parse_response(
    op: &OpKind,
    output: &str,
    host: &HostRef,
    request: &ReadOnlyRequest,
) -> Result<OpsResponse, OpsError> {
    match op {
        OpKind::Status => Ok(OpsResponse::Status(HostStatus {
            host_id: host.id.clone(),
            online: true,
            detail: sanitize(&first_line(output)),
        })),
        OpKind::Files => {
            let entries = parse_ls(output);
            let max_entries = request.max_lines.unwrap_or(usize::MAX);
            let max_bytes = request.max_bytes.unwrap_or(10 * 1024 * 1024);
            limit_files(entries, max_entries, max_bytes).map(OpsResponse::Files)
        }
        OpKind::Logs => {
            let max_lines = request.max_lines.unwrap_or(200);
            let max_bytes = request.max_bytes.unwrap_or(1024 * 1024);
            tail_content(output, max_lines, max_bytes).map(OpsResponse::Logs)
        }
        OpKind::Process => Ok(OpsResponse::Process(sanitize(output))),
        OpKind::ListHosts => Ok(OpsResponse::Hosts(Vec::new())),
    }
}

fn parse_ls(output: &str) -> Vec<crate::ops::FileEntry> {
    output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let tokens: Vec<&str> = line.split_whitespace().collect();
            if tokens.len() < 9 {
                return None;
            }
            let size = tokens[4].parse::<u64>().unwrap_or(0);
            let path = tokens.get(8..).map(|slice| slice.join(" ")).unwrap_or_default();
            if path.is_empty() {
                None
            } else {
                Some(crate::ops::FileEntry::new(path, size))
            }
        })
        .collect()
}

fn first_line(output: &str) -> String {
    output.lines().next().unwrap_or("").to_string()
}

fn reject_unsafe_path(path: &str) -> Result<(), OpsError> {
    let p = path.trim();
    if p.starts_with('/')
        || p.starts_with("\\\\")
        || p.starts_with('~')
        || p.contains("..")
        || p.chars().any(|c| [';', '&', '|', '`', '$', '>', '<', '\n', '\r'].contains(&c))
    {
        return Err(OpsError::unsafe_path(path));
    }
    Ok(())
}

fn reject_unsafe_arg(arg: &str) -> Result<(), OpsError> {
    let a = arg.trim();
    if a.is_empty()
        || a.len() > 128
        || a.chars().any(|c| [';', '&', '|', '`', '$', '>', '<', '\n', '\r'].contains(&c))
    {
        return Err(OpsError::invalid("unsafe argument"));
    }
    Ok(())
}

fn shell_quote(arg: &str) -> String {
    let escaped = arg.replace('\\', "\\\\").replace('\'', "'\\''");
    format!("'{}'", escaped)
}

/// True if the op is the read-only host-status probe.
pub fn is_status(op: &OpKind) -> bool {
    matches!(op, OpKind::Status)
}

/// Redact sensitive tokens from any value that leaves via a response, status
/// detail, or error message. Never emit passwords / tokens / keys.
pub fn sanitize(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if lower.contains("token=")
        || lower.contains("password=")
        || lower.contains("secret")
        || lower.contains("credential_ref")
        || lower.contains("auth denied")
    {
        "[redacted]".to_string()
    } else {
        value.to_string()
    }
}
