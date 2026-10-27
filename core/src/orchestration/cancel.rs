//! 编排取消：按会话登记运行中的 run，供「停止」按钮真正取消执行。
//!
//! 为什么需要它：`chat.abort` 此前只 abort 了对话流任务（`runtimes().tasks`），
//! 蜂群 / 专家团 / 子智能体跑在**独立**的后台任务里 —— 用户点停止后界面显示已停，
//! 实际模型调用与工具仍在继续，泳道卡片也永远停在「运行中」。
//!
//! 设计取舍：
//! - 注册表按 `session` 分组、组内按 `runId` 去重，`CancelGuard` 随 `Orchestrator`
//!   一起 drop 时自动注销，不需要任何手动清理路径（避免泄漏与幽灵取消）。
//! - 取消是**协作式**的：成员任务在 `select!` 里等这个信号，命中后立刻收口并上报
//!   `member_completed(ok:false)`；调度层随后发 `run_failed`，前端卡片才会真正结束。
//! - 用 `AtomicBool + Notify` 而不是 `tokio-util` 的 `CancellationToken`：仓库没有
//!   该依赖，且这里只需要「一次性、不可恢复」的取消语义。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::Notify;

/// 一次 run 的取消信号（同一 run 内所有成员共享）。
#[derive(Debug, Default)]
pub struct CancelSignal {
    cancelled: AtomicBool,
    notify: Notify,
}

impl CancelSignal {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        // 唤醒所有正在 `cancelled()` 上等待的任务（成员可能同时有多个）。
        self.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// 等待取消。已取消时立即返回（幂等，可被多次 await）。
    pub async fn cancelled(&self) {
        loop {
            // 先建 notified 再查标志：避免「查完标志 → cancel() 发出通知 → 才开始等」的丢唤醒。
            let notified = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
            if self.is_cancelled() {
                return;
            }
        }
    }
}

type Registry = Mutex<HashMap<String, HashMap<String, Arc<CancelSignal>>>>;

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 取消句柄：持有期间该 run 可被 `cancel_session` 命中；drop 即注销。
#[derive(Debug)]
pub struct CancelGuard {
    session: String,
    run_id: String,
    signal: Arc<CancelSignal>,
}

impl CancelGuard {
    /// 供成员任务 `select!` 使用的信号（克隆后带进 spawned task）。
    pub fn signal(&self) -> Arc<CancelSignal> {
        self.signal.clone()
    }
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(runs) = reg.get_mut(&self.session) {
            runs.remove(&self.run_id);
            if runs.is_empty() {
                reg.remove(&self.session);
            }
        }
    }
}

/// 登记一个 run，返回可取消句柄。
pub fn register(session: &str, run_id: &str) -> CancelGuard {
    let signal = Arc::new(CancelSignal::default());
    {
        let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
        reg.entry(session.to_string())
            .or_default()
            .insert(run_id.to_string(), signal.clone());
    }
    CancelGuard {
        session: session.to_string(),
        run_id: run_id.to_string(),
        signal,
    }
}

/// 取消该会话下全部运行中的编排，返回被取消的 run 数。
pub fn cancel_session(session: &str) -> usize {
    let signals: Vec<Arc<CancelSignal>> = {
        let reg = registry().lock().unwrap_or_else(|e| e.into_inner());
        reg.get(session)
            .map(|runs| runs.values().cloned().collect())
            .unwrap_or_default()
    };
    for signal in &signals {
        signal.cancel();
    }
    signals.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancel_session_hits_only_its_own_runs() {
        let a = register("sess-a", "run-1");
        let _b = register("sess-b", "run-2");
        assert_eq!(cancel_session("sess-a"), 1);
        assert!(a.signal().is_cancelled());
        assert_eq!(cancel_session("sess-c"), 0);
        // 已取消的会话再次取消仍返回条目数（幂等）
        assert_eq!(cancel_session("sess-a"), 1);
    }

    #[tokio::test]
    async fn cancelled_wakes_a_waiting_task() {
        let guard = register("sess-wait", "run-w");
        let signal = guard.signal();
        let waiter = tokio::spawn(async move {
            signal.cancelled().await;
            true
        });
        // 让 waiter 先进入等待，再取消
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        cancel_session("sess-wait");
        let woke = tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
            .await
            .expect("取消信号必须唤醒等待中的任务")
            .expect("waiter 不应 panic");
        assert!(woke);
    }

    #[tokio::test]
    async fn drop_unregisters_the_run() {
        {
            let _guard = register("sess-drop", "run-d");
            assert_eq!(cancel_session("sess-drop"), 1);
        }
        // 句柄 drop 后不再登记：不该再返回旧条目
        assert_eq!(cancel_session("sess-drop"), 0);
    }
}
