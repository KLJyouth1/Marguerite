//! Python 侧车 WS 事件 → Rust EventBus 桥
//!
//! Python 网关发出的所有 event 帧：
//! ```json
//! {"type":"event","event":"session.event.xxx","payload":...,"meta":{...},"seq":N}
//! ```
//! 统一 emit 到 Rust [`EventBus`]：
//! - `Envelope.type = EventType::Status`（Python 事件在 Rust 总线统一视为 Status 类，
//!   细分事件名放在 payload 的 `py_event` 字段）
//! - `Envelope.session = "pyengine.{meta.session | \"global\"}"`
//! - payload 保持原 Python payload，额外注入 `py_event` / `seq` / `meta` 信息。

use anyhow::Result;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::events::bus::EventBus;
use crate::events::envelope::{Envelope, EventType};

/// 给 Python 事件 payload 补齐 Rust 侧消费所需字段。
///
/// 关键一项是 `session_key`：Python 的 payload **不带**会话键（key 只在
/// `meta.session`），而 Rust 事件总线把帧广播给**所有** WS 客户端。前端的
/// `isCurrentSessionPayload` 对「没有 key」是宽容放行（视为当前会话），于是
/// 别的 pyengine 会话的 `state_change` 会被当前聊天当成自己的：凭空拉起
/// streaming（界面出现 Working/Sending）并冲掉在跑的审批卡片
/// （2026-10-26 e2e 门禁实测：同一份代码 94~95 通过随机波动，卡片先出现后消失）。
fn enrich_payload(event: &str, meta: &Value, payload: Value, seq: u64) -> (String, Value) {
    let session = meta
        .get("session")
        .and_then(Value::as_str)
        .unwrap_or("global")
        .to_string();
    let payload = match payload {
        Value::Object(mut obj) => {
            // 附原始事件元数据：消费方据此区分不同 Python 原生事件
            obj.insert("py_event".to_string(), Value::String(event.to_string()));
            obj.insert("py_seq".to_string(), Value::from(seq));
            obj.insert("py_meta".to_string(), meta.clone());
            // 缺失或**空串**都要补：前端对 `!key` 是宽容放行（视为当前会话），
            // 空串会绕过会话过滤，和缺字段一样危险。
            let needs_key = obj
                .get("session_key")
                .and_then(Value::as_str)
                .map(|key| key.trim().is_empty())
                .unwrap_or(true);
            if needs_key {
                obj.insert("session_key".to_string(), Value::String(session.clone()));
            }
            Value::Object(obj)
        }
        // 非对象 payload 无法承载 session_key。旧实现原样透传 → 前端把「没有 key」
        // 当成当前会话，跨会话污染依旧。包一层对象保留原值，并带上归属会话。
        other => json!({
            "session_key": session,
            "py_event": event,
            "py_seq": seq,
            "py_meta": meta,
            "py_payload": other,
        }),
    };
    (session, payload)
}

/// 消费 PyGatewayClient 暴露的 EVENT_SINK，向 EventBus emit 对应 topic
pub async fn bridge(bus: EventBus) -> Option<JoinHandle<()>> {
    let rx = super::client::take_event_sink().await?;
    Some(tokio::spawn(async move {
        if let Err(e) = run(rx, bus).await {
            warn!(err = %e, "事件桥异常退出");
        }
    }))
}

/// P1-8：事件 sink 是 broadcast——本桥是慢消费者，洪峰时会收到 `Lagged`
/// （最旧事件已被驱动循环丢弃）。这是**刻意的 lossy 语义**：UI 状态类事件
/// 会按新快照重拉，丢中间帧不致命；绝不能为「不丢」而阻塞 WS 驱动循环。
async fn run(mut rx: broadcast::Receiver<Value>, bus: EventBus) -> Result<()> {
    loop {
        match rx.recv().await {
            Ok(frame) => {
                let event = frame
                    .get("event")
                    .and_then(Value::as_str)
                    .unwrap_or("pyengine.event");
                let meta = frame.get("meta").cloned().unwrap_or(Value::Null);
                let seq = frame.get("seq").and_then(Value::as_u64).unwrap_or_default();
                let payload = frame.get("payload").cloned().unwrap_or(Value::Null);
                let (session, payload) = enrich_payload(event, &meta, payload, seq);
                let env = Envelope::new(EventType::Status, payload, format!("pyengine.{session}"));
                bus.publish(env);
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                // 丢了 n 个最旧事件帧：记录但不重排——最新状态随后就到。
                warn!(
                    skipped = n,
                    "pyengine 事件桥落后于驱动循环，丢弃 {n} 个最旧事件帧（lossy 背压）"
                );
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
    debug!("pyengine 事件桥 sink 已关闭");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn enrich_payload_tags_the_originating_session() {
        let meta = json!({"session": "agent:main:webchat:a", "ts": 1});
        let (session, payload) = enrich_payload(
            "session.event.state_change",
            &meta,
            json!({"to_state": "running"}),
            7,
        );
        assert_eq!(session, "agent:main:webchat:a");
        // 前端按 session_key/key 过滤其它会话；缺这个字段时会被当成当前会话。
        assert_eq!(payload["session_key"], "agent:main:webchat:a");
        assert_eq!(payload["py_event"], "session.event.state_change");
        assert_eq!(payload["py_seq"], 7);
        assert_eq!(payload["py_meta"]["session"], "agent:main:webchat:a");
    }

    #[test]
    fn enrich_payload_keeps_an_explicit_session_key() {
        let meta = json!({"session": "agent:main:webchat:a"});
        let (_, payload) = enrich_payload(
            "session.event.text_delta",
            &meta,
            json!({"session_key": "agent:main:webchat:b"}),
            1,
        );
        assert_eq!(payload["session_key"], "agent:main:webchat:b");
    }

    #[test]
    fn enrich_payload_wraps_non_object_payloads_with_the_session_key() {
        let meta = json!({"session": "agent:main:webchat:a"});
        let (session, payload) = enrich_payload("session.event.state_change", &meta, Value::Null, 3);
        assert_eq!(session, "agent:main:webchat:a");
        assert_eq!(payload["session_key"], "agent:main:webchat:a");
        assert!(payload.get("py_payload").is_some(), "原始 null 载荷应保留在 py_payload");
        assert_eq!(payload["py_event"], "session.event.state_change");
    }

    #[test]
    fn enrich_payload_replaces_an_empty_session_key() {
        let meta = json!({"session": "agent:main:webchat:a"});
        let (_, payload) = enrich_payload(
            "session.event.text_delta",
            &meta,
            json!({"session_key": "  "}),
            1,
        );
        assert_eq!(payload["session_key"], "agent:main:webchat:a");
    }

    #[test]
    fn enrich_payload_without_meta_is_global_not_current_chat() {
        let (session, payload) =
            enrich_payload("sessions.changed", &Value::Null, json!({"count": 1}), 0);
        assert_eq!(session, "global");
        // "global" ≠ 任何真实会话键，因此前端会把它判为「别的会话」，不再误伤当前聊天。
        assert_eq!(payload["session_key"], "global");
    }
}
