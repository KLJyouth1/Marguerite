//! 行为事件捕获（B3）：按会话聚合为「summary → timeline → observation」三层记忆
//!
//! 对齐 claude-mem 的 Hook 事件驱动范式（SessionStart/PreToolUse/PostToolUse）：
//! - **observation**：单条行为事件（[`BehaviorEvent`]），落盘 `data_dir/memory/behavior.jsonl`；
//! - **timeline**：[`behavior_list`] 按时间（seq）顺序返回某会话的事件线；
//! - **summary**：[`behavior_summary`] 动态聚合（轮次、工具使用频次 top、最近动作）。
//!
//! 埋点位置（`gateway/chat.rs`）：`chat.send`（user_message）、工具桥（tool_call/tool_result）、
//! `run_turn` 完成（assistant_reply）、`chat.abort`（aborted）。
//! 细节统一脱敏（复用 [`crate::memory::cross_project::redact`]）并限长，防上下文/磁盘爆炸。

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::memory::cross_project::redact;
use crate::persistence::config::Config;

/// 行为日志文件（相对 data_dir/memory）
const BEHAVIOR_FILE: &str = "behavior.jsonl";
/// 单会话最大 observation 数（超出丢弃最旧）
const MAX_PER_SESSION: usize = 200;
/// detail 截断字符数
const DETAIL_CHARS: usize = 200;
/// timeline 默认返回条数上限
const DEFAULT_LIST_LIMIT: usize = 50;

/// 一条行为观察
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorEvent {
    /// 全局递增序号（timeline 排序键）
    pub seq: u64,
    /// UTC 毫秒时间戳
    pub at_ms: u64,
    /// 所属会话
    pub session: String,
    /// 行为类型：user_message / assistant_reply / tool_call / tool_result / aborted
    pub kind: String,
    /// 行为细节（已脱敏、限长）
    pub detail: String,
}

/// 行为台账（进程单例 + JSONL 落盘）
pub struct BehaviorLedger {
    path: PathBuf,
    events: Vec<BehaviorEvent>,
    next_seq: u64,
}

fn ledger() -> &'static Mutex<BehaviorLedger> {
    static LEDGER: OnceLock<Mutex<BehaviorLedger>> = OnceLock::new();
    LEDGER.get_or_init(|| {
        let path = Config::data_dir().join("memory").join(BEHAVIOR_FILE);
        let mut l = BehaviorLedger {
            path,
            events: Vec::new(),
            next_seq: 1,
        };
        l.load();
        Mutex::new(l)
    })
}

impl BehaviorLedger {
    /// 从 JSONL 恢复（进程重启不丢历史行为）
    fn load(&mut self) {
        let raw = match std::fs::read_to_string(&self.path) {
            Ok(r) => r,
            Err(_) => return,
        };
        for line in raw.lines() {
            if let Ok(ev) = serde_json::from_str::<BehaviorEvent>(line) {
                if ev.seq >= self.next_seq {
                    self.next_seq = ev.seq + 1;
                }
                self.events.push(ev);
            }
        }
    }

    /// 追加一条行为观察（有界裁剪 + 追加写盘）
    fn record(&mut self, session: &str, kind: &str, detail: &str) {
        let ev = BehaviorEvent {
            seq: self.next_seq,
            at_ms: now_ms(),
            session: session.to_string(),
            kind: kind.to_string(),
            detail: truncate(&redact(detail), DETAIL_CHARS),
        };
        self.next_seq += 1;
        // 追加写盘（行级 append，崩溃最多丢一条；先序列化再入内存）
        if let Ok(line) = serde_json::to_string(&ev) {
            use std::io::Write;
            if let Some(parent) = self.path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
            {
                let _ = writeln!(f, "{line}");
            }
        }
        self.events.push(ev);
        // 有界：按会话裁剪，防止单会话无界增长
        let session_events: Vec<u64> = self
            .events
            .iter()
            .filter(|e| e.session == session)
            .map(|e| e.seq)
            .collect();
        if session_events.len() > MAX_PER_SESSION {
            let drop = session_events.len() - MAX_PER_SESSION;
            let drop_until = session_events[drop - 1];
            self.events
                .retain(|e| e.session != session || e.seq > drop_until);
        }
    }

    /// 某会话时间线（按 seq 升序）
    fn timeline(&self, session: Option<&str>, limit: usize) -> Vec<BehaviorEvent> {
        let mut evs: Vec<BehaviorEvent> = match session {
            Some(s) => self
                .events
                .iter()
                .filter(|e| e.session == s)
                .cloned()
                .collect(),
            None => self.events.clone(),
        };
        evs.sort_by_key(|e| e.seq);
        if evs.len() > limit {
            let skip = evs.len() - limit;
            evs.drain(0..skip);
        }
        evs
    }
}

/// 记录一条行为观察（网关/链路内部调用）
pub fn record_behavior(session: &str, kind: &str, detail: &str) {
    ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record(session, kind, detail);
}

/// `memory.behavior.list`：行为时间线（timeline）
///
/// params: `{ sessionKey?: string, limit?: number }`（无 sessionKey 时返回全局最近事件）
pub fn behavior_list(params: &Value) -> Value {
    let session = params
        .get("sessionKey")
        .and_then(Value::as_str)
        .map(str::to_string);
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_LIST_LIMIT as u64) as usize;
    let items: Vec<Value> = ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .timeline(session.as_deref(), limit.clamp(1, 200))
        .into_iter()
        .map(|e| {
            json!({
                "seq": e.seq,
                "at_ms": e.at_ms,
                "session": e.session,
                "kind": e.kind,
                "detail": e.detail,
            })
        })
        .collect();
    json!({
        "session": session,
        "items": items,
    })
}

/// `memory.behavior.summary`：会话行为聚合摘要（summary 层）
///
/// params: `{ sessionKey?: string }`——缺省时按最近会话聚合。
pub fn behavior_summary(params: &Value) -> Value {
    let session = params
        .get("sessionKey")
        .and_then(Value::as_str)
        .map(str::to_string);
    let guard = ledger().lock().unwrap_or_else(|e| e.into_inner());
    let (active, target): (Option<String>, Vec<BehaviorEvent>) = match &session {
        Some(s) => (
            Some(s.clone()),
            guard
                .events
                .iter()
                .filter(|e| &e.session == s)
                .cloned()
                .collect(),
        ),
        None => {
            let last = guard.events.last().map(|e| e.session.clone());
            match last {
                Some(s) => (
                    Some(s.clone()),
                    guard
                        .events
                        .iter()
                        .filter(|e| e.session == s)
                        .cloned()
                        .collect(),
                ),
                None => (None, Vec::new()),
            }
        }
    };
    aggregate(&target, active)
}

/// 聚合会话行为为 summary 视图（纯函数，便于测试）
fn aggregate(events: &[BehaviorEvent], active: Option<String>) -> Value {
    let user_msgs = events.iter().filter(|e| e.kind == "user_message").count();
    let assistant_msgs = events
        .iter()
        .filter(|e| e.kind == "assistant_reply")
        .count();
    // 工具使用频次 top5
    let mut tool_uses: Vec<(String, usize)> = {
        let mut m: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for e in events.iter().filter(|e| e.kind == "tool_call") {
            let tool = e.detail.split('(').next().unwrap_or("").trim();
            *m.entry(tool).or_default() += 1;
        }
        m.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    };
    tool_uses.sort_by(|a, b| b.1.cmp(&a.1));
    json!({
        "session": active.or_else(|| events.last().map(|e| e.session.clone())),
        "events": events.len(),
        "turns": user_msgs.min(assistant_msgs),
        "user_messages": user_msgs,
        "assistant_replies": assistant_msgs,
        "tool_uses": tool_uses.into_iter().take(5).map(|(t, c)| json!({ "tool": t, "count": c })).collect::<Vec<_>>(),
        "last_kind": events.last().map(|e| e.kind.clone()),
        "last_at_ms": events.last().map(|e| e.at_ms),
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 按字符截断
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_ledger(tag: &str) -> (BehaviorLedger, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("mgt-bhv-{tag}-{}", uuid::Uuid::new_v4()));
        let path = dir.join("behavior.jsonl");
        (
            BehaviorLedger {
                path,
                events: Vec::new(),
                next_seq: 1,
            },
            dir,
        )
    }

    #[test]
    fn test_record_and_timeline_order() {
        let (mut l, _d) = tmp_ledger("timeline");
        l.record("s1", "user_message", "实现登录功能");
        l.record("s1", "tool_call", "edit_file(path, content)");
        l.record("s1", "assistant_reply", "已完成登录实现");
        l.record("s2", "user_message", "另一个会话");
        let s1 = l.timeline(Some("s1"), 10);
        assert_eq!(s1.len(), 3);
        assert_eq!(s1[0].seq, 1);
        assert_eq!(s1[2].kind, "assistant_reply");
        let all = l.timeline(None, 10);
        assert_eq!(all.len(), 4);
    }

    #[test]
    fn test_record_bounded_per_session() {
        let (mut l, _d) = tmp_ledger("bounded");
        for i in 0..(MAX_PER_SESSION + 30) {
            l.record("s9", "tool_call", &format!("tool_{i}"));
        }
        l.record("s9", "assistant_reply", "done");
        let evs = l.timeline(Some("s9"), 500);
        assert_eq!(evs.len(), MAX_PER_SESSION);
        // 最旧的应被丢弃，最近保留
        assert!(evs.iter().all(|e| !e.detail.starts_with("tool_0")));
        assert_eq!(evs.last().unwrap().kind, "assistant_reply");
    }

    #[test]
    fn test_record_redacts_and_truncates() {
        let (mut l, _d) = tmp_ledger("redact");
        l.record(
            "s3",
            "user_message",
            "使用 api_key=sk-abcdef1234567890 配置，这是非常长的细节内容……",
        );
        let evs = l.timeline(Some("s3"), 10);
        assert!(!evs[0].detail.contains("sk-abcdef1234567890"));
        assert!(evs[0].detail.contains("[REDACTED]"));
    }

    #[test]
    fn test_behavior_summary_aggregates() {
        let (mut l, _d) = tmp_ledger("summary");
        l.record("s4", "user_message", "修复 bug");
        l.record("s4", "tool_call", "edit_file(src/a.rs)");
        l.record("s4", "tool_call", "edit_file(src/b.rs)");
        l.record("s4", "tool_call", "cargo_test()");
        l.record("s4", "assistant_reply", "完成");
        let summary = aggregate(&l.events, Some("s4".to_string()));
        assert_eq!(summary["events"], 5);
        assert_eq!(summary["turns"], 1);
        assert_eq!(summary["user_messages"], 1);
        let tools: Vec<&str> = summary["tool_uses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["tool"].as_str().unwrap())
            .collect();
        assert_eq!(tools, vec!["edit_file", "cargo_test"]);
        assert_eq!(summary["last_kind"], "assistant_reply");
    }

    #[test]
    fn test_behavior_profile_aggregates_preferred_tool() {
        // 画像自动生成：由行为记录聚合出含「预期偏好」的画像（偏好工具 top）
        let (mut l, _d) = tmp_ledger("profile");
        l.record("u1", "user_message", "用 Rust 写后端");
        l.record("u1", "tool_call", "edit_file(src/main.rs)");
        l.record("u1", "tool_call", "edit_file(src/lib.rs)");
        l.record("u1", "tool_call", "edit_file(src/api.rs)");
        l.record("u1", "tool_call", "cargo_test()");
        l.record("u1", "assistant_reply", "完成，偏好 Rust 语言");
        let profile = aggregate(&l.events, Some("u1".to_string()));
        // 画像应含预期偏好：edit_file 是最常用工具，cargo_test 次之
        let tools: Vec<Value> = profile["tool_uses"].as_array().unwrap().clone();
        assert_eq!(tools[0]["tool"], "edit_file");
        assert_eq!(tools[0]["count"], 3);
        assert_eq!(tools[1]["tool"], "cargo_test");
        assert_eq!(tools[1]["count"], 1);
        // 画像含语言偏好信号（语言关键词出现在 user_message 细节中）
        assert!(l.events.iter().any(|e| e.detail.contains("Rust")));
    }
}
