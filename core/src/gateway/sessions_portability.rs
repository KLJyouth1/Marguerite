//! 会话可移植性：导出 / 分享（Markdown）/ 导入。
//!
//! ## 设计取舍
//!
//! - 直接按 SQL 行导出，而不是把 `Message` 反序列化再序列化：消息表里
//!   `thinking / tool_name / tool_input / tool_output / elapsed_ms` 是**一等信息**，
//!   走结构体中转容易丢字段。行级导出天然无损。
//! - 导入**不保留原会话 ID**（除非显式 `keepId` 且不冲突）：导入两份同名导出是常态，
//!   静默覆盖用户已有会话是不可接受的副作用。
//! - 导入后的会话 `state` 强制为 `Ready`。
//! - 项目归属（P2-9 多根）：导出携带 `projectPaths`；导入时**仅当**该主路径在
//!   本机存在才恢复绑定，否则保持未绑定（导入的是历史记录，不该挂到不存在的目录）。
//!   恢复情况在结果的 `boundProjects` 字段里如实回报。
//! - 不做任何内容执行/求值：`tool_input` 等字段原样当文本处理。

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OpenFlags};
use serde_json::{json, Value};

use crate::persistence::config::Config;
use crate::persistence::session::SessionManager;

/// 导出格式版本（消费方必须校验）
pub const SCHEMA_VERSION: u32 = 1;
const MAX_SESSIONS_EXPORT: usize = 200;
const MAX_MESSAGES_PER_SESSION: usize = 20_000;
const MAX_IMPORT_SESSIONS: usize = 200;
const MAX_MARKDOWN_CHARS: usize = 400_000;

fn db_path() -> PathBuf {
    Config::config_dir().join("sessions.db")
}

/// 探测表里是否存在某列（导出走只读连接，不会跑迁移，需自行兼容旧库）。
fn has_column(conn: &Connection, table: &str, column: &str) -> bool {
    let sql = format!("PRAGMA table_info({table})");
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return false;
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(1)) else {
        return false;
    };
    let names: Vec<String> = rows.filter_map(Result::ok).collect();
    names.iter().any(|n| n == column)
}


fn err(msg: impl Into<String>, code: &str) -> Value {
    json!({ "ok": false, "error": msg.into(), "code": code })
}

struct SessionRow {
    session_id: String,
    summary: String,
    state: String,
    message_count: i64,
    tokens_used: i64,
    created_at: String,
    updated_at: String,
    project_path: Option<String>,
    /// P2-9 多根：JSON 数组字符串（旧库行为 NULL）
    project_paths: Option<String>,
    reasoning_effort: Option<String>,
    context_window: Option<i64>,
}

struct MsgRow {
    role: String,
    content: String,
    thinking: Option<String>,
    tool_name: Option<String>,
    tool_input: Option<String>,
    tool_output: Option<String>,
    elapsed_ms: Option<i64>,
    timestamp: String,
}

/// 一行会话的项目归属：`project_paths` 有效且非空优先，否则回退 `project_path`。
fn row_projects(s: &SessionRow) -> Vec<String> {
    if let Some(raw) = s.project_paths.as_deref() {
        if let Ok(parsed) = serde_json::from_str::<Vec<String>>(raw) {
            if !parsed.is_empty() {
                return parsed;
            }
        }
    }
    s.project_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| vec![p.to_string()])
        .unwrap_or_default()
}

fn session_json(s: &SessionRow, msgs: &[MsgRow]) -> Value {
    let messages: Vec<Value> = msgs
        .iter()
        .map(|m| {
            json!({
                "role": m.role,
                "content": m.content,
                "thinking": m.thinking,
                "toolName": m.tool_name,
                "toolInput": m.tool_input,
                "toolOutput": m.tool_output,
                "elapsedMs": m.elapsed_ms,
                "timestamp": m.timestamp,
            })
        })
        .collect();
    json!({
        "sessionId": s.session_id,
        "title": s.summary,
        "state": s.state,
        "messageCount": s.message_count,
        "tokensUsed": s.tokens_used,
        "createdAt": s.created_at,
        "updatedAt": s.updated_at,
        "projectPath": s.project_path,
        "projectPaths": row_projects(s),
        "reasoningEffort": s.reasoning_effort,
        "contextWindow": s.context_window,
        "messages": messages,
    })
}

fn markdown_for(s: &SessionRow, msgs: &[MsgRow]) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", s.summary.trim()));
    out.push_str(&format!(
        "> 会话 `{}` · {} 条消息 · 更新于 {}\n\n",
        s.session_id, msgs.len(), s.updated_at
    ));
    for m in msgs {
        let who = match m.role.as_str() {
            "user" => "用户",
            "assistant" => "助手",
            "system" => "系统",
            "tool" => "工具",
            other => other,
        };
        out.push_str(&format!("### {who} · {}\n\n", m.timestamp));
        if let Some(t) = m.thinking.as_deref().filter(|t| !t.trim().is_empty()) {
            out.push_str("<details><summary>思考过程</summary>\n\n");
            out.push_str(t.trim());
            out.push_str("\n\n</details>\n\n");
        }
        if let Some(name) = m.tool_name.as_deref() {
            out.push_str(&format!("**工具调用** `{name}`\n\n"));
            if let Some(inp) = m.tool_input.as_deref().filter(|v| !v.trim().is_empty()) {
                out.push_str(&format!("输入：\n```json\n{inp}\n```\n\n"));
            }
            if let Some(o) = m.tool_output.as_deref().filter(|v| !v.trim().is_empty()) {
                let clipped: String = o.chars().take(4000).collect();
                out.push_str(&format!("输出：\n```\n{clipped}\n```\n\n"));
            }
        }
        let body = m.content.trim();
        if !body.is_empty() {
            out.push_str(body);
            out.push_str("\n\n");
        }
        if out.chars().count() > MAX_MARKDOWN_CHARS {
            out.push_str("\n> 内容过长，已截断。请使用 JSON 格式导出完整数据。\n");
            break;
        }
    }
    out
}

/// `sessions.export`
///
/// - `sessionId`：只导出该会话；缺省 = 最近 [`MAX_SESSIONS_EXPORT`] 个
/// - `format`：`json`（默认，完整可再用）| `markdown`（分享用）
/// - `includeMessages`：默认 true
pub fn export(params: &Value) -> Value {
    // 参数校验必须早于「数据库不存在」的早退：否则没有会话库时
    // `format=pdf` 会被当成成功（ok:true, sessions:[]），调用方拿到假成功。
    // 并行测试里会话库时有时无，这个顺序错误会表现成随机失败。
    let format = params
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("json");
    if !matches!(format, "json" | "markdown") {
        return err("format 只支持 json / markdown", "INVALID_REQUEST");
    }
    let path = db_path();
    if !path.exists() {
        return json!({
            "ok": true,
            "schemaVersion": SCHEMA_VERSION,
            "sessions": [],
            "format": "json",
            "note": "尚无会话数据库（首次对话后生成）",
        });
    }
    let include_messages = params
        .get("includeMessages")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let session_id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let conn = match Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(c) => c,
        Err(e) => return err(format!("打开会话库失败: {e}"), "SESSION_DB_UNAVAILABLE"),
    };
    // 旧库可能还没有 `project_paths` 列（只读连接不会跑迁移）→ 用 NULL 占位。
    let paths_col = if has_column(&conn, "sessions", "project_paths") {
        "project_paths"
    } else {
        "NULL AS project_paths"
    };
    // 单会话查询需要额外参数：用两条路径避免 SQL 拼接注入。
    let rows = if let Some(id) = session_id {
        let sql = format!(
            "SELECT session_id, summary, state, message_count, tokens_used, created_at, updated_at,
                    project_path, reasoning_effort, context_window, {paths_col}
             FROM sessions WHERE session_id = ?1"
        );
        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(e) => return err(format!("查询会话失败: {e}"), "SESSION_QUERY_FAILED"),
        };
        match stmt.query_row(params![id], |r| {
            Ok(SessionRow {
                session_id: r.get(0)?,
                summary: r.get(1)?,
                state: r.get(2)?,
                message_count: r.get(3)?,
                tokens_used: r.get(4)?,
                created_at: r.get(5)?,
                updated_at: r.get(6)?,
                project_path: r.get(7)?,
                project_paths: r.get(10)?,
                reasoning_effort: r.get(8)?,
                context_window: r.get(9)?,
            })
        }) {
            Ok(s) => vec![s],
            Err(rusqlite::Error::QueryReturnedNoRows) => Vec::new(),
            Err(e) => return err(format!("查询会话失败: {e}"), "SESSION_QUERY_FAILED"),
        }
    } else {
        let sql = format!(
            "SELECT session_id, summary, state, message_count, tokens_used, created_at, updated_at,
                    project_path, reasoning_effort, context_window, {paths_col}
             FROM sessions ORDER BY updated_at DESC LIMIT ?1"
        );
        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(e) => return err(format!("查询会话失败: {e}"), "SESSION_QUERY_FAILED"),
        };
        let mapped = stmt.query_map(params![MAX_SESSIONS_EXPORT as i64], |r| {
            Ok(SessionRow {
                session_id: r.get(0)?,
                summary: r.get(1)?,
                state: r.get(2)?,
                message_count: r.get(3)?,
                tokens_used: r.get(4)?,
                created_at: r.get(5)?,
                updated_at: r.get(6)?,
                project_path: r.get(7)?,
                project_paths: r.get(10)?,
                reasoning_effort: r.get(8)?,
                context_window: r.get(9)?,
            })
        });
        let rows: Vec<SessionRow> = match mapped {
            Ok(rows) => rows.filter_map(Result::ok).collect(),
            Err(e) => return err(format!("查询会话失败: {e}"), "SESSION_QUERY_FAILED"),
        };
        rows
    };
    let mut sessions: Vec<Value> = Vec::with_capacity(rows.len());
    let mut markdowns: Vec<String> = Vec::new();
    for s in &rows {
        let msgs: Vec<MsgRow> = if include_messages {
            conn.prepare(
                "SELECT role, content, thinking, tool_name, tool_input, tool_output, elapsed_ms, timestamp
                 FROM session_messages WHERE session_id = ?1 ORDER BY id",
            )
            .and_then(|mut st| {
                st.query_map(params![s.session_id], |r| {
                    Ok(MsgRow {
                        role: r.get(0)?,
                        content: r.get(1)?,
                        thinking: r.get(2)?,
                        tool_name: r.get(3)?,
                        tool_input: r.get(4)?,
                        tool_output: r.get(5)?,
                        elapsed_ms: r.get(6)?,
                        timestamp: r.get(7)?,
                    })
                })
                .map(|it| it.filter_map(Result::ok).take(MAX_MESSAGES_PER_SESSION).collect())
            })
            .unwrap_or_default()
        } else {
            Vec::new()
        };
        if format == "markdown" {
            markdowns.push(markdown_for(s, &msgs));
        } else {
            sessions.push(session_json(s, &msgs));
        }
    }

    if format == "markdown" {
        return json!({
            "ok": true,
            "schemaVersion": SCHEMA_VERSION,
            "format": "markdown",
            "markdown": markdowns.join("\n\n---\n\n"),
            "sessions": rows.len(),
            "generatedAt": chrono::Utc::now().to_rfc3339(),
        });
    }
    json!({
        "ok": true,
        "schemaVersion": SCHEMA_VERSION,
        "format": "json",
        "app": "marguerite",
        "generatedAt": chrono::Utc::now().to_rfc3339(),
        "sessions": sessions,
    })
}

/// 从导入项读取项目归属：`projectPaths` 数组优先，回退旧单值 `projectPath`。
fn import_projects(obj: &serde_json::Map<String, Value>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(arr) = obj.get("projectPaths").and_then(Value::as_array) {
        for v in arr {
            let Some(p) = v.as_str().map(str::trim).filter(|p| !p.is_empty()) else {
                continue;
            };
            if !out.iter().any(|x| x == p) {
                out.push(p.to_string());
            }
        }
    }
    if out.is_empty() {
        if let Some(p) = obj
            .get("projectPath")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            out.push(p.to_string());
        }
    }
    out
}

fn new_session_id(seq: usize) -> String {
    format!(
        "imp-{}-{:02}",
        chrono::Utc::now().format("%Y%m%d%H%M%S"),
        seq
    )
}

/// `sessions.import`
///
/// `payload`：`sessions.export` 的返回体（对象或 JSON 字符串）。
/// `keepId`：true 时优先保留原 ID（冲突则改名）。
pub fn import(params: &Value) -> Value {
    let payload = match params.get("payload") {
        Some(Value::String(s)) => match serde_json::from_str::<Value>(s) {
            Ok(v) => v,
            Err(e) => return err(format!("payload 不是合法 JSON: {e}"), "INVALID_PAYLOAD"),
        },
        Some(v @ Value::Object(_)) => v.clone(),
        _ => return err("缺少 payload（导出 JSON 对象或字符串）", "INVALID_REQUEST"),
    };
    let version = payload.get("schemaVersion").and_then(Value::as_u64).unwrap_or(0);
    if version == 0 || version > SCHEMA_VERSION as u64 {
        return err(
            format!("不支持的导出版本 {version}（当前支持 ≤ {SCHEMA_VERSION}）"),
            "UNSUPPORTED_SCHEMA",
        );
    }
    let Some(list) = payload.get("sessions").and_then(Value::as_array) else {
        return err("payload.sessions 必须是数组", "INVALID_PAYLOAD");
    };
    if list.is_empty() {
        return json!({ "ok": true, "imported": [], "skipped": [], "note": "没有可导入的会话" });
    }
    let keep_id = params
        .get("keepId")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let path = db_path();
    // 复用 SessionManager 的建表/迁移逻辑，避免导入到旧 schema 上失败。
    if let Err(e) = SessionManager::new(&path).init() {
        return err(format!("准备会话库失败: {e}"), "SESSION_DB_UNAVAILABLE");
    }
    let conn = match Connection::open(&path) {
        Ok(c) => c,
        Err(e) => return err(format!("打开会话库失败: {e}"), "SESSION_DB_UNAVAILABLE"),
    };

    let mut imported: Vec<Value> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let mut seq = 0usize;
    for item in list.iter().take(MAX_IMPORT_SESSIONS) {
        let Some(obj) = item.as_object() else {
            skipped.push(json!({ "reason": "not-an-object" }));
            continue;
        };
        let title = obj
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("导入的会话");
        let original_id = obj
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let messages = obj
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        seq += 1;
        let mut target = if keep_id && !original_id.is_empty() {
            original_id.clone()
        } else {
            new_session_id(seq)
        };
        // 冲突检测（keepId 时）：改名而不是覆盖
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM sessions WHERE session_id = ?1",
                params![target],
                |_| Ok(()),
            )
            .is_ok();
        if exists {
            target = format!("{target}-{}", new_session_id(seq));
        }

        let now = chrono::Utc::now().to_rfc3339();
        let created = obj
            .get("createdAt")
            .and_then(Value::as_str)
            .unwrap_or(&now)
            .to_string();
        let kept = messages.len().min(MAX_MESSAGES_PER_SESSION);
        // 项目归属：仅当主路径在本机真实存在才恢复绑定（否则保持未绑定）。
        let projects = import_projects(obj);
        let restore = projects
            .first()
            .map(|p| Path::new(p).is_dir())
            .unwrap_or(false);
        let (bound_primary, bound_json): (Option<String>, Option<String>) = if restore {
            (
                projects.first().cloned(),
                serde_json::to_string(&projects).ok(),
            )
        } else {
            (None, None)
        };
        let tx = match conn.unchecked_transaction() {
            Ok(t) => t,
            Err(e) => {
                skipped.push(json!({ "sessionId": original_id, "reason": format!("{e}") }));
                continue;
            }
        };
        let insert = tx.execute(
            "INSERT INTO sessions (session_id, summary, state, message_count, tokens_used, created_at, updated_at, project_path, project_paths)
             VALUES (?1, ?2, 'Ready', ?3, 0, ?4, ?5, ?6, ?7)",
            params![target, title, kept as i64, created, now, bound_primary, bound_json],
        );
        if let Err(e) = insert {
            skipped.push(json!({ "sessionId": original_id, "reason": format!("{e}") }));
            continue;
        }
        let mut failed = 0usize;
        for m in messages.iter().take(MAX_MESSAGES_PER_SESSION) {
            let role = m.get("role").and_then(Value::as_str).unwrap_or("user");
            let content = m.get("content").and_then(Value::as_str).unwrap_or("");
            let ts = m
                .get("timestamp")
                .and_then(Value::as_str)
                .unwrap_or(&now)
                .to_string();
            let r = tx.execute(
                "INSERT INTO session_messages (session_id, role, content, thinking, tool_name, tool_input, tool_output, elapsed_ms, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    target,
                    role,
                    content,
                    m.get("thinking").and_then(Value::as_str),
                    m.get("toolName").and_then(Value::as_str),
                    m.get("toolInput").and_then(Value::as_str),
                    m.get("toolOutput").and_then(Value::as_str),
                    m.get("elapsedMs").and_then(Value::as_i64),
                    ts
                ],
            );
            if r.is_err() {
                failed += 1;
            }
        }
        if let Err(e) = tx.commit() {
            skipped.push(json!({ "sessionId": original_id, "reason": format!("{e}") }));
            continue;
        }
        imported.push(json!({
            "from": original_id,
            "to": target,
            "messages": kept - failed,
            "failedMessages": failed,
            "title": title,
            // 空数组 = 本机没有这些目录，未恢复项目绑定
            "boundProjects": if restore { json!(projects) } else { json!([]) },
        }));
    }

    json!({
        "ok": true,
        "schemaVersion": SCHEMA_VERSION,
        "imported": imported,
        "skipped": skipped,
        "total": imported.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_rejects_bad_payload_shapes() {
        assert_eq!(import(&json!({}))["code"], "INVALID_REQUEST");
        assert_eq!(import(&json!({ "payload": "{not json" }))["code"], "INVALID_PAYLOAD");
        assert_eq!(
            import(&json!({ "payload": { "schemaVersion": 99, "sessions": [] } }))["code"],
            "UNSUPPORTED_SCHEMA"
        );
        assert_eq!(
            import(&json!({ "payload": { "schemaVersion": 1, "sessions": {} } }))["code"],
            "INVALID_PAYLOAD"
        );
    }

    #[test]
    fn export_validates_format() {
        let v = export(&json!({ "format": "pdf" }));
        assert_eq!(v["code"], "INVALID_REQUEST");
    }

    #[test]
    fn markdown_keeps_tool_and_thinking_blocks() {
        let s = SessionRow {
            session_id: "s1".into(),
            summary: "标题".into(),
            state: "Ready".into(),
            message_count: 2,
            tokens_used: 0,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:01Z".into(),
            project_path: None,
            project_paths: None,
            reasoning_effort: None,
            context_window: None,
        };
        let msgs = vec![
            MsgRow {
                role: "user".into(),
                content: "帮我查天气".into(),
                thinking: None,
                tool_name: None,
                tool_input: None,
                tool_output: None,
                elapsed_ms: None,
                timestamp: "2026-01-01T00:00:00Z".into(),
            },
            MsgRow {
                role: "assistant".into(),
                content: "好的".into(),
                thinking: Some("需要联网".into()),
                tool_name: Some("web_search".into()),
                tool_input: Some("{\"q\":\"桂林\"}".into()),
                tool_output: Some("晴".into()),
                elapsed_ms: Some(120),
                timestamp: "2026-01-01T00:00:01Z".into(),
            },
        ];
        let md = markdown_for(&s, &msgs);
        assert!(md.contains("### 用户"));
        assert!(md.contains("思考过程"));
        assert!(md.contains("`web_search`"));
        assert!(md.contains("桂林"));
    }

    #[test]
    fn row_projects_prefers_multi_and_falls_back_to_legacy() {
        let mut s = SessionRow {
            session_id: "s1".into(),
            summary: "t".into(),
            state: "Ready".into(),
            message_count: 0,
            tokens_used: 0,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            project_path: Some("D:/only".into()),
            project_paths: None,
            reasoning_effort: None,
            context_window: None,
        };
        assert_eq!(row_projects(&s), vec!["D:/only".to_string()]);
        s.project_paths = Some(r#"["D:/a","D:/b"]"#.into());
        assert_eq!(row_projects(&s), vec!["D:/a".to_string(), "D:/b".to_string()]);
        s.project_paths = Some("not-json".into());
        assert_eq!(row_projects(&s), vec!["D:/only".to_string()]);
    }

    #[test]
    fn import_projects_reads_multi_then_legacy() {
        let multi = json!({ "projectPaths": ["D:/a", "D:/a", " ", "D:/b"] });
        assert_eq!(
            import_projects(multi.as_object().unwrap()),
            vec!["D:/a".to_string(), "D:/b".to_string()]
        );
        let legacy = json!({ "projectPath": "D:/only" });
        assert_eq!(
            import_projects(legacy.as_object().unwrap()),
            vec!["D:/only".to_string()]
        );
        assert!(import_projects(json!({}).as_object().unwrap()).is_empty());
    }

    #[test]
    fn new_session_ids_are_unique_per_sequence() {
        assert_ne!(new_session_id(1), new_session_id(2));
        assert!(new_session_id(1).starts_with("imp-"));
    }
}
