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
    /// 参数指纹（见 [`args_fingerprint`]）。
    ///
    /// `None` = 该工具的**任意参数**都免确认（记忆授权/永久信任的常规形态）；
    /// `Some` = 只对授予时那一次的参数形态免确认 —— 「记住这次 `rm -rf build/`」
    /// 不应该顺带放行下一次 `rm -rf ~/`。存指纹不存原文：参数可能含密钥/口令。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args_fingerprint: Option<String>,
    /// 过期时间（毫秒，`None` = 不过期）。
    ///
    /// 到期即回到 0-确认，与从未授予完全一致（不是"过期后仍宽松"）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<i64>,
}

impl TrustEntry {
    /// 判断是否命中「工具 × 模式 × 会话 × 参数」的信任查询
    fn matches(
        &self,
        tool: &str,
        mode: &str,
        session: &str,
        args_fp: Option<&str>,
        now_ms: i64,
    ) -> bool {
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
        // 过期即不命中（fail-closed：时钟不可用时也只可能更严格）
        if let Some(exp) = self.expires_at_ms {
            if now_ms >= exp {
                return false;
            }
        }
        // 参数绑定：未绑定的条目放行全部参数；绑定过的必须逐字相同，
        // 查询方拿不出指纹时**不放行**（无法证明是同一形态）。
        match (&self.args_fingerprint, args_fp) {
            (None, _) => true,
            (Some(want), Some(got)) => want == got,
            (Some(_), None) => false,
        }
    }
}

/// 参数指纹：规范化 JSON → SHA-256 → 前 16 字节 hex。
///
/// 规范化**自己排序**，不依赖 `serde_json` 的默认键序：本仓的依赖图里
/// 对象保留了插入顺序，直接 `to_vec` 会让 `{"a":1,"b":2}` 与 `{"b":2,"a":1}`
/// 算出不同指纹。而这两个形状来自不同入口 —— 审批链给的是构造出的
/// `Value`，工具调用给的是解析出来的 `Value` —— 键序不一致就会让
/// 参数级信任**静默失效**（用户点了「记住这次」却仍被反复追问）。
///
/// **只存指纹不存原文**：审批参数里可能就有密钥/口令，落进 `trust.db`
/// 等于把它们复制到另一个文件。
pub fn args_fingerprint(args: &serde_json::Value) -> String {
    hash_fingerprint(&serde_json::to_vec(&canonicalize(args)).unwrap_or_default())
}

/// 递归重建 JSON，对象键按字典序排列（数组保持原序 —— 顺序是语义的一部分）。
///
/// 两种 `serde_json` 配置下都正确：启用 `preserve_order` 时插入序即迭代序，
/// 未启用时底层本就是 `BTreeMap` 已有序；这里显式排序，与配置无关。
fn canonicalize(value: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable();
            let mut out = serde_json::Map::with_capacity(map.len());
            for k in keys {
                out.insert(k.clone(), canonicalize(&map[k]));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
        other => other.clone(),
    }
}

/// 原始 JSON 文本的参数指纹（解析成功则与 [`args_fingerprint`] 等价）。
pub fn args_fingerprint_str(raw: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(v) => args_fingerprint(&v),
        Err(_) => hash_fingerprint(raw.as_bytes()),
    }
}

fn hash_fingerprint(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect()
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

    /// 授予一条信任（全部参数、不过期），返回条目 ID
    pub fn grant(
        &mut self,
        level: TrustLevel,
        tool: impl Into<String>,
        mode: Option<String>,
        session: impl Into<String>,
    ) -> String {
        self.grant_scoped(level, tool, mode, session, None, None)
    }

    /// 授予一条受约束的信任，返回条目 ID。
    ///
    /// - `args_fp`：`Some` 时只对**该参数形态**免确认（指纹用 [`args_fingerprint`] 算）
    /// - `ttl_sec`：`Some(n)` 时 n 秒后自动失效；`Some(0)` 立即失效
    pub fn grant_scoped(
        &mut self,
        level: TrustLevel,
        tool: impl Into<String>,
        mode: Option<String>,
        session: impl Into<String>,
        args_fp: Option<String>,
        ttl_sec: Option<u64>,
    ) -> String {
        self.seq += 1;
        let now = chrono::Utc::now().timestamp_millis();
        let entry = TrustEntry {
            id: format!("tr_{}", self.seq),
            level,
            tool: tool.into(),
            mode,
            session: session.into(),
            granted_at_ms: now,
            args_fingerprint: args_fp,
            expires_at_ms: ttl_sec.map(|s| now + (s as i64) * 1000),
        };
        let id = entry.id.clone();
        self.entries.push(entry);
        id
    }

    /// 查询**工具级**信任（只匹配未绑定参数的条目）；命中返回档位。
    ///
    /// 用于没有具体参数的场景（能力签发 / 内容通道）：一条参数级信任不应被
    /// 误报成「整个工具已签发」。
    pub fn is_trusted(&self, tool: &str, mode: &str, session: &str) -> Option<TrustLevel> {
        self.is_trusted_for(tool, mode, session, None)
    }

    /// 查询**参数级**信任：`args_fp` 是本次调用参数的指纹。
    ///
    /// 未绑定参数的条目对任意 `args_fp` 都命中（向后兼容既有授权语义）；
    /// 绑定过的条目在 `args_fp` 缺失或不同时一律不命中（fail-closed）。
    pub fn is_trusted_for(
        &self,
        tool: &str,
        mode: &str,
        session: &str,
        args_fp: Option<&str>,
    ) -> Option<TrustLevel> {
        let now = chrono::Utc::now().timestamp_millis();
        self.entries
            .iter()
            .find(|e| e.matches(tool, mode, session, args_fp, now))
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
            granted_at_ms INTEGER NOT NULL,
            args_fingerprint TEXT,
            expires_at_ms INTEGER
        );",
    )?;
    // 旧库升级：`CREATE TABLE IF NOT EXISTS` 对**已存在**的表什么都不做，
    // 所以新增列必须单独补（否则老用户一升级就报 "no such column"）。
    ensure_column(&db, "args_fingerprint", "TEXT")?;
    ensure_column(&db, "expires_at_ms", "INTEGER")?;
    Ok(db)
}

/// 给既有 `trust_entries` 补列（幂等：列已存在则 no-op）。
fn ensure_column(
    db: &rusqlite::Connection,
    name: &str,
    ty: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let existing: Vec<String> = {
        let mut stmt = db.prepare("PRAGMA table_info(trust_entries)")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        rows.filter_map(Result::ok).collect()
    };
    if !existing.iter().any(|c| c == name) {
        db.execute_batch(&format!("ALTER TABLE trust_entries ADD COLUMN {name} {ty}"))?;
    }
    Ok(())
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
                "INSERT INTO trust_entries
                    (id, level, tool, mode, session, granted_at_ms, args_fingerprint, expires_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(id) DO UPDATE SET
                    level=excluded.level, tool=excluded.tool, mode=excluded.mode,
                    session=excluded.session, granted_at_ms=excluded.granted_at_ms,
                    args_fingerprint=excluded.args_fingerprint,
                    expires_at_ms=excluded.expires_at_ms",
                rusqlite::params![
                    e.id,
                    level_str(e.level),
                    e.tool,
                    mode_inner(&e.mode),
                    e.session,
                    e.granted_at_ms,
                    e.args_fingerprint,
                    e.expires_at_ms
                ],
            )?;
        }
        Ok(())
    }

    /// 从 SQLite 恢复（Low/High 跨会话生效；Medium 不落盘故不在此）。
    pub fn restore(&mut self) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let db = open_db()?;
        // 先完整收集再处理：`stmt` 借用 `db`，边遍历边 DELETE 编译不过。
        let stored: Vec<TrustEntry> = {
            let mut stmt = db.prepare(
                "SELECT id, level, tool, mode, session, granted_at_ms, args_fingerprint, expires_at_ms
                 FROM trust_entries",
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
                    args_fingerprint: row.get::<_, Option<String>>(6)?,
                    expires_at_ms: row.get::<_, Option<i64>>(7)?,
                })
            })?;
            rows.filter_map(Result::ok).collect()
        };
        let now = chrono::Utc::now().timestamp_millis();
        let mut loaded = 0usize;
        for e in stored {
            // 过期条目既不入内存也不留在库里：否则设置界面会永远显示一条
            // 早已失效的「永久信任」，用户以为它还生效。
            if e.expires_at_ms.is_some_and(|t| now >= t) {
                let _ = db.execute(
                    "DELETE FROM trust_entries WHERE id = ?1",
                    rusqlite::params![e.id],
                );
                continue;
            }
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
        // 只暴露「是否绑定参数」这个布尔，不暴露指纹本身（界面用不上，也少一个可篡改面）
        "args_scoped": e.args_fingerprint.is_some(),
        "expires_at_ms": e.expires_at_ms,
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

    // -----------------------------------------------------------------
    // 参数级绑定与有效期（B3）：信任不能从「这次」泛化到「那个工具」
    // -----------------------------------------------------------------

    #[test]
    fn args_fingerprint_is_stable_different_and_never_plaintext() {
        let a = serde_json::json!({"command": "rm -rf build/", "cwd": "/proj"});
        let b = serde_json::json!({"cwd": "/proj", "command": "rm -rf build/"}); // 键序不同
        let c = serde_json::json!({"command": "rm -rf ~/", "cwd": "/proj"});
        assert_eq!(args_fingerprint(&a), args_fingerprint(&a), "同参数必须同指纹");
        assert_eq!(args_fingerprint(&a), args_fingerprint(&b), "键序不影响指纹");
        assert_ne!(args_fingerprint(&a), args_fingerprint(&c), "不同参数必须不同指纹");

        let secret = serde_json::json!({"api_key": "sk-LEAK-CANARY"});
        let fp = args_fingerprint(&secret);
        assert!(!fp.contains("sk-LEAK"), "指纹里不得出现参数原文: {fp}");
        assert_eq!(fp.len(), 32, "取 SHA-256 前 16 字节 = 32 hex");
        // 原始 JSON 文本入口与 Value 入口必须等价（调用方拿到的形态不同）
        assert_eq!(fp, args_fingerprint_str(r#"{"api_key":"sk-LEAK-CANARY"}"#));
    }

    #[test]
    fn arg_scoped_trust_only_matches_the_same_args() {
        let _g = TRUST_TESTS_LOCK.lock().unwrap();
        let mut s = TrustStore::new();
        let granted = serde_json::json!({"command": "rm -rf build/"});
        let fp = args_fingerprint(&granted);
        s.grant_scoped(
            TrustLevel::High,
            "shell_exec",
            None,
            "sess_a",
            Some(fp.clone()),
            None,
        );

        // 同一参数形态 → 命中
        assert_eq!(
            s.is_trusted_for("shell_exec", "agent", "sess_a", Some(&fp)),
            Some(TrustLevel::High)
        );
        // 换了参数 → 不命中（这正是要修的漏洞：不能"记住一次 = 信任全部"）
        let other = args_fingerprint(&serde_json::json!({"command": "rm -rf ~/"}));
        assert_eq!(
            s.is_trusted_for("shell_exec", "agent", "sess_a", Some(&other)),
            None
        );
        // 查询方拿不出参数 → 不命中（fail-closed）
        assert_eq!(s.is_trusted_for("shell_exec", "agent", "sess_a", None), None);
        // 工具级查询也不得把它当成"整个工具已签发"
        assert_eq!(s.is_trusted("shell_exec", "agent", "sess_a"), None);
    }

    #[test]
    fn unscoped_trust_still_matches_any_args() {
        // 既有语义不能回退：不绑定参数的授权对任意参数都生效。
        let _g = TRUST_TESTS_LOCK.lock().unwrap();
        let mut s = TrustStore::new();
        s.grant(TrustLevel::Low, "edit_file", None, "sess_u");
        let fp = args_fingerprint(&serde_json::json!({"path": "a.txt"}));
        assert_eq!(
            s.is_trusted_for("edit_file", "agent", "sess_u", Some(&fp)),
            Some(TrustLevel::Low)
        );
        assert_eq!(s.is_trusted("edit_file", "agent", "sess_u"), Some(TrustLevel::Low));
    }

    #[test]
    fn expired_trust_stops_matching_and_is_purged_on_restore() {
        let _g = TRUST_TESTS_LOCK.lock().unwrap();
        let (tdb, path) = with_temp_db("exp1");
        let mut s = TrustStore::new();
        // ttl=0 → 立即过期
        s.grant_scoped(TrustLevel::High, "delete", None, "sess_e", None, Some(0));
        assert_eq!(s.is_trusted("delete", "agent", "sess_e"), None, "过期即不命中");
        s.persist().unwrap();

        // 恢复时把过期条目从库里清掉（否则设置界面永远显示一条失效的"永久信任"）
        let mut s2 = TrustStore::new();
        assert_eq!(s2.restore().unwrap(), 0);
        let db = open_db().unwrap();
        let n: i64 = db
            .query_row("SELECT COUNT(*) FROM trust_entries", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "过期条目应从库中清除");

        // 未过期的仍能恢复（反向自证不是"恢复一律清空"）
        let mut s3 = TrustStore::new();
        s3.grant_scoped(TrustLevel::High, "delete", None, "sess_e", None, Some(3600));
        s3.persist().unwrap();
        let mut s4 = TrustStore::new();
        assert_eq!(s4.restore().unwrap(), 1);
        assert_eq!(s4.is_trusted("delete", "agent", "sess_e"), Some(TrustLevel::High));

        let _ = tdb;
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// 老用户的 `trust.db` 没有新列：升级后必须自动补列而不是报 "no such column"。
    #[test]
    fn legacy_database_without_new_columns_is_migrated() {
        let _g = TRUST_TESTS_LOCK.lock().unwrap();
        let (tdb, path) = with_temp_db("mig1");
        {
            let db = rusqlite::Connection::open(&path).unwrap();
            db.execute_batch(
                "CREATE TABLE trust_entries (
                    id TEXT PRIMARY KEY,
                    level TEXT NOT NULL,
                    tool TEXT NOT NULL,
                    mode TEXT,
                    session TEXT NOT NULL,
                    granted_at_ms INTEGER NOT NULL
                );",
            )
            .unwrap();
            db.execute(
                "INSERT INTO trust_entries (id, level, tool, mode, session, granted_at_ms)
                 VALUES ('tr_7', 'high', 'edit_file', NULL, 'sess_old', 1700000000000)",
                [],
            )
            .unwrap();
        }

        let mut s = TrustStore::new();
        assert_eq!(s.restore().unwrap(), 1, "老库条目应能加载");
        let e = s.list()[0].clone();
        assert_eq!(e.args_fingerprint, None, "老条目视为不绑定参数");
        assert_eq!(e.expires_at_ms, None, "老条目视为不过期");
        assert_eq!(s.is_trusted("edit_file", "agent", "sess_x"), Some(TrustLevel::High));

        let _ = tdb;
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
