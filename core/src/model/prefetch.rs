//! W-9 请求级去重预填充（输出优化 + 命中率优化）
//!
//! ## 为什么需要它
//!
//! 蓝图 §2.7 点名「同 shard 多专家并行时，对相同子任务只预填充一次（软件可得，未入 P-12）」、
//! §2.5 点名「批处理：相似请求合并预填充」。多个并发请求若共享同一稳定前缀（system/persona/KB），
//! 各自 `warmup` 会对**同一前缀重复预填充**，重复烧掉预填充的 prompt 与无效输出 token。
//!
//! ## 设计
//!
//! - **去重**：`PrefetchDedup` 维护「稳定前缀哈希 → 最近预热时间」的集合。窗口（TTL）内
//!   再次遇到同前缀即**跳过**预填充（计入 `deduped`），否则真正发一次 `warmup`（计入 `prefetches`）。
//! - **命中率**：相同前缀只预热一次，后续请求直接复用服务端已填充的 KV 块（`prefix_hits` 上升）。
//! - **输出优化**：省去重复预填充产生的无效输出 token（`max_tokens=1` 的空调用也非零成本）。
//! - **懒清理**：预热时顺手剔除过期条目，集合不无限膨胀。
//! - **诚实**：跳过是「窗口内已预热」的事实，如实计入 `deduped`，不冒充命中。
//! - **尽力而为**：预热失败静默忽略（`warmup` 已保证），去重层不改变这一点。

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::model::llm::LLMClient;
use crate::types::Message;

/// 进程级真实计数：实际发出的预填充次数（跨 `PrefetchDedup` 实例累计）。
static PREFETCH_WARMUPS: AtomicUsize = AtomicUsize::new(0);
/// 进程级真实计数：因窗口内已预热而跳过（去重）的次数。
static PREFETCH_DEDUP_SKIPS: AtomicUsize = AtomicUsize::new(0);

/// 读取进程级预填充真实计数 `(warmups, dedup_skips)`。
///
/// 供 `cache.status` 如实上报；无任何调用时返回 `(0, 0)`，绝不编造。
pub fn prefetch_counters() -> (u64, u64) {
    (
        PREFETCH_WARMUPS.load(Ordering::Relaxed) as u64,
        PREFETCH_DEDUP_SKIPS.load(Ordering::Relaxed) as u64,
    )
}

/// 提取消息正文（去重 key 用）。稳定前缀只含 System 消息，故取 content 即可。
fn message_content(m: &Message) -> &str {
    match m {
        Message::User { content, .. }
        | Message::Assistant { content, .. }
        | Message::System { content, .. } => content,
        Message::Tool { input, .. } => input,
    }
}

/// 去重预填充的默认窗口（TTL）：窗口内同前缀不重复预热。
pub const DEFAULT_PREFETCH_TTL: Duration = Duration::from_secs(300);

/// 去重预填充统计（如实计量）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PrefetchStats {
    /// 实际发出的预填充请求数
    pub prefetches: u64,
    /// 因窗口内已预热而跳过的（去重）次数
    pub deduped: u64,
}

impl PrefetchStats {
    /// 去重率 = 跳过 / 总请求（含跳过）。无请求时返回 0。
    pub fn dedup_ratio(&self) -> f64 {
        let total = self.prefetches + self.deduped;
        if total == 0 {
            0.0
        } else {
            self.deduped as f64 / total as f64
        }
    }
}

/// 请求级去重预填充：对相同稳定前缀只预填充一次（窗口内）。
///
/// 线程安全（`&self` 方法），可注入共享到多专家/多成员的并行预填充层。
#[derive(Debug)]
pub struct PrefetchDedup {
    client: LLMClient,
    /// 全局去重：稳定前缀哈希 → 最近预热时间（TTL 窗口语义）
    warmed: Mutex<HashMap<String, Instant>>,
    /// 会话级去重：`session|前缀` 集合（无 TTL，会话生命周期内恒去重；容量上限见
    /// [`SESSION_DEDUP_CAP`]）
    session_warmed: Mutex<std::collections::HashSet<String>>,
    /// 去重窗口（TTL）
    ttl: Duration,
    stats: Mutex<PrefetchStats>,
}

/// 会话级去重键的容量上限（超出即整体清空——进程级 LRU 近似，防无界膨胀）。
pub const SESSION_DEDUP_CAP: usize = 4096;

impl PrefetchDedup {
    /// 创建去重预填充器。
    pub fn new(client: LLMClient, ttl: Duration) -> Self {
        Self {
            client,
            warmed: Mutex::new(HashMap::new()),
            session_warmed: Mutex::new(std::collections::HashSet::new()),
            ttl,
            stats: Mutex::new(PrefetchStats::default()),
        }
    }

    /// 默认窗口（`DEFAULT_PREFETCH_TTL`）。
    pub fn with_default_ttl(client: LLMClient) -> Self {
        Self::new(client, DEFAULT_PREFETCH_TTL)
    }

    /// 对稳定前缀做一次去重预填充：
    /// - 无稳定前缀（无 system 消息）→ 直接返回，不发请求。
    /// - 窗口内同前缀已预热 → 计入 `deduped` 并跳过。
    /// - 否则发一次 `warmup`，记录前缀与时间，计入 `prefetches`。
    ///
    /// 预热失败静默忽略（尽力而为，不阻塞调用方）。
    pub async fn warmup_once(&self, messages: &[Message]) -> Result<()> {
        // 兼容入口：无会话维度（全局窗口语义保持既有行为）
        self.warmup_once_for_session("", messages).await
    }

    /// 会话级去重预填充（v10：窗口对齐会话生命周期）：
    ///
    /// 键 = `session | 稳定前缀`。**会话维度的去重不受 TTL 约束**——同一会话的
    /// 稳定前缀在会话生命周期内只预热一次，服务端 KV 块在整个会话中持续复用
    /// （对齐验收报告 §4 的剩余边界：把去重窗口从固定 5 分钟拉长为会话全程）。
    /// `session` 为空时回退全局 TTL 窗口语义（向后兼容）。
    ///
    /// 内存有界：会话键集合超过 `SESSION_DEDUP_CAP` 时整体清空（进程级 LRU 近似），
    /// 绝不无限膨胀。
    pub async fn warmup_once_for_session(&self, session: &str, messages: &[Message]) -> Result<()> {
        let ordered = crate::context::optimize::reorder_stable_prefix(messages);
        // M2 降耗引擎：稳定前缀 = leading run（开头连续 System 消息）。mid-sequence
        // System 是装配层刻意放置的逐轮变化段（记忆召回 / 经验提示 / Track A 片段，
        // 见 `agent::runtime::build_model_messages` 动态区），不得进入预热去重键——
        // 否则键逐轮漂移，会话级去重失效、预热退化为每轮一次。
        let stable: Vec<Message> = ordered
            .into_iter()
            .take_while(|m| matches!(m, Message::System { .. }))
            .collect();
        if stable.is_empty() {
            return Ok(());
        }
        // 稳定前缀标识：按 (role:content) 逐条拼接，去重判定只看前缀本体是否一致
        let key = stable
            .iter()
            .map(|m| format!("{}:{}", m.role(), message_content(m)))
            .collect::<Vec<_>>()
            .join("|");
        let now = Instant::now();

        if !session.is_empty() {
            // 会话生命周期去重：无 TTL，仅受容量上限约束
            let session_key = format!("{session}|{key}");
            {
                let mut seen = self
                    .session_warmed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if seen.len() >= SESSION_DEDUP_CAP {
                    seen.clear();
                }
                if !seen.insert(session_key) {
                    let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
                    s.deduped = s.deduped.saturating_add(1);
                    PREFETCH_DEDUP_SKIPS.fetch_add(1, Ordering::Relaxed);
                    return Ok(());
                }
            }
            // 真正预热
            let _ = self.client.warmup(messages).await;
            let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
            s.prefetches = s.prefetches.saturating_add(1);
            PREFETCH_WARMUPS.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }

        // 窗口内已预热 → 跳过（去重命中）
        {
            let warmed = self.warmed.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(t) = warmed.get(&key) {
                if now.duration_since(*t) < self.ttl {
                    let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
                    s.deduped = s.deduped.saturating_add(1);
                    PREFETCH_DEDUP_SKIPS.fetch_add(1, Ordering::Relaxed);
                    return Ok(());
                }
            }
        }

        // 真正预热
        let _ = self.client.warmup(messages).await;

        // 懒清理过期条目，再记录本次
        let mut warmed = self.warmed.lock().unwrap_or_else(|e| e.into_inner());
        warmed.retain(|_, t| now.duration_since(*t) < self.ttl);
        warmed.insert(key, now);
        let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        s.prefetches = s.prefetches.saturating_add(1);
        PREFETCH_WARMUPS.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// 当前统计快照。
    pub fn stats(&self) -> PrefetchStats {
        self.stats.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::llm::LLMConfig;
    use chrono::Utc;

    fn sys(s: &str) -> Message {
        Message::System {
            content: s.to_string(),
            timestamp: Utc::now(),
        }
    }

    fn user(s: &str) -> Message {
        Message::User {
            content: s.to_string(),
            timestamp: Utc::now(),
        }
    }

    fn client() -> LLMClient {
        LLMClient::new(LLMConfig {
            base_url: "http://127.0.0.1:1".to_string(), // 不可达——无 system 时不触发请求
            api_key: "k".to_string(),
            timeout_secs: 1,
            ..Default::default()
        })
    }

    #[tokio::test]
    async fn warmup_once_noops_without_system() {
        let dedup = PrefetchDedup::new(client(), Duration::from_secs(60));
        dedup
            .warmup_once(&[user("hi")])
            .await
            .expect("无稳定前缀应直接返回");
        let s = dedup.stats();
        assert_eq!(s.prefetches, 0);
        assert_eq!(s.deduped, 0);
    }

    #[tokio::test]
    async fn warmup_once_dedups_identical_prefix_within_window() {
        // 不可达地址 + 有 system 前缀 → warmup 会失败,但 PrefetchDedup 容忍失败并记录。
        // 这里用足够短的 TTL 观察去重逻辑;失败的预填充仍计入 prefetches(尽力而为)。
        let dedup = PrefetchDedup::new(client(), Duration::from_secs(60));
        let msgs = vec![sys("persona"), user("q1")];
        // 第一次: 尝试预热(可能失败,不 panic)
        dedup.warmup_once(&msgs).await.expect("尽力而为不报错");
        // 第二次: 同前缀窗口内 → 跳过
        dedup.warmup_once(&msgs).await.expect("尽力而为不报错");
        let s = dedup.stats();
        assert!(s.deduped >= 1, "同前缀窗口内应去重, got {s:?}");
        assert_eq!(s.prefetches, 1, "同前缀只预热一次, got {s:?}");
    }

    #[tokio::test]
    async fn warmup_once_does_not_dedup_distinct_prefixes() {
        let dedup = PrefetchDedup::new(client(), Duration::from_secs(60));
        dedup
            .warmup_once(&[sys("persona-a"), user("q")])
            .await
            .unwrap();
        dedup
            .warmup_once(&[sys("persona-b"), user("q")])
            .await
            .unwrap();
        let s = dedup.stats();
        assert_eq!(s.deduped, 0, "不同前缀不应去重, got {s:?}");
        assert_eq!(s.prefetches, 2, "不同前缀各预热一次, got {s:?}");
    }

    #[test]
    fn dedup_ratio_math() {
        let a = PrefetchStats {
            prefetches: 1,
            deduped: 3,
        };
        assert!((a.dedup_ratio() - 0.75).abs() < 1e-9);
        let empty = PrefetchStats::default();
        assert_eq!(empty.dedup_ratio(), 0.0);
    }

    #[tokio::test]
    async fn session_dedup_ignores_ttl_and_scopes_by_session() {
        // 极短 TTL：全局窗口转瞬过期；会话维度无 TTL —— 同会话恒去重
        let dedup = PrefetchDedup::new(client(), Duration::from_millis(1));
        let msgs = vec![sys("persona"), user("q")];
        dedup
            .warmup_once_for_session("s1", &msgs)
            .await
            .expect("尽力而为不报错");
        dedup
            .warmup_once_for_session("s1", &msgs)
            .await
            .expect("尽力而为不报错");
        let s = dedup.stats();
        assert_eq!(s.prefetches, 1, "同会话同前缀恒去重（无视 TTL）: {s:?}");
        assert!(s.deduped >= 1);
        // 不同会话同前缀 → 各自预热一次（会话隔离）
        dedup
            .warmup_once_for_session("s2", &msgs)
            .await
            .expect("尽力而为不报错");
        let s2 = dedup.stats();
        assert_eq!(s2.prefetches, 2, "不同会话各预热一次: {s2:?}");
        // 空会话回退全局 TTL 语义：与 s1/s2 互不影响
        dedup.warmup_once(&msgs).await.expect("尽力而为不报错");
        let s3 = dedup.stats();
        assert_eq!(s3.prefetches, 3, "无会话维度仍各自预热: {s3:?}");
    }

    #[tokio::test]
    async fn session_dedup_bounded_capacity_survives_clear() {
        let dedup = PrefetchDedup::new(client(), Duration::from_secs(60));
        // 直接注满会话键集合（绕过真实预热网络路径，容量语义纯逻辑可测）
        {
            let mut seen = dedup
                .session_warmed
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            for i in 0..SESSION_DEDUP_CAP {
                seen.insert(format!("seed{i}|p"));
            }
        }
        let msgs = vec![sys("final"), user("q")];
        dedup
            .warmup_once_for_session("s-final", &msgs)
            .await
            .expect("尽力而为不报错");
        dedup
            .warmup_once_for_session("s-final", &msgs)
            .await
            .expect("尽力而为不报错");
        assert!(dedup.stats().deduped >= 1, "容量超限整体清空后仍能正常去重");
    }
}
