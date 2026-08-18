//! 任务队列与持久化
//!
//! 借鉴 TUI 工作台 task_queue：SQLite 持久化任务队列，
//! 支持后台任务执行、状态跟踪、失败重试与并发控制。

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use tokio::sync::{Mutex, MutexGuard};
use uuid::Uuid;

/// 任务状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl TaskStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Pending,
        }
    }
}

/// 任务
#[derive(Debug, Clone)]
pub struct Task {
    /// 任务 ID
    pub id: String,
    /// 任务类型
    pub kind: String,
    /// 任务描述
    pub description: String,
    /// 状态
    pub status: TaskStatus,
    /// 错误信息（失败时）
    pub error: Option<String>,
    /// 结果
    pub result: Option<String>,
    /// 优先级（数字越大越先执行）
    pub priority: i32,
    /// 重试次数
    pub retries: u32,
    /// 最大重试次数
    pub max_retries: u32,
    /// 创建时间
    pub created_at: String,
    /// 更新时间
    pub updated_at: String,
}

impl Task {
    fn new(kind: &str, description: &str, priority: i32) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            id: Uuid::new_v4().to_string(),
            kind: kind.to_string(),
            description: description.to_string(),
            status: TaskStatus::Pending,
            error: None,
            result: None,
            priority,
            retries: 0,
            max_retries: 2,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

/// 任务处理器 trait
///
/// 由调用方实现具体任务逻辑。
#[async_trait::async_trait]
pub trait TaskHandler: Send + Sync {
    /// 执行任务，返回结果字符串
    async fn execute(&self, task: &Task) -> Result<String>;
}

/// 持久化任务队列
///
/// SQLite 持久化，支持并发消费与状态跟踪。
/// 数据库连接由 Arc<Mutex> 共享，队列可 Clone 供 worker 使用。
#[derive(Clone)]
pub struct TaskQueue {
    db: Arc<Mutex<Connection>>,
    /// 活跃任务计数
    active: Arc<AtomicU64>,
    /// 处理器
    handler: Arc<dyn TaskHandler>,
}

impl TaskQueue {
    /// 创建任务队列（处理器必填）
    pub fn new(path: &Path, handler: Arc<dyn TaskHandler>) -> Self {
        let conn = Connection::open(path).expect("open task queue db");
        Self {
            db: Arc::new(Mutex::new(conn)),
            active: Arc::new(AtomicU64::new(0)),
            handler,
        }
    }

    /// 初始化数据库（建表与索引）
    pub async fn init(&self) -> Result<()> {
        let conn = self.conn().await;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                description TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                error TEXT,
                result TEXT,
                priority INTEGER DEFAULT 0,
                retries INTEGER DEFAULT 0,
                max_retries INTEGER DEFAULT 2,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status, priority);",
        )?;
        Ok(())
    }

    /// 获取数据库连接锁
    async fn conn(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().await
    }

    /// 入队任务
    pub async fn enqueue(&self, kind: &str, description: &str, priority: i32) -> Result<String> {
        let task = Task::new(kind, description, priority);
        let conn = self.conn().await;
        conn.execute(
            "INSERT INTO tasks (id, kind, description, status, error, result, priority, retries, max_retries, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5, 0, ?6, ?7, ?7)",
            rusqlite::params![
                task.id,
                task.kind,
                task.description,
                TaskStatus::Pending.as_str(),
                task.priority,
                task.max_retries,
                task.created_at
            ],
        )?;
        Ok(task.id)
    }

    /// 取出下一个待执行任务（按优先级），原子标记为 Running
    pub async fn dequeue(&self) -> Result<Option<Task>> {
        let conn = self.conn().await;
        let mut stmt = conn.prepare(
            "SELECT id, kind, description, status, error, result, priority, retries, max_retries, created_at, updated_at
             FROM tasks
             WHERE status = 'pending'
             ORDER BY priority DESC, created_at ASC
             LIMIT 1",
        )?;
        let mut rows = stmt.query([])?;
        let row = rows.next()?;
        let mut task: Option<Task> = match row {
            Some(r) => Some(task_from_row(r)?),
            None => None,
        };

        if let Some(t) = task.as_mut() {
            conn.execute(
                "UPDATE tasks SET status = 'running', updated_at = ?1 WHERE id = ?2",
                rusqlite::params![Utc::now().to_rfc3339(), t.id],
            )?;
            t.status = TaskStatus::Running;
            self.active.fetch_add(1, Ordering::SeqCst);
        }
        Ok(task)
    }

    /// 完成任务
    pub async fn complete(&self, id: &str, result: &str) -> Result<()> {
        let conn = self.conn().await;
        conn.execute(
            "UPDATE tasks SET status = 'done', result = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![result, Utc::now().to_rfc3339(), id],
        )?;
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }

    /// 标记失败（自动重试，超出则 Failed）
    pub async fn fail(&self, id: &str, error: &str) -> Result<()> {
        let conn = self.conn().await;
        let (retries, max_retries): (i64, i64) = conn.query_row(
            "SELECT retries, max_retries FROM tasks WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        self.active.fetch_sub(1, Ordering::SeqCst);
        if retries < max_retries {
            conn.execute(
                "UPDATE tasks SET status = 'pending', retries = ?1, error = ?2, updated_at = ?3 WHERE id = ?4",
                rusqlite::params![retries + 1, error, Utc::now().to_rfc3339(), id],
            )?;
        } else {
            conn.execute(
                "UPDATE tasks SET status = 'failed', error = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![error, Utc::now().to_rfc3339(), id],
            )?;
        }
        Ok(())
    }

    /// 取消任务
    pub async fn cancel(&self, id: &str) -> Result<()> {
        let conn = self.conn().await;
        conn.execute(
            "UPDATE tasks SET status = 'cancelled', updated_at = ?1 WHERE id = ?2 AND status IN ('pending', 'running')",
            rusqlite::params![Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    /// 查询任务
    pub async fn get(&self, id: &str) -> Result<Option<Task>> {
        let conn = self.conn().await;
        let mut stmt = conn.prepare(
            "SELECT id, kind, description, status, error, result, priority, retries, max_retries, created_at, updated_at
             FROM tasks WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        let row = rows.next()?;
        match row {
            Some(r) => Ok(Some(task_from_row(r)?)),
            None => Ok(None),
        }
    }

    /// 列出任务（按更新时间倒序）
    pub async fn list(&self, limit: usize) -> Result<Vec<Task>> {
        let conn = self.conn().await;
        let mut stmt = conn.prepare(
            "SELECT id, kind, description, status, error, result, priority, retries, max_retries, created_at, updated_at
             FROM tasks ORDER BY updated_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], task_from_row)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// 统计各状态任务数
    pub async fn stats(&self) -> Result<std::collections::HashMap<String, usize>> {
        let conn = self.conn().await;
        let mut stmt = conn.prepare("SELECT status, COUNT(*) FROM tasks GROUP BY status")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        Ok(rows
            .filter_map(|r| r.ok())
            .map(|(k, v)| (k, v as usize))
            .collect())
    }

    /// 当前活跃任务数
    pub fn active_count(&self) -> u64 {
        self.active.load(Ordering::SeqCst)
    }

    /// 运行后台工作线程：持续消费队列
    ///
    /// 返回的 JoinHandle 用于停止（abort）。
    pub async fn run_worker(&self) -> Result<tokio::task::JoinHandle<()>> {
        let queue = self.clone();
        let handle = tokio::spawn(async move {
            loop {
                let Ok(Some(task)) = queue.dequeue().await else {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    continue;
                };
                match queue.handler.execute(&task).await {
                    Ok(result) => {
                        let _ = queue.complete(&task.id, &result).await;
                    }
                    Err(e) => {
                        let _ = queue.fail(&task.id, &e.to_string()).await;
                    }
                }
            }
        });
        Ok(handle)
    }
}

/// 从 SQL 行解析任务
fn task_from_row(r: &rusqlite::Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: r.get(0)?,
        kind: r.get(1)?,
        description: r.get(2)?,
        status: TaskStatus::from_str(&r.get::<_, String>(3)?),
        error: r.get(4)?,
        result: r.get(5)?,
        priority: r.get(6)?,
        retries: r.get::<_, i64>(7)? as u32,
        max_retries: r.get::<_, i64>(8)? as u32,
        created_at: r.get(9)?,
        updated_at: r.get(10)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct EchoHandler;

    #[async_trait::async_trait]
    impl TaskHandler for EchoHandler {
        async fn execute(&self, task: &Task) -> Result<String> {
            Ok(format!("done:{}", task.description))
        }
    }

    fn temp_db(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("marguerite_tq_{name}_{}.db", std::process::id()))
    }

    async fn setup(name: &str) -> TaskQueue {
        let path = temp_db(name);
        let _ = std::fs::remove_file(&path);
        let queue = TaskQueue::new(&path, Arc::new(EchoHandler));
        queue.init().await.unwrap();
        queue
    }

    #[tokio::test]
    async fn test_enqueue_dequeue_complete() {
        let queue = setup("e2e").await;
        let id = queue.enqueue("echo", "hello", 0).await.unwrap();
        let task = queue.dequeue().await.unwrap().unwrap();
        assert_eq!(task.id, id);
        assert_eq!(task.status, TaskStatus::Running);
        queue.complete(&id, "result").await.unwrap();
        let done = queue.get(&id).await.unwrap().unwrap();
        assert_eq!(done.status, TaskStatus::Done);
        let _ = std::fs::remove_file(temp_db("e2e"));
    }

    #[tokio::test]
    async fn test_priority_order() {
        let queue = setup("prio").await;
        queue.enqueue("t", "low", 0).await.unwrap();
        queue.enqueue("t", "high", 10).await.unwrap();
        let first = queue.dequeue().await.unwrap().unwrap();
        assert_eq!(first.description, "high");
        let _ = std::fs::remove_file(temp_db("prio"));
    }

    #[tokio::test]
    async fn test_fail_and_retry() {
        let queue = setup("retry").await;
        let id = queue.enqueue("t", "task", 0).await.unwrap();
        let task = queue.dequeue().await.unwrap().unwrap();
        queue.fail(&task.id, "boom").await.unwrap();
        // 重试后回到 pending
        let retried = queue.get(&id).await.unwrap().unwrap();
        assert_eq!(retried.status, TaskStatus::Pending);
        assert_eq!(retried.retries, 1);
        let _ = std::fs::remove_file(temp_db("retry"));
    }

    #[tokio::test]
    async fn test_fail_exhaust_retries() {
        let queue = setup("exhaust").await;
        let id = queue.enqueue("t", "task", 0).await.unwrap();
        for _ in 0..3 {
            let task = queue.dequeue().await.unwrap().unwrap();
            queue.fail(&task.id, "boom").await.unwrap();
        }
        let failed = queue.get(&id).await.unwrap().unwrap();
        assert_eq!(failed.status, TaskStatus::Failed);
        let _ = std::fs::remove_file(temp_db("exhaust"));
    }

    #[tokio::test]
    async fn test_cancel() {
        let queue = setup("cancel").await;
        let id = queue.enqueue("t", "task", 0).await.unwrap();
        queue.cancel(&id).await.unwrap();
        let task = queue.get(&id).await.unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Cancelled);
        assert!(queue.dequeue().await.unwrap().is_none());
        let _ = std::fs::remove_file(temp_db("cancel"));
    }

    #[tokio::test]
    async fn test_stats() {
        let queue = setup("stats").await;
        queue.enqueue("t", "a", 0).await.unwrap();
        queue.enqueue("t", "b", 0).await.unwrap();
        let stats = queue.stats().await.unwrap();
        assert_eq!(stats.get("pending"), Some(&2));
        let _ = std::fs::remove_file(temp_db("stats"));
    }

    #[tokio::test]
    async fn test_worker_executes() {
        let queue = setup("worker").await;
        let id = queue.enqueue("echo", "w", 0).await.unwrap();
        let handle = queue.run_worker().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        handle.abort();
        let task = queue.get(&id).await.unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Done);
        assert_eq!(task.result.as_deref(), Some("done:w"));
        let _ = std::fs::remove_file(temp_db("worker"));
    }
}
