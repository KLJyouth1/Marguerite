//! 目标循环（Goal Round Driver）—— 上游 harness 二开落地（Track 1）。
//!
//! 对齐 DeepSeek Harness 的 goal 子系统：单一持久化「完成目标」，
//! 支持自治续轮（armed/disarmed）、resume/pause/complete/blocked 状态机，
//! blocked 需目标先经过 MIN_BLOCKED_ROUNDS 轮续轮（rounds_started 达标）方可置为 blocked，
//! 阻止过早「放弃」；阻塞原因由调用方（Agent/驱动）在 blocked_reason 中说明。
//!
//! 语义（与 DSH 对齐）：
//! - create：登记唯一进行中的完成目标（active/armed）；已有 active/paused 目标时报冲突。
//! - get / activation：读取目标与其 armed/disarmed 激活态。
//! - update（edit/pause/resume/complete/blocked）：状态机迁移，每次 revision+1。
//! - 会话恢复/fork 后目标自动 disarmed（armed 为会话级、不落库），
//!   需人工 resume 重新 armed 后才继续自治续轮。
//! - begin_round：续轮驱动——仅在 armed 且 active 且未达 max_goal_rounds 时放行一轮。
//!
//! 持久化采用 SQLite（rusqlite bundled），路径默认 ~/.marguerite/goal.db，
//! 测试可用 MARGUERITE_GOAL_DB 环境变量覆盖隔离。

use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

pub const MIN_BLOCKED_ROUNDS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoalPhase {
    Active,
    Paused,
    Blocked,
    Complete,
}

impl GoalPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            GoalPhase::Active => "active",
            GoalPhase::Paused => "paused",
            GoalPhase::Blocked => "blocked",
            GoalPhase::Complete => "complete",
        }
    }

    pub fn from_name(s: &str) -> Self {
        match s {
            "paused" => GoalPhase::Paused,
            "blocked" => GoalPhase::Blocked,
            "complete" => GoalPhase::Complete,
            _ => GoalPhase::Active,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activation {
    Armed,
    Disarmed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub revision: u64,
    pub objective: String,
    pub phase: GoalPhase,
    pub rounds_started: u32,
    pub max_goal_rounds: Option<u32>,
    pub blocked_reason: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug)]
pub struct GoalRegistry {
    goal: Option<Goal>,
    armed: bool,
}

impl Default for GoalRegistry {
    fn default() -> Self { Self::new() }
}

impl GoalRegistry {
    pub fn new() -> Self {
        Self { goal: None, armed: false }
    }

    pub fn get(&self) -> Option<&Goal> { self.goal.as_ref() }

    pub fn activation(&self) -> Activation {
        if self.armed { Activation::Armed } else { Activation::Disarmed }
    }

    pub fn is_armed(&self) -> bool { self.armed }

    pub fn disarm(&mut self) { self.armed = false; }

    fn require_goal(&self) -> Result<&Goal> {
        self.goal.as_ref().ok_or_else(|| anyhow!("尚无目标，请先 goal create"))
    }

    pub fn create(&mut self, objective: &str, max_goal_rounds: Option<u32>) -> Result<Goal> {
        if let Some(g) = &self.goal {
            if matches!(g.phase, GoalPhase::Active | GoalPhase::Paused) {
                return Err(anyhow!("已有进行中的目标（{}），请先 complete/pause 后再创建", g.id));
            }
        }
        let trimmed = objective.trim();
        if trimmed.is_empty() { return Err(anyhow!("目标不可为空")); }
        let now = Utc::now().timestamp_millis();
        let goal = Goal {
            id: format!("goal-{now}"),
            revision: 1,
            objective: trimmed.to_string(),
            phase: GoalPhase::Active,
            rounds_started: 0,
            max_goal_rounds,
            blocked_reason: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        self.goal = Some(goal.clone());
        self.armed = true;
        self.persist_quiet();
        Ok(goal)
    }

    pub fn edit(&mut self, objective: &str) -> Result<Goal> {
        let g = self.require_goal()?.clone();
        let trimmed = objective.trim();
        if trimmed.is_empty() { return Err(anyhow!("目标不可为空")); }
        let updated = Goal {
            objective: trimmed.to_string(),
            revision: g.revision + 1,
            updated_at_ms: Utc::now().timestamp_millis(),
            ..g
        };
        self.goal = Some(updated.clone());
        self.persist_quiet();
        Ok(updated)
    }

    pub fn pause(&mut self) -> Result<Goal> {
        let g = self.require_goal()?.clone();
        if g.phase != GoalPhase::Active {
            return Err(anyhow!("仅 active 目标可暂停，当前为 {}", g.phase.as_str()));
        }
        let updated = Goal {
            phase: GoalPhase::Paused,
            revision: g.revision + 1,
            updated_at_ms: Utc::now().timestamp_millis(),
            ..g
        };
        self.goal = Some(updated.clone());
        self.armed = false;
        self.persist_quiet();
        Ok(updated)
    }

    pub fn resume(&mut self) -> Result<Goal> {
        let g = self.require_goal()?.clone();
        let updated = Goal {
            phase: GoalPhase::Active,
            blocked_reason: None,
            revision: g.revision + 1,
            updated_at_ms: Utc::now().timestamp_millis(),
            ..g
        };
        self.goal = Some(updated.clone());
        self.armed = true;
        self.persist_quiet();
        Ok(updated)
    }

    pub fn complete(&mut self) -> Result<Goal> {
        let g = self.require_goal()?.clone();
        let updated = Goal {
            phase: GoalPhase::Complete,
            blocked_reason: None,
            revision: g.revision + 1,
            updated_at_ms: Utc::now().timestamp_millis(),
            ..g
        };
        self.goal = Some(updated.clone());
        self.armed = false;
        self.persist_quiet();
        Ok(updated)
    }

    pub fn block(&mut self, reason: &str) -> Result<Goal> {
        let trimmed = reason.trim();
        if trimmed.is_empty() { return Err(anyhow!("阻塞原因不可为空")); }
        let g = self.require_goal()?.clone();
        if g.rounds_started < MIN_BLOCKED_ROUNDS {
            return Err(anyhow!(
                "blocked 需至少经过 {MIN_BLOCKED_ROUNDS} 轮续轮（当前 roundsStarted={}），暂不置为 blocked",
                g.rounds_started
            ));
        }
        let updated = Goal {
            phase: GoalPhase::Blocked,
            blocked_reason: Some(trimmed.to_string()),
            revision: g.revision + 1,
            updated_at_ms: Utc::now().timestamp_millis(),
            ..g
        };
        self.goal = Some(updated.clone());
        self.armed = false;
        self.persist_quiet();
        Ok(updated)
    }

    pub fn can_continue(&self) -> bool {
        let Some(g) = &self.goal else { return false; };
        if !self.armed || g.phase != GoalPhase::Active { return false; }
        match g.max_goal_rounds {
            Some(cap) => g.rounds_started < cap,
            None => true,
        }
    }

    pub fn begin_round(&mut self) -> bool {
        if !self.can_continue() { return false; }
        let g = self.goal.clone().unwrap();
        self.goal = Some(Goal {
            rounds_started: g.rounds_started + 1,
            updated_at_ms: Utc::now().timestamp_millis(),
            ..g
        });
        self.persist_quiet();
        true
    }
}

pub fn sqlite_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("MARGUERITE_GOAL_DB") {
        if !p.trim().is_empty() { return std::path::PathBuf::from(p); }
    }
    crate::persistence::config::Config::config_dir().join("goal.db")
}

fn open_db() -> Result<rusqlite::Connection> {
    let path = sqlite_path();
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    let db = rusqlite::Connection::open(&path)?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS goals (
            id TEXT PRIMARY KEY,
            revision INTEGER NOT NULL,
            objective TEXT NOT NULL,
            phase TEXT NOT NULL,
            rounds_started INTEGER NOT NULL,
            max_goal_rounds INTEGER,
            blocked_reason TEXT,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL
        );",
    )?;
    Ok(db)
}

impl GoalRegistry {
    pub fn persist(&self) -> Result<()> {
        let Some(g) = &self.goal else { return Ok(()); };
        let db = open_db()?;
        db.execute(
            "INSERT INTO goals
                (id, revision, objective, phase, rounds_started, max_goal_rounds,
                 blocked_reason, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                revision=excluded.revision, objective=excluded.objective,
                phase=excluded.phase, rounds_started=excluded.rounds_started,
                max_goal_rounds=excluded.max_goal_rounds, blocked_reason=excluded.blocked_reason,
                created_at_ms=excluded.created_at_ms, updated_at_ms=excluded.updated_at_ms",
            rusqlite::params![
                g.id, g.revision, g.objective, g.phase.as_str(), g.rounds_started,
                g.max_goal_rounds, g.blocked_reason, g.created_at_ms, g.updated_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn restore_from_sqlite(&mut self) -> Result<()> {
        let db = open_db()?;
        let mut stmt = db.prepare(
            "SELECT id, revision, objective, phase, rounds_started, max_goal_rounds,
                    blocked_reason, created_at_ms, updated_at_ms
             FROM goals ORDER BY updated_at_ms DESC LIMIT 1",
        )?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            self.goal = Some(Goal {
                id: row.get(0)?,
                revision: row.get(1)?,
                objective: row.get(2)?,
                phase: GoalPhase::from_name(&row.get::<_, String>(3)?),
                rounds_started: row.get(4)?,
                max_goal_rounds: row.get(5)?,
                blocked_reason: row.get(6)?,
                created_at_ms: row.get(7)?,
                updated_at_ms: row.get(8)?,
            });
        }
        self.armed = false;
        Ok(())
    }

    pub fn persist_quiet(&self) {
        if let Err(e) = self.persist() {
            tracing::warn!(err = %e, "目标持久化失败（忽略，继续内存态）");
        }
    }
}

pub fn global() -> &'static std::sync::Mutex<GoalRegistry> {
    static S: OnceLock<std::sync::Mutex<GoalRegistry>> = OnceLock::new();
    S.get_or_init(|| {
        let mut r = GoalRegistry::new();
        if let Err(e) = r.restore_from_sqlite() {
            tracing::warn!(err = %e, "目标 SQLite 恢复失败（继续内存态运行）");
        }
        std::sync::Mutex::new(r)
    })
}

pub fn goal_view(g: &Goal, armed: bool) -> serde_json::Value {
    serde_json::json!({
        "id": g.id,
        "revision": g.revision,
        "objective": g.objective,
        "phase": g.phase.as_str(),
        "roundsStarted": g.rounds_started,
        "maxGoalRounds": g.max_goal_rounds,
        "blockedReason": g.blocked_reason,
        "activation": if armed { "armed" } else { "disarmed" },
        "createdAtMs": g.created_at_ms,
        "updatedAtMs": g.updated_at_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    fn lock_tests() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.get_or_init(|| std::sync::Mutex::new(())).lock().unwrap_or_else(|e| e.into_inner())
    }

    fn isolated_db(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("marguerite_goal_test_{}_{}", tag, Utc::now().timestamp_millis()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("goal.db");
        std::env::set_var("MARGUERITE_GOAL_DB", db.to_string_lossy().to_string());
        db
    }

    #[test]
    fn create_sets_active_armed() {
        let _g = lock_tests();
        isolated_db("create");
        let mut r = GoalRegistry::new();
        let g = r.create("重构部署流水线", None).unwrap();
        assert_eq!(g.phase, GoalPhase::Active);
        assert_eq!(g.revision, 1);
        assert!(r.is_armed());
    }

    #[test]
    fn create_rejects_empty_objective() {
        let _g = lock_tests();
        isolated_db("empty");
        let mut r = GoalRegistry::new();
        assert!(r.create("   ", None).is_err());
    }

    #[test]
    fn create_conflicts_with_active_goal() {
        let _g = lock_tests();
        isolated_db("conflict");
        let mut r = GoalRegistry::new();
        r.create("A", None).unwrap();
        assert!(r.create("B", None).is_err());
    }

    #[test]
    fn create_allowed_after_complete() {
        let _g = lock_tests();
        isolated_db("recreate");
        let mut r = GoalRegistry::new();
        r.create("A", None).unwrap();
        r.complete().unwrap();
        assert!(r.create("B", None).is_ok());
    }

    #[test]
    fn edit_bumps_revision_and_keeps_id() {
        let _g = lock_tests();
        isolated_db("edit");
        let mut r = GoalRegistry::new();
        let g = r.create("旧目标", None).unwrap();
        let e = r.edit("新目标").unwrap();
        assert_eq!(e.id, g.id);
        assert_eq!(e.revision, 2);
        assert_eq!(e.objective, "新目标");
    }

    #[test]
    fn pause_resume_complete_transitions() {
        let _g = lock_tests();
        isolated_db("transitions");
        let mut r = GoalRegistry::new();
        r.create("x", None).unwrap();
        assert_eq!(r.pause().unwrap().phase, GoalPhase::Paused);
        assert!(!r.is_armed());
        assert_eq!(r.resume().unwrap().phase, GoalPhase::Active);
        assert!(r.is_armed());
        assert_eq!(r.complete().unwrap().phase, GoalPhase::Complete);
        assert!(!r.is_armed());
    }

    #[test]
    fn pause_only_from_active() {
        let _g = lock_tests();
        isolated_db("pause_guard");
        let mut r = GoalRegistry::new();
        r.create("x", None).unwrap();
        r.pause().unwrap();
        assert!(r.pause().is_err());
    }

    #[test]
    fn block_requires_min_rounds() {
        let _g = lock_tests();
        isolated_db("block");
        let mut r = GoalRegistry::new();
        r.create("x", None).unwrap();
        for _ in 0..MIN_BLOCKED_ROUNDS {
            let err = r.block("网络不通").unwrap_err();
            assert!(err.to_string().contains("暂不置为 blocked"));
            assert!(r.begin_round(), "每轮都应可续轮");
        }
        let g = r.block("网络不通").unwrap();
        assert_eq!(g.phase, GoalPhase::Blocked);
        assert_eq!(g.blocked_reason.as_deref(), Some("网络不通"));
        assert!(!r.is_armed());
    }

    #[test]
    fn block_rejects_empty_reason() {
        let _g = lock_tests();
        isolated_db("block_empty");
        let mut r = GoalRegistry::new();
        r.create("x", None).unwrap();
        assert!(r.block("  ").is_err());
    }

    #[test]
    fn disarm_blocks_continuation_until_resume() {
        let _g = lock_tests();
        isolated_db("disarm");
        let mut r = GoalRegistry::new();
        r.create("x", None).unwrap();
        assert!(r.can_continue());
        r.disarm();
        assert!(!r.can_continue());
        assert!(!r.begin_round());
        r.resume().unwrap();
        assert!(r.can_continue());
        assert!(r.begin_round());
    }

    #[test]
    fn begin_round_respects_max_rounds() {
        let _g = lock_tests();
        isolated_db("maxrounds");
        let mut r = GoalRegistry::new();
        r.create("x", Some(2)).unwrap();
        assert!(r.begin_round());
        assert!(r.begin_round());
        assert!(!r.begin_round());
        assert_eq!(r.get().unwrap().rounds_started, 2);
        assert!(!r.can_continue());
    }

    #[test]
    fn begin_round_blocked_when_not_active() {
        let _g = lock_tests();
        isolated_db("round_not_active");
        let mut r = GoalRegistry::new();
        r.create("x", None).unwrap();
        r.pause().unwrap();
        assert!(!r.begin_round());
    }

    #[test]
    fn sqlite_persist_and_restore_roundtrip_disarmed() {
        let _g = lock_tests();
        let db = isolated_db("roundtrip");
        {
            let mut r = GoalRegistry::new();
            let g = r.create("跨会话目标", Some(5)).unwrap();
            r.begin_round();
            r.persist().unwrap();
            assert!(db.exists());
            assert_eq!(g.revision, 1);
        }
        {
            let mut r2 = GoalRegistry::new();
            r2.restore_from_sqlite().unwrap();
            let g = r2.get().expect("恢复后应有目标");
            assert_eq!(g.objective, "跨会话目标");
            assert_eq!(g.max_goal_rounds, Some(5));
            assert_eq!(g.rounds_started, 1);
            assert_eq!(g.phase, GoalPhase::Active);
            assert!(!r2.is_armed());
            assert!(!r2.can_continue());
        }
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }
}
