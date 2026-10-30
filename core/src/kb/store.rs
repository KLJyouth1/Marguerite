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

/// 索引里的**文件来源记录**：把一篇文档绑定到磁盘上的源文件，
/// 并记住入库当时的 mtime 与字节数 —— 增量索引（[`KnowledgeBase::apply_changes`]）
/// 与启动自检（`integrity::audit_sources`）都以它作为唯一比对依据。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KbSource {
    /// 源文件路径，按调用方传入的形态持久化（比较时按字符串精确匹配）。
    pub path: String,
    /// 该文件对应的文档 id（`doc_N`）。
    pub doc_id: String,
    /// 入库时的修改时间（自 UNIX_EPOCH 起的毫秒数；不可得时为 0）。
    pub mtime_ms: u64,
    /// 入库时的文件字节数。
    pub size: u64,
}

/// 增量索引结果。三个桶里存的都是**源文件路径**（与 `apply_changes` 入参同一形态）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyReport {
    /// 磁盘变更后重建了索引的文件。
    pub rebuilt: Vec<String>,
    /// 磁盘上的文件已消失、索引记录被移除的文件。
    pub removed: Vec<String>,
    /// mtime/size 与索引记录一致，本次未做任何改动的文件。
    pub unchanged: Vec<String>,
    /// 被移除文档的 doc_id：调用方据此清理图谱等下游索引，避免悬空节点。
    pub removed_doc_ids: Vec<String>,
}

/// 索引文件根结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct KbIndex {
    version: u32,
    next_id: usize,
    documents: Vec<KbDocument>,
    #[serde(default)]
    stats: KbStats,
    /// 文件来源记录（旧索引缺字段时为空：视为「没有任何文件来源」，不是「校验通过」）。
    #[serde(default)]
    sources: Vec<KbSource>,
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
    sources: Vec<KbSource>,
}

/// 文件修改时间（自 UNIX_EPOCH 起的毫秒数，不可得时为 0）。
pub(crate) fn mtime_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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

    /// 文件来源记录（启动自检 / 增量索引的比对依据）。
    pub fn sources(&self) -> &[KbSource] {
        &self.sources
    }

    /// 增量索引：按 **磁盘 mtime + 字节数** 判断每个路径是否需要重建。
    ///
    /// - 新文件 / mtime 或 size 变化的文件 → 先摘掉旧文档（含分块），再整篇重建；
    /// - 磁盘上已消失且索引里有记录 → 删文档并清掉来源记录（图谱清理见 `removed_doc_ids`）；
    /// - 记录与磁盘一致 → `unchanged`，不重切块、不重算 BM25；
    /// - 磁盘上不存在且从未索引 → `unchanged`（无可做之事）。
    ///
    /// 读不出来的文件保持原状（不静默丢掉已有索引），同样记入 `unchanged`。
    /// 不做后台监听：调用方（启动自检 / 显式刷新）决定何时调用。
    pub fn apply_changes(&mut self, paths: &[PathBuf]) -> ApplyReport {
        let mut report = ApplyReport::default();
        for path in paths {
            let key = path.display().to_string();
            match std::fs::metadata(path) {
                Ok(meta) => {
                    let (mtime, size) = (mtime_ms(&meta), meta.len());
                    let indexed = self.sources.iter().find(|s| s.path == key).cloned();
                    if let Some(src) = &indexed {
                        if src.mtime_ms == mtime && src.size == size {
                            report.unchanged.push(key);
                            continue;
                        }
                    }
                    let Ok(text) = std::fs::read_to_string(path) else {
                        // 元数据在、正文读不出来：保持旧索引，如实记为未改动。
                        report.unchanged.push(key);
                        continue;
                    };
                    if let Some(src) = indexed {
                        self.delete_document(&src.doc_id);
                    }
                    let title = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("untitled")
                        .to_string();
                    let hash = document_hash(&title, &text);
                    let doc_id = self.insert_document(&title, &text, &hash);
                    self.sources.push(KbSource {
                        path: key.clone(),
                        doc_id,
                        mtime_ms: mtime,
                        size,
                    });
                    report.rebuilt.push(key);
                }
                Err(_) => {
                    if let Some(pos) = self.sources.iter().position(|s| s.path == key) {
                        let src = self.sources.remove(pos);
                        self.delete_document(&src.doc_id);
                        report.removed_doc_ids.push(src.doc_id);
                        report.removed.push(key);
                    } else {
                        report.unchanged.push(key);
                    }
                }
            }
        }
        report
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

    /// 删除指定文档的所有分块（同时清掉它的文件来源记录，避免自检误报「已索引」）
    pub fn delete_document(&mut self, doc_id: &str) -> bool {
        let before = self.documents.len();
        self.documents.retain(|d| d.doc_id != doc_id);
        self.chunks.retain(|c| c.doc_id != doc_id);
        self.sources.retain(|s| s.doc_id != doc_id);
        self.documents.len() < before
    }

    /// 序列化为索引 JSON（`version` / `next_id` / `documents` + 质量计数 + 文件来源记录）。
    pub fn to_index_json(&self) -> String {
        let index = KbIndex {
            version: KB_INDEX_VERSION,
            next_id: self.next_id,
            documents: self.documents.clone(),
            stats: self.stats.clone(),
            sources: self.sources.clone(),
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
            sources: index.sources,
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

/// 模块级入口：对给定路径执行增量索引（等价于 [`KnowledgeBase::apply_changes`]）。
pub fn apply_changes(kb: &mut KnowledgeBase, paths: &[PathBuf]) -> ApplyReport {
    kb.apply_changes(paths)
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

    /// 增量索引测试用的临时源文件：目录名带用例内唯一前缀，避免并行用例互相清目录。
    fn temp_source(tag: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mgt_kb_incremental_{}_{}",
            std::process::id(),
            tag
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("doc.md");
        std::fs::write(&path, body).unwrap();
        path
    }

    /// 增量索引：只有被改动的那个文件重建，其余文件连 doc_id 都不动。
    #[test]
    fn apply_changes_rebuilds_only_the_changed_file() {
        let a = temp_source("rebuild-a", "Alpha 文档描述三缓冲渲染与固定时间步长。");
        let b = temp_source("rebuild-b", "Beta 文档描述 goroutine 调度与抢占式并发。");
        let mut kb = KnowledgeBase::new();
        let first = kb.apply_changes(&[a.clone(), b.clone()]);
        assert_eq!(first.rebuilt.len(), 2, "{first:?}");
        assert!(first.removed.is_empty() && first.unchanged.is_empty());
        assert_eq!(kb.document_count(), 2);
        assert_eq!(kb.sources().len(), 2);

        let b_doc = kb
            .sources()
            .iter()
            .find(|s| s.path == b.display().to_string())
            .expect("b.md 应有来源记录")
            .doc_id
            .clone();
        let b_chunks = kb
            .list_documents()
            .iter()
            .find(|(id, _, _)| id == &b_doc)
            .expect("b 的文档应在索引里")
            .2;

        // 只改 a（正文与字节数都变了 → 必须重建）
        std::fs::write(&a, "Alpha 文档改版：新增增量索引与局部重建说明。").unwrap();
        let second = kb.apply_changes(std::slice::from_ref(&a));
        assert_eq!(second.rebuilt, vec![a.display().to_string()], "{second:?}");
        assert!(second.removed.is_empty(), "{second:?}");
        assert!(second.unchanged.is_empty(), "{second:?}");
        assert!(second.removed_doc_ids.is_empty(), "{second:?}");

        // b 未被重建：doc_id 与分块数原样保留
        assert_eq!(kb.document_count(), 2);
        assert!(kb
            .list_documents()
            .iter()
            .any(|(id, _, n)| id == &b_doc && *n == b_chunks));

        // a 的新正文可命中，旧正文不再命中
        assert!(kb.retrieve("三缓冲", 3).is_empty(), "旧正文应随重建移除");
        assert!(!kb.retrieve("增量索引", 3).is_empty(), "新正文应可检索");

        let _ = std::fs::remove_dir_all(a.parent().unwrap());
        let _ = std::fs::remove_dir_all(b.parent().unwrap());
    }

    /// 删文件：索引侧文档与分块被清掉，图谱侧节点与悬空边一并清除。
    #[test]
    fn apply_changes_removes_file_and_purges_graph_nodes_and_edges() {
        use crate::kb::graph::KnowledgeGraph;
        use std::collections::HashSet;

        let a = temp_source("purge-a", "Rust and Kubernetes are cloud native systems.");
        let b = temp_source("purge-b", "Go and Rust are systems languages.");
        let mut kb = KnowledgeBase::new();
        kb.apply_changes(&[a.clone(), b.clone()]);

        // 下游图谱按 doc_id 索引这两篇文档
        let mut graph = KnowledgeGraph::new();
        for src in kb.sources().to_vec() {
            let text = std::fs::read_to_string(&src.path).unwrap();
            graph.index_document(&src.doc_id, &text);
        }
        assert!(graph.entities().contains(&"Kubernetes"));

        std::fs::remove_file(&a).unwrap();
        let report = kb.apply_changes(std::slice::from_ref(&a));
        assert_eq!(report.removed, vec![a.display().to_string()], "{report:?}");
        assert_eq!(report.removed_doc_ids.len(), 1, "{report:?}");
        for doc_id in &report.removed_doc_ids {
            assert!(graph.remove_document(doc_id), "图谱应能摘除 {doc_id}");
        }

        // 只属于 a.md 的节点与边全部消失
        assert!(
            !graph.entities().contains(&"Kubernetes"),
            "{:?}",
            graph.entities()
        );
        assert!(!graph.neighbors("Rust").contains(&"Kubernetes".to_string()));
        // 无悬空边：每条边的两端都仍在实体集里
        let set: HashSet<String> = graph.entities().iter().map(|e| e.to_string()).collect();
        for e in graph.edges() {
            assert!(set.contains(&e.from), "悬空边 from={}", e.from);
            assert!(set.contains(&e.to), "悬空边 to={}", e.to);
        }
        // b.md 的实体保留
        assert!(graph.entities().contains(&"Rust"));
        assert!(graph.entities().contains(&"Go"));

        // 索引侧同步收缩
        assert_eq!(kb.document_count(), 1);
        assert_eq!(kb.sources().len(), 1);
        assert!(kb.retrieve("Kubernetes", 3).is_empty());

        let _ = std::fs::remove_dir_all(b.parent().unwrap());
    }

    /// 文件未被改动时：第二个调用必须是 `unchanged`，不得重复切块或换 doc_id。
    #[test]
    fn apply_changes_reports_unchanged_when_file_untouched() {
        let a = temp_source("unchanged", "内容未变的文件：第二次调用不应重建。");
        let mut kb = KnowledgeBase::new();
        let first = kb.apply_changes(std::slice::from_ref(&a));
        assert_eq!(first.rebuilt.len(), 1, "{first:?}");
        let doc_id = kb.sources()[0].doc_id.clone();
        let chunks = kb.chunk_count();

        let again = kb.apply_changes(std::slice::from_ref(&a));
        assert_eq!(again.unchanged, vec![a.display().to_string()], "{again:?}");
        assert!(again.rebuilt.is_empty(), "{again:?}");
        assert!(again.removed.is_empty(), "{again:?}");
        assert_eq!(kb.sources()[0].doc_id, doc_id, "未改动就不得换 doc_id");
        assert_eq!(kb.document_count(), 1);
        assert_eq!(kb.chunk_count(), chunks);

        let _ = std::fs::remove_dir_all(a.parent().unwrap());
    }
}
