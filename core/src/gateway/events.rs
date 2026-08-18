//! 事件推送：`events::Envelope` → WebSocket `event` 帧
//!
//! 网关为每个已认证连接订阅事件总线 `**`，把内核事件映射为
//! Marguerite 命名的事件帧（`session.event.<type>`），并按客户端协议
//! 要求携带从 1 递增的 `seq`（断档即触发客户端重连）。

use serde_json::{json, Value};

use crate::events::envelope::Envelope;

/// 把内核事件信封映射为 WS 事件帧
///
/// 事件名采用 `session.event.<type>` 命名空间（UI 的
/// `useChatRpcSubscriptions` 订阅同前缀）；payload 沿用信封负载，
/// meta 携带 session / ts / trace_id 便于前端归因。
pub fn envelope_to_frame(env: &Envelope, seq: u64) -> Value {
    let event = match env.r#type.as_str() {
        // 兼容前端事件命名：tool_call → tool_use_start（前端 Marguerite 契约）
        "tool_call" => "session.event.tool_use_start".to_string(),
        // 会话运行态变更：顶层事件名（前端 `useSessionListSubscription` 直接订阅
        // `sessions.changed`），不套 `session.event.*` 命名空间。
        "sessions_changed" => "sessions.changed".to_string(),
        // 产物登记/更新（Epic A R2）：顶层事件名 `artifact.updated`，不套 namespace。
        "artifact_updated" => "artifact.updated".to_string(),
        // 桌面动作完成（Epic C）：顶层事件名 `desktop.action_done`，不套 namespace。
        "desktop_action_done" => "desktop.action_done".to_string(),
        other => format!("session.event.{other}"),
    };
    json!({
        "type": "event",
        "event": event,
        "payload": env.payload,
        "meta": {
            "session": env.session,
            "ts": env.ts.to_rfc3339(),
            "trace_id": env.trace_id,
            "v": env.v,
        },
        "seq": seq,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::envelope::{Envelope, EventType};
    use serde_json::json;

    #[test]
    fn test_envelope_to_frame_mapping() {
        let env = Envelope::new(EventType::ToolCall, json!({ "tool": "read_file" }), "ws.s1");
        let frame = envelope_to_frame(&env, 3);
        assert_eq!(frame["type"], "event");
        // WebUI 实际订阅 `session.event.tool_use_start`
        // （webui/src/composables/chat/useChatRpcSubscriptions.ts:60、
        //  useChatRpcEventHandlers.ts:452、types/rpc.ts:791），
        // 故信封 `tool_call` 经 envelope_to_frame 映射为该前端契约名。
        assert_eq!(frame["event"], "session.event.tool_use_start");
        assert_eq!(frame["seq"], 3);
        assert_eq!(frame["payload"]["tool"], "read_file");
        assert_eq!(frame["meta"]["session"], "ws.s1");
        assert!(frame["meta"]["ts"].is_string());
    }

    #[test]
    fn test_status_and_done_mapping() {
        let env = Envelope::mode_changed("core", "agent", "bloom");
        let frame = envelope_to_frame(&env, 1);
        assert_eq!(frame["event"], "session.event.status");
        assert_eq!(frame["payload"]["kind"], "mode.changed");
    }
}
