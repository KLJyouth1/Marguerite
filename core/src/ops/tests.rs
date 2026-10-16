use super::*;

#[test]
fn host_requires_non_empty_identity() {
    assert!(HostRef::new("", "host", "cred").is_err());
    assert!(HostRef::new("id", "", "cred").is_err());
}

#[test]
fn credential_ref_is_safe_to_debug_and_serialize() {
    let host = HostRef::new("h1", "127.0.0.1", "secret-ref").unwrap();
    let debug = format!("{host:?}");
    let json = serde_json::to_string(&host).unwrap();
    assert!(debug.contains("credential_ref"));
    assert!(!debug.contains("password"));
    assert!(json.contains("credential_ref"));
    assert!(!json.contains("secret-key"));
}

#[test]
fn unknown_operation_is_rejected() {
    let req = ReadOnlyRequest::new("format_disk");
    assert_eq!(req.validate().unwrap_err().code(), "OPS_INVALID_REQUEST");
}

#[test]
fn write_operation_is_rejected_structurally() {
    let req = ReadOnlyRequest::new("delete_file");
    assert_eq!(req.validate().unwrap_err().code(), "OPS_FORBIDDEN_WRITE");
}

#[test]
fn unsafe_paths_are_rejected() {
    let req = ReadOnlyRequest::new("list_files").with_path("../../etc");
    assert_eq!(req.validate().unwrap_err().code(), "OPS_UNSAFE_PATH");
    let req = ReadOnlyRequest::new("tail_logs").with_path("/etc/passwd");
    assert_eq!(req.validate().unwrap_err().code(), "OPS_UNSAFE_PATH");
}

#[test]
fn tail_limits_are_enforced() {
    assert!(tail_content("a\nb\nc", 2, 100).is_err());
    assert!(tail_content("abcdef", 10, 3).is_err());
    assert_eq!(tail_content("a\nb", 2, 100).unwrap(), "a\nb");
}

#[test]
fn file_limits_are_enforced() {
    let entries = vec![FileEntry::new("a", 2), FileEntry::new("b", 3)];
    assert!(limit_files(entries.clone(), 1, 100).is_err());
    assert!(limit_files(entries, 10, 4).is_err());
}

#[test]
fn errors_have_stable_codes() {
    assert_eq!(OpsError::invalid("x").code(), "OPS_INVALID_REQUEST");
    assert_eq!(OpsError::forbidden_write().code(), "OPS_FORBIDDEN_WRITE");
    assert_eq!(OpsError::unsafe_path("x").code(), "OPS_UNSAFE_PATH");
    assert_eq!(OpsError::credential_required().code(), "OPS_CREDENTIAL_REQUIRED");
}

#[test]
fn serialization_is_deterministic() {
    let req = ReadOnlyRequest::new("status").with_host(HostRef::new("h", "a", "c").unwrap());
    assert_eq!(serde_json::to_string(&req).unwrap(), serde_json::to_string(&req).unwrap());
}

#[test]
fn empty_input_is_rejected() {
    assert_eq!(ReadOnlyRequest::new("").validate().unwrap_err().code(), "OPS_INVALID_REQUEST");
    assert_eq!(HostRef::new(" ", "h", "c").unwrap_err().code(), "OPS_INVALID_REQUEST");
}

#[test]
fn null_executor_does_not_perform_side_effects() {
    let req = ReadOnlyRequest::new("status").with_host(HostRef::new("h", "a", "c").unwrap());
    let err = NullExecutor.execute(&req).unwrap_err();
    assert_eq!(err.code(), "OPS_CREDENTIAL_REQUIRED");
}
