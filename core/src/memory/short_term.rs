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
    /// 创建短期记忆
    pub fn new(path: &Path) -> Self {
        Self {
            db_path: path.to_path_buf(),
            db: None,
            max_sessions: 50,
        }
    }

    /// 初始化数据库和 FTS5 索引
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
                updated_at TEXT NOT NULL
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS short_term_sessions_fts USING fts5(
                session_id, summary, content='short_term_sessions', content_rowid='rowid'
            );",
        )?;
        // 迁移：旧库的 `sessions` 表 → `short_term_sessions`（schema 漂移修复，幂等）
        let has_old_table: bool = db
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='sessions'")
            .map(|mut stmt| stmt.exists([]).unwrap_or(false))
            .unwrap_or(false);
        if has_old_table {
            db.execute_batch(
                "INSERT OR IGNORE INTO short_term_sessions
                    (session_id, summary, state, message_count, tokens_used, created_at, updated_at)
                 SELECT session_id, summary, COALESCE(state, 'Ready'), message_count, 0, created_at, updated_at
                 FROM sessions;
                 DROP TABLE IF EXISTS sessions_fts;
                 DROP TABLE sessions;",
            )?;
            // 为迁移的数据重建 FTS 索引
            db.execute_batch(
                "INSERT OR IGNORE INTO short_term_sessions_fts (rowid, session_id, summary)
                 SELECT rowid, session_id, summary FROM short_term_sessions;",
            )?;
        }
        self.db = Some(db);
        // 一次性重建外部内容 FTS 索引，清理历史遗留的孤儿 FTS 行。
        // 旧实现在 REPLACE 时只 INSERT 不 DELETE，每轮沉淀都孤儿一行、无限增长；
        // `'rebuild'` 是外部内容 FTS5 官方指令，按内容表整体重建，幂等且能自愈。
        {
            let db = self.db()?;
            db.execute_batch(
                "INSERT INTO short_term_sessions_fts(short_term_sessions_fts) VALUES('rebuild');",
            )?;
        }
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

        // 先撤销旧行的 FTS 条目。外部内容 FTS5 **必须**用 `'delete'` 指令，直接
        // `DELETE`/`UPDATE` 不被支持。历史缺陷：只 INSERT 不 DELETE —— session_id 是
        // 文本主键，`INSERT OR REPLACE` 会换掉 rowid，旧 FTS 行遂成孤儿，每次沉淀
        // 泄漏一行、永不回收（跨轮无限增长）。
        if let Some((old_rowid, old_summary)) = db
            .query_row(
                "SELECT rowid, summary FROM short_term_sessions WHERE session_id = ?1",
                rusqlite::params![session_id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
        {
            db.execute(
                "INSERT INTO short_term_sessions_fts(short_term_sessions_fts, rowid, session_id, summary)
                 VALUES('delete', ?1, ?2, ?3)",
                rusqlite::params![old_rowid, session_id, old_summary],
            )?;
        }

        db.execute(
            "INSERT OR REPLACE INTO short_term_sessions (session_id, summary, message_count, created_at, updated_at)
             VALUES (?1, ?2, ?3, COALESCE((SELECT created_at FROM short_term_sessions WHERE session_id = ?1), ?4), ?4)",
            rusqlite::params![session_id, summary, message_count as i32, now],
        )?;

        // 同步 FTS 索引（新行）
        db.execute(
            "INSERT INTO short_term_sessions_fts (rowid, session_id, summary)
             VALUES ((SELECT rowid FROM short_term_sessions WHERE session_id = ?1), ?1, ?2)",
            rusqlite::params![session_id, summary],
        )?;

        // 强制最大会话数限制
        self.enforce_max_sessions()?;

        Ok(())
    }

    /// FTS5 全文搜索
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SessionSummary>> {
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
            .query_map(rusqlite::params![query, limit as i64], |row| {
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
        // 再删内容行 —— 顺序不能反，否则取不到旧 rowid/summary。
        if let Some((rowid, summary)) = db
            .query_row(
                "SELECT rowid, summary FROM short_term_sessions WHERE session_id = ?1",
                rusqlite::params![session_id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
        {
            db.execute(
                "INSERT INTO short_term_sessions_fts(short_term_sessions_fts, rowid, session_id, summary)
                 VALUES('delete', ?1, ?2, ?3)",
                rusqlite::params![rowid, session_id, summary],
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
    fn enforce_max_sessions(&self) -> Result<()> {
        let db = self.db()?;
        let count = self.session_count()?;
        if count > self.max_sessions {
            // 删除最旧的会话
            db.execute(
                "DELETE FROM short_term_sessions WHERE session_id IN (
                    SELECT session_id FROM short_term_sessions ORDER BY updated_at ASC LIMIT ?
                )",
                rusqlite::params![(count - self.max_sessions) as i64],
            )?;
            // 重建 FTS 索引（外部内容表用官方 `'rebuild'` 指令，不能用直接 DELETE）
            db.execute_batch(
                "INSERT INTO short_term_sessions_fts(short_term_sessions_fts) VALUES('rebuild');",
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
}
