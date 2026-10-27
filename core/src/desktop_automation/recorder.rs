//! 审计录制：每步 UI 动作落 `.marguerite/audit-snapshots/<session>/<ts>.json`。
//!
//! 复用规范要求的审计快照目录约定（SPEC §6）：动作 + 动作前后目标摘要 + 进程上下文。
//!
//! # observe→act→verify（W6.3）
//!
//! `before` / `after` 是**目标节点的真实状态摘要**（`NodeState` 序列化），不是屏幕快照路径。
//! 这两个键此前恒为 `None`，审计里只有「动作发生了」，没有「发生了什么」——现在每次
//! 写动作都会落真实的前后状态，可直接用于事后回放与「回执对不对」的对账。
//!
//! # 谁落盘、谁不落盘（调用方负责）
//!
//! - **UIA 驱动的写动作**（click / invoke / set_value / type / toggle / select / expand /
//!   collapse / scroll / focus / window_activate）：调用 [`Recorder::record_step`]，带真实的
//!   `before` / `after`；
//! - **只读动作**（snapshot / find / wait）：**不调用**本模块。它们每次调用都会读一遍界面，
//!   落盘只会把审计目录刷满，而审计要回答的是「谁动了界面」。
//!
//! 判断标准不是「动作名字」，而是「这次调用是否可能改变桌面状态」。
//! ⚠️ WPS COM 驱动（`com.rs`）目前仍以 `None, None` 落盘 —— 那是 COM 调用序列的审计，
//! 不属 W6.3 的 observe→act→verify 范围；不要把它的 `null` 读成「观测失败」。
//!
//! # 测试隔离
//!
//! 目录默认取 `dirs::home_dir()`。单测必须调用 [`Recorder::set_root_override`] 把根指到
//! 临时目录，**绝不**往真实 `~/.marguerite/audit-snapshots` 写夹具（与 usage 台账同一条纪律）。
//! 覆盖是**线程局部**的，因为测试并行跑且每个用例在自己的线程里。

use std::cell::RefCell;
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::desktop_automation::driver::UiAction;

thread_local! {
    /// 线程局部的审计根目录覆盖（测试/隔离网关用）。
    static ROOT_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

pub struct Recorder;

impl Recorder {
    /// 覆盖审计根目录（`None` 恢复默认）。**线程局部**：只影响当前线程。
    ///
    /// 测试必须调用它；生产侧也可用于隔离网关，避免与常驻 daemon 争同一目录。
    pub fn set_root_override(root: Option<PathBuf>) {
        ROOT_OVERRIDE.with(|r| *r.borrow_mut() = root);
    }

    /// 当前审计根目录（默认 `<config_dir>/audit-snapshots`）。
    ///
    /// 走 `Config::config_dir()` 而不是 `dirs::home_dir()`：隔离网关（e2e / 探针）
    /// 必须能通过 `MARGUERITE_CONFIG_DIR` 把审计快照挡在临时目录里，否则一次
    /// `desktop.action` 就会往真实用户的 `~/.marguerite` 写文件（与 R64 usage
    /// 台账同一条纪律）。
    pub fn root_dir() -> PathBuf {
        if let Some(p) = ROOT_OVERRIDE.with(|r| r.borrow().clone()) {
            return p;
        }
        crate::persistence::config::Config::config_dir().join("audit-snapshots")
    }

    /// 快照目录：`<root>/<session>`
    pub fn snapshot_dir(session: &str) -> PathBuf {
        Self::root_dir().join(if session.is_empty() {
            "default"
        } else {
            session
        })
    }

    /// 记录单步动作快照，返回写入的文件路径。
    ///
    /// `before` / `after` 是目标节点的状态摘要；`snapshot_path` 的语义保持不变
    /// （屏幕快照文件路径），当前没有任何驱动产出它，因此始终为 `null`。
    pub fn record_step(
        session: &str,
        action: &UiAction,
        before: Option<&Value>,
        after: Option<&Value>,
        context: &str,
    ) -> PathBuf {
        let dir = Self::snapshot_dir(session);
        let _ = std::fs::create_dir_all(&dir);
        let ts = chrono::Utc::now().timestamp();
        let nanos = chrono::Utc::now().timestamp_subsec_nanos();
        let file = dir.join(format!("{ts}_{nanos}.json"));
        let payload = json!({
            "session": session,
            "ts": ts,
            "action": action.kind,
            "params": action.params,
            "before": before,
            "after": after,
            // 兼容旧读端：这两个键历史上是屏幕快照路径，当前仍无驱动产出。
            "before_snapshot": Value::Null,
            "after_snapshot": Value::Null,
            "context": context,
        });
        if let Ok(s) = serde_json::to_string_pretty(&payload) {
            let _ = std::fs::write(&file, s);
        }
        file
    }

    /// 列出某会话全部快照（按 ts 倒序），供 `desktop.snapshots` 返回。
    pub fn list_snapshots(session: &str) -> Vec<Value> {
        let dir = Self::snapshot_dir(session);
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) != Some("json") {
                    continue;
                }
                if let Ok(text) = std::fs::read_to_string(&p) {
                    if let Ok(v) = serde_json::from_str::<Value>(&text) {
                        out.push(v);
                    }
                }
            }
        }
        out.sort_by(|a, b| b["ts"].as_i64().cmp(&a["ts"].as_i64()));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn action() -> UiAction {
        UiAction {
            kind: "set_value".into(),
            params: json!({ "ref_id": "edit-1", "value": "hello" }),
        }
    }

    #[test]
    fn record_step_writes_real_before_and_after() {
        let root = std::env::temp_dir().join(format!(
            "mgt-recorder-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        Recorder::set_root_override(Some(root.clone()));

        let before = json!({ "node_digest": "aaa", "value": "old", "enabled": true, "focused": true });
        let after = json!({ "node_digest": "bbb", "value": "new", "enabled": true, "focused": true });
        let path = Recorder::record_step("sess-1", &action(), Some(&before), Some(&after), "uia_set_value");
        assert!(path.exists(), "审计文件必须真的落盘: {path:?}");

        let text = std::fs::read_to_string(&path).expect("读取审计文件");
        let v: Value = serde_json::from_str(&text).expect("审计文件必须是合法 JSON");
        assert_eq!(v["action"], "set_value");
        assert_eq!(v["before"], before, "审计必须含真实 before");
        assert_eq!(v["after"], after, "审计必须含真实 after");
        assert_eq!(v["context"], "uia_set_value");
        // 目录必须在覆盖根下，绝不能落到真实 home。
        assert!(
            path.starts_with(&root),
            "审计文件落到覆盖根之外: {path:?}"
        );

        // 清理 + 复原，避免影响同线程后续用例。
        let _ = std::fs::remove_dir_all(&root);
        Recorder::set_root_override(None);
    }

    #[test]
    fn snapshot_dir_follows_the_override_and_never_the_real_home() {
        let root = std::env::temp_dir().join(format!(
            "mgt-recorder-dir-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        Recorder::set_root_override(Some(root.clone()));
        assert_eq!(Recorder::snapshot_dir("abc"), root.join("abc"));
        assert_eq!(Recorder::snapshot_dir(""), root.join("default"));
        Recorder::set_root_override(None);
    }
}
