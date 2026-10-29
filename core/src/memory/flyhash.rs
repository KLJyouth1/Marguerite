//! 果蝇嗅觉稀疏投影哈希（FlyHash）与 WTA 压缩指纹
//!
//! 模拟果蝇嗅球 → 肯扬细胞（Kenyon cells）的神经编码：
//! - **稀疏投影**：文本 token 经种子化确定性随机投影到高维向量
//!   （`FLY_DIM=2048`，每 token 激活约 5% 的肯扬细胞），同族记忆在
//!   Hash 空间近邻、异族远离，天然去重与粗排；
//! - **WTA 压缩**：多组带种子随机窗口的 Winner-Take-All，把高维向量
//!   压缩为短指纹（`WTA_ROUNDS` 个 u16 坐标），相似文本共享更多 winner；
//! - **近邻检索**：共享 winner 数近似 Jaccard 粗排，为 embedding 精排
//!   提供候选集，避免全量扫描。
//!
//! 纯 Rust、无宿主依赖、可单测；确定性由 splitmix64 种子化伪随机保证。

use std::collections::HashMap;

use crate::memory::lexical;

/// 高维投影维度（肯扬细胞数量）
pub const FLY_DIM: usize = 2048;
/// 目标稀疏度：每个 token 激活的肯扬细胞比例（约 5%）
pub const FLY_SPARSITY: f64 = 0.05;
/// 每个 token 激活的肯扬细胞数量（`FLY_DIM * FLY_SPARSITY = 102`）
pub const FLY_ON_BITS: usize = 102;
/// WTA 窗口大小：每轮比较的候选坐标数
pub const WTA_WINDOW: usize = 16;
/// WTA 轮数：压缩指纹长度
pub const WTA_ROUNDS: usize = 64;
/// WTA 哨兵值：窗口内无激活位时记录，表示该轮无 winner
pub const WTA_EMPTY: u16 = FLY_DIM as u16;

/// splitmix64：确定性伪随机数生成
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// FNV-1a 哈希（带种子），与 long_term 的 hash_to 相同口径
fn fnv1a(s: &str, seed: u64) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ seed.wrapping_mul(0x100_0000_01b3);
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// 由 (seed, index) 确定性生成一个 0..FLY_DIM 坐标
fn coord_for(seed: u64, index: usize) -> usize {
    let mut state = seed
        .wrapping_mul(0xd1b5_4a32_d192_ed03)
        .wrapping_add((index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    (splitmix64(&mut state) % FLY_DIM as u64) as usize
}

/// 激活一个 token：生成 `FLY_ON_BITS` 个互异坐标（确定性、可复现）
fn activate_token(token: &str, seed: u64) -> Vec<usize> {
    let mut out = Vec::with_capacity(FLY_ON_BITS);
    let mut seen = Vec::with_capacity(FLY_ON_BITS);
    let mut i = 0usize;
    let mut guard = 0usize;
    while out.len() < FLY_ON_BITS && guard < 4096 {
        let idx = coord_for(seed.wrapping_add(fnv1a(token, seed)), i);
        i += 1;
        guard += 1;
        if !seen.contains(&idx) {
            seen.push(idx);
            out.push(idx);
        }
    }
    out
}

/// 分词口径统一在 [`crate::memory::lexical`]（单一事实源）。
///
/// 这里原本另有一份拷贝，注释写着「与 long_term 的 tokenize 相同口径」——
/// 靠注释维持的口径一致必然漂移，而两路（flyhash 粗排 / 余弦精排）一旦口径不同，
/// 「粗排候选 + 精排分数」的组合就变成两个互不相干的模型在猜同一件事。
fn tokenize(text: &str) -> Vec<String> {
    lexical::tokens(text)
}

/// 高维稀疏向量（激活计数，u16 防重叠溢出）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparseVector {
    counts: Vec<u16>,
}

impl SparseVector {
    pub fn new() -> Self {
        Self {
            counts: vec![0u16; FLY_DIM],
        }
    }

    pub fn add(&mut self, idx: usize) {
        if let Some(c) = self.counts.get_mut(idx) {
            *c = c.saturating_add(1);
        }
    }

    pub fn get(&self, idx: usize) -> u16 {
        self.counts.get(idx).copied().unwrap_or(0)
    }

    /// 已激活维度数量（支撑集大小）
    pub fn activated(&self) -> usize {
        self.counts.iter().filter(|&&c| c > 0).count()
    }

    /// 稀疏度（激活维度 / 总维度）
    pub fn sparsity(&self) -> f64 {
        self.activated() as f64 / FLY_DIM as f64
    }

    /// 支撑集对称差（Hamming 距离：仅一方激活的维度数）
    pub fn hamming(&self, other: &SparseVector) -> usize {
        self.counts
            .iter()
            .zip(&other.counts)
            .filter(|(a, b)| (**a > 0) != (**b > 0))
            .count()
    }

    /// 支撑集 Jaccard 相似度
    pub fn jaccard(&self, other: &SparseVector) -> f64 {
        let mut shared = 0usize;
        let mut union = 0usize;
        for (a, b) in self.counts.iter().zip(&other.counts) {
            if a > &0 || b > &0 {
                union += 1;
            }
            if a > &0 && b > &0 {
                shared += 1;
            }
        }
        if union == 0 {
            0.0
        } else {
            shared as f64 / union as f64
        }
    }

    /// Top-K 稀疏化：只保留激活计数最高的 `k` 个维度（并列时保留小下标），
    /// 其余清零。记忆项整体稀疏度由此稳定在 `k / FLY_DIM`（约 5%），
    /// 同时保留「同族共享高计数维度 → 近邻」的区分力。
    pub fn sparsify_topk(&mut self, k: usize) {
        if k == 0 {
            self.counts.iter_mut().for_each(|c| *c = 0);
            return;
        }
        let mut order: Vec<usize> = (0..FLY_DIM).filter(|&i| self.counts[i] > 0).collect();
        order.sort_by(|&a, &b| {
            self.counts[b]
                .cmp(&self.counts[a])
                .then_with(|| a.cmp(&b))
        });
        let keep: std::collections::HashSet<usize> = order.into_iter().take(k).collect();
        for (i, c) in self.counts.iter_mut().enumerate() {
            if !keep.contains(&i) {
                *c = 0;
            }
        }
    }
}

impl Default for SparseVector {
    fn default() -> Self {
        Self::new()
    }
}

/// 将文本投影为高维稀疏向量（确定性：同文本同 seed 结果一致）
pub fn project(text: &str, seed: u64) -> SparseVector {
    let mut vec = SparseVector::new();
    let tokens = tokenize(text);
    if tokens.is_empty() {
        return vec;
    }
    for t in &tokens {
        for idx in activate_token(t, seed) {
            vec.add(idx);
        }
    }
    // 记忆项整体 Top-K 稀疏化：无论 token 多少，激活维度稳定在 ~5%
    vec.sparsify_topk(FLY_ON_BITS);
    vec
}

/// WTA 压缩指纹：`WTA_ROUNDS` 个 u16 坐标（`WTA_EMPTY` 表示该轮无 winner）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WtaFingerprint {
    winners: Vec<u16>,
}

impl WtaFingerprint {
    pub fn len(&self) -> usize {
        self.winners.len()
    }

    pub fn is_empty(&self) -> bool {
        self.winners.is_empty()
    }

    pub fn winners(&self) -> &[u16] {
        &self.winners
    }

    /// 非空 winner 的数量
    pub fn nonempty(&self) -> usize {
        self.winners.iter().filter(|&&w| w != WTA_EMPTY).count()
    }

    /// 相似度：双方共享的真实 winner 数 / 双方非空 winner 数的较小值
    /// （近似 Jaccard，0..1 单调，用于粗排）
    pub fn similarity(&self, other: &WtaFingerprint) -> f64 {
        let mut shared = 0usize;
        let mut denom = 0usize;
        for (a, b) in self.winners.iter().zip(&other.winners) {
            if *a != WTA_EMPTY && *b != WTA_EMPTY {
                denom += 1;
                if a == b {
                    shared += 1;
                }
            }
        }
        if denom == 0 {
            0.0
        } else {
            shared as f64 / denom as f64
        }
    }
}

/// 对稀疏向量做 WTA 压缩（确定性）
pub fn wta_compress(vec: &SparseVector) -> WtaFingerprint {
    let mut winners = Vec::with_capacity(WTA_ROUNDS);
    for r in 0..WTA_ROUNDS {
        let mut coords = Vec::with_capacity(WTA_WINDOW);
        let mut state = (r as u64).wrapping_mul(0xe703_7ed1_a0b4_28db) ^ 0x9e37_79b9_7f4a_7c15;
        let mut guard = 0usize;
        while coords.len() < WTA_WINDOW && guard < 256 {
            let idx = (splitmix64(&mut state) % FLY_DIM as u64) as usize;
            guard += 1;
            if !coords.contains(&idx) {
                coords.push(idx);
            }
        }
        let mut best = WTA_EMPTY;
        let mut best_val = 0u16;
        for &idx in &coords {
            let val = vec.get(idx);
            if val > best_val || (val == best_val && (best == WTA_EMPTY || idx < best as usize)) {
                best = idx as u16;
                best_val = val;
            }
        }
        winners.push(best);
    }
    WtaFingerprint { winners }
}

/// 单条索引条目
#[derive(Debug, Clone)]
pub struct FlyIndexEntry {
    pub key: String,
    pub text: String,
    pub sparse: SparseVector,
    pub fingerprint: WtaFingerprint,
}

/// 查询命中
#[derive(Debug, Clone)]
pub struct FlyHit {
    pub key: String,
    pub text: String,
    pub score: f64,
}

/// FlyHash 稀疏记忆索引：插入与近邻粗排
#[derive(Debug, Clone, Default)]
pub struct FlyHashIndex {
    entries: HashMap<String, FlyIndexEntry>,
    seed: u64,
}

impl FlyHashIndex {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            seed: 0x5eed_2026_c0de_2026,
        }
    }

    pub fn with_seed(seed: u64) -> Self {
        Self {
            entries: HashMap::new(),
            seed,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// 插入（或覆盖）一条记忆
    pub fn insert(&mut self, key: impl Into<String>, text: impl Into<String>) {
        let key = key.into();
        let text = text.into();
        let sparse = project(&text, self.seed);
        let fingerprint = wta_compress(&sparse);
        self.entries.insert(
            key.clone(),
            FlyIndexEntry {
                key,
                text,
                sparse,
                fingerprint,
            },
        );
    }

    pub fn remove(&mut self, key: &str) -> Option<FlyIndexEntry> {
        self.entries.remove(key)
    }

    pub fn get(&self, key: &str) -> Option<&FlyIndexEntry> {
        self.entries.get(key)
    }

    /// 查询文本的近邻（按指纹相似度降序取 top-k）
    pub fn query(&self, text: &str, k: usize) -> Vec<FlyHit> {
        self.query_sparse(&project(text, self.seed), k)
    }

    /// 查询稀疏向量的近邻（按指纹相似度降序取 top-k）
    pub fn query_sparse(&self, sparse: &SparseVector, k: usize) -> Vec<FlyHit> {
        let fp = wta_compress(sparse);
        let mut hits: Vec<FlyHit> = self
            .entries
            .values()
            .map(|e| FlyHit {
                key: e.key.clone(),
                text: e.text.clone(),
                score: e.fingerprint.similarity(&fp),
            })
            .collect();
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(k);
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAMILY_A: [&str; 3] = [
        "the quick brown fox jumps over the lazy dog",
        "a quick brown fox leaps over the sleepy canine",
        "the fast brown fox vaults above the resting hound",
    ];
    const FAMILY_B: [&str; 3] = [
        "machine learning models are trained on large datasets",
        "deep neural networks learn from massive data corpora",
        "artificial intelligence systems require gigabytes of training data",
    ];

    fn unrelated_corpus(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| format!("sensor reading channel {} reports voltage level {} and temperature {}", i, i * 7 % 13, i * 3 + 20))
            .collect()
    }

    #[test]
    fn sparse_projection_is_deterministic() {
        // 同文本同 seed → 完全相同的高维向量与指纹
        let a = project("the quick brown fox jumps over the lazy dog", 42);
        let b = project("the quick brown fox jumps over the lazy dog", 42);
        assert_eq!(a, b);
        assert_eq!(wta_compress(&a), wta_compress(&b));

        // 不同 seed → 投影不同（保证 seed 生效）
        let c = project("the quick brown fox jumps over the lazy dog", 43);
        assert_ne!(a, c);
    }

    #[test]
    fn sparse_projection_reaches_target_sparsity() {
        let text = "the quick brown fox jumps over the lazy dog and the agile cat chases the mouse";
        let vec = project(text, 7);
        // 稀疏度接近 5%（允许 2%~10% 波动，由 token 去重决定）
        assert!(vec.sparsity() + 1e-9 >= 0.02, "sparsity too low: {}", vec.sparsity());
        assert!(vec.sparsity() - 1e-9 <= 0.10, "sparsity too high: {}", vec.sparsity());
        // 确有一定量激活维度
        assert!(vec.activated() > 0);
    }

    #[test]
    fn wta_compression_reduces_dimension() {
        let text = "the quick brown fox jumps over the lazy dog";
        let vec = project(text, 0);
        let fp = wta_compress(&vec);
        // 指纹长度固定 WTA_ROUNDS，且远小于高维向量
        assert_eq!(fp.len(), WTA_ROUNDS);
        assert!(fp.nonempty() > 0);
    }

    #[test]
    fn wta_similarity_separates_families() {
        let mut idx = FlyHashIndex::new();
        for (i, t) in FAMILY_A.iter().enumerate() {
            idx.insert(format!("a{}", i), *t);
        }
        for (i, t) in FAMILY_B.iter().enumerate() {
            idx.insert(format!("b{}", i), *t);
        }
        let query = "a quick brown fox jumps over the lazy dog";
        let hits = idx.query(query, 10);
        assert!(!hits.is_empty());
        // 命中项里同族靠前
        let top = &hits[0];
        assert!(top.key.starts_with('a'), "top hit should be family A, got {}", top.key);
        // 同族分数超过异族最低分（同族更近）
        let a_min = hits
            .iter()
            .filter(|h| h.key.starts_with('a'))
            .map(|h| h.score)
            .fold(f64::INFINITY, f64::min);
        let b_max = hits
            .iter()
            .filter(|h| h.key.starts_with('b'))
            .map(|h| h.score)
            .fold(0.0, f64::max);
        assert!(
            a_min > b_max,
            "family A min {} should exceed family B max {}",
            a_min,
            b_max
        );
    }

    #[test]
    fn nearest_neighbor_recall_meets_threshold() {
        // 语料：50 条不相关 + 同族目标；查询改写后 top-k 能召回同族
        let mut idx = FlyHashIndex::new();
        for (i, t) in unrelated_corpus(50).iter().enumerate() {
            idx.insert(format!("u{}", i), t.clone());
        }
        // 在噪声后注入同族，模拟跨会话记忆库中的真实分布
        for (i, t) in FAMILY_A.iter().enumerate() {
            idx.insert(format!("family{}", i), *t);
        }
        let query = "the quick fox leaps over the lazy dog tonight";
        let hits = idx.query(query, 10);
        let recalled = hits
            .iter()
            .filter(|h| h.key.starts_with("family"))
            .count();
        assert!(recalled >= 1, "expected >=1 family recall in top-10, got {recalled}");
        // 精度：top-5 内不允许无关项挤占同族（同族应至少占 1 席）
        let top5_family = hits
            .iter()
            .take(5)
            .filter(|h| h.key.starts_with("family"))
            .count();
        assert!(top5_family >= 1, "expected family in top-5");
    }
}