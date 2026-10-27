//! 轻量知识库：文档分块 + BM25 检索 + 索引持久化
//!
//! 设计目标：
//! - 零外部模型依赖，单测可确定性验证；
//! - `retrieve` 接口稳定，未来可无缝替换为向量检索；
//! - 分块策略保留段落边界，长段落退化为滑动窗口；
//! - 索引落到 `<data_dir>/kb/store.json`（`version` / `next_id` / `documents`），
//!   重启后 `load_from` 可原样恢复，不再「重启即空库」。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 索引文件格式版本。结构不兼容变更时递增，加载侧据此判断。
pub const KB_INDEX_VERSION: u32 = 1;

/// 持久化文件上限：32 MiB。超过则拒绝加载并给出明确错误（不静默吞掉半截索引）。
pub const MAX_STORE_BYTES: u64 = 32 * 1024 * 1024;

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

/// 持久化文档：一篇文档 = doc_id + 标题 + 分块 + 内容哈希（去重依据）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KbDocument {
    pub doc_id: String,
    pub title: String,
    pub chunks: Vec<String>,
    /// `sha256(title + "\n" + text)`；旧索引缺字段时为空（按「无法判重」处理）。
    #[serde(default)]
    pub hash: String,
}

/// 进程内累计的入库质量计数（随索引一并落盘，重启后不丢）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KbStats {
    #[serde(default)]
    pub duplicate_rejected: u64,
    /// 被去噪丢弃的**段落**数（不是文档数）。
    #[serde(default)]
    pub denoised_dropped: u64,
    /// 因空内容 / 超长 / 配额 / 全篇判噪而被拒绝的入库次数。
    #[serde(default)]
    pub rejected: u64,
    #[serde(default)]
    pub last_ingest_at: Option<String>,
}

/// 索引文件根结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct KbIndex {
    version: u32,
    next_id: usize,
    documents: Vec<KbDocument>,
    #[serde(default)]
    stats: KbStats,
}

/// 内容哈希：`sha256(title + "\n" + text)`，用于跨重启的去重判定。
pub fn document_hash(title: &str, text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(title.as_bytes());
    hasher.update(b"\n");
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 内存知识库（可由索引文件恢复）
#[derive(Debug, Clone, Default)]
pub struct KnowledgeBase {
    chunks: Vec<Chunk>,
    next_id: usize,
    documents: Vec<KbDocument>,
    stats: KbStats,
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

    /// 加入一篇文档（按标题区分来源），返回分配的 doc_id。
    pub fn add_document(&mut self, title: &str, text: &str) -> String {
        self.insert_document(title, text, &document_hash(title, text))
    }

    /// 加入一篇文档并显式携带内容哈希（调用方已判重时复用同一哈希）。
    pub fn insert_document(&mut self, title: &str, text: &str, hash: &str) -> String {
        let doc_id = format!("doc_{}", self.next_id);
        self.next_id += 1;
        let chunks = chunk_text(text, 400, 80);
        self.documents.push(KbDocument {
            doc_id: doc_id.clone(),
            title: title.to_string(),
            chunks: chunks.clone(),
            hash: hash.to_string(),
        });
        for chunk in chunks {
            self.chunks.push(Chunk {
                doc_id: doc_id.clone(),
                title: title.to_string(),
                text: chunk,
            });
        }
        doc_id
    }

    /// 按内容哈希查找已存在文档的 doc_id（跨重启有效）。
    pub fn find_by_hash(&self, hash: &str) -> Option<&str> {
        if hash.is_empty() {
            return None;
        }
        self.documents
            .iter()
            .find(|d| d.hash == hash)
            .map(|d| d.doc_id.as_str())
    }

    /// 已索引的文档数
    pub fn document_count(&self) -> usize {
        self.documents.len()
    }

    /// 分块总数
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// 已索引文档（持久化视图）
    pub fn documents(&self) -> &[KbDocument] {
        &self.documents
    }

    /// 质量计数快照
    pub fn stats(&self) -> &KbStats {
        &self.stats
    }

    /// 质量计数可变引用（网关在入库/拒绝时累加）
    pub fn stats_mut(&mut self) -> &mut KbStats {
        &mut self.stats
    }

    /// 暴露所有分块文本（供完整性审计 / 图谱构建等下游消费）。
    pub fn chunk_texts(&self) -> Vec<String> {
        self.chunks.iter().map(|c| c.text.clone()).collect()
    }

    /// 列出所有已索引文档（按 doc_id 去重，返回 doc_id + title + chunk 数）
    pub fn list_documents(&self) -> Vec<(String, String, usize)> {
        let mut docs: Vec<(String, String, usize)> = self
            .documents
            .iter()
            .map(|d| (d.doc_id.clone(), d.title.clone(), d.chunks.len()))
            .collect();
        docs.sort_by(|a, b| a.0.cmp(&b.0));
        docs
    }

    /// 删除指定文档的所有分块
    pub fn delete_document(&mut self, doc_id: &str) -> bool {
        let before = self.documents.len();
        self.documents.retain(|d| d.doc_id != doc_id);
        self.chunks.retain(|c| c.doc_id != doc_id);
        self.documents.len() < before
    }

    /// 序列化为索引 JSON（`version` / `next_id` / `documents` + 质量计数）。
    pub fn to_index_json(&self) -> String {
        let index = KbIndex {
            version: KB_INDEX_VERSION,
            next_id: self.next_id,
            documents: self.documents.clone(),
            stats: self.stats.clone(),
        };
        serde_json::to_string_pretty(&index).unwrap_or_else(|_| "{}".to_string())
    }

    /// 原子写回索引：先写 `index.json.tmp`，再 rename 覆盖正式文件。
    ///
    /// rename 在 Windows 上使用 `MOVEFILE_REPLACE_EXISTING` 语义，可直接覆盖；
    /// 这样即使进程在写盘中途崩溃，也不会留下半截 `index.json`。
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = PathBuf::from(format!("{}.tmp", path.display()));
        std::fs::write(&tmp, self.to_index_json())?;
        std::fs::rename(&tmp, path)
    }

    /// 从索引文件恢复；文件缺失或损坏返回可读错误（调用方据此标记未持久化）。
    ///
    /// 单文件超过 [`MAX_STORE_BYTES`] 时直接拒绝：一次性读入 32 MiB 以上的 JSON
    /// 会把网关内存打满，且这类文件通常意味着被误写/损坏。
    pub fn load_from(path: &Path) -> Result<Self, String> {
        match std::fs::metadata(path) {
            Ok(meta) if meta.len() > MAX_STORE_BYTES => {
                return Err(format!(
                    "索引文件 {} 为 {} 字节，超过上限 {} 字节（32MB）",
                    path.display(),
                    meta.len(),
                    MAX_STORE_BYTES
                ));
            }
            Ok(_) => {}
            Err(e) => return Err(format!("读取索引失败 {}: {e}", path.display())),
        }
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("读取索引失败 {}: {e}", path.display()))?;
        let index: KbIndex =
            serde_json::from_str(&raw).map_err(|e| format!("索引 JSON 损坏: {e}"))?;
        if index.version > KB_INDEX_VERSION {
            return Err(format!(
                "索引版本 {} 高于当前支持的 {}",
                index.version, KB_INDEX_VERSION
            ));
        }
        let mut kb = KnowledgeBase {
            chunks: Vec::new(),
            next_id: index.next_id,
            documents: index.documents,
            stats: index.stats,
        };
        for doc in &kb.documents {
            for chunk in &doc.chunks {
                kb.chunks.push(Chunk {
                    doc_id: doc.doc_id.clone(),
                    title: doc.title.clone(),
                    text: chunk.clone(),
                });
            }
        }
        // 旧索引可能缺 next_id：按已有 doc_id 推导，避免新文档覆盖旧 id。
        let max_id = kb
            .documents
            .iter()
            .filter_map(|d| d.doc_id.strip_prefix("doc_")?.parse::<usize>().ok())
            .max();
        if let Some(max_id) = max_id {
            kb.next_id = kb.next_id.max(max_id + 1);
        }
        Ok(kb)
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

    fn temp_index(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("mgt_kb_{}_{}", std::process::id(), name))
            .join("kb")
            .join("index.json")
    }

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

    /// 持久化 roundtrip：保存 → 重新加载 → 文档/分块/计数/哈希全部保留。
    #[test]
    fn index_roundtrip_preserves_documents_and_stats() {
        let path = temp_index("roundtrip");
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
        let mut kb = KnowledgeBase::new();
        let id = kb.add_document(
            "部署手册",
            "网关只监听回环地址，首次启动需要写入 gateway-token。",
        );
        kb.stats_mut().duplicate_rejected = 3;
        kb.stats_mut().denoised_dropped = 2;
        kb.stats_mut().rejected = 1;
        kb.stats_mut().last_ingest_at = Some("2026-10-25T10:00:00Z".to_string());
        kb.save_to(&path).expect("保存索引");

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"version\""), "索引必须含 version: {raw}");
        assert!(raw.contains("\"documents\""), "索引必须含 documents: {raw}");
        assert!(raw.contains(&id), "索引必须含 doc_id: {raw}");

        let loaded = KnowledgeBase::load_from(&path).expect("重新加载");
        assert_eq!(loaded.document_count(), 1);
        assert_eq!(loaded.chunk_count(), kb.chunk_count());
        assert_eq!(loaded.list_documents()[0].1, "部署手册");
        assert_eq!(loaded.stats().duplicate_rejected, 3);
        assert_eq!(loaded.stats().denoised_dropped, 2);
        assert_eq!(loaded.stats().rejected, 1);
        assert_eq!(
            loaded.find_by_hash(&document_hash(
                "部署手册",
                "网关只监听回环地址，首次启动需要写入 gateway-token。"
            )),
            Some(id.as_str()),
            "重启后仍能按内容哈希判重"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    /// 重启语义：新进程（新实例）从文件加载后可直接检索，且 doc_id 不重排。
    #[test]
    fn load_after_restart_is_searchable() {
        let path = temp_index("restart");
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
        let mut kb = KnowledgeBase::new();
        kb.add_document("alpha", "alpha 文档描述三缓冲渲染与固定时间步长。");
        kb.save_to(&path).unwrap();
        drop(kb);

        let loaded = KnowledgeBase::load_from(&path).unwrap();
        let hits = loaded.retrieve("三缓冲", 3);
        assert!(!hits.is_empty(), "重启后应能检索到已持久化文档");
        assert_eq!(hits[0].doc_id, "doc_0");

        let mut loaded = loaded;
        let new_id = loaded.add_document("beta", "beta 文档描述新的主题内容。");
        assert_eq!(new_id, "doc_1", "加载后 next_id 必须接续，不能复用 doc_0");
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    /// 损坏/缺失文件必须给出可读错误（网关据此标记 persisted:false）。
    #[test]
    fn load_missing_or_corrupt_index_reports_error() {
        let missing = temp_index("missing");
        let _ = std::fs::remove_dir_all(missing.parent().unwrap().parent().unwrap());
        let err = KnowledgeBase::load_from(&missing).unwrap_err();
        assert!(err.contains("读取索引失败"), "{err}");

        let corrupt = temp_index("corrupt");
        std::fs::create_dir_all(corrupt.parent().unwrap()).unwrap();
        std::fs::write(&corrupt, "{ not json").unwrap();
        let err = KnowledgeBase::load_from(&corrupt).unwrap_err();
        assert!(err.contains("损坏"), "{err}");
        let _ = std::fs::remove_dir_all(corrupt.parent().unwrap().parent().unwrap());
    }

    /// 32MB 上限：超限索引直接拒绝加载（明确错误，不尝试把文件读进内存）。
    #[test]
    fn load_rejects_store_larger_than_32mb() {
        let path = temp_index("too_large");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        {
            let f = std::fs::File::create(&path).unwrap();
            f.set_len(MAX_STORE_BYTES + 1).unwrap();
        }
        let err = KnowledgeBase::load_from(&path).unwrap_err();
        assert!(err.contains("超过上限"), "{err}");
        assert!(err.contains("32MB"), "{err}");
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn delete_removes_document_and_chunks() {
        let mut kb = KnowledgeBase::new();
        let id = kb.add_document("x", "内容用于验证删除会同时清掉分块索引。");
        assert!(kb.delete_document(&id));
        assert_eq!(kb.document_count(), 0);
        assert_eq!(kb.chunk_count(), 0);
        assert!(!kb.delete_document(&id), "重复删除应如实返回 false");
    }
}
