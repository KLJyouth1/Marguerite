//! 助理作用域：L1 全局（`global`）/ L2 项目（`project{workspace}`）。
//!
//! 落盘 `Config::config_dir()/assistant_scope.json`：
//! ```json
//! { "scope": "project", "workspace": "D:\\Downloads\\Marguerite", "since": "2026-10-30T02:11:00Z" }
//! ```
//! 读取是**宽容**的：文件缺失 / 损坏 / `project` 却没有 `workspace` → 一律回落 `global`，
//! 助理不会因为一个坏 JSON 就不可用。
//!
//! 对应 RPC（由主控接线）：
//! - `assistant.scope.get` → [`AssistantScopeState::as_rpc_value`]（`{scope, workspace, since}`）
//! - `assistant.scope.set` → [`save_scope`]（切 `project` 必须带 `workspace`，否则报错）

use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 当前助理作用域
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssistantScope {
    /// L1：跨项目，回答「这个人在别的项目里怎么做事」
    Global,
    /// L2：绑定某个 workspace
    Project { workspace: PathBuf },
}

impl AssistantScope {
    /// 线上名称（`"global"` / `"project"`）
    pub fn kind(&self) -> &'static str {
        match self {
            AssistantScope::Global => "global",
            AssistantScope::Project { .. } => "project",
        }
    }

    /// 项目作用域的 workspace；全局为 `None`
    pub fn workspace(&self) -> Option<&Path> {
        match self {
            AssistantScope::Global => None,
            AssistantScope::Project { workspace } => Some(workspace.as_path()),
        }
    }
}

/// `assistant_scope.json` 的落盘形态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantScopeState {
    /// `"global"` | `"project"`
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathBuf>,
    /// 进入当前作用域的 UTC 时间（RFC3339）
    pub since: String,
}

impl AssistantScopeState {
    /// 当前作用域（`project` 缺 workspace 时回落到 `global`）
    pub fn scope(&self) -> AssistantScope {
        match (self.scope.as_str(), self.workspace.clone()) {
            ("project", Some(ws)) if !ws.as_os_str().is_empty() => {
                AssistantScope::Project { workspace: ws }
            }
            _ => AssistantScope::Global,
        }
    }

    /// `assistant.scope.get` 的返回形状：`{ scope, workspace, since }`
    pub fn as_rpc_value(&self) -> Value {
        let scope = self.scope();
        json!({
            "scope": scope.kind(),
            "workspace": scope.workspace().map(|p| p.to_string_lossy().to_string()),
            "since": self.since,
        })
    }
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// `assistant_scope.json` 的路径
pub fn scope_path() -> PathBuf {
    crate::persistence::config::Config::config_dir().join("assistant_scope.json")
}

/// 读取当前作用域状态；任何异常都回落到 `global`（不报错）。
pub fn load_state() -> AssistantScopeState {
    let fallback = || AssistantScopeState {
        scope: "global".to_string(),
        workspace: None,
        since: now_rfc3339(),
    };
    let raw = match std::fs::read_to_string(scope_path()) {
        Ok(r) => r,
        Err(_) => return fallback(),
    };
    match serde_json::from_str::<AssistantScopeState>(&raw) {
        Ok(state) => AssistantScopeState {
            scope: state.scope().kind().to_string(),
            workspace: state.scope().workspace().map(Path::to_path_buf),
            since: state.since,
        },
        Err(_) => fallback(),
    }
}

/// 切换并持久化作用域（`assistant.scope.set`）。
///
/// 切到 `project` 但 `workspace` 为空 → 报错（计划 §4.1 的硬约束）。
pub fn save_scope(scope: &AssistantScope) -> Result<AssistantScopeState, String> {
    if let AssistantScope::Project { workspace } = scope {
        if workspace.as_os_str().is_empty() {
            return Err("切到 project 时必须带 workspace".to_string());
        }
    }
    let state = AssistantScopeState {
        scope: scope.kind().to_string(),
        workspace: scope.workspace().map(Path::to_path_buf),
        since: now_rfc3339(),
    };
    let body = serde_json::to_string_pretty(&state).map_err(|e| format!("序列化失败: {e}"))?;
    super::write_atomic_text(&scope_path(), &body)?;
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "marguerite_assist_scope_{}_{}",
            tag,
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        crate::persistence::config::Config::set_config_dir_override(dir.clone());
        dir
    }

    #[test]
    fn scope_switch_persists_across_reads() {
        let dir = isolated_dir("switch");
        assert_eq!(load_state().scope(), AssistantScope::Global);

        let ws = dir.join("proj");
        std::fs::create_dir_all(&ws).unwrap();
        let saved = save_scope(&AssistantScope::Project {
            workspace: ws.clone(),
        })
        .expect("切到 project 应成功");
        assert_eq!(saved.scope(), AssistantScope::Project { workspace: ws.clone() });
        assert!(scope_path().exists(), "assistant_scope.json 应落盘");
        assert!(scope_path().starts_with(&dir));

        // 新一次读取（模拟进程重启）拿到同样的作用域
        assert_eq!(
            load_state().scope(),
            AssistantScope::Project { workspace: ws }
        );
        assert_eq!(load_state().as_rpc_value()["scope"], "project");

        // 切回全局后持久化同样生效
        save_scope(&AssistantScope::Global).expect("切回 global 应成功");
        assert_eq!(load_state().scope(), AssistantScope::Global);
        assert_eq!(load_state().as_rpc_value()["workspace"], Value::Null);
    }

    #[test]
    fn project_scope_without_workspace_is_rejected() {
        let _dir = isolated_dir("no_ws");
        let err = save_scope(&AssistantScope::Project {
            workspace: PathBuf::new(),
        })
        .unwrap_err();
        assert!(err.contains("workspace"), "错误信息应点明缺 workspace: {err}");
        assert_eq!(load_state().scope(), AssistantScope::Global);
    }

    #[test]
    fn corrupt_state_file_falls_back_to_global() {
        let dir = isolated_dir("corrupt");
        assert!(dir.exists());
        std::fs::write(scope_path(), "{ not json").unwrap();
        let state = load_state();
        assert_eq!(state.scope(), AssistantScope::Global);
        assert!(!state.since.is_empty());
    }
}
