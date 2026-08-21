//! Token 用量台账：真实记录每次 LLM 调用（prompt/completion tokens），
//! 供 `usage.status` / `usage.query` 聚合返回，替换原先硬编码全零的占位实现。
//!
//! 设计要点：
//! - 进程级单例 [`LEDGER`]；网关托管时由 chat 运行时首次触发 [`init`]，CLI / 测试
//!   默认不初始化 → [`record`] 自动 no-op，不污染用户数据。
//! - 会话维度用 tokio task-local 注入（`chat.send` 的 turn 内设置），跨 await 安全，
//!   多个并行会话互不串扰。

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use serde_json::{json, Value};

use crate::events::{global_bus, Envelope};
use crate::model::llm::TokenUsage;
use crate::persistence::config::Config;

/// Usage 查询服务子模块（T-2.1 FR-B1 聚合 + 查询 + budget 告警）。
pub mod query_service;

/// 用量台账（SQLite 落盘 `~/.marguerite/usage.db`）。
pub struct UsageLedger {
    conn: Mutex<Connection>,
}

impl UsageLedger {
    /// 兄弟模块可见：拿到底层 `Mutex<Connection>` 引用（只读查询用）。
    pub(crate) fn conn_ref(&self) -> &Mutex<Connection> {
        &self.conn
    }

    /// 兄弟模块可见：把一个已打开的 Connection 包成 UsageLedger，
    /// 用于 `query_service::UsageQueryService::from_connection` 支持
    /// `rusqlite::Connection::open_in_memory()`（测试用，不碰真实数据目录）。
    pub(crate) fn wrap_for_query_service(conn: Connection) -> Self {
        Self {
            conn: Mutex::new(conn),
        }
    }
}

static LEDGER: OnceLock<Arc<UsageLedger>> = OnceLock::new();

// 当前会话（chat turn 内经 task-local 注入，供记录归会话）。
tokio::task_local! {
    pub static SESSION: String;
}

impl UsageLedger {
    /// 打开（不存在则创建）台账数据库。
    pub fn new(db_path: &std::path::Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS usage_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session TEXT NOT NULL DEFAULT '',
                model TEXT NOT NULL,
                prompt_tokens INTEGER NOT NULL DEFAULT 0,
                completion_tokens INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                created_ms INTEGER NOT NULL
            );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 记录一次调用。
    pub fn record(&self, session: &str, model: &str, usage: &TokenUsage) -> rusqlite::Result<()> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO usage_events (session, model, prompt_tokens, completion_tokens, total_tokens, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                session,
                model,
                usage.prompt_tokens as i64,
                usage.completion_tokens as i64,
                usage.total_tokens as i64,
                now_ms
            ],
        )?;
        Ok(())
    }

    /// 聚合状态（`session` 为 Some 时仅统计该会话）。
    fn status(&self, session: Option<&str>) -> rusqlite::Result<Value> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let (runs, total_tokens, sessions, since, prompt_tokens, completion_tokens): (i64, i64, i64, Option<i64>, i64, i64) = match session {
            Some(s) => conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(total_tokens), 0), COUNT(DISTINCT session), MIN(created_ms),
                        COALESCE(SUM(prompt_tokens), 0), COALESCE(SUM(completion_tokens), 0)
                 FROM usage_events WHERE session = ?1",
                rusqlite::params![s],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )?,
            None => conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(total_tokens), 0), COUNT(DISTINCT session), MIN(created_ms),
                        COALESCE(SUM(prompt_tokens), 0), COALESCE(SUM(completion_tokens), 0)
                 FROM usage_events",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )?,
        };

        let sql = match session {
            Some(_) => "SELECT model, COUNT(*), SUM(prompt_tokens), SUM(completion_tokens), SUM(total_tokens)
                        FROM usage_events WHERE session = ?1 GROUP BY model ORDER BY SUM(total_tokens) DESC",
            None => "SELECT model, COUNT(*), SUM(prompt_tokens), SUM(completion_tokens), SUM(total_tokens)
                     FROM usage_events GROUP BY model ORDER BY SUM(total_tokens) DESC",
        };
        let mut stmt = conn.prepare(sql)?;
        let models = if let Some(s) = session {
            let rows = stmt.query_map(rusqlite::params![s], |r| {
                Ok(json!({
                    "model": r.get::<_, String>(0)?,
                    "calls": r.get::<_, i64>(1)?,
                    "prompt_tokens": r.get::<_, i64>(2)?,
                    "completion_tokens": r.get::<_, i64>(3)?,
                    "total_tokens": r.get::<_, i64>(4)?,
                }))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        } else {
            let rows = stmt.query_map([], |r| {
                Ok(json!({
                    "model": r.get::<_, String>(0)?,
                    "calls": r.get::<_, i64>(1)?,
                    "prompt_tokens": r.get::<_, i64>(2)?,
                    "completion_tokens": r.get::<_, i64>(3)?,
                    "total_tokens": r.get::<_, i64>(4)?,
                }))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        Ok(json!({
            "total_tokens": total_tokens,
            "totalTokens": total_tokens,
            "total_input_tokens": prompt_tokens,
            "totalInputTokens": prompt_tokens,
            "total_output_tokens": completion_tokens,
            "totalOutputTokens": completion_tokens,
            "total_sessions": sessions,
            "totalSessions": sessions,
            "sessions": sessions,
            "runs": runs,
            "models": models,
            "since": since,
        }))
    }

    /// 最近 `limit` 条调用明细（倒序）。
    fn query(&self, limit: i64) -> rusqlite::Result<Value> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT session, model, prompt_tokens, completion_tokens, total_tokens, created_ms
             FROM usage_events ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit], |r| {
            Ok(json!({
                "session": r.get::<_, String>(0)?,
                "model": r.get::<_, String>(1)?,
                "prompt_tokens": r.get::<_, i64>(2)?,
                "completion_tokens": r.get::<_, i64>(3)?,
                "total_tokens": r.get::<_, i64>(4)?,
                "ts": r.get::<_, i64>(5)?,
            }))
        })?;
        let items = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "items": items }))
    }

    /// 最近 `days` 天按日聚合（UTC 日边界）。
    /// 返回升序 `(day_start_ms, total_tokens, calls)`，供用量页热力图/月度开销。
    pub fn daily(&self, days: i64) -> rusqlite::Result<Vec<(i64, i64, i64)>> {
        const DAY_MS: i64 = 86_400_000;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let cutoff = now_ms - days.clamp(1, 90) * DAY_MS;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT (created_ms / ?1) * ?1 AS day_start, COALESCE(SUM(total_tokens), 0), COUNT(*)
             FROM usage_events WHERE created_ms >= ?2
             GROUP BY day_start ORDER BY day_start ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![DAY_MS, cutoff], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()
    }
}

/// 初始化进程级台账（幂等；网关托管时由 chat 运行时首次触发）。
pub fn init() {
    if LEDGER.get().is_some() {
        return;
    }
    let db_path = Config::config_dir().join("usage.db");
    match UsageLedger::new(&db_path) {
        Ok(ledger) => {
            let _ = LEDGER.set(Arc::new(ledger));
        }
        Err(e) => {
            tracing::warn!(err = %e, path = %db_path.display(), "用量台账初始化失败（用量不记录）")
        }
    }
}

/// 记录一次 LLM 调用用量。台账未初始化或无真实用量时为 no-op，绝不阻塞调用方。
pub fn record(model: &str, usage: &TokenUsage) {
    if usage.total_tokens == 0 && usage.prompt_tokens == 0 && usage.completion_tokens == 0 {
        return;
    }
    if let Some(ledger) = LEDGER.get() {
        let session = SESSION.try_with(|s| s.clone()).unwrap_or_default();
        if let Err(e) = ledger.record(&session, model, usage) {
            tracing::warn!(err = %e, "用量台账记录失败");
        }
        // 用量实时事件：发布 `session.event.usage` 让前端即时刷新（替代 15s 轮询）。
        // 网关总线未注册（CLI / 测试）时静默跳过。
        if let Some(bus) = global_bus() {
            bus.publish(Envelope::usage(
                &session,
                model,
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.total_tokens,
            ));
        }
    }
}

/// `usage.status` 契约：全局（或按 `sessionKey` 过滤）聚合。
pub fn status(params: &Value) -> Value {
    let session = params.get("sessionKey").and_then(Value::as_str);
    match LEDGER.get() {
        None => json!({
            "total_tokens": 0, "totalTokens": 0,
            "total_input_tokens": 0, "totalInputTokens": 0,
            "total_output_tokens": 0, "totalOutputTokens": 0,
            "total_sessions": 0, "totalSessions": 0,
            "sessions": 0, "runs": 0, "models": [], "since": null,
        }),
        Some(ledger) => ledger.status(session).unwrap_or_else(|e| {
            tracing::warn!(err = %e, "用量台账状态查询失败");
            json!({
                "total_tokens": 0, "totalTokens": 0,
                "total_input_tokens": 0, "totalInputTokens": 0,
                "total_output_tokens": 0, "totalOutputTokens": 0,
                "total_sessions": 0, "totalSessions": 0,
                "sessions": 0, "runs": 0, "models": [], "since": null,
            })
        }),
    }
}

/// `usage.query` 契约：最近 N 条调用明细（前端用量页/图表数据源）。
pub fn query(params: &Value) -> Value {
    let limit = params
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(100)
        .clamp(1, 500);
    match LEDGER.get() {
        None => json!({ "items": [] }),
        Some(ledger) => ledger.query(limit).unwrap_or_else(|e| {
            tracing::warn!(err = %e, "用量台账明细查询失败");
            json!({ "items": [] })
        }),
    }
}

/// `usage.daily` 契约：最近 N 天按日聚合（用量页热力图/月度开销数据源）。
pub fn daily(days: u32) -> Value {
    match LEDGER.get() {
        None => json!({ "days": [] }),
        Some(ledger) => {
            let days_i = days.clamp(1, 90) as i64;
            match ledger.daily(days_i) {
                Ok(rows) => {
                    let days: Vec<Value> = rows
                        .iter()
                        .map(|(d, t, c)| json!({ "day": d, "total_tokens": t, "calls": c }))
                        .collect();
                    json!({ "days": days })
                }
                Err(e) => {
                    tracing::warn!(err = %e, "用量日聚合失败");
                    json!({ "days": [] })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db(name: &str) -> UsageLedger {
        let dir = std::env::temp_dir().join(format!("marguerite-usage-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir); // 清掉上次运行残留，避免累计污染
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("usage.db");
        UsageLedger::new(&db).expect("open temp ledger")
    }

    fn usage(p: u32, c: u32) -> TokenUsage {
        TokenUsage {
            prompt_tokens: p,
            completion_tokens: c,
            total_tokens: p + c,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        }
    }

    #[test]
    fn ledger_records_and_aggregates() {
        let ledger = tmp_db("agg");
        ledger
            .record("ws.a", "deepseek-chat", &usage(100, 20))
            .unwrap();
        ledger
            .record("ws.a", "deepseek-reasoner", &usage(200, 40))
            .unwrap();
        ledger
            .record("ws.b", "deepseek-chat", &usage(50, 10))
            .unwrap();

        let st = ledger.status(None).unwrap();
        assert_eq!(st["total_tokens"], 420);
        assert_eq!(st["runs"], 3);
        assert_eq!(st["sessions"], 2);
        assert_eq!(st["models"].as_array().unwrap().len(), 2);

        // 会话过滤
        let st_a = ledger.status(Some("ws.a")).unwrap();
        assert_eq!(st_a["total_tokens"], 360);
        assert_eq!(st_a["sessions"], 1);

        // 明细倒序
        let q = ledger.query(2).unwrap();
        let items = q["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["model"], "deepseek-chat");
        assert_eq!(items[0]["total_tokens"], 60);
    }

    #[test]
    fn record_is_noop_without_init() {
        // 全局台账未初始化（或已被其他测试初始化）时，record 都不应 panic、无副作用
        let u = usage(10, 5);
        record("never-initialized-model", &u);
    }

    #[test]
    fn daily_aggregates_by_day() {
        let ledger = tmp_db("daily");
        ledger
            .record("ws.a", "deepseek-chat", &usage(100, 20))
            .unwrap();
        ledger
            .record("ws.b", "deepseek-reasoner", &usage(200, 40))
            .unwrap();
        let rows = ledger.daily(7).unwrap();
        // 两条都落在今天（UTC 日边界）→ 恰好 1 个日桶
        assert_eq!(rows.len(), 1, "rows={rows:?}");
        let (day_start, tokens, calls) = rows[0];
        assert!(day_start > 0);
        assert_eq!(tokens, 360);
        assert_eq!(calls, 2);
        // 空台账 → 空桶
        let empty = tmp_db("daily-empty");
        assert!(empty.daily(30).unwrap().is_empty());
    }
}
