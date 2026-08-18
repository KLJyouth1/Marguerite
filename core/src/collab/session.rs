//! 协作会话：多人同看一个 Agent 运行，实时事件流广播 + 回放缓冲
//!
//! 复用核心 `Envelope` 事件信封，保证与单用户事件总线同构；新加入成员可先 `replay`
//! 拉取最近事件，再实时接收 `broadcast` 推送（对应 FR-15 的实时协作事件流）。

use std::collections::VecDeque;

use crate::events::{Envelope, EventType};

/// 回放缓冲上限（与单用户事件总线 2000 对齐）
const REPLAY_CAP: usize = 2000;

/// 协作会话
#[derive(Debug, Clone)]
pub struct CollabSession {
    /// 会话 ID
    pub id: String,
    /// 在线成员 ID 列表（去重）
    members: Vec<String>,
    /// 事件回放缓冲（最早在前）
    events: VecDeque<Envelope>,
}

impl CollabSession {
    /// 创建空会话
    pub fn new(id: &str) -> Self {
        Self {
            id: id.into(),
            members: Vec::new(),
            events: VecDeque::with_capacity(REPLAY_CAP),
        }
    }

    /// 成员加入（幂等，重复加入返回 false）
    pub fn join(&mut self, member_id: &str) -> bool {
        if self.members.iter().any(|m| m == member_id) {
            return false;
        }
        self.members.push(member_id.into());
        // 广播一条成员加入事件，便于其他成员右栏感知
        self.broadcast(Envelope::new(
            EventType::Status,
            serde_json::json!({ "kind": "collab.member_joined", "member": member_id }),
            &self.id,
        ));
        true
    }

    /// 成员离开（返回是否曾在线）
    pub fn leave(&mut self, member_id: &str) -> bool {
        let before = self.members.len();
        self.members.retain(|m| m != member_id);
        before != self.members.len()
    }

    /// 在线成员数
    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    /// 在线成员 ID 列表（克隆，供视图层渲染）
    pub fn members(&self) -> Vec<String> {
        self.members.clone()
    }

    /// 广播事件（写入回放缓冲，超出上限按 FIFO 丢弃最早）
    pub fn broadcast(&mut self, env: Envelope) {
        self.events.push_back(env);
        while self.events.len() > REPLAY_CAP {
            self.events.pop_front();
        }
    }

    /// 回放最近事件（克隆，供新成员追赶）
    pub fn replay(&self) -> Vec<Envelope> {
        self.events.iter().cloned().collect()
    }

    /// 回放缓冲长度
    pub fn buffered(&self) -> usize {
        self.events.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventType;

    #[test]
    fn test_join_idempotent_and_broadcasts_join_event() {
        let mut s = CollabSession::new("ws1");
        assert!(s.join("u1"));
        assert!(!s.join("u1")); // 重复加入幂等
        assert_eq!(s.member_count(), 1);
        // join 会产生一条 collab.member_joined 事件进入缓冲
        assert!(s.buffered() >= 1);
        let last = s.replay().last().unwrap().clone();
        assert_eq!(last.r#type, EventType::Status);
        assert_eq!(last.payload["member"], "u1");
    }

    #[test]
    fn test_leave() {
        let mut s = CollabSession::new("ws1");
        s.join("u1");
        s.join("u2");
        assert_eq!(s.member_count(), 2);
        assert!(s.leave("u1"));
        assert!(!s.leave("u1")); // 已不在
        assert_eq!(s.member_count(), 1);
    }

    #[test]
    fn test_broadcast_and_replay() {
        let mut s = CollabSession::new("ws1");
        for i in 0..3 {
            s.broadcast(Envelope::new(
                EventType::Thinking,
                serde_json::json!({ "i": i }),
                "ws1",
            ));
        }
        let r = s.replay();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].payload["i"], 0);
        assert_eq!(r[2].payload["i"], 2);
    }

    #[test]
    fn test_replay_buffer_capped() {
        let mut s = CollabSession::new("ws1");
        for i in 0..(REPLAY_CAP + 50) {
            s.broadcast(Envelope::new(
                EventType::Thinking,
                serde_json::json!({ "i": i }),
                "ws1",
            ));
        }
        assert_eq!(s.buffered(), REPLAY_CAP);
        // 最早被丢弃，回放头应为 i=50
        assert_eq!(s.replay()[0].payload["i"], 50);
    }
}
