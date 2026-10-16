//! 会话管理方法族
//!
//! 提供会话列表、历史水合等核心能力。数据源为 sessions.db + 进程级 context 台账双轨。

use serde_json::{json, Value};

use crate::gateway::methods::GatewayContext;
use crate::persistence::config::Config;
use crate::persistence::session::SessionManager;

/// RFC3339 → 毫秒时间戳（解析失败返回 0）。
fn rfc3339_to_ms(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.timestamp_millis())
        .unwrap_or(0)
}

/// `sessions.list`：列出会话（DB 记录 + context 台账双轨合并）。
pub fn sessions_list(_ctx: &GatewayContext, params: &Value) -> Value {
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .min(200) as usize;
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    // 台账项目归属（用于 DB 行 project_path 为空时回填）
    let ledger_projects: std::collections::HashMap<String, Option<String>> =
        crate::context::session::context_sessions()
            .into_iter()
            .map(|cs| (cs.session, cs.project_path))
            .collect();
    let mut items: Vec<(i64, Value)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let running: std::collections::HashSet<String> = crate::gateway::chat::running_sessions()
        .into_iter()
        .collect();
    if let Ok(sessions) = manager.init().and_then(|_| manager.list_sessions(limit)) {
        for s in sessions {
            seen.insert(s.session_id.clone());
            let project_path = s
                .project_path
                .clone()
                .or_else(|| ledger_projects.get(&s.session_id).cloned().flatten());
            let is_running = running.contains(&s.session_id);
            items.push((
                rfc3339_to_ms(&s.updated_at),
                json!({
                    "key": s.session_id,
                    "session": s.session_id,
                    "sessionKey": s.session_id,
                    "title": s.summary,
                    "summary": s.summary,
                    "sessionKind": "chat",
                    "surface": "web",
                    "conversationKind": "chat",
                    "effectiveAgentId": "main",
                    "messageCount": s.message_count,
                    "updatedAt": rfc3339_to_ms(&s.updated_at),
                    "createdAt": rfc3339_to_ms(&s.created_at),
                    "status": if is_running { "running" } else { "idle" },
                    "runStatus": if is_running { "running" } else { "idle" },
                    "subtitle": "",
                    "projectPath": project_path,
                    // 会话级成本治理覆盖（null = 继承全局模型配置）
                    "reasoningEffort": s.reasoning_effort,
                    "reasoning_effort": s.reasoning_effort,
                    "contextWindow": s.context_window,
                    "context_window": s.context_window,
                }),
            ));
        }
    }
    // context 台账会话（防止重启后会话消失）
    for cs in crate::context::session::context_sessions() {
        if seen.contains(&cs.session) {
            continue;
        }
        let is_running = running.contains(&cs.session);
        items.push((
            cs.updated_ms as i64,
            json!({
                "key": cs.session,
                "session": cs.session,
                "sessionKey": cs.session,
                "title": cs.title,
                "summary": cs.title,
                "sessionKind": "chat",
                "surface": "web",
                "conversationKind": "chat",
                "effectiveAgentId": "main",
                "messageCount": cs.message_count,
                "updatedAt": cs.updated_ms,
                "createdAt": cs.updated_ms,
                "status": if is_running { "running" } else { "idle" },
                "runStatus": if is_running { "running" } else { "idle" },
                "subtitle": "",
                "projectPath": cs.project_path,
            }),
        ));
    }
    // 按最近更新时间降序，再按 limit 截断
    items.sort_by_key(|(updated_ms, _)| *updated_ms);
    items.reverse();
    items.truncate(limit);
    let items: Vec<Value> = items.into_iter().map(|(_, v)| v).collect();
    let keys: Vec<Value> = items.iter().map(|v| v["key"].clone()).collect();
    json!({ "sessions": items, "keys": keys })
}

/// `chat.history`：从 sessions.db 水合会话历史（DB 无记录时回退 context 台账）。
pub fn chat_history(_ctx: &GatewayContext, params: &Value) -> Value {
    let session = params
        .get("sessionKey")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if session.is_empty() {
        return json!({ "messages": [], "has_more": false });
    }
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    let messages = match manager.init().and_then(|_| manager.load_snapshot(session)) {
        Ok(snap) => snap.messages,
        Err(_) => Vec::new(),
    };
    let from_db = !messages.is_empty();
    let entries = if from_db {
        Vec::new()
    } else {
        crate::context::session::context_entries(session)
    };
    let out: Vec<Value> = if from_db {
        messages
            .iter()
            .enumerate()
            .map(|(idx, m)| {
                // P2-10/T14：工具行保留既有 `text`（兼容）并**附加结构化字段**
                // `elapsedMs`（可空；旧快照为 None → 前端降级不显示耗时）。
                let mut elapsed_ms: Option<u64> = None;
                let (role, text) = match m {
                    crate::types::Message::User { content, .. } => ("user", content.clone()),
                    crate::types::Message::Assistant { content, .. } => {
                        ("assistant", content.clone())
                    }
                    crate::types::Message::System { content, .. } => ("system", content.clone()),
                    crate::types::Message::Tool {
                        name,
                        input,
                        output,
                        elapsed_ms: tool_elapsed,
                        ..
                    } => {
                        elapsed_ms = *tool_elapsed;
                        ("tool", format!("[{name}] {input}\n→ {output}"))
                    }
                };
                let ts = m.timestamp().timestamp_millis();
                let mut row = json!({
                    "role": role,
                    "text": text,
                    "timestamp": ts,
                    "ts": ts,
                    "id": format!("hist-{idx}"),
                    "message_id": format!("hist-{idx}"),
                });
                if let Some(ms) = elapsed_ms {
                    row["elapsedMs"] = json!(ms);
                    row["elapsed_ms"] = json!(ms);
                }
                row
            })
            .collect()
    } else {
        entries
            .iter()
            .enumerate()
            .map(|(idx, e)| {
                let role = if e.role == "user" {
                    "user"
                } else {
                    "assistant"
                };
                json!({
                    "role": role,
                    "text": e.text,
                    "timestamp": e.at_ms,
                    "ts": e.at_ms,
                    "id": format!("hist-{idx}"),
                    "message_id": format!("hist-{idx}"),
                })
            })
            .collect()
    };
    json!({
        "messages": out,
        "has_more": false,
        "hasMore": false,
        "canonical_available": !out.is_empty(),
        "canonicalAvailable": !out.is_empty(),
        "canonical_complete": true,
        "canonicalComplete": true,
        "returned": out.len(),
    })
}

/// 构造与会话列表条目同构的对象（sessions.create 返回值复用）。
///
/// `reasoning_effort` / `context_window` 是**会话级成本治理覆盖**（None = 回退全局
/// 模型配置）。前端据此渲染"继承全局"而非谎报具体档位。
fn session_item(
    key: &str,
    title: &str,
    message_count: usize,
    updated_ms: i64,
    reasoning_effort: Option<&str>,
    context_window: Option<u32>,
) -> Value {
    let is_running = crate::gateway::chat::running_sessions().contains(&key.to_string());
    json!({
        "key": key,
        "session": key,
        "sessionKey": key,
        "title": title,
        "summary": title,
        "sessionKind": "chat",
        "surface": "web",
        "conversationKind": "chat",
        "effectiveAgentId": "main",
        "messageCount": message_count,
        "updatedAt": updated_ms,
        "createdAt": updated_ms,
        "status": if is_running { "running" } else { "idle" },
        "runStatus": if is_running { "running" } else { "idle" },
        "subtitle": "",
        // 会话级成本治理覆盖（null = 继承全局模型配置）
        "reasoningEffort": reasoning_effort,
        "reasoning_effort": reasoning_effort,
        "contextWindow": context_window,
        "context_window": context_window,
    })
}

/// 广播会话列表变更（前端据 `sessions.changed` 刷新侧栏）。
fn publish_sessions_changed(ctx: &GatewayContext, reason: &str) {
    ctx.bus.publish(crate::events::Envelope::new(
        crate::events::EventType::SessionsChanged,
        json!({ "reason": reason }),
        "system",
    ));
}

/// `sessions.create`：新建空会话并落库（Rust 回退，Phase 3 引擎桥离线时也可用）。
///
/// 此前该方法返回 NOT_IMPLEMENTED，前端无法通过 API 建会话。现在直接写
/// sessions.db（空快照占位），可选绑定项目路径；立即广播 `sessions.changed`。
pub fn sessions_create(ctx: &GatewayContext, params: &Value) -> Value {
    let key = params
        .get("key")
        .or_else(|| params.get("sessionKey"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("web.{}", uuid::Uuid::new_v4()));
    let title = params
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("新会话")
        .to_string();

    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    if let Err(e) = manager
        .init()
        .and_then(|_| manager.save_snapshot(&key, "idle", &[], 0))
    {
        return json!({ "ok": false, "error": format!("会话落库失败: {e}") });
    }
    // 项目归属（context 台账；save_snapshot_for_project 需要真实消息列表，此处仅登记）
    if let Some(project) = params
        .get("projectPath")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        crate::context::session::bind_session_project(&key, project);
    }

    let now_ms = chrono::Utc::now().timestamp_millis();
    let item = session_item(&key, &title, 0, now_ms, None, None);
    publish_sessions_changed(ctx, "created");
    json!({ "ok": true, "session": item })
}

/// `sessions.delete`：删除会话（DB 行 + 消息级联）。
///
/// 说明：进程内 context 台账无移除 API，重启后台账自然收敛；
/// DB 是持久层权威数据源，删除即生效。
pub fn sessions_delete(ctx: &GatewayContext, params: &Value) -> Value {
    let key = params
        .get("key")
        .or_else(|| params.get("sessionKey"))
        .or_else(|| params.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    if key.is_empty() {
        return json!({ "ok": false, "error": "缺少 key" });
    }
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    match manager.init().and_then(|_| manager.delete_session(key)) {
        Ok(()) => {
            publish_sessions_changed(ctx, "deleted");
            json!({ "ok": true, "deleted": key })
        }
        Err(e) => json!({ "ok": false, "error": format!("删除失败: {e}") }),
    }
}

/// `sessions.get`：读取单个会话元信息（DB 优先，回退台账摘要）。
pub fn sessions_get(_ctx: &GatewayContext, params: &Value) -> Value {
    let key = params
        .get("key")
        .or_else(|| params.get("sessionKey"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if key.is_empty() {
        return json!({ "ok": false, "error": "缺少 key" });
    }
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    if manager.init().is_ok() {
        if let Ok(Some(s)) = manager.get_session(key) {
            let item = session_item(
                &s.session_id,
                &s.summary,
                s.message_count,
                rfc3339_to_ms(&s.updated_at),
                s.reasoning_effort.as_deref(),
                s.context_window,
            );
            return json!({ "ok": true, "session": item });
        }
    }
    // 台账回退
    if let Some(cs) = crate::context::session::context_sessions()
        .into_iter()
        .find(|cs| cs.session == key)
    {
        let item = session_item(
            &cs.session,
            &cs.title,
            cs.message_count,
            cs.updated_ms as i64,
            None,
            None,
        );
        return json!({ "ok": true, "session": item });
    }
    json!({ "ok": false, "error": "会话不存在", "code": "session.not_found" })
}

/// `sessions.search`：搜索会话（标题/ID 模糊匹配）。
/// P0 契约补齐：此前前端调用但后端完全缺失。
pub fn sessions_search(ctx: &GatewayContext, params: &Value) -> Value {
    let query = params.get("query").and_then(Value::as_str).unwrap_or("");
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .min(200) as usize;
    // 复用 sessions_list 获取全部，再 filter
    let list = sessions_list(ctx, &json!({ "limit": 200 }));
    let items = list.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
    let filtered: Vec<Value> = items
        .iter()
        .filter(|item| {
            if query.is_empty() {
                return true;
            }
            let title = item.get("title").and_then(Value::as_str).unwrap_or("");
            let key = item.get("key").and_then(Value::as_str).unwrap_or("");
            title.to_lowercase().contains(&query.to_lowercase())
                || key.to_lowercase().contains(&query.to_lowercase())
        })
        .take(limit)
        .cloned()
        .collect();
    json!({ "items": filtered, "total": filtered.len(), "ok": true })
}

/// `sessions.abort`：中止会话（标记状态为 Aborted）。
pub fn sessions_abort(ctx: &GatewayContext, params: &Value) -> Value {
    let key = params
        .get("session")
        .or_else(|| params.get("sessionKey"))
        .or_else(|| params.get("key"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if key.is_empty() {
        return json!({ "ok": false, "error": "缺少 session key" });
    }
    publish_sessions_changed(ctx, "aborted");
    json!({ "ok": true, "session": key, "aborted": true })
}

/// `sessions.patch`：更新会话元数据（标题/摘要 + 会话级思考强度/上下文窗口覆盖）。
///
/// 成本治理语义（与 `SessionManager::set_reasoning_settings` 一致）：
/// - `reasoningEffort`: `off`/`normal`/`deep`/`max`；`null`/`""` = 清除覆盖、继承全局
/// - `contextWindow`: 正整数 token；`null`/`0` = 清除覆盖、继承全局
/// - 两个字段**缺省（不传）= 不改动**，传 `null` = 清除——区分"未提及"与"显式清除"。
pub fn sessions_patch(_ctx: &GatewayContext, params: &Value) -> Value {
    let key = params
        .get("session")
        .or_else(|| params.get("sessionKey"))
        .or_else(|| params.get("key"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let title = params.get("title").or_else(|| params.get("summary")).and_then(Value::as_str);
    if key.is_empty() {
        return json!({ "ok": false, "error": "缺少 session key" });
    }
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    if let Err(e) = manager.init() {
        return json!({ "ok": false, "error": format!("初始化失败: {e}") });
    }
    if let Some(t) = title {
        if let Err(e) = manager.update_summary(key, Some(t)) {
            return json!({ "ok": false, "error": format!("更新失败: {e}") });
        }
    }
    // 会话级成本治理覆盖：仅当字段**显式出现**时才写（缺省不动）。
    let effort_key = params
        .get("reasoningEffort")
        .or_else(|| params.get("reasoning_effort"));
    let ctx_key = params.get("contextWindow").or_else(|| params.get("context_window"));
    if effort_key.is_some() || ctx_key.is_some() {
        // 读现有值，仅覆盖本次显式出现的字段（避免另一个字段被误清空）。
        let (cur_effort, cur_ctx) = manager
            .get_reasoning_settings(key)
            .unwrap_or((None, None));
        let effort: Option<String> = match effort_key {
            Some(Value::Null) => None,
            Some(v) => v
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            None => cur_effort,
        };
        let ctx: Option<u32> = match ctx_key {
            Some(Value::Null) => None,
            Some(v) => v.as_u64().map(|x| x as u32).filter(|x| *x > 0),
            None => cur_ctx,
        };
        if let Err(e) = manager.set_reasoning_settings(key, effort.as_deref(), ctx) {
            return json!({ "ok": false, "error": format!("更新失败: {e}") });
        }
        return json!({
            "ok": true,
            "session": key,
            "title": title,
            "reasoningEffort": effort,
            "contextWindow": ctx,
        });
    }
    json!({ "ok": true, "session": key, "title": title })
}

/// `sessions.preview`：预览会话最近消息。
pub fn sessions_preview(_ctx: &GatewayContext, params: &Value) -> Value {
    let key = params
        .get("session")
        .or_else(|| params.get("sessionKey"))
        .or_else(|| params.get("key"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if key.is_empty() {
        return json!({ "ok": false, "error": "缺少 session key" });
    }
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    if manager.init().is_ok() {
        if let Ok(snapshot) = manager.load_snapshot(key) {
            let msgs: Vec<Value> = snapshot
                .messages
                .iter()
                .rev()
                .take(5)
                .rev()
                .map(|m| {
                    let content = match m {
                        crate::types::Message::User { content, .. } => content.clone(),
                        crate::types::Message::Assistant { content, .. } => content.clone(),
                        crate::types::Message::System { content, .. } => content.clone(),
                        crate::types::Message::Tool { name, input, output, .. } => {
                            format!("{}: {} → {}", name, input, output)
                        }
                    };
                    json!({
                        "role": m.role(),
                        "content": content,
                    })
                })
                .collect();
            return json!({ "ok": true, "session": key, "preview": msgs });
        }
    }
    json!({ "ok": false, "error": "会话不存在或无消息", "code": "session.not_found" })
}

/// `sessions.reset`：重置会话消息（保留会话记录，清除消息）。
pub fn sessions_reset(ctx: &GatewayContext, params: &Value) -> Value {
    let key = params
        .get("session")
        .or_else(|| params.get("sessionKey"))
        .or_else(|| params.get("key"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if key.is_empty() {
        return json!({ "ok": false, "error": "缺少 session key" });
    }
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    match manager.init().and_then(|_| manager.clear_messages(key)) {
        Ok(()) => {
            publish_sessions_changed(ctx, "reset");
            json!({ "ok": true, "session": key, "reset": true })
        }
        Err(e) => json!({ "ok": false, "error": format!("重置失败: {e}") }),
    }
}

/// `sessions.subscribe`：订阅会话变更事件。
pub fn sessions_subscribe(_ctx: &GatewayContext, params: &Value) -> Value {
    let key = params
        .get("session")
        .or_else(|| params.get("sessionKey"))
        .and_then(Value::as_str)
        .unwrap_or("*");
    json!({ "ok": true, "event": "sessions.changed", "session": key, "subscribed": true })
}

/// `sessions.unsubscribe`：取消订阅会话变更事件。
pub fn sessions_unsubscribe(_ctx: &GatewayContext, params: &Value) -> Value {
    let key = params
        .get("session")
        .or_else(|| params.get("sessionKey"))
        .and_then(Value::as_str)
        .unwrap_or("*");
    json!({ "ok": true, "event": "sessions.changed", "session": key, "subscribed": false })
}
