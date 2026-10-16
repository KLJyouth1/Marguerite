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
/// 行为日志文件字节上限；超过则截断保留最近行（防 append-only 无界增长）
const MAX_BEHAVIOR_BYTES: u64 = 1_000_000;
/// 截断后保留的最近行数（须使保留体积 < [`MAX_BEHAVIOR_BYTES`]，否则会反复重写）
const BEHAVIOR_KEEP_LINES: usize = 2000;
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
        // 有界：文件按字节上限截断（保留最近行，绝不丢最新事件）
        self.compact_file_if_needed();
    }

    /// 文件超过 [`MAX_BEHAVIOR_BYTES`] 时，截断为最近 [`BEHAVIOR_KEEP_LINES`] 行。
    ///
    /// 在持有台账互斥锁时调用，无并发写；只保留最新行，绝不丢最近事件。
    fn compact_file_if_needed(&self) {
        let len = match std::fs::metadata(&self.path) {
            Ok(m) => m.len(),
            Err(_) => return,
        };
        if len <= MAX_BEHAVIOR_BYTES {
            return;
        }
        let raw = match std::fs::read_to_string(&self.path) {
            Ok(r) => r,
            Err(_) => return,
        };
        let lines: Vec<&str> = raw.lines().collect();
        if lines.len() <= BEHAVIOR_KEEP_LINES {
            return; // 行数不多（单行过长）：不裁剪，避免丢数据
        }
        let keep = &lines[lines.len() - BEHAVIOR_KEEP_LINES..];
        let mut buf = String::new();
        for l in keep {
            buf.push_str(l);
            buf.push('\n');
        }
        let _ = std::fs::write(&self.path, buf);
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

/// 工具结果事件 → 台账 `detail` 文本（纯函数，便于单测）。
///
/// **为什么值得单独抽出来**：此前失败分支恒写字面量 `"FAILED"`，把事件里本已
/// 携带的错误文本在**写入点**丢掉。实测后果 —— 支持包里只剩
/// `shell_exec -> FAILED` × 63，既没有预检拒绝原因，也分不清「越界 / 护栏拦截 /
/// spawn 失败 / 超时」，失败**完全无法归因**（这是上一轮排查卡住的主因）。
///
/// 契约：成功 → 用 `summary`；失败 → 尽力拼出 `FAILED[code] reason`，
/// 依次尝试 `summary` / `error` / `message` / `detail` 四个字段取第一个非空文本。
/// 全都为空时**只**留 `FAILED` —— 不臆造原因。
pub fn describe_tool_result(tool: &str, ok: bool, payload: &Value) -> String {
    if ok {
        let summary = payload
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return format!("{tool} -> {summary}");
    }
    let reason = ["summary", "error", "message", "detail"]
        .iter()
        .find_map(|k| {
            payload
                .get(*k)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_default();
    let code = payload
        .get("code")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_default();
    match (code.is_empty(), reason.is_empty()) {
        (false, false) => format!("{tool} -> FAILED[{code}] {reason}"),
        (false, true) => format!("{tool} -> FAILED[{code}]"),
        (true, false) => format!("{tool} -> FAILED {reason}"),
        (true, true) => format!("{tool} -> FAILED"),
    }
}

/// 最近 `limit` 条行为事件（全局，按 seq 升序）。
///
/// 供元智能体监控层抽取失败信号（[`crate::harness::meta`]）——只读快照，
/// 不触发任何写入，也不改变台账的有界裁剪语义。
pub fn recent_events(limit: usize) -> Vec<BehaviorEvent> {
    ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .timeline(None, limit)
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
    tool_uses.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
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
    fn test_behavior_file_stays_bounded() {
        // B4 回归：behavior.jsonl 曾为 append-only 无上限 → 无界增长。
        let (mut l, dir) = tmp_ledger("filebound");
        let padding = "y".repeat(180);
        let n = 5000usize;
        for i in 0..n {
            l.record("s", "tool_call", &format!("tool_{i}_{padding}"));
        }
        let len = std::fs::metadata(&l.path).unwrap().len();
        assert!(
            len <= MAX_BEHAVIOR_BYTES + 4096,
            "行为日志文件应有界: {len} 字节"
        );
        // 最近事件不得丢失
        let raw = std::fs::read_to_string(&l.path).unwrap();
        assert!(
            raw.contains(&format!("tool_{}_", n - 1)),
            "最新事件不得丢失"
        );
        let _ = std::fs::remove_dir_all(&dir);
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

    /* ── 工具结果台账：失败必须可归因（回归钉子） ───────────────────────── */

    /// 成功：直接用 summary，不带任何 "FAILED" 前缀。
    #[test]
    fn describe_tool_result_success_uses_summary() {
        let p = json!({ "tool": "shell_exec", "ok": true, "summary": "exit 0" });
        assert_eq!(describe_tool_result("shell_exec", true, &p), "shell_exec -> exit 0");
    }

    /// **核心回归**：失败时必须保留真实原因 —— 这正是旧实现丢掉的东西。
    /// 旧行为恒为 `"shell_exec -> FAILED"`，本用例把它钉死。
    #[test]
    fn describe_tool_result_failure_keeps_reason() {
        let p = json!({
            "tool": "shell_exec",
            "ok": false,
            "summary": "命令被 L2 护栏拒绝：rm -rf"
        });
        let out = describe_tool_result("shell_exec", false, &p);
        assert_ne!(out, "shell_exec -> FAILED", "不得退化为无信息的 FAILED");
        assert!(out.contains("L2 护栏拒绝"), "必须保留真实原因: {out}");
    }

    /// 有稳定错误码时，码与原因并存（前端/人工都按码归因）。
    #[test]
    fn describe_tool_result_failure_includes_code_when_present() {
        let p = json!({ "ok": false, "code": "EXEC_ERROR", "error": "spawn 失败" });
        assert_eq!(
            describe_tool_result("shell_exec", false, &p),
            "shell_exec -> FAILED[EXEC_ERROR] spawn 失败"
        );
    }

    /// 字段回退顺序：summary → error → message → detail（第一个非空者胜出）。
    #[test]
    fn describe_tool_result_falls_back_across_fields() {
        let only_message = json!({ "ok": false, "message": "超时 120s" });
        assert!(describe_tool_result("t", false, &only_message).contains("超时 120s"));

        let only_detail = json!({ "ok": false, "detail": "无工作区" });
        assert!(describe_tool_result("t", false, &only_detail).contains("无工作区"));

        // 空白串不算「有原因」，必须继续回退。
        let blank_then_real = json!({ "ok": false, "summary": "   ", "error": "真的原因" });
        assert!(describe_tool_result("t", false, &blank_then_real).contains("真的原因"));
    }

    /// 全空时**只**留 FAILED —— 绝不臆造原因。
    #[test]
    fn describe_tool_result_never_invents_reason() {
        let empty = json!({ "ok": false });
        assert_eq!(describe_tool_result("shell_exec", false, &empty), "shell_exec -> FAILED");
    }
}
