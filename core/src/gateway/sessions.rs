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
    // 台账项目归属（用于 DB 行 project_path 为空时回填）；多根优先，旧单值回退
    let ledger_projects: std::collections::HashMap<String, Vec<String>> =
        crate::context::session::context_sessions()
            .into_iter()
            .map(|cs| (cs.session, cs.project_paths))
            .collect();
    let mut items: Vec<(i64, Value)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let running: std::collections::HashSet<String> = crate::gateway::chat::running_sessions()
        .into_iter()
        .collect();
    if let Ok(sessions) = manager.init().and_then(|_| manager.list_sessions(limit)) {
        for s in sessions {
            seen.insert(s.session_id.clone());
            let ledger_paths = ledger_projects
                .get(&s.session_id)
                .cloned()
                .unwrap_or_default();
            let db_paths = manager.project_paths_of(&s.session_id).unwrap_or_default();
            let project_paths = if db_paths.is_empty() {
                ledger_paths
            } else {
                db_paths
            };
            let project_path = s.project_path.clone().or_else(|| project_paths.first().cloned());
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
                    "projectPaths": project_paths,
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
        // 台账多根优先；旧台账只有单值 → 回退成单元素集合
        let ledger_paths: Vec<String> = if cs.project_paths.is_empty() {
            cs.project_path.clone().into_iter().collect()
        } else {
            cs.project_paths.clone()
        };
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
                "projectPaths": ledger_paths,
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

/// `chat.history` 的游标。
///
/// 支持两种形态，兼容旧客户端与前端既有实现：
/// - `idx:<n>` —— **绝对下标**游标（本实现返回的形态）：不受时间戳并列影响，
///   翻页边界不会漏消息；
/// - 纯数字 —— 时间戳游标（旧客户端可能传），按 `ts < before` 语义处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HistoryCursor {
    Index(usize),
    Timestamp(i64),
}

fn parse_history_cursor(v: Option<&Value>) -> Option<HistoryCursor> {
    let v = v?;
    if let Some(s) = v.as_str() {
        if let Some(rest) = s.strip_prefix("idx:") {
            return rest.trim().parse::<usize>().ok().map(HistoryCursor::Index);
        }
        return s.trim().parse::<i64>().ok().map(HistoryCursor::Timestamp);
    }
    v.as_i64().map(HistoryCursor::Timestamp)
}

/// 分页核心（纯函数，便于单测）：返回 `(本页行, 是否还有更早, 最旧游标, 最新游标)`。
///
/// 语义：`before` 是**上界（不含）**，从该位置往前取 `limit` 条。
/// 游标一律用 `idx:<绝对下标>`，避免时间戳并列时整段跳过消息。
fn paginate_history(
    rows: &[(usize, i64, Value)],
    limit: usize,
    before: Option<HistoryCursor>,
) -> (Vec<Value>, bool, Value, Value) {
    let total = rows.len();
    let upper = match before {
        Some(HistoryCursor::Index(n)) => n.min(total),
        Some(HistoryCursor::Timestamp(t)) => rows
            .iter()
            .position(|(_, ts, _)| *ts >= t)
            .unwrap_or(total),
        None => total,
    };
    let start = upper.saturating_sub(limit);
    let page: Vec<Value> = rows[start..upper].iter().map(|(_, _, v)| v.clone()).collect();
    let has_more = start > 0;
    if page.is_empty() {
        return (page, false, Value::Null, Value::Null);
    }
    (
        page,
        has_more,
        json!(format!("idx:{start}")),
        json!(format!("idx:{}", upper - 1)),
    )
}

/// `chat.history`：从 sessions.db 水合会话历史（DB 无记录时回退 context 台账）。
///
/// # 分页契约（R69 修）
///
/// 此前本函数**完全忽略 `limit` 与 `before`**，并且从不返回 `oldest_cursor` /
/// `newest_cursor` —— 而两个前端消费方（`useChatHistory`、`useSessionInspect`）
/// 都按「游标 + has_more」实现分页。于是：
/// - 「加载更早」按钮永远不出现（游标恒为 null，前端 `loadEarlierHistory` 直接 return）；
/// - 每次加载都把**整段会话**塞进 DOM（长会话首屏卡顿、内存暴涨）。
///
/// 现在如实兑现：返回最近 `limit` 条（或 `before` 之前的一页）、给出双向游标与
/// `has_more`。游标用**绝对下标**而不是时间戳 —— 同一毫秒内的多条消息在时间戳
/// 游标下会被整段跳过，那是静默丢历史。
pub fn chat_history(_ctx: &GatewayContext, params: &Value) -> Value {
    let session = params
        .get("sessionKey")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let empty = || {
        json!({
            "messages": [],
            "has_more": false, "hasMore": false,
            "oldest_cursor": Value::Null, "oldestCursor": Value::Null,
            "newest_cursor": Value::Null, "newestCursor": Value::Null,
            "canonical_available": true, "canonicalAvailable": true,
            "canonical_complete": true, "canonicalComplete": true,
            "returned": 0, "total": 0,
        })
    };
    if session.is_empty() {
        return empty();
    }
    // 单页上限 500：再大就不是「分页」而是「整段塞入」，与本次修复的目的相反。
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .clamp(1, 500) as usize;
    let before = parse_history_cursor(params.get("before"));

    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    let loaded = manager.init().and_then(|_| manager.load_snapshot(session));
    let db_ok = loaded.is_ok();
    let messages = loaded.map(|snap| snap.messages).unwrap_or_default();
    let from_db = !messages.is_empty();
    let entries = if from_db {
        Vec::new()
    } else {
        crate::context::session::context_entries(session)
    };

    // 统一成 (绝对下标, 时间戳, 行)。下标必须是**全量列表里的绝对位置**：
    // 前端按 id 去重，id 用绝对下标生成才能在翻页时保持稳定、不重不漏。
    let mut rows: Vec<(usize, i64, Value)> = Vec::new();
    if from_db {
        for (idx, m) in messages.iter().enumerate() {
            // P2-10/T14：工具行保留既有 `text`（兼容）并**附加结构化字段**
            // `elapsedMs`（可空；旧快照为 None → 前端降级不显示耗时）。
            let mut elapsed_ms: Option<u64> = None;
            // 思考正文必须随历史一起下发：`session_messages.thinking` 一直有落盘，
            // 但旧的历史 RPC 只吐 role/text，前端 `mapHistoryMessage` 读不到
            // `reasoning_content`，于是「思考」折叠块在回合结束后永远是空的
            // ——用户点开只能看到「思考过程」标题，看不到任何内容。
            let mut reasoning: Option<String> = None;
            let (role, text) = match m {
                crate::types::Message::User { content, .. } => ("user", content.clone()),
                crate::types::Message::Assistant {
                    content, thinking, ..
                } => {
                    reasoning = thinking.clone();
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
            if let Some(thought) = reasoning
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                row["reasoning_content"] = json!(thought);
            }
            if let Some(ms) = elapsed_ms {
                row["elapsedMs"] = json!(ms);
                row["elapsed_ms"] = json!(ms);
            }
            rows.push((idx, ts, row));
        }
    } else {
        for (idx, e) in entries.iter().enumerate() {
            let role = if e.role == "user" { "user" } else { "assistant" };
            let row = json!({
                "role": role,
                "text": e.text,
                "timestamp": e.at_ms,
                "ts": e.at_ms,
                "id": format!("hist-{idx}"),
                "message_id": format!("hist-{idx}"),
            });
            rows.push((idx, e.at_ms as i64, row));
        }
    }

    let total = rows.len();
    let (page, has_more, oldest_cursor, newest_cursor) = paginate_history(&rows, limit, before);
    // 诚实语义：DB 读到内容 → 权威可用；DB 空且台账也空 → 这是**空会话**（不是不可用）；
    // 只有「DB 读失败且靠台账兜底」才算权威不可用（前端据此提示历史可能不完整）。
    let canonical_available = from_db || (db_ok && entries.is_empty());

    json!({
        "messages": page,
        "has_more": has_more,
        "hasMore": has_more,
        "oldest_cursor": oldest_cursor,
        "oldestCursor": oldest_cursor,
        "newest_cursor": newest_cursor,
        "newestCursor": newest_cursor,
        "canonical_available": canonical_available,
        "canonicalAvailable": canonical_available,
        "canonical_complete": true,
        "canonicalComplete": true,
        "returned": page.len(),
        "total": total,
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
    // 同一会话里可能还有后台编排在跑（蜂群 / 专家团 / 子智能体都不属于聊天任务）：
    // 会话面板的中止也应真正停掉它们，否则只是改了 UI 状态。
    let cancelled_runs = crate::orchestration::cancel::cancel_session(key);
    publish_sessions_changed(ctx, "aborted");
    json!({
        "ok": true,
        "session": key,
        "aborted": true,
        "orchestrationCancelled": cancelled_runs,
    })
}

/// `session.thinking.get`：会话推理档位的读取入口（输入框滑块初始值的**单一事实源**）。
///
/// 返回 `effort: null` 表示该会话从未设定过档位 —— 调用方必须保留自己的默认值，
/// **不能**把它当成 `off`（那会把「没设过」误读成「用户要求不思考」）。
pub fn thinking_get(params: &Value) -> Result<Value, crate::gateway::protocol::RpcError> {
    use crate::gateway::protocol::RpcError;
    let key = super::methods::session_key_param(params)?;
    let prefs = crate::session::thinking_prefs::load(&key);
    if prefs.is_none() {
        // 兼容旧路径：档位可能只写进了 sessions.db（旧版抽屉）。
        let db_path = Config::config_dir().join("sessions.db");
        let mut manager = SessionManager::new(&db_path);
        if manager.init().is_ok() {
            if let Ok((Some(raw), _)) = manager.get_reasoning_settings(&key) {
                let effort = crate::model::thinking::ThinkingEffort::from_key(&raw)
                    .ok_or_else(|| RpcError::new("INVALID_REQUEST", "会话档位非法"))?;
                return Ok(json!({
                    "sessionKey": key,
                    "effort": effort.as_key(),
                    "source": "sessions.db",
                    "records": 0,
                }));
            }
        }
    }
    Ok(json!({
        "sessionKey": key,
        "effort": prefs.as_ref().map(|p| p.effort.as_key()),
        "updatedAt": prefs.as_ref().map(|p| p.updated_at),
        "records": prefs.as_ref().map(|p| p.history.len()).unwrap_or(0),
    }))
}

/// `session.thinking.set`：拖动滑块即落盘（不需要先发一条消息）。
///
/// 写两处：`thinking_prefs`（运行时决策）+ `sessions.db`（列表/抽屉展示）。
/// 非法档位**响亮拒绝**——成本治理不允许 typo 静默升级成 Deep。
pub fn thinking_set(params: &Value) -> Result<Value, crate::gateway::protocol::RpcError> {
    use crate::gateway::protocol::RpcError;
    use crate::model::thinking::ThinkingEffort;
    use crate::session::thinking_prefs::EffortSource;

    let key = super::methods::session_key_param(params)?;
    let raw = params
        .get("effort")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::new("INVALID_REQUEST", "effort 不可为空"))?;
    let effort = ThinkingEffort::from_key(raw).ok_or_else(|| {
        RpcError::new(
            "INVALID_REQUEST",
            format!(
                "effort='{}' 非法；合法值: off|normal|deep|max（别名 low|medium|high）",
                raw.trim()
            ),
        )
    })?;
    let prefs = crate::session::thinking_prefs::set_effort(
        &key,
        effort,
        EffortSource::Session,
        "gateway:session.thinking.set",
    )
    .map_err(|e| RpcError::new("INTERNAL_ERROR", format!("保存思考档位失败: {e}")))?;
    mirror_effort_to_db(&key, Some(effort));
    Ok(json!({
        "ok": true,
        "sessionKey": key,
        "effort": prefs.effort.as_key(),
        // 请求体真正会收到的字段（off 为 null = 不注入）
        "reasoningEffort": prefs.effort.reasoning_effort(),
        "updatedAt": prefs.updated_at,
    }))
}

/// 把会话推理档位镜像进 `sessions.db`（会话列表 / 检查抽屉读的那一列）。
///
/// 为什么要镜像：同一根轴有两个读者 —— 运行时读 `session::thinking_prefs`，
/// 会话列表/抽屉读 `sessions.db.reasoning_effort`。只写一侧，界面上就会出现
/// 「滑块显示深度、抽屉显示继承」的分裂。best-effort：镜像失败只告警，
/// **绝不**因为展示层同步失败而回滚已经生效的运行时档位。
pub fn mirror_effort_to_db(session: &str, effort: Option<crate::model::thinking::ThinkingEffort>) {
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    if let Err(e) = manager.init() {
        tracing::warn!("mirror_effort_to_db init 失败: {}", e);
        return;
    }
    let cur_ctx = manager
        .get_reasoning_settings(session)
        .map(|(_, ctx)| ctx)
        .unwrap_or(None);
    if let Err(e) = manager.set_reasoning_settings(session, effort.map(|e| e.as_key()), cur_ctx) {
        tracing::warn!("mirror_effort_to_db 写入失败: {}", e);
    }
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
        // 非法档位**响亮拒绝**（只校验本次显式提交的值，避免历史脏值卡住 contextWindow 更新）。
        // 存进 DB 一个引擎不认的字符串，等于把「保存成功」变成一句谎话。
        if effort_key.is_some() {
            if let Some(raw) = effort.as_deref() {
                if crate::model::thinking::ThinkingEffort::from_key(raw).is_none() {
                    return json!({
                        "ok": false,
                        "error": format!(
                            "reasoningEffort='{}' 非法；合法值 off|normal|deep|max（别名 low|medium|high）",
                            raw
                        ),
                    });
                }
            }
        }
        if let Err(e) = manager.set_reasoning_settings(key, effort.as_deref(), ctx) {
            return json!({ "ok": false, "error": format!("更新失败: {e}") });
        }
        // ── 写穿：会话思考偏好才是引擎真正读的那一份 ──────────────────────
        //
        // 缺陷背景（实测）：本函数此前只写 `sessions.db.reasoning_effort`，而引擎
        // 决定「本轮想多深」时读的是 `session::thinking_prefs`（`chat.send` 的
        // effort 解析链）—— 全仓没有任何代码读 `reasoning_overrides`，于是抽屉里
        // 改档位表现为「保存成功、模型行为不变」的静默失效。
        //
        // 现在两侧同时写：DB 供列表/审计查询，prefs 供运行时决策，`null` = 清除
        // 覆盖（回落自适应），与 `off`（显式不思考）严格区分。
        if effort_key.is_some() {
            use crate::model::thinking::ThinkingEffort;
            use crate::session::thinking_prefs::{self, EffortSource};
            match effort.as_deref() {
                Some(raw) => match ThinkingEffort::from_key(raw) {
                    Some(e) => {
                        if let Err(err) =
                            thinking_prefs::set_effort(key, e, EffortSource::Session, "sessions.patch")
                        {
                            tracing::warn!("sessions.patch 写穿思考档位失败: {}", err);
                        }
                    }
                    None => {
                        // 走到这里说明校验被绕过（例如直接写 DB 的旧客户端）：
                        // 记一条告警，但绝不让引擎读到非法档位。
                        tracing::warn!(
                            "sessions.patch 收到非法 reasoningEffort='{}'，已忽略运行时生效",
                            raw
                        );
                    }
                },
                None => {
                    if let Err(err) = thinking_prefs::clear(key) {
                        tracing::warn!("sessions.patch 清除思考档位失败: {}", err);
                    }
                }
            }
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

/// 会话绑定的项目根路径：sessions.db 优先，context 台账回退。
///
/// 前端 `onSessionMetadata` → `applyProjectPaths` 靠它恢复「这个会话属于哪个项目」。
/// 此前 subscribe/hydrate 的占位实现不返回该字段，前端只能再发一次 `sessions.list`
/// 兜底（`hydrateProjectRootsFromSessionList`）——多一次往返，且列表分页时可能拿不到。
pub fn session_project_paths(key: &str) -> Vec<String> {
    if key.is_empty() {
        return Vec::new();
    }
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    if manager.init().is_ok() {
        if let Ok(paths) = manager.project_paths_of(key) {
            if !paths.is_empty() {
                return paths;
            }
        }
    }
    crate::context::session::context_sessions()
        .into_iter()
        .find(|cs| cs.session == key)
        .map(|cs| cs.project_paths)
        .unwrap_or_default()
}

/// `sessions.messages.subscribe`（Rust fast ack）：真实元数据 + 诚实的降级声明。
///
/// Rust 引擎没有可回放的实时流缓冲，所以 `current_stream_seq` 恒为 0；
/// `hydration_complete: false` 让前端继续走一次 `hydrate`（在线时由侧车补齐
/// run_mode_lock / 待输入等字段）。**不再**声称本次订阅已完成全部水合。
pub fn messages_subscribe(params: &Value) -> Value {
    let key = params.get("key").and_then(Value::as_str).unwrap_or_default();
    json!({
        "key": key,
        "subscribed": true,
        "hydration_complete": false,
        "deferred_fields": ["workspaceId", "run_mode_lock", "active_task_group_ids"],
        "current_stream_seq": 0,
        "active_task_group_ids": [],
        "projectPaths": session_project_paths(key),
        "degraded": true,
        "degradedReason": "rust_live_run_state_unavailable: Rust 引擎没有可回放的实时流缓冲",
    })
}

/// `sessions.messages.snapshot` 的离线诚实版：空事件 + degraded（绝不冒充完整快照）。
pub fn messages_snapshot_degraded(params: &Value) -> Value {
    let key = params.get("key").and_then(Value::as_str).unwrap_or_default();
    json!({
        "key": key,
        "task_id": Value::Null,
        "current_stream_seq": 0,
        "events": [],
        "degraded": true,
        "degradedReason": "rust_stream_state_unavailable: 实时轮次快照由 Python 侧车提供",
    })
}

/// `sessions.messages.hydrate` 的 Rust 子集：真实项目根 + 明确声明缺失字段。
pub fn messages_hydrate_rust(params: &Value) -> Value {
    let key = params.get("key").and_then(Value::as_str).unwrap_or_default();
    json!({
        "key": key,
        "projectPaths": session_project_paths(key),
        "hydration_complete": true,
        "deferred_fields": ["workspaceId", "run_mode_lock", "active_task_group_ids"],
        "degraded": true,
        "degradedReason": "rust_metadata_subset: 运行态元数据由 Python 侧车提供",
    })
}

/// 侧车 hydrate 与 Rust 子集合并：侧车字段为底，Rust 的非空 `projectPaths` 覆盖。
///
/// 两个引擎的会话库不同源（Rust 读 `config_dir/sessions.db`，侧车读自己的 state 目录），
/// 所以「本引擎会话绑定到哪个项目」只有 Rust 自己知道；其余字段（run_mode_lock /
/// active_task_group_ids / pendingUserInputs）由侧车提供。
pub fn merge_hydration(rust: Value, sidecar: Value) -> Value {
    let mut out = sidecar;
    let rust_paths = rust.get("projectPaths").cloned().unwrap_or(Value::Null);
    let has_paths = rust_paths
        .as_array()
        .map(|paths| !paths.is_empty())
        .unwrap_or(false);
    if let Some(obj) = out.as_object_mut() {
        if has_paths {
            obj.insert("projectPaths".to_string(), rust_paths);
        }
        obj.entry("hydration_complete".to_string())
            .or_insert(json!(true));
    }
    out
}

#[cfg(test)]
mod hydration_shape_tests {
    use super::*;
    use std::path::PathBuf;

    /// 每个用例独占配置目录：`session_project_paths` 会打开（必要时创建）
    /// `config_dir/sessions.db`，绝不能落到真实用户的 ~/.marguerite。
    fn isolated_config_dir(tag: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "marguerite-sessions-{tag}-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("创建临时配置目录");
        Config::set_config_dir_override(dir.clone());
        dir
    }

    #[test]
    fn subscribe_reports_metadata_but_never_claims_complete_hydration() {
        let dir = isolated_config_dir("subscribe");
        let value = messages_subscribe(&json!({ "key": "agent:main:webchat:shape" }));
        assert_eq!(value["subscribed"], true);
        assert_eq!(
            value["hydration_complete"], false,
            "Rust 没有流缓冲时不得声称已完整水合（否则前端不会再调 hydrate）"
        );
        assert_eq!(value["current_stream_seq"], 0);
        assert_eq!(value["degraded"], true);
        assert!(value["degradedReason"].as_str().unwrap_or("").contains("rust"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn snapshot_degraded_is_empty_and_never_claims_events() {
        let value = messages_snapshot_degraded(&json!({ "key": "agent:main:webchat:shape" }));
        assert_eq!(value["events"].as_array().map(Vec::len), Some(0));
        assert_eq!(value["task_id"], Value::Null);
        assert_eq!(value["degraded"], true);
    }

    #[test]
    fn hydrate_subset_fills_project_paths_and_marks_what_is_missing() {
        let dir = isolated_config_dir("hydrate");
        let value = messages_hydrate_rust(&json!({ "key": "agent:main:webchat:shape" }));
        assert_eq!(value["hydration_complete"], true);
        let deferred = value["deferred_fields"].as_array().cloned().unwrap_or_default();
        assert!(deferred.iter().any(|field| field == "run_mode_lock"));
        assert_eq!(value["degraded"], true);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn merge_keeps_sidecar_fields_and_prefers_rust_project_paths() {
        let rust = json!({
            "projectPaths": ["/rust/project"],
            "hydration_complete": true,
        });
        let sidecar = json!({
            "workspaceId": "ws-1",
            "run_mode_lock": { "locked": false },
            "projectPaths": ["/sidecar/stale"],
            "hydration_complete": true,
        });
        let merged = merge_hydration(rust, sidecar);
        assert_eq!(merged["workspaceId"], "ws-1");
        assert_eq!(merged["run_mode_lock"]["locked"], false);
        assert_eq!(merged["projectPaths"][0], "/rust/project");
    }

    #[test]
    fn merge_does_not_wipe_sidecar_paths_when_rust_has_none() {
        let rust = json!({ "projectPaths": [] });
        let sidecar = json!({ "projectPaths": ["/sidecar/project"] });
        let merged = merge_hydration(rust, sidecar);
        assert_eq!(merged["projectPaths"][0], "/sidecar/project");
    }
}

#[cfg(test)]
mod history_pagination_tests {
    use super::*;

    fn rows(n: usize) -> Vec<(usize, i64, Value)> {
        (0..n)
            .map(|i| (i, 1_700_000_000_000 + i as i64, json!({ "id": format!("hist-{i}") })))
            .collect()
    }

    #[test]
    fn first_page_returns_the_most_recent_slice() {
        let (page, has_more, oldest, newest) = paginate_history(&rows(120), 50, None);
        assert_eq!(page.len(), 50);
        assert_eq!(page[0]["id"], "hist-70", "首屏应是最近 50 条");
        assert_eq!(page[49]["id"], "hist-119");
        assert!(has_more, "更早还有 70 条");
        assert_eq!(oldest, json!("idx:70"));
        assert_eq!(newest, json!("idx:119"));
    }

    #[test]
    fn prepend_walks_backwards_without_gaps_or_duplicates() {
        let all = rows(120);
        let (p1, more1, oldest1, _) = paginate_history(&all, 50, None);
        assert!(more1);
        let cursor = parse_history_cursor(Some(&oldest1)).expect("游标应可解析");
        let (p2, more2, oldest2, _) = paginate_history(&all, 50, Some(cursor));
        assert_eq!(p2.len(), 50);
        assert_eq!(p2[0]["id"], "hist-20");
        assert_eq!(p2[49]["id"], "hist-69", "第二页必须紧邻第一页，不重叠");
        assert!(more2);
        let cursor2 = parse_history_cursor(Some(&oldest2)).unwrap();
        let (p3, more3, _, _) = paginate_history(&all, 50, Some(cursor2));
        assert_eq!(p3.len(), 20);
        assert_eq!(p3[0]["id"], "hist-0");
        assert!(!more3, "到头后不得再报 has_more（否则前端会无限拉取）");
        // 三页拼起来正好是全部，无重复无缺失
        let mut ids: Vec<String> = p3
            .iter()
            .chain(p2.iter())
            .chain(p1.iter())
            .map(|v| v["id"].as_str().unwrap().to_string())
            .collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 120);
    }

    #[test]
    fn same_millisecond_messages_are_not_skipped_by_timestamp_cursor() {
        // 时间戳游标的经典坑：10 条消息同一毫秒，按 ts < before 会整段跳过。
        // 用下标游标后不会。
        let mut r: Vec<(usize, i64, Value)> = (0..10)
            .map(|i| (i, 1_700_000_000_000, json!({ "id": format!("hist-{i}") })))
            .collect();
        let (p1, _, oldest, _) = paginate_history(&r, 4, None);
        assert_eq!(p1.len(), 4);
        let cursor = parse_history_cursor(Some(&oldest)).unwrap();
        let (p2, _, oldest2, _) = paginate_history(&r, 4, Some(cursor));
        assert_eq!(p2[0]["id"], "hist-2", "同毫秒也必须逐条推进");
        let cursor2 = parse_history_cursor(Some(&oldest2)).unwrap();
        let (p3, more3, _, _) = paginate_history(&r, 4, Some(cursor2));
        assert_eq!(p3.len(), 2);
        assert!(!more3);
        r.clear();
    }

    #[test]
    fn legacy_timestamp_cursor_still_works() {
        let all = rows(10);
        // 传时间戳（第 5 条的 ts）→ 返回它之前的全部
        let ts = all[5].1;
        let (page, has_more, _, _) =
            paginate_history(&all, 50, Some(HistoryCursor::Timestamp(ts)));
        assert_eq!(page.len(), 5);
        assert_eq!(page[4]["id"], "hist-4");
        assert!(!has_more);
    }

    #[test]
    fn empty_history_yields_null_cursors_and_no_more() {
        let (page, more, oldest, newest) = paginate_history(&[], 50, None);
        assert!(page.is_empty());
        assert!(!more);
        assert_eq!(oldest, Value::Null);
        assert_eq!(newest, Value::Null);
    }

    #[test]
    fn limit_is_clamped_by_caller_but_helper_handles_any_value() {
        let (page, _, _, _) = paginate_history(&rows(3), 100, None);
        assert_eq!(page.len(), 3, "limit 大于总量时返回全部，不 panic");
    }
}
