use super::policy::{Policy, RBAC_FORBIDDEN};
use super::roles::{Actor, Role};

#[test]
fn role_matrix_allows_expected_methods() {
    let policy = Policy;
    assert!(policy.check("team.users.create", Role::Admin).is_ok());
    assert!(policy.check("ops.file.write", Role::Admin).is_ok());
    assert!(policy.check("ops.host.list", Role::Developer).is_ok());
    assert!(policy.check("dev.intent.classify", Role::Developer).is_ok());
    assert!(policy.check("status", Role::Guest).is_ok());
    assert!(policy.check("config.get", Role::Guest).is_ok());
}

#[test]
fn guest_and_developer_are_denied_admin_methods() {
    let policy = Policy;
    let admin_methods = [
        "ops.file.write",
        "ops.process.kill",
        "team.users.create",
        "credentials.write",
    ];
    for method in admin_methods {
        assert_eq!(
            policy.check(method, Role::Guest).unwrap_err().code,
            RBAC_FORBIDDEN
        );
        assert_eq!(
            policy.check(method, Role::Developer).unwrap_err().code,
            RBAC_FORBIDDEN
        );
    }
}

#[test]
fn unknown_methods_default_to_deny() {
    let err = Policy
        .check("ops.unknown.danger", Role::Admin)
        .unwrap_err();
    assert_eq!(err.code, RBAC_FORBIDDEN);
    assert_eq!(err.method, "ops.unknown.danger");
}

#[test]
fn explicit_dispatch_seam_returns_structured_error() {
    let err = super::check("credentials.write", &Actor::from_declaration("guest")).unwrap_err();
    assert_eq!(err.code, RBAC_FORBIDDEN);
    assert_eq!(err.method, "credentials.write");
}

#[test]
fn actor_role_parses_case_insensitively_and_defaults_guest() {
    assert_eq!(Actor::from_declaration("admin").role, Role::Admin);
    assert_eq!(Actor::from_declaration("DEV").role, Role::Developer);
    assert_eq!(Actor::from_declaration("other").role, Role::Guest);
}
