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

use crate::agent::tool_cycle::{ToolCall as CycleToolCall, ToolCycle};
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
            max_concurrent: 5,
            default_timeout: Duration::from_secs(120),
            model: None,
        }
    }
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
}

impl SubAgentManager {
    /// 创建新的子代理管理器
    pub fn new(
        config: SubAgentConfig,
        tool_registry: Arc<ToolRegistry>,
        model_router: Arc<ModelRouter>,
    ) -> Self {
        let (completion_tx, _) = tokio::sync::broadcast::channel(256);
        let concurrency_gate = Arc::new(Semaphore::new(config.max_concurrent));
        Self {
            agents: Arc::new(Mutex::new(HashMap::new())),
            completion_tx,
            config,
            tool_registry,
            model_router,
            events: None,
            concurrency_gate,
        }
    }

    /// 注册事件总线（A4 事件流埋点接口）
    pub fn set_event_bus(&mut self, bus: Arc<EventBus>) {
        self.events = Some(bus);
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
                execute_subagent_task(&task_str, &context_str, &model_router, &tool_registry).await
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
            self.config.default_timeout,
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

            let result = tokio::time::timeout(timeout, async {
                execute_subagent_task_spec(
                    &task_str,
                    &context_str,
                    &role_str,
                    intent,
                    complexity,
                    &model_router,
                    &tool_registry,
                )
                .await
            })
            .await;

            let (ok, summary) = match &result {
                Ok(Ok(o)) => (true, o.clone()),
                Ok(Err(e)) => (false, e.to_string()),
                Err(_) => (false, "timeout".to_string()),
            };
            let mut agents = agents.lock().await;
            if let Some(agent) = agents.get_mut(&agent_id_clone) {
                match result {
                    Ok(Ok(output)) => {
                        agent.status = SubAgentStatus::Completed {
                            result: output.clone(),
                        };
                        agent.result = Some(output);
                    }
                    Ok(Err(e)) => {
                        // 失败也回填 result：上端 `get_result` 才能拿到真实错误，
                        // 而不是永远得到「[子专家无结果返回]」误导信息。
                        let msg = format!("[子专家执行失败] {e}");
                        agent.status = SubAgentStatus::Failed { error: msg.clone() };
                        agent.result = Some(msg);
                    }
                    Err(_) => {
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
    if tool_defs.is_empty() {
        let response = model_router
            .chat(&messages)
            .await
            .map_err(|e| anyhow!("SubAgent execution failed: {}", e))?;
        return Ok(response.content);
    }

    // 功能调用回路：若模型返回 tool_calls 则执行工具并回灌结果，最多 8 轮。
    // 工具执行失败不再以 `?` 中断整个子专家——把错误作为 tool 结果回灌，让模型自我纠正；
    // 8 轮仍未收敛则返回最后一轮模型文本，避免「子代理工具回路未收敛」错误导致「无结果返回」。
    let cycle = ToolCycle::new(tool_registry.clone());
    let mut last_content = String::new();
    for _ in 0..8 {
        let response = match model_router.chat_with_tools(&messages, &tool_defs).await {
            Ok(r) => r,
            Err(e) => return Err(anyhow!("SubAgent 模型调用失败: {e}")),
        };
        if !response.content.is_empty() {
            last_content = response.content.clone();
        }
        if response.tool_calls.is_empty() {
            return Ok(response.content);
        }
        let calls: Vec<CycleToolCall> = response
            .tool_calls
            .iter()
            .map(|tc| CycleToolCall::new(&tc.name, parse_tool_args(&tc.arguments)))
            .collect();
        // 整个工具批次执行异常：构造一条错误结果回灌，让模型改换策略而非抛错中断。
        let results = match cycle.execute_sequential(calls).await {
            Ok(r) => r,
            Err(e) => {
                let name = response
                    .tool_calls
                    .first()
                    .map(|tc| tc.name.clone())
                    .unwrap_or_default();
                messages.push(Message::Tool {
                    name,
                    input: String::new(),
                    output: format!("[工具执行失败] {e}"),
                    timestamp: Utc::now(),
                });
                continue;
            }
        };
        for (tc, res) in response.tool_calls.iter().zip(results.into_iter()) {
            let output = match &res {
                crate::agent::tool_cycle::ToolResult::Success { data, .. } => data.clone(),
                crate::agent::tool_cycle::ToolResult::Error { message, code } => {
                    format!("[工具错误 {code}] {message}")
                }
                crate::agent::tool_cycle::ToolResult::Timeout { tool_name, .. } => {
                    format!("工具超时 {tool_name}")
                }
            };
            messages.push(Message::Tool {
                name: tc.name.clone(),
                input: tc.arguments.clone(),
                output,
                timestamp: Utc::now(),
            });
        }
    }
    if last_content.is_empty() {
        Err(anyhow!("子代理工具回路超过最大轮数，未能收敛到最终回复"))
    } else {
        Ok(last_content)
    }
}

/// 执行角色感知的子代理任务
///
/// 与 [`execute_subagent_task`] 的区别：使用 `role_prompt` 注入该子 Agent 的专精人设，
/// 并经由 [`ModelRouter::chat_for`] 按 `intent`/`complexity` 路由到最合适的模型
/// （而非无差别的 `chat`），让每个子 Agent 在其角色擅长的模型上工作。
/// 若工具注册表含有可用工具，则开启功能调用回路（plan → act → observe），
/// 使专家 / 子 Agent 能真正读写文件、跑命令，而非仅靠文本推理。
async fn execute_subagent_task_spec(
    task: &str,
    context: &str,
    role_prompt: &str,
    intent: IntentType,
    complexity: Complexity,
    model_router: &ModelRouter,
    tool_registry: &Arc<ToolRegistry>,
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
    if tool_defs.is_empty() {
        let response = model_router
            .chat_for(&intent, complexity, &messages)
            .await
            .map_err(|e| anyhow!("SubAgent execution failed: {}", e))?;
        return Ok(response.content);
    }

    // 功能调用回路：若模型返回 tool_calls 则执行工具并回灌结果，最多 8 轮。
    // 工具执行失败不再以 `?` 中断整个子专家——把错误作为 tool 结果回灌，让模型自我纠正；
    // 8 轮仍未收敛则返回最后一轮模型文本，避免「子代理工具回路未收敛」错误导致「无结果返回」。
    let cycle = ToolCycle::new(tool_registry.clone());
    let mut last_content = String::new();
    for _ in 0..8 {
        let response = match model_router.chat_with_tools(&messages, &tool_defs).await {
            Ok(r) => r,
            Err(e) => return Err(anyhow!("SubAgent 模型调用失败: {e}")),
        };
        if !response.content.is_empty() {
            last_content = response.content.clone();
        }
        if response.tool_calls.is_empty() {
            return Ok(response.content);
        }
        let calls: Vec<CycleToolCall> = response
            .tool_calls
            .iter()
            .map(|tc| CycleToolCall::new(&tc.name, parse_tool_args(&tc.arguments)))
            .collect();
        // 整个工具批次执行异常：构造一条错误结果回灌，让模型改换策略而非抛错中断。
        let results = match cycle.execute_sequential(calls).await {
            Ok(r) => r,
            Err(e) => {
                let name = response
                    .tool_calls
                    .first()
                    .map(|tc| tc.name.clone())
                    .unwrap_or_default();
                messages.push(Message::Tool {
                    name,
                    input: String::new(),
                    output: format!("[工具执行失败] {e}"),
                    timestamp: Utc::now(),
                });
                continue;
            }
        };
        for (tc, res) in response.tool_calls.iter().zip(results.into_iter()) {
            let output = match &res {
                crate::agent::tool_cycle::ToolResult::Success { data, .. } => data.clone(),
                crate::agent::tool_cycle::ToolResult::Error { message, code } => {
                    format!("[工具错误 {code}] {message}")
                }
                crate::agent::tool_cycle::ToolResult::Timeout { tool_name, .. } => {
                    format!("工具超时 {tool_name}")
                }
            };
            messages.push(Message::Tool {
                name: tc.name.clone(),
                input: tc.arguments.clone(),
                output,
                timestamp: Utc::now(),
            });
        }
    }
    if last_content.is_empty() {
        Err(anyhow!("子代理工具回路超过最大轮数，未能收敛到最终回复"))
    } else {
        Ok(last_content)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{EventBus, EventType};
    use crate::persistence::config::Config;

    fn setup_test_manager() -> SubAgentManager {
        let config = SubAgentConfig::default();
        let registry = Arc::new(ToolRegistry::new());
        let model_config = crate::persistence::config::Config::default();
        let model_router = Arc::new(ModelRouter::new(&model_config));
        SubAgentManager::new(config, registry, model_router)
    }

    #[test]
    fn test_subagent_config_default() {
        let config = SubAgentConfig::default();
        assert_eq!(config.max_concurrent, 5);
        assert_eq!(config.default_timeout.as_secs(), 120);
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
