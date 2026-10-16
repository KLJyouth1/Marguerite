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
use std::sync::{Arc, Mutex};

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

/// 文本嵌入器（可插拔；默认 `LocalNgramEmbedder`，生产走 [`ProviderEmbedder`]）。
///
/// 要求 `Debug` 以保证 `MultiTierCache` 可观测（嵌入器配置可打印）。
pub trait Embedder: Send + Sync + std::fmt::Debug {
    fn embed(&self, text: &str) -> Embedding;

    /// 向量维度（可观测：换后端后旧语义条目按「重新嵌入」处理，维度不可跨后端比较）
    fn dim(&self) -> usize;

    /// 当前生效后端名（诚实上报用；默认 `custom`，本地特征哈希为 `local-hash`）
    fn backend_name(&self) -> &'static str {
        "custom"
    }
}

/// 真实嵌入后端适配器：把进程级 [`EmbeddingProvider`]（`embeddings.backend` 可切成真实模型）
/// 接到 [`Embedder`] 接缝上。
///
/// 这是文档 1「语义缓存未接真实 embedding」的收口：此前生产路径（`cache::install_semantic_layer`、
/// `agent::bloom_cache::embedding_similarity`）**写死** `LocalNgramEmbedder`/`LocalEmbedding`，
/// 于是配置了真实嵌入后端也不会被语义缓存使用。
///
/// 持有构造时捕获的 `Arc<dyn Embedding>` 而非每次读全局：热路径少一次加锁，
/// 且单测可注入假后端验证「缓存真的用了它」，不必改动进程级全局（避免并行测试串扰）。
pub struct ProviderEmbedder {
    backend: Arc<dyn crate::memory::embedding::Embedding>,
}

impl ProviderEmbedder {
    /// 取当前进程生效后端（生产路径：`EmbeddingProvider::configure` 之后构造）
    #[must_use]
    pub fn new() -> Self {
        Self::from_backend(crate::memory::embedding::EmbeddingProvider::global())
    }

    /// 显式注入后端（DI 接缝：单测用假后端，或按调用方配置定制）
    #[must_use]
    pub fn from_backend(backend: Arc<dyn crate::memory::embedding::Embedding>) -> Self {
        Self { backend }
    }
}

impl Default for ProviderEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ProviderEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderEmbedder")
            .field("backend", &self.backend.name())
            .field("dim", &self.backend.dim())
            .finish()
    }
}

impl Embedder for ProviderEmbedder {
    fn embed(&self, text: &str) -> Embedding {
        Embedding(
            self.backend
                .embed(text)
                .into_iter()
                .map(|x| x as f32)
                .collect(),
        )
    }

    fn dim(&self) -> usize {
        self.backend.dim()
    }

    fn backend_name(&self) -> &'static str {
        self.backend.name()
    }
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

    fn dim(&self) -> usize {
        self.dims
    }

    fn backend_name(&self) -> &'static str {
        "local-ngram"
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
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
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
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(SemanticEntry {
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
    /// Tier 0 精确命中省下的 prompt token（**分层真实计量**，供 UI 如实展示三层来源；
    /// 绝不用总量按比例摊派——那会把观测变成编造）。
    pub exact_saved_tokens: u64,
    /// Tier 1 规范化命中省下的 prompt token
    pub normalized_saved_tokens: u64,
    /// Tier 2 语义命中省下的 prompt token
    pub semantic_saved_tokens: u64,
    /// 工具路径（永远 `ExactOnly` 策略）真实查询数
    pub tool_lookups: u64,
    /// 工具路径精确命中数
    pub tool_hits: u64,
    /// 工具路径未命中数（`tool_lookups == tool_hits + tool_misses`）
    pub tool_misses: u64,
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

/// 分层查询策略：决定 `get` 允许逐层短路到哪些层级。
///
/// 设计取舍（与 CHANGELOG「100% 缓存命中率」目标对齐但**绝不冒充**）：
/// - `ExactOnly`：仅 Tier 0 精确命中（零假阳性），用于携带工具定义的请求路径——
///   近似命中会静默丢弃工具调用，是真实正确性风险，故工具路径永远走这一档。
/// - `NormalizedOnly`：Tier 0 + Tier 1（规范化精确），用于普通文本路径的默认行为；
///   规范化仅剥离易变后缀（UUID/时间戳/易变键）并小写折叠，假阳性极低。
/// - `Full`：再叠加 Tier 2 语义近似（需显式开启，默认关闭）——近似命中可能有假阳性，
///   仅适合纯问答且启用方已接受风险。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierPolicy {
    ExactOnly,
    NormalizedOnly,
    Full,
}

/// 多层语义缓存：组合 Tier 0 精确 + Tier 1 规范化精确 + Tier 2 语义近似。
///
/// `exact` 可为共享的 `Arc<PrefixCache>`（网关路径即复用进程级 `gateway_prefix_cache()`，
/// 保证 Tier 0 与既有精确缓存同源、零重复计数）。`exact_shared = true` 时 `save` 不再
/// 落盘 `exact` 层（由共享方负责），`open_with_shared_exact` 也跳过恢复它。
#[derive(Debug)]
pub struct MultiTierCache {
    exact: Arc<PrefixCache>,
    exact_shared: bool,
    normalized: PrefixCache,
    semantic: SemanticIndex,
    embedder: Box<dyn Embedder>,
    semantic_threshold: f64,
    /// Tier 2 语义近似是否开启（默认 false：近似命中有假阳性风险，需显式开启）。
    semantic_enabled: bool,
    stats: Mutex<MultiTierStatsInner>,
}

/// 解析 `MARGUERITE_SEMANTIC_CACHE` 策略值（纯函数，便于测试，不触碰进程 env）。
/// - `off` → false（Tier 2 近似永不参与，最大确定性）
/// - `on` / `auto` / 空 / 其他 → true（受控场景提命中率；阈值 0.92 保守）
/// - `None`（未设置）→ true（保持历史契约：默认开启）
fn parse_semantic_env_policy(raw: Option<&str>) -> bool {
    match raw {
        Some(v) => {
            let s = v.trim().to_ascii_lowercase();
            s != "off"
        }
        None => true,
    }
}

/// 语义层默认开启策略（upgrade-masterplan §6 阶段 C）。
///
/// 读 `MARGUERITE_SEMANTIC_CACHE` 环境变量，经 [`parse_semantic_env_policy`] 解析。
///
/// 注意：网关侧的 `with_shared_exact` 始终强制 `semantic_enabled = false`，
/// 不受此函数影响——网关精确优先契约独立成立。
fn semantic_default_enabled() -> bool {
    parse_semantic_env_policy(std::env::var("MARGUERITE_SEMANTIC_CACHE").ok().as_deref())
}

/// Tier 2（语义近似）**显式选入**开关：只有 `on` / `1` / `true` 才算开启。
///
/// 与 [`parse_semantic_env_policy`] 的区别是「默认值的方向」：
/// - 后者是构造 `MultiTierCache` 时的历史默认（未设置 → 开），只对**独立构造**的缓存生效；
/// - 本函数用于**生产路径**（网关多层缓存 + LLM 请求策略），未设置/`auto`/其它值一律**关闭**，
///   因为近似命中一旦误判就会返回一个错误的缓存答案；要它生效必须显式选入。
///
/// 这也是文档 1「语义缓存未接真实 embedding」的第二半：光把嵌入器接上真实后端还不够——
/// 生产路径此前既不选入 Tier 2（网关强制关闭）、也从不传 `TierPolicy::Full`，
/// 于是整个语义层永远走不到，真实嵌入后端配了也无从生效。
#[must_use]
pub fn tier2_opted_in(raw: Option<&str>) -> bool {
    matches!(
        raw.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("on") | Some("1") | Some("true")
    )
}

/// 从进程环境读 Tier 2 选入开关（`MARGUERITE_SEMANTIC_CACHE`，唯一读取点）
#[must_use]
pub fn tier2_opted_in_from_env() -> bool {
    tier2_opted_in(std::env::var("MARGUERITE_SEMANTIC_CACHE").ok().as_deref())
}

/// LLM 请求路径应采用的查询策略（纯函数，便于单测与「策略只定义一次」）：
/// 选入 Tier 2 时用 [`TierPolicy::Full`]（Tier 0 → Tier 1 → Tier 2），否则用
/// [`TierPolicy::NormalizedOnly`]（只做精确 + 规范化精确，零近似假阳性）。
#[must_use]
pub fn text_path_policy(tier2: bool) -> TierPolicy {
    if tier2 {
        TierPolicy::Full
    } else {
        TierPolicy::NormalizedOnly
    }
}

impl MultiTierCache {
    /// 构造（指定容量、嵌入器、语义阈值）。`exact` 层独立持有（非共享）。
    /// `semantic_enabled` 由 `semantic_default_enabled()` 决定（env 驱动，默认开启）。
    #[must_use]
    pub fn with_embedder(
        capacity: usize,
        embedder: Box<dyn Embedder>,
        semantic_threshold: f64,
    ) -> Self {
        Self {
            exact: Arc::new(PrefixCache::new(capacity)),
            exact_shared: false,
            normalized: PrefixCache::new(capacity),
            semantic: SemanticIndex::new(),
            embedder,
            semantic_threshold: semantic_threshold.clamp(0.0, 1.0),
            semantic_enabled: semantic_default_enabled(),
            stats: Mutex::new(MultiTierStatsInner::default()),
        }
    }

    /// 默认构造：局部 n-gram 嵌入器，容量 1024，语义阈值 0.92（保守）。
    #[must_use]
    pub fn new() -> Self {
        Self::with_embedder(1024, Box::new(LocalNgramEmbedder::default()), 0.92)
    }

    /// 共享 `exact` 层构造：Tier 0 复用调用方已有的 `Arc<PrefixCache>`（如网关进程级缓存），
    /// 使本缓存与既有精确缓存**同源同对象**，Tier 0 命中由共享方计量、绝不重复计数。
    /// `normalized`/`semantic` 层仍独立持有。
    ///
    /// Tier 2 默认关闭；生产安装点（[`crate::cache::install_semantic_layer`]）按
    /// [`tier2_opted_in_from_env`] 显式选入。
    #[must_use]
    pub fn with_shared_exact(
        exact: Arc<PrefixCache>,
        capacity: usize,
        embedder: Box<dyn Embedder>,
        semantic_threshold: f64,
    ) -> Self {
        Self {
            exact,
            exact_shared: true,
            normalized: PrefixCache::new(capacity),
            semantic: SemanticIndex::new(),
            embedder,
            semantic_threshold: semantic_threshold.clamp(0.0, 1.0),
            semantic_enabled: false,
            stats: Mutex::new(MultiTierStatsInner::default()),
        }
    }

    /// 显式开启 Tier 2 语义近似（默认关闭：近似命中可能有假阳性，需调用方确认风险）。
    /// 仅 [`TierPolicy::Full`] 会真正走到语义层；其余策略即便开启也不查询语义层。
    #[must_use]
    pub fn with_semantic_enabled(mut self, enabled: bool) -> Self {
        self.semantic_enabled = enabled;
        self
    }

    /// 查询：按 `policy` 逐层短路。每次调用**仅计一次 lookup**（诚实，不重复计数）。
    ///
    /// - `ExactOnly`：仅 Tier 0（携带工具定义的路径，零假阳性）。
    /// - `NormalizedOnly`：Tier 0 + Tier 1（普通文本默认）。
    /// - `Full`：再叠加 Tier 2 语义近似（需 `with_semantic_enabled(true)` 且未因策略降级）。
    #[must_use]
    pub fn get_with_policy(&self, prompt: &str, policy: TierPolicy) -> Option<CachedResponse> {
        // 工具路径（ExactOnly）真实计数：每次查询/命中/未命中都如实记一次
        let tool_path = policy == TierPolicy::ExactOnly;
        // Tier 0：原始 prompt 精确
        if let Some(e) = self.exact.get(hash_text(prompt)) {
            self.record(HitTier::Exact, e.prompt_tokens, tool_path);
            return Some(CachedResponse {
                content: e.content,
                completion_tokens: e.completion_tokens,
                tier: HitTier::Exact,
            });
        }
        if policy == TierPolicy::ExactOnly {
            self.record_miss(tool_path);
            return None;
        }
        // Tier 1：规范化后精确（易变后缀已剥离）
        let norm = normalize_prompt(prompt);
        if let Some(e) = self.normalized.get(hash_text(&norm)) {
            self.record(HitTier::Normalized, e.prompt_tokens, false);
            return Some(CachedResponse {
                content: e.content,
                completion_tokens: e.completion_tokens,
                tier: HitTier::Normalized,
            });
        }
        // Tier 2：语义近似（仅 `Full` 策略且显式开启时才触发；默认关闭）
        if policy == TierPolicy::Full && self.semantic_enabled {
            let emb = self.embedder.embed(prompt);
            if let Some((content, sim, ct)) = self.semantic.search(&emb, self.semantic_threshold) {
                self.record(HitTier::Semantic(sim), 0, false);
                return Some(CachedResponse {
                    content,
                    completion_tokens: ct,
                    tier: HitTier::Semantic(sim),
                });
            }
        }
        // 全部未命中
        self.record_miss(false);
        None
    }

    /// 便捷兼容：等价于 `get_with_policy(Full)`（原模块契约：精确 → 规范化 → 语义逐层短路）。
    #[must_use]
    pub fn get(&self, prompt: &str) -> Option<CachedResponse> {
        self.get_with_policy(prompt, TierPolicy::Full)
    }

    /// 写入三层：精确 + 规范化精确 + 语义索引（均以 prompt 为键）。
    /// 语义层在 `semantic_enabled == false` 时跳过插入（默认关闭，省内存且无近似污染）。
    pub fn put(&self, prompt: &str, content: &str, prompt_tokens: u32, completion_tokens: u32) {
        let entry = CacheEntry {
            content: content.to_string(),
            prompt_tokens,
            completion_tokens,
            last_used_ms: super::now_ms(),
        };
        self.exact.put(hash_text(prompt), entry.clone());
        let norm = normalize_prompt(prompt);
        self.normalized.put(hash_text(&norm), entry);
        if self.semantic_enabled {
            let emb = self.embedder.embed(prompt);
            self.semantic
                .insert(prompt, emb, content.to_string(), completion_tokens);
        }
    }

    /// 携带工具定义的写入：仅写精确 + 规范化层（语义近似对工具请求无意义且有害），避免
    /// 近似命中静默丢弃工具调用。与 `get_with_policy(ExactOnly)` 对称。
    pub fn put_exact(
        &self,
        prompt: &str,
        content: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
    ) {
        let entry = CacheEntry {
            content: content.to_string(),
            prompt_tokens,
            completion_tokens,
            last_used_ms: super::now_ms(),
        };
        self.exact.put(hash_text(prompt), entry.clone());
        let norm = normalize_prompt(prompt);
        self.normalized.put(hash_text(&norm), entry);
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

    /// 暴露 Tier 0 共享精确缓存的 `Arc`，供 `model::llm` 用 `Arc::ptr_eq` 判断是否同源，
    /// 避免对两个不同的 `PrefixCache` 做重复计数或误短路。
    #[must_use]
    pub fn exact_arc(&self) -> &Arc<PrefixCache> {
        &self.exact
    }

    /// Tier 2 语义近似是否开启（默认 false：近似命中可能有假阳性，需显式开启）。
    #[must_use]
    pub fn semantic_enabled(&self) -> bool {
        self.semantic_enabled
    }

    /// 当前嵌入器后端名（可观测：`cache.status` 据此如实说明语义层用的哪个嵌入后端）
    #[must_use]
    pub fn embedder_backend_name(&self) -> &'static str {
        self.embedder.backend_name()
    }

    /// 当前嵌入器维度（换后端后维度会变，跨维度相似度无意义——故如实暴露）
    #[must_use]
    pub fn embedder_dim(&self) -> usize {
        self.embedder.dim()
    }

    fn record(&self, tier: HitTier, saved: u32, tool_path: bool) {
        let mut g = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        g.stats.lookups += 1;
        g.stats.saved_prompt_tokens += saved as u64;
        if tool_path {
            g.stats.tool_lookups += 1;
            g.stats.tool_hits += 1;
        }
        let saved_tokens = saved as u64;
        match tier {
            HitTier::Exact => {
                g.stats.exact_hits += 1;
                g.stats.exact_saved_tokens += saved_tokens;
            }
            HitTier::Normalized => {
                g.stats.normalized_hits += 1;
                g.stats.normalized_saved_tokens += saved_tokens;
            }
            HitTier::Semantic(_) => {
                g.stats.semantic_hits += 1;
                g.stats.semantic_saved_tokens += saved_tokens;
            }
        }
    }

    fn record_miss(&self, tool_path: bool) {
        let mut g = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        g.stats.lookups += 1;
        g.stats.misses += 1;
        if tool_path {
            g.stats.tool_lookups += 1;
            g.stats.tool_misses += 1;
        }
    }

    /// 清空三层缓存与统计（仅供测试隔离使用）。
    /// Tier 0 `exact` 与 `gateway_prefix_cache()` 共享同一 Arc，`clear()` 一并清之。
    #[cfg(test)]
    pub fn clear(&self) {
        self.exact.clear();
        self.normalized.clear();
        self.semantic
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        let mut g = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        g.stats = MultiTierStats::default();
    }

    /// 落盘：精确 + 规范化 + 语义（语义存原始 prompt，加载时由 embedder 重嵌入，保证与嵌入器一致）。
    /// `exact_shared == true` 时不落盘 `exact` 层（由共享方负责落盘，避免重复 IO 与双写漂移）。
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let exact_snapshot = if self.exact_shared {
            PersistedCache::empty()
        } else {
            self.exact.snapshot()
        };
        let payload = PersistedMultiTier {
            exact: exact_snapshot,
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
        cache.restore_from(path);
        cache
    }

    /// 共享 `exact` 层加载：除 `exact` 复用调用方传入的 `Arc<PrefixCache>` 外，
    /// 其余层从磁盘恢复（规范化 + 语义）。`exact` 层由共享方自身落盘恢复。
    #[must_use]
    pub fn open_with_shared_exact(
        path: &Path,
        exact: Arc<PrefixCache>,
        embedder: Box<dyn Embedder>,
        semantic_threshold: f64,
    ) -> Self {
        let mut cache = Self::with_shared_exact(exact, 256, embedder, semantic_threshold);
        cache.restore_from(path);
        cache
    }

    /// 从磁盘恢复 `normalized` + `semantic` 层（`exact` 始终由构造方决定，不在此覆盖）。
    fn restore_from(&mut self, path: &Path) {
        if let Ok(data) = std::fs::read(path) {
            if let Ok(p) = serde_json::from_slice::<PersistedMultiTier>(&data) {
                if !self.exact_shared {
                    let cap = p.exact.map.len().max(p.normalized.map.len()).max(256);
                    self.exact = Arc::new(PrefixCache::from_snapshot(p.exact, cap));
                }
                let cap = p.normalized.map.len().max(256);
                self.normalized = PrefixCache::from_snapshot(p.normalized, cap);
                for rec in &p.semantic {
                    let emb = self.embedder.embed(&rec.prompt);
                    self.semantic.insert(
                        &rec.prompt,
                        emb,
                        rec.content.clone(),
                        rec.completion_tokens,
                    );
                }
            }
        }
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
        assert!(
            idx.search(&paraphrase, 0.5).is_some(),
            "paraphrase 应超过低阈值"
        );
        assert!(idx.search(&bread, 0.5).is_none(), "无关文本不应命中");
        // 高阈值：paraphrase 也不命中（防止假阳性）
        assert!(
            idx.search(&paraphrase, 0.999).is_none(),
            "过高阈值应拒绝近似命中"
        );
    }

    #[test]
    fn multi_tier_exact_normalized_semantic_in_order() {
        let cache = MultiTierCache::with_embedder(64, Box::new(LocalNgramEmbedder::default()), 0.5);
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
        assert_eq!(s.saved_prompt_tokens, (5 + 10)); // 语义命中不计入 prompt 节省
    }

    // ---- Task #5 接驳守卫 ----

    /// `ExactOnly` 策略即便存在 Tier1 规范化条目也**不**短路，避免工具路径近似命中丢调用。
    #[test]
    fn exact_only_policy_never_uses_normalized_tier() {
        let cache =
            MultiTierCache::with_embedder(64, Box::new(LocalNgramEmbedder::default()), 0.92);
        // 仅易变 UUID 后缀不同（UUID 会被 normalize 剥离）→ 规范化后同键
        cache.put(
            "Q: run step 3 id=123e4567-e89b-12d3-a456-426614174000",
            "ok",
            4,
            1,
        );
        // 同规范化键、不同原始键 → NormalizedOnly 应命中
        assert!(cache
            .get_with_policy(
                "Q: run step 3 id=999e9999-e89b-12d3-a456-426614174000",
                TierPolicy::NormalizedOnly
            )
            .is_some());
        // ExactOnly 必须 miss（工具路径安全性）
        assert!(cache
            .get_with_policy(
                "Q: run step 3 id=999e9999-e89b-12d3-a456-426614174000",
                TierPolicy::ExactOnly
            )
            .is_none());
    }

    /// 工具路径真实计数：`ExactOnly` 策略每次查询/命中/未命中都如实计入，
    /// 且非 `ExactOnly` 路径绝不污染该计数。
    #[test]
    fn exact_only_policy_records_tool_path_counters() {
        let cache =
            MultiTierCache::with_embedder(64, Box::new(LocalNgramEmbedder::default()), 0.92);
        cache.put("tool-key", "resp", 10, 2);
        // 命中一次
        assert!(cache.get_with_policy("tool-key", TierPolicy::ExactOnly).is_some());
        // 未命中一次
        assert!(cache
            .get_with_policy("missing-key", TierPolicy::ExactOnly)
            .is_none());
        let s = cache.stats();
        assert_eq!(s.tool_lookups, 2);
        assert_eq!(s.tool_hits, 1);
        assert_eq!(s.tool_misses, 1);
        assert_eq!(s.tool_hits + s.tool_misses, s.tool_lookups);
        // 普通路径（NormalizedOnly）不得污染工具路径计数
        let _ = cache.get_with_policy("tool-key", TierPolicy::NormalizedOnly);
        assert_eq!(cache.stats().tool_lookups, 2, "普通路径不得计入工具路径");
    }

    /// 共享 exact 构造（网关路径）默认关闭语义近似：即便 `Full` 策略也不触发 Tier2，
    /// 网关工具路径因此绝不会因近似命中而丢调用（安全默认）。
    #[test]
    fn shared_exact_semantic_off_by_default_under_full_policy() {
        let shared = Arc::new(PrefixCache::new(64));
        let cache = MultiTierCache::with_shared_exact(
            shared,
            64,
            Box::new(LocalNgramEmbedder::default()),
            0.5,
        );
        cache.put("What is the capital of France?", "Paris", 12, 1);
        assert!(!cache.semantic_enabled(), "网关共享实例语义层应默认关闭");
        assert!(cache
            .get_with_policy("Tell me the capital city of France.", TierPolicy::Full)
            .is_none());
        // 显式开启的新实例（语义层开启）后才命中——锁定 enabled，不依赖 env 默认值
        let on = MultiTierCache::with_embedder(64, Box::new(LocalNgramEmbedder::default()), 0.5)
            .with_semantic_enabled(true);
        on.put("What is the capital of France?", "Paris", 12, 1);
        assert!(on
            .get_with_policy("Tell me the capital city of France.", TierPolicy::Full)
            .is_some());
    }

    /// 共享 `exact`：多层缓存与独立 PrefixCache 同源同对象（Arc::ptr_eq 为真），
    /// 保证 LLM 层接驳时不重复计数。
    #[test]
    fn shared_exact_uses_same_arc_as_source() {
        let shared = Arc::new(PrefixCache::new(64));
        let layer = MultiTierCache::with_shared_exact(
            shared.clone(),
            64,
            Box::new(LocalNgramEmbedder::default()),
            0.92,
        );
        assert!(
            Arc::ptr_eq(layer.exact_arc(), &shared),
            "共享 exact 应为同一 Arc"
        );
        // 经共享 exact 写入能被共享方直接命中（同源）
        let key = crate::cache::hash_text("shared key");
        shared.put(
            key,
            CacheEntry {
                content: "v".into(),
                prompt_tokens: 0,
                completion_tokens: 1,
                last_used_ms: 0,
            },
        );
        assert!(layer
            .get_with_policy("shared key", TierPolicy::NormalizedOnly)
            .is_some());
    }

    /// 命中率诚实：仅在真实命中时上升，且永不冒充 100%（返回比值 0–1）。
    #[test]
    fn hit_rate_is_honest_and_bounded() {
        let cache =
            MultiTierCache::with_embedder(64, Box::new(LocalNgramEmbedder::default()), 0.92);
        assert_eq!(cache.hit_rate(), 0.0, "无查询时命中率为 0");
        cache.put("a", "1", 0, 1);
        let _ = cache.get("a"); // 命中
        let _ = cache.get("b"); // 未命中
        let rate = cache.hit_rate();
        assert!((rate - 0.5).abs() < 1e-9, "1 命中 / 2 查询 = 0.5");
        // 全未命中场景
        let miss = MultiTierCache::with_embedder(64, Box::new(LocalNgramEmbedder::default()), 0.92);
        let _ = miss.get("x");
        assert_eq!(miss.hit_rate(), 0.0);
    }

    #[test]
    fn multi_tier_persistence_roundtrip() {
        let path = std::env::temp_dir().join(format!("marg_multitier_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let cache =
                MultiTierCache::with_embedder(64, Box::new(LocalNgramEmbedder::default()), 0.5);
            cache.put("hello world", "Hi there", 5, 1);
            cache.put("What is the capital of France?", "Paris", 12, 1);
            cache.save(&path).expect("落盘应成功");
        }
        // 重新加载（模拟重启），embedder 重新嵌入语义条目
        let cache = MultiTierCache::open(&path, Box::new(LocalNgramEmbedder::default()), 0.5);
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
            // 分层省 token 与各层命中同源；此辅助函数只关心总量口径，分层按层数摊平
            exact_saved_tokens: if exact > 0 { saved } else { 0 },
            // 其余字段（含工具路径计量）走默认值：对后续新增字段免疫，避免加字段即编译失败
            ..Default::default()
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
        let cache = MultiTierCache::with_embedder(64, Box::new(LocalNgramEmbedder::default()), 0.5);
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

    // 阶段 C：语义层默认开启策略由 env `MARGUERITE_SEMANTIC_CACHE` 驱动。
    // 直接测纯函数 `parse_semantic_env_policy`，避免并行测试中进程 env 竞态。
    #[test]
    fn parse_semantic_env_off_disables() {
        assert!(
            !parse_semantic_env_policy(Some("off")),
            "off 必须关闭语义层"
        );
        assert!(!parse_semantic_env_policy(Some("OFF")), "大小写不敏感");
        assert!(!parse_semantic_env_policy(Some(" off ")), "前后空白应 trim");
    }

    #[test]
    fn parse_semantic_env_on_and_auto_enable() {
        assert!(parse_semantic_env_policy(Some("on")), "on 必须开启");
        assert!(parse_semantic_env_policy(Some("auto")), "auto 应开启");
        assert!(parse_semantic_env_policy(Some("")), "空串按默认开启");
        assert!(
            parse_semantic_env_policy(Some("anything-else")),
            "未知值按默认开启"
        );
    }

    #[test]
    fn parse_semantic_env_unset_defaults_on() {
        assert!(
            parse_semantic_env_policy(None),
            "未设置时默认开启语义层（保持契约）"
        );
    }

    #[test]
    fn semantic_default_enabled_reads_env() {
        // `semantic_default_enabled` 包装了 env 读取；这里确认 None 路径与纯函数一致。
        // 进程 env 可能在并行测试中被改动，故仅断言「未显式 off 时返回 true」这一不变量。
        let live = std::env::var("MARGUERITE_SEMANTIC_CACHE").ok();
        if live.as_deref() != Some("off") {
            assert!(semantic_default_enabled(), "非 off 环境默认开启");
        }
    }

    // 阶段 C：语义开启时，近似 prompt（同义改写）在 Full 策略下命中，且精确路径
    // 不被污染（ExactOnly 仍只走 Tier 0）。用显式低阈值验证「命中路径」逻辑。
    #[test]
    fn semantic_boosts_synonym_hit_under_full_only() {
        let cache = MultiTierCache::with_embedder(64, Box::new(LocalNgramEmbedder::default()), 0.5)
            .with_semantic_enabled(true);
        cache.put("What is the capital of France?", "Paris", 12, 1);

        // 语义近似命中（同义改写，相似度 ≥ 0.5）→ Full 策略命中
        let hit = cache.get_with_policy("Tell me the capital city of France.", TierPolicy::Full);
        assert!(hit.is_some(), "语义开启下近似 prompt 应命中");
        assert_eq!(hit.unwrap().content, "Paris");

        // 精确路径独立：ExactOnly 不查语义层，近似 prompt 不应命中（无假阳性）
        let exact_only =
            cache.get_with_policy("Tell me the capital city of France.", TierPolicy::ExactOnly);
        assert!(exact_only.is_none(), "ExactOnly 不应被语义近似污染");
    }

    // ---- R46：语义缓存接真实 embedding（文档 1 缺口）----

    /// 假后端：只有「包含 alpha」这一语义维度——任何文本级哈希嵌入都做不出这种几何。
    /// 用它可判定「缓存的相似度到底来自谁」。
    struct AlphaOnlyBackend;

    impl crate::memory::embedding::Embedding for AlphaOnlyBackend {
        fn embed(&self, text: &str) -> Vec<f64> {
            if text.contains("alpha") {
                vec![1.0, 0.0]
            } else {
                vec![0.0, 1.0]
            }
        }
        fn dim(&self) -> usize {
            2
        }
        fn name(&self) -> &'static str {
            "fake-alpha-2d"
        }
    }

    /// `ProviderEmbedder` 必须**转发到注入的后端**（而不是偷偷回落到本地 n-gram 哈希）
    #[test]
    fn provider_embedder_delegates_to_injected_backend() {
        let emb = ProviderEmbedder::from_backend(Arc::new(AlphaOnlyBackend));
        assert_eq!(emb.dim(), 2, "维度必须来自注入后端");
        assert_eq!(emb.backend_name(), "fake-alpha-2d", "后端名必须来自注入后端");
        assert_eq!(emb.embed("alpha channel").0, vec![1.0, 0.0]);
        assert_eq!(emb.embed("beta gamma").0, vec![0.0, 1.0]);
    }

    /// 决定性证据：Tier 2 语义命中必须由**注入后端**的几何决定。
    ///
    /// 存入与查询的**字面完全不同的两句话**：本地 n-gram 哈希嵌入绝无可能把它们判为相似，
    /// 而假后端（按语义维度）能让它们余弦为 1 → 命中即证明缓存真的用了注入嵌入。
    #[test]
    fn semantic_tier_uses_injected_backend_geometry() {
        let cache = MultiTierCache::with_embedder(
            16,
            Box::new(ProviderEmbedder::from_backend(Arc::new(AlphaOnlyBackend))),
            0.9,
        )
        .with_semantic_enabled(true);
        assert_eq!(cache.embedder_dim(), 2);
        assert_eq!(cache.embedder_backend_name(), "fake-alpha-2d");

        cache.put("the alpha channel holds transparency", "OK", 9, 1);
        // 字面毫无重合、仅共享语义维度 → 只有真实嵌入类后端能命中
        let hit = cache
            .get_with_policy("alpha transparency", TierPolicy::Full)
            .expect("注入后端的语义维度应命中");
        assert_eq!(hit.content, "OK");
        assert!(matches!(hit.tier, HitTier::Semantic(_)));

        // 反证：本地 n-gram 嵌入器在同样语料上不该命中（否则本用例不能证明「注入生效」）
        let local = MultiTierCache::with_embedder(16, Box::new(LocalNgramEmbedder::default()), 0.9)
            .with_semantic_enabled(true);
        local.put("the alpha channel holds transparency", "OK", 9, 1);
        assert!(
            local
                .get_with_policy("alpha transparency", TierPolicy::Full)
                .is_none(),
            "本地哈希嵌入不应把这两句判为相似；若命中则本用例失去判别力"
        );
    }

    /// 默认构造的 `ProviderEmbedder` 取进程级后端：维度必须与之一致（不切后端 → 并行测试安全）
    #[test]
    fn provider_embedder_default_follows_process_backend() {
        let e = ProviderEmbedder::new();
        assert_eq!(e.dim(), crate::memory::embedding::EmbeddingProvider::global().dim());
        // 默认装配下就是本地特征哈希，后端名如实上报（绝不谎称「真实模型」）
        assert_eq!(e.backend_name(), "local-hash");
    }

    /// Tier 2 选入开关：只有显式 `on`/`1`/`true` 才算开（生产路径的默认方向是**关**）
    #[test]
    fn tier2_opt_in_requires_explicit_on() {
        for raw in ["on", "ON", " on ", "1", "true", "True"] {
            assert!(tier2_opted_in(Some(raw)), "`{raw}` 应视为显式选入");
        }
        for raw in [None, Some(""), Some("off"), Some("OFF"), Some("auto"), Some("0"), Some("yes")] {
            assert!(
                !tier2_opted_in(raw),
                "{raw:?} 不应开启 Tier 2（近似命中默认必须关闭，避免假阳性）"
            );
        }
    }

    /// 策略只定义一次：未选入 → NormalizedOnly（零近似假阳性）；选入 → Full（语义层参与）
    #[test]
    fn text_path_policy_matches_opt_in() {
        assert_eq!(text_path_policy(false), TierPolicy::NormalizedOnly);
        assert_eq!(text_path_policy(true), TierPolicy::Full);
    }
}

#[allow(clippy::items_after_test_module)] // Default 实现紧随 MultiTierCache 定义，位于测试模块之后（组织性 lint）
impl Default for MultiTierCache {
    fn default() -> Self {
        Self::new()
    }
}
