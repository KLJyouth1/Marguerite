//! Method-level role based access control for the local gateway.
//!
//! This module is deliberately explicit: the actor role is a local, declarative
//! claim today. A real authentication mechanism must replace it before binding
//! the gateway outside the loopback interface.

pub mod policy;
pub mod roles;

/// Explicit authorization seam for dispatchers. Existing callers that do not
/// carry an actor remain compatible; new authenticated paths should call this
/// before invoking a gateway method.
pub fn check(method: &str, actor: &roles::Actor) -> Result<(), policy::Forbidden> {
    policy::Policy.check(method, actor.role)
}

#[cfg(test)]
mod tests;
