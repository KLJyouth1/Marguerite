//! Actor roles used by the method ACL.

use serde::{Deserialize, Serialize};

/// Local role claim. It is not an authentication mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Role {
    Admin,
    Developer,
    #[default]
    Guest,
}


impl Role {
    /// Parse configuration/actor declarations using least privilege on unknown values.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "admin" | "administrator" => Self::Admin,
            "developer" | "dev" => Self::Developer,
            _ => Self::Guest,
        }
    }
}

/// Declarative actor identity for an explicit ACL check.
///
/// This remains local and caller-declared until real multi-user authentication
/// is added. Before binding outside 127.0.0.1, authenticated identity and
/// transport protection must replace this declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub id: String,
    pub role: Role,
}

impl Actor {
    pub fn new(id: impl Into<String>, role: Role) -> Self {
        Self {
            id: id.into(),
            role,
        }
    }

    pub fn from_declaration(value: &str) -> Self {
        Self::new("local", Role::parse(value))
    }
}
