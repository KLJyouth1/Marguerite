//! 澄清问卷 RPC 面：`chat.clarify`（开启/快照一批提问）与 `chat.clarify_submit`（回填答案）。
//!
//! # 为什么必须由 **Rust** 网关提供
//!
//! 控制台的澄清卡片完全由事件流驱动：前端（`webui/src/composables/chat/useChatApprovals.ts`
//! 的 `handleToolResult`）只认 `session.event.tool_result` 里 `kind:"user_input",
//! paused:true` 的载荷，答案经 `chat.clarify_submit` 回传。此前这两个方法都在
//! `PY_ENGINE_PREFIXES`（`methods.rs` 的 `"chat.clarify"` 前缀）里 —— 纯 Rust 模式
//! （侧车离线/失败）下 `chat.clarify_submit` 只能拿到 NOT_IMPLEMENTED：卡片或许推得出
//! （历史遗留载荷），答案却永远提交不回去，**整条澄清链在纯 Rust 模式下是断的**。
//! 而 `mode/ask.rs` 的 `QuestionQueue`（ask_user 二开的提问模型，fail-closed 语义）
//! 一直没有消费者 —— 本模块就是它的第一任消费者：Rust 侧真实开卡、真实回填、真实推进分支。
//!
//! # 契约（与 pyengine `rpc_chat.py::chat_clarify_submit` / 前端解析器对齐）
//!
//! - 挂起载荷（tool_result 事件 payload）：
//!   `{ session_key, key, result: { kind:"user_input", paused:true, request_id, run_id,
//!   step, clarify_schema:{ intro, fields:[{name, prompt, type:"choice"|"string", required,
//!   options:[{label,description}], choices:[label…], allow_other?, header?}] } } }`
//!   —— 刻意**不**携带 `presentation:"plan_questionnaire_v1"`：那是计划问卷的逐题向导
//!   模式，会把计划专属 dock 行为拖进团队开工场景。同批字段整卡渲染。
//! - 终态载荷：`{ kind:"user_input", paused:false, status:"answered", request_id }`
//!   （前端据此折叠卡片）。
//! - `chat.clarify_submit` 请求：`{ sessionKey, fields:{name:value}, request_id?, run_id? }`。
//! - 每次阶段推进都会**换新 request_id**（前端 ClarifyCard 仅在 requestId 变化时重置表单，
//!   复用旧 id 会让第二阶段字段无法渲染）。
//!
//! # 两侧队列（双引擎）与诚实规则
//!
//! 侧车自己也会为交互工具挂起抬澄清（它的 request_id 不在本注册表里）。与
//! [`crate::gateway::approvals`] 同一口径：**先查本地注册表，未命中如实转发给侧车**
//! （3s 超时，与 `py_call_timeout` 一致），两侧都没有 → 明确报错，绝不假成功；
//! 选项标签之外的提交按自定义回答记录（`custom`），未知字段名进 `ignored_fields`
//! 如实上报，不静默吞掉。
//!
//! 会员/素材库探测字段维持三态诚实口径（[`crate::gateway::team::MembershipProbe`]）：
//! 自动探测需要 computer-use 感知层（VLMM 接地 + 多步 UIA）才能完成，本 RPC 链路
//! 不做屏幕抓取，题面按 `未探测` 呈现 —— 详见 `team::current_membership_probe`。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};

use crate::events::{Envelope, EventBus, EventType};
use crate::gateway::methods::GatewayContext;
use crate::gateway::protocol::RpcError;
use crate::gateway::server;
use crate::gateway::team;
use crate::mode::ask::{Answer, Question, QuestionQueue};
use uuid::Uuid;

/// 转发侧车的超时：与 `methods.rs::py_call_timeout` 同值（3s）—— WS dispatch
/// 不能被慢侧车拖住（tick 停发即前端断连降级）。
const SIDECAR_FORWARD_TIMEOUT: Duration = Duration::from_secs(3);

/// 一个会话的进行中澄清队列（进程级注册表；每个会话至多一张开工问卷）。
struct SessionClarify {
    run_id: String,
    /// 阶段号（1 起）。request_id = `<run_id>-s<stage>`，阶段推进必须换 id。
    stage: u32,
    queue: QuestionQueue,
    intro: &'static str,
}

impl SessionClarify {
    fn request_id(&self) -> String {
        format!("{}-s{}", self.run_id, self.stage)
    }
}

fn registry() -> &'static Mutex<HashMap<String, SessionClarify>> {
    static REG: OnceLock<Mutex<HashMap<String, SessionClarify>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 取注册表锁：中毒时取内值继续 —— 澄清队列是纯内存状态，宁可带着未完成的
/// 队列继续服务，也不能让一次 panic 把整条 RPC 面变成永久 panic。
fn lock_registry() -> std::sync::MutexGuard<'static, HashMap<String, SessionClarify>> {
    registry().lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `chat.clarify` / `chat.clarify_submit` 的模块入口（由 `methods::dispatch` 在
/// 大 match 之后、侧车代理之前拦截分派 —— 见 methods.rs 注册点注释）。
pub(crate) async fn dispatch(ctx: &GatewayContext, method: &str, params: &Value) -> Result<Value, RpcError> {
    match method {
        "chat.clarify" => clarify_open(ctx, params).await,
        "chat.clarify_submit" => clarify_submit(ctx, params).await,
        other => Err(RpcError::not_implemented(other)),
    }
}

/// `chat.clarify`：开启（或快照）一个会话的澄清队列。
///
/// - 会话已有队列 → 快照当前阶段并原 request_id 重发挂起载荷（前端按 requestId 去重，幂等安全）；
/// - 参数带团队 id 且该团队有开工问卷 → 用 [`QuestionQueue`] 装载并开启第一阶段；
/// - 两者皆无 → 侧车在线时转发（保留侧车自身交互工具的澄清兼容面），
///   否则如实报 `CLARIFY_NO_QUESTIONNAIRE`（不伪造一张空问卷冒充成功）。
async fn clarify_open(ctx: &GatewayContext, params: &Value) -> Result<Value, RpcError> {
    let key = crate::gateway::methods::session_key_param(params)?;
    let team_id = crate::gateway::_helpers::param_str_keys(params, &["teamId", "team_id", "team"], "");
    let local = {
        let mut reg = lock_registry();
        // 用 contains_key 判存在，而不是 `if let Some(entry)=reg.get_mut(..) … else reg.insert(..)`
        // —— 后者的 get_mut 借用会覆盖整个 if/else 链，else 分支再碰 reg 直接 E0499。
        if reg.contains_key(&key) {
            let entry = reg.get_mut(&key).expect("contains_key 刚确认存在");
            let batch = entry.queue.ready_batch();
            let rid = entry.request_id();
            Some(publish_stage_and_summary(ctx, &key, entry, &rid, batch))
        } else if let Some((intro, questions)) = team::kickoff_questionnaire_for(team_id.trim()) {
            let mut queue = QuestionQueue::new();
            let enqueued = queue.ask_many(questions);
            if enqueued.is_empty() {
                return Err(RpcError::internal("开工问卷校验失败：没有任何问题入队"));
            }
            let mut entry = SessionClarify {
                run_id: format!("clr-{}", Uuid::new_v4()),
                stage: 1,
                queue,
                intro,
            };
            let batch = entry.queue.ready_batch();
            let rid = entry.request_id();
            let summary = publish_stage_and_summary(ctx, &key, &entry, &rid, batch);
            reg.insert(key.clone(), entry);
            Some(summary)
        } else {
            None
        }
    }; // 锁在此释放 —— 转发路径的 await 不得持锁
    if let Some(res) = local {
        return res;
    }
    if let Some(res) = forward_to_sidecar("chat.clarify", params).await {
        return res;
    }
    Err(RpcError::new(
        "CLARIFY_NO_QUESTIONNAIRE",
        format!(
            "会话 {key} 没有进行中的澄清队列，参数也未命中任何团队的开工问卷\
             （侧车不在线，无法转发；不伪造空问卷冒充成功）"
        ),
    ))
}

/// 发布当前阶段的挂起载荷并返回 RPC 摘要（快照与首轮开启共用）。
fn publish_stage_and_summary(
    ctx: &GatewayContext,
    key: &str,
    entry: &SessionClarify,
    rid: &str,
    batch: Vec<Question>,
) -> Result<Value, RpcError> {
    emit_paused(ctx, key, &entry.run_id, rid, entry.stage, entry.intro, &batch);
    Ok(json!({
        "ok": true,
        "session_key": key,
        "run_id": entry.run_id,
        "request_id": rid,
        "stage": entry.stage,
        "intro": entry.intro,
        "fields": batch.iter().map(|q| q.id.clone()).collect::<Vec<_>>(),
        "pending_count": entry.queue.pending_count(),
    }))
}

/// `chat.clarify_submit`：回填一批答案，并推进分支/阶段。
///
/// 与 approvals 的 `forward_resolve` 同型：**本地注册表命中 → 真实记录 Answer；
/// 未命中（无队列 / request_id 不是当前阶段）→ 转发侧车；两侧皆无 → 明确错误**。
/// 错误消息必须说明「本次提交没有记录到任何地方」，否则就是一次假成功。
async fn clarify_submit(ctx: &GatewayContext, params: &Value) -> Result<Value, RpcError> {
    let key = crate::gateway::methods::session_key_param(params)?;
    let provided_rid = params
        .get("request_id")
        .or_else(|| params.get("requestId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let local = {
        let mut reg = lock_registry();
        submit_locked(ctx, &mut reg, &key, params, provided_rid.as_deref())
    }; // 锁释放后才 await 转发
    if let Some(res) = local {
        return res;
    }
    if let Some(res) = forward_to_sidecar("chat.clarify_submit", params).await {
        return res;
    }
    Err(RpcError::new(
        "CLARIFY_REQUEST_NOT_FOUND",
        format!(
            "会话 {key} 在 Rust 侧没有匹配的澄清队列（request_id 缺失/过期，或该澄清由侧车挂起），\
             且侧车当前不可转发 —— 本次提交未被记录到任何地方"
        ),
    ))
}

/// 本地注册表内的提交处理。返回 `None` = 本地无主（无队列，或提交的 request_id
/// 与当前阶段不符）→ 调用方转侧车/报错；返回 `Some(Err(..))` = 本地命中但请求本身非法。
fn submit_locked(
    ctx: &GatewayContext,
    reg: &mut HashMap<String, SessionClarify>,
    key: &str,
    params: &Value,
    provided_rid: Option<&str>,
) -> Option<Result<Value, RpcError>> {
    let entry = reg.get_mut(key)?;
    let current_rid = entry.request_id();
    if let Some(rid) = provided_rid {
        if rid != current_rid {
            // 阶段已推进（或这是侧车侧的 request_id）：本地拒收旧 id，交由调用方转发
            return None;
        }
    }
    let Some(fields) = params.get("fields").and_then(Value::as_object) else {
        return Some(Err(RpcError::new("INVALID_REQUEST", "fields 必须是 {name: value} 对象")));
    };
    let mut recorded = Vec::new();
    let mut ignored: Vec<String> = Vec::new();
    for (name, value) in fields {
        // 只回填当前仍挂起的问题；未知 id / 已回答 / 空白值都进 ignored，不静默吞
        let pending = entry.queue.pending().iter().find(|q| q.id == *name).cloned();
        match pending {
            Some(q) => match classify_answer(&q, value) {
                Some(ans) => {
                    let ok = entry.queue.answer(ans);
                    if ok {
                        recorded.push(json!({
                            "id": q.id,
                            "selected": value_cloned_selected(&q, value),
                            "custom": value_cloned_custom(&q, value),
                        }));
                    } else {
                        ignored.push(name.clone());
                    }
                }
                None => ignored.push(name.clone()),
            },
            None => ignored.push(name.clone()),
        }
    }
    if recorded.is_empty() {
        return Some(Err(RpcError::new(
            "CLARIFY_NO_VALID_ANSWER",
            format!("提交的字段里没有任何可回填的挂起问题（ignored_fields={ignored:?}）"),
        )));
    }
    let run_id = entry.run_id.clone();
    emit_answered(ctx, key, &run_id, &current_rid);
    // 推进：ready_batch 顺带丢弃不可达分支追问；剩余就绪题开启新阶段
    let next = entry.queue.ready_batch();
    let summary = json!({
        "ok": true,
        "session_key": key,
        "run_id": run_id,
        "answered_request_id": current_rid,
        "recorded": recorded,
        "ignored_fields": ignored,
        "completed": next.is_empty() && entry.queue.is_empty(),
        "pending_count": entry.queue.pending_count(),
    });
    if next.is_empty() {
        let completed = entry.queue.is_empty();
        if completed {
            reg.remove(key);
        }
        // 就绪批为空但仍有 pending（依赖死锁）→ 保留队列，如实报告 pending_count
        return Some(Ok(summary));
    }
    entry.stage += 1;
    let next_rid = entry.request_id();
    let next_fields: Vec<String> = next.iter().map(|q| q.id.clone()).collect();
    emit_paused(ctx, key, &run_id, &next_rid, entry.stage, entry.intro, &next);
    Some(Ok(json!({
        "ok": true,
        "session_key": key,
        "run_id": run_id,
        "answered_request_id": current_rid,
        "recorded": recorded,
        "ignored_fields": ignored,
        "completed": false,
        "next_request_id": next_rid,
        "next_fields": next_fields,
        "pending_count": entry.queue.pending_count(),
    })))
}

/// 把提交值分类成 [`Answer`]：命中选项标签进 `selected`，其余文本进 `custom`
/// （「其他-自填」与多选混合都走这条口径）。全空白 → `None`（不算回答）。
fn classify_answer(q: &Question, value: &Value) -> Option<Answer> {
    let strings = value_strings(value);
    let mut selected: Vec<String> = Vec::new();
    let mut custom: Option<String> = None;
    for s in strings {
        if q.options.iter().any(|o| o.label == s) {
            selected.push(s);
        } else if custom.is_none() {
            custom = Some(s);
        }
    }
    if selected.is_empty() && custom.is_none() {
        return None;
    }
    Some(Answer {
        id: q.id.clone(),
        selected,
        custom,
    })
}

fn value_strings(value: &Value) -> Vec<String> {
    let one = |v: &Value| -> Option<String> {
        v.as_str()
            .map(str::to_string)
            .or_else(|| v.as_bool().map(|b| b.to_string()))
    };
    let raw: Vec<String> = match value {
        Value::Array(items) => items.iter().filter_map(one).collect(),
        other => one(other).map(|s| vec![s]).unwrap_or_else(|| vec![other.to_string()]),
    };
    raw.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// 回执里如实回显分类结果（与 classify_answer 同一口径；调用方据此核对，
/// 而不是「记录成功」四个字糊弄 —— 假成功不可接受）。
fn value_cloned_selected(q: &Question, value: &Value) -> Vec<String> {
    value_strings(value)
        .into_iter()
        .filter(|s| q.options.iter().any(|o| &o.label == s))
        .collect()
}

fn value_cloned_custom(q: &Question, value: &Value) -> Option<String> {
    value_strings(value)
        .into_iter()
        .find(|s| !q.options.iter().any(|o| &o.label == s))
}

/// 澄清 schema 的单字段序列化（形状对齐 `webui/src/utils/chat/clarify.ts` 解析器；
/// 未知键会被前端忽略，因此这里只写前端消费过的键）。
fn field_json(q: &Question) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("name".into(), json!(q.id));
    obj.insert("prompt".into(), json!(q.question));
    obj.insert(
        "type".into(),
        json!(if q.options.is_empty() { "string" } else { "choice" }),
    );
    obj.insert("required".into(), json!(true));
    if !q.options.is_empty() {
        obj.insert(
            "options".into(),
            json!(q
                .options
                .iter()
                .map(|o| json!({
                    "label": o.label,
                    "description": o.description.clone().unwrap_or_default(),
                }))
                .collect::<Vec<_>>()),
        );
        obj.insert(
            "choices".into(),
            json!(q.options.iter().map(|o| o.label.clone()).collect::<Vec<_>>()),
        );
    }
    if q.allow_other {
        obj.insert("allow_other".into(), json!(true));
    }
    if let Some(header) = &q.header {
        obj.insert("header".into(), json!(header));
    }
    if q.multi_select {
        obj.insert("multi_select".into(), json!(true));
    }
    Value::Object(obj)
}

/// 挂起态 tool_result：前端 ClarifyCard 的唯一开卡入口。
///
/// `session_key` 与 `key` 都要写 —— 前端的会话过滤读
/// `payload.key || payload.session_key || payload.sessionKey`（streamEvents.ts）。
fn emit_paused(ctx: &GatewayContext, key: &str, run_id: &str, request_id: &str, stage: u32, intro: &str, fields: &[Question]) {
    let payload = json!({
        "session_key": key,
        "key": key,
        "tool": "clarify",
        "result": {
            "kind": "user_input",
            "paused": true,
            "request_id": request_id,
            "run_id": run_id,
            "step": format!("kickoff-stage-{stage}"),
            "clarify_schema": {
                "intro": intro,
                "fields": fields.iter().map(field_json).collect::<Vec<_>>(),
            },
        }
    });
    publish_tool_result(&ctx.bus, key, payload);
}

/// 终态 tool_result：`paused:false` + `status:"answered"`，前端据此折叠卡片。
fn emit_answered(ctx: &GatewayContext, key: &str, run_id: &str, request_id: &str) {
    let payload = json!({
        "session_key": key,
        "key": key,
        "tool": "clarify",
        "result": {
            "kind": "user_input",
            "paused": false,
            "status": "answered",
            "request_id": request_id,
            "run_id": run_id,
        }
    });
    publish_tool_result(&ctx.bus, key, payload);
}

fn publish_tool_result(bus: &Arc<EventBus>, key: &str, payload: Value) {
    bus.publish(Envelope::new(EventType::ToolResult, payload, key));
}

/// 把请求原样转发给侧车；`None` = 侧车不可用（离线/超时），调用方据此给明确错误。
async fn forward_to_sidecar(method: &str, params: &Value) -> Option<Result<Value, RpcError>> {
    let client = server::py_client()?;
    let res = tokio::time::timeout(SIDECAR_FORWARD_TIMEOUT, client.call(method, params.clone())).await;
    Some(match res {
        Ok(inner) => inner,
        Err(_) => Err(RpcError::internal(&format!("Python 侧车响应超时（{method}）"))
            .with_retryable(true, Some(800))),
    })
}

#[cfg(test)]
mod clarify_chain_tests {
    //! 纯 Rust 模式澄清链的路线级测试：全部经 `methods::dispatch` 走真实分派
    //! 路径（含 methods.rs 的拦截注册点），断言事件面载荷形状与队列推进。
    //! 侧车在单测里未注册（`server::py_client()` 为 None）→ 转发路径天然不可用，
    //! 正好钉住「两侧皆无时必须如实报错」。

    use super::*;
    use crate::gateway::methods;
    use crate::persistence::config::Config;
    use std::time::Instant;

    fn test_ctx() -> (GatewayContext, Arc<EventBus>) {
        let bus = Arc::new(EventBus::default());
        (
            GatewayContext {
                config: Config::default(),
                token: String::new(),
                started_at: Instant::now(),
                bus: bus.clone(),
            },
            bus,
        )
    }

    fn unique_key(tag: &str) -> String {
        format!("clarify-test-{tag}-{}", Uuid::new_v4())
    }

    /// 事件历史里最新的澄清载荷（`result` 为 object 的 tool_result 信封）。
    fn clarify_results(bus: &Arc<EventBus>, since: Instant) -> Vec<Value> {
        bus.history(since)
            .into_iter()
            .filter(|e| e.r#type == EventType::ToolResult)
            .filter_map(|e| e.payload.get("result").cloned())
            .collect()
    }

    #[tokio::test]
    async fn chat_clarify_opens_deck_craft_and_publishes_pause_card() {
        let (ctx, bus) = test_ctx();
        let key = unique_key("open");
        let since = Instant::now();
        let res = methods::dispatch(&ctx, "chat.clarify", json!({ "sessionKey": key, "teamId": "deck_craft" }))
            .await
            .expect("chat.clarify 必须已被 Rust 网关服务（纯 Rust 模式）");
        assert_eq!(res["ok"], true);
        assert_eq!(res["fields"][0], "deck_form", "第一阶段只出分支 gate");
        assert_eq!(res["pending_count"], 5);
        let rid = res["request_id"].as_str().unwrap().to_string();
        assert!(rid.ends_with("-s1"), "阶段 request_id 必须可派生: {rid}");

        let events = clarify_results(&bus, since);
        let paused = events.iter().find(|r| r["paused"] == json!(true)).expect("应发布挂起载荷");
        assert_eq!(paused["kind"], "user_input");
        assert_eq!(paused["request_id"], rid);
        let schema = &paused["clarify_schema"];
        let fields = schema["fields"].as_array().unwrap();
        assert_eq!(fields[0]["name"], "deck_form");
        assert_eq!(fields[0]["type"], "choice");
        assert_eq!(fields[0]["allow_other"], true, "C 分支（其他自填）入口必须在字段上");
        assert_eq!(fields[0]["options"].as_array().unwrap().len(), 2);
        assert!(schema.get("presentation").is_none(), "不得携带 plan_questionnaire_v1（会拖进计划向导语义）");
        // 会话过滤键：前端读 payload.key / session_key
        let envelopes = bus.history(since);
        let ev = envelopes.iter().find(|e| e.r#type == EventType::ToolResult).unwrap();
        assert_eq!(ev.payload["session_key"], key);
        assert_eq!(ev.payload["key"], key);
        assert_eq!(ev.session, key);
    }

    #[tokio::test]
    async fn computer_branch_unlocks_software_and_membership_then_completes() {
        let (ctx, bus) = test_ctx();
        let key = unique_key("branch-a");
        let since = Instant::now();
        let opened = methods::dispatch(&ctx, "chat.clarify", json!({ "sessionKey": key, "teamId": "deck_craft" }))
            .await
            .unwrap();
        let rid1 = opened["request_id"].as_str().unwrap().to_string();

        let after_form = methods::dispatch(
            &ctx,
            "chat.clarify_submit",
            json!({
                "sessionKey": key,
                "request_id": rid1,
                "fields": { "deck_form": "操控电脑" },
            }),
        )
        .await
        .unwrap();
        assert_eq!(after_form["completed"], false);
        let next_fields: Vec<&str> = after_form["next_fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(next_fields.contains(&"deck_software"), "A 分支必须追问软件: {next_fields:?}");
        assert!(next_fields.contains(&"deck_membership"), "A 分支必须追问会员可用性: {next_fields:?}");
        let rid2 = after_form["next_request_id"].as_str().unwrap().to_string();
        assert_ne!(rid2, rid1, "阶段推进必须换新 request_id（前端靠它重置卡片）");

        let done = methods::dispatch(
            &ctx,
            "chat.clarify_submit",
            json!({
                "sessionKey": key,
                "request_id": rid2,
                "fields": {
                    "deck_software": "WPS",
                    "deck_membership": "无法确定",
                    "deck_outline": "按需求中的大纲执行",
                    "deck_image_source": "我自己供图，路径 D:\\pics",
                },
            }),
        )
        .await
        .unwrap();
        assert_eq!(done["completed"], true);
        assert_eq!(done["pending_count"], 0);
        // 自填值必须落到 custom，而不是被硬塞进 selected（伪造「选项命中」）
        let recorded = done["recorded"].as_array().unwrap();
        let membership = recorded
            .iter()
            .find(|r| r["id"] == "deck_membership")
            .unwrap();
        assert_eq!(membership["selected"].as_array().unwrap()[0], "无法确定");
        let image = recorded.iter().find(|r| r["id"] == "deck_image_source").unwrap();
        assert!(image["selected"].as_array().unwrap().is_empty());
        assert_eq!(image["custom"], "我自己供图，路径 D:\\pics");

        // 事件面：两次挂起 + 两次 answered 终态，顺序与阶段号一致
        let events = clarify_results(&bus, since);
        let paused: Vec<&Value> = events.iter().filter(|r| r["paused"] == json!(true)).collect();
        let answered: Vec<&Value> = events.iter().filter(|r| r["paused"] == json!(false)).collect();
        assert_eq!(paused.len(), 2, "阶段 1 + 阶段 2 各一条挂起载荷");
        assert_eq!(answered.len(), 2, "两次提交各一条 answered 终态（前端据此折叠卡片）");
        assert_eq!(answered[0]["request_id"], rid1);
        assert_eq!(answered[1]["request_id"], rid2);
        assert_eq!(answered[0]["status"], "answered");
    }

    #[tokio::test]
    async fn script_branch_never_asks_computer_questions_and_still_completes() {
        let (ctx, _bus) = test_ctx();
        let key = unique_key("branch-b");
        let opened = methods::dispatch(&ctx, "chat.clarify", json!({ "sessionKey": key, "teamId": "deck_craft" }))
            .await
            .unwrap();
        let rid1 = opened["request_id"].as_str().unwrap().to_string();
        let after = methods::dispatch(
            &ctx,
            "chat.clarify_submit",
            json!({ "sessionKey": key, "request_id": rid1, "fields": { "deck_form": "脚本生成" } }),
        )
        .await
        .unwrap();
        let next_fields: Vec<&str> = after["next_fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(!next_fields.contains(&"deck_software"), "B 分支不得追问软件");
        assert!(!next_fields.contains(&"deck_membership"), "B 分支不得追问会员");
        assert_eq!(next_fields, vec!["deck_outline", "deck_image_source"]);
        let rid2 = after["next_request_id"].as_str().unwrap().to_string();
        let done = methods::dispatch(
            &ctx,
            "chat.clarify_submit",
            json!({
                "sessionKey": key,
                "request_id": rid2,
                "fields": { "deck_outline": "先生成大纲再逐项确认", "deck_image_source": "自动生成图片" },
            }),
        )
        .await
        .unwrap();
        assert_eq!(done["completed"], true);
    }

    #[tokio::test]
    async fn custom_branch_skips_label_gated_followups() {
        // C 分支：自填「其他」不匹配任何选项标签 → A 分支专属追问被丢弃（不伪造回答），
        // 但「gate 已答」的大纲/图源仍要确认。
        let (ctx, _bus) = test_ctx();
        let key = unique_key("branch-c");
        let opened = methods::dispatch(&ctx, "chat.clarify", json!({ "sessionKey": key, "teamId": "deck_craft" }))
            .await
            .unwrap();
        let rid1 = opened["request_id"].as_str().unwrap().to_string();
        let after = methods::dispatch(
            &ctx,
            "chat.clarify_submit",
            json!({
                "sessionKey": key,
                "request_id": rid1,
                "fields": { "deck_form": "用在线协作工具直接产出" },
            }),
        )
        .await
        .unwrap();
        let next_fields: Vec<&str> = after["next_fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(next_fields, vec!["deck_outline", "deck_image_source"]);
    }

    #[tokio::test]
    async fn unknown_requests_fail_loudly_without_any_fake_success() {
        let (ctx, bus) = test_ctx();
        let key = unique_key("honest");
        let since = Instant::now();
        // 未开启的队列 + 侧车不在线 → 如实报错，不发任何假事件
        let err = methods::dispatch(
            &ctx,
            "chat.clarify_submit",
            json!({ "sessionKey": key, "fields": { "deck_form": "操控电脑" } }),
        )
        .await
        .expect_err("两侧皆无主时必须报错（假成功是红线）");
        assert_eq!(err.code, "CLARIFY_REQUEST_NOT_FOUND");
        assert!(bus.history(since).is_empty(), "失败提交不得发布任何事件");

        let err2 = methods::dispatch(
            &ctx,
            "chat.clarify",
            json!({ "sessionKey": unique_key("nope"), "teamId": "no_such_team" }),
        )
        .await
        .expect_err("无此团队问卷 → 如实拒绝");
        assert_eq!(err2.code, "CLARIFY_NO_QUESTIONNAIRE");

        // 缺 sessionKey → 响亮拒绝（复用 session_key_param 的口径）
        let err3 = methods::dispatch(&ctx, "chat.clarify", json!({ "teamId": "deck_craft" }))
            .await
            .expect_err("空会话键必须拒绝");
        assert_eq!(err3.code, "INVALID_REQUEST");
    }

    #[tokio::test]
    async fn stale_request_id_after_advance_is_not_locally_accepted() {
        let (ctx, _bus) = test_ctx();
        let key = unique_key("stale");
        let opened = methods::dispatch(&ctx, "chat.clarify", json!({ "sessionKey": key, "teamId": "deck_craft" }))
            .await
            .unwrap();
        let rid1 = opened["request_id"].as_str().unwrap().to_string();
        methods::dispatch(
            &ctx,
            "chat.clarify_submit",
            json!({ "sessionKey": key, "request_id": rid1, "fields": { "deck_form": "脚本生成" } }),
        )
        .await
        .unwrap();
        // 再拿旧 rid 提交 → 本地不再命中 → 侧车缺席 → NOT_FOUND（不能把重复提交记成新答案）
        let err = methods::dispatch(
            &ctx,
            "chat.clarify_submit",
            json!({ "sessionKey": key, "request_id": rid1, "fields": { "deck_form": "操控电脑" } }),
        )
        .await
        .expect_err("过期 request_id 必须拒收");
        assert_eq!(err.code, "CLARIFY_REQUEST_NOT_FOUND");
    }

    #[test]
    fn field_json_matches_the_frontend_parser_contract() {
        // 前端 clarify.ts 消费的键：name/prompt/type/required/options/choices/allow_other/header
        let q = Question {
            id: "q1".into(),
            header: Some("头".into()),
            question: "问题？".into(),
            options: vec![crate::mode::ask::QuestionOption {
                label: "甲".into(),
                description: None,
            }],
            multi_select: false,
            allow_other: true,
            depends_on: None,
        };
        let f = field_json(&q);
        assert_eq!(f["name"], "q1");
        assert_eq!(f["type"], "choice");
        assert_eq!(f["required"], true);
        assert_eq!(f["options"][0]["label"], "甲");
        assert_eq!(f["options"][0]["description"], "");
        assert_eq!(f["choices"][0], "甲");
        assert_eq!(f["allow_other"], true);
        assert_eq!(f["header"], "头");
        assert!(f.get("multi_select").is_none(), "未多选时不铺多余键");
    }
}
