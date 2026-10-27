//! 长任务断点续跑（P2-11，一期）。
//!
//! 与 Python 侧 `pyengine/src/opensquilla/engine/task_checkpoints.py` 字段、路径、
//! 语义对齐：
//! - 每个长任务一个 JSON，落 `Config::data_dir()/checkpoints`
//!   （`MARGUERITE_DATA_DIR` 可整体隔离）；
//! - 原子写（tmp + rename），写到一半崩溃不会留下半份 checkpoint；
//! - 重启后列出 checkpoint 仍为 `running` / `queued_for_replay` 的「被中断」任务；
//! - 操作员可 list / abandon / mark_for_replay / finalize。
//!
//! # 一期不做（二期，架构级，未假装实现）
//! - 步骤级精确续跑：一期 replay 是「整任务重发」，不是从断点接着跑；
//! - 已落副作用的幂等账本（重跑前的去重/补偿）；
//! - 跨进程文件锁。
//!
//! 一期 replay 语义 = `reset_and_reissue`：把状态重置为可重发，由调用方重新
//! 发起整任务。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 重启后仍视为「在飞」的状态。
pub const INTERRUPTED_STATUSES: &[&str] = &["running", "queued_for_replay"];

/// 单个长任务的 checkpoint（JSON 字段与 Python 侧逐字对齐）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskCheckpoint {
    pub task_id: String,
    pub session_id: String,
    pub title: String,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub completed_steps: Vec<Value>,
    #[serde(default)]
    pub side_effects: Vec<Value>,
    #[serde(default)]
    pub total_steps: u64,
    #[serde(default)]
    pub updated_at: u64,
}

fn default_status() -> String {
    "running".to_string()
}

impl TaskCheckpoint {
    /// 新建一个 `running` checkpoint。
    pub fn new(
        task_id: impl Into<String>,
        session_id: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            session_id: session_id.into(),
            title: title.into(),
            status: "running".to_string(),
            completed_steps: Vec::new(),
            side_effects: Vec::new(),
            total_steps: 0,
            updated_at: 0,
        }
    }
}

/// 文件支撑的 checkpoint 存储（每任务一 JSON，原子写）。
pub struct CheckpointStore {
    root: PathBuf,
}

impl CheckpointStore {
    /// 打开（必要时创建）checkpoint 根目录。
    pub fn open(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// 根目录（测试/对账用）。
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path_for(&self, task_id: &str) -> Option<PathBuf> {
        let safe = safe_name(task_id).ok()?;
        Some(self.root.join(format!("{safe}.json")))
    }

    /// Upsert 一个 checkpoint（原子写，自动刷新 updated_at）。
    pub fn record(&self, mut checkpoint: TaskCheckpoint) -> std::io::Result<TaskCheckpoint> {
        checkpoint.updated_at = now_ms();
        let path = self
            .path_for(&checkpoint.task_id)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "unsafe task_id"))?;
        let tmp = path.with_extension("json.tmp");
        let payload = serde_json::to_vec_pretty(&checkpoint)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&tmp, payload)?;
        std::fs::rename(&tmp, &path)?;
        Ok(checkpoint)
    }

    /// 读取单个 checkpoint；缺失/损坏一律返回 None（损坏不得卡住恢复）。
    pub fn get(&self, task_id: &str) -> Option<TaskCheckpoint> {
        let path = self.path_for(task_id)?;
        let bytes = std::fs::read(path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// 列出全部 checkpoint（按文件名排序，跳过损坏文件）。
    pub fn list_all(&self) -> Vec<TaskCheckpoint> {
        let mut paths: Vec<PathBuf> = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
                .collect(),
            Err(_) => return Vec::new(),
        };
        paths.sort();
        let mut out = Vec::new();
        for p in paths {
            if let Ok(bytes) = std::fs::read(&p) {
                if let Ok(cp) = serde_json::from_slice::<TaskCheckpoint>(&bytes) {
                    out.push(cp);
                }
            }
        }
        out
    }

    /// 进程重启时仍在飞的 checkpoint。
    pub fn list_interrupted(&self) -> Vec<TaskCheckpoint> {
        self.list_all()
            .into_iter()
            .filter(|c| INTERRUPTED_STATUSES.contains(&c.status.as_str()))
            .collect()
    }

    /// 追加一步已完成记录（用于工具回路每步落 checkpoint）。
    pub fn append_step(&self, task_id: &str, step: Value) -> Option<TaskCheckpoint> {
        let mut existing = self.get(task_id)?;
        existing.completed_steps.push(step);
        existing.total_steps = existing.completed_steps.len() as u64;
        self.record(existing).ok()
    }

    fn change_status(&self, task_id: &str, status: &str) -> Option<TaskCheckpoint> {
        let existing = self.get(task_id)?;
        let mut next = existing;
        next.status = status.to_string();
        self.record(next).ok()
    }

    /// 放弃（标记 abandoned）。
    pub fn abandon(&self, task_id: &str) -> Option<TaskCheckpoint> {
        self.change_status(task_id, "abandoned")
    }

    /// 标记为重放（一期：重置后整任务重发）。
    pub fn mark_for_replay(&self, task_id: &str) -> Option<TaskCheckpoint> {
        self.change_status(task_id, "queued_for_replay")
    }

    /// 完成（默认 succeeded，也可传 failed / cancelled 等）。
    pub fn finalize(&self, task_id: &str, status: &str) -> Option<TaskCheckpoint> {
        self.change_status(task_id, status)
    }
}

/// 默认 checkpoint 根目录：`<data_dir>/checkpoints`。
pub fn default_checkpoint_root() -> PathBuf {
    crate::persistence::config::Config::data_dir().join("checkpoints")
}

/// 打开默认存储。
pub fn default_store() -> std::io::Result<CheckpointStore> {
    CheckpointStore::open(default_checkpoint_root())
}

// ---------------------------------------------------------------------------
// 纯函数（离线可测）
// ---------------------------------------------------------------------------

/// 当前 epoch 毫秒（UTC）。
pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 把 task_id 清洗成安全文件名（纯函数，与 Python `_safe_name` 对齐）。
///
/// 先**拒绝**可穿越目录的 id（在清洗之前——把 "../escape" 洗成 ".._escape" 会
/// 掩盖坏 id），再把非 `[A-Za-z0-9_.-]` 替换为 `_`。
pub fn safe_name(task_id: &str) -> Result<String, String> {
    let raw = task_id;
    if raw.is_empty()
        || raw == "."
        || raw == ".."
        || raw.contains('/')
        || raw.contains('\\')
        || raw.contains('\0')
        || raw.starts_with('.')
    {
        return Err(format!("unsafe task_id: {raw:?}"));
    }
    let mut cleaned = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
            cleaned.push(ch);
        } else {
            cleaned.push('_');
        }
    }
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return Err(format!("unsafe task_id: {raw:?}"));
    }
    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    /// 并行测试各自唯一目录：`now_ms()` 同毫秒会撞目录，故加进程内自增计数。
    static DIR_SEQ: AtomicU64 = AtomicU64::new(0);
    fn unique_dir() -> PathBuf {
        let n = DIR_SEQ.fetch_add(1, AtomicOrdering::SeqCst);
        std::env::temp_dir().join(format!("marg-cp-{}-{n}", std::process::id()))
    }

    #[test]
    fn safe_name_sanitizes_and_rejects_traversal() {
        assert_eq!(safe_name("task-1_2").as_deref(), Ok("task-1_2"));
        // 非允许字符 → _
        assert_eq!(safe_name("task:1").as_deref(), Ok("task_1"));
        // 穿越 / 隐藏路径直接拒绝
        assert!(safe_name("../escape").is_err());
        assert!(safe_name("..\\escape").is_err());
        assert!(safe_name("a/b").is_err());
        assert!(safe_name(".hidden").is_err());
        assert!(safe_name("").is_err());
        assert!(safe_name("..").is_err());
        assert!(safe_name("a\0b").is_err());
    }

    #[test]
    fn record_get_roundtrip_atomic() {
        let dir = unique_dir();
        let store = CheckpointStore::open(&dir).unwrap();
        let cp = TaskCheckpoint::new("t1", "sess-1", "A title");
        let saved = store.record(cp).unwrap();
        assert!(saved.updated_at > 0);
        let loaded = store.get("t1").expect("应能读回");
        assert_eq!(loaded.task_id, "t1");
        assert_eq!(loaded.status, "running");
        // 无残留 tmp
        assert!(std::fs::read_dir(&dir)
            .unwrap()
            .any(|e| !e.unwrap().path().to_string_lossy().ends_with(".tmp")));
        let tmp_exists = std::fs::read_dir(&dir)
            .unwrap()
            .any(|e| e.unwrap().path().to_string_lossy().ends_with(".tmp"));
        assert!(!tmp_exists);
    }

    #[test]
    fn append_steps_and_interrupted_list() {
        let dir = unique_dir();
        let store = CheckpointStore::open(&dir).unwrap();
        store.record(TaskCheckpoint::new("a", "s", "A")).unwrap();
        store.record(TaskCheckpoint::new("b", "s", "B")).unwrap();
        store.append_step("a", json!({"step": 1}));
        store.append_step("a", json!({"step": 2}));
        // a 已完成两步
        let a = store.get("a").unwrap();
        assert_eq!(a.completed_steps.len(), 2);
        assert_eq!(a.total_steps, 2);
        // 两个都在飞
        let interrupted = store.list_interrupted();
        assert_eq!(interrupted.len(), 2);
        // 完成 b 后只剩 a 在飞
        store.finalize("b", "succeeded");
        assert_eq!(store.list_interrupted().len(), 1);
        assert_eq!(store.get("b").unwrap().status, "succeeded");
    }

    #[test]
    fn abandon_and_replay_status_transitions() {
        let dir = unique_dir();
        let store = CheckpointStore::open(&dir).unwrap();
        store.record(TaskCheckpoint::new("x", "s", "X")).unwrap();
        let abandoned = store.abandon("x").unwrap();
        assert_eq!(abandoned.status, "abandoned");
        // abandoned 不算在飞
        assert!(store.list_interrupted().is_empty());
        // 新任务走 replay
        store.record(TaskCheckpoint::new("y", "s", "Y")).unwrap();
        let replay = store.mark_for_replay("y").unwrap();
        assert_eq!(replay.status, "queued_for_replay");
        // queued_for_replay 仍算在飞
        assert_eq!(store.list_interrupted().len(), 1);
        // 缺失任务返回 None
        assert!(store.abandon("nope").is_none());
    }

    #[test]
    fn corrupt_checkpoint_treated_as_absent() {
        let dir = unique_dir();
        let store = CheckpointStore::open(&dir).unwrap();
        std::fs::write(dir.join("broken.json"), b"{not json").unwrap();
        assert!(store.get("broken").is_none());
        // list_all 跳过损坏
        assert!(store.list_all().is_empty());
    }
}
