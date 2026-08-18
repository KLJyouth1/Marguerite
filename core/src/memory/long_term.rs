//! 长期记忆：向量化存储与分层检索
//!
//! 本地轻量语义嵌入（特征哈希，无外部模型依赖）：
//! - 文档切块 → 特征向量 → SQLite 持久化
//! - 余弦相似度检索
//! - 自动摘要与索引（后台异步维护）
//! - 分层检索：感知 → 短期 → 长期（带置信度与降级）

use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;

use crate::memory::embedding::EmbeddingProvider;
use crate::memory::sensory::SensoryMemory;
use crate::memory::short_term::ShortTermMemory;

/// 向量维度（特征哈希桶数）
const VEC_DIM: usize = 256;
/// 余弦相似度阈值（低于视为无相关）
const SIM_THRESHOLD: f64 = 0.35;

/// 记忆条目
#[derive(Debug, Clone)]
pub struct MemoryItem {
    /// 内容
    pub content: String,
    /// 元数据（如会话 ID）
    pub meta: String,
    /// 时间戳
    pub timestamp: String,
    /// 相似度分数
    pub score: f64,
}

/// 分层检索结果
#[derive(Debug, Clone)]
pub struct RetrievalResult {
    /// 命中条目
    pub items: Vec<MemoryItem>,
    /// 最高置信度
    pub confidence: f64,
    /// 是否降级（高层记忆无命中时降级到低层）
    pub degraded: bool,
}

/// 本地特征哈希嵌入
///
/// 将文本映射为固定维稀疏向量。仅需确定性哈希，零依赖、毫秒级。
pub struct LocalEmbedding;

impl LocalEmbedding {
    /// 向量维度（特征哈希桶数）
    pub fn dim() -> usize {
        VEC_DIM
    }

    /// 计算文本的特征向量（L2 归一化）
    pub fn embed(text: &str) -> Vec<f64> {
        let mut vec = vec![0.0f64; VEC_DIM];
        // 词袋特征：按词与双词哈希到桶
        let tokens = tokenize(text);
        for t in &tokens {
            let bucket = hash_to(t, 0) % VEC_DIM;
            vec[bucket] += 1.0;
        }
        for w in tokens.windows(2) {
            let bigram = format!("{} {}", w[0], w[1]);
            let bucket = hash_to(&bigram, 1) % VEC_DIM;
            vec[bucket] += 0.5;
        }
        // L2 归一化
        let norm = vec.iter().map(|v| v * v).sum::<f64>().sqrt();
        if norm > 0.0 {
            for v in &mut vec {
                *v /= norm;
            }
        }
        vec
    }

    /// 余弦相似度
    pub fn cosine(a: &[f64], b: &[f64]) -> f64 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }
}

/// 简单分词：字母数字序列（对中文按字符粒度切分）
fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    // 拉丁词：按非字母数字分隔（含空格）
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        let word = word.trim().to_lowercase();
        if !word.is_empty() && word.is_ascii() {
            tokens.push(word);
        }
    }
    // 中日韩字符按单字切分（保证中文召回）
    for ch in text.chars() {
        if ch.is_alphabetic() && !ch.is_ascii() {
            tokens.push(ch.to_string());
        }
    }
    tokens
}

/// FNV-1a 哈希（带种子）
fn hash_to(s: &str, seed: u64) -> usize {
    let mut h: u64 = 0xcbf29ce484222325 ^ seed.wrapping_mul(0x100000001b3);
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h as usize
}

/// 长期记忆
pub struct LongTermMemory {
    db_path: PathBuf,
    db: Option<Connection>,
}

impl LongTermMemory {
    /// 创建长期记忆（不初始化 DB）
    pub fn new(path: &Path) -> Self {
        Self {
            db_path: path.to_path_buf(),
            db: None,
        }
    }

    /// 初始化数据库
    pub fn init(&mut self) -> Result<()> {
        let db = Connection::open(&self.db_path)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS long_term_memory (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content TEXT NOT NULL,
                meta TEXT DEFAULT '',
                vector TEXT NOT NULL,
                timestamp TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_ltm_time ON long_term_memory(timestamp);",
        )?;
        self.db = Some(db);
        Ok(())
    }

    fn db(&self) -> Result<&Connection> {
        self.db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("LongTermMemory not initialized"))
    }

    /// 存储一条记忆（自动嵌入向量，使用当前注入的嵌入后端）
    pub fn store(&mut self, content: &str, meta: &str) -> Result<()> {
        let db = self.db()?;
        let vec = EmbeddingProvider::global().embed(content);
        let vector = vec
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",");
        db.execute(
            "INSERT INTO long_term_memory (content, meta, vector, timestamp) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![content, meta, vector, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// 语义检索：返回相似度排序的记忆
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryItem>> {
        let db = self.db()?;
        let provider = EmbeddingProvider::global();
        let qvec = provider.embed(query);
        let dim = provider.dim();
        let mut stmt =
            db.prepare("SELECT content, meta, vector, timestamp FROM long_term_memory")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;

        let mut scored = Vec::new();
        for row in rows.flatten() {
            let vec: Vec<f64> = row
                .2
                .split(',')
                .filter_map(|s| s.parse::<f64>().ok())
                .collect();
            // 维度不符的旧向量（如切换模型后端前写入）跳过，待 reindex_all 重建
            if vec.len() != dim {
                continue;
            }
            let score = provider.cosine(&qvec, &vec);
            if score >= SIM_THRESHOLD {
                scored.push(MemoryItem {
                    content: row.0,
                    meta: row.1,
                    timestamp: row.3,
                    score,
                });
            }
        }
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);
        Ok(scored)
    }

    /// 用当前嵌入后端重建全部向量（模型切换后调用，使旧维度向量对齐新后端）
    ///
    /// 返回重建的条目数。若某条目内容无法嵌入会跳过该条（理论上不会失败）。
    pub fn reindex_all(&mut self) -> Result<usize> {
        let db = self.db()?;
        let rows = db
            .prepare("SELECT id, content FROM long_term_memory")?
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let provider = EmbeddingProvider::global();
        let mut updated = 0usize;
        for (id, content) in rows {
            let vector = provider
                .embed(&content)
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(",");
            db.execute(
                "UPDATE long_term_memory SET vector = ?1 WHERE id = ?2",
                rusqlite::params![vector, id],
            )?;
            updated += 1;
        }
        Ok(updated)
    }

    /// 自动摘要索引：将感知记忆内容周期性地提升为长期记忆
    ///
    /// 调用方按需周期调用（如每 N 条消息后）。已存在的完全相同内容会跳过
    /// （去重），避免重复调用导致长期记忆膨胀。
    pub fn consolidate(&mut self, sensory: &SensoryMemory) -> Result<usize> {
        let messages = sensory.get_all_messages();
        let mut stored = 0;
        for msg in messages {
            let content = render_for_memory(msg);
            if content.chars().count() < 20 {
                continue; // 过短内容不值得长期记忆
            }
            // 去重：相同内容已存在则跳过（consolidate 会被周期调用）
            let dup = self
                .search(&content, 1)
                .map(|hits| hits.iter().any(|h| h.content == content))
                .unwrap_or(false);
            if dup {
                continue;
            }
            self.store(&content, "consolidated")?;
            stored += 1;
        }
        Ok(stored)
    }

    /// 记忆条目总数
    pub fn count(&self) -> Result<usize> {
        let db = self.db()?;
        let n: i64 = db.query_row("SELECT COUNT(*) FROM long_term_memory", [], |r| r.get(0))?;
        Ok(n as usize)
    }
}

/// 分层记忆检索器
///
/// 检索顺序：长期（语义）→ 短期（FTS）→ 感知（当前会话）。
/// 每层返回带置信度结果，低层命中时标记降级。
pub struct HierarchicalMemory {
    pub long_term: LongTermMemory,
    pub short_term: ShortTermMemory,
    pub sensory: SensoryMemory,
}

impl HierarchicalMemory {
    pub fn new(
        long_term: LongTermMemory,
        short_term: ShortTermMemory,
        sensory: SensoryMemory,
    ) -> Self {
        Self {
            long_term,
            short_term,
            sensory,
        }
    }

    /// 分层检索
    pub fn retrieve(&self, query: &str, limit: usize) -> Result<RetrievalResult> {
        // 第一层：长期语义记忆
        let items: Vec<MemoryItem> = self.long_term.search(query, limit).unwrap_or_default();
        let confidence = items.first().map(|i| i.score).unwrap_or(0.0);
        let empty = items.is_empty();

        if items.len() >= limit {
            return Ok(RetrievalResult {
                items,
                confidence,
                degraded: false,
            });
        }

        // 第二层：短期 FTS 记忆
        let mut items = items;
        if let Ok(summaries) = self.short_term.search(query, limit - items.len()) {
            for s in summaries {
                items.push(MemoryItem {
                    content: s.summary,
                    meta: format!("session:{}", s.session_id),
                    timestamp: s.updated_at,
                    score: 0.4,
                });
            }
        }

        Ok(RetrievalResult {
            items,
            confidence: if empty { 0.0 } else { confidence.max(0.4) },
            degraded: empty,
        })
    }
}

/// 渲染消息为记忆内容
fn render_for_memory(msg: &crate::types::Message) -> String {
    match msg {
        crate::types::Message::User { content, .. }
        | crate::types::Message::Assistant { content, .. }
        | crate::types::Message::System { content, .. } => content.clone(),
        crate::types::Message::Tool { name, output, .. } => {
            format!("[tool:{name}] {output}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("marguerite_ltm_{name}_{}.db", std::process::id()))
    }

    #[test]
    fn test_embedding_cosine() {
        let a = LocalEmbedding::embed("rust async runtime tokio");
        let b = LocalEmbedding::embed("tokio async runtime rust");
        let c = LocalEmbedding::embed("pizza recipe tomato cheese");
        assert!(LocalEmbedding::cosine(&a, &b) > 0.7);
        assert!(LocalEmbedding::cosine(&a, &c) < 0.5);
    }

    #[test]
    fn test_store_and_search() {
        let path = temp_db("search");
        let _ = std::fs::remove_file(&path);
        let mut mem = LongTermMemory::new(&path);
        mem.init().unwrap();
        mem.store("如何配置 tokio 异步运行时", "session1").unwrap();
        mem.store("番茄披萨的烘焙方法", "session2").unwrap();

        let results = mem.search("tokio 异步配置", 5).unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("tokio"));
        assert!(results[0].score > 0.5);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_search_no_match() {
        let path = temp_db("nomatch");
        let _ = std::fs::remove_file(&path);
        let mut mem = LongTermMemory::new(&path);
        mem.init().unwrap();
        mem.store("完全无关的内容 xyz123", "s").unwrap();
        let results = mem.search("异步编程", 5).unwrap();
        assert!(results.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_reindex_all_preserves_searchable_results() {
        let path = temp_db("reindex");
        let _ = std::fs::remove_file(&path);
        let mut mem = LongTermMemory::new(&path);
        mem.init().unwrap();
        mem.store("如何配置 tokio 异步运行时", "session1").unwrap();
        mem.store("番茄披萨的烘焙方法", "session2").unwrap();

        // 用当前默认后端重建全部向量后，检索仍应命中
        let updated = mem.reindex_all().unwrap();
        assert_eq!(updated, 2);

        let results = mem.search("tokio 异步配置", 5).unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("tokio"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_hash_deterministic() {
        assert_eq!(hash_to("tokio", 0), hash_to("tokio", 0));
        assert_ne!(hash_to("tokio", 0), hash_to("tokio", 1));
    }

    #[test]
    fn test_tokenize_cjk() {
        let tokens = tokenize("异步运行 Rust 代码");
        assert!(tokens.iter().any(|t| t == "异步") || tokens.iter().any(|t| t == "rust"));
        assert!(tokens.iter().any(|t| t == "代"));
    }

    #[test]
    fn test_count() {
        let path = temp_db("count");
        let _ = std::fs::remove_file(&path);
        let mut mem = LongTermMemory::new(&path);
        mem.init().unwrap();
        mem.store("第一条记忆内容", "m").unwrap();
        mem.store("第二条记忆内容", "m").unwrap();
        assert_eq!(mem.count().unwrap(), 2);
        let _ = std::fs::remove_file(&path);
    }
}
