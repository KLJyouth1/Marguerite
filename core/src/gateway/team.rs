//! 专家团队方法族（team.* / experts.* / agent.profile.*）
//!
//! 管理专家团队组合的持久化，执行专家团队一键启动（Team::run），
//! 以及用户画像分析与自动抽取。

use serde_json::{json, Value};

use crate::gateway::_helpers::param_str;
use crate::persistence::config::Config;

// ── 专家团队持久化组合（data_dir/team_compositions.json）──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, PartialEq)]
struct TeamCompositionMember {
    role: String,
    brief: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, PartialEq)]
struct TeamComposition {
    id: String,
    name: String,
    goal: String,
    members: Vec<TeamCompositionMember>,
}

fn team_compositions_path() -> std::path::PathBuf {
    Config::data_dir().join("team_compositions.json")
}

fn team_compositions_load() -> Vec<TeamComposition> {
    match std::fs::read_to_string(team_compositions_path()) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn team_compositions_save_all(items: &[TeamComposition]) {
    let p = team_compositions_path();
    let _ = std::fs::write(&p, serde_json::to_string_pretty(items).unwrap_or_default());
}

/// `team.compositions.list`：已保存的专家团队组合。
pub fn team_compositions_list() -> Value {
    let items: Vec<Value> = team_compositions_load()
        .into_iter()
        .map(|c| {
            json!({
                "id": c.id,
                "name": c.name,
                "goal": c.goal,
                "members": c.members.iter().map(|m| json!({ "role": m.role, "brief": m.brief })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({ "ok": true, "items": items, "total": items.len() })
}

/// `team.compositions.save`：保存/覆盖团队组合（name 唯一，成员数组 [{role,brief}]）。
pub fn team_compositions_save(params: &Value) -> Value {
    let name = param_str(params, "name", "name", "").trim().to_string();
    if name.is_empty() {
        return json!({ "ok": false, "error": "team.compositions.save 需要 name" });
    }
    let goal = param_str(params, "goal", "goal", "").trim().to_string();
    let mut members = Vec::new();
    if let Some(arr) = params.get("members").and_then(Value::as_array) {
        for m in arr {
            let role = m
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let brief = m
                .get("brief")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if role.is_empty() {
                continue;
            }
            members.push(TeamCompositionMember { role, brief });
        }
    }
    let mut items = team_compositions_load();
    items.retain(|c| c.name != name);
    let id = format!("team-{}", name.replace(" ", "-"));
    items.push(TeamComposition {
        id,
        name,
        goal,
        members,
    });
    team_compositions_save_all(&items);
    team_compositions_list()
}

/// `team.compositions.remove`：删除团队组合。
pub fn team_compositions_remove(params: &Value) -> Value {
    let name = param_str(params, "name", "name", "").trim();
    if name.is_empty() {
        return json!({ "ok": false, "error": "team.compositions.remove 需要 name" });
    }
    let mut items = team_compositions_load();
    items.retain(|c| c.name != name);
    team_compositions_save_all(&items);
    team_compositions_list()
}

/// `team.run`：专家团队一键启动（Team::run 真实并行 spawn：自动角色检测 → 成员并行 → lead 综合）。
/// 未配置 API Key 时成员真实启动但 LLM 调用失败——如实回报错误，不做假成功。
pub async fn team_run(params: &Value) -> Value {
    let goal = param_str(params, "goal", "goal", "");
    if goal.trim().is_empty() {
        return json!({ "ok": false, "error": "team.run 需要 goal（目标描述）" });
    }
    let cfg = Config::load().unwrap_or_default();
    let mut orch = crate::orchestration::Orchestrator::new(cfg);
    match orch.dispatch_team(goal).await {
        Ok(out) => json!({
            "ok": true,
            "strategy": out.strategy.label(),
            "members": out.subagent_summaries.len(),
            "summaries": out.subagent_summaries,
            "lead_reply": out.lead_reply,
        }),
        Err(e) => json!({ "ok": false, "error": format!("团队编排失败: {e}") }),
    }
}

/// `team.run.background`：蜂群后台化——注册 long_running 台账（心跳/检查点/完成态），
/// 立即返回 taskId；执行体在后台 tokio 任务中运行 dispatch_team，结果写入检查点。
pub async fn team_run_background(params: &Value) -> Value {
    let goal = param_str(params, "goal", "goal", "");
    if goal.trim().is_empty() {
        return json!({ "ok": false, "error": "team.run.background 需要 goal" });
    }
    let goal = goal.trim().to_string();
    let task_id = {
        let mut reg = crate::automation::long_running::global()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reg.register(&format!("[蜂群] {goal}"), 60)
    };
    let goal2 = goal.clone();
    let tid = task_id.clone();
    // 心跳循环：任务存续期每 15s 打点（complete/fail 后 heartbeat 返回 Err → 自动退出）
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(15));
        loop {
            tick.tick().await;
            let mut reg = crate::automation::long_running::global()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if reg.heartbeat(&tid).is_err() {
                break;
            }
        }
    });
    let tid2 = task_id.clone();
    tokio::spawn(async move {
        let cfg = Config::load().unwrap_or_default();
        let mut orch = crate::orchestration::Orchestrator::new(cfg);
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3600),
            orch.dispatch_team(&goal2),
        )
        .await;
        let mut reg = crate::automation::long_running::global()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(Ok(out)) => {
                let _ = reg.save_checkpoint(
                    &tid2,
                    crate::automation::long_running::Stage::Execute,
                    json!({ "lead_reply": out.lead_reply, "summaries": out.subagent_summaries, "strategy": out.strategy.label() }),
                );
                let _ = reg.complete(&tid2);
            }
            Ok(Err(e)) => {
                let _ = reg.save_checkpoint(
                    &tid2,
                    crate::automation::long_running::Stage::Execute,
                    json!({ "error": e.to_string() }),
                );
                let _ = reg.fail(&tid2);
            }
            Err(_) => {
                let _ = reg.save_checkpoint(
                    &tid2,
                    crate::automation::long_running::Stage::Execute,
                    json!({ "error": "后台蜂群执行超时（1h）" }),
                );
                let _ = reg.fail(&tid2);
            }
        }
    });
    json!({ "ok": true, "taskId": task_id, "message": "蜂群已后台启动（long_running 台账可见心跳/检查点/完成态）" })
}

/// `experts.list`：列出行业领军人经验蒸馏的专家档案库。
pub fn experts_list(_params: &Value) -> Value {
    let experts: Vec<Value> = crate::agent::expertise::experts_list()
        .iter()
        .map(|e| {
            json!({
                "id": e.id,
                "label": e.label,
                "lineage": e.lineage,
                "mindset": e.mindset,
                "expertise": e.expertise,
                "skills": e.skills,
                "playbook": e.playbook,
            })
        })
        .collect();
    json!({ "experts": experts })
}

/// `agent.profile.analyze`：从对话历史确定性抽取用户画像。
pub fn agent_profile_analyze(params: &Value) -> Value {
    let Some(messages) = params.get("messages").and_then(Value::as_array) else {
        return json!({ "error": "missing 'messages'", "code": "invalid_params" });
    };
    let history: Vec<crate::domain::Message> = messages
        .iter()
        .filter_map(|m| {
            let role = match m.get("role").and_then(Value::as_str).unwrap_or("") {
                "user" => crate::domain::Role::User,
                "assistant" => crate::domain::Role::Assistant,
                "system" => crate::domain::Role::System,
                _ => return None,
            };
            let content = m.get("content").and_then(Value::as_str).unwrap_or("");
            Some(crate::domain::Message {
                role,
                content: content.to_string(),
                meta: None,
            })
        })
        .collect();
    let persona = crate::agent::profile::auto_profile(&history);
    json!({
        "role": persona.role,
        "tone": persona.tone,
        "expertise": persona.expertise,
    })
}

/// `agent.profile.auto`：自动从记忆与会话历史中抽取用户画像，无需手动粘贴。
pub fn agent_profile_auto() -> Value {
    let mut all_entries = Vec::new();
    // 从上下文台账取最近会话的条目
    for cs in crate::context::session::context_sessions()
        .into_iter()
        .take(20)
    {
        let entries = crate::context::session::context_entries(&cs.session);
        all_entries.extend(entries.into_iter().take(20));
    }
    if all_entries.is_empty() {
        return json!({
            "role": "通用用户",
            "tone": "自然",
            "expertise": "暂无足够数据生成画像，多对话后自动更新",
            "auto": true,
            "source_count": 0,
        });
    }
    let history: Vec<crate::domain::Message> = all_entries
        .iter()
        .map(|e| {
            let role = match e.role.as_str() {
                "user" => crate::domain::Role::User,
                "assistant" => crate::domain::Role::Assistant,
                _ => crate::domain::Role::User,
            };
            crate::domain::Message {
                role,
                content: e.text.clone(),
                meta: None,
            }
        })
        .collect();
    let persona = crate::agent::profile::auto_profile(&history);
    json!({
        "role": persona.role,
        "tone": persona.tone,
        "expertise": persona.expertise,
        "auto": true,
        "source_count": all_entries.len(),
    })
}
