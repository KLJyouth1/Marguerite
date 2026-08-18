//! 经验台账：自进化记忆沉淀层
//!
//! 在既有三层记忆（感知 / 短期 / 长期）之上，提供结构化、可检索的「经验」沉淀：
//! - `Pitfall`（踩坑）：稳定签名 + 根因 + 修法 + 是否已验证
//! - `Decision`（决策）：ADR 式 —— 背景 / 决策 / 影响
//! - `Pattern`（解法）：可复用的问题→方案映射
//!
//! 复用 `long_term::LocalEmbedding` 做语义检索（零外部依赖、确定性哈希）。

use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;

use crate::memory::embedding::EmbeddingProvider;
use crate::memory::long_term::LongTermMemory;
use crate::memory::sensory::SensoryMemory;
use crate::types::Message;

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
}

/// 经验台账（SQLite 持久化，语义检索）
pub struct ExperienceLedger {
    db_path: PathBuf,
    db: Option<Connection>,
}

impl ExperienceLedger {
    pub fn new(path: &Path) -> Self {
        Self {
            db_path: path.to_path_buf(),
            db: None,
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
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_exp_kind ON experience_ledger(kind);
            CREATE INDEX IF NOT EXISTS idx_exp_sig ON experience_ledger(signature);",
        )?;
        self.db = Some(db);
        Ok(())
    }

    fn db(&self) -> Result<&Connection> {
        self.db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("ExperienceLedger not initialized"))
    }

    /// 记录一条经验；`signature` 相同则更新而非重复插入（幂等防重）
    pub fn record(
        &mut self,
        kind: ExperienceKind,
        title: &str,
        body: &str,
        tags: &[String],
        signature: &str,
    ) -> Result<i64> {
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
                    "UPDATE experience_ledger SET kind=?1, title=?2, body=?3, tags=?4, vector=?5, created_at=?6 WHERE id=?7",
                    rusqlite::params![kind.as_str(), title, body, tags_str, vector, now, existing],
                )?;
                return Ok(existing);
            }
        }

        db.execute(
            "INSERT INTO experience_ledger (kind, title, body, tags, signature, vector, validated, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
            rusqlite::params![kind.as_str(), title, body, tags_str, signature, vector, now],
        )?;
        Ok(db.last_insert_rowid())
    }

    /// 语义检索（低于阈值视为无相关）
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Experience>> {
        const SIM_THRESHOLD: f64 = 0.25;
        let db = self.db()?;
        let qvec = EmbeddingProvider::global().embed(query);
        let mut stmt = db.prepare(
            "SELECT id, kind, title, body, tags, vector, signature, validated, created_at FROM experience_ledger",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, String>(8)?,
            ))
        })?;

        let mut scored = Vec::new();
        for row in rows.flatten() {
            let vec: Vec<f64> = row
                .5
                .split(',')
                .filter_map(|s| s.parse::<f64>().ok())
                .collect();
            if vec.len() != EmbeddingProvider::global().dim() {
                continue;
            }
            let score = EmbeddingProvider::global().cosine(&qvec, &vec);
            if score >= SIM_THRESHOLD {
                scored.push((
                    score,
                    Experience {
                        id: row.0,
                        kind: ExperienceKind::from_kind_str(&row.1),
                        title: row.2,
                        body: row.3,
                        tags: if row.4.is_empty() {
                            Vec::new()
                        } else {
                            row.4.split(',').map(|s| s.to_string()).collect()
                        },
                        signature: row.6,
                        validated: row.7 != 0,
                        created_at: row.8,
                    },
                ));
            }
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        Ok(scored.into_iter().map(|(_, e)| e).collect())
    }

    /// 按类型列出（最新在前）
    pub fn list_by_kind(&self, kind: ExperienceKind, limit: usize) -> Result<Vec<Experience>> {
        let db = self.db()?;
        let mut stmt = db.prepare(
            "SELECT id, kind, title, body, tags, signature, validated, created_at
             FROM experience_ledger WHERE kind = ?1 ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![kind.as_str(), limit as i64], |row| {
            Ok(Experience {
                id: row.get(0)?,
                kind: ExperienceKind::from_kind_str(&row.get::<_, String>(1)?),
                title: row.get(2)?,
                body: row.get(3)?,
                tags: row
                    .get::<_, String>(4)?
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect(),
                signature: row.get(5)?,
                validated: row.get::<_, i64>(6)? != 0,
                created_at: row.get(7)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// 列出最近条目（全类型，最新在前；网关 `memory.list` 默认路径）
    pub fn list_recent(&self, limit: usize) -> Result<Vec<Experience>> {
        let db = self.db()?;
        let mut stmt = db.prepare(
            "SELECT id, kind, title, body, tags, signature, validated, created_at
             FROM experience_ledger ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            Ok(Experience {
                id: row.get(0)?,
                kind: ExperienceKind::from_kind_str(&row.get::<_, String>(1)?),
                title: row.get(2)?,
                body: row.get(3)?,
                tags: row
                    .get::<_, String>(4)?
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect(),
                signature: row.get(5)?,
                validated: row.get::<_, i64>(6)? != 0,
                created_at: row.get(7)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// 标记某条经验已验证（踩坑闭环：修法被真实运行验证通过）
    pub fn mark_validated(&mut self, id: i64) -> Result<()> {
        let db = self.db()?;
        db.execute(
            "UPDATE experience_ledger SET validated = 1 WHERE id = ?1",
            rusqlite::params![id],
        )?;
        Ok(())
    }

    /// 条目总数
    pub fn count(&self) -> Result<usize> {
        let db = self.db()?;
        let n: i64 = db.query_row("SELECT COUNT(*) FROM experience_ledger", [], |r| r.get(0))?;
        Ok(n as usize)
    }
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
}
