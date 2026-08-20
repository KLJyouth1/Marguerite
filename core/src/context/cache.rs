//! 摘要缓存
//!
//! LRU 缓存 + 内容 hash 键，避免重复摘要计算，缓存命中率统计。

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
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
    /// 代数：每次访问刷新时递增，用于惰性丢弃淘汰堆里的过期节点
    gen: u64,
}

/// 摘要缓存（LRU）
///
/// 淘汰顺序由 `BinaryHeap` 按访问时间维护，`evict_one` 为 O(log n) 摊还
/// （旧实现每次淘汰 O(n) 全表扫描）；堆节点经 `gen` 校验惰性失效。
pub struct SummaryCache {
    entries: HashMap<u64, CacheEntry>,
    /// (访问时间, key, gen) 最小堆：堆顶 = 最久未访问
    lru_heap: BinaryHeap<(Reverse<Instant>, u64, u64)>,
    next_gen: u64,
    max_entries: usize,
    ttl: Duration,
    pub stats: CacheStats,
}

impl SummaryCache {
    pub fn new(max_entries: usize, ttl_secs: u64) -> Self {
        Self {
            entries: HashMap::new(),
            lru_heap: BinaryHeap::new(),
            next_gen: 0,
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
            // TTL 过期：立即移除陈旧条目（旧实现只返回 None，条目泄漏且阻塞同键重写）
            self.entries.remove(&key);
            self.stats.misses += 1;
            return None;
        }
        self.stats.hits += 1;
        // 刷新 LRU：代数递增，旧堆节点在后续淘汰时被惰性丢弃
        let mut entry = self.entries.remove(&key)?;
        entry.accessed = Instant::now();
        entry.gen = self.next_gen;
        self.next_gen += 1;
        let summary = entry.summary.clone();
        let heap_node = (Reverse(entry.accessed), key, entry.gen);
        self.entries.insert(key, entry);
        self.lru_heap.push(heap_node);
        Some(summary)
    }

    /// 写入摘要（超容量时淘汰最久未访问项）
    pub fn put(&mut self, key: u64, summary: String) {
        if let Some(existing) = self.entries.get(&key) {
            if existing.accessed.elapsed() <= self.ttl {
                return; // 内容寻址：同键新鲜条目直接复用
            }
            // 过期条目：先移除再重写（旧实现被 contains_key 挡死，无法刷新）
            self.entries.remove(&key);
        }
        if self.entries.len() >= self.max_entries {
            self.evict_one();
        }
        let now = Instant::now();
        let gen = self.next_gen;
        self.next_gen += 1;
        self.entries.insert(
            key,
            CacheEntry {
                summary,
                accessed: now,
                gen,
            },
        );
        self.lru_heap.push((Reverse(now), key, gen));
    }

    /// 淘汰最久未访问项（O(log n) 摊还）
    fn evict_one(&mut self) {
        while let Some((_, key, gen)) = self.lru_heap.pop() {
            let Some(entry) = self.entries.get(&key) else {
                continue; // 已被 TTL 移除的陈旧节点
            };
            if entry.gen != gen {
                continue; // 已被访问刷新，堆中此节点过期
            }
            self.entries.remove(&key);
            self.stats.evictions += 1;
            return;
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
        self.lru_heap.clear();
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

    #[test]
    fn test_expired_entry_removed_and_reinsertable() {
        // ttl=0：任何条目一经访问即过期。get 必须移除条目（旧实现泄漏），
        // 且 put 允许同键重写（旧实现被 contains_key 挡死）。
        let mut cache = SummaryCache::new(10, 0);
        let key = 7;
        cache.put(key, "old".to_string());
        assert_eq!(cache.len(), 1);
        assert!(cache.get(key).is_none(), "ttl=0 时应判定过期");
        assert_eq!(cache.len(), 0, "过期条目必须被移除，不得泄漏");
        cache.put(key, "new".to_string());
        assert_eq!(cache.len(), 1, "同键过期后允许重新写入");
        assert_eq!(cache.stats.misses, 1);
    }

    #[test]
    fn test_eviction_ignores_stale_heap_nodes() {
        // 关键回归：get 刷新后旧堆节点必须因 gen 不匹配被跳过，
        // 否则会误淘汰刚访问过的条目（LRU 语义被破坏）。
        let mut cache = SummaryCache::new(3, 60);
        cache.put(1, "a".to_string());
        cache.put(2, "b".to_string());
        cache.put(3, "c".to_string());
        assert!(cache.get(1).is_some()); // 刷新 key=1 → 新 gen，旧堆节点失效
        cache.put(4, "d".to_string()); // 触发淘汰
        assert!(cache.get(1).is_some(), "刚访问的 key=1 不得被误淘汰");
        assert!(cache.get(2).is_none(), "真正的 LRU（key=2）应被淘汰");
        assert_eq!(cache.stats.evictions, 1);
    }
}
