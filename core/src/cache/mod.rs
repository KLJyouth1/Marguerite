//! KV-Cache 极简工程实践：客户端前缀/语义缓存与命中率计量。
//!
//! 设计原则（源自 KV Cache 研究，详见 `deliverables/KV-CACHE-OPTIMIZATION-BLUEPRINT.md`）：
//!
//! 1. **缓存不变前缀**。系统提示 / 人设 / 静态知识库是请求中稳定的部分，应作为缓存键的
//!    前缀；时间戳、会话 id、动态采样等易变数据必须放在后缀，否则前缀永远不命中。
//! 2. **精确匹配优先**。先走 hash 精确命中（诚实、零假阳性）；语义层作为可选增强，不喧宾夺主。
//! 3. **命中率可观测**。没有 hit-rate 指标的缓存是黑箱——本模块内置 [`CacheStats`]，
//!    并把"省下的 prompt token"如实计入用量台账，绝不虚报节省。
//!
//! 本模块零外部依赖（仅 `std`），可直接被 `model::llm` 与上层调度复用。

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub mod semantic;

/// FNV-1a 64 位哈希：快、无依赖、足够做缓存键（精确命中不需要密码学强度）。
pub fn hash_text(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 缓存可观测性：命中率与"省下 token"比例，没有它缓存就是黑箱。
#[derive(Clone, Copy, Debug, Default)]
pub struct CacheStats {
    /// 总查询次数（命中 + 未命中）
    pub lookups: u64,
    /// 精确命中次数
    pub hits: u64,
    /// 未命中次数
    pub misses: u64,
    /// 已被缓存的 prompt token 总量（写入侧）
    pub stored_prompt_tokens: u64,
    /// 命中缓存而省下的 prompt token 总量（读取侧）
    pub saved_prompt_tokens: u64,
    /// 稳定前缀（KV-cache 不变前缀）被复用次数（服务端 KV 块复用，非客户端命中）
    pub prefix_hits: u64,
    /// 稳定前缀复用而省下的 prompt token 总量（服务端不重复预填充的那部分）
    pub prefix_saved_tokens: u64,
    /// 服务端前缀缓存命中 token（DeepSeek `prompt_cache_hit_tokens` 归一）——诚实计量，不虚报
    pub server_cache_read_tokens: u64,
    /// 服务端前缀缓存写入 token（首次为该前缀建立缓存）
    pub server_cache_creation_tokens: u64,
}

impl CacheStats {
    /// 记录一次命中（saved = 该请求原本的 prompt token 数）
    pub fn hit(&mut self, saved_prompt_tokens: u32) {
        self.lookups += 1;
        self.hits += 1;
        self.saved_prompt_tokens += saved_prompt_tokens as u64;
    }

    /// 记录一次未命中（仅计数；stored 在 put 时累加）
    pub fn miss(&mut self) {
        self.lookups += 1;
        self.misses += 1;
    }

    /// 记录一次稳定前缀复用（KV-cache 原则 P1：不变前缀的 KV 块在服务端被复用，
    /// 无需重新预填充）。`saved_prefix_tokens` 为该前缀的 token 数（客户端如实计量，不虚报命中）。
    pub fn record_prefix_reuse(&mut self, saved_prefix_tokens: u32) {
        self.prefix_hits += 1;
        self.prefix_saved_tokens += saved_prefix_tokens as u64;
    }

    /// 记录一次服务端前缀缓存命中/写入（诚实计量：仅当 DeepSeek 真实返回 cache 字段时调用）。
    pub fn record_server_cache(&mut self, read_tokens: u32, creation_tokens: u32) {
        self.server_cache_read_tokens += read_tokens as u64;
        self.server_cache_creation_tokens += creation_tokens as u64;
    }

    /// 服务端前缀缓存命中率 ∈ [0, 1]：命中 token / (命中 + 写入) token。
    /// 反映 DeepSeek 原生 prompt caching 的真实复用程度（不是客户端精确缓存）。
    pub fn server_cache_hit_rate(&self) -> f64 {
        let total = self.server_cache_read_tokens + self.server_cache_creation_tokens;
        if total == 0 {
            0.0
        } else {
            self.server_cache_read_tokens as f64 / total as f64
        }
    }

    /// 命中率 ∈ [0, 1]
    pub fn hit_rate(&self) -> f64 {
        if self.lookups == 0 {
            0.0
        } else {
            self.hits as f64 / self.lookups as f64
        }
    }

    /// 复用率：被命中复用的 token 占（写入 + 复用）总量的比例 ∈ [0, 1]
    pub fn reuse_ratio(&self) -> f64 {
        let total = self.stored_prompt_tokens + self.saved_prompt_tokens;
        if total == 0 {
            0.0
        } else {
            self.saved_prompt_tokens as f64 / total as f64
        }
    }
}

/// 单条缓存条目
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheEntry {
    pub content: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub last_used_ms: u64,
}

/// 落盘载荷（W-7b）：仅持久化可恢复的部分（条目 + 已观测的稳定前缀集合），
/// 统计字段在加载时按已存条目重新累加，避免重复计数。
#[derive(Serialize, Deserialize)]
pub(crate) struct PersistedCache {
    map: HashMap<u64, CacheEntry>,
    prefix_seen: std::collections::HashSet<u64>,
}

#[derive(Debug)]
struct Inner {
    map: HashMap<u64, CacheEntry>,
    stats: CacheStats,
    /// 已观察到的稳定前缀哈希集合（KV-cache 不变前缀复用观测，用于命中率可观测）
    prefix_seen: std::collections::HashSet<u64>,
}

/// 客户端前缀缓存：精确哈希命中 + LRU 驱逐 + 内置命中率计量。
///
/// 线程安全（`Mutex` 包裹），可包进 `Arc` 在异步任务间共享。
#[derive(Debug)]
pub struct PrefixCache {
    inner: Mutex<Inner>,
    capacity: usize,
}

impl PrefixCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                map: HashMap::new(),
                stats: CacheStats::default(),
                prefix_seen: std::collections::HashSet::new(),
            }),
            capacity: capacity.max(1),
        }
    }

    /// 精确命中查询；命中则更新 LRU 时间戳并计入 stats，否则计入一次 miss。
    pub fn get(&self, key: u64) -> Option<CacheEntry> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        // 先以不可变借用克隆出条目，释放 map 借用后再更新 stats（避免双重可变借用）
        let hit = g.map.get(&key).cloned();
        match hit {
            Some(mut e) => {
                g.stats.hit(e.prompt_tokens);
                e.last_used_ms = now_ms();
                Some(e)
            }
            None => {
                g.stats.miss();
                None
            }
        }
    }

    /// 写入缓存；累加 stored token；超出容量时驱逐最久未用条目（LRU）。
    pub fn put(&self, key: u64, entry: CacheEntry) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.stats.stored_prompt_tokens += entry.prompt_tokens as u64;
        g.map.insert(key, entry);
        if g.map.len() > self.capacity {
            if let Some(evict) = g
                .map
                .iter()
                .min_by_key(|(_, v)| v.last_used_ms)
                .map(|(k, _)| *k)
            {
                g.map.remove(&evict);
            }
        }
    }

    /// 观测一次稳定前缀：若该前缀首次出现返回 false 并记录；若已出现过返回 true 并计入前缀复用。
    ///
    /// 这是 KV-Cache 原则 P1 的客户端观测：相同稳定前缀的请求会让服务端复用已计算的 KV 块
    /// （prompt caching），无需重新预填充。`prefix_tokens` 为该前缀 token 数，如实计入
    /// `CacheStats.prefix_saved_tokens`，不冒充为「客户端命中」（那需要逐字节一致的全请求）。
    pub fn note_prefix(&self, hash: u64, prefix_tokens: u32) -> bool {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.prefix_seen.contains(&hash) {
            g.stats.record_prefix_reuse(prefix_tokens);
            true
        } else {
            g.prefix_seen.insert(hash);
            false
        }
    }

    /// 记录一次服务端前缀缓存命中/写入（诚实计量：仅当 DeepSeek 真实返回 cache 字段时调用）。
    pub fn record_server_cache(&self, read_tokens: u32, creation_tokens: u32) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.stats.record_server_cache(read_tokens, creation_tokens);
    }

    pub fn stats(&self) -> CacheStats {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).stats
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 从磁盘加载冷上下文缓存（W-7b）：读取 JSON 文件并恢复条目与已观测稳定前缀集合。
    ///
    /// 加载失败（文件不存在 / 解析错误 / 磁盘只读）时**静默回退为空缓存**，不致命——
    /// 技能执行等调用方不应因落盘问题而崩溃。容量取 `max(256, 已加载条目数)`，
    /// 避免刚加载的冷上下文立即被 LRU 驱逐。
    pub fn open(path: &Path) -> Self {
        let mut cache = Self::new(256);
        if let Ok(data) = std::fs::read(path) {
            if let Ok(p) = serde_json::from_slice::<PersistedCache>(&data) {
                let mut g = cache.inner.lock().unwrap_or_else(|e| e.into_inner());
                for e in p.map.values() {
                    g.stats.stored_prompt_tokens += e.prompt_tokens as u64;
                }
                g.map = p.map;
                g.prefix_seen = p.prefix_seen;
                cache.capacity = cache.capacity.max(g.map.len());
            }
        }
        cache
    }

    /// 落盘冷上下文缓存（W-7b）：原子写（临时文件 + rename），使跨进程重启可复用。
    ///
    /// 失败返回 `io::Error`（由调用方决定是否忽略，如 `SkillEngine` 静默忽略）。
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let p = PersistedCache {
            map: g.map.clone(),
            prefix_seen: g.prefix_seen.clone(),
        };
        let data = serde_json::to_vec(&p)?;
        drop(g);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &data)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// 导出当前落盘载荷（供 `MultiTierCache` 组合持久化复用）。
    pub(crate) fn snapshot(&self) -> PersistedCache {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        PersistedCache {
            map: g.map.clone(),
            prefix_seen: g.prefix_seen.clone(),
        }
    }

    /// 由落盘载荷重建（容量取 `max(capacity, 已加载条目数)`）。
    pub(crate) fn from_snapshot(p: PersistedCache, capacity: usize) -> Self {
        let cap = capacity.max(p.map.len());
        Self {
            inner: Mutex::new(Inner {
                map: p.map,
                stats: CacheStats::default(),
                prefix_seen: p.prefix_seen,
            }),
            capacity: cap.max(1),
        }
    }
}

/// 当前毫秒时间戳（供 LRU 使用）
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 进程级前缀缓存追踪器（DeepSeek `prompt_cache_hit_tokens` 归一后统一上报）。
///
/// 设计要点（呼应 GUI_Agent `PrefixCacheTracker` 思路，纯 Rust 零依赖落地）：
/// - 服务端前缀缓存命中/写入 token 累积为进程级可观测指标；无需把 `PrefixCache` 实例
///   穿透到每一个 LLM 调用点（`model::llm` 现状即无该实例，归一 usage 后调用
///   [`report_prefix_cache_usage`] 即可）。
/// - [`PrefixCacheTracker::observe_stable_prefix`] 提供「稳定前缀漂移守卫」：压缩器/序列化层
///   在发出「应被服务端缓存的不变前缀」时登记其规范指纹，后续调用校验字节一致——
///   直接封堵「非规范重序列化」这一头号缓存杀手（四点缓存失效模式之二：工具结果
///   重序列化致字节漂移，前缀永不命中）。与 W-4 `ToolCompressor` 的逐字节确定性相辅相成。
pub struct PrefixCacheTracker {
    inner: Mutex<PrefixCacheTrackerInner>,
}

struct PrefixCacheTrackerInner {
    /// 上报次数（LLM 调用次数）
    calls: u64,
    /// 累计 prompt token（含命中 + 写入 + 未缓存）
    prompt_tokens_total: u64,
    /// 累计服务端前缀缓存命中 token（DeepSeek `prompt_cache_hit_tokens` 归一）
    cache_read_total: u64,
    /// 累计服务端前缀缓存写入 token（首次为该前缀建立 KV 缓存）
    cache_creation_total: u64,
    /// 稳定前缀规范指纹（FNV-1a）；`None` 表示尚未登记
    stable_prefix_fp: Option<u64>,
    /// 稳定前缀漂移次数（同一逻辑前缀两次登记指纹不同 = 缓存前缀被破坏）
    stable_prefix_drift: u64,
}

impl PrefixCacheTracker {
    /// 构造独立实例（单测用）；生产路径用 [`prefix_cache_tracker`] 获取进程级单例。
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(PrefixCacheTrackerInner {
                calls: 0,
                prompt_tokens_total: 0,
                cache_read_total: 0,
                cache_creation_total: 0,
                stable_prefix_fp: None,
                stable_prefix_drift: 0,
            }),
        }
    }

    /// 上报一次 LLM 调用的前缀缓存用量（已归一：cache_read = 命中，cache_creation = 写入）。
    pub fn report(&self, prompt_tokens: u64, cache_read: u64, cache_creation: u64) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.calls += 1;
        g.prompt_tokens_total += prompt_tokens;
        g.cache_read_total += cache_read;
        g.cache_creation_total += cache_creation;
    }

    /// 登记 / 校验「稳定前缀」：传入应被服务端缓存的不变前缀规范串。
    ///
    /// - 首次登记：记录指纹并返回 `true`（视作基线）。
    /// - 后续一致：返回 `true`（无漂移，前缀可稳定命中）。
    /// - 后续不一致：累加 [`PrefixCacheTrackerInner::stable_prefix_drift`]、更新指纹、返回 `false`
    ///   （漂移告警——压缩/序列化破坏了缓存前缀，需排查）。
    pub fn observe_stable_prefix(&self, prefix: &str) -> bool {
        let fp = hash_text(prefix);
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match g.stable_prefix_fp {
            Some(prev) if prev == fp => true,
            Some(_) => {
                g.stable_prefix_drift += 1;
                g.stable_prefix_fp = Some(fp);
                false
            }
            None => {
                g.stable_prefix_fp = Some(fp);
                true
            }
        }
    }

    /// 快照当前可观测指标。
    pub fn stats(&self) -> PrefixCacheTrackerStats {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        PrefixCacheTrackerStats {
            calls: g.calls,
            prompt_tokens_total: g.prompt_tokens_total,
            cache_read_total: g.cache_read_total,
            cache_creation_total: g.cache_creation_total,
            stable_prefix_drift: g.stable_prefix_drift,
        }
    }

    /// 服务端前缀缓存命中率 ∈ [0, 1]：命中 token / (命中 + 写入) token。
    ///
    /// 反映 DeepSeek 原生 prompt caching 的真实复用程度（不是客户端精确缓存）。
    /// 稳态（无新建缓存）下趋近 1.0；首调或前缀漂移时会回落。
    pub fn hit_rate(&self) -> f64 {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let total = g.cache_read_total + g.cache_creation_total;
        if total == 0 {
            0.0
        } else {
            g.cache_read_total as f64 / total as f64
        }
    }

    /// 稳定前缀漂移次数（>0 表示压缩/序列化破坏了缓存前缀，应告警）。
    pub fn stable_prefix_drift(&self) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stable_prefix_drift
    }
}

/// 进程级前缀缓存追踪器快照。
#[derive(Clone, Copy, Debug, Default)]
pub struct PrefixCacheTrackerStats {
    pub calls: u64,
    pub prompt_tokens_total: u64,
    pub cache_read_total: u64,
    pub cache_creation_total: u64,
    pub stable_prefix_drift: u64,
}

static PREFIX_CACHE_TRACKER: OnceLock<PrefixCacheTracker> = OnceLock::new();

/// 获取进程级前缀缓存追踪器（惰性初始化，全局唯一）。
pub fn prefix_cache_tracker() -> &'static PrefixCacheTracker {
    PREFIX_CACHE_TRACKER.get_or_init(|| PrefixCacheTracker {
        inner: Mutex::new(PrefixCacheTrackerInner {
            calls: 0,
            prompt_tokens_total: 0,
            cache_read_total: 0,
            cache_creation_total: 0,
            stable_prefix_fp: None,
            stable_prefix_drift: 0,
        }),
    })
}

/// 便捷上报：LLM 调用点归一 usage 后调用，累积进程级前缀缓存命中率。
pub fn report_prefix_cache_usage(prompt_tokens: u64, cache_read: u64, cache_creation: u64) {
    prefix_cache_tracker().report(prompt_tokens, cache_read, cache_creation);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_then_get_returns_entry() {
        let c = PrefixCache::new(8);
        c.put(
            1,
            CacheEntry {
                content: "A".into(),
                prompt_tokens: 10,
                completion_tokens: 5,
                last_used_ms: 1,
            },
        );
        let e = c.get(1).expect("应命中");
        assert_eq!(e.content, "A");
        assert_eq!(e.completion_tokens, 5);
    }

    #[test]
    fn miss_records_stats() {
        let c = PrefixCache::new(8);
        assert!(c.get(999).is_none());
        let s = c.stats();
        assert_eq!(s.lookups, 1);
        assert_eq!(s.hits, 0);
        assert_eq!(s.misses, 1);
    }

    #[test]
    fn hit_records_stats_and_hit_rate() {
        let c = PrefixCache::new(8);
        c.put(
            7,
            CacheEntry {
                content: "B".into(),
                prompt_tokens: 20,
                completion_tokens: 3,
                last_used_ms: 1,
            },
        );
        c.get(7);
        let s = c.stats();
        assert_eq!(s.hits, 1);
        assert_eq!(s.lookups, 1);
        assert!((s.hit_rate() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn lru_eviction_keeps_capacity() {
        let c = PrefixCache::new(2);
        for k in 0..5u64 {
            c.put(
                k,
                CacheEntry {
                    content: k.to_string(),
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    last_used_ms: k,
                },
            );
        }
        assert!(c.len() <= 2, "超过容量应被 LRU 驱逐");
        // 最近写入的 key=4 必然还在
        assert!(c.get(4).is_some());
    }

    #[test]
    fn hash_is_stable_and_distinct() {
        assert_eq!(hash_text("hello"), hash_text("hello"));
        assert_ne!(hash_text("hello"), hash_text("world"));
    }

    #[test]
    fn reuse_ratio_reflects_savings() {
        let c = PrefixCache::new(8);
        c.put(
            1,
            CacheEntry {
                content: "x".into(),
                prompt_tokens: 100,
                completion_tokens: 1,
                last_used_ms: 1,
            },
        );
        c.get(1); // 命中，省下 100 prompt token
        let s = c.stats();
        // stored=100, saved=100 → reuse 0.5
        assert!((s.reuse_ratio() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn prefix_reuse_recorded_on_repeat() {
        let c = PrefixCache::new(8);
        assert!(!c.note_prefix(42, 100), "首次应记为新前缀");
        assert!(c.note_prefix(42, 100), "重复应记为前缀复用");
        let s = c.stats();
        assert_eq!(s.prefix_hits, 1);
        assert_eq!(s.prefix_saved_tokens, 100);
    }

    #[test]
    fn record_prefix_reuse_updates_stats() {
        let mut s = CacheStats::default();
        s.record_prefix_reuse(50);
        assert_eq!(s.prefix_hits, 1);
        assert_eq!(s.prefix_saved_tokens, 50);
    }

    #[test]
    fn distinct_prefixes_do_not_reuse() {
        let c = PrefixCache::new(8);
        assert!(!c.note_prefix(1, 10));
        assert!(!c.note_prefix(2, 20), "不同前缀不应复用");
        assert_eq!(c.stats().prefix_hits, 0);
    }

    #[test]
    fn open_missing_file_returns_empty_cache() {
        let path =
            std::env::temp_dir().join(format!("marg_prefix_missing_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let c = PrefixCache::open(&path);
        assert!(c.is_empty(), "不存在的文件应加载为空缓存（不致命）");
    }

    #[test]
    fn prefix_cache_tracker_reports_and_hit_rate() {
        let t = PrefixCacheTracker::new();
        // 3 次调用：稳态（cache_read 远大于 creation）应趋近高命中率。
        t.report(1000, 800, 0);
        t.report(1000, 800, 0);
        t.report(1000, 800, 0);
        let s = t.stats();
        assert_eq!(s.calls, 3);
        assert_eq!(s.cache_read_total, 2400);
        assert!((t.hit_rate() - 1.0).abs() < 1e-9, "稳态应 100% 命中");
    }

    #[test]
    fn prefix_cache_tracker_detects_stable_prefix_drift() {
        let t = PrefixCacheTracker::new();
        assert!(
            t.observe_stable_prefix("system-prefix-v1"),
            "首次登记应返回 true"
        );
        assert!(t.observe_stable_prefix("system-prefix-v1"), "相同前缀应无漂移");
        assert!(
            !t.observe_stable_prefix("system-prefix-v2-changed"),
            "前缀变化应检出漂移并返回 false"
        );
        assert_eq!(t.stable_prefix_drift(), 1, "应记录 1 次漂移");
    }

    #[test]
    fn global_prefix_cache_tracker_is_singleton() {
        // 进程级单例：两次获取应为同一实例（指针相等）。
        let a = prefix_cache_tracker() as *const PrefixCacheTracker;
        let b = prefix_cache_tracker() as *const PrefixCacheTracker;
        assert_eq!(a, b, "prefix_cache_tracker() 应返回同一全局实例");
    }

    #[test]
    fn save_then_open_roundtrips_entries_and_prefixes() {
        let path = std::env::temp_dir().join(format!("marg_prefix_rt_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let c = PrefixCache::new(16);
        c.put(
            1,
            CacheEntry {
                content: "A".into(),
                prompt_tokens: 10,
                completion_tokens: 5,
                last_used_ms: 1,
            },
        );
        c.put(
            2,
            CacheEntry {
                content: "B".into(),
                prompt_tokens: 20,
                completion_tokens: 3,
                last_used_ms: 2,
            },
        );
        assert!(!c.note_prefix(42, 100));
        c.save(&path).expect("落盘应成功");
        // 重新打开（模拟进程重启）
        let c2 = PrefixCache::open(&path);
        assert_eq!(c2.get(1).expect("A 应命中").content, "A");
        assert_eq!(c2.get(2).expect("B 应命中").content, "B");
        assert!(c2.note_prefix(42, 100), "已观测前缀应从落盘恢复");
        assert_eq!(c2.stats().stored_prompt_tokens, 30, "stored token 应恢复");
        let _ = std::fs::remove_file(&path);
    }
}
