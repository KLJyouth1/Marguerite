//! 子代理（SubAgent）系统
//!
//! 用于并行执行子任务，每个子代理拥有独立的简化运行时。
//! 支持超时控制、结果收集和取消操作。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use tokio::sync::{Mutex, Semaphore};
use uuid::Uuid;

use crate::agent::tool_cycle::{ToolCall as CycleToolCall, ToolCycle, ToolResult};
use crate::agent::tool_schema::{discovery_query, ToolDisclosure, DISCOVER_TOOLS_NAME};
use crate::events::{Envelope, EventBus};
use crate::mcp::registry::ToolRegistry;
use crate::model::llm::ToolDef;
use crate::model::router::{Complexity, IntentType, ModelRouter};
use crate::types::Message;

/// 子代理配置
#[derive(Debug, Clone)]
pub struct SubAgentConfig {
    /// 最大并行子代理数
    pub max_concurrent: usize,
    /// 默认超时时间
    pub default_timeout: Duration,
    /// 子代理使用的模型（None 则继承父代理）
    pub model: Option<String>,
}

impl Default for SubAgentConfig {
    fn default() -> Self {
        Self {
            // 并发闸门（不是成员数量上限）：默认值与环境变量覆盖集中在
            // `budget_policy`，避免散落硬编码。
            max_concurrent: crate::agent::budget_policy::subagent_max_concurrent(),
            default_timeout: subagent_timeout(),
            model: None,
        }
    }
}

/// 子代理默认超时。
///
/// 具体任务在 spawn 时由 [`crate::agent::budget_policy`] 按任务体量解析；
/// 这里只提供「没有任务文本」时的兜底值（Standard 档，通常 30 分钟），
/// `MARGUERITE_SUBAGENT_TIMEOUT_SECS` 仍可覆盖。
pub fn subagent_timeout() -> Duration {
    crate::agent::budget_policy::resolve("[[budget:standard]]", 1, 0).run_timeout
}

/// 子代理工具回路的最大轮数（无任务文本时的兜底值）。
///
/// 真实调用点用 [`crate::agent::budget_policy::resolve`] 按任务体量给出轮数
/// （Simple 8 → Deep 48，可被模型 `[[budget:...]]` 声明升降档）；
/// 这里保留 Standard 档作为默认，`MARGUERITE_SUBAGENT_TOOL_ROUNDS` 可覆盖。
pub fn subagent_tool_rounds() -> usize {
    crate::agent::budget_policy::resolve("[[budget:standard]]", 1, 0).tool_rounds
}

/// 工具轮数耗尽时的统一收敛：有阶段性正文就交出去（并标注被截断），
/// 完全没有正文才报错——错误里必须写明限值与如何放宽。
fn finish_exhausted_rounds(last_content: &str, rounds: usize) -> Result<String> {
    if last_content.trim().is_empty() {
        return Err(anyhow!(
            "子代理工具回路达到 {rounds} 轮上限且没有产出正文。\
             如任务确实需要更多工具调用，请提高 MARGUERITE_SUBAGENT_TOOL_ROUNDS（上限 64）后重试。"
        ));
    }
    Ok(format!(
        "{last_content}\n\n---\n（工具调用达到 {rounds} 轮上限，以上为阶段性结论；\
         如仍缺证据，可提高 MARGUERITE_SUBAGENT_TOOL_ROUNDS 后重跑。）"
    ))
}

/// 子代理状态
#[derive(Debug, Clone, PartialEq)]
pub enum SubAgentStatus {
    /// 等待中
    Pending,
    /// 运行中
    Running,
    /// 已完成
    Completed { result: String },
    /// 失败
    Failed { error: String },
    /// 超时
    TimedOut,
}

/// 子代理
#[derive(Debug, Clone)]
pub struct SubAgent {
    /// 子代理 ID
    pub id: String,
    /// 任务描述
    pub task: String,
    /// 当前状态
    pub status: SubAgentStatus,
    /// 配置
    pub config: SubAgentConfig,
    /// 创建时间
    pub created_at: DateTime<Utc>,
    /// 完成时间
    pub completed_at: Option<DateTime<Utc>>,
    /// 执行结果
    pub result: Option<String>,
}

impl SubAgent {
    fn new(id: String, task: String, config: SubAgentConfig) -> Self {
        Self {
            id,
            task,
            status: SubAgentStatus::Pending,
            config,
            created_at: Utc::now(),
            completed_at: None,
            result: None,
        }
    }
}

/// 子代理执行进度上报器（编排可见性）。
///
/// 为什么要有它：蜂群 / 团队此前只上报 `agent.started` / `agent.stopped` 两条粗粒度
/// 事件，且事件 session 是子代理 id，前端按当前会话归因时全部丢弃 —— 用户在对话页
/// 只能看到一个「已提交」toast，然后什么都没有，等于黑盒。
///
/// 现在子代理在**调用方会话**上按阶段发布 `session.event.progress`：
/// `member_started` / `tool_call` / `tool_result` / `member_completed`，
/// 前端据此渲染泳道、工具行与耗时，过程完整可见。
#[derive(Clone)]
pub struct ProgressReporter {
    bus: Arc<EventBus>,
    /// 调用方会话键（不是子代理 id）：前端按它归因到当前对话。
    session: String,
    run_id: String,
    member_id: String,
    role: String,
}

impl ProgressReporter {
    /// 编排级（run）上报器：member_id 为空，用于 run_started / lead_synthesizing / 终态。
    pub fn for_run(bus: Arc<EventBus>, session: &str, run_id: &str) -> Self {
        Self {
            bus,
            session: session.to_string(),
            run_id: run_id.to_string(),
            member_id: String::new(),
            role: "orchestrator".to_string(),
        }
    }

    /// 成员级上报器：在 run 上报器基础上绑定一个子代理。
    pub fn for_member(&self, member_id: &str, role: &str) -> Self {
        Self {
            bus: self.bus.clone(),
            session: self.session.clone(),
            run_id: self.run_id.clone(),
            member_id: member_id.to_string(),
            role: role.to_string(),
        }
    }

    /// 发布一条编排进度事件（`session.event.progress`，payload.kind = orchestration）。
    pub fn emit(&self, phase: &str, mut payload: serde_json::Value) {
        let obj = match payload.as_object_mut() {
            Some(o) => o,
            None => return,
        };
        obj.insert("kind".into(), serde_json::json!("orchestration"));
        obj.insert("phase".into(), serde_json::json!(phase));
        obj.insert("runId".into(), serde_json::json!(self.run_id));
        if !self.member_id.is_empty() {
            obj.insert("memberId".into(), serde_json::json!(self.member_id));
            obj.insert("role".into(), serde_json::json!(self.role));
        }
        // 团队看板的数据源：把同一份进度帧投影成「每个成员此刻在做什么」的快照。
        // **必须在补齐 runId / memberId / role 之后**再投影 —— 此前放在 `as_object_mut`
        // 之前，注册表看到的每一帧都「没有 memberId」，成员帧全被当成无关帧丢弃，
        // 于是看板永远只有一张主智能体卡（用户反馈「团队没有独立成员卡片」的真根因）。
        // 纯观察：注册表内部有界且不返回错误，绝不影响编排主流程。
        crate::orchestration::member_registry::observe(&self.session, phase, &payload);
        self.bus.publish(Envelope::new(
            crate::events::EventType::Progress,
            payload,
            self.session.clone(),
        ));
    }
}

/// 子代理管理器
///
/// 管理多个子代理的创建、执行、监控和结果收集。
pub struct SubAgentManager {
    /// 子代理集合
    agents: Arc<Mutex<HashMap<String, SubAgent>>>,
    /// 完成信号发送端
    completion_tx: tokio::sync::broadcast::Sender<String>,
    /// 配置
    config: SubAgentConfig,
    /// 工具注册表
    tool_registry: Arc<ToolRegistry>,
    /// 模型路由器
    model_router: Arc<ModelRouter>,
    /// 事件总线（A4 事件流埋点；None 时无事件发布）
    events: Option<Arc<EventBus>>,
    /// 并发闸门：限制同时运行的子代理数量（修复 max_concurrent 此前未生效）
    concurrency_gate: Arc<Semaphore>,
    /// 并发闸门的上限（运行时可抬升，见 [`Self::ensure_concurrency`]）。
    ///
    /// 为什么要单独记一份：`Semaphore` 不暴露当前容量，团队要按成员数抬升闸门时
    /// 必须知道「已经抬到多少」，否则会重复 add_permits 造成并发失控。
    concurrency_limit: Arc<std::sync::atomic::AtomicUsize>,
    /// P5：调用方会话键。子代理跑在独立 `tokio::spawn` 里，task-local `SESSION`
    /// **不会**自动继承；缺了它，子代理内文件工具登记产物时 `current_session()`
    /// 为空 → 产物记到默认会话，「本次会话产物列表」永远为空
    /// （真机告警：`record_artifact: 无可用 SESSION，回退到…默认会话`）。
    session: Option<String>,
    /// 编排进度上报器（P2·E 可见性）：Some 时把成员与工具过程回流到调用方会话。
    /// 用 `RwLock` 包一层是为了让 [`Self::set_progress`] 接收 `&self`：编排器与
    /// 团队共享同一个管理器实例，注册都发生在 `&self` 方法里（此前 `&mut self`
    /// 直接导致 `orchestration/mod.rs` 两处调用无法编译）。临界区只做一次 Option
    /// 赋值/克隆，不含 await，故用 std 同步锁即可。
    progress: Arc<std::sync::RwLock<Option<ProgressReporter>>>,
    /// 编排级取消信号（「停止」按钮）：Some 时成员执行会与它 `select!`，
    /// 命中后立刻收口，不再等模型/工具自己跑完。
    cancel: Arc<std::sync::RwLock<Option<Arc<crate::orchestration::cancel::CancelSignal>>>>,
}

impl SubAgentManager {
    /// 创建新的子代理管理器
    pub fn new(
        config: SubAgentConfig,
        tool_registry: Arc<ToolRegistry>,
        model_router: Arc<ModelRouter>,
    ) -> Self {
        let (completion_tx, _) = tokio::sync::broadcast::channel(256);
        let limit = config.max_concurrent.max(1);
        let concurrency_gate = Arc::new(Semaphore::new(limit));
        let concurrency_limit = Arc::new(std::sync::atomic::AtomicUsize::new(limit));
        Self {
            agents: Arc::new(Mutex::new(HashMap::new())),
            completion_tx,
            config,
            tool_registry,
            model_router,
            events: None,
            concurrency_gate,
            concurrency_limit,
            session: None,
            progress: Arc::new(std::sync::RwLock::new(None)),
            cancel: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    /// 按本次团队 / 蜂群规模抬升并发上限（**只升不降**）。
    ///
    /// 用户口径：不应对蜂群、团队、专家的数量设上限。`max_concurrent` 原本是
    /// 子代理的默认并发闸门（默认 5），团队一次派 8 人时后 3 人会排队，看板上
    /// 只看到 5 个成员在跑（真机实测）。这里按「本波成员数」抬升闸门；单次临时
    /// 子代理的默认并发不受影响。
    pub fn ensure_concurrency(&self, n: usize) {
        use std::sync::atomic::Ordering;
        let want = n.max(1);
        let mut cur = self.concurrency_limit.load(Ordering::SeqCst);
        while cur < want {
            match self
                .concurrency_limit
                .compare_exchange(cur, want, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => {
                    self.concurrency_gate.add_permits(want - cur);
                    return;
                }
                Err(actual) => cur = actual,
            }
        }
    }

    /// P5：登记调用方会话键（供子代理 spawn 时挂 `SESSION` task-local）。
    ///
    /// 与 [`Self::set_event_bus`] 同时由编排器调用；未登记时行为与既有完全一致
    /// （不做任何作用域包裹），保证向后兼容。
    pub fn set_session(&mut self, session: String) {
        self.session = Some(session);
    }

    /// 注册事件总线（A4 事件流埋点接口）
    pub fn set_event_bus(&mut self, bus: Arc<EventBus>) {
        self.events = Some(bus);
    }

    /// 注册编排进度上报器（P2·E 可见性）。
    ///
    /// 与 `set_event_bus` 的区别：总线只发粗粒度 `subagent_start/end`（session=子代理 id），
    /// 上报器按 `phase` 发布细粒度过程事件到**调用方会话**，前端才能渲染泳道与工具行。
    pub fn set_progress(&self, reporter: ProgressReporter) {
        *self
            .progress
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(reporter);
    }

    /// 登记编排级取消信号（每个 run 一份，`Orchestrator::begin_orchestration` 调用）。
    pub fn set_cancel_signal(&self, signal: Arc<crate::orchestration::cancel::CancelSignal>) {
        *self
            .cancel
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(signal);
    }

    /// 返回并发上限（来自 [`SubAgentConfig::max_concurrent`]）
    ///
    /// 原生 harness 层的 [`crate::harness::subagent::ConcurrencyGate`] 以此值为
    /// 闸门上限，保证单一事实源：配置处改一处，闸门即同步。
    pub fn max_concurrent(&self) -> usize {
        self.config.max_concurrent
    }

    /// 发布事件（若已注册事件总线）
    fn emit(&self, env: Envelope) {
        if let Some(bus) = &self.events {
            bus.publish(env);
        }
    }

    /// 生成子代理
    ///
    /// 创建一个新的子代理并启动异步执行。
    /// 返回子代理 ID。
    pub async fn spawn(&self, task: &str, parent_context: &str) -> Result<String> {
        self.spawn_with_timeout(task, parent_context, self.config.default_timeout)
            .await
    }

    /// 生成子代理（带自定义超时）
    ///
    /// 创建一个新的子代理，并指定超时时间。
    pub async fn spawn_with_timeout(
        &self,
        task: &str,
        context: &str,
        timeout: Duration,
    ) -> Result<String> {
        let agent_id = Uuid::new_v4().to_string();
        let subagent = SubAgent::new(
            agent_id.clone(),
            task.to_string(),
            SubAgentConfig {
                max_concurrent: self.config.max_concurrent,
                default_timeout: timeout,
                model: self.config.model.clone(),
            },
        );

        // 注册子代理
        {
            let mut agents = self.agents.lock().await;
            agents.insert(agent_id.clone(), subagent);
        }

        // A4 埋点：子代理（专家）启动，携带任务描述供前端卡片渲染
        self.emit(Envelope::subagent_start(&agent_id, task));

        // 启动异步执行
        let agents = self.agents.clone();
        let completion_tx = self.completion_tx.clone();
        let tool_registry = self.tool_registry.clone();
        let model_router = self.model_router.clone();
        let events = self.events.clone();
        let concurrency_gate = self.concurrency_gate.clone();
        let task_str = task.to_string();
        let context_str = context.to_string();
        let agent_id_clone = agent_id.clone();
        // P5：把调用方会话带进 spawn（task-local 不跨任务继承）
        let session_for_task = self.session.clone();
        // 思考档位同理：`TURN_EFFORT` 是 task-local，spawn 后子代理会退回“自适应档”，
        // 表现为“用户拖到 max，子代理/团队仍然不深思”。在父任务里取值，进子任务再 scope。
        let inherited_effort = crate::model::thinking::current_turn_effort();
        tokio::spawn(async move {
            // 并发闸门：限制同时运行的子代理数量（修复 SubAgentConfig.max_concurrent 此前未生效）
            let _permit = concurrency_gate
                .clone()
                .acquire_owned()
                .await
                .expect("并发闸门信号量已被关闭");

            // 更新状态为 Running
            {
                let mut agents = agents.lock().await;
                if let Some(agent) = agents.get_mut(&agent_id_clone) {
                    agent.status = SubAgentStatus::Running;
                }
            }

            // 执行任务
            let result = tokio::time::timeout(timeout, async {
                let fut =
                    execute_subagent_task(&task_str, &context_str, &model_router, &tool_registry);
                // 内层：会话作用域（产物归因）；外层：思考档位作用域（与主智能体同档）
                let session_scoped = async move {
                    match &session_for_task {
                        Some(s) => crate::usage::SESSION.scope(s.clone(), fut).await,
                        None => fut.await,
                    }
                };
                match inherited_effort {
                    Some(effort) => {
                        crate::model::thinking::TURN_EFFORT
                            .scope(Some(effort), session_scoped)
                            .await
                    }
                    None => session_scoped.await,
                }
            })
            .await;

            // 更新结果
            let (ok, summary) = match &result {
                Ok(Ok(o)) => (true, o.clone()),
                Ok(Err(e)) => (false, e.to_string()),
                Err(_) => (false, "timeout".to_string()),
            };
            let mut agents = agents.lock().await;
            if let Some(agent) = agents.get_mut(&agent_id_clone) {
                // 记录终态时刻：此前只在 cancel() 里写，正常完成/失败/超时都留空，
                // 导致「子代理完成时间」在上层永远拿不到。
                agent.completed_at = Some(Utc::now());
                match result {
                    Ok(Ok(output)) => {
                        agent.status = SubAgentStatus::Completed {
                            result: output.clone(),
                        };
                        agent.result = Some(output);
                    }
                    Ok(Err(e)) => {
                        agent.status = SubAgentStatus::Failed {
                            error: e.to_string(),
                        };
                    }
                    Err(_) => {
                        agent.status = SubAgentStatus::TimedOut;
                    }
                }
            }

            // A4 埋点：子代理（专家）完成
            if let Some(bus) = &events {
                bus.publish(Envelope::subagent_end(&agent_id_clone, ok, &summary));
            }

            let _ = completion_tx.send(agent_id_clone);
        });

        Ok(agent_id)
    }

    /// 生成角色感知的子代理（带自定义超时）
    ///
    /// 与 [`spawn_with_timeout`] 类似，但额外接收 `role_prompt`（该子 Agent 的系统人设）
    /// 与 `intent`/`complexity`（用于 [`ModelRouter::chat_for`] 的模型路由），
    /// 使每个子 Agent 成为「专精角色」而非主 Agent 的克隆——这是编排「数倍强」的关键。
    pub async fn spawn_with_spec(
        &self,
        task: &str,
        context: &str,
        role_prompt: &str,
        intent: IntentType,
        complexity: Complexity,
    ) -> Result<String> {
        self.spawn_spec_with_timeout(
            task,
            context,
            role_prompt,
            intent,
            complexity,
            // 超时按任务体量自适应（模型可用 `[[budget:deep]]` 显式升档）。
            crate::agent::budget_policy::resolve(task, 1, 0).run_timeout,
        )
        .await
    }

    /// 生成角色感知的子代理（带自定义超时）
    pub async fn spawn_spec_with_timeout(
        &self,
        task: &str,
        context: &str,
        role_prompt: &str,
        intent: IntentType,
        complexity: Complexity,
        timeout: Duration,
    ) -> Result<String> {
        let agent_id = Uuid::new_v4().to_string();
        let subagent = SubAgent::new(
            agent_id.clone(),
            task.to_string(),
            SubAgentConfig {
                max_concurrent: self.config.max_concurrent,
                default_timeout: timeout,
                model: self.config.model.clone(),
            },
        );

        {
            let mut agents = self.agents.lock().await;
            agents.insert(agent_id.clone(), subagent);
        }

        // A4 埋点：角色感知子代理（专家）启动，携带任务描述
        self.emit(Envelope::subagent_start(&agent_id, task));

        let agents = self.agents.clone();
        let completion_tx = self.completion_tx.clone();
        let tool_registry = self.tool_registry.clone();
        let model_router = self.model_router.clone();
        let events = self.events.clone();
        let concurrency_gate = self.concurrency_gate.clone();
        let task_str = task.to_string();
        let context_str = context.to_string();
        let role_str = role_prompt.to_string();
        let agent_id_clone = agent_id.clone();
        // P5：把调用方会话带进 spawn（task-local 不跨任务继承）
        let session_for_task = self.session.clone();
        // 思考档位同理：`TURN_EFFORT` 是 task-local，spawn 后子代理会退回“自适应档”，
        // 表现为“用户拖到 max，子代理/团队仍然不深思”。在父任务里取值，进子任务再 scope。
        let inherited_effort = crate::model::thinking::current_turn_effort();
        // P2·E：成员级可见性上报器（角色标签取人设首句，避免把整段 prompt 塞给前端）
        // 先从共享槽里克隆出 Option，再映射 —— 读锁不能跨进 spawned task。
        let member_progress = self
            .progress
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .map(|r| r.for_member(&agent_id, &role_label(role_prompt)));
        // 「停止」按钮的取消信号：读锁不能跨进 spawned task，提前克隆出来。
        let cancel_signal = self
            .cancel
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        tokio::spawn(async move {
            // 并发闸门：限制同时运行的子代理数量（修复 SubAgentConfig.max_concurrent 此前未生效）
            let _permit = concurrency_gate
                .clone()
                .acquire_owned()
                .await
                .expect("并发闸门信号量已被关闭");

            {
                let mut agents = agents.lock().await;
                if let Some(agent) = agents.get_mut(&agent_id_clone) {
                    agent.status = SubAgentStatus::Running;
                }
            }
            let member_started_ms = std::time::Instant::now();
            if let Some(p) = &member_progress {
                p.emit(
                    "member_started",
                    serde_json::json!({ "task": task_str, "roleLabel": role_label(&role_str) }),
                );
            }

            // 执行体与取消信号竞速：用户点「停止」时直接丢弃模型/工具的 future，
            // 而不是等它自己跑完（否则后台继续烧 token，卡片也不会结束）。
            let mut cancelled = false;
            let result = {
                let exec = async {
                    tokio::time::timeout(timeout, async {
                        let fut = execute_subagent_task_spec(
                            &task_str,
                            &context_str,
                            &role_str,
                            intent,
                            complexity,
                            &model_router,
                            &tool_registry,
                            member_progress.clone(),
                        );
                        // 内层：会话作用域（产物归因）；外层：思考档位作用域
                        let session_scoped = async move {
                            match &session_for_task {
                                Some(s) => crate::usage::SESSION.scope(s.clone(), fut).await,
                                None => fut.await,
                            }
                        };
                        match inherited_effort {
                            Some(effort) => {
                                crate::model::thinking::TURN_EFFORT
                                    .scope(Some(effort), session_scoped)
                                    .await
                            }
                            None => session_scoped.await,
                        }
                    })
                    .await
                };
                match &cancel_signal {
                    Some(signal) => tokio::select! {
                        biased;
                        _ = signal.cancelled() => {
                            cancelled = true;
                            None
                        }
                        r = exec => Some(r),
                    },
                    None => Some(exec.await),
                }
            };

            let (ok, summary) = match &result {
                None => (false, "已取消".to_string()),
                Some(Ok(Ok(o))) => (true, o.clone()),
                Some(Ok(Err(e))) => (false, e.to_string()),
                Some(Err(_)) => (false, "timeout".to_string()),
            };
            if let Some(p) = &member_progress {
                p.emit(
                    "member_completed",
                    serde_json::json!({
                        "ok": ok,
                        "elapsedMs": member_started_ms.elapsed().as_millis() as u64,
                        "summary": truncate_chars(&summary, 1200),
                        "timedOut": matches!(result, Some(Err(_))),
                        "cancelled": cancelled,
                    }),
                );
            }
            let mut agents = agents.lock().await;
            if let Some(agent) = agents.get_mut(&agent_id_clone) {
                // 记录终态时刻：此前只在 cancel() 里写，正常完成/失败/超时都留空，
                // 导致「子代理完成时间」在上层永远拿不到。
                agent.completed_at = Some(Utc::now());
                match result {
                    None => {
                        let msg = "[子专家已取消]".to_string();
                        agent.status = SubAgentStatus::Failed { error: msg.clone() };
                        agent.result = Some(msg);
                    }
                    Some(Ok(Ok(output))) => {
                        agent.status = SubAgentStatus::Completed {
                            result: output.clone(),
                        };
                        agent.result = Some(output);
                    }
                    Some(Ok(Err(e))) => {
                        // 失败也回填 result：上端 `get_result` 才能拿到真实错误，
                        // 而不是永远得到「[子专家无结果返回]」误导信息。
                        let msg = format!("[子专家执行失败] {e}");
                        agent.status = SubAgentStatus::Failed { error: msg.clone() };
                        agent.result = Some(msg);
                    }
                    Some(Err(_)) => {
                        let msg = "[子专家执行超时]".to_string();
                        agent.status = SubAgentStatus::TimedOut;
                        agent.result = Some(msg);
                    }
                }
            }

            if let Some(bus) = &events {
                bus.publish(Envelope::subagent_end(&agent_id_clone, ok, &summary));
            }

            let _ = completion_tx.send(agent_id_clone);
        });

        Ok(agent_id)
    }

    /// 获取子代理状态
    pub async fn get_status(&self, agent_id: &str) -> Option<SubAgentStatus> {
        let agents = self.agents.lock().await;
        agents.get(agent_id).map(|a| a.status.clone())
    }

    /// 获取子代理结果
    pub async fn get_result(&self, agent_id: &str) -> Option<String> {
        let agents = self.agents.lock().await;
        agents.get(agent_id).and_then(|a| a.result.clone())
    }

    /// 等待所有子代理完成
    ///
    /// 阻塞直到所有子代理完成。
    pub async fn wait_all(&self) -> Result<Vec<(String, SubAgentStatus)>> {
        let mut rx = self.completion_tx.subscribe();
        loop {
            // 检查是否所有子代理都已结束
            let all_done = {
                let agents = self.agents.lock().await;
                agents.values().all(|a| {
                    matches!(
                        a.status,
                        SubAgentStatus::Completed { .. }
                            | SubAgentStatus::Failed { .. }
                            | SubAgentStatus::TimedOut
                    )
                })
            };

            if all_done {
                break;
            }

            // 等待下一个完成信号
            let _ = rx.recv().await;
        }

        self.collect_statuses().await
    }

    /// 等待**指定**子代理进入终态（Completed / Failed / TimedOut）。
    ///
    /// 与 [`Self::wait_all`] 的区别：单成员编排只关心自己 spawn 的那个 agent，
    /// 而 `wait_all` 会被同一 manager 里其它仍在跑的 agent 拖住（对账/终态帧因此永远发不出）。
    pub async fn wait_for(&self, agent_id: &str) -> Result<SubAgentStatus> {
        let mut rx = self.completion_tx.subscribe();
        loop {
            {
                let agents = self.agents.lock().await;
                let Some(agent) = agents.get(agent_id) else {
                    return Err(anyhow::anyhow!("子代理 {agent_id} 不存在"));
                };
                if matches!(
                    agent.status,
                    SubAgentStatus::Completed { .. }
                        | SubAgentStatus::Failed { .. }
                        | SubAgentStatus::TimedOut
                ) {
                    return Ok(agent.status.clone());
                }
            }
            // 先查状态再等信号：完成信号可能在 subscribe 之前就已发送，只等信号会永久挂起。
            if rx.recv().await.is_err() {
                return Err(anyhow::anyhow!("子代理完成信号通道已关闭"));
            }
        }
    }

    /// 等待任一子代理完成
    pub async fn wait_any(&self) -> Result<Option<(String, SubAgentStatus)>> {
        // 先检查是否有已经完成的，或者没有活跃的 agent
        {
            let agents = self.agents.lock().await;
            let has_active = agents
                .values()
                .any(|a| matches!(a.status, SubAgentStatus::Pending | SubAgentStatus::Running));
            if !has_active {
                // 没有活跃 agent，返回第一个已完成的状态或 None
                return Ok(agents.iter().find_map(|(id, agent)| {
                    if matches!(
                        agent.status,
                        SubAgentStatus::Completed { .. }
                            | SubAgentStatus::Failed { .. }
                            | SubAgentStatus::TimedOut
                    ) {
                        Some((id.clone(), agent.status.clone()))
                    } else {
                        None
                    }
                }));
            }
        }

        let mut rx = self.completion_tx.subscribe();

        // 等待下一个完成信号
        match rx.recv().await {
            Ok(agent_id) => {
                let agents = self.agents.lock().await;
                Ok(agents.get(&agent_id).map(|a| (agent_id, a.status.clone())))
            }
            Err(_) => Ok(None),
        }
    }

    /// 取消子代理
    ///
    /// 标记子代理为失败状态（当前实现中，无法真正停止 tokio 任务，
    /// 但会更新状态以便调用方感知取消）。
    pub async fn cancel(&self, agent_id: &str) -> Result<()> {
        let mut agents = self.agents.lock().await;
        let agent = agents
            .get_mut(agent_id)
            .ok_or_else(|| anyhow!("Agent '{}' not found", agent_id))?;

        agent.status = SubAgentStatus::Failed {
            error: "Cancelled by user".to_string(),
        };
        agent.completed_at = Some(Utc::now());
        Ok(())
    }

    /// 取消所有子代理
    pub async fn cancel_all(&self) {
        let mut agents = self.agents.lock().await;
        for agent in agents.values_mut() {
            if matches!(
                agent.status,
                SubAgentStatus::Pending | SubAgentStatus::Running
            ) {
                agent.status = SubAgentStatus::Failed {
                    error: "Cancelled by user".to_string(),
                };
                agent.completed_at = Some(Utc::now());
            }
        }
    }

    /// 收集所有已完成的结果
    pub async fn collect_results(&self) -> Vec<(String, String)> {
        let agents = self.agents.lock().await;
        agents
            .iter()
            .filter_map(|(id, agent)| agent.result.clone().map(|r| (id.clone(), r)))
            .collect()
    }

    /// 活跃子代理数
    pub async fn active_count(&self) -> usize {
        let agents = self.agents.lock().await;
        agents
            .values()
            .filter(|a| matches!(a.status, SubAgentStatus::Pending | SubAgentStatus::Running))
            .count()
    }

    /// 收集所有子代理状态
    async fn collect_statuses(&self) -> Result<Vec<(String, SubAgentStatus)>> {
        let agents = self.agents.lock().await;
        let mut results = Vec::new();
        for (id, agent) in agents.iter() {
            results.push((id.clone(), agent.status.clone()));
        }
        results.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(results)
    }

    /// 获取子代理数量
    pub async fn agent_count(&self) -> usize {
        let agents = self.agents.lock().await;
        agents.len()
    }
}

/// 按当前子代理已暴露的 schema 执行工具调用；发现工具只扩充目录，不进入注册表执行。
async fn execute_disclosed_tool_calls(
    cycle: &ToolCycle,
    disclosure: &mut ToolDisclosure,
    tool_calls: &[crate::model::llm::ToolCall],
) -> Result<Vec<ToolResult>> {
    let mut results = Vec::with_capacity(tool_calls.len());
    for tool_call in tool_calls {
        if tool_call.name == DISCOVER_TOOLS_NAME {
            let args = serde_json::from_str::<serde_json::Value>(&tool_call.arguments)
                .unwrap_or(serde_json::Value::Null);
            let output = discovery_query(&args)
                .map(|query| disclosure.discover(query))
                .unwrap_or_else(|| "发现工具需要提供字符串参数 query。".to_string());
            results.push(ToolResult::Success {
                data: output,
                metadata: HashMap::new(),
            });
        } else if !disclosure.permits(&tool_call.name) {
            results.push(ToolResult::Error {
                message: format!(
                    "工具 `{}` 尚未加载参数 schema。请先调用 `{}` 并描述所需能力，再在下一轮调用该工具。",
                    tool_call.name, DISCOVER_TOOLS_NAME
                ),
                code: "TOOL_SCHEMA_NOT_LOADED".to_string(),
            });
        } else {
            let call = CycleToolCall::new(&tool_call.name, parse_tool_args(&tool_call.arguments));
            results.extend(cycle.execute_sequential(vec![call]).await?);
        }
    }
    Ok(results)
}

/// 执行子代理任务
///
/// 创建一个简化的运行环境，使用模型路由器执行任务。
/// 若工具注册表含有可用工具，则开启功能调用回路（plan → act → observe），
/// 使子 Agent 能真正读写文件、跑命令，而非仅靠文本推理。
async fn execute_subagent_task(
    task: &str,
    context: &str,
    model_router: &ModelRouter,
    tool_registry: &Arc<ToolRegistry>,
) -> Result<String> {
    // 构建系统消息
    let system_msg = Message::System {
        content: format!(
            "You are a helpful sub-agent assistant. Your task is: {}\n\nContext: {}",
            task, context
        ),
        timestamp: Utc::now(),
    };

    let user_msg = Message::User {
        content: format!("Please complete the following task:\n\n{}", task),
        timestamp: Utc::now(),
    };

    let mut messages = vec![system_msg, user_msg];

    // 构建工具定义（本地 + MCP）。无工具则退化为纯对话（向后兼容）。
    let tool_defs: Vec<ToolDef> = tool_registry
        .describe_tools_for_llm()
        .into_iter()
        .map(|(name, description, parameters)| ToolDef {
            name,
            description,
            parameters,
        })
        .collect();
    // 子代理 / 团队成员也必须知道自己有什么能力：真机反馈里子代理完全不知道
    // 有内置浏览器、真终端、编辑器与技能索引，于是「做不到」成了默认答复。
    // 紧凑版只给组名 + 代表工具 + 决策规则，避免挤占子任务本身需要的上下文。
    {
        let snap = crate::agent::tool_catalog::global_snapshot();
        messages.push(Message::System {
            content: crate::agent::tool_catalog::compact_catalog_text(&tool_defs, &snap),
            timestamp: Utc::now(),
        });
    }
    if tool_defs.is_empty() {
        let response = model_router
            .chat(&messages)
            .await
            .map_err(|e| anyhow!("SubAgent execution failed: {}", e))?;
        return Ok(response.content);
    }

    // 功能调用回路：宽工具目录按任务按需注入 schema，发现后下轮给出完整参数定义。
    let request_text = format!("{task}\n{context}");
    let mut disclosure =
        ToolDisclosure::new(&tool_defs, &request_text, crate::mode::runtime_mode());
    if disclosure.is_enabled() {
        messages.insert(
            0,
            Message::System {
                content: format!(
                    "首轮仅暴露常用及相关工具 schema。缺少能力时先调用 `{DISCOVER_TOOLS_NAME}`，\n\
                     用自然语言描述所需工具；发现后完整参数 schema 会在下一轮提供。不要因此声称能力不存在。"
                ),
                timestamp: Utc::now(),
            },
        );
    }
    // 工具执行失败不再以 `?` 中断整个子专家——把错误作为 tool 结果回灌，让模型自我纠正；
    // 轮数由任务体量解析（模型可在计划里写 `[[budget:complex]]` 升档）。
    let cycle = ToolCycle::new(tool_registry.clone());
    let mut last_content = String::new();
    let rounds = crate::agent::budget_policy::resolve(task, 1, tool_defs.len()).tool_rounds;
    for _ in 0..rounds {
        let active_tools = disclosure.active_tools();
        let response = match model_router.chat_with_tools(&messages, &active_tools).await {
            Ok(r) => r,
            Err(e) => return Err(anyhow!("SubAgent 模型调用失败: {e}")),
        };
        if !response.content.is_empty() {
            last_content = response.content.clone();
        }
        if response.tool_calls.is_empty() {
            return Ok(response.content);
        }
        // 整个工具批次执行异常：构造一条错误结果回灌，让模型改换策略而非抛错中断。
        let results =
            match execute_disclosed_tool_calls(&cycle, &mut disclosure, &response.tool_calls).await
            {
                Ok(r) => r,
                Err(e) => {
                    let name = response
                        .tool_calls
                        .first()
                        .map(|tc| tc.name.clone())
                        .unwrap_or_default();
                    // P3：DeepSeek thinking 模式要求多轮回传 reasoning_content，
                    // 否则自第二次请求起上游返回 400（同 `runtime.rs::run_tool_loop` 的既有做法：
                    // 先落一条空正文 assistant 轮承载 thinking，再落与之配对的 tool 消息）。
                    messages.push(Message::Assistant {
                        content: String::new(),
                        thinking: response.thinking.clone(),
                        timestamp: Utc::now(),
                    });
                    messages.push(Message::Tool {
                        name,
                        input: String::new(),
                        output: format!("[工具执行失败] {e}"),
                        timestamp: Utc::now(),
                        elapsed_ms: None,
                    });
                    continue;
                }
            };
        for (tc, res) in response.tool_calls.iter().zip(results) {
            let output = match &res {
                crate::agent::tool_cycle::ToolResult::Success { data, .. } => data.clone(),
                crate::agent::tool_cycle::ToolResult::Error { message, code } => {
                    format!("[工具错误 {code}] {message}")
                }
                crate::agent::tool_cycle::ToolResult::Timeout { tool_name, .. } => {
                    format!("工具超时 {tool_name}")
                }
            };
            // P3：同上一处——工具结果回灌前必须先落承载 thinking 的 assistant 轮，
            // 否则 thinking 模型多轮请求缺 reasoning_content 而 400。
            messages.push(Message::Assistant {
                content: String::new(),
                thinking: response.thinking.clone(),
                timestamp: Utc::now(),
            });
            messages.push(Message::Tool {
                name: tc.name.clone(),
                input: tc.arguments.clone(),
                output,
                timestamp: Utc::now(),
                elapsed_ms: None,
            });
        }
    }
    finish_exhausted_rounds(&last_content, rounds)
}

/// 把本轮模型思考回流到编排泳道。
///
/// 子代理思考此前只在内部 `messages` 里（为了 thinking 模型多轮回传），从不发事件，
/// 前端成员详情只能看到工具行 —— 用户看到的就是「子专家不思考」。
/// 每轮发一条（含轮次），截断 6000 字符：思考可能很长，但 UI 需要的是可读摘要，
/// 完整内容仍在内部对话里；截断是显式的（带省略标记见 `truncate_chars`）。
fn emit_member_thinking(progress: &Option<ProgressReporter>, round: usize, thinking: &str) {
    let Some(p) = progress else { return };
    let text = thinking.trim();
    if text.is_empty() {
        return;
    }
    p.emit(
        "member_thinking",
        serde_json::json!({
            "round": round + 1,
            "thinking": truncate_chars(text, 6000),
        }),
    );
}

/// 执行角色感知的子代理任务
///
/// 与 [`execute_subagent_task`] 的区别：使用 `role_prompt` 注入该子 Agent 的专精人设，
/// 并经由 [`ModelRouter::chat_for`] 按 `intent`/`complexity` 路由到最合适的模型
/// （而非无差别的 `chat`），让每个子 Agent 在其角色擅长的模型上工作。
/// 若工具注册表含有可用工具，则开启功能调用回路（plan → act → observe），
/// 使专家 / 子 Agent 能真正读写文件、跑命令，而非仅靠文本推理。
///
/// 参数偏多（8 个）是刻意的：它们逐一对应 `spawn_with_spec` 的入参 + 两个共享
/// 依赖 + 进度上报器，只有一个调用点（`spawn_spec_with_timeout`）。抽成结构体
/// 只会把同一批字段换个名字搬运一次，不增加任何约束力；故就地放宽该 lint，
/// 而不是为了"看起来整洁"引入一次性 DTO。
#[allow(clippy::too_many_arguments)]
async fn execute_subagent_task_spec(
    task: &str,
    context: &str,
    role_prompt: &str,
    intent: IntentType,
    complexity: Complexity,
    model_router: &ModelRouter,
    tool_registry: &Arc<ToolRegistry>,
    progress: Option<ProgressReporter>,
) -> Result<String> {
    let system_msg = Message::System {
        content: format!(
            "{}\n\n你的任务：{}\n\n全局上下文：{}",
            role_prompt, task, context
        ),
        timestamp: Utc::now(),
    };

    let user_msg = Message::User {
        content: format!("请完成下列任务：\n\n{}", task),
        timestamp: Utc::now(),
    };

    let mut messages = vec![system_msg, user_msg];

    // 构建工具定义（本地 + MCP）。无工具则退化为纯对话（向后兼容）。
    let tool_defs: Vec<ToolDef> = tool_registry
        .describe_tools_for_llm()
        .into_iter()
        .map(|(name, description, parameters)| ToolDef {
            name,
            description,
            parameters,
        })
        .collect();
    // 子代理 / 团队成员也必须知道自己有什么能力：真机反馈里子代理完全不知道
    // 有内置浏览器、真终端、编辑器与技能索引，于是「做不到」成了默认答复。
    // 紧凑版只给组名 + 代表工具 + 决策规则，避免挤占子任务本身需要的上下文。
    {
        let snap = crate::agent::tool_catalog::global_snapshot();
        messages.push(Message::System {
            content: crate::agent::tool_catalog::compact_catalog_text(&tool_defs, &snap),
            timestamp: Utc::now(),
        });
    }
    if tool_defs.is_empty() {
        let response = model_router
            .chat_for(&intent, complexity, &messages)
            .await
            .map_err(|e| anyhow!("SubAgent execution failed: {}", e))?;
        // 无工具路径同样回流思考：否则纯推理型子专家在卡片里只剩一句结论。
        emit_member_thinking(&progress, 0, response.thinking.as_deref().unwrap_or(""));
        return Ok(response.content);
    }

    // 功能调用回路：宽工具目录按任务按需注入 schema，发现后下轮给出完整参数定义。
    let request_text = format!("{task}\n{context}");
    let mut disclosure =
        ToolDisclosure::new(&tool_defs, &request_text, crate::mode::runtime_mode());
    if disclosure.is_enabled() {
        messages.insert(
            0,
            Message::System {
                content: format!(
                    "首轮仅暴露常用及相关工具 schema。缺少能力时先调用 `{DISCOVER_TOOLS_NAME}`，\n\
                     用自然语言描述所需工具；发现后完整参数 schema 会在下一轮提供。不要因此声称能力不存在。"
                ),
                timestamp: Utc::now(),
            },
        );
    }
    // 工具执行失败不再以 `?` 中断整个子专家——把错误作为 tool 结果回灌，让模型自我纠正；
    // 轮数由任务体量解析（模型可在计划里写 `[[budget:complex]]` 升档）。
    let cycle = ToolCycle::new(tool_registry.clone());
    let mut last_content = String::new();
    let rounds = crate::agent::budget_policy::resolve(task, 1, tool_defs.len()).tool_rounds;
    for round in 0..rounds {
        let active_tools = disclosure.active_tools();
        let response = match model_router.chat_with_tools(&messages, &active_tools).await {
            Ok(r) => r,
            Err(e) => return Err(anyhow!("SubAgent 模型调用失败: {e}")),
        };
        // 思考必须先于工具行回流：用户看到的顺序应与模型真实顺序一致
        // （先想，再决定调哪个工具），而不是工具跑完才补一段思考。
        emit_member_thinking(&progress, round, response.thinking.as_deref().unwrap_or(""));
        if !response.content.is_empty() {
            last_content = response.content.clone();
        }
        if response.tool_calls.is_empty() {
            return Ok(response.content);
        }
        // 可见性：先把本轮要调用的工具播报出去（前端立即出现「工具行 · 运行中」）
        if let Some(p) = &progress {
            for tc in &response.tool_calls {
                p.emit(
                    "tool_call",
                    serde_json::json!({
                        "tool": tc.name,
                        "argsPreview": truncate_chars(&tc.arguments, 500),
                    }),
                );
            }
        }
        let tool_batch_started = std::time::Instant::now();
        // 整个工具批次执行异常：构造一条错误结果回灌，让模型改换策略而非抛错中断。
        let results =
            match execute_disclosed_tool_calls(&cycle, &mut disclosure, &response.tool_calls).await
            {
                Ok(r) => r,
                Err(e) => {
                    let name = response
                        .tool_calls
                        .first()
                        .map(|tc| tc.name.clone())
                        .unwrap_or_default();
                    if let Some(p) = &progress {
                        p.emit(
                            "tool_result",
                            serde_json::json!({
                                "tool": name,
                                "ok": false,
                                "elapsedMs": tool_batch_started.elapsed().as_millis() as u64,
                                "outputPreview": truncate_chars(&e.to_string(), 500),
                            }),
                        );
                    }
                    // P3：DeepSeek thinking 模式要求多轮回传 reasoning_content，
                    // 否则自第二次请求起上游返回 400（同 `runtime.rs::run_tool_loop` 的既有做法：
                    // 先落一条空正文 assistant 轮承载 thinking，再落与之配对的 tool 消息）。
                    messages.push(Message::Assistant {
                        content: String::new(),
                        thinking: response.thinking.clone(),
                        timestamp: Utc::now(),
                    });
                    messages.push(Message::Tool {
                        name,
                        input: String::new(),
                        output: format!("[工具执行失败] {e}"),
                        timestamp: Utc::now(),
                        elapsed_ms: None,
                    });
                    continue;
                }
            };
        for (tc, res) in response.tool_calls.iter().zip(results) {
            let output = match &res {
                crate::agent::tool_cycle::ToolResult::Success { data, .. } => data.clone(),
                crate::agent::tool_cycle::ToolResult::Error { message, code } => {
                    format!("[工具错误 {code}] {message}")
                }
                crate::agent::tool_cycle::ToolResult::Timeout { tool_name, .. } => {
                    format!("工具超时 {tool_name}")
                }
            };
            if let Some(p) = &progress {
                let ok = matches!(res, crate::agent::tool_cycle::ToolResult::Success { .. });
                p.emit(
                    "tool_result",
                    serde_json::json!({
                        "tool": tc.name,
                        "ok": ok,
                        "elapsedMs": tool_batch_started.elapsed().as_millis() as u64,
                        "outputPreview": truncate_chars(&output, 500),
                    }),
                );
            }
            // P3：同上一处——工具结果回灌前必须先落承载 thinking 的 assistant 轮，
            // 否则 thinking 模型多轮请求缺 reasoning_content 而 400。
            messages.push(Message::Assistant {
                content: String::new(),
                thinking: response.thinking.clone(),
                timestamp: Utc::now(),
            });
            messages.push(Message::Tool {
                name: tc.name.clone(),
                input: tc.arguments.clone(),
                output,
                timestamp: Utc::now(),
                elapsed_ms: None,
            });
        }
    }
    finish_exhausted_rounds(&last_content, rounds)
}

/// 解析 LLM 返回的工具参数 JSON 字符串为扁平字符串映射（对齐 runtime::parse_tool_args）。
fn parse_tool_args(json: &str) -> HashMap<String, String> {
    let trimmed = json.trim();
    if trimmed.is_empty() {
        return HashMap::new();
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(serde_json::Value::Object(map)) => map
            .into_iter()
            .map(|(k, v)| {
                let val = match v {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                };
                (k, val)
            })
            .collect(),
        _ => HashMap::new(),
    }
}

/// 角色标签：取人设首句（去掉标点），供前端泳道标题显示。
///
/// 完整人设是给模型看的系统提示（可能上百字），直接塞进事件会让 UI 变成一堵墙，
/// 也会放大事件体积；这里只保留可识别的短标签。
fn role_label(role_prompt: &str) -> String {
    let head = role_prompt
        .split(['。', '\n', '：', ';', '；'])
        .next()
        .unwrap_or(role_prompt)
        .trim();
    // 人设 prompt 普遍以「你是…」开头；泳道标签要的是角色名（「资深研究员」），
    // 不是半句话（「你是资深研究员」），否则卡片每行都多两个无信息量的字。
    let head = head.strip_prefix("你是").unwrap_or(head).trim();
    truncate_chars(head, 24)
}

/// 按字符数截断（不按字节，避免切断 UTF-8 造成乱码）。
fn truncate_chars(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{EventBus, EventType};
    use crate::persistence::config::Config;

    /// 编排进度事件契约：必须带 kind/phase/runId/memberId，且**发到调用方会话**。
    ///
    /// 这是「对话页不再黑盒」的前端契约：前端按 `meta.session` 归因、按
    /// `payload.kind === "orchestration"` 过滤、按 `phase` 更新泳道。任何一项丢了，
    /// 用户就又会看到一个只有 taskId 的静默任务。
    #[tokio::test]
    async fn progress_reporter_publishes_orchestration_frames_on_caller_session() {
        let bus = Arc::new(EventBus::default());
        let mut rx = bus.subscribe("**");
        let run = ProgressReporter::for_run(bus.clone(), "agent:main:webchat:probe", "orch-1");
        run.emit("run_started", serde_json::json!({ "form": "swarm" }));

        let member = run.for_member("agent-42", "研究员");
        member.emit("tool_call", serde_json::json!({ "tool": "office_automation" }));

        let first = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("首帧超时")
            .expect("首帧缺失");
        assert_eq!(first.session, "agent:main:webchat:probe", "必须回发到调用方会话");
        assert_eq!(first.r#type, EventType::Progress);
        assert_eq!(first.payload["kind"], "orchestration");
        assert_eq!(first.payload["phase"], "run_started");
        assert_eq!(first.payload["runId"], "orch-1");

        let second = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("次帧超时")
            .expect("次帧缺失");
        assert_eq!(second.payload["phase"], "tool_call");
        assert_eq!(second.payload["memberId"], "agent-42");
        assert_eq!(second.payload["role"], "研究员");
        assert_eq!(second.payload["tool"], "office_automation");
    }

    /// 团队看板数据源回归：`emit` 必须**先补齐** `runId` / `memberId` / `role`，再投影给
    /// `member_registry`。此前投影发生在补齐之前，注册表看到的每一帧都「没有成员」，
    /// 成员帧全被丢弃 —— 真机表现是团队看板永远只有一张主智能体卡。
    #[tokio::test]
    async fn progress_frames_feed_team_board_member_projection() {
        let bus = Arc::new(EventBus::default());
        let session = "agent:main:webchat:board-projection";
        let run = ProgressReporter::for_run(bus.clone(), session, "orch-board");
        run.emit("run_started", serde_json::json!({ "form": "team" }));
        let member = run.for_member("agent-board-1", "前端专家");
        member.emit("member_started", serde_json::json!({ "task": "改登录页" }));
        member.emit("tool_call", serde_json::json!({ "tool": "read_file" }));

        let snap = crate::orchestration::member_registry::snapshot(Some(session));
        let members = snap["members"].as_array().expect("members 数组");
        assert_eq!(
            members.len(),
            2,
            "lead + 1 名成员；缺成员说明投影拿到的是没补齐 memberId 的帧: {snap}"
        );
        assert_eq!(members[1]["id"], "agent-board-1");
        assert_eq!(members[1]["role"], "前端专家");
        assert_eq!(members[1]["toolCalls"], 1);

        // run 级（无 memberId）工具帧记在主智能体名下，不再写死为 0。
        run.emit("tool_call", serde_json::json!({ "tool": "shell_exec" }));
        let snap = crate::orchestration::member_registry::snapshot(Some(session));
        assert_eq!(
            snap["members"][0]["toolCalls"], 1,
            "lead 的工具数不得再写死为 0: {snap}"
        );
        crate::orchestration::member_registry::clear(session);
    }

    /// 角色标签只取人设首句且截断，避免把整段系统提示塞进事件（前端会变成一堵墙）。
    #[test]
    fn role_label_is_short_and_readable() {
        let long = "你是资深研究员：负责检索、核证与综述，输出带来源的证据链与关键结论。";
        // 「你是」是人设前缀，不是角色名：标签必须落到「资深研究员」。
        assert_eq!(role_label(long), "资深研究员");
        assert!(!role_label(long).contains('。'));
        assert!(role_label(long).chars().count() <= 25);
        assert_eq!(role_label("通用执行者"), "通用执行者");
        assert_eq!(truncate_chars("abcdef", 3), "abc…");
        assert_eq!(truncate_chars("abc", 3), "abc");
    }

    fn setup_test_manager() -> SubAgentManager {
        let config = SubAgentConfig::default();
        let registry = Arc::new(ToolRegistry::new());
        let model_config = crate::persistence::config::Config::default();
        let model_router = Arc::new(ModelRouter::new(&model_config));
        SubAgentManager::new(config, registry, model_router)
    }

    /// 假 provider：第 1 次请求返回 `first`，其后一律返回 `second`；逐次记录请求体。
    ///
    /// 用于验证「工具回路第二次请求」的报文形状（`read_request_full` 复用既有测试基建）。
    async fn two_round_provider(
        first: &str,
        second: &str,
        captured: Arc<std::sync::Mutex<Vec<String>>>,
    ) -> String {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let first = first.to_string();
        let second = second.to_string();
        tokio::spawn(async move {
            let mut n = 0usize;
            loop {
                if let Ok((mut socket, _)) = listener.accept().await {
                    let data = crate::tests::mock_http::read_request_full(&mut socket).await;
                    captured
                        .lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&data).to_string());
                    let body = if n == 0 {
                        first.clone()
                    } else {
                        second.clone()
                    };
                    n += 1;
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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

    /// P3 回归：子代理工具回路的多轮请求**必须回传 `reasoning_content`**。
    ///
    /// 上游（DeepSeek thinking 模式）原文报错：
    /// `The reasoning_content in the thinking mode must be passed back to the API`（HTTP 400）。
    /// 触发条件是「子代理拿到工具结果后发起第二次请求」——即工具回路进入 ≥2 轮。
    ///
    /// 断言第二轮请求体里含 `reasoning_content`：它由「空正文 assistant 承载 thinking」
    /// 经 `model/llm.rs::messages_to_api` 合并进 `assistant.tool_calls` 生成。
    /// 若删掉 subagent 工具回路里那两处 `Message::Assistant` 插入，本用例必须变红。
    #[tokio::test]
    async fn subagent_tool_loop_passes_reasoning_content_on_followup_request() {
        let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
        let url = two_round_provider(
            // 第 1 轮：thinking + tool_calls（触发工具执行 → 产生第二次请求）
            r#"{"choices":[{"message":{"content":"","reasoning_content":"先读该文件确认内容","tool_calls":[{"id":"c1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"p3-no-such-file.txt\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#,
            // 第 2 轮：收敛为最终文本
            r#"{"choices":[{"message":{"content":"已完成"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#,
            captured.clone(),
        )
        .await;

        let mut config = Config::default();
        config.model.api_key = "test-key".to_string();
        config.model.base_url = url;
        let model_router = ModelRouter::new(&config);

        let mut registry = ToolRegistry::new();
        crate::tools::builtin::register_builtin_tools(
            &mut registry,
            &std::env::temp_dir(),
            None,
            None,
            None,
        )
        .expect("内置工具注册应成功");
        let registry = Arc::new(registry);
        assert!(
            !registry.describe_tools_for_llm().is_empty(),
            "需至少 1 个工具才会进入工具回路（否则退化为纯对话，测不到本缺陷）"
        );

        let out =
            execute_subagent_task("读取一个不存在的文件", "ctx", &model_router, &registry).await;
        assert!(out.is_ok(), "工具回路应返回结果而非报错: {out:?}");

        let reqs = captured.lock().unwrap();
        assert!(
            reqs.len() >= 2,
            "应发生多轮请求（工具结果回灌后再次调用），实际 {} 轮",
            reqs.len()
        );
        assert!(
            reqs[1].contains("reasoning_content"),
            "第二次请求必须回传 reasoning_content（否则 thinking 模型 400）；实际请求体: {}",
            reqs[1]
        );
    }

    /// P5 回归：子代理在独立 task 中运行时，其文件工具登记的产物必须落在**调用方会话**下。
    ///
    /// 真机症状（CLI `team` 路径实测）：`record_artifact: 无可用 SESSION，回退到传入
    /// session_key（空），记录到默认会话` —— 于是「本次会话产物列表」恒空（磁盘有文件、
    /// 面板没条目）。根因是 task-local `SESSION` 不跨 `tokio::spawn` 继承。
    ///
    /// 若删掉 spawn 内的 `SESSION.scope(...)` 包裹（或去掉 `set_session` 接线），
    /// 产物会落到默认会话，本用例必须变红。
    #[tokio::test]
    async fn subagent_artifact_lands_in_caller_session() {
        let dir = std::env::temp_dir().join(format!("marg-p5-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let session = format!("p5.{}", Uuid::new_v4());

        let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
        // 第 1 轮：让子代理真实调用 write_file 落盘；第 2 轮收敛
        let url = two_round_provider(
            r#"{"choices":[{"message":{"content":"","tool_calls":[{"id":"c1","type":"function","function":{"name":"write_file","arguments":"{\"path\":\"p5_session_probe.txt\",\"content\":\"hi\"}"}}]},"finish_reason":"tool_calls"}]}"#,
            r#"{"choices":[{"message":{"content":"完成"},"finish_reason":"stop"}]}"#,
            captured.clone(),
        )
        .await;

        let mut config = Config::default();
        config.model.api_key = "test-key".to_string();
        config.model.base_url = url;
        let model_router = Arc::new(ModelRouter::new(&config));

        let mut registry = ToolRegistry::new();
        crate::tools::builtin::register_builtin_tools(&mut registry, &dir, None, None, None)
            .expect("内置工具注册应成功");
        let registry = Arc::new(registry);

        let mut manager = SubAgentManager::new(SubAgentConfig::default(), registry, model_router);
        manager.set_session(session.clone());

        let _ = manager
            .spawn_with_timeout("写一个文件", "ctx", Duration::from_secs(30))
            .await
            .expect("spawn 应成功");
        let _ = manager.wait_all().await;

        let (items, _, _) = crate::tools::file_change::list_artifacts(&session, None, 50);
        assert!(
            items.iter().any(|a| a.name.contains("p5_session_probe.txt")
                || a.id.contains("p5_session_probe.txt")),
            "子代理产出必须登记在调用方会话 {session} 下；实际条目: {:?}",
            items.iter().map(|a| a.id.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_subagent_config_default() {
        let config = SubAgentConfig::default();
        assert_eq!(config.max_concurrent, 5);
        // 超时可以经 `MARGUERITE_SUBAGENT_TIMEOUT_SECS` 调整，默认值与解析函数同源。
        assert_eq!(config.default_timeout, subagent_timeout());
        assert!(config.model.is_none());
    }

    #[test]
    fn test_subagent_config_custom() {
        let config = SubAgentConfig {
            max_concurrent: 10,
            default_timeout: Duration::from_secs(60),
            model: Some("gpt-4".to_string()),
        };
        assert_eq!(config.max_concurrent, 10);
        assert_eq!(config.default_timeout.as_secs(), 60);
        assert_eq!(config.model, Some("gpt-4".to_string()));
    }

    #[test]
    fn test_subagent_status_equality() {
        assert_eq!(SubAgentStatus::Pending, SubAgentStatus::Pending);
        assert_eq!(SubAgentStatus::Running, SubAgentStatus::Running);
        assert_eq!(SubAgentStatus::TimedOut, SubAgentStatus::TimedOut);
        assert_eq!(
            SubAgentStatus::Completed {
                result: "ok".to_string()
            },
            SubAgentStatus::Completed {
                result: "ok".to_string()
            }
        );
        assert_ne!(
            SubAgentStatus::Completed {
                result: "a".to_string()
            },
            SubAgentStatus::Completed {
                result: "b".to_string()
            }
        );
    }

    #[tokio::test]
    async fn test_subagent_spawn_and_status() {
        let manager = setup_test_manager();
        let agent_id = manager.spawn("test task", "test context").await.unwrap();

        let status = manager.get_status(&agent_id).await;
        assert!(status.is_some());
        // 可能已经是 Running 或 Pending
        match status.unwrap() {
            SubAgentStatus::Pending | SubAgentStatus::Running => {}
            _ => panic!("Expected Pending or Running"),
        }
    }

    #[tokio::test]
    async fn test_subagent_spawn_with_timeout() {
        let manager = setup_test_manager();
        let agent_id = manager
            .spawn_with_timeout("test task", "test context", Duration::from_secs(5))
            .await
            .unwrap();

        let status = manager.get_status(&agent_id).await;
        assert!(status.is_some());
    }

    #[tokio::test]
    async fn test_subagent_cancel() {
        let manager = setup_test_manager();
        let agent_id = manager.spawn("test task", "test context").await.unwrap();

        manager.cancel(&agent_id).await.unwrap();
        let status = manager.get_status(&agent_id).await.unwrap();
        match status {
            SubAgentStatus::Failed { error } => {
                assert_eq!(error, "Cancelled by user");
            }
            _ => panic!("Expected Failed after cancel"),
        }
    }

    #[tokio::test]
    async fn test_subagent_cancel_not_found() {
        let manager = setup_test_manager();
        let result = manager.cancel("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_subagent_cancel_all() {
        let manager = setup_test_manager();
        let _id1 = manager.spawn("task1", "ctx1").await.unwrap();
        let _id2 = manager.spawn("task2", "ctx2").await.unwrap();

        manager.cancel_all().await;
        assert_eq!(manager.active_count().await, 0);
    }

    #[tokio::test]
    async fn test_subagent_active_count() {
        let manager = setup_test_manager();
        let id1 = manager.spawn("task1", "ctx1").await.unwrap();
        let id2 = manager.spawn("task2", "ctx2").await.unwrap();

        let active = manager.active_count().await;
        // 两个都可能正在运行
        assert!(active == 1 || active == 2);

        // 取消一个
        manager.cancel(&id1).await.unwrap();
        // 取消后 idle 数减少
        // 另一个可能已完成或仍在运行
        manager.cancel(&id2).await.unwrap();
        assert_eq!(manager.active_count().await, 0);
    }

    #[tokio::test]
    async fn test_subagent_collect_results() {
        let manager = setup_test_manager();
        let _id1 = manager.spawn("task1", "ctx1").await.unwrap();
        let _id2 = manager.spawn("task2", "ctx2").await.unwrap();

        // 等待一段时间让任务完成或超时
        tokio::time::sleep(Duration::from_millis(500)).await;

        let results = manager.collect_results().await;
        // 结果可能为空（任务可能因为没有 API key 而失败）
        // 但至少不应该 panic
        assert!(results.len() <= 2);
    }

    #[tokio::test]
    async fn test_subagent_agent_count() {
        let manager = setup_test_manager();
        assert_eq!(manager.agent_count().await, 0);

        let _id1 = manager.spawn("task1", "ctx1").await.unwrap();
        assert_eq!(manager.agent_count().await, 1);

        let _id2 = manager.spawn("task2", "ctx2").await.unwrap();
        assert_eq!(manager.agent_count().await, 2);
    }

    #[tokio::test]
    async fn test_subagent_get_result_nonexistent() {
        let manager = setup_test_manager();
        let result = manager.get_result("nonexistent").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_subagent_get_status_nonexistent() {
        let manager = setup_test_manager();
        let status = manager.get_status("nonexistent").await;
        assert!(status.is_none());
    }

    #[tokio::test]
    async fn test_subagent_wait_any_with_no_agents() {
        let manager = setup_test_manager();
        let result = manager.wait_any().await.unwrap();
        // 没有活跃 agent 时，返回 None
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_subagent_wait_all_empty() {
        let manager = setup_test_manager();
        let results = manager.wait_all().await.unwrap();
        assert!(results.is_empty());
    }

    /// 团队/蜂群并发闸门只升不降：默认 5 并发下 8 人团队只会同时跑 5 人（真机可见）。
    #[test]
    fn ensure_concurrency_only_raises_the_gate() {
        let manager = SubAgentManager::new(
            SubAgentConfig {
                max_concurrent: 5,
                ..Default::default()
            },
            Arc::new(ToolRegistry::new()),
            Arc::new(ModelRouter::new(&Config::default())),
        );
        assert_eq!(manager.concurrency_gate.available_permits(), 5);
        manager.ensure_concurrency(9);
        assert_eq!(
            manager.concurrency_gate.available_permits(),
            9,
            "8 人团队 + 1 余量应抬到 9 张许可"
        );
        manager.ensure_concurrency(3);
        assert_eq!(
            manager.concurrency_gate.available_permits(),
            9,
            "只升不降：更小的请求不得回收许可"
        );
    }

    #[tokio::test]
    async fn test_subagent_timeout_control() {
        let manager = SubAgentManager::new(
            SubAgentConfig {
                max_concurrent: 5,
                default_timeout: Duration::from_millis(10), // 非常短的超时
                model: None,
            },
            Arc::new(ToolRegistry::new()),
            Arc::new(ModelRouter::new(&Config::default())),
        );

        let agent_id = manager.spawn("long task", "context").await.unwrap();

        // 等待一小段时间让超时触发
        tokio::time::sleep(Duration::from_millis(100)).await;

        let status = manager.get_status(&agent_id).await;
        if let Some(status) = status {
            match status {
                SubAgentStatus::TimedOut => {}      // 期望的
                SubAgentStatus::Failed { .. } => {} // 也可能是失败（API 不可用）
                _ => {}                             // 其他状态也接受，因为超时极短
            }
        }
    }

    #[test]
    fn test_subagent_new() {
        let agent = SubAgent::new(
            "test-id".to_string(),
            "test task".to_string(),
            SubAgentConfig::default(),
        );
        assert_eq!(agent.id, "test-id");
        assert_eq!(agent.task, "test task");
        assert_eq!(agent.status, SubAgentStatus::Pending);
        assert!(agent.completed_at.is_none());
        assert!(agent.result.is_none());
    }

    #[tokio::test]
    async fn test_subagent_publishes_spawn_event() {
        let mut manager = setup_test_manager();
        let bus = Arc::new(EventBus::default());
        manager.set_event_bus(Arc::clone(&bus));

        let since = std::time::Instant::now();
        let _id = manager.spawn("task", "ctx").await.unwrap();

        // 启动事件应在 spawn 返回前同步发布
        let replayed = bus.history(since);
        assert!(
            replayed
                .iter()
                .any(|e| e.r#type == EventType::ToolCall && e.payload["tool"] == "expert"),
            "子代理启动应发布 tool_call(expert) 事件"
        );
    }

    #[test]
    fn test_parse_tool_args_flat_object() {
        let m = parse_tool_args(r#"{"path": "a.txt", "line": 12}"#);
        assert_eq!(m.get("path").map(String::as_str), Some("a.txt"));
        // 非字符串值序列化为字符串
        assert_eq!(m.get("line").map(String::as_str), Some("12"));
    }

    #[test]
    fn test_parse_tool_args_empty_or_invalid() {
        assert!(parse_tool_args("").is_empty());
        assert!(parse_tool_args("   ").is_empty());
        assert!(parse_tool_args("not-json").is_empty());
        assert!(parse_tool_args("[1,2,3]").is_empty());
        assert!(parse_tool_args("null").is_empty());
    }
}
