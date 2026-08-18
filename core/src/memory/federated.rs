//! 6 层联邦记忆编排层（GUI_Agent Block 3）
//!
//! 在既有分层记忆（`sensory` / `short_term` / `long_term` / `experience` / `cross_project` / `compact`）
//! 之上提供**联邦编排**：把 6 个记忆层统一为一个可召回、可追溯、可晋升的有机整体。
//!
//! ## 6 层定义（按召回优先级，数字越大越先被查询）
//! | 层 | 含义 | 后端 |
//! |----|------|------|
//! | `Experience`  (60) | 经验台账（踩坑/决策/解法，最高信号） | `ExperienceBackend`（内含 `SelfEvolution`，一并产出 LongTerm 命中） |
//! | `CrossProject`(55) | 跨项目联邦（扫描本机各 AI 工具记忆，渐进披露） | `CrossProjectBackend` |
//! | `LongTerm`    (50) | 长期语义向量（特征哈希，零依赖） | 由 `ExperienceBackend` 在 recall 中一并返回 |
//! | `ShortTerm`   (40) | 短期 FTS5 跨会话全文 | `ShortTermBackend` |
//! | `Sensory`     (30) | 感知/工作记忆（当前会话，毫秒级） | `SensoryBackend` |
//! | `Compact`     (0)  | 压缩/稳定前缀（输出层，渲染为 System 消息供前缀缓存） | `to_stable_prefix` |
//!
//! ## 设计原则（对齐 `cache`/`semantic` 三原则）
//! - **精确优先**：召回按分数降序、同分按层优先级，确定性可复现。
//! - **去重保守**：相同内容只保留最高分命中，避免跨层重复注入上下文。
//! - **可观测**：每次召回产出 `FederatedStats`（每层命中数），便于调参与前缀缓存命中率分析。
//! - **解耦可测**：核心逻辑只依赖 `LayerBackend` trait，单测用 `FakeBackend` 注入，无需 SQLite/文件系统。

use std::collections::HashMap;

use crate::memory::compact::{as_stable_prefix, Observation};
use crate::memory::cross_project;
use crate::memory::experience::{Experience, SelfEvolution};
use crate::memory::long_term::MemoryItem;
use crate::memory::short_term::{SessionSummary, ShortTermMemory};
use crate::memory::sensory::SensoryMemory;
use crate::types::Message;

/// 联邦记忆层标识
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryTier {
    /// 感知/工作记忆（当前会话，毫秒级）
    Sensory,
    /// 短期记忆（FTS5 跨会话全文）
    ShortTerm,
    /// 跨项目联邦（扫描本机各 AI 工具记忆）
    CrossProject,
    /// 长期语义向量
    LongTerm,
    /// 经验台账（踩坑/决策/解法）
    Experience,
    /// 压缩/稳定前缀（输出层，渲染为 System 消息）
    Compact,
}

impl MemoryTier {
    /// 层级名（写入 Observation.kind / 可观测日志）
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryTier::Sensory => "sensory",
            MemoryTier::ShortTerm => "short_term",
            MemoryTier::CrossProject => "cross_project",
            MemoryTier::LongTerm => "long_term",
            MemoryTier::Experience => "experience",
            MemoryTier::Compact => "compact",
        }
    }

    /// 人类可读标签
    pub fn label(&self) -> &'static str {
        match self {
            MemoryTier::Sensory => "感知/工作记忆",
            MemoryTier::ShortTerm => "短期记忆(FTS)",
            MemoryTier::CrossProject => "跨项目联邦",
            MemoryTier::LongTerm => "长期语义记忆",
            MemoryTier::Experience => "经验台账",
            MemoryTier::Compact => "压缩/稳定前缀",
        }
    }

    /// 召回优先级（越大越先被查询）
    pub fn priority(&self) -> u8 {
        match self {
            MemoryTier::Experience => 60,
            MemoryTier::CrossProject => 55,
            MemoryTier::LongTerm => 50,
            MemoryTier::ShortTerm => 40,
            MemoryTier::Sensory => 30,
            MemoryTier::Compact => 0,
        }
    }
}

/// 一条归一化的联邦记忆命中（跨层统一表示）
#[derive(Debug, Clone)]
pub struct FederatedHit {
    /// 来源层
    pub tier: MemoryTier,
    /// 记忆正文
    pub content: String,
    /// 来源标识（session_id / project / 路径等）
    pub source: String,
    /// 相似度/相关度分数（0..1）
    pub score: f64,
    /// 时间戳（RFC3339 字符串）
    pub timestamp: String,
    /// 附加元数据（kind / validated / rank / tags 等）
    pub meta: HashMap<String, String>,
}

/// 单条记忆后端抽象（可注入假后端做纯单测）
pub trait LayerBackend: Send {
    /// 该后端代表的层
    fn tier(&self) -> MemoryTier;
    /// 召回与查询相关的记忆（已按各层自身逻辑过滤/打分）
    fn recall(&self, query: &str, limit: usize) -> Vec<FederatedHit>;
    /// 分层晋升（把上游感知记忆沉淀到本层）。默认无操作，返回沉淀条数。
    fn consolidate(&mut self, _sensory: &SensoryMemory) -> usize {
        0
    }
}

/// 召回模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecallMode {
    /// 扇出：查询所有层，合并去重后按分数排序
    FanOut,
    /// 升级：按层优先级逐个查询，填满配额即停止（高层命中不足才降级到低层）
    Escalate,
}

/// 单次召回的每层命中统计（可观测）
#[derive(Debug, Clone, Default)]
pub struct FederatedStats {
    /// 每层命中条数
    pub per_tier: HashMap<MemoryTier, usize>,
}

/// 单次召回结果
#[derive(Debug, Clone)]
pub struct FederatedResult {
    /// 去重排序后的命中（已截断到 limit）
    pub hits: Vec<FederatedHit>,
    /// 每层命中统计
    pub tier_stats: FederatedStats,
    /// 是否降级（升级模式下高层无命中、低层补位）
    pub degraded: bool,
}

/// 分层晋升报告
#[derive(Debug, Clone, Default)]
pub struct ConsolidationReport {
    /// 各层沉淀总条数
    pub total: usize,
}

/// 6 层联邦记忆编排器
pub struct FederatedMemory {
    backends: Vec<Box<dyn LayerBackend + Send>>,
}

impl Default for FederatedMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl FederatedMemory {
    /// 创建空的联邦记忆
    pub fn new() -> Self {
        Self {
            backends: Vec::new(),
        }
    }

    /// 注册一个记忆后端
    pub fn register(&mut self, backend: Box<dyn LayerBackend + Send>) {
        self.backends.push(backend);
    }

    /// 用既有真实分层构造联邦记忆（覆盖 6 层中的 5 个召回源 + Compact 输出层）。
    ///
    /// 说明：LongTerm 召回由 `ExperienceBackend`（内含 `SelfEvolution`）在 `recall` 中一并返回，
    /// 避免与 Experience 后端重复持有 `LongTermMemory` 所有权。
    pub fn with_real_layers(
        sensory: SensoryMemory,
        short_term: ShortTermMemory,
        self_evolution: SelfEvolution,
    ) -> Self {
        let mut fm = FederatedMemory::new();
        fm.register(Box::new(SensoryBackend::new(sensory)));
        fm.register(Box::new(ShortTermBackend::new(short_term)));
        fm.register(Box::new(ExperienceBackend::new(self_evolution)));
        fm.register(Box::new(CrossProjectBackend));
        fm
    }

    /// 联邦召回：跨层合并，去重后按分数降序（同分按层优先级）。
    pub fn recall(&self, query: &str, limit: usize, mode: RecallMode) -> FederatedResult {
        let mut collected: Vec<FederatedHit> = Vec::new();
        let mut per_tier: HashMap<MemoryTier, usize> = HashMap::new();

        // 按层优先级降序排列后端
        let mut order: Vec<usize> = (0..self.backends.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(self.backends[i].tier().priority()));
        let top_tier = order.first().map(|&i| self.backends[i].tier());

        'tiers: for &i in &order {
            let tier = self.backends[i].tier();
            let hits = self.backends[i].recall(query, limit);
            *per_tier.entry(tier).or_insert(0) += hits.len();

            match mode {
                RecallMode::FanOut => collected.extend(hits),
                RecallMode::Escalate => {
                    for h in hits {
                        if collected.len() < limit {
                            collected.push(h);
                        }
                        if collected.len() >= limit {
                            // 配额已满，停止向低层升级
                            break 'tiers;
                        }
                    }
                }
            }
        }

        // 降级判定（仅升级模式）：最高优先级层无命中，但低层补位成功
        let mut degraded = false;
        if let (RecallMode::Escalate, Some(top)) = (mode, top_tier) {
            let top_count = per_tier.get(&top).copied().unwrap_or(0);
            degraded = top_count == 0 && !collected.is_empty();
        }

        // 去重：相同内容只保留最高分
        let mut best: HashMap<String, FederatedHit> = HashMap::new();
        for h in collected {
            match best.entry(h.content.clone()) {
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    if h.score > e.get().score {
                        e.insert(h);
                    }
                }
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(h);
                }
            }
        }

        let mut hits: Vec<FederatedHit> = best.into_values().collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.tier.priority().cmp(&a.tier.priority()))
                .then_with(|| a.content.cmp(&b.content))
        });
        hits.truncate(limit);

        FederatedResult {
            hits,
            tier_stats: FederatedStats { per_tier },
            degraded,
        }
    }

    /// 分层晋升：按层优先级顺序把感知记忆沉淀到各层，累计条数。
    pub fn consolidate(&mut self, sensory: &SensoryMemory) -> ConsolidationReport {
        let mut order: Vec<usize> = (0..self.backends.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(self.backends[i].tier().priority()));
        let mut total = 0usize;
        for &i in &order {
            total += self.backends[i].consolidate(sensory);
        }
        ConsolidationReport { total }
    }

    /// 联邦召回并渲染为**稳定前缀** `Message::System`（Compact 输出层）。
    ///
    /// 把跨层命中点积成 `Observation` 后交给 `compact::as_stable_prefix` 渲染为固定格式，
    /// 使相同召回结果逐字节稳定，可被 DeepSeek 前缀缓存 / `PrefixCache` 复用
    /// —— 直接服务「deepseek-v4-flash/pro 缓存命中率做到极致」目标。
    /// 无命中时返回 `None`，绝不阻塞对话。
    pub fn to_stable_prefix(&self, query: &str, limit: usize) -> Option<Message> {
        let res = self.recall(query, limit, RecallMode::FanOut);
        if res.hits.is_empty() {
            return None;
        }
        let observations: Vec<Observation> = res
            .hits
            .iter()
            .map(|h| Observation {
                kind: h.tier.as_str().to_string(),
                title: h.source.clone(),
                facts: vec![h.content.clone()],
                concepts: Vec::new(),
                discovery_tokens: 0,
                created_at_ms: 0,
            })
            .collect();
        as_stable_prefix(&observations)
    }
}

/// 把 `Message` 渲染为纯文本（供命中内容提取）
fn message_text(msg: &Message) -> String {
    match msg {
        Message::User { content, .. } => content.clone(),
        Message::Assistant { content, .. } => content.clone(),
        Message::System { content, .. } => content.clone(),
        Message::Tool { output, .. } => output.clone(),
    }
}

// ───────────────────────────── 真实后端适配 ─────────────────────────────

/// 感知/工作记忆后端
pub struct SensoryBackend {
    inner: SensoryMemory,
    session_id: String,
}

impl SensoryBackend {
    pub fn new(inner: SensoryMemory) -> Self {
        let session_id = inner.session_id().to_string();
        Self { inner, session_id }
    }
}

impl LayerBackend for SensoryBackend {
    fn tier(&self) -> MemoryTier {
        MemoryTier::Sensory
    }

    fn recall(&self, query: &str, limit: usize) -> Vec<FederatedHit> {
        match self.inner.search(query) {
            Ok(msgs) => msgs
                .iter()
                .take(limit)
                .map(|m| FederatedHit {
                    tier: MemoryTier::Sensory,
                    content: message_text(m),
                    source: self.session_id.clone(),
                    score: 1.0,
                    timestamp: m.timestamp().to_rfc3339(),
                    meta: HashMap::new(),
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }
    // 感知记忆是晋升的源头，自身不消费上游
}

/// 短期 FTS 记忆后端
pub struct ShortTermBackend {
    inner: ShortTermMemory,
}

impl ShortTermBackend {
    pub fn new(inner: ShortTermMemory) -> Self {
        Self { inner }
    }
}

impl LayerBackend for ShortTermBackend {
    fn tier(&self) -> MemoryTier {
        MemoryTier::ShortTerm
    }

    fn recall(&self, query: &str, limit: usize) -> Vec<FederatedHit> {
        match self.inner.search(query, limit) {
            Ok(summaries) => summaries.into_iter().map(summary_to_hit).collect(),
            Err(_) => Vec::new(),
        }
    }
    // 短期记忆在会话结束时由 store_session 显式写入，不从感知晋升
}

fn summary_to_hit(s: SessionSummary) -> FederatedHit {
    let mut meta = HashMap::new();
    meta.insert("kind".to_string(), "session".to_string());
    if let Some(rank) = s.rank {
        meta.insert("rank".to_string(), rank.to_string());
    }
    FederatedHit {
        tier: MemoryTier::ShortTerm,
        content: s.summary,
        source: s.session_id,
        score: 0.4,
        timestamp: s.updated_at,
        meta,
    }
}

/// 经验台账后端（内含 `SelfEvolution`，其 `recall` 一并产出 LongTerm 语义命中）
pub struct ExperienceBackend {
    inner: SelfEvolution,
}

impl ExperienceBackend {
    pub fn new(inner: SelfEvolution) -> Self {
        Self { inner }
    }
}

impl LayerBackend for ExperienceBackend {
    fn tier(&self) -> MemoryTier {
        MemoryTier::Experience
    }

    fn recall(&self, query: &str, limit: usize) -> Vec<FederatedHit> {
        let mut out = Vec::new();
        // 长期语义向量命中（LongTerm 层）
        if let Ok(items) = self.inner.long_term.search(query, limit) {
            out.extend(items.into_iter().map(memory_item_to_hit));
        }
        // 经验台账命中（Experience 层）
        if let Ok(exp) = self.inner.recall(query, limit) {
            out.extend(exp.into_iter().map(experience_to_hit));
        }
        out
    }

    fn consolidate(&mut self, sensory: &SensoryMemory) -> usize {
        // SelfEvolution 内部同时完成 长期记忆 + 经验台账 双重固化
        self.inner.consolidate_session(sensory).unwrap_or(0)
    }
}

fn memory_item_to_hit(it: MemoryItem) -> FederatedHit {
    let mut meta = HashMap::new();
    if !it.meta.is_empty() {
        meta.insert("meta".to_string(), it.meta);
    }
    FederatedHit {
        tier: MemoryTier::LongTerm,
        content: it.content,
        source: "long_term".to_string(),
        score: it.score,
        timestamp: it.timestamp,
        meta,
    }
}

fn experience_to_hit(e: Experience) -> FederatedHit {
    let mut meta = HashMap::new();
    meta.insert("kind".to_string(), e.kind.as_str().to_string());
    meta.insert("validated".to_string(), e.validated.to_string());
    if !e.tags.is_empty() {
        meta.insert("tags".to_string(), e.tags.join(","));
    }
    FederatedHit {
        tier: MemoryTier::Experience,
        content: format!("[{}] {}", e.title, e.body),
        source: "experience".to_string(),
        score: 0.6,
        timestamp: e.created_at,
        meta,
    }
}

/// 跨项目联邦后端（无状态，直接复用 `cross_project` 检索面）
pub struct CrossProjectBackend;

impl LayerBackend for CrossProjectBackend {
    fn tier(&self) -> MemoryTier {
        MemoryTier::CrossProject
    }

    fn recall(&self, query: &str, limit: usize) -> Vec<FederatedHit> {
        let v = cross_project::cross_project_search(&serde_json::json!({
            "query": query,
            "limit": limit,
        }));
        v["items"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|it| {
                        let project = it["project"].as_str()?;
                        let excerpt = it["excerpt"].as_str()?;
                        let score = it["score"].as_f64().unwrap_or(0.0);
                        let mut meta = HashMap::new();
                        meta.insert(
                            "file".to_string(),
                            it["file"].as_str().unwrap_or("").to_string(),
                        );
                        meta.insert(
                            "kind".to_string(),
                            it["kind"].as_str().unwrap_or("").to_string(),
                        );
                        Some(FederatedHit {
                            tier: MemoryTier::CrossProject,
                            content: excerpt.to_string(),
                            source: project.to_string(),
                            score,
                            timestamp: it["updated_at"].as_str().unwrap_or("").to_string(),
                            meta,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    // 跨项目记忆是只读联邦检索，不从感知晋升
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;
    use chrono::Utc;
    use std::sync::{Arc, Mutex};

    /// 可注入的假后端：返回预设命中，记录 consolidate 调用顺序（用 Arc<Mutex> 跨实例共享）
    struct FakeBackend {
        tier: MemoryTier,
        hits: Vec<FederatedHit>,
        consolidate_log: Arc<Mutex<Vec<MemoryTier>>>,
    }

    impl LayerBackend for FakeBackend {
        fn tier(&self) -> MemoryTier {
            self.tier
        }
        fn recall(&self, _query: &str, limit: usize) -> Vec<FederatedHit> {
            self.hits.iter().take(limit).cloned().collect()
        }
        fn consolidate(&mut self, _sensory: &SensoryMemory) -> usize {
            self.consolidate_log.lock().unwrap().push(self.tier);
            1
        }
    }

    fn hit(tier: MemoryTier, content: &str, score: f64) -> FederatedHit {
        FederatedHit {
            tier,
            content: content.to_string(),
            source: "src".to_string(),
            score,
            timestamp: String::new(),
            meta: HashMap::new(),
        }
    }

    #[test]
    fn tier_priority_ordering() {
        assert!(MemoryTier::Experience.priority() > MemoryTier::CrossProject.priority());
        assert!(MemoryTier::CrossProject.priority() > MemoryTier::LongTerm.priority());
        assert!(MemoryTier::LongTerm.priority() > MemoryTier::ShortTerm.priority());
        assert!(MemoryTier::ShortTerm.priority() > MemoryTier::Sensory.priority());
        assert!(MemoryTier::Sensory.priority() > MemoryTier::Compact.priority());
    }

    #[test]
    fn fanout_merges_dedups_and_sorts_by_score() {
        // 后端 A（高层）与 B（低层）返回重叠内容，去重保留最高分
        let a = FakeBackend {
            tier: MemoryTier::Experience,
            hits: vec![hit(MemoryTier::Experience, "X", 0.5), hit(MemoryTier::Experience, "Y", 0.9)],
            consolidate_log: Arc::new(Mutex::new(Vec::new())),
        };
        let b = FakeBackend {
            tier: MemoryTier::Sensory,
            hits: vec![
                hit(MemoryTier::Sensory, "Y", 0.3), // 与 A 的 Y 重复，分数更低 → 被覆盖
                hit(MemoryTier::Sensory, "Z", 0.7),
            ],
            consolidate_log: Arc::new(Mutex::new(Vec::new())),
        };
        let mut fm = FederatedMemory::new();
        fm.register(Box::new(a));
        fm.register(Box::new(b));

        let res = fm.recall("q", 10, RecallMode::FanOut);
        assert_eq!(res.hits.len(), 3, "X/Y/Z 三去重后应 3 条");
        // 分数降序：Y(0.9) > Z(0.7) > X(0.5)
        assert_eq!(res.hits[0].content, "Y");
        assert_eq!(res.hits[0].score, 0.9);
        assert_eq!(res.hits[1].content, "Z");
        assert_eq!(res.hits[2].content, "X");
        assert!(!res.degraded);
    }

    #[test]
    fn escalate_stops_at_quota_when_top_satisfied() {
        let a = FakeBackend {
            tier: MemoryTier::Experience,
            hits: vec![hit(MemoryTier::Experience, "A1", 1.0), hit(MemoryTier::Experience, "A2", 0.8)],
            consolidate_log: Arc::new(Mutex::new(Vec::new())),
        };
        let b = FakeBackend {
            tier: MemoryTier::Sensory,
            hits: vec![hit(MemoryTier::Sensory, "B1", 0.5)],
            consolidate_log: Arc::new(Mutex::new(Vec::new())),
        };
        let mut fm = FederatedMemory::new();
        fm.register(Box::new(a));
        fm.register(Box::new(b));

        let res = fm.recall("q", 1, RecallMode::Escalate);
        assert_eq!(res.hits.len(), 1, "配额 1 应只取 1 条");
        assert_eq!(res.hits[0].content, "A1");
        assert!(!res.degraded, "高层已满足配额，不应降级");
    }

    #[test]
    fn escalate_degrades_when_top_empty() {
        let a = FakeBackend {
            tier: MemoryTier::Experience,
            hits: vec![], // 高层无命中
            consolidate_log: Arc::new(Mutex::new(Vec::new())),
        };
        let b = FakeBackend {
            tier: MemoryTier::Sensory,
            hits: vec![hit(MemoryTier::Sensory, "B1", 0.5)],
            consolidate_log: Arc::new(Mutex::new(Vec::new())),
        };
        let mut fm = FederatedMemory::new();
        fm.register(Box::new(a));
        fm.register(Box::new(b));

        let res = fm.recall("q", 5, RecallMode::Escalate);
        assert_eq!(res.hits.len(), 1);
        assert_eq!(res.hits[0].content, "B1");
        assert!(res.degraded, "高层空、低层补位应标记降级");
    }

    #[test]
    fn consolidate_calls_backends_in_tier_priority_order() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let sensory = SensoryMemory::new("sess");
        let mut fm = FederatedMemory::new();
        // 故意乱序注册
        fm.register(Box::new(FakeBackend {
            tier: MemoryTier::Sensory,
            hits: vec![],
            consolidate_log: log.clone(),
        }));
        fm.register(Box::new(FakeBackend {
            tier: MemoryTier::Experience,
            hits: vec![],
            consolidate_log: log.clone(),
        }));
        fm.register(Box::new(FakeBackend {
            tier: MemoryTier::CrossProject,
            hits: vec![],
            consolidate_log: log.clone(),
        }));

        let report = fm.consolidate(&sensory);
        assert_eq!(report.total, 3);
        let ordered = log.lock().unwrap().clone();
        // 应按优先级：Experience(60) → CrossProject(55) → Sensory(30)
        assert_eq!(ordered[0], MemoryTier::Experience);
        assert_eq!(ordered[1], MemoryTier::CrossProject);
        assert_eq!(ordered[2], MemoryTier::Sensory);
    }

    #[test]
    fn to_stable_prefix_renders_observations() {
        let a = FakeBackend {
            tier: MemoryTier::Sensory,
            hits: vec![hit(MemoryTier::Sensory, "use redact() for secrets", 1.0)],
            consolidate_log: Arc::new(Mutex::new(Vec::new())),
        };
        let mut fm = FederatedMemory::new();
        fm.register(Box::new(a));

        let msg = fm.to_stable_prefix("secrets", 4).expect("应渲染出稳定前缀");
        match msg {
            Message::System { content, .. } => {
                assert!(
                    content.contains("use redact() for secrets"),
                    "稳定前缀应含命中内容：{content}"
                );
            }
            _ => panic!("应为 System 消息"),
        }
    }

    #[test]
    fn to_stable_prefix_empty_returns_none() {
        let a = FakeBackend {
            tier: MemoryTier::Sensory,
            hits: vec![],
            consolidate_log: Arc::new(Mutex::new(Vec::new())),
        };
        let mut fm = FederatedMemory::new();
        fm.register(Box::new(a));
        assert!(fm.to_stable_prefix("anything", 4).is_none());
    }

    // 确保 `message_text` 与 `SensoryMemory::add_message` 路径一致（构造一条消息不 panic）
    #[test]
    fn message_text_extracts_content() {
        let now = Utc::now();
        let m = Message::Tool {
            name: "calc".to_string(),
            input: "1+1".to_string(),
            output: "2".to_string(),
            timestamp: now,
        };
        assert_eq!(message_text(&m), "2");
    }
}
