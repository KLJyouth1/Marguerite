//! 协同会话持久化（Track G 地基）
//!
//! 协同会话（collab session）的元信息存储：哪一个 owner 创建了该会话、创建 / 更新时间。
//! 仅保存元数据；文档 CRDT 内容本身由 [`crate::collab::crdt`] 管理，不在此处落库。
//! 连接 / 建表模式与 [`crate::persistence::session`] 的 `SessionManager` 对齐。

use std::path::Path;

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;

/// 协同会话元信息
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollabSession {
    /// 会话唯一 ID
    pub id: String,
    /// 创建者（owner）ID
    pub owner_id: String,
    /// 创建时间（RFC3339）
    pub created_at: String,
    /// 更新时间（RFC3339）
    pub updated_at: String,
}

/// 协同会话存储：包裹 rusqlite 连接，提供建表、创建、存在性查询。
pub struct CollabSessionStore {
    conn: Connection,
}

impl CollabSessionStore {
    /// 打开内存库（测试 / 临时会话），并初始化表结构。
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let mut store = Self { conn };
        store.init()?;
        Ok(store)
    }

    /// 打开文件库（持久化会话），并初始化表结构。
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
                id TEXT PRIMARY KEY,
                owner_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );",
        )?;
        Ok(())
    }

    /// 创建协同会话。重复 ID 会因主键约束返回错误（调用方应先查 `exists`）。
    pub fn create(&self, id: &str, owner_id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO collab_session (id, owner_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![id, owner_id, now, now],
        )?;
        Ok(())
    }

    /// 会话是否存在。
    pub fn exists(&self, id: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM collab_session WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count > 0)
    }

    /// 读取会话元信息（不存在返回 None）。
    pub fn get(&self, id: &str) -> Result<Option<CollabSession>> {
        let result = self.conn.query_row(
            "SELECT id, owner_id, created_at, updated_at FROM collab_session WHERE id = ?1",
            rusqlite::params![id],
            |row| {
                Ok(CollabSession {
                    id: row.get(0)?,
                    owner_id: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            },
        );
        match result {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_then_exists_true() {
        let store = CollabSessionStore::open_in_memory().unwrap();
        assert!(!store.exists("s1").unwrap());
        store.create("s1", "owner-1").unwrap();
        assert!(store.exists("s1").unwrap());
        let s = store.get("s1").unwrap().expect("应存在");
        assert_eq!(s.owner_id, "owner-1");
        assert!(!s.created_at.is_empty());
        assert!(!s.updated_at.is_empty());
    }

    #[test]
    fn duplicate_create_is_idempotent_safe() {
        // 不变式：重复 create 返回错误，但已存在的会话依然可读（不静默覆盖、不损坏）。
        let store = CollabSessionStore::open_in_memory().unwrap();
        store.create("s1", "owner-1").unwrap();
        assert!(store.create("s1", "owner-2").is_err(), "重复主键应报错");
        assert!(store.exists("s1").unwrap());
        let s = store.get("s1").unwrap().expect("应存在");
        assert_eq!(s.owner_id, "owner-1", "重复创建不得覆盖既有 owner");
    }

    #[test]
    fn missing_session_returns_none() {
        let store = CollabSessionStore::open_in_memory().unwrap();
        assert!(store.get("nope").unwrap().is_none());
    }

    #[test]
    fn multiple_sessions_isolated() {
        let store = CollabSessionStore::open_in_memory().unwrap();
        store.create("a", "o1").unwrap();
        store.create("b", "o2").unwrap();
        assert!(store.exists("a").unwrap());
        assert!(store.exists("b").unwrap());
        assert!(!store.exists("c").unwrap());
    }
}
