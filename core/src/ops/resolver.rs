//! Host address to SSH target resolution with command-injection defence.
//!
//! Pure, offline-testable. Validates that a host address is a safe
//! `[user@]host[:port]` shape and rejects any shell metacharacter, so the
//! resulting [`SshTarget`] can never inject a command into the remote shell.

use crate::deploy::ssh::target::SshTarget;
use crate::deploy::ssh::transport::SystemSshTransport;
use crate::ops::OpsError;

/// Characters that must never appear in a host address.
const UNSAFE_CHARS: &[char] = &[
    ';', '&', '|', '$', '`', '>', '<', '\n', '\r', ' ', '\t',
];

/// True if the address contains a character that could escape a quoted shell
/// argument and inject a command.
pub fn reject_shell_metachar(address: &str) -> bool {
    address
        .chars()
        .any(|c| UNSAFE_CHARS.contains(&c))
}

/// Resolve a `HostRef.address` (`[user@]host[:port]`) into an [`SshTarget`].
pub fn resolve_address(address: &str) -> Result<SshTarget, OpsError> {
    let raw = address.trim();
    if raw.is_empty() {
        return Err(OpsError::unsafe_path("host address is empty"));
    }
    if reject_shell_metachar(raw) {
        return Err(OpsError::unsafe_path(
            "host address contains unsafe shell metacharacter",
        ));
    }
    let spec = crate::deploy::ssh::parse_hostspec(raw)
        .map_err(|e| OpsError::unsafe_path(format!("invalid host address: {e}")))?;
    Ok(spec.into_ssh_target("/"))
}

/// Build a system SSH transport for a resolved target.
///
/// Production path: shells out to `ssh` (key-based, no password prompt).
/// Unit tests never call this; they inject a recording/stub transport.
pub fn build_system_transport(target: &SshTarget) -> SystemSshTransport {
    SystemSshTransport::new(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_full_address() {
        let t = resolve_address("deploy@192.168.1.10:2222").unwrap();
        assert_eq!(t.user, "deploy");
        assert_eq!(t.host, "192.168.1.10");
        assert_eq!(t.port, 2222);
        assert_eq!(t.remote_dir, "/");
        assert!(t.identity.is_none());
    }

    #[test]
    fn resolves_defaults() {
        let t = resolve_address("host").unwrap();
        assert_eq!(t.user, "root");
        assert_eq!(t.host, "host");
        assert_eq!(t.port, 22);
    }

    #[test]
    fn rejects_semicolon_injection() {
        assert_eq!(resolve_address("host; rm -rf /").unwrap_err().code(), "OPS_UNSAFE_PATH");
    }

    #[test]
    fn rejects_shell_metacharacters() {
        for addr in [
            "host | id", "host$(whoami)", "host`id`", "host>out", "host&echo", "host pwd",
        ] {
            assert_eq!(
                resolve_address(addr).unwrap_err().code(),
                "OPS_UNSAFE_PATH",
                "should reject: {addr}"
            );
        }
    }

    #[test]
    fn rejects_empty_address() {
        assert_eq!(resolve_address("").unwrap_err().code(), "OPS_UNSAFE_PATH");
        assert_eq!(resolve_address("   ").unwrap_err().code(), "OPS_UNSAFE_PATH");
    }

    #[test]
    fn rejects_malformed_shapes() {
        for addr in ["user@", "@host", "host:", ":22", "a@b@c", "host:0", "host:99999"] {
            assert_eq!(
                resolve_address(addr).unwrap_err().code(),
                "OPS_UNSAFE_PATH",
                "should reject malformed: {addr}"
            );
        }
    }

    #[test]
    fn build_transport_is_system_backed() {
        let t = resolve_address("root@host:22").unwrap();
        let _transport = build_system_transport(&t);
        let _: SshTarget = t;
    }

    #[test]
    fn reject_flag_matches() {
        assert!(reject_shell_metachar("host;id"));
        assert!(reject_shell_metachar("host|id"));
        assert!(!reject_shell_metachar("user@host:22"));
    }
}
