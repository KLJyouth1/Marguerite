//! 多层语义缓存（GUI_Agent Block 1）：在已有精确前缀缓存之上叠加「规范化精确」与「语义近似」两层。
//!
//! 设计严格遵循 `cache/mod.rs` 的三原则：
//! 1. **精确优先**——Tier 0 原始 prompt 哈希精确命中（零假阳性）始终最先；
//! 2. **语义可选**——Tier 2 语义层仅作为增强，且阈值保守（默认 0.92），
//!    仅在精确/Tier1 未命中时触发，绝不喧宾夺主，也绝不用于语义关键请求；
//! 3. **可观测**——每层命中分别计量，语义命中附带相似度，便于审计假阳性风险。
//!
//! 所有组件零外部依赖（`std` + `serde`），`Embedder` 以 trait 形式可插拔：默认提供
//! `LocalNgramEmbedder`（字符/词 n-gram + 特征哈希的稠密向量，确定性、可单测）；
//! 生产可注入真实 embedding 模型（DeepSeek/OpenAI embeddings），接口一致。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::{hash_text, CacheEntry, PersistedCache, PrefixCache};

/// 稠密嵌入向量（特征哈希到固定维度，L2 归一化）。
#[derive(Clone, Debug, Default)]
pub struct Embedding(pub Vec<f32>);

impl Embedding {
    /// 余弦相似度 ∈ [-1, 1]；零向量返回 0。
    pub fn cosine(&self, other: &Embedding) -> f64 {
        let n = self.0.len().min(other.0.len());
        let mut dot = 0.0f64;
        let mut na = 0.0f64;
        let mut nb = 0.0f64;
        for i in 0..n {
            let a = self.0[i] as f64;
            let b = other.0[i] as f64;
            dot += a * b;
            na += a * a;
            nb += b * b;
        }
        let denom = na.sqrt() * nb.sqrt();
        if denom == 0.0 {
            0.0
        } else {
            dot / denom
        }
    }
}

/// 文本嵌入器（可插拔；默认 `LocalNgramEmbedder`，生产可换真实模型）。
///
/// 要求 `Debug` 以保证 `MultiTierCache` 可观测（嵌入器配置可打印）。
pub trait Embedder: Send + Sync + std::fmt::Debug {
    fn embed(&self, text: &str) -> Embedding;
}

/// 依赖无关的局部嵌入器：词 + 字符 n-gram 经特征哈希映射到固定维度，TF 加权后 L2 归一化。
///
/// 这是「语义近似」的确定性代理，足以在单测中验证分层逻辑；真实语义强度由生产嵌入器提供。
#[derive(Debug, Clone)]
pub struct LocalNgramEmbedder {
    dims: usize,
}

impl Default for LocalNgramEmbedder {
    fn default() -> Self {
        Self { dims: 1024 }
    }
}

impl LocalNgramEmbedder {
    #[must_use]
    pub fn new(dims: usize) -> Self {
        Self { dims: dims.max(64) }
    }

    fn tokens(&self, text: &str) -> Vec<String> {
        let lower = text.to_ascii_lowercase();
        let mut toks = Vec::new();
        // 词元（alnum 连续段，长度>=2）
        for w in lower.split(|c: char| !c.is_alphanumeric()) {
            if w.len() >= 2 {
                toks.push(w.to_string());
            }
        }
        // 字符 n-gram（2..=3），含标点与空格，捕捉子词/形态相似
        let chars: Vec<char> = lower.chars().collect();
        for n in 2..=3 {
            for w in chars.windows(n) {
                toks.push(w.iter().collect());
            }
        }
        toks
    }
}

impl Embedder for LocalNgramEmbedder {
    fn embed(&self, text: &str) -> Embedding {
        let toks = self.tokens(text);
        let mut tf: HashMap<u64, f64> = HashMap::new();
        for t in &toks {
            *tf.entry(hash_text(t)).or_insert(0.0) += 1.0;
        }
        let mut v = vec![0.0f32; self.dims];
        for (h, c) in tf {
            let dim = (h as usize) % self.dims;
            // TF 加权：1 + log(tf)，抑制高频 token 主导
            v[dim] += (1.0 + c.ln()) as f32;
        }
        let norm = v
            .iter()
            .map(|x| (*x as f64) * (*x as f64))
            .sum::<f64>()
            .sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x = (*x as f64 / norm) as f32;
            }
        }
        Embedding(v)
    }
}

/// 把 prompt 规范化为「应被缓存的不变部分」：
/// 小写 + 剥离易变后缀（UUID / 10+ 位纪元数字 / 已知易变键）+ 压缩空白。
///
/// 仅规范化真正易变的标识，保留其余数字（如「step 3 of 5」与「step 4 of 5」仍不同），
/// 避免 Tier1 规范化精确命中产生假阳性。
#[must_use]
pub fn normalize_prompt(prompt: &str) -> String {
    let lower = prompt.to_ascii_lowercase();
    let s = strip_uuids(&lower);
    let s = strip_epoch_runs(&s);
    let s = strip_volatile_keys(&s);
    s.split_whitespace().collect::<Vec<_>>().join(" ").trim().to_string()
}

fn is_hex(b: u8) -> bool {
    b.is_ascii_hexdigit()
}

/// 剥离 UUID（8-4-4-4-12 十六进制，连字符分隔）。
fn strip_uuids(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if let Some(end) = match_uuid(b, i) {
            out.push_str("<uuid>");
            i = end;
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

fn match_uuid(b: &[u8], i: usize) -> Option<usize> {
    let groups = [8usize, 4, 4, 4, 12];
    let mut pos = i;
    for (gi, &g) in groups.iter().enumerate() {
        if pos + g > b.len() {
            return None;
        }
        for k in 0..g {
            if !is_hex(b[pos + k]) {
                return None;
            }
        }
        pos += g;
        if gi < 4 {
            if pos >= b.len() || b[pos] != b'-' {
                return None;
            }
            pos += 1;
        }
    }
    Some(pos)
}

/// 剥离 ≥10 位的纯数字串（纪元毫秒/纳秒等易变时间戳）。
fn strip_epoch_runs(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i - start >= 10 {
                out.push_str("<epoch>");
            } else {
                out.push_str(&s[start..i]);
            }
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

/// 剥离已知易变键（`key=value` / `key: value` 中的 value）。
fn strip_volatile_keys(s: &str) -> String {
    let keys = [
        "session_id",
        "request_id",
        "trace_id",
        "timestamp",
        "nonce",
        "epoch",
        "created_at",
        "date",
        "time",
    ];
    let mut s = s.to_string();
    for k in keys {
        if let Some(idx) = find_case_insensitive(&s, k) {
            let after = idx + k.len();
            let mut j = after;
            while j < s.len() && (s.as_bytes()[j] == b' ' || s.as_bytes()[j] == b'\t') {
                j += 1;
            }
            if j < s.len() && (s.as_bytes()[j] == b'=' || s.as_bytes()[j] == b':') {
                j += 1;
                while j < s.len() && (s.as_bytes()[j] == b' ' || s.as_bytes()[j] == b'\t') {
                    j += 1;
                }
                let val_start = j;
                while j < s.len() && !s.as_bytes()[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j > val_start {
                    s.replace_range(idx..j, &format!("{k}<redacted>"));
                }
            }
        }
    }
    s
}

fn find_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    let h = haystack.to_ascii_lowercase();
    let n = needle.to_ascii_lowercase();
    h.find(&n)
}

/// 单条语义索引条目（保留原始 prompt，供持久化后由 embedder 重嵌入）。
#[derive(Clone, Debug)]
pub struct SemanticEntry {
    pub prompt: String,
    pub embedding: Embedding,
    pub content: String,
    pub completion_tokens: u32,
}

/// 语义索引：线性扫描 + 余弦相似度，取超过阈值的最高相似条目。
///
/// 规模受限（线性扫描），但 GUI_Agent 语义缓存条目量级（千级）足够；后期可换 ANN 索引。
#[derive(Debug, Default)]
pub struct SemanticIndex {
    entries: Mutex<Vec<SemanticEntry>>,
}

impl SemanticIndex {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(
        &self,
        prompt: &str,
        embedding: Embedding,
        content: String,
        completion_tokens: u32,
    ) {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).push(SemanticEntry {
            prompt: prompt.to_string(),
            embedding,
            content,
            completion_tokens,
        });
    }

    /// 返回相似度 ≥ `threshold` 的最高相似条目（内容 + 相似度 + completion token）。
    #[must_use]
    pub fn search(&self, query: &Embedding, threshold: f64) -> Option<(String, f64, u32)> {
        let g = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let mut best: Option<(f64, usize)> = None;
        for (i, e) in g.iter().enumerate() {
            let sim = e.embedding.cosine(query);
            if sim >= threshold {
                match best {
                    None => best = Some((sim, i)),
                    Some((b, _)) if sim > b => best = Some((sim, i)),
                    _ => {}
                }
            }
        }
        best.map(|(sim, i)| {
            let e = &g[i];
            (e.content.clone(), sim, e.completion_tokens)
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 导出条目（供持久化；加载时由 embedder 重新嵌入）。
    #[must_use]
    pub fn entries(&self) -> Vec<SemanticEntry> {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// 命中来源层级（用于可观测与假阳性审计）。
#[derive(Debug, Clone, PartialEq)]
pub enum HitTier {
    /// Tier 0：原始 prompt 哈希精确命中。
    Exact,
    /// Tier 1：规范化后哈希精确命中（易变后缀已剥离）。
    Normalized,
    /// Tier 2：语义近似命中，附带余弦相似度。
    Semantic(f64),
}

/// 缓存命中响应。
#[derive(Debug, Clone)]
pub struct CachedResponse {
    pub content: String,
    pub completion_tokens: u32,
    pub tier: HitTier,
}

/// 多层缓存可观测指标（每 `get` 计为一次 lookup，诚实不重复计数）。
#[derive(Debug, Clone, Copy, Default)]
pub struct MultiTierStats {
    pub lookups: u64,
    pub exact_hits: u64,
    pub normalized_hits: u64,
    pub semantic_hits: u64,
    pub misses: u64,
    /// 命中而省下的 prompt token 总量（取命中条目的 prompt token）。
    pub saved_prompt_tokens: u64,
}

impl MultiTierStats {
    /// 命中率 = (精确 + 规范化 + 语义命中) / 查询数。`lookups == 0` 时返回 0.0 避免除零；
    /// 结果夹紧到 [0, 1] 以吸收任何计数异常。
    #[must_use]
    pub fn hit_rate(&self) -> f64 {
        if self.lookups == 0 {
            return 0.0;
        }
        let hits = self.total_hits();
        let rate = hits as f64 / self.lookups as f64;
        if rate > 1.0 {
            1.0
        } else {
            rate
        }
    }

    /// 三层命中总数（精确 + 规范化 + 语义）。
    #[must_use]
    pub fn total_hits(&self) -> u64 {
        self.exact_hits + self.normalized_hits + self.semantic_hits
    }

    /// 省下 token 占比的保守估计 = 省下 prompt token / (省下 + 未命中)。
    /// 缺失的"未命中 token 成本"以每次未命中至少 1 token 当量近似；分母为 0 时返回 0.0。
    /// 生产环境应喂入真实未命中 token 计数以得到精确值。
    #[must_use]
    pub fn saved_tokens_ratio(&self) -> f64 {
        let saved = self.saved_prompt_tokens as f64;
        let denom = saved + self.misses as f64;
        if denom <= 0.0 {
            return 0.0;
        }
        let ratio = saved / denom;
        if ratio > 1.0 {
            1.0
        } else {
            ratio
        }
    }
}

#[derive(Default, Debug)]
struct MultiTierStatsInner {
    stats: MultiTierStats,
}

/// 多层语义缓存：组合 Tier 0 精确 + Tier 1 规范化精确 + Tier 2 语义近似。
#[derive(Debug)]
pub struct MultiTierCache {
    exact: PrefixCache,
    normalized: PrefixCache,
    semantic: SemanticIndex,
    embedder: Box<dyn Embedder>,
    semantic_threshold: f64,
    stats: Mutex<MultiTierStatsInner>,
}

impl MultiTierCache {
    /// 构造（指定容量、嵌入器、语义阈值）。
    #[must_use]
    pub fn with_embedder(
        capacity: usize,
        embedder: Box<dyn Embedder>,
        semantic_threshold: f64,
    ) -> Self {
        Self {
            exact: PrefixCache::new(capacity),
            normalized: PrefixCache::new(capacity),
            semantic: SemanticIndex::new(),
            embedder,
            semantic_threshold: semantic_threshold.clamp(0.0, 1.0),
            stats: Mutex::new(MultiTierStatsInner::default()),
        }
    }

    /// 默认构造：局部 n-gram 嵌入器，容量 1024，语义阈值 0.92（保守）。
    #[must_use]
    pub fn new() -> Self {
        Self::with_embedder(1024, Box::new(LocalNgramEmbedder::default()), 0.92)
    }

    /// 查询：精确 → 规范化精确 → 语义近似，逐层短路。
    #[must_use]
    pub fn get(&self, prompt: &str) -> Option<CachedResponse> {
        // Tier 0：原始 prompt 精确
        if let Some(e) = self.exact.get(hash_text(prompt)) {
            self.record(HitTier::Exact, e.prompt_tokens);
            return Some(CachedResponse {
                content: e.content,
                completion_tokens: e.completion_tokens,
                tier: HitTier::Exact,
            });
        }
        // Tier 1：规范化后精确（易变后缀已剥离）
        let norm = normalize_prompt(prompt);
        if let Some(e) = self.normalized.get(hash_text(&norm)) {
            self.record(HitTier::Normalized, e.prompt_tokens);
            return Some(CachedResponse {
                content: e.content,
                completion_tokens: e.completion_tokens,
                tier: HitTier::Normalized,
            });
        }
        // Tier 2：语义近似（仅精确/Tier1 未命中才触发）
        let emb = self.embedder.embed(prompt);
        if let Some((content, sim, ct)) = self.semantic.search(&emb, self.semantic_threshold) {
            self.record(HitTier::Semantic(sim), 0);
            return Some(CachedResponse {
                content,
                completion_tokens: ct,
                tier: HitTier::Semantic(sim),
            });
        }
        // 全部未命中
        self.record_miss();
        None
    }

    /// 写入三层：精确 + 规范化精确 + 语义索引（均以 prompt 为键）。
    pub fn put(&self, prompt: &str, content: &str, prompt_tokens: u32, completion_tokens: u32) {
        let entry = CacheEntry {
            content: content.to_string(),
            prompt_tokens,
            completion_tokens,
            last_used_ms: super::now_ms(),
        };
        self.exact.put(hash_text(prompt), entry.clone());
        let norm = normalize_prompt(prompt);
        self.normalized
            .put(hash_text(&norm), entry);
        let emb = self.embedder.embed(prompt);
        self.semantic
            .insert(prompt, emb, content.to_string(), completion_tokens);
    }

    #[must_use]
    pub fn stats(&self) -> MultiTierStats {
        self.stats.lock().unwrap_or_else(|e| e.into_inner()).stats
    }

    /// 便捷出口：直接取当前缓存命中率（等价于 `self.stats().hit_rate()`）。
    #[must_use]
    pub fn hit_rate(&self) -> f64 {
        self.stats().hit_rate()
    }

    /// 语义层条目数（容量观测）。
    #[must_use]
    pub fn semantic_len(&self) -> usize {
        self.semantic.len()
    }

    fn record(&self, tier: HitTier, saved: u32) {
        let mut g = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        g.stats.lookups += 1;
        g.stats.saved_prompt_tokens += saved as u64;
        match tier {
            HitTier::Exact => g.stats.exact_hits += 1,
            HitTier::Normalized => g.stats.normalized_hits += 1,
            HitTier::Semantic(_) => g.stats.semantic_hits += 1,
        }
    }

    fn record_miss(&self) {
        let mut g = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        g.stats.lookups += 1;
        g.stats.misses += 1;
    }

    /// 落盘：精确 + 规范化 + 语义（语义存原始 prompt，加载时由 embedder 重嵌入，保证与嵌入器一致）。
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let payload = PersistedMultiTier {
            exact: self.exact.snapshot(),
            normalized: self.normalized.snapshot(),
            semantic: self
                .semantic
                .entries()
                .into_iter()
                .map(|e| SemanticRecord {
                    prompt: e.prompt,
                    content: e.content,
                    completion_tokens: e.completion_tokens,
                })
                .collect(),
        };
        let data = serde_json::to_vec(&payload)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &data)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// 加载（容量取 `max(256, 已加载条目数)`，避免冷上下文立即被驱逐）。
    #[must_use]
    pub fn open(path: &Path, embedder: Box<dyn Embedder>, semantic_threshold: f64) -> Self {
        let mut cache = Self::with_embedder(256, embedder, semantic_threshold);
        if let Ok(data) = std::fs::read(path) {
            if let Ok(p) = serde_json::from_slice::<PersistedMultiTier>(&data) {
                for rec in &p.semantic {
                    let emb = cache.embedder.embed(&rec.prompt);
                    cache
                        .semantic
                        .insert(&rec.prompt, emb, rec.content.clone(), rec.completion_tokens);
                }
                let cap = p
                    .exact
                    .map
                    .len()
                    .max(p.normalized.map.len())
                    .max(256);
                cache.exact = PrefixCache::from_snapshot(p.exact, cap);
                cache.normalized = PrefixCache::from_snapshot(p.normalized, cap);
            }
        }
        cache
    }
}

#[derive(Serialize, Deserialize)]
struct PersistedMultiTier {
    exact: PersistedCache,
    normalized: PersistedCache,
    semantic: Vec<SemanticRecord>,
}

#[derive(Serialize, Deserialize)]
struct SemanticRecord {
    prompt: String,
    content: String,
    completion_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ngram_embedder_is_deterministic_and_orders_similarity() {
        let e = LocalNgramEmbedder::default();
        let a = e.embed("What is the capital of France?");
        let a2 = e.embed("What is the capital of France?");
        assert_eq!(a.0, a2.0, "同一文本应得到相同嵌入");
        let para = e.embed("Tell me the capital city of France.");
        let unrelated = e.embed("How do I bake sourdough bread from scratch?");
        let sim_para = a.cosine(&para);
        let sim_unrel = a.cosine(&unrelated);
        assert!(
            sim_para > sim_unrel,
            " paraphrased 应比无关文本更相似 (para={sim_para:.3} unrel={sim_unrel:.3})"
        );
        assert!(sim_para > 0.0);
    }

    #[test]
    fn normalize_strips_volatile_suffix_but_keeps_real_content() {
        let a = normalize_prompt("system: helper\ntimestamp=2026-08-18T05:47:38Z\nQ: what is 2+2?");
        let b = normalize_prompt("system: helper\ntimestamp=2026-01-01T00:00:00Z\nQ: what is 2+2?");
        assert_eq!(a, b, "仅时间戳不同应规范化为同一键（Tier1 可命中）");
        // UUID 也应被剥离
        let c = normalize_prompt("id=123e4567-e89b-12d3-a456-426614174000 step 3 of 5");
        let d = normalize_prompt("id=999e9999-e89b-12d3-a456-426614174000 step 3 of 5");
        assert_eq!(c, d, "UUID 不同应规范化为同一键");
        // 真实内容不同（step 编号短数字）应保留差异
        let e1 = normalize_prompt("step 3 of 5");
        let e2 = normalize_prompt("step 4 of 5");
        assert_ne!(e1, e2, "短数字（非纪元）应保留，避免 Tier1 假阳性");
    }

    #[test]
    fn semantic_index_threshold_guards_false_positives() {
        let idx = SemanticIndex::new();
        let e = LocalNgramEmbedder::default();
        idx.insert(
            "What is the capital of France?",
            e.embed("What is the capital of France?"),
            "Paris".into(),
            1,
        );
        let paraphrase = e.embed("Tell me the capital city of France.");
        let bread = e.embed("How do I bake sourdough bread from scratch?");
        // 低阈值：paraphrase 命中，bread 不命中
        assert!(idx.search(&paraphrase, 0.5).is_some(), "paraphrase 应超过低阈值");
        assert!(idx.search(&bread, 0.5).is_none(), "无关文本不应命中");
        // 高阈值：paraphrase 也不命中（防止假阳性）
        assert!(
            idx.search(&paraphrase, 0.999).is_none(),
            "过高阈值应拒绝近似命中"
        );
    }

    #[test]
    fn multi_tier_exact_normalized_semantic_in_order() {
        let cache = MultiTierCache::with_embedder(
            64,
            Box::new(LocalNgramEmbedder::default()),
            0.5,
        );
        // Tier 0：精确
        cache.put("hello world", "Hi there", 5, 1);
        let r = cache.get("hello world").expect("Tier0 应命中");
        assert_eq!(r.tier, HitTier::Exact);
        // Tier 1：仅时间戳不同的同源 prompt
        cache.put(
            "Q: translate hello\ntimestamp=2026-08-18T05:47:38Z",
            "Bonjour",
            10,
            1,
        );
        let r = cache
            .get("Q: translate hello\ntimestamp=2026-01-01T00:00:00Z")
            .expect("Tier1 应命中");
        assert_eq!(r.tier, HitTier::Normalized);
        assert_eq!(r.content, "Bonjour");
        // Tier 2：释义性 query（无精确/Tier1 命中，靠语义）
        cache.put("What is the capital of France?", "Paris", 12, 1);
        let r = cache
            .get("Tell me the capital city of France.")
            .expect("Tier2 应语义命中");
        assert!(matches!(r.tier, HitTier::Semantic(_)));
        assert_eq!(r.content, "Paris");
        // 全未命中
        assert!(cache.get("completely unrelated xyz query 12345").is_none());
        // 统计
        let s = cache.stats();
        assert_eq!(s.exact_hits, 1);
        assert_eq!(s.normalized_hits, 1);
        assert_eq!(s.semantic_hits, 1);
        assert_eq!(s.misses, 1);
        assert_eq!(s.lookups, 4, "每次 get 计一次 lookup，不重复计数");
        assert_eq!(s.saved_prompt_tokens, 5 + 10 + 0); // 语义命中不计入 prompt 节省
    }

    #[test]
    fn multi_tier_persistence_roundtrip() {
        let path = std::env::temp_dir().join(format!("marg_multitier_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let cache = MultiTierCache::with_embedder(
                64,
                Box::new(LocalNgramEmbedder::default()),
                0.5,
            );
            cache.put("hello world", "Hi there", 5, 1);
            cache.put("What is the capital of France?", "Paris", 12, 1);
            cache.save(&path).expect("落盘应成功");
        }
        // 重新加载（模拟重启），embedder 重新嵌入语义条目
        let cache = MultiTierCache::open(
            &path,
            Box::new(LocalNgramEmbedder::default()),
            0.5,
        );
        let r = cache.get("hello world").expect("精确应恢复");
        assert_eq!(r.tier, HitTier::Exact);
        assert_eq!(r.content, "Hi there");
        let r = cache
            .get("Tell me the capital city of France.")
            .expect("语义应重建并命中");
        assert!(matches!(r.tier, HitTier::Semantic(_)));
        assert_eq!(r.content, "Paris");
        let _ = std::fs::remove_file(&path);
    }

    // ---- Track E: MultiTierStats 命中率导出（最小加性） ----

    fn stats_with(
        lookups: u64,
        exact: u64,
        normalized: u64,
        semantic: u64,
        misses: u64,
        saved: u64,
    ) -> MultiTierStats {
        MultiTierStats {
            lookups,
            exact_hits: exact,
            normalized_hits: normalized,
            semantic_hits: semantic,
            misses,
            saved_prompt_tokens: saved,
        }
    }

    #[test]
    fn hit_rate_computed_correctly() {
        // 3+2+1 = 6 命中 / 10 查询 -> 0.6
        let s = stats_with(10, 3, 2, 1, 4, 100);
        assert_eq!(s.total_hits(), 6);
        assert!((s.hit_rate() - 0.6).abs() < 1e-9);
    }

    #[test]
    fn hit_rate_zero_lookups_no_div_by_zero() {
        let s = stats_with(0, 0, 0, 0, 0, 0);
        assert_eq!(s.hit_rate(), 0.0);
        assert_eq!(s.saved_tokens_ratio(), 0.0);
    }

    #[test]
    fn hit_rate_in_range_zero_to_one() {
        // 全命中：命中率恰为 1.0
        let s = stats_with(10, 10, 0, 0, 0, 0);
        assert_eq!(s.hit_rate(), 1.0);
        // 全未命中：0.0
        let s = stats_with(10, 0, 0, 0, 10, 0);
        assert_eq!(s.hit_rate(), 0.0);
    }

    #[test]
    fn saved_tokens_ratio_estimate() {
        // 省下 100 / (100 + 100 未命中) -> 0.5
        let s = stats_with(10, 1, 1, 1, 100, 100);
        assert!((s.saved_tokens_ratio() - 0.5).abs() < 1e-9);
        // 零未命中且零省下 -> 0.0
        let s = stats_with(5, 5, 0, 0, 0, 0);
        assert_eq!(s.saved_tokens_ratio(), 0.0);
    }

    #[test]
    fn cache_hit_rate_matches_stats() {
        let cache = MultiTierCache::with_embedder(
            64,
            Box::new(LocalNgramEmbedder::default()),
            0.5,
        );
        cache.put("hello world", "Hi there", 5, 1);
        let _ = cache.get("hello world"); // 命中
        let _ = cache.get("totally unrelated query xyz 98765"); // 未命中
        assert!((cache.hit_rate() - cache.stats().hit_rate()).abs() < 1e-12);
        assert_eq!(cache.stats().lookups, 2);
    }

    // 变异测试证伪橡皮章：若 `hit_rate` 分子被写死为 0（total_hits() 返回 0），
    // 以下断言会 FAILED —— 证明测试真的在观测计算，而非空过。
    #[test]
    fn hit_rate_mutation_proof_numerator_not_hardcoded_zero() {
        let s = stats_with(10, 3, 2, 1, 4, 100);
        assert!(s.hit_rate() > 0.0, "命中率应 > 0；若被写死 0 则变异暴露");
    }
}
