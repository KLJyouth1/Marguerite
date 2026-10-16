//! WP2 MCP·A2A 双向协议底座 —— Agent-to-Agent 消息信封原语。
//!
//! 纯数据 + 纯校验：
//! - [`A2aEnvelope`] 是 agent 之间路由消息的统一边界格式；
//! - 序列化走 serde（`serde_json`），校验是同步纯函数，**不连网、不做 I/O**；
//! - 具体传输（回环 WebSocket / 侧车 stdio）由后续阶段在本信封之上接线。
//!
//! 不变量（违反即缺陷，由 [`A2aEnvelope::validate`] fail-closed 检查）：
//! - `from` / `to` 非空；
//! - `from != to`（自寻址无意义，拒绝以杜绝回声死循环）；
//! - `payload` 必须是 JSON object（数组 / 字符串 / null 一律拒收，避免下游
//!   对 payload 形状做大量分支）。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// 信封校验错误（同步、纯数据，可判等以便测试断言）。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum A2aValidationError {
    #[error("a2a.envelope.from 不可为空")]
    EmptyFrom,
    #[error("a2a.envelope.to 不可为空")]
    EmptyTo,
    #[error("a2a.envelope.from == to（自寻址无意义，拒绝以杜绝回声死循环）")]
    SelfAddressed,
    #[error("a2a.envelope.payload 必须是 JSON object")]
    PayloadNotObject,
}

/// 消息种类（粗粒度枚举；具体任务语义由 `payload` 内部自由扩展）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum A2aKind {
    /// 请求某个 agent 执行任务。
    Task,
    /// 任务结果回传。
    Result,
    /// 流式中间事件（进度 / 思考）。
    Event,
    /// 心跳 / 存活探测。
    Ping,
}

impl A2aKind {
    pub fn as_str(self) -> &'static str {
        match self {
            A2aKind::Task => "task",
            A2aKind::Result => "result",
            A2aKind::Event => "event",
            A2aKind::Ping => "ping",
        }
    }
}

/// A2A 消息信封：agent 之间路由的统一单元。
///
/// 字段字面就是 `{ from, to, kind, payload, ts }`：
/// - `from` / `to`：agent 身份 id（字符串；路由层不解析其内部结构）；
/// - `kind`：[`A2aKind`]；
/// - `payload`：业务载荷，**必须是 JSON object**；
/// - `ts`：发送方时钟的 Unix 毫秒时间戳（接收方不做 NTP 校正，只做展示 / 去重）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct A2aEnvelope {
    pub from: String,
    pub to: String,
    pub kind: A2aKind,
    pub payload: Value,
    pub ts: i64,
}

impl A2aEnvelope {
    /// 构造并以当前 UTC 时间盖 `ts`。
    pub fn new(
        from: impl Into<String>,
        to: impl Into<String>,
        kind: A2aKind,
        payload: Value,
    ) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            kind,
            payload,
            ts: chrono::Utc::now().timestamp_millis(),
        }
    }

    /// 纯函数校验：按顺序返回**第一个**违反的不变量。
    ///
    /// 不做「尽力而为」：任一不变量被违反即 `Err`，调用方必须拒收该信封。
    pub fn validate(&self) -> Result<(), A2aValidationError> {
        if self.from.trim().is_empty() {
            return Err(A2aValidationError::EmptyFrom);
        }
        if self.to.trim().is_empty() {
            return Err(A2aValidationError::EmptyTo);
        }
        if self.from == self.to {
            return Err(A2aValidationError::SelfAddressed);
        }
        if !self.payload.is_object() {
            return Err(A2aValidationError::PayloadNotObject);
        }
        Ok(())
    }

    /// 序列化为 JSON 字节（落盘 / 跨进程投递用）。
    pub fn to_json_vec(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// 从 JSON 字节反序列化 + 校验一步完成。
    ///
    /// 反序列化成功但校验失败（例如 payload 是数组）→ [`A2aDecodeError::Validation`]，
    /// 绝不把「形状对但语义错」的信封悄悄放过去。
    pub fn from_slice(bytes: &[u8]) -> Result<Self, A2aDecodeError> {
        let env: Self = serde_json::from_slice(bytes)?;
        env.validate()?;
        Ok(env)
    }
}

/// 反序列化 + 校验合并错误。
#[derive(Debug, Error)]
pub enum A2aDecodeError {
    #[error("a2a 信封 JSON 反序列化失败: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("a2a 信封校验失败: {0}")]
    Validation(#[from] A2aValidationError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn good_envelope() -> A2aEnvelope {
        A2aEnvelope::new("agent-a", "agent-b", A2aKind::Task, json!({"goal": "ping"}))
    }

    #[test]
    fn new_stamps_ts_and_fields() {
        let before = chrono::Utc::now().timestamp_millis();
        let env = good_envelope();
        let after = chrono::Utc::now().timestamp_millis();
        assert_eq!(env.from, "agent-a");
        assert_eq!(env.to, "agent-b");
        assert_eq!(env.kind, A2aKind::Task);
        assert_eq!(env.payload, json!({"goal": "ping"}));
        // ts 在 [before, after] 区间内（含边界）。
        assert!(env.ts >= before && env.ts <= after, "ts={} not in [{before},{after}]", env.ts);
    }

    #[test]
    fn validate_accepts_well_formed_envelope() {
        assert!(good_envelope().validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_from() {
        let mut env = good_envelope();
        env.from = "   ".to_string();
        assert_eq!(env.validate(), Err(A2aValidationError::EmptyFrom));
    }

    #[test]
    fn validate_rejects_empty_to() {
        let mut env = good_envelope();
        env.to = String::new();
        assert_eq!(env.validate(), Err(A2aValidationError::EmptyTo));
    }

    #[test]
    fn validate_rejects_self_addressed() {
        let env = A2aEnvelope::new("agent-x", "agent-x", A2aKind::Ping, json!({}));
        assert_eq!(env.validate(), Err(A2aValidationError::SelfAddressed));
    }

    #[test]
    fn validate_rejects_non_object_payload() {
        for bad in [json!(null), json!([1, 2]), json!("str"), json!(42)] {
            let label = format!("{bad}");
            let env = A2aEnvelope::new("a", "b", A2aKind::Event, bad);
            assert_eq!(
                env.validate(),
                Err(A2aValidationError::PayloadNotObject),
                "未拒绝 payload={label}"
            );
        }
    }

    #[test]
    fn kind_serializes_to_snake_case() {
        assert_eq!(serde_json::to_string(&A2aKind::Task).unwrap(), "\"task\"");
        assert_eq!(serde_json::to_string(&A2aKind::Result).unwrap(), "\"result\"");
        assert_eq!(serde_json::to_string(&A2aKind::Ping).unwrap(), "\"ping\"");
        assert_eq!(A2aKind::Event.as_str(), "event");
    }

    #[test]
    fn serde_roundtrip_through_bytes() {
        let env = good_envelope();
        let bytes = env.to_json_vec().unwrap();
        let back = A2aEnvelope::from_slice(&bytes).unwrap();
        assert_eq!(back, env);
    }

    #[test]
    fn from_slice_rejects_malformed_json() {
        let err = A2aEnvelope::from_slice(b"{not json").unwrap_err();
        assert!(matches!(err, A2aDecodeError::Serde(_)), "得到 {err:?}");
    }

    #[test]
    fn from_slice_rejects_deserializable_but_invalid_envelope() {
        // 形状合法（能反序列化出 struct），但 payload 是数组 → 校验必须拦下来。
        let bytes = br#"{"from":"a","to":"b","kind":"task","payload":[1],"ts":1}"#;
        let err = A2aEnvelope::from_slice(bytes).unwrap_err();
        match err {
            A2aDecodeError::Validation(A2aValidationError::PayloadNotObject) => {}
            other => panic!("期望 PayloadNotObject，得到 {other:?}"),
        }
    }
}
