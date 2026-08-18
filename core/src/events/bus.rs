//! 事件总线（EventBus）
//!
//! 主题订阅 + 通配匹配 + 历史回放缓冲（LRU，上限可配，默认 2000）。
//! 路由键取自 [`Envelope::route_key`]（`{session}.{type}`），
//! 订阅模式用 `.` 分段，支持单段通配 `*` 与多段通配 `**`。

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::mpsc;

use crate::events::envelope::Envelope;
use tracing::debug;

/// 回放缓冲默认上限
pub const DEFAULT_HISTORY_CAPACITY: usize = 2000;

/// 单个订阅者事件缓冲上限（有界：提供反压，防止慢消费者拖垮发布方内存）
pub const SUBSCRIBER_BUFFER_CAPACITY: usize = 1024;

/// 事件总线
#[derive(Debug, Clone)]
pub struct EventBus {
    /// 历史回放缓冲（LRU 裁剪，最旧出队）
    history: Arc<Mutex<VecDeque<Envelope>>>,
    /// 各信封的发布时刻（与 `history` 同步，用于 `history(since)` 回放）
    pub_at: Arc<Mutex<VecDeque<Instant>>>,
    /// 主题 → 订阅者发送端列表（有界 `mpsc::Sender`，提供反压）
    subs: Arc<Mutex<HashMap<String, Vec<mpsc::Sender<Envelope>>>>>,
    /// 因订阅者缓冲满而丢弃的事件计数（可观测性）
    dropped: Arc<AtomicU64>,
    /// 回放缓冲上限
    capacity: usize,
}

impl EventBus {
    /// 创建总线（指定回放缓冲上限）
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            history: Arc::new(Mutex::new(VecDeque::with_capacity(cap))),
            pub_at: Arc::new(Mutex::new(VecDeque::with_capacity(cap))),
            subs: Arc::new(Mutex::new(HashMap::new())),
            dropped: Arc::new(AtomicU64::new(0)),
            capacity: cap,
        }
    }

    /// 订阅主题，返回事件接收端
    ///
    /// `topic` 支持通配：`agent.*` 匹配 `agent.x`，`ws.**` 匹配 `ws.s1.tool_call`，
    /// `**` 匹配任意事件。
    pub fn subscribe(&self, topic: &str) -> mpsc::Receiver<Envelope> {
        let (tx, rx) = mpsc::channel(SUBSCRIBER_BUFFER_CAPACITY);
        self.subs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(topic.to_string())
            .or_default()
            .push(tx);
        rx
    }

    /// 发布事件：写入历史回放缓冲并分发给所有匹配订阅者
    pub fn publish(&self, env: Envelope) {
        // 1) 历史回放缓冲（与发布时刻同步入队，超容量 LRU 裁剪最旧）
        {
            let mut h = self.history.lock().unwrap_or_else(|e| e.into_inner());
            let mut t = self.pub_at.lock().unwrap_or_else(|e| e.into_inner());
            h.push_back(env.clone());
            t.push_back(Instant::now());
            while h.len() > self.capacity {
                h.pop_front();
                t.pop_front();
            }
        }

        // 2) 收集匹配订阅者（持锁期间仅克隆发送端，发送在锁外进行）
        let route = env.route_key();
        let mut targets: Vec<mpsc::Sender<Envelope>> = Vec::new();
        {
            let subs = self.subs.lock().unwrap_or_else(|e| e.into_inner());
            for (pattern, senders) in subs.iter() {
                if Self::match_topic(pattern, &route) {
                    targets.extend(senders.iter().cloned());
                }
            }
        }

        // 3) 分发（订阅者已断开或缓冲满则忽略发送失败；满时计入丢弃计数）
        for tx in targets {
            if tx.try_send(env.clone()).is_err() {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                debug!(route = route, "事件订阅者缓冲已满，丢弃事件（慢消费者）");
            }
        }
    }

    /// 因订阅者缓冲满而累计丢弃的事件数（可观测性 / 反压告警）
    pub fn dropped_events(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// 回放 `since` 之后发布的事件（断线重连补全）
    pub fn history(&self, since: Instant) -> Vec<Envelope> {
        let h = self.history.lock().unwrap_or_else(|e| e.into_inner());
        let t = self.pub_at.lock().unwrap_or_else(|e| e.into_inner());
        h.iter()
            .zip(t.iter())
            .filter(|(_, ti)| **ti >= since)
            .map(|(e, _)| e.clone())
            .collect()
    }

    /// 主题通配匹配
    ///
    /// 模式用 `.` 分段：`*` 匹配单段，`**` 匹配零或多段（可置于任意位置）。
    /// 订阅模式在前，事件路由键在后。
    pub fn match_topic(pattern: &str, topic: &str) -> bool {
        let p: Vec<&str> = pattern.split('.').collect();
        let t: Vec<&str> = topic.split('.').collect();

        fn go(pi: usize, ti: usize, p: &[&str], t: &[&str]) -> bool {
            if pi == p.len() && ti == t.len() {
                return true;
            }
            if pi == p.len() {
                return false; // 模式耗尽但主题仍有剩余段（无尾部 **）
            }
            match p[pi] {
                "**" => {
                    // 零段匹配：跳过 ** 看后续是否对齐
                    if go(pi + 1, ti, p, t) {
                        return true;
                    }
                    // 一段匹配：消耗一个主题段继续
                    if ti < t.len() {
                        return go(pi, ti + 1, p, t);
                    }
                    false
                }
                "*" => {
                    if ti < t.len() {
                        go(pi + 1, ti + 1, p, t)
                    } else {
                        false
                    }
                }
                seg => {
                    if ti < t.len() && t[ti] == seg {
                        go(pi + 1, ti + 1, p, t)
                    } else {
                        false
                    }
                }
            }
        }

        go(0, 0, &p, &t)
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(DEFAULT_HISTORY_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::envelope::EventType;
    use serde_json::json;
    use std::time::Duration;

    fn env(session: &str, ty: EventType) -> Envelope {
        Envelope::new(ty, json!({}), session)
    }

    // ---- E3 通配匹配 ----

    #[test]
    fn test_e3_topic_match_exact() {
        assert!(EventBus::match_topic("agent.foo", "agent.foo"));
        assert!(!EventBus::match_topic("agent.foo", "agent.bar"));
    }

    #[test]
    fn test_e3_topic_match_single_wildcard() {
        assert!(EventBus::match_topic("agent.*", "agent.foo"));
        assert!(!EventBus::match_topic("agent.*", "agent.foo.bar"));
        assert!(!EventBus::match_topic("agent.*", "ws.foo"));
    }

    #[test]
    fn test_e3_topic_match_double_wildcard() {
        assert!(EventBus::match_topic("ws.**", "ws.s1.tool_call"));
        assert!(EventBus::match_topic("ws.**", "ws.s1"));
        assert!(EventBus::match_topic("**", "anything.here.now"));
        assert!(EventBus::match_topic("**", "x"));
    }

    #[test]
    fn test_e3_topic_no_match() {
        assert!(!EventBus::match_topic("agent.*", "ws.foo"));
        assert!(!EventBus::match_topic("ws.s1.*", "ws.s2.tool_call"));
        assert!(!EventBus::match_topic("a.b.c", "a.b"));
    }

    // ---- E4 历史回放 / 订阅分发 ----

    #[test]
    fn test_subscribe_and_publish() {
        let bus = EventBus::default();
        let mut rx = bus.subscribe("core.*");
        bus.publish(env("core", EventType::Status));
        let got = rx.try_recv().expect("应收到事件");
        assert_eq!(got.session, "core");
        assert_eq!(got.r#type, EventType::Status);
    }

    #[test]
    fn test_wildcard_star_pair_receives_all() {
        // 路由键为 `{session}.{type}`（两段），单段通配需逐位使用 `*`
        let bus = EventBus::default();
        let mut rx = bus.subscribe("*.*");
        bus.publish(env("a", EventType::Thinking));
        bus.publish(env("b", EventType::Done));
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_double_wildcard_receives_all() {
        let bus = EventBus::default();
        let mut rx = bus.subscribe("**");
        bus.publish(env("ws.s1", EventType::ToolCall));
        bus.publish(env("agent", EventType::Progress));
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn test_specific_subscriber_not_receives_other() {
        // 订阅 ws.s1 会话全事件；route_key = ws.s1.status（三段）
        let bus = EventBus::default();
        let mut rx = bus.subscribe("ws.s1.*");
        bus.publish(env("agent", EventType::Status)); // route: agent.status → 不匹配
        assert!(rx.try_recv().is_err());
        bus.publish(env("ws.s1", EventType::Status)); // route: ws.s1.status → 匹配
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn test_multiple_subscribers_same_topic() {
        let bus = EventBus::default();
        let mut r1 = bus.subscribe("core.*");
        let mut r2 = bus.subscribe("core.*");
        bus.publish(env("core", EventType::Done));
        assert!(r1.try_recv().is_ok());
        assert!(r2.try_recv().is_ok());
    }

    #[test]
    fn test_publish_no_subscribers_no_panic() {
        let bus = EventBus::default();
        bus.publish(env("core", EventType::Error)); // 无订阅者，不应 panic
    }

    #[test]
    fn test_history_replay_since() {
        let bus = EventBus::default();
        // since 之前发布 2 条
        bus.publish(env("core", EventType::Thinking));
        bus.publish(env("core", EventType::Thinking));
        let since = Instant::now();
        std::thread::sleep(Duration::from_millis(5));
        // since 之后发布 3 条
        bus.publish(env("core", EventType::Status));
        bus.publish(env("core", EventType::Progress));
        bus.publish(env("ws.s1", EventType::ToolCall));

        let replayed = bus.history(since);
        assert_eq!(replayed.len(), 3);
        assert!(replayed.iter().all(|e| e.ts.timestamp() >= 0));
    }

    // ---- E5 回放缓冲上限 ----

    #[test]
    fn test_history_capacity_trim() {
        let bus = EventBus::new(5); // 小容量便于测试
        let since = Instant::now();
        for i in 0..10u32 {
            bus.publish(env(&format!("s{i}"), EventType::Status));
        }
        let retained = bus.history(since);
        assert_eq!(retained.len(), 5, "超出容量应裁剪为 5 条");
        // 最旧（s0..s4）应被丢弃，仅保留 s5..s9
        assert_eq!(retained[0].session, "s5");
        assert_eq!(retained[4].session, "s9");
    }

    #[test]
    fn test_mode_changed_builder_published() {
        let bus = EventBus::default();
        let mut rx = bus.subscribe("**");
        let since = Instant::now();
        bus.publish(Envelope::mode_changed("core", "agent", "bloom"));
        let got = rx.try_recv().expect("应收到 mode.changed");
        assert_eq!(got.payload["kind"], "mode.changed");
        assert_eq!(got.payload["from"], "agent");
        assert_eq!(got.payload["to"], "bloom");
        assert_eq!(bus.history(since).len(), 1);
    }
}
