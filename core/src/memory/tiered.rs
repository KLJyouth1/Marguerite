//! 分层记忆索引与渐进式检索
//!
//! 在既有 L1 感官 / L2 短时之上，新增 **L3 语义长时**：flyhash 稀疏索引
//! 的跨会话持久记忆库。渐进式检索流程：
//!
//! 1. **粗排**：查询文本 → flyhash 稀疏投影 → WTA 指纹近邻，取 top-k 候选；
//! 2. **精排**：对候选做 LocalEmbedding 余弦精排；
//! 3. **门控**：fly 命中强度 + 最近访问 + 任务相关度（余弦）综合打分，
//!    低于阈值（`SIM_THRESHOLD`）的无关项不注入；
//! 4. **返回**：按合并分数 top-k，命中项更新访问计数。
//!
//! 上下文从此是「按需检索 + 智能压缩」的视图，同时给出 L2→L3 升库接口。

use std::time::{SystemTime, UNIX_EPOCH};

use crate::memory::flyhash::{FlyHashIndex, FlyHit};
use crate::memory::long_term::LocalEmbedding;

/// 与 long_term 相同的余弦阈值（低于视为无相关）
const SIM_THRESHOLD: f64 = 0.35;
/// 精排前每层的 flyhash 候选基数（渐进式：先粗排再精排）
const DEFAULT_CANDIDATES: usize = 16;
/// 每层容量上限（FIFO 淘汰最旧）
const DEFAULT_MAX_PER_TIER: usize = 256;

/// 记忆分层（对齐 L1–L3 约定）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryTierVariant {
    /// L1 感官：最近原始上下文，极短 TTL
    Sensory,
    /// L2 短时/工作：本轮任务工作记忆
    ShortTerm,
    /// L3 语义长时：flyhash 索引的稀疏记忆库（跨会话持久）
    LongTerm,
}

impl MemoryTierVariant {
    pub fn label(&self) -> &'static str {
        match self {
            MemoryTierVariant::Sensory => "sensory",
            MemoryTierVariant::ShortTerm => "short",
            MemoryTierVariant::LongTerm => "long",
        }
    }
}

/// 分层记忆条目
#[derive(Debug, Clone)]
pub struct TieredEntry {
    pub id: u64,
    pub tier: MemoryTierVariant,
    pub text: String,
    pub meta: String,
    pub created_at: i64,
    pub access_count: u64,
    pub last_access: i64,
}

/// 检索结果（含门控打分）
#[derive(Debug, Clone)]
pub struct TieredHit {
    pub entry: TieredEntry,
    /// flyhash 近邻粗排分（0..1；非长时层为 0）
    pub fly_score: f64,
    /// 余弦精排分
    pub cosine: f64,
    /// 综合门控分
    pub rank: f64,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 分层记忆库
#[derive(Debug, Clone)]
pub struct TieredMemory {
    next_id: u64,
    sensory: Vec<TieredEntry>,
    short: Vec<TieredEntry>,
    long: Vec<TieredEntry>,
    fly: FlyHashIndex,
    candidates_per_tier: usize,
    sim_threshold: f64,
    max_per_tier: usize,
}

impl Default for TieredMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl TieredMemory {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            sensory: Vec::new(),
            short: Vec::new(),
            long: Vec::new(),
            fly: FlyHashIndex::new(),
            candidates_per_tier: DEFAULT_CANDIDATES,
            sim_threshold: SIM_THRESHOLD,
            max_per_tier: DEFAULT_MAX_PER_TIER,
        }
    }

    pub fn with_capacity(max_per_tier: usize) -> Self {
        Self {
            max_per_tier,
            ..Self::new()
        }
    }

    pub fn len(&self, tier: MemoryTierVariant) -> usize {
        match tier {
            MemoryTierVariant::Sensory => self.sensory.len(),
            MemoryTierVariant::ShortTerm => self.short.len(),
            MemoryTierVariant::LongTerm => self.long.len(),
        }
    }

    pub fn is_empty(&self, tier: MemoryTierVariant) -> bool {
        self.len(tier) == 0
    }

    pub fn total(&self) -> usize {
        self.sensory.len() + self.short.len() + self.long.len()
    }

    /// flyhash 稀疏索引的条目数（只读观测面用，供 `memory.tiered.status`）。
    ///
    /// 正常情况下它应等于 `long` 池长度；两者不一致说明有条目漏建/漏删索引
    /// （`trim_pool` 与 `promote` 是唯一改索引的地方），这是可观测的健全性信号。
    pub fn fly_len(&self) -> usize {
        self.fly.len()
    }

    /// 每层容量上限（只读观测面用）。
    pub fn max_per_tier(&self) -> usize {
        self.max_per_tier
    }

    /// 余弦门控阈值（只读观测面用；<=0 表示关闭）。
    pub fn sim_threshold(&self) -> f64 {
        self.sim_threshold
    }

    /// 插入一条记忆；返回条目 id。
    /// 长时层同时写入 flyhash 稀疏索引（粗排候选来源）。
    pub fn insert(&mut self, tier: MemoryTierVariant, text: impl Into<String>, meta: impl Into<String>) -> u64 {
        let text = text.into();
        let meta = meta.into();
        let id = self.next_id;
        self.next_id += 1;
        let ts = now_secs();
        let entry = TieredEntry {
            id,
            tier,
            text: text.clone(),
            meta,
            created_at: ts,
            access_count: 0,
            last_access: 0,
        };
        match tier {
            MemoryTierVariant::Sensory => {
                self.sensory.push(entry);
                trim_pool(&mut self.sensory, self.max_per_tier, &mut self.fly);
            }
            MemoryTierVariant::ShortTerm => {
                self.short.push(entry);
                trim_pool(&mut self.short, self.max_per_tier, &mut self.fly);
            }
            MemoryTierVariant::LongTerm => {
                self.fly.insert(id.to_string(), text);
                self.long.push(entry);
                trim_pool(&mut self.long, self.max_per_tier, &mut self.fly);
            }
        }
        id
    }

    /// L2→L3 升库：把短时/感官中的条目提升为长时（在 flyhash 建索引）
    pub fn promote(&mut self, id: u64, to_tier: MemoryTierVariant) -> bool {
        if to_tier == MemoryTierVariant::LongTerm {
            let mut promoted = None;
            for pool in [&mut self.sensory, &mut self.short] {
                if let Some(pos) = pool.iter().position(|e| e.id == id) {
                    let mut e = pool.remove(pos);
                    e.tier = MemoryTierVariant::LongTerm;
                    e.last_access = now_secs();
                    promoted = Some(e);
                    break;
                }
            }
            if let Some(e) = promoted {
                self.fly.insert(e.id.to_string(), e.text.clone());
                self.long.push(e);
                trim_pool(&mut self.long, self.max_per_tier, &mut self.fly);
                return true;
            }
        } else if let Some(pos) = self.long.iter().position(|e| e.id == id) {
            let mut e = self.long.remove(pos);
            e.tier = to_tier;
            e.last_access = now_secs();
            self.fly.remove(&id.to_string());
            match to_tier {
                MemoryTierVariant::Sensory => self.sensory.push(e),
                MemoryTierVariant::ShortTerm => self.short.push(e),
                MemoryTierVariant::LongTerm => unreachable!(),
            }
            return true;
        }
        false
    }

    /// 单层直接检索（Sensory/ShortTerm 余弦全排；LongTerm flyhash 粗排 + 余弦精排）
    pub fn recall(&mut self, tier: MemoryTierVariant, text: &str, top_k: usize) -> Vec<TieredHit> {
        let query_vec = LocalEmbedding::embed(text);
        let mut hits = match tier {
            MemoryTierVariant::Sensory => self
                .sensory
                .iter()
                .map(|e| {
                    let cosine = LocalEmbedding::cosine(&query_vec, &LocalEmbedding::embed(&e.text));
                    self.hit(0.0, cosine, e.clone())
                })
                .collect::<Vec<_>>(),
            MemoryTierVariant::ShortTerm => self
                .short
                .iter()
                .map(|e| {
                    let cosine = LocalEmbedding::cosine(&query_vec, &LocalEmbedding::embed(&e.text));
                    self.hit(0.0, cosine, e.clone())
                })
                .collect::<Vec<_>>(),
            MemoryTierVariant::LongTerm => {
                // 渐进式：先用 flyhash 粗排拿候选，避免全量精排
                let cands = self.fly.query(text, self.candidates_per_tier.max(top_k));
                let mut seen = std::collections::HashSet::new();
                let mut out = Vec::new();
                for FlyHit { key, text: _t, score } in cands {
                    let id: u64 = key.parse().unwrap_or(0);
                    if !seen.insert(id) {
                        continue;
                    }
                    if let Some(e) = self.long.iter().find(|e| e.id == id) {
                        let cosine = LocalEmbedding::cosine(&query_vec, &LocalEmbedding::embed(&e.text));
                        out.push((id, self.hit(score, cosine, e.clone())));
                    }
                }
                // 退化兜底：候选不足时补全量余弦（仅库小时触发）
                if out.len() < top_k.saturating_mul(2) {
                    let mut known: std::collections::HashSet<u64> = out.iter().map(|(id, _)| *id).collect();
                    for e in &self.long {
                        if known.contains(&e.id) {
                            continue;
                        }
                        let cosine = LocalEmbedding::cosine(&query_vec, &LocalEmbedding::embed(&e.text));
                        out.push((e.id, self.hit(0.0, cosine, e.clone())));
                        known.insert(e.id);
                    }
                }
                out.sort_by(|a, b| b.1.rank.partial_cmp(&a.1.rank).unwrap_or(std::cmp::Ordering::Equal));
                out.into_iter().map(|(_, h)| h).collect()
            }
        };
        if tier != MemoryTierVariant::LongTerm {
            hits.sort_by(|a, b| b.rank.partial_cmp(&a.rank).unwrap_or(std::cmp::Ordering::Equal));
        }
        // 低相关门控：低于阈值（sim_threshold）的无关项不注入；阈值<=0 视为关闭。
        if self.sim_threshold > 0.0 {
            hits.retain(|h| h.cosine >= self.sim_threshold);
        }
        hits.truncate(top_k);
        // 命中即更新访问计数
        self.touch(&hits);
        hits
    }

    /// 渐进式检索：感官 + 短时 + 长时三层合并，门控后返回 top_k
    pub fn progressive_retrieve(&mut self, text: &str, top_k: usize) -> Vec<TieredHit> {
        let long_hits = self.recall(MemoryTierVariant::LongTerm, text, top_k * 2);
        let short_hits = self.recall(MemoryTierVariant::ShortTerm, text, top_k * 2);
        let sensory_hits = self.recall(MemoryTierVariant::Sensory, text, top_k * 2);

        let mut merged: Vec<TieredHit> = Vec::with_capacity(long_hits.len() + short_hits.len() + sensory_hits.len());
        merged.extend(long_hits);
        merged.extend(short_hits);
        merged.extend(sensory_hits);
        // 同 id 去重（升库后可能同时在多池出现）
        let mut seen = std::collections::HashSet::new();
        merged.retain(|h| seen.insert(h.entry.id));
        merged.sort_by(|a, b| b.rank.partial_cmp(&a.rank).unwrap_or(std::cmp::Ordering::Equal));
        merged.truncate(top_k);
        merged
    }

    /// 组装命中并计算综合门控分：0.60 余弦 + 0.30 fly + 0.10 最近访问
    fn hit(&self, fly_score: f64, cosine: f64, entry: TieredEntry) -> TieredHit {
        let recency = (entry.access_count.min(3) as f64 / 3.0) * 0.10;
        let rank = cosine * 0.60 + fly_score * 0.30 + recency;
        TieredHit {
            entry,
            fly_score,
            cosine,
            rank,
        }
    }

    /// 命中项更新访问计数（提升最近访问信号）
    fn touch(&mut self, hits: &[TieredHit]) {
        let ts = now_secs();
        for h in hits {
            for pool in [&mut self.sensory, &mut self.short, &mut self.long] {
                if let Some(e) = pool.iter_mut().find(|e| e.id == h.entry.id) {
                    e.access_count += 1;
                    e.last_access = ts;
                    break;
                }
            }
        }
    }

    /// 层级容量控制：FIFO 淘汰最旧。调用方应直接用 [`trim_pool`]，
    /// 以避免 `self.trim(&mut self.sensory)` 同时可变借用 `self` 与 `self.sensory`。
    #[allow(dead_code)]
    fn trim(&mut self, pool: &mut Vec<TieredEntry>) {
        trim_pool(pool, self.max_per_tier, &mut self.fly);
    }

    /// 直接访问条目（供上层按 id 取文）
    pub fn get(&self, id: u64) -> Option<&TieredEntry> {
        self.sensory
            .iter()
            .chain(&self.short)
            .chain(&self.long)
            .find(|e| e.id == id)
    }
}

/// 层级容量控制：FIFO 淘汰最旧；同时把被淘汰条目从 flyhash 粗排索引移除。
///
/// 作为自由函数（而非 `&mut self` 方法），调用方可在同一次调用中只对**不同字段**
/// 分别可变借用（`&mut self.sensory`、`&mut self.fly` 互不相交），
/// 解决 `self.trim(&mut self.sensory)` 对 `self` 与 `self.sensory` 的双重可变借用。
fn trim_pool(pool: &mut Vec<TieredEntry>, max_per_tier: usize, fly: &mut FlyHashIndex) {
    while pool.len() > max_per_tier {
        if let Some(evicted) = pool.first() {
            fly.remove(&evicted.id.to_string());
        }
        pool.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_counts_per_tier() {
        let mut m = TieredMemory::new();
        let id1 = m.insert(MemoryTierVariant::Sensory, "sensory note", "s");
        let id2 = m.insert(MemoryTierVariant::ShortTerm, "short note", "s2");
        let id3 = m.insert(MemoryTierVariant::LongTerm, "long note alpha", "l");
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(id3, 3);
        assert_eq!(m.len(MemoryTierVariant::Sensory), 1);
        assert_eq!(m.len(MemoryTierVariant::ShortTerm), 1);
        assert_eq!(m.len(MemoryTierVariant::LongTerm), 1);
        assert_eq!(m.total(), 3);
        assert!(m.get(id1).is_some());
        assert!(m.get(99).is_none());
    }

    fn fill_corpus(m: &mut TieredMemory) -> u64 {
        // 短时/感官噪声
        for i in 0..6 {
            m.insert(
                MemoryTierVariant::ShortTerm,
                format!("work note {} about invoice billing totals", i),
                "work",
            );
            m.insert(
                MemoryTierVariant::Sensory,
                format!("recent utterance {} about coffee machine", i),
                "chat",
            );
        }
        // 长时库：50 条无关 + 3 条同族目标
        for i in 0..50 {
            m.insert(
                MemoryTierVariant::LongTerm,
                format!("telemetry record {} temperature {} pressure {}", i, i * 11 % 40, i * 5 + 3),
                "sensor",
            );
        }
        let target_a = m.insert(
            MemoryTierVariant::LongTerm,
            "the quick brown fox jumps over the lazy dog",
            "story",
        );
        m.insert(
            MemoryTierVariant::LongTerm,
            "a quick brown fox leaps over the sleepy canine",
            "story",
        );
        m.insert(
            MemoryTierVariant::LongTerm,
            "the fast brown fox vaults above the resting hound",
            "story",
        );
        target_a
    }

    #[test]
    fn progressive_retrieve_recalls_related_topk() {
        let mut m = TieredMemory::new();
        let target = fill_corpus(&mut m);
        let query = "the quick fox leaps over the lazy dog";
        let hits = m.progressive_retrieve(query, 5);
        assert!(!hits.is_empty(), "expected at least one hit");
        // 目标族应进入 top-5
        let has_family = hits.iter().any(|h| h.entry.meta == "story");
        assert!(has_family, "expected story-family in top-5, got {:?}", hits.iter().map(|h| (&h.entry.meta, h.entry.id, h.cosine)).collect::<Vec<_>>());
        // 第一命中应为同族目标（余弦精排后最高）
        let top = &hits[0];
        assert_eq!(top.entry.id, target, "expected exact target as top hit");
        assert!(top.cosine >= SIM_THRESHOLD);
    }

    #[test]
    fn unrelated_items_do_not_pollute() {
        let mut m = TieredMemory::new();
        m.insert(MemoryTierVariant::LongTerm, "quantum entanglement observation log", "physics");
        m.insert(MemoryTierVariant::LongTerm, "shipment tracking order 8823 arrived", "logistics");
        m.insert(MemoryTierVariant::LongTerm, "meeting about quarterly budget review", "office");
        let target = m.insert(MemoryTierVariant::LongTerm, "the quick brown fox jumps over the lazy dog", "story");
        let hits = m.progressive_retrieve("a quick fox jumps over the dog", 10);
        assert!(!hits.is_empty());
        // 与目标无关的条目（cosine 低于阈值）不得混入结果
        for h in &hits {
            assert!(
                h.entry.id == target || h.cosine >= SIM_THRESHOLD,
                "unrelated item {} polluted results with cosine {}",
                h.entry.id,
                h.cosine
            );
        }
        assert_eq!(hits[0].entry.id, target, "target should rank first");
    }

    #[test]
    fn recall_long_term_uses_fly_candidates() {
        let mut m = TieredMemory::new();
        fill_corpus(&mut m);
        let hits = m.recall(MemoryTierVariant::LongTerm, "the quick fox leaps over the lazy dog", 3);
        assert_eq!(hits.len(), 3);
        // 高频命中过的条目访问计数应增长
        let first_id = hits[0].entry.id;
        let second = m.recall(MemoryTierVariant::LongTerm, "the quick fox leaps over the lazy dog", 3);
        assert!(second.iter().any(|h| h.entry.id == first_id && h.entry.access_count > 0));
    }

    #[test]
    fn promote_moves_short_to_long() {
        let mut m = TieredMemory::new();
        let id = m.insert(MemoryTierVariant::ShortTerm, "draft idea about fast fox retrieval", "draft");
        assert!(m.promote(id, MemoryTierVariant::LongTerm));
        assert_eq!(m.len(MemoryTierVariant::ShortTerm), 0);
        assert_eq!(m.len(MemoryTierVariant::LongTerm), 1);
        assert_eq!(m.get(id).unwrap().tier, MemoryTierVariant::LongTerm);
        // 升库后可通过长时检索命中
        let hits = m.recall(MemoryTierVariant::LongTerm, "draft idea about fast fox", 3);
        assert!(hits.iter().any(|h| h.entry.id == id));
        // 降级回短时也生效
        assert!(m.promote(id, MemoryTierVariant::ShortTerm));
        assert_eq!(m.len(MemoryTierVariant::LongTerm), 0);
        assert_eq!(m.len(MemoryTierVariant::ShortTerm), 1);
    }
}