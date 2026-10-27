//! Token 用量台账：真实记录每次 LLM 调用（prompt/completion tokens），
//! 供 `usage.status` / `usage.query` 聚合返回，替换原先硬编码全零的占位实现。
//!
//! 设计要点：
//! - 进程级单例 [`LEDGER`]；网关托管时由 chat 运行时首次触发 [`init`]，CLI / 测试
//!   默认不初始化 → [`record`] 自动 no-op，不污染用户数据。
//! - 会话维度用 tokio task-local 注入（`chat.send` 的 turn 内设置），跨 await 安全，
//!   多个并行会话互不串扰。

use std::collections::{BTreeMap, BTreeSet};
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

/// `query_contract` 的可选视图开关。
///
/// 把 4 个开关/上限收成一个结构：8 参函数既难读，也过不了 `clippy::too_many_arguments`。
#[derive(Debug, Clone, Copy)]
pub struct UsageQueryOptions<'a> {
    /// 时间范围预设（`today` / `7d` / `30d` / `all`），原样回填到 `range.preset`
    pub preset: &'a str,
    /// IANA 时区名，原样回填到 `range.timezone`
    pub timezone: &'a str,
    /// 是否返回按本地日分桶的 `days` 视图
    pub include_days: bool,
    /// 是否返回按模型聚合的 `models` 视图
    pub include_models: bool,
    /// 是否返回按会话聚合的 `sessions` 视图
    pub include_sessions: bool,
    /// 旧消费方读取的 `items` 明细条数上限
    pub item_limit: i64,
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
    ///
    /// 生产路径已改走 [`Self::query_contract`]（四视图），这里只服务单测
    /// （断言明细字段形状）；`cfg(test)` 避免 lib 构建出现 dead_code 告警。
    #[cfg(test)]
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

    /// `usage.query` 的完整体：totals / sessions / models / days 四视图。
    ///
    /// # 为什么必须由 Rust 提供（真机缺陷，2026-10-23）
    /// 前端 `useUsageQuery` 只认 `{ totals, sessions, models, days, coverage }`，
    /// 而 Rust 旧实现返回 `{ items: [...] }`。于是 `/usage` 页面拿到的 `totals`
    /// 是 undefined → 全部 KPI 归零：用户发起请求、台账明明有 10 万 token，
    /// 页面显示 0。这正是「Token 消耗计量是否准确」这条验收点会失败的原因。
    ///
    /// 聚合口径与前端 `normalizeUsageTotals` 对齐（camelCase 字段名）：
    /// `inputTokens / outputTokens / totalTokens / cacheReadTokens / cacheWriteTokens /
    /// costNanos / sessionCount / eventCount`。
    /// 日维度用 SQLite `localtime` 分桶（与用户看到的「今天」一致），并把范围
    /// 起点显式算成毫秒传进来，避免 SQL 里做时区算术。
    pub fn query_contract(
        &self,
        from_ms: Option<i64>,
        opts: UsageQueryOptions<'_>,
    ) -> rusqlite::Result<Value> {
        let UsageQueryOptions {
            preset,
            timezone,
            include_days,
            include_models,
            include_sessions,
            item_limit,
        } = opts;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());

        // 单趟扫描：按 (本地日, 会话, 模型) 聚合，再在 Rust 里折叠成四个视图。
        // 行数 = 组合数（远小于事件数），比四次 GROUP BY 更省。
        let mut stmt = conn.prepare(
            "SELECT COALESCE(strftime('%Y-%m-%d', created_ms/1000, 'unixepoch', 'localtime'), ''),
                    session, model,
                    COUNT(*),
                    COALESCE(SUM(prompt_tokens), 0),
                    COALESCE(SUM(completion_tokens), 0),
                    COALESCE(SUM(total_tokens), 0),
                    COALESCE(SUM(cache_read_tokens), 0),
                    COALESCE(SUM(cache_creation_tokens), 0),
                    MIN(created_ms), MAX(created_ms)
             FROM usage_events
             WHERE (?1 IS NULL OR created_ms >= ?1)
             GROUP BY 1, session, model
             ORDER BY 1 ASC",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![from_ms], |r| {
                Ok(UsageAggRow {
                    day: r.get::<_, String>(0)?,
                    session: r.get::<_, String>(1)?,
                    model: r.get::<_, String>(2)?,
                    calls: r.get::<_, i64>(3)?,
                    input: r.get::<_, i64>(4)?,
                    output: r.get::<_, i64>(5)?,
                    total: r.get::<_, i64>(6)?,
                    cache_read: r.get::<_, i64>(7)?,
                    cache_write: r.get::<_, i64>(8)?,
                    first_ms: r.get::<_, i64>(9)?,
                    last_ms: r.get::<_, i64>(10)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut totals = UsageTotalsAcc::default();
        let mut sessions: BTreeMap<String, UsageTotalsAcc> = BTreeMap::new();
        let mut models: BTreeMap<String, UsageTotalsAcc> = BTreeMap::new();
        let mut days: BTreeMap<String, UsageTotalsAcc> = BTreeMap::new();
        let mut model_sessions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut all_sessions: BTreeSet<String> = BTreeSet::new();
        // 会话 → 首末时间 / 模型明细：前端明细表的「修改时间 / 来源 / 模型」三列
        // 只认这些字段；缺了就是三列全 `-`（真机可见）。
        let mut session_span: BTreeMap<String, (i64, i64)> = BTreeMap::new();
        let mut session_models: BTreeMap<String, BTreeMap<String, UsageTotalsAcc>> =
            BTreeMap::new();

        for row in &rows {
            totals.add(row);
            all_sessions.insert(row.session.clone());
            sessions.entry(row.session.clone()).or_default().add(row);
            models.entry(row.model.clone()).or_default().add(row);
            days.entry(row.day.clone()).or_default().add(row);
            model_sessions
                .entry(row.model.clone())
                .or_default()
                .insert(row.session.clone());
            session_models
                .entry(row.session.clone())
                .or_default()
                .entry(row.model.clone())
                .or_default()
                .add(row);
            let span = session_span
                .entry(row.session.clone())
                .or_insert((row.first_ms, row.last_ms));
            span.0 = span.0.min(row.first_ms);
            span.1 = span.1.max(row.last_ms);
        }

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let mut out = json!({
            "schemaVersion": 1,
            "asOfMs": now_ms,
            "range": {
                "preset": preset,
                "timezone": timezone,
                "fromMs": from_ms,
                "toMs": null,
            },
            "totals": totals.to_json(all_sessions.len()),
            "coverage": {
                "status": "complete",
                "reasonCodes": [],
                "legacyUnattributed": null,
                "nativeBilling": { "status": "unavailable", "reasonCodes": ["no_billing_receipts"] },
            },
            // 旧消费方（审计脚本 / 明细面板）仍按 `items` 读取最近 N 条：
            // 保留它，避免这次修复把别的调用方打断。
            "items": query_recent(&conn, item_limit)?,
        });
        let obj = out.as_object_mut().expect("json! 构造的一定是对象");
        if include_days {
            obj.insert(
                "days".into(),
                Value::Array(
                    days.iter()
                        .map(|(day, acc)| json!({ "date": day, "totals": acc.to_json(1) }))
                        .collect(),
                ),
            );
        }
        if include_models {
            obj.insert(
                "models".into(),
                Value::Array(
                    models
                        .iter()
                        .map(|(model, acc)| {
                            json!({
                                "model": model,
                                "provider": model.split(['/', ':']).next().unwrap_or(model),
                                "sessionCount": model_sessions.get(model).map(|s| s.len()).unwrap_or(0),
                                "totals": acc.to_json(model_sessions.get(model).map(|s| s.len()).unwrap_or(0)),
                            })
                        })
                        .collect(),
                ),
            );
        }
        if include_sessions {
            obj.insert(
                "sessions".into(),
                Value::Array(
                    sessions
                        .iter()
                        .map(|(session, acc)| {
                            let span = session_span.get(session).copied().unwrap_or((0, 0));
                            let breakdown: Vec<Value> = session_models
                                .get(session)
                                .map(|m| {
                                    m.iter()
                                        .map(|(model, a)| {
                                            json!({ "model": model, "totals": a.to_json(1) })
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            json!({
                                "sessionKey": session,
                                "sessionId": session,
                                "firstUsageAtMs": span.0,
                                "lastUsageAtMs": span.1,
                                "costSource": "Marguerite_estimate",
                                "modelBreakdown": breakdown,
                                "totals": acc.to_json(1),
                            })
                        })
                        .collect(),
                ),
            );
        }
        Ok(out)
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

/// `created_ms >= since_ms` 以来的累计用量 `(tokens, usd)`（按模型单价估算）。
///
/// `session = None` 表示全部会话。单价口径与 [`session_cost_locked`] 完全一致
/// （同一 [`price_for`] 表），避免"预算按 A 单价算、账单按 B 单价算"的双口径。
fn spend_locked(
    conn: &Connection,
    session: Option<&str>,
    since_ms: i64,
) -> rusqlite::Result<(i64, f64)> {
    let sql_all = "SELECT model, COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(completion_tokens),0), COALESCE(SUM(total_tokens),0)
                   FROM usage_events WHERE created_ms >= ?1 GROUP BY model";
    let sql_session = "SELECT model, COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(completion_tokens),0), COALESCE(SUM(total_tokens),0)
                       FROM usage_events WHERE created_ms >= ?1 AND session = ?2 GROUP BY model";
    let mut stmt = conn.prepare(if session.is_some() { sql_session } else { sql_all })?;
    let mut tokens = 0i64;
    let mut cost = 0f64;
    let acc = |r: &rusqlite::Row<'_>| -> rusqlite::Result<(i64, f64)> {
        let (model, inp, outp, total): (String, i64, i64, i64) =
            (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
        let (p_in, p_out) = price_for(&model);
        Ok((
            total,
            inp as f64 / 1_000_000.0 * p_in + outp as f64 / 1_000_000.0 * p_out,
        ))
    };
    let rows = match session {
        Some(s) => stmt
            .query_map(rusqlite::params![since_ms, s], |r| acc(r))?
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
        None => stmt
            .query_map(rusqlite::params![since_ms], |r| acc(r))?
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
    };
    for (t, c) in rows {
        tokens += t;
        cost += c;
    }
    Ok((tokens, cost))
}

/// 已配置预算的**实测快照**：上限来自 `budget_meta`，已用来自台账真实事件。
///
/// # 为什么必须由台账来算「已用」
///
/// `model::tier::SessionBudget` 是**生效**的预算口径（档位解析与 `model.tier.get`
/// 都读它），但它的 `tokens_used/usd_used` **全仓没有任何写入点** ——
/// `record_usage()` 零调用者，于是 `remaining_ratio()` 恒为 1.0：
/// 预算降档与 `Depleted` 永不触发，用户在控制台看到的是「剩余 100%」这个假读数。
/// 上限（用户经 `usage.budget.set` 设定）与已用（台账事件）必须落在同一处，
/// 这里就是那个「同一处」。
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetSnapshot {
    /// 归属会话（全局视图下为「最紧的那个」）
    pub session: String,
    pub tokens_cap: u64,
    pub usd_cap: f64,
    pub tokens_used: u64,
    pub usd_used: f64,
    /// 剩余比例 0..1（两个口径取更紧者）
    pub remaining_ratio: f64,
    /// 计量起点（= 该预算最近一次设定时刻）：预算一改就从零重新计
    pub since_ms: i64,
}

impl BudgetSnapshot {
    /// 由上限与已用算出剩余比例（与 `SessionBudget::remaining_ratio` 同口径）。
    fn ratio(tokens_cap: u64, tokens_used: u64, usd_cap: f64, usd_used: f64) -> f64 {
        let token_ratio = if tokens_cap == 0 {
            1.0
        } else {
            1.0 - (tokens_used as f64 / tokens_cap as f64)
        };
        let usd_ratio = if usd_cap <= 0.0 {
            1.0
        } else {
            1.0 - (usd_used / usd_cap)
        };
        token_ratio.min(usd_ratio).clamp(0.0, 1.0)
    }
}

impl UsageLedger {
    /// 某会话的预算实测快照；该会话未配置预算（或两个阈值都为空）时返回 `None`。
    pub fn budget_snapshot_for(&self, session: &str) -> rusqlite::Result<Option<BudgetSnapshot>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let Some((s, tc, uc, since)) = configured_budgets_locked(&conn)?
            .into_iter()
            .find(|b| b.0 == session)
        else {
            return Ok(None);
        };
        snapshot_of_locked(&conn, &s, tc, uc, since)
    }

    /// **最紧**的已配置预算（按剩余比例最小者；并列则按会话名升序取第一个）。
    ///
    /// 全局档位态（`SessionState` 无会话维度）用它作为预算来源：
    /// 「任何一个已配置预算吃紧」都该让全局生效档降下来，而不是各自为政。
    pub fn tightest_budget_snapshot(&self) -> rusqlite::Result<Option<BudgetSnapshot>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut best: Option<BudgetSnapshot> = None;
        let mut configured = configured_budgets_locked(&conn)?;
        configured.sort_by(|a, b| a.0.cmp(&b.0)); // 确定性：会话名升序
        for (s, tc, uc, since) in configured {
            let Some(snap) = snapshot_of_locked(&conn, &s, tc, uc, since)? else {
                continue;
            };
            let better = match &best {
                None => true,
                Some(cur) => {
                    snap.remaining_ratio < cur.remaining_ratio
                        || (snap.remaining_ratio == cur.remaining_ratio
                            && snap.session < cur.session)
                }
            };
            if better {
                best = Some(snap);
            }
        }
        Ok(best)
    }
}

/// `budget_meta` 中所有**已配置**（至少一个阈值非空）的预算：`(session, tokens_cap, usd_cap, updated_ms)`。
fn configured_budgets_locked(
    conn: &Connection,
) -> rusqlite::Result<Vec<(String, u64, f64, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT session, threshold_tokens, threshold_usd, updated_ms
         FROM budget_meta WHERE threshold_tokens IS NOT NULL OR threshold_usd IS NOT NULL",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let session: String = r.get(0)?;
            let tokens: Option<i64> = r.get(1)?;
            let usd: Option<f64> = r.get(2)?;
            let updated: i64 = r.get(3)?;
            Ok((
                session,
                tokens.unwrap_or(0).max(0) as u64,
                usd.unwrap_or(0.0).max(0.0),
                updated,
            ))
        })?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    Ok(rows)
}

/// 由上限 + 计量起点算出的实测快照（已用 = 起点以来的真实台账事件）。
fn snapshot_of_locked(
    conn: &Connection,
    session: &str,
    tokens_cap: u64,
    usd_cap: f64,
    since_ms: i64,
) -> rusqlite::Result<Option<BudgetSnapshot>> {
    if tokens_cap == 0 && usd_cap <= 0.0 {
        return Ok(None); // 两个口径都无上限 = 未配置预算，不得假装有
    }
    let (tokens_used, usd_used) = spend_locked(conn, Some(session), since_ms)?;
    Ok(Some(BudgetSnapshot {
        session: session.to_string(),
        tokens_cap,
        usd_cap,
        tokens_used: tokens_used.max(0) as u64,
        usd_used,
        remaining_ratio: BudgetSnapshot::ratio(tokens_cap, tokens_used.max(0) as u64, usd_cap, usd_used),
        since_ms,
    }))
}

/// 全局（最紧）已配置预算的实测快照；台账未初始化或无任何预算配置时 `None`。
pub fn tightest_budget_snapshot() -> Option<BudgetSnapshot> {
    LEDGER
        .get()
        .and_then(|l| l.tightest_budget_snapshot().ok().flatten())
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

/// 当前 turn 的会话键（task-local；无上下文或空串时返回 `None`）。
///
/// 为什么需要公开：`tokio::spawn` **不会**继承 task-local。流式 LLM 的生产者
/// 跑在独立任务里，`usage::record` 在那里读到的是空串 —— 结果所有流式用量都归到
/// 空会话（真机：`usage_events.session=''`，按会话统计全错）。生产者必须在
/// spawn **之前**把归属取出来带走。
pub fn current_session() -> Option<String> {
    SESSION
        .try_with(|s| s.clone())
        .ok()
        .filter(|s| !s.trim().is_empty())
}

/// 当前 turn 的 run 归因（同上，用于跨 `tokio::spawn` 携带）。
pub fn current_run() -> Option<String> {
    RUN_ID
        .try_with(|r| r.clone())
        .ok()
        .filter(|r| !r.trim().is_empty())
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
                usage.cache_read_input_tokens,
                usage.cache_creation_input_tokens,
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

/// 最近 N 条明细（倒序）。独立函数是因为 `query_contract` 已持有连接锁，不能重入。
fn query_recent(conn: &Connection, limit: i64) -> rusqlite::Result<Value> {
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
    Ok(Value::Array(items))
}

/// `usage.query` 聚合扫描的一行（按 本地日 × 会话 × 模型 分组）。
struct UsageAggRow {
    day: String,
    session: String,
    model: String,
    calls: i64,
    input: i64,
    output: i64,
    total: i64,
    cache_read: i64,
    cache_write: i64,
    first_ms: i64,
    last_ms: i64,
}

/// 累加器：把若干 [`UsageAggRow`] 折叠成前端 `normalizeUsageTotals` 认得的形状。
#[derive(Default)]
struct UsageTotalsAcc {
    calls: i64,
    input: i64,
    output: i64,
    total: i64,
    cache_read: i64,
    cache_write: i64,
    cost_nanos: i64,
}

impl UsageTotalsAcc {
    fn add(&mut self, row: &UsageAggRow) {
        self.calls += row.calls;
        self.input += row.input;
        self.output += row.output;
        self.total += row.total;
        self.cache_read += row.cache_read;
        self.cache_write += row.cache_write;
        // 与 `task_cost_view` 同口径（同一份 `price_for` 单价表），避免两处账单打架。
        let (p_in, p_out) = price_for(&row.model);
        let usd = row.input as f64 / 1_000_000.0 * p_in
            + row.output as f64 / 1_000_000.0 * p_out;
        self.cost_nanos += (usd * 1_000_000_000.0).round() as i64;
    }

    fn to_json(&self, sessions: usize) -> Value {
        json!({
            "inputTokens": self.input,
            "outputTokens": self.output,
            "totalTokens": if self.total > 0 { self.total } else { self.input + self.output },
            "cacheReadTokens": self.cache_read,
            "cacheWriteTokens": self.cache_write,
            "costNanos": self.cost_nanos,
            // 前端 `costSourceKey` 只认 `Marguerite_estimate` / `provider_billed` 等
            // 固定枚举；写 `estimated` 会落进 `default → none`，明细表「来源」列显示「无」。
            "costSource": "Marguerite_estimate",
            "sessionCount": sessions as i64,
            "eventCount": self.calls,
            "missingCostEntries": 0,
        })
    }
}

/// `usage.query` 契约：前端用量页的唯一数据源。
///
/// 参数（与 webui `useUsageQuery.queryParams` 一一对应）：
/// ```json
/// { "schemaVersion": 1, "range": { "preset": "today|last_7_calendar_days|all" },
///   "timezone": "Asia/Shanghai", "include": { "days": true, "models": true, "sessions": true } }
/// ```
/// 返回 `{ totals, days, models, sessions, coverage, range, asOfMs, items }`。
///
/// **不能用 `{items}` 糊弄**：前端 `normalizeUsageQueryResponse` 只读 `totals` 等
/// 字段，只返回 items 会让整页 KPI 归零（真机缺陷，见 `query_contract` 注释）。
pub fn query(params: &Value) -> Value {
    let preset = params
        .pointer("/range/preset")
        .and_then(Value::as_str)
        .unwrap_or("all");
    let timezone = params
        .get("timezone")
        .and_then(Value::as_str)
        .unwrap_or("UTC");
    let include_bool = |key: &str| {
        params
            .pointer(&format!("/include/{key}"))
            .and_then(Value::as_bool)
            .unwrap_or(true)
    };
    let limit = params
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(200)
        .clamp(1, 500);
    let from_ms = range_from_ms(preset);
    match LEDGER.get() {
        None => json!({
            "schemaVersion": 1,
            "asOfMs": now_ms(),
            "range": { "preset": preset, "timezone": timezone, "fromMs": from_ms, "toMs": null },
            "totals": UsageTotalsAcc::default().to_json(0),
            "days": [], "models": [], "sessions": [], "items": [],
            "coverage": { "status": "complete", "reasonCodes": ["ledger_unavailable"], "legacyUnattributed": null },
        }),
        Some(ledger) => ledger
            .query_contract(
                from_ms,
                UsageQueryOptions {
                    preset,
                    timezone,
                    include_days: include_bool("days"),
                    include_models: include_bool("models"),
                    include_sessions: include_bool("sessions"),
                    item_limit: limit,
                },
            )
            .unwrap_or_else(|e| {
                tracing::warn!(err = %e, "用量台账查询失败");
                json!({
                    "schemaVersion": 1,
                    "asOfMs": now_ms(),
                    "range": { "preset": preset, "timezone": timezone, "fromMs": from_ms, "toMs": null },
                    "totals": UsageTotalsAcc::default().to_json(0),
                    "days": [], "models": [], "sessions": [], "items": [],
                    "coverage": { "status": "partial", "reasonCodes": ["ledger_query_failed"], "legacyUnattributed": null },
                })
            }),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 预设范围 → 本地时区「今天/N 天前」的零点毫秒。
///
/// `all` / 未知预设返回 `None`（不限时间）。日边界用 `chrono::Local`，与用户
/// 看到的「今天」一致；SQL 只拿到毫秒阈值，不做时区算术。
fn range_from_ms(preset: &str) -> Option<i64> {
    use chrono::{Local, TimeZone};

    let days: i64 = match preset {
        "today" => 1,
        "last_7_calendar_days" => 7,
        "last_14_calendar_days" => 14,
        "last_30_calendar_days" => 30,
        _ => return None,
    };
    let today = Local::now().date_naive();
    let start = today - chrono::Duration::days(days - 1);
    let naive = start.and_hms_opt(0, 0, 0)?;
    Local
        .from_local_datetime(&naive)
        .single()
        .map(|dt| dt.timestamp_millis())
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

    /// 预算「已用」必须由**真实台账事件**算出，且计量起点 = 预算设定时刻
    /// （审计：SessionBudget 的已用字段全仓无人写入 ⇒ 剩余恒 100% 的假读数）。
    #[test]
    fn budget_snapshot_counts_only_events_after_budget_set() {
        let ledger = tmp_db("budget-since");
        // 预算设定**之前**的历史用量：不得计入（否则一设预算就"已用"一大截）
        ledger
            .record_attributed(None, "s1", "deepseek-v4-pro", &usage(1_000, 0))
            .unwrap();
        ledger.set_session_budget("s1", Some(100_000), None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        ledger
            .record_attributed(None, "s1", "deepseek-v4-pro", &usage(2_000, 1_000))
            .unwrap();

        let snap = ledger
            .budget_snapshot_for("s1")
            .unwrap()
            .expect("已配置预算 → 必须给快照");
        assert_eq!(snap.tokens_cap, 100_000);
        assert_eq!(snap.tokens_used, 3_000, "只计设定之后的 3000 token");
        assert!(
            (snap.remaining_ratio - 0.97).abs() < 1e-9,
            "剩余比例必须实测: {}",
            snap.remaining_ratio
        );

        // 别的会话用量不得混入本会话预算
        ledger
            .record_attributed(None, "s2", "deepseek-v4-pro", &usage(50_000, 0))
            .unwrap();
        let again = ledger.budget_snapshot_for("s1").unwrap().unwrap();
        assert_eq!(again.tokens_used, 3_000, "跨会话不得串味");
    }

    /// 未配置预算 → 必须是 `None`，不得假装有预算（否则会凭默认值私自降档）。
    #[test]
    fn budget_snapshot_is_none_without_configured_budget() {
        let ledger = tmp_db("budget-none");
        ledger
            .record_attributed(None, "s1", "deepseek-v4-pro", &usage(500, 0))
            .unwrap();
        assert!(ledger.budget_snapshot_for("s1").unwrap().is_none());
        assert!(
            ledger.tightest_budget_snapshot().unwrap().is_none(),
            "无任何配置时全局视图也必须是 None"
        );
    }

    /// 多会话各自配了预算 → 全局视图取**最紧**的那个（剩余比例最小）。
    #[test]
    fn tightest_budget_renders_the_tightest_session() {
        let ledger = tmp_db("budget-tightest");
        ledger.set_session_budget("loose", Some(1_000_000), None).unwrap();
        ledger.set_session_budget("tight", Some(1_000), None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        ledger
            .record_attributed(None, "tight", "deepseek-v4-pro", &usage(900, 0))
            .unwrap();
        ledger
            .record_attributed(None, "loose", "deepseek-v4-pro", &usage(10, 0))
            .unwrap();

        let snap = ledger.tightest_budget_snapshot().unwrap().expect("应有快照");
        assert_eq!(snap.session, "tight", "必须选中吃紧的那个会话");
        assert!(snap.remaining_ratio < 0.2, "ratio={}", snap.remaining_ratio);
    }

    /// 美元口径用**同一张单价表**估算，避免「预算按 A 价、账单按 B 价」双口径。
    #[test]
    fn budget_snapshot_uses_price_table_for_usd_cap() {
        let ledger = tmp_db("budget-usd");
        ledger.set_session_budget("s1", None, Some(1.0)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        // gpt-4o 输入 $2.5/MTok → 100k 输入 ≈ $0.25
        ledger
            .record_attributed(None, "s1", "gpt-4o", &usage(100_000, 0))
            .unwrap();
        let snap = ledger.budget_snapshot_for("s1").unwrap().unwrap();
        assert!((snap.usd_used - 0.25).abs() < 1e-6, "usd_used={}", snap.usd_used);
        assert!(
            (snap.remaining_ratio - 0.75).abs() < 1e-6,
            "ratio={}",
            snap.remaining_ratio
        );
    }

    /// `usage.query` 必须返回前端认得的四视图（totals/sessions/models/days）。
    ///
    /// 回归的缺陷：旧实现只回 `{items}`，前端 `normalizeUsageQueryResponse` 读不到
    /// `totals` → `/usage` 整页 KPI 归零（台账明明有数据，页面显示 0 token）。
    #[test]
    fn query_contract_aggregates_four_views_for_webui() {
        let ledger = tmp_db("query-contract");
        ledger
            .record_attributed(None, "s-a", "deepseek-chat", &usage(1000, 200))
            .unwrap();
        ledger
            .record_attributed(None, "s-a", "deepseek-chat", &usage(500, 100))
            .unwrap();
        ledger
            .record_attributed(None, "s-b", "deepseek-v4-pro", &usage(2000, 300))
            .unwrap();

        let v = ledger
            .query_contract(
                None,
                UsageQueryOptions {
                    preset: "all",
                    timezone: "Asia/Shanghai",
                    include_days: true,
                    include_models: true,
                    include_sessions: true,
                    item_limit: 10,
                },
            )
            .expect("query contract");

        // totals：三次调用合计
        assert_eq!(v["totals"]["inputTokens"], 3500);
        assert_eq!(v["totals"]["outputTokens"], 600);
        assert_eq!(v["totals"]["totalTokens"], 4100);
        assert_eq!(v["totals"]["eventCount"], 3);
        assert_eq!(v["totals"]["sessionCount"], 2);
        assert!(v["totals"]["costNanos"].as_i64().unwrap_or(0) > 0, "成本必须有估算值");

        // sessions：两个会话，合计守恒
        let sessions = v["sessions"].as_array().expect("sessions 数组");
        assert_eq!(sessions.len(), 2);
        let sum_in: i64 = sessions
            .iter()
            .map(|s| s["totals"]["inputTokens"].as_i64().unwrap_or(0))
            .sum();
        assert_eq!(sum_in, 3500, "会话合计必须等于总量");

        // 明细表三列（修改时间 / 来源 / 模型）必须真的有字段，不能全是 `-`。
        let sa = sessions
            .iter()
            .find(|s| s["sessionKey"] == "s-a")
            .expect("应有 s-a 会话");
        assert!(sa["lastUsageAtMs"].as_i64().unwrap_or(0) > 0, "缺 lastUsageAtMs");
        assert!(sa["firstUsageAtMs"].as_i64().unwrap_or(0) > 0, "缺 firstUsageAtMs");
        assert_eq!(sa["costSource"], "Marguerite_estimate");
        let breakdown = sa["modelBreakdown"].as_array().expect("modelBreakdown 数组");
        assert_eq!(breakdown.len(), 1);
        assert_eq!(breakdown[0]["model"], "deepseek-chat");

        // models：两个模型
        let models = v["models"].as_array().expect("models 数组");
        assert_eq!(models.len(), 2);
        let pro = models
            .iter()
            .find(|m| m["model"] == "deepseek-v4-pro")
            .expect("应有 deepseek-v4-pro");
        assert_eq!(pro["totals"]["inputTokens"], 2000);
        assert_eq!(pro["sessionCount"], 1);

        // days：今天一条
        let days = v["days"].as_array().expect("days 数组");
        assert_eq!(days.len(), 1);
        assert_eq!(days[0]["totals"]["totalTokens"], 4100);

        // 兼容：items 仍在（审计脚本按它读明细）
        assert_eq!(v["items"].as_array().map(|a| a.len()), Some(3));
        // 范围元数据
        assert_eq!(v["range"]["preset"], "all");
        assert!(v["range"]["fromMs"].is_null());
        assert_eq!(v["coverage"]["status"], "complete");
    }

    /// 预设范围必须真的过滤（`today` 只算今天，历史行不能被算进来）。
    #[test]
    fn query_contract_respects_range_preset() {
        let ledger = tmp_db("query-range");
        ledger
            .record_attributed(None, "s-today", "deepseek-chat", &usage(100, 10))
            .unwrap();
        // 手工插一条 40 天前的历史行
        {
            let conn = ledger.conn.lock().unwrap();
            let old_ms = now_ms() - 40 * 86_400_000;
            conn.execute(
                "INSERT INTO usage_events (run_id, session, model, prompt_tokens, completion_tokens, total_tokens, cache_read_tokens, cache_creation_tokens, created_ms)
                 VALUES (NULL, 's-old', 'deepseek-chat', 9999, 1, 10000, 0, 0, ?1)",
                rusqlite::params![old_ms],
            )
            .unwrap();
        }
        let all = ledger
            .query_contract(
                None,
                UsageQueryOptions {
                    preset: "all",
                    timezone: "UTC",
                    include_days: false,
                    include_models: false,
                    include_sessions: false,
                    item_limit: 10,
                },
            )
            .unwrap();
        let today = ledger
            .query_contract(
                range_from_ms("today"),
                UsageQueryOptions {
                    preset: "today",
                    timezone: "UTC",
                    include_days: false,
                    include_models: false,
                    include_sessions: false,
                    item_limit: 10,
                },
            )
            .unwrap();
        assert_eq!(all["totals"]["inputTokens"], 10099);
        assert_eq!(today["totals"]["inputTokens"], 100, "today 不得包含 40 天前的行");
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
