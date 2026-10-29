//! 短期记忆：跨会话的全文搜索
//!
//! 使用 SQLite FTS5 实现跨会话的全文搜索能力。
//! 借鉴 混合检索引擎 sessionDB 设计。
//!
//! Schema 对齐说明（R6 单一权威来源）：
//! 本表的权威 schema 定义在 `persistence/session.rs` 的 `sessions` 表。
//! 此处使用 `short_term_sessions` 表名（而非 `sessions`）以避免
//! 跨数据库文件的同名表 schema 漂移。列 `state` / `tokens_used`
//! 与权威定义保持一致，供 FTS5 搜索结果过滤使用。

use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};

use crate::memory::lexical;
use crate::persistence::config::Config;
use crate::types::Message;

/// 会话摘要
#[derive(Debug, Clone)]
pub struct SessionSummary {
    /// 会话 ID
    pub session_id: String,
    /// 摘要内容
    pub summary: String,
    /// 消息数量
    pub message_count: usize,
    /// 创建时间
    pub created_at: String,
    /// 更新时间
    pub updated_at: String,
    /// FTS5 搜索排名
    pub rank: Option<f64>,
}

/// 默认留存上限（配置项 `memory.short_term_max_sessions` 缺席时的回落值）。
///
/// 旧值 **50** 意味着每轮沉淀占一个槽位、第 51 场会话起最早的会话连内容带索引一起消失 ——
/// 「跑两周就把自己检索不回来了」，而且消失是静默的（检索只会回「没有」）。
pub const DEFAULT_MAX_SESSIONS: usize = 2000;

/// FTS5 的运算符词元。
///
/// 它们出现在**查询**里时不是数据而是语法。⚠️ 实测 SQLite 对此**一个错误都不抛**，
/// 只是静默改语义（未加引号的 `session_id:s1` 会当成**列选择器**生效），所以调用方看到的
/// 是一次「正常但不符合预期」的检索，谁也不会怀疑查询语法 —— 见
/// `user_text_never_reaches_the_fts_query_parser`。逐词元加引号后这些词本身已经退化成普通
/// 词元，仍然剔除它们，是因为它们作为**数据**也只贡献噪声：`and/or/not` 本就是停用词
/// （`content_words` 已滤），`near` 则是这四个里唯一不是停用词的那个语法词，留着它等于
/// 允许一次「文档里恰好写着 near」的假命中。
const FTS_OPERATOR_WORDS: &[&str] = &["and", "or", "not", "near"];

/// 单条查询最多编译多少个词元。
///
/// 表达式的项数受 SQLite 约束，超限是**报错**而不是降级；截断只会放宽条件
/// （OR 链少几个尾部词元），不会静默改语义。
const MAX_QUERY_TERMS: usize = 32;

/// 索引用的归一化副本：写入的是**空格分隔的词元流**，与查询侧同一个分词器。
///
/// FTS5 的 unicode61 把一整段没有空格的中文当成**一个** token，所以旧 schema 下
/// 中文摘要对中文查询 0 命中（实测 fts5 hit@5 = **0.0%**）。把分词结果落成文本再索引，
/// 中文才第一次进得了全文检索。原始 `summary` 列一字不改，展示字段不受影响。
fn normalize_for_index(text: &str) -> String {
    lexical::tokens(text).join(" ")
}

/// 丢掉「非 ascii 单字」词元：中文双字以上才带区分度，单字（技/术/方）在 bm25 里是噪声源。
/// 拉丁单字符词元（a、i）本来就是真实词，留着。
pub(crate) fn drop_cjk_unigrams(terms: &[String]) -> Vec<String> {
    terms
        .iter()
        .filter(|t| t.is_ascii() || t.chars().count() > 1)
        .cloned()
        .collect()
}

/// 剔除运算符词、去重、截断到 `MAX_QUERY_TERMS`。
///
/// 两条词元管道（[`query_terms`] / [`content_terms`]）共用同一次过滤，
/// 「测的」与「跑的」因此不可能各走各的口径。
fn compile(ts: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    ts.into_iter()
        .filter(|t| !FTS_OPERATOR_WORDS.contains(&t.as_str()))
        .filter(|t| seen.insert(t.clone()))
        .take(MAX_QUERY_TERMS)
        .collect()
}

/// **全词元**口径：不过停用词，只剔运算符/去重/截断。
///
/// 单独列出来是为了让探针能把它和实词口径并排测 —— 实测它做 AND 全库 0 命中、
/// 做 OR 精度只有 0.409（停用词碰撞），所以**不是**生产口径。
pub(crate) fn query_terms(query: &str) -> Vec<String> {
    compile(lexical::tokens(query))
}

/// **实词**口径：去停用词、去纯数字、去单拉丁字母，可选是否保留中文单字。
pub(crate) fn content_terms(query: &str, keep_cjk_unigrams: bool) -> Vec<String> {
    let words = lexical::content_words(query);
    compile(if keep_cjk_unigrams {
        words
    } else {
        drop_cjk_unigrams(&words)
    })
}

/// 词元 → FTS5 表达式：逐词元加引号后用 `join` 连接。
///
/// 不引号时 `and/or/not/near`、`:`、`*`、`"` 会被 FTS5 当语法解析，而 SQLite 对语义被
/// 静默改写**一个错误都不抛**（见 [`FTS_OPERATOR_WORDS`] 上方注释）。逐词元加引号后它们
/// 一律只是数据。
/// ⚠️ 词元里永远不会有裸 `"` —— `tokens()` 只产出拉丁字母数字与非 ascii 字母。
pub(crate) fn to_expression(terms: &[String], join: &str) -> String {
    terms
        .iter()
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(join)
}

/// 把用户文本编译成 FTS5 表达式；**没有词元时返回 `None`**（调用方如实返回空结果，
/// 绝不退化成「匹配全部」）。
///
/// # 口径（四个自由度，全部由实测选定，见 `docs/prd/2026-10-29-memory-substrate-retrieval-fix.md` §D4）
///
/// 夹具 75 条语料 / 45 条查询 / K=5 的并排测量（`memory::recall_probe` 的 FTS 档常驻复现，
/// 「探针档位」那一列就是报告 `d4_fts_join_modes.txt` 里的段名；只有 `production` 那行
/// 由 `d4_fts_join_mode_measurement` 末尾的棘轮断言钉住，其余四行是参照系）：
///
/// | 口径 | 探针档位 | hit@5 | 饿死 | 注入精度 p |
/// |---|---|---|---|---|
/// | **AND + 全词元**（首版拍的） | `and` | **0.0%** | 45 | 0.000 |
/// | OR + 全词元 | `or` | 88.9% | 0 | 0.409 |
/// | AND 不中再 OR | `and_then_or` | 88.9% | 0 | 0.409（与 `or` **逐档相同**：AND 从不命中，白跑一次查询） |
/// | OR + 实词（含中文单字） | `or_content` | 77.8% | 5 | 0.547 |
/// | **OR + 实词 + 去中文单字**（本函数） | `production` | 75.6% | 6 | **0.663** |
/// | 参照：`loose` 关键词基线 | — | 77.8% | 5 | 0.557 |
/// | 参照：`tiered` 分层通路 | — | 77.8% | 5 | 0.655 |
///
/// - **连接符只能是 OR。** AND 在**中英文都是 0 命中**：它要求每个词元都出现，而查询是疑问句
///   （`why does the …`）、语料是陈述句，`why/does` 根本不在文档里。中文那边同理 ——
///   双字分词让「技术方案设计」的 AND 链要求 `案设` 相邻出现，多带一个词就全丢。
///   首版只在真实库上探过 `配置`、`技术方案` 这类**两词查询**，正好落在 AND 还能命中的区间里，
///   于是把 0/45 的口径当成了可用口径 —— 这是「单点测量给结论」的代价。
/// - **词元只取实词。** 全词元 OR 的「命中」里有一批是停用词碰撞：`hard·paraphrase` 档显示
///   62.5% 命中、精度 **0.125**（每投十行错八行），而那档的查询与同族语料**零实词重合**（夹具自证）。
/// - **中文单字不进查询。** 「本地全绿但干净构建其实是坏的」这条与 `zh-gate` 族只共享
///   `坏`、`全` 两个单字，去掉后它如实饿死 —— 与 P0 从 `tiered` 拆掉「凑数注入」是同一条原则。
///   代价是全库 hit@5 少 2.2 个百分点，且少掉的正是那条假命中。
/// - **只有摘要参与匹配。** 虚表把 `session_id` 声明成 `UNINDEXED`：id 形如
///   `agent:main:webchat:…`，一旦进索引，`agent`/`main` 这类词就能靠 **id** 命中整表，
///   查询与摘要写的是什么毫无关系。旧 AND 口径把它藏住了（要求所有词元同现），
///   换 OR 之后 `session_id:s1` 立刻回 `["s1"]` —— 由测试
///   `session_id_is_not_a_searchable_column` 与 `init_rebuilds_an_fts_table_that_indexes_the_session_id`
///   各钉一次。
///
/// 回落分支（实词被滤空时退回全词元）留给「单字查询」：`坏` 这类一条实词都没有，
/// 但不该因此什么都搜不到。
pub(crate) fn match_expression(query: &str) -> Option<String> {
    let mut terms = content_terms(query, false);
    if terms.is_empty() {
        terms = query_terms(query);
    }
    (!terms.is_empty()).then(|| to_expression(&terms, " OR "))
}

/// 表里是否有某列。对 FTS5 **虚表同样有效**（实测 `pragma_table_info` 会列出虚表声明的列），
/// 所以虚表的列名可以直接当作「索引的是哪一列」的判据。
fn column_exists(db: &Connection, table: &str, column: &str) -> Result<bool> {
    Ok(db
        .prepare("SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2 LIMIT 1")?
        .exists(rusqlite::params![table, column])?)
}

/// 留存上限（配置项 `memory.short_term_max_sessions`）。
///
/// 放在**构造点**而不是 `new()` 里：单元测试因此不会读到真实用户配置
/// （AGENTS.md §4：配置目录与凭据目录同源，测试碰它就是未定义行为）。
pub fn max_sessions_from_config() -> usize {
    Config::load()
        .map(|c| c.memory.short_term_max_sessions)
        .unwrap_or(DEFAULT_MAX_SESSIONS)
        .max(1)
}

/// 短期记忆
///
/// 跨会话的全文搜索，使用 SQLite FTS5 实现。
/// 借鉴 混合检索引擎 sessionDB 设计。
pub struct ShortTermMemory {
    db_path: PathBuf,
    db: Option<Connection>,
    max_sessions: usize,
}

impl ShortTermMemory {
    /// 创建短期记忆（默认留存 `DEFAULT_MAX_SESSIONS`；生产入口用
    /// [`with_max_sessions`] 注入配置值）
    pub fn new(path: &Path) -> Self {
        Self {
            db_path: path.to_path_buf(),
            db: None,
            max_sessions: DEFAULT_MAX_SESSIONS,
        }
    }

    /// 覆盖留存上限（配置项 `memory.short_term_max_sessions` 的注入点）。
    pub fn with_max_sessions(mut self, max_sessions: usize) -> Self {
        self.max_sessions = max_sessions.max(1);
        self
    }

    /// 初始化数据库和 FTS5 索引
    ///
    /// 三段迁移必须都在这里，且**幂等**：磁盘上躺着的是真实用户机器上的旧库
    /// （`sessions` 表 / 无 `search_text` 列 / FTS 索引的是原始 `summary`）。
    pub fn init(&mut self) -> Result<()> {
        let db = Connection::open(&self.db_path)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS short_term_sessions (
                session_id TEXT PRIMARY KEY,
                summary TEXT NOT NULL,
                state TEXT NOT NULL DEFAULT 'Ready',
                message_count INTEGER DEFAULT 0,
                tokens_used INTEGER DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                search_text TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS short_term_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )?;

        // 迁移 ①：旧库的 `sessions` 表 → `short_term_sessions`（schema 漂移修复，幂等）。
        //
        // ⚠️ 列必须**逐个探测**：真实用户机器上的 `sessions` 比 `persistence/session.rs`
        // 的权威形状还要老，只有 5 列（没有 `state`，也没有 `tokens_used`）。照抄列名写死
        // SELECT 就是 `no such column: state` —— 而 init() 一报错，`gateway/chat.rs` 的
        // `memory_stack` 直接 `return None`，整个短期记忆功能**静默消失**（实测踩在真实库上）。
        // 这里**不再**往 FTS 里灌数据：索引列已经换成 `search_text`，直接 `INSERT … summary`
        // 会指着一个不存在的虚表列；内容由迁移 ②③ 的回填 + rebuild 负责。
        let has_old_table: bool = db
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='sessions'")
            .map(|mut stmt| stmt.exists([]).unwrap_or(false))
            .unwrap_or(false);
        let old_row_is_a_memory = column_exists(&db, "sessions", "session_id")?
            && column_exists(&db, "sessions", "summary")?;
        if has_old_table && old_row_is_a_memory {
            let state = if column_exists(&db, "sessions", "state")? {
                "COALESCE(state, 'Ready')"
            } else {
                "'Ready'"
            };
            let message_count = if column_exists(&db, "sessions", "message_count")? {
                "COALESCE(message_count, 0)"
            } else {
                "0"
            };
            let tokens_used = if column_exists(&db, "sessions", "tokens_used")? {
                "COALESCE(tokens_used, 0)"
            } else {
                "0"
            };
            let created_at = if column_exists(&db, "sessions", "created_at")? {
                "created_at"
            } else {
                "''"
            };
            let updated_at = if column_exists(&db, "sessions", "updated_at")? {
                "updated_at"
            } else {
                "''"
            };
            // 拼进 SQL 的只有上面这组**常量分支**，没有一个来自用户数据。
            db.execute_batch(&format!(
                "INSERT OR IGNORE INTO short_term_sessions
                    (session_id, summary, state, message_count, tokens_used, created_at, updated_at)
                 SELECT session_id, summary, {state}, {message_count}, {tokens_used}, {created_at}, {updated_at}
                 FROM sessions;
                 DROP TABLE IF EXISTS sessions_fts;
                 DROP TABLE sessions;",
            ))?;
        }

        // 迁移 ②：内容表补 `search_text`（老库里没有这一列，ALTER 后所有既有行为 ''）。
        if !column_exists(&db, "short_term_sessions", "search_text")? {
            db.execute(
                "ALTER TABLE short_term_sessions ADD COLUMN search_text TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }

        // 迁移 ③：FTS 虚表必须索引 `search_text` 而不是 `summary`。虚表定义不能 ALTER，
        // 只能删了重建 —— 删虚表**不碰内容表**，数据一条不少。
        // 迁移 ④：`session_id` 必须是 **UNINDEXED** 列（同一次重建里一起解决）。
        //
        // 会话 id 本身被索引时，它就成了一第二个可查询的「内容面」：id 形如
        // `agent:main:webchat:…`，unicode61 把它切成 `agent`/`main`/`webchat`，于是任何
        // 含这些词的查询都能靠 **id** 命中**全部**行，摘要写的是什么根本不重要。
        // 旧的 AND 口径把这件事藏住了（要求所有词元同现），改成 OR 之后它立刻显形。
        let fts_sql: Option<String> = db
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'short_term_sessions_fts'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let session_id_is_indexed = fts_sql
            .map(|sql| !sql.contains("UNINDEXED"))
            .unwrap_or(true);
        if session_id_is_indexed || !column_exists(&db, "short_term_sessions_fts", "search_text")? {
            db.execute_batch("DROP TABLE IF EXISTS short_term_sessions_fts;")?;
        }
        db.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS short_term_sessions_fts USING fts5(
                session_id UNINDEXED, search_text,
                content='short_term_sessions', content_rowid='rowid'
            );",
        )?;

        // 分词口径换代 → 回填索引副本。
        // ⚠️ `'rebuild'` 只会照抄内容表的列值，**造不出** `search_text` —— 必须由 Rust 侧
        // 用当前分词器逐行算一遍，否则老库升完 schema 仍是一份空索引（中文照旧搜不到）。
        let indexed_with: Option<String> = db
            .query_row(
                "SELECT value FROM short_term_meta WHERE key = 'tokenizer_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if indexed_with.as_deref() != Some(lexical::TOKENIZER_VERSION) {
            let rows: Vec<(i64, String)> = db
                .prepare("SELECT rowid, summary FROM short_term_sessions")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .filter_map(|r| r.ok())
                .collect();
            {
                let mut stmt = db.prepare("UPDATE short_term_sessions SET search_text = ?1 WHERE rowid = ?2")?;
                for (rowid, summary) in &rows {
                    stmt.execute(rusqlite::params![normalize_for_index(summary), rowid])?;
                }
            }
            db.execute(
                "INSERT OR REPLACE INTO short_term_meta (key, value) VALUES ('tokenizer_version', ?1)",
                rusqlite::params![lexical::TOKENIZER_VERSION],
            )?;
        }

        // 一次性重建外部内容 FTS 索引，清理历史遗留的孤儿 FTS 行。
        // 旧实现在 REPLACE 时只 INSERT 不 DELETE，每轮沉淀都孤儿一行、无限增长；
        // `'rebuild'` 是外部内容 FTS5 官方指令，按内容表整体重建，幂等且能自愈。
        db.execute_batch(
            "INSERT INTO short_term_sessions_fts(short_term_sessions_fts) VALUES('rebuild');",
        )?;

        self.db = Some(db);
        Ok(())
    }

    /// 获取数据库连接引用
    fn db(&self) -> Result<&Connection> {
        self.db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Database not initialized"))
    }

    /// 存储会话摘要
    pub fn store_session(
        &mut self,
        session_id: &str,
        summary: &str,
        messages: &[Message],
    ) -> Result<()> {
        let db = self.db()?;
        let now = Utc::now().to_rfc3339();
        let message_count = messages.len();
        let search_text = normalize_for_index(summary);

        // 先撤销旧行的 FTS 条目。外部内容 FTS5 **必须**用 `'delete'` 指令，直接
        // `DELETE`/`UPDATE` 不被支持。⚠️ 该指令要求的是**当前索引里的列值**，所以传的是
        // 旧的 `search_text`（不是新算出来的那个），列也必须与虚表定义逐一对应。
        // 历史缺陷：只 INSERT 不 DELETE —— session_id 是文本主键，`INSERT OR REPLACE`
        // 会换掉 rowid，旧 FTS 行遂成孤儿，每次沉淀泄漏一行、永不回收（跨轮无限增长）。
        if let Some((old_rowid, old_search_text)) = db
            .query_row(
                "SELECT rowid, search_text FROM short_term_sessions WHERE session_id = ?1",
                rusqlite::params![session_id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
        {
            db.execute(
                "INSERT INTO short_term_sessions_fts(short_term_sessions_fts, rowid, session_id, search_text)
                 VALUES('delete', ?1, ?2, ?3)",
                rusqlite::params![old_rowid, session_id, old_search_text],
            )?;
        }

        db.execute(
            "INSERT OR REPLACE INTO short_term_sessions (session_id, summary, search_text, message_count, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, COALESCE((SELECT created_at FROM short_term_sessions WHERE session_id = ?1), ?5), ?5)",
            rusqlite::params![session_id, summary, search_text, message_count as i32, now],
        )?;

        // 同步 FTS 索引（新行）
        db.execute(
            "INSERT INTO short_term_sessions_fts (rowid, session_id, search_text)
             VALUES ((SELECT rowid FROM short_term_sessions WHERE session_id = ?1), ?1, ?2)",
            rusqlite::params![session_id, search_text],
        )?;

        // 强制最大会话数限制
        self.enforce_max_sessions()?;

        Ok(())
    }

    /// FTS5 全文搜索
    ///
    /// 用户文本先过 [`match_expression`] 编译成 FTS5 表达式：原始串直接进 `MATCH` 等于
    /// 把查询语法暴露给用户（实测 `session_id:s1` 会当成**列选择器**生效）。
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SessionSummary>> {
        let Some(expression) = match_expression(query) else {
            // 一个词元都没有 = 没有条件。如实返回空，绝不退化成「匹配全部」。
            return Ok(Vec::new());
        };
        self.search_match(&expression, limit)
    }

    /// 按**已编译的 FTS5 表达式**检索（[`search`] 的底层实现）。
    ///
    /// 单独开出来是给 `recall_probe` 当注入点：探针要把不同连接语义（AND / OR / 实词口径）
    /// 各编成表达式后并排测，只有走同一条 SQL，才能证明「差的只是口径，不是查询路径」。
    pub(crate) fn search_match(
        &self,
        expression: &str,
        limit: usize,
    ) -> Result<Vec<SessionSummary>> {
        let db = self.db()?;
        let mut stmt = db.prepare(
            "SELECT s.session_id, s.summary, s.message_count, s.created_at, s.updated_at, fts.rank
             FROM short_term_sessions_fts fts
             JOIN short_term_sessions s ON s.rowid = fts.rowid
             WHERE short_term_sessions_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;

        let results = stmt
            .query_map(rusqlite::params![expression, limit as i64], |row| {
                Ok(SessionSummary {
                    session_id: row.get(0)?,
                    summary: row.get(1)?,
                    message_count: row.get::<_, i32>(2)? as usize,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    rank: Some(row.get::<_, f64>(5)?),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    /// 获取最近 N 个会话
    pub fn get_recent_sessions(&self, n: usize) -> Result<Vec<SessionSummary>> {
        let db = self.db()?;
        let mut stmt = db.prepare(
            "SELECT session_id, summary, message_count, created_at, updated_at
             FROM short_term_sessions ORDER BY updated_at DESC LIMIT ?1",
        )?;

        let results = stmt
            .query_map(rusqlite::params![n as i64], |row| {
                Ok(SessionSummary {
                    session_id: row.get(0)?,
                    summary: row.get(1)?,
                    message_count: row.get::<_, i32>(2)? as usize,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    rank: None,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    /// 删除会话
    pub fn delete_session(&self, session_id: &str) -> Result<()> {
        let db = self.db()?;
        // 外部内容 FTS5：先用 `'delete'` 指令撤销索引条目（直接 DELETE 不被支持），
        // 再删内容行 —— 顺序不能反，否则取不到旧 rowid 与**索引里的那份列值**。
        if let Some((rowid, search_text)) = db
            .query_row(
                "SELECT rowid, search_text FROM short_term_sessions WHERE session_id = ?1",
                rusqlite::params![session_id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
        {
            db.execute(
                "INSERT INTO short_term_sessions_fts(short_term_sessions_fts, rowid, session_id, search_text)
                 VALUES('delete', ?1, ?2, ?3)",
                rusqlite::params![rowid, session_id, search_text],
            )?;
        }
        db.execute(
            "DELETE FROM short_term_sessions WHERE session_id = ?1",
            rusqlite::params![session_id],
        )?;
        Ok(())
    }

    /// 会话数量
    pub fn session_count(&self) -> Result<usize> {
        let db = self.db()?;
        let count: i32 = db.query_row("SELECT COUNT(*) FROM short_term_sessions", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    /// 强制最大会话数限制
    ///
    /// 淘汰按 `updated_at`（最旧的先走），不是插入序。
    ///
    /// ⚠️ 撤索引条目逐行走 `'delete'`，**不用**全表 `'rebuild'`。rebuild 语义上没问题，
    /// 但成本是 O(留存上限)：实测（debug 档，2000/500/50 行三档）整表 rebuild 分别是
    /// **11.97 / 6.78 / 3.54 ms 每次**，而逐行 `'delete'` + 删内容行是 **~7.2 ms 每行**、
    /// 与留存上限无关（每轮沉淀只淘汰一两行）。把上限从旧的 50 提到 2000 时，
    /// rebuild 口径的每轮代价会跟着涨 3.4 倍，逐行口径一点不涨 —— 而上限本身是可配项
    /// （`memory.short_term_max_sessions`），调大它不该让每轮对话越写越慢。
    /// 顺序与 [`delete_session`] 一致：先撤索引条目（`'delete'` 要的是**索引里那份列值**），
    /// 再删内容行。
    fn enforce_max_sessions(&self) -> Result<()> {
        let db = self.db()?;
        let count = self.session_count()?;
        if count <= self.max_sessions {
            return Ok(());
        }
        let victims: Vec<(i64, String, String)> = db
            .prepare(
                "SELECT rowid, session_id, search_text FROM short_term_sessions
                 ORDER BY updated_at ASC LIMIT ?1",
            )?
            .query_map(
                rusqlite::params![(count - self.max_sessions) as i64],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?
            .filter_map(|r| r.ok())
            .collect();
        for (rowid, session_id, search_text) in &victims {
            db.execute(
                "INSERT INTO short_term_sessions_fts(short_term_sessions_fts, rowid, session_id, search_text)
                 VALUES('delete', ?1, ?2, ?3)",
                rusqlite::params![rowid, session_id, search_text],
            )?;
            db.execute(
                "DELETE FROM short_term_sessions WHERE rowid = ?1",
                rusqlite::params![rowid],
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;
    use chrono::Utc;

    fn setup_memory(name: &str) -> (ShortTermMemory, PathBuf) {
        let tmp_dir = std::env::temp_dir().join(format!("marguerite_test_{}", name));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let db_path = tmp_dir.join("short_term.db");
        // 删除已有数据库文件，确保测试隔离
        let _ = std::fs::remove_file(&db_path);
        let mut memory = ShortTermMemory::new(&db_path);
        memory.init().unwrap();
        (memory, tmp_dir)
    }

    #[test]
    fn test_short_term_init() {
        let (memory, tmp_dir) = setup_memory("st_init");
        assert_eq!(memory.session_count().unwrap(), 0);
        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    #[test]
    fn test_short_term_store_and_search() {
        let (mut memory, tmp_dir) = setup_memory("st_search");
        let now = Utc::now();

        let msgs = vec![
            Message::User {
                content: "What is Rust?".to_string(),
                timestamp: now,
            },
            Message::Assistant {
                content: "Rust is a systems programming language.".to_string(),
                timestamp: now,
                thinking: None,
            },
        ];
        memory
            .store_session(
                "session-1",
                "Discussion about Rust programming language",
                &msgs,
            )
            .unwrap();

        let msgs2 = vec![
            Message::User {
                content: "Tell me about Python.".to_string(),
                timestamp: now,
            },
            Message::Assistant {
                content: "Python is a high-level programming language.".to_string(),
                timestamp: now,
                thinking: None,
            },
        ];
        memory
            .store_session("session-2", "Discussion about Python", &msgs2)
            .unwrap();

        // Search for Rust
        let results = memory.search("Rust", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id, "session-1");

        // Search for programming (only in session-1's summary)
        let results = memory.search("programming", 10).unwrap();
        assert_eq!(results.len(), 1);

        // Search for Python
        let results = memory.search("Python", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id, "session-2");

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    #[test]
    fn test_short_term_recent_sessions() {
        let (mut memory, tmp_dir) = setup_memory("st_recent");
        let now = Utc::now();

        for i in 0..5 {
            let msgs = vec![Message::User {
                content: format!("msg {}", i),
                timestamp: now,
            }];
            memory
                .store_session(&format!("session-{}", i), &format!("Session {}", i), &msgs)
                .unwrap();
        }

        let recent = memory.get_recent_sessions(3).unwrap();
        assert_eq!(recent.len(), 3);
        // 最新的在前
        assert_eq!(recent[0].session_id, "session-4");

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    #[test]
    fn test_short_term_delete_session() {
        let (mut memory, tmp_dir) = setup_memory("st_delete");
        let now = Utc::now();

        let msgs = vec![Message::User {
            content: "hello".to_string(),
            timestamp: now,
        }];
        memory
            .store_session("session-1", "Test session", &msgs)
            .unwrap();
        assert_eq!(memory.session_count().unwrap(), 1);

        memory.delete_session("session-1").unwrap();
        assert_eq!(memory.session_count().unwrap(), 0);

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    /// 回归守卫：重复沉淀同一会话不得产生孤儿 FTS 行。
    ///
    /// 旧实现 REPLACE 内容行却不撤销旧 FTS 条目，每次沉淀泄漏一行、跨轮无限增长。
    #[test]
    fn test_short_term_repeated_store_does_not_orphan_fts() {
        let (mut memory, tmp_dir) = setup_memory("st_no_orphan");
        let now = Utc::now();
        let msgs = vec![Message::User {
            content: "hello".to_string(),
            timestamp: now,
        }];
        for i in 0..5 {
            memory
                .store_session("s1", &format!("summary {i}"), &msgs)
                .unwrap();
        }
        assert_eq!(memory.session_count().unwrap(), 1, "内容表应只有 1 行");
        let db = memory.db().unwrap();
        let fts_rows: i64 = db
            .query_row("SELECT COUNT(*) FROM short_term_sessions_fts", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(fts_rows, 1, "FTS 索引不得残留孤儿行（旧实现会留 4 行）");
        let hits = memory.search("summary", 10).unwrap();
        assert_eq!(hits.len(), 1, "同一会话只应命中一次");

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    #[test]
    fn test_short_term_session_count() {
        let (mut memory, tmp_dir) = setup_memory("st_count");
        let now = Utc::now();

        assert_eq!(memory.session_count().unwrap(), 0);

        for i in 0..3 {
            let msgs = vec![Message::User {
                content: format!("msg {}", i),
                timestamp: now,
            }];
            memory
                .store_session(&format!("session-{}", i), &format!("Session {}", i), &msgs)
                .unwrap();
        }
        assert_eq!(memory.session_count().unwrap(), 3);

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    // ── D4：FTS5 中文可用 + 上限可配 + 查询语法不得暴露 ──────────────────────

    fn one_msg(now: chrono::DateTime<Utc>) -> Vec<Message> {
        vec![Message::User { content: "x".into(), timestamp: now }]
    }

    /// D4 判据（spec §D4）：**中文摘要必须能被中文查询检索到**。
    ///
    /// 旧 schema 让 FTS5 直接索引原始 `summary`，而 unicode61 把一整段没有空格的中文
    /// 当成**一个** token ——「审批卡片」匹配不到「审批卡片在重连以后自己折叠了」，
    /// 实测 fts5 hit@5 = **0.0%**（这正是 S1 探针判「先修地基」的直接原因）。
    #[test]
    fn chinese_summaries_are_found_by_chinese_queries() {
        let (mut memory, tmp_dir) = setup_memory("st_cjk");
        let now = Utc::now();
        memory
            .store_session("s1", "审批卡片在重连以后自己折叠了", &one_msg(now))
            .unwrap();
        memory
            .store_session("s2", "侧栏会话列表的删除确认框文案", &one_msg(now))
            .unwrap();
        memory
            .store_session("s3", "打包脚本漏掉了 migrations 目录", &one_msg(now))
            .unwrap();
        memory
            .store_session("s4", "Discussion about Rust programming language", &one_msg(now))
            .unwrap();

        for (query, expect) in [
            ("审批卡片", "s1"),
            ("折叠", "s1"),
            ("删除确认", "s2"),
            ("打包脚本", "s3"),
            ("migrations", "s3"),
            ("Rust", "s4"),
        ] {
            let hits = memory.search(query, 5).unwrap();
            assert!(
                hits.iter().any(|h| h.session_id == expect),
                "查询 {query:?} 应命中 {expect}，实得 {hits:?}"
            );
        }

        // 精确度否定面：中文查询不得把英文会话一起捞上来
        let hits = memory.search("审批卡片", 5).unwrap();
        assert_eq!(hits.len(), 1, "中文查询应只命中一条，实得 {hits:?}");

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    /// 检索用的归一化副本**不得**污染展示字段：返回的 `summary` 必须与写入时逐字相同。
    #[test]
    fn search_returns_the_original_summary_unchanged() {
        let (mut memory, tmp_dir) = setup_memory("st_display_copy");
        let now = Utc::now();
        let original = "审批卡片在重连以后自己折叠了";
        memory.store_session("s1", original, &one_msg(now)).unwrap();

        let hits = memory.search("审批", 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].summary, original, "展示文案不得带上分词插入的空格");

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    /// `match_expression` 的四条口径**各自**都得有牙。
    ///
    /// ⚠️ 为什么不能只靠下面那条行为用例：实测把引号那层摘掉（`.map(|t| t)`）后，
    /// 本模块 12 条行为用例**全绿** —— 因为分词器早就吃掉了 `:` `*` `(` `-` 这些标点，
    /// 运算符词又被另一道防线剔除，两道防线盖的是同一块面，行为测试看不见其中任何一层。
    /// 所以直接把编译结果钉住（同 AGENTS.md §4「单点采样 = 恒绿的守卫」那条教训）。
    ///
    /// 连接符与词元口径都写在断言里，不是形式主义：`recall_probe` 的并排测量给出
    /// AND + 全词元 = **0/45 命中**、OR + 全词元 = 精度 0.409（停用词碰撞），
    /// 这两个数只有钉在字符串上才拦得住「改回 AND」或「顺手去掉实词过滤」。
    #[test]
    fn match_expression_compiles_user_text_into_data() {
        // ① 运算符词元被剔除，其余逐词元加引号、以 OR 连接
        assert_eq!(match_expression("Rust AND").as_deref(), Some("\"rust\""));
        assert_eq!(match_expression("NOT Rust").as_deref(), Some("\"rust\""));
        assert_eq!(
            match_expression("审批 OR 卡片").as_deref(),
            Some("\"审批\" OR \"卡片\""),
            "中文只留二字以上的词元：单字 审/批/卡/片 是 bm25 噪声源（实测精度 0.547 → 0.663）"
        );
        // ② 语法字符永远进不了表达式：`:` 不成列选择器、`*` 不成前缀、`"` 不闭合引号
        assert_eq!(
            match_expression("session_id:s1 (审批*) \" NOT").as_deref(),
            Some("\"session\" OR \"id\" OR \"s1\" OR \"审批\""),
            "去重 + 剔除运算符 + 逐词元引号，三者同时生效"
        );
        // ③ 停用词与纯数字不进表达式（否则「why does the …」会去匹配恰好写着 the 的文档）
        assert_eq!(
            match_expression("why does the gateway return 501").as_deref(),
            Some("\"gateway\" OR \"return\"")
        );
        // ④ 实词被滤空时回落到全词元：单字查询不该什么都搜不到
        assert_eq!(match_expression("坏").as_deref(), Some("\"坏\""));

        // 没有词元 = 没有条件，绝不能退化成「匹配全部」
        assert_eq!(match_expression(""), None);
        assert_eq!(match_expression("and or not near"), None);
        assert_eq!(match_expression("！！！，。，"), None);

        // 表达式有长度上限（超限是 SQLite 报错，不是降级）：截断只放宽条件
        let long: String = (0..40).map(|i| format!("w{i} ")).collect();
        assert_eq!(match_expression(&long).unwrap().split(" OR ").count(), 32);
    }

    /// 反向自证：**两个词都真实存在、但从不共现**的查询必须仍能召回。
    ///
    /// 这条是「连接符 = OR」的行为级证据。首版用 AND，实测全库 hit@5 = **0.0%**（45 条查询
    /// 全饿死），而当时的真实库探针只试了 `配置`、`技术方案` 这种两词查询，恰好落在 AND
    /// 还能命中的区间，于是把坏口径当成可用口径放行了。
    #[test]
    fn terms_that_never_co_occur_still_recall_their_own_rows() {
        let (mut memory, tmp_dir) = setup_memory("st_or_not_and");
        let now = Utc::now();
        memory.store_session("s1", "审批卡片的队列转发", &one_msg(now)).unwrap();
        memory.store_session("s2", "限流键采信了调用方的头", &one_msg(now)).unwrap();

        // 「审批」只在 s1、「限流」只在 s2：AND 语义下这条查询必然 0 命中
        let ids: Vec<String> = memory
            .search("审批 限流", 5)
            .unwrap()
            .into_iter()
            .map(|h| h.session_id)
            .collect();
        assert_eq!(ids, ["s1", "s2"], "OR 口径下两个词各回自己的行");

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    /// 用户文本永远是**数据**，不是 FTS5 查询语法。
    ///
    /// 旧实现把原始串直接塞进 `MATCH ?1`。实测（修前先探了一遍，结论和直觉不一样）：
    /// **一个错误都不抛，全部静默改语义** —— 这比报错更糟，因为调用方看到的是一次
    /// 「正常但空」的检索，谁也不会怀疑查询语法：
    /// - `"Rust AND"` → 0 条（`Rust` 单独查是 1 条）；`"NOT Rust"` → 0 条
    /// - `"审批 OR Rust"` → **只回英文那条**（`OR` 当了运算符）
    /// - `"审批*"` → 前缀匹配生效；`"NEAR 审批 卡片"` → 邻近运算符生效
    /// - `"session_id:s1"` → 列选择器生效，命中 s1（内部列名成了一个可注入的查询面）
    /// - `"\\""` / `"("` → 被吞掉，0 条
    ///
    /// 逐词元加引号后，它们一律只是词元。
    ///
    /// ⚠️ 第二条防线在 [`session_id_is_not_a_searchable_column`] 里：改成 OR 之后，
    /// 光靠「不成列选择器」不够 —— `session_id` 这一列**本身被索引**时，
    /// `"session"` / `"s1"` 这些词元照样能靠 id 命中整行。
    #[test]
    fn user_text_never_reaches_the_fts_query_parser() {
        let (mut memory, tmp_dir) = setup_memory("st_match_escape");
        let now = Utc::now();
        memory.store_session("s1", "审批卡片在重连以后自己折叠了", &one_msg(now)).unwrap();
        memory.store_session("s2", "Discussion about Rust programming", &one_msg(now)).unwrap();

        for hostile in [
            "Rust AND", "Rust OR", "\"", "-审批", "(审批", "审批*", "NEAR 审批 卡片",
            "*", "审批 \" 卡片", "session_id:s1",
        ] {
            let r = memory.search(hostile, 5);
            assert!(r.is_ok(), "查询 {hostile:?} 不该让检索失败： {:?}", r.err());
        }

        let ids = |q: &str| -> Vec<String> {
            memory
                .search(q, 5)
                .unwrap()
                .into_iter()
                .map(|h| h.session_id)
                .collect()
        };
        // 运算符词是**剔除**，不是「文档里必须有 or 这个词」的硬要求（那等于又造一个绝对门）
        assert_eq!(ids("审批 OR 卡片"), ["s1"], "旧行为：只回英文那条");
        assert_eq!(ids("Rust AND"), ["s2"], "旧行为：谁都不回（0 条）");
        assert_eq!(ids("NOT Rust"), ["s2"], "旧行为：0 条");
        assert_eq!(ids("(审批"), ["s1"], "旧行为：0 条");
        assert_eq!(ids("审批*"), ["s1"], "前缀运算符不再参与，只按词元匹配");
        // 列名不再被当成列选择器：那几个拉丁词在中文摘要里根本不存在
        assert!(ids("session_id:s1").is_empty(), "列选择器不得生效，实得 {:?}", ids("session_id:s1"));

        // 空查询：没有词元就没有条件，如实返回空，绝不退化成「匹配全部」
        assert!(ids("").is_empty());
        assert!(ids("！！！，。，").is_empty());
        assert!(ids("and or not").is_empty(), "整条查询全是运算符时也不得匹配全部");

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    /// 会话 id 是**标识符**，不是内容：它一旦进了 FTS 索引，任何含 id 片段的查询都会靠
    /// id 命中「全部」会话。
    ///
    /// 真实 id 形如 `agent:main:webchat:1730…`，unicode61 把它切成 `agent`/`main`/`webchat`，
    /// 于是「how does the agent loop work」这类查询的命中与摘要写的是什么**完全无关**
    /// （而且 bm25 会把这些行一起排进 top-K）。
    ///
    /// ⚠️ 旧的 AND 口径把这条藏住了 —— 它要求每个词元同现，而 `session`/`id` 几乎不与
    /// 摘要同现。把连接符改成 OR 之后，上面那条 `session_id:s1` 断言**当场变红**（实测
    /// 返回 `["s1"]`），这条才显形。所以修法不是「别把 `:` 当语法」，而是**这一列根本
    /// 不该参与匹配**。
    #[test]
    fn session_id_is_not_a_searchable_column() {
        let (mut memory, tmp_dir) = setup_memory("st_id_not_indexed");
        let now = Utc::now();
        // id 里带一个拉丁词，摘要里一次都不出现。
        memory
            .store_session("agent:main:zebra:1", "审批卡片在重连以后自己折叠了", &one_msg(now))
            .unwrap();
        memory
            .store_session("s2", "Discussion about Rust programming", &one_msg(now))
            .unwrap();

        let ids = |q: &str| -> Vec<String> {
            memory
                .search(q, 5)
                .unwrap()
                .into_iter()
                .map(|h| h.session_id)
                .collect()
        };
        // 正向自证：索引没坏、摘要照常可检索（否则下面三条会因为「整个索引是空的」而恒绿）。
        assert_eq!(ids("审批卡片"), ["agent:main:zebra:1"]);
        assert_eq!(ids("Rust"), ["s2"]);
        // 只出现在 id 里的词必须什么都搜不到（旧 schema 下它命中整表）。
        assert!(ids("zebra").is_empty(), "id 片段不得成为匹配来源，实得 {:?}", ids("zebra"));
        assert!(ids("agent").is_empty(), "真实 id 前缀不得成为匹配来源，实得 {:?}", ids("agent"));
        assert!(ids("main").is_empty(), "实得 {:?}", ids("main"));

        // schema 层再钉一次：行为断言只证明「这一列不匹配」，DDL 才证明「它为什么不匹配」。
        let sql: String = memory
            .db()
            .unwrap()
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'short_term_sessions_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            sql.contains("session_id UNINDEXED"),
            "虚表定义没把 id 排除出索引：{sql}"
        );

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    /// 机器上躺着的是**已索引 id 的那一版虚表**，而 `CREATE VIRTUAL TABLE IF NOT EXISTS`
    /// 对它是个 no-op —— 所以迁移必须能认出「定义存在、口径不对」并重建。
    ///
    /// 少了这一步，修 DDL 只对新建库生效，真实用户的库带着旧定义继续把 id 当内容匹配。
    #[test]
    fn init_rebuilds_an_fts_table_that_indexes_the_session_id() {
        let tmp_dir = std::env::temp_dir().join("marguerite_test_st_id_migration");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let db_path = tmp_dir.join("short_term.db");
        let _ = std::fs::remove_file(&db_path);

        {
            let old = Connection::open(&db_path).unwrap();
            old.execute_batch(
                "CREATE TABLE short_term_sessions (
                    session_id TEXT PRIMARY KEY,
                    summary TEXT NOT NULL,
                    state TEXT NOT NULL DEFAULT 'Ready',
                    message_count INTEGER DEFAULT 0,
                    tokens_used INTEGER DEFAULT 0,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    search_text TEXT NOT NULL DEFAULT ''
                );
                CREATE VIRTUAL TABLE short_term_sessions_fts USING fts5(
                    session_id, search_text, content='short_term_sessions', content_rowid='rowid'
                );
                INSERT INTO short_term_sessions (session_id, summary, created_at, updated_at, search_text)
                    VALUES ('agent:main:zebra:1', '审批卡片在重连以后自己折叠了',
                            '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '审 批 审批 卡 片 卡片');
                -- 外部内容虚表不会自动同步，旧库是靠这一句把 id 写进索引的。
                INSERT INTO short_term_sessions_fts (rowid, session_id, search_text)
                    SELECT rowid, session_id, search_text FROM short_term_sessions;",
            )
            .unwrap();
            // 旧库的索引里确实带着 id 列的词元 —— 先把「缺陷现场」造出来。
            let hit: i64 = old
                .query_row(
                    "SELECT count(*) FROM short_term_sessions_fts WHERE short_term_sessions_fts MATCH '\"zebra\"'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(hit, 1, "夹具没造出旧 schema 的 id 命中，这条测试将什么都证明不了");
        }

        let mut memory = ShortTermMemory::new(&db_path);
        memory.init().unwrap();
        assert_eq!(memory.session_count().unwrap(), 1, "重建虚表不许丢数据");
        assert!(
            memory.search("审批卡片", 5)
                .unwrap()
                .iter()
                .any(|h| h.session_id == "agent:main:zebra:1"),
            "旧库升完必须照常可检索"
        );
        assert!(
            memory.search("zebra", 5).unwrap().is_empty(),
            "升级后 id 片段仍不得命中：实得 {:?}",
            memory.search("zebra", 5).unwrap()
        );

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }
    /// 默认留存上限必须容纳真实工作量：旧值 50 意味着**跑两周就把自己检索不回来了**。
    #[test]
    fn default_retention_keeps_far_more_than_fifty_sessions() {
        let (mut memory, tmp_dir) = setup_memory("st_retention");
        let now = Utc::now();
        for i in 0..600 {
            memory
                .store_session(
                    &format!("session-{i:04}"),
                    &format!("第 {i} 场会话的中文摘要 · 关于审批卡片"),
                    &one_msg(now),
                )
                .unwrap();
        }
        assert_eq!(memory.session_count().unwrap(), 600, "600 条写入后必须全部留存");

        let hits = memory.search("审批卡片", 5).unwrap();
        assert_eq!(hits.len(), 5, "检索应照常工作（limit 生效，不是全表）");

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    /// 上限**可配**，且淘汰仍然真实发生（外部内容 FTS 不得留下孤儿行）。
    #[test]
    fn max_sessions_is_configurable_and_eviction_stays_consistent() {
        let tmp_dir = std::env::temp_dir().join("marguerite_test_st_cap");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let db_path = tmp_dir.join("short_term.db");
        let _ = std::fs::remove_file(&db_path);
        let mut memory = ShortTermMemory::new(&db_path).with_max_sessions(3);
        memory.init().unwrap();
        let now = Utc::now();
        for i in 0..5 {
            memory
                .store_session(&format!("s{i}"), &format!("会话 {i} 的摘要"), &one_msg(now))
                .unwrap();
        }
        assert_eq!(memory.session_count().unwrap(), 3);

        let db = memory.db().unwrap();
        let fts_rows: i64 = db
            .query_row("SELECT COUNT(*) FROM short_term_sessions_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts_rows, 3, "内容行与 FTS 行必须一一对应（孤儿行是历史缺陷）");
        let orphans: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM short_term_sessions_fts f
                 WHERE NOT EXISTS (SELECT 1 FROM short_term_sessions s WHERE s.rowid = f.rowid)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            orphans, 0,
            "FTS 里不得有指向已删除会话的行",
            // ⚠️ 这两条**都看不见 ghost**：无 MATCH 的扫描取不出词项索引里的残留条目。
            // 淘汰真正留下的 ghost 由 `evicted_sessions_stop_matching_survivors_keep_matching`
            // 用原始 MATCH 往返钉住（那里附了变异前后的实测对照表）。
        );

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    /// 淘汰的**用户可见契约**：被赶走的会话再也搜不到，留下的照旧搜得到。
    ///
    /// ⚠️ 判据必须是**原始 MATCH 往返**，不能是 `COUNT(*)` / rowid 扫描 / 「索引行 JOIN 内容行」。
    /// 实测（同一夹具 6 存 3 留，两列分别对应「逐行 delete」与「把逐行 delete 摘掉的变异」）：
    ///
    /// | 观测 | 逐行 `'delete'` | 摘掉 `'delete'` |
    /// |---|---|---|
    /// | `COUNT(*) FROM …_fts` | 3 | 3 |
    /// | `SELECT rowid FROM …_fts`（无 MATCH） | `[4,5,6]` | `[4,5,6]` |
    /// | 无 MATCH 扫描下的孤儿行数 | 0 | 0 |
    /// | `MATCH '"alpha0"'` 的 rowid | **空** | **`[1]`** ← 只有这里看得见 |
    ///
    /// 外部内容表的 ghost 条目躺在**词项索引**里：不 MATCH 就取不出来，一 MATCH 就还一个
    /// 内容表里已经不存在的 docid。而 [`search_match`] 末尾那个 `JOIN short_term_sessions`
    /// 会把 ghost **静默丢掉**，所以「检索结果里没有它」同样证明不了索引干净 ——
    /// 它真正的代价是 bm25 把这些 ghost 一起排进候选、**白占 top-K 的名额**，
    /// 幸存的会话因此被挤出窗口（这正是 2026 年那次「每次沉淀泄漏一行、永不回收」的形态）。
    #[test]
    fn evicted_sessions_stop_matching_survivors_keep_matching() {
        let tmp_dir = std::env::temp_dir().join("marguerite_test_st_evict_match");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let db_path = tmp_dir.join("short_term.db");
        let _ = std::fs::remove_file(&db_path);
        let mut memory = ShortTermMemory::new(&db_path).with_max_sessions(3);
        memory.init().unwrap();
        let now = Utc::now();
        for i in 0..6 {
            memory
                .store_session(
                    &format!("s{i}"),
                    &format!("alpha{i} 会话摘要"),
                    &one_msg(now),
                )
                .unwrap();
        }

        let retained: Vec<String> = memory
            .get_recent_sessions(10)
            .unwrap()
            .into_iter()
            .map(|s| s.session_id)
            .collect();
        assert_eq!(retained.len(), 3, "上限 3 就该只剩 3 场");

        let db = memory.db().unwrap();
        let indexed_hits = |term: &str| -> Vec<i64> {
            db.prepare(
                "SELECT rowid FROM short_term_sessions_fts WHERE short_term_sessions_fts MATCH ?1",
            )
            .unwrap()
            .query_map([format!("\"{term}\"")], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
        };

        // 哪几条被淘汰由 `updated_at` 决定，所以不断言具体编号，只断言
        // 「索引里的集合 == 内容表里的集合」——夹具里时间戳打平也不会偶发。
        for i in 0..6 {
            let id = format!("s{i}");
            let term = format!("alpha{i}");
            let in_index = indexed_hits(&term);
            if retained.contains(&id) {
                assert_eq!(in_index.len(), 1, "留存的 {id} 必须在索引里恰好一条：{in_index:?}");
                assert_eq!(
                    memory.search(&term, 5).unwrap().len(),
                    1,
                    "留存的 {id} 必须还能按自己的摘要词搜到"
                );
            } else {
                assert!(
                    in_index.is_empty(),
                    "已淘汰的 {id} 在索引里还剩 {in_index:?} 条 ghost——它们不参与返回，\
                     但会占住 bm25 的 top-K 名额，把幸存会话挤出窗口"
                );
                assert!(
                    memory.search(&term, 5).unwrap().is_empty(),
                    "已淘汰的 {id} 仍被检索返回"
                );
            }
        }
        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    /// 淘汰**不许**改回全表 `'rebuild'`。
    ///
    /// 这条是静态守卫而不是行为用例，因为 rebuild 语义完全正确，唯一的问题是成本：
    /// 它是 O(留存上限) 的**每轮写入**代价。实测（debug 档，同一台机器；绝对值会随机型变，
    /// 要看的是**随行数涨 vs 不涨**这个形状）整表 rebuild 每次 **3.54 / 6.78 / 11.97 ms**
    /// 对应 **50 / 500 / 2000** 行，逐行 `'delete'` + 删内容行恒定 **~7.2 ms/行**、每轮只淘汰一两行。
    /// 留存上限是可配项（`memory.short_term_max_sessions`），调大它只会让 rebuild 口径「越写越慢」。
    /// 所以这里钉的是形状：`'rebuild'` 只允许躺在 `init()` 的分词换代回填里。
    ///
    /// ⚠️ 两个「守卫自己造盲区」的坑都绕开了：
    /// - **被检字符串不许在本文件里字面出现**，否则守卫会数到自己。首版就是这么写的，
    ///   于是「全文件只有一处 rebuild」当场判红（left=3：真的一处 + 守卫自己的两处）。
    ///   needle 一律 `format!` 拼出来。
    /// - **范围必须收窄到函数体**，不能用 `&src[i..]` 一路看到文件尾 —— 尾部的测试模块里
    ///   本来就写着同样的 SQL 字面量，那样「淘汰里有 delete」会恒真。
    #[test]
    fn eviction_never_rebuilds_the_whole_index() {
        let src = include_str!("./short_term.rs");
        let rebuild_cmd = format!("VALUES('{}')", "rebuild");
        let delete_cmd = format!("VALUES('{}'", "delete");

        let eviction = src
            .find("fn enforce_max_sessions(")
            .expect("淘汰函数必须还在");
        // 方法缩进 4 格，函数体内不会出现列 0 的 `}`；下一个列 0 的 `}` 就是 impl 收尾。
        let body = &src[eviction..eviction + src[eviction..].find("\n}").unwrap() + 2];
        assert!(
            body.contains(&delete_cmd),
            "淘汰必须逐行撤索引条目，否则内容行删了、索引还留着"
        );
        assert_eq!(
            body.matches(&rebuild_cmd).count(),
            0,
            "enforce_max_sessions 里出现了全表 rebuild：每次沉淀都会重建整份索引"
        );
        assert_eq!(
            src.matches(&rebuild_cmd).count(),
            1,
            "全文件应只剩 init() 回填那一处 rebuild，实测多于一处"
        );
    }

    /// 迁移：磁盘上**已经存在**的就是旧 schema（无 `search_text` 列 + FTS 索引原始 summary）。
    ///
    /// 这条用例模拟真实用户机器：旧库里有数据，`init()` 必须把它升到新 schema 并**回填**
    /// 索引，而不是静默留下一份「中文搜不到、英文也搜不到」的空索引。
    #[test]
    fn init_migrates_a_legacy_database_and_backfills_the_index() {
        let tmp_dir = std::env::temp_dir().join("marguerite_test_st_legacy");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let db_path = tmp_dir.join("short_term.db");
        let _ = std::fs::remove_file(&db_path);

        {
            let legacy = Connection::open(&db_path).unwrap();
            legacy
                .execute_batch(
                    "CREATE TABLE short_term_sessions (
                        session_id TEXT PRIMARY KEY,
                        summary TEXT NOT NULL,
                        state TEXT NOT NULL DEFAULT 'Ready',
                        message_count INTEGER DEFAULT 0,
                        tokens_used INTEGER DEFAULT 0,
                        created_at TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    );
                    CREATE VIRTUAL TABLE short_term_sessions_fts USING fts5(
                        session_id, summary, content='short_term_sessions', content_rowid='rowid'
                    );
                    INSERT INTO short_term_sessions (session_id, summary, created_at, updated_at)
                        VALUES ('old-1', '审批卡片在重连以后自己折叠了', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
                               ('old-2', 'Discussion about Rust programming', '2026-01-02T00:00:00Z', '2026-01-02T00:00:00Z');
                    INSERT INTO short_term_sessions_fts (rowid, session_id, summary)
                        SELECT rowid, session_id, summary FROM short_term_sessions;",
                )
                .unwrap();
        }

        let mut memory = ShortTermMemory::new(&db_path);
        memory.init().unwrap();
        assert_eq!(memory.session_count().unwrap(), 2, "旧数据一条不许丢");

        let zh = memory.search("审批卡片", 5).unwrap();
        assert!(zh.iter().any(|h| h.session_id == "old-1"), "旧库中文行迁移后必须可检索，实得 {zh:?}");
        let en = memory.search("Rust", 5).unwrap();
        assert!(en.iter().any(|h| h.session_id == "old-2"), "旧库英文行迁移后必须可检索，实得 {en:?}");

        // 幂等：第二次 init 不得把索引搞坏，也不得把行数搞变
        drop(memory);
        let mut again = ShortTermMemory::new(&db_path);
        again.init().unwrap();
        assert_eq!(again.session_count().unwrap(), 2);
        assert!(again.search("审批卡片", 5).unwrap().iter().any(|h| h.session_id == "old-1"));

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    /// 真实机器上最老的那一种形状：`sessions` **只有 5 列**（没有 `state` / `tokens_used`）。
    ///
    /// 这条不是假想敌 —— DDL 是从作者本机 `marguerite_short_term.db`（2026-09-04 停写）里
    /// `SELECT sql FROM sqlite_master` 抄下来的。写死列名的迁移在这份库上直接
    /// `no such column: state`，而 `init()` 一报错，`gateway/chat.rs::memory_stack` 就
    /// `return None`：**整个短期记忆静默消失**，用户看到的只是「什么都搜不到」。
    #[test]
    fn init_migrates_the_oldest_five_column_sessions_table() {
        let tmp_dir = std::env::temp_dir().join("marguerite_test_st_oldest");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let db_path = tmp_dir.join("short_term.db");
        let _ = std::fs::remove_file(&db_path);

        {
            let legacy = Connection::open(&db_path).unwrap();
            legacy
                .execute_batch(
                    "CREATE TABLE sessions (
                        session_id TEXT PRIMARY KEY,
                        summary TEXT NOT NULL,
                        message_count INTEGER DEFAULT 0,
                        created_at TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    );
                    INSERT INTO sessions (session_id, summary, message_count, created_at, updated_at)
                        VALUES ('s1', '审批卡片在重连以后自己折叠了', 3, '2026-08-25T12:40:51Z', '2026-08-25T12:40:51Z'),
                               ('s2', 'Discussion about Rust programming', 2, '2026-09-04T11:38:43Z', '2026-09-04T11:38:43Z');",
                )
                .unwrap();
        }

        let mut memory = ShortTermMemory::new(&db_path);
        memory.init().expect("5 列旧库必须能迁移，不能让 init() 报错");
        assert_eq!(memory.session_count().unwrap(), 2, "50 行真实数据一条不许丢（这里是 2 行夹具）");
        assert!(memory
            .search("审批卡片", 5)
            .unwrap()
            .iter()
            .any(|h| h.session_id == "s1"));
        assert!(memory
            .search("Rust", 5)
            .unwrap()
            .iter()
            .any(|h| h.session_id == "s2"));

        // 旧表已经搬走并删除，`state` 落的是默认值
        let db = memory.db().unwrap();
        let old_still_there: bool = db
            .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='sessions'")
            .unwrap()
            .exists([])
            .unwrap();
        assert!(!old_still_there, "迁移完成后旧表不得残留（否则下次 init 会重复搬）");
        let state: String = db
            .query_row("SELECT state FROM short_term_sessions WHERE session_id='s1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(state, "Ready");

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }
}
