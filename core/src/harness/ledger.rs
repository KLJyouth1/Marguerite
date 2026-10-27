//! Harness 运行台账（SQLite，`harness.db`）
//!
//! 数据链路对齐 Codex 的 `thread → turn → item` 三级事件：
//! `session_id`（会话）→ `run_id`（一次隔离执行）→ `id/seq`（追加写入的条目）。
//!
//! 设计约束：
//! - **追加写**：`harness_items` 由 SQLite trigger 拒绝 UPDATE / DELETE，
//!   与 [`crate::audit_store`] 的 append-only 同思路（同一防线，两套数据）。
//! - **诚实迁移**：`PRAGMA user_version` 不认识的高版本直接报错，不静默降级。
//! - **无 panic**：所有公开 API 返回 `Result`；调用方一律 best-effort，
//!   台账写失败绝不能影响主流程。
//! - **确定性回放**：`replay_digest` 只串接业务字段（`seq|type|name|status|tokens_in|
//!   tokens_out|elapsed_ms`），**不含时间戳**。依据 DeepInfer 的 batch-invariant /
//!   确定性采样思想：同一输入序列必须得到同一摘要，否则「回放」只是换个名字的日志。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

/// 当前 schema 版本（见模块头的迁移约定）
pub const SCHEMA_VERSION: i64 = 2;

/// 单 run 查询时返回的 items 上限
pub const MAX_ITEMS_PER_RUN: usize = 200;

/// 合法验收结论（与「完成门禁」的四个取值一致）。
///
/// - `passed`：有可复现证据（命令输出 / 产物回查）证明达成；
/// - `failed`：证据表明未达成；
/// - `unverified`：跑完了但**没有**验证证据（默认值，绝不拿 `status=ok` 冒充通过）；
/// - `unknown`：无法判定（例如外部系统超时、证据缺失）。
pub const VERDICTS: [&str; 4] = ["passed", "failed", "unverified", "unknown"];

/// 是否合法结论
pub fn is_valid_verdict(verdict: &str) -> bool {
    VERDICTS.contains(&verdict)
}

/// run 落终态时推导结论：**只有显式验证**才能把 run 标成 `passed`，
/// 这里绝不假设「跑完了 = 达成」。
fn derive_verdict(status: &str) -> &'static str {
    if status == "ok" {
        "unverified"
    } else {
        "failed"
    }
}

/// `target` 列只存任务摘要，不存全文（隐私 + 体积防线：长任务 prompt 可达数十 KB）
pub const MAX_TARGET_CHARS: usize = 160;

/// 截断任务文本为可读摘要（按字符而非字节，避免切坏 CJK）
fn clamp_target(target: &str) -> String {
    let t = target.trim();
    if t.chars().count() <= MAX_TARGET_CHARS {
        return t.to_string();
    }
    let mut s: String = t.chars().take(MAX_TARGET_CHARS).collect();
    s.push('…');
    s
}

/// 一次工具/审批/检查点条目的写入形态
#[derive(Debug, Clone, Default)]
pub struct ItemRecord {
    /// 所属 run（`Harness::run_isolated` 的 run_id；纯工具钩子可留空）
    pub run_id: String,
    /// 所属会话
    pub session_id: String,
    /// 条目类型：tool_call | approval | checkpoint | hook | error | ...
    pub item_type: String,
    /// 条目名（工具名 / 检查点名 / 钩子名）
    pub name: String,
    /// 状态（默认 ok）
    pub status: String,
    /// 耗时（毫秒）
    pub elapsed_ms: u64,
    /// 入参 token
    pub tokens_in: u64,
    /// 出参 token
    pub tokens_out: u64,
    /// 是否成功
    pub ok: bool,
    /// 失败错误码
    pub error_code: Option<String>,
}

impl ItemRecord {
    /// 构造一条最小条目（默认 `status="ok"` / `ok=true`）
    pub fn new(item_type: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            run_id: String::new(),
            session_id: String::new(),
            item_type: item_type.into(),
            name: name.into(),
            status: "ok".to_string(),
            elapsed_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
            ok: true,
            error_code: None,
        }
    }

    /// 指定所属会话
    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = session_id.into();
        self
    }

    /// 指定所属 run
    pub fn with_run(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = run_id.into();
        self
    }

    /// 指定耗时 / token / 成败
    pub fn with_metrics(
        mut self,
        elapsed_ms: u64,
        tokens_in: u64,
        tokens_out: u64,
        ok: bool,
    ) -> Self {
        self.elapsed_ms = elapsed_ms;
        self.tokens_in = tokens_in;
        self.tokens_out = tokens_out;
        self.ok = ok;
        self.status = if ok { "ok".into() } else { "error".into() };
        self
    }

    /// 指定失败错误码
    pub fn with_error(mut self, code: impl Into<String>) -> Self {
        self.error_code = Some(code.into());
        self
    }
}

/// 一条 item 的读回形态（`seq` 即自增主键 id）
#[derive(Debug, Clone)]
pub struct ItemRow {
    pub seq: i64,
    pub item_type: String,
    pub name: String,
    pub status: String,
    pub elapsed_ms: i64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub ok: bool,
    pub error_code: Option<String>,
}

/// 一条 run 的读回形态（含条目与截断标记）
#[derive(Debug, Clone)]
pub struct RunRow {
    pub run_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub target: String,
    pub status: String,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub elapsed_ms: Option<i64>,
    pub prompt_tokens: i64,
    pub output_tokens: i64,
    pub model: String,
    pub policy: String,
    /// 验收结论：`passed | failed | unverified | unknown`（见 [`VERDICTS`]）
    pub verdict: String,
    pub item_count: i64,
    pub items: Vec<ItemRow>,
    /// 该 run 的 items 超过 [`MAX_ITEMS_PER_RUN`]
    pub items_truncated: bool,
}

/// 台账总览（`harness.ledger.status` 的数据源）
#[derive(Debug, Clone, Default)]
pub struct LedgerStatus {
    pub runs: i64,
    pub items: i64,
    pub bytes: u64,
    pub oldest_ts_ms: Option<i64>,
    pub newest_ts_ms: Option<i64>,
    /// 真实计数：已落终态的 run 里 `status="ok"` 的百分比（0..100）
    pub success_rate: f64,
    pub avg_elapsed_ms: f64,
    pub p95_elapsed_ms: f64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub last_error: Option<String>,
    /// 验收结论分布（`passed / failed / unverified / unknown` 的真实计数）
    pub verdicts: std::collections::BTreeMap<String, i64>,
}

/// 回放摘要
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayDigest {
    pub digest: String,
    pub item_count: i64,
}

/// SQLite 运行台账
pub struct HarnessLedger {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl HarnessLedger {
    /// 打开（或创建）指定路径的台账库
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(&path)?;

        // WAL + NORMAL：台账是旁路观测数据，换取出入吞吐；崩溃丢最后一条可接受。
        // journal_mode 会返回一行，必须用 query_row 读，不能用 execute_batch。
        let mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            tracing::debug!(mode = %mode, "harness 台账未能切到 WAL，继续以当前模式运行");
        }
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;

        let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            bail!(
                "unsupported schema version {version}（harness.db 由更高版本写入，本引擎仅支持 {SCHEMA_VERSION}）"
            );
        }
        // 0：全新库；1：v1 旧库（无 verdict 列）；2：当前版本。
        // 迁移必须**先于** DDL：`idx_harness_runs_verdict` 引用 verdict 列，
        // 在旧库上先建索引会直接 SQL 报错（真机测试捕获）。
        if version == 1 {
            // v1 → v2：补 `verdict` 列。新库由 DDL 直接带列，这里只处理旧库；
            // 用 table_info 探测而不是盲目 ALTER，避免「DDL 已建好列」时重复加列报错。
            let has_verdict = conn
                .prepare("PRAGMA table_info(harness_runs)")?
                .query_map([], |row| row.get::<_, String>(1))?
                .filter_map(Result::ok)
                .any(|name| name == "verdict");
            if !has_verdict {
                conn.execute_batch(
                    "ALTER TABLE harness_runs ADD COLUMN verdict TEXT NOT NULL DEFAULT 'unverified';",
                )?;
            }
        }
        // 幂等 DDL（防止「version 写了但表缺失」）
        conn.execute_batch(SCHEMA_DDL)?;
        if version < SCHEMA_VERSION {
            conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
        }

        Ok(Self {
            conn: Mutex::new(conn),
            path,
        })
    }

    /// 默认路径：`Config::data_dir()/harness.db`
    ///
    /// **测试态**重定向到进程级临时文件：`run_isolated` / usage 钩子的单测会真实写入，
    /// 绝不能污染用户的 `harness.db`（与 `usage::ledger_path` 同一纪律）。
    pub fn open_default() -> Result<Self> {
        Self::open(default_path())
    }

    /// 数据库文件路径
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| anyhow!("harness 台账连接锁已中毒"))
    }

    /// 记录一次 run 开始（`status="running"`）
    pub fn record_run_start(
        &self,
        run_id: &str,
        session_id: &str,
        trace_id: &str,
        target: &str,
        model: &str,
        policy: &str,
    ) -> Result<()> {
        let conn = self.lock()?;
        let target = clamp_target(target);
        conn.execute(
            "INSERT INTO harness_runs
                (run_id, session_id, trace_id, target, status, started_at_ms, model, policy)
             VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6, ?7)",
            params![
                run_id,
                session_id,
                trace_id,
                target,
                now_ms(),
                model,
                policy
            ],
        )?;
        Ok(())
    }

    /// 记录一次 run 结束：回填状态 / 耗时 / token / 模型 / 错误码。
    ///
    /// run 行不存在时**不补写幽灵行**（那会让台账伪造一次没发生的执行），直接 no-op。
    pub fn record_run_finish(
        &self,
        run_id: &str,
        status: &str,
        prompt_tokens: u64,
        output_tokens: u64,
        model: &str,
        error_code: Option<&str>,
    ) -> Result<()> {
        let conn = self.lock()?;
        let started: Option<i64> = conn
            .query_row(
                "SELECT started_at_ms FROM harness_runs WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(started) = started else {
            return Ok(());
        };
        let ended = now_ms();
        let elapsed = (ended - started).max(0);
        conn.execute(
            "UPDATE harness_runs SET
                status = ?2,
                ended_at_ms = ?3,
                elapsed_ms = ?4,
                prompt_tokens = ?5,
                output_tokens = ?6,
                model = ?7,
                error_code = ?8,
                verdict = ?9
             WHERE run_id = ?1",
            params![
                run_id,
                status,
                ended,
                elapsed,
                saturate(prompt_tokens),
                saturate(output_tokens),
                model,
                error_code,
                derive_verdict(status)
            ],
        )?;
        Ok(())
    }

    /// 显式写入验收结论（由完成门禁 / 人工复核调用）。
    ///
    /// 返回 `Ok(false)` 表示 run 不存在 —— **不补写幽灵行**（那会伪造一次没发生的执行）。
    pub fn set_verdict(&self, run_id: &str, verdict: &str) -> Result<bool> {
        if !is_valid_verdict(verdict) {
            bail!("非法 verdict `{verdict}`，仅支持 {VERDICTS:?}");
        }
        let conn = self.lock()?;
        let changed = conn.execute(
            "UPDATE harness_runs SET verdict = ?2 WHERE run_id = ?1",
            params![run_id, verdict],
        )?;
        Ok(changed > 0)
    }

    /// 追加一条 item，返回其 `seq`（全局单调自增 id）
    pub fn record_item(&self, item: ItemRecord) -> Result<i64> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO harness_items
                (run_id, session_id, item_type, name, status, elapsed_ms,
                 tokens_in, tokens_out, ok, error_code)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                item.run_id,
                item.session_id,
                item.item_type,
                item.name,
                item.status,
                saturate(item.elapsed_ms),
                saturate(item.tokens_in),
                saturate(item.tokens_out),
                if item.ok { 1i64 } else { 0i64 },
                item.error_code
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// 总览：真实计数 + 文件体积 + 真实耗时统计
    pub fn status(&self) -> Result<LedgerStatus> {
        let conn = self.lock()?;
        let runs: i64 = conn.query_row("SELECT COUNT(*) FROM harness_runs", [], |r| r.get(0))?;
        let items: i64 = conn.query_row("SELECT COUNT(*) FROM harness_items", [], |r| r.get(0))?;
        let oldest: Option<i64> = conn
            .query_row("SELECT MIN(started_at_ms) FROM harness_runs", [], |r| {
                r.get(0)
            })
            .optional()?
            .flatten();
        let newest: Option<i64> = conn
            .query_row("SELECT MAX(started_at_ms) FROM harness_runs", [], |r| {
                r.get(0)
            })
            .optional()?
            .flatten();

        let finished: i64 = conn.query_row(
            "SELECT COUNT(*) FROM harness_runs WHERE ended_at_ms IS NOT NULL",
            [],
            |r| r.get(0),
        )?;
        let ok_runs: i64 = conn.query_row(
            "SELECT COUNT(*) FROM harness_runs WHERE ended_at_ms IS NOT NULL AND status = 'ok'",
            [],
            |r| r.get(0),
        )?;
        let success_rate = if finished > 0 {
            ok_runs as f64 * 100.0 / finished as f64
        } else {
            0.0
        };

        let mut elapsed: Vec<i64> = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT elapsed_ms FROM harness_runs WHERE elapsed_ms IS NOT NULL ORDER BY elapsed_ms ASC",
            )?;
            let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
            for v in rows {
                elapsed.push(v?);
            }
        }
        let avg_elapsed_ms = if elapsed.is_empty() {
            0.0
        } else {
            elapsed.iter().sum::<i64>() as f64 / elapsed.len() as f64
        };
        let p95_elapsed_ms = percentile95(&elapsed);

        let tokens_in: i64 = conn.query_row(
            "SELECT COALESCE(SUM(prompt_tokens), 0) FROM harness_runs",
            [],
            |r| r.get(0),
        )?;
        let tokens_out: i64 = conn.query_row(
            "SELECT COALESCE(SUM(output_tokens), 0) FROM harness_runs",
            [],
            |r| r.get(0),
        )?;

        // 最近一次错误：先看 run 级，再退到 item 级（工具调用失败）
        let last_error: Option<String> = conn
            .query_row(
                "SELECT error_code FROM harness_runs
                 WHERE error_code IS NOT NULL AND error_code <> ''
                 ORDER BY started_at_ms DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?
            .or_else(|| {
                conn.query_row(
                    "SELECT error_code FROM harness_items
                     WHERE error_code IS NOT NULL AND error_code <> ''
                     ORDER BY id DESC LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .optional()
                .ok()
                .flatten()
            });

        let bytes = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);

        // 验收结论分布：四个取值都建键（缺的报 0），前端不必自己补默认值。
        let mut verdicts: std::collections::BTreeMap<String, i64> = VERDICTS
            .iter()
            .map(|v| (v.to_string(), 0))
            .collect();
        {
            let mut stmt = conn.prepare("SELECT verdict, COUNT(*) FROM harness_runs GROUP BY verdict")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
            for row in rows {
                let (k, v) = row?;
                verdicts.insert(k, v);
            }
        }

        Ok(LedgerStatus {
            runs,
            items,
            bytes,
            oldest_ts_ms: oldest,
            newest_ts_ms: newest,
            success_rate,
            avg_elapsed_ms,
            p95_elapsed_ms,
            tokens_in,
            tokens_out,
            last_error,
            verdicts,
        })
    }

    /// 查询 run（可按 session / run_id 过滤），每个 run 附带最多
    /// [`MAX_ITEMS_PER_RUN`] 条 items。
    ///
    /// `limit` 会被夹到 `1..=100`；返回值的 `truncated` 同时覆盖
    /// 「run 列表被 limit 截断」与「某 run 的 items 被截断」。
    pub fn query(
        &self,
        session_key: Option<&str>,
        run_id: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<RunRow>, bool)> {
        self.query_filtered(session_key, run_id, None, limit)
    }

    /// 同 [`Self::query`]，额外支持按验收结论过滤（`verdict=None` 表示不过滤）。
    ///
    /// SQL 用动态拼接 + `Value` 参数，避免「固定占位符 + 空串占位」那种易错写法。
    pub fn query_filtered(
        &self,
        session_key: Option<&str>,
        run_id: Option<&str>,
        verdict: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<RunRow>, bool)> {
        let limit = limit.clamp(1, 100);
        let conn = self.lock()?;

        // 动态 WHERE：只拼固定片段，值一律走参数绑定。
        let mut sql = String::from(
            "SELECT run_id, session_id, trace_id, target, status, started_at_ms,
                    ended_at_ms, elapsed_ms, prompt_tokens, output_tokens, model, policy, verdict
             FROM harness_runs WHERE 1 = 1",
        );
        let mut args: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(rid) = run_id {
            sql.push_str(" AND run_id = ?");
            args.push(rusqlite::types::Value::Text(rid.to_string()));
        }
        if let Some(sk) = session_key {
            sql.push_str(" AND session_id = ?");
            args.push(rusqlite::types::Value::Text(sk.to_string()));
        }
        if let Some(v) = verdict {
            if !is_valid_verdict(v) {
                bail!("非法 verdict `{v}`，仅支持 {VERDICTS:?}");
            }
            sql.push_str(" AND verdict = ?");
            args.push(rusqlite::types::Value::Text(v.to_string()));
        }
        sql.push_str(" ORDER BY started_at_ms DESC LIMIT ?");
        args.push(rusqlite::types::Value::Integer(limit as i64));

        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(args.iter()))?;

        let mut runs = Vec::new();
        let mut truncated = false;
        while let Some(row) = rows.next()? {
            let run_id: String = row.get(0)?;
            let item_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM harness_items WHERE run_id = ?1",
                params![run_id],
                |r| r.get(0),
            )?;
            let mut items = Vec::new();
            {
                let mut istmt = conn.prepare(
                    "SELECT id, item_type, name, status, elapsed_ms, tokens_in, tokens_out, ok, error_code
                     FROM harness_items WHERE run_id = ?1 ORDER BY id ASC LIMIT ?2",
                )?;
                let irows = istmt.query_map(params![run_id, MAX_ITEMS_PER_RUN as i64], |r| {
                    Ok(ItemRow {
                        seq: r.get(0)?,
                        item_type: r.get(1)?,
                        name: r.get(2)?,
                        status: r.get(3)?,
                        elapsed_ms: r.get(4)?,
                        tokens_in: r.get(5)?,
                        tokens_out: r.get(6)?,
                        ok: r.get::<_, i64>(7)? != 0,
                        error_code: r.get(8)?,
                    })
                })?;
                for it in irows {
                    items.push(it?);
                }
            }
            let items_truncated = item_count > items.len() as i64;
            truncated |= items_truncated;
            runs.push(RunRow {
                run_id,
                session_id: row.get(1)?,
                trace_id: row.get(2)?,
                target: row.get(3)?,
                status: row.get(4)?,
                started_at_ms: row.get(5)?,
                ended_at_ms: row.get(6)?,
                elapsed_ms: row.get(7)?,
                prompt_tokens: row.get(8)?,
                output_tokens: row.get(9)?,
                model: row.get(10)?,
                policy: row.get(11)?,
                verdict: row.get(12)?,
                item_count,
                items,
                items_truncated,
            });
        }

        // run 列表是否被 limit 截断（比总命中数更严格的「恰好等于上限」也算潜在截断）
        if !truncated && runs.len() == limit {
            truncated = true;
        }

        Ok((runs, truncated))
    }

    /// run 是否存在
    pub fn run_exists(&self, run_id: &str) -> Result<bool> {
        let conn = self.lock()?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM harness_runs WHERE run_id = ?1",
            params![run_id],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// 确定性回放摘要：SHA-256 over 按 `id` 升序的
    /// `seq|type|name|status|tokens_in|tokens_out|elapsed_ms\n` 串接。
    ///
    /// 时间戳**不入摘要**（否则同一输入序列每次重放都不同，摘要就失去意义）。
    /// 依据 DeepInfer 的 batch-invariant / 确定性采样思想：同一输入序列 ⇒ 同一摘要。
    pub fn replay_digest(&self, run_id: &str) -> Result<ReplayDigest> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, item_type, name, status, tokens_in, tokens_out, elapsed_ms
             FROM harness_items WHERE run_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![run_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })?;

        let mut hasher = Sha256::new();
        let mut count = 0i64;
        for row in rows {
            let (seq, item_type, name, status, tokens_in, tokens_out, elapsed_ms) = row?;
            hasher.update(
                format!(
                    "{seq}|{item_type}|{name}|{status}|{tokens_in}|{tokens_out}|{elapsed_ms}\n"
                )
                .as_bytes(),
            );
            count += 1;
        }
        Ok(ReplayDigest {
            digest: format!("{:x}", hasher.finalize()),
            item_count: count,
        })
    }
}

/// 建表 / 索引 / append-only trigger（幂等）
const SCHEMA_DDL: &str = "
CREATE TABLE IF NOT EXISTS harness_runs (
  run_id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL DEFAULT '',
  trace_id TEXT NOT NULL DEFAULT '',
  target TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'running',
  started_at_ms INTEGER NOT NULL,
  ended_at_ms INTEGER,
  elapsed_ms INTEGER,
  prompt_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0,
  model TEXT NOT NULL DEFAULT '',
  policy TEXT NOT NULL DEFAULT '',
  error_code TEXT,
  verdict TEXT NOT NULL DEFAULT 'unverified'
);
CREATE INDEX IF NOT EXISTS idx_harness_runs_session ON harness_runs(session_id, started_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_harness_runs_started ON harness_runs(started_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_harness_runs_verdict ON harness_runs(verdict, started_at_ms DESC);
CREATE TABLE IF NOT EXISTS harness_items (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id TEXT NOT NULL DEFAULT '',
  session_id TEXT NOT NULL DEFAULT '',
  item_type TEXT NOT NULL,
  name TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'ok',
  elapsed_ms INTEGER NOT NULL DEFAULT 0,
  tokens_in INTEGER NOT NULL DEFAULT 0,
  tokens_out INTEGER NOT NULL DEFAULT 0,
  ok INTEGER NOT NULL DEFAULT 1,
  error_code TEXT
);
CREATE INDEX IF NOT EXISTS idx_harness_items_run ON harness_items(run_id, id);
CREATE INDEX IF NOT EXISTS idx_harness_items_session ON harness_items(session_id, id);
CREATE TRIGGER IF NOT EXISTS harness_items_no_update
BEFORE UPDATE ON harness_items BEGIN
  SELECT RAISE(ABORT, 'harness items are append-only');
END;
CREATE TRIGGER IF NOT EXISTS harness_items_no_delete
BEFORE DELETE ON harness_items BEGIN
  SELECT RAISE(ABORT, 'harness items are append-only');
END;
";

/// 全局 lazy 台账（`MARGUERITE_HARNESS_LEDGER=0` 时禁用）
static GLOBAL: OnceLock<Option<HarnessLedger>> = OnceLock::new();

/// 进程级台账实例；禁用或打开失败时返回 `None`（**不 panic**）。
///
/// 打开失败只降级为「无台账」，绝不阻断 harness 主流程；失败原因记一条 warn。
pub fn global() -> Option<&'static HarnessLedger> {
    GLOBAL
        .get_or_init(|| {
            if std::env::var("MARGUERITE_HARNESS_LEDGER").ok().as_deref() == Some("0") {
                return None;
            }
            match HarnessLedger::open_default() {
                Ok(l) => Some(l),
                Err(e) => {
                    tracing::warn!(err = %e, "harness 台账打开失败，已降级为不记录");
                    None
                }
            }
        })
        .as_ref()
}

/// 默认库路径（`cfg(test)` 下重定向到进程级临时文件，绝不碰用户真实数据）
#[cfg(test)]
fn default_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "marguerite-harness-ledger-test-{}.db",
        std::process::id()
    ))
}

#[cfg(not(test))]
fn default_path() -> PathBuf {
    crate::persistence::config::Config::data_dir().join("harness.db")
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// u64 → i64 饱和转换（SQLite INTEGER 是 i64；越界宁可截顶也不 panic）
fn saturate(v: u64) -> i64 {
    v.min(i64::MAX as u64) as i64
}

/// 升序切片上的 nearest-rank p95
fn percentile95(sorted_asc: &[i64]) -> f64 {
    if sorted_asc.is_empty() {
        return 0.0;
    }
    let rank = ((sorted_asc.len() as f64) * 0.95).ceil() as usize;
    let idx = rank.saturating_sub(1).min(sorted_asc.len() - 1);
    sorted_asc[idx] as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_db() -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("marguerite_ledger_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("harness.db");
        (dir, db)
    }

    fn seed_run(l: &HarnessLedger, run_id: &str, session: &str) {
        l.record_run_start(run_id, session, "t-1", "task", "m-1", "balanced")
            .unwrap();
    }

    /// 隐私/体积守卫：`target` 只落摘要，不得把整段任务 prompt 写进台账。
    #[test]
    fn run_target_is_clamped_to_summary() {
        let (dir, db) = temp_db();
        let l = HarnessLedger::open(&db).unwrap();
        let long = "任".repeat(MAX_TARGET_CHARS * 3);
        l.record_run_start("r-clamp", "s-1", "t-1", &long, "m-1", "balanced")
            .unwrap();
        let (runs, _) = l.query(None, Some("r-clamp"), 10).unwrap();
        let t = &runs[0].target;
        assert_eq!(
            t.chars().count(),
            MAX_TARGET_CHARS + 1,
            "长任务必须被截断为 MAX_TARGET_CHARS + 省略号"
        );
        assert!(t.ends_with('…'), "截断后应有可读省略号");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn open_creates_schema_and_sets_version() {
        let (dir, db) = temp_db();
        let l = HarnessLedger::open(&db).unwrap();
        let conn = l.lock().unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'
                 AND name IN ('harness_runs','harness_items')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 2);
        drop(conn);
        drop(l);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reopen_is_idempotent_and_keeps_rows() {
        let (dir, db) = temp_db();
        {
            let l = HarnessLedger::open(&db).unwrap();
            seed_run(&l, "r1", "s1");
        }
        let l = HarnessLedger::open(&db).unwrap();
        let st = l.status().unwrap();
        assert_eq!(st.runs, 1);
        drop(l);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_start_then_finish_backfills_metrics() {
        let (dir, db) = temp_db();
        let l = HarnessLedger::open(&db).unwrap();
        seed_run(&l, "r1", "s1");
        l.record_run_finish("r1", "ok", 120, 45, "m-2", None)
            .unwrap();
        let (runs, _) = l.query(None, Some("r1"), 10).unwrap();
        assert_eq!(runs.len(), 1);
        let r = &runs[0];
        assert_eq!(r.status, "ok");
        assert_eq!(r.prompt_tokens, 120);
        assert_eq!(r.output_tokens, 45);
        assert_eq!(r.model, "m-2");
        assert!(r.ended_at_ms.is_some());
        assert!(r.elapsed_ms.unwrap() >= 0);
        drop(l);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finish_for_unknown_run_writes_nothing() {
        let (dir, db) = temp_db();
        let l = HarnessLedger::open(&db).unwrap();
        l.record_run_finish("ghost", "ok", 1, 1, "m", None).unwrap();
        assert_eq!(l.status().unwrap().runs, 0, "不得补写幽灵 run");
        drop(l);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn items_append_with_monotonic_seq() {
        let (dir, db) = temp_db();
        let l = HarnessLedger::open(&db).unwrap();
        seed_run(&l, "r1", "s1");
        let a = l
            .record_item(ItemRecord::new("tool_call", "file_read").with_run("r1"))
            .unwrap();
        let b = l
            .record_item(ItemRecord::new("tool_call", "terminal_exec").with_run("r1"))
            .unwrap();
        assert!(b > a, "seq 必须全局单调：{a} -> {b}");
        assert_eq!(l.status().unwrap().items, 2);
        drop(l);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn query_filters_by_session() {
        let (dir, db) = temp_db();
        let l = HarnessLedger::open(&db).unwrap();
        seed_run(&l, "r1", "s1");
        seed_run(&l, "r2", "s2");
        let (runs, _) = l.query(Some("s1"), None, 20).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "r1");
        let (all, _) = l.query(None, None, 20).unwrap();
        assert_eq!(all.len(), 2);
        drop(l);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn query_by_run_returns_items_in_seq_order() {
        let (dir, db) = temp_db();
        let l = HarnessLedger::open(&db).unwrap();
        seed_run(&l, "r1", "s1");
        for name in ["a", "b", "c"] {
            l.record_item(ItemRecord::new("hook", name).with_run("r1"))
                .unwrap();
        }
        let (runs, truncated) = l.query(None, Some("r1"), 10).unwrap();
        assert!(!truncated);
        assert_eq!(runs[0].item_count, 3);
        let names: Vec<&str> = runs[0].items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
        drop(l);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn query_truncates_items_at_cap() {
        let (dir, db) = temp_db();
        let l = HarnessLedger::open(&db).unwrap();
        seed_run(&l, "r1", "s1");
        for i in 0..(MAX_ITEMS_PER_RUN + 5) {
            l.record_item(ItemRecord::new("tool_call", format!("t{i}")).with_run("r1"))
                .unwrap();
        }
        let (runs, truncated) = l.query(None, Some("r1"), 10).unwrap();
        assert!(truncated, "超过 200 条必须置 truncated");
        assert_eq!(runs[0].items.len(), MAX_ITEMS_PER_RUN);
        assert_eq!(
            runs[0].item_count,
            (MAX_ITEMS_PER_RUN + 5) as i64,
            "itemCount 必须是真实总数"
        );
        drop(l);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn items_are_append_only_for_update_and_delete() {
        let (dir, db) = temp_db();
        let l = HarnessLedger::open(&db).unwrap();
        seed_run(&l, "r1", "s1");
        let seq = l
            .record_item(ItemRecord::new("tool_call", "x").with_run("r1"))
            .unwrap();
        {
            let conn = l.lock().unwrap();
            let upd = conn.execute(
                "UPDATE harness_items SET name = 'mutated' WHERE id = ?1",
                params![seq],
            );
            assert!(upd.is_err(), "UPDATE 必须被 trigger 拒绝");
            let del = conn.execute("DELETE FROM harness_items WHERE id = ?1", params![seq]);
            assert!(del.is_err(), "DELETE 必须被 trigger 拒绝");
        }
        assert_eq!(l.status().unwrap().items, 1, "被拒绝后条目仍在");
        drop(l);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replay_digest_is_deterministic() {
        let (dir, db) = temp_db();
        let l = HarnessLedger::open(&db).unwrap();
        seed_run(&l, "r1", "s1");
        l.record_item(
            ItemRecord::new("tool_call", "a")
                .with_run("r1")
                .with_metrics(10, 3, 5, true),
        )
        .unwrap();
        l.record_item(
            ItemRecord::new("tool_call", "b")
                .with_run("r1")
                .with_metrics(20, 0, 0, false)
                .with_error("E_B"),
        )
        .unwrap();
        let d1 = l.replay_digest("r1").unwrap();
        let d2 = l.replay_digest("r1").unwrap();
        assert_eq!(d1, d2, "同一输入序列必须得到同一摘要");
        assert_eq!(d1.item_count, 2);
        assert_eq!(d1.digest.len(), 64, "SHA-256 hex 应为 64 字符");

        // 仅时间无关的业务字段变化 ⇒ 摘要必须变化
        l.record_item(
            ItemRecord::new("tool_call", "c")
                .with_run("r1")
                .with_metrics(0, 9, 9, true),
        )
        .unwrap();
        let d3 = l.replay_digest("r1").unwrap();
        assert_ne!(d1.digest, d3.digest, "不同 tokens 必须得到不同摘要");
        drop(l);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_path_does_not_panic_and_reports_error() {
        let (dir, _db) = temp_db();
        let blocker = dir.join("not_a_dir");
        std::fs::write(&blocker, b"x").unwrap();
        // 以「文件」为父目录：create_dir_all 必然失败 → Err，而不是 panic
        let res = HarnessLedger::open(blocker.join("harness.db"));
        assert!(res.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn newer_schema_version_is_rejected() {
        let (dir, db) = temp_db();
        {
            let l = HarnessLedger::open(&db).unwrap();
            let conn = l.lock().unwrap();
            conn.execute_batch(&format!("PRAGMA user_version = {};", SCHEMA_VERSION + 1))
                .unwrap();
        }
        let err = match HarnessLedger::open(&db) {
            Ok(_) => panic!("更高版本必须被拒绝"),
            Err(e) => e.to_string(),
        };
        assert!(
            err.contains("unsupported schema version"),
            "错误应如实说明版本不受支持：{err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_path_is_never_the_real_user_database_under_test() {
        let p = default_path();
        assert!(
            p.to_string_lossy()
                .contains("marguerite-harness-ledger-test-"),
            "测试态台账必须落在可辨识的临时文件上，实际是 {}",
            p.display()
        );
    }

    #[test]
    fn status_reports_real_counts_and_rate() {
        let (dir, db) = temp_db();
        let l = HarnessLedger::open(&db).unwrap();
        seed_run(&l, "r1", "s1");
        l.record_run_finish("r1", "ok", 10, 20, "m", None)
            .unwrap();
        seed_run(&l, "r2", "s1");
        l.record_run_finish("r2", "failed", 5, 0, "m", Some("E2"))
            .unwrap();
        let st = l.status().unwrap();
        assert_eq!(st.runs, 2);
        assert_eq!(st.tokens_in, 15);
        assert_eq!(st.tokens_out, 20);
        assert!((st.success_rate - 50.0).abs() < 1e-9, "1/2 成功 = 50%");
        assert_eq!(st.last_error.as_deref(), Some("E2"));
        assert!(st.oldest_ts_ms.is_some() && st.newest_ts_ms.is_some());
        drop(l);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 结论默认值必须诚实：跑成功 ≠ 验收通过。
    #[test]
    fn verdict_defaults_never_claim_passed() {
        let (dir, db) = temp_db();
        let l = HarnessLedger::open(&db).unwrap();
        seed_run(&l, "r-ok", "s1");
        l.record_run_finish("r-ok", "ok", 1, 1, "m", None).unwrap();
        seed_run(&l, "r-bad", "s1");
        l.record_run_finish("r-bad", "failed", 1, 0, "m", Some("E1"))
            .unwrap();

        let (runs, _) = l.query(None, None, 10).unwrap();
        let ok = runs.iter().find(|r| r.run_id == "r-ok").unwrap();
        let bad = runs.iter().find(|r| r.run_id == "r-bad").unwrap();
        assert_eq!(ok.verdict, "unverified", "没有验证证据不得标 passed");
        assert_eq!(bad.verdict, "failed");

        // 分布四项都必须在，缺的报 0
        let st = l.status().unwrap();
        assert_eq!(st.verdicts.get("unverified"), Some(&1));
        assert_eq!(st.verdicts.get("failed"), Some(&1));
        assert_eq!(st.verdicts.get("passed"), Some(&0));
        assert_eq!(st.verdicts.get("unknown"), Some(&0));
        drop(l);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 显式写结论 + 按结论过滤；非法取值与不存在的 run 都必须被拒绝。
    #[test]
    fn set_verdict_and_filter_by_verdict() {
        let (dir, db) = temp_db();
        let l = HarnessLedger::open(&db).unwrap();
        seed_run(&l, "r1", "s1");
        seed_run(&l, "r2", "s1");
        l.record_run_finish("r1", "ok", 0, 0, "m", None).unwrap();
        l.record_run_finish("r2", "ok", 0, 0, "m", None).unwrap();

        assert!(l.set_verdict("r1", "passed").unwrap(), "存在则返回 true");
        assert!(!l.set_verdict("r-missing", "passed").unwrap(), "不存在时不得补写");
        assert!(l.set_verdict("r2", "bogus").is_err(), "非法结论必须报错");

        let (passed, _) = l
            .query_filtered(None, None, Some("passed"), 10)
            .unwrap();
        assert_eq!(passed.len(), 1);
        assert_eq!(passed[0].run_id, "r1");
        let (unverified, _) = l
            .query_filtered(None, None, Some("unverified"), 10)
            .unwrap();
        assert_eq!(unverified.len(), 1);
        assert_eq!(unverified[0].run_id, "r2");
        assert!(l
            .query_filtered(None, None, Some("not-a-verdict"), 10)
            .is_err());
        drop(l);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// v1 旧库升级：没有 verdict 列时自动补列，且历史行落到 `unverified`。
    #[test]
    fn migrates_v1_database_to_verdict_column() {
        let (dir, db) = temp_db();
        {
            // 手工建一个 v1 库（无 verdict 列，user_version=1）
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE harness_runs (
                    run_id TEXT PRIMARY KEY, session_id TEXT NOT NULL DEFAULT '',
                    trace_id TEXT NOT NULL DEFAULT '', target TEXT NOT NULL DEFAULT '',
                    status TEXT NOT NULL DEFAULT 'running', started_at_ms INTEGER NOT NULL,
                    ended_at_ms INTEGER, elapsed_ms INTEGER,
                    prompt_tokens INTEGER NOT NULL DEFAULT 0,
                    output_tokens INTEGER NOT NULL DEFAULT 0,
                    model TEXT NOT NULL DEFAULT '', policy TEXT NOT NULL DEFAULT '',
                    error_code TEXT);
                 CREATE TABLE harness_items (
                    id INTEGER PRIMARY KEY AUTOINCREMENT, run_id TEXT NOT NULL DEFAULT '',
                    session_id TEXT NOT NULL DEFAULT '', item_type TEXT NOT NULL,
                    name TEXT NOT NULL DEFAULT '', status TEXT NOT NULL DEFAULT 'ok',
                    elapsed_ms INTEGER NOT NULL DEFAULT 0, tokens_in INTEGER NOT NULL DEFAULT 0,
                    tokens_out INTEGER NOT NULL DEFAULT 0, ok INTEGER NOT NULL DEFAULT 1,
                    error_code TEXT);
                 INSERT INTO harness_runs (run_id, session_id, status, started_at_ms)
                    VALUES ('legacy', 's-old', 'ok', 1);
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        }
        let l = HarnessLedger::open(&db).unwrap();
        let (runs, _) = l.query(None, Some("legacy"), 10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].verdict, "unverified", "历史行必须落到未验证");
        let version: i64 = l
            .lock()
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        drop(l);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
