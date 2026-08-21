//! 网关会话运行时（Bloom 模式融合落地）
//!
//! 在纯 Rust 引擎上跑真实对话：`chat.send` 后台 spawn [`RealEngine::run_turn_reply`]
//! （召回经验 → 自动编排 → 沉淀经验），事件经「私总线 → 网关总线」桥回流 Web UI；
//! 由网关统一发布富 `session.event.done` 收据（`text_snapshot` / `reason` / `task_id`），
//! 对齐 Web UI `useChatRpcEventHandlers` 的终端渲染契约。
//!
//! - **会话隔离**：引擎跑在私有 [`EventBus`] 上，桥只订阅 `{session}.*`，子代理事件
//!   （按 agent_id 分 session）天然不回流；转发前注入 `session_key` / `task_id`，
//!   多标签页 `isCurrentSessionPayload` 过滤互不串台。
//! - **单一收据**：桥跳过 `Done`（子代理 / lead 中间完成事件都不回流），由本模块在
//!   整轮结束后发布唯一一条富 Done。
//! - **幂等**：`clientRequestId` 登记后重放返回既有 `task_id`，防丢响应重试重复执行。
//! - **中止**：`chat.abort` 直接 abort 后台任务并发布 `reason=aborted` 收据。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::events::{Envelope, EventBus, EventType};
use crate::gateway::methods::GatewayContext;
use crate::gateway::protocol::RpcError;
use crate::memory::{
    cross_project::recall_stable_prefix, ExperienceLedger, HierarchicalMemory, LongTermMemory,
    SelfEvolution, SensoryMemory, ShortTermMemory,
};
use crate::persistence::config::Config;
use crate::router::RouteDecision;
use crate::tui::workbench::RealEngine;
use crate::types::Message;

/// 一次已接受发送的登记（幂等重放用）
#[derive(Clone)]
struct RecentSend {
    session_key: String,
    task_id: String,
}

/// 跨连接共享的会话运行时台账
struct ChatRuntimes {
    /// sessionKey → 正在运行的后台任务句柄（chat.abort 据此中止）
    tasks: Mutex<HashMap<String, JoinHandle<()>>>,
    /// clientRequestId → 已接受的发送（bounded，防无界增长）
    recent: Mutex<HashMap<String, RecentSend>>,
}

impl ChatRuntimes {
    fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            recent: Mutex::new(HashMap::new()),
        }
    }

    fn cap_recent(&self) {
        const MAX: usize = 512;
        let mut guard = match self.recent.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        if guard.len() > MAX {
            let drop_keys: Vec<String> = guard.keys().take(guard.len() - MAX).cloned().collect();
            for k in drop_keys {
                guard.remove(&k);
            }
        }
    }
}

fn runtimes() -> &'static ChatRuntimes {
    static R: OnceLock<ChatRuntimes> = OnceLock::new();
    // 网关托管时初始化用量台账（真实 LLM 用量落盘 ~/.marguerite/usage.db）
    crate::usage::init();
    R.get_or_init(ChatRuntimes::new)
}

// ── 会话级锚定台账（DSH 融合 v2 §5 接线收尾）──────────────────────────────
// 梁神语义：锚定状态属于**会话**（晋升一次、之后保持），而非每轮重建。
// 编排器按轮新建（dispatch 一次 = 一轮），因此晋升态在此跨轮持久：
// 每轮开始时取出注入编排器，本轮结束后回收推进后的状态。

/// 会话锚定台账（sessionKey → 锚定态；bounded，防无界增长）
struct SessionAnchors {
    map: Mutex<HashMap<String, crate::mode::anchoring::AnchorState>>,
}

impl SessionAnchors {
    fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
        }
    }

    fn load(&self, session: &str) -> Option<crate::mode::anchoring::AnchorState> {
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(session)
            .cloned()
    }

    fn save(&self, session: &str, anchor: crate::mode::anchoring::AnchorState) {
        const MAX: usize = 1024;
        let mut guard = self
            .map
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if guard.len() >= MAX && !guard.contains_key(session) {
            guard.clear();
        }
        guard.insert(session.to_string(), anchor);
    }
}

fn session_anchors() -> &'static SessionAnchors {
    static A: OnceLock<SessionAnchors> = OnceLock::new();
    A.get_or_init(SessionAnchors::new)
}

/// 网关级凭证台账（凭证缝接入，进程级缓存）：凭证台账优先解析 API Key，
/// config/env 直读回退。进程内只加载一次（网关是只读消费方）。
fn gateway_credentials() -> Arc<crate::credentials::CredentialsStore> {
    static S: OnceLock<Arc<crate::credentials::CredentialsStore>> = OnceLock::new();
    S.get_or_init(|| Arc::new(crate::credentials::CredentialsStore::open_default()))
        .clone()
}

/// 网关级提醒台账（DSH schedule v7，进程级缓存）；打开失败退内存模式
/// （离线可运行：绝不因提醒台账不可写而阻断对话）。
fn schedule_store() -> Arc<Mutex<crate::schedule::ScheduleStore>> {
    static S: OnceLock<Arc<Mutex<crate::schedule::ScheduleStore>>> = OnceLock::new();
    S.get_or_init(|| {
        let store = crate::schedule::ScheduleStore::open_default().unwrap_or_else(|e| {
            eprintln!("[schedule] 台账打开失败，退内存模式: {e}");
            crate::schedule::ScheduleStore::open(None).expect("内存提醒库必可用")
        });
        Arc::new(Mutex::new(store))
    })
    .clone()
}

/// DSH schedule 会话级提醒（v7）：每轮对话结束后触发到期提醒——
/// `schedule.due` Status 事件（前端可渲染），after/at 一次性置 fired、
/// every 由台账自动锚点重挂载（不补跑）。
fn emit_due_schedule_reminders(session: &str, bus: &EventBus) {
    let due = match schedule_store().lock() {
        Ok(mut store) => store.due(session, chrono::Utc::now().timestamp_millis()),
        Err(_) => return,
    };
    for r in due {
        bus.publish(Envelope::new(
            EventType::Status,
            json!({
                "kind": "schedule.due",
                "reminder": {
                    "id": r.id,
                    "message": r.message,
                    "trigger": r.trigger.kind(),
                }
            }),
            session,
        ));
    }
}

/// 网关级会话投影库（DSH 融合 v8 消费方接线）：`<data_dir>/session-projections.db`；
/// 打开失败退内存（离线可运行）。
fn gateway_projection_store() -> Arc<Mutex<crate::projection::ProjectionStore>> {
    static P: OnceLock<Arc<Mutex<crate::projection::ProjectionStore>>> = OnceLock::new();
    P.get_or_init(|| {
        let store = crate::projection::ProjectionStore::open(Some(
            crate::persistence::config::Config::data_dir().join("session-projections.db"),
        ))
        .unwrap_or_else(|e| {
            eprintln!("[projection] 会话投影库打开失败，退内存: {e}");
            crate::projection::ProjectionStore::open(None).expect("内存投影库必可用")
        });
        Arc::new(Mutex::new(store))
    })
    .clone()
}

/// 折叠一轮事件进会话投影并渲染快照载荷（纯函数，可离线单测）。
///
/// 投影单元：`turnCount`（user/assistant 整轮计数，带视图）与 `hostAudit`
/// （host-only 事件计数，无视图）——projection fold 契约的真实消费方。
fn build_projection_snapshot(
    store: &mut crate::projection::ProjectionStore,
    session: &str,
    turn_events: &[serde_json::Value],
) -> Option<serde_json::Value> {
    let turns = store
        .project(&crate::projection::TurnCounterUnit, session, turn_events, 0)
        .ok()?;
    let audit = store
        .project(&crate::projection::HostAuditUnit, session, turn_events, 0)
        .ok()?;
    Some(json!({
        "kind": "projection.snapshot",
        "projections": {
            "turns": turns,
            "hostAudit": audit,
        }
    }))
}

/// 每轮对话结束后：把本轮事件折叠进会话投影库，并发布 `projection.snapshot`
/// 事件（前端可渲染会话级投影视图——对齐上游 client 侧 surface-projection 消费）。
fn emit_projection_snapshot(session: &str, bus: &EventBus) {
    let turn_events = vec![
        serde_json::json!({ "type": "user/message" }),
        serde_json::json!({ "type": "assistant/message" }),
    ];
    let snapshot = match gateway_projection_store().lock() {
        Ok(mut store) => build_projection_snapshot(&mut store, session, &turn_events),
        Err(_) => None,
    };
    if let Some(payload) = snapshot {
        bus.publish(Envelope::new(EventType::Status, payload, session));
    }
}

/// `chat.send`：接受一轮 Bloom 对话（后台执行，事件经总线回流）
pub async fn send(ctx: &GatewayContext, params: &Value) -> Result<Value, RpcError> {
    let message = params
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| RpcError::new("INVALID_REQUEST", "chat.send 的 message 不可为空"))?
        .to_string();
    let session_key = params
        .get("sessionKey")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("ws.{}", Uuid::new_v4()));
    let client_request_id = params
        .get("clientRequestId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let client_message_id = params
        .get("clientMessageId")
        .and_then(Value::as_str)
        .map(str::to_string);

    let reg = runtimes();
    // 幂等重放：同一 clientRequestId 返回先前已接受的 turn（不重复执行）
    if let Some(crid) = &client_request_id {
        if let Some(rec) = reg
            .recent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(crid)
            .cloned()
        {
            return Ok(send_response(
                &rec.session_key,
                &rec.task_id,
                client_message_id.as_deref(),
                true,
            ));
        }
    }

    let task_id = format!("turn-{}", Uuid::new_v4());
    // 先登记幂等键再 spawn，防丢响应重试（同 crid 并发）重复执行
    if let Some(crid) = &client_request_id {
        reg.recent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(crid.clone())
            .or_insert(RecentSend {
                session_key: session_key.clone(),
                task_id: task_id.clone(),
            });
        reg.cap_recent();
    }
    // 上下文台账：用户输入无论引擎成败都属于会话上下文（引擎失败也会被记录）
    // F1：先清上一轮可能残留的流式 partial_text，避免跨轮串扰
    crate::context::session::clear_partial(&session_key);
    crate::context::session::record_turn(&session_key, "user", &message);
    // B3 行为捕获：用户消息观察（observation）
    crate::memory::behavior::record_behavior(&session_key, "user_message", &message);
    // 项目工作区绑定：chat.send 携带 workspaceId 时，把会话关联到项目根目录
    // （persist_session 落盘 project_path，后续可切换工具根目录）
    if let Some(wid) = params.get("workspaceId").and_then(Value::as_str) {
        if let Some(path) = crate::gateway::workspaces::workspace_path_for(wid) {
            session_workspaces()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(session_key.clone(), path.clone());
            // 台账持久化归属：进程内绑定 + 台账 JSON 双写，重启后仍能恢复项目归属
            crate::context::session::bind_session_project(&session_key, &path);
        }
    }
    // 立即落盘：用户消息已记入台账，先在 sessions.db 占位（整轮完成后再覆盖为
    // 完整快照）。防止本轮中途失败/应用关闭导致「有对话但会话列表查不到」。
    persist_session(&session_key);

    let bus = ctx.bus.clone();
    let config = ctx.config.clone();
    let model_name = ctx.config.model.model_name.clone();
    let sk = session_key.clone();
    let tid = task_id.clone();
    // Track A 路由决策（纯规则，零 LLM 开销）：供富事件 router_decision 携带思维模式/策略。
    let route_decision = crate::router::classify(&crate::router::signals::classify_input_from_turn(
        &message,
        &session_key,
        &config,
    ));
    let handle = tokio::spawn(async move {
        // 用量归会话：task-local 在 turn 内生效，跨 await 安全，并行会话互不串扰
        let sk_usage = sk.clone();
        let sk_inner = sk.clone();
        crate::usage::SESSION
            .scope(sk_usage, async move {
                match run_turn(config, message, sk_inner.clone(), tid.clone(), bus.clone()).await {
                    Ok(outcome) => {
                        // 融合路由决策：发布 router_decision（对齐 Web UI `session.event.router_decision`）
                        bus.publish(Envelope::new(
                            EventType::RouterDecision,
                            router_decision_payload(
                                &outcome,
                                &model_name,
                                &sk_inner,
                                &tid,
                                &route_decision,
                            ),
                            sk_inner.as_str(),
                        ));
                        // 富终端收据（Web UI 据此渲染最终答案并结束本轮）
                        bus.publish(Envelope::new(
                            EventType::Done,
                            done_payload(&outcome.lead_reply, "completed", &sk_inner, &tid),
                            sk_inner.as_str(),
                        ));
                        // 运行态变更：turn 结束（正常完成）→ 前端把该会话标记回 idle。
                        bus.publish(Envelope::sessions_changed(&sk_inner, "idle", "turn_end"));
                    }
                    Err(e) => {
                        // Orchestrator 失败即发布 Error 事件（Web UI 依 `.error` 终止本轮）
                        bus.publish(Envelope::error(
                            sk_inner.as_str(),
                            "orchestrator",
                            &format!("{e}"),
                        ));
                        // 运行态变更：turn 结束（失败）→ 前端把该会话标记回 error/idle。
                        bus.publish(Envelope::sessions_changed(&sk_inner, "error", "turn_end"));
                    }
                }
            })
            .await;
        runtimes()
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&sk);
    });
    runtimes()
        .tasks
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(session_key.clone(), handle);

    // 运行态变更：发布 sessions.changed（turn 开始），前端据此把该会话标记为 running
    // 并重算 lastActivityAt / idle（前端 useSessionListSubscription 订阅此事件刷新列表）。
    ctx.bus.publish(Envelope::sessions_changed(
        &session_key,
        "running",
        "turn_start",
    ));

    Ok(send_response(
        &session_key,
        &task_id,
        client_message_id.as_deref(),
        false,
    ))
}

/// 返回当前正在运行的会话键集合（供 sessions.list 标记 runStatus）
pub fn running_sessions() -> Vec<String> {
    runtimes()
        .tasks
        .lock()
        .map(|g| g.keys().cloned().collect())
        .unwrap_or_default()
}

/// 判断指定会话是否正在运行
pub fn is_session_running(session_key: &str) -> bool {
    runtimes()
        .tasks
        .lock()
        .map(|g| g.contains_key(session_key))
        .unwrap_or(false)
}

/// `chat.abort`：中止正在运行的本轮并发布 `reason=aborted` 收据（幂等）
pub async fn abort(ctx: &GatewayContext, params: &Value) -> Result<Value, RpcError> {
    let session_key = params
        .get("sessionKey")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| RpcError::new("INVALID_REQUEST", "chat.abort 的 sessionKey 不可为空"))?;

    let removed = runtimes()
        .tasks
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(session_key);
    if let Some(handle) = removed {
        handle.abort();
    }
    // 无论是否存在运行中任务，都发布中止收据（Web UI 据此结束流并回填 composer）
    ctx.bus.publish(Envelope::new(
        EventType::Done,
        json!({
            "reason": "aborted",
            "session_key": session_key,
            "key": session_key,
            "text_snapshot": "",
        }),
        session_key,
    ));
    // B3 行为捕获：中止观察（observation）
    crate::memory::behavior::record_behavior(session_key, "aborted", "chat.abort");
    Ok(json!({ "accepted": true, "sessionKey": session_key }))
}

/// 构建自进化经验台账（与 GUI 同款接线；初始化失败返回 None，不阻塞对话）
///
/// 进程级单例：SQLite 连接可安全复用（每条语句都能看到其他连接的最新提交），
/// 避免每轮对话重复 open + CREATE TABLE/INDEX DDL（2 个库 × 每轮）。
fn build_evolution() -> Option<Arc<Mutex<SelfEvolution>>> {
    static EVO: OnceLock<Option<Arc<Mutex<SelfEvolution>>>> = OnceLock::new();
    EVO.get_or_init(|| {
        let data_dir = Config::data_dir();
        let _ = std::fs::create_dir_all(&data_dir);
        let mut ledger = ExperienceLedger::new(&data_dir.join("marguerite_experience.db"));
        if ledger.init().is_err() {
            return None;
        }
        let mut long_term = LongTermMemory::new(&data_dir.join("marguerite_ltm_gw.db"));
        if long_term.init().is_err() {
            return None;
        }
        Some(Arc::new(Mutex::new(SelfEvolution::new(ledger, long_term))))
    })
    .clone()
}

/// 分层记忆栈（感知 → 短期 FTS → 长期语义）进程单例。
///
/// 感知/短期/长期三层此前只有定义与单测、从未被对话运行时实例化（孤岛代码）。
/// 本单例让 `persist_session` 每次沉淀时真实写入三层：感知记录消息、短期同步
/// FTS 跨会话检索、长期语义向量提升；沉淀后清空感知层避免重复写入。
fn memory_stack() -> Option<Arc<Mutex<HierarchicalMemory>>> {
    static STACK: OnceLock<Option<Arc<Mutex<HierarchicalMemory>>>> = OnceLock::new();
    STACK
        .get_or_init(|| {
            let data_dir = Config::data_dir();
            let _ = std::fs::create_dir_all(&data_dir);
            let Ok(sensory) =
                SensoryMemory::with_db("gateway", &data_dir.join("marguerite_sensory.db"))
            else {
                return None;
            };
            let mut short_term = ShortTermMemory::new(&data_dir.join("marguerite_short_term.db"));
            if short_term.init().is_err() {
                return None;
            }
            let mut long_term = LongTermMemory::new(&data_dir.join("marguerite_ltm_gw.db"));
            if long_term.init().is_err() {
                return None;
            }
            Some(Arc::new(Mutex::new(HierarchicalMemory::new(
                long_term, short_term, sensory,
            ))))
        })
        .clone()
}

/// 沉淀本会话上下文到三层记忆（best-effort，任何失败仅记录日志）。
///
/// 感知层累积当前对话消息 → 长期语义提升 + 短期 FTS 同步 → 清空感知防重复。
fn consolidate_runtime_memory(session: &str) {
    let Some(stack) = memory_stack() else {
        return;
    };
    let mut guard = match stack.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    let all: Vec<Message> = guard
        .sensory
        .get_all_messages()
        .into_iter()
        .cloned()
        .collect();
    if all.is_empty() {
        return;
    }
    let summary = all
        .last()
        .map(|m| match m {
            Message::User { content, .. }
            | Message::Assistant { content, .. }
            | Message::System { content, .. } => content.clone(),
            Message::Tool { name, output, .. } => format!("[{name}] {output}"),
        })
        .unwrap_or_default();
    // 用独立承载实例喂给长期记忆，避免在同一 guard 上同时可变借用 long_term
    // 与不可变借用 sensory。
    let mut scratch = SensoryMemory::new("scratch");
    for m in &all {
        let _ = scratch.add_message(m.clone());
    }
    if let Err(e) = guard.long_term.consolidate(&scratch) {
        tracing::warn!(err = %e, session = %session, "长期记忆沉淀失败");
    }
    if let Err(e) = guard.short_term.store_session(session, &summary, &all) {
        tracing::warn!(err = %e, session = %session, "短期记忆同步失败");
    }
    let _ = guard.sensory.clear();
}

/// 跨项目记忆自动注入：检索本机各项目记忆，产出系统提示片段（无命中/无记忆库返回空串）。
fn cross_project_hint(text: &str) -> String {
    crate::memory::cross_project::cross_project_recall_hint(text, 4)
}

/// 智能体人设/技能注入：会话为 `agent:{agentId}:...` 时，取该智能体的
/// system_prompt + 技能组合 + 工具清单构造系统提示片段（非智能体会话返回空串）。
/// 这是「自定义性格与技能组合智能体可与之对话」的真实落地：对话时其性格生效。
///
/// DSH persona 模板解析（v6）：`{{model}}` / `{{cwd}}` 在注入时替换为实际值，
/// 未知变量替换为空串（对齐上游 persona `{{model}}`/`{{cwd}}` 语义）。
fn agent_personality_hint(session: &str, model: &str, cwd: &str) -> String {
    let Some(id) = crate::agent::store::agent_id_from_session(session) else {
        return String::new();
    };
    let raw = crate::agent::store::personality_for(id).unwrap_or_default();
    if raw.is_empty() {
        return raw;
    }
    let mut vars = std::collections::BTreeMap::new();
    vars.insert("model".to_string(), model.to_string());
    vars.insert("cwd".to_string(), cwd.to_string());
    crate::persona::resolve_template(&raw, &vars)
}

/// 合并跨项目记忆 + 智能体人设为一个注入片段（各段非空才拼接）。
fn merged_turn_hint(text: &str, session: &str, model: &str, cwd: &str) -> String {
    let cp = cross_project_hint(text);
    let agent = agent_personality_hint(session, model, cwd);
    match (cp.is_empty(), agent.is_empty()) {
        (false, false) => format!("{cp}\n{agent}"),
        (false, true) => cp,
        (true, false) => agent,
        (true, true) => String::new(),
    }
}

/// 跨会话记忆召回：合并长期语义记忆 + 短期 FTS 记忆 + 跨项目记忆为单一稳定前缀
/// （`Message::System`），注入 lead 会话上下文最前端（P-7 stable 半区），使智能体
/// 跨会话 / 跨项目复用既有经验（`long_term`/`short_term` 此前仅沉淀、从未被召回）。
/// 无命中返回 `None`（不阻塞对话）。
///
/// 各层 top-K（按相关性 / 最近更新）截断，避免 token 膨胀。
///
/// 异步化 + `spawn_blocking`：四路召回含 SQLite 查询与磁盘 JSON 读取，原实现
/// 直接阻塞 tokio worker（与「禁止同步 TcpStream 阻塞 worker」同类的坑）；
/// 现在整体挪到阻塞池，内部再以 `thread::scope` 四路并行，结果按固定顺序拼装
/// 保证前缀内容确定（稳定前缀 = KV-cache 键的一部分，顺序必须确定）。
async fn memory_stable_prefix(text: &str) -> Option<Message> {
    let text = text.to_string();
    let parts = tokio::task::spawn_blocking(move || {
        let (cp, stack, recall, exp) = std::thread::scope(|s| {
            // 1) 跨项目记忆（已渲染为 Observation 列表的 System 消息）
            let h_cp = s.spawn(|| {
                let mut parts = Vec::new();
                if let Some(Message::System { content, .. }) = recall_stable_prefix(&text, 4) {
                    if !content.is_empty() {
                        parts.push(content);
                    }
                }
                parts
            });
            // 2) 长期语义记忆（向量相似度 top-K）+ 3) 短期 FTS 记忆（跨会话全文检索 top-K），
            // 复用对话运行时分层记忆单例（与沉淀同一库，读取不阻塞写入）。
            let h_stack = s.spawn(|| {
                let mut parts = Vec::new();
                if let Some(stack) = memory_stack() {
                    let guard = match stack.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    if let Ok(hits) = guard.long_term.search(&text, 4) {
                        for h in hits {
                            parts.push(format!("[长期记忆] {}", h.content));
                        }
                    }
                    if let Ok(summaries) = guard.short_term.search(&text, 4) {
                        for s in summaries {
                            parts.push(format!("[短期记忆·{}] {}", s.session_id, s.summary));
                        }
                    }
                }
                parts
            });
            // 4) 召回记忆（claude-mem 四类持久记忆：user / project / feedback / auto）
            //    带 TTL 的进程缓存：避免每轮对话重复读盘 + JSON 解析，同时保证新导入的
            //    记忆最多延迟 RECALL_CACHE_TTL 即可被召回（导入写盘走的是独立实例）。
            let h_recall = s.spawn(|| {
                use crate::memory::recall::MemoryBackend as _;
                let mut parts = Vec::new();
                let mem_dir = crate::persistence::config::Config::data_dir().join("memory");
                let kb = cached_recall_kb(&mem_dir);
                for category in crate::memory::recall::MemoryCategory::all() {
                    for h in kb.recall(*category, &text, 3) {
                        if !h.snippet.is_empty() {
                            parts.push(format!("[召回记忆·{}] {}", h.category.as_str(), h.snippet));
                        }
                    }
                }
                parts
            });
            // 5) 经验记忆（自进化台账：踩坑/决策/模式，余弦相似度 top-K）
            //    SQLite 连接进程级复用：语句级读事务天然可见其他连接的最新提交，
            //    无需每轮重新 open + DDL。
            let h_exp = s.spawn(|| {
                let mut parts = Vec::new();
                let mem_dir = crate::persistence::config::Config::data_dir().join("memory");
                let exp_dir = mem_dir.join("experience.db");
                let ledger = cached_experience_ledger(&exp_dir);
                if let Ok(exps) = ledger
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .search(&text, 3)
                {
                    for e in exps {
                        let body: String = e.body.chars().take(200).collect();
                        parts.push(format!("[经验记忆·{}] {}：{}", e.kind.as_str(), e.title, body));
                    }
                }
                parts
            });
            (
                h_cp.join().unwrap_or_default(),
                h_stack.join().unwrap_or_default(),
                h_recall.join().unwrap_or_default(),
                h_exp.join().unwrap_or_default(),
            )
        });
        let mut parts: Vec<String> =
            Vec::with_capacity(cp.len() + stack.len() + recall.len() + exp.len());
        parts.extend(cp);
        parts.extend(stack);
        parts.extend(recall);
        parts.extend(exp);
        parts
    })
    .await
    .unwrap_or_default();

    // 注：sensory（感知记忆）即当前会话上下文，已由 session_history_messages 注入历史，
    // 无需重复；behavior（行为记忆）尚无面向注入的检索 API，待补后一并纳入。

    if parts.is_empty() {
        return None;
    }
    Some(Message::System {
        content: parts.join("\n\n"),
        timestamp: chrono::Utc::now(),
    })
}

/// 召回记忆 KB 的进程缓存 TTL：命中窗口内免读盘，超窗后重载保证新鲜度。
const RECALL_CACHE_TTL: Duration = Duration::from_secs(5);

/// 召回记忆 KB 进程缓存（TTL 刷新；`KbBackedMemory` 构造时会读盘 + JSON 解析）。
fn cached_recall_kb(dir: &std::path::Path) -> Arc<crate::memory::recall::KbBackedMemory> {
    static CACHE: OnceLock<
        Mutex<Option<(Instant, Arc<crate::memory::recall::KbBackedMemory>)>>,
    > = OnceLock::new();
    let cell = CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_ref() {
        Some((t, kb)) if t.elapsed() < RECALL_CACHE_TTL => kb.clone(),
        _ => {
            let kb = Arc::new(crate::memory::recall::KbBackedMemory::with_persistence(dir));
            *guard = Some((Instant::now(), kb.clone()));
            kb
        }
    }
}

/// 经验台账连接进程级单例（按路径缓存；未初始化成功时不缓存，降级为空台账）。
type ExperienceLedgerMap = Mutex<
    HashMap<
        std::path::PathBuf,
        Arc<Mutex<crate::memory::experience::ExperienceLedger>>,
    >,
>;

fn cached_experience_ledger(
    path: &std::path::Path,
) -> Arc<Mutex<crate::memory::experience::ExperienceLedger>> {
    static LEDGERS: OnceLock<ExperienceLedgerMap> = OnceLock::new();
    let map = LEDGERS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(arc) = guard.get(path) {
        return arc.clone();
    }
    let mut ledger = crate::memory::experience::ExperienceLedger::new(path);
    if ledger.init().is_err() {
        // 不缓存失败实例：search 返回 Err，上层静默降级（与旧行为一致）
        return Arc::new(Mutex::new(ledger));
    }
    let arc = Arc::new(Mutex::new(ledger));
    guard.insert(path.to_path_buf(), arc.clone());
    arc
}

/// 从上下文台账取本会话历史消息（排除当前用户输入，最多最近 20 轮），
/// 转换为 Message 向量供 orchestrator 灌入 lead 会话上下文。
pub(crate) fn session_history_messages(
    session: &str,
    current_text: &str,
) -> Vec<crate::types::Message> {
    let entries = crate::context::session::context_entries(session);
    let now = chrono::Utc::now();
    entries
        .iter()
        .rev()
        .take(60) // 最多 60 条（30 轮）
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .filter_map(|e| {
            // 跳过当前这条用户消息（dispatch 会自己加）
            if e.role == "user" && e.text == current_text {
                return None;
            }
            Some(match e.role.as_str() {
                "user" => crate::types::Message::User {
                    content: e.text.clone(),
                    timestamp: now,
                },
                "summary" => crate::types::Message::Assistant {
                    content: format!("【历史摘要】{}", e.text),
                    timestamp: now,
                    thinking: None,
                },
                _ => crate::types::Message::Assistant {
                    content: e.text.clone(),
                    timestamp: now,
                    thinking: None,
                },
            })
        })
        .collect()
}

/// 会话 → 项目根目录（workspaceId 绑定，进程内生命周期；重启后由 workspace 注册表恢复）
fn session_workspaces() -> &'static Mutex<HashMap<String, String>> {
    static M: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 解析会话关联的项目根目录（跨重启粘住）。
///
/// 优先级：进程内 workspaceId 绑定（chat.send 携带）> 台账 JSON（bind_session_project
/// 持久化，重启后不丢）> DB 已有 `project_path`。
/// 进程内绑定是「最近一次 chat.send 声明」的最权威来源；台账/DB 回退保证应用重启后，
/// 即便前端续聊未再带 `workspaceId`，会话仍能恢复项目归属而不会掉进「最近」栏。
/// 查询失败/无绑定静默回退 None，不阻塞对话。
fn resolve_session_project(session: &str) -> Option<String> {
    // 1) 进程内绑定（最权威，来自 chat.send 的 workspaceId）
    if let Ok(g) = session_workspaces().lock() {
        if let Some(p) = g.get(session) {
            return Some(p.clone());
        }
    }
    // 2) 台账持久化绑定（重启后仍可恢复，无需等到 DB 回退）
    if let Some(p) = crate::context::session::session_project(session) {
        return Some(p);
    }
    // 3) DB 回退：会话曾绑定过项目，重启后仍能恢复归属
    //    连接进程级复用（与 persist_session 共用同一实例），避免每轮 open + DDL。
    let mgr = session_db();
    mgr.lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_session(session)
        .ok()
        .flatten()
        .and_then(|s| s.project_path)
}

/// sessions.db 连接进程级单例（chat 路径读写共用；SQLite 语句级可见性保证
/// 其他连接（session 管理命令）的提交也能被读到）。
fn session_db() -> &'static Mutex<crate::persistence::session::SessionManager> {
    static MGR: OnceLock<Mutex<crate::persistence::session::SessionManager>> = OnceLock::new();
    MGR.get_or_init(|| {
        let db_path = crate::persistence::config::Config::config_dir().join("sessions.db");
        let mut manager = crate::persistence::session::SessionManager::new(&db_path);
        if manager.init().is_err() {
            tracing::warn!("sessions.db 初始化失败，后续读写将返回 Err（忽略）");
        }
        Mutex::new(manager)
    })
}

/// 把会话上下文台账落盘到 SQLite（sessions.db），供 `sessions.list` / `chat.history` 恢复。
/// best-effort：任何失败仅记录日志，不阻塞对话。
pub(crate) fn persist_session(session: &str) {
    let entries = crate::context::session::context_entries(session);
    if entries.is_empty() {
        return;
    }
    let now = chrono::Utc::now();
    let messages: Vec<crate::types::Message> = entries
        .iter()
        .map(|e| match e.role.as_str() {
            "user" => crate::types::Message::User {
                content: e.text.clone(),
                timestamp: now,
            },
            "summary" => crate::types::Message::Assistant {
                content: format!("【历史摘要】{}", e.text),
                timestamp: now,
                thinking: None,
            },
            _ => crate::types::Message::Assistant {
                content: e.text.clone(),
                timestamp: now,
                thinking: None,
            },
        })
        .collect();
    // 分层记忆沉淀：感知层先接收本会话消息，再由长期/短期提升（best-effort）。
    // 与 SQLite 落盘同源，任何记忆失败不影响会话保存。
    if let Some(stack) = memory_stack() {
        if let Ok(mut guard) = stack.lock() {
            for m in &messages {
                let _ = guard.sensory.add_message(m.clone());
            }
        }
        consolidate_runtime_memory(session);
    }
    // 先解析项目归属（其内部锁用完即放），再取写库单例：
    // std Mutex 不可重入，顺序颠倒会自锁（persist_session → resolve 同抢 session_db）。
    let project = resolve_session_project(session);
    let manager = session_db();
    let manager = manager.lock().unwrap_or_else(|e| e.into_inner());
    let result = match project {
        Some(p) => manager.save_snapshot_for_project(session, &p, "idle", &messages, 0),
        None => manager.save_snapshot(session, "idle", &messages, 0),
    };
    if let Err(e) = result {
        tracing::warn!(err = %e, session = %session, "会话落盘失败（忽略，不影响本轮）");
    }
}

/// 运行一轮：引擎在私总线上执行，事件经桥回流网关总线
///
/// 桥只订阅 `{session}.*`（子代理按 agent_id 分 session，天然隔离），
/// 跳过 `Done`（由调用方统一发富收据），转发前注入 `session_key` / `task_id`。
async fn run_turn(
    config: Config,
    text: String,
    session: String,
    task_id: String,
    bus: Arc<EventBus>,
) -> anyhow::Result<crate::orchestration::OrchestrationOutcome> {
    let private_bus = Arc::new(EventBus::default());
    let mut rx = private_bus.subscribe(&format!("{session}.*"));
    let gw_bus = bus.clone();
    let gw_session = session.clone();
    let gw_task = task_id.clone();
    tokio::spawn(async move {
        while let Some(env) = rx.recv().await {
            if env.r#type == EventType::Done {
                continue; // 中间完成事件不回流，终端收据由调用方统一发布
            }
            // B3 行为捕获：工具调用/结果观察（仅记录，不改变转发）
            if env.r#type == EventType::ToolCall {
                let tool = env
                    .payload
                    .get("tool")
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                let args = env
                    .payload
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str())
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_default();
                crate::memory::behavior::record_behavior(
                    &gw_session,
                    "tool_call",
                    &format!("{tool}({args})"),
                );
            } else if env.r#type == EventType::ToolResult {
                let tool = env
                    .payload
                    .get("tool")
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                let ok = env
                    .payload
                    .get("ok")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let summary = env
                    .payload
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                crate::memory::behavior::record_behavior(
                    &gw_session,
                    "tool_result",
                    &format!("{tool} -> {}", if ok { summary } else { "FAILED" }),
                );
            }
            gw_bus.publish(with_identity(env, &gw_session, &gw_task));
        }
    });

    // ── Track A 规则版任务路由（思维模式 + persona 档 + 决策日志）──
    // 纯规则、零 LLM 开销；先于自动拆解 / Bloom / 简单路径计算，决策流向后续各路径。
    let route_input = crate::router::signals::classify_input_from_turn(&text, &session, &config);
    let route_decision = crate::router::classify(&route_input);
    crate::router::decision_log::DecisionLog::record(
        &route_decision,
        &session,
        &task_id,
        &config.model.model_name,
    );

    // DSH persona 模板变量（v6）：{{model}}/{{cwd}} 注入值
    let persona_model = config.model.model_name.clone();
    let persona_cwd = resolve_session_project(&session).unwrap_or_default();

    // ── 复杂任务自动拆解（Bloom 长续航）：检测到复杂任务 → 计划驱动长执行 ──
    // 简单/中等任务保持原单轮路径（行为完全向后兼容）。
    if crate::plan::runner::should_auto_decompose(&config, &text) {
        tracing::info!(session = %session, "复杂任务：进入计划驱动长执行（自动拆解 + 分步执行）");
        let outcome = crate::plan::runner::run_autonomous(
            config.clone(),
            text.clone(),
            session.clone(),
            private_bus.clone(),
            merged_turn_hint(&text, &session, &persona_model, &persona_cwd),
        )
        .await?;
        // 上下文台账：记录 assistant 回复（user 输入已在 send 接受时记账）
        crate::context::session::record_turn(&session, "assistant", &outcome.lead_reply);
        crate::memory::behavior::record_behavior(&session, "assistant_reply", &outcome.lead_reply);
        if crate::context::session::context_status(&session)["alert"] == "compact_required" {
            crate::context::session::context_compact(&session, Some(&bus));
        }
        emit_due_schedule_reminders(&session, &bus);
        emit_projection_snapshot(&session, &bus);
        persist_session(&session);
        return Ok(outcome);
    }

    // ── Bloom 并行编排（配置开启或运行时模式为 bloom 时）：Captain + Copilot 双模型并行推理 ──
    // 推理层优化，不改沙箱/工具权限；RouterFxStrip 显示 DualLeader 形态。
    // 运行时模式（`mode.switch`）优先：前端切到 Bloom 无需重启即刻走并行路径。
    let bloom_runtime = crate::mode::runtime_mode() == crate::mode::ModeKind::Bloom;
    if config.bloom.parallel_enabled || bloom_runtime {
        tracing::info!(session = %session, "Bloom 并行编排：Captain + Copilot 并行");
        let mut orch = crate::orchestration::Orchestrator::new_with_credentials(
            config.clone(),
            Some(gateway_credentials()),
        );
        orch.set_event_bus(private_bus.clone(), session.clone());
        // 置为 Bloom 模式后走统一的 dispatch 入口：并行由模式路由保证，
        // 而非调用方各自绕开 dispatch 直呼内部方法。
        orch.set_mode(crate::mode::ModeKind::Bloom);
        // 会话级锚定态（跨轮持久）：取出注入编排器，本轮结束后回收推进后的状态；
        // 消息源 = user（goal 自动轮接入后按其来源标记，锚定白名单据此过滤）
        let session_anchor = session_anchors()
            .load(&session)
            .unwrap_or_default();
        orch.set_anchor(Some(session_anchor.clone()));
        orch.set_message_source("user");
        // Track A：路由决策（思考预算 + prompt 策略）注入 lead 运行时
        orch.set_route_decision(route_decision.clone());
        // 跨会话记忆召回（W-6）：长期 + 短期 + 跨项目记忆合并为稳定前缀 System 消息，
        // 落进 lead 上下文最前端 + 智能体人设（经验提示）拆分注入，避免记忆被重复塞入。
        if let Some(prefix) = memory_stable_prefix(&text).await {
            orch.set_stable_prefix(prefix);
        }
        // 智能体人设（agent-instructions，deferred 延迟注入）：锚定期与晋升后
        // grace 步内扣住，越过才注入（对齐梁神 deferredSources + deferredGraceSteps）
        let persona = agent_personality_hint(&session, &persona_model, &persona_cwd);
        if !persona.is_empty() && session_anchor.deferred_allows("agent-instructions") {
            orch.set_experience_hint(persona);
        }
        // 重启后上下文恢复：把本会话历史消息灌入 lead，避免每轮从零开始
        orch.load_history(session_history_messages(&session, &text));
        let outcome = orch.dispatch(&text).await?;
        // 回收本轮推进后的锚定态，跨轮持久（晋升一次后不再重锚）
        if let Some(advanced) = orch.take_anchor() {
            session_anchors().save(&session, advanced);
        }
        // 上下文台账：记录 assistant 回复并自动压缩
        crate::context::session::record_turn(&session, "assistant", &outcome.lead_reply);
        crate::memory::behavior::record_behavior(&session, "assistant_reply", &outcome.lead_reply);
        if crate::context::session::context_status(&session)["alert"] == "compact_required" {
            crate::context::session::context_compact(&session, Some(&bus));
        }
        emit_due_schedule_reminders(&session, &bus);
        emit_projection_snapshot(&session, &bus);
        persist_session(&session);
        return Ok(outcome);
    }

    let engine = RealEngine::from_config(config)
        .with_evolution(build_evolution())
        .with_cross_project_hint(agent_personality_hint(&session, &persona_model, &persona_cwd))
        .with_stable_prefix(memory_stable_prefix(&text).await)
        .with_workspace(resolve_session_project(&session).map(std::path::PathBuf::from))
        .with_route_decision(Some(route_decision.clone()))
        .with_credentials(Some(gateway_credentials()));
    let outcome = engine
        .run_turn_reply(text.clone(), session.clone(), private_bus)
        .await?;

    // ── 上下文台账：记录 assistant 回复并自动压缩（user 输入已在 send 接受时记账）──
    crate::context::session::record_turn(&session, "assistant", &outcome.lead_reply);
    // B3 行为捕获：assistant 回复观察（observation）
    crate::memory::behavior::record_behavior(&session, "assistant_reply", &outcome.lead_reply);
    if crate::context::session::context_status(&session)["alert"] == "compact_required" {
        crate::context::session::context_compact(&session, Some(&bus));
    }
    emit_due_schedule_reminders(&session, &bus);
    emit_projection_snapshot(&session, &bus);
    persist_session(&session);

    Ok(outcome)
}

/// 融合路由决策负载（对齐 Web UI `RouterDecisionPayload`：strategy/model/routing_applied/decision）
///
/// 扩展：携带 Track A 路由决策的思维模式 / prompt 策略 / persona 档 / 置信度 / 来源
/// （复用既有 `EventType::RouterDecision`，不新增事件类型）。
fn router_decision_payload(
    outcome: &crate::orchestration::OrchestrationOutcome,
    model: &str,
    session: &str,
    task_id: &str,
    route_decision: &RouteDecision,
) -> Value {
    let analysis = &outcome.analysis;
    json!({
        "strategy": outcome.strategy.label(),
        "model": model,
        "routed_model": model,
        "source": "orchestrator",
        "routing_applied": true,
        // Track A 路由决策（思维模式 / prompt 策略 / persona 档）
        "thinking_mode": format!("{:?}", route_decision.thinking_mode),
        "prompt_policy": format!("{:?}", route_decision.prompt_policy),
        "tier": format!("{:?}", route_decision.tier),
        "confidence": route_decision.confidence,
        "router_source": route_decision.source.as_str(),
        "fallback_reason": route_decision.fallback_reason,
        "decision": {
            "intent": format!("{:?}", analysis.intent),
            "complexity": format!("{:?}", analysis.complexity),
            "needs_llm": analysis.needs_llm,
            "subtask_count": analysis.subtasks.len(),
            "rationale": analysis.rationale,
        },
        "session_key": session,
        "key": session,
        "task_id": task_id,
        "turn_id": task_id,
    })
}

/// 转发前注入会话身份（Web UI 的 `isCurrentSessionPayload` / `isCurrentTaskPayload`）
fn with_identity(mut env: Envelope, session: &str, task_id: &str) -> Envelope {
    let mut payload = env.payload.take();
    if let Some(obj) = payload.as_object_mut() {
        obj.entry("session_key").or_insert_with(|| json!(session));
        obj.entry("key").or_insert_with(|| json!(session));
        obj.entry("task_id").or_insert_with(|| json!(task_id));
        obj.entry("turn_id").or_insert_with(|| json!(task_id));
    }
    env.payload = payload;
    env
}

/// 富 Done 收据：Web UI 的 `doneTextSnapshot` 读 `text_snapshot` / `text`
fn done_payload(reply: &str, reason: &str, session: &str, task_id: &str) -> Value {
    json!({
        "text_snapshot": reply,
        "text": reply,
        "reason": reason,
        "session_key": session,
        "key": session,
        "task_id": task_id,
        "turn_id": task_id,
    })
}

/// chat.send 响应（对齐 ChatSendResponse：sessionKey / task_id / user_message_id …）
fn send_response(
    session_key: &str,
    task_id: &str,
    client_message_id: Option<&str>,
    replayed: bool,
) -> Value {
    let mut v = json!({
        "sessionKey": session_key,
        "task_id": task_id,
        "taskId": task_id,
        "user_message_id": task_id,
        "message_id": task_id,
        "accepted": true,
        "replayed": replayed,
    });
    if let Some(cmid) = client_message_id {
        v["client_message_id"] = json!(cmid);
        v["clientMessageId"] = json!(cmid);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn projection_snapshot_folds_turns_per_round() {
        // projection fold 契约的真实消费方：每轮 user+assistant 各计 1 轮（累计）
        let mut store = crate::projection::ProjectionStore::open(None).unwrap();
        let events = vec![
            serde_json::json!({ "type": "user/message" }),
            serde_json::json!({ "type": "assistant/message" }),
        ];
        let first = build_projection_snapshot(&mut store, "s1", &events).expect("快照应可构建");
        assert_eq!(first["projections"]["turns"]["turns"], 2);
        assert_eq!(first["projections"]["hostAudit"]["events"], 2);
        // 第二轮：跨轮累计（版本一致续折叠）
        let second = build_projection_snapshot(&mut store, "s1", &events).unwrap();
        assert_eq!(second["projections"]["turns"]["turns"], 4);
        assert_eq!(second["projections"]["hostAudit"]["events"], 4);
    }

    fn ctx_with_bus() -> (GatewayContext, Arc<EventBus>) {
        let bus = Arc::new(EventBus::default());
        let ctx = GatewayContext {
            config: Config::default(),
            token: "test-token".to_string(),
            started_at: std::time::Instant::now(),
            bus: bus.clone(),
        };
        (ctx, bus)
    }

    /// 简易 mock LLM HTTP 服务器：单次响应
    async fn mock_llm_server(status: u16, body: &str) -> String {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = body.to_string();
        tokio::spawn(async move {
            loop {
                if let Ok((mut socket, _)) = listener.accept().await {
                    let _ = crate::tests::mock_http::read_request_full(&mut socket).await;
                    let reason = if status >= 500 { "Server Error" } else { "OK" };
                    let header = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(body.as_bytes()).await;
                    let _ = socket.flush().await;
                }
            }
        });
        format!("http://{addr}")
    }

    /// 简易 mock LLM HTTP 服务器：循环响应 `times` 次（复杂任务分步执行需要多次调用）
    async fn mock_llm_server_multi(status: u16, body: &str, times: usize) -> String {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = body.to_string();
        tokio::spawn(async move {
            for _ in 0..times {
                let (mut socket, _) = match listener.accept().await {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let _ = crate::tests::mock_http::read_request_full(&mut socket).await;
                let reason = if status >= 500 { "Server Error" } else { "OK" };
                let header = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(body.as_bytes()).await;
                let _ = socket.flush().await;
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn test_send_runs_turn_and_publishes_enriched_done() {
        let url = mock_llm_server(
            200,
            r#"{"choices":[{"message":{"content":"你好，Marguerite！"},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":3,"total_tokens":8}}"#,
        )
        .await;
        let (mut ctx, bus) = ctx_with_bus();
        ctx.config.model.api_key = "test-key".to_string();
        ctx.config.model.base_url = url.to_string();

        let mut rx = bus.subscribe("**");
        let resp = send(
            &ctx,
            &json!({
                "message": "说句话",
                "sessionKey": "ws.chat_test",
                "clientRequestId": "crid-1",
                "clientMessageId": "cmid-1",
            }),
        )
        .await
        .expect("chat.send 应接受");
        assert_eq!(resp["accepted"], true);
        assert_eq!(resp["sessionKey"], "ws.chat_test");
        assert!(resp["task_id"]
            .as_str()
            .is_some_and(|t| t.starts_with("turn-")));

        // 等一轮结束后应收到唯一的富 Done（text_snapshot = 引擎答复）
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let mut got_done = false;
        let mut got_thinking = false;
        while std::time::Instant::now() < deadline {
            match rx.try_recv() {
                Ok(env) => {
                    if env.r#type == EventType::Thinking {
                        got_thinking = true;
                    }
                    if env.r#type == EventType::Done {
                        assert_eq!(env.payload["text_snapshot"], "你好，Marguerite！");
                        assert_eq!(env.payload["reason"], "completed");
                        assert_eq!(env.payload["session_key"], "ws.chat_test");
                        assert_eq!(env.payload["task_id"], resp["task_id"]);
                        got_done = true;
                        break;
                    }
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
        assert!(got_thinking, "应至少收到 Thinking 事件");
        assert!(got_done, "应收到富 Done 收据");
    }

    #[tokio::test]
    async fn test_send_idempotent_replay_by_client_request_id() {
        let url = mock_llm_server(
            200,
            r#"{"choices":[{"message":{"content":"hi"},"finish_reason":"stop"}],"usage":{"total_tokens":4}}"#,
        )
        .await;
        let (mut ctx, _bus) = ctx_with_bus();
        ctx.config.model.api_key = "test-key".to_string();
        ctx.config.model.base_url = url.to_string();

        let first = send(
            &ctx,
            &json!({
                "message": "hi",
                "sessionKey": "ws.replay",
                "clientRequestId": "crid-replay",
            }),
        )
        .await
        .expect("首次发送应接受");
        // 同一 clientRequestId 重放：不重复执行，返回同一 task_id
        let second = send(
            &ctx,
            &json!({
                "message": "hi",
                "sessionKey": "ws.replay",
                "clientRequestId": "crid-replay",
            }),
        )
        .await
        .expect("重放应命中幂等键");
        assert_eq!(second["task_id"], first["task_id"]);
        assert_eq!(second["replayed"], true);
    }

    #[tokio::test]
    async fn test_abort_is_idempotent_and_publishes_aborted_done() {
        let (ctx, bus) = ctx_with_bus();
        let mut rx = bus.subscribe("**");
        // 无运行任务时 abort 也应幂等成功
        let resp = abort(&ctx, &json!({ "sessionKey": "ws.ghost" }))
            .await
            .expect("abort 应幂等成功");
        assert_eq!(resp["accepted"], true);
        let env = rx.try_recv().expect("应收到 aborted 收据");
        assert_eq!(env.r#type, EventType::Done);
        assert_eq!(env.payload["reason"], "aborted");
        assert_eq!(env.payload["session_key"], "ws.ghost");
    }

    #[test]
    fn test_send_requires_message() {
        let (ctx, _bus) = ctx_with_bus();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt
            .block_on(send(&ctx, &json!({ "sessionKey": "ws.x" })))
            .unwrap_err();
        assert_eq!(err.code, "INVALID_REQUEST");
    }

    /// 复杂任务 → 自动拆解 + 计划驱动长执行：
    /// 事件流出现 plan_revision/plan_run，plans 落库，长时任务登记，最终 Done/Error 收尾。
    #[tokio::test]
    async fn test_complex_task_auto_decomposes_and_registers_long_running() {
        let url = mock_llm_server_multi(
            200,
            r#"{"choices":[{"message":{"content":"子步骤完成：已交付有依据的结论。"},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":8,"total_tokens":20}}"#,
            100,
        )
        .await;
        let (mut ctx, bus) = ctx_with_bus();
        ctx.config.model.api_key = "test-key".to_string();
        ctx.config.model.base_url = url.to_string();

        let mut rx = bus.subscribe("**");
        let complex = "帮我做一个完善的FPS第一人称射击游戏";
        let resp = send(
            &ctx,
            &json!({
                "message": complex,
                "sessionKey": "ws.auto_decompose",
                "clientRequestId": "crid-complex",
            }),
        )
        .await
        .expect("复杂任务应被接受");
        assert_eq!(resp["accepted"], true);

        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let (mut got_rev, mut got_run, mut terminal) = (false, false, false);
        while std::time::Instant::now() < deadline && !terminal {
            match rx.try_recv() {
                Ok(env) => match env.r#type {
                    EventType::PlanRevision => got_rev = true,
                    EventType::PlanRun => got_run = true,
                    EventType::Done | EventType::Error => terminal = true,
                    _ => {}
                },
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
        assert!(
            got_rev,
            "复杂任务应自动发布 session.event.plan_revision（拆解生效）"
        );
        assert!(
            got_run,
            "复杂任务应自动发布 session.event.plan_run（分步执行启动）"
        );
        assert!(terminal, "执行应以 Done/Error 收尾");

        // plans 落库
        let plans = crate::plan::plans_list();
        let runs = plans["runs"].as_array().expect("plans.runs 应为数组");
        assert!(
            runs.iter().any(|r| {
                matches!(
                    r["status"].as_str(),
                    Some("running" | "completed" | "cancelled" | "blocked")
                )
            }),
            "应存在计划运行记录"
        );
        // 长时任务登记（心跳/检查点调度器）
        let lrt = crate::automation::long_running::global()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert!(!lrt.list().is_empty(), "长时任务应已登记");
    }

    /// 简单任务 → 不触发自动拆解（向后兼容：无 plan 事件，仍正常 Done）
    #[tokio::test]
    async fn test_simple_task_skips_auto_decompose() {
        let url = mock_llm_server(
            200,
            r#"{"choices":[{"message":{"content":"好的"},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":3,"total_tokens":8}}"#,
        )
        .await;
        let (mut ctx, bus) = ctx_with_bus();
        ctx.config.model.api_key = "test-key".to_string();
        ctx.config.model.base_url = url.to_string();

        let mut rx = bus.subscribe("**");
        let resp = send(
            &ctx,
            &json!({
                "message": "说句话",
                "sessionKey": "ws.simple",
                "clientRequestId": "crid-simple",
            }),
        )
        .await
        .expect("简单任务应被接受");
        assert_eq!(resp["accepted"], true);

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let (mut got_plan, mut got_done) = (false, false);
        while std::time::Instant::now() < deadline && !got_done {
            match rx.try_recv() {
                Ok(env) => match env.r#type {
                    EventType::PlanRevision | EventType::PlanRun => got_plan = true,
                    EventType::Done => got_done = true,
                    _ => {}
                },
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
        assert!(got_done, "简单任务应正常完成");
        assert!(!got_plan, "简单任务不应触发自动拆解（向后兼容）");
    }
}
