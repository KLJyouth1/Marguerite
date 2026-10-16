//! 升级确认 RPC（M2-C4）
//!
//! 事件发布仅在有总线上下文时进行（由宿主注入）；本模块提供无状态的
//! `escalation.ack`（按 escalation_id 决策 继续/中止）与 `escalation.pending`
//! （拉取当前挂起的「需要确认」清单），决策统一走进程级
//! [`crate::guard::escalation_registry::global_registry`]，任一端都能闭环。

use serde_json::{json, Value};

use crate::guard::escalation_registry::{global_registry, EscalationAck, EscalationResolution};

/// `escalation.ack`：按 escalation_id 决策。返回解析结果或 NotFound。
pub fn escalation_ack(params: &Value) -> Value {
    let id = params
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let decision_raw = params
        .get("decision")
        .and_then(Value::as_str)
        .unwrap_or("");
    let Some(decision) = EscalationResolution::parse(decision_raw) else {
        return json!({ "ok": false, "error": "invalid_decision",
                       "decision": decision_raw });
    };
    let reg = global_registry();
    match reg.resolve(id, decision) {
        EscalationAck::Resolved(ev, _) => json!({
            "ok": true,
            "id": id,
            "decision": decision.as_str(),
            "escalation": ev,
        }),
        EscalationAck::NotFound => json!({ "ok": false, "error": "not_found", "id": id }),
    }
}

/// `escalation.pending`：当前待确认的升级事件清单（供前端 badge / 初始化拉取）。
pub fn escalation_pending() -> Value {
    let list: Vec<Value> = global_registry()
        .pending()
        .into_iter()
        .map(|ev| ev.to_payload())
        .collect();
    json!({ "ok": true, "count": list.len(), "escalations": list })
}
