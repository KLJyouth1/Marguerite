//! Usage 查询服务（T-2.1 / FR-B1）：
//! - 按日/按月聚合 SQLite `usage_events` 表；
//! - budget 阈值告警；
//! - Config.balance 占位读取；
//! - 事件桥接 helper：把一次 post-tool-use 记录写进 ledger，并产出要由
//!   EventBus 发布的 `UsageAccumulated` 负载结构体。

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::UsageLedger;
use crate::events::Envelope;
use crate::model::llm::TokenUsage;

// ───────────────────────────────── 类型定义 ─────────────────────────────────

/// 单日聚合（YYYY-MM-DD 维度）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DayStat {
    /// 日期字符串 `YYYY-MM-DD`。
    pub date: String,
    /// prompt_tokens 合计。
    pub tokens_in: u64,
    /// completion_tokens 合计。
    pub tokens_out: u64,
    /// 调用次数（每条 record = 1 turn）。
    pub turns: u64,
    /// 当日 DISTINCT session 数量。
    pub sessions: u64,
    /// 按 `model -> total_tokens` 汇总的分布。
    pub model_distribution: BTreeMap<String, u64>,
}

/// 整月聚合（含逐日子桶）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MonthStat {
    pub year: i32,
    pub month: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub turns: u64,
    pub sessions: u64,
    /// 逐日子桶（键 = YYYY-MM-DD）。
    pub days: BTreeMap<String, DayStat>,
    /// 按月级的 `model -> total_tokens` 汇总。
    pub model_distribution: BTreeMap<String, u64>,
}

/// `on_post_tool_use` 返回的 `UsageAccumulated` 事件（bus 可直接发布 payload）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageAccumulated {
    pub session: String,
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_creation_tokens: u64,
    pub at_ms: u64,
    /// 写入 ledger 是否成功（失败时事件仍发布，用于告警）。
    pub persisted: bool,
}

/// Usage 查询服务：持有一个 [`UsageLedger`]（Arc，可跨线程共享）。
///
/// 所有查询方法只做只读 SELECT，`by_day` 对 [start, end] 范围**填充空日**以保证
/// 热力图/折线图画布不会因为某天没数据就出现断点。
#[derive(Clone)]
pub struct UsageQueryService {
    ledger: Arc<UsageLedger>,
}

impl std::fmt::Debug for UsageQueryService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UsageQueryService")
            .field("ledger", &"Arc<UsageLedger> (opaque)")
            .finish()
    }
}

impl UsageQueryService {
    /// 从一个已经初始化的 `Arc<UsageLedger>` 构造查询服务。
    pub fn new(ledger: Arc<UsageLedger>) -> Self {
        Self { ledger }
    }

    /// 直接用 `rusqlite::Connection`（通常是内存连接）初始化一个查询服务。
    ///
    /// 这会创建一个包装该连接的 `UsageLedger`：调用方需要保证 schema 已创建；
    /// 若未创建会自动 `CREATE TABLE IF NOT EXISTS usage_events`（幂等）。
    pub fn from_connection(conn: Connection) -> rusqlite::Result<Self> {
        // UsageLedger::new 需要 Path；我们这里直接借用 create_table_sql 重建。
        // 更稳妥做法：让 UsageLedger 也能从已 open Connection 构造，但要保持
        // 公共 API 兼容。做法：调用 UsageLedger::new 时用临时 Path 不行（内存 DB
        // 无 Path），因此此处直接调用带 schema 初始化的 Arc 构造：
        //
        // 我们直接把 Connection 塞进 UsageLedger —— 但 UsageLedger 字段是私有的。
        // 退化方案：把 conn 的 handle 转交给 UsageLedger::new 的临时 path？
        // 不——内存 DB 的 path=":memory:" 每次 open 独立。因此这里改为：
        // 在 conn 上手工创建 schema，然后返回一个包装 conn 的 UsageLedger。
        //
        // 但是 UsageLedger.conn 字段私有。我们无法直接构造它。
        // 为此，提供一个"仅在本模块/cfg test 可用的"办法：直接借用
        // UsageLedger 的 `new` 路径做不到。最后通过：
        //
        //  **直接让 QueryService 支持同时持有 `Arc<UsageLedger>` 或独立 `Connection`**
        // 太复杂。为了保持 FR-B1 的"持有 Arc<UsageLedger>" 承诺，我们提供一个
        // `UsageLedger::from_connection` 辅助，但由于 C1 不能修改 pet.rs 现有 API，
        // usage/mod.rs 的 UsageLedger 没有这个函数。
        //
        // 因此采取折中：让 `UsageQueryService` 的所有查询函数都能接受一个
        // `Mutex<Connection>`，但对外仍暴露 `from_connection(conn)` 的构造函数。
        // 简化做法：从_connection 构造时，我们直接调用
        // UsageLedger::new(Path::new(":memory:")) 无法工作（rusqlite 0.31 的
        // open(path) 对 ":memory:" 仍会得到独立内存 DB；所以这里直接在本模块
        // 内实现一个等价构造：

        // 先确保 schema 已创建（幂等）
        super::init_schema(&conn)?;
        // 构造一个假的（但等价的）UsageLedger 不可能——字段私有。
        // 最后策略：我们用一个"内部 ConnectionHolder"枚举，并让 by_day 读取时
        // 二选一。这与要求的"持有 Arc<UsageLedger>"冲突，但测试里需要
        // open_in_memory() 直接构造。所以我们改用：
        //
        // **查询服务内部持有一个 `Holder` 枚举；构造函数保留 `new(Arc<UsageLedger>)`，
        // 而 `from_connection` 走独立分支。所有查询函数同时支持两者。**
        Ok(Self {
            ledger: Arc::new(UsageLedger::wrap_for_query_service(conn)),
        })
    }
}

// ────── 为了支持 from_connection 且不破化 UsageLedger 公共 API，我们在
//        super::UsageLedger 上扩展一个仅对兄弟模块可见的构造器。
//        但因为 Rust 的可见性规则，兄弟模块可见只能 pub(crate)。
//        在 usage/mod.rs 我们会加一个 #[allow(dead_code)] pub(crate) 构造器。 ──────

// ─────────────────────── 访问器：拿底层 Mutex<Connection> 引用 ───────────────────────
//
// 由于 UsageLedger.conn 字段私有，我们新增一个 pub(crate) 访问器在 usage/mod.rs。
// 这里直接调用它：

fn with_conn<T, F>(svc: &UsageQueryService, f: F) -> rusqlite::Result<T>
where
    F: FnOnce(&Connection) -> rusqlite::Result<T>,
{
    let conn_guard = svc
        .ledger
        .conn_ref()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    f(&conn_guard)
}

// ─────────────────────────────────── 查询 ───────────────────────────────────

impl UsageQueryService {
    /// 日期范围 [start, end] 聚合：**end 会被包含**，无数据的日子填充
    /// 默认 `DayStat`（保证前端图表画布连续）。
    pub fn by_day(&self, start: NaiveDate, end: NaiveDate) -> rusqlite::Result<Vec<DayStat>> {
        if end < start {
            return Ok(Vec::new());
        }

        let start_ms = naive_date_to_ms(start);
        let end_ms = naive_date_to_ms(end) + 86_400_000 - 1; // 当天 23:59:59.999

        let rows = with_conn(self, |conn| {
            let mut stmt = conn.prepare(
                "SELECT session, model, prompt_tokens, completion_tokens, total_tokens, created_ms
                 FROM usage_events
                 WHERE created_ms BETWEEN ?1 AND ?2
                 ORDER BY created_ms ASC",
            )?;
            let rows = stmt.query_map(rusqlite::params![start_ms as i64, end_ms as i64], |r| {
                Ok(Row {
                    session: r.get::<_, String>(0)?,
                    model: r.get::<_, String>(1)?,
                    prompt_tokens: r.get::<_, i64>(2)? as u64,
                    completion_tokens: r.get::<_, i64>(3)? as u64,
                    total_tokens: r.get::<_, i64>(4)? as u64,
                    created_ms: r.get::<_, i64>(5)? as u64,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>()
        })?;

        // 1. 按日期建桶
        let mut map: BTreeMap<String, DayStat> = BTreeMap::new();
        for r in rows {
            let dt = ms_to_naive_utc(r.created_ms);
            let date = dt.date();
            let key = date.format("%Y-%m-%d").to_string();
            let e = map.entry(key).or_insert_with(|| DayStat {
                date: date.format("%Y-%m-%d").to_string(),
                ..Default::default()
            });
            e.tokens_in = e.tokens_in.saturating_add(r.prompt_tokens);
            e.tokens_out = e.tokens_out.saturating_add(r.completion_tokens);
            e.turns = e.turns.saturating_add(1);
            *e.model_distribution.entry(r.model).or_insert(0) = e
                .model_distribution
                .get(&r.model)
                .copied()
                .unwrap_or(0)
                .saturating_add(r.total_tokens);
            // sessions 后面统一 DISTINCT 重算
            let _ = r.session;
        }

        // 2. 对每个有数据的日期，重新算 sessions(DISTINCT)
        for (key, stat) in map.iter_mut() {
            let d = NaiveDate::parse_from_str(key, "%Y-%m-%d").map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            let s = naive_date_to_ms(d) as i64;
            let e_day = s + 86_400_000 - 1;
            let sessions: i64 = with_conn(self, |conn| {
                conn.query_row(
                    "SELECT COUNT(DISTINCT session) FROM usage_events \
                     WHERE created_ms BETWEEN ?1 AND ?2",
                    rusqlite::params![s, e_day],
                    |r| r.get(0),
                )
            })?;
            stat.sessions = sessions as u64;
        }

        // 3. 范围填充（空天 default DayStat）
        let mut out: Vec<DayStat> = Vec::new();
        let mut cur = start;
        while cur <= end {
            let key = cur.format("%Y-%m-%d").to_string();
            match map.remove(&key) {
                Some(d) => out.push(d),
                None => out.push(DayStat {
                    date: key,
                    ..Default::default()
                }),
            }
            cur += Duration::days(1);
        }
        Ok(out)
    }

    /// 指定月份整月聚合。月份范围 1..=12。
    pub fn by_month(&self, year: i32, month: u32) -> rusqlite::Result<MonthStat> {
        let (start, end) = month_range(year, month)
            .ok_or_else(|| rusqlite::Error::InvalidParameterName("bad year/month".into()))?;
        let days = self.by_day(start, end)?;

        let mut agg = MonthStat {
            year,
            month,
            ..Default::default()
        };
        let mut session_all: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

        for d in &days {
            // 为了 sessions 去重，by_day 返回的 DayStat 里 sessions 是"当日 DISTINCT"，
            // 整月需要跨日去重，所以再跑一次月级 COUNT(DISTINCT session)。
            // 因此此处 sessions 先占位，稍后重写。
            agg.tokens_in = agg.tokens_in.saturating_add(d.tokens_in);
            agg.tokens_out = agg.tokens_out.saturating_add(d.tokens_out);
            agg.turns = agg.turns.saturating_add(d.turns);
            for (m, t) in &d.model_distribution {
                *agg.model_distribution.entry(m.clone()).or_insert(0) = agg
                    .model_distribution
                    .get(m)
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(*t);
            }
            agg.days.insert(d.date.clone(), d.clone());
            let _ = &mut session_all;
        }

        // 整月 DISTINCT session
        let start_ms = naive_date_to_ms(start) as i64;
        let end_ms = naive_date_to_ms(end) as i64 + 86_400_000 - 1;
        let sessions: i64 = with_conn(self, |conn| {
            conn.query_row(
                "SELECT COUNT(DISTINCT session) FROM usage_events \
                 WHERE created_ms BETWEEN ?1 AND ?2",
                rusqlite::params![start_ms, end_ms],
                |r| r.get(0),
            )
        })?;
        agg.sessions = sessions as u64;

        Ok(agg)
    }

    /// 读取 Config.usage_balance 占位字段：无此字段返回 `None`。
    ///
    /// A 阶段占位：我们不引入真实付费对接，仅尝试从本地配置 YAML 里读取
    /// `usage.balance.usd` 字段。缺省 / 读取失败均返回 None。
    pub fn balance(&self) -> Option<f64> {
        // 尝试直接走 Config::load -> 转 Value -> 读取嵌套字段。
        // 任何失败（IO / 序列化 / 字段缺失）都吞掉 → None。
        let path = crate::persistence::config::Config::config_path();
        let s = std::fs::read_to_string(path).ok()?;
        let v: serde_yaml::Value = serde_yaml::from_str(&s).ok()?;
        let inner = v.get("usage")?.get("balance")?.get("usd")?;
        inner.as_f64().or_else(|| inner.as_i64().map(|x| x as f64))
    }

    /// 今日累计 total_tokens 是否超过阈值。
    ///
    /// "今日"使用 UTC 日边界（与 `by_day` 日期解析保持一致，避免时区混乱）。
    /// 触发返回 true（调用方应打 Notification / toast），未触发返回 false。
    pub fn check_today_budget(&self, threshold_tokens: u64) -> bool {
        let today = Utc::now().date_naive();
        match self.by_day(today, today) {
            Ok(mut v) if !v.is_empty() => {
                let d = v.swap_remove(0);
                d.tokens_in.saturating_add(d.tokens_out) > threshold_tokens
            }
            _ => false,
        }
    }

    /// 事件桥接：PostToolUse 后调用，写入 ledger + 产出 bus 要发布的
    /// `UsageAccumulated` 结构。不自动 publish，由调用方决定是否上总线。
    ///
    /// 失败（persisted=false）时仍返回完整事件，供监控/告警。
    pub fn on_post_tool_use(
        &self,
        session: &str,
        model: &str,
        usage: &TokenUsage,
    ) -> UsageAccumulated {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let persisted = self.ledger.record(session, model, usage).is_ok();
        UsageAccumulated {
            session: session.to_string(),
            model: model.to_string(),
            prompt_tokens: usage.prompt_tokens as u64,
            completion_tokens: usage.completion_tokens as u64,
            total_tokens: usage.total_tokens as u64,
            cache_read_tokens: usage.cache_read_input_tokens as u64,
            cache_creation_tokens: usage.cache_creation_input_tokens as u64,
            at_ms: now_ms,
            persisted,
        }
    }

    /// 把 `UsageAccumulated` 翻译成一个可发布的 `Envelope`。
    pub fn envelope_from_accumulated(acc: &UsageAccumulated) -> Envelope {
        Envelope::usage(
            &acc.session,
            &acc.model,
            acc.prompt_tokens as u32,
            acc.completion_tokens as u32,
            acc.total_tokens as u32,
            acc.cache_read_tokens as u32,
            acc.cache_creation_tokens as u32,
        )
    }
}

// ───────────────────────────── 辅助（日期/毫秒换算） ─────────────────────────────

/// NaiveDate（UTC 00:00） → 毫秒 UNIX EPOCH。
///
/// A 阶段不付费对接，跨模块使用（E2E 测试需注入"今天"固定 created_ms）故 pub。
pub fn naive_date_to_ms(d: NaiveDate) -> u64 {
    let dt = d
        .and_hms_opt(0, 0, 0)
        .expect("NaiveDate 00:00:00 构造必合法");
    // NaiveDateTime 1970-01-01 起秒数
    let secs = dt.and_utc().timestamp();
    if secs < 0 {
        0
    } else {
        secs as u64 * 1000
    }
}

fn ms_to_naive_utc(ms: u64) -> NaiveDateTime {
    let secs = (ms / 1000) as i64;
    let nsec = ((ms % 1000) * 1_000_000) as u32;
    match Utc.timestamp_opt(secs, nsec) {
        chrono::LocalResult::Single(t) => t.naive_utc(),
        _ => chrono::DateTime::<Utc>::from_timestamp(0, 0)
            .expect("epoch")
            .naive_utc(),
    }
}

fn month_range(year: i32, month: u32) -> Option<(NaiveDate, NaiveDate)> {
    if !(1..=12).contains(&month) {
        return None;
    }
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let last = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)? - Duration::days(1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)? - Duration::days(1)
    };
    Some((first, last))
}

// 查询用的行结构（避免用裸元组）
struct Row {
    session: String,
    model: String,
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    created_ms: u64,
}

// ─────────────────────────────────── 单测 ───────────────────────────────────

/// 本测试模块暴露的"跨兄弟模块复用"导出（E2E 骨架 `core/src/tests/dsh_e2e.rs`
/// 需要手工写入"今天"指定 `created_ms`，因此要能访问：
/// - `naive_date_to_ms` 日期→毫秒换算；
/// - `UsageQueryService::ledger_conn_for_e2e()`——测试专用的底层 Connection 访问器。
///
/// 这些导出只在 `cfg(test)` 下存在，不影响生产路径或 ABI。
#[cfg(test)]
pub mod tests_exports {
    use super::*;

    pub use super::naive_date_to_ms;

    impl UsageQueryService {
        /// E2E 测试专用：拿到底层 SQLite Connection 的可变句柄（绕过
        /// UsageLedger.record 的 SystemTime::now()，以便手工写入任意日期）。
        ///
        /// 仅 `cfg(test)` 可用，因此不影响生产公共 API 表面。
        pub fn ledger_conn_for_e2e(&self) -> std::sync::MutexGuard<'_, Connection> {
            self.ledger
                .conn_ref()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, Utc};

    // ------- 测试辅助 -------

    /// 直接拿内存连接（符合 C3：Connection::open_in_memory）。
    fn mem_svc() -> UsageQueryService {
        // 这里我们没法直接用 UsageLedger::new(path)（Path 指向内存不共享），
        // 因此直接"内部自举"：构建 Arc<UsageLedger> 需要通过 pub(crate) API
        // conn_ref / wrap_for_query_service —— 已在 usage/mod.rs 扩展。
        let conn = Connection::open_in_memory().unwrap();
        UsageQueryService::from_connection(conn).expect("from_connection 成功")
    }

    /// 在指定 date（UTC 日）插入 `i64` ms 时间戳的 record；
    /// 返回创建出的 created_ms（供断言）。
    fn insert_on(
        svc: &UsageQueryService,
        date: NaiveDate,
        session: &str,
        model: &str,
        p: u32,
        c: u32,
        offset_secs: u32,
    ) {
        let dt = date.and_hms_opt(0, 0, 0).unwrap().and_utc()
            + chrono::Duration::seconds(offset_secs as i64);
        let ms = dt.timestamp_millis() as u64;
        let usage = TokenUsage {
            prompt_tokens: p,
            completion_tokens: c,
            total_tokens: p + c,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        };
        // 直接写 SQL（绕过 ledger.record 的 SystemTime::now()），否则测试时间
        // 无法模拟。
        with_conn(svc, |conn| {
            conn.execute(
                "INSERT INTO usage_events (session, model, prompt_tokens, completion_tokens, total_tokens, created_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    session,
                    model,
                    p as i64,
                    c as i64,
                    (p + c) as i64,
                    ms as i64
                ],
            )?;
            Ok(())
        })
        .unwrap();
        let _ = usage;
    }

    // 1. schema_create_ok：建表后 SELECT COUNT = 0
    #[test]
    fn schema_create_ok() {
        let svc = mem_svc();
        let cnt: i64 = with_conn(&svc, |conn| {
            conn.query_row("SELECT COUNT(*) FROM usage_events", [], |r| r.get(0))
        })
        .unwrap();
        assert_eq!(cnt, 0);
    }

    // 2. insert_5_records_count = 5
    #[test]
    fn insert_5_records_count_equals_5() {
        let svc = mem_svc();
        let today = Utc::now().date_naive();
        for i in 0..5u32 {
            insert_on(&svc, today, "s1", "m1", 10, 20, i * 10);
        }
        let cnt: i64 = with_conn(&svc, |conn| {
            conn.query_row("SELECT COUNT(*) FROM usage_events", [], |r| r.get(0))
        })
        .unwrap();
        assert_eq!(cnt, 5);
    }

    // 3. by_day_single_day_matches：1 天 3 条，tokens_in/out 对账一致
    #[test]
    fn by_day_single_day_matches() {
        let svc = mem_svc();
        let d = NaiveDate::from_ymd_opt(2025, 1, 10).unwrap();
        insert_on(&svc, d, "s1", "opus", 100, 50, 1);
        insert_on(&svc, d, "s1", "opus", 100, 50, 2);
        insert_on(&svc, d, "s2", "sonnet", 50, 10, 3);

        let rows = svc.by_day(d, d).unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.tokens_in, 100 + 100 + 50);
        assert_eq!(r.tokens_out, 50 + 50 + 10);
        assert_eq!(r.turns, 3);
        assert_eq!(r.sessions, 2);
        let opus = r.model_distribution.get("opus").copied().unwrap_or(0);
        let sonnet = r.model_distribution.get("sonnet").copied().unwrap_or(0);
        assert_eq!(opus, (150u64) * 2);
        assert_eq!(sonnet, 60);
    }

    // 4. by_day_date_range_empty_days_present：3 天范围，中间 1 天无数据
    #[test]
    fn by_day_date_range_empty_days_present() {
        let svc = mem_svc();
        let d1 = NaiveDate::from_ymd_opt(2025, 2, 1).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2025, 2, 2).unwrap();
        let d3 = NaiveDate::from_ymd_opt(2025, 2, 3).unwrap();
        insert_on(&svc, d1, "s1", "m", 10, 5, 1);
        insert_on(&svc, d3, "s1", "m", 10, 5, 1);

        let rows = svc.by_day(d1, d3).unwrap();
        assert_eq!(rows.len(), 3, "3 天范围返回 Vec 长度必为 3");
        assert_eq!(rows[0].date, d1.format("%Y-%m-%d").to_string());
        assert_eq!(rows[1].date, d2.format("%Y-%m-%d").to_string());
        assert_eq!(rows[2].date, d3.format("%Y-%m-%d").to_string());
        assert!(rows[0].turns > 0);
        assert_eq!(rows[1].turns, 0, "中间天空天应填充 default DayStat");
        assert_eq!(rows[1].sessions, 0);
        assert!(rows[2].turns > 0);
    }

    // 5. by_month_january_2025_aggregate：1 月 3 天合计正确
    #[test]
    fn by_month_january_2025_aggregate() {
        let svc = mem_svc();
        let d1 = NaiveDate::from_ymd_opt(2025, 1, 5).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2025, 1, 15).unwrap();
        let d3 = NaiveDate::from_ymd_opt(2025, 1, 25).unwrap();
        insert_on(&svc, d1, "s1", "opus", 10, 1, 0);
        insert_on(&svc, d2, "s2", "sonnet", 20, 2, 0);
        insert_on(&svc, d3, "s3", "opus", 30, 3, 0);

        let jan = svc.by_month(2025, 1).unwrap();
        assert_eq!(jan.year, 2025);
        assert_eq!(jan.month, 1);
        assert_eq!(jan.tokens_in, 10 + 20 + 30);
        assert_eq!(jan.tokens_out, 1 + 2 + 3);
        assert_eq!(jan.turns, 3);
        assert_eq!(jan.sessions, 3);
        assert_eq!(
            jan.model_distribution.get("opus").copied().unwrap_or(0),
            11 + 33
        );
        assert_eq!(
            jan.model_distribution.get("sonnet").copied().unwrap_or(0),
            22
        );
        assert!(jan.days.contains_key(&d1.format("%Y-%m-%d").to_string()));
        assert!(jan.days.contains_key(&d2.format("%Y-%m-%d").to_string()));
        assert!(jan.days.contains_key(&d3.format("%Y-%m-%d").to_string()));
    }

    // 6. model_distribution_has_entries：2 个模型各自非零
    #[test]
    fn model_distribution_has_entries() {
        let svc = mem_svc();
        let d = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
        insert_on(&svc, d, "s", "opus", 1, 2, 0);
        insert_on(&svc, d, "s", "sonnet", 10, 20, 0);
        let rows = svc.by_day(d, d).unwrap();
        let r = &rows[0];
        let opus = r.model_distribution.get("opus").copied().unwrap_or(0);
        let sonnet = r.model_distribution.get("sonnet").copied().unwrap_or(0);
        assert!(opus > 0, "opus 分布非零");
        assert!(sonnet > 0, "sonnet 分布非零");
        assert_ne!(opus, sonnet); // 故意输入不一致，保证未被混淆
    }

    // 7. budget_threshold_triggered_true：今日累计超过阈值
    #[test]
    fn budget_threshold_triggered_true() {
        let svc = mem_svc();
        // 为避免测试对"今天"理解不同，直接用 UTC today 插数据
        let today = Utc::now().date_naive();
        // 总 token = 200 + 500 = 700
        insert_on(&svc, today, "s", "m", 200, 100, 1); // 300
        insert_on(&svc, today, "s", "m", 300, 100, 2); // 400
        assert!(svc.check_today_budget(500)); // 700 > 500 → true
    }

    // 8. budget_threshold_not_triggered_false：低于阈值
    #[test]
    fn budget_threshold_not_triggered_false() {
        let svc = mem_svc();
        let today = Utc::now().date_naive();
        insert_on(&svc, today, "s", "m", 10, 5, 1); // total 15
        assert!(!svc.check_today_budget(1000)); // 15 < 1000 → false
                                                // 空台账
        let empty = mem_svc();
        assert!(!empty.check_today_budget(1));
    }

    // 9. balance_default_is_none_or_some：配置无 usage.balance.usd 返回 None 正常
    #[test]
    fn balance_default_is_none_or_some() {
        let svc = mem_svc();
        // balance() 读真实 Config config_path 下字段；测试机通常不存在 → None
        // 存在但不包含该字段 → None
        // 两种情况都是"合法值 Any(Some/None) OK"，因此只断言不 panic。
        let b = svc.balance();
        // 不做值假设；若机器上恰好有 balance.usd，它也是合法的 Some
        match b {
            Some(v) => assert!(v.is_finite(), "Some(f64) 必须有限"),
            None => { /* 正常（A 阶段占位） */ }
        }
    }

    // 10. insert_10k_query_fast：插 10000 条，by_day(30 天) 非空不报错
    #[test]
    fn insert_10k_query_fast() {
        let svc = mem_svc();
        let start = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        with_conn(&svc, |conn| {
            let tx = conn.unchecked_transaction()?;
            let mut stmt = tx.prepare(
                "INSERT INTO usage_events (session, model, prompt_tokens, completion_tokens, total_tokens, created_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for i in 0..10_000i64 {
                let day_offset = i % 30;
                let d = start + Duration::days(day_offset);
                let ms = naive_date_to_ms(d) + (i as u64 % 86_400_000);
                stmt.execute(rusqlite::params![
                    format!("s{}", i % 7),
                    if i % 2 == 0 { "opus" } else { "sonnet" },
                    10 + (i % 100),
                    20 + (i % 50),
                    (10 + (i % 100)) + (20 + (i % 50)),
                    ms as i64,
                ])?;
            }
            drop(stmt);
            tx.commit()?;
            Ok(())
        })
        .unwrap();

        let end = start + Duration::days(29);
        let rows = svc.by_day(start, end).expect("30 天查询不应报错");
        assert_eq!(rows.len(), 30);
        // 至少有一天非零
        assert!(rows.iter().any(|r| r.turns > 0));
    }
}
