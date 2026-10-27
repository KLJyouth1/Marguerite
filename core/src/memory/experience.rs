//! 经验台账：自进化记忆沉淀层
//!
//! 在既有三层记忆（感知 / 短期 / 长期）之上，提供结构化、可检索的「经验」沉淀：
//! - `Pitfall`（踩坑）：稳定签名 + 根因 + 修法 + 是否已验证
//! - `Decision`（决策）：ADR 式 —— 背景 / 决策 / 影响
//! - `Pattern`（解法）：可复用的问题→方案映射
//!
//! 复用 `long_term::LocalEmbedding` 做语义检索（零外部依赖、确定性哈希）。

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;

use crate::memory::embedding::EmbeddingProvider;
use crate::memory::long_term::LongTermMemory;
use crate::memory::sensory::SensoryMemory;
use crate::types::Message;

/// 经验台账总条数上限：超出后按「最旧 + 未验证优先」裁剪，防 DB 无界增长
/// （每轮会话的每条 ≥20 字符消息都会沉淀为 Pattern）。
const MAX_EXPERIENCE_ROWS: usize = 1000;

/// 经验类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExperienceKind {
    /// 踩坑记录
    Pitfall,
    /// 架构 / 设计决策（ADR 式）
    Decision,
    /// 可复用解法模式
    Pattern,
}

impl ExperienceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExperienceKind::Pitfall => "pitfall",
            ExperienceKind::Decision => "decision",
            ExperienceKind::Pattern => "pattern",
        }
    }

    pub fn from_kind_str(s: &str) -> ExperienceKind {
        match s {
            "decision" => ExperienceKind::Decision,
            "pattern" => ExperienceKind::Pattern,
            _ => ExperienceKind::Pitfall,
        }
    }
}

/// 经验条目
#[derive(Debug, Clone)]
pub struct Experience {
    pub id: i64,
    pub kind: ExperienceKind,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub signature: String,
    pub validated: bool,
    pub created_at: String,
    /// 来源：`user` / `agent` / `tool` / `import`（缺省 `agent`）
    pub source: String,
    /// 主体：会话键或 agent id（缺省空串）
    pub actor: String,
    /// 置信度 0..1（缺省 0.6）
    pub confidence: f64,
    /// 状态：`draft` / `active`（缺省 `draft`）
    pub status: String,
    /// 会话键（可选，便于按会话追溯来源）
    pub session_key: String,
}

/// 写入经验时携带的治理元数据（来源 / 主体 / 置信度 / 状态）。
///
/// 单独成结构是为了让 `record` 的签名保持向后兼容，同时避免
/// `clippy::too_many_arguments`（参数超过 7 个即告警）。
#[derive(Debug, Clone, PartialEq)]
pub struct ExperienceMeta {
    pub source: String,
    pub actor: String,
    pub confidence: f64,
    pub status: String,
    /// 会话键（`memory.save` 的 `sessionKey`）
    pub session_key: String,
}

impl Default for ExperienceMeta {
    fn default() -> Self {
        Self {
            source: "agent".to_string(),
            actor: String::new(),
            confidence: 0.6,
            status: "draft".to_string(),
            session_key: String::new(),
        }
    }
}

impl ExperienceMeta {
    /// 归一化来源/状态/置信度：未知取值退回默认（不因前端多传字段而丢数据）。
    pub fn normalized(mut self) -> Self {
        self.source = match self.source.to_ascii_lowercase().as_str() {
            "user" => "user",
            "tool" => "tool",
            "import" => "import",
            _ => "agent",
        }
        .to_string();
        self.status = match self.status.to_ascii_lowercase().as_str() {
            "active" => "active",
            _ => "draft",
        }
        .to_string();
        if !self.confidence.is_finite() {
            self.confidence = 0.6;
        }
        self.confidence = self.confidence.clamp(0.0, 1.0);
        self
    }
}

/// 经验台账（SQLite 持久化，语义检索）
pub struct ExperienceLedger {
    db_path: PathBuf,
    db: Option<Connection>,
    /// 进程内向量索引（惰性加载）：`(PRAGMA data_version, 行向量)`。
    /// 消除 search 每次全表扫描 + 逐行 split/parse 向量的重复开销；
    /// 本实例 record/mark_validated 就地增量维护，其他连接提交后 data_version
    /// 变化触发全量重建。
    index: RefCell<Option<(i64, Vec<ExpIndexRow>)>>,
}

/// 索引行：与 experience_ledger 表一一对应（向量已解析为 f64）
struct ExpIndexRow {
    id: i64,
    kind: String,
    title: String,
    body: String,
    tags: String,
    signature: String,
    validated: bool,
    created_at: String,
    source: String,
    actor: String,
    confidence: f64,
    status: String,
    session_key: String,
    vec: Vec<f64>,
}

impl ExperienceLedger {
    pub fn new(path: &Path) -> Self {
        Self {
            db_path: path.to_path_buf(),
            db: None,
            index: RefCell::new(None),
        }
    }

    pub fn init(&mut self) -> Result<()> {
        let db = Connection::open(&self.db_path)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS experience_ledger (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                body TEXT NOT NULL,
                tags TEXT DEFAULT '',
                signature TEXT DEFAULT '',
                vector TEXT NOT NULL,
                validated INTEGER DEFAULT 0,
                created_at TEXT NOT NULL,
                source TEXT DEFAULT 'agent',
                actor TEXT DEFAULT '',
                confidence REAL DEFAULT 0.6,
                status TEXT DEFAULT 'draft',
                session_key TEXT DEFAULT '',
                deleted_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_exp_kind ON experience_ledger(kind);
            CREATE INDEX IF NOT EXISTS idx_exp_sig ON experience_ledger(signature);",
        )?;
        Self::migrate_governance_columns(&db)?;
        self.db = Some(db);
        Ok(())
    }

    /// 幂等迁移：为升级前建库的台账补上治理列。
    ///
    /// SQLite 的 `ALTER TABLE ... ADD COLUMN` 没有 `IF NOT EXISTS`，所以先读
    /// `PRAGMA table_info` 再按差集补齐；重复执行是 no-op，旧库首次启动即完成迁移。
    fn migrate_governance_columns(db: &Connection) -> Result<()> {
        let mut stmt = db.prepare("PRAGMA table_info(experience_ledger)")?;
        let existing: std::collections::HashSet<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(Result::ok)
            .collect();
        let columns: [(&str, &str); 6] = [
            ("source", "TEXT DEFAULT 'agent'"),
            ("actor", "TEXT DEFAULT ''"),
            ("confidence", "REAL DEFAULT 0.6"),
            ("status", "TEXT DEFAULT 'draft'"),
            ("session_key", "TEXT DEFAULT ''"),
            // 软删标记：NULL = 未删除；时间戳 = 已软删（检索自动排除）
            ("deleted_at", "TEXT"),
        ];
        for (name, ddl) in columns {
            if !existing.contains(name) {
                db.execute(
                    &format!("ALTER TABLE experience_ledger ADD COLUMN {name} {ddl}"),
                    [],
                )?;
            }
        }
        Ok(())
    }

    fn db(&self) -> Result<&Connection> {
        self.db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("ExperienceLedger not initialized"))
    }

    /// 记录一条经验（默认治理元数据）；`signature` 相同则更新而非重复插入（幂等防重）
    pub fn record(
        &mut self,
        kind: ExperienceKind,
        title: &str,
        body: &str,
        tags: &[String],
        signature: &str,
    ) -> Result<i64> {
        self.record_meta(
            kind,
            title,
            body,
            tags,
            signature,
            &ExperienceMeta::default(),
        )
    }

    /// 记录一条经验并落库治理元数据（来源 / 主体 / 置信度 / 状态）。
    pub fn record_meta(
        &mut self,
        kind: ExperienceKind,
        title: &str,
        body: &str,
        tags: &[String],
        signature: &str,
        meta: &ExperienceMeta,
    ) -> Result<i64> {
        let meta = meta.clone().normalized();
        let db = self.db()?;
        let vec = EmbeddingProvider::global().embed(&format!("{title} {body}"));
        let vector = vec
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let now = Utc::now().to_rfc3339();
        let tags_str = tags.join(",");

        // 幂等：signature 相同则更新
        if !signature.is_empty() {
            if let Ok(existing) = db.query_row(
                "SELECT id FROM experience_ledger WHERE signature = ?1 LIMIT 1",
                rusqlite::params![signature],
                |r| r.get::<_, i64>(0),
            ) {
                db.execute(
                    "UPDATE experience_ledger SET kind=?1, title=?2, body=?3, tags=?4, vector=?5, created_at=?6,
                        source=?7, actor=?8, confidence=?9, status=?10, session_key=?11, deleted_at=NULL WHERE id=?12",
                    rusqlite::params![
                        kind.as_str(),
                        title,
                        body,
                        tags_str,
                        vector,
                        now,
                        meta.source,
                        meta.actor,
                        meta.confidence,
                        meta.status,
                        meta.session_key,
                        existing
                    ],
                )?;
                // 索引已加载时原地替换该行（向量/正文/治理字段已变）
                if let Ok(mut idx) = self.index.try_borrow_mut() {
                    if let Some((_, rows)) = idx.as_mut() {
                        if let Some(row) = rows.iter_mut().find(|r| r.id == existing) {
                            row.kind = kind.as_str().to_string();
                            row.title = title.to_string();
                            row.body = body.to_string();
                            row.tags = tags_str.clone();
                            row.created_at = now.clone();
                            row.source = meta.source.clone();
                            row.actor = meta.actor.clone();
                            row.confidence = meta.confidence;
                            row.status = meta.status.clone();
                            row.session_key = meta.session_key.clone();
                            row.vec = vec;
                        }
                    }
                }
                return Ok(existing);
            }
        }

        db.execute(
            "INSERT INTO experience_ledger (kind, title, body, tags, signature, vector, validated, created_at, source, actor, confidence, status, session_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                kind.as_str(),
                title,
                body,
                tags_str,
                signature,
                vector,
                now,
                meta.source,
                meta.actor,
                meta.confidence,
                meta.status,
                meta.session_key
            ],
        )?;
        let id = db.last_insert_rowid();
        // 有界：超出总条数上限则裁剪最旧 / 最不有价值的条目（防 DB 无界增长）。
        let pruned = self.prune_over_cap()?;
        if pruned > 0 {
            // DB 行已删除：进程内索引与库不一致 → 作废，下次 search 全量重建。
            *self.index.borrow_mut() = None;
        } else if let Ok(mut idx) = self.index.try_borrow_mut() {
            // 索引已加载时增量追加（复用刚算出的向量）
            if let Some((_, rows)) = idx.as_mut() {
                rows.push(ExpIndexRow {
                    id,
                    kind: kind.as_str().to_string(),
                    title: title.to_string(),
                    body: body.to_string(),
                    tags: tags_str.clone(),
                    signature: signature.to_string(),
                    validated: false,
                    created_at: now,
                    source: meta.source.clone(),
                    actor: meta.actor.clone(),
                    confidence: meta.confidence,
                    status: meta.status.clone(),
                    session_key: meta.session_key.clone(),
                    vec,
                });
            }
        }
        Ok(id)
    }

    /// 超出总条数上限时裁剪：优先删除最旧的**未验证**条目，仍超则删最旧条目。
    ///
    /// 返回删除条数（0 表示未超限）。已 `mark_validated` 的高价值经验在容量足够时
    /// 不会被优先删除。调用后由调用方负责作废进程内索引。
    fn prune_over_cap(&self) -> Result<usize> {
        let db = self.db()?;
        let total: i64 = db.query_row(
            "SELECT COUNT(*) FROM experience_ledger WHERE deleted_at IS NULL",
            [],
            |r| r.get(0),
        )?;
        let total = total.max(0) as usize;
        if total <= MAX_EXPERIENCE_ROWS {
            return Ok(0);
        }
        let mut excess = total - MAX_EXPERIENCE_ROWS;
        let mut removed = 0usize;
        // 第一轮：最旧的未验证条目（least-valuable）。
        removed += db.execute(
            "DELETE FROM experience_ledger WHERE id IN (
                SELECT id FROM experience_ledger WHERE validated = 0 AND deleted_at IS NULL
                ORDER BY created_at ASC, id ASC LIMIT ?1
             )",
            rusqlite::params![excess as i64],
        )?;
        excess = excess.saturating_sub(removed);
        if excess > 0 {
            // 第二轮：仍超限则删最旧条目（含已验证）。
            removed += db.execute(
                "DELETE FROM experience_ledger WHERE id IN (
                    SELECT id FROM experience_ledger WHERE deleted_at IS NULL
                    ORDER BY created_at ASC, id ASC LIMIT ?1
                 )",
                rusqlite::params![excess as i64],
            )?;
        }
        Ok(removed)
    }

    /// 语义检索（低于阈值视为无相关）
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Experience>> {
        const SIM_THRESHOLD: f64 = 0.25;
        let provider = EmbeddingProvider::global();
        let qvec = provider.embed(query);
        // 索引失效检测：data_version 变化（其他连接提交了写）→ 全量重建。
        // 本实例自己的 record 已就地增量维护。
        let version = self.db_data_version().unwrap_or(-1);
        let need_load = match self.index.borrow().as_ref() {
            Some((v, _)) => *v != version,
            None => true,
        };
        if need_load {
            let rows = self.load_index_rows()?;
            *self.index.borrow_mut() = Some((version, rows));
        }
        let index = self.index.borrow();
        let rows = &index.as_ref().expect("索引刚加载").1;

        let mut scored = Vec::new();
        for row in rows {
            let score = provider.cosine(&qvec, &row.vec);
            if score >= SIM_THRESHOLD {
                scored.push((
                    score,
                    Experience {
                        id: row.id,
                        kind: ExperienceKind::from_kind_str(&row.kind),
                        title: row.title.clone(),
                        body: row.body.clone(),
                        tags: if row.tags.is_empty() {
                            Vec::new()
                        } else {
                            row.tags.split(',').map(|s| s.to_string()).collect()
                        },
                        signature: row.signature.clone(),
                        validated: row.validated,
                        created_at: row.created_at.clone(),
                        source: row.source.clone(),
                        actor: row.actor.clone(),
                        confidence: row.confidence,
                        status: row.status.clone(),
                        session_key: row.session_key.clone(),
                    },
                ));
            }
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        Ok(scored.into_iter().map(|(_, e)| e).collect())
    }

    /// 当前连接的 PRAGMA data_version：其他连接提交写时递增（本连接自己的写不增）。
    fn db_data_version(&self) -> Result<i64> {
        let db = self.db()?;
        Ok(db.query_row("PRAGMA data_version", [], |r| r.get(0))?)
    }

    /// 从 DB 全量加载并解析索引行（维度不符的旧向量跳过，与旧逐行检索逻辑一致）。
    fn load_index_rows(&self) -> Result<Vec<ExpIndexRow>> {
        let db = self.db()?;
        let dim = EmbeddingProvider::global().dim();
        let mut stmt = db.prepare(
            "SELECT id, kind, title, body, tags, vector, signature, validated, created_at,
                    source, actor, confidence, status, session_key
             FROM experience_ledger WHERE deleted_at IS NULL",
        )?;
        let iter = stmt.query_map([], |row| {
            let raw_vec: String = row.get(5)?;
            Ok((
                ExpIndexRow {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    title: row.get(2)?,
                    body: row.get(3)?,
                    tags: row.get(4)?,
                    signature: row.get(6)?,
                    validated: row.get::<_, i64>(7)? != 0,
                    created_at: row.get(8)?,
                    source: row
                        .get::<_, Option<String>>(9)?
                        .unwrap_or_else(|| "agent".to_string()),
                    actor: row.get::<_, Option<String>>(10)?.unwrap_or_default(),
                    confidence: row.get::<_, Option<f64>>(11)?.unwrap_or(0.6),
                    status: row
                        .get::<_, Option<String>>(12)?
                        .unwrap_or_else(|| "draft".to_string()),
                    session_key: row.get::<_, Option<String>>(13)?.unwrap_or_default(),
                    vec: Vec::new(),
                },
                raw_vec,
            ))
        })?;
        let mut rows = Vec::new();
        for item in iter.flatten() {
            let (mut row, raw_vec) = item;
            let vec: Vec<f64> = raw_vec
                .split(',')
                .filter_map(|s| s.parse::<f64>().ok())
                .collect();
            if vec.len() != dim {
                continue;
            }
            row.vec = vec;
            rows.push(row);
        }
        Ok(rows)
    }

    /// 按类型列出（最新在前）
    pub fn list_by_kind(&self, kind: ExperienceKind, limit: usize) -> Result<Vec<Experience>> {
        let db = self.db()?;
        let mut stmt = db.prepare(&format!(
            "SELECT {EXPERIENCE_COLUMNS}
             FROM experience_ledger WHERE kind = ?1 AND deleted_at IS NULL
             ORDER BY created_at DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(rusqlite::params![kind.as_str(), limit as i64], row_to_experience)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// 列出最近条目（全类型，最新在前；网关 `memory.list` 默认路径）
    pub fn list_recent(&self, limit: usize) -> Result<Vec<Experience>> {
        let db = self.db()?;
        let mut stmt = db.prepare(&format!(
            "SELECT {EXPERIENCE_COLUMNS}
             FROM experience_ledger WHERE deleted_at IS NULL
             ORDER BY created_at DESC LIMIT ?1"
        ))?;
        let rows = stmt.query_map([limit as i64], row_to_experience)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// 管理面分页查询：按 kind / status 过滤（`None` = 不过滤），最新在前。
    pub fn list_entries(
        &self,
        kind: Option<&str>,
        status: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Experience>> {
        use rusqlite::types::Value as SqlValue;
        let db = self.db()?;
        let mut sql = format!("SELECT {EXPERIENCE_COLUMNS} FROM experience_ledger");
        let mut conds: Vec<&str> = vec!["deleted_at IS NULL"];
        let mut binds: Vec<SqlValue> = Vec::new();
        if let Some(k) = kind.filter(|k| !k.is_empty()) {
            conds.push("kind = ?");
            binds.push(SqlValue::Text(k.to_string()));
        }
        if let Some(s) = status.filter(|s| !s.is_empty()) {
            conds.push("status = ?");
            binds.push(SqlValue::Text(s.to_string()));
        }
        if !conds.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conds.join(" AND "));
        }
        sql.push_str(" ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?");
        binds.push(SqlValue::Integer(limit.max(1) as i64));
        binds.push(SqlValue::Integer(offset as i64));
        let mut stmt = db.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(binds), row_to_experience)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// 按 id 取单条（管理面 validate / delete 回读用；已软删的条目视为不存在）。
    pub fn get(&self, id: i64) -> Result<Option<Experience>> {
        let db = self.db()?;
        let mut stmt = db.prepare(&format!(
            "SELECT {EXPERIENCE_COLUMNS} FROM experience_ledger
             WHERE id = ?1 AND deleted_at IS NULL"
        ))?;
        let mut rows = stmt.query_map([id], row_to_experience)?;
        Ok(rows.next().and_then(|r| r.ok()))
    }

    /// 标记某条经验已验证（踩坑闭环：修法被真实运行验证通过）
    pub fn mark_validated(&mut self, id: i64) -> Result<()> {
        self.set_validated(id, true).map(|_| ())
    }

    /// 设置验证状态；返回是否真的存在该条目（未知 id 不谎称成功）。
    pub fn set_validated(&mut self, id: i64, validated: bool) -> Result<bool> {
        let db = self.db()?;
        let flag: i64 = if validated { 1 } else { 0 };
        let changed = db.execute(
            "UPDATE experience_ledger SET validated = ?1 WHERE id = ?2 AND deleted_at IS NULL",
            rusqlite::params![flag, id],
        )?;
        // 低频管理动作：索引直接作废，下次 search 重建
        *self.index.borrow_mut() = None;
        Ok(changed > 0)
    }

    /// 软删单条：写入 `deleted_at` 时间戳（检索/列表自动排除），返回删除时间。
    ///
    /// `Ok(None)` = 条目不存在或已删除（调用方据此返回 `NOT_FOUND`）。
    pub fn soft_delete(&mut self, id: i64) -> Result<Option<String>> {
        let db = self.db()?;
        let now = Utc::now().to_rfc3339();
        let changed = db.execute(
            "UPDATE experience_ledger SET deleted_at = ?1
             WHERE id = ?2 AND deleted_at IS NULL",
            rusqlite::params![now, id],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        *self.index.borrow_mut() = None;
        Ok(Some(now))
    }

    /// 批量软删：返回 `(实际删除数, 不存在的 id 列表)`。
    pub fn delete_ids(&mut self, ids: &[i64]) -> Result<(usize, Vec<i64>)> {
        let mut deleted = 0usize;
        let mut missing: Vec<i64> = Vec::new();
        for id in ids {
            match self.soft_delete(*id)? {
                Some(_) => deleted += 1,
                None => missing.push(*id),
            }
        }
        Ok((deleted, missing))
    }

    /// 未删除条目总数（经验层计数；软删条目不计入）。
    pub fn count(&self) -> Result<usize> {
        let db = self.db()?;
        let n: i64 = db.query_row(
            "SELECT COUNT(*) FROM experience_ledger WHERE deleted_at IS NULL",
            [],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// 已软删条目数。
    pub fn count_deleted(&self) -> Result<usize> {
        let db = self.db()?;
        let n: i64 = db.query_row(
            "SELECT COUNT(*) FROM experience_ledger WHERE deleted_at IS NOT NULL",
            [],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// 待验证（`validated = 0` 且未删除）条目数。
    pub fn count_pending_validation(&self) -> Result<usize> {
        let db = self.db()?;
        let n: i64 = db.query_row(
            "SELECT COUNT(*) FROM experience_ledger
             WHERE validated = 0 AND deleted_at IS NULL",
            [],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// 按类型统计未删除条目数（管理面板概览）。
    pub fn count_by_kind(&self, kind: ExperienceKind) -> Result<usize> {
        let db = self.db()?;
        let n: i64 = db.query_row(
            "SELECT COUNT(*) FROM experience_ledger
             WHERE kind = ?1 AND deleted_at IS NULL",
            rusqlite::params![kind.as_str()],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }
}

/// 管理面查询列（顺序必须与 [`row_to_experience`] 的下标一致）。
const EXPERIENCE_COLUMNS: &str =
    "id, kind, title, body, tags, signature, validated, created_at, source, actor, confidence, status, session_key";

/// 统一行映射：治理列用 `Option` 读取，兼容迁移前遗留的 NULL 值。
fn row_to_experience(row: &rusqlite::Row<'_>) -> rusqlite::Result<Experience> {
    Ok(Experience {
        id: row.get(0)?,
        kind: ExperienceKind::from_kind_str(&row.get::<_, String>(1)?),
        title: row.get(2)?,
        body: row.get(3)?,
        tags: row
            .get::<_, String>(4)?
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        signature: row.get(5)?,
        validated: row.get::<_, i64>(6)? != 0,
        created_at: row.get(7)?,
        source: row
            .get::<_, Option<String>>(8)?
            .unwrap_or_else(|| "agent".to_string()),
        actor: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
        confidence: row.get::<_, Option<f64>>(10)?.unwrap_or(0.6),
        status: row
            .get::<_, Option<String>>(11)?
            .unwrap_or_else(|| "draft".to_string()),
        session_key: row.get::<_, Option<String>>(12)?.unwrap_or_default(),
    })
}

/// 自进化编排器：沉淀 + 召回闭环
///
/// - 沉淀（`consolidate_session`）：把会话有价值内容提升为长期记忆 + 经验台账
/// - 召回（`recall`）：执行前先读历史经验，让 Agent 越用越聪明
pub struct SelfEvolution {
    pub ledger: ExperienceLedger,
    pub long_term: LongTermMemory,
}

impl SelfEvolution {
    pub fn new(ledger: ExperienceLedger, long_term: LongTermMemory) -> Self {
        Self { ledger, long_term }
    }

    /// 会话沉淀：长期记忆 + 经验台账双重固化
    ///
    /// 返回沉淀条数。幂等（同内容按签名去重），可安全周期调用。
    pub fn consolidate_session(&mut self, sensory: &SensoryMemory) -> Result<usize> {
        let mut n = self.long_term.consolidate(sensory)?;
        for msg in sensory.get_all_messages() {
            let (role, content) = match msg {
                Message::User { content, .. } => ("user", content.clone()),
                Message::Assistant { content, .. } => ("assistant", content.clone()),
                Message::System { content, .. } => ("system", content.clone()),
                Message::Tool { output, .. } => ("tool", output.clone()),
            };
            if content.chars().count() < 20 {
                continue; // 过短内容不值得沉淀
            }
            let sig = format!("msg:{:x}", simple_hash(&content));
            self.ledger.record(
                ExperienceKind::Pattern,
                &format!("{role} 发言"),
                &content,
                &[role.to_string()],
                &sig,
            )?;
            n += 1;
        }
        Ok(n)
    }

    /// 召回与查询相关的经验（自进化：执行前先读历史）
    pub fn recall(&self, query: &str, limit: usize) -> Result<Vec<Experience>> {
        self.ledger.search(query, limit)
    }
}

/// 轻量 FNV-1a 哈希（用于经验签名，零依赖）
fn simple_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;
    use chrono::Utc;

    fn temp_ledger(name: &str) -> (ExperienceLedger, PathBuf) {
        let p = std::env::temp_dir().join(format!("mgt_exp_{name}_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let mut l = ExperienceLedger::new(&p);
        l.init().unwrap();
        (l, p)
    }

    #[test]
    fn test_record_and_search() {
        let (mut l, p) = temp_ledger("search");
        l.record(
            ExperienceKind::Pitfall,
            "Tauri 全局快捷键注册",
            "on_shortcut 返回 Result 不可链式，需在 setup 内注册并带 handler",
            &["tauri".into(), "shortcut".into()],
            "sig:tauri-shortcut",
        )
        .unwrap();

        let results = l.search("tauri 快捷键 注册", 5).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].kind, ExperienceKind::Pitfall);
        assert!(results[0].body.contains("on_shortcut"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn test_record_idempotent_by_signature() {
        let (mut l, p) = temp_ledger("idem");
        let id1 = l
            .record(
                ExperienceKind::Pattern,
                "复用 LocalEmbedding",
                "经验台账应复用 long_term 的特征哈希嵌入",
                &["memory".into()],
                "sig:reuse-embed",
            )
            .unwrap();
        let id2 = l
            .record(
                ExperienceKind::Pattern,
                "复用 LocalEmbedding（修订）",
                "经验台账应复用 long_term 的特征哈希嵌入（含 dim()）",
                &["memory".into(), "refactor".into()],
                "sig:reuse-embed",
            )
            .unwrap();
        // 同签名应更新而非新增
        assert_eq!(id1, id2);
        assert_eq!(l.count().unwrap(), 1);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn test_search_no_match_below_threshold() {
        let (mut l, p) = temp_ledger("nomatch");
        l.record(
            ExperienceKind::Decision,
            "Cargo.toml 约束",
            "Phase C 放开 Cargo.toml 约束以引入 GUI 依赖",
            &["build".into()],
            "sig:cargo",
        )
        .unwrap();
        let results = l.search("披萨 烘焙 番茄", 5).unwrap();
        assert!(results.is_empty());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn test_mark_validated() {
        let (mut l, p) = temp_ledger("valid");
        let id = l
            .record(
                ExperienceKind::Pitfall,
                "Image::from_path 需 image-png feature",
                "Tauri Image 的 PNG 解码被 feature 门控，需开启 image-png",
                &["tauri".into()],
                "sig:image-png",
            )
            .unwrap();
        assert!(!l.search("image png feature", 1).unwrap()[0].validated);
        l.mark_validated(id).unwrap();
        assert!(l.search("image png feature", 1).unwrap()[0].validated);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn test_list_by_kind() {
        let (mut l, p) = temp_ledger("kind");
        l.record(ExperienceKind::Pitfall, "P1", "x", &[], "s1")
            .unwrap();
        l.record(ExperienceKind::Decision, "D1", "y", &[], "s2")
            .unwrap();
        l.record(ExperienceKind::Pattern, "Pa1", "z", &[], "s3")
            .unwrap();
        assert_eq!(
            l.list_by_kind(ExperienceKind::Pitfall, 10).unwrap().len(),
            1
        );
        assert_eq!(
            l.list_by_kind(ExperienceKind::Decision, 10).unwrap().len(),
            1
        );
        assert_eq!(
            l.list_by_kind(ExperienceKind::Pattern, 10).unwrap().len(),
            1
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn test_consolidate_session() {
        let tmp = std::env::temp_dir().join(format!("mgt_exp_cons_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let ldb = tmp.join("ledger.db");
        let lt = tmp.join("ltm.db");
        let _ = std::fs::remove_file(&ldb);
        let _ = std::fs::remove_file(&lt);

        let mut ledger = ExperienceLedger::new(&ldb);
        ledger.init().unwrap();
        let mut long_term = LongTermMemory::new(&lt);
        long_term.init().unwrap();
        let mut evo = SelfEvolution::new(ledger, long_term);

        let mut sensory = SensoryMemory::new("sess-1");
        let now = Utc::now();
        sensory
            .add_message(Message::User {
                content: "如何配置 Tauri 的全局快捷键与托盘".to_string(),
                timestamp: now,
            })
            .unwrap();
        sensory
            .add_message(Message::Assistant {
                content: "在 setup 内用 global_shortcut().on_shortcut 注册，托盘用 TrayIconBuilder"
                    .to_string(),
                timestamp: now,
                thinking: None,
            })
            .unwrap();

        let n = evo.consolidate_session(&sensory).unwrap();
        assert!(n >= 2, "应至少沉淀 2 条（用户+助手）");
        let recalled = evo.recall("Tauri 托盘 全局快捷键", 10).unwrap();
        assert!(!recalled.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_index_reflects_record_upsert() {
        // 索引加载后按签名 upsert：必须原地替换（新正文可检索、旧正文不可见）
        let tmp = std::env::temp_dir().join(format!("exp_idx_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let dbp = tmp.join("ledger.db");
        let _ = std::fs::remove_file(&dbp);
        let mut l = ExperienceLedger::new(&dbp);
        l.init().unwrap();
        l.record(
            ExperienceKind::Pitfall,
            "坑",
            "旧内容：同步阻塞 tokio worker",
            &[],
            "sig-1",
        )
        .unwrap();
        assert!(!l.search("tokio worker", 5).unwrap().is_empty()); // 触发索引加载
        l.record(
            ExperienceKind::Pitfall,
            "坑",
            "新内容：spawn_blocking 卸载阻塞调用",
            &[],
            "sig-1",
        )
        .unwrap();
        let hits = l.search("spawn_blocking", 5).unwrap();
        assert!(
            hits.iter().any(|e| e.body.contains("新内容")),
            "upsert 后索引应反映新正文"
        );
        assert!(hits.iter().all(|e| !e.body.contains("旧内容")));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_ledger_bounded_by_cap() {
        // B3 回归：每轮会话把每条 ≥20 字符消息沉淀为 Pattern，旧实现无上限 → DB 无界增长。
        let (mut l, p) = temp_ledger("cap");
        let n = MAX_EXPERIENCE_ROWS + 50;
        for i in 0..n {
            l.record(
                ExperienceKind::Pattern,
                &format!("p{i}"),
                &format!("内容 {i} 用于验证台账有界裁剪"),
                &[],
                &format!("sig-{i}"),
            )
            .unwrap();
        }
        assert_eq!(
            l.count().unwrap(),
            MAX_EXPERIENCE_ROWS,
            "台账条数应被上限约束"
        );
        let recent = l.list_recent(5).unwrap();
        assert!(
            recent.iter().any(|e| e.title == format!("p{}", n - 1)),
            "最新条目不得被裁剪"
        );
        let all = l.list_recent(MAX_EXPERIENCE_ROWS + 100).unwrap();
        assert!(
            all.iter().all(|e| e.title != "p0"),
            "最旧条目应被裁剪"
        );
        let _ = std::fs::remove_file(&p);
    }

    /// 关闭 `memory.auto_capture_enabled` 后：显式写入路径（`memory.save` 的底层
    /// `record`）**仍然落库** —— 该开关只约束「自动捕获」，不约束用户显式意图。
    #[test]
    fn explicit_record_ignores_auto_capture_switch() {
        let cfg_dir = std::env::temp_dir().join(format!(
            "mgt_exp_acoff_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&cfg_dir).unwrap();
        crate::persistence::config::Config::set_config_dir_override(cfg_dir.clone());
        let mut cfg = crate::persistence::config::Config::default();
        cfg.memory.auto_capture_enabled = false;
        cfg.save().unwrap();
        let loaded = crate::persistence::config::Config::load().unwrap();
        assert!(!loaded.memory.auto_capture_enabled, "开关必须落盘为 false");

        let (mut l, p) = temp_ledger("explicit_ac");
        let id = l
            .record(
                ExperienceKind::Pitfall,
                "显式写入",
                "自动捕获关闭时，用户显式保存的记忆仍必须落库。",
                &[],
                "sig:explicit",
            )
            .unwrap();
        assert!(l.get(id).unwrap().is_some(), "显式写入不应被开关拦住");
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_dir_all(&cfg_dir);
    }

    #[test]
    fn test_governance_meta_is_persisted_and_filterable() {
        let (mut l, p) = temp_ledger("gov");
        let meta = ExperienceMeta {
            source: "user".to_string(),
            actor: "sess-42".to_string(),
            confidence: 0.91,
            status: "active".to_string(),
            session_key: "sess:42".to_string(),
        };
        let id = l
            .record_meta(
                ExperienceKind::Decision,
                "治理字段",
                "来源/主体/置信度/状态必须落库，管理面才能按状态过滤。",
                &["gov".into()],
                "sig:gov",
                &meta,
            )
            .unwrap();
        let row = l.get(id).unwrap().expect("条目存在");
        assert_eq!(row.source, "user");
        assert_eq!(row.actor, "sess-42");
        assert!((row.confidence - 0.91).abs() < 1e-9);
        assert_eq!(row.status, "active");
        assert_eq!(row.session_key, "sess:42");

        let hits = l
            .list_entries(Some("decision"), Some("active"), 50, 0)
            .unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].id, id);
        // 不匹配的过滤条件返回空（不是忽略过滤条件）
        assert!(l
            .list_entries(Some("decision"), Some("draft"), 50, 0)
            .unwrap()
            .is_empty());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn test_validate_and_delete_unknown_ids() {
        let (mut l, p) = temp_ledger("govdel");
        let id = l
            .record(
                ExperienceKind::Pitfall,
                "待验证",
                "内容用于验证 memory.validate / memory.delete 的真实写入。",
                &[],
                "sig:gov-del",
            )
            .unwrap();
        assert!(l.set_validated(id, true).unwrap());
        assert!(l.get(id).unwrap().unwrap().validated);
        assert!(!l.set_validated(id + 9999, true).unwrap(), "未知 id 不得谎称成功");

        let (deleted, missing) = l.delete_ids(&[id, id + 9999]).unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(missing, vec![id + 9999]);
        assert!(l.get(id).unwrap().is_none(), "软删后按 id 读取应视为不存在");
        assert_eq!(l.count_deleted().unwrap(), 1);
        let _ = std::fs::remove_file(&p);
    }

    /// 软删：检索/列表立即不可见，但行仍保留（`deleted_at` 可追溯）。
    #[test]
    fn test_soft_delete_hides_from_search_and_lists() {
        let (mut l, p) = temp_ledger("softdel");
        let id = l
            .record(
                ExperienceKind::Pitfall,
                "软删探针",
                "这条经验用于验证软删后检索不到，但数据库行仍可被统计。",
                &[],
                "sig:soft-del",
            )
            .unwrap();
        assert!(!l.search("软删探针", 5).unwrap().is_empty());
        let deleted_at = l.soft_delete(id).unwrap().expect("首次软删应成功");
        assert!(!deleted_at.is_empty());
        assert!(l.soft_delete(id).unwrap().is_none(), "重复软删不得谎称成功");
        assert!(l.search("软删探针", 5).unwrap().is_empty(), "软删后检索不到");
        assert!(l.list_recent(10).unwrap().is_empty(), "软删后列表不返回");
        assert!(l.list_entries(None, None, 10, 0).unwrap().is_empty());
        assert_eq!(l.count().unwrap(), 0);
        assert_eq!(l.count_deleted().unwrap(), 1);
        let _ = std::fs::remove_file(&p);
    }

    /// 幂等迁移：升级前建库（无治理列）→ init 自动补列，重复 init 不报错。
    #[test]
    fn test_governance_columns_migrate_idempotently() {
        let p = std::env::temp_dir().join(format!(
            "mgt_exp_legacy_{}_{}.db",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        {
            let db = Connection::open(&p).unwrap();
            db.execute_batch(
                "CREATE TABLE experience_ledger (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    kind TEXT NOT NULL,
                    title TEXT NOT NULL,
                    body TEXT NOT NULL,
                    tags TEXT DEFAULT '',
                    signature TEXT DEFAULT '',
                    vector TEXT NOT NULL,
                    validated INTEGER DEFAULT 0,
                    created_at TEXT NOT NULL
                );",
            )
            .unwrap();
        }
        let mut l = ExperienceLedger::new(&p);
        l.init().unwrap();
        // 再次 init：列已存在，迁移必须是 no-op（不是 duplicate column 报错）
        l.init().unwrap();
        let id = l
            .record(
                ExperienceKind::Pattern,
                "旧库",
                "旧库迁移后仍可正常写入并读出治理字段默认值。",
                &[],
                "sig:legacy",
            )
            .unwrap();
        let row = l.get(id).unwrap().unwrap();
        assert_eq!(row.source, "agent");
        assert_eq!(row.status, "draft");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn test_ledger_prune_keeps_validated() {
        // 裁剪应「最旧 + 未验证优先」：已验证的高价值经验在容量充足时不被删除。
        let (mut l, p) = temp_ledger("capvalid");
        let keep = l
            .record(
                ExperienceKind::Pitfall,
                "重要踩坑",
                "已验证的根因与修法",
                &[],
                "sig-keep",
            )
            .unwrap();
        l.mark_validated(keep).unwrap();
        for i in 0..(MAX_EXPERIENCE_ROWS + 50) {
            l.record(
                ExperienceKind::Pattern,
                &format!("p{i}"),
                &format!("填充内容 {i}"),
                &[],
                &format!("s{i}"),
            )
            .unwrap();
        }
        let all = l.list_recent(MAX_EXPERIENCE_ROWS + 100).unwrap();
        assert!(
            all.iter().any(|e| e.id == keep),
            "已验证条目不应被裁剪: {} 条",
            all.len()
        );
        let _ = std::fs::remove_file(&p);
    }
}
