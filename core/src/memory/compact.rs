//! 记忆压缩 + 混合检索（KV-Cache 极简工程 §2.10，蒸馏 claude-mem）
//!
//! claude-mem 的核心模型不是"存全文"，而是把一整段会话压成一条 **Observation**：
//! `{type, title, facts[], concepts[], discovery_tokens}`。
//! 「发现」这些结论原本烧掉的 token（`discovery_tokens`）远大于「读回」它的 token
//! （`read_tokens`），二者之差就是记忆压缩的**真实收益**（`TokenEconomics`）。
//!
//! 本模块蒸馏其三件套，全部零外部依赖、确定性（不调 LLM）：
//! 1. `compact_messages` —— 抽取式压缩（长会话 → 一条 Observation）
//! 2. `token_economics` —— 如实计量收益（**允许为负**，压不动就是压不动，不虚报）
//! 3. `render_timeline` / `as_stable_prefix` —— 紧凑扁平行渲染，并作为
//!    **稳定前缀**（`Message::System`）注入，直接接上 P-7 的 KV-Cache P1 原则
//!
//! 外加混合检索 `hybrid_retrieve`（词法覆盖 + 向量余弦）。

use std::collections::{HashMap, HashSet};

use crate::memory::embedding::EmbeddingProvider;
use crate::model::llm::token_estimate;
use crate::types::Message;

/// 词法腿权重（精确词覆盖：可解释、零假阳性）
const LEXICAL_WEIGHT: f64 = 0.6;
/// 向量腿权重（特征哈希词袋 + bigram：补短语级与长度归一化信号）
const VECTOR_WEIGHT: f64 = 0.4;

/// 单条 fact 的最大字符数（压缩必须真的压，否则 `TokenEconomics` 会如实变负）
const MAX_FACT_CHARS: usize = 200;
/// 标题最大字符数
const MAX_TITLE_CHARS: usize = 120;
/// 概念标签数量上限
const MAX_CONCEPTS: usize = 6;

/// 极小停用词表（只挡最泛的功能词；不做词干化，保持确定性可解释）
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "is", "are", "was", "were", "be", "been", "to", "of",
    "in", "on", "for", "with", "at", "by", "it", "its", "this", "that", "these", "those", "as",
    "from", "not", "no", "do", "does", "did", "so", "if", "then", "than", "there", "here", "we",
    "you", "they", "he", "she", "i",
];

fn is_stopword(t: &str) -> bool {
    STOPWORDS.contains(&t)
}

/// 分词：按非字母数字切分并小写。
///
/// 诚实说明：CJK 连续段会整体成为一个"词"（未接分词库，保持零依赖）；
/// 中文场景的召回主要由向量腿（`LocalEmbedding` 内部逐字 + bigram）补足。
fn tokens_of(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

/// 句子切分（中英标点 + 换行）
fn split_sentences(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if matches!(c, '.' | '!' | '?' | '。' | '！' | '？' | '\n' | ';' | '；') {
            let t = cur.trim();
            if !t.is_empty() {
                out.push(t.to_string());
            }
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    let t = cur.trim();
    if !t.is_empty() {
        out.push(t.to_string());
    }
    out
}

/// 信息密度打分：去重后的实义词数量（停用词与超短词不计）
fn informativeness(s: &str) -> usize {
    let mut seen: HashSet<String> = HashSet::new();
    for t in tokens_of(s) {
        if t.chars().count() >= 3 && !is_stopword(&t) {
            seen.insert(t);
        }
    }
    seen.len()
}

/// 按字符数截断（不切裂 UTF-8）
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

/// 提取消息的可读正文（Tool 消息取 输入+输出）
fn message_text(m: &Message) -> String {
    match m {
        Message::User { content, .. }
        | Message::Assistant { content, .. }
        | Message::System { content, .. } => content.clone(),
        Message::Tool { input, output, .. } => format!("{input}\n{output}"),
    }
}

/// 一条压缩后的记忆单元（claude-mem `Observation` 蒸馏）
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    /// 观测类型（claude-mem `type`：如 "session" / "bugfix" / "research"）
    pub kind: String,
    /// 一行要点（会话主诉）
    pub title: String,
    /// 抽取出的关键事实
    pub facts: Vec<String>,
    /// 概念标签（检索锚点）
    pub concepts: Vec<String>,
    /// 「发现」这些结论原本消耗的 token（= 原始会话全文 token）
    pub discovery_tokens: usize,
    /// 创建时间（毫秒）
    pub created_at_ms: u64,
}

impl Observation {
    /// 检索/渲染用的正文（title + facts）
    pub fn text(&self) -> String {
        let mut s = self.title.clone();
        for f in &self.facts {
            s.push('\n');
            s.push_str(f);
        }
        s
    }

    /// 「读回」这条记忆所需的 token
    pub fn read_tokens(&self, family: f64) -> usize {
        token_estimate(&self.text(), family)
    }
}

/// 记忆压缩的收益账本（claude-mem `TokenEconomics` 蒸馏）
///
/// `savings` 用 `i64` 而非 `usize`：压缩失败（读回比发现还贵）时如实为负，不夹带私货。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TokenEconomics {
    /// 观测条数
    pub observations: usize,
    /// 读回全部观测的 token
    pub read_tokens: usize,
    /// 发现它们原本烧掉的 token
    pub discovery_tokens: usize,
    /// 净节省（discovery - read，可为负）
    pub savings: i64,
    /// 节省百分比（discovery 为 0 时记 0）
    pub savings_percent: i64,
}

/// 混合检索命中
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryHit {
    /// 在输入切片中的下标
    pub index: usize,
    /// 融合得分
    pub score: f64,
    /// 命中观测的标题（便于直接展示）
    pub title: String,
}

/// 把一段会话压成一条 Observation（抽取式，确定性，零 LLM 调用）。
///
/// - `title`：首条 User 消息的第一句（会话主诉）；无 User 则退化为首条非空消息首句。
/// - `facts`：从 Assistant / Tool 输出里按信息密度取 top `max_facts` 句，去重后**恢复原文顺序**。
/// - `concepts`：全文高频实义词，作为检索锚点。
/// - `discovery_tokens`：**整段会话**的 token 成本（claude-mem 的关键度量）。
///
/// 抽取式而非生成式：完全确定性、可单测、不烧 token，也永远不会编造原文没有的内容。
pub fn compact_messages(
    messages: &[Message],
    kind: &str,
    max_facts: usize,
    family: f64,
) -> Observation {
    // 1. 发现成本 = 整段会话的 token（压缩收益的分母）
    let discovery_tokens: usize = messages
        .iter()
        .map(|m| match m {
            Message::Tool { input, output, .. } => {
                token_estimate(input, family) + token_estimate(output, family)
            }
            other => token_estimate(&message_text(other), family),
        })
        .sum();

    // 2. 标题：首条 User 首句；退化为首条非空消息首句
    let title = messages
        .iter()
        .find(|m| matches!(m, Message::User { .. }))
        .or_else(|| messages.iter().find(|m| !message_text(m).trim().is_empty()))
        .and_then(|m| split_sentences(&message_text(m)).into_iter().next())
        .map(|s| truncate_chars(&s, MAX_TITLE_CHARS))
        .unwrap_or_else(|| "(empty session)".to_string());

    // 3. 事实：Assistant / Tool 输出的句子，按信息密度选 top-N，再恢复原文顺序
    let mut candidates: Vec<(usize, usize, String)> = Vec::new(); // (原序, 分数, 句子)
    let mut seen: HashSet<String> = HashSet::new();
    for m in messages {
        if matches!(m, Message::User { .. } | Message::System { .. }) {
            continue;
        }
        for s in split_sentences(&message_text(m)) {
            let key = s.to_lowercase();
            if !seen.insert(key) {
                continue; // 去重（重复段落不重复计入）
            }
            let score = informativeness(&s);
            if score == 0 {
                continue;
            }
            candidates.push((candidates.len(), score, truncate_chars(&s, MAX_FACT_CHARS)));
        }
    }
    // 分数降序，同分按原序（确定性）
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    candidates.truncate(max_facts);
    candidates.sort_by_key(|c| c.0); // 恢复原文顺序，便于人读
    let facts: Vec<String> = candidates.into_iter().map(|c| c.2).collect();

    // 4. 概念标签：全文高频实义词
    let mut freq: HashMap<String, usize> = HashMap::new();
    for m in messages {
        for t in tokens_of(&message_text(m)) {
            if t.chars().count() >= 3 && !is_stopword(&t) {
                *freq.entry(t).or_insert(0) += 1;
            }
        }
    }
    let mut ranked: Vec<(String, usize)> = freq.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0))); // 同频按字典序，保证确定性
    let concepts: Vec<String> = ranked
        .into_iter()
        .take(MAX_CONCEPTS)
        .map(|(t, _)| t)
        .collect();

    Observation {
        kind: kind.to_string(),
        title,
        facts,
        concepts,
        discovery_tokens,
        created_at_ms: chrono::Utc::now().timestamp_millis().max(0) as u64,
    }
}

/// 统计一批观测的压缩收益。
///
/// **不夹私货**：`savings` 与 `savings_percent` 都用 `i64`，压缩失败时如实为负。
pub fn token_economics(observations: &[Observation], family: f64) -> TokenEconomics {
    let read_tokens: usize = observations.iter().map(|o| o.read_tokens(family)).sum();
    let discovery_tokens: usize = observations.iter().map(|o| o.discovery_tokens).sum();
    let savings = discovery_tokens as i64 - read_tokens as i64;
    let savings_percent = if discovery_tokens > 0 {
        savings * 100 / discovery_tokens as i64
    } else {
        0
    };
    TokenEconomics {
        observations: observations.len(),
        read_tokens,
        discovery_tokens,
        savings,
        savings_percent,
    }
}

/// 紧凑扁平行渲染（claude-mem `AgentFormatter` 蒸馏：给 LLM 读的是扁平行，不是表格）。
///
/// 每条观测一行 `- [kind] title — fact; fact`。`|` 会被替换为 `/`，
/// 保证输出永远不会被误解析成 markdown 表格。
pub fn render_timeline(observations: &[Observation]) -> String {
    observations
        .iter()
        .map(|o| {
            let facts = o.facts.join("; ");
            let line = if facts.is_empty() {
                format!("- [{}] {}", o.kind, o.title)
            } else {
                format!("- [{}] {} — {}", o.kind, o.title, facts)
            };
            line.replace('|', "/")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 把压缩记忆渲染成 **稳定前缀**（`Message::System`），直接落进 P-7
/// `reorder_stable_prefix` 的 stable 半区 —— KV-Cache 原则 P1。
///
/// 表头是固定字面量、正文只由观测内容决定，因此同一批召回结果产出的 `content`
/// **逐字节稳定**，可被 `PrefixCache::note_prefix` 观测为前缀复用。
pub fn as_stable_prefix(observations: &[Observation]) -> Option<Message> {
    if observations.is_empty() {
        return None;
    }
    Some(Message::System {
        content: format!(
            "## Recalled memory (compacted)\n{}",
            render_timeline(observations)
        ),
        timestamp: chrono::Utc::now(),
    })
}

/// 混合检索：词法覆盖（0.6）+ 向量余弦（0.4）。
///
/// - **词法腿**：query 实义词在观测正文中的覆盖率 ∈[0,1]，精确、可解释、零假阳性。
/// - **向量腿**：当前嵌入后端（默认 `LocalEmbedding` 特征哈希词袋，含 bigram + L2 归一化）的余弦。
///   诚实说明：默认后端**不是**真语义嵌入 —— 完全无共享词的两段文本余弦≈0。它提供的是
///   **短语级（bigram）与长度归一化**信号，用来在词法打平时给出更优排序；启用真实模型后端后此处即真语义。
///
/// query 无实义词、观测为空或 `top_k == 0` 时返回空。
pub fn hybrid_retrieve(observations: &[Observation], query: &str, top_k: usize) -> Vec<MemoryHit> {
    let q_tokens: Vec<String> = tokens_of(query)
        .into_iter()
        .filter(|t| !is_stopword(t))
        .collect();
    if q_tokens.is_empty() || observations.is_empty() || top_k == 0 {
        return Vec::new();
    }

    let q_vec = EmbeddingProvider::global().embed(query);
    let mut hits: Vec<MemoryHit> = Vec::new();
    for (index, o) in observations.iter().enumerate() {
        let text = o.text();
        let doc_tokens: HashSet<String> = tokens_of(&text).into_iter().collect();
        let covered = q_tokens.iter().filter(|t| doc_tokens.contains(*t)).count();
        let lexical = covered as f64 / q_tokens.len() as f64;
        let semantic = EmbeddingProvider::global()
            .cosine(&q_vec, &EmbeddingProvider::global().embed(&text))
            .clamp(0.0, 1.0);
        let score = LEXICAL_WEIGHT * lexical + VECTOR_WEIGHT * semantic;
        if score > 0.0 {
            hits.push(MemoryHit {
                index,
                score,
                title: o.title.clone(),
            });
        }
    }

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.index.cmp(&b.index))
    });
    hits.truncate(top_k);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn user(c: &str) -> Message {
        Message::User {
            content: c.into(),
            timestamp: Utc::now(),
        }
    }

    fn assistant(c: &str) -> Message {
        Message::Assistant {
            content: c.into(),
            timestamp: Utc::now(),
            thinking: None,
        }
    }

    fn obs(title: &str, facts: &[&str]) -> Observation {
        Observation {
            kind: "session".into(),
            title: title.into(),
            facts: facts.iter().map(|s| s.to_string()).collect(),
            concepts: Vec::new(),
            discovery_tokens: 0,
            created_at_ms: 0,
        }
    }

    #[test]
    fn compact_extracts_title_from_first_user_message() {
        let msgs = vec![
            user("Fix the failover chain bug. It retries forever."),
            assistant("The router loops because the candidate list is never advanced."),
        ];
        let o = compact_messages(&msgs, "bugfix", 5, 1.0);
        assert_eq!(o.kind, "bugfix");
        assert!(
            o.title.starts_with("Fix the failover chain bug"),
            "title should come from first user message, got {:?}",
            o.title
        );
    }

    #[test]
    fn compact_records_discovery_tokens_of_whole_session() {
        let long_answer = "The router loops because the candidate list is never advanced. \
             Each attempt reuses the same index. The fix is to iterate over candidates. \
             Tests must cover the all-fail path as well."
            .repeat(4);
        let msgs = vec![user("Why does failover loop?"), assistant(&long_answer)];
        let o = compact_messages(&msgs, "bugfix", 3, 1.0);

        let whole: usize =
            token_estimate("Why does failover loop?", 1.0) + token_estimate(&long_answer, 1.0);
        assert_eq!(
            o.discovery_tokens, whole,
            "discovery_tokens must equal the whole session cost"
        );
        assert!(
            o.read_tokens(1.0) < o.discovery_tokens,
            "compaction should shrink: read={} discovery={}",
            o.read_tokens(1.0),
            o.discovery_tokens
        );
    }

    #[test]
    fn compact_limits_facts_to_max() {
        let msgs = vec![
            user("Summarize the audit."),
            assistant(
                "Failover is missing. Streaming never yields chunks. \
                 Anthropic format is unsupported. Cache has no prefix reuse metric. \
                 Personas are not injected into skills.",
            ),
        ];
        let o = compact_messages(&msgs, "research", 2, 1.0);
        assert!(o.facts.len() <= 2, "facts={:?}", o.facts);
        assert!(!o.facts.is_empty(), "should extract at least one fact");
    }

    #[test]
    fn token_economics_reports_savings() {
        let mut a = obs("short title", &["one fact"]);
        a.discovery_tokens = 1000;
        let mut b = obs("another title", &["second fact"]);
        b.discovery_tokens = 500;

        let e = token_economics(&[a.clone(), b.clone()], 1.0);
        let expected_read = a.read_tokens(1.0) + b.read_tokens(1.0);
        assert_eq!(e.observations, 2);
        assert_eq!(e.discovery_tokens, 1500);
        assert_eq!(e.read_tokens, expected_read);
        assert_eq!(e.savings, 1500 - expected_read as i64);
        assert!(e.savings_percent > 0 && e.savings_percent <= 100);
    }

    #[test]
    fn token_economics_negative_savings_reported_honestly() {
        // 压不动的场景：discovery 极小，读回反而更贵 —— 必须如实为负，不许夹成 0。
        let mut a = obs(
            "a very long stored title that costs far more to read back than it ever cost to find",
            &["and an equally verbose fact line that inflates the read cost even further"],
        );
        a.discovery_tokens = 1;

        let e = token_economics(&[a.clone()], 1.0);
        assert!(
            e.savings < 0,
            "negative savings must not be clamped, got {:?}",
            e
        );
        assert!(
            e.savings_percent < 0,
            "percent must follow sign, got {:?}",
            e
        );
    }

    #[test]
    fn render_timeline_uses_compact_flat_lines() {
        let out = render_timeline(&[obs("first thing", &["f1"]), obs("second thing", &["f2"])]);
        assert!(out.contains("first thing"));
        assert!(out.contains("second thing"));
        assert!(
            !out.contains('|'),
            "agent mode is flat lines, not markdown tables: {out}"
        );
        assert_eq!(
            out.lines().filter(|l| !l.trim().is_empty()).count(),
            2,
            "one compact line per observation: {out}"
        );
    }

    #[test]
    fn as_stable_prefix_emits_system_message() {
        let m = as_stable_prefix(&[obs("recalled thing", &["fact"])])
            .expect("non-empty observations should yield a prefix");
        match m {
            Message::System { ref content, .. } => {
                assert!(content.contains("recalled thing"), "content={content}")
            }
            other => panic!(
                "memory prefix must be System (KV-Cache P1), got {:?}",
                other.role()
            ),
        }
    }

    #[test]
    fn as_stable_prefix_empty_is_none() {
        assert!(as_stable_prefix(&[]).is_none());
    }

    #[test]
    fn compacted_memory_lands_in_volatile_zone_not_prefix_head() {
        // P-9 ↔ P-7 接线证据（M2 降耗引擎修订）：压缩记忆是按本轮查询文本召回的
        // 逐轮变化内容，注入后必须落在 reorder_stable_prefix 不再移动的 volatile
        // 区（首个非 System 消息之后保持原位），不得抢占稳定前缀头——否则服务端
        // KV 前缀从第 0 字节失配（实测 0/40 客户端命中、仅 9.7% 服务端命中的根因）。
        let mem = as_stable_prefix(&[obs("prior session learned X", &["X works"])]).unwrap();
        let mixed = vec![user("now do Y"), mem, assistant("ok")];
        let ordered = crate::context::optimize::reorder_stable_prefix(&mixed);
        assert_eq!(
            ordered[1].role(),
            "system",
            "压缩记忆注入后保持原位（mid-sequence System 不被前置）"
        );
        assert_ne!(
            ordered[0].role(),
            "system",
            "稳定前缀头不得被逐轮变化的召回内容占据"
        );
    }

    #[test]
    fn hybrid_retrieve_ranks_lexical_match_first() {
        let items = vec![
            obs("bananas and oranges", &["fruit basket contents"]),
            obs(
                "kubernetes pod scheduling",
                &["scheduler binds pods to nodes"],
            ),
        ];
        let hits = hybrid_retrieve(&items, "kubernetes pod scheduling", 2);
        assert!(!hits.is_empty(), "should find the matching observation");
        assert_eq!(hits[0].index, 1, "hits={:?}", hits);
        assert!(hits[0].score > 0.0);
    }

    #[test]
    fn hybrid_retrieve_vector_leg_breaks_lexical_tie_with_phrase() {
        // 两条都含 rust 与 ownership（词法覆盖打平），只有第一条是相邻短语；
        // 向量腿（特征哈希含 bigram）必须把短语那条顶上去 —— 证明向量腿真的参与了打分。
        let items = vec![
            obs("rust ownership", &["rust ownership prevents data races"]),
            obs(
                "language notes",
                &["rust is one option and separately ownership is a concept in many languages"],
            ),
        ];
        let hits = hybrid_retrieve(&items, "rust ownership", 2);
        assert_eq!(hits.len(), 2, "both contain the query terms: {hits:?}");
        assert_eq!(
            hits[0].index, 0,
            "adjacent-phrase match must outrank scattered tokens: {hits:?}"
        );
    }

    #[test]
    fn hybrid_retrieve_respects_top_k_and_empty_query() {
        let items = vec![
            obs("alpha topic", &["alpha detail"]),
            obs("alpha again", &["more alpha"]),
            obs("alpha third", &["alpha alpha"]),
        ];
        assert_eq!(hybrid_retrieve(&items, "alpha", 2).len(), 2);
        assert!(hybrid_retrieve(&items, "", 5).is_empty());
        assert!(hybrid_retrieve(&[], "alpha", 5).is_empty());
    }
}
