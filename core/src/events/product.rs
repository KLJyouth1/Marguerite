//! 产品行为事件（M3-D1）
//!
//! 统一入口：产品行为事件 = 「事件总线发布」 + 「本地审计日志落盘」双写。
//! 12 个产品事件覆盖方案要求的口径：任务发起 / 计划确认 / 工具调用 / 完成 /
//! 失败 / 取消 / 切换 Mode / 切换信任档位 / 审批请求 / 审批通过 / 审批拒绝 /
//! 预算告警。
//!
//! 审计日志复用 [`crate::audit_store`]（append-only SQLite + SHA-256 哈希链），
//! WebUI「最近活动」时间线通过 `product.recent` RPC 直接读取该日志——保证
//! 「审计日志实落盘」与「时间线可见」同源、可对账。

use std::path::PathBuf;
// `OnceLock` 只服务生产路径的 LOG_PATH；测试路径改用 `RwLock`（见下方说明）。
#[cfg(not(test))]
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use serde_json::{json, Value};

use crate::audit_store::{AuditRecord, AuditStore};
use crate::events::{Envelope, EventType};

// ---------------------------------------------------------------------------
// 12 个产品行为事件（验收口径见 docs/M3_产品化_验收报告.md）
// ---------------------------------------------------------------------------

/// 任务发起（队列入队 / 会话新 turn）
pub const TASK_INITIATED: &str = "task.initiated";
/// 计划确认（Plan Approval 通过 / 计划卡确认）
pub const PLAN_CONFIRMED: &str = "plan.confirmed";
/// 工具调用
pub const TOOL_CALL: &str = "tool.call";
/// 任务完成
pub const TASK_COMPLETED: &str = "task.completed";
/// 任务失败
pub const TASK_FAILED: &str = "task.failed";
/// 任务取消
pub const TASK_CANCELLED: &str = "task.cancelled";
/// 切换 Mode
pub const MODE_CHANGED: &str = "mode.changed";
/// 切换信任档位（trust.grant / trust.revoke）
pub const TRUST_CHANGED: &str = "trust.changed";
/// 审批请求（高风险动作进入确认态）
pub const APPROVAL_REQUESTED: &str = "approval.requested";
/// 审批通过
pub const APPROVAL_GRANTED: &str = "approval.granted";
/// 审批拒绝
pub const APPROVAL_DENIED: &str = "approval.denied";
/// 成本预算告警（D2 FinOps 升级）
pub const BUDGET_WARNING: &str = "budget.warning";

/// 12 个产品事件全集（验收口径用）
pub const PRODUCT_EVENTS: [&str; 12] = [
    TASK_INITIATED,
    PLAN_CONFIRMED,
    TOOL_CALL,
    TASK_COMPLETED,
    TASK_FAILED,
    TASK_CANCELLED,
    MODE_CHANGED,
    TRUST_CHANGED,
    APPROVAL_REQUESTED,
    APPROVAL_GRANTED,
    APPROVAL_DENIED,
    BUDGET_WARNING,
];

/// kind → 事件信封类型（前端消费的事件类型段）
pub fn envelope_type(kind: &str) -> EventType {
    match kind {
        PLAN_CONFIRMED => EventType::Plan,
        TOOL_CALL => EventType::ToolCall,
        TASK_COMPLETED => EventType::Done,
        TASK_FAILED => EventType::Error,
        APPROVAL_REQUESTED | APPROVAL_GRANTED | APPROVAL_DENIED => EventType::Approval,
        _ => EventType::Status,
    }
}

// ---------------------------------------------------------------------------
// 审计日志：进程级单例，可注入路径（测试）
// ---------------------------------------------------------------------------
//
// 生产路径用 `OnceLock`：路径只能在首次 `emit` 前设置一次，之后不可变。
// 测试路径改用 `RwLock`：`cargo test` 在同一个进程里跑成千上万个用例，而
// `usage` / `plan::approval` / `gateway::mode` / `task_queue` / `chain::executor`
// 等模块的并行用例也会调用 [`emit`] / [`audit`]。若路径只能设一次，本模块的
// 第二个用例就会被迫复用第一个用例（或别的模块）留下的数据库文件，断言
// 「应有 N 条」必然受残留行影响而偶发失败。
//
// `#[cfg(test)]` 分流保证**生产行为零变化**：非 test 构建下仍是「首次生效」。
#[cfg(not(test))]
static LOG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
#[cfg(test)]
static LOG_PATH: std::sync::RwLock<Option<Option<PathBuf>>> = std::sync::RwLock::new(None);

/// 已打开的审计库句柄（进程级单例；`reset_store` 在测试中清空以切换文件）。
static STORE: Mutex<Option<Result<Arc<AuditStore>, String>>> = Mutex::new(None);

/// 注入审计日志路径（生产：幂等，仅首次生效；测试：每次调用都生效，便于逐用例隔离）。
/// 测试/嵌入式场景在首次 `emit` 前调用。
pub fn configure_log_path(path: Option<PathBuf>) {
    #[cfg(not(test))]
    {
        let _ = LOG_PATH.set(path);
    }
    #[cfg(test)]
    {
        if let Ok(mut g) = LOG_PATH.write() {
            *g = Some(path);
        }
    }
}

/// 读取当前注入的审计路径（`None` = 从未注入过）。
fn configured_log_path() -> Option<PathBuf> {
    #[cfg(not(test))]
    {
        LOG_PATH.get().and_then(|p| p.clone())
    }
    #[cfg(test)]
    {
        LOG_PATH
            .read()
            .ok()
            .and_then(|g| g.clone())
            .flatten()
    }
}

/// 是否「从未注入过路径」（测试环境据此拒绝写用户真实审计库）。
#[cfg(test)]
fn log_path_unset() -> bool {
    LOG_PATH.read().map(|g| g.is_none()).unwrap_or(false)
}

fn init_store() -> Result<Arc<AuditStore>, String> {
    // 测试环境下若未显式注入审计路径，直接报错而非落到用户真实 audit.db。
    #[cfg(test)]
    if log_path_unset() {
        return Err("audit path 未注入（测试环境不写真实审计库）".to_string());
    }
    let res = match configured_log_path() {
        Some(p) => AuditStore::open(p),
        None => AuditStore::open_default(),
    };
    res.map(Arc::new).map_err(|e| e.to_string())
}

fn store() -> Result<Arc<AuditStore>, String> {
    let mut g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = g.as_ref() {
        return r.clone();
    }
    let r = init_store();
    *g = Some(r.clone());
    r
}

/// 测试用：重置单例 store（配合全新审计文件隔离各用例）。
#[cfg(test)]
fn reset_store() {
    if let Ok(mut g) = STORE.lock() {
        *g = None;
    }
}

// ---------------------------------------------------------------------------
// 发射 / 落盘
// ---------------------------------------------------------------------------

/// 发布产品事件：总线广播 + 审计落盘。
///
/// `fields` 与会话标识共同构成事件载荷；统一附加 `event` 字段便于前端时间线归类。
pub fn emit(kind: &str, session: &str, fields: Value) {
    let mut payload = json!({ "event": kind });
    if let Value::Object(map) = &mut payload {
        if let Value::Object(extra) = fields {
            for (k, v) in extra {
                map.insert(k, v);
            }
        }
    }
    if let Some(bus) = crate::events::global_bus() {
        bus.publish(Envelope::new(envelope_type(kind), payload.clone(), session));
    }
    let _ = append_audit(kind, session, &payload);
}

/// 仅落盘审计（事件总线已由调用方发布，避免 `mode.changed` 等重复广播）。
pub fn audit(kind: &str, session: &str, fields: Value) {
    let payload = json!({ "event": kind, "fields": fields });
    let _ = append_audit(kind, session, &payload);
}

fn append_audit(kind: &str, session: &str, payload: &Value) -> Result<(), String> {
    let timestamp = Utc::now().to_rfc3339();
    let record = AuditRecord::new(
        timestamp,
        session.to_string(),
        format!("event.{kind}"),
        payload.to_string(),
        "ok",
    );
    let s = store()?;
    s.append(record).map(|_| ()).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// 读取：WebUI「最近活动」时间线数据源（审计日志同源）
// ---------------------------------------------------------------------------

/// 最近行为事件（含审计元数据），`limit` 最大 200。
pub fn recent(limit: usize) -> Value {
    let limit = limit.clamp(1, 200) as i64;
    let s = match store() {
        Ok(s) => s,
        Err(e) => return json!({ "error": e }),
    };
    let conn = match s.conn().lock() {
        Ok(c) => c,
        Err(e) => return json!({ "error": format!("audit lock: {e}") }),
    };
    let mut stmt = match conn.prepare(
        "SELECT id, timestamp, actor, method, target, result FROM audit_log ORDER BY id DESC LIMIT ?1",
    ) {
        Ok(st) => st,
        Err(e) => return json!({ "error": e.to_string() }),
    };
    let rows = stmt
        .query_map([&limit], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "ts": row.get::<_, String>(1)?,
                "session": row.get::<_, String>(2)?,
                "method": row.get::<_, String>(3)?,
                "target": row.get::<_, String>(4)?,
                "result": row.get::<_, String>(5)?,
            }))
        })
        .map(|it| it.filter_map(Result::ok).collect::<Vec<_>>())
        .unwrap_or_default();
    // 解析 target 中的 event 字段，归类为产品事件
    let entries: Vec<Value> = rows
        .into_iter()
        .filter(|r| {
            r["method"]
                .as_str()
                .map(|m| m.starts_with("event."))
                .unwrap_or(false)
        })
        .take(limit as usize)
        .map(|mut r| {
            let target = r["target"].as_str().unwrap_or("{}");
            let ev = serde_json::from_str::<Value>(target).unwrap_or_default();
            r["event"] = ev.get("event").cloned().unwrap_or(Value::Null);
            if let Value::Object(m) = &mut r {
                m.remove("target");
                m.remove("result");
            }
            r
        })
        .collect();
    json!({ "entries": entries, "total": entries.len() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex as TestMutex;

    // 用例串行化：所有用例共享同一审计文件，通过 remove+reset 重建隔离
    static TEST_LOCK: TestMutex<()> = TestMutex::new(());

    /// 为**每个用例**生成互不相同的审计库路径。
    ///
    /// 为什么必须唯一（两条独立的污染路径，都会让本模块的用例非确定性失败）：
    ///
    /// 1. **跨进程**：`cargo test` 会为每个测试**二进制**起一个进程，而同一台机器上
    ///    两次并发 `cargo test` 更会同时跑多个进程。若路径写成固定的
    ///    `marguerite_product_events_test.db`，两个进程会共用同一个 SQLite 文件；
    ///    而 `remove_file` 在 Windows 上遇到被占用的文件会**静默失败**
    ///    （原来写的是 `let _ = ...`），于是上一个进程留下的行会留在库里，
    ///    本轮断言「应有 N 条」直接失败。
    /// 2. **同进程**：`STORE` 是进程级单例，而 `usage` / `plan::approval` /
    ///    `gateway::mode` / `task_queue` / `chain::executor` 等模块在各自的并行用例里
    ///    也会调用 [`emit`] / [`audit`]；它们**不受本模块 `TEST_LOCK` 保护**，
    ///    会写进同一个库。因此断言行也必须按「增量」判定，见 [`count_after`]。
    ///
    /// 路径按 `pid` + 原子计数器唯一化，同时消除跨进程与跨用例的文件复用。
    fn tmp_db() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let mut p = std::env::temp_dir();
        p.push(format!(
            "marguerite_product_events_test_{}_{}.db",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&p);
        p
    }

    /// 建立本用例**独占**的审计库并返回其路径；调用方须已持有 `TEST_LOCK`。
    fn with_fresh_audit_db() -> PathBuf {
        let db = tmp_db();
        configure_log_path(Some(db.clone()));
        reset_store();
        db
    }

    /// 当前审计库中的条目总数。
    fn entry_count() -> usize {
        recent(200)["entries"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0)
    }

    /// 执行 `body` 并返回它在审计库中**新增**的条目数。
    ///
    /// 为什么按增量断言：`emit` / `audit` 落盘的是进程级共享的审计库，同进程内
    /// **其它模块的并行用例**也会往里写。断言绝对条数会把「别人也写了」误判为
    /// 本模块的 bug（这正是本模块此前偶发 `应有 3 条，实得 10 条` 的原因之一）。
    /// 增量断言只约束「本用例写了几条」，与并发无关，强度不降。
    fn count_after<F: FnOnce()>(body: F) -> usize {
        let before = entry_count();
        body();
        let after = entry_count();
        after.saturating_sub(before)
    }

    #[test]
    fn product_events_table_has_12_kinds() {
        assert_eq!(PRODUCT_EVENTS.len(), 12);
        assert!(PRODUCT_EVENTS.contains(&TASK_INITIATED));
        assert!(PRODUCT_EVENTS.contains(&MODE_CHANGED));
        assert!(PRODUCT_EVENTS.contains(&BUDGET_WARNING));
        assert!(PRODUCT_EVENTS.contains(&APPROVAL_GRANTED));
    }

    #[test]
    fn emit_persists_audit_and_can_read_recent() {
        let _g = TEST_LOCK.lock().unwrap();
        let db = with_fresh_audit_db();

        let added = count_after(|| {
            emit(TASK_INITIATED, "ws.s1", json!({ "task_id": "t1" }));
            emit(TASK_COMPLETED, "ws.s1", json!({ "task_id": "t1", "ok": true }));
            emit(BUDGET_WARNING, "ws.s1", json!({ "spent": 12.5, "threshold": 10.0 }));
        });
        assert_eq!(added, 3, "本用例应落盘 3 条产品事件");

        // 按 id 倒序：最新在前，故本用例最后发的 budget.warning 必在最前。
        let r = recent(10);
        let entries = r["entries"].as_array().expect("entries should be array");
        assert_eq!(entries[0]["event"], "budget.warning");
        // 本用例三条都在库里（顺序：budget.warning → task.completed → task.initiated）。
        let mine: Vec<&str> = entries
            .iter()
            .take(3)
            .filter_map(|e| e["event"].as_str())
            .collect();
        assert_eq!(
            mine,
            vec!["budget.warning", "task.completed", "task.initiated"],
            "本用例三条应按倒序连续出现: {r}"
        );

        // 先释放句柄再删文件（Windows 下锁定文件不能被删除）
        reset_store();
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn envelope_mapping_for_kinds() {
        assert_eq!(envelope_type(TOOL_CALL), EventType::ToolCall);
        assert_eq!(envelope_type(TASK_COMPLETED), EventType::Done);
        assert_eq!(envelope_type(APPROVAL_DENIED), EventType::Approval);
        assert_eq!(envelope_type(MODE_CHANGED), EventType::Status);
    }

    #[test]
    fn audit_only_also_persists() {
        let _g = TEST_LOCK.lock().unwrap();
        let db = with_fresh_audit_db();

        let added = count_after(|| {
            audit(MODE_CHANGED, "core", json!({ "from": "agent", "to": "bloom" }));
        });
        assert_eq!(added, 1, "本用例应落盘 1 条");

        let r = recent(5);
        let entries = r["entries"].as_array().expect("entries should be array");
        assert_eq!(entries[0]["event"], "mode.changed");

        reset_store();
        let _ = std::fs::remove_file(&db);
    }
}
