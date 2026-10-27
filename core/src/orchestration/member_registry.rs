//! 团队成员实时动向台账（P2-9 之后新增：团队看板的数据源）。
//!
//! ## 为什么要有它
//!
//! 用户反馈："团队没有独立的成员卡片可以查看每个成员的动向"。编排层其实**已经**
//! 在发布成员级进度帧（`ProgressReporter::emit` → `session.event.progress`，phase
//! 为 `member_started` / `tool_call` / `tool_result` / `member_completed`），但这些帧
//! 是**事件流**：页面在运行中能看到，晚一步打开就什么都没有，也没有"每个成员当前
//! 在做什么"的快照。
//!
//! 本模块是那些帧的**有界内存投影**：订阅同一个 emit 入口，按会话维护一份成员状态
//! 快照，供 `team.members` RPC 拉取。它只做聚合，不改变编排行为——emit 里调用点
//! 是纯观察，失败绝不影响主流程。
//!
//! ## 边界（如实登记）
//!
//! - **内存 + 结构性落盘**：进程内有界投影；`run_*` / `member_*` 等结构性阶段会写
//!   `activity/team-<session>.json`（原子替换），daemon 重启后 `team.members` 回落磁盘快照，
//!   于是「昨天那次团队跑成什么样」还能查到。逐帧的 `tool_call` **不**写盘（防打满磁盘）。
//! - **只覆盖走 ProgressReporter 的路径**：solo 单轮没有成员概念，返回空是**正确**的，
//!   不是"没实现"。
//! - `tokensUsed` 只有 payload 明确携带时才填，缺省为 null（不拿 0 冒充"没消耗"）。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

/// 单个会话最多保留的成员投影数。
///
/// **这是 UI 投影缓冲，不是执行上限**：执行侧按用户所选规模真实 spawn（不截断），
/// 这里只是看板快照的容量。超出时丢弃新增投影并回传 `dropped` 计数（见 [`render`]），
/// 让前端知道「看板只显示了前 N 个」，而不是误以为成员真的只有 N 个。
/// 取 512：足以覆盖常规蜂群（数十人），又不至于让长跑 daemon 的内存无界增长。
const MAX_MEMBERS_PER_SESSION: usize = 512;
/// 最多跟踪的会话数（LRU 按 updated_ms 淘汰）。
const MAX_SESSIONS: usize = 32;

/// 一个成员的当前状态。
#[derive(Debug, Clone)]
struct Member {
    id: String,
    label: String,
    role: String,
    kind: String,
    status: String,
    current_step: Option<String>,
    tool_calls: u64,
    last_activity_ms: Option<i64>,
    tokens_used: Option<u64>,
}

/// 一次编排运行在某个会话下的投影。
#[derive(Debug, Clone)]
struct Run {
    run_id: String,
    /// 编排形态：team / swarm / solo / …
    form: String,
    updated_ms: i64,
    lead_status: String,
    lead_step: Option<String>,
    /// 主智能体自己的工具调用数。
    ///
    /// 此前卡片上 lead 的 `toolCalls` 恒为 0（写死的字面量）：成员的工具帧带 `memberId`，
    /// 而 lead 自己的工具帧**不带** —— 旧代码把「没有 memberId」的帧直接丢掉了。
    lead_tool_calls: u64,
    members: Vec<Member>,
    /// 因投影缓冲满而被丢弃的成员数（对外如实回传，不静默）。
    dropped: u64,
}

fn store() -> &'static Mutex<HashMap<String, Run>> {
    static S: OnceLock<Mutex<HashMap<String, Run>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// 成员种类的归一化：把编排层的 role 文案映射到前端四类徽章。
///
/// 映射是**保守**的：认不出就归 `subagent`，绝不把普通成员吹成 `expert`。
fn kind_of(role: &str) -> &'static str {
    let r = role.to_ascii_lowercase();
    if r.contains("lead") || r.contains("主智能体") || r.contains("orchestrator") {
        "lead"
    } else if r.contains("swarm") || r.contains("蜂群") {
        "swarm"
    } else if r.contains("expert") || r.contains("专家") {
        "expert"
    } else if r.contains("team") || r.contains("团队") {
        "team"
    } else {
        "subagent"
    }
}

/// 从进度帧更新投影。
///
/// 未知 phase 直接忽略（编排层以后加新阶段不会把这里变成错误源）。
pub fn observe(session: &str, phase: &str, payload: &Value) {
    let session = session.trim();
    if session.is_empty() {
        return;
    }
    let run_id = payload
        .get("runId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let member_id = payload
        .get("memberId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let role = payload
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let now = now_ms();

    let Ok(mut guard) = store().lock() else {
        return;
    };

    // 会话数上限：超了就淘汰最久未更新的会话（避免长跑 daemon 内存缓慢增长）。
    if guard.len() >= MAX_SESSIONS && !guard.contains_key(session) {
        if let Some(oldest) = guard
            .iter()
            .min_by_key(|(_, r)| r.updated_ms)
            .map(|(k, _)| k.clone())
        {
            guard.remove(&oldest);
        }
    }

    let entry = guard.entry(session.to_string()).or_insert_with(|| Run {
        run_id: run_id.clone(),
        form: "team".to_string(),
        updated_ms: now,
        lead_status: "waiting".to_string(),
        lead_step: None,
        lead_tool_calls: 0,
        members: Vec::new(),
        dropped: 0,
    });
    // 新 run 覆盖旧 run：同一会话里后一次编排才是用户此刻关心的。
    if !run_id.is_empty() && entry.run_id != run_id {
        entry.run_id = run_id.clone();
        entry.members.clear();
        entry.lead_status = "waiting".to_string();
        entry.lead_step = None;
        entry.lead_tool_calls = 0;
        entry.dropped = 0;
    }
    entry.updated_ms = now;

    match phase {
        "run_started" => {
            entry.form = payload
                .get("form")
                .and_then(Value::as_str)
                .unwrap_or("team")
                .to_string();
            entry.lead_status = "running".to_string();
            entry.lead_step = Some("编排中：分配任务给成员".to_string());
        }
        "lead_synthesizing" => {
            entry.lead_status = "running".to_string();
            let n = payload.get("members").and_then(Value::as_u64).unwrap_or(0);
            entry.lead_step = Some(format!("综合 {n} 名成员的结果"));
        }
        "run_completed" => {
            entry.lead_status = "finished".to_string();
            entry.lead_step = Some("已完成".to_string());
        }
        "run_failed" => {
            entry.lead_status = "failed".to_string();
            entry.lead_step = payload
                .get("error")
                .and_then(Value::as_str)
                .map(|e| format!("失败：{e}"));
        }
        "member_started" => {
            if member_id.is_empty() {
                return;
            }
            let step = payload
                .get("task")
                .or_else(|| payload.get("goal"))
                .or_else(|| payload.get("step"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let existed = entry.members.iter().any(|m| m.id == member_id);
            let full = entry.members.len() >= MAX_MEMBERS_PER_SESSION;
            upsert_member(entry, &member_id, &role, "running", now, |m| {
                if step.is_some() {
                    m.current_step = step;
                }
            });
            // 每个成员只在「首次出现」且缓冲区**已满**时计一次丢弃，
            // 避免后续 tool 帧把计数灌水，也避免把「刚好填满最后一个槽」误记为丢弃。
            if !existed && full {
                entry.dropped += 1;
            }
        }
        "tool_call" => {
            let tool = payload
                .get("tool")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string();
            if member_id.is_empty() {
                // 没有 memberId = 主智能体（run 级上报器）自己的工具调用。
                // 旧代码在这里直接 return，lead 的 toolCalls 于是永远是写死的 0。
                entry.lead_tool_calls += 1;
                entry.lead_step = Some(format!("调用 {tool}"));
            } else {
                upsert_member(entry, &member_id, &role, "running", now, |m| {
                    m.tool_calls += 1;
                    m.current_step = Some(format!("调用 {tool}"));
                });
            }
        }
        "tool_result" => {
            let tool = payload
                .get("tool")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string();
            let ok = payload.get("ok").and_then(Value::as_bool);
            let outcome = match ok {
                Some(true) => format!("{tool} 完成"),
                Some(false) => format!("{tool} 失败"),
                None => format!("{tool} 返回"),
            };
            if member_id.is_empty() {
                entry.lead_step = Some(outcome);
            } else {
                upsert_member(entry, &member_id, &role, "running", now, |m| {
                    m.current_step = Some(outcome);
                });
            }
        }
        "member_completed" => {
            if member_id.is_empty() {
                return;
            }
            let ok = payload.get("ok").and_then(Value::as_bool).unwrap_or(true);
            let tokens = payload.get("tokensUsed").and_then(Value::as_u64);
            upsert_member(
                entry,
                &member_id,
                &role,
                if ok { "finished" } else { "failed" },
                now,
                |m| {
                    m.current_step = Some(if ok {
                        "已完成".to_string()
                    } else {
                        "失败".to_string()
                    });
                    if tokens.is_some() {
                        m.tokens_used = tokens;
                    }
                },
            );
        }
        _ => {}
    }
    // 落盘只在「结构性变化」时做：run 边界与成员终态。
    // tool_call 是最频繁的帧，逐帧写盘会把磁盘打满，而它的信息在下一帧就被覆盖。
    if matches!(
        phase,
        "run_started" | "member_started" | "member_completed" | "run_completed" | "run_failed"
    ) {
        let payload = render(session, entry);
        crate::persistence::activity_store::persist_team_snapshot(session, &payload);
    }
}

/// 插入或更新成员；`f` 只在成员已存在时执行（新成员从默认态开始）。
fn upsert_member(
    run: &mut Run,
    id: &str,
    role: &str,
    status: &str,
    now: i64,
    f: impl FnOnce(&mut Member),
) {
    if let Some(m) = run.members.iter_mut().find(|m| m.id == id) {
        m.status = status.to_string();
        m.last_activity_ms = Some(now);
        if !role.is_empty() {
            m.role = role.to_string();
        }
        f(m);
        return;
    }
    if run.members.len() >= MAX_MEMBERS_PER_SESSION {
        // 缓冲满：不覆盖既有成员（丢弃计数由 `observe` 在 member_started 处按成员去重累计）。
        return;
    }
    let label = if role.is_empty() {
        id.to_string()
    } else {
        role.to_string()
    };
    let mut m = Member {
        id: id.to_string(),
        label,
        role: if role.is_empty() {
            "成员".to_string()
        } else {
            role.to_string()
        },
        kind: kind_of(role).to_string(),
        status: status.to_string(),
        current_step: None,
        tool_calls: 0,
        last_activity_ms: Some(now),
        tokens_used: None,
    };
    f(&mut m);
    run.members.push(m);
}

/// 把一次运行渲染成对外 JSON（落盘与返回共用同一份形状，避免两边漂移）。
fn render(session: &str, run: &Run) -> Value {
    let mut members: Vec<Value> = Vec::with_capacity(run.members.len() + 1);
    members.push(json!({
        "id": format!("lead:{}", run.run_id),
        "label": "主智能体",
        "role": "lead",
        "kind": "lead",
        "status": run.lead_status,
        "currentStep": run.lead_step,
        "toolCalls": run.lead_tool_calls,
        "lastActivityMs": run.updated_ms,
        "tokensUsed": Value::Null,
    }));
    for m in &run.members {
        members.push(json!({
            "id": m.id,
            "label": m.label,
            "role": m.role,
            "kind": m.kind,
            "status": m.status,
            "currentStep": m.current_step,
            "toolCalls": m.tool_calls,
            "lastActivityMs": m.last_activity_ms,
            "tokensUsed": m.tokens_used,
        }));
    }
    json!({
        "ok": true,
        "session": session,
        "runId": run.run_id,
        "form": run.form,
        "updatedAt": run.updated_ms,
        "members": members,
        // 投影缓冲容量与丢弃计数：让「只显示了前 N 名」可见，而不是假装全部成员都在。
        "memberLimit": MAX_MEMBERS_PER_SESSION,
        "dropped": run.dropped,
    })
}

/// `team.members` 的返回体。
///
/// `session` 为空时取**最近更新的**会话（对话页没传会话也能看到正在跑的团队）。
pub fn snapshot(session: Option<&str>) -> Value {
    let Ok(guard) = store().lock() else {
        return json!({ "ok": false, "code": "MEMBER_REGISTRY_UNAVAILABLE", "members": [] });
    };
    let picked: Option<(&String, &Run)> = match session.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => guard.get_key_value(s),
        None => guard.iter().max_by_key(|(_, r)| r.updated_ms),
    };
    let Some((key, run)) = picked else {
        // 内存里没有（daemon 重启 / 会话已结束）→ 回落落盘快照：
        // 「昨天那次团队跑成什么样」不该因为重启就查不到。
        if let Some(mut saved) = crate::persistence::activity_store::load_team_snapshot_any(
            session.map(str::trim).filter(|s| !s.is_empty()),
        ) {
            if let Some(obj) = saved.as_object_mut() {
                obj.insert("ok".into(), json!(true));
                obj.insert("fromDisk".into(), json!(true));
            }
            return saved;
        }
        return json!({
            "ok": true,
            "session": session.unwrap_or(""),
            "updatedAt": Value::Null,
            "form": Value::Null,
            "members": [],
            "note": "当前没有团队编排运行记录（单选直答或尚未派活）",
        });
    };

    // 与落盘同形：渲染逻辑只有 `render` 一处，避免内存视图与磁盘快照字段漂移。
    render(key, run)
}

/// 清空某会话的投影（`sessions.delete` / 测试隔离用）。
pub fn clear(session: &str) {
    if let Ok(mut g) = store().lock() {
        g.remove(session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observes_member_lifecycle_into_snapshot() {
        let s = "sess-member-test";
        observe(s, "run_started", &json!({ "runId": "r1", "form": "swarm" }));
        observe(
            s,
            "member_started",
            &json!({ "runId": "r1", "memberId": "m1", "role": "前端专家", "task": "改登录页" }),
        );
        observe(
            s,
            "tool_call",
            &json!({ "runId": "r1", "memberId": "m1", "role": "前端专家", "tool": "read_file" }),
        );
        observe(
            s,
            "tool_result",
            &json!({ "runId": "r1", "memberId": "m1", "role": "前端专家", "tool": "read_file", "ok": true }),
        );

        let v = snapshot(Some(s));
        assert_eq!(v["ok"], true);
        let members = v["members"].as_array().unwrap();
        // lead + 1 名成员
        assert_eq!(members.len(), 2, "{v}");
        assert_eq!(members[0]["kind"], "lead");
        assert_eq!(members[0]["status"], "running");
        let m = &members[1];
        assert_eq!(m["id"], "m1");
        assert_eq!(m["kind"], "expert", "「前端专家」应归为 expert 类");
        assert_eq!(m["status"], "running");
        assert_eq!(m["toolCalls"], 1);
        assert!(m["currentStep"].as_str().unwrap().contains("read_file"));

        observe(
            s,
            "member_completed",
            &json!({ "runId": "r1", "memberId": "m1", "role": "前端专家", "ok": true, "tokensUsed": 1234 }),
        );
        observe(s, "run_completed", &json!({ "runId": "r1", "ok": true }));
        let v = snapshot(Some(s));
        let members = v["members"].as_array().unwrap();
        assert_eq!(members[0]["status"], "finished");
        assert_eq!(members[1]["status"], "finished");
        assert_eq!(members[1]["tokensUsed"], 1234);
        clear(s);
    }

    #[test]
    fn empty_session_returns_honest_empty_not_fake_members() {
        let v = snapshot(Some("never-existed-session"));
        assert_eq!(v["ok"], true);
        assert_eq!(v["members"].as_array().unwrap().len(), 0);
        assert!(v["note"].as_str().unwrap().contains("没有团队编排运行记录"));
    }

    #[test]
    fn new_run_resets_previous_members() {
        let s = "sess-run-reset";
        observe(s, "run_started", &json!({ "runId": "r1", "form": "team" }));
        observe(s, "member_started", &json!({ "runId": "r1", "memberId": "old", "role": "x" }));
        observe(s, "run_started", &json!({ "runId": "r2", "form": "team" }));
        let v = snapshot(Some(s));
        let members = v["members"].as_array().unwrap();
        assert_eq!(members.len(), 1, "新 run 应清掉旧成员，只剩 lead：{v}");
        assert_eq!(v["runId"], "r2");
        clear(s);
    }

    #[test]
    fn failed_member_is_marked_failed_not_finished() {
        let s = "sess-fail";
        observe(s, "run_started", &json!({ "runId": "r9", "form": "team" }));
        observe(s, "member_started", &json!({ "runId": "r9", "memberId": "m", "role": "后端" }));
        observe(
            s,
            "member_completed",
            &json!({ "runId": "r9", "memberId": "m", "role": "后端", "ok": false }),
        );
        let v = snapshot(Some(s));
        assert_eq!(v["members"][1]["status"], "failed");
        clear(s);
    }

    #[test]
    fn unknown_phase_is_ignored_without_panic() {
        let s = "sess-unknown";
        observe(s, "brand_new_phase", &json!({ "runId": "r" }));
        // 未见过任何帧 → 空成员，不 panic
        let v = snapshot(Some(s));
        assert_eq!(v["members"].as_array().unwrap().len(), 1, "只有 lead 占位：{v}");
        clear(s);
    }

    /// 投影缓冲是 UI 容量（512），不是执行上限：超出时必须回传 `dropped`，
    /// 让前端知道「只显示了前 N 名」，而不是误以为成员真的只有 N 个。
    #[test]
    fn member_projection_limit_reports_dropped_instead_of_silently_hiding() {
        let s = "sess-member-limit";
        observe(s, "run_started", &json!({ "runId": "r-cap", "form": "swarm" }));
        // 用 tool_call 填满缓冲：该阶段不落盘（逐帧写盘会拖慢测试/打满磁盘）。
        for i in 0..MAX_MEMBERS_PER_SESSION {
            observe(
                s,
                "tool_call",
                &json!({ "runId": "r-cap", "memberId": format!("m{i}"), "role": "蜂群成员", "tool": "read_file" }),
            );
        }
        // 第 513 名成员的首次出现被丢弃 → 必须计数。
        observe(
            s,
            "member_started",
            &json!({ "runId": "r-cap", "memberId": "overflow", "role": "蜂群成员" }),
        );
        let v = snapshot(Some(s));
        assert_eq!(v["memberLimit"], MAX_MEMBERS_PER_SESSION);
        // lead + 512 名投影成员
        assert_eq!(
            v["members"].as_array().unwrap().len(),
            MAX_MEMBERS_PER_SESSION + 1,
            "投影只保留缓冲容量：{v}"
        );
        assert_eq!(v["dropped"], 1, "超出容量的成员必须如实计数：{v}");
        clear(s);
    }
}
