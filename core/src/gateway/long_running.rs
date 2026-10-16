//! 长时任务 RPC：列出 / 暂停 / 续跑 / 手动保存检查点。
//!
//! 封装 `automation::long_running` 的全局台账，给前端「后台任务」面板
//! 提供生命周期操控。所有操作在台账不可用时返回结构化错误。

use serde_json::{json, Value};

use crate::automation::long_running::{self, Stage};

/// `long_running.list`：列出全部长时任务（含检查点与心跳状态）
pub fn long_running_list() -> Value {
    let g = long_running::global()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tasks: Vec<Value> = g
        .list()
        .iter()
        .map(|t| long_running::task_view(t))
        .collect();
    json!({ "tasks": tasks })
}

/// `long_running.pause`：显式暂停（running/resumed → paused，等待续跑）
pub fn long_running_pause(params: &Value) -> Value {
    let task_id = params.get("taskId").and_then(Value::as_str).unwrap_or("");
    if task_id.is_empty() {
        return json!({ "error": "taskId 不可为空" });
    }
    let mut g = long_running::global()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match g.pause(task_id) {
        Ok(()) => match g.get(task_id) {
            Some(t) => json!({ "task": long_running::task_view(t) }),
            None => json!({ "error": "任务不存在" }),
        },
        Err(e) => json!({ "error": e.to_string() }),
    }
}

/// `long_running.resume`：从检查点续跑（paused → resumed，retry+1）
pub fn long_running_resume(params: &Value) -> Value {
    let task_id = params.get("taskId").and_then(Value::as_str).unwrap_or("");
    if task_id.is_empty() {
        return json!({ "error": "taskId 不可为空" });
    }
    let mut g = long_running::global()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match g.resume(task_id) {
        Ok(()) => match g.get(task_id) {
            Some(t) => json!({ "task": long_running::task_view(t) }),
            None => json!({ "error": "任务不存在" }),
        },
        Err(e) => json!({ "error": e.to_string() }),
    }
}

/// `long_running.checkpoint`：手动保存检查点（stage 可选，默认 execute）
pub fn long_running_checkpoint(params: &Value) -> Value {
    let task_id = params.get("taskId").and_then(Value::as_str).unwrap_or("");
    if task_id.is_empty() {
        return json!({ "error": "taskId 不可为空" });
    }
    let stage = match params
        .get("stage")
        .and_then(Value::as_str)
        .unwrap_or("execute")
    {
        "analyze" => Stage::Analyze,
        "plan" => Stage::Plan,
        "aggregate" => Stage::Aggregate,
        "refine" => Stage::Refine,
        _ => Stage::Execute,
    };
    let state = params.get("state").cloned().unwrap_or_else(|| json!({}));
    let mut g = long_running::global()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match g.save_checkpoint(task_id, stage, state) {
        Ok(()) => match g.get(task_id) {
            Some(t) => json!({ "task": long_running::task_view(t) }),
            None => json!({ "error": "任务不存在" }),
        },
        Err(e) => json!({ "error": e.to_string() }),
    }
}
