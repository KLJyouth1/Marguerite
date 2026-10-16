//! 项目工作区注册表与 RPC（解锁 WebUI「新建任务 → 打开 / 创建项目文件夹」引导）
//!
//! - 持久化到 `Config::data_dir()/workspaces.json`（`id/name/path/pinned`）；
//! - `workspaces.open` 按路径幂等注册（同一路径复用同一 id）；
//! - `taskCount` 按 `sessions.db` 中 `project_path` 匹配统计；
//! - `chat.send` 可通过 `workspaceId` 把会话绑定到具体项目目录（工具根目录随之切换）。

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::persistence::config::Config;
use crate::persistence::session::SessionManager;

/// 一条项目工作区
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub id: String,
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub created_at: String,
}

/// 进程级工作区台账（读盘加载，写盘持久化）
struct WorkspaceStore {
    entries: Vec<WorkspaceEntry>,
}

fn store() -> &'static Mutex<WorkspaceStore> {
    static STORE: OnceLock<Mutex<WorkspaceStore>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(WorkspaceStore::load()))
}

/// 基于路径生成稳定 id（FNV-1a hex），同一路径幂等
fn id_for_path(path: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in path.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("ws-{h:016x}")
}

fn default_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "项目".to_string())
}

impl WorkspaceStore {
    fn path() -> std::path::PathBuf {
        Config::data_dir().join("workspaces.json")
    }

    fn load() -> Self {
        let entries = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|raw| serde_json::from_str::<Vec<WorkspaceEntry>>(&raw).ok())
            .unwrap_or_default();
        Self { entries }
    }

    fn save(&self) {
        if let Ok(raw) = serde_json::to_string_pretty(&self.entries) {
            let p = Self::path();
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(p, raw);
        }
    }

    fn get(&self, id: &str) -> Option<&WorkspaceEntry> {
        self.entries.iter().find(|w| w.id == id)
    }

    fn upsert(&mut self, entry: WorkspaceEntry) {
        match self.entries.iter_mut().find(|w| w.id == entry.id) {
            Some(existing) => *existing = entry,
            None => self.entries.push(entry),
        }
        self.save();
    }

    fn remove(&mut self, id: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|w| w.id != id);
        let removed = self.entries.len() < before;
        if removed {
            self.save();
        }
        removed
    }
}

/// 会话数（sessions.db 中 project_path 匹配）
fn task_count(project_path: &str) -> usize {
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    manager
        .init()
        .and_then(|_| manager.list_sessions_by_project(project_path, 1000))
        .map(|v| v.len())
        .unwrap_or(0)
}

/// 工作区 → webui 契约视图
fn view(entry: &WorkspaceEntry) -> Value {
    let available = Path::new(&entry.path).is_dir();
    json!({
        "id": entry.id,
        "name": entry.name,
        "path": entry.path,
        "taskCount": task_count(&entry.path),
        "pinned": entry.pinned,
        "available": available,
        "availabilityReason": if available { Value::Null } else { json!("目录不存在") },
    })
}

/// 按 id 取工作区条目（编辑器文件树/读写共用）
pub fn get(id: &str) -> Option<WorkspaceEntry> {
    let guard = store().lock().unwrap_or_else(|e| e.into_inner());
    guard.get(id).cloned()
}

/// `workspaces.list`
pub fn list(_params: &Value) -> Value {
    let guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let items: Vec<Value> = guard.entries.iter().map(view).collect();
    json!({ "workspaces": items })
}

/// `workspaces.open`：把本地目录注册为项目工作区（幂等）。
///
/// params: `{ path: string, trusted?: bool }` → `{ workspace: {...} }`
pub fn open(params: &Value) -> Value {
    let path = params
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if path.is_empty() {
        return json!({ "error": "path 不可为空" });
    }
    if !Path::new(path).is_dir() {
        return json!({ "error": format!("目录不存在：{path}") });
    }
    let id = id_for_path(path);
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    match guard.get(&id) {
        Some(existing) => json!({ "workspace": view(existing) }),
        None => {
            let entry = WorkspaceEntry {
                id,
                name: default_name(path),
                path: path.to_string(),
                pinned: false,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            guard.upsert(entry.clone());
            json!({ "workspace": view(&entry) })
        }
    }
}

/// `workspaces.create`：新建项目工作区文件夹并注册（对齐 Tauri create_workspace_folder）。
///
/// params: `{ name: string }` → `{ workspace: {...} }`。目录落在 data_dir/workspaces/<name>。
pub fn create(params: &Value) -> Value {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if name.is_empty() {
        return json!({ "error": "name 不可为空" });
    }
    if name.contains("..") || name.contains('/') || name.contains(std::path::MAIN_SEPARATOR) {
        return json!({ "error": "目录名不可包含路径分隔符或 .." });
    }
    let base = Config::data_dir().join("workspaces");
    let path = base.join(name);
    if let Err(e) = std::fs::create_dir_all(&path) {
        return json!({ "error": format!("创建工作区目录失败({}): {}", path.display(), e) });
    }
    let path_str = path.to_string_lossy().to_string();
    let id = id_for_path(&path_str);
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let entry = WorkspaceEntry {
        id,
        name: name.to_string(),
        path: path_str,
        pinned: false,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    guard.upsert(entry.clone());
    json!({ "workspace": view(&entry) })
}

/// `workspaces.update`：重命名工作区。params: `{ workspaceId, name }`
pub fn update(params: &Value) -> Value {
    let id = params
        .get("workspaceId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if id.is_empty() || name.is_empty() {
        return json!({ "error": "workspaceId 与 name 不可为空" });
    }
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    match guard.entries.iter_mut().find(|w| w.id == id) {
        Some(entry) => {
            entry.name = name.to_string();
            let view = view(entry);
            guard.save();
            json!({ "ok": true, "workspace": view })
        }
        None => json!({ "error": "工作区不存在" }),
    }
}

/// `workspaces.pin`：置顶工作区。params: `{ workspaceId, pinned }`
pub fn pin(params: &Value) -> Value {
    let id = params
        .get("workspaceId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let pinned = params
        .get("pinned")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    match guard.entries.iter_mut().find(|w| w.id == id) {
        Some(entry) => {
            entry.pinned = pinned;
            guard.save();
            json!({ "ok": true })
        }
        None => json!({ "error": "工作区不存在" }),
    }
}

/// `workspaces.remove`：移除工作区（不删除目录）。params: `{ workspaceId }`
pub fn remove(params: &Value) -> Value {
    let id = params
        .get("workspaceId")
        .and_then(Value::as_str)
        .unwrap_or("");
    if id.is_empty() {
        return json!({ "error": "workspaceId 不可为空" });
    }
    let removed = store().lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    json!({ "ok": removed })
}

/// `workspaces.history.delete`：删除工作区下的会话历史。
///
/// params: `{ workspaceId }` → `{ workspaceId, deletedTaskCount, deletedSessionKeys }`
pub fn history_delete(params: &Value) -> Value {
    let id = params
        .get("workspaceId")
        .and_then(Value::as_str)
        .unwrap_or("");
    if id.is_empty() {
        return json!({ "error": "workspaceId 不可为空" });
    }
    let path = store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .map(|w| w.path.clone());
    let Some(project_path) = path else {
        return json!({ "workspaceId": id, "deletedTaskCount": 0, "deletedSessionKeys": [] });
    };
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    let mut deleted = Vec::new();
    if let Ok(()) = manager.init() {
        if let Ok(sessions) = manager.list_sessions_by_project(&project_path, 1000) {
            for s in &sessions {
                if manager.delete_session(&s.session_id).is_ok() {
                    deleted.push(s.session_id.clone());
                }
            }
        }
    }
    json!({
        "workspaceId": id,
        "deletedTaskCount": deleted.len(),
        "deletedSessionKeys": deleted,
    })
}

/// 由 workspaceId 解析项目根目录（供 chat.send 绑定会话工作区）；无效返回 None
pub fn workspace_path_for(workspace_id: &str) -> Option<String> {
    let guard = store().lock().unwrap_or_else(|e| e.into_inner());
    guard.get(workspace_id).map(|w| w.path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id_stable() {
        assert_eq!(id_for_path(r"D:\proj"), id_for_path(r"D:\proj"));
        assert_ne!(id_for_path(r"D:\proj"), id_for_path(r"D:\proj2"));
    }

    #[test]
    fn test_default_name_from_path() {
        assert_eq!(default_name(r"D:\work\my-app"), "my-app");
        assert!(!default_name(r"D:\").is_empty());
    }

    #[test]
    fn test_open_rejects_missing_dir() {
        let v = open(&json!({ "path": "Z:\\definitely-missing-xyz" }));
        assert!(v["error"].as_str().is_some());
    }

    #[test]
    fn test_open_empty_path() {
        let v = open(&json!({}));
        assert!(v["error"].as_str().is_some());
    }

    #[test]
    fn test_store_roundtrip_via_api() {
        // 用临时目录注册 → list 可见 → remove 移除（写入 data_dir 的真实注册表，
        // 仅验证 handler 逻辑，不依赖具体目录存在性之外的断言）
        let tmp = std::env::temp_dir();
        let p = tmp.to_string_lossy().to_string();
        let opened = open(&json!({ "path": p }));
        let ws = &opened["workspace"];
        let id = ws["id"].as_str().unwrap();
        assert_eq!(ws["path"], p);
        let listed = list(&Value::Null);
        assert!(listed["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["id"] == id));
        let renamed = update(&json!({ "workspaceId": id, "name": "临时项目" }));
        assert_eq!(renamed["workspace"]["name"], "临时项目");
        let pinned = pin(&json!({ "workspaceId": id, "pinned": true }));
        assert_eq!(pinned["ok"], true);
        let removed = remove(&json!({ "workspaceId": id }));
        assert_eq!(removed["ok"], true);
        let listed2 = list(&Value::Null);
        assert!(!listed2["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["id"] == id));
    }
}
