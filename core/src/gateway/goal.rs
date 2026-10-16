//! Goal engine gateway adapter.
//!
//! This module exposes a structured, transport-neutral API for the existing
//! [`crate::goal::GoalRegistry`]. It intentionally does not modify the WS
//! dispatcher; methods.rs can delegate to [`dispatch`] when that seam is ready.

use crate::goal::{goal_view, Goal, GoalRegistry, MIN_BLOCKED_ROUNDS};
use crate::gateway::protocol::RpcError;
use serde_json::{json, Value};

/// Stateful adapter used by callers that need an isolated registry (tests or
/// an embedding runtime). The process-wide [`dispatch`] uses `goal::global`.
pub struct GoalGateway {
    registry: GoalRegistry,
}

impl Default for GoalGateway {
    fn default() -> Self { Self::new() }
}

impl GoalGateway {
    pub fn new() -> Self { Self { registry: GoalRegistry::new() } }

    pub fn call(&mut self, method: &str, params: &Value) -> Result<Value, RpcError> {
        dispatch_registry(&mut self.registry, method, params)
    }
}

/// Gateway method adapter for `goal.*` methods. This is callable without a
/// GatewayContext and is suitable for a future methods.rs dispatch branch.
pub fn dispatch(method: &str, params: Value) -> Result<Value, RpcError> {
    let mut guard = crate::goal::global().lock().map_err(|_| {
        RpcError::new("GOAL_LOCKED", "目标状态暂不可用")
    })?;
    dispatch_registry(&mut guard, method, &params)
}

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::new("GOAL_INVALID_ARGUMENT", message)
}

fn require_id(params: &Value) -> Result<&str, RpcError> {
    params.get("goalId").and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| invalid("goalId 不可为空"))
}

fn require_current(registry: &GoalRegistry, params: &Value) -> Result<Goal, RpcError> {
    let id = require_id(params)?;
    let goal = registry.get().ok_or_else(|| RpcError::new("GOAL_NOT_FOUND", "目标不存在"))?;
    if goal.id != id {
        return Err(RpcError::new("GOAL_NOT_FOUND", "目标不存在"));
    }
    if let Some(expected) = params.get("expectedRevision").and_then(Value::as_u64) {
        if expected != goal.revision {
            return Err(RpcError::new("GOAL_STALE_REVISION", "目标修订号已过期").with_details(json!({
                "expectedRevision": expected,
                "currentRevision": goal.revision,
            })));
        }
    }
    Ok(goal.clone())
}

fn view(registry: &GoalRegistry) -> Result<Value, RpcError> {
    let goal = registry.get().ok_or_else(|| RpcError::new("GOAL_NOT_FOUND", "目标不存在"))?;
    let mut result = goal_view(goal, registry.is_armed());
    if goal.max_goal_rounds.is_some_and(|cap| goal.rounds_started >= cap) {
        result["terminal"] = json!(true);
        result["terminalReason"] = json!("max_goal_rounds_reached");
    } else {
        result["terminal"] = json!(false);
    }
    Ok(result)
}

fn objective(params: &Value) -> Result<&str, RpcError> {
    params.get("objective").and_then(Value::as_str)
        .map(str::trim).filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("objective 不可为空"))
}

fn dispatch_registry(registry: &mut GoalRegistry, method: &str, params: &Value) -> Result<Value, RpcError> {
    match method {
        "goal.create" => {
            let objective = objective(params)?;
            let max = params.get("maxGoalRounds").map(|v| v.as_u64()
                .ok_or_else(|| invalid("maxGoalRounds 必须是正整数")))
                .transpose()?.map(|n| u32::try_from(n).map_err(|_| invalid("maxGoalRounds 超出范围")))
                .transpose()?;
            if max == Some(0) { return Err(invalid("maxGoalRounds 必须大于 0")); }
            let goal = registry.create(objective, max).map_err(|e| RpcError::new("GOAL_CONFLICT", e.to_string()))?;
            Ok(goal_view(&goal, registry.is_armed()))
        }
        "goal.get" => { require_current(registry, params)?; view(registry) }
        "goal.edit" => {
            require_current(registry, params)?;
            let goal = registry.edit(objective(params)?).map_err(|e| invalid(e.to_string()))?;
            Ok(goal_view(&goal, registry.is_armed()))
        }
        "goal.pause" => {
            require_current(registry, params)?;
            let goal = registry.pause().map_err(|e| RpcError::new("GOAL_INVALID_STATE", e.to_string()))?;
            Ok(goal_view(&goal, registry.is_armed()))
        }
        "goal.resume" => {
            require_current(registry, params)?;
            let goal = registry.resume().map_err(|e| RpcError::new("GOAL_INVALID_STATE", e.to_string()))?;
            Ok(goal_view(&goal, registry.is_armed()))
        }
        "goal.complete" => {
            require_current(registry, params)?;
            let goal = registry.complete().map_err(|e| RpcError::new("GOAL_INVALID_STATE", e.to_string()))?;
            Ok(goal_view(&goal, registry.is_armed()))
        }
        "goal.blocked" => {
            require_current(registry, params)?;
            let reason = params.get("reason").and_then(Value::as_str)
                .map(str::trim).filter(|s| !s.is_empty())
                .ok_or_else(|| invalid("reason 不可为空"))?;
            let goal = registry.block(reason).map_err(|e| {
                if registry.get().is_some_and(|g| g.rounds_started < MIN_BLOCKED_ROUNDS) {
                    RpcError::new("GOAL_BLOCKED_MIN_ROUNDS", e.to_string())
                } else { RpcError::new("GOAL_INVALID_STATE", e.to_string()) }
            })?;
            Ok(goal_view(&goal, registry.is_armed()))
        }
        "goal.begin_round" => {
            let current = require_current(registry, params)?;
            let expected = params.get("expectedRevision").and_then(Value::as_u64)
                .ok_or_else(|| invalid("expectedRevision 不可为空"))?;
            if expected != current.revision {
                return Err(RpcError::new("GOAL_STALE_REVISION", "目标修订号已过期").with_details(json!({
                    "expectedRevision": expected, "currentRevision": current.revision,
                })));
            }
            if current.max_goal_rounds.is_some_and(|cap| current.rounds_started >= cap) {
                return Err(RpcError::new("GOAL_MAX_ROUNDS", "目标已达到最大续轮次数").with_details(json!({
                    "maxGoalRounds": current.max_goal_rounds, "roundsStarted": current.rounds_started,
                })));
            }
            if !registry.can_continue() {
                return Err(RpcError::new("GOAL_INVALID_STATE", "目标当前不可续轮"));
            }
            if !registry.begin_round_at(expected) {
                return Err(RpcError::new("GOAL_STALE_REVISION", "目标修订号已过期"));
            }
            view(registry)
        }
        _ => Err(RpcError::new("GOAL_UNKNOWN_METHOD", format!("未知目标方法: {method}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 隔离夹具：目标注册表只写注入的临时目录。
    ///
    /// 此前这些用例直接构造默认路径的注册表，每次 `cargo test` 都往用户真实
    /// `~/.marguerite/goal.db` 追加夹具行（累计 900+ 行），进程重启后
    /// `restore_from_sqlite` 会把最新一行夹具当成当前目标。
    fn isolated_gateway() -> GoalGateway {
        let dir = std::env::temp_dir().join(format!(
            "marguerite_gateway_goal_test_{}_{}",
            std::process::id(),
            NEXT_TEST_DIR.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        crate::persistence::config::Config::set_config_dir_override(dir);
        GoalGateway::new()
    }

    static NEXT_TEST_DIR: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    #[test]
    fn goal_gateway_crud_and_round_guard() {
        let mut gateway = isolated_gateway();
        let created = gateway.call("goal.create", &json!({"objective":"ship safely", "maxGoalRounds": 2})).unwrap();
        assert_eq!(created["phase"], "active");
        let id = created["id"].as_str().unwrap().to_string();
        let stale = gateway.call("goal.begin_round", &json!({"goalId": id, "expectedRevision": 999})).unwrap_err();
        assert_eq!(stale.code, "GOAL_STALE_REVISION");
        let first = gateway.call("goal.begin_round", &json!({"goalId": id, "expectedRevision": created["revision"]})).unwrap();
        assert_eq!(first["roundsStarted"], 1);
        let second = gateway.call("goal.begin_round", &json!({"goalId": id, "expectedRevision": first["revision"]})).unwrap();
        assert_eq!(second["terminal"], true);
        let capped = gateway.call("goal.begin_round", &json!({"goalId": id, "expectedRevision": second["revision"]})).unwrap_err();
        assert_eq!(capped.code, "GOAL_MAX_ROUNDS");
    }

    #[test]
    fn blocked_requires_three_rounds_and_structured_validation() {
        let mut gateway = isolated_gateway();
        let created = gateway.call("goal.create", &json!({"objective":"blocked flow"})).unwrap();
        let id = created["id"].as_str().unwrap().to_string();
        let err = gateway.call("goal.blocked", &json!({"goalId": id, "reason":"network"})).unwrap_err();
        assert_eq!(err.code, "GOAL_BLOCKED_MIN_ROUNDS");
        for _ in 0..3 {
            let current = gateway.call("goal.get", &json!({"goalId": id})).unwrap();
            gateway.call("goal.begin_round", &json!({"goalId": id, "expectedRevision": current["revision"]})).unwrap();
        }
        let blocked = gateway.call("goal.blocked", &json!({"goalId": id, "reason":"network"})).unwrap();
        assert_eq!(blocked["phase"], "blocked");
        let invalid = gateway.call("goal.edit", &json!({"goalId": id, "objective":""})).unwrap_err();
        assert_eq!(invalid.code, "GOAL_INVALID_ARGUMENT");
    }

    #[test]
    fn real_dispatch_reaches_goal_adapter() {
        use crate::events::EventBus;
        use crate::gateway::methods::{dispatch_sync, GatewayContext};
        use crate::persistence::config::Config;
        use std::sync::Arc;
        use std::time::Instant;

        // 隔离：本用例走全局注册表，读写也必须落临时目录（此前每跑一次测试就往
        // 用户真实 goal.db 追加一行 "dispatch probe"）。
        let dir = std::env::temp_dir().join(format!(
            "marguerite_gateway_goal_dispatch_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Config::set_config_dir_override(dir);
        // Ensure a prior process-wide goal cannot make this reachability check flaky.
        if let Ok(mut registry) = crate::goal::global().lock() {
            if registry.get().is_some() {
                let _ = registry.complete();
            }
        }
        let ctx = GatewayContext {
            config: Config::default(),
            token: "test".into(),
            started_at: Instant::now(),
            bus: Arc::new(EventBus::default()),
        };
        let result = dispatch_sync(&ctx, "goal.create", &json!({"objective":"dispatch probe"}));
        let created = result.expect("goal.create should be reachable through dispatch");
        assert_eq!(created["phase"], "active");
        let id = created["id"].as_str().expect("goal id");
        let fetched = dispatch_sync(&ctx, "goal.get", &json!({"goalId": id})).expect("goal.get");
        assert_eq!(fetched["id"], id);
        let _ = dispatch_sync(&ctx, "goal.complete", &json!({"goalId": id}));
    }
}
