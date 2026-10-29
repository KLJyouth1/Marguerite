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

/// 从会话 key 推导列表元数据 `(sessionKind, surface, conversationKind)`。
///
/// 此前 `sessions.list` 对每一行都硬编码 `chat` / `web` / `main`：CLI 与 TUI
/// 会话因此混进 Web 侧栏、子智能体会话被当成普通聊天（不缩进、不归任务），
/// 多智能体场景下所有行都挂在「main」名下。前端虽有按 key 推导的兜底，但
/// **后端只要给出显式值（哪怕是假的）就被采信**，兜底永远不会执行。
/// 这里按 pyengine `session/keys.py` 的权威契约如实推导：
/// `agent:<id>:main`、`agent:<id>:webchat:<rest>`、`agent:<id>:<channel>:<kind>:<peer>`、
/// `agent:<id>:subagent:<run>`、内部面（cli/tui/mcp/standalone）、`cron:*`、`web.<id>`。
/// 渠道适配器名不受限（pyengine 里就是 `channel or "unknown"`），不能按白名单判。
fn session_list_meta(key: &str) -> (&'static str, &str, &'static str) {
    // 旧 `subagent:<base>` 前缀（pyengine `is_subagent_key` 仍认），没有 agent 段。
    if key.starts_with("subagent:") {
        return ("task", "subagent", "internal");
    }
    // `web.<id>`（`sessions.create` 的旧形态）没有 `:` 分段，会一路掉到下面的
    // `""` 分支被当成 unknown。它其实是 main 的直连 chat 会话 —— 用既有的
    // 判定函数显式接住，别让合法旧键在列表里显示成「未知类型」。
    if is_legacy_webchat_key(key) {
        return ("chat", "web", "direct");
    }
    let parts: Vec<&str> = key.split(':').collect();
    let surface = if parts.first() == Some(&"agent") && parts.len() >= 3 {
        parts[2]
    } else {
        ""
    };
    match surface {
        "subagent" => ("task", "subagent", "internal"),
        "cli" | "standalone" => ("chat", "cli", "internal"),
        "tui" => ("chat", "tui", "internal"),
        "mcp" => ("chat", "mcp", "internal"),
        "cron" => ("cron", "cron", "unknown"),
        "main" => ("chat", "main", "direct"),
        "webchat" => ("chat", "webchat", "direct"),
        _ if key.starts_with("cron:") || key.contains(":cron:") => ("cron", "cron", "unknown"),
        "" => ("unknown", "unknown", "unknown"),
        // 其余 surface 是渠道适配器名（slack / discord / feishu / …）：pyengine
        // 用 `agent:<id>:<channel>:<kind>:<peer>` 建键，kind 决定会话类型。
        _ => {
            let kind = if key.contains(":group:") {
                "group"
            } else if key.contains(":channel:") {
                "channel"
            } else if key.contains(":direct:") || key.contains(":dm:") {
                "direct"
            } else {
                "group"
            };
            ("channel", surface, kind)
        }
    }
}

/// `web.<id>`：`sessions.create` 的既有默认键（前端从不生成，但 API 客户端
/// 可能用），属于合法的旧形态。
fn is_legacy_webchat_key(key: &str) -> bool {
    key.strip_prefix("web.")
        .map(|rest| !rest.is_empty())
        .unwrap_or(false)
}

/// 会话 key 是否属于产品**可寻址**的会话形态（决定它是否进入 `sessions.list`）。
///
/// 契约来自 pyengine `session/keys.py` —— 合法的 `agent:` 键至少三段
/// （`agent:<id>:main` / `agent:<id>:<surface>:<rest>`），surface 既可以是内部面，
/// 也可以是任意渠道适配器名；另有旧前缀 `subagent:`、`cron:*` 与 `web.<id>`。
/// 探针 / 压测客户端自选的裸键（`ws.*` / `qa*` / `bench1_*` …）不符合任何形态：
/// 它们没有 surface，无法归属导航分组，也不该出现在任何人的会话列表里。
/// 数据仍然保留 —— 知道 key 时 `chat.history` / `sessions.get` 照常可读。
fn is_addressable_session_key(key: &str) -> bool {
    let parts: Vec<&str> = key.split(':').collect();
    if parts.first() == Some(&"agent") && parts.len() >= 3 {
        return !parts[1].is_empty() && !parts[2].is_empty();
    }
    if let Some(rest) = key.strip_prefix("subagent:") {
        return !rest.is_empty();
    }
    if let Some(rest) = key.strip_prefix("cron:") {
        return !rest.is_empty();
    }
    is_legacy_webchat_key(key)
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
    if let Ok(sessions) = manager.init().and_then(|_| manager.list_all_sessions()) {
        for s in sessions {
            // 列表是**产品会话索引**：只收有明确 surface 归属、且真的有消息的会话。
            // 探针裸键与零消息占位行（`sessions.create` 的空快照等）不再以
            // 「普通聊天」的身份出现 —— 真机库实测 325 行里 196 行是前者、16 行
            // 是后者，侧栏因此长期满屏重复标题。过滤必须在截断之前做，否则它们
            // 会挤掉真实会话（见 `list_all_sessions` 的注释）。
            if !is_addressable_session_key(&s.session_id) || s.message_count == 0 {
                continue;
            }
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
            let (session_kind, surface, conversation_kind) = session_list_meta(&s.session_id);
            let effective_agent_id =
                crate::agent::store::agent_id_from_session(&s.session_id).unwrap_or("main");
            items.push((
                rfc3339_to_ms(&s.updated_at),
                json!({
                    "key": s.session_id,
                    "session": s.session_id,
                    "sessionKey": s.session_id,
                    "title": s.summary,
                    "summary": s.summary,
                    "sessionKind": session_kind,
                    "surface": surface,
                    "conversationKind": conversation_kind,
                    "effectiveAgentId": effective_agent_id,
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
        // 与 DB 行同一口径：无 surface 归属 / 无消息的台账不进入产品列表。
        if !is_addressable_session_key(&cs.session) || cs.message_count == 0 {
            continue;
        }
        let is_running = running.contains(&cs.session);
        // 台账多根优先；旧台账只有单值 → 回退成单元素集合
        let ledger_paths: Vec<String> = if cs.project_paths.is_empty() {
            cs.project_path.clone().into_iter().collect()
        } else {
            cs.project_paths.clone()
        };
        let (session_kind, surface, conversation_kind) = session_list_meta(&cs.session);
        let effective_agent_id =
            crate::agent::store::agent_id_from_session(&cs.session).unwrap_or("main");
        items.push((
            cs.updated_ms as i64,
            json!({
                "key": cs.session,
                "session": cs.session,
                "sessionKey": cs.session,
                "title": cs.title,
                "summary": cs.title,
                "sessionKind": session_kind,
                "surface": surface,
                "conversationKind": conversation_kind,
                "effectiveAgentId": effective_agent_id,
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
///
/// # 工具行归一（会话历史与流式同形）
///
/// `session_messages` 里的 `role='tool'` 行**不再**作为独立消息下发：WebUI 没有
/// `tool` 角色的渲染分支，它会落到 `SystemMessage` 兜底，把 `[name] input → output`
/// 原样显示成系统气泡；而同一回合在流式期间渲染的是可折叠工具卡。刷新一次就变成
/// 两副样子，用户感知为「历史内容和刚才不一样」。本函数把成串的工具行折成结构化
/// `tool_calls`（与流式载荷同形），由**紧邻其后**的 assistant 行承接；没有紧邻
/// assistant 行（回合中断）时合成一条空文本 assistant 行，绝不跨 user/system 行
/// 归属。`total` / 游标基于折叠后的行集，因此分页条数等于用户真正看到的条数。
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
    // 工具成败信号：只有 DB 路径带 `tool_ok`；台账条目没有该列，按未知呈现。
    let tool_outcomes = if from_db {
        manager.tool_outcome_flags(session).unwrap_or_default()
    } else {
        Vec::new()
    };
    let entries = if from_db {
        Vec::new()
    } else {
        crate::context::session::context_entries(session)
    };

    // 统一成 (绝对下标, 时间戳, 行)。下标必须是**全量列表里的绝对位置**：
    // 前端按 id 去重，id 用绝对下标生成才能在翻页时保持稳定、不重不漏。
    let mut rows: Vec<(usize, i64, Value)> = Vec::new();
    if from_db {
        // 工具行不再以独立 `role:"tool"` 消息下发：前端 `ChatMessageList`
        // 只为 user / maintenance / assistant 分派组件，其余一律落到
        // `SystemMessage` 兜底 —— 于是 `[name] input → output` 这坨原始文本
        // 会变成系统气泡，而同一回合在流式期间渲染的是可折叠工具卡。
        // 用户看到的就是「刷新一次，工具卡变回原始文本、内容对不上」。
        //
        // 现在折成结构化 `tool_calls`（与流式载荷同形）：
        // - 紧跟其后的 assistant 行承接（正常一回合：user → tool* → assistant，
        //   与流式渲染同形：工具卡属于它后面那条回复）；
        // - 没有紧邻 assistant 行（回合被打断 / 仍在执行）时单独合成一条
        //   空文本 assistant 行 —— 绝不把上一回合的工具算到下一回合头上
        //   （中间只要有 user/system 行就断开）。
        let mut tool_calls_by_row: std::collections::HashMap<usize, Vec<Value>> =
            std::collections::HashMap::new();
        let mut synthetic_rows: std::collections::HashMap<usize, (i64, Vec<Value>)> =
            std::collections::HashMap::new();
        let mut cursor = 0usize;
        while cursor < messages.len() {
            if !matches!(messages[cursor], crate::types::Message::Tool { .. }) {
                cursor += 1;
                continue;
            }
            let run_start = cursor;
            let first_ts = messages[run_start].timestamp().timestamp_millis();
            let mut calls: Vec<Value> = Vec::new();
            while let Some(crate::types::Message::Tool {
                name,
                input,
                output,
                elapsed_ms,
                ..
            }) = messages.get(cursor)
            {
                let call_idx = cursor;
                cursor += 1;
                // 空工具名的行没有可展示的卡片；`normalizeToolName` 也会把它
                // 丢掉，留着只会造出一条空 assistant 气泡。
                // `router_control` 是控制面内部工具，WebUI 的
                // `isInternalToolName` 同样把它当透明项 —— 一并跳过，
                // 否则「只调了 router_control 的回合」会渲染成一张空卡片。
                let trimmed = name.trim();
                if trimmed.is_empty() || trimmed == "router_control" {
                    continue;
                }
                let mut call = json!({
                    "type": "tool_call",
                    "tool_use_id": format!("hist-tool-{call_idx}"),
                    "name": name,
                    "input": input,
                    "result": output,
                });
                if let Some(ms) = elapsed_ms {
                    call["elapsedMs"] = json!(ms);
                }
                // `tool_ok` 是权威成败信号（与流式事件同源落盘）。不带上它，
                // 前端只能按「有没有输出」推断 —— 失败工具刷新后显示成功。
                // NULL（旧数据/回合中断）= 无信号：不下发，前端按未知处理。
                match tool_outcomes.get(call_idx).copied().flatten() {
                    Some(true) => {
                        call["ok"] = json!(true);
                        call["execution_status"] = json!({ "status": "success" });
                    }
                    Some(false) => {
                        call["ok"] = json!(false);
                        call["is_error"] = json!(true);
                        call["execution_status"] = json!({ "status": "error" });
                    }
                    None => {}
                }
                calls.push(call);
            }
            if calls.is_empty() {
                continue;
            }
            if matches!(
                messages.get(cursor),
                Some(crate::types::Message::Assistant { .. })
            ) {
                tool_calls_by_row.entry(cursor).or_default().append(&mut calls);
                cursor += 1;
            } else {
                synthetic_rows.insert(run_start, (first_ts, calls));
            }
        }

        for (idx, m) in messages.iter().enumerate() {
            // 悬挂工具（后面没有 assistant 行）：合成一条空文本 assistant 行，
            // 让工具卡仍按回合出现，而不是被静默丢弃。
            if let Some((ts, calls)) = synthetic_rows.get(&idx) {
                rows.push((
                    idx,
                    *ts,
                    json!({
                        "role": "assistant",
                        "text": "",
                        "timestamp": ts,
                        "ts": ts,
                        "id": format!("hist-{idx}"),
                        "message_id": format!("hist-{idx}"),
                        "tool_calls": calls,
                    }),
                ));
                continue;
            }
            if matches!(m, crate::types::Message::Tool { .. }) {
                continue;
            }
            // 思考正文必须随历史一起下发：`session_messages.thinking` 一直有落盘，
            // 但旧的历史 RPC 只吐 role/text，前端 `mapHistoryMessage` 读不到
            // `reasoning_content`，于是「思考」折叠块在回合结束后永远是空的
            // ——用户点开只能看到「思考过程」标题，看不到任何内容。
            let mut reasoning: Option<String> = None;
            let (role, text): (&str, String) = match m {
                crate::types::Message::User { content, .. } => ("user", content.clone()),
                crate::types::Message::Assistant {
                    content, thinking, ..
                } => {
                    reasoning = thinking.clone();
                    ("assistant", content.clone())
                }
                crate::types::Message::System { content, .. } => ("system", content.clone()),
                // 工具行已在上方整体折叠；`continue` 让新增变体时编译器
                // 仍然要求这里处理，而不是静默落成一条没人认领的行。
                crate::types::Message::Tool { .. } => continue,
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
            if let Some(calls) = tool_calls_by_row.get(&idx) {
                row["tool_calls"] = json!(calls);
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
    let (session_kind, surface, conversation_kind) = session_list_meta(key);
    let effective_agent_id = crate::agent::store::agent_id_from_session(key).unwrap_or("main");
    json!({
        "key": key,
        "session": key,
        "sessionKey": key,
        "title": title,
        "summary": title,
        "sessionKind": session_kind,
        "surface": surface,
        "conversationKind": conversation_kind,
        "effectiveAgentId": effective_agent_id,
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
    if let Err(e) = manager.init() {
        return json!({ "ok": false, "error": format!("会话落库失败: {e}") });
    }
    // 幂等创建：键已存在时**绝不覆盖**。
    //
    // 旧实现无条件 `save_snapshot(&key, "idle", &[], 0)`，而 `save_snapshot`
    // 的语义是「整段重写」—— 于是任何复用已有键的创建调用（重连、前端重试、
    // 从历史里「新建」同键会话）都会把整段历史清空，且返回 `ok:true`。
    if let Ok(Some(existing)) = manager.get_session(&key) {
        if let Some(project) = params
            .get("projectPath")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            crate::context::session::bind_session_project(&key, project);
        }
        let item = session_item(
            &existing.session_id,
            &existing.summary,
            existing.message_count,
            rfc3339_to_ms(&existing.updated_at),
            existing.reasoning_effort.as_deref(),
            existing.context_window,
        );
        return json!({ "ok": true, "session": item, "existing": true });
    }
    if let Err(e) = manager.save_snapshot(&key, "idle", &[], 0) {
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

/// `sessions.delete`：删除会话（DB 行 + 消息级联 + 上下文台账 + 压缩快照）。
///
/// 此前只删 `sessions.db`：`data_dir/context/<key>.json` 与压缩快照留在磁盘，
/// 而 `sessions.list` 有「台账兜底」—— 用户删掉的会话会在下一次刷新时**复活**
/// （甚至在压缩快照里继续占空间）。同时停掉本进程内的运行句柄：正在跑的任务
/// 结束后会重新 `save_snapshot`，等于删除后又写回一条。
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
    let stopped = crate::gateway::chat::stop_running_task(key);
    let cancelled_runs = crate::orchestration::cancel::cancel_session(key);
    let ledger_removed = crate::context::session::remove_session(key);
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    match manager.init().and_then(|_| manager.delete_session(key)) {
        Ok(()) => {
            publish_sessions_changed(ctx, "deleted");
            json!({
                "ok": true,
                "deleted": key,
                "stopped_running": stopped,
                "orchestration_cancelled": cancelled_runs,
                "ledger_removed": ledger_removed,
            })
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

/// `sessions.search`：搜索会话（标题/ID 模糊匹配 + 消息全文子串匹配）。
/// P0 契约补齐：此前前端调用但后端完全缺失。
///
/// 前端命令面板读的是 `res.sessions` 与 `res.messages`（`CommandPalette.vue`），
/// 而本函数此前只返回 `items`、还从 `sessions_list` 里取不存在的 `items` 键 ——
/// 标题和对话两个面都恒为空。现在按前端契约返回，`items` 仅为兼容保留。
pub fn sessions_search(ctx: &GatewayContext, params: &Value) -> Value {
    let query = params.get("query").and_then(Value::as_str).unwrap_or("");
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .min(200) as usize;
    // 复用 sessions_list 获取全部，再 filter
    let list = sessions_list(ctx, &json!({ "limit": 200 }));
    let items = list
        .get("sessions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
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

    // 对话面：DB 消息全文子串搜索（空查询不扫全表）。
    let messages: Vec<Value> = if query.trim().is_empty() {
        Vec::new()
    } else {
        let db_path = Config::config_dir().join("sessions.db");
        let mut manager = SessionManager::new(&db_path);
        let hits = manager
            .init()
            .and_then(|_| manager.search_messages(query, limit.min(50)))
            .unwrap_or_default();
        hits.into_iter()
            // 命令面板是产品面：探针裸键的消息同样不出现（与列表同一口径）。
            .filter(|hit| is_addressable_session_key(&hit.session_id))
            .map(|hit| {
                json!({
                    // 命令面板用 key 跳转会话，必须是会话键而不是消息唯一 id
                    "key": hit.session_id,
                    "title": hit.title,
                    "role": hit.role,
                    "snippet": hit.snippet,
                    "createdAt": hit.created_at_ms,
                    "effectiveAgentId": "main",
                })
            })
            .collect()
    };

    json!({
        "items": filtered,
        "sessions": filtered,
        "messages": messages,
        "total": filtered.len(),
        "ok": true,
    })
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

/// `sessions.reset`：清空会话的对话内容（保留会话记录本身）。
///
/// 1. **先停运行中的任务**：不清空一个还在产出的回合 —— 否则流式输出会在用户
///    刚清空的对话里继续长出消息。侧车的同一 RPC 也会先取消运行任务（对齐语义）。
/// 2. **清 context 台账**：模型历史是从台账灌进上下文的，只清 `sessions.db`
///    等于「界面空了、模型还记得」。项目归属与预算保留（会话仍属同一工作区）。
/// 3. 清 `sessions.db` 的消息行（前端翻看的历史）。
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
    let stopped = crate::gateway::chat::stop_running_task(key);
    let cancelled_runs = crate::orchestration::cancel::cancel_session(key);
    let ledger_cleared = crate::context::session::reset_session(key);
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    match manager.init().and_then(|_| manager.clear_messages(key)) {
        Ok(()) => {
            publish_sessions_changed(ctx, "reset");
            json!({
                "ok": true,
                "session": key,
                "reset": true,
                "stopped_running": stopped,
                "orchestration_cancelled": cancelled_runs,
                "ledger_cleared": ledger_cleared,
            })
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

    /// 「清空对话」必须让模型真的忘记。
    ///
    /// 旧实现的 `sessions.reset` 只删 `sessions.db` 的消息行，而模型历史是从
    /// context 台账灌进上下文的（`chat::session_history_messages`）—— 于是界面
    /// 空了，模型仍记得全部前文。这条用例盯住台账被清、工作区归属保留，以及
    /// 回包如实报告清到了什么。
    #[test]
    fn reset_clears_model_memory_and_reports_what_it_cleared() {
        let dir = isolated_config_dir("reset-memory");
        let ctx = test_ctx();
        let key = "web.reset-memory";
        crate::context::session::record_turn(key, "user", "记住这个暗号：青鸟");
        crate::context::session::record_turn(key, "assistant", "已记住");
        crate::context::session::bind_session_projects(key, &["D:/proj/a".to_string()]);
        assert_eq!(crate::context::session::context_entries(key).len(), 2);

        let out = sessions_reset(&ctx, &json!({ "key": key }));
        assert_eq!(out["ok"], true);
        assert_eq!(out["reset"], true);
        assert_eq!(out["ledger_cleared"], true, "台账必须被清，回包要如实说");
        assert!(
            crate::context::session::context_entries(key).is_empty(),
            "模型历史必须真的被清空，否则「清空对话」只是把界面擦干净"
        );
        assert_eq!(
            crate::context::session::session_projects(key),
            vec!["D:/proj/a".to_string()],
            "清空对话不得抹掉工作区归属"
        );

        // 再清一次：已经没有内容，如实回报「没清到东西」，不假装成功清了一大堆。
        let again = sessions_reset(&ctx, &json!({ "key": key }));
        assert_eq!(again["ok"], true);
        assert_eq!(again["ledger_cleared"], false);

        let missing = sessions_reset(&ctx, &json!({}));
        assert_eq!(missing["ok"], false, "缺 key 必须拒绝");
        let _ = std::fs::remove_dir_all(dir);
    }

    fn test_ctx() -> GatewayContext {
        GatewayContext {
            config: Config::default(),
            token: "t".into(),
            started_at: std::time::Instant::now(),
            bus: std::sync::Arc::new(crate::events::EventBus::default()),
        }
    }

    /// `sessions.create` 对已存在的键必须幂等：复用键（重连 / 前端重试）
    /// 过去会走 `save_snapshot(..., &[], 0)` 把整段历史清空，且回 `ok:true`。
    #[test]
    fn create_is_idempotent_and_never_wipes_existing_history() {
        let dir = isolated_config_dir("create-idempotent");
        let ctx = test_ctx();
        let key = "web.create-idempotent";
        let db_path = dir.join("sessions.db");

        let mut manager = SessionManager::new(&db_path);
        manager.init().expect("init");
        let messages = vec![
            crate::types::Message::User {
                content: "第一问".into(),
                timestamp: chrono::Utc::now(),
            },
            crate::types::Message::Assistant {
                content: "第一答".into(),
                timestamp: chrono::Utc::now(),
                thinking: None,
            },
        ];
        manager
            .save_snapshot(key, "idle", &messages, 0)
            .expect("写入历史");

        let created = sessions_create(&ctx, &json!({ "key": key }));
        assert_eq!(created["ok"], true);
        assert_eq!(created["existing"], true, "已存在的键必须走幂等分支");
        assert_eq!(created["session"]["messageCount"], 2);

        let mut check = SessionManager::new(&db_path);
        check.init().expect("init");
        let snap = check.load_snapshot(key).expect("会话应仍存在");
        assert_eq!(snap.messages.len(), 2, "create 绝不能清空历史");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `chat.history` 必须把 `role='tool'` 行折成结构化 `tool_calls`。
    ///
    /// 此前的行为是：工具行原样成一条 `role:"tool"` 消息，而前端
    /// `ChatMessageList` 没有 `tool` 分支 → 落到 `SystemMessage` 兜底，
    /// `[name] input → output` 的原始文本直接铺成系统气泡；同一回合在流式
    /// 期间却是可折叠工具卡。「刷新一次，历史内容和刚才不一样」就是这么来的。
    #[test]
    fn history_folds_tool_rows_into_structured_tool_calls() {
        let dir = isolated_config_dir("history-tool-fold");
        let key = "web.history-tool-fold";
        let mut manager = SessionManager::new(&dir.join("sessions.db"));
        manager.init().expect("init");
        // 正常一回合：user → tool → tool → assistant（工具卡属于后面的回复）
        manager
            .append_message(key, "user", "看看目录", "2026-10-26T10:00:00Z", Some("看看目录"))
            .expect("user");
        manager
            .append_tool_message(
                key,
                "shell_exec",
                "dir",
                "a.txt",
                Some(12),
                Some(true),
                "2026-10-26T10:00:01Z",
            )
            .expect("tool 1");
        manager
            .append_tool_message(
                key,
                "read_file",
                "a.txt",
                "hello",
                None,
                Some(true),
                "2026-10-26T10:00:02Z",
            )
            .expect("tool 2");
        manager
            .append_message(key, "assistant", "目录里只有 a.txt", "2026-10-26T10:00:03Z", None)
            .expect("assistant");
        // 悬挂工具：回合被打断，后面没有 assistant 行
        manager
            .append_message(key, "user", "再跑一次", "2026-10-26T10:01:00Z", None)
            .expect("user 2");
        manager
            .append_tool_message(
                key,
                "shell_exec",
                "dir /w",
                "b.txt",
                Some(7),
                Some(false),
                "2026-10-26T10:01:01Z",
            )
            .expect("tool 3");

        let value = chat_history(&test_ctx(), &json!({ "sessionKey": key }));
        let messages = value["messages"].as_array().cloned().unwrap_or_default();

        assert!(
            messages.iter().all(|m| m["role"] != "tool"),
            "工具行不得再作为独立消息下发（会被渲染成系统气泡）：{messages:?}"
        );
        assert_eq!(value["total"], 4, "折叠后的条数才是用户看到的条数：{messages:?}");
        assert_eq!(
            messages.iter().map(|m| m["role"].clone()).collect::<Vec<_>>(),
            vec!["user", "assistant", "user", "assistant"],
            "{messages:?}"
        );

        let reply = &messages[1];
        assert_eq!(reply["text"], "目录里只有 a.txt", "回复正文必须原样保留");
        assert_eq!(reply["message_id"], "hist-3", "承接行的 id 保持绝对下标");
        let calls = reply["tool_calls"].as_array().cloned().unwrap_or_default();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["name"], "shell_exec");
        assert_eq!(calls[0]["input"], "dir");
        assert_eq!(calls[0]["result"], "a.txt");
        assert_eq!(calls[0]["elapsedMs"], 12);
        assert_eq!(calls[1]["name"], "read_file");
        assert_eq!(calls[1]["elapsedMs"], Value::Null, "未测到耗时不臆造");

        let dangling = &messages[3];
        assert_eq!(dangling["message_id"], "hist-5");
        assert_eq!(dangling["text"], "");
        let dangling_calls = dangling["tool_calls"].as_array().cloned().unwrap_or_default();
        assert_eq!(dangling_calls.len(), 1);
        assert_eq!(dangling_calls[0]["name"], "shell_exec");
        assert_eq!(dangling_calls[0]["result"], "b.txt");

        let _ = std::fs::remove_dir_all(dir);
    }

    /// `sessions.list` 的会话元数据必须从 key 如实推导。
    ///
    /// 此前每行都硬编码 `chat` / `web` / `main`：子智能体会话被当成普通聊天、
    /// CLI/TUI 会话混进 Web 侧栏，且前端「按 key 推导」的兜底永远不执行
    /// （显式值优先）。此用例钉住三类真实 key 与一个未知形态的回落。
    #[test]
    fn list_derives_agent_surface_and_kind_from_the_session_key() {
        let dir = isolated_config_dir("list-meta");
        let ctx = test_ctx();
        let mut manager = SessionManager::new(&dir.join("sessions.db"));
        manager.init().expect("init");
        for key in [
            "agent:data-analyst:subagent:run-1",
            "agent:main:cli:abc",
            "web.legacy",
        ] {
            manager
                .append_message(key, "user", "问一句", "2026-10-27T10:00:00Z", Some("问一句"))
                .expect("append");
        }

        let value = sessions_list(&ctx, &json!({ "limit": 50 }));
        let rows = value["sessions"].as_array().cloned().unwrap_or_default();
        let row = |key: &str| {
            rows.iter()
                .find(|r| r["key"] == key)
                .cloned()
                .unwrap_or_else(|| panic!("缺少会话行 {key}：{rows:?}"))
        };

        let subagent = row("agent:data-analyst:subagent:run-1");
        assert_eq!(subagent["effectiveAgentId"], "data-analyst");
        assert_eq!(subagent["sessionKind"], "task");
        assert_eq!(subagent["surface"], "subagent");
        assert_eq!(subagent["conversationKind"], "internal");

        let cli = row("agent:main:cli:abc");
        assert_eq!(cli["effectiveAgentId"], "main");
        assert_eq!(cli["sessionKind"], "chat");
        assert_eq!(cli["surface"], "cli");
        assert_eq!(cli["conversationKind"], "internal");

        let legacy = row("web.legacy");
        assert_eq!(legacy["effectiveAgentId"], "main", "`web.<id>` 旧形态归属 main");
        assert_eq!(legacy["sessionKind"], "chat", "`web.<id>` 是 sessions.create 的旧形态");
        assert_eq!(legacy["surface"], "web");

        // 探针裸键不再伪装成 chat/web（它们不会进入列表，这里直接钉 meta）。
        assert_eq!(
            session_list_meta("ws.probe-key"),
            ("unknown", "unknown", "unknown")
        );
        assert_eq!(
            session_list_meta("agent:main:main"),
            ("chat", "main", "direct"),
            "pyengine 的 DM 主键只有三段，不得因段数少被判成未知"
        );
        assert_eq!(
            session_list_meta("agent:main:webchat:x"),
            ("chat", "webchat", "direct")
        );
        assert_eq!(
            session_list_meta("agent:main:slack:group:C123"),
            ("channel", "slack", "group"),
            "渠道适配器名不受限：`agent:<id>:<channel>:<kind>:<peer>` 必须归到渠道"
        );
        assert_eq!(
            session_list_meta("agent:main:discord:direct:u1"),
            ("channel", "discord", "direct")
        );
        assert_eq!(
            session_list_meta("subagent:agent:main:webchat:x"),
            ("task", "subagent", "internal"),
            "旧 `subagent:` 前缀没有 agent 段，仍须归为子智能体任务"
        );
        assert_eq!(
            session_list_meta("cron:job-1:run:abc"),
            ("cron", "cron", "unknown")
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    /// 列表只收**可寻址且非空**的会话。
    ///
    /// 真机库实测 325 行里 196 行是探针裸键（`ws.*` / `qa*` / `bench1_*` …）、
    /// 16 行是零消息占位行 —— 前者过去被 meta 兜底成「普通 Web 聊天」，
    /// 于是侧栏满屏近似重复的标题（用户可见投诉）。过滤必须发生在截断之前，
    /// 否则它们会挤掉真实会话。数据本身保留：按 key 直读历史仍然可用。
    #[test]
    fn list_hides_probe_keys_and_empty_sessions_but_keeps_history_addressable() {
        let dir = isolated_config_dir("list-addressable");
        let ctx = test_ctx();
        let mut manager = SessionManager::new(&dir.join("sessions.db"));
        manager.init().expect("init");
        for key in [
            "agent:main:webchat:real-1",
            "agent:main:main",
            "agent:main:slack:group:C123",
            "subagent:agent:main:webchat:x",
            "web.legacy-used",
            "cron:job-1",
            "ws.probe-key",
            "qa.bench-1",
            "bench1_1788236785679",
            "sess-fully-bogus",
        ] {
            manager
                .append_message(key, "user", "问一句", "2026-10-27T10:00:00Z", Some("问一句"))
                .expect("append");
        }
        // 零消息占位行（sessions.create 空快照 / touch_session 的既有产物）
        manager
            .touch_session("agent:main:webchat:fresh-but-empty")
            .expect("touch");
        manager
            .save_snapshot("web.empty-created", "idle", &[], 0)
            .expect("empty snapshot");

        let value = sessions_list(&ctx, &json!({ "limit": 50 }));
        let keys: Vec<String> = value["sessions"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|r| r["key"].as_str().map(str::to_string))
            .collect();

        for visible in [
            "agent:main:webchat:real-1",
            "agent:main:main",
            "agent:main:slack:group:C123",
            "subagent:agent:main:webchat:x",
            "web.legacy-used",
            "cron:job-1",
        ] {
            assert!(
                keys.iter().any(|k| k == visible),
                "应列出 {visible}：{keys:?}"
            );
        }
        for hidden in [
            "ws.probe-key",
            "qa.bench-1",
            "bench1_1788236785679",
            "sess-fully-bogus",
            "agent:main:webchat:fresh-but-empty",
            "web.empty-created",
        ] {
            assert!(
                !keys.iter().any(|k| k == hidden),
                "不得列出 {hidden}：{keys:?}"
            );
        }

        // 过滤只是「不把探针当产品会话」，历史仍在、按 key 可读。
        let history = chat_history(&ctx, &json!({ "sessionKey": "ws.probe-key" }));
        assert_eq!(history["total"], 1, "探针会话历史按 key 仍可读：{history:?}");

        let _ = std::fs::remove_dir_all(dir);
    }

    /// `sessions.search` 的返回键必须与前端命令面板一致（`sessions` / `messages`）。
    ///
    /// 旧实现从 `sessions_list` 里读取不存在的 `items` 键，且只回 `items` ——
    /// 命令面板两个面都恒为空，看起来像「没有历史」。
    #[test]
    fn search_returns_the_contract_the_command_palette_reads() {
        let dir = isolated_config_dir("search-contract");
        let ctx = test_ctx();
        let key = "web.search-contract";
        let mut manager = SessionManager::new(&dir.join("sessions.db"));
        manager.init().expect("init");
        manager
            .save_snapshot(
                key,
                "idle",
                &[
                    crate::types::Message::User {
                        content: "帮我排查部署失败的原因".into(),
                        timestamp: chrono::Utc::now(),
                    },
                    crate::types::Message::Assistant {
                        content: "日志里显示端口被占用".into(),
                        timestamp: chrono::Utc::now(),
                        thinking: None,
                    },
                ],
                0,
            )
            .expect("写入历史");

        // 会话面：按键名检索
        let by_key = sessions_search(&ctx, &json!({ "query": "search-contract", "limit": 10 }));
        assert_eq!(by_key["ok"], true);
        let sessions = by_key["sessions"].as_array().expect("sessions 数组");
        assert!(
            sessions.iter().any(|s| s["key"] == key),
            "会话面应命中：{sessions:?}"
        );
        assert_eq!(by_key["items"], by_key["sessions"], "items 仅为兼容别名");

        // 对话面：消息正文命中，snippet 带高亮标记，key 是会话键（前端据此跳转）
        let by_msg = sessions_search(&ctx, &json!({ "query": "端口", "limit": 10 }));
        let messages = by_msg["messages"].as_array().expect("messages 数组");
        let hit = messages
            .iter()
            .find(|m| m["key"] == key)
            .unwrap_or_else(|| panic!("对话面应命中：{messages:?}"));
        assert!(
            hit["snippet"].as_str().unwrap_or("").contains(">>>端口<<<"),
            "命中片段应带高亮标记：{hit:?}"
        );
        assert_eq!(hit["role"], "assistant");

        // 空查询不扫全表
        let empty = sessions_search(&ctx, &json!({ "query": "  ", "limit": 10 }));
        assert_eq!(empty["messages"].as_array().map(Vec::len), Some(0));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `sessions.delete` 必须把 context 台账一起删掉，否则列表的台账兜底
    /// 会让用户刚删掉的会话在下次刷新时「复活」。
    #[test]
    fn delete_removes_db_row_and_context_ledger() {
        let dir = isolated_config_dir("delete-ledger");
        let ctx = test_ctx();
        let key = "web.delete-ledger";
        crate::context::session::record_turn(key, "user", "hello");
        let mut manager = SessionManager::new(&dir.join("sessions.db"));
        manager.init().expect("init");
        manager.save_snapshot(key, "idle", &[], 0).expect("建会话");

        let deleted = sessions_delete(&ctx, &json!({ "key": key }));
        assert_eq!(deleted["ok"], true);
        assert_eq!(deleted["ledger_removed"], true);

        let mut check = SessionManager::new(&dir.join("sessions.db"));
        check.init().expect("init");
        assert!(check.get_session(key).expect("读会话").is_none());
        assert!(
            crate::context::session::context_entries(key).is_empty(),
            "删除后台账不得留下任何条目（否则会话会复活）"
        );
        let _ = std::fs::remove_dir_all(dir);
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
