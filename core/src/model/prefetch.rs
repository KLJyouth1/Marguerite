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
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::model::llm::LLMClient;
use crate::types::Message;

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
    /// 稳定前缀哈希 → 最近预热时间
    warmed: Mutex<HashMap<String, Instant>>,
    /// 去重窗口（TTL）
    ttl: Duration,
    stats: Mutex<PrefetchStats>,
}

impl PrefetchDedup {
    /// 创建去重预填充器。
    pub fn new(client: LLMClient, ttl: Duration) -> Self {
        Self {
            client,
            warmed: Mutex::new(HashMap::new()),
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
        let ordered = crate::context::optimize::reorder_stable_prefix(messages);
        let stable: Vec<Message> = ordered
            .into_iter()
            .filter(|m| matches!(m, Message::System { .. }))
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

        // 窗口内已预热 → 跳过（去重命中）
        {
            let warmed = self.warmed.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(t) = warmed.get(&key) {
                if now.duration_since(*t) < self.ttl {
                    let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
                    s.deduped = s.deduped.saturating_add(1);
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
}
