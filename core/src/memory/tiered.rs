//! 分层记忆索引与渐进式检索
//!
//! 三层：L1 感官 / L2 短时 / L3 语义长时（长时池另建 flyhash 稀疏索引）。
//!
//! # 检索打分（2026-10-29 重做；依据见 `docs/prd/2026-10-29-memory-substrate-retrieval-fix.md`）
//!
//! 旧实现是 `0.60 余弦 + 0.30 flyhash + 0.10 最近访问`，再用**绝对门** `cosine ≥ 0.35` 丢弃。
//! 离线实测（同一夹具六档）它打不过一个朴素关键词基线：中文 40% vs 100%、英文 41.7% vs 70.8%，
//! 且近半查询被绝对门**饿死**——表现不是「没找到」，而是「什么都不注入」。三条根因：
//! - 三个系数从未被测量过，是魔数；
//! - `最近访问` 来自读路径的写副作用（`recall()` 里 `touch()` 递增 `access_count`），
//!   于是「这条被查过几次」会改变相关性 —— 同库同查询不再稳定，前缀与排查双双被污染；
//! - flyhash 粗排在主路上净贡献为负：6/6 档「全量兜底」触发率 **100%**，候选覆盖率只有 0–89%。
//!
//! 新规则只有两条，每条都能向用户解释（规则本身定义在 [`scoring`]，与
//! `long_term::search` 共用同一份 —— 两份阈值各写一遍正是上一段的病根）：
//! 1. **两路证据，词面优先**：幅度 `rank = fuse(实词重合数, 实词覆盖率, 余弦)`，
//!    次序「先比有没有词面证据、再比幅度」（[`scoring::lexical_first`]）。
//!    中间被实测推翻过两次：先否掉 RRF（把分数压成 `1/(60+r)`，相对判据无法共存），
//!    再否掉「`max` 同时当次序」（零重合的哈希余弦压掉真实的词面命中，漏 3 条查询），
//!    最后否掉「零重合候选可以单独成立」（默认后端在零重合句子上分不开改述与无关，
//!    详见 [`scoring`] 模块头）。
//! 2. **截断只看这次查询有没有证据**：`keeps(best, r) = best > 0 且 r ≥ 0.60 × best`。
//!    低于线的不注入；`best == 0`（整库共享不到一个实词）时一条都不给。
//!    既不再饿死，也不会把整库倒进上下文。
//!
//! flyhash 仍随长时池建/销索引（`memory.tiered.status` 用它做健全性观测，P1 把它转作
//! 写入去重与同族聚类），但**不再参与打分**。渐进式流程保留：L3 → L2 → L1 逐层召回后合并。

use std::time::{SystemTime, UNIX_EPOCH};

use crate::memory::flyhash::FlyHashIndex;
use crate::memory::lexical::{content_words, shared_content_words};
use crate::memory::long_term::LocalEmbedding;
use crate::memory::scoring::{self, RELATIVE_KEEP_RATIO, WEAK_EVIDENCE};

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

/// 检索结果（含两路证据与相对截断后的综合分）
#[derive(Debug, Clone)]
pub struct TieredHit {
    pub entry: TieredEntry,
    /// 余弦精排分（特征哈希词袋，见 `long_term::LocalEmbedding`）
    pub cosine: f64,
    /// 与查询共享的**实词**个数（路 A 的原始证据）
    pub keyword_overlap: usize,
    /// 幅度分 = [`scoring::fuse`] 的结果（词面为闸门，档内 `max(实词覆盖率, 余弦)`）。
    /// 相对截断与弱证据标注用它；**次序**还要先看 [`TieredHit::keyword_overlap`]
    /// （[`scoring::lexical_first`]），单靠这个数排不出正确的顺序。
    pub rank: f64,
    /// 两路证据都低于 `WEAK_EVIDENCE`：注入时须标注为候选而非事实
    pub weak: bool,
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
    weak_evidence: f64,
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
            weak_evidence: WEAK_EVIDENCE,
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

    /// 弱证据标注线（只读观测面用）。
    ///
    /// 历史名字叫「余弦门控阈值」，语义已变：它**不再决定丢弃**，只决定是否给
    /// 注入行加 `(弱相关)` 前缀。丢弃见 [`scoring::keeps`]。
    pub fn weak_evidence(&self) -> f64 {
        self.weak_evidence
    }

    /// 截断规则（只读观测面用）：`(相对保留比例, 零重合是否可单独成立)`。
    ///
    /// 第二项就是 `scoring::LOCAL_HASH_IS_SEMANTIC`：默认 `false`，意味着
    /// 「与查询共享 0 个实词」的候选一律不给；接上真嵌入模型翻成 `true` 后，
    /// 这一路才重新由余弦单独决定。观测面把它如实报出来，否则「为什么这条不注入」
    /// 又是一个只能读代码才能回答的问题。
    pub fn cutoff_rule(&self) -> (f64, bool) {
        (RELATIVE_KEEP_RATIO, scoring::LOCAL_HASH_IS_SEMANTIC)
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

    /// 单层检索：按 [`scoring`] 算幅度与次序（词面优先、零重合不单独成立），再按相对截断丢弃。
    ///
    /// 三层走同一条代码——flyhash 粗排不再参与长时层（实测 6/6 档「全量兜底」触发率 100%，
    /// 候选覆盖只有 0–89%，它在主路上的净贡献是成本）。
    pub fn recall(&mut self, tier: MemoryTierVariant, text: &str, top_k: usize) -> Vec<TieredHit> {
        if top_k == 0 {
            return Vec::new();
        }
        let query_vec = LocalEmbedding::embed(text);
        let query_words = content_words(text);
        let mut hits: Vec<TieredHit> = match tier {
            MemoryTierVariant::Sensory => &self.sensory,
            MemoryTierVariant::ShortTerm => &self.short,
            MemoryTierVariant::LongTerm => &self.long,
        }
        .iter()
        .map(|e| self.fuse(&query_vec, &query_words, e.clone()))
        .collect();
        sort_hits(&mut hits);
        keep_by_relative_score(&mut hits, top_k);
        // 观测用：命中计数递增。**绝不**回流进打分——「查过几次」不是「是否相关」。
        self.touch(&hits);
        hits
    }

    /// 渐进式检索：长时 → 短时 → 感官逐层召回，合并去重后**再**做一次相对截断。
    ///
    /// 逐层截断只保证「本层内不比最好那条差 40%」；跨层合并后仍要给全局地板，
    /// 否则某层整体不相关时，它的局部最佳仍会挤进上下文。
    pub fn progressive_retrieve(&mut self, text: &str, top_k: usize) -> Vec<TieredHit> {
        let wide = top_k.saturating_mul(2).max(1);
        let mut merged = self.recall(MemoryTierVariant::LongTerm, text, wide);
        merged.extend(self.recall(MemoryTierVariant::ShortTerm, text, wide));
        merged.extend(self.recall(MemoryTierVariant::Sensory, text, wide));
        // 同 id 去重（升库后可能同时在多池出现）
        let mut seen = std::collections::HashSet::new();
        merged.retain(|h| seen.insert(h.entry.id));
        sort_hits(&mut merged);
        keep_by_relative_score(&mut merged, top_k);
        merged
    }

    /// 单条候选的幅度：委托 [`scoring::fuse`]（词面为闸门，档内 `max(实词覆盖率, 余弦)`）。
    ///
    /// 实词覆盖率 = 共享实词数 / 查询实词数。分母与候选无关，所以**词面档内排序与
    /// 「按共享词数降序」完全一致**（那正是实测 70.8%/100% 的基线），同时得到一个
    /// 可跨查询比较的分数，供相对截断使用。
    fn fuse(&self, query_vec: &[f64], query_words: &[String], entry: TieredEntry) -> TieredHit {
        let cosine = LocalEmbedding::cosine(query_vec, &LocalEmbedding::embed(&entry.text));
        let overlap = shared_content_words(query_words, &content_words(&entry.text));
        let rank = scoring::fuse(
            overlap,
            scoring::coverage(query_words.len(), overlap),
            cosine,
        );
        TieredHit {
            // 标注线取实例字段（默认即 `scoring::WEAK_EVIDENCE`），便于观测面如实回报
            weak: rank < self.weak_evidence,
            entry,
            cosine,
            keyword_overlap: overlap,
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
/// 排序：**先词面档、后幅度**（[`scoring::lexical_first`]），再按 id 升序保证确定性。
///
/// 后一级 tie-break 不是为了美观，而是为了**确定性**：命中集合会进 System 前缀，
/// 顺序不稳定 = 前缀不稳定 = 前缀缓存失效 + 排查时无法复现（旧实现把 `access_count`
/// 放进分数，正是这个毛病的来源）。
fn sort_hits(hits: &mut [TieredHit]) {
    hits.sort_by(|a, b| {
        scoring::lexical_first(a.keyword_overlap, a.rank, b.keyword_overlap, b.rank)
            .then(a.entry.id.cmp(&b.entry.id))
    });
}

/// 截断（D1）：保留 `scoring::keeps(最佳分, 本条分)` 为真的前 `top_k` 条。
/// 原地修改为截断后的结果。
///
/// 替代旧的绝对门 `cosine ≥ 0.35`。绝对门的失败模式是**饿死**（实测近半查询一条都不注入，
/// 中文最佳余弦中位数 0.346 恰好压在门下）；相对判据只问「这条比本查询能找到的最好那条
/// 差多少」。而「整库都不相关」不再靠数字地板表达：词面证据为零时 [`fuse`] 直接记 0，
/// 于是 `best == 0` → [`keeps`](scoring::keeps) 全假 → 什么都不注入（宁缺毋滥）。
fn keep_by_relative_score(hits: &mut Vec<TieredHit>, top_k: usize) {
    let best = hits.first().map(|h| h.rank).unwrap_or(0.0);
    hits.retain(|h| scoring::keeps(best, h.rank));
    hits.truncate(top_k);
}

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
        assert!(has_family, "expected story-family in top-5, got {:?}", hits.iter().map(|h| (&h.entry.meta, h.entry.id, h.rank)).collect::<Vec<_>>());
        // 第一命中应为同族目标（词面证据最多的一条）
        let top = &hits[0];
        assert_eq!(top.entry.id, target, "expected exact target as top hit");
        assert!(top.rank > 0.0, "词面证据为 0 的命中不该是第一名: {}", top.rank);
        assert!(!top.weak, "共享 6 个实词的命中不该被标成弱证据");
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
        assert_eq!(hits[0].entry.id, target, "target should rank first");
        // 每一条注入的命中都必须满足截断判据（不再有「过阈值的无关项」这一类）
        let best = hits[0].rank;
        for h in &hits {
            assert!(
                scoring::keeps(best, h.rank),
                "命中 {} 的分数 {} 不该通过 cutoff {:.3}",
                h.entry.id,
                h.rank,
                scoring::cutoff(best)
            );
        }
    }

    /// D1 的反面：整库都不相关时，必须真的挡住注入（宁缺毋滥），
    /// 而不是「至少给一条」——后者就是旧绝对门之外的另一种饿死。
    ///
    /// 这条以前靠 `HARD_FLOOR` 挡，实测挡不住：零词面重合的两句话在特征哈希空间里
    /// 也能拿到 0.21（那条 0.211 就是哈希桶碰撞，比 0.15 的地板还高）。现在由
    /// 「共享 0 个实词 = 没有证据」这条闸来挡，与地板的高低无关。
    #[test]
    fn a_wholly_unrelated_library_injects_nothing() {
        let mut m = TieredMemory::new();
        m.insert(MemoryTierVariant::LongTerm, "quantum entanglement observation log", "physics");
        m.insert(MemoryTierVariant::LongTerm, "shipment tracking order 8823 arrived", "logistics");
        let hits = m.recall(MemoryTierVariant::LongTerm, "coffee machine descaling schedule", 5);
        assert!(
            hits.is_empty(),
            "零词面重合、语义也不相关的库不该注入任何一行，got {:?}",
            hits.iter().map(|h| (&h.entry.text, h.rank)).collect::<Vec<_>>()
        );
    }

    /// 读路径不得改变分数：`touch()` 递增 `access_count` 只是观测，
    /// 旧实现把访问次数算进 `rank`，于是「查过几次」会改变相关性 → 前缀不稳定。
    #[test]
    fn repeated_recall_is_deterministic_and_access_count_does_not_leak_into_rank() {
        let mut m = TieredMemory::new();
        fill_corpus(&mut m);
        let query = "the quick fox leaps over the lazy dog";
        let first = m.recall(MemoryTierVariant::LongTerm, query, 5);
        let second = m.recall(MemoryTierVariant::LongTerm, query, 5);
        let ids = |v: &[TieredHit]| v.iter().map(|h| h.entry.id).collect::<Vec<_>>();
        assert_eq!(ids(&first), ids(&second), "同一库同一查询的命中顺序必须稳定");
        for (a, b) in first.iter().zip(&second) {
            assert_eq!(a.rank, b.rank, "分数不得随访问次数变化");
        }
        // 但观测计数确实涨了（证明 touch 仍在，只是不参与打分）
        let touched = second.iter().filter(|h| h.entry.access_count > 0).count();
        assert!(
            touched == second.len(),
            "命中项应全部被计数，got {touched}/{}",
            second.len()
        );
    }

    #[test]
    fn recall_long_term_scans_the_whole_pool() {
        let mut m = TieredMemory::new();
        fill_corpus(&mut m);
        let hits = m.recall(MemoryTierVariant::LongTerm, "the quick fox leaps over the lazy dog", 3);
        assert!(!hits.is_empty());
        assert!(hits.len() <= 3);
        // flyhash 粗排已退出打分：同族三条都该在候选里（旧实现靠粗排拿候选，实测覆盖率 0–89%）
        let story = hits.iter().filter(|h| h.entry.meta == "story").count();
        assert!(story >= 1, "同族目标应被召回");
    }

    /// 弱证据标注（`WEAK_EVIDENCE` 从丢弃门降级为标签）：过得了相对截断、
    /// 但两路证据都弱时仍注入，只是必须带上「这是候选不是事实」的信号。
    #[test]
    fn a_weak_but_injected_hit_is_labelled_not_dropped() {
        let mut m = TieredMemory::new();
        let id = m.insert(MemoryTierVariant::LongTerm, "the quick brown fox", "story");
        let hits = m.recall(MemoryTierVariant::LongTerm, "the quick brown fox", 5);
        let hit = hits
            .iter()
            .find(|h| h.entry.id == id)
            .expect("完全相同的文本必须命中自己");
        assert!(!hit.weak, "同一句话不该被判成弱证据");

        // 弱标签的活样本（实测数：8 个实词的查询共享 1 个实词 → 覆盖率 0.125、
        // 哈希余弦 0.161 → rank 0.161）：够得上截断线，却低于 WEAK_EVIDENCE，
        // 所以它**带着标签进上下文**，而不是被丢弃。
        let mut m2 = TieredMemory::new();
        m2.insert(
            MemoryTierVariant::LongTerm,
            "gateway approval card collapsed after reconnect",
            "note",
        );
        m2.insert(
            MemoryTierVariant::LongTerm,
            "shipment tracking order 8823 arrived",
            "logistics",
        );
        let got = m2.recall(
            MemoryTierVariant::LongTerm,
            "approval ledger snapshot persisted before daemon shutdown rotate logs",
            5,
        );
        assert_eq!(got.len(), 1, "只有那一条有词面证据，另一条零重合不该进上下文");
        assert!(
            got.iter().filter(|h| h.weak).count() == 1,
            "8 个实词里只共享 1 个的命中必须带上弱证据标签（rank={})",
            got[0].rank
        );
        for h in &got {
            assert_eq!(
                h.weak,
                h.rank < WEAK_EVIDENCE,
                "标注与分数自相矛盾：rank={} weak={}",
                h.rank,
                h.weak
            );
            assert!(
                scoring::keeps(got[0].rank, h.rank),
                "被注入的那条应当过得了截断判据"
            );
        }
    }

    /// top_k = 0 是「不要注入」，不是「全都要」——早退，避免把整库排序后交给调用方截断。
    #[test]
    fn zero_top_k_returns_nothing_without_touching() {
        let mut m = TieredMemory::new();
        fill_corpus(&mut m);
        let hits = m.recall(MemoryTierVariant::LongTerm, "the quick fox", 0);
        assert!(hits.is_empty());
        assert!(m.progressive_retrieve("the quick fox", 0).is_empty());
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