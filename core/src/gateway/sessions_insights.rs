//! 任务复盘：把 `sessions.db` 里的执行证据聚合成可读的复盘视图。
//!
//! ## 为什么需要它
//!
//! 复盘要回答的四件事——**任务类型、工具调度、智能体思路、会话粒度表现**——
//! 分散在 `session_messages` 的 `role / thinking / tool_name / elapsed_ms / tool_ok`
//! 里，没有任何聚合入口。本模块只读 `sessions.db`，不写库、不触发执行。
//!
//! ## 口径（诚实优先）
//!
//! - **任务类型**按「一轮 assistant 回复」分类：该轮有没有工具行、有没有思考正文。
//!   这是可观测的行为分类，不是对用户意图的猜测。
//! - **工具失败**只认 `tool_ok = 0`；`NULL`（旧数据/缺信号）计入 `unknown`，
//!   绝不当成成功，也绝不当成失败。
//! - **思考**只统计 assistant 行上的 `thinking` 列；网关路径在 2026-10-24 之前
//!   不写该列，因此旧会话的 `thinkingMessages` 为 0 —— 这是数据缺口，不是「没思考」。
//!   覆盖度缺口在 `coverage` 与 `gaps` 里如实列出。

use std::collections::BTreeMap;
use std::path::PathBuf;

use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};

use crate::persistence::config::Config;

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;

fn db_path() -> PathBuf {
    Config::config_dir().join("sessions.db")
}

fn err(msg: impl Into<String>, code: &str) -> Value {
    json!({ "ok": false, "error": msg.into(), "code": code })
}

#[derive(Default)]
struct ToolStat {
    calls: u64,
    failures: u64,
    unknown: u64,
    durations: Vec<u64>,
}

#[derive(Default)]
struct SessionRollup {
    key: String,
    title: String,
    messages: u64,
    user_turns: u64,
    assistant_turns: u64,
    tool_calls: u64,
    tool_failures: u64,
    thinking_messages: u64,
    thinking_chars: u64,
    tool_ms_total: u64,
    last_tool: Option<String>,
    updated_at: String,
}

/// `sessions.insights`：聚合复盘视图。
///
/// 参数：
/// - `sessionKey`（可选）：只复盘一个会话；
/// - `limit`（可选，1–200，默认 50）：参与聚合的最近会话数。
pub fn insights(params: &Value) -> Value {
    let conn = match Connection::open_with_flags(db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(c) => c,
        Err(e) => {
            return err(
                format!("无法打开 sessions.db：{e}"),
                "SESSION_DB_UNAVAILABLE",
            )
        }
    };
    insights_with(&conn, params)
}

fn insights_with(conn: &Connection, params: &Value) -> Value {
    let only_session = params
        .get("sessionKey")
        .or_else(|| params.get("session"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_LIMIT);

    // 1) 选出参与复盘的会话（最近更新优先）。
    let sessions: Vec<(String, String, String)> = if let Some(key) = only_session {
        let mut stmt = match conn.prepare(
            "SELECT session_id, summary, updated_at FROM sessions WHERE session_id = ?1",
        ) {
            Ok(s) => s,
            Err(e) => return err(format!("读取会话失败：{e}"), "SESSION_QUERY_FAILED"),
        };
        let rows = stmt.query_map([key], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        });
        match rows {
            Ok(rows) => rows.filter_map(Result::ok).collect(),
            Err(e) => return err(format!("读取会话失败：{e}"), "SESSION_QUERY_FAILED"),
        }
    } else {
        let mut stmt = match conn.prepare(
            "SELECT session_id, summary, updated_at FROM sessions ORDER BY updated_at DESC LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(e) => return err(format!("读取会话失败：{e}"), "SESSION_QUERY_FAILED"),
        };
        let rows = stmt.query_map([limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        });
        match rows {
            Ok(rows) => rows.filter_map(Result::ok).collect(),
            Err(e) => return err(format!("读取会话失败：{e}"), "SESSION_QUERY_FAILED"),
        }
    };

    let mut totals = Totals::default();
    let mut task_types = TaskTypes::default();
    let mut tools: BTreeMap<String, ToolStat> = BTreeMap::new();
    let mut rollups: Vec<Value> = Vec::new();
    let mut coverage = Coverage::default();

    for (key, title, updated_at) in &sessions {
        let mut roll = SessionRollup {
            key: key.clone(),
            title: title.clone(),
            updated_at: updated_at.clone(),
            ..SessionRollup::default()
        };
        let mut stmt = match conn.prepare(
            "SELECT role, thinking, tool_name, elapsed_ms, tool_ok
             FROM session_messages WHERE session_id = ?1 ORDER BY id ASC",
        ) {
            Ok(s) => s,
            Err(e) => return err(format!("读取消息失败：{e}"), "SESSION_QUERY_FAILED"),
        };
        let rows = stmt.query_map([key.as_str()], |row| {
            Ok(MessageRow {
                role: row.get::<_, String>(0)?,
                thinking: row.get::<_, Option<String>>(1)?,
                tool_name: row.get::<_, Option<String>>(2)?,
                elapsed_ms: row.get::<_, Option<i64>>(3)?,
                tool_ok: row.get::<_, Option<i64>>(4)?,
            })
        });
        let rows: Vec<MessageRow> = match rows {
            Ok(rows) => rows.filter_map(Result::ok).collect(),
            Err(e) => return err(format!("读取消息失败：{e}"), "SESSION_QUERY_FAILED"),
        };

        let mut pending_tools = 0u64;
        let mut saw_any_message = false;
        for row in &rows {
            saw_any_message = true;
            roll.messages += 1;
            totals.messages += 1;
            match row.role.as_str() {
                "user" => {
                    roll.user_turns += 1;
                    totals.user_turns += 1;
                    pending_tools = 0;
                }
                "tool" => {
                    let name = row.tool_name.clone().unwrap_or_else(|| "unknown".to_string());
                    pending_tools += 1;
                    roll.tool_calls += 1;
                    totals.tool_calls += 1;
                    roll.last_tool = Some(name.clone());
                    let entry = tools.entry(name).or_default();
                    entry.calls += 1;
                    match row.tool_ok {
                        Some(0) => {
                            entry.failures += 1;
                            roll.tool_failures += 1;
                            totals.tool_failures += 1;
                        }
                        Some(_) => {}
                        None => {
                            entry.unknown += 1;
                            totals.tool_unknown += 1;
                        }
                    }
                    if let Some(ms) = row.elapsed_ms.filter(|v| *v >= 0) {
                        let ms = ms as u64;
                        entry.durations.push(ms);
                        roll.tool_ms_total += ms;
                        totals.tool_ms_total += ms;
                    }
                }
                "assistant" => {
                    roll.assistant_turns += 1;
                    totals.assistant_turns += 1;
                    let thinking_chars = row
                        .thinking
                        .as_deref()
                        .map(|t| t.trim().chars().count())
                        .unwrap_or(0);
                    if thinking_chars > 0 {
                        roll.thinking_messages += 1;
                        roll.thinking_chars += thinking_chars as u64;
                        totals.thinking_messages += 1;
                        totals.thinking_chars += thinking_chars as u64;
                    }
                    match (pending_tools > 0, thinking_chars > 0) {
                        (true, true) => task_types.tool_and_reasoning += 1,
                        (true, false) => task_types.tool_assisted += 1,
                        (false, true) => task_types.reasoning += 1,
                        (false, false) => task_types.direct_answer += 1,
                    }
                    pending_tools = 0;
                }
                _ => {}
            }
        }

        if !saw_any_message {
            coverage.sessions_without_messages += 1;
        }
        if roll.tool_calls > 0 {
            coverage.sessions_with_tool_rows += 1;
        }
        if roll.thinking_messages > 0 {
            coverage.sessions_with_thinking += 1;
        }
        if roll.assistant_turns == 0 {
            coverage.sessions_without_assistant += 1;
        }

        rollups.push(json!({
            "key": roll.key,
            "title": roll.title,
            "messages": roll.messages,
            "userTurns": roll.user_turns,
            "assistantTurns": roll.assistant_turns,
            "toolCalls": roll.tool_calls,
            "toolFailures": roll.tool_failures,
            "thinkingMessages": roll.thinking_messages,
            "thinkingChars": roll.thinking_chars,
            "toolMs": roll.tool_ms_total,
            "lastTool": roll.last_tool,
            "updatedAt": roll.updated_at,
        }));
    }

    // 2) 工具聚合：次数 / 失败 / 平均 / p95（按毫秒）。
    let tool_rows: Vec<Value> = tools
        .iter()
        .map(|(name, stat)| {
            let mut durations = stat.durations.clone();
            durations.sort_unstable();
            let avg = if durations.is_empty() {
                Value::Null
            } else {
                json!(durations.iter().sum::<u64>() / durations.len() as u64)
            };
            let p95 = if durations.is_empty() {
                Value::Null
            } else {
                let idx = ((durations.len() as f64 - 1.0) * 0.95).round() as usize;
                json!(durations[idx.min(durations.len() - 1)])
            };
            json!({
                "name": name,
                "calls": stat.calls,
                "failures": stat.failures,
                "unknown": stat.unknown,
                "avgMs": avg,
                "p95Ms": p95,
            })
        })
        .collect();

    let mut gaps: Vec<&str> = Vec::new();
    if totals.tool_calls == 0 {
        gaps.push("所选范围内没有任何工具调用记录（可能都是纯对话，或会话早于工具落盘上线）");
    }
    if totals.tool_unknown > 0 {
        gaps.push("部分工具结果缺少成功/失败标记（旧数据），已计入 unknown，不臆测成败");
    }
    if totals.tool_calls > 0 && totals.thinking_messages == 0 {
        gaps.push("有工具调用但没有任何思考正文：会话可能未开启思考，或早于思考落盘上线");
    }
    if coverage.sessions_without_assistant > 0 {
        gaps.push("部分会话没有 assistant 回复（可能仍在运行或已中断）");
    }

    json!({
        "ok": true,
        "generatedAt": chrono::Utc::now().to_rfc3339(),
        "source": {
            "db": db_path().to_string_lossy(),
            "sessionsScanned": sessions.len(),
            "limit": limit,
            "sessionKey": only_session,
        },
        "totals": {
            "sessions": sessions.len(),
            "messages": totals.messages,
            "userTurns": totals.user_turns,
            "assistantTurns": totals.assistant_turns,
            "toolCalls": totals.tool_calls,
            "toolFailures": totals.tool_failures,
            "toolUnknown": totals.tool_unknown,
            "toolMs": totals.tool_ms_total,
            "thinkingMessages": totals.thinking_messages,
            "thinkingChars": totals.thinking_chars,
        },
        "taskTypes": {
            "directAnswer": task_types.direct_answer,
            "reasoning": task_types.reasoning,
            "toolAssisted": task_types.tool_assisted,
            "toolAndReasoning": task_types.tool_and_reasoning,
        },
        "tools": tool_rows,
        "sessions": rollups,
        "coverage": {
            "sessionsWithToolRows": coverage.sessions_with_tool_rows,
            "sessionsWithThinking": coverage.sessions_with_thinking,
            "sessionsWithoutAssistant": coverage.sessions_without_assistant,
            "sessionsWithoutMessages": coverage.sessions_without_messages,
        },
        "gaps": gaps,
    })
}

struct MessageRow {
    role: String,
    thinking: Option<String>,
    tool_name: Option<String>,
    elapsed_ms: Option<i64>,
    tool_ok: Option<i64>,
}

#[derive(Default)]
struct Totals {
    messages: u64,
    user_turns: u64,
    assistant_turns: u64,
    tool_calls: u64,
    tool_failures: u64,
    tool_unknown: u64,
    tool_ms_total: u64,
    thinking_messages: u64,
    thinking_chars: u64,
}

#[derive(Default)]
struct TaskTypes {
    direct_answer: u64,
    reasoning: u64,
    tool_assisted: u64,
    tool_and_reasoning: u64,
}

#[derive(Default)]
struct Coverage {
    sessions_with_tool_rows: u64,
    sessions_with_thinking: u64,
    sessions_without_assistant: u64,
    sessions_without_messages: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::session::SessionManager;

    fn temp_db() -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("marg_insights_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        (dir.clone(), dir.join("sessions.db"))
    }

    /// 四类任务形态各构造一轮，验证分类口径。
    #[test]
    fn classifies_assistant_turns_by_tools_and_thinking() {
        let (dir, path) = temp_db();
        let mut mgr = SessionManager::new(&path);
        mgr.init().unwrap();
        // 轮 1：直答
        mgr.append_message("s1", "user", "你好", "2026-10-24T10:00:00Z", Some("你好")).unwrap();
        mgr.append_message_with("s1", "assistant", "你好呀", "2026-10-24T10:00:01Z", None, None)
            .unwrap();
        // 轮 2：只有思考
        mgr.append_message("s1", "user", "解释一下", "2026-10-24T10:00:02Z", None).unwrap();
        mgr.append_message_with("s1", "assistant", "解释", "2026-10-24T10:00:03Z", None, Some("想了想"))
            .unwrap();
        // 轮 3：调了工具
        mgr.append_message("s1", "user", "读文件", "2026-10-24T10:00:04Z", None).unwrap();
        mgr.append_tool_message("s1", "read_file", "a.txt", "内容", Some(10), Some(true), "2026-10-24T10:00:05Z")
            .unwrap();
        mgr.append_message_with("s1", "assistant", "读到了", "2026-10-24T10:00:06Z", None, None)
            .unwrap();
        // 轮 4：工具 + 思考
        mgr.append_message("s1", "user", "修一下", "2026-10-24T10:00:07Z", None).unwrap();
        mgr.append_tool_message("s1", "str_replace_editor", "x", "ok", Some(20), Some(false), "2026-10-24T10:00:08Z")
            .unwrap();
        mgr.append_message_with("s1", "assistant", "改好了", "2026-10-24T10:00:09Z", None, Some("定位后替换"))
            .unwrap();

        let conn = Connection::open(&path).unwrap();
        let out = insights_with(&conn, &json!({}));
        assert_eq!(out["ok"], true);
        assert_eq!(out["taskTypes"]["directAnswer"], 1);
        assert_eq!(out["taskTypes"]["reasoning"], 1);
        assert_eq!(out["taskTypes"]["toolAssisted"], 1);
        assert_eq!(out["taskTypes"]["toolAndReasoning"], 1);
        assert_eq!(out["totals"]["toolCalls"], 2);
        assert_eq!(out["totals"]["toolFailures"], 1);
        assert_eq!(out["totals"]["thinkingMessages"], 2);
        assert_eq!(out["coverage"]["sessionsWithToolRows"], 1);
        assert_eq!(out["coverage"]["sessionsWithThinking"], 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 工具耗时聚合：平均与 p95 都按真实记录算，无耗时则返回 null。
    #[test]
    fn aggregates_tool_durations_and_p95() {
        let (dir, path) = temp_db();
        let mut mgr = SessionManager::new(&path);
        mgr.init().unwrap();
        mgr.append_message("s2", "user", "跑一下", "2026-10-24T10:00:00Z", Some("跑一下")).unwrap();
        for (i, ms) in [10u64, 20, 30, 40, 500].iter().enumerate() {
            mgr.append_tool_message(
                "s2",
                "shell_exec",
                "ls",
                "ok",
                Some(*ms),
                Some(true),
                &format!("2026-10-24T10:00:0{}Z", i + 1),
            )
            .unwrap();
        }
        mgr.append_message_with("s2", "assistant", "完成", "2026-10-24T10:00:09Z", None, None)
            .unwrap();

        let conn = Connection::open(&path).unwrap();
        let out = insights_with(&conn, &json!({}));
        let tool = &out["tools"][0];
        assert_eq!(tool["name"], "shell_exec");
        assert_eq!(tool["calls"], 5);
        assert_eq!(tool["avgMs"], 120, "平均应为 (10+20+30+40+500)/5 = 120");
        assert_eq!(tool["p95Ms"], 500, "p95 取排序后 95% 分位");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 旧数据：工具行没有 ok 标记 → 计入 unknown，不臆测成功/失败。
    #[test]
    fn missing_ok_marker_is_reported_as_unknown() {
        let (dir, path) = temp_db();
        let mut mgr = SessionManager::new(&path);
        mgr.init().unwrap();
        mgr.append_tool_message("s3", "browser", "open", "done", None, None, "2026-10-24T10:00:00Z")
            .unwrap();
        let conn = Connection::open(&path).unwrap();
        let out = insights_with(&conn, &json!({}));
        assert_eq!(out["totals"]["toolFailures"], 0);
        assert_eq!(out["totals"]["toolUnknown"], 1);
        assert_eq!(out["tools"][0]["unknown"], 1);
        let gaps = out["gaps"].as_array().unwrap();
        assert!(gaps.iter().any(|g| g.as_str().unwrap_or_default().contains("unknown")));
        let _ = std::fs::remove_dir_all(dir);
    }
}
