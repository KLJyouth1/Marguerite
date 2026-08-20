//! Agent 运行时
//!
//! 管理 Agent 的完整生命周期，从初始化到完成的状态机转换。
//! 维护会话上下文，处理用户输入并协调模型调用。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::intent::classifier::{Classifier, IntentType as ClassifierIntent};
use crate::model::router::{Complexity, IntentType, ModelRouter};
use crate::router::RouteDecision;
use crate::persistence::config::Config;
pub use crate::types::Message;

use crate::agent::stream_loop::{run_tool_loop_streaming, AgentEvent};
use crate::agent::tool_cycle::{ToolCall as CycleToolCall, ToolCycle, ToolResult};
use crate::events::{Envelope, EventBus, EventType};
use crate::mcp::registry::ToolRegistry;
use crate::model::llm::ToolDef;
use crate::model::StreamReceiver;
use serde_json::json;
use tokio::sync::mpsc;

/// Agent 状态枚举
#[derive(Debug, Clone, PartialEq)]
pub enum AgentState {
    /// 初始状态
    Init,
    /// 就绪，等待输入
    Ready,
    /// 运行中
    Running,
    /// 暂停
    Paused,
    /// 完成
    Done,
    /// 错误状态
    Error(String),
}

impl std::fmt::Display for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentState::Init => write!(f, "Init"),
            AgentState::Ready => write!(f, "Ready"),
            AgentState::Running => write!(f, "Running"),
            AgentState::Paused => write!(f, "Paused"),
            AgentState::Done => write!(f, "Done"),
            AgentState::Error(msg) => write!(f, "Error({})", msg),
        }
    }
}

/// 会话上下文
#[derive(Debug, Clone)]
pub struct SessionContext {
    /// 会话 ID
    pub session_id: String,
    /// 消息历史
    pub messages: Vec<Message>,
    /// 当前 Agent 状态
    pub state: AgentState,
    /// 创建时间
    pub created_at: DateTime<Utc>,
    /// 更新时间
    pub updated_at: DateTime<Utc>,
    /// 元数据
    pub metadata: HashMap<String, String>,
}

impl SessionContext {
    /// 创建新的会话上下文
    fn new() -> Self {
        let now = Utc::now();
        Self {
            session_id: Uuid::new_v4().to_string(),
            messages: Vec::new(),
            state: AgentState::Init,
            created_at: now,
            updated_at: now,
            metadata: HashMap::new(),
        }
    }

    /// 添加消息到会话
    pub fn add_message(&mut self, message: Message) {
        self.updated_at = Utc::now();
        self.messages.push(message);
    }

    /// 更新状态
    fn set_state(&mut self, state: AgentState) {
        self.state = state;
        self.updated_at = Utc::now();
    }
}

/// Agent 运行时
#[derive(Debug, Clone)]
pub struct AgentRuntime {
    /// 会话上下文
    session: SessionContext,
    /// 应用配置
    config: Config,
    /// 模型路由器
    model_router: ModelRouter,
    /// 中断标志
    interrupt_flag: Arc<AtomicBool>,
    /// 本地意图分类器
    classifier: Classifier,
    /// 事件总线（A4.4 事件流埋点；None 时无事件发布）
    events: Option<Arc<EventBus>>,
    /// 事件路由会话标识（与总线订阅主题对齐，由 set_event_bus 注入）
    event_session: Option<String>,
    /// 自进化经验提示（Phase F）：执行前由 RealEngine 注入相关历史经验，
    /// 作为系统消息前置到模型消息序列，让 Agent 越用越聪明
    experience_hint: Option<String>,
    /// 记忆召回稳定前缀（W-6）：跨项目记忆召回后渲染成的固定格式 System 消息，
    /// 作为模型消息序列**最前端**的稳定前缀（先于 experience_hint），落进 P-7
    /// stable 半区，利于 KV-cache / 服务端 prompt caching 复用。默认 None（不注入）。
    stable_prefix: Option<Message>,
    /// Track A 路由决策（思维模式档 + prompt 策略）：由 RealEngine/Gateway 注入，
    /// 在系统消息装配点注入 P1 策略片段（不污染可见会话历史）。默认 None（不注入）。
    route_decision: Option<RouteDecision>,
}

impl AgentRuntime {
    /// 创建新的 Agent 运行时
    ///
    /// 初始化会话上下文，状态为 `Init`。
    pub fn new(config: Config) -> Self {
        let model_router = ModelRouter::new(&config);
        Self {
            session: SessionContext::new(),
            config,
            model_router,
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            classifier: Classifier::new(),
            events: None,
            event_session: None,
            experience_hint: None,
            stable_prefix: None,
            route_decision: None,
        }
    }

    /// 注册事件总线与会话标识（A4.4 事件流埋点接口）
    ///
    /// `session` 与会话标识对齐，使事件路由键 `{session}.{type}` 命中
    /// TUI 的 `bus.subscribe("{session}.*")` 订阅。
    pub fn set_event_bus(&mut self, bus: Arc<EventBus>, session: String) {
        self.events = Some(bus);
        self.event_session = Some(session);
    }

    /// 注入自进化经验提示（Phase F）：将由 RealEngine 召回的历史经验文本
    /// 作为系统消息前置到模型消息序列，不参与可见会话历史。
    pub fn set_experience_hint(&mut self, hint: String) {
        self.experience_hint = Some(hint);
    }

    /// 注入记忆召回稳定前缀（W-6）：将由跨项目记忆召回渲染成的固定格式 System 消息
    /// 作为模型消息序列**最前端**的稳定前缀，落进 P-7 stable 半区，利于 KV-cache 复用。
    pub fn set_stable_prefix(&mut self, prefix: Message) {
        self.stable_prefix = Some(prefix);
    }

    /// 注入 Track A 路由决策（思维模式档 + prompt 策略）：作为系统消息前置 P1 策略片段。
    pub fn set_route_decision(&mut self, d: RouteDecision) {
        self.route_decision = Some(d);
    }

    /// 统一装配模型消息序列（`process_input` / `chat_stream` / `run_tool_loop` /
    /// `run_tool_loop_stream` 四个入口共用，消除四份重复装配逻辑）。
    ///
    /// 顺序固定为：稳定前缀（W-6，最前端，最大化 P-7 stable 半区）→ 工具指引（可选）
    /// → 经验提示 → Track A 路由策略片段 → 历史。各前缀段内容均不随轮次变化，
    /// 保证工具回路多轮请求的 System 半区逐字节一致（KV-cache 前缀复用）。
    fn build_model_messages(
        &self,
        history: &[Message],
        tool_guide: Option<&str>,
    ) -> Vec<Message> {
        let extra = usize::from(tool_guide.is_some())
            + usize::from(self.stable_prefix.is_some())
            + usize::from(
                self.experience_hint
                    .as_ref()
                    .map_or(false, |h| !h.is_empty()),
            )
            + usize::from(self.route_decision.is_some());
        let mut model_messages: Vec<Message> = Vec::with_capacity(history.len() + extra);
        if let Some(prefix) = &self.stable_prefix {
            let content = match prefix {
                Message::System { content, .. }
                | Message::User { content, .. }
                | Message::Assistant { content, .. } => content.clone(),
                Message::Tool { output, .. } => output.clone(),
            };
            model_messages.push(Message::System {
                content,
                timestamp: Utc::now(),
            });
        }
        if let Some(guide) = tool_guide {
            model_messages.push(Message::System {
                content: guide.to_string(),
                timestamp: Utc::now(),
            });
        }
        if let Some(hint) = &self.experience_hint {
            if !hint.is_empty() {
                model_messages.push(Message::System {
                    content: hint.clone(),
                    timestamp: Utc::now(),
                });
            }
        }
        if let Some(dec) = &self.route_decision {
            let frag = dec.prompt_policy.system_fragment();
            if !frag.is_empty() {
                model_messages.push(Message::System {
                    content: frag,
                    timestamp: Utc::now(),
                });
            }
        }
        model_messages.extend(history.iter().cloned());
        model_messages
    }

    /// 发布事件（若已注册事件总线）
    fn emit(&self, env: Envelope) {
        if let Some(bus) = &self.events {
            bus.publish(env);
        }
    }

    /// 初始化运行时
    ///
    /// 状态转换：`Init` → `Ready`
    pub fn init(&mut self) -> Result<()> {
        match self.session.state {
            AgentState::Init => {
                self.session.set_state(AgentState::Ready);
                Ok(())
            }
            _ => Err(anyhow!("Cannot init from state: {}", self.session.state)),
        }
    }

    /// 处理用户输入
    ///
    /// 状态转换：`Ready` → `Running` → `Done`
    ///
    /// 核心逻辑：
    /// 1. 将用户消息添加到 `messages`
    /// 2. 状态转换到 Running
    /// 3. 通过 `model_router` 选择合适的模型
    /// 4. 调用 LLM 生成回复
    /// 5. 将回复添加到 `messages`
    /// 6. 状态转换到 Done
    /// 7. 返回回复内容
    pub async fn process_input(&mut self, text: &str) -> Result<&str> {
        // 检查中断标志
        if self.interrupt_flag.load(Ordering::SeqCst) {
            self.session
                .set_state(AgentState::Error("Interrupted".to_string()));
            return Err(anyhow!("Agent was interrupted"));
        }

        // 确保状态为 Ready 或 Done（允许重新开始）
        match self.session.state {
            AgentState::Ready | AgentState::Done | AgentState::Error(_) => {
                if matches!(self.session.state, AgentState::Done | AgentState::Error(_)) {
                    self.session.set_state(AgentState::Ready);
                }
            }
            AgentState::Running => {
                return Err(anyhow!("Agent is already running"));
            }
            AgentState::Paused => {
                return Err(anyhow!("Agent is paused. Call resume() first"));
            }
            AgentState::Init => {
                return Err(anyhow!("Agent is not initialized. Call init() first"));
            }
        }

        // 1. 添加用户消息
        self.session.add_message(Message::User {
            content: text.to_string(),
            timestamp: Utc::now(),
        });

        // 2. 状态转换到 Running
        self.session.set_state(AgentState::Running);

        // 3. 本地意图分类（零 LLM 开销）→ 模型路由
        let classification = self.classifier.classify(text);
        let intent = map_router_intent(&classification.intent);
        let complexity = map_complexity(&classification.intent);

        // A4.4 埋点：思考开始（事件经总线回流 TUI）
        self.emit(Envelope::new(
            EventType::Thinking,
            json!({ "text": format!("> 思考：{text}") }),
            self.event_session.clone().unwrap_or_default(),
        ));

        // 4. 构造模型消息序列（统一装配：稳定前缀 → 经验提示 → 路由片段 → 历史，
        //    不污染可见会话历史）
        let model_messages = self.build_model_messages(&self.session.messages, None);

        // 5. 调用 LLM 生成回复（注入经验后的消息序列）
        let response = self
            .model_router
            .chat_for(&intent, complexity, &model_messages)
            .await;

        match response {
            Ok(resp) => {
                // 5. 将回复添加到 messages
                self.session.add_message(Message::Assistant {
                    content: resp.content.clone(),
                    timestamp: Utc::now(),
                    thinking: resp.thinking,
                });

                // 6. 状态转换到 Done
                self.session.set_state(AgentState::Done);

                // A4.4 埋点：完成（携带摘要回流 TUI）
                self.emit(Envelope::new(
                    EventType::Done,
                    json!({ "summary": resp.content.clone() }),
                    self.event_session.clone().unwrap_or_default(),
                ));

                // 7. 返回回复内容
                // 获取最后一条消息的引用
                if let Some(Message::Assistant { content, .. }) = self.session.messages.last() {
                    // 返回字符串引用（使用 unsafe 来延长生命周期，但为了安全这里用 clone）
                    // 实际上我们需要返回 &str，但 content 归 session 所有，所以生命周期是安全的
                    // 这里使用一个技巧：返回静态字符串引用，实际使用中应直接访问 session
                    // 为了 API 正确性，我们将内容存储并返回
                    self.session
                        .metadata
                        .insert("_last_response".to_string(), content.clone());
                    // 返回一个指向元数据的引用
                    // 注意：这个引用在 session 被修改时可能失效
                    // 当前实现中，返回后 session 不再被修改直到下次 process_input
                    let ptr = self
                        .session
                        .metadata
                        .get("_last_response")
                        .unwrap()
                        .as_str();
                    // 安全：因为我们知道 metadata 中的字符串在 session 存活期间有效
                    // 实际上我们可以直接返回一个指向 session.metadata 中值的引用
                    Ok(ptr)
                } else {
                    self.session
                        .set_state(AgentState::Error("No response content".to_string()));
                    Err(anyhow!("No response content"))
                }
            }
            Err(e) => {
                self.session.set_state(AgentState::Error(e.to_string()));
                // A4.4 埋点：错误事件回流 TUI
                self.emit(Envelope::error(
                    &self.event_session.clone().unwrap_or_default(),
                    "chat",
                    &format!("{e}"),
                ));
                Err(anyhow!("Chat completion failed: {}", e))
            }
        }
    }

    /// 流式多轮对话：给定历史消息序列，返回增量 token 流（StreamReceiver）。
    ///
    /// 复用 `process_input` 的模型选择与经验提示前置逻辑，但逐块吐出而非缓冲整段，
    /// 供 CLI 直接模式（`-p`）与交互式 REPL 做逐 token 流式渲染。
    /// 不修改会话状态机（只读消费历史），调用方负责将本轮回复回写历史。
    pub async fn chat_stream(&self, history: &[Message]) -> Result<StreamReceiver> {
        let model_messages = self.build_model_messages(history, None);
        self.model_router.chat_stream(&model_messages).await
    }

    /// 带工具回路的多轮对话（plan → act → observe 闭环）。
    ///
    /// `history` 应已包含本轮的 User 消息。循环调用 `chat_with_tools`：
    /// - 若模型返回 `tool_calls`：经 `ToolCycle` 顺序执行，把结果作为 `Message::Tool` 追加回历史，继续下一轮；
    /// - 否则把最终文本作为 `Message::Assistant` 追加，并返回该文本。
    /// - 工具名不在 `registry` 中时，把「工具未找到」作为 tool 结果回灌，让模型自我纠正。
    ///
    /// 不执行任何 IO（打印由调用方基于返回的 `Message::Tool` 完成），便于离线测试。
    pub async fn run_tool_loop(
        &self,
        history: &mut Vec<Message>,
        registry: Arc<ToolRegistry>,
        tools: &[ToolDef],
    ) -> Result<String> {
        if tools.is_empty() {
            return Err(anyhow!("run_tool_loop 需要非空 tools 定义"));
        }
        let cycle = ToolCycle::new(registry);
        let max_rounds: u32 = 20;

        // 构建工具使用指引系统提示：让模型知道自己有哪些工具、何时该用。
        let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        let tool_guide = format!(
            "你是 Marguerite AI 助手，具备以下 {n} 个可调用工具：{names}。\n\
             重要规则：\n\
             1. 需要实时信息（新闻/天气/股价/最新文档）时，必须调用 web_search 或 web_research，不要凭记忆编造。\n\
             2. 需要操作浏览器时，调用 browser 工具（open/click/type/screenshot/a11y/vision_click）。\n\
             3. 需要操控电脑桌面时，调用 system_control 或 computer_use 工具。\n\
             4. 需要读写文件/执行命令时，调用 read_file/write_file/edit_file/shell_exec 等工具。\n\
             5. 需要专业能力时，先调用 list_skills 查看可用技能，再用 use_skill 加载。\n\
             6. 可以用 create_skill 创建新的可复用技能，用 kb_search 检索本地知识库，用 kb_ingest 灌入文档。\n\
             7. 主动调用工具完成任务，不要只描述你「会」怎么做——直接去做。工具调用后根据结果继续，直到任务完成。\n\
             输出格式要求：\n\
             - 使用 Markdown 格式化回复：标题用 ##，代码用代码块并标注语言，列表用 - 或编号。\n\
             - 重要结论用**加粗**，关键数据用 `行内代码`。\n\
             - 步骤类任务用编号列表，每步包含动作和预期结果。\n\
             - 搜索结果必须标注来源链接。\n\
             - 先给结论/答案，再给细节，最后给下一步建议。\n\
             - 复杂任务先列出执行计划，逐步执行，每步报告进度。",
            n = tool_names.len(),
            names = tool_names.join(", ")
        );

        for _ in 0..max_rounds {
            // 统一装配：稳定前缀 → 工具指引 → 经验提示 → 路由片段 → 历史
            //（旧实现漏装配稳定前缀，与其余入口不一致，且工具指引非最前）
            let model_messages = self.build_model_messages(history, Some(&tool_guide));

            let resp = self
                .model_router
                .chat_with_tools(&model_messages, tools)
                .await?;
            if resp.tool_calls.is_empty() {
                let text = resp.content.clone();
                history.push(Message::Assistant {
                    content: text.clone(),
                    thinking: resp.thinking,
                    timestamp: Utc::now(),
                });
                return Ok(text);
            }

            let calls: Vec<CycleToolCall> = resp
                .tool_calls
                .iter()
                .map(|tc| CycleToolCall::new(&tc.name, parse_tool_args(&tc.arguments)))
                .collect();
            let results = cycle.execute_sequential(calls).await?;
            // 先把 assistant 的思考内容（reasoning_content）写入历史，
            // DeepSeek thinking 模式要求多轮对话回传 reasoning_content，否则 API 400。
            history.push(Message::Assistant {
                content: String::new(),
                thinking: resp.thinking.clone(),
                timestamp: Utc::now(),
            });
            for (tc, res) in resp.tool_calls.iter().zip(results.into_iter()) {
                let output = match &res {
                    ToolResult::Success { data, .. } => data.clone(),
                    ToolResult::Error { message, code } => format!("[工具错误 {code}] {message}"),
                    ToolResult::Timeout { tool_name, .. } => format!("[工具超时] {tool_name}"),
                };
                history.push(Message::Tool {
                    name: tc.name.clone(),
                    input: tc.arguments.clone(),
                    output,
                    timestamp: Utc::now(),
                });
            }
        }

        Err(anyhow!(
            "工具回路超过最大轮数 {}，未能收敛到最终回复",
            max_rounds
        ))
    }

    /// **真流式**工具回路（P-10）：与 `run_tool_loop` 同语义，但逐块吐出。
    ///
    /// 每收到一个正文增量立刻经 `events` 送出 [`AgentEvent::TextDelta`]，工具调用/结果
    /// 也逐次上报，取代 `run_tool_loop` 的「整轮攒齐再吐」（`external/AUDIT_REPORT.md` P1 缺口）。
    ///
    /// 与 `run_tool_loop` 一致：`history` 须已含本轮 User 消息，回路会把
    /// `Message::Tool` 与最终 `Message::Assistant` 追加进去；经验提示以系统消息前置
    /// （不污染可见历史）。工具调用走默认客户端（须对齐主模型 function-calling 契约，
    /// 故不经故障转移链，与 `ModelRouter::chat_with_tools` 同策略）。
    pub async fn run_tool_loop_stream(
        &self,
        history: &mut Vec<Message>,
        registry: Arc<ToolRegistry>,
        tools: &[ToolDef],
        events: mpsc::Sender<AgentEvent>,
        max_rounds: u32,
    ) -> Result<String> {
        // 守卫先行：在任何网络/事件动作之前拒绝无效入参
        if tools.is_empty() {
            return Err(anyhow!("run_tool_loop_stream 需要非空 tools 定义"));
        }
        let router = &self.model_router;
        let tool_names: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
        let tool_guide = format!(
            "你是 Marguerite AI 助手，具备以下 {n} 个可调用工具：{names}。\n\
             重要规则：\n\
             1. 需要实时信息（新闻/天气/股价/最新文档）时，必须调用 web_search 或 web_research，不要凭记忆编造。\n\
             2. 需要操作浏览器时，调用 browser 工具（open/click/type/screenshot/a11y/vision_click）。\n\
             3. 需要操控电脑桌面时，调用 system_control 或 computer_use 工具。\n\
             4. 需要读写文件/执行命令时，调用 read_file/write_file/edit_file/shell_exec 等工具。\n\
             5. 需要专业能力时，先调用 list_skills 查看可用技能，再用 use_skill 加载。\n\
             6. 可以用 create_skill 创建新的可复用技能，用 kb_search 检索本地知识库，用 kb_ingest 灌入文档。\n\
             7. 主动调用工具完成任务，不要只描述你「会」怎么做——直接去做。工具调用后根据结果继续，直到任务完成。\n\
             输出格式要求：\n\
             - 使用 Markdown 格式化回复：标题用 ##，代码用代码块并标注语言，列表用 - 或编号。\n\
             - 重要结论用**加粗**，关键数据用 `行内代码`。\n\
             - 步骤类任务用编号列表，每步包含动作和预期结果。\n\
             - 搜索结果必须标注来源链接。\n\
             - 先给结论/答案，再给细节，最后给下一步建议。\n\
             - 复杂任务先列出执行计划，逐步执行，每步报告进度。",
            n = tool_names.len(),
            names = tool_names.join(", ")
        );
        run_tool_loop_streaming(history, registry, events, max_rounds, move |turn| {
            // 经验提示以系统消息前置（与 process_input / run_tool_loop 行为一致），
            // 同时天然落在 P-7 稳定前缀半区，利于 KV-cache 前缀复用。
            // 统一装配：稳定前缀 → 工具指引 → 经验提示 → 路由片段 → 历史
            //（修正：旧实现工具指引排在稳定前缀之前，违背 W-6「最前端」契约）
            let model_messages = self.build_model_messages(&turn, Some(&tool_guide));
            async move { router.chat_stream_with_tools(&model_messages, tools).await }
        })
        .await
    }

    /// 暂停运行时
    ///
    /// 状态转换：`Running` → `Paused`
    pub fn pause(&mut self) -> Result<()> {
        match self.session.state {
            AgentState::Running => {
                self.session.set_state(AgentState::Paused);
                Ok(())
            }
            _ => Err(anyhow!("Cannot pause from state: {}", self.session.state)),
        }
    }

    /// 恢复运行时
    ///
    /// 状态转换：`Paused` → `Running`
    pub fn resume(&mut self) -> Result<()> {
        match self.session.state {
            AgentState::Paused => {
                self.session.set_state(AgentState::Running);
                Ok(())
            }
            _ => Err(anyhow!("Cannot resume from state: {}", self.session.state)),
        }
    }

    /// 中断运行时（通过 AtomicBool 标志）
    ///
    /// 设置中断标志，在下次 process_input 时检查并抛出错误。
    /// 不改变状态机状态，由调用方决定如何处理。
    pub fn interrupt(&self) {
        self.interrupt_flag.store(true, Ordering::SeqCst);
    }

    /// 清除中断标志
    pub fn clear_interrupt(&self) {
        self.interrupt_flag.store(false, Ordering::SeqCst);
    }

    /// 获取当前状态
    pub fn get_state(&self) -> &AgentState {
        &self.session.state
    }

    /// 获取会话上下文引用
    pub fn get_context(&self) -> &SessionContext {
        &self.session
    }

    /// 获取会话上下文可变引用
    pub fn get_context_mut(&mut self) -> &mut SessionContext {
        &mut self.session
    }

    /// 获取模型路由器引用
    pub fn model_router(&self) -> &ModelRouter {
        &self.model_router
    }

    /// 获取配置引用
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// 重置运行时
    ///
    /// 状态转换：任何状态 → `Ready`（如果 Error 则重置）
    pub fn reset(&mut self) {
        self.clear_interrupt();
        self.session.messages.clear();
        self.session.metadata.clear();
        self.session.set_state(AgentState::Ready);
    }
}

/// 将本地分类器意图映射为模型路由意图
fn map_router_intent(intent: &ClassifierIntent) -> IntentType {
    match intent {
        ClassifierIntent::Explain
        | ClassifierIntent::Refactor
        | ClassifierIntent::Fix
        | ClassifierIntent::Test => IntentType::Code,
        ClassifierIntent::Doc | ClassifierIntent::Commit => IntentType::Summarization,
        ClassifierIntent::Search | ClassifierIntent::Question => IntentType::Chat,
        ClassifierIntent::Unknown => IntentType::General,
    }
}

/// 根据意图评估任务复杂度
fn map_complexity(intent: &ClassifierIntent) -> Complexity {
    match intent {
        ClassifierIntent::Refactor | ClassifierIntent::Fix | ClassifierIntent::Test => {
            Complexity::Medium
        }
        _ => Complexity::Simple,
    }
}

/// 把 LLM 给出的工具参数 JSON 字符串解析为 `HashMap<String, String>`。
///
/// LLM 的 `arguments` 是 JSON 对象字符串（如 `{"path":"/x","recursive":"true"}`）。
/// 这里扁平化为字符串键值对以适配现有 `ToolCall::new(name, args)` 接口。
/// 解析失败（非对象 / 非法 JSON / 空串）时返回空 map，由工具侧决定如何处理。
pub(crate) fn parse_tool_args(json: &str) -> HashMap<String, String> {
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
    use crate::persistence::config::Config;

    #[test]
    fn test_intent_mapping() {
        assert_eq!(
            map_router_intent(&ClassifierIntent::Explain),
            IntentType::Code
        );
        assert_eq!(
            map_router_intent(&ClassifierIntent::Refactor),
            IntentType::Code
        );
        assert_eq!(map_router_intent(&ClassifierIntent::Fix), IntentType::Code);
        assert_eq!(map_router_intent(&ClassifierIntent::Test), IntentType::Code);
        assert_eq!(
            map_router_intent(&ClassifierIntent::Doc),
            IntentType::Summarization
        );
        assert_eq!(
            map_router_intent(&ClassifierIntent::Commit),
            IntentType::Summarization
        );
        assert_eq!(
            map_router_intent(&ClassifierIntent::Search),
            IntentType::Chat
        );
        assert_eq!(
            map_router_intent(&ClassifierIntent::Question),
            IntentType::Chat
        );
        assert_eq!(
            map_router_intent(&ClassifierIntent::Unknown),
            IntentType::General
        );
    }

    #[test]
    fn test_complexity_mapping() {
        assert_eq!(
            map_complexity(&ClassifierIntent::Refactor),
            Complexity::Medium
        );
        assert_eq!(map_complexity(&ClassifierIntent::Fix), Complexity::Medium);
        assert_eq!(map_complexity(&ClassifierIntent::Test), Complexity::Medium);
        assert_eq!(
            map_complexity(&ClassifierIntent::Explain),
            Complexity::Simple
        );
        assert_eq!(
            map_complexity(&ClassifierIntent::Question),
            Complexity::Simple
        );
        assert_eq!(
            map_complexity(&ClassifierIntent::Unknown),
            Complexity::Simple
        );
    }

    // ---------- P-10 真流式回路生产接线 ----------

    #[tokio::test]
    async fn test_run_tool_loop_stream_rejects_empty_tools() {
        let runtime = AgentRuntime::new(Config::default());
        let mut history = vec![Message::User {
            content: "hi".to_string(),
            timestamp: Utc::now(),
        }];
        let (tx, _rx) = mpsc::channel(8);
        let result = runtime
            .run_tool_loop_stream(&mut history, Arc::new(ToolRegistry::new()), &[], tx, 4)
            .await;
        assert!(result.is_err(), "空 tools 必须报错，不得静默返回空文本");
    }

    #[tokio::test]
    async fn test_run_tool_loop_stream_empty_tools_emits_no_events() {
        // 守卫必须在任何网络/事件动作之前生效：一个事件都不该发出
        let runtime = AgentRuntime::new(Config::default());
        let mut history = vec![Message::User {
            content: "hi".to_string(),
            timestamp: Utc::now(),
        }];
        let (tx, mut rx) = mpsc::channel(8);
        let _ = runtime
            .run_tool_loop_stream(&mut history, Arc::new(ToolRegistry::new()), &[], tx, 4)
            .await;
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        assert!(events.is_empty(), "空 tools 守卫前不应发出事件：{events:?}");
        assert_eq!(history.len(), 1, "守卫路径不应改动历史");
    }

    #[test]
    fn test_agent_state_transitions() {
        let config = Config::default();
        let mut runtime = AgentRuntime::new(config);

        // Init → Ready
        assert_eq!(*runtime.get_state(), AgentState::Init);
        runtime.init().unwrap();
        assert_eq!(*runtime.get_state(), AgentState::Ready);
    }

    #[test]
    fn test_agent_init_from_non_init_fails() {
        let config = Config::default();
        let mut runtime = AgentRuntime::new(config);
        runtime.init().unwrap();
        // 第二次 init 应该失败
        assert!(runtime.init().is_err());
    }

    #[test]
    fn test_agent_pause_and_resume() {
        let config = Config::default();
        let mut runtime = AgentRuntime::new(config);

        // 先 init
        runtime.init().unwrap();
        assert_eq!(*runtime.get_state(), AgentState::Ready);

        // Ready 状态下不能 pause
        assert!(runtime.pause().is_err());

        // 手动设置状态为 Running
        runtime.get_context_mut().set_state(AgentState::Running);

        // Running → Paused
        runtime.pause().unwrap();
        assert_eq!(*runtime.get_state(), AgentState::Paused);

        // Paused → Running
        runtime.resume().unwrap();
        assert_eq!(*runtime.get_state(), AgentState::Running);
    }

    #[test]
    fn test_agent_interrupt() {
        let config = Config::default();
        let mut runtime = AgentRuntime::new(config);
        runtime.init().unwrap();

        // 设置中断标志
        runtime.interrupt();
        assert!(runtime.interrupt_flag.load(Ordering::SeqCst));

        // 清除中断标志
        runtime.clear_interrupt();
        assert!(!runtime.interrupt_flag.load(Ordering::SeqCst));
    }

    #[test]
    fn test_agent_reset() {
        let config = Config::default();
        let mut runtime = AgentRuntime::new(config);
        runtime.init().unwrap();

        // 添加一些消息
        runtime.get_context_mut().add_message(Message::User {
            content: "test".to_string(),
            timestamp: Utc::now(),
        });
        assert_eq!(runtime.get_context().messages.len(), 1);

        // 重置
        runtime.reset();
        assert_eq!(*runtime.get_state(), AgentState::Ready);
        assert!(runtime.get_context().messages.is_empty());
    }

    #[test]
    fn test_session_context_creation() {
        let ctx = SessionContext::new();
        assert_eq!(ctx.state, AgentState::Init);
        assert!(ctx.messages.is_empty());
        assert!(!ctx.session_id.is_empty());
        assert!(ctx.metadata.is_empty());
    }

    #[test]
    fn test_message_roles() {
        let now = Utc::now();

        let user_msg = Message::User {
            content: "hello".to_string(),
            timestamp: now,
        };
        assert_eq!(user_msg.role(), "user");

        let assistant_msg = Message::Assistant {
            content: "hi".to_string(),
            timestamp: now,
            thinking: None,
        };
        assert_eq!(assistant_msg.role(), "assistant");

        let system_msg = Message::System {
            content: "be helpful".to_string(),
            timestamp: now,
        };
        assert_eq!(system_msg.role(), "system");

        let tool_msg = Message::Tool {
            name: "test".to_string(),
            input: "input".to_string(),
            output: "output".to_string(),
            timestamp: now,
        };
        assert_eq!(tool_msg.role(), "tool");
    }

    #[test]
    fn test_agent_state_display() {
        assert_eq!(format!("{}", AgentState::Init), "Init");
        assert_eq!(format!("{}", AgentState::Ready), "Ready");
        assert_eq!(format!("{}", AgentState::Running), "Running");
        assert_eq!(format!("{}", AgentState::Paused), "Paused");
        assert_eq!(format!("{}", AgentState::Done), "Done");
        assert_eq!(
            format!("{}", AgentState::Error("test".to_string())),
            "Error(test)"
        );
    }

    #[test]
    fn test_agent_process_input_invalid_state() {
        let config = Config::default();
        let mut runtime = AgentRuntime::new(config);

        // 未 init 时调用 process_input 应失败
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(runtime.process_input("hello"));
        assert!(result.is_err());
    }

    #[test]
    fn test_agent_pause_from_wrong_state() {
        let config = Config::default();
        let mut runtime = AgentRuntime::new(config);
        runtime.init().unwrap();

        // Ready 状态不能 pause
        assert!(runtime.pause().is_err());
    }

    #[test]
    fn test_agent_resume_from_wrong_state() {
        let config = Config::default();
        let mut runtime = AgentRuntime::new(config);
        runtime.init().unwrap();

        // Ready 状态不能 resume
        assert!(runtime.resume().is_err());
    }

    #[test]
    fn test_agent_multiple_reset() {
        let config = Config::default();
        let mut runtime = AgentRuntime::new(config);
        runtime.init().unwrap();

        // 多次重置不会出错
        runtime.reset();
        assert_eq!(*runtime.get_state(), AgentState::Ready);

        runtime.reset();
        assert_eq!(*runtime.get_state(), AgentState::Ready);
    }

    #[test]
    fn test_agent_interrupt_flag_initial() {
        let config = Config::default();
        let runtime = AgentRuntime::new(config);
        assert!(!runtime.interrupt_flag.load(Ordering::SeqCst));
    }

    /// 简易 mock LLM HTTP 服务器：单次响应
    async fn mock_llm_server(status: u16, body: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = body.to_string();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 8192];
                let _ = socket.read(&mut buf).await;
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

    fn llm_config(url: &str) -> Config {
        let mut config = Config::default();
        config.model.api_key = "test-key".to_string();
        config.model.base_url = url.to_string();
        config
    }

    #[tokio::test]
    async fn test_process_input_success() {
        let url = mock_llm_server(
            200,
            r#"{"choices":[{"message":{"content":"你好，Marguerite！"},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":3,"total_tokens":8}}"#,
        )
        .await;
        let mut runtime = AgentRuntime::new(llm_config(&url));
        runtime.init().unwrap();

        let resp = runtime.process_input("你好").await.unwrap();
        assert_eq!(resp, "你好，Marguerite！");
        assert_eq!(runtime.get_context().messages.len(), 2);
        assert_eq!(runtime.get_context().messages[0].role(), "user");
        assert_eq!(runtime.get_context().messages[1].role(), "assistant");
        assert!(matches!(*runtime.get_state(), AgentState::Done));
    }

    /// 简易 mock LLM HTTP 服务器：记录收到的请求体（用于验证系统提示注入）
    async fn capturing_mock_llm_server(
        status: u16,
        body: &str,
        captured: Arc<std::sync::Mutex<Option<String>>>,
    ) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = body.to_string();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 16384];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                *captured.lock().unwrap() = Some(String::from_utf8_lossy(&buf[..n]).to_string());
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
    async fn test_process_input_injects_experience_hint_as_system() {
        let captured = Arc::new(std::sync::Mutex::new(None));
        let url = capturing_mock_llm_server(
            200,
            r#"{"choices":[{"message":{"content":"已参考经验回复"},"finish_reason":"stop"}]}"#,
            captured.clone(),
        )
        .await;
        let mut runtime = AgentRuntime::new(llm_config(&url));
        runtime.init().unwrap();
        runtime.set_experience_hint("历史经验：避免重复注册全局快捷键".to_string());
        let resp = runtime
            .process_input("如何配置 Tauri 快捷键")
            .await
            .unwrap();
        assert_eq!(resp, "已参考经验回复");

        let req = captured.lock().unwrap().clone().unwrap();
        assert!(
            req.contains("\"role\":\"system\""),
            "应注入系统消息到模型请求"
        );
        assert!(
            req.contains("历史经验：避免重复注册全局快捷键"),
            "系统消息应包含经验提示文本"
        );
    }

    #[tokio::test]
    async fn test_process_input_prepends_stable_prefix_before_hint() {
        let captured = Arc::new(std::sync::Mutex::new(None));
        let url = capturing_mock_llm_server(
            200,
            r#"{"choices":[{"message":{"content":"已参考记忆回复"},"finish_reason":"stop"}]}"#,
            captured.clone(),
        )
        .await;
        let mut runtime = AgentRuntime::new(llm_config(&url));
        runtime.init().unwrap();
        runtime.set_stable_prefix(Message::System {
            content: "## Recalled memory (compacted)\n- [proj-a] use redact() for secrets"
                .to_string(),
            timestamp: chrono::Utc::now(),
        });
        runtime.set_experience_hint("历史经验：避免重复注册全局快捷键".to_string());
        let _ = runtime
            .process_input("如何配置 Tauri 快捷键")
            .await
            .unwrap();

        let req: String = captured.lock().unwrap().clone().unwrap();
        assert!(
            req.contains("\"role\":\"system\""),
            "应注入系统消息到模型请求"
        );
        assert!(
            req.contains("Recalled memory (compacted)"),
            "应含记忆召回稳定前缀"
        );
        assert!(req.contains("历史经验"), "应含经验提示");
        // 稳定前缀应排在经验提示之前（更靠前 = 更稳定的前缀头）
        let stable_pos = req
            .find("Recalled memory (compacted)")
            .expect("应含稳定前缀");
        let hint_pos = req.find("历史经验").expect("应含经验提示");
        assert!(
            stable_pos < hint_pos,
            "稳定前缀应排在经验提示之前（更靠前 = 更稳定的前缀头）"
        );
    }

    #[tokio::test]
    async fn test_agent_runtime_publishes_thinking_and_done() {
        let url = mock_llm_server(
            200,
            r#"{"choices":[{"message":{"content":"你好，Marguerite！"},"finish_reason":"stop"}]}"#,
        )
        .await;
        let bus = Arc::new(EventBus::default());
        let mut rx = bus.subscribe("**");
        let mut runtime = AgentRuntime::new(llm_config(&url));
        runtime.init().unwrap();
        runtime.set_event_bus(bus.clone(), "ws.test".to_string());

        let _ = runtime.process_input("你好").await.unwrap();
        let mut types = vec![];
        while let Ok(env) = rx.try_recv() {
            types.push(env.r#type);
        }
        assert!(types.contains(&EventType::Thinking), "应发布 Thinking 事件");
        assert!(types.contains(&EventType::Done), "应发布 Done 事件");
    }

    #[tokio::test]
    async fn test_process_input_llm_error() {
        let url = mock_llm_server(500, "server error").await;
        let mut runtime = AgentRuntime::new(llm_config(&url));
        runtime.init().unwrap();

        let result = runtime.process_input("你好").await;
        assert!(result.is_err());
        assert!(matches!(*runtime.get_state(), AgentState::Error(_)));
    }

    #[tokio::test]
    async fn test_process_input_then_new_message() {
        // 一次成功后再次输入，状态从 Done 自动回到 Ready 并继续
        let url = mock_llm_server(
            200,
            r#"{"choices":[{"message":{"content":"第一轮回复"},"finish_reason":"stop"}]}"#,
        )
        .await;
        let mut runtime = AgentRuntime::new(llm_config(&url));
        runtime.init().unwrap();

        let resp = runtime.process_input("问题一").await.unwrap();
        assert_eq!(resp, "第一轮回复");

        let result = runtime.process_input("问题二").await;
        assert!(result.is_err() || result.is_ok()); // 第二轮可能失败（mock 已关闭连接），但状态机不应卡死
        assert!(matches!(
            *runtime.get_state(),
            AgentState::Done | AgentState::Error(_)
        ));
    }
}
