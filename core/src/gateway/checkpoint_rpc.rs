//! P2-11 checkpoint RPC（Rust 侧本地实现，不依赖 Python 侧车）。
//!
//! 与 Python 侧 `tasks.checkpoint.list/abandon/replay` 同名同语义，这样控制台
//! 在侧车离线/刚重启时也能列出被中断的长任务、放弃或重放，而不会拿到 501。
//! 一期 replay 语义 = `reset_and_reissue`（整任务重发）。

use serde_json::{json, Value};

use crate::agent::task_checkpoint::{default_store, TaskCheckpoint};

fn view(cp: &TaskCheckpoint) -> Value {
    serde_json::to_value(cp).unwrap_or(Value::Null)
}

/// `tasks.checkpoint.list`：列出重启后仍在飞的 checkpoint。
pub fn checkpoint_list() -> Value {
    let store = match default_store() {
        Ok(s) => s,
        Err(e) => {
            return json!({ "ok": false, "error": format!("checkpoint store unavailable: {e}") })
        }
    };
    let interrupted = store.list_interrupted();
    let items: Vec<Value> = interrupted.iter().map(view).collect();
    json!({
        "ok": true,
        "interrupted": items,
        "count": items.len(),
        "phase": 1,
        "replay_semantics": "reset_and_reissue",
    })
}

/// `tasks.checkpoint.abandon`：params { taskId }。
pub fn checkpoint_abandon(params: &Value) -> Value {
    let task_id = params
        .get("taskId")
        .and_then(Value::as_str)
        .or_else(|| params.get("task_id").and_then(Value::as_str))
        .unwrap_or("");
    if task_id.is_empty() {
        return json!({ "ok": false, "error": "taskId 不可为空" });
    }
    let store = match default_store() {
        Ok(s) => s,
        Err(e) => {
            return json!({ "ok": false, "error": format!("checkpoint store unavailable: {e}") })
        }
    };
    match store.abandon(task_id) {
        Some(cp) => json!({ "ok": true, "checkpoint": view(&cp) }),
        None => json!({ "ok": false, "error": "checkpoint 不存在" }),
    }
}

/// `tasks.checkpoint.replay`：params { taskId }。
/// 一期：标记为 queued_for_replay（重置后整任务重发，由调用方重新发起）。
pub fn checkpoint_replay(params: &Value) -> Value {
    let task_id = params
        .get("taskId")
        .and_then(Value::as_str)
        .or_else(|| params.get("task_id").and_then(Value::as_str))
        .unwrap_or("");
    if task_id.is_empty() {
        return json!({ "ok": false, "error": "taskId 不可为空" });
    }
    let store = match default_store() {
        Ok(s) => s,
        Err(e) => {
            return json!({ "ok": false, "error": format!("checkpoint store unavailable: {e}") })
        }
    };
    match store.mark_for_replay(task_id) {
        Some(cp) => json!({ "ok": true, "checkpoint": view(&cp), "replay_semantics": "reset_and_reissue" }),
        None => json!({ "ok": false, "error": "checkpoint 不存在" }),
    }
}
