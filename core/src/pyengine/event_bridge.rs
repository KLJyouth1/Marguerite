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
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::events::bus::EventBus;
use crate::events::envelope::{Envelope, EventType};

/// 消费 PyGatewayClient 暴露的 EVENT_SINK，向 EventBus emit 对应 topic
pub async fn bridge(bus: EventBus) -> Option<JoinHandle<()>> {
    let rx = super::client::take_event_sink().await?;
    Some(tokio::spawn(async move {
        if let Err(e) = run(rx, bus).await {
            warn!(err = %e, "事件桥异常退出");
        }
    }))
}

async fn run(mut rx: mpsc::Receiver<Value>, bus: EventBus) -> Result<()> {
    while let Some(frame) = rx.recv().await {
        let event = frame
            .get("event")
            .and_then(Value::as_str)
            .unwrap_or("pyengine.event");
        let meta = frame.get("meta").cloned().unwrap_or(Value::Null);
        let session = meta
            .get("session")
            .and_then(Value::as_str)
            .unwrap_or("global");
        let seq = frame.get("seq").and_then(Value::as_u64).unwrap_or_default();
        let mut payload = frame.get("payload").cloned().unwrap_or(Value::Null);
        // 附原始事件元数据：消费方据此区分不同 Python 原生事件
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("py_event".to_string(), Value::String(event.to_string()));
            obj.insert("py_seq".to_string(), Value::from(seq));
            obj.insert("py_meta".to_string(), meta.clone());
        }
        let env = Envelope::new(EventType::Status, payload, format!("pyengine.{session}"));
        bus.publish(env);
    }
    debug!("pyengine 事件桥 sink 已关闭");
    Ok(())
}
