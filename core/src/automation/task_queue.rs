//! 持久任务队列（M2-C6 Persistent Task Queue）
//!
//! 跨会话 Backlog 队列：任务先入队（排队等待），支持 暂停 / 恢复 / 取消，
//! 生命周期由 SQLite 落盘，进程重启后可跨会话恢复延续。
//!
//! 状态机：
//! ```text
//!            enqueue                start               complete
//!   (none) ────────► Queued ───────────► Running ────────────────► Completed
//!                    │  ▲                │  │
//!          pause / cancel │ resume        │  cancel / fail
//!                    ▼  │                ▼  ▼
//!                  Paused ──────────► Cancelled / Failed
//! ```
//! - `pause`：Queued / Running → Paused
//! - `resume`：Paused → Queued（重新排队等待调度）
//! - `cancel`：Queued / Paused / Running → Cancelled（不可逆）
//! - `fail`：Running / Queued → Failed（记录错误）

use std::sync::Mutex;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// 队列任务状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueStatus {
    /// 排队中（Backlog）
    Queued,
    /// 运行中
    Running,
    /// 已暂停
    Paused,
    /// 已取消
    Cancelled,
    /// 已完成
    Completed,
    /// 失败
    Failed,
}

impl QueueStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            QueueStatus::Queued => "queued",
            QueueStatus::Running => "running",
            QueueStatus::Paused => "paused",
            QueueStatus::Cancelled => "cancelled",
            QueueStatus::Completed => "completed",
            QueueStatus::Failed => "failed",
        }
    }

    pub fn from_key(s: &str) -> QueueStatus {
        match s {
            "queued" => QueueStatus::Queued,
            "running" => QueueStatus::Running,
            "paused" => QueueStatus::Paused,
            "cancelled" => QueueStatus::Cancelled,
            "completed" => QueueStatus::Completed,
            "failed" => QueueStatus::Failed,
            _ => QueueStatus::Queued,
        }
    }

    /// 仍在活跃生命周期中（可继续演进）
    pub fn is_active(&self) -> bool {
        matches!(self, QueueStatus::Queued | QueueStatus::Running | QueueStatus::Paused)
    }
}

/// 队列中的任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueItem {
    /// 任务 ID
    pub id: String,
    /// 展示标题
    pub title: String,
    /// 完整目标
    pub goal: String,
    /// 目标模式
    pub mode: String,
    /// 提交会话
    pub session: String,
    /// 状态
    pub status: QueueStatus,
    /// 优先级（数值越小越先调度）
    pub priority: i32,
    /// 创建时间（毫秒）
    pub created_at_ms: i64,
    /// 最近更新时间（毫秒）
    pub updated_at_ms: i64,
    /// 失败原因
    pub error: Option<String>,
    /// 附加元数据（JSON 字符串）
    pub metadata: String,
}

/// 持久任务队列
#[derive(Debug, Default)]
pub struct TaskQueue {
    items: Vec<QueueItem>,
    seq: u64,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl TaskQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// 入队（排队等待）。返回任务 ID。
    pub fn enqueue(
        &mut self,
        title: impl Into<String>,
        goal: impl Into<String>,
        mode: impl Into<String>,
        session: impl Into<String>,
        priority: i32,
    ) -> String {
        self.seq += 1;
        let id = format!("q_{}", self.seq);
        let now = now_ms();
        let title_s = title.into();
        let session_s = session.into();
        self.items.push(QueueItem {
            id: id.clone(),
            title: title_s.clone(),
            goal: goal.into(),
            mode: mode.into(),
            session: session_s.clone(),
            status: QueueStatus::Queued,
            priority,
            created_at_ms: now,
            updated_at_ms: now,
            error: None,
            metadata: "{}".into(),
        });
        // M3-D1 产品事件：任务发起
        crate::events::product::emit(
            crate::events::product::TASK_INITIATED,
            &session_s,
            serde_json::json!({ "task_id": id, "title": title_s, "status": "queued" }),
        );
        id
    }

    /// 开始执行：Queued → Running
    pub fn start(&mut self, id: &str) -> bool {
        self.transition(id, |s| matches!(s, QueueStatus::Queued), QueueStatus::Running, None)
    }

    /// 暂停：Queued / Running → Paused
    pub fn pause(&mut self, id: &str) -> bool {
        let i = match self.items.iter_mut().find(|i| i.id == id) {
            Some(i) => i,
            None => return false,
        };
        match i.status {
            QueueStatus::Queued | QueueStatus::Running => {
                i.status = QueueStatus::Paused;
                i.updated_at_ms = now_ms();
                true
            }
            _ => false,
        }
    }

    /// 恢复：Paused → Queued（重新排队）
    pub fn resume(&mut self, id: &str) -> bool {
        let i = match self.items.iter_mut().find(|i| i.id == id) {
            Some(i) => i,
            None => return false,
        };
        if i.status == QueueStatus::Paused {
            i.status = QueueStatus::Queued;
            i.updated_at_ms = now_ms();
            true
        } else {
            false
        }
    }

    /// 取消：Queued / Paused / Running → Cancelled（不可逆）
    pub fn cancel(&mut self, id: &str) -> bool {
        let i = match self.items.iter_mut().find(|i| i.id == id) {
            Some(i) => i,
            None => return false,
        };
        if i.status.is_active() {
            i.status = QueueStatus::Cancelled;
            i.updated_at_ms = now_ms();
            // M3-D1 产品事件：任务取消
            crate::events::product::emit(
                crate::events::product::TASK_CANCELLED,
                &i.session,
                serde_json::json!({ "task_id": id }),
            );
            true
        } else {
            false
        }
    }

    /// 完成：Running → Completed
    pub fn complete(&mut self, id: &str) -> bool {
        let session = self.items.iter().find(|i| i.id == id).map(|i| i.session.clone());
        let ok = self.transition(id, |s| s == QueueStatus::Running, QueueStatus::Completed, None);
        if ok {
            // M3-D1 产品事件：任务完成
            crate::events::product::emit(
                crate::events::product::TASK_COMPLETED,
                &session.unwrap_or_default(),
                serde_json::json!({ "task_id": id, "status": "completed" }),
            );
        }
        ok
    }

    /// 失败：Running / Queued → Failed
    pub fn fail(&mut self, id: &str, err: impl Into<String>) -> bool {
        let msg = err.into();
        let session = self.items.iter().find(|i| i.id == id).map(|i| i.session.clone());
        let i = match self.items.iter_mut().find(|i| i.id == id) {
            Some(i) => i,
            None => return false,
        };
        match i.status {
            QueueStatus::Running | QueueStatus::Queued => {
                i.status = QueueStatus::Failed;
                i.error = Some(msg.clone());
                i.updated_at_ms = now_ms();
                // M3-D1 产品事件：任务失败
                crate::events::product::emit(
                    crate::events::product::TASK_FAILED,
                    &session.unwrap_or_default(),
                    serde_json::json!({ "task_id": id, "error": msg }),
                );
                true
            }
            _ => false,
        }
    }

    fn transition(
        &mut self,
        id: &str,
        from: impl Fn(QueueStatus) -> bool,
        to: QueueStatus,
        err: Option<String>,
    ) -> bool {
        let i = match self.items.iter_mut().find(|i| i.id == id) {
            Some(i) => i,
            None => return false,
        };
        if from(i.status) {
            i.status = to;
            i.error = err;
            i.updated_at_ms = now_ms();
            true
        } else {
            false
        }
    }

    /// 下一个待调度任务（优先级最高 → 队列最老；仅 Queued）
    pub fn next_pending(&self) -> Option<&QueueItem> {
        self.items
            .iter()
            .filter(|i| i.status == QueueStatus::Queued)
            .min_by_key(|i| (i.priority, i.created_at_ms, i.id.clone()))
    }

    /// 调整调度顺序：与相邻待调度（Queued）任务交换优先级。
    /// `up=true` 上移一步（提高优先级、往前插队）；`up=false` 下移一步（让位）。
    /// 仅对 Queued 任务且存在相邻 Queued 时生效；返回是否发生交换。
    pub fn move_step(&mut self, id: &str, up: bool) -> bool {
        let mut order: Vec<(String, i32, i64)> = self
            .items
            .iter()
            .filter(|i| i.status == QueueStatus::Queued)
            .map(|i| (i.id.clone(), i.priority, i.created_at_ms))
            .collect();
        // 调度序 =（优先级, 入队时间），与 next_pending 排序口径一致
        order.sort_by_key(|(_, p, c)| (*p, *c));
        if order.is_empty() {
            return false;
        }
        let pos = order
            .iter()
            .position(|(oid, _, _)| oid == id)
            .unwrap_or(usize::MAX);
        if pos == usize::MAX {
            return false;
        }
        let target = if up {
            pos.checked_sub(1).unwrap_or(usize::MAX)
        } else {
            if pos + 1 >= order.len() {
                usize::MAX
            } else {
                pos + 1
            }
        };
        if target == usize::MAX {
            return false;
        }
        let cur = order[pos].clone();
        let other = order[target].clone();
        if cur.0 == other.0 {
            return false;
        }
        for i in self.items.iter_mut().filter(|i| i.id == cur.0 || i.id == other.0) {
            if i.status != QueueStatus::Queued {
                return false;
            }
        }
        let i1 = self.items.iter_mut().find(|i| i.id == cur.0).unwrap();
        i1.priority = other.1;
        i1.updated_at_ms = now_ms();
        let i2 = self.items.iter_mut().find(|i| i.id == other.0).unwrap();
        i2.priority = cur.1;
        i2.updated_at_ms = now_ms();
        true
    }

    /// 全部任务（创建顺序）
    pub fn list(&self) -> Vec<&QueueItem> {
        let mut v: Vec<&QueueItem> = self.items.iter().collect();
        v.sort_by_key(|i| i.created_at_ms);
        v
    }

    /// 按 id 取任务
    pub fn get(&self, id: &str) -> Option<&QueueItem> {
        self.items.iter().find(|i| i.id == id)
    }

    /// 任务数
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

// ---------------------------------------------------------------------------
// SQLite 持久化（跨会话 Backlog 保留；rusqlite bundled）
// ---------------------------------------------------------------------------

/// 队列库路径（`~/.marguerite/task_queue.db`；测试可用 `MARGUERITE_QUEUE_DB` 覆盖）
pub fn sqlite_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("MARGUERITE_QUEUE_DB") {
        if !p.trim().is_empty() {
            return std::path::PathBuf::from(p);
        }
    }
    crate::persistence::config::Config::config_dir().join("task_queue.db")
}

mod db {
    pub fn open() -> Result<rusqlite::Connection, Box<dyn std::error::Error + Send + Sync>> {
        let path = super::sqlite_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = rusqlite::Connection::open(&path)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS queue_items (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                goal TEXT NOT NULL,
                mode TEXT NOT NULL,
                session TEXT NOT NULL,
                status TEXT NOT NULL,
                priority INTEGER NOT NULL,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                error TEXT,
                metadata TEXT NOT NULL
            );",
        )?;
        Ok(db)
    }
}

impl TaskQueue {
    /// 持久化全部任务到 SQLite（幂等 upsert）。
    pub fn persist(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = db::open()?;
        for i in &self.items {
            conn.execute(
                "INSERT INTO queue_items
                    (id, title, goal, mode, session, status, priority,
                     created_at_ms, updated_at_ms, error, metadata)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(id) DO UPDATE SET
                    title=excluded.title, goal=excluded.goal, mode=excluded.mode,
                    session=excluded.session, status=excluded.status, priority=excluded.priority,
                    created_at_ms=excluded.created_at_ms, updated_at_ms=excluded.updated_at_ms,
                    error=excluded.error, metadata=excluded.metadata",
                rusqlite::params![
                    i.id, i.title, i.goal, i.mode, i.session, i.status.as_str(), i.priority,
                    i.created_at_ms, i.updated_at_ms, i.error, i.metadata
                ],
            )?;
        }
        Ok(())
    }

    /// 从 SQLite 恢复全部任务（已有 id 不覆盖）。
    pub fn restore(&mut self) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let conn = db::open()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, goal, mode, session, status, priority,
                    created_at_ms, updated_at_ms, error, metadata
             FROM queue_items",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(QueueItem {
                id: row.get::<_, String>(0)?,
                title: row.get::<_, String>(1)?,
                goal: row.get::<_, String>(2)?,
                mode: row.get::<_, String>(3)?,
                session: row.get::<_, String>(4)?,
                status: QueueStatus::from_key(&row.get::<_, String>(5)?),
                priority: row.get::<_, i32>(6)?,
                created_at_ms: row.get::<_, i64>(7)?,
                updated_at_ms: row.get::<_, i64>(8)?,
                error: row.get::<_, Option<String>>(9)?,
                metadata: row.get::<_, String>(10)?,
            })
        })?;
        let mut loaded = 0;
        for row in rows {
            let i = row?;
            if !self.items.iter().any(|x| x.id == i.id) {
                if let Ok(n) = i.id.trim_start_matches("q_").parse::<u64>() {
                    self.seq = self.seq.max(n);
                }
                self.items.push(i);
                loaded += 1;
            }
        }
        Ok(loaded)
    }
}

// ---------------------------------------------------------------------------
// 全局单例（网关 RPC / 前端队列视图用）
// ---------------------------------------------------------------------------

static QUEUE: OnceLock<Mutex<TaskQueue>> = OnceLock::new();

/// 进程级全局任务队列
pub fn global() -> &'static Mutex<TaskQueue> {
    QUEUE.get_or_init(|| Mutex::new(TaskQueue::new()))
}

/// 启动时恢复跨会话 Backlog（best-effort）
pub fn restore_global() -> usize {
    let mut g = global().lock().unwrap_or_else(|e| e.into_inner());
    g.restore().unwrap_or(0)
}

/// 任务视图（供 RPC / WebUI）
pub fn item_view(i: &QueueItem) -> serde_json::Value {
    serde_json::json!({
        "id": i.id,
        "title": i.title,
        "goal": i.goal,
        "mode": i.mode,
        "session": i.session,
        "status": i.status.as_str(),
        "priority": i.priority,
        "created_at_ms": i.created_at_ms,
        "updated_at_ms": i.updated_at_ms,
        "error": i.error,
        "metadata": i.metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SQLite 测试依赖进程级环境变量 + 全局态，串行执行。
    static QUEUE_TESTS_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_db(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("marguerite_queue_t_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("queue.db");
        std::env::set_var("MARGUERITE_QUEUE_DB", path.as_os_str());
        path
    }

    #[test]
    fn enqueue_lists_backlog() {
        let _g = QUEUE_TESTS_LOCK.lock().unwrap();
        let mut q = TaskQueue::new();
        let a = q.enqueue("整理文档", "t1", "agent", "s1", 5);
        let b = q.enqueue("生成报告", "t2", "executor", "s1", 1);
        assert_eq!(q.len(), 2);
        assert_ne!(a, b);
        assert_eq!(q.get(&a).unwrap().status, QueueStatus::Queued);
        // priority 1 的先进
        assert_eq!(q.next_pending().unwrap().id, b);
    }

    #[test]
    fn lifecycle_start_pause_resume_cancel() {
        let _g = QUEUE_TESTS_LOCK.lock().unwrap();
        let mut q = TaskQueue::new();
        let id = q.enqueue("x", "g", "agent", "s", 1);
        assert!(q.start(&id));
        assert_eq!(q.get(&id).unwrap().status, QueueStatus::Running);
        assert!(q.pause(&id));
        assert_eq!(q.get(&id).unwrap().status, QueueStatus::Paused);
        // 暂停状态下 start 失败
        assert!(!q.start(&id));
        assert!(q.resume(&id));
        assert_eq!(q.get(&id).unwrap().status, QueueStatus::Queued);
        assert!(q.cancel(&id));
        assert_eq!(q.get(&id).unwrap().status, QueueStatus::Cancelled);
        // Cancelled 后不可再演进
        assert!(!q.start(&id));
        assert!(!q.pause(&id));
        assert!(!q.cancel(&id));
    }

    #[test]
    fn complete_and_fail() {
        let _g = QUEUE_TESTS_LOCK.lock().unwrap();
        let mut q = TaskQueue::new();
        let ok = q.enqueue("ok", "g", "agent", "s", 0);
        assert!(q.start(&ok));
        assert!(q.complete(&ok));
        assert_eq!(q.get(&ok).unwrap().status, QueueStatus::Completed);

        let bad = q.enqueue("bad", "g", "agent", "s", 0);
        assert!(q.start(&bad));
        assert!(q.fail(&bad, "磁盘不足"));
        assert_eq!(q.get(&bad).unwrap().status, QueueStatus::Failed);
        assert_eq!(q.get(&bad).unwrap().error.as_deref(), Some("磁盘不足"));
    }

    #[test]
    fn unknown_id_operations_fail() {
        let _g = QUEUE_TESTS_LOCK.lock().unwrap();
        let mut q = TaskQueue::new();
        assert!(!q.start("q_zzz"));
        assert!(!q.pause("q_zzz"));
        assert!(!q.resume("q_zzz"));
        assert!(!q.cancel("q_zzz"));
        assert!(!q.complete("q_zzz"));
        assert!(!q.fail("q_zzz", "e"));
        assert!(q.next_pending().is_none());
    }

    #[test]
    fn next_pending_prefers_priority_then_oldest() {
        let _g = QUEUE_TESTS_LOCK.lock().unwrap();
        let mut q = TaskQueue::new();
        q.enqueue("low", "g", "agent", "s", 9);
        q.enqueue("high", "g", "agent", "s", 1);
        q.enqueue("mid", "g", "agent", "s", 5);
        assert_eq!(q.next_pending().unwrap().title, "high");
        // 已完成的不参与调度
        let done = q.enqueue("done", "g", "agent", "s", 0);
        assert!(q.start(&done));
        assert!(q.complete(&done));
        assert_eq!(q.next_pending().unwrap().title, "high");
    }

    #[test]
    fn move_step_reorders_queued_priority() {
        let _g = QUEUE_TESTS_LOCK.lock().unwrap();
        let mut q = TaskQueue::new();
        let a = q.enqueue("a", "g", "agent", "s", 3);
        let b = q.enqueue("b", "g", "agent", "s", 2);
        let c = q.enqueue("c", "g", "agent", "s", 1);
        assert_eq!(q.next_pending().unwrap().id, c);
        // b 上移一步 → b 与 c 交换优先级，b 成为最优先
        assert!(q.move_step(&b, true));
        assert_eq!(q.next_pending().unwrap().id, b);
        // a 上移一步 → a 与 c 交换优先级，a 领先于 c
        assert!(q.move_step(&a, true));
        assert_eq!(q.next_pending().unwrap().id, b);
        // 最优先的 b 不能再上移
        assert!(!q.move_step(&b, true));
        assert_eq!(q.next_pending().unwrap().id, b);
        // b 下移一步 → 让位于 a
        assert!(q.move_step(&b, false));
        assert_eq!(q.next_pending().unwrap().id, a);
        // 不存在/非 Queued 的任务不可调整顺序
        assert!(!q.move_step("q_zzz", true));
        let done = q.enqueue("done", "g", "agent", "s", 0);
        assert!(q.start(&done));
        assert!(q.complete(&done));
        assert!(!q.move_step(&done, true));
    }

    #[test]
    fn persists_and_restores_across_sessions() {
        let _g = QUEUE_TESTS_LOCK.lock().unwrap();
        let path = with_temp_db("p1");
        {
            let mut q = TaskQueue::new();
            let id = q.enqueue("跨会话任务", "goal1", "bloom", "sess_a", 3);
            assert!(q.pause(&id));
            q.persist().unwrap();
        }
        let mut q2 = TaskQueue::new();
        let n = q2.restore().unwrap();
        assert_eq!(n, 1);
        let item = q2.get("q_1").unwrap();
        assert_eq!(item.title, "跨会话任务");
        assert_eq!(item.status, QueueStatus::Paused); // 暂停态跨会话保留
        // 恢复后可继续生命周期
        assert!(q2.resume("q_1"));
        assert_eq!(q2.get("q_1").unwrap().status, QueueStatus::Queued);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn persists_running_and_completed() {
        let _g = QUEUE_TESTS_LOCK.lock().unwrap();
        let path = with_temp_db("p2");
        {
            let mut q = TaskQueue::new();
            let a = q.enqueue("run", "g", "agent", "s", 0);
            let b = q.enqueue("done", "g", "agent", "s", 0);
            assert!(q.start(&a));
            assert!(q.start(&b));
            assert!(q.complete(&b));
            q.persist().unwrap();
        }
        let mut q2 = TaskQueue::new();
        q2.restore().unwrap();
        assert_eq!(q2.get("q_1").unwrap().status, QueueStatus::Running);
        assert_eq!(q2.get("q_2").unwrap().status, QueueStatus::Completed);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
