//! action event loop —— 界面交互事件回传模型（Track C1 契约硬性要求）。
//!
//! 生成式 UI 不是静态渲染：用户点按钮、提交表单、答测验，都必须能**回到模型**，
//! 否则界面只是死图。此处定义最小回路：
//! `UI 交互 → ActionEvent → route_action → ActionSink → （编排层）模型`。
//!
//! [`ActionSink`] 抽掉了「事件送去哪」，使回路可在纯单测中用
//! [`VecActionSink`] 收集断言，不需要真实模型或网络。

use serde::{Deserialize, Serialize};

/// 一次界面交互事件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionEvent {
    /// 触发事件的 widget id（对应 [`Widget::id`](crate::genui::Widget::id)）
    pub widget_id: String,
    /// 动作名（如 "submit" / "select" / "click"）
    pub action: String,
    /// 动作载荷（表单值 / 选项索引等，结构随 action 而变）
    pub payload: serde_json::Value,
}

impl ActionEvent {
    /// 构造事件（载荷为任意可序列化 JSON 值）。
    pub fn new(widget_id: &str, action: &str, payload: serde_json::Value) -> Self {
        Self {
            widget_id: widget_id.to_string(),
            action: action.to_string(),
            payload,
        }
    }
}

/// 事件汇：把 [`ActionEvent`] 交给下游（编排层 → 模型）。
pub trait ActionSink {
    /// 投递一个事件。实现方负责真正的转发/落库/入队。
    fn emit(&mut self, event: ActionEvent);
}

/// 收集型事件汇：把事件存入 `events`，供测试与本地回放断言。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VecActionSink {
    /// 已收到的事件（按投递顺序）
    pub events: Vec<ActionEvent>,
}

impl ActionSink for VecActionSink {
    fn emit(&mut self, event: ActionEvent) {
        self.events.push(event);
    }
}

/// 回模型入口：编排层持有 `sink`，界面事件统一经此路由。
///
/// MVP 语义即「原样转投 sink」；后续可在此接入节流、鉴权、
/// widget_id 存在性校验等策略，而调用方签名不变。
pub fn route_action(sink: &mut dyn ActionSink, event: ActionEvent) {
    sink.emit(event);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn vec_sink_collects_emitted_event() {
        let mut sink = VecActionSink::default();
        let ev = ActionEvent::new("f1", "submit", json!({"email":"a@b.c"}));
        sink.emit(ev.clone());
        assert_eq!(sink.events.len(), 1, "emit 后应收到 1 条事件");
        assert_eq!(sink.events[0], ev, "收集到的事件应与投递的一致");
    }

    #[test]
    fn route_action_delivers_to_sink() {
        let mut sink = VecActionSink::default();
        let ev = ActionEvent::new("q1", "select", json!({"index":2}));
        route_action(&mut sink, ev.clone());
        assert_eq!(sink.events, vec![ev], "route_action 必须调用 sink.emit");
    }

    #[test]
    fn route_action_preserves_order_of_multiple_events() {
        let mut sink = VecActionSink::default();
        route_action(&mut sink, ActionEvent::new("a", "click", json!(null)));
        route_action(&mut sink, ActionEvent::new("b", "submit", json!({"k":1})));
        let ids: Vec<&str> = sink.events.iter().map(|e| e.widget_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"], "事件顺序应保持投递顺序");
        assert_eq!(sink.events[1].payload, json!({"k":1}));
    }

    #[test]
    fn action_event_serde_roundtrip() {
        let ev = ActionEvent::new("c1", "hover", json!({"x":1.5,"y":[1,2]}));
        let s = serde_json::to_string(&ev).expect("序列化");
        let back: ActionEvent = serde_json::from_str(&s).expect("反序列化");
        assert_eq!(ev, back, "事件 JSON 往返应一致（可跨进程回传模型）");
    }
}
