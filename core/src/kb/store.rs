//! 轻量知识库：文档分块 + BM25 检索
//!
//! 设计目标：
//! - 零外部模型依赖，单测可确定性验证；
//! - `retrieve` 接口稳定，未来可无缝替换为向量检索；
//! - 分块策略保留段落边界，长段落退化为滑动窗口。

use std::collections::HashMap;

/// 单条检索结果
#[derive(Debug, Clone)]
pub struct RetrievalResult {
    pub doc_id: String,
    pub title: String,
    pub chunk: String,
    pub score: f64,
}

#[derive(Debug, Clone)]
struct Chunk {
    doc_id: String,
    title: String,
    text: String,
}

/// 内存知识库
#[derive(Debug, Clone, Default)]
pub struct KnowledgeBase {
    chunks: Vec<Chunk>,
    next_id: usize,
}

pub(crate) fn tokenize(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // 拉丁/数字按非字母数字切词；CJK（中文/日文/韩文）逐字切分（零依赖，
    // 无分词库，单字 + 双字组合提升中文检索召回）。
    let mut word = String::new();
    for c in s.to_lowercase().chars() {
        if is_cjk(c) {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            out.push(c.to_string());
        } else if c.is_alphanumeric() {
            word.push(c);
        } else if !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    // 中文双字组合（bigram）增强语义命中
    let chars: Vec<String> = out.iter().filter(|t| is_cjk_char(t)).cloned().collect();
    if chars.len() >= 2 {
        for pair in chars.windows(2) {
            out.push(format!("{}{}", pair[0], pair[1]));
        }
    }
    out.retain(|t| t.chars().count() >= 1 && !t.is_empty());
    out
}

fn is_cjk(c: char) -> bool {
    let cp = c as u32;
    (0x4E00..=0x9FFF).contains(&cp)
        || (0x3400..=0x4DBF).contains(&cp)
        || (0x3040..=0x30FF).contains(&cp) // 日文假名
        || (0xAC00..=0xD7AF).contains(&cp) // 韩文
}

fn is_cjk_char(s: &str) -> bool {
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => is_cjk(c),
        _ => false,
    }
}

/// 将文本切分为检索块：优先按段落，超长段落退化为词级滑动窗口
fn chunk_text(text: &str, size: usize, overlap: usize) -> Vec<String> {
    let approx_words = (size / 6).max(8);
    let overlap_words = (overlap / 6).max(2);
    let paras: Vec<&str> = text
        .split("\n\n")
        .filter(|p| !p.trim().is_empty())
        .collect();
    let mut out: Vec<String> = Vec::new();
    for p in paras {
        if p.chars().count() <= size {
            out.push(p.to_string());
            continue;
        }
        let words: Vec<&str> = p.split_whitespace().collect();
        let mut start = 0usize;
        while start < words.len() {
            let end = (start + approx_words).min(words.len());
            out.push(words[start..end].join(" "));
            if end == words.len() {
                break;
            }
            start = end.saturating_sub(overlap_words);
        }
    }
    if out.is_empty() {
        out.push(text.to_string());
    }
    out
}

impl KnowledgeBase {
    pub fn new() -> Self {
        Self::default()
    }

    /// 加入一篇文档（按标题区分来源）
    pub fn add_document(&mut self, title: &str, text: &str) {
        let doc_id = format!("doc_{}", self.next_id);
        self.next_id += 1;
        for chunk in chunk_text(text, 400, 80) {
            self.chunks.push(Chunk {
                doc_id: doc_id.clone(),
                title: title.to_string(),
                text: chunk,
            });
        }
    }

    /// 已索引的文档数（按 doc_id 去重）
    pub fn document_count(&self) -> usize {
        let mut ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for c in &self.chunks {
            ids.insert(c.doc_id.clone());
        }
        ids.len()
    }

    /// 分块总数
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// 暴露所有分块文本（供完整性审计 / 图谱构建等下游消费）。
    pub fn chunk_texts(&self) -> Vec<String> {
        self.chunks.iter().map(|c| c.text.clone()).collect()
    }

    /// 列出所有已索引文档（按 doc_id 去重，返回 doc_id + title + chunk 数）
    pub fn list_documents(&self) -> Vec<(String, String, usize)> {
        let mut map: HashMap<String, (String, usize)> = HashMap::new();
        for c in &self.chunks {
            let entry = map.entry(c.doc_id.clone()).or_insert((c.title.clone(), 0));
            entry.1 += 1;
        }
        let mut docs: Vec<(String, String, usize)> = map
            .into_iter()
            .map(|(id, (title, count))| (id, title, count))
            .collect();
        docs.sort_by(|a, b| a.0.cmp(&b.0));
        docs
    }

    /// 删除指定文档的所有分块
    pub fn delete_document(&mut self, doc_id: &str) -> bool {
        let before = self.chunks.len();
        self.chunks.retain(|c| c.doc_id != doc_id);
        self.chunks.len() < before
    }

    /// BM25 检索，返回 top_k 结果（按得分降序）
    pub fn retrieve(&self, query: &str, top_k: usize) -> Vec<RetrievalResult> {
        let docs: Vec<&str> = self.chunks.iter().map(|c| c.text.as_str()).collect();
        bm25_score(&docs, query)
            .into_iter()
            .take(top_k)
            .map(|(i, s)| {
                let c = &self.chunks[i];
                RetrievalResult {
                    doc_id: c.doc_id.clone(),
                    title: c.title.clone(),
                    chunk: c.text.clone(),
                    score: s,
                }
            })
            .collect()
    }
}

/// BM25 打分：对文档列表计算得分，返回降序的 (文档索引, 分数)（只含非零分数）。
///
/// 供知识库检索与代码索引（codeintel）复用，避免两处重复 BM25 实现。
pub(crate) fn bm25_score(docs: &[&str], query: &str) -> Vec<(usize, f64)> {
    let q_tokens = tokenize(query);
    if q_tokens.is_empty() || docs.is_empty() {
        return Vec::new();
    }

    let n = docs.len() as f64;
    let mut df: HashMap<String, f64> = HashMap::new();
    let mut tfs: Vec<HashMap<String, f64>> = Vec::with_capacity(docs.len());
    let mut lens: Vec<f64> = Vec::with_capacity(docs.len());

    for text in docs {
        let toks = tokenize(text);
        let mut tf: HashMap<String, f64> = HashMap::new();
        for t in &toks {
            *tf.entry(t.clone()).or_insert(0.0) += 1.0;
        }
        for t in tf.keys() {
            *df.entry(t.clone()).or_insert(0.0) += 1.0;
        }
        lens.push(toks.len() as f64);
        tfs.push(tf);
    }

    let avgdl = lens.iter().sum::<f64>() / n.max(1.0);
    let k1 = 1.5f64;
    let b = 0.75f64;

    let mut scored: Vec<(usize, f64)> = Vec::new();
    for (i, tf) in tfs.iter().enumerate() {
        let mut score = 0.0f64;
        for qt in &q_tokens {
            if let Some(&df_t) = df.get(qt) {
                let idf = ((n - df_t + 0.5) / (df_t + 0.5) + 1.0).ln();
                let f = tf.get(qt).copied().unwrap_or(0.0);
                if f > 0.0 {
                    score += idf * (f * (k1 + 1.0)) / (f + k1 * (1.0 - b + b * (lens[i] / avgdl)));
                }
            }
        }
        if score > 0.0 {
            scored.push((i, score));
        }
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_returns_nothing() {
        let mut kb = KnowledgeBase::new();
        kb.add_document("rust", "Rust is a systems programming language.");
        assert!(kb.retrieve("", 3).is_empty());
        assert!(kb.retrieve("   ", 3).is_empty());
    }

    #[test]
    fn retrieves_relevant_chunk() {
        let mut kb = KnowledgeBase::new();
        kb.add_document(
            "langs",
            "Rust emphasizes memory safety without a garbage collector. Go is simple and concurrent. Python is dynamic and readable.",
        );
        kb.add_document(
            "fruit",
            "Apples and oranges are common fruits. Bananas are rich in potassium.",
        );
        let res = kb.retrieve("memory safety systems programming", 2);
        assert!(!res.is_empty(), "should find something");
        assert_eq!(
            res[0].doc_id, "doc_0",
            "most relevant should be the rust doc"
        );
        assert!(res[0].score > 0.0);
    }

    #[test]
    fn irrelevant_query_ranks_low_or_empty() {
        let mut kb = KnowledgeBase::new();
        kb.add_document("a", "The quick brown fox jumps over the lazy dog.");
        let res = kb.retrieve("quantum entanglement astrophysics", 1);
        // 相关性极低，BM25 仍可能给微小分；这里验证不会崩溃且分数有限
        assert!(res.len() <= 1);
    }

    #[test]
    fn multiple_documents_ranked() {
        let mut kb = KnowledgeBase::new();
        kb.add_document(
            "d1",
            "Kubernetes orchestrates containers at scale. Pods and services define topology.",
        );
        kb.add_document(
            "d2",
            "Docker packages applications into containers with images and layers.",
        );
        kb.add_document("d3", "Rust ownership prevents data races at compile time.");
        let res = kb.retrieve("containers orchestration kubernetes", 3);
        assert!(!res.is_empty());
        assert_eq!(res[0].doc_id, "doc_0");
        assert!(!res.is_empty() && res.len() <= 3, "len={}", res.len());
    }

    #[test]
    fn chunking_splits_long_paragraph() {
        let mut kb = KnowledgeBase::new();
        let long = "word ".repeat(300);
        kb.add_document("big", &long);
        assert!(
            kb.chunk_count() > 1,
            "long paragraph should be split into multiple chunks"
        );
    }

    #[test]
    fn document_count_dedupes_chunks() {
        let mut kb = KnowledgeBase::new();
        kb.add_document("x", "short one.\n\nsecond short.");
        assert_eq!(kb.document_count(), 1);
        assert!(kb.chunk_count() >= 2);
    }
}
