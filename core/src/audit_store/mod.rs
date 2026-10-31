//! Independent append-only SQLite audit log with SHA-256 hash chaining.

use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Fixed predecessor for the first audit row.
pub const GENESIS_HASH: &str = "genesis";

/// WP5: offline / air-gapped egress policy decision logic.
pub mod airgap;
/// WP5: offline CSV serialization of audit entries.
pub mod export;

/// Business fields recorded by the audit log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    pub timestamp: String,
    pub actor: String,
    pub method: String,
    pub target: String,
    pub result: String,
}

impl AuditRecord {
    pub fn new(
        timestamp: impl Into<String>,
        actor: impl Into<String>,
        method: impl Into<String>,
        target: impl Into<String>,
        result: impl Into<String>,
    ) -> Self {
        Self {
            timestamp: timestamp.into(),
            actor: actor.into(),
            method: method.into(),
            target: target.into(),
            result: result.into(),
        }
    }
}

/// A persisted row, including its generated id and chain values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAuditRecord {
    pub id: i64,
    pub timestamp: String,
    pub actor: String,
    pub method: String,
    pub target: String,
    pub result: String,
    pub prev_hash: String,
    pub hash: String,
}

/// SQLite-backed append-only audit store. The database is always a separate audit.db file.
pub struct AuditStore {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl AuditStore {
    /// Open the default independent audit database at Config::data_dir()/audit.db.
    pub fn open_default() -> Result<Self> {
        let path = crate::persistence::config::Config::data_dir().join("audit.db");
        Self::open(path)
    }

    /// Open or create an audit database at an explicitly injected path (useful for tests).
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(&path)?;
        // W3（2026-10-30）：**只借 WAL，不降 synchronous**。
        // WAL 省掉的是「每条语句建一个 journal、提交后再删掉」这一整轮额外写，
        // 而 FULL 保留的是「最后一条已提交的审计不能在掉电时消失」——那是安全属性，
        // 不是性能参数，所以这里刻意不与 sessions/usage 两处对齐。
        // 下面两个 RAISE(ABORT) 触发器（禁 UPDATE / 禁 DELETE）同样**不动**。
        crate::persistence::apply_sqlite_write_pragmas(&conn, "audit.db", "FULL");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS audit_log (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 timestamp TEXT NOT NULL,
                 actor TEXT NOT NULL,
                 method TEXT NOT NULL,
                 target TEXT NOT NULL,
                 result TEXT NOT NULL,
                 prev_hash TEXT NOT NULL,
                 hash TEXT NOT NULL
             );
             CREATE TRIGGER IF NOT EXISTS audit_log_no_update
             BEFORE UPDATE ON audit_log BEGIN
                 SELECT RAISE(ABORT, 'audit log is append-only');
             END;
             CREATE TRIGGER IF NOT EXISTS audit_log_no_delete
             BEFORE DELETE ON audit_log BEGIN
                 SELECT RAISE(ABORT, 'audit log is append-only');
             END;",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
            path,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 底层连接访问器（供同库消费者读取日志，如 `events::product::recent`）。
    pub fn conn(&self) -> &Mutex<Connection> {
        &self.conn
    }

    /// Append one row, linking it to the previous row's hash.
    pub fn append(&self, record: AuditRecord) -> Result<StoredAuditRecord> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("audit database lock poisoned"))?;
        let tx = conn.transaction()?;
        let previous: Option<(i64, String)> = tx
            .query_row(
                "SELECT id, hash FROM audit_log ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let prev_hash = previous
            .map(|(_, hash)| hash)
            .unwrap_or_else(|| GENESIS_HASH.to_owned());
        let id = tx.query_row(
            "SELECT COALESCE(MAX(id), 0) + 1 FROM audit_log",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        let hash = hash_payload(id, &record, &prev_hash);
        tx.execute(
            "INSERT INTO audit_log (id,timestamp,actor,method,target,result,prev_hash,hash) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![id, record.timestamp, record.actor, record.method, record.target, record.result, prev_hash, hash],
        )?;
        tx.commit()?;
        Ok(StoredAuditRecord {
            id,
            timestamp: record.timestamp,
            actor: record.actor,
            method: record.method,
            target: record.target,
            result: record.result,
            prev_hash,
            hash,
        })
    }

    /// Verify row order, predecessor links, and every row hash.
    pub fn verify_chain(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("audit database lock poisoned"))?;
        let mut stmt = conn.prepare("SELECT id,timestamp,actor,method,target,result,prev_hash,hash FROM audit_log ORDER BY id ASC")?;
        let mut rows = stmt.query([])?;
        let mut expected_id = 1_i64;
        let mut previous_hash = GENESIS_HASH.to_owned();
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let record = AuditRecord::new(
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            );
            let prev_hash: String = row.get(6)?;
            let hash: String = row.get(7)?;
            if id != expected_id {
                return Err(anyhow!("audit chain row order is invalid at id {id}"));
            }
            if prev_hash != previous_hash {
                return Err(anyhow!("audit chain predecessor mismatch at id {id}"));
            }
            let expected_hash = hash_payload(id, &record, &prev_hash);
            if hash != expected_hash {
                return Err(anyhow!("audit chain hash mismatch at id {id}"));
            }
            previous_hash = hash;
            expected_id += 1;
        }
        Ok(())
    }

    pub fn len(&self) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("audit database lock poisoned"))?;
        Ok(conn.query_row("SELECT count(*) FROM audit_log", [], |row| row.get(0))?)
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }
}

fn hash_payload(id: i64, record: &AuditRecord, prev_hash: &str) -> String {
    let mut payload = String::new();
    for value in [
        id.to_string(),
        record.timestamp.clone(),
        record.actor.clone(),
        record.method.clone(),
        record.target.clone(),
        record.result.clone(),
        prev_hash.to_owned(),
    ] {
        payload.push_str(&value.len().to_string());
        payload.push(':');
        payload.push_str(&value);
        payload.push('|');
    }
    let digest = Sha256::digest(payload.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
