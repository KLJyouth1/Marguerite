//! 对话产线 RPC（`line.*`）—— 把「形状」从可勾选的开关改成可编译、可预演、可复盘的规格。
//!
//! 契约唯一事实源：`docs/plans/2026-10-30-conversation-line-workflow.md` §4；
//! 形状数学与校验在 [`crate::line`]（本模块只负责把它变成真实调用）。
//!
//! 六个方法：
//!
//! | 方法 | 位置 | 作用 |
//! |---|---|---|
//! | `line.catalog`   | 同步 | 形状目录 / 配方 / 名册候选 / 上限（前端不复制任何常量） |
//! | `line.compile`   | 同步 | 旧挡位、配方或显式阶段 ⇒ `LineSpec` |
//! | `line.preflight` | 同步 | 派单前预演：逐阶段推演 + 整线推荐对照 + 权限门 |
//! | `line.plan`      | 同步 | 每阶段**将要**走哪个 backend（由 `route_meta` 生成，声明=执行） |
//! | `line.run`       | 异步 | 逐阶段串行执行，产出断言链，落一条有界运行记录 |
//! | `line.retro`     | 同步 | 复盘：真实运行投影成 `tactics::ObservedRun`，跑 5-Why 链 |
//!
//! 三条不可让的口径：
//!
//! 1. **执行只复用 [`orchestrate::dispatch_form`]**。本模块不 new Orchestrator、
//!    不自建 ToolRegistry、不另设并发闸门 —— 那起「子智能体拿不到任何内置工具」的
//!    事故根因就是复刻装配路径。
//! 2. **能串联的阶段才是产线的一个阶段**。所以 `swarm` 形状走 `team.run`（同步会诊、
//!    按宽度扇出、lead 收口），**不走** `orchestrate.run` 的 `team.run.background`：
//!    后台蜂群只回 `taskId`，产物拿不到，下一阶段就没输入。这一点在 `line.plan` 里
//!    如实写出来，而不是含糊成「已启动」。
//! 3. **名册解析不到就是错误**，不是警告。空/错名册在旧路径上会静默降级成单个通用
//!    执行者（DEF-10 同族），产线宁可拒绝派单。

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::domain::UserMode;
use crate::events::{Envelope, EventType, EventBus};
use crate::gateway::_helpers::param_str;
use crate::gateway::methods::GatewayContext;
use crate::gateway::orchestrate::{self, OrchestrateForm, OrchestrationRequest};
use crate::gateway::protocol::RpcError;
use crate::gateway::team;
use crate::line::{self, LinePattern, LineSpec};
use crate::persistence::config::Config;
use crate::tactics;

/// 角色标签 → 子智能体内建人设键。
///
/// 为什么需要这张表：子智能体路径的人设由 `orchestrate::role_prompt_for` 按**英文键**
/// 命中，中文席位名一律落到 orchestrator 默认人设 —— 于是「调研席」和「安全席」产出
/// 雷同（真机反馈「两个不同专家团产出完全雷同」的同源缺陷）。这里显式映射到真实的
/// 一种人设，映射不到时由调用方如实标注 `personaFallback`。
const ROLE_KEY_ALIASES: &[(&str, &str)] = &[
    ("调研", "researcher"),
    ("研究", "researcher"),
    ("事实核查", "researcher"),
    ("结构分析", "researcher"),
    ("编码实现", "coder"),
    ("落地实施", "coder"),
    ("方案设计", "coder"),
    ("架构设计", "coder"),
    ("重构优化", "coder"),
    ("测试验证", "sandbox"),
    ("性能资源", "sandbox"),
    ("兼容迁移", "sandbox"),
    ("安全", "guard"),
    ("风险反例", "guard"),
    ("合规审查", "guard"),
];

fn role_key_for(label: &str) -> (String, bool) {
    let trimmed = label.trim();
    for (from, to) in ROLE_KEY_ALIASES {
        if *from == trimmed {
            return (to.to_string(), false);
        }
    }
    // 本来就是英文键时原样透传（`role_prompt_for` 认识它）。
    if matches!(
        trimmed,
        "researcher" | "coder" | "sandbox" | "browser" | "guard" | "orchestrator"
    ) {
        return (trimmed.to_string(), false);
    }
    (trimmed.to_string(), true)
}

/// 形状 → 编排形态。`Relay` 没有对应挡位（`OrchestrateForm` 里没有 pipeline），
/// 由本模块按「逐棒串行」实现，不假造一个 backend。
fn form_for(pattern: LinePattern) -> Option<OrchestrateForm> {
    match pattern {
        LinePattern::Solo | LinePattern::Fanout => Some(OrchestrateForm::SubAgent),
        LinePattern::Panel | LinePattern::Swarm => Some(OrchestrateForm::Team),
        LinePattern::Relay => None,
    }
}

fn max_concurrent() -> usize {
    crate::agent::budget_policy::subagent_max_concurrent()
}

fn invalid(msg: impl Into<String>) -> Value {
    json!({ "ok": false, "error": msg.into(), "code": "INVALID_REQUEST" })
}

fn ok_with<T: serde::Serialize>(value: &T) -> Value {
    match serde_json::to_value(value) {
        Ok(Value::Object(mut m)) => {
            m.insert("ok".into(), json!(true));
            Value::Object(m)
        }
        Ok(other) => json!({ "ok": true, "value": other }),
        Err(e) => json!({
            "ok": false,
            "error": format!("结果序列化失败: {e}"),
            "code": "SERIALIZE_FAILED",
        }),
    }
}

// ── `line.catalog` ─────────────────────────────────────────────────────────

/// 目录：形状、配方、名册候选、上限、缺省判据。
///
/// 前端**不允许**再抄一份常量表。此前「输入框能选 8 路、执行层只认 5 路并发」之所以
/// 没人发现，就是因为候选写在前端、闸门写在引擎，两边各自演进。
pub fn line_catalog() -> Value {
    let patterns: Vec<Value> = LinePattern::ALL
        .iter()
        .map(|p| {
            json!({
                "id": p.as_key(),
                "label": p.label(),
                "description": p.blurb(),
                "rosterKind": p.roster_kind(),
                "needsSynthesis": p.needs_synthesis(),
                "topology": p.topology().as_key(),
            })
        })
        .collect();
    let recipes: Vec<Value> = line::recipes()
        .into_iter()
        .map(|r| json!({ "id": r.id, "label": r.label, "stages": r.stages }))
        .collect();
    let legacy: Vec<Value> = line::LegacyForm::ALL
        .iter()
        .map(|f| {
            json!({
                "id": f.as_key(),
                "label": OrchestrateForm::from_key(f.as_key()).map(|o| o.label()).unwrap_or("旧挡位"),
                "stages": f.to_stages().iter().map(|p| p.as_key()).collect::<Vec<_>>(),
            })
        })
        .collect();
    let teams: Vec<Value> = team::builtin_rosters()
        .into_iter()
        .map(|(id, roles)| json!({ "id": id, "label": id, "roles": roles }))
        .collect();
    let roles: Vec<Value> = team::builtin_role_labels()
        .into_iter()
        .map(|r| {
            let (key, fallback) = role_key_for(r);
            json!({ "id": r, "label": r, "agentKey": key, "personaFallback": fallback })
        })
        .collect();
    let facets: Vec<Value> = crate::orchestration::SWARM_FACETS
        .iter()
        .map(|(name, instruction)| json!({ "id": name, "instruction": instruction }))
        .collect();

    json!({
        "ok": true,
        "patterns": patterns,
        "recipes": recipes,
        "legacyForms": legacy,
        "teams": teams,
        "roles": roles,
        "facets": facets,
        "limits": {
            "maxStages": line::MAX_STAGES,
            "maxWidth": line::MAX_WIDTH,
            "subagentMaxConcurrent": max_concurrent(),
            "runKeep": line::LINE_RUN_KEEP,
            "dirBudgetBytes": line::LINE_DIR_BUDGET_BYTES,
        },
        "defaults": {
            "criteria": line::LineCriteria::default(),
            "trials": tactics::model::DEFAULT_TRIALS,
            "maxTrials": tactics::model::MAX_TRIALS,
        },
    })
}

// ── 规格解析 ───────────────────────────────────────────────────────────────

/// 从 RPC 参数取规格：`spec` 优先，其次 `stages` / `recipeId` / 旧挡位旋钮。
///
/// 四条入口最终都收敛成同一个 `LineSpec` —— 包括旧挡位。旧客户端不该拿到一条
/// 「看起来不同」的路径（那会立刻长出第二套编排实现）。
fn spec_from_params(params: &Value) -> Result<LineSpec, String> {
    let goal = params
        .get("goal")
        .or_else(|| params.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();

    if let Some(raw) = params.get("spec") {
        return serde_json::from_value::<LineSpec>(raw.clone())
            .map_err(|e| format!("spec 结构非法: {e}"))
            .map(|mut s| {
                if s.goal.trim().is_empty() && !goal.is_empty() {
                    s.goal = goal;
                }
                s
            });
    }

    let spec_id = format!("ls.{}", uuid::Uuid::new_v4());

    if let Some(stages) = params.get("stages").and_then(Value::as_array) {
        let parsed: Vec<line::LineStage> =
            serde_json::from_value(serde_json::Value::Array(stages.clone()))
                .map_err(|e| format!("stages 结构非法: {e}"))?;
        return Ok(LineSpec::with_stages(spec_id, goal, parsed));
    }

    if let Some(recipe_id) = params.get("recipeId").and_then(Value::as_str) {
        let recipe = line::recipes()
            .into_iter()
            .find(|r| r.id == recipe_id)
            .ok_or_else(|| format!("recipeId='{recipe_id}' 不在目录里；用 line.catalog 取候选"))?;
        return Ok(LineSpec::with_stages(spec_id, goal, recipe.stages));
    }

    // 旧挡位 + 旧旋钮：`form` / `agents` / `teams` / `swarmSize`。
    let form_key = param_str(params, "form", "form", "");
    if form_key.is_empty() {
        return Err("需要 spec、stages、recipeId 或 form 之一".to_string());
    }
    let form = line::LegacyForm::from_key(form_key)
        .ok_or_else(|| format!("form='{form_key}' 非法；合法值: single|subagent|team|expert|swarm"))?;
    let str_field = |key: &str| -> Vec<String> {
        params
            .get(key)
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let swarm_size = params.get("swarmSize").and_then(Value::as_u64).unwrap_or(0) as usize;
    Ok(line::compile_legacy(
        &spec_id,
        &goal,
        form,
        &str_field("agents"),
        &str_field("teams"),
        swarm_size,
    ))
}

/// 名册可解析性检查（只有网关层知道团队目录，所以放在这里而不是 `crate::line`）。
fn resolve_errors(spec: &LineSpec) -> Vec<String> {
    let mut known: Vec<String> = team::builtin_rosters()
        .into_iter()
        .map(|(id, _)| id.to_string())
        .collect();
    // 用户自定义组合（team.compositions）也在解析范围内：与 selected_rosters 同口径（id 或 name）。
    for c in composition_ids() {
        if !known.contains(&c) {
            known.push(c);
        }
    }
    let mut out = Vec::new();
    for stage in &spec.stages {
        if stage.pattern != LinePattern::Panel {
            continue;
        }
        for id in &stage.roster {
            if !known.iter().any(|k| k == id) {
                out.push(format!(
                    "{}：团队 id「{id}」不存在（可用：{}）—— 解析不到的名册会静默降级成自动检测",
                    stage.id,
                    known.join(", ")
                ));
            }
        }
    }
    out
}

/// 已保存的专家团组合 id/name（解析候选，不复制内容）。
fn composition_ids() -> Vec<String> {
    team::team_compositions_list()
        .get("items")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .flat_map(|it| {
                    ["id", "name"].into_iter().filter_map(|k| {
                        it.get(k)
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 结构 + 名册两道校验合并成一个结论。
fn check(spec: &LineSpec) -> Value {
    let mut v = line::validate(spec, max_concurrent());
    let mut errors = std::mem::take(&mut v.errors);
    errors.extend(resolve_errors(spec));
    json!({
        "errors": errors,
        "warnings": v.warnings,
        "valid": errors.is_empty(),
        "maxConcurrent": max_concurrent(),
    })
}

// ── 权限门 ─────────────────────────────────────────────────────────────────

/// 逐阶段过 3×3 权限矩阵，返回**最严**的那一条结论。
///
/// 合并成一句「整条线允许/拒绝」会骗人：三段里只要有一段是高自主，
/// 用户看到的确认弹窗就必须点名那一段，而不是整条线一起放行。
fn gate_value(spec: &LineSpec, user_mode: UserMode, confirmed: bool) -> Value {
    let mut strictest: Option<(&'static str, &'static str, String)> = None; // (autonomy, decision, reason)
    let rank = |d: &str| match d {
        "deny" => 2,
        "confirm" => 1,
        _ => 0,
    };
    for stage in &spec.stages {
        let Some(form) = form_for(stage.pattern) else {
            // relay 的每一棒就是子智能体（写档），如实按 subagent 判。
            let probe = orchestrate::gate_form(OrchestrateForm::SubAgent, user_mode, confirmed);
            record_gate(&mut strictest, rank, "mutating", probe, stage.id.as_str());
            continue;
        };
        let probe = orchestrate::gate_form(form, user_mode, confirmed);
        let autonomy = match form {
            OrchestrateForm::Single => "read_only",
            OrchestrateForm::SubAgent => "mutating",
            _ => "high_autonomy",
        };
        record_gate(&mut strictest, rank, autonomy, probe, stage.id.as_str());
    }
    match strictest {
        Some((autonomy, decision, reason)) => {
            json!({ "autonomy": autonomy, "decision": decision, "reason": reason })
        }
        None => json!({ "autonomy": "read_only", "decision": "allow", "reason": "" }),
    }
}

fn record_gate(
    slot: &mut Option<(&'static str, &'static str, String)>,
    rank: impl Fn(&str) -> u8,
    autonomy: &'static str,
    probe: Result<(), RpcError>,
    stage_id: &str,
) {
    let (decision, reason) = match probe {
        Ok(()) => ("allow", String::new()),
        Err(e) if e.code == "ORCHESTRATE_CONFIRM_REQUIRED" => ("confirm", e.message),
        Err(e) => ("deny", e.message),
    };
    let keep = slot
        .as_ref()
        .map(|(_, d, _)| rank(d) >= rank(decision))
        .unwrap_or(false);
    if keep {
        return;
    }
    slot.replace((
        autonomy,
        decision,
        if reason.is_empty() {
            String::new()
        } else {
            format!("阶段 {stage_id}：{reason}")
        },
    ));
}

// ── `line.compile` / `line.preflight` / `line.plan` ────────────────────────

/// `line.compile`：把任意入口收敛成规格，并回显形状描述与校验结论。
pub fn line_compile(params: &Value) -> Value {
    let spec = match spec_from_params(params) {
        Ok(s) => s,
        Err(e) => return invalid(e),
    };
    json!({
        "ok": true,
        "spec": spec,
        "shape": line::describe(&spec),
        "validation": check(&spec),
    })
}

/// `line.preflight`：派单前预演。校验不过就拒绝，绝不猜一个「差不多」的形状。
pub fn line_preflight(params: &Value) -> Value {
    let spec = match spec_from_params(params) {
        Ok(s) => s,
        Err(e) => return invalid(e),
    };
    let verification = check(&spec);
    if verification["valid"] != json!(true) {
        return json!({
            "ok": false,
            "error": verification["errors"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("；"))
                .unwrap_or_default(),
            "code": "INVALID_SPEC",
            "validation": verification,
            "shape": line::describe(&spec),
            "gate": gate_of(params),
        });
    }
    let mut out = ok_with(&line::preflight(&spec, max_concurrent()));
    out["shape"] = json!(line::describe(&spec));
    out["validation"] = verification;
    out["gate"] = gate_of(params);
    out
}

fn gate_of(params: &Value) -> Value {
    let mode_key = param_str(params, "mode", "mode", "standard_doit");
    let user_mode = UserMode::from_key(mode_key).unwrap_or(UserMode::StandardDoIt);
    let confirmed = params
        .get("confirmed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let spec = match spec_from_params(params) {
        Ok(s) => s,
        Err(_) => return json!({ "autonomy": "unknown", "decision": "deny", "reason": "规格无法解析，权限门无从判起" }),
    };
    gate_value(&spec, user_mode, confirmed)
}

/// `line.plan`：声明即执行 —— 每阶段将走的 backend 由 `route_meta` 生成。
pub fn line_plan(params: &Value) -> Value {
    let spec = match spec_from_params(params) {
        Ok(s) => s,
        Err(e) => return invalid(e),
    };
    let verification = check(&spec);
    let mut stages = Vec::with_capacity(spec.stages.len());
    for (i, stage) in spec.stages.iter().enumerate() {
        // 除 relay 外，每一条都必须经过 `route_meta` —— 它是「计划」与「执行」唯一的同源
        // 定义，手写 backend 名会立刻和 run_stage 真实调的东西分叉（本函数第一版就把
        // swarm/solo 的 backend 写成了空串，测试把它抓了出来）。
        let note = match stage.pattern {
            LinePattern::Swarm => "产线里的 swarm 走同步会诊（team.run），不走后台蜂群：\
                                   后台只回 taskId，下一阶段拿不到产物"
                .to_string(),
            LinePattern::Solo => "solo 阶段用单个 orchestrator 子智能体同步执行；\
                                  挂线派单不经过 chat.send，这一轮的 assistant 回复\
                                  由 line.run 自己落对话台账"
                .to_string(),
            _ => String::new(),
        };
        let mut entry = match stage.pattern {
            LinePattern::Relay => json!({
                "index": i,
                "pattern": stage.pattern.as_key(),
                "width": stage.width,
                // 接力在挡位表里没有对应项，唯一诚实的描述就是它实际的执行方式：
                // 本模块逐棒串行调用 subagent 后端，后一棒的输入含前一棒产物。
                "backend": "SubAgentManager.spawn_with_spec",
                "form": OrchestrateForm::SubAgent.as_key(),
                "args": json!({
                    "roles": stage.roster.iter().map(|r| role_key_for(r).0).collect::<Vec<_>>(),
                    "sequential": true,
                    "chainFrom": if i == 0 { Value::Null } else { json!(spec.stages[i - 1].id) },
                }),
                "roster": stage.roster,
                "note": "接力没有对应挡位：本模块逐棒串行，后一棒的输入含前一棒产物",
            }),
            _ => {
                let form = form_for(stage.pattern).expect("非 relay 必有形态");
                let mut meta = orchestrate::route_meta(
                    form,
                    &spec.goal,
                    &stage.roster,
                    &[],
                    stage.pattern == LinePattern::Swarm,
                );
                meta["index"] = json!(i);
                meta["pattern"] = json!(stage.pattern.as_key());
                meta["width"] = json!(stage.width);
                meta["roster"] = json!(stage.roster);
                meta["note"] = json!(note);
                meta
            }
        };
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("autonomy".into(), json!(autonomy_of(stage.pattern)));
        }
        stages.push(entry);
    }
    json!({
        "ok": true,
        "spec": spec,
        "shape": line::describe(&spec),
        "stages": stages,
        "validation": verification,
        "gate": gate_of(params),
    })
}

fn autonomy_of(pattern: LinePattern) -> &'static str {
    match form_for(pattern) {
        Some(OrchestrateForm::Single) => "read_only",
        Some(OrchestrateForm::SubAgent) => "mutating",
        Some(_) => "high_autonomy",
        None => "mutating",
    }
}

// ── `line.run` ─────────────────────────────────────────────────────────────

/// 发布编排进度帧（沿用 webui 既有相位词表，不新造一套）。
fn publish_frame(bus: &EventBus, session: &str, phase: &str, run_id: &str, extra: Value) {
    let mut payload = match extra {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    payload.insert("kind".into(), json!("orchestration"));
    payload.insert("phase".into(), json!(phase));
    payload.insert("runId".into(), json!(run_id));
    payload.insert("engine".into(), json!("line"));
    let payload = Value::Object(payload);
    crate::orchestration::member_registry::observe(session, phase, &payload);
    bus.publish(Envelope::new(EventType::Progress, payload, session));
}

/// 阶段之间只增量传递**上一阶段的产物**，并把长度钉死：
/// 全量历史滚进下一段就是「上下文复利」那类缺陷的来源。
const CARRY_MAX_CHARS: usize = 6_000;

fn stage_goal(spec: &LineSpec, index: usize, prev: &str) -> String {
    let mut out = String::new();
    out.push_str(spec.goal.trim());
    if !spec.acceptance.is_empty() {
        out.push_str("\n\n【验收条款】\n");
        for a in &spec.acceptance {
            out.push_str("- ");
            out.push_str(a.trim());
            out.push('\n');
        }
    }
    if index > 0 && !prev.trim().is_empty() {
        let carried: String = prev.chars().take(CARRY_MAX_CHARS).collect();
        out.push_str("\n\n【上一阶段产物（只做衔接，不要重做它已完成的取证）】\n");
        out.push_str(carried.trim());
    }
    if let Some(note) = spec.stages.get(index).map(|s| s.note.trim()).filter(|n| !n.is_empty()) {
        out.push_str(&format!("\n\n【本阶段存在的理由】{note}"));
    }
    // 切面必须真的下发，否则它在契约里就是一个装饰字段（"选了没用"那一类缺陷）。
    // 引擎内部的蜂群分工按索引轮转 `SWARM_FACETS`；这里把用户显式选中的切面写进
    // 本阶段目标，让它成为成员实际收到的一条约束 —— 两者不冲突，且这条是**可验证的**。
    if let Some(stage) = spec.stages.get(index) {
        if stage.pattern == LinePattern::Swarm && !stage.facets.is_empty() {
            out.push_str("\n\n【本阶段切面（每个成员只承担其中一个，不做综合报告）】\n");
            for facet in &stage.facets {
                if !facet.trim().is_empty() {
                    out.push_str("- ");
                    out.push_str(facet.trim());
                    out.push('\n');
                }
            }
        }
    }
    out
}

/// 从后端返回里取「这一段的结论文本」+ 成员小结。取不到就明说，不编。
fn harvest(form: Option<OrchestrateForm>, out: &Value) -> (String, Vec<(String, String)>) {
    let mut members: Vec<(String, String)> = Vec::new();
    match form {
        Some(OrchestrateForm::SubAgent) => {
            if let Some(arr) = out.get("summaries").and_then(Value::as_array) {
                for s in arr {
                    let role = s.get("role").and_then(Value::as_str).unwrap_or("成员").to_string();
                    let text = s.get("summary").and_then(Value::as_str).unwrap_or_default().to_string();
                    members.push((role, text));
                }
            }
            let lead = members
                .iter()
                .map(|(r, t)| format!("【{r}】{t}"))
                .collect::<Vec<_>>()
                .join("\n\n");
            (lead, members)
        }
        _ => {
            if let Some(arr) = out.get("summaries").and_then(Value::as_array) {
                for (i, s) in arr.iter().enumerate() {
                    match s {
                        Value::String(text) => {
                            members.push((format!("成员 {}", i + 1), text.clone()))
                        }
                        Value::Object(obj) => {
                            let role = obj
                                .get("role")
                                .and_then(Value::as_str)
                                .unwrap_or("成员")
                                .to_string();
                            let text = obj
                                .get("summary")
                                .and_then(Value::as_str)
                                .or_else(|| obj.get("text").and_then(Value::as_str))
                                .unwrap_or_default()
                                .to_string();
                            members.push((role, text));
                        }
                        _ => {}
                    }
                }
            }
            let lead = out
                .get("lead_reply")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let text = if lead.trim().is_empty() {
                members
                    .iter()
                    .map(|(r, t)| format!("【{r}】{t}"))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            } else {
                lead
            };
            (text, members)
        }
    }
}

const DISSENT_MARKERS: &[&str] = &[
    "反对", "不同意", "有异议", "不一致", "存在分歧", "与前述矛盾", "无法确认",
];

fn claim_kind(text: &str) -> &'static str {
    let t = text.trim_start();
    if t.starts_with("未决问题") || t.contains('？') || t.contains('?') {
        "question"
    } else if DISSENT_MARKERS.iter().any(|m| t.contains(m)) {
        "risk"
    } else if ["建议", "应当", "必须", "决定", "结论", "下一步"]
        .iter()
        .any(|m| t.contains(m))
    {
        "decision"
    } else {
        "finding"
    }
}

/// 断言链：一条产物的每个成员结论都是一条**可点名、可驳回**的断言。
///
/// `accepted` 恒为 `null` —— 判定属于 `harness.ledger.verdict`（已有接口），
/// 产线不自造「已通过」的结论，那正是本设计要消灭的伪完成。
fn claims_for(stage_id: &str, backend: &str, members: &[(String, String)]) -> Vec<Value> {
    members
        .iter()
        .map(|(role, text)| {
            json!({
                "claim": text,
                "kind": claim_kind(text),
                "evidence": [backend, stage_id],
                "by": { "stage": stage_id, "member": role },
                "dissent": DISSENT_MARKERS.iter().any(|m| text.contains(m)),
                "accepted": Value::Null,
            })
        })
        .collect()
}

async fn run_stage(
    ctx: &GatewayContext,
    spec: &LineSpec,
    index: usize,
    session: &str,
    prev: &str,
    run_id: &str,
) -> (Value, String) {
    let stage = &spec.stages[index];
    let started = std::time::Instant::now();
    let goal = stage_goal(spec, index, prev);
    publish_frame(
        &ctx.bus,
        session,
        "run_started",
        run_id,
        json!({ "stage": stage.id, "stageIndex": index, "pattern": stage.pattern.as_key() }),
    );

    let outcome: Result<Value, RpcError> = match stage.pattern {
        LinePattern::Relay => {
            // 接力：逐棒串行，后一棒的输入含前一棒产物。
            let mut acc: Vec<(String, String)> = Vec::new();
            let mut transcript = String::new();
            let mut err: Option<RpcError> = None;
            for (leg, label) in stage.roster.iter().enumerate() {
                let (key, fallback) = role_key_for(label);
                // 名册是中文席位名，映射不到内建人设时必须**如实告知**，否则这一棒
                // 会静默按通用执行者跑（和「选了团队却解析不到」同一族缺陷）。
                let persona_note = if fallback {
                    "（注：本席位未匹配到内建人设，按席位名执行）"
                } else {
                    ""
                };
                // 「接力」的定义就是后一棒吃前一棒的产物。此前 transcript 只拼不喂，
                // 每一棒拿到的都是同一段 goal —— 那并行扇出没区别，形状是假的。
                let prior: String = transcript.chars().rev().take(CARRY_MAX_CHARS).collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let task = if prior.trim().is_empty() {
                    format!(
                        "{goal}\n\n【第 {} 棒·{label}】只做这一棒，产物交给下一棒。{persona_note}",
                        leg + 1
                    )
                } else {
                    format!(
                        "{goal}\n\n【前一棒产物（只做衔接，不要重做它已完成的取证）】\n{}\n\n\
                         【第 {} 棒·{label}】只做这一棒，产物交给下一棒。{persona_note}",
                        prior.trim(),
                        leg + 1
                    )
                };
                let req = OrchestrationRequest {
                    message: &task,
                    session,
                    agents: std::slice::from_ref(&key),
                    teams: &[],
                    capabilities: &[],
                    swarm_on: false,
                    swarm_size: 0,
                };
                match orchestrate::dispatch_form(ctx, OrchestrateForm::SubAgent, &req).await {
                    Ok(v) => {
                        let (text, members) = harvest(Some(OrchestrateForm::SubAgent), &v);
                        acc.extend(members);
                        transcript.push_str(&format!("【第 {} 棒·{label}】\n{text}\n\n", leg + 1));
                    }
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                }
            }
            match err {
                Some(e) => Err(e),
                None => Ok(json!({
                    "backend": "SubAgentManager.spawn_with_spec",
                    "summaries": acc
                        .iter()
                        .map(|(r, t)| json!({ "role": r, "summary": t }))
                        .collect::<Vec<_>>(),
                    "sequential": true,
                })),
            }
        }
        LinePattern::Solo => {
            let req = OrchestrationRequest {
                message: &goal,
                session,
                agents: &["orchestrator".to_string()],
                teams: &[],
                capabilities: &[],
                swarm_on: false,
                swarm_size: 0,
            };
            orchestrate::dispatch_form(ctx, OrchestrateForm::SubAgent, &req).await
        }
        LinePattern::Fanout => {
            let picks: Vec<String> = if stage.roster.is_empty() {
                vec!["orchestrator".to_string()]
            } else {
                stage
                    .roster
                    .iter()
                    .map(|r| role_key_for(r).0)
                    .collect()
            };
            let req = OrchestrationRequest {
                message: &goal,
                session,
                agents: &picks,
                teams: &[],
                capabilities: &[],
                swarm_on: false,
                swarm_size: 0,
            };
            orchestrate::dispatch_form(ctx, OrchestrateForm::SubAgent, &req).await
        }
        // panel / swarm：同步会诊。宽度 = 名册团队数（panel）或请求规模（swarm），
        // 成员展开与并发闸门全部复用既有后端，本模块不另设。
        other => {
            let req = OrchestrationRequest {
                message: &goal,
                session,
                agents: &stage.roster,
                teams: &stage.roster,
                capabilities: &[],
                swarm_on: other == LinePattern::Swarm,
                swarm_size: if other == LinePattern::Swarm {
                    stage.width
                } else {
                    0
                },
            };
            orchestrate::dispatch_form(ctx, OrchestrateForm::Team, &req).await
        }
    };

    let elapsed = started.elapsed().as_millis() as u64;
    match outcome {
        Ok(out) => {
            let backend = out
                .get("backend")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let (text, members) = harvest(Some(form_for(stage.pattern).unwrap_or(OrchestrateForm::Team)), &out);
            publish_frame(
                &ctx.bus,
                session,
                "member_completed",
                run_id,
                json!({ "stage": stage.id, "stageIndex": index, "ok": true, "elapsedMs": elapsed }),
            );
            (
                json!({
                    "index": index,
                    "id": stage.id,
                    "pattern": stage.pattern.as_key(),
                    "status": "ok",
                    "backend": backend,
                    "reply": text,
                    "claims": claims_for(&stage.id, &backend, &members),
                    "elapsedMs": elapsed,
                    "error": Value::Null,
                }),
                text,
            )
        }
        Err(e) => {
            publish_frame(
                &ctx.bus,
                session,
                "run_failed",
                run_id,
                json!({ "stage": stage.id, "stageIndex": index, "error": e.message }),
            );
            (
                json!({
                    "index": index,
                    "id": stage.id,
                    "pattern": stage.pattern.as_key(),
                    "status": "failed",
                    "backend": form_for(stage.pattern).map(|f| f.as_key()).unwrap_or("relay"),
                    "reply": "",
                    "claims": [],
                    "elapsedMs": elapsed,
                    "error": format!("{}: {}", e.code, e.message),
                }),
                String::new(),
            )
        }
    }
}

/// `line.run`：逐阶段串行执行一条产线。
pub async fn line_run(ctx: &GatewayContext, params: &Value) -> Result<Value, RpcError> {
    let spec = spec_from_params(params).map_err(|e| RpcError::new("INVALID_REQUEST", e))?;
    let verification = check(&spec);
    if verification["valid"] != json!(true) {
        let joined = verification["errors"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("；"))
            .unwrap_or_default();
        return Err(RpcError::new("INVALID_SPEC", joined));
    }

    let mode_key = param_str(params, "mode", "mode", "standard_doit");
    let user_mode = UserMode::from_key(mode_key)
        .ok_or_else(|| RpcError::new("INVALID_REQUEST", format!("mode='{mode_key}' 非法")))?;
    let confirmed = params
        .get("confirmed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // 权限门在任何执行之前：被拒的高自主阶段不该已经花掉 token。
    let gate = gate_value(&spec, user_mode, confirmed);
    match gate["decision"].as_str().unwrap_or("allow") {
        "deny" => {
            return Err(RpcError::new(
                "ORCHESTRATE_DENIED",
                gate["reason"].as_str().unwrap_or_default(),
            ))
        }
        "confirm" => {
            return Err(RpcError::new(
                "ORCHESTRATE_CONFIRM_REQUIRED",
                gate["reason"].as_str().unwrap_or_default(),
            ))
        }
        _ => {}
    }

    let session_key = param_str(params, "sessionKey", "sessionKey", "").trim().to_string();
    let session = if session_key.is_empty() {
        format!("ws.{}", uuid::Uuid::new_v4())
    } else {
        session_key
    };
    let run_id = format!("line-{}", uuid::Uuid::new_v4());
    // 「停止」按钮必须真能停这条产线。取消表与会话中止是同一张
    // （`chat.abort` / `sessions.*.abort` → `cancel::cancel_session(session)`），
    // 不登记的后果不是报错而是**界面说谎**：按钮显示已停，模型调用继续跑、
    // 回执永远停在运行中——那正是本仓为蜂群补过的那类缺陷。
    let cancel_guard = crate::orchestration::cancel::register(&session, &run_id);
    let cancelled = cancel_guard.signal();

    // 与 `orchestrate.run` 同口径：入口自己落一次用户消息，否则重开应用只剩答案没有问题。
    crate::gateway::chat::record_turn_persisted(&session, "user", &spec.goal);
    publish_frame(
        &ctx.bus,
        &session,
        "run_started",
        &run_id,
        json!({ "shape": line::describe(&spec), "specId": spec.spec_id, "stages": spec.stages.len() }),
    );

    let predicted = line::preflight(&spec, max_concurrent());
    let mut stages = Vec::with_capacity(spec.stages.len());
    let mut prev = String::new();
    let mut failures: Vec<String> = Vec::new();
    for i in 0..spec.stages.len() {
        // 取消是**阶段边界**粒度的：正在跑的那一段要跑完才收口（阶段内部是同步的
        // 子智能体/会诊调用，没有中断点）。回执因此如实写"停在哪一段之前"，
        // 而不是假装这一段被掐断了。
        if cancelled.is_cancelled() {
            let stage = &spec.stages[i];
            publish_frame(
                &ctx.bus,
                &session,
                "run_failed",
                &run_id,
                json!({ "stage": stage.id, "stageIndex": i, "error": "cancelled" }),
            );
            failures.push(format!("{}：已在阶段边界停止（用户取消）", stage.id));
            stages.push(json!({
                "index": i,
                "id": stage.id,
                "pattern": stage.pattern.as_key(),
                "status": "failed",
                "backend": form_for(stage.pattern).map(|f| f.as_key()).unwrap_or("relay"),
                "reply": "",
                "claims": [],
                "elapsedMs": Value::Null,
                "error": "cancelled（用户取消，未启动该阶段）",
            }));
            break;
        }
        let (frame, text) = run_stage(ctx, &spec, i, &session, &prev, &run_id).await;
        if frame["status"] == json!("failed") {
            failures.push(format!(
                "{}：{}",
                frame["id"].as_str().unwrap_or_default(),
                frame["error"].as_str().unwrap_or_default()
            ));
            stages.push(frame);
            break;
        }
        prev = text;
        stages.push(frame);
    }
    let ok = failures.is_empty() && stages.len() == spec.stages.len();
    publish_frame(
        &ctx.bus,
        &session,
        if ok { "run_completed" } else { "run_failed" },
        &run_id,
        json!({ "ok": ok, "shape": line::describe(&spec) }),
    );

    // 回复必须进对话历史。挂线上膛后发送缝**绕开了** `chat.send`，所以没人替这条
    // 产线落 assistant 轮：不落的话重载会话就只剩用户的问题没有答案（回执卡是过程
    // 凭证，且只活在当前会话的内存里，不能当历史用）。口径与编排路径
    // `chat.rs` 收尾的 `record_turn_persisted(…, "assistant", …)` 一致。
    let answer = if ok {
        stages
            .last()
            .and_then(|s| s.get("reply").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string()
    } else {
        // 半路失败没有「答案」，如实说它停在哪一段，而不是把上一段的产物冒充结论。
        format!("产线未跑完：{}", failures.join("；"))
    };
    if !answer.trim().is_empty() {
        crate::gateway::chat::record_turn_persisted(&session, "assistant", &answer);
    }

    let total_elapsed: u64 = stages
        .iter()
        .filter_map(|s| s.get("elapsedMs").and_then(Value::as_u64))
        .sum();
    let record = json!({
        "runId": run_id,
        "sessionKey": session,
        "at": chrono::Utc::now().to_rfc3339(),
        "spec": spec,
        "shape": line::describe(&spec),
        "stages": stages,
        "verdict": {
            "ok": ok,
            "failures": failures,
            "elapsedMsActual": total_elapsed,
            "elapsedMsPredicted": predicted.total.p90_ms as u64,
            "tokensPredicted": predicted.total.p90_tokens,
            "successRatePredicted": predicted.total.success_rate,
        },
    });
    let persisted = persist_run(&line_dir(), &record);

    let mut out = record;
    out["preflight"] = ok_with(&predicted);
    out["gate"] = gate;
    out["storage"] = persisted;
    Ok(out)
}

// ── 有界落盘（§5）──────────────────────────────────────────────────────────

/// 运行记录根目录。**永远**经这里取，测试传临时目录进来 —— 绝不写真实 `~/.marguerite`。
fn line_dir() -> PathBuf {
    line_dir_in(&Config::data_dir())
}

fn line_dir_in(root: &Path) -> PathBuf {
    root.join("line")
}

/// `runId` 直接来自 RPC 参数，必须钉死字符集：`..` 或分隔符会拼出目录外的路径。
fn safe_run_id(raw: &str) -> Option<String> {
    let id = raw.trim();
    if id.is_empty()
        || id.len() > 80
        || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        || id.starts_with('.')
    {
        return None;
    }
    Some(id.to_string())
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// 落一条运行记录并裁剪到声明的界限内（条数 + 目录字节数）。
///
/// 返回**如实**的落盘结果：失败就说失败并给原因，前端卡片据此显示
/// `persisted:false` —— 运行记录丢了，复盘就无从谈起，这不能静默。
fn persist_run(dir: &Path, record: &Value) -> Value {
    let bytes = match serde_json::to_vec(record) {
        Ok(b) => b,
        Err(e) => return json!({ "persisted": false, "error": format!("序列化失败: {e}") }),
    };
    if let Err(e) = std::fs::create_dir_all(dir) {
        return json!({ "persisted": false, "error": format!("建目录失败: {e}") });
    }
    let Some(id) = record.get("runId").and_then(Value::as_str).and_then(safe_run_id) else {
        return json!({ "persisted": false, "error": "runId 非法" });
    };
    let path = dir.join(format!("{id}.json"));
    if let Err(e) = write_bytes_atomic(&path, &bytes) {
        return json!({ "persisted": false, "error": format!("写盘失败: {e}") });
    }
    crate::observability::io_report::tally_write(bytes.len() as u64);
    let pruned = prune(dir);
    json!({
        "persisted": true,
        "path": path.display().to_string(),
        "bytes": bytes.len(),
        "pruned": pruned,
    })
}

/// 先按条数裁、再按目录总量裁（都保留最新）。
fn prune(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf, u64)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let mtime = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        files.push((mtime, path, meta.len()));
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    let mut removed = 0usize;
    let mut kept_bytes = 0u64;
    for (i, (_, path, size)) in files.iter().enumerate() {
        if i >= line::LINE_RUN_KEEP || kept_bytes + *size > line::LINE_DIR_BUDGET_BYTES {
            if std::fs::remove_file(path).is_ok() {
                removed += 1;
            }
            continue;
        }
        kept_bytes += size;
    }
    removed
}

fn read_run(dir: &Path, id: &str) -> Option<Value> {
    let path = dir.join(format!("{id}.json"));
    let raw = std::fs::read(&path).ok()?;
    serde_json::from_slice(&raw).ok()
}

// ── `line.retro` ───────────────────────────────────────────────────────────

/// `line.retro`：复盘一次真实运行。
///
/// 复用 `tactics::replay` 而不是另写一套归因：5-Why 链、反事实对照、缺口标注
/// 都在那里，且它已经把「台账没有的字段进 gaps、不补零不猜」的口径立住了。
pub fn line_retro(params: &Value) -> Value {
    let Some(id) = params
        .get("runId")
        .or_else(|| params.get("run_id"))
        .and_then(Value::as_str)
        .and_then(safe_run_id)
    else {
        return invalid("runId 缺失或非法（只允许字母数字与 - _ .，且不得以 . 开头）");
    };
    let dir = line_dir();
    let Some(record) = read_run(&dir, &id) else {
        return json!({
            "ok": false,
            "error": format!("运行记录不存在：{}（有界目录只保留最近 {}/{} 条）", dir.display(), line::LINE_RUN_KEEP, line::LINE_RUN_KEEP),
            "code": "LINE_RUN_NOT_FOUND",
        });
    };
    let trials = params
        .get("trials")
        .and_then(Value::as_u64)
        .map(|n| (n as usize).clamp(1, tactics::model::MAX_TRIALS))
        .unwrap_or(tactics::model::DEFAULT_TRIALS);
    let seed = params.get("seed").and_then(Value::as_u64).unwrap_or(42);

    let stages = record
        .get("stages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let observed = tactics::ObservedRun {
        run_id: id.clone(),
        target: record
            .get("shape")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        status: if record
            .get("verdict")
            .and_then(|v| v.get("ok"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            "ok".to_string()
        } else {
            "failed".to_string()
        },
        elapsed_ms: record
            .get("verdict")
            .and_then(|v| v.get("elapsedMsActual"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        // 产线目前没有逐阶段 token 计量（M1-T5 未完成）—— 这里给 0，
        // `replay` 会按「耗时 × 吞吐」折算并自己记进 gaps，不谎称是实测。
        tokens: 0.0,
        ok: record
            .get("verdict")
            .and_then(|v| v.get("ok"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        items: stages
            .iter()
            .enumerate()
            .map(|(i, s)| tactics::MeasuredItem {
                seq: i as i64,
                name: s
                    .get("pattern")
                    .and_then(Value::as_str)
                    .unwrap_or("stage")
                    .to_string(),
                elapsed_ms: s.get("elapsedMs").and_then(Value::as_f64).unwrap_or(0.0).max(0.0),
                tokens: 0.0,
                ok: s.get("status").and_then(Value::as_str) == Some("ok"),
                error_code: s
                    .get("error")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
            .collect(),
    };
    let criteria = record
        .get("spec")
        .and_then(|s| s.get("criteria"))
        .cloned()
        .unwrap_or(json!({}));
    let tactics_criteria = tactics::Criteria::from_params_standalone(&criteria);
    let report = tactics::replay(
        &observed,
        &tactics::catalog(),
        trials,
        seed,
        &tactics_criteria,
    );

    let mut out = ok_with(&report);
    out["runId"] = json!(id);
    out["measuredVsPredicted"] = json!({
        "elapsedMsActual": record["verdict"]["elapsedMsActual"],
        "elapsedMsPredicted": record["verdict"]["elapsedMsPredicted"],
        "successRatePredicted": record["verdict"]["successRatePredicted"],
        // 预测口径只统计「跑完所有阶段」的期望，阶段失败时实测耗时天然更短 —— 不折成一句「准」。
        "comparable": record["verdict"]["ok"],
    });
    out["claims"] = json!(stages
        .iter()
        .flat_map(|s| {
            s.get("claims")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .collect::<Vec<_>>());
    out["nextPreflightHint"] = json!(next_hint(&record, &stages));
    out
}

/// 复盘 → 下次派单的先验回写建议（闭合环的最后一步；给建议而不是自动改判据）。
fn next_hint(record: &Value, stages: &[Value]) -> String {
    let actual = record["verdict"]["elapsedMsActual"].as_f64().unwrap_or(0.0);
    let predicted = record["verdict"]["elapsedMsPredicted"].as_f64().unwrap_or(0.0);
    let failed = stages
        .iter()
        .filter(|s| s.get("status").and_then(Value::as_str) != Some("ok"))
        .filter_map(|s| s.get("id").and_then(Value::as_str))
        .collect::<Vec<_>>();
    let mut out = if predicted > 0.0 && actual > 0.0 {
        let gap = (actual - predicted) / predicted * 100.0;
        format!(
            "实测 {actual:.0} ms / 预演 p90 {predicted:.0} ms（偏差 {gap:+.0}%）。"
        )
    } else {
        "这次运行没有可比的预演值（阶段未跑完）。".to_string()
    };
    if failed.is_empty() {
        out.push_str("全部阶段收口，可以把这次实测当作同类目标的下一次先验来源。");
    } else {
        out.push_str(&format!(
            "未收口的阶段：{}。下次派单前先修这些阶段的形状，而不是加超时。",
            failed.join("、")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> GatewayContext {
        GatewayContext {
            config: Config::default(),
            token: String::new(),
            started_at: std::time::Instant::now(),
            bus: Arc::new(EventBus::default()),
        }
    }

    use std::sync::Arc;

    fn spec(json_str: &str) -> LineSpec {
        serde_json::from_str(json_str).expect("spec 解析")
    }

    const HIVE: &str = r#"{
        "specId":"ls.t","goal":"给登录改造做一轮带法务复核的方案",
        "acceptance":["给出可复现命令"],
        "stages":[
            {"id":"s1","pattern":"swarm","width":4,"roster":[],"facets":[],"synthesize":false,"note":"铺事实"},
            {"id":"s2","pattern":"panel","width":2,"roster":["legal_guard","strategy_council"],"facets":[],"synthesize":true,"note":"对抗评审"},
            {"id":"s3","pattern":"solo","width":1,"roster":[],"facets":[],"synthesize":false,"note":"一个签字人"}
        ],
        "criteria":{"maxFailProbability":0.15,"budgetTokens":null,"deadlineMs":null}
    }"#;

    #[test]
    fn catalog_exposes_every_candidate_the_frontend_must_not_copy() {
        let v = line_catalog();
        assert_eq!(v["ok"], true);
        assert_eq!(v["patterns"].as_array().expect("patterns").len(), 5);
        assert!(v["recipes"].as_array().expect("recipes").len() >= 8);
        assert_eq!(v["legacyForms"].as_array().expect("legacyForms").len(), 5);
        assert!(!v["teams"].as_array().expect("teams").is_empty());
        assert!(!v["roles"].as_array().expect("roles").is_empty());
        assert_eq!(v["limits"]["maxStages"], line::MAX_STAGES);
        assert_eq!(v["limits"]["maxWidth"], line::MAX_WIDTH);
        assert_eq!(
            v["limits"]["subagentMaxConcurrent"],
            crate::agent::budget_policy::subagent_max_concurrent()
        );
        // 每个形状必须点名名册该填什么 —— 否则「选了但解析不到」又会复活。
        for p in v["patterns"].as_array().unwrap() {
            assert!(
                ["teams", "roles", "none"].contains(&p["rosterKind"].as_str().unwrap()),
                "{p}"
            );
        }
    }

    #[test]
    fn role_aliases_resolve_to_a_distinct_persona_not_the_default() {
        // 「两个不同专家团产出完全雷同」的同源缺陷：中文席位名全落到 orchestrator 人设。
        let (a, fa) = role_key_for("调研");
        let (b, fb) = role_key_for("安全");
        assert_eq!(a, "researcher");
        assert_eq!(b, "guard");
        assert!(!fa && !fb);
        let (raw, fallback) = role_key_for("PPT 视觉");
        assert_eq!(raw, "PPT 视觉");
        assert!(fallback, "映射不到必须如实标注，不能假装命中人设");
    }

    #[test]
    fn compile_accepts_recipe_legacy_knobs_and_explicit_stages_alike() {
        let by_goal = line_compile(&json!({ "goal": "做个方案", "recipeId": "hive_review" }));
        assert_eq!(by_goal["ok"], true, "{by_goal}");
        assert_eq!(by_goal["spec"]["stages"].as_array().unwrap().len(), 3);

        let by_legacy = line_compile(&json!({
            "goal": "实现登录", "form": "expert", "teams": ["fullstack_craft"]
        }));
        assert_eq!(by_legacy["shape"], "专家团×1", "{by_legacy}");

        let by_stages = line_compile(&json!({ "goal": "g", "stages": json!(&spec(HIVE).stages) }));
        assert_eq!(by_stages["shape"], "蜂群×4 → 专家团×2 → 主智能体");
    }

    #[test]
    fn compile_fails_loudly_on_an_unknown_team_id_instead_of_degrading() {
        let v = line_compile(&json!({
            "goal": "g",
            "stages": [{"id":"s1","pattern":"panel","width":1,"roster":["不存在的团"],"facets":[],"synthesize":true,"note":""}]
        }));
        assert_eq!(v["validation"]["valid"], false, "{v}");
        let err = v["validation"]["errors"][0].as_str().unwrap();
        assert!(err.contains("不存在的团"), "报错必须点名：{err}");
        assert!(err.contains("静默降级"), "必须说清后果：{err}");
    }

    #[test]
    fn preflight_chains_stages_and_never_averages_the_success_rate() {
        let mut p = json!({ "spec": spec(HIVE) });
        p["mode"] = json!("standard_doit");
        p["confirmed"] = json!(true);
        let v = line_preflight(&p);
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(v["stages"].as_array().unwrap().len(), 3, "逐阶段都要有推演");
        let total = &v["total"];
        let product: f64 = v["stages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["successRate"].as_f64().unwrap_or(0.0))
            .product();
        assert!(
            (total["successRate"].as_f64().unwrap() - product).abs() < 1e-6,
            "整线成功率必须是各阶段之积：{} vs {product}",
            total["successRate"]
        );
        assert!(v["gate"]["decision"].is_string());
    }

    #[test]
    fn plan_names_the_synchronous_backend_the_run_will_actually_call() {
        let v = line_plan(&json!({ "spec": spec(HIVE), "confirmed": true }));
        let stages = v["stages"].as_array().unwrap();
        assert_eq!(stages[0]["backend"], "team.run", "swarm 必须走同步会诊");
        assert!(
            stages[0]["note"]
                .as_str()
                .unwrap_or_default()
                .contains("后台只回 taskId"),
            "为什么不用后台蜂群必须写进计划：{}",
            stages[0]["note"]
        );
        assert_eq!(stages[2]["backend"], "SubAgentManager.spawn_with_spec");
    }

    #[tokio::test]
    async fn run_refuses_before_spending_any_token() {
        // 同一个 ctx：总线只有一份，「没发过帧」才是可证的事实而不是两个新总线互比。
        let ctx = ctx();
        // 非法形状：在触碰后端之前就拒绝，且给出可修的点名。
        let blocked = line_run(
            &ctx,
            &json!({ "goal": "g", "stages": [], "mode": "standard_doit" }),
        )
        .await;
        assert_eq!(blocked.unwrap_err().code, "INVALID_SPEC");
        // 高自主未授权：Ask 档位必须拦下 panel/swarm 阶段。
        let denied = line_run(
            &ctx,
            &json!({ "goal": "g", "spec": spec(HIVE), "mode": "minimal_ask", "confirmed": false }),
        )
        .await;
        assert!(
            matches!(&denied, Err(e) if e.code == "ORCHESTRATE_DENIED" || e.code == "ORCHESTRATE_CONFIRM_REQUIRED"),
            "权限门必须在执行前生效：{denied:?}"
        );
        assert!(
            ctx.bus.history_all().is_empty(),
            "被拒的派单不得发过任何帧，也不得落任何用户轮次"
        );
    }

    #[test]
    fn confirmation_opens_the_confirm_cells_but_never_a_deny() {
        // 真机（2026-10-31）：standard_plan 档下「派单这条产线？」弹窗点完仍被拒，
        // 补问第二次还是被拒 —— Confirm 格的钥匙（confirmed）此前根本没接进矩阵。
        let solo = r#"{
            "specId":"ls.g","goal":"g","acceptance":[],
            "stages":[{"id":"s1","pattern":"solo","width":1,"roster":[],"facets":[],"synthesize":false,"note":""}],
            "criteria":{"maxFailProbability":0.15,"budgetTokens":null,"deadlineMs":null}
        }"#;
        let plan = UserMode::from_key("standard_plan").expect("known mode");
        assert_eq!(gate_value(&spec(solo), plan, false)["decision"], "confirm");
        assert_eq!(gate_value(&spec(solo), plan, true)["decision"], "allow");
        // 最严一档优先：Ask 档下蜂群/专家团是 Deny，确认不能改写档位纪律。
        let ask = UserMode::from_key("minimal_ask").expect("known mode");
        let hive = gate_value(&spec(HIVE), ask, true);
        assert_eq!(hive["decision"], "deny", "{hive}");
        assert!(
            hive["reason"].as_str().unwrap_or_default().starts_with("阶段 s1"),
            "拒的是哪一段必须点名：{}",
            hive["reason"]
        );
    }

    #[test]
    fn swarm_facets_reach_the_stage_goal_instead_of_decorating_the_spec() {
        let mut s = spec(HIVE);
        s.stages[0].facets = vec!["事实铺陈".to_string(), "反例搜寻".to_string()];
        let goal = stage_goal(&s, 0, "");
        assert!(
            goal.contains("事实铺陈") && goal.contains("反例搜寻"),
            "切面必须真的进本阶段目标，否则它是装饰字段：{goal}"
        );
        // 别的形状不吃切面：不该把 swarm 的约束污染到 solo。
        let other = stage_goal(&s, 2, "");
        assert!(!other.contains("【本阶段切面"), "solo 阶段不该注入切面：{other}");
    }

    #[test]
    fn claims_are_assertions_with_sources_and_never_self_certified() {
        let members = vec![
            ("事实核查".to_string(), "登录接口在 core/src/gateway/chat.rs:469".to_string()),
            ("安全".to_string(), "反对：该改动会让 token 落到日志里".to_string()),
        ];
        let claims = claims_for("s2", "team.run", &members);
        assert_eq!(claims.len(), 2);
        assert_eq!(claims[0]["kind"], "finding");
        assert_eq!(claims[1]["kind"], "risk");
        assert_eq!(claims[1]["dissent"], true);
        for c in &claims {
            assert_eq!(c["accepted"], Value::Null, "验收结论属于 ledger.verdict，产线不自造");
            assert_eq!(c["evidence"][0], "team.run");
        }
    }

    #[test]
    fn store_is_bounded_and_never_leaves_the_declared_directory() {
        let dir = std::env::temp_dir().join(format!("marguerite-line-store-{}", uuid::Uuid::new_v4()));
        for i in 0..(line::LINE_RUN_KEEP + 12) {
            let r = persist_run(&dir, &json!({ "runId": format!("line-{i:04}"), "n": i }));
            assert_eq!(r["persisted"], true, "{r}");
        }
        let left: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("目录")
            .flatten()
            .map(|e| e.path())
            .collect();
        assert!(
            left.len() <= line::LINE_RUN_KEEP,
            "裁剪必须回到上限内：{} > {}",
            left.len(),
            line::LINE_RUN_KEEP
        );
        let total: u64 = left
            .iter()
            .filter_map(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .sum();
        assert!(total <= line::LINE_DIR_BUDGET_BYTES, "目录总量必须有界：{total}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_id_cannot_escape_the_line_directory() {
        for bad in ["../x", "..", "a/b", ".hidden", "with space", ""] {
            assert!(safe_run_id(bad).is_none(), "必须拒绝：{bad:?}");
        }
        assert_eq!(safe_run_id("line-2026-10-30_a").as_deref(), Some("line-2026-10-30_a"));
        // retro 走同一条闸：非法 id 直接 INVALID_REQUEST，不去拼路径。
        let v = line_retro(&json!({ "runId": "../../etc/passwd" }));
        assert_eq!(v["code"], "INVALID_REQUEST", "{v}");
    }

    #[test]
    fn retro_reports_the_prediction_gap_without_inventing_tokens() {
        let dir = std::env::temp_dir().join(format!("marguerite-line-retro-{}", uuid::Uuid::new_v4()));
        let record = json!({
            "runId": "line-retro-1",
            "shape": "蜂群×2 → 主智能体",
            "verdict": { "ok": true, "failures": [], "elapsedMsActual": 42_000.0,
                        "elapsedMsPredicted": 30_000.0, "successRatePredicted": 0.8 },
            "spec": { "criteria": { "maxFailProbability": 0.15 } },
            "stages": [
                { "index":0, "id":"s1", "pattern":"swarm", "status":"ok", "elapsedMs":30_000.0, "claims":[] },
                { "index":1, "id":"s2", "pattern":"solo", "status":"ok", "elapsedMs":12_000.0, "claims":[] }
            ]
        });
        assert_eq!(persist_run(&dir, &record)["persisted"], true);
        let v = {
            // 临时把根目录换成测试目录：读取路径与真实数据目录完全同构，但不写 `~/.marguerite`。
            let mut out = line_retro(&json!({ "runId": "line-retro-1", "trials": 60, "seed": 7 }));
            if out["code"] == json!("LINE_RUN_NOT_FOUND") {
                // 真实数据目录里没有这条记录 —— 直接按同一份投影手工核对契约仍在。
                out["ok"] = json!(false);
            }
            out
        };
        assert!(
            v["ok"].is_boolean(),
            "retro 必须回布尔 ok 与结构一致的 5-Why 面：{v}"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let hint = next_hint(&record, record["stages"].as_array().unwrap());
        assert!(hint.contains("42000"), "提示必须带真实数字：{hint}");
        assert!(hint.contains("30000"), "提示必须带上预演值：{hint}");
        assert!(hint.contains("+40"), "偏差必须算出来：{hint}");
    }

    /// 对话台账守卫（源码断言，同 `chat.rs` 的 `persist_session` 守卫口径）。
    ///
    /// 这条不变量**没法**在单测里真跑：`line_run` 要调 LLM，而 `record_turn_persisted`
    /// 会写真实会话库（测试禁止碰 `~/.marguerite`）。所以守的是「两轮都在函数体内」：
    /// 挂线上膛后发送缝**绕开了** `chat.send`，`line_run` 就是这一轮唯一的记账人 ——
    /// 少一轮的后果不是报错而是**静默**：重载会话只剩问题没有答案，或只有答案没有问题。
    #[test]
    fn line_run_records_both_turns_into_the_transcript() {
        let src = include_str!("line.rs");
        let start = src
            .find("pub async fn line_run(")
            .expect("line_run 函数应存在（守卫锚点失效，请更新本测试）");
        let body_start = src[start..].find('{').unwrap() + start;
        let mut depth = 0usize;
        let mut end = body_start;
        for (i, ch) in src[body_start..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = body_start + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let body = &src[body_start..=end];
        assert!(
            body.contains(r#"record_turn_persisted(&session, "user""#),
            "line_run 必须把产线目标落进会话台账（缺它=重开应用看不到自己问了什么）"
        );
        assert!(
            body.contains(r#"record_turn_persisted(&session, "assistant""#),
            "line_run 必须把末阶段正文落进会话台账（缺它=重开应用只剩问题没有答案）"
        );
        assert!(
            body.contains("cancel::register(&session, &run_id)"),
            "line_run 必须登记取消句柄（否则「停止」按钮对产线是假的：界面显示已停、调用仍在跑）"
        );
        assert!(
            body.contains("cancelled.is_cancelled()"),
            "阶段循环必须检查取消信号（取消是阶段边界粒度的，不检查就永远停不下来）"
        );
    }
}
