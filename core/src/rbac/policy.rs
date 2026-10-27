//! Explicit method-level ACL policy.

use serde::{Deserialize, Serialize};

use super::roles::Role;

pub const RBAC_FORBIDDEN: &str = "RBAC_FORBIDDEN";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forbidden {
    pub code: String,
    pub method: String,
    pub role: Role,
    pub message: String,
}

impl Forbidden {
    fn new(method: &str, role: Role) -> Self {
        Self {
            code: RBAC_FORBIDDEN.to_string(),
            method: method.to_string(),
            role,
            message: format!("role {:?} is not allowed to call method {method}", role),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Requirement {
    Read,
    Developer,
    Admin,
}

/// Exact methods and prefixes are centralized here. Unknown methods deny by default.
const READ_METHODS: &[&str] = &[
    "health",
    "status",
    "system.status",
    "doctor.status",
    "config.get",
];
const DEVELOPER_METHODS: &[&str] = &[
    "dev.intent.classify",
    // 记忆治理写入口（本地 owner 恒通过；登记后未授权角色会被明确拒绝）
    "memory.save",
    "memory.delete",
    "memory.validate",
    // 知识库写入口
    "kb.ingest",
    "kb.ingest_file",
    "kb.delete",
    "kb.update",
];
const DEVELOPER_PREFIXES: &[&str] = &["dev.", "ops.host.", "ops.log."];
const ADMIN_METHODS: &[&str] = &[
    "ops.file.write",
    "ops.process.kill",
    "team.users.create",
    "credentials.write",
];

#[derive(Debug, Clone, Copy, Default)]
pub struct Policy;

impl Policy {
    pub fn check(&self, method: &str, role: Role) -> Result<(), Forbidden> {
        let requirement = requirement_for(method).ok_or_else(|| Forbidden::new(method, role))?;
        let allowed = match requirement {
            Requirement::Read => true,
            Requirement::Developer => matches!(role, Role::Admin | Role::Developer),
            Requirement::Admin => matches!(role, Role::Admin),
        };
        allowed
            .then_some(())
            .ok_or_else(|| Forbidden::new(method, role))
    }
}

fn requirement_for(method: &str) -> Option<Requirement> {
    if ADMIN_METHODS.contains(&method) {
        return Some(Requirement::Admin);
    }
    if READ_METHODS.contains(&method) {
        return Some(Requirement::Read);
    }
    if DEVELOPER_METHODS.contains(&method)
        || DEVELOPER_PREFIXES
            .iter()
            .any(|prefix| method.starts_with(prefix))
    {
        return Some(Requirement::Developer);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_methods_are_exact_not_prefixes() {
        assert_eq!(
            requirement_for("credentials.write"),
            Some(Requirement::Admin)
        );
        assert!(requirement_for("ops.process.kill.extra").is_none());
    }

    /// 记忆/知识库写入口必须登记，且 Developer 及以上才放行（Guest 被拒）。
    #[test]
    fn memory_and_kb_writes_require_developer() {
        for method in [
            "memory.save",
            "memory.delete",
            "memory.validate",
            "kb.ingest",
            "kb.ingest_file",
            "kb.delete",
            "kb.update",
        ] {
            assert_eq!(
                requirement_for(method),
                Some(Requirement::Developer),
                "{method} 必须登记"
            );
            assert!(Policy.check(method, Role::Developer).is_ok(), "{method}");
            assert!(Policy.check(method, Role::Admin).is_ok(), "{method}");
            assert!(
                Policy.check(method, Role::Guest).is_err(),
                "{method} 不得放行 Guest"
            );
        }
    }
}
