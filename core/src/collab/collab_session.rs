//! 协同会话持久化（Track G 地基，G4）
//!
//! 协同会话（collab session）元信息存储：哪一个 owner 创建了该会话、关联哪个协同文档、
//! 创建时间。仅保存元数据；文档 CRDT 内容本身由 [`crate::collab::crdt`] 管理，不在此落库。
//! 建表 / 连接方式对齐 [`crate::persistence::session`] 的 `SessionManager` 模式（rusqlite + 懒初始化）。

use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

/// 协同会话元信息
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollabSessionRecord {
    /// 会话唯一 ID
    pub session_id: String,
    /// 关联协同文档 ID（对应 `crdt::CollabDoc` 的逻辑标识）
    pub doc_id: String,
    /// 创建者（owner）ID
    pub owner: String,
    /// 创建时间（epoch millis）
    pub created_at: i64,
}

/// 协同会话存储：包裹 rusqlite 连接，提供建表、创建、按 owner 列举。
pub struct CollabSessionStore {
    conn: Connection,
}

impl CollabSessionStore {
    /// 打开内存库（测试 / 临时会话）并初始化表结构。
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let mut store = Self { conn };
        store.init()?;
        Ok(store)
    }

    /// 打开文件库（持久化会话）并初始化表结构。
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        let mut store = Self { conn };
        store.init()?;
        Ok(store)
    }

    /// 初始化 `collab_session` 表（幂等）。
    fn init(&mut self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS collab_session (
                session_id TEXT PRIMARY KEY,
                doc_id      TEXT NOT NULL,
                owner       TEXT NOT NULL,
                created_at  INTEGER NOT NULL
            );",
        )?;
        Ok(())
    }

    /// 创建协同会话。重复 `session_id` 会因主键约束返回错误（调用方应先查 `get_collab_session`）。
    pub fn create_collab_session(
        &self,
        session_id: &str,
        doc_id: &str,
        owner: &str,
    ) -> Result<CollabSessionRecord> {
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        self.conn.execute(
            "INSERT INTO collab_session (session_id, doc_id, owner, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, doc_id, owner, created_at],
        )?;
        Ok(CollabSessionRecord {
            session_id: session_id.to_string(),
            doc_id: doc_id.to_string(),
            owner: owner.to_string(),
            created_at,
        })
    }

    /// 按 owner 列举其创建的全部协同会话（按创建时间倒序）。
    pub fn list_collab_sessions(&self, owner: &str) -> Result<Vec<CollabSessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, doc_id, owner, created_at
             FROM collab_session WHERE owner = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![owner], |row| {
                Ok(CollabSessionRecord {
                    session_id: row.get(0)?,
                    doc_id: row.get(1)?,
                    owner: row.get(2)?,
                    created_at: row.get::<_, i64>(3)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// 读取单个会话元信息（不存在返回 None）。
    pub fn get_collab_session(&self, session_id: &str) -> Result<Option<CollabSessionRecord>> {
        let res = self.conn.query_row(
            "SELECT session_id, doc_id, owner, created_at FROM collab_session WHERE session_id = ?1",
            rusqlite::params![session_id],
            |row| {
                Ok(CollabSessionRecord {
                    session_id: row.get(0)?,
                    doc_id: row.get(1)?,
                    owner: row.get(2)?,
                    created_at: row.get::<_, i64>(3)?,
                })
            },
        );
        match res {
            Ok(r) => Ok(Some(r)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_then_list_by_owner() {
        let store = CollabSessionStore::open_in_memory().unwrap();
        store.create_collab_session("s1", "d1", "alice").unwrap();
        store.create_collab_session("s2", "d2", "alice").unwrap();
        store.create_collab_session("s3", "d3", "bob").unwrap();

        let alice = store.list_collab_sessions("alice").unwrap();
        assert_eq!(alice.len(), 2);
        assert!(alice.iter().all(|r| r.owner == "alice"));

        let bob = store.list_collab_sessions("bob").unwrap();
        assert_eq!(bob.len(), 1);
        assert_eq!(bob[0].doc_id, "d3");
        assert!(bob[0].created_at > 0);
    }

    #[test]
    fn get_collab_session_roundtrip() {
        let store = CollabSessionStore::open_in_memory().unwrap();
        let rec = store.create_collab_session("s1", "d1", "alice").unwrap();
        let got = store.get_collab_session("s1").unwrap().expect("应存在");
        assert_eq!(got, rec);
        assert!(store.get_collab_session("missing").unwrap().is_none());
    }

    #[test]
    fn duplicate_session_is_error() {
        let store = CollabSessionStore::open_in_memory().unwrap();
        store.create_collab_session("s1", "d1", "alice").unwrap();
        assert!(
            store.create_collab_session("s1", "d2", "bob").is_err(),
            "重复主键应报错且不覆盖"
        );
    }
}
