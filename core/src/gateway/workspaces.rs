//! 项目工作区注册表与 RPC（解锁 WebUI「新建任务 → 打开 / 创建项目文件夹」引导）
//!
//! - 持久化到 `Config::data_dir()/workspaces.json`（`id/name/path/pinned`）；
//! - `workspaces.open` 按路径幂等注册（同一路径复用同一 id）；
//! - `taskCount` 按 `sessions.db` 中 `project_path` 命中主根、或 `project_paths`
//!   数组包含该路径（P2-9 多根）统计；
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
    /// 最近一次被打开/选中（RFC3339）。旧 `workspaces.json` 没有该字段 →
    /// `serde(default)` 得到空串，读取不报错，排序时排最后。
    #[serde(default)]
    pub last_opened_at: String,
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
        // 单测隔离（与 `gateway::pet::prefs_path` 同一范式）：绝不写真实用户数据目录。
        #[cfg(test)]
        let path = match Config::current_config_dir_override() {
            Some(dir) => dir.join("workspaces.json"),
            None => std::env::temp_dir().join(format!(
                "marguerite_test_workspaces_{}.json",
                std::process::id()
            )),
        };
        #[cfg(not(test))]
        let path = Config::data_dir().join("workspaces.json");
        path
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

/// 会话数（sessions.db 中 project_path 命中主根，或 project_paths 含该路径）
fn task_count(project_path: &str) -> usize {
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    manager
        .init()
        .and_then(|_| manager.list_sessions_by_any_project(project_path, 1000))
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
        // 前端两种命名都在读：驼峰是主字段，snake_case 是兼容别名。
        "lastOpenedAt": entry.last_opened_at,
        "last_opened_at": entry.last_opened_at,
    })
}

/// `workspaces.list` 的排序：`pinned desc, last_opened_at desc, created_at desc`。
///
/// 空的 `last_opened_at`（旧版 `workspaces.json`）永远排在有值之后；RFC3339 同格式
/// 字符串可安全按字典序比较时间。该函数是纯函数，便于直接对排序规则做单测。
fn sort_for_list(entries: &mut [WorkspaceEntry]) {
    entries.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then_with(
                || match (a.last_opened_at.is_empty(), b.last_opened_at.is_empty()) {
                    (true, false) => std::cmp::Ordering::Greater,
                    (false, true) => std::cmp::Ordering::Less,
                    _ => b.last_opened_at.cmp(&a.last_opened_at),
                },
            )
            .then_with(|| b.created_at.cmp(&a.created_at))
    });
}

/// `workspaces.list` 最多返回的条数（`pinned` 优先、最近打开在前）。
const LIST_LIMIT: usize = 50;

/// 按 id 取工作区条目（编辑器文件树/读写共用）
pub fn get(id: &str) -> Option<WorkspaceEntry> {
    let guard = store().lock().unwrap_or_else(|e| e.into_inner());
    guard.get(id).cloned()
}

/// `workspaces.list`
pub fn list(_params: &Value) -> Value {
    let guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let mut ordered = guard.entries.clone();
    sort_for_list(&mut ordered);
    let items: Vec<Value> = ordered.iter().take(LIST_LIMIT).map(view).collect();
    json!({ "workspaces": items })
}

/// `workspaces.open`：把本地目录注册为项目工作区（幂等，每次打开都会刷新
/// `last_opened_at`，让「最近打开」列表反映真实选择顺序）。
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
    let now = chrono::Utc::now().to_rfc3339();
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    match guard.entries.iter().position(|w| w.id == id) {
        Some(index) => {
            // 幂等：同一路径复用同一 id，但「打开」这个动作本身要计入最近时间。
            guard.entries[index].last_opened_at = now;
            let entry = guard.entries[index].clone();
            guard.save();
            json!({ "workspace": view(&entry) })
        }
        None => {
            let entry = WorkspaceEntry {
                id,
                name: default_name(path),
                path: path.to_string(),
                pinned: false,
                created_at: now.clone(),
                last_opened_at: now,
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
    let now = chrono::Utc::now().to_rfc3339();
    let entry = WorkspaceEntry {
        id,
        name: name.to_string(),
        path: path_str,
        pinned: false,
        created_at: now.clone(),
        // 新建工作区即被选中，视同一次打开；否则它会因 last_opened_at 为空排到最后。
        last_opened_at: now,
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
        if let Ok(sessions) = manager.list_sessions_by_any_project(&project_path, 1000) {
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
        // 用临时目录注册 → list 可见 → remove 移除。
        // 注册表路径在 cfg(test) 下落到临时目录（见 `WorkspaceStore::path`），
        // 因此这里不会读写真实用户数据。
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

    /// 每个用例独立临时目录（配置目录覆盖是线程局部的，并行用例互不串扰）。
    fn temp_root(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "marguerite-workspaces-test-{tag}-{}-{seq}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn position_of(listed: &Value, id: &str) -> usize {
        listed["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .position(|w| w["id"] == id)
            .unwrap_or_else(|| panic!("工作区 {id} 不在 list 结果里"))
    }

    #[test]
    fn list_orders_by_recent_open_and_refreshes_on_reopen() {
        let base = temp_root("order");
        Config::set_config_dir_override(base.join("config"));
        let alpha = base.join("alpha");
        let beta = base.join("beta");
        std::fs::create_dir_all(&alpha).unwrap();
        std::fs::create_dir_all(&beta).unwrap();

        let a = open(&json!({ "path": alpha.to_string_lossy() }))["workspace"].clone();
        // RFC3339 的纳秒精度在 Windows 上足够，但显式隔开 2ms 让顺序断言与时钟无关。
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = open(&json!({ "path": beta.to_string_lossy() }))["workspace"].clone();

        // view() 同时下发驼峰与 snake_case 两个别名，值都是非空 RFC3339。
        assert!(!a["lastOpenedAt"].as_str().unwrap().is_empty());
        assert_eq!(a["lastOpenedAt"], a["last_opened_at"]);

        let a_id = a["id"].as_str().unwrap();
        let b_id = b["id"].as_str().unwrap();

        let listed = list(&Value::Null);
        assert!(
            position_of(&listed, b_id) < position_of(&listed, a_id),
            "后打开的 beta 必须排在 alpha 之前: {listed}"
        );

        // 幂等重开 alpha：不新增条目，且刷新 last_opened_at 后排到最前。
        std::thread::sleep(std::time::Duration::from_millis(2));
        let reopened = open(&json!({ "path": alpha.to_string_lossy() }))["workspace"].clone();
        assert_eq!(reopened["id"], a["id"]);
        assert_ne!(reopened["lastOpenedAt"], a["lastOpenedAt"]);

        let listed = list(&Value::Null);
        assert!(
            position_of(&listed, a_id) < position_of(&listed, b_id),
            "重开后的 alpha 必须排在 beta 之前: {listed}"
        );
        assert_eq!(
            listed["workspaces"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|w| w["id"] == a["id"])
                .count(),
            1,
            "幂等打开不得产生重复条目"
        );

        remove(&json!({ "workspaceId": a_id }));
        remove(&json!({ "workspaceId": b_id }));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn list_keeps_pinned_workspaces_first() {
        let base = temp_root("pinned");
        Config::set_config_dir_override(base.join("config"));
        let old = base.join("old");
        let fresh = base.join("fresh");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&fresh).unwrap();

        let o = open(&json!({ "path": old.to_string_lossy() }))["workspace"].clone();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let f = open(&json!({ "path": fresh.to_string_lossy() }))["workspace"].clone();
        let o_id = o["id"].as_str().unwrap();
        let f_id = f["id"].as_str().unwrap();

        // 未置顶时按最近打开排序：fresh 在前。
        let listed = list(&Value::Null);
        assert!(position_of(&listed, f_id) < position_of(&listed, o_id));

        // 置顶旧的之后，pinned 优先于 last_opened_at。
        assert_eq!(
            pin(&json!({ "workspaceId": o_id, "pinned": true }))["ok"],
            true
        );
        let listed = list(&Value::Null);
        assert!(
            position_of(&listed, o_id) < position_of(&listed, f_id),
            "pinned 工作区必须永远排在未置顶项之前: {listed}"
        );

        pin(&json!({ "workspaceId": o_id, "pinned": false }));
        remove(&json!({ "workspaceId": o_id }));
        remove(&json!({ "workspaceId": f_id }));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn legacy_entries_without_last_opened_at_still_load() {
        // 旧版 workspaces.json 结构（没有 last_opened_at）：serde(default) 必须给空串，
        // 而不是整份注册表反序列化失败。
        let raw = r#"[{
            "id": "ws-legacy",
            "name": "Legacy",
            "path": "C:/definitely-not-a-dir",
            "pinned": false,
            "created_at": "2026-01-01T00:00:00+00:00"
        }]"#;
        let entries: Vec<WorkspaceEntry> = serde_json::from_str(raw).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].last_opened_at, "");

        let base = temp_root("legacy");
        Config::set_config_dir_override(base.join("config"));
        let v = view(&entries[0]);
        assert_eq!(v["lastOpenedAt"], "");
        assert_eq!(v["last_opened_at"], "");
        let _ = std::fs::remove_dir_all(&base);
    }
}
