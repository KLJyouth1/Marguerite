//! 事件推送：`events::Envelope` → WebSocket `event` 帧
//!
//! 网关为每个已认证连接订阅事件总线 `**`，把内核事件映射为
//! Marguerite 命名的事件帧（`session.event.<type>`），并按客户端协议
//! 要求携带从 1 递增的 `seq`（断档即触发客户端重连）。

use serde_json::{json, Value};

use crate::events::envelope::Envelope;

/// pyengine 侧车事件名透传
///
/// 侧车事件经 [`crate::pyengine::event_bridge`] 统一以 [`crate::events::envelope::EventType::Status`]
/// 进总线，原始事件名保存在 `payload.py_event`。此前 `envelope_to_frame` 只看枚举
/// 类型，于是**所有**侧车事件都被拍平成 `session.event.status` —— 前端订阅的
/// `state_change` / `cron_result` / `task_group.*` 在双引擎模式下一条也收不到。
///
/// 只放行两种名字，防止侧车借此注入任意 WS 事件名：
/// - `session.event.<[a-z0-9_.]+>`
/// - 顶层 `sessions.changed`（其余顶层名如 `artifact.updated` 是 Rust 引擎自己的
///   事件，pyengine 没有对应发射点，放行只会扩大可伪造面）
///
/// payload 不是对象时桥接层不会注入 `py_event`，返回 `None` 并回退到原有映射。
fn pyengine_event_name(env: &Envelope) -> Option<String> {
    if !env.session.starts_with("pyengine.") {
        return None;
    }
    let raw = env.payload.get("py_event")?.as_str()?;
    let charset_ok = !raw.is_empty()
        && raw
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.');
    if !charset_ok {
        return None;
    }
    if let Some(name) = raw.strip_prefix("session.event.") {
        // 命名卫生：拒绝空尾、空段（`session.event..x`）与以点开头的段。
        // 这些名字前端不会订阅，放行只会扩大伪造事件面。
        let valid = !name.is_empty()
            && name.split('.').all(|segment| {
                !segment.is_empty()
                    && segment
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_alphanumeric() || c == '_')
                        .unwrap_or(false)
            });
        return valid.then(|| raw.to_string());
    }
    (raw == "sessions.changed").then(|| raw.to_string())
}

/// 把内核事件信封映射为 WS 事件帧
///
/// 事件名采用 `session.event.<type>` 命名空间（UI 的
/// `useChatRpcSubscriptions` 订阅同前缀）；payload 沿用信封负载，
/// meta 携带 session / ts / trace_id 便于前端归因。
pub fn envelope_to_frame(env: &Envelope, seq: u64) -> Value {
    let event = pyengine_event_name(env).unwrap_or_else(|| match env.r#type.as_str() {
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
    });
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

    fn pyengine_envelope(py_event: Value, session: &str) -> Envelope {
        Envelope::new(
            EventType::Status,
            json!({ "py_event": py_event, "py_seq": 7, "py_meta": { "session": "s1" } }),
            session,
        )
    }

    /// W1：侧车的细分事件名必须原样到达前端，而不是被拍平成 status。
    #[test]
    fn pyengine_state_change_is_forwarded_verbatim() {
        let frame = envelope_to_frame(
            &pyengine_envelope(json!("session.event.state_change"), "pyengine.s1"),
            9,
        );
        assert_eq!(frame["event"], "session.event.state_change");
        assert_eq!(frame["payload"]["py_event"], "session.event.state_change");
        assert_eq!(frame["seq"], 9);
    }

    #[test]
    fn pyengine_cron_result_and_task_group_are_forwarded() {
        for name in [
            "session.event.cron_result",
            "session.event.task_group.waiting",
            "session.event.task_group.synthesizing",
            "session.event.task_group.done",
            "session.event.task_group.failed",
        ] {
            let frame = envelope_to_frame(
                &pyengine_envelope(json!(name), "pyengine.s1"),
                1,
            );
            assert_eq!(frame["event"], name, "{name} 必须原样透传");
        }
    }

    #[test]
    fn pyengine_sessions_changed_keeps_top_level_name() {
        let frame = envelope_to_frame(
            &pyengine_envelope(json!("sessions.changed"), "pyengine.global"),
            1,
        );
        assert_eq!(frame["event"], "sessions.changed");
    }

    /// 非法事件名绝不进入 WS 帧：回退到既有 status 映射（不 panic、不静默丢帧）。
    #[test]
    fn pyengine_illegal_event_name_falls_back_to_status() {
        for bad in [
            "session.event.x;drop",
            "session.event.UPPER",
            "session.event.",
            "session.event..dropped",
            "session.event.x..y",
            "session.event.\u{4e2d}\u{6587}",
            "artifact.updated",
            "",
        ] {
            let frame = envelope_to_frame(
                &pyengine_envelope(json!(bad), "pyengine.s1"),
                1,
            );
            assert_eq!(frame["event"], "session.event.status", "{bad:?} 必须被拒绝");
        }
    }

    /// 非 pyengine 会话即使带了 py_event 也不得改写事件名（Rust 引擎自身事件）。
    #[test]
    fn pyengine_non_pyengine_session_is_not_rewritten() {
        let env = pyengine_envelope(json!("session.event.state_change"), "ws.s1");
        assert_eq!(envelope_to_frame(&env, 1)["event"], "session.event.status");
    }

    /// 非对象 payload 不会被桥接层注入 py_event → 保持原有回退语义。
    #[test]
    fn pyengine_non_object_payload_falls_back() {
        let env = Envelope::new(EventType::Status, json!("raw-string"), "pyengine.s1");
        assert_eq!(envelope_to_frame(&env, 1)["event"], "session.event.status");
    }

    // ── W1.2 跨语言契约守卫 ──────────────────────────────────────────────
    // 前端订阅的事件名必须在 pyengine 有真实发射点，pyengine 的发射点也必须在
    // 前端有订阅。任一侧改名，这里立刻变红（include_str! 在编译期读取源码，
    // 不需要网络，也不会因 CWD 不同而漂移）。
    const PY_BOOT: &str = include_str!("../../../pyengine/src/opensquilla/gateway/boot.py");
    const PY_RPC_SESSIONS: &str =
        include_str!("../../../pyengine/src/opensquilla/gateway/rpc_sessions.py");
    const PY_BACKGROUND: &str =
        include_str!("../../../pyengine/src/opensquilla/gateway/background_completion.py");
    const PY_CHANNEL_DISPATCH: &str =
        include_str!("../../../pyengine/src/opensquilla/gateway/channel_dispatch.py");
    const PY_TYPES: &str = include_str!("../../../pyengine/src/opensquilla/engine/types.py");
    const WEBUI_SUBSCRIPTIONS: &str =
        include_str!("../../../webui/src/composables/chat/useChatRpcSubscriptions.ts");

    #[test]
    fn pyengine_event_subscriptions_match_across_languages() {
        let cross_engine = [
            "session.event.state_change",
            "session.event.cron_result",
            "session.event.task_group.waiting",
            "session.event.task_group.synthesizing",
            "session.event.task_group.done",
            "session.event.task_group.failed",
            "session.event.run_heartbeat",
            "sessions.changed",
        ];
        for name in cross_engine {
            let subscription = format!("'{name}'");
            assert!(
                WEBUI_SUBSCRIPTIONS.contains(subscription.as_str()),
                "前端 useChatRpcSubscriptions 必须订阅 {name}"
            );
        }

        // pyengine 发射点：字面量或 f-string（state_change 由事件 kind 派生）
        assert!(PY_BOOT.contains("\"session.event.cron_result\""));
        assert!(PY_BOOT.contains("\"sessions.changed\""));
        assert!(PY_CHANNEL_DISPATCH.contains("\"session.event.run_heartbeat\""));
        assert!(PY_BACKGROUND.contains("f\"session.event.task_group.{phase}\""));
        for phase in ["waiting", "synthesizing", "done", "failed"] {
            let call = format!("_emit(parent_session_key, \"{phase}\"");
            assert!(
                PY_BACKGROUND.contains(call.as_str()),
                "task_group.{phase} 缺少真实发射点"
            );
        }
        assert!(PY_RPC_SESSIONS.contains("f\"session.event.{event_kind}\""));
        assert!(PY_TYPES.contains("Literal[\"state_change\"]"));
    }
}
