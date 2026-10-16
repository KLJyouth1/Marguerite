//! Gateway adapter for transport-neutral, read-only operations.
//!
//! This module only translates gateway method names and JSON parameters to the
//! `ops` contract. It never executes host commands; callers inject an
//! [`OpsExecutor`] implementation.

use serde_json::{Map, Value};

use crate::gateway::protocol::RpcError;
use crate::ops::{OpsError, OpsExecutor, OpsResponse, ReadOnlyRequest};

const METHODS: &[(&str, &str)] = &[
    ("ops.list_hosts", "list_hosts"),
    ("ops.status", "status"),
    ("ops.list_files", "list_files"),
    ("ops.tail_logs", "tail_logs"),
    ("ops.inspect_process", "inspect_process"),
];

/// Dispatch one supported `ops.*` method through the injected executor.
///
/// `None` means this is not an ops method and lets the outer gateway continue
/// routing. Errors preserve the stable `OPS_*` code and never echo credentials.
pub fn dispatch_with_executor<E: OpsExecutor + ?Sized>(
    method: &str,
    params: &Value,
    executor: &E,
) -> Option<Result<Value, RpcError>> {
    let operation = METHODS.iter().find(|(name, _)| *name == method).map(|(_, op)| *op);
    let operation = match operation {
        Some(operation) => operation,
        None if method.starts_with("ops.") => {
            let requested = method.trim_start_matches("ops.");
            let error = if ["delete_file", "write_file", "execute", "restart", "shutdown"].contains(&requested) {
                OpsError::forbidden_write()
            } else {
                OpsError::invalid("unknown operation")
            };
            return Some(Err(to_rpc_error(error)));
        }
        None => return None,
    };

    let request = match request_from_params(operation, params) {
        Ok(request) => request,
        Err(error) => return Some(Err(to_rpc_error(error))),
    };
    Some(executor.execute(&request).map(serialize_response).map_err(to_rpc_error))
}

/// Dispatch with the default no-side-effect executor. Production wiring can
/// replace this call with an executor-backed adapter without changing routing.
pub fn dispatch(method: &str, params: &Value) -> Option<Result<Value, RpcError>> {
    dispatch_with_executor(method, params, &crate::ops::NullExecutor)
}

/// Dispatch ops.* methods through a config-backed executor.
///
/// Production path:
/// - `list_hosts` returns the **configured** host registry (`config.ops.hosts`)
///   instead of the empty default — so the panel reports real hosts.
/// - per-host read-only ops build a real [`SystemSshTransport`] from the
///   request's validated host address (never a wrong/default host), then run the
///   read-only command over it. Both branches go through the same executor, so
///   [`ReadOnlyRequest::validate`] (unsafe-path rejection), the write-op block,
///   and [`redact_credentials`] (no `credential_ref`/secret leak) all apply.
///
/// The transport is only built from an address that already passed
/// [`crate::ops::resolver::reject_shell_metachar`] inside `resolve_address`, so
/// a shell-metachar injection is impossible here. If the host address is absent
/// for a per-host op, the executor returns the standard structured error instead
/// of ssh-ing anywhere.
pub fn dispatch_with_config(
    method: &str,
    params: &Value,
    config: &crate::persistence::config::Config,
) -> Option<Result<Value, RpcError>> {
    let operation = METHODS.iter().find(|(name, _)| *name == method).map(|(_, op)| *op);
    if operation.is_none() && !method.starts_with("ops.") {
        return None;
    }

    let hosts = config.ops.hosts.clone();

    // `list_hosts` needs no transport; report the configured registry.
    if operation == Some("list_hosts") {
        let executor = crate::ops::SshOpsExecutor::with_hosts(crate::ops::build_system_transport(&default_target()), hosts);
        return dispatch_with_executor(method, params, &executor);
    }

    // Per-host read-only ops: resolve the target host from params, build a real
    // system transport for *that* host. Unresolvable/absent host → structured
    // error (never ssh to a wrong host).
    let address = params
        .get("host")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let target = address.and_then(|a| crate::ops::resolver::resolve_address(a).ok());
    let executor = crate::ops::build_system_transport(&target.unwrap_or_else(default_target));
    let executor = crate::ops::SshOpsExecutor::with_hosts(executor, hosts);
    dispatch_with_executor(method, params, &executor)
}

/// Fallback `SshTarget` (localhost) used only to satisfy the transport
/// constructor for operations that never need a remote host (i.e. `list_hosts`).
/// Per-host ops resolve a real target from params; they never use this default.
fn default_target() -> crate::deploy::ssh::target::SshTarget {
    crate::deploy::ssh::parse_hostspec("root@localhost:22")
        .map(|spec| spec.into_ssh_target("/"))
        .unwrap_or_else(|_| crate::deploy::ssh::target::SshTarget {
            host: "localhost".into(),
            port: 22,
            user: "root".into(),
            remote_dir: "/".into(),
            identity: None,
        })
}

fn request_from_params(operation: &str, params: &Value) -> Result<ReadOnlyRequest, OpsError> {
    let object = params
        .as_object()
        .ok_or_else(|| OpsError::invalid("params must be an object"))?;
    let mut input = Map::with_capacity(object.len() + 1);
    for (key, value) in object {
        // The operation is selected by the authenticated gateway method, not
        // by user input, so a conflicting field is rejected rather than used.
        if key == "operation" {
            return Err(OpsError::invalid("operation is controlled by gateway method"));
        }
        input.insert(key.clone(), value.clone());
    }
    input.insert("operation".into(), Value::String(operation.into()));
    let request: ReadOnlyRequest = serde_json::from_value(Value::Object(input))
        .map_err(|_| OpsError::invalid("invalid ops request"))?;
    request.validate()?;
    Ok(request)
}

fn serialize_response(response: OpsResponse) -> Value {
    // HostRef intentionally contains credential_ref for executor use. Never
    // expose that secret reference over the gateway response boundary.
    let mut value = serde_json::to_value(response).unwrap_or(Value::Null);
    redact_credentials(&mut value);
    value
}

fn redact_credentials(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("credential_ref");
            for child in map.values_mut() {
                redact_credentials(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_credentials(item);
            }
        }
        _ => {}
    }
}

fn to_rpc_error(error: OpsError) -> RpcError {
    let message = if error.code == crate::ops::OPS_CREDENTIAL_REQUIRED {
        // Keep credential field names and references out of transport errors.
        "credentials are required".to_string()
    } else {
        error.message
    };
    RpcError::new(error.code, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBus;
    use crate::gateway::methods::{dispatch_sync, GatewayContext};
    use crate::ops::{HostRef, HostStatus};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    struct RecordingExecutor {
        seen: Mutex<Vec<ReadOnlyRequest>>,
        response: Result<OpsResponse, OpsError>,
    }

    impl OpsExecutor for RecordingExecutor {
        fn execute(&self, request: &ReadOnlyRequest) -> Result<OpsResponse, OpsError> {
            self.seen.lock().unwrap().push(request.clone());
            self.response.clone()
        }
    }

    fn executor() -> RecordingExecutor {
        RecordingExecutor {
            seen: Mutex::new(Vec::new()),
            response: Ok(OpsResponse::Status(HostStatus {
                host_id: "h1".into(),
                online: true,
                detail: "ok".into(),
            })),
        }
    }

    fn gateway_context() -> GatewayContext {
        GatewayContext {
            config: crate::persistence::config::Config::default(),
            token: "test-token".into(),
            started_at: Instant::now(),
            bus: Arc::new(EventBus::default()),
        }
    }

    fn gateway_context_with(config: crate::persistence::config::Config) -> GatewayContext {
        GatewayContext {
            config,
            token: "test-token".into(),
            started_at: Instant::now(),
            bus: Arc::new(EventBus::default()),
        }
    }

    #[test]
    fn gateway_sync_dispatch_reaches_ops_adapter() {
        let context = gateway_context();
        // list_hosts is now production-wired: it returns the configured host
        // registry (empty config → empty list, no error) instead of an
        // OPS_CREDENTIAL_REQUIRED failure from the old NullExecutor.
        let list_hosts = dispatch_sync(&context, "ops.list_hosts", &serde_json::json!({})).unwrap();
        assert_eq!(list_hosts["kind"], "Hosts");
        assert!(list_hosts["data"].as_array().unwrap().is_empty());

        // Per-host read-only ops require an explicit host; with none supplied
        // they report a structured (non-credential-leaking) error.
        for method in [
            "ops.status",
            "ops.list_files",
            "ops.tail_logs",
            "ops.inspect_process",
        ] {
            let error = dispatch_sync(&context, method, &serde_json::json!({})).unwrap_err();
            assert!(error.code.starts_with("OPS_"), "unexpected code for {method}: {}", error.code);
            assert!(!error.message.contains("credential_ref"));
        }
    }

    #[test]
    fn config_backed_list_hosts_returns_configured_hosts() {
        // M3 production wiring: `list_hosts` must report the hosts configured
        // in `ops.hosts`, not an empty vector (the old NullExecutor/empty-trait
        // behavior). This is the concrete gap the register flags as
        // "list_hosts 当前返回空向量".
        let mut config = crate::persistence::config::Config::default();
        config.ops.hosts = vec![
            crate::ops::HostRef::new("node-1", "deploy@10.0.0.5:22", "cred-ref-1").unwrap(),
            crate::ops::HostRef::new("node-2", "10.0.0.6", "cred-ref-2").unwrap(),
        ];
        let context = gateway_context_with(config);
        let result = dispatch_sync(&context, "ops.list_hosts", &serde_json::json!({})).unwrap();
        assert_eq!(result["kind"], "Hosts");
        let hosts = result["data"].as_array().unwrap();
        assert_eq!(hosts.len(), 2);
        // credential_ref 绝不跨响应边界泄露
        assert!(!result.to_string().contains("cred-ref"));
        assert!(!result.to_string().contains("credential_ref"));
    }

    #[test]
    fn all_read_only_methods_reach_injected_executor() {
        for method in [
            "ops.list_hosts",
            "ops.status",
            "ops.list_files",
            "ops.tail_logs",
            "ops.inspect_process",
        ] {
            let executor = executor();
            let result = dispatch_with_executor(method, &serde_json::json!({}), &executor)
                .expect("ops method should be selected")
                .expect("executor should succeed");
            assert_eq!(result["kind"], "Status");
            assert_eq!(executor.seen.lock().unwrap()[0].operation, method.trim_start_matches("ops."));
        }
    }

    #[test]
    fn write_and_unknown_methods_are_structured_errors() {
        let executor = executor();
        let write = dispatch_with_executor("ops.delete_file", &serde_json::json!({}), &executor)
            .unwrap()
            .unwrap_err();
        assert_eq!(write.code, "OPS_FORBIDDEN_WRITE");
        let unknown = dispatch_with_executor("ops.nope", &serde_json::json!({}), &executor)
            .unwrap()
            .unwrap_err();
        assert_eq!(unknown.code, "OPS_INVALID_REQUEST");
    }

    #[test]
    fn unsafe_path_and_conflicting_operation_are_rejected() {
        let executor = executor();
        let unsafe_result = dispatch_with_executor(
            "ops.list_files",
            &serde_json::json!({"path": "../secret"}),
            &executor,
        )
        .unwrap()
        .unwrap_err();
        assert_eq!(unsafe_result.code, "OPS_UNSAFE_PATH");
        let conflict = dispatch_with_executor(
            "ops.status",
            &serde_json::json!({"operation": "delete_file"}),
            &executor,
        )
        .unwrap()
        .unwrap_err();
        assert_eq!(conflict.code, "OPS_INVALID_REQUEST");
    }

    #[test]
    fn credential_ref_is_not_exposed_and_executor_codes_survive() {
        let executor = RecordingExecutor {
            seen: Mutex::new(Vec::new()),
            response: Ok(OpsResponse::Hosts(vec![HostRef::new("h1", "host", "secret-ref").unwrap()])),
        };
        let result = dispatch_with_executor("ops.list_hosts", &serde_json::json!({}), &executor)
            .unwrap()
            .unwrap();
        assert!(!result.to_string().contains("credential_ref"));
        assert!(!result.to_string().contains("secret-ref"));

        let failing = RecordingExecutor {
            seen: Mutex::new(Vec::new()),
            response: Err(OpsError::unsafe_path("private")),
        };
        let error = dispatch_with_executor("ops.status", &serde_json::json!({}), &failing)
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, "OPS_UNSAFE_PATH");
        assert!(!error.message.contains("secret-ref"));
    }
}
