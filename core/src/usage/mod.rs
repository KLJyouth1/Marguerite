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
/// Run/session/model 维度的 token 归因台账。
pub mod attribution;

#[cfg(test)]
mod attribution_tests;

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
    /// Optional run context. Missing context is preserved as NULL, never fabricated.
    pub static RUN_ID: String;
}

pub(crate) fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS usage_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id TEXT,
            session TEXT NOT NULL DEFAULT '',
            model TEXT NOT NULL,
            prompt_tokens INTEGER NOT NULL DEFAULT 0,
            completion_tokens INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            created_ms INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS budget_meta (
            session TEXT PRIMARY KEY,
            threshold_tokens INTEGER,
            threshold_usd REAL,
            alerted INTEGER NOT NULL DEFAULT 0,
            updated_ms INTEGER NOT NULL
        );",
    )?;
    // Migrate databases created by older releases without dropping existing usage.
    let mut stmt = conn.prepare("PRAGMA table_info(usage_events)")?;
    let columns: std::collections::HashSet<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<_, _>>()?;
    drop(stmt);
    for (name, definition) in [
        ("run_id", "TEXT"),
        ("cache_read_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("cache_creation_tokens", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        if !columns.contains(name) {
            conn.execute(&format!("ALTER TABLE usage_events ADD COLUMN {name} {definition}"), [])?;
        }
    }
    Ok(())
}

impl UsageLedger {
    /// 打开（不存在则创建）台账数据库。
    ///
    /// `busy_timeout` 必设：rusqlite 默认 0，一旦撞上文件锁立即返回
    /// SQLITE_BUSY。`init()` 有 5+ 个入口（gateway/chat、skills/engine、tui、
    /// cli、gateway/server），并发首初始化时竞态方的 `init_schema`（建表/迁移）
    /// 会瞬时持锁；没有 busy_timeout 时 `UsageLedger::new` 直接失败 → `init()`
    /// 静默不点亮台账 → 进程内**所有** `record` 退化为 no-op（T7 回归测试实测
    /// 复现：流式 usage 记账随机丢失）。等待而非立即失败，保证台账必被点亮。
    pub fn new(db_path: &std::path::Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 记录一次调用（历史兼容入口，无 run 上下文）。
    pub fn record(&self, session: &str, model: &str, usage: &TokenUsage) -> rusqlite::Result<()> {
        self.record_attributed(None, session, model, usage)
    }

    /// 记录 provider 用量并保留归因字段。`run_id=None` 表示上下文缺失，持久化为 NULL。
    pub fn record_attributed(
        &self,
        run_id: Option<&str>,
        session: &str,
        model: &str,
        usage: &TokenUsage,
    ) -> rusqlite::Result<()> {
        let attribution = crate::usage::attribution::UsageAttribution::from_token_usage(
            run_id.unwrap_or(""), session, model, usage,
        )
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO usage_events (run_id, session, model, prompt_tokens, completion_tokens, total_tokens, cache_read_tokens, cache_creation_tokens, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                run_id,
                session,
                model,
                attribution.prompt_tokens as i64,
                attribution.completion_tokens as i64,
                attribution.total_tokens as i64,
                attribution.cache_read_tokens.unwrap_or(0) as i64,
                usage.cache_creation_input_tokens as i64,
                now_ms
            ],
        )?;
        drop(conn);
        // M3-D2：预算阈值评估（超限→审计告警 + 总线升级事件）
        if let Err(e) = self.check_fire_budget(session) {
            tracing::warn!(err = %e, "预算告警评估失败");
        }
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
            "SELECT run_id, session, model, prompt_tokens, completion_tokens, total_tokens,
                    cache_read_tokens, cache_creation_tokens, created_ms
             FROM usage_events ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit], |r| {
            Ok(json!({
                "run_id": r.get::<_, Option<String>>(0)?,
                "session": r.get::<_, String>(1)?,
                "model": r.get::<_, String>(2)?,
                "prompt_tokens": r.get::<_, i64>(3)?,
                "completion_tokens": r.get::<_, i64>(4)?,
                "total_tokens": r.get::<_, i64>(5)?,
                "cache_read_tokens": r.get::<_, i64>(6)?,
                "cache_creation_tokens": r.get::<_, i64>(7)?,
                "ts": r.get::<_, i64>(8)?,
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

    // ─── M3-D2 FinOps：按会话/任务成本归因 + 预算阈值告警 ─────────────────

    /// 按 run 聚合 token 与估算成本（成本卡片数据源）。
    fn task_cost_view(&self, run_id: &str) -> rusqlite::Result<Value> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT model, COUNT(*), COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(completion_tokens),0), COALESCE(SUM(total_tokens),0)
             FROM usage_events WHERE run_id = ?1 GROUP BY model ORDER BY SUM(total_tokens) DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![run_id], |r| {
            let (model, calls, inp, outp, total): (String, i64, i64, i64, i64) =
                (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?);
            let (p_in, p_out) = price_for(&model);
            let cost = inp as f64 / 1_000_000.0 * p_in + outp as f64 / 1_000_000.0 * p_out;
            Ok(json!({
                "model": model, "calls": calls,
                "prompt_tokens": inp, "completion_tokens": outp, "total_tokens": total,
                "cost_usd": (cost * 100.0).round() / 100.0,
            }))
        })?;
        let models: Vec<Value> = rows.collect::<Result<_, _>>()?;
        let total_tokens: i64 = models.iter().map(|m| m["total_tokens"].as_i64().unwrap_or(0)).sum();
        let input_tokens: i64 = models.iter().map(|m| m["prompt_tokens"].as_i64().unwrap_or(0)).sum();
        let output_tokens: i64 = models.iter().map(|m| m["completion_tokens"].as_i64().unwrap_or(0)).sum();
        let calls: i64 = models.iter().map(|m| m["calls"].as_i64().unwrap_or(0)).sum();
        let cost_usd: f64 = models.iter().map(|m| m["cost_usd"].as_f64().unwrap_or(0.0)).sum();
        let session: Option<String> = if total_tokens == 0 {
            None
        } else {
            conn.query_row(
                "SELECT session FROM usage_events WHERE run_id = ?1 AND session <> '' LIMIT 1",
                rusqlite::params![run_id],
                |r| r.get(0),
            )
            .ok()
        };
        Ok(json!({
            "ok": true, "run_id": run_id,
            "session": session.unwrap_or_default(),
            "total_tokens": total_tokens, "input_tokens": input_tokens, "output_tokens": output_tokens,
            "calls": calls, "cost_usd": (cost_usd * 100.0).round() / 100.0,
            "currency": "USD",
            "models": models,
        }))
    }

    /// 某会话的预算设置与当下累计（含是否超限）。
    fn budget_view(&self, session: &str) -> rusqlite::Result<Value> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let (threshold_tokens, threshold_usd): (Option<i64>, Option<f64>) = conn
            .query_row(
                "SELECT threshold_tokens, threshold_usd FROM budget_meta WHERE session=?1",
                rusqlite::params![session],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((None, None));
        let (tokens, input, output, cost_usd) = session_cost_locked(&conn, session)?;
        let exceeded = match (threshold_tokens, threshold_usd) {
            (Some(t), Some(u)) => tokens > t || cost_usd > u,
            (Some(t), None) => tokens > t,
            (None, Some(u)) => cost_usd > u,
            (None, None) => false,
        };
        Ok(json!({
            "ok": true, "session": session,
            "threshold_tokens": threshold_tokens,
            "threshold_usd": threshold_usd,
            "total_tokens": tokens, "input_tokens": input, "output_tokens": output,
            "cost_usd": (cost_usd * 100.0).round() / 100.0,
            "currency": "USD",
            "exceeded": exceeded,
        }))
    }

    /// 设置会话预算阈值（token 与 USD 至少其一），并重置告警标记供新阈值重新评估。
    pub fn set_session_budget(
        &self,
        session: &str,
        threshold_tokens: Option<u64>,
        threshold_usd: Option<f64>,
    ) -> rusqlite::Result<()> {
        if threshold_tokens.is_none() && threshold_usd.is_none() {
            return Ok(());
        }
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO budget_meta (session, threshold_tokens, threshold_usd, alerted, updated_ms)
             VALUES (?1, ?2, ?3, 0, ?4)
             ON CONFLICT(session) DO UPDATE SET
               threshold_tokens=excluded.threshold_tokens,
               threshold_usd=excluded.threshold_usd,
               alerted=0, updated_ms=excluded.updated_ms",
            rusqlite::params![
                session,
                threshold_tokens.map(|v| v as i64),
                threshold_usd,
                now_ms
            ],
        )?;
        Ok(())
    }

    /// 移除会话预算设置。
    pub fn remove_session_budget(&self, session: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM budget_meta WHERE session=?1",
            rusqlite::params![session],
        )?;
        Ok(())
    }

    /// 每次真实调用落库后评估预算：超阈值 → 审计告警事件 + 总线 status 升级事件
    /// （每个阈值只触发一次，重新 set 后重置）。返回本次是否触发了新告警。
    fn check_fire_budget(&self, session: &str) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let Some((threshold_tokens, threshold_usd, alerted)) = conn
            .query_row(
                "SELECT threshold_tokens, threshold_usd, alerted FROM budget_meta WHERE session=?1",
                rusqlite::params![session],
                |r| {
                    Ok((
                        r.get::<_, Option<i64>>(0)?,
                        r.get::<_, Option<f64>>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            )
            .ok()
        else {
            return Ok(false);
        };
        if alerted != 0 {
            return Ok(false);
        }
        let (tokens, _, _, cost_usd) = session_cost_locked(&conn, session)?;
        let exceeded = match (threshold_tokens, threshold_usd) {
            (Some(t), Some(u)) => tokens > t || cost_usd > u,
            (Some(t), None) => tokens > t,
            (None, Some(u)) => cost_usd > u,
            (None, None) => false,
        };
        if !exceeded {
            return Ok(false);
        }
        conn.execute(
            "UPDATE budget_meta SET alerted=1 WHERE session=?1",
            rusqlite::params![session],
        )?;
        drop(conn);
        // ① 产品事件落审计日志
        crate::events::product::emit(
            crate::events::product::BUDGET_WARNING,
            session,
            json!({
                "threshold_tokens": threshold_tokens,
                "threshold_usd": threshold_usd,
                "total_tokens": tokens,
                "cost_usd": (cost_usd * 100.0).round() / 100.0,
            }),
        );
        // ② 总线 status 升级事件（实时推送前端「消费超预算」）
        if let Some(bus) = crate::events::global_bus() {
            bus.publish(crate::events::Envelope::new(
                crate::events::EventType::Status,
                json!({
                    "level": "warning",
                    "scope": "budget.exceeded",
                    "session": session,
                    "message": "消费超过预算阈值，已触发升级提示",
                }),
                session,
            ));
        }
        Ok(true)
    }
}

/// 按会话聚合 token 与估算成本（调用方需已持有 ledger 连接锁）。
fn session_cost_locked(
    conn: &Connection,
    session: &str,
) -> rusqlite::Result<(i64, i64, i64, f64)> {
    let mut stmt = conn.prepare(
        "SELECT model, COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(completion_tokens),0), COALESCE(SUM(total_tokens),0)
         FROM usage_events WHERE session=?1 GROUP BY model",
    )?;
    let rows: Vec<(i64, i64, i64, f64)> = stmt
        .query_map(rusqlite::params![session], |r| {
            let (model, inp, outp, total): (String, i64, i64, i64) =
                (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
            let (p_in, p_out) = price_for(&model);
            let cost = inp as f64 / 1_000_000.0 * p_in + outp as f64 / 1_000_000.0 * p_out;
            Ok((total, inp, outp, cost))
        })?
        .filter_map(Result::ok)
        .collect();
    let mut tokens = 0i64;
    let mut input = 0i64;
    let mut output = 0i64;
    let mut cost = 0f64;
    for (t, i, o, c) in rows {
        tokens += t;
        input += i;
        output += o;
        cost += c;
    }
    Ok((tokens, input, output, cost))
}

/// 台账落盘路径。
///
/// ⚠️ **单元测试永不打开用户真实目录**。`init()` 被 chat / engine / gateway / TUI
/// 多处入口调用，而 `LEDGER` 是**进程级** `OnceLock`：只要同进程里有一个测试走到
/// 那些入口，台账就被点亮，之后**所有** `record()`（包括
/// `record_is_noop_without_init` 那类夹具）都会写进 `~/.marguerite/usage.db`。
/// 实测（2026-09-27，真机库）：`never-initialized-model` 夹具累计 **567 行**、
/// `session=''` 的空归属事件 **6930 行**；前端把后者当成
/// 「本会话 token 680677 / 轮次 6930」显示给用户（一个全新会话根本不可能有 6930 轮）。
#[cfg(test)]
fn ledger_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("marguerite-usage-test-{}.db", std::process::id()))
}

#[cfg(not(test))]
fn ledger_path() -> std::path::PathBuf {
    Config::config_dir().join("usage.db")
}

/// 初始化进程级台账（幂等；网关托管时由 chat 运行时首次触发）。
pub fn init() {
    if LEDGER.get().is_some() {
        return;
    }
    let db_path = ledger_path();
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
    let session = SESSION.try_with(|s| s.clone()).unwrap_or_default();
    let run_id = RUN_ID
        .try_with(|r| r.clone())
        .ok()
        .filter(|r| !r.trim().is_empty());
    record_attributed(run_id.as_deref(), &session, model, usage);
}

/// Provider 适配器的归因记录入口。缺失 run 上下文时传 `None`，不会伪造 run_id。
/// 该函数复用进程级 SQLite ledger，与历史 `record` 入口共享同一台账。
pub fn record_attributed(
    run_id: Option<&str>,
    session: &str,
    model: &str,
    usage: &TokenUsage,
) {
    if usage.total_tokens == 0 && usage.prompt_tokens == 0 && usage.completion_tokens == 0 {
        return;
    }
    if let Some(ledger) = LEDGER.get() {
        if let Err(e) = ledger.record_attributed(run_id, session, model, usage) {
            tracing::warn!(err = %e, "用量台账记录失败");
        }
        // 用量实时事件：发布 `session.event.usage` 让前端即时刷新（替代 15s 轮询）。
        // 网关总线未注册（CLI / 测试）时静默跳过。
        if let Some(bus) = global_bus() {
            bus.publish(Envelope::usage(
                session,
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

// ─────────────────────────────────────────────────────────────────────────────
// 成本估算 + peak/off-peak 时段（2026-08「成本策略变化」启示落地）
// ─────────────────────────────────────────────────────────────────────────────

/// `usage.taskCost`（M3-D2）：按任务( run )归因的成本卡片数据。
pub fn task_cost(params: &Value) -> Value {
    let run_id = params
        .get("runId")
        .and_then(Value::as_str)
        .or_else(|| params.get("run_id").and_then(Value::as_str))
        .unwrap_or("");
    if run_id.is_empty() {
        return json!({ "ok": false, "error": "runId 不可为空" });
    }
    match LEDGER.get() {
        None => json!({
            "ok": false, "run_id": run_id,
            "error": "用量台账未初始化", "total_tokens": 0, "calls": 0, "cost_usd": 0.0, "models": [],
        }),
        Some(ledger) => ledger.task_cost_view(run_id).unwrap_or_else(|e| {
            tracing::warn!(err = %e, run = run_id, "任务成本查询失败");
            json!({ "ok": false, "run_id": run_id, "error": format!("查询失败: {e}") })
        }),
    }
}

/// `usage.budget.get`：读取某会话预算设置与当前累计（含超限标记）。
pub fn budget_get(params: &Value) -> Value {
    let session = params.get("session").and_then(Value::as_str).unwrap_or("");
    match LEDGER.get() {
        None => json!({ "ok": false, "session": session, "error": "用量台账未初始化" }),
        Some(ledger) => ledger.budget_view(session).unwrap_or_else(|e| {
            tracing::warn!(err = %e, session = session, "预算状态查询失败");
            json!({ "ok": false, "session": session, "error": format!("查询失败: {e}") })
        }),
    }
}

/// `usage.budget.set`：设置会话预算阈值（`thresholdTokens` / `thresholdUsd` 至少其一；
/// 传 `thresholdTokens=0` 且 `thresholdUsd=0` 表示移除预算）。
pub fn budget_set(params: &Value) -> Value {
    let session = params.get("session").and_then(Value::as_str).unwrap_or("");
    let threshold_tokens = params.get("thresholdTokens").and_then(Value::as_u64);
    let threshold_usd = params.get("thresholdUsd").and_then(Value::as_f64);
    if threshold_tokens.is_none() && threshold_usd.is_none() {
        return json!({ "ok": false, "session": session, "error": "请提供 thresholdTokens 或 thresholdUsd" });
    }
    match LEDGER.get() {
        None => json!({ "ok": false, "session": session, "error": "用量台账未初始化" }),
        Some(ledger) => {
            let remove = threshold_tokens == Some(0) && threshold_usd == Some(0.0);
            let res = if remove {
                ledger.remove_session_budget(session)
            } else {
                ledger.set_session_budget(session, threshold_tokens, threshold_usd)
            };
            match res {
                Ok(()) => json!({ "ok": true, "session": session, "thresholdTokens": threshold_tokens, "thresholdUsd": threshold_usd }),
                Err(e) => {
                    tracing::warn!(err = %e, "预算设置失败");
                    json!({ "ok": false, "session": session, "error": format!("设置失败: {e}") })
                }
            }
        }
    }
}

/// 已知模型的单价表（美元 / 百万 token）。DeepSeek V4 自 2026-08-16 起分时段定价，
/// 此处取峰值档近似；未命中前缀按通用档估算。
/// 格式：(模型名前缀, 输入 $/MTok, 输出 $/MTok)
const PRICE_TABLE: &[(&str, f64, f64)] = &[
    ("deepseek-v4-pro", 1.2, 6.0),
    ("deepseek-v4-flash", 0.3, 1.2),
    ("deepseek-chat", 0.27, 1.1),
    ("gpt-4o", 2.5, 10.0),
    ("gpt-4o-mini", 0.15, 0.6),
    ("o4-mini", 1.1, 4.4),
    ("claude-sonnet", 3.0, 15.0),
    ("claude-opus", 15.0, 75.0),
    ("claude-haiku", 0.8, 4.0),
    ("glm-4-plus", 0.5, 2.0),
    ("glm-4v", 0.5, 2.0),
    ("qwen-max", 1.6, 6.4),
    ("kimi-k2", 0.6, 2.5),
    ("gemini-2.5-pro", 1.25, 10.0),
    ("gemini-2.5-flash", 0.30, 2.50),
];

/// 按模型名前缀取单价（默认：通用档输入 $0.6 / 输出 $2.5）。
///
/// **最长前缀优先**：同名包含多个前缀时取最具体者（如 `gpt-4o-mini` 同时命中
/// `gpt-4o` 与 `gpt-4o-mini`，须取后者），否则会错误复用更贵档位的单价。
fn price_for(model: &str) -> (f64, f64) {
    let m = model.to_ascii_lowercase();
    PRICE_TABLE
        .iter()
        .filter(|(prefix, _, _)| m.contains(prefix))
        .max_by_key(|(prefix, _, _)| prefix.len())
        .map(|(_, i, o)| (*i, *o))
        .unwrap_or((0.6, 2.5))
}

/// 当前本地小时 → 时段（DeepSeek 式分时段定价启发：23:00-07:59 off-peak，其余 peak）。
fn current_period() -> (&'static str, u32) {
    let hour = {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // 本地时区小时（近似：取 UTC+8 所在日的小时；不引入 chrono 依赖）
        ((secs % 86400 + 8 * 3600) % 86400) / 3600
    } as u32;
    let period = if !(8..23).contains(&hour) {
        "off-peak"
    } else {
        "peak"
    };
    (period, hour)
}

/// `usage.cost`：根据台账聚合 token 估算美元成本 + 当前时段建议。
pub fn cost(params: &Value) -> Value {
    let session = params.get("sessionKey").and_then(Value::as_str);
    let status = status(params);
    let input = status["total_input_tokens"].as_u64().unwrap_or(0);
    let output = status["total_output_tokens"].as_u64().unwrap_or(0);
    // 模型维度：按台账 models 列表取首个已知模型定价（多模型时取 max 加权近似）
    let models: Vec<Value> = status["models"].as_array().cloned().unwrap_or_default();
    let (p_in, p_out) = models
        .iter()
        .filter_map(|m| m["model"].as_str())
        .map(price_for)
        .max_by(|a, b| {
            (a.0 + a.1)
                .partial_cmp(&(b.0 + b.1))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or((0.6, 2.5));
    let est = input as f64 / 1_000_000.0 * p_in + output as f64 / 1_000_000.0 * p_out;
    let (period, hour) = current_period();

    json!({
        "ok": true,
        "session": session.unwrap_or(""),
        "estimated_cost_usd": (est * 100.0).round() / 100.0,
        "input_tokens": input,
        "output_tokens": output,
        "price_input_per_mtok": p_in,
        "price_output_per_mtok": p_out,
        "model_priced": models.iter().filter_map(|m| m["model"].as_str()).next().unwrap_or("default"),
        "period": period,
        "local_hour": hour,
        "schedule": "DeepSeek 等厂商已采用分时段定价：长任务建议排程到 off-peak（本地 23:00-07:59）执行，可显著降本。",
        "advice": if period == "peak" {
            "当前为峰值时段：批量/长任务可延后至 off-peak 排程。"
        } else {
            "当前为低谷时段：适合排程批量与长任务。"
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_table_covers_major_providers() {
        assert!(price_for("deepseek-v4-pro").0 > 0.0);
        assert!(price_for("gpt-4o").0 > price_for("gpt-4o-mini").0);
        assert!(price_for("unknown-model").0 == 0.6);
    }

    #[test]
    fn cost_estimates_positive_and_period_valid() {
        let v = cost(&json!({}));
        assert_eq!(v["ok"], true);
        assert!(v["estimated_cost_usd"].as_f64().unwrap() >= 0.0);
        let period = v["period"].as_str().unwrap();
        assert!(period == "peak" || period == "off-peak");
        assert!(v["advice"].as_str().is_some_and(|s| !s.is_empty()));
    }

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
    fn provider_record_persists_attribution_columns() {
        let ledger = tmp_db("attribution-columns");
        let usage = TokenUsage {
            prompt_tokens: 8,
            completion_tokens: 3,
            total_tokens: 11,
            cache_read_input_tokens: 5,
            cache_creation_input_tokens: 0,
        };
        ledger
            .record_attributed(Some("run-1"), "session-1", "model-a", &usage)
            .expect("provider attribution record");
        let conn = ledger.conn.lock().unwrap();
        let row: (String, String, i64) = conn
            .query_row(
                "SELECT run_id, session, cache_read_tokens FROM usage_events",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("attribution columns must be queryable");
        assert_eq!(row, ("run-1".to_string(), "session-1".to_string(), 5));
    }

    #[test]
    fn legacy_record_keeps_run_unset_and_query_exposes_cache_read() {
        let ledger = tmp_db("legacy-compatible");
        let usage = TokenUsage {
            prompt_tokens: 8,
            completion_tokens: 3,
            total_tokens: 11,
            cache_read_input_tokens: 5,
            cache_creation_input_tokens: 0,
        };
        ledger.record("session-legacy", "model-a", &usage).unwrap();
        let item = ledger.query(1).unwrap()["items"][0].clone();
        assert!(item["run_id"].is_null());
        assert_eq!(item["session"], "session-legacy");
        assert_eq!(item["cache_read_tokens"], 5);
    }

    #[test]
    fn invalid_total_is_rejected_without_insert() {
        let ledger = tmp_db("invalid-total");
        let usage = TokenUsage {
            prompt_tokens: 8,
            completion_tokens: 3,
            total_tokens: 99,
            ..Default::default()
        };
        assert!(ledger.record_attributed(Some("r"), "s", "m", &usage).is_err());
        let count: i64 = ledger.conn.lock().unwrap().query_row(
            "SELECT COUNT(*) FROM usage_events", [], |r| r.get(0)
        ).unwrap();
        assert_eq!(count, 0);
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

    /// 守卫（有牙）：测试态下 `init()` 必须落在临时文件上。
    ///
    /// 这条断言挡的是真实发生过的数据污染：把 `ledger_path()` 改回
    /// `Config::config_dir().join("usage.db")` 时本用例立刻失败。
    #[test]
    fn ledger_path_is_never_the_real_user_database_under_test() {
        let path = ledger_path();
        assert_ne!(
            path,
            Config::config_dir().join("usage.db"),
            "单元测试不得打开用户真实台账：夹具事件会写进 ~/.marguerite/usage.db"
        );
        assert!(
            path.to_string_lossy().contains("marguerite-usage-test-"),
            "测试态台账应落在可辨识的临时文件名上，实际是 {}",
            path.display()
        );
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

    #[test]
    fn budget_threshold_alert_fires_once_and_rearms_on_reset() {
        let ledger = tmp_db("budget");
        // 无预算 → 不触发
        assert!(!ledger.check_fire_budget("sess-budget").unwrap());
        ledger.set_session_budget("sess-budget", Some(100), None).unwrap();
        // 未超限 → 不触发
        assert!(!ledger.check_fire_budget("sess-budget").unwrap());
        // 一次真实调用落库（160 tokens 跨过 100 阈值）→ 触发告警
        ledger
            .record_attributed(Some("r1"), "sess-budget", "deepseek-chat", &usage(80, 80))
            .unwrap();
        let view = ledger.budget_view("sess-budget").unwrap();
        assert_eq!(view["exceeded"], true);
        assert_eq!(view["total_tokens"], 160);
        // 同一阈值只触发一次（幂等）
        assert!(!ledger.check_fire_budget("sess-budget").unwrap());
        // 调高阈值 + 重置告警标记 → 重新评估为不超限
        ledger.set_session_budget("sess-budget", Some(10000), None).unwrap();
        let view2 = ledger.budget_view("sess-budget").unwrap();
        assert_eq!(view2["exceeded"], false);
        // 移除预算后永不触发
        ledger.remove_session_budget("sess-budget").unwrap();
        assert!(!ledger.check_fire_budget("sess-budget").unwrap());
    }

    #[test]
    fn budget_threshold_also_checks_usd_limit() {
        let ledger = tmp_db("budget-usd");
        // USD 阈值：deepseek-chat 500k/100k ≈ $0.245 > $0.20
        ledger.set_session_budget("sess-usd", None, Some(0.20)).unwrap();
        ledger
            .record_attributed(Some("r1"), "sess-usd", "deepseek-chat", &usage(500_000, 100_000))
            .unwrap();
        let view = ledger.budget_view("sess-usd").unwrap();
        assert_eq!(view["exceeded"], true);
        assert!((view["cost_usd"].as_f64().unwrap() - 0.25).abs() < 1e-6);
    }

    #[test]
    fn task_cost_card_attributes_per_run() {
        let ledger = tmp_db("taskcost");
        ledger
            .record_attributed(Some("task-7"), "sess-a", "deepseek-chat", &usage(500_000, 100_000))
            .unwrap();
        ledger
            .record_attributed(Some("task-7"), "sess-a", "deepseek-chat", &usage(100, 50))
            .unwrap();
        ledger
            .record_attributed(Some("task-8"), "sess-a", "gpt-4o-mini", &usage(10, 5))
            .unwrap();
        let card = ledger.task_cost_view("task-7").unwrap();
        assert_eq!(card["ok"], true);
        assert_eq!(card["session"], "sess-a");
        assert_eq!(card["calls"], 2);
        assert!(card["total_tokens"].as_i64().unwrap() >= 600_000);
        assert_eq!(card["models"].as_array().unwrap().len(), 1);
        assert!(card["cost_usd"].as_f64().unwrap() >= 0.2);
        // 另一任务互不串扰
        let card8 = ledger.task_cost_view("task-8").unwrap();
        assert_eq!(card8["total_tokens"], 15);
        // 未知 run → 空卡片（不 panic）
        let empty_card = ledger.task_cost_view("task-nope").unwrap();
        assert_eq!(empty_card["calls"], 0);
    }
}
