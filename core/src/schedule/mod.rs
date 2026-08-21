//! DSH schedule 会话级提醒移植（上游 dsh-schedule 包）
//!
//! 三种触发（对齐上游 ScheduleRecord）：
//! - `after <seconds>`：相对注册时刻的一次性延迟；
//! - `at <ts_ms>`：绝对时刻一次性（必须位于未来，否则 `NotFuture`）；
//! - `every <seconds>`：固定间隔（下限 [`MIN_EVERY_INTERVAL_SECONDS`]，
//!   低于即 `FrequencyTooHigh`）。
//!
//! 状态由台账（SQLite）持有；定时器与提醒是**可弃投影**——`due()` 是纯查询，
//! 扫描驱动可任意重建、错过不补跑（对齐上游「事件日志持有状态，定时器是可弃
//! 投影」）。`every` 重挂载**不枚举积压**：一次性推进到最近一个大于当前时刻
//! 的锚点（对齐上游 `EveryOccurrence` 决策）。
//!
//! 持久化：`<data_dir>/schedule.db`；测试可用 `MARGUERITE_SCHEDULE_DB` 覆盖隔离。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 固定间隔提醒的固定下限（秒）——对齐上游 `MIN_EVERY_INTERVAL_SECONDS`。
pub const MIN_EVERY_INTERVAL_SECONDS: u64 = 300;

/// 输入错误（对齐上游 `ScheduleInputError` 错误码族）。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ScheduleError {
    /// invalid_rule：规则/提示文本无法成为记录
    #[error("无效提醒规则: {0}")]
    InvalidRule(String),
    /// not_future：at 时刻不在未来 / after ≤ 0
    #[error("提醒时刻不在未来")]
    NotFuture,
    /// time_out_of_range：时刻超出可表示范围
    #[error("提醒时刻超出可表示范围")]
    TimeOutOfRange,
    /// frequency_too_high：every 间隔低于 5 分钟下限
    #[error("固定间隔低于下限（{MIN_EVERY_INTERVAL_SECONDS} 秒）")]
    FrequencyTooHigh,
}

/// 触发方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReminderTrigger {
    /// 相对注册时刻的一次性延迟（秒）
    After { seconds: u64 },
    /// 绝对时刻一次性（毫秒时间戳）
    At { ts_ms: i64 },
    /// 固定间隔（秒，≥ 300）
    Every { interval_seconds: u64 },
}

impl ReminderTrigger {
    pub fn kind(&self) -> &'static str {
        match self {
            ReminderTrigger::After { .. } => "after",
            ReminderTrigger::At { .. } => "at",
            ReminderTrigger::Every { .. } => "every",
        }
    }
}

/// 一条持久提醒。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reminder {
    pub id: String,
    pub session: String,
    pub message: String,
    pub trigger: ReminderTrigger,
    pub created_at_ms: i64,
    /// 下次触发时刻（every 重挂载时推进）
    pub next_fire_ms: i64,
    /// 一次性（after/at）触发后置 true；every 恒 false
    pub fired: bool,
}

/// 会话提醒台账。
pub struct ScheduleStore {
    conn: rusqlite::Connection,
}

impl ScheduleStore {
    /// 打开台账；`None` = 内存库（测试）。
    pub fn open(db_path: Option<PathBuf>) -> Result<Self, String> {
        let conn = match db_path {
            Some(p) => {
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                rusqlite::Connection::open(&p).map_err(|e| format!("打开提醒库失败: {e}"))?
            }
            None => rusqlite::Connection::open_in_memory()
                .map_err(|e| format!("创建内存提醒库失败: {e}"))?,
        };
        let store = Self { conn };
        store.ensure_schema()?;
        Ok(store)
    }

    /// 默认台账：`<data_dir>/schedule.db`。
    pub fn open_default() -> Result<Self, String> {
        Self::open(Some(schedule_path()))
    }

    fn ensure_schema(&self) -> Result<(), String> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS reminders (
                   id TEXT PRIMARY KEY,
                   session_id TEXT NOT NULL,
                   message TEXT NOT NULL,
                   trigger_kind TEXT NOT NULL,
                   trigger_value INTEGER NOT NULL,
                   created_at_ms INTEGER NOT NULL,
                   next_fire_ms INTEGER NOT NULL,
                   fired INTEGER NOT NULL DEFAULT 0
                 );",
            )
            .map_err(|e| format!("建提醒表失败: {e}"))
    }

    /// 注册提醒（校验规则，计算 next_fire，落库）。
    pub fn register(
        &mut self,
        session: &str,
        message: &str,
        trigger: ReminderTrigger,
        now_ms: i64,
    ) -> Result<Reminder, ScheduleError> {
        let msg = message.trim();
        if msg.is_empty() {
            return Err(ScheduleError::InvalidRule("提醒消息不可为空".into()));
        }
        let next_fire_ms = match trigger {
            ReminderTrigger::After { seconds } => {
                if seconds == 0 {
                    return Err(ScheduleError::NotFuture);
                }
                now_ms
                    .checked_add(seconds as i64 * 1000)
                    .ok_or(ScheduleError::TimeOutOfRange)?
            }
            ReminderTrigger::At { ts_ms } => {
                if ts_ms <= now_ms {
                    return Err(ScheduleError::NotFuture);
                }
                ts_ms
            }
            ReminderTrigger::Every { interval_seconds } => {
                if interval_seconds < MIN_EVERY_INTERVAL_SECONDS {
                    return Err(ScheduleError::FrequencyTooHigh);
                }
                now_ms
                    .checked_add(interval_seconds as i64 * 1000)
                    .ok_or(ScheduleError::TimeOutOfRange)?
            }
        };
        let reminder = Reminder {
            id: format!("rem_{}", uuid::Uuid::new_v4()),
            session: session.to_string(),
            message: msg.to_string(),
            trigger,
            created_at_ms: now_ms,
            next_fire_ms,
            fired: false,
        };
        let (kind, value) = match trigger {
            ReminderTrigger::After { seconds } => ("after", seconds as i64),
            ReminderTrigger::At { ts_ms } => ("at", ts_ms),
            ReminderTrigger::Every { interval_seconds } => ("every", interval_seconds as i64),
        };
        self.conn
            .execute(
                "INSERT INTO reminders
                   (id, session_id, message, trigger_kind, trigger_value,
                    created_at_ms, next_fire_ms, fired)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
                rusqlite::params![
                    reminder.id,
                    reminder.session,
                    reminder.message,
                    kind,
                    value,
                    reminder.created_at_ms,
                    reminder.next_fire_ms,
                ],
            )
            .map_err(|e| ScheduleError::InvalidRule(format!("落库失败: {e}")))?;
        Ok(reminder)
    }

    fn row_to_reminder(row: &rusqlite::Row<'_>) -> rusqlite::Result<Reminder> {
        let kind: String = row.get(2)?;
        let value: i64 = row.get(3)?;
        let trigger = match kind.as_str() {
            "after" => ReminderTrigger::After {
                seconds: value.max(0) as u64,
            },
            "at" => ReminderTrigger::At { ts_ms: value },
            "every" => ReminderTrigger::Every {
                interval_seconds: value.max(0) as u64,
            },
            // 损坏数据：整行作废（上层 filter_map 丢弃，台账其余条目不受影响）
            _ => return Err(rusqlite::Error::InvalidQuery),
        };
        Ok(Reminder {
            id: row.get(0)?,
            session: row.get(1)?,
            message: row.get(4)?,
            trigger,
            created_at_ms: row.get(5)?,
            next_fire_ms: row.get(6)?,
            fired: row.get::<_, i64>(7)? != 0,
        })
    }

    /// 某会话全部提醒（未触发在前，按下次触发时刻升序）。
    pub fn list(&self, session: &str) -> Vec<Reminder> {
        let mut stmt = match self.conn.prepare(
            "SELECT id, session_id, trigger_kind, trigger_value, message,
                    created_at_ms, next_fire_ms, fired
             FROM reminders WHERE session_id = ?1
             ORDER BY fired ASC, next_fire_ms ASC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let rows = stmt.query_map(rusqlite::params![session], Self::row_to_reminder);
        match rows {
            Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    /// 全部提醒（跨会话；供扫描驱动）。
    pub fn all(&self) -> Vec<Reminder> {
        let mut stmt = match self.conn.prepare(
            "SELECT id, session_id, trigger_kind, trigger_value, message,
                    created_at_ms, next_fire_ms, fired
             FROM reminders ORDER BY next_fire_ms ASC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let rows = stmt.query_map([], Self::row_to_reminder);
        match rows {
            Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    /// 到期提醒（纯查询）：`next_fire_ms <= now && !fired`。
    ///
    /// 副作用（对齐上游投影语义，仅当到期才发生）：
    /// - after/at：置 `fired = true`（一次性）；
    /// - every：`next_fire_ms` 一次性推进到最近一个大于 now 的锚点
    ///   （**不枚举积压**——错过不补跑，对齐上游 EveryOccurrence 决策）。
    pub fn due(&mut self, session: &str, now_ms: i64) -> Vec<Reminder> {
        let pending = self.list(session);
        let due: Vec<Reminder> = pending
            .iter()
            .filter(|r| !r.fired && r.next_fire_ms <= now_ms)
            .cloned()
            .collect();
        for r in &due {
            match r.trigger {
                ReminderTrigger::Every { interval_seconds } => {
                    let interval_ms = interval_seconds as i64 * 1000;
                    let missed = now_ms.saturating_sub(r.next_fire_ms) / interval_ms;
                    // 最近一个大于 now 的锚点（无积压枚举）
                    let next = r
                        .next_fire_ms
                        .saturating_add((missed + 1).saturating_mul(interval_ms));
                    let _ = self.conn.execute(
                        "UPDATE reminders SET next_fire_ms = ?1 WHERE id = ?2",
                        rusqlite::params![next, r.id],
                    );
                }
                _ => {
                    let _ = self.conn.execute(
                        "UPDATE reminders SET fired = 1 WHERE id = ?1",
                        rusqlite::params![r.id],
                    );
                }
            }
        }
        due
    }

    /// 取消提醒；返回是否确实存在并取消。
    pub fn cancel(&mut self, id: &str) -> bool {
        self.conn
            .execute("DELETE FROM reminders WHERE id = ?1", rusqlite::params![id])
            .map(|n| n > 0)
            .unwrap_or(false)
    }
}

/// 台账路径：`<config_dir>/schedule.db`；`MARGUERITE_SCHEDULE_DB` 可覆盖（测试隔离）。
pub fn schedule_path() -> PathBuf {
    if let Ok(p) = std::env::var("MARGUERITE_SCHEDULE_DB") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    crate::persistence::config::Config::config_dir().join("schedule.db")
}

/// `/schedule` 斜杠命令执行面（v7 接线点 3）：解析 → 执行 → 返回展示文本。
///
/// REPL / 前端处理器共用；纯同步、失败不 panic（返回错误文本）。
pub fn execute_slash(session: &str, input: &str) -> String {
    let cmd = match crate::cli::dslash::parse_dslash(input) {
        Ok(cmd) => cmd,
        Err(e) => return e.to_string(),
    };
    match cmd.payload {
        crate::cli::dslash::DshSlashPayload::Schedule(sched) => execute_cmd(session, &sched),
        _ => "该命令不由 schedule 执行面处理".to_string(),
    }
}

/// 执行一条已解析的 `/schedule` 子命令（against 默认台账）。
fn execute_cmd(session: &str, cmd: &crate::cli::dslash::ScheduleCmd) -> String {
    let mut store = match ScheduleStore::open_default() {
        Ok(s) => s,
        Err(e) => return format!("提醒台账打开失败: {e}"),
    };
    let now_ms = chrono::Utc::now().timestamp_millis();
    match cmd {
        crate::cli::dslash::ScheduleCmd::List => {
            let items = store.list(session);
            if items.is_empty() {
                return "暂无提醒。".to_string();
            }
            let mut lines = vec![format!("提醒（{}）：", items.len())];
            for r in items {
                let state = if r.fired { "已触发" } else { "待触发" };
                lines.push(format!(
                    "[{}] {} {} — {}（{}）",
                    r.id,
                    r.trigger.kind(),
                    r.message,
                    state,
                    chrono::DateTime::from_timestamp_millis(r.next_fire_ms)
                        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| r.next_fire_ms.to_string())
                ));
            }
            lines.join("\n")
        }
        crate::cli::dslash::ScheduleCmd::After { seconds, message } => {
            match store.register(
                session,
                message,
                ReminderTrigger::After { seconds: *seconds },
                now_ms,
            ) {
                Ok(r) => format!("已注册提醒 {}：{} 秒后「{}」", r.id, seconds, r.message),
                Err(e) => format!("注册失败: {e}"),
            }
        }
        crate::cli::dslash::ScheduleCmd::At { iso, message } => {
            let ts = match chrono::DateTime::parse_from_rfc3339(iso) {
                Ok(t) => t.timestamp_millis(),
                Err(_) => return format!("时刻解析失败（需要 RFC3339，如 2026-08-22T10:00:00Z）: {iso}"),
            };
            match store.register(session, message, ReminderTrigger::At { ts_ms: ts }, now_ms) {
                Ok(r) => format!("已注册提醒 {}：{}「{}」", r.id, iso, r.message),
                Err(e) => format!("注册失败: {e}"),
            }
        }
        crate::cli::dslash::ScheduleCmd::Every { seconds, message } => {
            match store.register(
                session,
                message,
                ReminderTrigger::Every {
                    interval_seconds: *seconds,
                },
                now_ms,
            ) {
                Ok(r) => format!("已注册提醒 {}：每 {} 秒「{}」", r.id, seconds, r.message),
                Err(e) => format!("注册失败: {e}"),
            }
        }
        crate::cli::dslash::ScheduleCmd::Cancel { id } => {
            if store.cancel(id) {
                format!("已取消提醒 {id}。")
            } else {
                format!("未找到提醒 {id}。")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;
    use std::sync::Mutex;

    /// 测试串行锁（与 goal 同策略：DB 路径为进程级环境变量，测试需互斥）
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn lock_tests() -> std::sync::MutexGuard<'static, ()> {
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    fn isolated_db(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mgt_schedule_{name}_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("schedule.db")
    }

    const NOW: i64 = 1_700_000_000_000;

    #[test]
    fn register_after_fires_once() {
        let _g = lock_tests();
        let mut store = ScheduleStore::open(Some(isolated_db("after"))).unwrap();
        let r = store
            .register("s1", "休息提醒", ReminderTrigger::After { seconds: 60 }, NOW)
            .unwrap();
        assert_eq!(r.next_fire_ms, NOW + 60_000);
        assert!(store.due("s1", NOW + 59_999).is_empty());
        let fired = store.due("s1", NOW + 60_000);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].id, r.id);
        // 一次性：再次 due 不再触发
        assert!(store.due("s1", NOW + 120_000).is_empty());
    }

    #[test]
    fn register_at_requires_future() {
        let _g = lock_tests();
        let mut store = ScheduleStore::open(Some(isolated_db("at"))).unwrap();
        assert_eq!(
            store
                .register("s1", "过去", ReminderTrigger::At { ts_ms: NOW - 1 }, NOW)
                .unwrap_err(),
            ScheduleError::NotFuture
        );
        let r = store
            .register("s1", "未来", ReminderTrigger::At { ts_ms: NOW + 5 }, NOW)
            .unwrap();
        assert!(store.due("s1", NOW + 5).iter().any(|x| x.id == r.id));
    }

    #[test]
    fn register_after_zero_rejected() {
        let _g = lock_tests();
        let mut store = ScheduleStore::open(Some(isolated_db("after0"))).unwrap();
        assert_eq!(
            store
                .register("s1", "x", ReminderTrigger::After { seconds: 0 }, NOW)
                .unwrap_err(),
            ScheduleError::NotFuture
        );
    }

    #[test]
    fn register_every_enforces_min_interval() {
        let _g = lock_tests();
        let mut store = ScheduleStore::open(Some(isolated_db("every_min"))).unwrap();
        assert_eq!(
            store
                .register("s1", "x", ReminderTrigger::Every { interval_seconds: 299 }, NOW)
                .unwrap_err(),
            ScheduleError::FrequencyTooHigh
        );
        store
            .register(
                "s1",
                "x",
                ReminderTrigger::Every {
                    interval_seconds: MIN_EVERY_INTERVAL_SECONDS,
                },
                NOW,
            )
            .unwrap();
    }

    #[test]
    fn every_rearms_without_backlog_enumeration() {
        let _g = lock_tests();
        let mut store = ScheduleStore::open(Some(isolated_db("every_rearm"))).unwrap();
        let r = store
            .register(
                "s1",
                "打卡",
                ReminderTrigger::Every {
                    interval_seconds: MIN_EVERY_INTERVAL_SECONDS,
                },
                NOW,
            )
            .unwrap();
        let interval_ms = MIN_EVERY_INTERVAL_SECONDS as i64 * 1000;
        // 错过 3 个周期：due 只触发一次，且 next 直接跳到最近一个未来锚点（不补跑）
        let fired = store.due("s1", NOW + interval_ms * 3 + 10);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].id, r.id);
        let after = store.list("s1");
        assert_eq!(after.len(), 1);
        assert!(!after[0].fired, "every 不置 fired");
        assert_eq!(after[0].next_fire_ms, NOW + interval_ms * 4, "不枚举积压，锚点对齐");
    }

    #[test]
    fn cancel_removes_reminder() {
        let _g = lock_tests();
        let mut store = ScheduleStore::open(Some(isolated_db("cancel"))).unwrap();
        let r = store
            .register("s1", "x", ReminderTrigger::After { seconds: 60 }, NOW)
            .unwrap();
        assert!(store.cancel(&r.id));
        assert!(!store.cancel(&r.id));
        assert!(store.list("s1").is_empty());
        assert!(store.due("s1", NOW + 60_000).is_empty());
    }

    #[test]
    fn persistence_survives_reopen() {
        let _g = lock_tests();
        let db = isolated_db("persist");
        {
            let mut store = ScheduleStore::open(Some(db.clone())).unwrap();
            store
                .register("s1", "持久", ReminderTrigger::After { seconds: 60 }, NOW)
                .unwrap();
        }
        let mut store = ScheduleStore::open(Some(db)).unwrap();
        let due = store.due("s1", NOW + 60_000);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].message, "持久");
    }

    #[test]
    fn empty_message_rejected() {
        let _g = lock_tests();
        let mut store = ScheduleStore::open(Some(isolated_db("empty"))).unwrap();
        assert!(matches!(
            store.register("s1", "  ", ReminderTrigger::After { seconds: 60 }, NOW),
            Err(ScheduleError::InvalidRule(_))
        ));
    }

    #[test]
    fn sessions_isolated() {
        let _g = lock_tests();
        let mut store = ScheduleStore::open(Some(isolated_db("iso"))).unwrap();
        store
            .register("a", "x", ReminderTrigger::After { seconds: 60 }, NOW)
            .unwrap();
        store
            .register("b", "y", ReminderTrigger::After { seconds: 60 }, NOW)
            .unwrap();
        assert_eq!(store.list("a").len(), 1);
        assert_eq!(store.due("a", NOW + 60_000).len(), 1);
        assert!(store.due("b", NOW).is_empty());
    }

    #[test]
    fn execute_slash_registers_lists_and_cancels() {
        let _g = lock_tests();
        let db = isolated_db("slash");
        std::env::set_var("MARGUERITE_SCHEDULE_DB", db.to_string_lossy().to_string());
        let out = execute_slash("s1", "/schedule after 600 喝水");
        assert!(out.contains("已注册提醒"), "{out}");
        assert!(out.contains("喝水"));
        let list = execute_slash("s1", "/schedule list");
        assert!(list.contains("喝水"), "{list}");
        // 取首条提醒 id 并取消
        let id = list
            .lines()
            .nth(1)
            .and_then(|l| l.split_whitespace().next())
            .map(|s| s.trim_start_matches('[').trim_end_matches(']').to_string())
            .expect("应有提醒行");
        let cancel = execute_slash("s1", &format!("/schedule cancel {id}"));
        assert!(cancel.contains("已取消"), "{cancel}");
        assert!(execute_slash("s1", "/schedule list").contains("暂无提醒"));
        std::env::remove_var("MARGUERITE_SCHEDULE_DB");
    }

    #[test]
    fn execute_slash_reports_errors() {
        let _g = lock_tests();
        let db = isolated_db("slash_err");
        std::env::set_var("MARGUERITE_SCHEDULE_DB", db.to_string_lossy().to_string());
        assert!(
            execute_slash("s1", "/schedule every 60 太快").contains("下限"),
            "低于 5 分钟应报 FrequencyTooHigh"
        );
        assert!(execute_slash("s1", "/schedule at 不是时刻 x").contains("解析失败"));
        assert!(execute_slash("s1", "/schedule foobar").contains("无效子命令"));
        std::env::remove_var("MARGUERITE_SCHEDULE_DB");
    }
}
