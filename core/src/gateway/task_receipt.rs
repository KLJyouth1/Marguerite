//! 交付回执（`task.receipt`）——「改了什么 / 产出了什么 / 验证了什么 / 花了多少」。
//!
//! ## 为什么需要它
//!
//! 此前一回合结束后，用户能看到的只有一段自然语言回复：改了哪些文件要翻「文件更改卡片」、
//! 产物要翻抽屉、工具与耗时散在复盘页、验证到底跑没跑完全没有记录。回执把这些证据
//! **按同一回合聚合成一条**，并显式区分「已验证 / 未验证」——不允许用模型自述冒充验证。
//!
//! ## 数据源（全部是既有真实台账，不新增旁路写入）
//!
//! - `session_messages`：用户/助手轮次、工具名与入参、耗时、成败（`tool_ok`）；
//! - `tools::file_change`：本会话真实落盘的文件改动（含会话键）；
//! - `tools::file_change` 产物台账：本会话登记的产物，并**回查磁盘**是否仍在；
//! - `harness::ledger`：本会话 run 的 token / 模型 / 策略（台账关闭时如实置空）。
//!
//! ## 诚实边界
//!
//! - 「验证通过」只在**确有验证命令记录且全部成功**时给出；没有验证记录就是
//!   `unverified`，绝不因为「模型说跑过了」而变成 `passed`；
//! - `tool_ok` 为 NULL 的旧数据计入 `unknown`，既不算成功也不算失败；
//! - 回执按**时间窗**（本回合起始用户消息 → 本条助手消息）裁剪，窗口缺失时退化为
//!   整个会话，并在 `scope.degraded` 里如实说明。

use std::path::PathBuf;

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};

use crate::persistence::config::Config;

/// 单次回执最多列出的文件改动 / 产物条数。
const MAX_ITEMS: usize = 200;
/// 单次回执最多列出的验证命令条数。
const MAX_CHECKS: usize = 50;
/// 验证命令展示长度上限（入参在库里已被截断，这里再收一次，避免卡片被长命令撑爆）。
const MAX_COMMAND_CHARS: usize = 160;

/// 具备「可执行验证」语义的工具：只有这些工具跑的东西才算机器验证。
const VERIFY_TOOLS: &[&str] = &["shell_exec", "shell", "code_exec", "run_command", "terminal"];

/// 验证意图关键词（对工具入参做大小写不敏感匹配）。
///
/// 刻意保守：只认构建/测试/类型检查/静态检查这几类**会返回退出码**的动作。
/// `git status`、`dir`、`echo` 这类不构成验证，不能算进「已验证」。
const VERIFY_HINTS: &[&str] = &[
    "cargo test",
    "cargo build",
    "cargo check",
    "cargo clippy",
    "npm test",
    "npm run test",
    "npm run build",
    "npm run typecheck",
    "npx vitest",
    "vitest",
    "jest",
    "pytest",
    "python -m pytest",
    "python3 -m pytest",
    "go test",
    "go build",
    "dotnet test",
    "mvn test",
    "gradle test",
    "tsc",
    "eslint",
    "ruff check",
    "mypy",
];

fn db_path() -> PathBuf {
    Config::config_dir().join("sessions.db")
}

fn err(msg: impl Into<String>, code: &str) -> Value {
    json!({ "ok": false, "error": msg.into(), "code": code })
}

/// 时间窗（闭开区间：`since <= t <= until`）。
#[derive(Debug, Clone, Default)]
struct Window {
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
}

impl Window {
    fn contains(&self, ts: DateTime<Utc>) -> bool {
        if let Some(s) = self.since {
            if ts < s {
                return false;
            }
        }
        if let Some(u) = self.until {
            if ts > u {
                return false;
            }
        }
        true
    }

    fn is_open(&self) -> bool {
        self.since.is_none() && self.until.is_none()
    }
}

/// 解析时间边界：接受 RFC3339 字符串或数字（≥1e12 视为毫秒，否则秒）。
fn parse_bound(raw: Option<&Value>) -> Option<DateTime<Utc>> {
    let v = raw?;
    if let Some(s) = v.as_str() {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            return Some(dt.with_timezone(&Utc));
        }
        if let Ok(epoch) = s.parse::<i64>() {
            return from_epoch(epoch);
        }
        return None;
    }
    from_epoch(v.as_i64()?)
}

fn from_epoch(n: i64) -> Option<DateTime<Utc>> {
    if n > 1_000_000_000_000 {
        Utc.timestamp_millis_opt(n).single()
    } else {
        Utc.timestamp_opt(n, 0).single()
    }
}

/// `task.receipt`：聚合一回合（或整个会话）的交付证据。
///
/// 参数：
/// - `sessionKey`（必填）：会话 key；
/// - `since` / `until`（可选）：RFC3339 或 epoch；缺省为整个会话；
/// - `limit`（可选，1–200，默认 50）：改动 / 产物的展示上限。
pub fn task_receipt(params: &Value) -> Value {
    let session = params
        .get("sessionKey")
        .or_else(|| params.get("session_key"))
        .or_else(|| params.get("session"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let Some(session) = session else {
        return err("缺少 sessionKey", "INVALID_REQUEST");
    };
    let window = Window {
        since: parse_bound(params.get("since")),
        until: parse_bound(params.get("until")),
    };
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map(|v| (v as usize).clamp(1, MAX_ITEMS))
        .unwrap_or(50);

    // sessions.db 打不开不是致命错误：文件改动 / 产物 / 台账仍可给出，
    // 只是「会话元信息 + 工具行」缺失，在 gaps 里如实说明。
    let conn = Connection::open_with_flags(db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY).ok();
    receipt_with(conn.as_ref(), session, &window, limit)
}

/// 可测形态：连接可注入（`None` 表示 sessions.db 不可用）。
fn receipt_with(conn: Option<&Connection>, session: &str, window: &Window, limit: usize) -> Value {
    let mut gaps: Vec<Value> = Vec::new();

    // ── 1) 会话元信息 ──────────────────────────────────────────────────────
    let mut session_meta = json!({ "key": session });
    let mut user_turns = 0u64;
    let mut assistant_turns = 0u64;
    let mut thinking_chars = 0u64;
    let mut tool_calls = 0u64;
    let mut tool_failures = 0u64;
    let mut tool_unknown = 0u64;
    let mut tool_ms = 0u64;
    let mut checks: Vec<Value> = Vec::new();

    match conn {
        None => gaps.push(json!({
            "code": "session_db_unavailable",
            "detail": format!("无法以只读方式打开 {}", db_path().display()),
        })),
        Some(c) => {
            if let Ok(meta) = session_row(c, session) {
                session_meta = meta;
            }
            match message_rows(c, session) {
                Err(e) => gaps.push(json!({
                    "code": "history_unreadable",
                    "detail": format!("读取会话消息失败：{e}"),
                })),
                Ok(rows) => {
                    for row in rows {
                        if !in_window(&row.timestamp, window) {
                            continue;
                        }
                        match row.role.as_str() {
                            "user" => user_turns += 1,
                            "assistant" => {
                                assistant_turns += 1;
                                if let Some(t) = row.thinking.as_deref() {
                                    thinking_chars += t.trim().chars().count() as u64;
                                }
                            }
                            "tool" => {
                                tool_calls += 1;
                                match row.tool_ok {
                                    Some(0) => tool_failures += 1,
                                    Some(_) => {}
                                    None => tool_unknown += 1,
                                }
                                if let Some(ms) = row.elapsed_ms.filter(|v| *v >= 0) {
                                    tool_ms += ms as u64;
                                }
                                if checks.len() < MAX_CHECKS {
                                    if let Some(check) = verification_check(&row) {
                                        checks.push(check);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    // ── 2) 文件改动（按会话精确归因，再按时间窗裁剪）─────────────────────
    let changes: Vec<Value> = crate::tools::file_change::list_file_changes_for(session, MAX_ITEMS)
        .into_iter()
        .filter(|c| window.contains(Utc.timestamp_opt(c.ts, 0).single().unwrap_or_else(Utc::now)))
        .take(limit)
        .map(|c| {
            json!({
                "path": c.path,
                "kind": c.kind,
                "tool": c.tool,
                "ts": c.ts,
                "beforeChars": c.before.chars().count(),
                "afterChars": c.after.chars().count(),
            })
        })
        .collect();

    // ── 3) 产物（并回查磁盘是否还在——不假设「登记过就等于还在」）──────────
    let (artifact_rows, _has_more, _) =
        crate::tools::file_change::list_artifacts(session, None, MAX_ITEMS);
    let mut present = 0u64;
    let mut missing_paths: Vec<String> = Vec::new();
    let artifacts: Vec<Value> = artifact_rows
        .into_iter()
        .filter(|a| window.contains(Utc.timestamp_opt(a.ts, 0).single().unwrap_or_else(Utc::now)))
        .take(limit)
        .map(|a| {
            let on_disk = std::path::Path::new(&a.path).is_file();
            if on_disk {
                present += 1;
            } else {
                missing_paths.push(a.path.clone());
            }
            json!({
                "id": a.id,
                "name": a.name,
                "path": a.path,
                "kind": a.kind,
                "size": a.size,
                "ts": a.ts,
                "present": on_disk,
            })
        })
        .collect();

    // ── 4) 用量（harness 台账；关闭时不编造）──────────────────────────────
    let (ledger, ledger_available) = ledger_budget(session, window);
    if !ledger_available {
        gaps.push(json!({
            "code": "usage_ledger_unavailable",
            "detail": "harness 台账不可用，本回合 token / 模型无法归因",
        }));
    }

    // ── 5) 验证状态（证据优先：没有命令记录就是 unverified）────────────────
    let (status, verified_count) = verification_status(&checks);
    if checks.is_empty() {
        gaps.push(json!({
            "code": "no_verification",
            "detail": "本回合没有可识别的验证命令（构建 / 测试 / 类型检查），产物与改动未经机器验证",
        }));
    }
    if tool_unknown > 0 {
        gaps.push(json!({
            "code": "tool_result_unknown",
            "count": tool_unknown,
            "detail": "部分工具结果缺少成功/失败标记（旧数据），已计入 unknown，不臆测成败",
        }));
    }
    if !changes.is_empty() && checks.is_empty() {
        gaps.push(json!({
            "code": "changes_without_verification",
            "count": changes.len(),
            "detail": "改了文件但没有验证命令，改动是否可用需人工确认",
        }));
    }
    if !missing_paths.is_empty() {
        gaps.push(json!({
            "code": "artifact_missing_on_disk",
            "count": missing_paths.len(),
            "paths": missing_paths,
            "detail": "产物台账里有登记，但文件已不在磁盘（被移动 / 删除 / 由外部清理）",
        }));
    }
    if changes.is_empty() && artifacts.is_empty() && tool_calls == 0 {
        gaps.push(json!({
            "code": "no_deliverables",
            "detail": "本回合没有文件改动、产物或工具调用（纯对话）",
        }));
    }

    json!({
        "ok": true,
        "generatedAt": Utc::now().to_rfc3339(),
        "session": session_meta,
        "scope": {
            "since": window.since.map(|t| t.to_rfc3339()),
            "until": window.until.map(|t| t.to_rfc3339()),
            "degraded": window.is_open(),
            "note": if window.is_open() {
                "未提供时间窗：按整个会话聚合"
            } else {
                "按本回合时间窗聚合"
            },
        },
        "counts": {
            "userTurns": user_turns,
            "assistantTurns": assistant_turns,
            "toolCalls": tool_calls,
            "toolFailures": tool_failures,
            "toolUnknown": tool_unknown,
            "thinkingChars": thinking_chars,
            "toolMs": tool_ms,
            "changes": changes.len(),
            "artifacts": artifacts.len(),
            "artifactsPresent": present,
        },
        "changes": changes,
        "artifacts": artifacts,
        "verification": {
            "status": status,
            "passedChecks": verified_count,
            "checks": checks,
        },
        "budget": ledger,
        "unattested": gaps,
    })
}

/// 会话元信息（摘要 / 状态 / 项目根 / 创建与更新时间）。
fn session_row(conn: &Connection, session: &str) -> rusqlite::Result<Value> {
    let mut stmt = conn.prepare(
        "SELECT session_id, summary, state, project_path, created_at, updated_at
         FROM sessions WHERE session_id = ?1",
    )?;
    let row = stmt.query_row([session], |r| {
        Ok(json!({
            "key": r.get::<_, String>(0)?,
            "title": r.get::<_, String>(1)?,
            "state": r.get::<_, String>(2)?,
            "projectPath": r.get::<_, Option<String>>(3)?,
            "createdAt": r.get::<_, String>(4)?,
            "updatedAt": r.get::<_, String>(5)?,
        }))
    });
    match row {
        Ok(v) => Ok(v),
        // 会话行缺失（新会话尚未落盘）不是错误：返回只有 key 的最小元信息。
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(json!({ "key": session })),
        Err(e) => Err(e),
    }
}

struct MsgRow {
    role: String,
    thinking: Option<String>,
    tool_name: Option<String>,
    tool_input: Option<String>,
    elapsed_ms: Option<i64>,
    tool_ok: Option<i64>,
    timestamp: String,
}

fn message_rows(conn: &Connection, session: &str) -> rusqlite::Result<Vec<MsgRow>> {
    let mut stmt = conn.prepare(
        "SELECT role, thinking, tool_name, tool_input, elapsed_ms, tool_ok, timestamp
         FROM session_messages WHERE session_id = ?1 ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([session], |r| {
        Ok(MsgRow {
            role: r.get::<_, String>(0)?,
            thinking: r.get::<_, Option<String>>(1)?,
            tool_name: r.get::<_, Option<String>>(2)?,
            tool_input: r.get::<_, Option<String>>(3)?,
            elapsed_ms: r.get::<_, Option<i64>>(4)?,
            tool_ok: r.get::<_, Option<i64>>(5)?,
            timestamp: r.get::<_, String>(6)?,
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// 时间戳不可解析时**保留**该行：丢掉证据比留一条不精确的证据更危险。
fn in_window(timestamp: &str, window: &Window) -> bool {
    match DateTime::parse_from_rfc3339(timestamp.trim()) {
        Ok(dt) => window.contains(dt.with_timezone(&Utc)),
        Err(_) => true,
    }
}

/// 一条工具行是否构成「验证命令」；构成则返回可展示的检查项。
fn verification_check(row: &MsgRow) -> Option<Value> {
    let tool = row.tool_name.as_deref().unwrap_or("");
    if !VERIFY_TOOLS.iter().any(|t| t.eq_ignore_ascii_case(tool)) {
        return None;
    }
    let input = row.tool_input.as_deref().unwrap_or("").trim();
    if input.is_empty() {
        return None;
    }
    let lower = input.to_lowercase();
    if !VERIFY_HINTS.iter().any(|h| lower.contains(h)) {
        return None;
    }
    Some(json!({
        "tool": tool,
        "command": truncate_chars(input, MAX_COMMAND_CHARS),
        "ok": match row.tool_ok {
            Some(0) => Some(false),
            Some(_) => Some(true),
            None => None,
        },
        "elapsedMs": row.elapsed_ms.filter(|v| *v >= 0).unwrap_or(0),
    }))
}

/// 验证裁决：`passed` 只在「有检查且全部明确成功」时给出。
fn verification_status(checks: &[Value]) -> (&'static str, u64) {
    if checks.is_empty() {
        return ("unverified", 0);
    }
    let mut passed = 0u64;
    let mut failed = false;
    let mut unknown = false;
    for c in checks {
        match c.get("ok") {
            Some(Value::Bool(true)) => passed += 1,
            Some(Value::Bool(false)) => failed = true,
            _ => unknown = true,
        }
    }
    if failed {
        ("failed", passed)
    } else if unknown {
        // 有检查但成败不明：不能宣称通过。
        ("unknown", passed)
    } else {
        ("passed", passed)
    }
}

/// 从 harness 台账取本会话（及时间窗内）的 token / 模型 / 策略。
///
/// 返回 `(json, 台账是否可用)`；台账关闭时返回全 null，绝不编造用量。
fn ledger_budget(session: &str, window: &Window) -> (Value, bool) {
    let Some(l) = crate::harness::ledger::global() else {
        return (
            json!({
                "available": false,
                "runs": 0,
                "tokensIn": Value::Null,
                "tokensOut": Value::Null,
                "model": Value::Null,
                "policy": Value::Null,
                "runsElapsedMs": Value::Null,
            }),
            false,
        );
    };
    let Ok((runs, _truncated)) = l.query(Some(session), None, 50) else {
        return (
            json!({
                "available": false,
                "runs": 0,
                "tokensIn": Value::Null,
                "tokensOut": Value::Null,
                "model": Value::Null,
                "policy": Value::Null,
                "runsElapsedMs": Value::Null,
            }),
            false,
        );
    };
    let mut tokens_in = 0i64;
    let mut tokens_out = 0i64;
    let mut elapsed = 0i64;
    let mut count = 0u64;
    let mut last_model: Option<String> = None;
    let mut last_policy: Option<String> = None;
    for r in &runs {
        let ts = Utc
            .timestamp_millis_opt(r.started_at_ms)
            .single()
            .unwrap_or_else(Utc::now);
        if !window.contains(ts) {
            continue;
        }
        count += 1;
        tokens_in += r.prompt_tokens;
        tokens_out += r.output_tokens;
        elapsed += r.elapsed_ms.unwrap_or(0);
        if last_model.is_none() && !r.model.trim().is_empty() {
            last_model = Some(r.model.clone());
        }
        if last_policy.is_none() && !r.policy.trim().is_empty() {
            last_policy = Some(r.policy.clone());
        }
    }
    (
        json!({
            "available": true,
            "runs": count,
            "tokensIn": tokens_in,
            "tokensOut": tokens_out,
            "model": last_model,
            "policy": last_policy,
            "runsElapsedMs": elapsed,
        }),
        true,
    )
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::session::SessionManager;

    fn temp_db() -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("marg_receipt_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        (dir.clone(), dir.join("sessions.db"))
    }

    /// 只有工具行、没有验证命令 → `unverified`，且 gaps 明确说明「未验证」。
    #[test]
    fn receipt_without_verification_is_unverified() {
        let (dir, path) = temp_db();
        let mut mgr = SessionManager::new(&path);
        mgr.init().unwrap();
        let sk = format!("rcpt-unverified-{}", uuid::Uuid::new_v4());
        mgr.append_message(&sk, "user", "改一下", "2026-10-25T10:00:00Z", None).unwrap();
        mgr.append_tool_message(&sk, "read_file", "a.txt", "ok", Some(12), Some(true), "2026-10-25T10:00:01Z")
            .unwrap();
        mgr.append_message_with(&sk, "assistant", "改好了", "2026-10-25T10:00:02Z", None, None)
            .unwrap();

        let conn = Connection::open(&path).unwrap();
        let out = receipt_with(Some(&conn), &sk, &Window::default(), 50);
        assert_eq!(out["ok"], true);
        assert_eq!(out["verification"]["status"], "unverified");
        assert_eq!(out["counts"]["toolCalls"], 1);
        assert_eq!(out["counts"]["assistantTurns"], 1);
        let gaps = out["unattested"].as_array().unwrap();
        assert!(gaps.iter().any(|g| g["code"] == "no_verification"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 有验证命令且全部成功 → `passed`；任一失败 → `failed`。
    #[test]
    fn verification_status_follows_recorded_exit_marks() {
        let (dir, path) = temp_db();
        let mut mgr = SessionManager::new(&path);
        mgr.init().unwrap();
        let ok_sk = format!("rcpt-pass-{}", uuid::Uuid::new_v4());
        mgr.append_tool_message(&ok_sk, "shell_exec", "cargo test", "ok", Some(900), Some(true), "2026-10-25T10:00:00Z")
            .unwrap();
        let fail_sk = format!("rcpt-fail-{}", uuid::Uuid::new_v4());
        mgr.append_tool_message(&fail_sk, "shell_exec", "cargo test", "boom", Some(900), Some(false), "2026-10-25T10:00:00Z")
            .unwrap();

        let conn = Connection::open(&path).unwrap();
        let pass = receipt_with(Some(&conn), &ok_sk, &Window::default(), 50);
        assert_eq!(pass["verification"]["status"], "passed");
        assert_eq!(pass["verification"]["checks"][0]["command"], "cargo test");

        let fail = receipt_with(Some(&conn), &fail_sk, &Window::default(), 50);
        assert_eq!(fail["verification"]["status"], "failed");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 非验证类命令（`dir` / `git status`）不得被算成验证证据。
    #[test]
    fn non_verification_shell_commands_are_ignored() {
        let (dir, path) = temp_db();
        let mut mgr = SessionManager::new(&path);
        mgr.init().unwrap();
        let sk = format!("rcpt-nonverify-{}", uuid::Uuid::new_v4());
        mgr.append_tool_message(&sk, "shell_exec", "dir", "files", Some(5), Some(true), "2026-10-25T10:00:00Z")
            .unwrap();
        let conn = Connection::open(&path).unwrap();
        let out = receipt_with(Some(&conn), &sk, &Window::default(), 50);
        assert_eq!(out["verification"]["status"], "unverified");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 时间窗之外的证据不得计入本回合回执。
    #[test]
    fn window_excludes_earlier_turns() {
        let (dir, path) = temp_db();
        let mut mgr = SessionManager::new(&path);
        mgr.init().unwrap();
        let sk = format!("rcpt-window-{}", uuid::Uuid::new_v4());
        mgr.append_tool_message(&sk, "shell_exec", "cargo test", "ok", Some(900), Some(true), "2026-10-25T09:00:00Z")
            .unwrap();
        mgr.append_tool_message(&sk, "shell_exec", "dir", "x", Some(5), Some(true), "2026-10-25T11:00:00Z")
            .unwrap();
        let conn = Connection::open(&path).unwrap();
        let window = Window {
            since: parse_bound(Some(&json!("2026-10-25T10:00:00Z"))),
            until: None,
        };
        let out = receipt_with(Some(&conn), &sk, &window, 50);
        assert_eq!(out["counts"]["toolCalls"], 1, "只有 11:00 那条落在窗内");
        assert_eq!(out["verification"]["status"], "unverified");
        assert_eq!(out["scope"]["degraded"], false);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 文件改动按会话归因：其它会话的改动绝不出现在回执里。
    #[test]
    fn changes_are_scoped_to_session() {
        let (dir, path) = temp_db();
        let mut mgr = SessionManager::new(&path);
        mgr.init().unwrap();
        let sk = format!("rcpt-changes-{}", uuid::Uuid::new_v4());
        mgr.append_message(&sk, "user", "写文件", "2026-10-25T10:00:00Z", None).unwrap();
        crate::tools::file_change::record_file_change_for(&sk, "mine.txt", "create", "", "hi", "write_file");
        crate::tools::file_change::record_file_change_for("someone-else", "theirs.txt", "create", "", "hi", "write_file");

        let conn = Connection::open(&path).unwrap();
        let out = receipt_with(Some(&conn), &sk, &Window::default(), 50);
        let paths: Vec<&str> = out["changes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["path"].as_str().unwrap_or_default())
            .collect();
        assert!(paths.contains(&"mine.txt"), "{paths:?}");
        assert!(!paths.contains(&"theirs.txt"), "其它会话的改动不得混入: {paths:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 缺 sessionKey 必须在触碰任何数据源之前拒绝。
    #[test]
    fn missing_session_key_is_invalid_request() {
        let out = task_receipt(&json!({}));
        assert_eq!(out["ok"], false);
        assert_eq!(out["code"], "INVALID_REQUEST");
    }
}
