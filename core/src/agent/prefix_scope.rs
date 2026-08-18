//! 团队 / 同 agent 多轮共享的稳定前缀缓存（KV-Cache 原则 P1「共享不变前缀」落地）
//!
//! 同 agent 的多轮对话、团队内所有成员，system+KB 相同 → 稳定前缀哈希相同 →
//! 共享同一个 [`PrefixCache`] 即自动命中（块级 KV 复用，`prefix_hits` 单调上升）。
//!
//! 复用 [`crate::context::optimize`] 的 `reorder_stable_prefix` 半区逻辑：仅 System 段参与
//! 前缀键，volatile（user/assistant/tool）被排除，保证键逐字节稳定。
//!
//! 蒸馏意图：把蓝图 §2.6「Agent 级 cache 共享（同 agent 多轮复用 system 前缀）」与
//! §2.7「团队内共享 stable 前缀缓存」合并为一个可注入的作用域原语。

use std::sync::Arc;

use crate::cache::{hash_text, PrefixCache};
use crate::context::optimize::{stable_prefix_text, stable_prefix_tokens};
use crate::types::Message;

/// 计算消息序列的稳定前缀键（仅 System 段，逐字节稳定 → 同前缀同键）。
pub fn stable_prefix_key(messages: &[Message]) -> u64 {
    hash_text(&stable_prefix_text(messages))
}

/// 一个可跨成员 / 多轮共享的稳定前缀缓存作用域。
///
/// 把 `Arc<PrefixCache>` 注入团队或 agent，每次请求前 `observe` 一次稳定前缀，
/// 即把 P1 的「不变前缀前置」杠杆放大为可观测的命中率复用。
#[derive(Clone, Debug)]
pub struct SharedPrefixCache {
    cache: Arc<PrefixCache>,
}

impl SharedPrefixCache {
    pub fn new(cache: Arc<PrefixCache>) -> Self {
        Self { cache }
    }

    pub fn cache(&self) -> &Arc<PrefixCache> {
        &self.cache
    }

    /// 观测一次稳定前缀：相同前缀的后续请求记为复用（`prefix_hits++`），返回是否发生复用。
    ///
    /// 复用 `context::optimize` 的稳定前缀半区与 token 估算；把 P1「不变前缀前置」杠杆
    /// 放大为可观测的命中率复用——共享同一 `PrefixCache` 的团队/多轮请求即可自动命中。
    pub fn observe(&self, messages: &[Message], family: f64) -> bool {
        let key = stable_prefix_key(messages);
        let tokens = stable_prefix_tokens(messages, family) as u32;
        self.cache.note_prefix(key, tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sys(content: &str) -> Message {
        Message::System {
            content: content.to_string(),
            timestamp: Utc::now(),
        }
    }

    fn shared_cache() -> SharedPrefixCache {
        SharedPrefixCache::new(Arc::new(PrefixCache::new(16)))
    }

    #[test]
    fn stable_prefix_key_ignores_volatile() {
        // 仅 System 段参与键；volatile 内容不同但前缀相同 → 键相同。
        let a = vec![
            sys("common system"),
            Message::User {
                content: "x".into(),
                timestamp: Utc::now(),
            },
        ];
        let b = vec![
            sys("common system"),
            Message::User {
                content: "y".into(),
                timestamp: Utc::now(),
            },
        ];
        assert_eq!(stable_prefix_key(&a), stable_prefix_key(&b));
    }

    #[test]
    fn stable_prefix_key_differs_on_system_change() {
        let a = vec![sys("system A")];
        let b = vec![sys("system B")];
        assert_ne!(stable_prefix_key(&a), stable_prefix_key(&b));
    }

    #[test]
    fn observe_records_reuse_on_repeat() {
        let s = shared_cache();
        let msgs = vec![sys("shared kb + persona")];
        assert!(!s.observe(&msgs, 1.0), "首次应记为新前缀");
        assert!(s.observe(&msgs, 1.0), "重复应记为前缀复用");
        let stats = s.cache().stats();
        assert_eq!(stats.prefix_hits, 1, "prefix_hits 应单调上升");
    }

    #[test]
    fn team_members_with_same_system_share_prefix() {
        // 同一团队两个成员若 system 相同，共享缓存即自动复用（跨成员命中）。
        let s = shared_cache();
        let m1 = vec![sys("你是团队共享的检索专家")];
        let m2 = vec![sys("你是团队共享的检索专家")];
        assert!(!s.observe(&m1, 1.0));
        assert!(s.observe(&m2, 1.0), "同系统前缀的第二个成员应复用");
    }

    #[test]
    fn distinct_systems_do_not_reuse() {
        let s = shared_cache();
        assert!(!s.observe(&[sys("A")], 1.0));
        assert!(!s.observe(&[sys("B")], 1.0), "不同 system 不应复用");
        assert_eq!(s.cache().stats().prefix_hits, 0);
    }
}
