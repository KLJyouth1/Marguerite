//! 摘要缓存
//!
//! LRU 缓存 + 内容 hash 键，避免重复摘要计算，缓存命中率统计。

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// 缓存统计
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheStats {
    /// 命中次数
    pub hits: u64,
    /// 未命中次数
    pub misses: u64,
    /// 缓存淘汰次数
    pub evictions: u64,
}

impl CacheStats {
    /// 命中率
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// 缓存条目
struct CacheEntry {
    summary: String,
    accessed: Instant,
}

/// 摘要缓存（LRU）
pub struct SummaryCache {
    entries: HashMap<u64, CacheEntry>,
    max_entries: usize,
    ttl: Duration,
    pub stats: CacheStats,
}

impl SummaryCache {
    pub fn new(max_entries: usize, ttl_secs: u64) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries,
            ttl: Duration::from_secs(ttl_secs),
            stats: CacheStats::default(),
        }
    }

    /// 计算内容的 hash 键
    pub fn content_hash(content: &str) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        content.hash(&mut hasher);
        hasher.finish()
    }

    /// 获取摘要（命中则刷新访问时间）
    pub fn get(&mut self, key: u64) -> Option<String> {
        let Some(entry) = self.entries.get(&key) else {
            self.stats.misses += 1;
            return None;
        };
        if entry.accessed.elapsed() > self.ttl {
            self.stats.misses += 1;
            return None;
        }
        self.stats.hits += 1;
        // 刷新 LRU
        let mut entry = self.entries.remove(&key)?;
        entry.accessed = Instant::now();
        let summary = entry.summary.clone();
        self.entries.insert(key, entry);
        Some(summary)
    }

    /// 写入摘要（超容量时淘汰最久未访问项）
    pub fn put(&mut self, key: u64, summary: String) {
        if self.entries.contains_key(&key) {
            return;
        }
        if self.entries.len() >= self.max_entries {
            self.evict_one();
        }
        self.entries.insert(
            key,
            CacheEntry {
                summary,
                accessed: Instant::now(),
            },
        );
    }

    /// 淘汰最久未访问项
    fn evict_one(&mut self) {
        if let Some(oldest) = self
            .entries
            .iter()
            .min_by_key(|(_, e)| e.accessed)
            .map(|(k, _)| *k)
        {
            self.entries.remove(&oldest);
            self.stats.evictions += 1;
        }
    }

    /// 当前条目数
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 清空
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_get_put() {
        let mut cache = SummaryCache::new(10, 60);
        let key = SummaryCache::content_hash("hello world");
        assert!(cache.get(key).is_none());
        cache.put(key, "summary".to_string());
        assert_eq!(cache.get(key).unwrap(), "summary");
    }

    #[test]
    fn test_cache_stats() {
        let mut cache = SummaryCache::new(10, 60);
        let key = 42;
        cache.get(key);
        cache.put(key, "s".to_string());
        cache.get(key);
        assert_eq!(cache.stats.hits, 1);
        assert_eq!(cache.stats.misses, 1);
        assert!((cache.stats.hit_rate() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cache_hit_rate_measurable() {
        // 可度量命中率：插入 3 项 → 查询 2 命中 1 未命中 → 命中率 = 命中/总数
        let mut cache = SummaryCache::new(10, 60);
        cache.put(1, "a".to_string());
        cache.put(2, "b".to_string());
        cache.put(3, "c".to_string());
        assert!(cache.get(1).is_some());
        assert!(cache.get(2).is_some());
        assert!(cache.get(99).is_none());
        assert_eq!(cache.stats.hits, 2);
        assert_eq!(cache.stats.misses, 1);
        let total = cache.stats.hits + cache.stats.misses;
        assert_eq!(total, 3);
        assert!((cache.stats.hit_rate() - (2.0 / 3.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cache_eviction() {
        let mut cache = SummaryCache::new(2, 60);
        cache.put(1, "a".to_string());
        cache.put(2, "b".to_string());
        // 访问 key=1 使其成为最近使用
        cache.get(1);
        cache.put(3, "c".to_string());
        // key=2 应被淘汰
        assert!(cache.get(2).is_none());
        assert!(cache.get(1).is_some());
        assert!(cache.get(3).is_some());
        assert_eq!(cache.stats.evictions, 1);
    }

    #[test]
    fn test_content_hash_deterministic() {
        let a = SummaryCache::content_hash("same content");
        let b = SummaryCache::content_hash("same content");
        let c = SummaryCache::content_hash("different");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
