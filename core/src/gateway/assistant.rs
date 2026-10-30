//! 两层助理 + 会话分支/恢复/回放的网关接线层。
//!
//! 这里**只做**「入参解析 → 调既有库函数 → 兑成线上形状」：
//! - 助理作用域 / 简报 / 钉住与偏好：`crate::assistant::{scope, brief, profile}`
//! - 会话分支 / 恢复 / 回放：`crate::session::fork`
//!
//! 失败一律给机器可判定的 `code`（前端据此分支），绝不返回空结果冒充成功。

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::assistant::brief::{self, PinnedItem};
use crate::assistant::profile::UserProfile;
use crate::assistant::scope::{self, AssistantScope};
use crate::gateway::methods::GatewayContext;
use crate::persistence::config::Config;
use crate::persistence::session::SessionManager;
use crate::session::fork::{self, SessionError};

/// `assistant.scope.get`：`{scope, workspace, since}`（缺失/损坏状态文件回落 `global`）。
pub fn scope_get(_ctx: &GatewayContext, _params: &Value) -> Value {
    scope::load_state().as_rpc_value()
}

/// `assistant.scope.set`：入参 `{scope, workspace?}`，成功返回与 `scope.get` 同形状。
///
/// `project` 缺 workspace 是硬约束拒绝（`ASSISTANT_SCOPE_NO_WORKSPACE`），
/// 不静默降级成 `global` —— 那会让用户在「项目模式」里看到全局答案却毫无提示。
pub fn scope_set(_ctx: &GatewayContext, params: &Value) -> Value {
    let kind = params
        .get("scope")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let workspace = params
        .get("workspace")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty());

    let target = match kind {
        "global" => AssistantScope::Global,
        "project" => match workspace {
            Some(path) => AssistantScope::Project {
                workspace: PathBuf::from(path),
            },
            None => {
                return json!({
                    "success": false,
                    "code": "ASSISTANT_SCOPE_NO_WORKSPACE",
                    "message": "切到 project 作用域必须带 workspace（例如 D:\\proj\\demo）",
                })
            }
        },
        other => {
            return json!({
                "success": false,
                "code": "ASSISTANT_SCOPE_INVALID",
                "message": format!("scope='{other}' 非法；合法值 global|project"),
            })
        }
    };

    match scope::save_scope(&target) {
        Ok(state) => state.as_rpc_value(),
        Err(message) => json!({
            "success": false,
            "code": "ASSISTANT_SCOPE_SAVE_FAILED",
            "message": format!("保存作用域失败: {message}"),
        }),
    }
}

/// `assistant.brief.get`：入参 `{query?, max_chars?}`，按**当前**作用域合成结构化简报。
pub fn brief_get(_ctx: &GatewayContext, params: &Value) -> Value {
    let query = params.get("query").and_then(Value::as_str).unwrap_or("");
    let max_chars = params
        .get("max_chars")
        .and_then(Value::as_u64)
        .map(|chars| chars as usize);
    let state = scope::load_state();
    brief::build_brief(&state.scope(), query, max_chars)
}

/// `assistant.prefs.list`：`{pinned:[{text,source,at}], preferences:[{key,value,confidence,evidence}]}`
pub fn prefs_list(_ctx: &GatewayContext, _params: &Value) -> Value {
    json!({
        "pinned": brief::load_pins(),
        "preferences": UserProfile::load().to_preferences(),
    })
}

/// `assistant.prefs.pin`：钉住一条用户偏好；空文本拒绝（`ASSISTANT_PIN_EMPTY`）。
///
/// 返回的 `pinned` 是**落盘后的真实列表**（`save_pins` 按 text 去重），不是本次入参的臆测。
pub fn prefs_pin(_ctx: &GatewayContext, params: &Value) -> Value {
    let text = params
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if text.is_empty() {
        return json!({
            "success": false,
            "code": "ASSISTANT_PIN_EMPTY",
            "message": "pin 的 text 不能为空",
        });
    }
    let mut items: Vec<PinnedItem> = brief::load_pins();
    items.push(PinnedItem {
        text: text.to_string(),
        source: "user".to_string(),
        at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    });
    if let Err(message) = brief::save_pins(&items) {
        return json!({
            "success": false,
            "code": "ASSISTANT_PIN_SAVE_FAILED",
            "message": format!("保存钉住条目失败: {message}"),
        });
    }
    json!({ "success": true, "pinned": brief::load_pins() })
}

/// `assistant.prefs.unpin`：按 text 精确移除一条钉住条目，返回落盘后的列表。
pub fn prefs_unpin(_ctx: &GatewayContext, params: &Value) -> Value {
    let text = params
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let remaining: Vec<PinnedItem> = brief::load_pins()
        .into_iter()
        .filter(|item| item.text != text)
        .collect();
    if let Err(message) = brief::save_pins(&remaining) {
        return json!({
            "success": false,
            "code": "ASSISTANT_PIN_SAVE_FAILED",
            "message": format!("保存钉住条目失败: {message}"),
        });
    }
    json!({ "success": true, "pinned": brief::load_pins() })
}

/// `sessions.fork`：入参 `{key, forkPoint?, title?}`，父会话只读、子会话返回新 key。
pub fn sessions_fork(_ctx: &GatewayContext, params: &Value) -> Value {
    let mut manager = match open_manager() {
        Ok(manager) => manager,
        Err(err) => return err,
    };
    let key = session_key(params);
    let point = match fork_point(params) {
        Ok(point) => point,
        Err(err) => return err,
    };
    let title = params
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty());
    match fork::fork_session(&mut manager, &key, point, title) {
        Ok(result) => serde_json::to_value(&result).unwrap_or_else(|err| {
            json!({
                "success": false,
                "code": "INTERNAL_ERROR",
                "message": format!("序列化分支结果失败: {err}"),
            })
        }),
        Err(err) => session_error_value(&err),
    }
}

/// `sessions.resume`：入参 `{key, checkpoint?}`，恢复可继续状态（含消息帧与压缩交接）。
pub fn sessions_resume(_ctx: &GatewayContext, params: &Value) -> Value {
    let manager = match open_manager() {
        Ok(manager) => manager,
        Err(err) => return err,
    };
    let key = session_key(params);
    let checkpoint = params
        .get("checkpoint")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty());
    match fork::resume_session(&manager, &key, checkpoint) {
        Ok(result) => serde_json::to_value(&result).unwrap_or_else(|err| {
            json!({
                "success": false,
                "code": "INTERNAL_ERROR",
                "message": format!("序列化恢复结果失败: {err}"),
            })
        }),
        Err(err) => session_error_value(&err),
    }
}

/// `sessions.replay`：入参 `{key}`，只读、按落库行序返回消息帧。
pub fn sessions_replay(_ctx: &GatewayContext, params: &Value) -> Value {
    let manager = match open_manager() {
        Ok(manager) => manager,
        Err(err) => return err,
    };
    let key = session_key(params);
    match fork::replay_session(&manager, &key) {
        Ok(result) => serde_json::to_value(&result).unwrap_or_else(|err| {
            json!({
                "success": false,
                "code": "INTERNAL_ERROR",
                "message": format!("序列化回放结果失败: {err}"),
            })
        }),
        Err(err) => session_error_value(&err),
    }
}

/// 打开 `sessions.db`（与 `sessions.list` 同一路径口径）；失败即 `SESSION_STORE_ERROR`。
fn open_manager() -> Result<SessionManager, Value> {
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    manager.init().map_err(|err| {
        json!({
            "success": false,
            "code": "SESSION_STORE_ERROR",
            "message": format!("会话存储初始化失败: {err}"),
        })
    })?;
    Ok(manager)
}

fn session_key(params: &Value) -> String {
    params
        .get("key")
        .or_else(|| params.get("sessionKey"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

/// `forkPoint`：缺省 = 全量分支；出现但不是正整数 → `FORK_POINT_INVALID`
/// （不能静默当成「没传」而全量分支，那会把用户的意图解码成另一件事）。
fn fork_point(params: &Value) -> Result<Option<usize>, Value> {
    match params.get("forkPoint") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(|n| Some(n as usize)).ok_or_else(|| {
            json!({
                "success": false,
                "code": "FORK_POINT_INVALID",
                "message": "forkPoint 必须是正整数（= 要复制进子会话的消息条数）",
            })
        }),
    }
}

fn session_error_value(err: &SessionError) -> Value {
    json!({
        "success": false,
        "code": err.code(),
        "message": err.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个用例独占配置目录：`sessions.db` / `assistant_scope.json` / `assistant_pins.json`
    /// 都落在注入的临时目录里，绝不读写真实用户的 `~/.marguerite`。
    fn isolated_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "marguerite-gateway-assistant-{tag}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("创建临时配置目录");
        Config::set_config_dir_override(dir.clone());
        dir
    }

    fn test_ctx() -> GatewayContext {
        GatewayContext {
            config: Config::default(),
            token: "t".into(),
            started_at: std::time::Instant::now(),
            bus: std::sync::Arc::new(crate::events::EventBus::default()),
        }
    }

    #[test]
    fn scope_get_defaults_to_global() {
        let dir = isolated_dir("scope-default");
        let value = scope_get(&test_ctx(), &json!({}));
        assert_eq!(value["scope"], "global", "没有状态文件时必须回落 global");
        assert_eq!(value["workspace"], Value::Null, "global 不带 workspace");
        assert!(
            !value["since"].as_str().unwrap_or("").is_empty(),
            "since 必须给出进入当前作用域的时间"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn scope_set_rejects_project_without_workspace() {
        let dir = isolated_dir("scope-no-ws");
        let value = scope_set(&test_ctx(), &json!({ "scope": "project" }));
        assert_eq!(value["success"], false);
        assert_eq!(value["code"], "ASSISTANT_SCOPE_NO_WORKSPACE");
        assert!(
            value["message"].as_str().unwrap_or("").contains("workspace"),
            "错误信息应点明缺 workspace: {value:?}"
        );
        assert_eq!(
            scope_get(&test_ctx(), &json!({}))["scope"],
            "global",
            "被拒的切换不得留下任何状态"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn scope_set_project_persists_and_get_reflects_it() {
        let dir = isolated_dir("scope-roundtrip");
        let ws = dir.join("proj");
        std::fs::create_dir_all(&ws).expect("创建项目目录");
        let ws_text = ws.to_string_lossy().to_string();

        let set = scope_set(&test_ctx(), &json!({ "scope": "project", "workspace": ws_text }));
        assert_eq!(set["scope"], "project");
        assert_eq!(set["workspace"], json!(ws_text), "成功回包应与 scope.get 同形状");
        assert!(!set["since"].as_str().unwrap_or("").is_empty());

        let got = scope_get(&test_ctx(), &json!({}));
        assert_eq!(got["scope"], "project");
        assert_eq!(got["workspace"], json!(ws_text));

        // 切回 global：workspace 必须清空（不能留上一轮的项目路径）
        let back = scope_set(&test_ctx(), &json!({ "scope": "global" }));
        assert_eq!(back["scope"], "global");
        assert_eq!(back["workspace"], Value::Null);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn brief_get_returns_structural_brief_and_honours_max_chars() {
        let dir = isolated_dir("brief");
        let value = brief_get(&test_ctx(), &json!({ "query": "", "max_chars": 42 }));
        assert_eq!(value["scope"], "global");
        assert_eq!(value["budget"]["max_chars"], 42, "max_chars 必须生效");
        assert_eq!(value["budget"]["used"], 0, "无任何状态时用量为零");
        assert_eq!(value["truncated"], false);
        for field in ["pinned", "preferences", "facts", "open_loops"] {
            assert!(
                value[field].is_array(),
                "简报必须给出结构化分区 {field}: {value:?}"
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn prefs_list_is_empty_on_fresh_state() {
        let dir = isolated_dir("prefs-empty");
        let value = prefs_list(&test_ctx(), &json!({}));
        assert_eq!(value["pinned"].as_array().map(Vec::len), Some(0));
        assert_eq!(value["preferences"].as_array().map(Vec::len), Some(0));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pin_then_list_shows_the_pinned_item() {
        let dir = isolated_dir("pin-list");
        let ctx = test_ctx();
        let pinned = prefs_pin(&ctx, &json!({ "text": "所有文档用中文" }));
        assert_eq!(pinned["success"], true);
        let items = pinned["pinned"].as_array().cloned().unwrap_or_default();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["text"], "所有文档用中文");
        assert_eq!(items[0]["source"], "user", "钉住来源必须是 user");
        assert!(
            !items[0]["at"].as_str().unwrap_or("").is_empty(),
            "钉住时间必须落成 RFC3339: {items:?}"
        );

        let listed = prefs_list(&ctx, &json!({}));
        let listed_items = listed["pinned"].as_array().cloned().unwrap_or_default();
        assert_eq!(listed_items.len(), 1, "pin 之后 list 必须能看到：{listed:?}");
        assert_eq!(listed_items[0]["text"], "所有文档用中文");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pin_empty_text_is_rejected() {
        let dir = isolated_dir("pin-empty");
        let ctx = test_ctx();
        for text in [json!(""), json!("   "), json!(null)] {
            let value = prefs_pin(&ctx, &json!({ "text": text }));
            assert_eq!(value["success"], false, "空文本必须被拒：{value:?}");
            assert_eq!(value["code"], "ASSISTANT_PIN_EMPTY");
        }
        assert_eq!(
            prefs_list(&ctx, &json!({}))["pinned"].as_array().map(Vec::len),
            Some(0),
            "被拒的 pin 不得落盘"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pin_is_deduplicated_by_text() {
        let dir = isolated_dir("pin-dedup");
        let ctx = test_ctx();
        prefs_pin(&ctx, &json!({ "text": "不要 git commit" }));
        let again = prefs_pin(&ctx, &json!({ "text": "不要 git commit" }));
        assert_eq!(again["success"], true);
        assert_eq!(
            again["pinned"].as_array().map(Vec::len),
            Some(1),
            "同一句钉两次只能留一条"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unpin_removes_the_pinned_item() {
        let dir = isolated_dir("unpin");
        let ctx = test_ctx();
        prefs_pin(&ctx, &json!({ "text": "先读架构文档" }));
        let value = prefs_unpin(&ctx, &json!({ "text": "先读架构文档" }));
        assert_eq!(value["success"], true);
        assert_eq!(value["pinned"].as_array().map(Vec::len), Some(0));
        assert_eq!(
            prefs_list(&ctx, &json!({}))["pinned"].as_array().map(Vec::len),
            Some(0),
            "unpin 必须真的落盘"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fork_unknown_key_returns_session_not_found() {
        let dir = isolated_dir("fork-missing");
        let ctx = test_ctx();
        let missing = "agent:main:webchat:missing";

        let forked = sessions_fork(&ctx, &json!({ "key": missing }));
        assert_eq!(forked["success"], false, "未知 key 不得伪成功：{forked:?}");
        assert_eq!(forked["code"], "SESSION_NOT_FOUND");
        assert_eq!(forked["key"], Value::Null, "失败时不得给出子会话 key");

        let resumed = sessions_resume(&ctx, &json!({ "key": missing }));
        assert_eq!(resumed["success"], false);
        assert_eq!(resumed["code"], "SESSION_NOT_FOUND");

        let replayed = sessions_replay(&ctx, &json!({ "key": missing }));
        assert_eq!(replayed["success"], false);
        assert_eq!(replayed["code"], "SESSION_NOT_FOUND");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fork_point_must_be_a_positive_integer() {
        let dir = isolated_dir("fork-point-type");
        let value = sessions_fork(
            &test_ctx(),
            &json!({ "key": "agent:main:webchat:x", "forkPoint": "2" }),
        );
        assert_eq!(value["success"], false);
        assert_eq!(
            value["code"], "FORK_POINT_INVALID",
            "forkPoint 写错类型不得静默当成全量分支"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fork_copies_parent_then_resume_and_replay_read_the_child() {
        let dir = isolated_dir("fork-roundtrip");
        let ctx = test_ctx();
        let parent = format!("agent:main:webchat:parent-{}", uuid::Uuid::new_v4());

        let mut manager = SessionManager::new(&dir.join("sessions.db"));
        manager.init().expect("init");
        manager
            .save_snapshot(
                &parent,
                "idle",
                &[
                    crate::types::Message::User {
                        content: "第一问".into(),
                        timestamp: chrono::Utc::now(),
                    },
                    crate::types::Message::Assistant {
                        content: "第一答".into(),
                        timestamp: chrono::Utc::now(),
                        thinking: None,
                    },
                ],
                0,
            )
            .expect("写入父会话");
        drop(manager);

        let forked = sessions_fork(
            &ctx,
            &json!({ "key": parent, "forkPoint": 1, "title": "分支标题" }),
        );
        assert_eq!(forked["success"], Value::Null, "成功分支不应带 success:false");
        assert_eq!(forked["forkPoint"], 1);
        assert_eq!(forked["forkMode"], "prefix");
        assert_eq!(forked["title"], "分支标题");
        assert_eq!(forked["parentKey"], json!(parent));
        let child = forked["key"]
            .as_str()
            .unwrap_or_else(|| panic!("必须返回子会话 key：{forked:?}"))
            .to_string();
        assert_ne!(child, parent, "分支必须产生新会话键");

        let resumed = sessions_resume(&ctx, &json!({ "key": child }));
        assert_eq!(resumed["session"], json!(child));
        assert_eq!(resumed["checkpoint"], "latest");
        assert_eq!(resumed["frameCount"], 1);
        assert_eq!(resumed["frames"][0]["text"], "第一问");

        let replayed = sessions_replay(&ctx, &json!({ "key": child }));
        assert_eq!(replayed["readOnly"], true, "回放必须只读");
        assert_eq!(replayed["frameCount"], 1);
        assert_eq!(replayed["frames"][0]["role"], "user");
        let _ = std::fs::remove_dir_all(dir);
    }
}
