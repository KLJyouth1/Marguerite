//! 信任校准 RPC（M2-C5）：grant / revoke / revoke_all / list。
//!
//! 封装 `guard::trust` 全局信任存储，供 WebUI 设置界面查看与撤销信任。

use serde_json::{json, Value};

use crate::events::product;
use crate::guard::trust::{self, TrustLevel};

/// `trust.list`：列出全部信任条目（设置界面查看）
pub fn trust_list() -> Value {
    let g = trust::global().lock().unwrap_or_else(|e| e.into_inner());
    let entries: Vec<Value> = g.list().iter().map(|e| trust::trust_view(e)).collect();
    json!({ "entries": entries, "total": entries.len() })
}

/// `trust.grant`：授予信任
/// params: { level: "low"|"medium"|"high", tool: "<tool>|*", mode?: "<mode>", session: "..." }
pub fn trust_grant(params: &Value) -> Value {
    let level = match params
        .get("level")
        .and_then(Value::as_str)
        .and_then(TrustLevel::from_key)
    {
        Some(l) => l,
        None => return json!({ "error": "level 必须是 low/medium/high" }),
    };
    let tool = params.get("tool").and_then(Value::as_str).unwrap_or("");
    if tool.is_empty() {
        return json!({ "error": "tool 不可为空" });
    }
    let mode = params.get("mode").and_then(Value::as_str).map(|s| s.to_string());
    let session = params
        .get("session")
        .and_then(Value::as_str)
        .unwrap_or("default");
    let mut g = trust::global().lock().unwrap_or_else(|e| e.into_inner());
    let id = g.grant(level, tool, mode, session);
    // Low/High 立即落盘（Medium 仅内存）
    if level.persists() {
        let _ = g.persist();
    }
    // M3-D1 产品事件：信任档位变更（switch trust level）
    product::emit(
        product::TRUST_CHANGED,
        session,
        json!({ "action": "grant", "id": id, "level": level.as_str(), "tool": tool }),
    );
    json!({ "id": id, "entry": trust::trust_view(g.list().iter().find(|e| e.id == id).unwrap()) })
}

/// `trust.revoke`：撤销指定信任
/// params: { id: "tr_1" }
pub fn trust_revoke(params: &Value) -> Value {
    let id = params.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() {
        return json!({ "error": "id 不可为空" });
    }
    let mut g = trust::global().lock().unwrap_or_else(|e| e.into_inner());
    let removed = g.revoke(id);
    if removed && g.persist().is_err() {
        // 落盘失败不阻塞撤销结果
    }
    product::emit(
        product::TRUST_CHANGED,
        "default",
        json!({ "action": "revoke", "id": id, "removed": removed }),
    );
    json!({ "removed": removed })
}

/// `trust.revoke_all`：一键撤销全部信任
pub fn trust_revoke_all() -> Value {
    let mut g = trust::global().lock().unwrap_or_else(|e| e.into_inner());
    let n = g.revoke_all();
    let _ = g.persist();
    product::emit(
        product::TRUST_CHANGED,
        "default",
        json!({ "action": "revoke_all", "removed": n }),
    );
    json!({ "removed": n })
}
