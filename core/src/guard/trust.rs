//! 信任校准（M2-C5 Trust Calibration）
//!
//! 对「高风险动作确认卡」按「工具 × 模式 × 会话」做三级信任记忆，命中信任则免确认放行：
//! - [`TrustLevel::Low`]   记忆授权：落盘、跨会话生效、可随时撤销
//! - [`TrustLevel::Medium`] 会话信任：仅当前会话内有效、不落盘（会话结束即失效）
//! - [`TrustLevel::High`]   永久信任：落盘、跨会话生效、可随时撤销
//! - 默认（未授予任何信任）= 0-确认：每次高风险动作都走确认卡，fail-closed
//!
//! 覆盖 5 个 Mode（Plan / Agent / Ensemble / Bloom / OmniTranscend）：信任条目可按
//! 模式限定（`mode == Some(...)`）或全局（`mode == None`），与实际门控矩阵正交。

use std::sync::Mutex;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// 信任档位
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustLevel {
    /// 记忆授权：单次批准后记住本次工具，落盘、跨会话、可撤销
    Low,
    /// 会话信任：本会话内免确认，不落盘
    Medium,
    /// 永久信任：长期免确认，落盘、可撤销
    High,
}

impl TrustLevel {
    /// wire 值
    pub fn as_str(&self) -> &'static str {
        match self {
            TrustLevel::Low => "low",
            TrustLevel::Medium => "medium",
            TrustLevel::High => "high",
        }
    }

    /// 是否落盘（Low/High 落盘；Medium 仅内存会话）
    pub fn persists(&self) -> bool {
        !matches!(self, TrustLevel::Medium)
    }

    /// 中文档位名（供设置界面展示）
    pub fn label(&self) -> &'static str {
        match self {
            TrustLevel::Low => "记忆授权",
            TrustLevel::Medium => "会话信任",
            TrustLevel::High => "永久信任",
        }
    }

    pub fn from_key(s: &str) -> Option<TrustLevel> {
        match s {
            "low" => Some(TrustLevel::Low),
            "medium" => Some(TrustLevel::Medium),
            "high" => Some(TrustLevel::High),
            _ => None,
        }
    }
}

/// 一条信任条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustEntry {
    /// 条目 ID（撤销用）
    pub id: String,
    /// 信任档位
    pub level: TrustLevel,
    /// 目标工具（`"*"` 表示全部工具）
    pub tool: String,
    /// 限定模式（`None` = 全局生效）
    pub mode: Option<String>,
    /// 授予时的会话（Medium 会话信任据此匹配）
    pub session: String,
    /// 授予时间（毫秒）
    pub granted_at_ms: i64,
}

impl TrustEntry {
    /// 判断是否命中「工具 × 模式 × 会话」的信任查询
    fn matches(&self, tool: &str, mode: &str, session: &str) -> bool {
        if self.tool != "*" && self.tool != tool {
            return false;
        }
        if let Some(m) = &self.mode {
            if m != mode {
                return false;
            }
        }
        // 会话信任（Medium）仅在授予时的会话内生效
        if self.level == TrustLevel::Medium && self.session != session {
            return false;
        }
        true
    }
}

/// 信任存储：内存台账 + SQLite 落盘（Low/High）。
#[derive(Debug, Default)]
pub struct TrustStore {
    entries: Vec<TrustEntry>,
    seq: u64,
}

impl TrustStore {
    /// 新建空存储
    pub fn new() -> Self {
        Self::default()
    }

    /// 授予一条信任，返回条目 ID
    pub fn grant(
        &mut self,
        level: TrustLevel,
        tool: impl Into<String>,
        mode: Option<String>,
        session: impl Into<String>,
    ) -> String {
        self.seq += 1;
        let entry = TrustEntry {
            id: format!("tr_{}", self.seq),
            level,
            tool: tool.into(),
            mode,
            session: session.into(),
            granted_at_ms: chrono::Utc::now().timestamp_millis(),
        };
        let id = entry.id.clone();
        self.entries.push(entry);
        id
    }

    /// 查询是否命中信任；命中返回档位。
    pub fn is_trusted(&self, tool: &str, mode: &str, session: &str) -> Option<TrustLevel> {
        self.entries
            .iter()
            .find(|e| e.matches(tool, mode, session))
            .map(|e| e.level)
    }

    /// 撤销指定条目
    pub fn revoke(&mut self, id: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        self.entries.len() != before
    }

    /// 撤销全部信任（一键重置）
    pub fn revoke_all(&mut self) -> usize {
        let n = self.entries.len();
        self.entries.clear();
        n
    }

    /// 撤销某会话授予的全部信任（会话切换清理）
    pub fn revoke_session(&mut self, session: &str) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| e.session != session);
        before - self.entries.len()
    }

    /// 全部条目
    pub fn list(&self) -> Vec<&TrustEntry> {
        self.entries.iter().collect()
    }

    /// 条目数
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ---------------------------------------------------------------------------
// SQLite 持久化（跨会话恢复；rusqlite bundled）
// ---------------------------------------------------------------------------

/// 信任库路径（`~/.marguerite/trust.db`；测试可用 `MARGUERITE_TRUST_DB` 覆盖）
pub fn sqlite_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("MARGUERITE_TRUST_DB") {
        if !p.trim().is_empty() {
            return std::path::PathBuf::from(p);
        }
    }
    crate::persistence::config::Config::config_dir().join("trust.db")
}

fn open_db() -> Result<rusqlite::Connection, Box<dyn std::error::Error + Send + Sync>> {
    let path = sqlite_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let db = rusqlite::Connection::open(&path)?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS trust_entries (
            id TEXT PRIMARY KEY,
            level TEXT NOT NULL,
            tool TEXT NOT NULL,
            mode TEXT,
            session TEXT NOT NULL,
            granted_at_ms INTEGER NOT NULL
        );",
    )?;
    Ok(db)
}

fn level_str(l: TrustLevel) -> &'static str {
    l.as_str()
}

fn mode_inner(m: &Option<String>) -> Option<&str> {
    m.as_deref()
}

impl TrustStore {
    /// 持久化全部条目到 SQLite（Low/High 落盘；Medium 仅内存不落盘）。
    pub fn persist(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let db = open_db()?;
        for e in self.entries.iter().filter(|e| e.level.persists()) {
            db.execute(
                "INSERT INTO trust_entries (id, level, tool, mode, session, granted_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                    level=excluded.level, tool=excluded.tool, mode=excluded.mode,
                    session=excluded.session, granted_at_ms=excluded.granted_at_ms",
                rusqlite::params![e.id, level_str(e.level), e.tool, mode_inner(&e.mode), e.session, e.granted_at_ms],
            )?;
        }
        Ok(())
    }

    /// 从 SQLite 恢复（Low/High 跨会话生效；Medium 不落盘故不在此）。
    pub fn restore(&mut self) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let db = open_db()?;
        let mut stmt = db.prepare(
            "SELECT id, level, tool, mode, session, granted_at_ms FROM trust_entries",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(TrustEntry {
                id: row.get::<_, String>(0)?,
                level: TrustLevel::from_key(&row.get::<_, String>(1)?)
                    .unwrap_or(TrustLevel::Low),
                tool: row.get::<_, String>(2)?,
                mode: row.get::<_, Option<String>>(3)?,
                session: row.get::<_, String>(4)?,
                granted_at_ms: row.get::<_, i64>(5)?,
            })
        })?;
        let mut loaded = 0usize;
        for row in rows {
            let e = row?;
            // 内存不为空时不覆盖已有条目（避免与在途授予冲突）
            if !self.entries.iter().any(|x| x.id == e.id) {
                if e.id.starts_with("tr_") {
                    if let Ok(n) = e.id.trim_start_matches("tr_").parse::<u64>() {
                        self.seq = self.seq.max(n);
                    }
                }
                self.entries.push(e);
                loaded += 1;
            }
        }
        Ok(loaded)
    }
}

// ---------------------------------------------------------------------------
// 全局单例（网关 RPC / executor 集成用）
// ---------------------------------------------------------------------------

static TRUST_STORE: OnceLock<Mutex<TrustStore>> = OnceLock::new();

/// 进程级全局信任存储
pub fn global() -> &'static Mutex<TrustStore> {
    TRUST_STORE.get_or_init(|| Mutex::new(TrustStore::new()))
}

/// 启动时恢复落盘的 Low/High 信任（best-effort）
pub fn restore_global() -> usize {
    let mut g = global().lock().unwrap_or_else(|e| e.into_inner());
    g.restore().unwrap_or(0)
}

/// 序列化命中条目为 JSON（供 RPC/事件展示）
pub fn trust_view(e: &TrustEntry) -> serde_json::Value {
    serde_json::json!({
        "id": e.id,
        "level": e.level.as_str(),
        "level_label": e.level.label(),
        "tool": e.tool,
        "mode": e.mode,
        "session": e.session,
        "granted_at_ms": e.granted_at_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SQLite 依赖进程级环境变量 + 全局单例，测试全部串行化。
    static TRUST_TESTS_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_db(tag: &str) -> (String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("marguerite_trust_t_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("trust.db");
        // 进程级覆盖路径（一次性设置；测试串行保证不互相污染）
        std::env::set_var("MARGUERITE_TRUST_DB", path.as_os_str());
        (format!("sess_{}", tag), path)
    }

    #[test]
    fn low_trust_hits_across_sessions() {
        let _g = TRUST_TESTS_LOCK.lock().unwrap();
        let mut s = TrustStore::new();
        s.grant(TrustLevel::Low, "edit_file", None, "sess_a");
        assert_eq!(s.is_trusted("edit_file", "agent", "sess_a"), Some(TrustLevel::Low));
        // Low：跨会话也生效
        assert_eq!(s.is_trusted("edit_file", "agent", "sess_b"), Some(TrustLevel::Low));
        // 其它工具不受信任
        assert_eq!(s.is_trusted("rm", "agent", "sess_a"), None);
    }

    #[test]
    fn medium_trust_session_scoped_not_persisted() {
        let _g = TRUST_TESTS_LOCK.lock().unwrap();
        let (tdb, path) = with_temp_db("m1");
        let mut s = TrustStore::new();
        s.grant(TrustLevel::Medium, "exec", Some("agent".into()), "sess_x");
        // 同会话命中
        assert_eq!(s.is_trusted("exec", "agent", "sess_x"), Some(TrustLevel::Medium));
        // 异会话不命中
        assert_eq!(s.is_trusted("exec", "agent", "sess_y"), None);
        // Medium 不落盘
        s.persist().unwrap();
        let mut s2 = TrustStore::new();
        assert_eq!(s2.restore().unwrap(), 0);
        assert!(s2.is_empty());
        let _ = tdb;
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn high_trust_persists_across_restore() {
        let _g = TRUST_TESTS_LOCK.lock().unwrap();
        let (tdb, path) = with_temp_db("h1");
        let mut s = TrustStore::new();
        s.grant(TrustLevel::High, "*", Some("agent".into()), "sess_x");
        s.persist().unwrap();
        let mut s2 = TrustStore::new();
        let n = s2.restore().unwrap();
        assert_eq!(n, 1);
        assert_eq!(s2.is_trusted("delete_folder", "agent", "other_sess"), Some(TrustLevel::High));
        let _ = tdb;
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn low_trust_persists_across_restore() {
        let _g = TRUST_TESTS_LOCK.lock().unwrap();
        let (tdb, path) = with_temp_db("l1");
        let mut s = TrustStore::new();
        s.grant(TrustLevel::Low, "edit_file", None, "sess_x");
        s.persist().unwrap();
        let mut s2 = TrustStore::new();
        assert_eq!(s2.restore().unwrap(), 1);
        assert_eq!(s2.is_trusted("edit_file", "plan", "any"), Some(TrustLevel::Low));
        let _ = tdb;
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn wildcard_tool_matches_all() {
        let _g = TRUST_TESTS_LOCK.lock().unwrap();
        let mut s = TrustStore::new();
        s.grant(TrustLevel::Medium, "*", None, "sess_w");
        assert_eq!(s.is_trusted("run_shell", "ensemble", "sess_w"), Some(TrustLevel::Medium));
        assert_eq!(s.is_trusted("delete", "bloom", "sess_w"), Some(TrustLevel::Medium));
    }

    #[test]
    fn mode_scope_matches_and_isolates() {
        let _g = TRUST_TESTS_LOCK.lock().unwrap();
        let mut s = TrustStore::new();
        // Ensemble 专用信任
        s.grant(TrustLevel::Low, "exec", Some("ensemble".into()), "sess_m");
        assert_eq!(s.is_trusted("exec", "ensemble", "sess_z"), Some(TrustLevel::Low));
        // 其它 Mode 不命中
        assert_eq!(s.is_trusted("exec", "agent", "sess_z"), None);
        assert_eq!(s.is_trusted("exec", "plan", "sess_z"), None);
        assert_eq!(s.is_trusted("exec", "bloom", "sess_z"), None);
        assert_eq!(s.is_trusted("exec", "omnitranscend", "sess_z"), None);
    }

    #[test]
    fn covers_five_modes() {
        let _g = TRUST_TESTS_LOCK.lock().unwrap();
        let mut s = TrustStore::new();
        for m in ["plan", "agent", "ensemble", "bloom", "omnitranscend"] {
            s.grant(TrustLevel::Low, "edit_file", Some(m.into()), "sess_5");
        }
        for m in ["plan", "agent", "ensemble", "bloom", "omnitranscend"] {
            assert_eq!(s.is_trusted("edit_file", m, "sess_5"), Some(TrustLevel::Low), "mode {m}");
        }
    }

    #[test]
    fn revoke_removes_trust() {
        let _g = TRUST_TESTS_LOCK.lock().unwrap();
        let mut s = TrustStore::new();
        let id = s.grant(TrustLevel::High, "delete", None, "sess_r");
        assert!(s.is_trusted("delete", "agent", "sess_r").is_some());
        assert!(s.revoke(&id));
        assert!(s.is_trusted("delete", "agent", "sess_r").is_none());
        // 假 ID 撤销失败
        assert!(!s.revoke("tr_nope"));
    }

    #[test]
    fn revoke_all_resets() {
        let _g = TRUST_TESTS_LOCK.lock().unwrap();
        let mut s = TrustStore::new();
        s.grant(TrustLevel::Low, "a", None, "s1");
        s.grant(TrustLevel::Medium, "b", None, "s2");
        s.grant(TrustLevel::High, "c", None, "s3");
        assert_eq!(s.len(), 3);
        assert_eq!(s.revoke_all(), 3);
        assert!(s.is_empty());
    }

    #[test]
    fn revoke_session_clears_that_session_only() {
        let _g = TRUST_TESTS_LOCK.lock().unwrap();
        let mut s = TrustStore::new();
        s.grant(TrustLevel::Low, "a", None, "s1");
        s.grant(TrustLevel::Medium, "b", None, "s2");
        assert_eq!(s.revoke_session("s1"), 1);
        assert_eq!(s.revoke_session("s1"), 0);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn default_no_trust_is_zero_confirm() {
        let _g = TRUST_TESTS_LOCK.lock().unwrap();
        let s = TrustStore::new();
        // 默认无任何信任 = 0-确认
        for m in ["plan", "agent", "ensemble", "bloom", "omnitranscend"] {
            assert_eq!(s.is_trusted("exec", m, "sess_z"), None);
        }
    }

    #[test]
    fn id_is_unique_and_incremental() {
        let _g = TRUST_TESTS_LOCK.lock().unwrap();
        let mut s = TrustStore::new();
        let a = s.grant(TrustLevel::Low, "x", None, "s");
        let b = s.grant(TrustLevel::Low, "y", None, "s");
        assert_ne!(a, b);
        assert_eq!(s.list().len(), 2);
    }
}
