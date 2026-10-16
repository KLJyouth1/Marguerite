//! 审计录制：每步 UI 动作落 `.marguerite/audit-snapshots/<session>/<ts>.json`。
//!
//! 复用规范要求的审计快照目录约定（SPEC §6）：动作 + 前后屏快照路径占位 + 进程上下文。

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::desktop_automation::driver::UiAction;

pub struct Recorder;

impl Recorder {
    /// 快照目录：`~/.marguerite/audit-snapshots/<session>`
    pub fn snapshot_dir(session: &str) -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".marguerite")
            .join("audit-snapshots")
            .join(if session.is_empty() {
                "default"
            } else {
                session
            })
    }

    /// 记录单步动作快照，返回写入的文件路径（前后屏快照路径当前为占位）。
    pub fn record_step(
        session: &str,
        action: &UiAction,
        before: Option<&str>,
        after: Option<&str>,
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
            "before_snapshot": before,
            "after_snapshot": after,
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
