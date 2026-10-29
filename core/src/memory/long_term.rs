//! 长期记忆：向量化存储与分层检索
//!
//! 本地轻量语义嵌入（特征哈希，无外部模型依赖）：
//! - 文档切块 → 特征向量 → SQLite 持久化
//! - 余弦相似度检索
//! - 自动摘要与索引（后台异步维护）
//! - 分层检索：感知 → 短期 → 长期（带置信度与降级）

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::Result;
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};
use tracing::warn;

use crate::memory::embedding::EmbeddingProvider;
use crate::memory::lexical::{self, content_words, shared_content_words};
use crate::memory::scoring;
use crate::memory::sensory::SensoryMemory;
use crate::memory::short_term::ShortTermMemory;

/// 向量维度（特征哈希桶数）
const VEC_DIM: usize = 256;

/// 已触发过「维度不符 → 自动重建」的后端标识集合（`名:维度`）。
/// 进程内每个后端至多自动重建一次，避免每次检索都重建（有界、不阻塞）。
static REINDEXED_BACKENDS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn reindexed_backends() -> &'static Mutex<HashSet<String>> {
    REINDEXED_BACKENDS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// `memory_meta` 里记录向量口径版本的那个键。
const VECTOR_FORMAT_KEY: &str = "vector_format";

/// 当前向量口径：嵌入后端名 + 分词版本，任一变化都让**已落盘的向量**失去可比性。
///
/// 为什么不能只靠「维度不符」来发现：换分词口径（2026-10-29 D3 加 CJK 双字）时维度
/// 仍是 256、后端名仍是 `local-hash`，旧的自动重建**结构上看不到任何变化** ——
/// 于是历史向量会被静默当作同口径数据参与余弦，检索结果悄悄变成噪声。
/// 把口径写进库本身，是让它可被检测的唯一办法。
fn vector_format_stamp() -> String {
    let provider = EmbeddingProvider::global();
    format!(
        "{}:{}",
        provider.name(),
        lexical::TOKENIZER_VERSION
    )
}

/// 记忆条目
#[derive(Debug, Clone)]
pub struct MemoryItem {
    /// 内容
    pub content: String,
    /// 元数据（如会话 ID）
    pub meta: String,
    /// 时间戳
    pub timestamp: String,
    /// 综合分 = `max(实词覆盖率, 余弦)`，见 [`scoring`]
    pub score: f64,
    /// 与查询共享的实词个数（词面证据）
    pub keyword_overlap: usize,
    /// 两路证据都弱：注入时应标注为候选而非事实
    pub weak: bool,
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
    ///
    /// 单字特征 +1.0、拉丁词组（bigram）+0.5。
    /// ⚠️ 口径变化过：旧实现把「拉丁词列表 + 全部 CJK 单字」拼在一起再取 `windows(2)`，
    /// 于是既造出「跨语言边界的假 bigram」，又让中文双字词（"审批""卡片"）在 256 维里
    /// 彼此无关 —— 实测中文改写档最佳余弦中位数只有 0.346。现在双字由
    /// [`lexical::tokens`] 显式给出，bigram 只走 [`lexical::latin_bigrams`]。
    pub fn embed(text: &str) -> Vec<f64> {
        let mut vec = vec![0.0f64; VEC_DIM];
        for t in lexical::tokens(text) {
            let bucket = hash_to(&t, 0) % VEC_DIM;
            vec[bucket] += 1.0;
        }
        for bg in lexical::latin_bigrams(text) {
            let bucket = hash_to(&bg, 1) % VEC_DIM;
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

/// FNV-1a 哈希（带种子）
fn hash_to(s: &str, seed: u64) -> usize {
    let mut h: u64 = 0xcbf29ce484222325 ^ seed.wrapping_mul(0x100000001b3);
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h as usize
}

/// 排序：综合分降序，同分时词面证据多的在前，再同则时间戳、内容升序。
///
/// 后几级 tie-break 是为了**可复现**：这一路的命中会进 System 前缀，顺序不稳定
/// 等于前缀不稳定（前缀缓存失效），而且排查时「同样两个数为什么这次反过来」。
fn sort_items(items: &mut [MemoryItem]) {
    items.sort_by(|a, b| {
        scoring::lexical_first(a.keyword_overlap, a.score, b.keyword_overlap, b.score)
            .then(a.timestamp.cmp(&b.timestamp))
            .then(a.content.cmp(&b.content))
    });
}

/// 截断（规则见 [`scoring`]）：本次查询拿不出词面证据时一条都不给，
/// 否则只保留「不比最好的那条差 40%」的命中，然后取前 `limit` 条。
fn keep_by_relative_score(items: &mut Vec<MemoryItem>, limit: usize) {
    let best = items.first().map(|i| i.score).unwrap_or(0.0);
    items.retain(|i| scoring::keeps(best, i.score));
    items.truncate(limit);
}

/// 长期记忆
pub struct LongTermMemory {
    db_path: PathBuf,
    db: Option<Connection>,
    /// 进程内向量索引（惰性加载）：`(PRAGMA data_version, 行向量)`。
    /// 消除 search 每次全表扫描 + 逐行 split/parse 256 维向量的重复开销；
    /// 本实例 store 时增量追加，其他连接提交后 data_version 变化触发重建。
    index: RefCell<Option<(i64, Vec<IndexRow>)>>,
}

/// 索引行：与 long_term_memory 表一一对应（向量已解析为 f64）
struct IndexRow {
    content: String,
    meta: String,
    timestamp: String,
    vec: Vec<f64>,
    /// 内容的实词集合，加载索引时算一次。
    ///
    /// 词面那一路证据需要它；不在每次检索里重算 —— 检索是全表扫描，
    /// 逐查询重新分词会把 O(库大小) 的常数再乘一遍。
    words: Vec<String>,
}

impl LongTermMemory {
    /// 创建长期记忆（不初始化 DB）
    pub fn new(path: &Path) -> Self {
        Self {
            db_path: path.to_path_buf(),
            db: None,
            index: RefCell::new(None),
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
            CREATE INDEX IF NOT EXISTS idx_ltm_time ON long_term_memory(timestamp);
            CREATE TABLE IF NOT EXISTS memory_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )?;
        self.db = Some(db);
        // 口径戳记：分词/嵌入后端变化后，**同维度**的旧向量无法靠「维度不符」发现，
        // 只能靠库里的这一行。失配就地重建一次（见 [`vector_format_stamp`]）。
        self.align_vector_format()?;
        Ok(())
    }

    /// 向量口径与库中戳记不一致时，用当前后端重写全部向量并刷新戳记。
    ///
    /// 全新库（空表）也会走一次：`reindex_all` 更新 0 行，代价是一次扫描，
    /// 换来的是「库里没有一行没被核对过口径」——不必为空库/旧库分两种逻辑。
    fn align_vector_format(&self) -> Result<()> {
        let want = vector_format_stamp();
        let stored: Option<String> = self
            .db()?
            .query_row(
                "SELECT value FROM memory_meta WHERE key = ?1",
                rusqlite::params![VECTOR_FORMAT_KEY],
                |r| r.get(0),
            )
            .optional()?;
        if stored.as_deref() == Some(want.as_str()) {
            return Ok(());
        }
        let updated = self.reindex_all()?;
        if updated > 0 {
            warn!(
                updated,
                stored = stored.as_deref().unwrap_or("(缺戳记)"),
                current = %want,
                "长期记忆：向量口径已变化，按当前后端重建全部向量"
            );
        }
        self.db()?.execute(
            "INSERT INTO memory_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![VECTOR_FORMAT_KEY, want],
        )?;
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
        let now = Utc::now().to_rfc3339();
        db.execute(
            "INSERT INTO long_term_memory (content, meta, vector, timestamp) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![content, meta, vector, now],
        )?;
        // 索引已加载时增量追加（复用刚算出的向量，零额外开销）；
        // 未加载则无需动作：下次 search 看到 data_version 未变时会走全量加载路径
        // 补齐本行（本连接自己的写不影响自身 PRAGMA data_version 视图）。
        if let Ok(mut idx) = self.index.try_borrow_mut() {
            if let Some((_, rows)) = idx.as_mut() {
                rows.push(IndexRow {
                    content: content.to_string(),
                    meta: meta.to_string(),
                    timestamp: now,
                    words: content_words(content),
                    vec,
                });
            }
        }
        Ok(())
    }

    /// 语义检索：幅度按 [`scoring::fuse`]（零词面重合在默认后端下直接记 0），次序按
    /// [`scoring::lexical_first`]（词面档在前），再按 [`scoring::keeps`] 相对截断丢弃
    /// （规则见 [`scoring`]）。
    ///
    /// ⚠️ 旧实现是「余弦 ≥ 0.35 的绝对门」。这一路是**默认开启**的 prompt 注入路，
    /// 所以绝对门的饿死效应直接对用户可见：实测中文改写档最佳余弦中位数 0.346
    /// 恰好压在门下 —— 那一整档什么都不注入，看起来就像「记忆功能不存在」。
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryItem>> {
        let provider = EmbeddingProvider::global();
        let qvec = provider.embed(query);
        let query_words = content_words(query);
        // 索引失效检测：data_version 变化（其他连接提交了写）→ 全量重建。
        // 本连接自己的 store 已就地增量维护，无需重建。
        let version = self.db_data_version().unwrap_or(-1);
        let need_load = match self.index.borrow().as_ref() {
            Some((v, _)) => *v != version,
            None => true,
        };
        if need_load {
            let (rows, skipped) = self.load_index_rows()?;
            // B2：存在维度不符的旧向量（如切换嵌入后端后）→ 自动重建一次并告警，
            // 否则这些历史记忆会被静默跳过、检索结果为空且用户无任何信号。
            // （分词口径变化走 init 的戳记，不依赖这里。）
            let rows = if skipped > 0 && self.maybe_reindex_dim_mismatch(skipped)? {
                self.load_index_rows()?.0
            } else {
                rows
            };
            *self.index.borrow_mut() = Some((version, rows));
        }
        let index = self.index.borrow();
        let rows = &index.as_ref().expect("索引刚加载").1;

        let mut scored: Vec<MemoryItem> = Vec::with_capacity(rows.len());
        for row in rows {
            // 维度不符的旧向量（如切换模型后端前写入）在加载时已跳过
            let cosine = provider.cosine(&qvec, &row.vec);
            let overlap = shared_content_words(&query_words, &row.words);
            let score = scoring::fuse(overlap, scoring::coverage(query_words.len(), overlap), cosine);
            scored.push(MemoryItem {
                content: row.content.clone(),
                meta: row.meta.clone(),
                timestamp: row.timestamp.clone(),
                score,
                keyword_overlap: overlap,
                weak: scoring::is_weak(score),
            });
        }
        sort_items(&mut scored);
        keep_by_relative_score(&mut scored, limit);
        Ok(scored)
    }

    /// 当前连接的 PRAGMA data_version：其他连接提交写时递增（本连接自己的写不增）。
    fn db_data_version(&self) -> Result<i64> {
        let db = self.db()?;
        Ok(db.query_row("PRAGMA data_version", [], |r| r.get(0))?)
    }

    /// 从 DB 全量加载并解析索引行（维度不符的旧向量跳过，与旧逐行检索逻辑一致）。
    ///
    /// 返回 `(可用行, 因维度不符被跳过的行数)`；后者用于触发 B2 的自动重建。
    fn load_index_rows(&self) -> Result<(Vec<IndexRow>, usize)> {
        let db = self.db()?;
        let dim = EmbeddingProvider::global().dim();
        let mut stmt =
            db.prepare("SELECT content, meta, vector, timestamp FROM long_term_memory ORDER BY id")?;
        let iter = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut rows = Vec::new();
        let mut skipped = 0usize;
        for item in iter.flatten() {
            let vec: Vec<f64> = item
                .2
                .split(',')
                .filter_map(|s| s.parse::<f64>().ok())
                .collect();
            if vec.len() != dim {
                skipped += 1;
                continue;
            }
            rows.push(IndexRow {
                words: content_words(&item.0),
                content: item.0,
                meta: item.1,
                timestamp: item.3,
                vec,
            });
        }
        Ok((rows, skipped))
    }

    /// 维度不符时的自动重建（每「后端名:维度」进程内至多一次）。
    ///
    /// 返回是否实际执行了重建。用全局集合做防抖，避免每次检索都重建；
    /// 重建是惰性触发（仅在检索命中失配时），不阻塞启动。
    fn maybe_reindex_dim_mismatch(&self, skipped: usize) -> Result<bool> {
        let provider = EmbeddingProvider::global();
        let key = format!("{}:{}", provider.name(), provider.dim());
        let first = {
            let mut done = reindexed_backends()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            done.insert(key)
        };
        if !first {
            return Ok(false);
        }
        warn!(
            skipped,
            backend = provider.name(),
            dim = provider.dim(),
            "长期记忆：检测到旧向量维度与当前嵌入后端不符，自动重建向量索引（每个后端仅一次）"
        );
        let updated = self.reindex_all()?;
        warn!(updated, "长期记忆：向量索引自动重建完成");
        Ok(true)
    }

    /// 用当前嵌入后端重建全部向量（模型切换后调用，使旧维度向量对齐新后端）
    ///
    /// 返回重建的条目数。若某条目内容无法嵌入会跳过该条（理论上不会失败）。
    pub fn reindex_all(&self) -> Result<usize> {
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
        // 向量全部重写：进程内索引作废，下次 search 按新维度全量重建
        *self.index.borrow_mut() = None;
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
                    // 0.4 是历史占位常量，不是测出来的相关性 —— 如实标成弱证据，
                    // 别让降级结果冒充一个有分数的语义命中。
                    score: 0.4,
                    keyword_overlap: 0,
                    weak: true,
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
    fn test_auto_reindex_on_dimension_mismatch() {
        // B2 回归：切换嵌入后端后历史向量维度不符，旧行为静默跳过 → 检索为空。
        // 现应在检索时检测到失配并自动重建（每后端一次），恢复可检索性。
        let path = temp_db("auto_reindex");
        let _ = std::fs::remove_file(&path);
        let mut mem = LongTermMemory::new(&path);
        mem.init().unwrap();
        mem.store("如何配置 tokio 异步运行时", "s1").unwrap();

        // 人为把持久化向量改成错误维度（模拟换后端前的历史向量）。
        let wrong: String = (0..4)
            .map(|i| (i as f64).to_string())
            .collect::<Vec<_>>()
            .join(",");
        mem.db
            .as_ref()
            .unwrap()
            .execute(
                "UPDATE long_term_memory SET vector = ?1",
                rusqlite::params![wrong],
            )
            .unwrap();

        let hits = mem.search("tokio 异步配置", 5).unwrap();
        assert!(
            hits.iter().any(|h| h.content.contains("tokio")),
            "维度不符时应自动重建索引并恢复检索: {hits:?}"
        );
        // 重建后持久化向量维度应与当前后端一致。
        let dim_now: String = mem
            .db
            .as_ref()
            .unwrap()
            .query_row("SELECT vector FROM long_term_memory LIMIT 1", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap();
        assert_eq!(dim_now.split(',').count(), EmbeddingProvider::global().dim());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_hash_deterministic() {
        assert_eq!(hash_to("tokio", 0), hash_to("tokio", 0));
        assert_ne!(hash_to("tokio", 0), hash_to("tokio", 1));
    }

    /// 分词口径归一后，本模块不再自带 tokenize：这里是「嵌入吃到什么词元」的钉子，
    /// 口径本身在 `memory::lexical` 里测。
    #[test]
    fn test_tokenize_cjk() {
        let tokens = lexical::tokens("异步运行 Rust 代码");
        assert!(
            tokens.iter().any(|t| t == "异步"),
            "双字组合必须进主通道（旧口径只有单字）: {tokens:?}"
        );
        assert!(tokens.iter().any(|t| t == "rust"));
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

    #[test]
    fn test_index_incremental_after_store() {
        // 索引加载后继续 store：必须增量可见（不依赖 data_version 重建）
        let path = temp_db("index_incr");
        let _ = std::fs::remove_file(&path);
        let mut mem = LongTermMemory::new(&path);
        mem.init().unwrap();
        mem.store("Rust tokio timeout 网络超时防护", "s1").unwrap();
        let hits = mem.search("tokio timeout", 5).unwrap(); // 触发索引加载
        assert!(!hits.is_empty());
        mem.store("索引缓存数据版本增量", "s2").unwrap();
        let hits = mem.search("数据版本", 5).unwrap();
        assert!(
            hits.iter().any(|h| h.content.contains("索引缓存")),
            "索引加载后的 store 应立即可检索"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_index_sees_writes_from_other_instance() {
        // 跨实例新鲜度：另一连接提交写 → data_version 变化 → 本实例索引重建
        let path = temp_db("index_cross");
        let _ = std::fs::remove_file(&path);
        let mut a = LongTermMemory::new(&path);
        a.init().unwrap();
        a.store("Rust tokio timeout 网络超时防护", "s1").unwrap();
        assert!(!a.search("tokio timeout", 5).unwrap().is_empty());
        let mut b = LongTermMemory::new(&path);
        b.init().unwrap();
        b.store("并发索引缓存数据版本", "s2").unwrap();
        let hits = a.search("数据版本", 5).unwrap();
        assert!(
            hits.iter().any(|h| h.content.contains("并发索引缓存")),
            "其他实例的写入应经 data_version 失效检测后可见"
        );
        let _ = std::fs::remove_file(&path);
    }
}
