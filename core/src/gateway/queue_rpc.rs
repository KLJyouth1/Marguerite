//! 持久任务队列 RPC（M2-C6）：list / enqueue / start / pause / resume / cancel。
//!
//! 封装 `automation::task_queue` 全局队列，供 WebUI 队列管理视图跨会话操作。

use serde_json::{json, Value};

use crate::automation::task_queue::{self, item_view};

/// `queue.list`：列出全部队列任务（跨会话 Backlog）
pub fn queue_list() -> Value {
    let q = task_queue::global().lock().unwrap_or_else(|e| e.into_inner());
    let items: Vec<Value> = q.list().iter().map(|i| item_view(i)).collect();
    json!({ "items": items, "total": items.len() })
}

/// `queue.enqueue`：入队（排队等待）
/// params: { title, goal, mode?, session?, priority? }
pub fn queue_enqueue(params: &Value) -> Value {
    let title = params.get("title").and_then(Value::as_str).unwrap_or("");
    if title.is_empty() {
        return json!({ "error": "title 不可为空" });
    }
    let goal = params
        .get("goal")
        .and_then(Value::as_str)
        .unwrap_or(title);
    let mode = params
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("agent");
    let session = params
        .get("session")
        .and_then(Value::as_str)
        .unwrap_or("default");
    let priority = params.get("priority").and_then(Value::as_i64).unwrap_or(5) as i32;
    let mut q = task_queue::global().lock().unwrap_or_else(|e| e.into_inner());
    let id = q.enqueue(title, goal, mode, session, priority);
    let _ = q.persist();
    json!({ "id": id, "item": item_view(q.get(&id).unwrap()) })
}

fn task_id_of(params: &Value) -> Result<String, &'static str> {
    let id = params.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() {
        Err("id 不可为空")
    } else {
        Ok(id.to_string())
    }
}

fn run_op<F>(params: &Value, op: F) -> Value
where
    F: Fn(&mut task_queue::TaskQueue, &str) -> bool,
{
    let id = match task_id_of(params) {
        Ok(id) => id,
        Err(e) => return json!({ "error": e }),
    };
    let mut q = task_queue::global().lock().unwrap_or_else(|e| e.into_inner());
    let ok = op(&mut q, &id);
    let _ = q.persist();
    match q.get(&id) {
        Some(item) => json!({ "ok": ok, "item": item_view(item) }),
        None => json!({ "ok": false, "error": "任务不存在" }),
    }
}

/// `queue.start`：开始执行
pub fn queue_start(params: &Value) -> Value {
    run_op(params, task_queue::TaskQueue::start)
}

/// `queue.pause`：暂停
pub fn queue_pause(params: &Value) -> Value {
    run_op(params, task_queue::TaskQueue::pause)
}

/// `queue.resume`：恢复（Paused → Queued 重新排队）
pub fn queue_resume(params: &Value) -> Value {
    run_op(params, task_queue::TaskQueue::resume)
}

/// `queue.cancel`：取消
pub fn queue_cancel(params: &Value) -> Value {
    run_op(params, task_queue::TaskQueue::cancel)
}

/// `queue.move`：调整调度顺序（上移/下移一步），仅对排队中任务生效。
/// params: { id, direction: "up" | "down" }
pub fn queue_move(params: &Value) -> Value {
    let id = match task_id_of(params) {
        Ok(id) => id,
        Err(e) => return json!({ "error": e }),
    };
    let dir = params.get("direction").and_then(Value::as_str).unwrap_or("");
    let up = match dir {
        "up" => true,
        "down" => false,
        _ => return json!({ "error": "direction 须为 up/down" }),
    };
    let mut q = task_queue::global().lock().unwrap_or_else(|e| e.into_inner());
    let ok = q.move_step(&id, up);
    let _ = q.persist();
    match q.get(&id) {
        Some(item) => json!({ "ok": ok, "item": item_view(item) }),
        None => json!({ "ok": false, "error": "任务不存在" }),
    }
}
