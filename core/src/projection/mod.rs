//! DSH session-projection fold 契约移植（上游 PR #2730 / #2702）
//!
//! 上游拆分：`ProjectionDefinition` 的 `stateSchema`（持久化 fold 状态）与
//! 可选 `wire.viewSchema`（客户端视图）分离；新增 `SessionProjectionStateMap`
//! 持久化表；支持 host-only 单元（无视图）；`stateOf()` 只读查询。
//!
//! 本移植的 Rust 形状：
//! - [`ProjectionUnit`]：`key` + `state_version` + 纯 `init`/`apply` fold +
//!   可选 `view`（返回 `None` 即 host-only 单元）。
//! - [`ProjectionStore`]：SQLite 表 `session_projection_state`
//!   `(projection_key, session_id)` → `(state_version, as_of_seq, state_json)`。
//! - [`ProjectionStore::replay`]：纯 fold 重建（不落盘，供重放/校验/测试）。
//!
//! fold 是**纯函数**（确定性、无副作用），重放同一事件序列必得同一状态——
//! 这是上游「事件日志持有状态，投影是可弃物」语义的本地形态。

use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

/// 一个投影单元：纯 `init`/`apply` fold + 可选客户端视图。
///
/// - `apply(state, event)` 必须确定性且无副作用；
/// - `view(state) -> None` 表示 host-only 单元（状态只进 `SessionProjectionStateMap`，
///   不进客户端视图表）。
pub trait ProjectionUnit: Send + Sync {
    /// 持久化 fold 状态类型（必须能 JSON 往返）。
    type State: Serialize + DeserializeOwned + Clone + Send + Sync + 'static;

    /// 投影键（同上游 key：`tokenUsage` / `contextPressure` / `turnCount`…）。
    fn key(&self) -> &'static str;

    /// 状态 schema 版本（迁移时据此弃旧重建）。
    fn state_version(&self) -> u32;

    /// 初始状态。
    fn init(&self) -> Self::State;

    /// 纯 fold：state × event → state。
    fn apply(&self, state: &Self::State, event: &Value) -> Self::State;

    /// 可选客户端视图（wire JSON 整值；渲染是槽位系统的事，与本层无关）。
    /// 默认 `None` = host-only 单元。
    fn view(&self, _state: &Self::State) -> Option<Value> {
        None
    }
}

/// 会话投影状态表（`SessionProjectionStateMap` 本地形态）。
///
/// `db_path = None` 时用内存库（测试/纯计算场景）。
pub struct ProjectionStore {
    conn: rusqlite::Connection,
}

impl ProjectionStore {
    /// 打开（或创建）投影状态库；`None` = 内存库。
    pub fn open(db_path: Option<PathBuf>) -> Result<Self, String> {
        let conn = match db_path {
            Some(p) => rusqlite::Connection::open(p).map_err(|e| format!("打开投影库失败: {e}"))?,
            None => rusqlite::Connection::open_in_memory()
                .map_err(|e| format!("创建内存投影库失败: {e}"))?,
        };
        let store = Self { conn };
        store.ensure_schema()?;
        Ok(store)
    }

    fn ensure_schema(&self) -> Result<(), String> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS session_projection_state (
                   projection_key TEXT NOT NULL,
                   session_id    TEXT NOT NULL,
                   state_version INTEGER NOT NULL,
                   as_of_seq     INTEGER NOT NULL DEFAULT 0,
                   state_json    TEXT NOT NULL,
                   PRIMARY KEY (projection_key, session_id)
                 );",
            )
            .map_err(|e| format!("建投影表失败: {e}"))
    }

    /// 读取某会话某投影的已持久化状态（原样 JSON；无记录返回 None）。
    pub fn state_of(&self, key: &str, session_id: &str) -> Option<Value> {
        self.conn
            .query_row(
                "SELECT state_json FROM session_projection_state
                 WHERE projection_key = ?1 AND session_id = ?2",
                rusqlite::params![key, session_id],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
    }

    /// 已持久化状态的版本（无记录返回 None）。
    pub fn state_version_of(&self, key: &str, session_id: &str) -> Option<u32> {
        self.conn
            .query_row(
                "SELECT state_version FROM session_projection_state
                 WHERE projection_key = ?1 AND session_id = ?2",
                rusqlite::params![key, session_id],
                |row| row.get::<_, i64>(0),
            )
            .ok()
            .map(|v| v as u32)
    }

    /// 折叠一批事件并把状态持久化（版本不一致时弃旧重建）。
    ///
    /// 返回折叠后的状态（JSON 整值）。`as_of_seq` 以事件数推进（调用方
    /// 可传 `base_seq` 对齐会话事件日志序号）。
    pub fn project<U: ProjectionUnit>(
        &mut self,
        unit: &U,
        session_id: &str,
        events: &[Value],
        base_seq: u64,
    ) -> Result<Value, String> {
        let mut state: U::State = match self.state_of(unit.key(), session_id) {
            Some(json)
                if self.state_version_of(unit.key(), session_id) == Some(unit.state_version()) =>
            {
                serde_json::from_value(json)
                    .map_err(|e| format!("投影状态反序列化失败({}): {e}", unit.key()))?
            }
            _ => unit.init(),
        };
        for event in events {
            state = unit.apply(&state, event);
        }
        let state_json = serde_json::to_string(&state)
            .map_err(|e| format!("投影状态序列化失败({}): {e}", unit.key()))?;
        let as_of = base_seq.saturating_add(events.len() as u64);
        self.conn
            .execute(
                "INSERT INTO session_projection_state
                   (projection_key, session_id, state_version, as_of_seq, state_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(projection_key, session_id) DO UPDATE SET
                   state_version = excluded.state_version,
                   as_of_seq = excluded.as_of_seq,
                   state_json = excluded.state_json",
                rusqlite::params![
                    unit.key(),
                    session_id,
                    unit.state_version() as i64,
                    as_of as i64,
                    state_json,
                ],
            )
            .map_err(|e| format!("投影状态落库失败: {e}"))?;
        serde_json::to_value(&state).map_err(|e| e.to_string())
    }

    /// 客户端视图（host-only 单元返回 None）。
    pub fn view_of<U: ProjectionUnit>(&self, unit: &U, session_id: &str) -> Option<Value> {
        let state: U::State =
            serde_json::from_value(self.state_of(unit.key(), session_id)?).ok()?;
        unit.view(&state)
    }

    /// 纯 fold 重建：从 `init` 折叠全部事件（不读库、不落盘）。
    pub fn replay<U: ProjectionUnit>(unit: &U, events: &[Value]) -> U::State {
        events
            .iter()
            .fold(unit.init(), |state, event| unit.apply(&state, event))
    }
}

/// 示例单元：轮次计数（视图 = `{"turns": n}`，对齐 token-meter 的 view 形状）。
#[derive(Debug, Default, Clone, Copy)]
pub struct TurnCounterUnit;

impl ProjectionUnit for TurnCounterUnit {
    type State = TurnCounterState;

    fn key(&self) -> &'static str {
        "turnCount"
    }

    fn state_version(&self) -> u32 {
        1
    }

    fn init(&self) -> Self::State {
        TurnCounterState {
            turns: 0,
            last: None,
        }
    }

    fn apply(&self, state: &Self::State, event: &Value) -> Self::State {
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let is_turn = matches!(kind, "user/message" | "assistant/message");
        if !is_turn {
            return state.clone();
        }
        TurnCounterState {
            turns: state.turns + 1,
            last: Some(kind.to_string()),
        }
    }

    fn view(&self, state: &Self::State) -> Option<Value> {
        Some(serde_json::json!({ "turns": state.turns }))
    }
}

/// 轮次计数状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct TurnCounterState {
    pub turns: u64,
    pub last: Option<String>,
}

/// 示例单元：host-only 审计（无视图——只进状态表，不出客户端）。
#[derive(Debug, Default, Clone, Copy)]
pub struct HostAuditUnit;

impl ProjectionUnit for HostAuditUnit {
    type State = HostAuditState;

    fn key(&self) -> &'static str {
        "hostAudit"
    }

    fn state_version(&self) -> u32 {
        1
    }

    fn init(&self) -> Self::State {
        HostAuditState {
            events: 0,
            last_event_type: None,
        }
    }

    fn apply(&self, state: &Self::State, event: &Value) -> Self::State {
        HostAuditState {
            events: state.events + 1,
            last_event_type: event
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string),
        }
    }
}

/// host-only 审计状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct HostAuditState {
    pub events: u64,
    pub last_event_type: Option<String>,
}

/// 会话 token 用量投影（移植上游 token-meter `tokenUsage` 单元语义）：
/// 样本先到先计；**同一步（`tool@ts`）的重复样本替换旧值而非双计**
/// （`addReplacing` 语义）；单槽 `last` 依赖「同一步的样本相邻」的日志不变量。
///
/// 事件形状（对齐 dsh_bridge `usage.accumulated` 载荷）：
/// `{"kind":"usage.accumulated","usage":{"tool_name","tokens_in","tokens_out","ts_ms"}}`
#[derive(Debug, Default, Clone, Copy)]
pub struct TokenUsageUnit;

impl ProjectionUnit for TokenUsageUnit {
    type State = TokenUsageState;

    fn key(&self) -> &'static str {
        "tokenUsage"
    }

    fn state_version(&self) -> u32 {
        1
    }

    fn init(&self) -> Self::State {
        TokenUsageState {
            totals: TokenUsageTotals {
                tokens_in: 0,
                tokens_out: 0,
                steps: 0,
            },
            last: None,
        }
    }

    fn apply(&self, state: &Self::State, event: &Value) -> Self::State {
        if event.get("kind").and_then(Value::as_str) != Some("usage.accumulated") {
            return state.clone();
        }
        let Some(usage) = event.get("usage") else {
            return state.clone();
        };
        let tool_name = usage
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let ts = usage.get("ts_ms").and_then(Value::as_i64).unwrap_or(0);
        let tokens_in = usage.get("tokens_in").and_then(Value::as_u64).unwrap_or(0);
        let tokens_out = usage.get("tokens_out").and_then(Value::as_u64).unwrap_or(0);
        let step_key = format!("{tool_name}@{ts}");
        // 同一步重复样本：替换旧值（-prev +next），否则累加
        let is_repeat = state.last.as_ref().is_some_and(|l| l.step_key == step_key);
        let (prev_in, prev_out, steps) = if is_repeat {
            let l = state.last.as_ref().expect("is_repeat 必有 last");
            (l.tokens_in, l.tokens_out, state.totals.steps)
        } else {
            (0, 0, state.totals.steps + 1)
        };
        TokenUsageState {
            totals: TokenUsageTotals {
                tokens_in: state.totals.tokens_in - prev_in + tokens_in,
                tokens_out: state.totals.tokens_out - prev_out + tokens_out,
                steps,
            },
            last: Some(LastUsageSample {
                step_key,
                tokens_in,
                tokens_out,
            }),
        }
    }

    fn view(&self, state: &Self::State) -> Option<Value> {
        Some(serde_json::json!({
            "tokens_in": state.totals.tokens_in,
            "tokens_out": state.totals.tokens_out,
            "steps": state.totals.steps,
        }))
    }
}

/// token 用量汇总桶。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct TokenUsageTotals {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub steps: u64,
}

/// token 用量投影状态（totals + 单槽 last）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct TokenUsageState {
    pub totals: TokenUsageTotals,
    pub last: Option<LastUsageSample>,
}

/// 最近一次样本（同一步替换语义的基准）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct LastUsageSample {
    pub step_key: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(kind: &str) -> Value {
        serde_json::json!({ "type": kind })
    }

    #[test]
    fn replay_folds_turn_events_deterministically() {
        let events = vec![
            turn("user/message"),
            turn("assistant/message"),
            turn("assistant/chunk"), // 非整轮，不计
            turn("user/message"),
        ];
        let state = ProjectionStore::replay(&TurnCounterUnit, &events);
        assert_eq!(state.turns, 3);
        assert_eq!(state.last.as_deref(), Some("user/message"));
        // 重放确定性：同序列同状态
        let again = ProjectionStore::replay(&TurnCounterUnit, &events);
        assert_eq!(again, state);
    }

    #[test]
    fn turn_counter_view_is_wire_json() {
        let state = ProjectionStore::replay(
            &TurnCounterUnit,
            &[turn("user/message"), turn("assistant/message")],
        );
        let view = TurnCounterUnit.view(&state).unwrap();
        assert_eq!(view, serde_json::json!({ "turns": 2 }));
    }

    #[test]
    fn host_only_unit_has_no_view() {
        let state = HostAuditUnit.init();
        assert!(HostAuditUnit.view(&state).is_none());
    }

    #[test]
    fn store_projects_and_persists_across_reopen() {
        let dir = std::env::temp_dir().join(format!("mgt_projection_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("projection.db");
        {
            let mut store = ProjectionStore::open(Some(db.clone())).unwrap();
            let state = store
                .project(
                    &TurnCounterUnit,
                    "s1",
                    &[turn("user/message"), turn("assistant/message")],
                    0,
                )
                .unwrap();
            assert_eq!(state["turns"], 2);
            assert_eq!(store.state_version_of(TurnCounterUnit.key(), "s1"), Some(1));
        }
        // 重开：状态仍在（事件日志持有状态之外的持久化投影表）
        let store = ProjectionStore::open(Some(db)).unwrap();
        let state = store.state_of(TurnCounterUnit.key(), "s1").unwrap();
        assert_eq!(state["turns"], 2);
        assert_eq!(
            store.view_of(&TurnCounterUnit, "s1"),
            Some(serde_json::json!({ "turns": 2 }))
        );
    }

    #[test]
    fn version_mismatch_rebuilds_from_init() {
        let dir = std::env::temp_dir().join(format!("mgt_projection_v_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("projection.db");
        let mut store = ProjectionStore::open(Some(db.clone())).unwrap();
        // 先以 v1 折叠 3 轮
        store
            .project(
                &TurnCounterUnit,
                "s1",
                &[
                    turn("user/message"),
                    turn("assistant/message"),
                    turn("user/message"),
                ],
                0,
            )
            .unwrap();
        // 人为把版本戳成 2（模拟 schema 升级后旧状态失效）
        store
            .conn
            .execute(
                "UPDATE session_projection_state SET state_version = 2 WHERE projection_key = 'turnCount'",
                [],
            )
            .unwrap();
        // 再折叠 1 轮：版本不一致 → 从 init 重建，只含新事件
        let state = store
            .project(&TurnCounterUnit, "s1", &[turn("user/message")], 3)
            .unwrap();
        assert_eq!(state["turns"], 1, "版本不一致应弃旧重建");
        assert_eq!(store.state_version_of(TurnCounterUnit.key(), "s1"), Some(1));
    }

    #[test]
    fn in_memory_store_isolates_sessions() {
        let mut store = ProjectionStore::open(None).unwrap();
        store
            .project(&HostAuditUnit, "a", &[turn("user/message")], 0)
            .unwrap();
        store
            .project(
                &HostAuditUnit,
                "b",
                &[turn("user/message"), turn("assistant/message")],
                0,
            )
            .unwrap();
        let a = store.state_of(HostAuditUnit.key(), "a").unwrap();
        let b = store.state_of(HostAuditUnit.key(), "b").unwrap();
        assert_eq!(a["events"], 1);
        assert_eq!(b["events"], 2);
        assert!(store.view_of(&HostAuditUnit, "a").is_none());
    }

    fn usage_event(tool: &str, ts: i64, tokens_in: u64, tokens_out: u64) -> Value {
        serde_json::json!({
            "kind": "usage.accumulated",
            "usage": {
                "tool_name": tool,
                "ts_ms": ts,
                "tokens_in": tokens_in,
                "tokens_out": tokens_out,
            }
        })
    }

    #[test]
    fn token_usage_fold_adds_across_steps_and_replaces_within_step() {
        let events = vec![
            usage_event("run_code", 1000, 100, 50),
            usage_event("web_search", 2000, 30, 10),
        ];
        let state = ProjectionStore::replay(&TokenUsageUnit, &events);
        assert_eq!(state.totals.tokens_in, 130);
        assert_eq!(state.totals.tokens_out, 60);
        assert_eq!(state.totals.steps, 2);
        // 同一步（同 tool@ts）重复样本：替换旧值而非双计（addReplacing）
        let replaced = ProjectionStore::replay(
            &TokenUsageUnit,
            &[
                usage_event("run_code", 1000, 100, 50),
                usage_event("run_code", 1000, 130, 70),
            ],
        );
        assert_eq!(replaced.totals.tokens_in, 130, "同一步应替换");
        assert_eq!(replaced.totals.tokens_out, 70);
        assert_eq!(replaced.totals.steps, 1, "替换不新增步数");
    }

    #[test]
    fn token_usage_ignores_foreign_events_and_views_totals() {
        let state = ProjectionStore::replay(
            &TokenUsageUnit,
            &[
                turn("user/message"), // 非 usage 事件：忽略
                usage_event("run_code", 1, 40, 20),
            ],
        );
        assert_eq!(state.totals.tokens_in, 40);
        let view = TokenUsageUnit.view(&state).unwrap();
        assert_eq!(
            view,
            serde_json::json!({ "tokens_in": 40, "tokens_out": 20, "steps": 1 })
        );
    }

    #[test]
    fn token_usage_store_roundtrip_across_reopen() {
        let dir =
            std::env::temp_dir().join(format!("mgt_projection_usage_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("projection.db");
        {
            let mut store = ProjectionStore::open(Some(db.clone())).unwrap();
            store
                .project(
                    &TokenUsageUnit,
                    "s1",
                    &[usage_event("run_code", 1, 100, 50)],
                    0,
                )
                .unwrap();
        }
        let store = ProjectionStore::open(Some(db)).unwrap();
        assert_eq!(
            store.view_of(&TokenUsageUnit, "s1"),
            Some(serde_json::json!({ "tokens_in": 100, "tokens_out": 50, "steps": 1 }))
        );
    }
}
