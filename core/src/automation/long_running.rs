//! 长时任务调度器（E 期 automation/ 差异能力）。
//!
//! 提供 7×24 持续运行能力，对齐 nexustar-api `long_running_scheduler.py`：
//! - 注册长时任务（重构/迁移/全量/7x24 等）
//! - 心跳保活、检查点保存（可恢复状态快照）
//! - 超时检测：心跳超 `heartbeat_interval_s * 3` → paused
//! - 自动续跑：paused 且有 checkpoint 且未达 max_retries → resumed（retry+1）
//! - 跨会话恢复：启动时把 running 但心跳超时的任务转 paused 等待续跑
//!
//! 设计为内存态、同步、离线可测；持久化钩子（SQLite）作为后续增量接入。

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

/// 最大重试次数
pub const MAX_RETRIES: u32 = 5;
/// 心跳超时倍数（心跳超 `interval * 此值` 视为超时）
pub const HEARTBEAT_TIMEOUT_MULTIPLIER: i64 = 3;
/// 默认最长运行小时数（7 天）
pub const DEFAULT_MAX_DURATION_HOURS: f64 = 168.0;

/// 合法阶段
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stage {
    /// 分析
    Analyze,
    /// 规划
    Plan,
    /// 执行
    Execute,
    /// 聚合
    Aggregate,
    /// 精炼
    Refine,
}

impl Stage {
    /// 阶段名
    pub fn as_str(&self) -> &'static str {
        match self {
            Stage::Analyze => "analyze",
            Stage::Plan => "plan",
            Stage::Execute => "execute",
            Stage::Aggregate => "aggregate",
            Stage::Refine => "refine",
        }
    }
}

/// 任务状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    /// 待运行
    Pending,
    /// 运行中
    Running,
    /// 已暂停（等待续跑）
    Paused,
    /// 已完成
    Completed,
    /// 已失败
    Failed,
    /// 已恢复
    Resumed,
    /// 已中断（上次运行未完成，重启后被标记；可对外展示）
    Interrupted,
}

impl TaskStatus {
    /// 状态名（snake_case，供 RPC/前端展示）
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::Running => "running",
            TaskStatus::Paused => "paused",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Resumed => "resumed",
            TaskStatus::Interrupted => "interrupted",
        }
    }
}

/// 任务检查点（可恢复状态快照）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// 阶段
    pub stage: Stage,
    /// 状态快照
    pub state: serde_json::Value,
    /// 时间戳（毫秒）
    pub timestamp_ms: i64,
}

/// 长时任务条目
#[derive(Debug, Clone)]
pub struct LongRunningTask {
    /// 任务 id
    pub task_id: String,
    /// 目标描述
    pub goal: String,
    /// 启动时间（毫秒）
    pub started_at_ms: i64,
    /// 最后心跳（毫秒）
    pub last_heartbeat_ms: i64,
    /// 状态
    pub status: TaskStatus,
    /// 检查点列表
    pub checkpoints: Vec<Checkpoint>,
    /// 心跳间隔（秒）
    pub heartbeat_interval_s: i64,
    /// 最长运行小时数
    pub max_duration_hours: f64,
    /// 已重试次数
    pub retry_count: u32,
    /// 最大重试次数
    pub max_retries: u32,
}

impl LongRunningTask {
    /// 构造任务（状态 Running，记录启动心跳）
    pub fn new(task_id: &str, goal: &str, heartbeat_interval_s: i64) -> Self {
        let now = Utc::now().timestamp_millis();
        Self {
            task_id: task_id.to_string(),
            goal: goal.to_string(),
            started_at_ms: now,
            last_heartbeat_ms: now,
            status: TaskStatus::Running,
            checkpoints: Vec::new(),
            heartbeat_interval_s: heartbeat_interval_s.max(1),
            max_duration_hours: DEFAULT_MAX_DURATION_HOURS,
            retry_count: 0,
            max_retries: MAX_RETRIES,
        }
    }

    /// 发送心跳（更新最后心跳时间）
    pub fn heartbeat(&mut self) {
        self.last_heartbeat_ms = Utc::now().timestamp_millis();
    }

    /// 保存检查点
    pub fn save_checkpoint(&mut self, stage: Stage, state: serde_json::Value) {
        self.checkpoints.push(Checkpoint {
            stage,
            state,
            timestamp_ms: Utc::now().timestamp_millis(),
        });
    }

    /// 心跳是否超时（now_ms - last_heartbeat_ms > interval_s * 倍数 * 1000ms）
    /// 注意：右侧须把「秒×倍数」换算成毫秒，与左侧 now_ms/last_heartbeat_ms 同单位。
    /// 此前漏乘 1000，导致 90ms 不心跳即误判超时（生产也会误暂停实时任务）。
    pub fn is_heartbeat_stale(&self, now_ms: i64) -> bool {
        now_ms - self.last_heartbeat_ms
            > self.heartbeat_interval_s * HEARTBEAT_TIMEOUT_MULTIPLIER * 1000
    }

    /// 最新检查点
    pub fn latest_checkpoint(&self) -> Option<&Checkpoint> {
        self.checkpoints.last()
    }
}

/// 长时任务调度器
#[derive(Debug, Default)]
pub struct LongRunningScheduler {
    /// 任务表
    pub tasks: HashMap<String, LongRunningTask>,
}

impl LongRunningScheduler {
    /// 创建空调度器
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册长时任务，返回 task_id
    pub fn register(&mut self, goal: &str, heartbeat_interval_s: i64) -> String {
        let id = format!("lrt-{}", Utc::now().timestamp_millis());
        let task = LongRunningTask::new(&id, goal, heartbeat_interval_s);
        self.tasks.insert(id.clone(), task);
        self.persist_quiet();
        id
    }

    /// 获取任务
    pub fn get(&self, id: &str) -> Option<&LongRunningTask> {
        self.tasks.get(id)
    }

    /// 心跳保活
    pub fn heartbeat(&mut self, id: &str) -> Result<()> {
        let t = self
            .tasks
            .get_mut(id)
            .ok_or_else(|| anyhow!("未找到长时任务: {id}"))?;
        t.heartbeat();
        Ok(())
    }

    /// 保存检查点
    pub fn save_checkpoint(
        &mut self,
        id: &str,
        stage: Stage,
        state: serde_json::Value,
    ) -> Result<()> {
        let t = self
            .tasks
            .get_mut(id)
            .ok_or_else(|| anyhow!("未找到长时任务: {id}"))?;
        t.save_checkpoint(stage, state);
        self.persist_quiet();
        Ok(())
    }

    /// 超时检测：把心跳超时的 running 任务转 paused，返回被暂停的任务 id。
    pub fn timeout_detect(&mut self, now_ms: i64) -> Vec<String> {
        let mut paused = Vec::new();
        for t in self.tasks.values_mut() {
            if t.status == TaskStatus::Running && t.is_heartbeat_stale(now_ms) {
                t.status = TaskStatus::Paused;
                paused.push(t.task_id.clone());
            }
        }
        if !paused.is_empty() {
            self.persist_quiet();
        }
        paused
    }

    /// 自动续跑：paused 且有检查点且未达 max_retries → Resumed（retry+1）。
    pub fn resume(&mut self, id: &str) -> Result<()> {
        let t = self
            .tasks
            .get_mut(id)
            .ok_or_else(|| anyhow!("未找到长时任务: {id}"))?;
        if t.status != TaskStatus::Paused {
            return Err(anyhow!("任务非 paused 状态，无法续跑: {:?}", t.status));
        }
        if t.latest_checkpoint().is_none() {
            return Err(anyhow!("无检查点，无法续跑"));
        }
        if t.retry_count >= t.max_retries {
            return Err(anyhow!("已达最大重试次数 {}", t.max_retries));
        }
        t.retry_count += 1;
        t.status = TaskStatus::Resumed;
        t.heartbeat();
        self.persist_quiet();
        Ok(())
    }

    /// 显式暂停：running/resumed → paused（心跳保活停止，等待续跑）
    pub fn pause(&mut self, id: &str) -> Result<()> {
        let t = self
            .tasks
            .get_mut(id)
            .ok_or_else(|| anyhow!("未找到长时任务: {id}"))?;
        if !matches!(t.status, TaskStatus::Running | TaskStatus::Resumed) {
            return Err(anyhow!(
                "任务非 running/resumed 状态，无法暂停: {:?}",
                t.status
            ));
        }
        t.status = TaskStatus::Paused;
        self.persist_quiet();
        Ok(())
    }

    /// 标记完成
    pub fn complete(&mut self, id: &str) -> Result<()> {
        let t = self
            .tasks
            .get_mut(id)
            .ok_or_else(|| anyhow!("未找到长时任务: {id}"))?;
        t.status = TaskStatus::Completed;
        self.persist_quiet();
        Ok(())
    }

    /// 标记失败
    pub fn fail(&mut self, id: &str) -> Result<()> {
        let t = self
            .tasks
            .get_mut(id)
            .ok_or_else(|| anyhow!("未找到长时任务: {id}"))?;
        t.status = TaskStatus::Failed;
        self.persist_quiet();
        Ok(())
    }

    /// 列出所有任务
    pub fn list(&self) -> Vec<&LongRunningTask> {
        self.tasks.values().collect()
    }

    /// 跨会话恢复：启动时把 running 但心跳超时的任务转 paused 等待续跑。
    pub fn restore_on_startup(&mut self, now_ms: i64) -> Vec<String> {
        self.timeout_detect(now_ms)
    }

    /// 启动恢复：上次未完成（running）的长任务在重启后视为中断，
    /// 一次性标记为 `Interrupted` 并持久化，供上层展示/人工处置。
    pub fn recover_unfinished_on_startup(&mut self) -> Vec<String> {
        let mut interrupted = Vec::new();
        for t in self.tasks.values_mut() {
            if t.status == TaskStatus::Running {
                t.status = TaskStatus::Interrupted;
                interrupted.push(t.task_id.clone());
            }
        }
        if !interrupted.is_empty() {
            self.persist_quiet();
        }
        interrupted
    }

    /// 查询未完成的长任务（非 completed/failed/interrupted），供上层展示待办。
    pub fn unfinished(&self) -> Vec<&LongRunningTask> {
        self.tasks
            .values()
            .filter(|t| {
                matches!(
                    t.status,
                    TaskStatus::Pending
                        | TaskStatus::Running
                        | TaskStatus::Paused
                        | TaskStatus::Resumed
                )
            })
            .collect()
    }
}

/// 全局长时任务调度器单例（网关/引擎/工具共用；进程内跨会话保活）。
pub fn global() -> &'static std::sync::Mutex<LongRunningScheduler> {
    static S: std::sync::OnceLock<std::sync::Mutex<LongRunningScheduler>> =
        std::sync::OnceLock::new();
    S.get_or_init(|| {
        let mut s = LongRunningScheduler::new();
        // 启动恢复：跨会话把 running 但心跳超时的任务转 paused（内存态恢复，
        // 同时从 SQLite 拉回已完成/未完成任务，保证 RPC 列表跨重启完整）。
        if let Err(e) = s.restore_from_sqlite() {
            tracing::warn!(err = %e, "长时任务 SQLite 恢复失败（继续内存态运行）");
        }
        std::sync::Mutex::new(s)
    })
}

// ---------------------------------------------------------------------------
// SQLite 持久化（跨会话恢复；rusqlite bundled，无外部依赖）
// ---------------------------------------------------------------------------

/// 长时任务数据库路径（`~/.marguerite/long_running.db`；测试可用 `MARGUERITE_LONG_RUNNING_DB` 覆盖）
pub fn sqlite_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("MARGUERITE_LONG_RUNNING_DB") {
        if !p.trim().is_empty() {
            return std::path::PathBuf::from(p);
        }
    }
    crate::persistence::config::Config::config_dir().join("long_running.db")
}

/// 打开（自动建目录 + 建表）
fn open_db() -> Result<rusqlite::Connection> {
    let path = sqlite_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let db = rusqlite::Connection::open(&path)?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS long_running_tasks (
            task_id TEXT PRIMARY KEY,
            goal TEXT NOT NULL,
            status TEXT NOT NULL,
            started_at_ms INTEGER NOT NULL,
            last_heartbeat_ms INTEGER NOT NULL,
            heartbeat_interval_s INTEGER NOT NULL,
            max_duration_hours REAL NOT NULL,
            retry_count INTEGER NOT NULL,
            max_retries INTEGER NOT NULL,
            checkpoints_json TEXT NOT NULL,
            updated_at_ms INTEGER NOT NULL
        );",
    )?;
    Ok(db)
}

fn status_str(s: TaskStatus) -> &'static str {
    s.as_str()
}

fn status_from_str(s: &str) -> TaskStatus {
    match s {
        "pending" => TaskStatus::Pending,
        "running" => TaskStatus::Running,
        "paused" => TaskStatus::Paused,
        "completed" => TaskStatus::Completed,
        "failed" => TaskStatus::Failed,
        "resumed" => TaskStatus::Resumed,
        "interrupted" => TaskStatus::Interrupted,
        _ => TaskStatus::Pending,
    }
}

fn checkpoint_json(cps: &[Checkpoint]) -> String {
    serde_json::to_string(cps).unwrap_or_else(|_| "[]".to_string())
}

fn checkpoint_from_json(s: &str) -> Vec<Checkpoint> {
    serde_json::from_str(s).unwrap_or_default()
}

impl LongRunningScheduler {
    /// 持久化全部任务到 SQLite（幂等 upsert）。
    pub fn persist(&self) -> Result<()> {
        let db = open_db()?;
        let now = Utc::now().timestamp_millis();
        for t in self.tasks.values() {
            db.execute(
                "INSERT INTO long_running_tasks
                    (task_id, goal, status, started_at_ms, last_heartbeat_ms,
                     heartbeat_interval_s, max_duration_hours, retry_count, max_retries,
                     checkpoints_json, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(task_id) DO UPDATE SET
                    goal=excluded.goal, status=excluded.status,
                    started_at_ms=excluded.started_at_ms, last_heartbeat_ms=excluded.last_heartbeat_ms,
                    heartbeat_interval_s=excluded.heartbeat_interval_s,
                    max_duration_hours=excluded.max_duration_hours,
                    retry_count=excluded.retry_count, max_retries=excluded.max_retries,
                    checkpoints_json=excluded.checkpoints_json, updated_at_ms=excluded.updated_at_ms",
                rusqlite::params![
                    t.task_id,
                    t.goal,
                    status_str(t.status),
                    t.started_at_ms,
                    t.last_heartbeat_ms,
                    t.heartbeat_interval_s,
                    t.max_duration_hours,
                    t.retry_count,
                    t.max_retries,
                    checkpoint_json(&t.checkpoints),
                    now,
                ],
            )?;
        }
        Ok(())
    }

    /// 从 SQLite 恢复全部任务（跨会话恢复；内存态为空时加载，已有则不覆盖）。
    pub fn restore_from_sqlite(&mut self) -> Result<()> {
        let db = open_db()?;
        let mut stmt = db.prepare(
            "SELECT task_id, goal, status, started_at_ms, last_heartbeat_ms,
                    heartbeat_interval_s, max_duration_hours, retry_count, max_retries,
                    checkpoints_json
             FROM long_running_tasks",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, f64>(6)?,
                row.get::<_, u32>(7)?,
                row.get::<_, u32>(8)?,
                row.get::<_, String>(9)?,
            ))
        })?;
        for row in rows.flatten() {
            let (
                task_id,
                goal,
                status,
                started,
                heartbeat,
                interval,
                max_hours,
                retry,
                max_retries,
                cps,
            ) = row;
            if self.tasks.contains_key(&task_id) {
                continue; // 不覆盖内存中已在运行的任务
            }
            self.tasks.insert(
                task_id.clone(),
                LongRunningTask {
                    task_id,
                    goal,
                    started_at_ms: started,
                    last_heartbeat_ms: heartbeat,
                    status: status_from_str(&status),
                    checkpoints: checkpoint_from_json(&cps),
                    heartbeat_interval_s: interval,
                    max_duration_hours: max_hours,
                    retry_count: retry,
                    max_retries,
                },
            );
        }
        // 跨会话恢复语义：上次未完成（running）的长任务在重启后标记为中断
        if !self.tasks.is_empty() {
            self.recover_unfinished_on_startup();
        }
        Ok(())
    }

    /// 便捷：持久化当前状态（供 RPC/runner 变更后调用）。
    pub fn persist_quiet(&self) {
        if let Err(e) = self.persist() {
            tracing::warn!(err = %e, "长时任务持久化失败（忽略，继续内存态）");
        }
    }
}

/// 任务只读画像（供 RPC 与前端展示）
pub fn task_view(t: &LongRunningTask) -> serde_json::Value {
    serde_json::json!({
        "taskId": t.task_id,
        "goal": t.goal,
        "status": t.status.as_str(),
        "startedAtMs": t.started_at_ms,
        "lastHeartbeatMs": t.last_heartbeat_ms,
        "heartbeatIntervalS": t.heartbeat_interval_s,
        "maxDurationHours": t.max_duration_hours,
        "retryCount": t.retry_count,
        "maxRetries": t.max_retries,
        "checkpoints": t.checkpoints.iter().map(|c| serde_json::json!({
            "stage": c.stage.as_str(),
            "state": c.state,
            "timestampMs": c.timestamp_ms,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 本模块全部测试共享进程级 SQLite 持久化：`sqlite_persist_and_restore_roundtrip`
    /// 会通过 `MARGUERITE_LONG_RUNNING_DB` 环境变量切换库路径，而其余测试的
    /// `register()` 内部 `persist_quiet()` 也会落库（默认配置目录或环境变量指向的临时库），
    /// 并行执行会互相污染（恢复计数偏差 / 心跳误判）。统一串行化保证确定性。
    static TEST_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    fn lock_tests() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn register_creates_running_task() {
        let _g = lock_tests();
        let mut s = LongRunningScheduler::new();
        let id = s.register("全量迁移", 30);
        let t = s.get(&id).unwrap();
        assert_eq!(t.status, TaskStatus::Running);
        assert_eq!(t.goal, "全量迁移");
    }

    #[test]
    fn heartbeat_updates_timestamp() {
        let _g = lock_tests();
        let mut s = LongRunningScheduler::new();
        let id = s.register("x", 30);
        // 推进任务时间：直接改 started 较早，再 heartbeat
        s.tasks.get_mut(&id).unwrap().last_heartbeat_ms -= 1000;
        let before = s.get(&id).unwrap().last_heartbeat_ms;
        s.heartbeat(&id).unwrap();
        let after = s.get(&id).unwrap().last_heartbeat_ms;
        assert!(after >= before);
    }

    #[test]
    fn save_and_latest_checkpoint() {
        let _g = lock_tests();
        let mut s = LongRunningScheduler::new();
        let id = s.register("x", 30);
        s.save_checkpoint(&id, Stage::Plan, json!({"step": 1}))
            .unwrap();
        s.save_checkpoint(&id, Stage::Execute, json!({"step": 2}))
            .unwrap();
        let cp = s.get(&id).unwrap().latest_checkpoint().unwrap();
        assert_eq!(cp.stage, Stage::Execute);
    }

    #[test]
    fn timeout_detect_pauses_stale_running() {
        let _g = lock_tests();
        let mut s = LongRunningScheduler::new();
        let id = s.register("x", 30); // interval=30s, timeout=90s
        let now = Utc::now().timestamp_millis();
        // 把最后心跳推到 200 秒前 → 超时
        s.tasks.get_mut(&id).unwrap().last_heartbeat_ms = now - 200_000;
        let paused = s.timeout_detect(now);
        assert_eq!(paused, vec![id.clone()]);
        assert_eq!(s.get(&id).unwrap().status, TaskStatus::Paused);
    }

    #[test]
    fn timeout_detect_keeps_fresh_running() {
        let _g = lock_tests();
        let mut s = LongRunningScheduler::new();
        let id = s.register("x", 30);
        let now = Utc::now().timestamp_millis();
        // 心跳就在当前 → 不超时
        let paused = s.timeout_detect(now);
        assert!(paused.is_empty());
        assert_eq!(s.get(&id).unwrap().status, TaskStatus::Running);
    }

    #[test]
    fn resume_from_paused_with_checkpoint() {
        let _g = lock_tests();
        let mut s = LongRunningScheduler::new();
        let id = s.register("x", 30);
        let now = Utc::now().timestamp_millis();
        s.tasks.get_mut(&id).unwrap().last_heartbeat_ms = now - 200_000;
        s.timeout_detect(now);
        s.save_checkpoint(&id, Stage::Execute, json!({"p": 1}))
            .unwrap();
        s.resume(&id).unwrap();
        let t = s.get(&id).unwrap();
        assert_eq!(t.status, TaskStatus::Resumed);
        assert_eq!(t.retry_count, 1);
    }

    #[test]
    fn resume_rejects_no_checkpoint() {
        let _g = lock_tests();
        let mut s = LongRunningScheduler::new();
        let id = s.register("x", 30);
        let now = Utc::now().timestamp_millis();
        s.tasks.get_mut(&id).unwrap().last_heartbeat_ms = now - 200_000;
        s.timeout_detect(now);
        // 无 checkpoint → 拒绝
        assert!(s.resume(&id).is_err());
    }

    #[test]
    fn resume_rejects_max_retries() {
        let _g = lock_tests();
        let mut s = LongRunningScheduler::new();
        let id = s.register("x", 30);
        let now = Utc::now().timestamp_millis();
        s.tasks.get_mut(&id).unwrap().last_heartbeat_ms = now - 200_000;
        s.timeout_detect(now);
        s.save_checkpoint(&id, Stage::Execute, json!({})).unwrap();
        let t = s.tasks.get_mut(&id).unwrap();
        t.retry_count = t.max_retries; // 已达上限
        assert!(s.resume(&id).is_err());
    }

    #[test]
    fn complete_and_fail_transitions() {
        let _g = lock_tests();
        let mut s = LongRunningScheduler::new();
        let id = s.register("x", 30);
        s.complete(&id).unwrap();
        assert_eq!(s.get(&id).unwrap().status, TaskStatus::Completed);
        s.fail(&id).unwrap();
        assert_eq!(s.get(&id).unwrap().status, TaskStatus::Failed);
    }

    #[test]
    fn restore_on_startup_pauses_stale() {
        let _g = lock_tests();
        let mut s = LongRunningScheduler::new();
        let id = s.register("x", 30);
        let now = Utc::now().timestamp_millis();
        s.tasks.get_mut(&id).unwrap().last_heartbeat_ms = now - 200_000;
        let restored = s.restore_on_startup(now);
        assert_eq!(restored, vec![id]);
    }

    #[test]
    fn sqlite_persist_and_restore_roundtrip() {
        let _g = lock_tests();
        // 临时目录 + 环境变量隔离（sqlite_path 支持 MARGUERITE_LONG_RUNNING_DB 覆盖），
        // 验证「注册→持久化→新调度器恢复」跨会话语义。
        let tmp =
            std::env::temp_dir().join(format!("mrg_lrt_test_{}", Utc::now().timestamp_millis()));
        std::fs::create_dir_all(&tmp).unwrap();
        let db_path = tmp.join("lrt.db");
        std::env::set_var(
            "MARGUERITE_LONG_RUNNING_DB",
            db_path.to_string_lossy().to_string(),
        );

        // 第一代调度器：注册 + 检查点 + 持久化
        {
            let mut s = LongRunningScheduler::new();
            let id = s.register("全量迁移", 30);
            s.save_checkpoint(&id, Stage::Plan, serde_json::json!({"step": 1}))
                .unwrap();
            s.persist().unwrap();
            assert!(db_path.exists(), "持久化后 db 应存在");
        }

        // 第二代调度器：恢复应看到同一任务与检查点（跨会话）
        {
            let mut s2 = LongRunningScheduler::new();
            s2.restore_from_sqlite().unwrap();
            assert_eq!(s2.tasks.len(), 1, "恢复后应有 1 个任务");
            let t = s2.tasks.values().next().unwrap();
            assert_eq!(t.goal, "全量迁移");
            assert_eq!(t.latest_checkpoint().map(|c| c.stage), Some(Stage::Plan));
            assert_eq!(t.checkpoints.len(), 1);
        }

        std::env::remove_var("MARGUERITE_LONG_RUNNING_DB");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn sqlite_restart_recovers_running_to_interrupted() {
        let _g = lock_tests();
        // 写入一个 running 任务 → 模拟重启（重新打开 DB）→ 恢复到 interrupted。
        let tmp =
            std::env::temp_dir().join(format!("mrg_lrt_recover_{}", Utc::now().timestamp_millis()));
        std::fs::create_dir_all(&tmp).unwrap();
        let db_path = tmp.join("lrt.db");
        std::env::set_var(
            "MARGUERITE_LONG_RUNNING_DB",
            db_path.to_string_lossy().to_string(),
        );

        let running_id;
        {
            let mut s = LongRunningScheduler::new();
            running_id = s.register("全量迁移", 30); // status = Running，register 内部已持久化
            s.persist().unwrap();
            assert_eq!(s.get(&running_id).unwrap().status, TaskStatus::Running);
        }

        // 第二代（模拟应用重启）：重新打开 DB 并恢复
        {
            let mut s2 = LongRunningScheduler::new();
            s2.restore_from_sqlite().unwrap();
            let t = s2.get(&running_id).unwrap();
            assert_eq!(
                t.status,
                TaskStatus::Interrupted,
                "重启后上次未完成的 running 任务应标记为 interrupted"
            );
            // 查询接口：interrupted 任务不应出现在未完成列表
            assert!(
                !s2.unfinished().iter().any(|x| x.task_id == running_id),
                "中断任务不应算作未完成"
            );
        }

        std::env::remove_var("MARGUERITE_LONG_RUNNING_DB");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
