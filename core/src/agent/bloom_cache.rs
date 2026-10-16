//! Bloom 缓存层（KV-Cache 极简工程 §2.13）
//!
//! 蒸馏来源：opencode `scoped-cache.ts` 的 scope-keyed 存储、TTL、容量上限观测，
//! 以及 `agent::prefix_scope::SharedPrefixCache` 同款 `PrefixCache` 前缀复用观测。
//!
//! 本模块保留三类缓存：
//! - [`SnapshotCache`]：相同 `(agent_id, intent)` 复用既有快照句柄，不重新打快照；
//! - [`BloomParallelPrefix`]：对 `cache::PrefixCache` 的语义清晰门面封装，供并行前缀复用；
//! - [`BloomDecisionCache`]：相同意图（提示词哈希）复用既有编排决策，跳过重复的
//!   双模型编排（KV-Cache「重算变复用」在调度层的体现）。
//!
//! 诚实事前边界（与 P-9/P-10/P-11/P-12 一脉相承）：
//! - 一切缓存查询都是「精确哈希优先」，绝不编造未命中的结果；
//! - 命中只对**逐字节一致**的意图成立，任何改动都视为未命中（不冒充语义命中）；
//! - 确定性、零 LLM、零网络，全部可单测。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::cache::{hash_text, PrefixCache};

// ===========================================================================
// 2. SnapshotCache —— 快照按 (agent, intent_hash) 复用
// ===========================================================================

/// 快照句柄缓存：相同 `(agent_id, intent)` 复用既有快照，不再重新打快照。
///
/// 「快照」在 Bloom 语境即 YOLO 快照安全网（`chain/executor.rs` C9）；本缓存只存
/// 一个不透明的 `String` 句柄（如快照 id），与具体快照引擎解耦，框架无关、可单测、不伪造。
pub struct SnapshotCache {
    inner: Mutex<HashMap<u64, String>>,
}

impl SnapshotCache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// 组合键：`agent_id` 与 `intent` 拼接后哈希（含顺序敏感分隔符，避免 `(a,b)` 与 `(b,a)` 同键）。
    pub fn key(agent_id: &str, intent: &str) -> u64 {
        hash_text(&format!("{}\u{1f}{}", agent_id, intent))
    }

    /// 查询既有快照句柄。
    pub fn get(&self, agent_id: &str, intent: &str) -> Option<String> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&Self::key(agent_id, intent))
            .cloned()
    }

    /// 存入快照句柄（执行真实快照后回填）。
    pub fn put(&self, agent_id: &str, intent: &str, handle: String) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(Self::key(agent_id, intent), handle);
    }

    /// 观测一次 `(agent, intent)`：若已有快照返回 `true`（复用），并保留原句柄；
    /// 否则记录 `handle` 并返回 `false`（首次，需真实打快照）。
    pub fn reuse_or_store(&self, agent_id: &str, intent: &str, handle: String) -> bool {
        let key = Self::key(agent_id, intent);
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let std::collections::hash_map::Entry::Vacant(e) = g.entry(key) {
            e.insert(handle);
            false
        } else {
            true
        }
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for SnapshotCache {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================

// ===========================================================================
// 4. BloomParallelPrefix —— 并行分支共享 system 前缀缓存
// ===========================================================================

/// Bloom 蜂群并行分支的前缀复用观测：同一 system 提示在所有分片/专家分支间共享，
/// 服务端 KV-Cache 据此复用不变前缀（KV-Cache 原则 P1）。
///
/// 复用 `cache::PrefixCache`（与 P-12 `SharedPrefixCache` 同构），仅封装一层语义清晰的门面。
pub struct BloomParallelPrefix {
    cache: Arc<PrefixCache>,
}

impl BloomParallelPrefix {
    pub fn new(cache: Arc<PrefixCache>) -> Self {
        Self { cache }
    }

    /// 观测一个并行分支的 system 前缀：首次返回 `false`（新前缀），后续返回 `true`（复用）。
    ///
    /// 委托 `PrefixCache::note_prefix`（KV-Cache 原则 P1 客户端观测：相同稳定前缀让服务端
    /// 复用已计算的 KV 块，无需重新预填充；`prefix_tokens` 如实计入 `CacheStats` 不冒充命中）。
    pub fn observe_branch(&self, system_prompt: &str, prefix_tokens: u32) -> bool {
        self.cache
            .note_prefix(hash_text(system_prompt), prefix_tokens)
    }

    pub fn stats(&self) -> crate::cache::CacheStats {
        self.cache.stats()
    }

    pub fn inner(&self) -> &Arc<PrefixCache> {
        &self.cache
    }
}

// ===========================================================================
// 5. BloomDecisionCache —— 相同意图复用编排决策（调度层「重算变复用」）
// ===========================================================================

/// 决策缓存命中统计（如实计量，不虚报）
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecisionCacheStats {
    /// 命中次数（逐字节一致且未过期）
    pub hits: u64,
    /// 未命中次数（首见 / 过期 / 意图改动）
    pub misses: u64,
    /// 命中省下的提示词 token 估算（由调用方如实上报）
    pub saved_prompt_tokens: u64,
}

impl DecisionCacheStats {
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// 单条缓存决策
#[derive(Clone)]
struct CachedDecision {
    /// 到期的毫秒时间戳（`crate::cache::now_ms` 基准）
    expires_at_ms: u64,
    /// 意图原文（用于命中时逐字节复核，杜绝哈希碰撞冒充）
    intent: String,
    /// 复用的编排决策文本
    decision: String,
}

impl CachedDecision {
    fn is_expired(&self, now: u64) -> bool {
        now >= self.expires_at_ms
    }
}

/// 相同意图复用既有编排决策的缓存。
///
/// 命中语义严格：`intent` 逐字节一致且未过期才命中；任何改动都视为未命中。
/// 不入缓存：过期条目、容量超限驱逐（FIFO）。命中省下的 token 由调用方如实上报，
/// 不冒充「语义命中」。
pub struct BloomDecisionCache {
    inner: Mutex<HashMap<u64, CachedDecision>>,
    /// 插入顺序队列（FIFO 驱逐依据）：队首为最早插入、优先被驱逐。
    order: Mutex<std::collections::VecDeque<u64>>,
    /// 容量上限（FIFO 驱逐）
    capacity: usize,
    /// TTL（毫秒）
    ttl_ms: u64,
    stats: Mutex<DecisionCacheStats>,
}

impl BloomDecisionCache {
    pub fn new(capacity: usize, ttl_ms: u64) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            order: Mutex::new(std::collections::VecDeque::new()),
            capacity: capacity.max(1),
            ttl_ms,
            stats: Mutex::new(DecisionCacheStats::default()),
        }
    }

    /// 默认形态：容量 64、TTL 10 分钟（对齐生产默认，供接线方直接使用）
    pub fn with_default_policy() -> Self {
        Self::new(64, 10 * 60 * 1000)
    }

    /// 意图键：对意图原文做 FNV 哈希
    pub fn key(intent: &str) -> u64 {
        hash_text(intent)
    }

    /// 查询：命中返回既有决策文本，否则返回 `None` 并计入一次 miss。
    ///
    /// 命中复核：哈希一致 + 原文逐字节一致 + 未过期，三者缺一不可。
    pub fn get(&self, intent: &str) -> Option<String> {
        let now = crate::cache::now_ms();
        let key = Self::key(intent);
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let hit = inner.get(&key).cloned();
        match hit {
            Some(entry) if !entry.is_expired(now) && entry.intent == intent => {
                self.stats.lock().unwrap_or_else(|e| e.into_inner()).hits += 1;
                Some(entry.decision)
            }
            Some(_) | None => {
                // 过期或哈希一致但原文不一致（碰撞）都算未命中；过期条目顺手清理。
                if let Some(entry) = inner.get(&key) {
                    if entry.is_expired(now) {
                        inner.remove(&key);
                    }
                }
                self.stats.lock().unwrap_or_else(|e| e.into_inner()).misses += 1;
                None
            }
        }
    }

    /// 存入一条决策（调用方在真实执行编排后回填）。
    ///
    /// 容量超限时按插入顺序（FIFO）驱逐最早条目；重放同一意图会刷新其入队位置。
    pub fn put(&self, intent: &str, decision: String, saved_prompt_tokens: u64) {
        let key = Self::key(intent);
        let now = crate::cache::now_ms();
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut order = self.order.lock().unwrap_or_else(|e| e.into_inner());
        // 刷新入队位置：已存在则先移除再入队尾（最新优先存活）。
        if inner.contains_key(&key) {
            order.retain(|k| *k != key);
        }
        inner.insert(
            key,
            CachedDecision {
                expires_at_ms: now.saturating_add(self.ttl_ms),
                intent: intent.to_string(),
                decision,
            },
        );
        order.push_back(key);
        // FIFO 驱逐：超出容量时从队首逐出最早插入的键。
        while inner.len() > self.capacity {
            if let Some(evict) = order.pop_front() {
                inner.remove(&evict);
            } else {
                break;
            }
        }
        // 命中省下的 token 在 put 时会计入 saved（由调用方用真实值上报；
        // 若调用方传 0，则如实为 0，绝不自造）。
        if saved_prompt_tokens > 0 {
            self.stats
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .saved_prompt_tokens += saved_prompt_tokens;
        }
    }

    pub fn stats(&self) -> DecisionCacheStats {
        self.stats.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ===========================================================================
// 6. SemanticIntentCache —— 语义命中协商（精确优先 + 语义候选，零假阳性）
// ===========================================================================

/// 语义相似度函数：返回 `[0,1]` 的相似度（越接近 1 越相似）。
///
/// 默认走 [`EmbeddingProvider`]（进程级可注入后端）：`embeddings.backend=model` + `real-embed`
/// 时是**真实模型**余弦；默认零依赖本地后端下数值与 `LocalEmbedding::cosine` 一致。
/// 可注入自定义实现而无需改动缓存结构。
pub type SimilarityFn = fn(&str, &str) -> f64;

/// 默认语义相似度：走进程级嵌入后端（真实模型或本地特征哈希），不再写死 `LocalEmbedding`。
///
/// 文档 1 的「语义缓存未接真实 embedding」在 Bloom 决策链路上的收口：此前这里直接调用
/// `LocalEmbedding`，配置了真实嵌入后端也无法生效。
pub fn embedding_similarity(a: &str, b: &str) -> f64 {
    let backend = crate::memory::embedding::EmbeddingProvider::global();
    let va = backend.embed(a);
    let vb = backend.embed(b);
    backend.cosine(&va, &vb)
}

/// 语义查询结果来源标记（如实上报，不冒充精确命中）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheQuery {
    /// 精确哈希命中（逐字节一致）
    Exact,
    /// 语义候选命中（相似度 ≥ 阈值，由调用方谨慎采纳）
    Semantic,
}

/// 语义候选命中条目
#[derive(Debug, Clone)]
pub struct SemanticHit {
    /// 复用的决策文本
    pub decision: String,
    /// 语义相似度 ∈ [0,1]
    pub score: f64,
}

/// 单条语义缓存条目
struct SemanticEntry {
    /// 意图原文（用于相似度比对的锚点）
    intent: String,
    /// 该意图对应的决策文本
    decision: String,
}

/// 语义意图缓存：精确优先、语义可选、零假阳性优先。
///
/// 语义命中**绝不自动套用**：查询只返回候选（`Semantic(score)`），是否采纳由调用方
/// 按阈值决定。语义命中计入 `semantic_hits`，与 `BloomDecisionCache` 的 `hits` 分开计量，
/// 避免把「近似复用」冒充「逐字节命中」。
pub struct SemanticIntentCache {
    inner: Mutex<Vec<SemanticEntry>>,
    /// 容量上限（FIFO 驱逐）
    capacity: usize,
    /// 语义采纳阈值（由调用方查询时复核，本缓存只记录相似度）
    threshold: f64,
    /// 相似度实现（默认 `embedding_similarity`）
    sim: SimilarityFn,
    /// 语义命中统计（如实计量）
    semantic_hits: Mutex<u64>,
}

impl SemanticIntentCache {
    pub fn new(capacity: usize, threshold: f64) -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
            capacity: capacity.max(1),
            threshold: threshold.clamp(0.0, 1.0),
            sim: embedding_similarity,
            semantic_hits: Mutex::new(0),
        }
    }

    /// 默认形态：容量 64、语义阈值 0.8（对齐生产默认，供接线方直接使用）
    pub fn with_default_policy() -> Self {
        Self::new(64, 0.8)
    }

    /// 注入自定义相似度实现（DI 接缝，如真实嵌入余弦）
    pub fn with_similarity(mut self, sim: SimilarityFn) -> Self {
        self.sim = sim;
        self
    }

    /// 查询：返回与 `query` 最相似未过期条目的语义候选（含 `score`），无则 `None`。
    ///
    /// 本方法只**报告**相似度，不替代调用方做阈值裁决；得分 < 阈值时由调用方自行放弃。
    pub fn get(&self, query: &str) -> Option<SemanticHit> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut best: Option<SemanticHit> = None;
        for e in inner.iter() {
            let score = (self.sim)(query, &e.intent);
            if let Some(b) = &best {
                if score > b.score {
                    best = Some(SemanticHit {
                        decision: e.decision.clone(),
                        score,
                    });
                }
            } else {
                best = Some(SemanticHit {
                    decision: e.decision.clone(),
                    score,
                });
            }
        }
        best
    }

    /// 决策式查询：命中且 `score >= threshold` 才返回候选，并计入一次语义命中。
    ///
    /// 这是生产路径最常用的形态——把「阈值裁决」收敛到缓存侧，调用方无需再复核。
    pub fn get_above_threshold(&self, query: &str) -> Option<SemanticHit> {
        let hit = self.get(query)?;
        if hit.score >= self.threshold {
            // 一次性加锁递增，避免二次加锁自死锁（Mutex 不可重入）。
            let mut g = self.semantic_hits.lock().unwrap_or_else(|e| e.into_inner());
            *g = g.saturating_add(1);
            Some(hit)
        } else {
            None
        }
    }

    /// 存入一条语义条目（调用方在真实执行编排后回填）。
    ///
    /// 容量超限时按插入顺序（FIFO）驱逐最早条目；重放同一意图会刷新其位置。
    pub fn put(&self, intent: &str, decision: String) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.retain(|e| e.intent != intent);
        inner.push(SemanticEntry {
            intent: intent.to_string(),
            decision,
        });
        while inner.len() > self.capacity {
            inner.remove(0);
        }
    }

    pub fn semantic_hits(&self) -> u64 {
        *self.semantic_hits.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn threshold(&self) -> f64 {
        self.threshold
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ===========================================================================
// 测试
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- SnapshotCache ----

    #[test]
    fn snapshot_cache_records_new_then_reuses() {
        let c = SnapshotCache::new();
        assert!(!c.reuse_or_store("agent-1", "意图A", "snap-001".into()));
        assert_eq!(c.get("agent-1", "意图A"), Some("snap-001".into()));
        // 第二次：复用既有快照
        assert!(c.reuse_or_store("agent-1", "意图A", "snap-002".into()));
        // 复用时不覆盖原句柄
        assert_eq!(c.get("agent-1", "意图A"), Some("snap-001".into()));
    }

    #[test]
    fn snapshot_cache_distinct_keys_independent() {
        let c = SnapshotCache::new();
        assert!(!c.reuse_or_store("agent-1", "意图A", "s1".into()));
        assert!(!c.reuse_or_store("agent-2", "意图A", "s2".into()));
        assert!(!c.reuse_or_store("agent-1", "意图B", "s3".into()));
        assert_eq!(c.len(), 3);
        assert_eq!(c.get("agent-1", "意图A"), Some("s1".into()));
        assert_eq!(c.get("agent-2", "意图A"), Some("s2".into()));
    }

    #[test]
    fn snapshot_cache_combined_key_deterministic() {
        let k1 = SnapshotCache::key("agent-x", "意图y");
        let k2 = SnapshotCache::key("agent-x", "意图y");
        assert_eq!(k1, k2);
        // 组合键应区分 (agent, intent) 的交换
        assert_ne!(SnapshotCache::key("a", "b"), SnapshotCache::key("b", "a"));
    }

    // ---- BloomParallelPrefix ----

    #[test]
    fn bloom_parallel_prefix_first_is_new() {
        let p = BloomParallelPrefix::new(Arc::new(PrefixCache::new(8)));
        assert!(
            !p.observe_branch("你是蜂群协调器", 50),
            "首分支应记为新前缀"
        );
    }

    #[test]
    fn bloom_parallel_prefix_reuses_same_system() {
        let p = BloomParallelPrefix::new(Arc::new(PrefixCache::new(8)));
        assert!(!p.observe_branch("system prompt 共享", 50));
        assert!(
            p.observe_branch("system prompt 共享", 50),
            "后续同前缀应复用"
        );
        let s = p.stats();
        assert_eq!(s.prefix_hits, 1);
        assert_eq!(s.prefix_saved_tokens, 50);
    }

    #[test]
    fn bloom_parallel_prefix_distinct_systems_no_reuse() {
        let p = BloomParallelPrefix::new(Arc::new(PrefixCache::new(8)));
        assert!(!p.observe_branch("system A", 10));
        assert!(!p.observe_branch("system B", 20), "不同 system 不应复用");
        assert_eq!(p.stats().prefix_hits, 0);
    }

    // ---- BloomDecisionCache ----

    #[test]
    fn decision_cache_stores_then_hits_exact_bytes() {
        let c = BloomDecisionCache::with_default_policy();
        assert!(c.get("意图A").is_none(), "首见未命中");
        c.put("意图A", "决策文本".into(), 100);
        // 逐字节一致 → 命中
        assert_eq!(c.get("意图A"), Some("决策文本".into()));
        let s = c.stats();
        assert_eq!(s.hits, 1);
        assert_eq!(s.misses, 1);
        assert_eq!(s.saved_prompt_tokens, 100);
    }

    #[test]
    fn decision_cache_miss_on_intent_change() {
        let c = BloomDecisionCache::with_default_policy();
        c.put("意图A", "决策A".into(), 0);
        // 任何改动都视为未命中，不冒充语义命中
        assert!(c.get("意图B").is_none());
        assert_eq!(c.stats().misses, 1, "put 不计 miss，仅 get 未命中计一次");
    }

    #[test]
    fn decision_cache_evicts_over_capacity_fifo() {
        let c = BloomDecisionCache::new(2, 60_000);
        c.put("a", "A".into(), 0);
        c.put("b", "B".into(), 0);
        c.put("c", "C".into(), 0); // 超出容量，驱逐最早插入的 a
        assert!(c.get("a").is_none(), "FIFO 驱逐最早条目");
        assert_eq!(c.get("b"), Some("B".into()));
        assert_eq!(c.get("c"), Some("C".into()));
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn decision_cache_ttl_expires() {
        // 用 0 TTL：立即过期，命中复核必须拦截。
        let c = BloomDecisionCache::new(4, 0);
        c.put("意图", "决策".into(), 0);
        assert!(c.get("意图").is_none(), "过期条目不算命中");
    }

    #[test]
    fn decision_cache_hit_rate() {
        let c = BloomDecisionCache::with_default_policy();
        assert_eq!(c.stats().hit_rate(), 0.0);
        c.put("x", "X".into(), 0);
        c.get("x");
        c.get("y");
        let s = c.stats();
        assert_eq!(s.hits, 1);
        assert_eq!(s.misses, 1);
        assert!((s.hit_rate() - 0.5).abs() < 1e-9);
    }

    // ---- SemanticIntentCache ----

    #[test]
    fn semantic_cache_returns_candidate_with_score() {
        let c = SemanticIntentCache::new(8, 0.5);
        c.put("rust async runtime tokio", "用 tokio 写异步".into());
        let hit = c.get("tokio async runtime rust").expect("语义候选应返回");
        assert!(hit.score > 0.5, "相似意图得分应较高，实际 {}", hit.score);
        assert_eq!(hit.decision, "用 tokio 写异步");
    }

    #[test]
    fn semantic_cache_above_threshold_counts_semantic_hit() {
        let c = SemanticIntentCache::new(8, 0.5);
        c.put("rust async runtime tokio", "决策A".into());
        let hit = c
            .get_above_threshold("tokio async runtime rust")
            .expect("应采纳");
        assert!(hit.score >= c.threshold());
        assert_eq!(c.semantic_hits(), 1, "语义命中应单独计量，不冒充精确命中");
    }

    #[test]
    fn semantic_cache_below_threshold_returns_none() {
        // 高阈值：近同串也不再采纳，严防把近似当精确。
        let c = SemanticIntentCache::new(8, 0.99);
        c.put("rust async runtime tokio", "决策A".into());
        assert!(c.get_above_threshold("tokio async runtime rust").is_none());
        assert_eq!(c.semantic_hits(), 0);
    }

    #[test]
    fn semantic_cache_empty_returns_none() {
        let c = SemanticIntentCache::with_default_policy();
        assert!(c.get("任何查询").is_none());
        assert_eq!(c.semantic_hits(), 0);
    }

    #[test]
    fn semantic_cache_evicts_over_capacity_fifo() {
        // 用「自己=1、他人=0」的恒等相似度 + 阈值 0.5，才能精确断言被驱逐的条目确实缺席。
        fn identity(a: &str, b: &str) -> f64 {
            if a == b {
                1.0
            } else {
                0.0
            }
        }
        let c = SemanticIntentCache::new(2, 0.5).with_similarity(identity);
        c.put("a", "A".into());
        c.put("b", "B".into());
        c.put("c", "C".into()); // 超出容量，驱逐最早插入的 a
        assert_eq!(c.len(), 2);
        let above = |q: &str| c.get_above_threshold(q).map(|h| h.decision);
        assert_eq!(above("a"), None, "FIFO 驱逐最早条目");
        assert_eq!(above("b"), Some("B".into()));
        assert_eq!(above("c"), Some("C".into()));
    }

    #[test]
    fn semantic_cache_injectable_similarity_fn() {
        // DI 接缝：注入恒等相似度（自己=1，其他=0），验证可替换而不改结构。
        fn identity(a: &str, b: &str) -> f64 {
            if a == b {
                1.0
            } else {
                0.0
            }
        }
        let c = SemanticIntentCache::with_default_policy().with_similarity(identity);
        c.put("目标意图", "决策".into());
        assert_eq!(c.get("目标意图").map(|h| h.score), Some(1.0));
        assert_eq!(c.get("其它意图").map(|h| h.score), Some(0.0));
    }
}
