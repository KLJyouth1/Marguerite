//! 工作区文件监听器（Phase C 桌面能力 · 实时 FS 事件）
//!
//! 基于 `notify`（RecommendedWatcher + RecursiveMode::Recursive）监听工作区，
//! 检测到变更时通过 [`EventBus`] 发布 `Status(kind=workspace.changed)` 事件，
//! 路由键 `{session}.status` 命中 TUI 订阅 `ws.{session}.*`，驱动侧栏/状态栏刷新。
//!
//! 早期轮询式实现（std::fs 快照 + mtime 对比）已替换为实时事件，
//! 解决了「同秒内等长内容改写」可能漏检的问题。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use notify::{Event as NotifyEvent, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::json;
use tokio::sync::mpsc as tokio_mpsc;

use crate::events::{Envelope, EventBus, EventType};

/// 工作区监听器（实时）
pub struct WorkspaceWatcher {
    root: PathBuf,
    ignore_dirs: Vec<String>,
}

impl WorkspaceWatcher {
    /// 以工作区根目录创建监听器（默认忽略常见噪声目录）
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            ignore_dirs: vec![
                ".git".into(),
                "target".into(),
                "node_modules".into(),
                ".workbuddy".into(),
            ],
        }
    }

    /// 保留以兼容调用点（实时监听无需轮询间隔，此处仅为 API 稳定）
    #[allow(dead_code)]
    pub fn with_interval(self, _d: StdDuration) -> Self {
        self
    }

    /// 保留以兼容调用点
    #[allow(dead_code)]
    pub fn with_max_depth(self, _d: usize) -> Self {
        self
    }

    fn is_ignored(&self, path: &Path) -> bool {
        path.components().any(|c| {
            let s = c.as_os_str().to_string_lossy();
            self.ignore_dirs.iter().any(|ig| ig == s.as_ref())
        })
    }

    /// 运行监听器：注册 notify 监听，变更经总线发布
    ///
    /// 无限循环，由持有方（WorkbenchApp 的 tokio Runtime）释放时终止。
    pub async fn run(self, bus: Arc<EventBus>, session: String) {
        let (tx, mut rx) = tokio_mpsc::channel::<notify::Result<NotifyEvent>>(1024);
        let mut watcher: RecommendedWatcher = match notify::recommended_watcher(move |res| {
            let _ = tx.blocking_send(res);
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("创建文件监听器失败: {e}");
                return;
            }
        };
        if let Err(e) = watcher.watch(Path::new(&self.root), RecursiveMode::Recursive) {
            tracing::error!("监听目录失败 {}: {e}", self.root.display());
            return;
        }
        // watcher 持有至函数结束（保活）；循环读取事件并发布到总线
        while let Some(res) = rx.recv().await {
            let ev = match res {
                Ok(ev) => ev,
                Err(_) => continue,
            };
            let change = match ev.kind {
                EventKind::Create(_) => "created",
                EventKind::Modify(_) => "modified",
                EventKind::Remove(_) => "deleted",
                _ => continue,
            };
            for path in ev.paths {
                if self.is_ignored(&path) {
                    continue;
                }
                bus.publish(Envelope::new(
                    EventType::Status,
                    json!({
                        "kind": "workspace.changed",
                        "change": change,
                        "path": path.display().to_string(),
                    }),
                    session.as_str(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tokio::time::{sleep, timeout};
    use uuid::Uuid;

    fn tmp() -> PathBuf {
        let p = std::env::temp_dir().join(format!("mgt_watch_{}", Uuid::new_v4()));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[tokio::test]
    async fn test_notify_publishes_change() {
        let root = tmp();
        let bus = Arc::new(EventBus::default());
        let mut rx = bus.subscribe("**");
        let w = WorkspaceWatcher::new(root.clone());
        let task = tokio::spawn(w.run(bus.clone(), "ws.test".into()));

        // 等待 watcher 注册
        sleep(StdDuration::from_millis(150)).await;

        // 创建
        let f = root.join("a.txt");
        fs::write(&f, "hello").unwrap();

        // 在 2s 内收集至少一个 workspace.changed(created) 事件
        let got = timeout(StdDuration::from_secs(2), async {
            while let Some(env) = rx.recv().await {
                if env.payload["kind"] == "workspace.changed" && env.payload["change"] == "created"
                {
                    return true;
                }
            }
            false
        })
        .await;

        task.abort();
        let _ = fs::remove_dir_all(&root);
        assert!(
            got.unwrap_or(false),
            "应收到 workspace.changed(created) 事件"
        );
    }

    #[tokio::test]
    async fn test_notify_ignores_git_dir() {
        let root = tmp();
        let git = root.join(".git");
        fs::create_dir_all(&git).unwrap();
        let bus = Arc::new(EventBus::default());
        let mut rx = bus.subscribe("**");
        let w = WorkspaceWatcher::new(root.clone());
        let task = tokio::spawn(w.run(bus.clone(), "ws.test".into()));
        sleep(StdDuration::from_millis(150)).await;
        fs::write(git.join("config"), "core").unwrap();
        let saw = timeout(StdDuration::from_secs(1), async {
            while let Some(env) = rx.recv().await {
                if env.payload["kind"] == "workspace.changed" {
                    return true;
                }
            }
            false
        })
        .await;
        task.abort();
        let _ = fs::remove_dir_all(&root);
        // .git 下变更应被忽略 → 不应收到 workspace.changed
        assert!(!saw.unwrap_or(false), ".git 下变更应被忽略");
    }
}
