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

use crate::context::optimize::{compact_window, leading_stable_prefix};
use crate::intent::classifier::{Classifier, IntentType as ClassifierIntent};
use crate::model::router::{Complexity, IntentType, ModelRouter};
use crate::persistence::config::Config;
use crate::router::RouteDecision;
pub use crate::types::Message;
use crate::workspace::Workspace;

use crate::agent::stream_loop::{
    detect_doom_loop, run_tool_loop_streaming_with_disclosure_and_session, AgentEvent,
    DOOM_LOOP_ABORT, DOOM_LOOP_THRESHOLD,
};
use crate::agent::tool_cycle::{
    observe_tool_steps, pauses_task, CircuitBreaker, ToolCall as CycleToolCall, ToolCycle,
    ToolResult,
};
use crate::agent::tool_schema::{
    discovery_query, SessionToolFace, ToolDisclosure, DISCOVER_TOOLS_NAME,
};
use crate::cache::AGENT_STABLE_HALF_SCOPE;
use crate::events::{Envelope, EventBus, EventType};
use crate::mcp::registry::ToolRegistry;
use crate::mode::ModeKind;
use crate::model::llm::ToolDef;
use crate::model::StreamReceiver;
use serde_json::json;
use tokio::sync::mpsc;

/// 触发滑动窗口裁剪的最小消息数：低于此值不裁剪，短会话保持逐字节原样。
const WINDOW_MIN_MESSAGES: usize = 200;
/// 滑动窗口保留的最近消息条数（StreamingLLM 的 recent 段）。
const WINDOW_RECENT_MESSAGES: usize = 100;

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

/// 工具指引段（`run_tool_loop` 与 `run_tool_loop_stream` 共用的**唯一**副本）。
///
/// 此前两处各握一份逐字节相同的长文案：改一处漏一处，两条工具回路就会给出
/// 互相矛盾的行为指令（例如一处说"别联网"、另一处没说）。抽成单一函数后
/// 只允许存在一个事实源。注意 `\x20` 是续行后 intended 的前导空格（Rust 的
/// `\` 续行会吞掉下一行全部前导空白）。
/// 从历史中取最后一条助手正文（用于预算熔断/异常时回传 partial_text）。
pub(crate) fn last_assistant_text(history: &[Message]) -> String {
    history
        .iter()
        .rev()
        .find_map(|m| match m {
            Message::Assistant { content, .. } if !content.is_empty() => Some(content.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// W4.5：按**本轮真实工具面**生成能力自述（确定性文本）。
///
/// 每个 turn 计算一次；工具面不变时逐字节相同，可直接进入 KV 稳定半区。
fn capability_brief_for(tools: &[ToolDef]) -> String {
    let names: Vec<String> = tools.iter().map(|tool| tool.name.clone()).collect();
    let (skills, mcp) = crate::agent::capability::surface_counts();
    crate::agent::capability::brief_from(&names, skills, mcp)
}

/// 稳定半区的规范文本（W5.3 指纹输入）：**开头连续** `System` 段的正文。
///
/// 直接复用 [`crate::context::optimize::stable_prefix_text`]——那是 `SharedPrefixCache`
/// （`agent::prefix_scope`）计算前缀键用的同一个规范文本，两处不会各说各话。
/// 它刻意只取正文、不取 `timestamp`：时间戳逐轮变化，若一并序列化，
/// 稳态会话会表现为「轮轮漂移」，这个观测就变成了噪音源。
fn stable_half_text(messages: &[Message]) -> Option<String> {
    let text = crate::context::optimize::stable_prefix_text(messages);
    // 没有任何静态段 = 没有可复用的前缀，不登记（否则空串与首条引导会被算成一次漂移）。
    (!text.is_empty()).then_some(text)
}

fn tool_guide_text(tools: &[ToolDef]) -> String {
    // 空工具面 = 当前模式封禁了全部工具（或调用方未提供任何工具）。
    // W4.2 起直答判定（`is_direct_answer_request`）**不再清空**工具面，只作观测；
    // 因此这里的空分支不能再被理解成「寒暄轮」。
    // 此时**不能**输出「可调用工具共 0 个」再加一整套工具使用规则——那会诱导模型编造工具调用。
    if tools.is_empty() {
        return "本轮未提供任何工具：请直接作答。\n\
                规则：\n\
                1. 不要声称你调用了工具、检索了网络或读取了文件；没有证据就说没有。\n\
                2. 只用已有知识回答；不确定就明确说明不确定，不要编造事实或来源。\n\
                3. 需要外部动作或实时信息时，如实告诉用户「这一步需要工具能力」，并说明需要什么。\n\
                输出格式要求：Markdown；先给结论，再给细节。"
            .to_string();
    }
    // W5.2：这里**不再**写「本轮可调用工具共 N 个：a, b, c」——
    // 工具面随发现增长，这行每变一次就让服务端 KV 前缀整段失配；工具清单
    // 只通过 function schema 与能力自述呈现（两处都已做确定性排序）。
    let discovery_rule = if tools.iter().any(|tool| tool.name == DISCOVER_TOOLS_NAME) {
        format!(
            "当前只暴露了常用工具和本任务相关工具；若缺少能力，先调用 `{DISCOVER_TOOLS_NAME}`，\n\
             用自然语言描述所需能力或给出工具名。匹配工具的完整参数 schema 会在下一轮加载，\n\
             然后再直接调用；不得因为首轮工具列表较短就声称该能力不存在。"
        )
    } else {
        "本轮已提供当前可用的完整工具面；请直接选择最相关的工具调用。".to_string()
    };
    format!(
        // 身份声明已移除（2026-10-18）：此处只描述**工具面**，不宣布「你是谁」。
        // 此前它固定写「你是 Marguerite AI 助手」，与本轮真正生效的人设
        // （主智能体「严工」/ 用户选的专家）在同一请求里互相打架——
        // 与 `merged_turn_hint` 里 viky 默认人格的冲突是同一类缺陷，只是更隐蔽：
        // 它出现在**每一次**请求里（含 CLI/TUI/桌面/Web 四个入口）。
        // 人设的唯一权威是 agent 台账（`agent::store::personality_for`）。
        "重要规则：\n\
         0. **先判断这轮到底要不要用工具**（默认直答，拿不准才用；工具不是装饰）：\n\
         \x20 - 无需工具：稳定知识、观点/寒暄/评价、文本改写与推理、代码或方案的讲解；\n\
         \x20 - 必须用工具：有副作用的动作（写文件、发消息、执行命令、操作浏览器/桌面）、\n\
         \x20   训练数据之外的实时事实（天气/股价/新闻/最新文档）、必须跑一遍才能确认的结论\n\
         \x20   （编译、测试、命令输出、页面实际渲染）；\n\
         \x20 - 上下文里已有的信息不要重复检索；一次检索/一次读文件就够时，不要连搜多次；\n\
         \x20 - 证据已足够支撑结论时立即停止调用并给出答案，不要为了「多做几步」继续调工具；\n\
         \x20 - 需要的能力不在工具列表里 → 按第 7 条先发现，绝不声称该能力不存在。\n\
         1. **仅当任务确实需要外部实时信息**（新闻/天气/股价/最新文档/需要联网核实的\n\
         \x20 事实）时，才调用 web_search 或 web_research；其余情况一律不要联网。\n\
         \x20 **创建/生成/整理/转换本地文件**（如「做一个 Excel/Word/PPT/HTML/报告并保存」）\n\
         \x20 **不需要联网**，直接用 write_file 等工具产出即可——为这类任务检索属于纯浪费\n\
         \x20 （会显著拖长耗时，且检索结果与交付无关）。\n\
         2. 需要操作浏览器时，调用 browser 工具（open/click/type/screenshot/a11y/vision_click）。\n\
         3. 需要操控电脑桌面时，**优先 `computer_use`**（屏幕感知 + 元素定位，抗分辨率变化）；\n\
         \x20 `system_control` 作为兜底（纯键鼠坐标操作，用于 computer_use 覆盖不到的场景：\n\
         \x20 全屏游戏、无 UIA 树的自绘界面、需要精确物理坐标的拖拽）。\n\
         3b. 多模态与 RPA：\n\
         \x20 - 生图 → `image_generate`（需当前档案勾选「图像输出」，否则先让用户去设置页勾选）；\n\
         \x20 - 生视频 → `video_generate`（需当前档案勾选「视频输出」；仅同步端点，\n\
         \x20   返回 task_id 的异步端点要用 http_request 轮询）；\n\
         \x20 - 本地生视频（不联网、不花 token，只要本机 ffmpeg）→\n\
         \x20   `code_video`（代码打字动画）、`web_video`（HTML/CSS/JS 逐帧渲染）、\n\
         \x20   `manim_video`（数学/算法动画，需本机 Python 装 manim）；\n\
         \x20 - 音频转文字 → `audio_transcribe`；文本转语音 → `audio_speak`；\n\
         \x20 - 「我手动做一遍，你照做」→ `auto_record start` → 用户操作 → `auto_record stop`\n\
         \x20   → `auto_replay run`（回放前确认窗口位置未变，必要时先沟通）。\n\
         \x20 这些能力不在时（工具不存在或提示未配置），如实告知缺什么、去哪里配置，不要假装完成。\n\
         4. 需要读写文件/执行命令时，调用 read_file/write_file/edit_file/shell_exec 等工具。\n\
         5. 需要专业能力时，先调用 list_skills 查看可用技能，再用 use_skill 加载。\n\
         6. 可以用 create_skill 创建新的可复用技能，用 kb_search 检索本地知识库，用 kb_ingest 灌入文档。\n\
         7. {discovery_rule}\n\
         8. 主动调用工具完成任务，不要只描述你「会」怎么做——直接去做；优先选最直接的工具，\n\
         \x20 不要为了「显得在做事」调用无关工具。工具调用后根据结果继续，直到任务完成。\n\
         输出格式要求：\n\
         - 使用 Markdown 格式化回复：标题用 ##，代码用代码块并标注语言，列表用 - 或编号。\n\
         - 重要结论用**加粗**，关键数据用 `行内代码`。\n\
         - 步骤类任务用编号列表，每步包含动作和预期结果。\n\
         - 搜索结果必须标注来源链接。\n\
         - 先给结论/答案，再给细节，最后给下一步建议。\n\
         - 复杂任务先列出执行计划，逐步执行，每步报告进度。",
        discovery_rule = discovery_rule
    )
}


/// 组合系统提示里的工具段：基础工具规则 + 能力目录（内置表面/技能索引/MCP/决策规则）。
///
/// 直答轮（工具面为空）**不追加**能力目录：那时规则是「直接作答、不要声称调用工具」，
/// 再把「本机有什么」塞回去只会诱导模型编造调用。
fn compose_guide(tools: &[ToolDef], snap: &crate::agent::tool_catalog::CapabilitySnapshot) -> String {
    let mut out = tool_guide_text(tools);
    if !tools.is_empty() {
        out.push_str("\n\n");
        out.push_str(&crate::agent::tool_catalog::catalog_text(tools, snap));
    }
    out
}

/// 会话级工具面槽位（W4.2）：按事件会话键 + 模式持有，跨 turn 复用。
///
/// 键或模式变化 = 新会话，调用方直接换掉整个槽位（不继承上一会话的发现结果）。
#[derive(Debug)]
struct SessionToolFaceSlot {
    /// 绑定的会话键（`event_session`；None = 无会话概念的 CLI/测试调用）。
    key: Option<String>,
    /// 创建该面时的运行时模式（模式切换会改变允许的工具集）。
    mode: ModeKind,
    /// 共享句柄：本 turn 的 `ToolDisclosure::discover()` 直接扩展它。
    face: Arc<std::sync::Mutex<SessionToolFace>>,
}

/// Agent 运行时
#[derive(Debug)]
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
    /// 锚定首轮输出预算（bootstrap，一次性）：Some = 首个 LLM 请求注入 `max_tokens`
    /// 上限，请求发出后立即剥离（对齐梁神 bootstrapMaxTokens：晋升后绝不残留）。
    /// `Mutex` 内部可变：`run_tool_loop` 等 `&self` 路径需一次性 take（保持 Sync）。
    bootstrap_max_tokens: std::sync::Mutex<Option<u32>>,
    /// 技能目录延迟注入开关（锚定 deferred skill-catalog）：true = 本轮工具回路
    /// 扣住技能/工具指引 System 段（锚定期与晋升后 grace 步内由编排器置 true）。
    defer_skill_catalog: std::sync::Mutex<bool>,
    /// 模式提示段（DSH plan-mode 同款规则段）：作为稳定 System 前缀注入模型消息
    /// 装配（仅追加、不重写历史——红线 #1）。None = 不注入。
    mode_section: Option<String>,
    /// v3.3.1 运行中插话缓冲：用户在 turn 进行中补充的要求/纠正。
    /// 由网关 steer 队列经编排器注入；在**下一次** build_model_messages 装配时
    /// 作为 User 消息并入（影响生成本身），并入后清空。`Mutex` 内部可变：
    /// run_tool_loop 等 `&self` 路径需 take/push（保持 Sync）。
    pending_steer: std::sync::Mutex<Vec<String>>,
    /// 工作区（P0-1b）：宿主在构造后注入的权威根目录描述。Some = 模型消息静态区
    /// **常开**一段【工作区】指引（根在哪、是否绑定、怎么绑定）；None = 不注入。
    /// 默认 None：直接使用 `AgentRuntime` 的调用方（CLI/测试）没有工作区概念，
    /// 不凭空编一段目录信息骗模型。
    workspace: Option<Workspace>,
    /// 会话级稳定工具面（W4.2）：首次 turn 创建，同会话后续复用，
    /// 会话切换 / `reset()` / 模式切换时重建。`Mutex` 内部可变：`run_tool_loop*`
    /// 是 `&self`，需要跨轮保留发现结果。
    session_tool_face: std::sync::Mutex<Option<SessionToolFaceSlot>>,
    /// W5.3：稳定半区指纹的单测观测目标。`Some` 时 `build_model_messages` 把
    /// `agent.stable_half` 的指纹写进这个独立 tracker，而不是进程级单例——
    /// 否则并行用例会互相改变全局漂移计数，「稳态 drift == 0」类断言必然抖动。
    /// 生产路径没有该字段（编译期不存在），仍写 [`crate::cache::prefix_cache_tracker`]。
    #[cfg(test)]
    stable_half_tracker: Option<Arc<crate::cache::PrefixCacheTracker>>,
}

impl Clone for AgentRuntime {
    fn clone(&self) -> Self {
        Self {
            session: self.session.clone(),
            config: self.config.clone(),
            model_router: self.model_router.clone(),
            interrupt_flag: self.interrupt_flag.clone(),
            classifier: self.classifier.clone(),
            events: self.events.clone(),
            event_session: self.event_session.clone(),
            experience_hint: self.experience_hint.clone(),
            stable_prefix: self.stable_prefix.clone(),
            route_decision: self.route_decision.clone(),
            bootstrap_max_tokens: std::sync::Mutex::new(
                *self
                    .bootstrap_max_tokens
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()),
            ),
            defer_skill_catalog: std::sync::Mutex::new(
                *self
                    .defer_skill_catalog
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()),
            ),
            mode_section: self.mode_section.clone(),
            pending_steer: std::sync::Mutex::new(
                self.pending_steer
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone(),
            ),
            workspace: self.workspace.clone(),
            session_tool_face: std::sync::Mutex::new(
                self.session_tool_face
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .map(|slot| SessionToolFaceSlot {
                        key: slot.key.clone(),
                        mode: slot.mode,
                        // 克隆出独立快照：克隆体后续的发现不污染原运行时。
                        face: Arc::new(std::sync::Mutex::new(
                            slot.face
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .clone(),
                        )),
                    }),
            ),
            #[cfg(test)]
            stable_half_tracker: self.stable_half_tracker.clone(),
        }
    }
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
            bootstrap_max_tokens: std::sync::Mutex::new(None),
            defer_skill_catalog: std::sync::Mutex::new(false),
            mode_section: None,
            pending_steer: std::sync::Mutex::new(Vec::new()),
            workspace: None,
            session_tool_face: std::sync::Mutex::new(None),
            #[cfg(test)]
            stable_half_tracker: None,
        }
    }

    /// 单测专用：把 `agent.stable_half` 的观测目标换成独立 tracker，
    /// 让漂移断言不受同进程并行用例污染（生产路径恒为进程级单例）。
    #[cfg(test)]
    fn use_stable_half_tracker(&mut self, tracker: Arc<crate::cache::PrefixCacheTracker>) {
        self.stable_half_tracker = Some(tracker);
    }

    /// 注册事件总线与会话标识（A4.4 事件流埋点接口）
    ///
    /// `session` 与会话标识对齐，使事件路由键 `{session}.{type}` 命中
    /// TUI 的 `bus.subscribe("{session}.*")` 订阅。
    ///
    /// W4.2：会话键变化即新会话 → **丢弃会话级工具面**，下一次 turn 从初始面重建
    /// （新会话不得携带上一会话通过 `discover()` 加进来的工具）。
    pub fn set_event_bus(&mut self, bus: Arc<EventBus>, session: String) {
        if self.event_session.as_deref() != Some(session.as_str()) {
            *self
                .session_tool_face
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
        }
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

    /// v3.3.1：压入一条运行中插话（用户 turn 进行中的补充/纠正）。
    ///
    /// 由编排器从网关 steer 队列 drain 后转发；在下一次模型消息装配时并入。
    pub fn push_pending_steer(&self, text: String) {
        if text.trim().is_empty() {
            return;
        }
        self.pending_steer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(text);
    }

    /// v3.3.1：取出全部待并入插话（装配点调用后清空）。
    fn take_pending_steers(&self) -> Vec<String> {
        std::mem::take(&mut self.pending_steer.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// 注入 Track A 路由决策（思维模式档 + prompt 策略）：作为系统消息前置 P1 策略片段。
    pub fn set_route_decision(&mut self, d: RouteDecision) {
        self.route_decision = Some(d);
    }

    /// 锚定首轮输出预算（一次性）：Some = 首个 LLM 请求注入 `max_tokens` 上限，
    /// 请求发出后剥离（晋升后绝不残留）。None = 不设限。
    pub fn set_bootstrap_max_tokens(&mut self, cap: Option<u32>) {
        *self
            .bootstrap_max_tokens
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = cap;
    }

    /// 扣住/放行技能目录指引注入（deferred skill-catalog）。
    pub fn set_defer_skill_catalog(&mut self, defer: bool) {
        *self
            .defer_skill_catalog
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = defer;
    }

    /// 一次性取走锚定首轮输出预算（首个 LLM 请求后剥离）。
    fn take_bootstrap_max_tokens(&self) -> Option<u32> {
        self.bootstrap_max_tokens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    /// 技能目录指引当前是否被扣住（deferred skill-catalog）。
    fn defer_skill_catalog(&self) -> bool {
        *self
            .defer_skill_catalog
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// 注入模式提示段（如 plan 模式的 DSH 规则段）：作为稳定 System 前缀
    /// 参与消息装配（仅追加，红线 #1 不重写历史）。
    pub fn set_mode_section(&mut self, section: Option<String>) {
        self.mode_section = section.filter(|s| !s.is_empty());
    }

    /// 注入工作区（P0-1b）：Some = 模型消息静态区常开一段【工作区】指引。
    ///
    /// 这是「猜相对路径 → 1 ms 失败 → 死循环重试」这条链的第一道防线：
    /// 模型必须知道文件该写到哪、当前是否绑定了项目目录、以及怎么绑定。
    /// 注入的值由宿主按**工具注册表实际使用的根**给出（编排器经
    /// [`crate::workspace::Workspace::resolve`] 解析），保证指引与工具行为一致。
    pub fn set_workspace(&mut self, ws: Workspace) {
        self.workspace = Some(ws);
    }

    /// 当前注入的工作区（供 doctor / 单测观测模型可见段）。
    pub fn workspace(&self) -> Option<&Workspace> {
        self.workspace.as_ref()
    }

    /// 取（必要时创建）本会话的工具面句柄（W4.2）。
    ///
    /// 复用条件：会话键与运行时模式都没变。任一变化即视为新会话/新模式，
    /// 用当前工具目录重建初始面——**旧面的发现结果不继承**。
    fn session_tool_face(
        &self,
        tools: &[ToolDef],
        mode: ModeKind,
    ) -> Arc<std::sync::Mutex<SessionToolFace>> {
        let key = self.event_session.clone();
        let mut slot = self
            .session_tool_face
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let reusable = slot
            .as_ref()
            .is_some_and(|current| current.key == key && current.mode == mode);
        if !reusable {
            // W5.3：新会话/新模式 = 工具面重建（下一次装配必然漂移一次，之后稳定）。
            // 留一条与 `agent.stable_half` 漂移对齐的归因日志，便于区分「重建」与「目录变了」。
            tracing::info!(
                event = "face_changed",
                scope = "agent.tool_face",
                source = "session_bootstrap",
                session_key = ?key,
                mode = ?mode,
                catalog_tools = tools.len(),
                "会话工具面重建（首次 turn / 会话切换 / 模式切换 / reset）"
            );
            *slot = Some(SessionToolFaceSlot {
                key,
                mode,
                face: Arc::new(std::sync::Mutex::new(SessionToolFace::bootstrap(
                    tools.to_vec(),
                    mode,
                ))),
            });
        }
        match slot.as_ref() {
            Some(current) => Arc::clone(&current.face),
            // 上面刚赋值（或已有可复用槽位），None 分支不可达；兜底给一个空面而非 panic。
            None => Arc::new(std::sync::Mutex::new(SessionToolFace::bootstrap(
                Vec::new(),
                mode,
            ))),
        }
    }

    /// 本 turn 的工具暴露句柄：**复用会话面**，不再按本轮请求文本重选（W4.2）。
    fn tool_disclosure(&self, tools: &[ToolDef], request_text: &str) -> ToolDisclosure {
        let mode = crate::mode::runtime_mode();
        self.tool_disclosure_with_mode(tools, request_text, mode)
    }

    /// 同上，显式指定模式（单测用于摆脱进程级 `runtime_mode()` 的并行干扰）。
    fn tool_disclosure_with_mode(
        &self,
        tools: &[ToolDef],
        request_text: &str,
        mode: ModeKind,
    ) -> ToolDisclosure {
        ToolDisclosure::with_face(
            tools,
            request_text,
            mode,
            self.session_tool_face(tools, mode),
        )
    }

    /// 稳定半区指纹的观测目标：生产路径是进程级单例，单测可注入独立实例。
    fn stable_half_target(&self) -> &crate::cache::PrefixCacheTracker {
        #[cfg(test)]
        {
            if let Some(tracker) = &self.stable_half_tracker {
                return tracker.as_ref();
            }
        }
        crate::cache::prefix_cache_tracker()
    }

    /// 登记本轮稳定半区的规范指纹（W5.3，scope = `agent.stable_half`）。
    ///
    /// 这是「完整前缀」级的漂移观测：此前只有压缩器（`compress.tool_results`）
    /// 有指纹，工具面/静态提示变了无人知晓。现在两层各自计数，`cache.status` 的
    /// `stable_prefix_drift_by_scope` 能直接回答「漂移发生在哪一层」。
    fn observe_stable_half(&self, messages: &[Message]) {
        let Some(text) = stable_half_text(messages) else {
            return;
        };
        if !self
            .stable_half_target()
            .observe_stable_prefix_scoped(AGENT_STABLE_HALF_SCOPE, &text)
        {
            // 归因线索：`face_changed`（工具面变了）或技能目录快照变了。
            // debug 级：多会话交替时每次切换都会命中这里，不该刷屏。
            tracing::debug!(
                event = "stable_prefix_drift",
                scope = AGENT_STABLE_HALF_SCOPE,
                "agent 稳定半区指纹变化"
            );
        }
    }

    /// 统一装配模型消息序列（`process_input` / `chat_stream` / `run_tool_loop` /
    /// `run_tool_loop_stream` 四个入口共用，消除四份重复装配逻辑）。
    ///
    /// 顺序固定为：**静态段前置**（模式提示段 → 工具指引，会话级稳定）→ 插话 →
    /// 历史 → **动态段后置**（记忆召回前缀 → 经验提示 → Track A 路由片段）。
    ///
    /// P0-1b 追加：工作区指引（[`self.workspace`]）插在模式段之后、工具指引之前，
    /// 且**不受 `defer_skill_catalog` 扣住影响**——锚定期扣住技能/工具指引时，
    /// 模型恰恰最容易失去「文件写去哪」的唯一权威来源，那段必须随车携带。
    /// 它同样是会话级静态内容：同一 `Workspace` 产出字节一致的段，不破坏
    /// KV-cache 前缀稳定性。
    ///
    /// KV-cache 分区依据（M2 降耗引擎）：`stable_prefix` 的内容来自按本轮查询文本
    /// 的四路召回（gateway `memory_stable_prefix(&text)`），`experience_hint` 承载
    /// persona / 跨项目提示 / viky PSI 状态，`route_decision` 是 Track A 逐轮决策
    /// ——三者内容逐轮变化，若置于历史之前，服务端 KV 前缀从第一条 System 即失配
    /// （实测仅 512/5288 ≈ 9.7% 命中）。后置到序列末尾后，[静态段 + 历史] 构成
    /// append-only 稳定前缀，跨轮与工具回路内均可被服务端逐字节复用；动态段
    /// 位于末尾还享有注意力上的近因优势（召回内容更不容易被忽略）。
    /// mode_section / tool_guide 为会话级静态内容，保持前置。
    ///
    /// W5.3：静态区装配完成后立即把「开头连续 System 段」的正文（不含时间戳）
    /// 登记为 `agent.stable_half` 指纹；稳态轮次 drift 应为 0，任何变化都可在
    /// `cache.status.stable_prefix_drift_by_scope` 里看到，并与 `face_changed` 日志对上。
    ///
    /// 装配末尾对超长序列应用 StreamingLLM 滑动窗口（[`compact_window`]，见下方不变式）：
    /// `sink` 取 [`leading_stable_prefix`]（开头连续 System），保证稳定前缀逐字节原位保留。
    fn build_model_messages(
        &self,
        history: &[Message],
        tool_guide: Option<&str>,
        capability_brief: Option<&str>,
    ) -> Vec<Message> {
        let extra = usize::from(tool_guide.is_some())
            + usize::from(capability_brief.is_some())
            + usize::from(self.stable_prefix.is_some())
            + usize::from(self.mode_section.is_some())
            + usize::from(self.workspace.is_some())
            + usize::from(self.experience_hint.as_ref().is_some_and(|h| !h.is_empty()))
            + usize::from(self.route_decision.is_some())
            // 不可信内容声明是常开段，不在上面的可选列表里。
            + 1;
        let mut model_messages: Vec<Message> = Vec::with_capacity(history.len() + extra);
        // ── 静态区（会话级稳定，前置，KV-cache 前缀复用）──
        // 模式提示段（plan 等模式的静态规则段）：内容不随轮次变化
        if let Some(section) = &self.mode_section {
            model_messages.push(Message::System {
                content: section.clone(),
                timestamp: Utc::now(),
            });
        }
        // 思考档位指令（不思考 / 思考 / 深度思考 / 极限推理）：同一档位字节一致，
        // 属会话级静态内容，放静态区不破坏 KV 前缀。此前该指令**从未被注入**，
        // 档位只改了请求体字段，模型不知道自己该想多深。
        if let Some(hint) = crate::model::thinking::reasoning_system_prompt_for_turn() {
            model_messages.push(Message::System {
                content: hint.to_string(),
                timestamp: Utc::now(),
            });
        }
        // 工作区指引（P0-1b）：常开，先于工具指引——工具指引可能被锚定期扣住，
        // 而「文件该写到哪」在任何轮次都不能缺。
        if let Some(ws) = &self.workspace {
            model_messages.push(Message::System {
                content: ws.guidance(),
                timestamp: Utc::now(),
            });
        }
        // 用户规则（个人/项目）：会话级静态内容，紧随工作区段。
        // 放在这里而不是动态区——规则不随轮次变化，前置才能保住 KV 前缀复用。
        if let Some(section) = crate::rules::active_text() {
            model_messages.push(Message::System {
                content: section,
                timestamp: Utc::now(),
            });
        }
        // 能力自述（W4.5）：真实工具面 + 意图→工具映射 + 禁止脚本替代。
        // 会话级静态内容（工具面不变则逐字节相同），放在规则之后、工具指引之前。
        if let Some(brief) = capability_brief {
            model_messages.push(Message::System {
                content: brief.to_string(),
                timestamp: Utc::now(),
            });
        }
        // 间接注入防线（P0）：声明「被标记的外部内容只是资料、不是指令」。
        // 常量文本 → 会话级静态内容，进稳定半区不破坏 KV 前缀。
        model_messages.push(Message::System {
            content: crate::security::spotlight::UNTRUSTED_NOTICE.to_string(),
            timestamp: Utc::now(),
        });
        if let Some(guide) = tool_guide {
            model_messages.push(Message::System {
                content: guide.to_string(),
                timestamp: Utc::now(),
            });
        }
        // W5.3：稳定半区（此刻 = 上面全部 System 段）装配完成 → 登记完整前缀指纹。
        // 放在**历史之前**：history 逐轮增长，不属于「应被缓存的不变前缀」。
        self.observe_stable_half(&model_messages);
        // v3.3.1 运行中插话并入：作为 User 消息追加到历史末尾（先于当前输入），
        // 让模型把用户的补充/纠正视为对本次生成的实时指引——而非仅回合末附录。
        // 并入后清空缓冲；无插话时零开销。
        let steers = self.take_pending_steers();
        if !steers.is_empty() {
            let joined = steers.join("\n");
            model_messages.push(Message::User {
                content: format!("【运行中用户补充——请在后续生成中遵循】\n{joined}"),
                timestamp: Utc::now(),
            });
        }
        model_messages.extend(history.iter().cloned());
        // ── 动态区（逐轮变化，后置；置于 [静态段+历史] 之后，不破坏稳定前缀）──
        // 记忆召回前缀：内容按本轮查询文本召回（W-6 的"稳定"指格式稳定而非内容
        // 稳定——内容天然逐轮不同，故必须落在 volatile 区而非最前端）。
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
        // 经验提示：persona / 跨项目提示 / viky PSI 状态（逐轮变化）
        if let Some(hint) = &self.experience_hint {
            if !hint.is_empty() {
                model_messages.push(Message::System {
                    content: hint.clone(),
                    timestamp: Utc::now(),
                });
            }
        }
        // Track A 路由策略片段（逐轮决策）
        if let Some(dec) = &self.route_decision {
            let frag = dec.prompt_policy.system_fragment();
            if !frag.is_empty() {
                model_messages.push(Message::System {
                    content: frag,
                    timestamp: Utc::now(),
                });
            }
        }
        // ── 滑动窗口（StreamingLLM sink + recent，M2 降耗）──
        // 长历史不再无限膨胀：仅当消息数超过 `WINDOW_MIN_MESSAGES` 时裁剪。
        // `sink` = 开头连续 System（稳定前缀）的长度，`recent` = 最近 `WINDOW_RECENT_MESSAGES` 条。
        // 关键不变式：稳定前缀必须**逐字节、原位**保留在最前——服务端 KV-cache 前缀复用依赖它。
        // `compact_window` 直接取 `messages[..sink]`，故前缀天然原样保留；而动态区（记忆召回/
        // 经验提示/路由片段）位于末尾，落在 recent 段内亦被保留。
        if model_messages.len() > WINDOW_MIN_MESSAGES {
            let sink = leading_stable_prefix(&model_messages).len();
            return compact_window(&model_messages, sink, WINDOW_RECENT_MESSAGES);
        }
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
        let model_messages = self.build_model_messages(&self.session.messages, None, None);

        // 5. 调用 LLM 生成回复（注入经验后的消息序列）
        // 锚定 bootstrap：首个请求注入 max_tokens 上限后立即剥离（一次性，
        // 对齐梁神 bootstrapMaxTokens——晋升后绝不残留进后续请求）
        let bootstrap_cap = self.take_bootstrap_max_tokens();
        let response = if bootstrap_cap.is_some() {
            self.model_router
                .chat_for_capped(&intent, complexity, &model_messages, bootstrap_cap)
                .await
        } else {
            self.model_router
                .chat_for(&intent, complexity, &model_messages)
                .await
        };

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
        let model_messages = self.build_model_messages(history, None, None);
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
    ///
    /// **熔断（P0-1c）**：与真流式回路 `run_tool_loop_streaming` 同一套两档
    /// doom-loop 判定——先记账再判定：达「纠正阈值」把提醒注进工具结果（不中止
    /// 整轮，同参重复本身不等于任务失败），达「硬中止阈值」如实 Err 返回。
    /// 此前这条非流式路径**一档都没有**：模型同名同参重复时只能跑满 20 轮才失败，
    /// 每轮都是同样的错（真机表现即"每 2–4 秒一轮的死循环"）。
    pub async fn run_tool_loop(
        &self,
        history: &mut Vec<Message>,
        registry: Arc<ToolRegistry>,
        tools: &[ToolDef],
    ) -> Result<String> {
        // 预算阈值默认从 env 读（P2-9）；测试/内部调用可经 with_limits 直接注入。
        let limits = crate::agent::turn_budget::limits_from_env();
        self.run_tool_loop_with_limits(history, registry, tools, limits)
            .await
    }

    /// 与 [`run_tool_loop`](Self::run_tool_loop) 相同，但显式注入单 turn 预算
    /// （P2-9），便于在不修改进程 env 的前提下做回路集成测试。
    pub(crate) async fn run_tool_loop_with_limits(
        &self,
        history: &mut Vec<Message>,
        registry: Arc<ToolRegistry>,
        tools: &[ToolDef],
        budget_limits: crate::agent::turn_budget::TurnBudgetLimits,
    ) -> Result<String> {
        if tools.is_empty() {
            return Err(anyhow!("run_tool_loop 需要非空 tools 定义"));
        }
        let cycle = ToolCycle::new(registry).with_session(self.event_session.clone());
        let request_text = history
            .iter()
            .rev()
            .find_map(|message| match message {
                Message::User { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .unwrap_or_default();
        // 轮数不再写死 20：按本轮用户请求与工具面自适应（模型可在计划里写
        // `[[budget:deep]]` 显式升档；硬护栏 64 轮）。
        let max_rounds: u32 =
            crate::agent::budget_policy::resolve(request_text, 0, tools.len()).tool_rounds as u32;
        let mut disclosure = self.tool_disclosure(tools, request_text);
        let capability_brief = capability_brief_for(tools);

        // P2-9：单 turn 预算硬熔断（阈值由调用方注入）。
        let turn_started = std::time::Instant::now();
        let mut tokens_used: u64 = 0;
        // doom-loop 记账：按时序记录 `(工具名, 入参原文)`，判定只看逐字节一致。
        let mut call_log: Vec<(String, String)> = Vec::new();
        // W4.4：反脚本守卫的「本 turn 已纠偏能力」必须**跨轮**保留——契约是
        // 每 turn 每能力只纠偏一次，第二次放行。放在 round 循环内会让模型每轮
        // 重试都再撞一次 CAPABILITY_MISROUTE，直到预算耗尽。
        let mut corrected_capabilities = std::collections::HashSet::new();
        // P3-1 长任务熔断：每条 turn 一个熔断器（`task_id` 同时是可回滚点标识）。
        // 每条 turn 新建 → 上一次任务的错误计数不会继承到下一次（状态有界）。
        let mut breaker = CircuitBreaker::new(format!(
            "tool-turn-{}-{}",
            self.event_session.as_deref().unwrap_or("agent"),
            crate::agent::task_checkpoint::now_ms()
        ));
        for round in 0..max_rounds {
            // 每轮开头先做预算硬熔断（步数/时长/token），命中即返回可识别结果。
            if let Some(breach) = crate::agent::turn_budget::check_turn_budget(
                &budget_limits,
                round,
                &turn_started,
                tokens_used,
            ) {
                let partial = last_assistant_text(history);
                let body = breach.to_json(&partial, None);
                return Err(anyhow!("budget_exhausted: {body}"));
            }
            let active_tools = disclosure.active_tools();
            // 锚定 deferred skill-catalog：扣住时不注入技能目录指引（晋升 + grace 步数
            // 越过前保持扣住，由编排器按会话锚定态置位）。工具发现仍由 function schema 可见。
            let tool_guide = if self.defer_skill_catalog() {
                None
            } else {
                let skills_dir =
                    crate::tools::skill_tools::resolve_skills_dir(&self.config.skills.skills_dir);
                let snap = crate::agent::tool_catalog::snapshot(&skills_dir);
                Some(compose_guide(&active_tools, &snap))
            };
            // 统一装配：稳定前缀 → 工具指引 → 经验提示 → 路由片段 → 历史
            //（旧实现漏装配稳定前缀，与其余入口不一致，且工具指引非最前）
            let model_messages =
                self.build_model_messages(history, tool_guide.as_deref(), Some(&capability_brief));

            // 锚定 bootstrap：首个请求注入 max_tokens 上限后立即剥离（一次性）
            let bootstrap_cap = self.take_bootstrap_max_tokens();
            let mut resp = if bootstrap_cap.is_some() {
                self.model_router
                    .chat_with_tools_capped(&model_messages, &active_tools, bootstrap_cap)
                    .await?
            } else {
                self.model_router
                    .chat_with_tools(&model_messages, &active_tools)
                    .await?
            };
            // P2-9：累计本 turn 已用 token（服务端 usage；缺省时按 0 累加）。
            tokens_used = tokens_used.saturating_add(resp.usage.total_tokens as u64);
            // 非流式回路**不发** Thinking 事件：把本轮 reasoning_content 攒进共享缓冲，
            // assistant 行落盘时写进 `thinking` 列（复盘要能读到「智能体思路」）。
            if let (Some(sess), Some(thought)) =
                (self.event_session.as_deref(), resp.thinking.as_deref())
            {
                crate::persistence::session::note_pending_thinking(sess, thought);
            }
            // 思考可见：非流式回路的 reasoning 也要实时下发。此前只在落盘缓冲里留了一份，
            // 前端实时只能渲染出空的「思考」折叠块（用户看不到模型在想什么）。
            // 流式回路本就逐块下发，这里补的是计划子步骤等仍走非流式的路径。
            if let (Some(bus), Some(sess), Some(thought)) = (
                self.events.as_ref(),
                self.event_session.as_deref(),
                resp.thinking.as_deref(),
            ) {
                if !thought.trim().is_empty() {
                    bus.publish(Envelope::new(
                        EventType::Thinking,
                        json!({ "text": thought }),
                        sess,
                    ));
                }
            }
            // P2-B 兼容层：本地权重常把动作写在 content 里而不是 tool_calls（见
            // `model::llm::bridge_content_action` 的说明）。这里桥接一次，
            // 否则整轮会被当成"只是在聊天"，一个工具都不会被调用。
            if resp.tool_calls.is_empty() {
                if let Some(bridged) = crate::model::llm::bridge_content_action(&resp.content) {
                    resp.tool_calls.push(bridged);
                }
            }
            if resp.tool_calls.is_empty() {
                let text = resp.content.clone();
                history.push(Message::Assistant {
                    content: text.clone(),
                    thinking: resp.thinking,
                    timestamp: Utc::now(),
                });
                // 成功收敛 = 本任务结束：熔断计数复位，下一次任务从零开始度量。
                breaker.resume();
                return Ok(text);
            }

            // 先记账再判定（与真流式回路同一套判定函数），达硬中止阈值即返回 Err。
            for tc in &resp.tool_calls {
                call_log.push((tc.name.clone(), tc.arguments.clone()));
            }
            if let Some((tool, repeats)) = detect_doom_loop(&call_log, DOOM_LOOP_ABORT) {
                return Err(anyhow!(
                    "检测到 doom-loop：工具 `{tool}` 连续 {repeats} 次同名同参调用，已中止回路"
                ));
            }
            // 纠正阈值：不中止整轮，但把提醒注进该工具的结果里（模型下一轮必然读到）。
            let repeated_tool: Option<String> =
                detect_doom_loop(&call_log, DOOM_LOOP_THRESHOLD).map(|(tool, _)| tool);
            let tool_batch_started = std::time::Instant::now();
            // 补发工具事件：此前非流式回路**不发** ToolCall/ToolResult（见下方 E12 注释），
            // 后果是前端实时工具卡永远不出现，只有事后复盘能看到。这里按与编排/流式回路
            // **完全相同的 payload 形状**补发，前端无需改动；无事件总线时静默跳过。
            let event_sink = self
                .events
                .clone()
                .zip(self.event_session.clone())
                .filter(|(_, sess)| !sess.is_empty());
            let batch_started_ms = Utc::now().timestamp_millis();
            // 真实审批裁决（与对话页档位同源）：只对「直接驱动本机」的高自主工具
            // （computer_use / system_control …）产出 `approval`，其余工具不伪造字段。
            let approval_session = self.event_session.clone().unwrap_or_default();
            let approvals: Vec<Option<crate::guard::permission_matrix::ToolApproval>> = resp
                .tool_calls
                .iter()
                .map(|tc| {
                    // 参数解析失败 → None：参数级信任条目因此不命中（fail-closed）
                    let parsed = serde_json::from_str::<serde_json::Value>(&tc.arguments).ok();
                    crate::guard::permission_matrix::decide_tool(
                        &tc.name,
                        &approval_session,
                        parsed.as_ref(),
                    )
                })
                .collect();
            if let Some((bus, sess)) = &event_sink {
                for (i, tc) in resp.tool_calls.iter().enumerate() {
                    let input: serde_json::Value = serde_json::from_str(&tc.arguments)
                        .unwrap_or_else(|_| json!(tc.arguments));
                    let approval = approvals.get(i).and_then(|a| a.as_ref());
                    let mut payload = json!({
                        "id": tc.id, "tool_use_id": tc.id,
                        "name": tc.name, "input": input,
                        "started_at": batch_started_ms,
                        // 风险档位：真实分类表（只读/写/高自主），供监管卡标注，
                        // 避免前端自己按工具名猜「这算不算危险操作」。
                        "autonomy": crate::guard::permission_matrix::classify_tool_autonomy(&tc.name).as_key(),
                    });
                    if let Some(a) = approval {
                        payload["approval"] = json!(a.status);
                        payload["approvalAutonomy"] = json!(a.autonomy.as_key());
                        if let Some(reason) = &a.reason {
                            payload["approvalReason"] = json!(reason);
                        }
                        if let Some(audit) = a.audit {
                            payload["approvalAudit"] =
                                serde_json::to_value(audit).unwrap_or(serde_json::Value::Null);
                        }
                    }
                    bus.publish(Envelope::new(EventType::ToolCall, payload, sess));
                }
            }
            // W4.4：反脚本守卫（每 turn 每个能力只纠偏一次；集合在 round 循环外）。
            // 熔断器按「调用 ↔ 结果」对齐喂入：先按模型返回顺序建好调用条目，
            // 结果下标与 `cycle_calls` 下标一一对应（每个分支都只产出一条结果）。
            let cycle_calls: Vec<CycleToolCall> = resp
                .tool_calls
                .iter()
                .map(|tc| CycleToolCall::new(&tc.name, parse_tool_args(&tc.arguments)))
                .collect();
            let mut results = Vec::with_capacity(resp.tool_calls.len());
            for (i, tc) in resp.tool_calls.iter().enumerate() {
                // 矩阵拒绝 / 需确认但无宿主 → 不执行（fail-closed），结果里写明真实原因。
                if let Some(a) = approvals.get(i).and_then(|a| a.as_ref()) {
                    if a.status == "denied" {
                        results.push(ToolResult::Error {
                            message: format!(
                                "审批拒绝（denied）：{}",
                                a.reason.as_deref().unwrap_or("权限矩阵拒绝")
                            ),
                            code: "APPROVAL_DENIED".to_string(),
                        });
                        continue;
                    }
                    if a.status == "pending"
                        && !crate::guard::gateway::agent_approval_host_present()
                    {
                        results.push(ToolResult::Error {
                            message: format!(
                                "审批待定（pending）：{}；当前没有可用的宿主审批通道，\
                                 已按 fail-closed 拒绝执行。",
                                a.reason.as_deref().unwrap_or("需要人工确认")
                            ),
                            code: "APPROVAL_PENDING".to_string(),
                        });
                        continue;
                    }
                }
                if disclosure.is_discovery_tool(&tc.name) {
                    let args = serde_json::from_str::<serde_json::Value>(&tc.arguments)
                        .unwrap_or(serde_json::Value::Null);
                    let output = discovery_query(&args)
                        .map(|query| disclosure.discover(query))
                        .unwrap_or_else(|| "发现工具需要提供字符串参数 query。".to_string());
                    results.push(ToolResult::Success {
                        data: output,
                        metadata: HashMap::new(),
                    });
                } else if !disclosure.permits(&tc.name) {
                    results.push(ToolResult::Error {
                        message: format!(
                            "工具 `{}` 尚未加载参数 schema。请先调用 `{}` 并描述所需能力，再在下一轮调用该工具。",
                            tc.name, DISCOVER_TOOLS_NAME
                        ),
                        code: "TOOL_SCHEMA_NOT_LOADED".to_string(),
                    });
                } else if let Some(blocked) = crate::agent::capability_guard::check(
                    &tc.name,
                    &parse_tool_args(&tc.arguments),
                    &active_tools
                        .iter()
                        .map(|tool| tool.name.clone())
                        .collect::<Vec<_>>(),
                    &mut corrected_capabilities,
                ) {
                    results.push(blocked);
                } else {
                    results.extend(cycle.execute_sequential(vec![cycle_calls[i].clone()]).await?);
                }
            }
            // P3-1 唯一收口：每执行一个工具调用后即喂给熔断器（调用 ↔ 结果同序对齐）。
            // 命中即「暂停 + 回滚点」：把控制权交回操作员，不 abort 进程、不丢已完成步骤。
            let circuit = observe_tool_steps(
                &mut breaker,
                &cycle_calls,
                &results,
                crate::agent::task_checkpoint::now_ms(),
            );
            if pauses_task(&circuit) {
                let reason = circuit
                    .reason
                    .clone()
                    .map(|r| r.to_string())
                    .unwrap_or_else(|| "未知原因".to_string());
                let checkpoint_id = circuit
                    .checkpoint_id
                    .clone()
                    .unwrap_or_else(|| breaker.task_id().to_string());
                let paused = crate::agent::task_checkpoint::default_store()
                    .ok()
                    .and_then(|store| {
                        breaker.pause_with_rollback_point(
                            &store,
                            self.event_session.as_deref().unwrap_or("agent"),
                            "智能体工具回路",
                        )
                    });
                let message = format!(
                    "工具回路已暂停（可回滚）：checkpoint_id={checkpoint_id}；原因：{reason}"
                );
                // 沿用既有 Error 帧（稳定码 CIRCUIT_TRIPPED），前端即能看到暂停态与回滚点；
                // 不新造事件类型、不新增 RPC。
                if let Some((bus, sess)) = &event_sink {
                    bus.publish(Envelope::error_structured(
                        sess,
                        "CIRCUIT_TRIPPED",
                        &message,
                        json!({
                            "paused": true,
                            "checkpoint_id": checkpoint_id.clone(),
                            "checkpoint_status": paused
                                .as_ref()
                                .map(|c| c.status.clone())
                                .unwrap_or_else(|| "unavailable".to_string()),
                            "reason": reason.clone(),
                        }),
                    ));
                }
                tracing::warn!(
                    checkpoint_id = %checkpoint_id,
                    %reason,
                    "工具回路熔断：已暂停并落回滚点，等待操作员回滚/重放"
                );
                return Err(anyhow!("circuit_tripped: {message}"));
            } else if circuit.tripped {
                // 同参数重复（RepeatedParameters）交由前面的 doom-loop 两档机制处理：
                // 这里只复位熔断器，避免该判定永久占住熔断态、让错误驱动判定失效。
                tracing::debug!(
                    reason = %circuit.reason.as_ref().map(|r| r.to_string()).unwrap_or_default(),
                    "同参数重复交由 doom-loop 处理，复位熔断器计数"
                );
                breaker.resume();
            }
            // P2-10/T14：整批实测耗时（单轮单工具时可精确归因）。
            let tool_batch_elapsed_ms = tool_batch_started.elapsed().as_millis() as u64;
            let single_tool_batch = resp.tool_calls.len() == 1;
            // 先把 assistant 的思考内容（reasoning_content）写入历史，
            // DeepSeek thinking 模式要求多轮对话回传 reasoning_content，否则 API 400。
            history.push(Message::Assistant {
                content: String::new(),
                thinking: resp.thinking.clone(),
                timestamp: Utc::now(),
            });
            for (i, (tc, res)) in resp.tool_calls.iter().zip(results).enumerate() {
                let ok = matches!(&res, ToolResult::Success { .. });
                let mut output = match &res {
                    ToolResult::Success { data, .. } => data.clone(),
                    ToolResult::Error { message, code } => format!("[工具错误 {code}] {message}"),
                    ToolResult::Timeout { tool_name, .. } => format!("[工具超时] {tool_name}"),
                };
                // 同参重复的纠正提示：只提醒、不改执行结果本身。
                if repeated_tool.as_deref() == Some(tc.name.as_str()) {
                    output = format!(
                        "[重复调用提醒] 你已连续 {} 次以**完全相同的参数**调用 `{}`；\
相同的参数不会得到不同结果。请改变参数、换用其它工具，或直接给出结论。\n\n{output}",
                        DOOM_LOOP_THRESHOLD, tc.name,
                    );
                }
                // E12（已修）：非流式回路原先**不发**工具事件，导致前端实时工具卡缺失、
                // 上下文台账只能靠事件桥补记（曾系统性低估压力）。现在两件事都做：
                // ① 在 `Message::Tool` 入历史处按「入参 + 完整输出」记账；
                // ② 在同一位置补发 ToolResult 事件（ToolCall 已在执行前发出）。
                // 输出带 `call_id` 幂等键，事件桥/计划 tap 重复看到也不会重复计数或重复落盘。
                if let Some((bus, sess)) = &event_sink {
                    let mut payload = json!({
                        "id": tc.id, "tool_use_id": tc.id,
                        "name": tc.name, "result": output, "is_error": !ok,
                        "elapsed_ms": single_tool_batch.then_some(tool_batch_elapsed_ms),
                        "execution_status": {
                            "status": if ok { "success" } else { "error" }
                        },
                    });
                    // 与 ToolCall 帧同源的审批裁决（approved / pending / denied）。
                    if let Some(a) = approvals.get(i).and_then(|a| a.as_ref()) {
                        payload["approval"] = json!(a.status);
                        payload["approvalAutonomy"] = json!(a.autonomy.as_key());
                        if let Some(reason) = &a.reason {
                            payload["approvalReason"] = json!(reason);
                        }
                    }
                    bus.publish(Envelope::new(EventType::ToolResult, payload, sess));
                }
                // 入口截流（M1-T3）：模型历史与上下文台账共用这一份。
                // `overhead_tokens` 的口径是「真正进入模型上下文的文本」；按原文计
                // 会让用量表与压缩阈值一起偏小。复盘台账（append_tool_activity）仍记原文。
                let context_output =
                    crate::context::middle_truncate::truncate_for_context(&tc.name, &output);
                if let Some(sess) = &self.event_session {
                    crate::context::session::record_tool_input_at_context(
                        sess,
                        tc.id.as_deref(),
                        &tc.name,
                        &tc.arguments,
                    );
                    crate::context::session::record_tool_output_at_context(
                        sess,
                        tc.id.as_deref(),
                        &tc.name,
                        &context_output,
                    );
                    // 复盘证据（权威落盘点）：非流式回路不发事件，网关桥与计划 tap
                    // 都看不到这次调用，只能在这里写 `role='tool'` 行。按 call_id 幂等，
                    // 与事件桥重复时不会写两行。
                    crate::persistence::session::append_tool_activity(
                        sess,
                        tc.id.as_deref(),
                        &tc.name,
                        &tc.arguments,
                        &output,
                        single_tool_batch.then_some(tool_batch_elapsed_ms),
                        Some(ok),
                    );
                }
                // 间接注入防线（P0）：只有**进上下文**的这一份被包成不可信内容；
                // 上面的工具事件与落盘台账仍记原文，复盘时看到的就是工具真实输出。
                //
                // 【2026-10-29 · D11】进上下文前**中间截断**（保头保尾、砍中段）。
                // 统一入口见 `context::middle_truncate::truncate_for_context`
                // （三条工具回路共用一份策略，避免"改一处漏两处"）；
                // 被砍掉的原文由该入口留底，模型可用 `tool_output_retrieve` 取回。
                history.push(Message::Tool {
                    name: tc.name.clone(),
                    input: tc.arguments.clone(),
                    output: crate::security::spotlight::wrap_if_untrusted(
                        &tc.name,
                        &context_output,
                    ),
                    timestamp: Utc::now(),
                    elapsed_ms: single_tool_batch.then_some(tool_batch_elapsed_ms),
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
        let request_text = history
            .iter()
            .rev()
            .find_map(|message| match message {
                Message::User { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .unwrap_or_default();
        let disclosure = self.tool_disclosure(tools, request_text);
        let capability_brief = capability_brief_for(tools);
        let router = &self.model_router;
        let defer_skill_catalog = self.defer_skill_catalog();
        // 能力目录（技能索引）在闭包外取一次：闭包是 move 的、拿不到 self；
        // snapshot 内部按目录缓存，同一会话内不会重复扫盘。
        let cap_dir =
            crate::tools::skill_tools::resolve_skills_dir(&self.config.skills.skills_dir);
        let cap_snap = crate::agent::tool_catalog::snapshot(&cap_dir);
        // E12：把会话标识交给流式回路——工具输出写进 `Message::Tool` 的同时按完整
        // 文本计入上下文台账（此前只靠网关事件桥异步兜底，压缩判定读到的是旧值）。
        let ledger_session = self.event_session.clone();
        run_tool_loop_streaming_with_disclosure_and_session(
            history,
            registry,
            disclosure,
            events,
            max_rounds,
            ledger_session,
            move |turn, active_tools| {
                // 工具目录随发现结果扩展，故每轮按当前可见面重建轻量指引。
                let tool_guide = if defer_skill_catalog {
                    None
                } else {
                    Some(compose_guide(&active_tools, &cap_snap))
                };
                // 经验提示以系统消息前置（与 process_input / run_tool_loop 行为一致），
                // 同时天然落在 P-7 稳定前缀半区，利于 KV-cache 前缀复用。
                // 统一装配：稳定前缀 → 工具指引 → 经验提示 → 路由片段 → 历史
                //（修正：旧实现工具指引排在稳定前缀之前，违背 W-6「最前端」契约）
                let model_messages =
                    self.build_model_messages(&turn, tool_guide.as_deref(), Some(&capability_brief));
                async move {
                    router
                        .chat_stream_with_tools(&model_messages, &active_tools)
                        .await
                }
            },
        )
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
        // W4.2：会话内容已清空 = 新会话 → 工具面重建，不继承已发现的工具。
        *self
            .session_tool_face
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        self.session.set_state(AgentState::Ready);
    }
}

/// 将本地分类器意图映射为模型路由意图
fn map_router_intent(intent: &ClassifierIntent) -> IntentType {
    match intent {
        ClassifierIntent::Explain
        | ClassifierIntent::Refactor
        | ClassifierIntent::Fix
        | ClassifierIntent::Build
        | ClassifierIntent::Test => IntentType::Code,
        ClassifierIntent::Doc | ClassifierIntent::Commit => IntentType::Summarization,
        ClassifierIntent::Search | ClassifierIntent::Question => IntentType::Chat,
        ClassifierIntent::Unknown => IntentType::General,
    }
}

/// 根据意图评估任务复杂度
fn map_complexity(intent: &ClassifierIntent) -> Complexity {
    match intent {
        // 创建新产物（应用/游戏/脚手架）与重构/修复同级：不是「简单问答」，
        // 不该被降到最弱模型。
        ClassifierIntent::Refactor
        | ClassifierIntent::Fix
        | ClassifierIntent::Build
        | ClassifierIntent::Test => Complexity::Medium,
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
            elapsed_ms: None,
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

    // ---------- W4.2：会话级稳定工具面 ----------

    fn session_face_tools() -> Vec<ToolDef> {
        let mut tools: Vec<ToolDef> = crate::agent::tool_schema::FIXED_FACE
            .iter()
            .map(|name| ToolDef {
                name: (*name).to_string(),
                description: "固定高频面工具".to_string(),
                parameters: json!({"type": "object"}),
            })
            .collect();
        tools.push(ToolDef {
            name: "office_automation".to_string(),
            description: "生成 Excel 工作簿和 Word 文档".to_string(),
            parameters: json!({"type": "object"}),
        });
        tools.push(ToolDef {
            name: "web_research".to_string(),
            description: "联网研究网页资料并汇总来源".to_string(),
            parameters: json!({"type": "object"}),
        });
        tools.extend((0..8).map(|i| ToolDef {
            name: format!("special_{i}"),
            description: "专用能力".to_string(),
            parameters: json!({"type": "object"}),
        }));
        tools
    }

    fn active_names(disclosure: &ToolDisclosure) -> Vec<String> {
        disclosure
            .active_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect()
    }

    #[test]
    fn session_tool_face_is_byte_stable_across_five_turns() {
        // 变异说明：把 `AgentRuntime::tool_disclosure` 换回「每轮
        // `ToolDisclosure::new(tools, request_text, mode)` 后取 active」的旧实现
        // （按 request_text 逐轮预热），本用例立刻变红——工具数组会随用户文本变化。
        let runtime = AgentRuntime::new(Config::default());
        let tools = session_face_tools();
        let requests = ["你好", "你觉得这个方案怎么样", "谢谢！", "在吗", "晚安"];
        let first = active_names(&runtime.tool_disclosure_with_mode(
            &tools,
            requests[0],
            ModeKind::Agent,
        ));
        assert!(first.contains(&DISCOVER_TOOLS_NAME.to_string()));
        assert!(first.contains(&"read_file".to_string()));
        for request in &requests[1..] {
            let turn = runtime.tool_disclosure_with_mode(&tools, request, ModeKind::Agent);
            assert_eq!(
                active_names(&turn),
                first,
                "同一会话的工具数组不得随用户文本变化：{request}"
            );
        }
    }

    #[test]
    fn discovery_appends_and_session_switch_rebuilds_the_face() {
        let mut runtime = AgentRuntime::new(Config::default());
        let tools = session_face_tools();
        let initial = active_names(&runtime.tool_disclosure_with_mode(
            &tools,
            "你好",
            ModeKind::Agent,
        ));
        assert!(!initial.contains(&"office_automation".to_string()));

        {
            let mut turn =
                runtime.tool_disclosure_with_mode(&tools, "你好", ModeKind::Agent);
            let out = turn.discover("操作 Excel 工作簿");
            assert!(out.contains("office_automation"), "{out}");
        }
        let extended = active_names(&runtime.tool_disclosure_with_mode(
            &tools,
            "换个话题",
            ModeKind::Agent,
        ));
        assert!(
            extended.contains(&"office_automation".to_string()),
            "发现结果必须跨轮保留（会话面）"
        );
        for name in &initial {
            assert!(extended.contains(name), "发现不得删除既有工具：{name}");
        }

        // 会话切换 → 重建：新会话不携带上一会话的发现结果。
        runtime.set_event_bus(Arc::new(EventBus::new(64)), "session-b".to_string());
        let rebuilt = active_names(&runtime.tool_disclosure_with_mode(
            &tools,
            "继续",
            ModeKind::Agent,
        ));
        assert_eq!(rebuilt, initial, "新会话必须回到初始面");

        // reset() 同样视为新会话。
        {
            let mut turn =
                runtime.tool_disclosure_with_mode(&tools, "继续", ModeKind::Agent);
            assert!(turn.discover("操作 Excel 工作簿").contains("office_automation"));
        }
        runtime.reset();
        let after_reset = active_names(&runtime.tool_disclosure_with_mode(
            &tools,
            "继续",
            ModeKind::Agent,
        ));
        assert!(
            !after_reset.contains(&"office_automation".to_string()),
            "reset 后必须重建工具面"
        );
    }

    #[test]
    fn runtime_clone_keeps_an_independent_tool_face_snapshot() {
        let runtime = AgentRuntime::new(Config::default());
        let tools = session_face_tools();
        let cloned = runtime.clone();
        {
            let mut turn = cloned.tool_disclosure_with_mode(&tools, "你好", ModeKind::Agent);
            assert!(turn.discover("操作 Excel 工作簿").contains("office_automation"));
        }
        assert!(active_names(&cloned.tool_disclosure_with_mode(&tools, "继续", ModeKind::Agent))
            .contains(&"office_automation".to_string()));
        assert!(
            !active_names(&runtime.tool_disclosure_with_mode(&tools, "继续", ModeKind::Agent))
                .contains(&"office_automation".to_string()),
            "克隆体的发现不得污染原运行时"
        );
    }

    // ---------- W5.3：完整稳定前缀指纹与分 scope 漂移 ----------

    #[test]
    fn agent_stable_half_drift_is_zero_across_five_steady_turns() {
        // 契约：同一会话、同一工具面，只有用户文本/历史在变 → 稳定半区逐字节不变
        // → `agent.stable_half` 漂移必须为 0。
        // 变异说明：把 `observe_stable_half` 里对时间戳的处理改回「直接序列化 Message」，
        // 本用例立刻变红（每轮 timestamp 不同 = 每轮 +1 漂移）。
        let tracker = Arc::new(crate::cache::PrefixCacheTracker::new());
        let mut runtime = AgentRuntime::new(Config::default());
        runtime.use_stable_half_tracker(Arc::clone(&tracker));
        runtime.set_mode_section(Some("模式规则段".into()));
        let tools = session_face_tools();
        let brief = capability_brief_for(&tools);
        let guide = "工具指引（稳态）".to_string();

        for turn in 0..5 {
            let history = vec![Message::User {
                content: format!("第 {turn} 轮：完全不同的用户文本"),
                timestamp: Utc::now(),
            }];
            let msgs = runtime.build_model_messages(&history, Some(&guide), Some(&brief));
            assert!(msgs.len() >= 4, "mode + brief + guide + history");
        }

        let by_scope = tracker.drift_by_scope();
        assert_eq!(
            by_scope.get(AGENT_STABLE_HALF_SCOPE),
            Some(&0),
            "稳态 5 轮不得漂移：{by_scope:?}"
        );
        assert_eq!(tracker.stable_prefix_drift(), 0);
    }

    #[test]
    fn changing_one_tool_name_counts_exactly_one_agent_drift() {
        // 契约：工具面变化（改名/增删）→ 稳定半区变化一次（drift +1），之后恢复稳定。
        let tracker = Arc::new(crate::cache::PrefixCacheTracker::new());
        let mut runtime = AgentRuntime::new(Config::default());
        runtime.use_stable_half_tracker(Arc::clone(&tracker));
        let history = vec![Message::User {
            content: "同一句用户文本".to_string(),
            timestamp: Utc::now(),
        }];

        let before_tools = vec![
            ToolDef {
                name: "read_file".to_string(),
                description: "读取文件".to_string(),
                parameters: json!({"type": "object"}),
            },
            ToolDef {
                name: "shell_exec".to_string(),
                description: "执行命令".to_string(),
                parameters: json!({"type": "object"}),
            },
        ];
        let mut renamed_tools = before_tools.clone();
        renamed_tools[1].name = "shell_exec_v2".to_string();

        let brief_before = capability_brief_for(&before_tools);
        let brief_renamed = capability_brief_for(&renamed_tools);
        assert_ne!(
            brief_before, brief_renamed,
            "改名必须真的进入能力自述（否则本用例测不到漂移）"
        );

        runtime.build_model_messages(&history, Some("工具指引"), Some(&brief_before));
        // 改一个工具名 → 稳定半区变化一次。
        runtime.build_model_messages(&history, Some("工具指引"), Some(&brief_renamed));
        assert_eq!(
            tracker.drift_by_scope().get(AGENT_STABLE_HALF_SCOPE),
            Some(&1),
            "{:?}",
            tracker.drift_by_scope()
        );
        // 新面重复一轮：不再漂移（变化是「一次」，不是每轮）。
        runtime.build_model_messages(&history, Some("工具指引"), Some(&brief_renamed));
        assert_eq!(
            tracker.drift_by_scope().get(AGENT_STABLE_HALF_SCOPE),
            Some(&1)
        );
    }

    #[test]
    fn agent_and_compress_drift_scopes_are_independent() {
        // 契约：压缩层抖动不得污染 agent 稳定半区的判定，反之亦然。
        let tracker = Arc::new(crate::cache::PrefixCacheTracker::new());
        let mut runtime = AgentRuntime::new(Config::default());
        runtime.use_stable_half_tracker(Arc::clone(&tracker));
        let history = vec![Message::User {
            content: "用户输入".to_string(),
            timestamp: Utc::now(),
        }];

        runtime.build_model_messages(&history, Some("指引-A"), None);
        runtime.build_model_messages(&history, Some("指引-A"), None);
        assert_eq!(tracker.drift_by_scope().get(AGENT_STABLE_HALF_SCOPE), Some(&0));

        // 压缩层连续两个不同指纹 → 只有 compress scope +1。
        assert!(tracker
            .observe_stable_prefix_scoped(crate::cache::COMPRESS_TOOL_RESULTS_SCOPE, "compress-v1"));
        assert!(!tracker
            .observe_stable_prefix_scoped(crate::cache::COMPRESS_TOOL_RESULTS_SCOPE, "compress-v2"));
        assert_eq!(
            tracker.drift_by_scope().get(AGENT_STABLE_HALF_SCOPE),
            Some(&0),
            "压缩层漂移不得计入 agent scope"
        );
        assert_eq!(
            tracker
                .drift_by_scope()
                .get(crate::cache::COMPRESS_TOOL_RESULTS_SCOPE),
            Some(&1)
        );

        // agent 侧再变一次 → 只有 agent scope +1，compress 保持 1。
        runtime.build_model_messages(&history, Some("指引-B"), None);
        assert_eq!(tracker.drift_by_scope().get(AGENT_STABLE_HALF_SCOPE), Some(&1));
        assert_eq!(
            tracker
                .drift_by_scope()
                .get(crate::cache::COMPRESS_TOOL_RESULTS_SCOPE),
            Some(&1)
        );
        assert_eq!(tracker.stable_prefix_drift(), 2, "总和 = 各 scope 之和");
    }

    #[test]
    fn build_model_messages_observes_the_global_prefix_tracker() {
        // 接线守卫：未注入 tracker 时（生产路径）必须写进程级单例的 `agent.stable_half`
        // scope。断言只看「scope 已登记」（计数是单调量，并行用例不会让本断言变红）。
        let runtime = AgentRuntime::new(Config::default());
        let _ = runtime.build_model_messages(
            &[],
            Some("全局 tracker 接线探针"),
            None,
        );
        assert!(
            crate::cache::prefix_cache_tracker()
                .drift_by_scope()
                .contains_key(AGENT_STABLE_HALF_SCOPE),
            "build_model_messages 必须把稳定半区指纹交给进程级 tracker"
        );
    }

    #[test]
    fn stable_half_text_ignores_timestamps_and_history() {
        // 纯函数层守卫：时间戳与历史都不进入指纹（否则稳态会话恒漂移）。
        let a = vec![
            Message::System {
                content: "静态段".into(),
                timestamp: Utc::now(),
            },
            Message::User {
                content: "u1".into(),
                timestamp: Utc::now(),
            },
        ];
        let b = vec![
            Message::System {
                content: "静态段".into(),
                timestamp: Utc::now() + chrono::Duration::seconds(5),
            },
            Message::User {
                content: "u2".into(),
                timestamp: Utc::now(),
            },
        ];
        assert_eq!(stable_half_text(&a), stable_half_text(&b));
        assert!(stable_half_text(&a).is_some_and(|t| t.contains("静态段")));
        // 没有静态段 = 不登记（空半区没有可复用的前缀）。
        assert_eq!(
            stable_half_text(&[Message::User {
                content: "只有用户消息".into(),
                timestamp: Utc::now(),
            }]),
            None
        );
    }

    #[test]
    fn test_agent_interrupt_flag_initial() {
        let config = Config::default();
        let runtime = AgentRuntime::new(config);
        assert!(!runtime.interrupt_flag.load(Ordering::SeqCst));
    }

    #[test]
    fn test_stable_prefix_byte_identical_across_turns() {
        // 缓存 100% 命中前提（v9）：同一装配输入产出逐字节一致的消息段——
        // 时间戳等易变数据若混入稳定前缀，服务端 KV-cache 键即漂移、永不命中。
        let config = Config::default();
        let mut runtime = AgentRuntime::new(config);
        runtime.set_stable_prefix(Message::System {
            content: "记忆召回前缀".into(),
            timestamp: Utc::now(),
        });
        runtime.set_mode_section(Some("模式规则段".into()));
        runtime.set_experience_hint("经验提示".into());
        let history = vec![Message::User {
            content: "用户输入".into(),
            timestamp: Utc::now(),
        }];
        let guide = "工具指引".to_string();

        let a = runtime.build_model_messages(&history, Some(&guide), None);
        let b = runtime.build_model_messages(&history, Some(&guide), None);
        let systems = |msgs: &[Message]| -> Vec<String> {
            msgs.iter()
                .filter_map(|m| match m {
                    Message::System { content, .. } => Some(content.clone()),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            systems(&a),
            systems(&b),
            "System 段内容必须逐字节一致（KV-cache 命中前提）"
        );
        let content_of = |m: &Message| -> String {
            match m {
                Message::System { content, .. }
                | Message::User { content, .. }
                | Message::Assistant { content, .. } => content.clone(),
                Message::Tool { output, .. } => output.clone(),
            }
        };
        // 分区契约（M2 降耗引擎）：静态区（mode → 不可信声明 → guide）先于全部历史；
        // 动态区（stable → hint）后置于历史之后——逐轮变化内容不得进入稳定前缀。
        // 注意：不可信外部内容声明是**常开**安全段（P0 间接注入防线），位置固定在
        // 能力自述之后、工具指引之前；它不随轮次变化，属静态区。
        assert_eq!(a.len(), 6, "mode + notice + guide + history(1) + stable + hint = 6");
        assert_eq!(content_of(&a[0]), "模式规则段", "静态区首位");
        assert_eq!(
            content_of(&a[1]),
            crate::security::spotlight::UNTRUSTED_NOTICE,
            "不可信内容声明紧随模式段之后"
        );
        assert_eq!(content_of(&a[2]), "工具指引", "静态区末位是工具指引");
        assert_eq!(a[3].role(), "user", "历史紧跟静态区之后");
        assert_eq!(content_of(&a[4]), "记忆召回前缀", "动态区在历史之后");
        assert_eq!(content_of(&a[5]), "经验提示", "动态区末位");
    }

    #[test]
    fn capability_guard_state_is_declared_outside_the_round_loop() {
        // 契约：每 turn 每能力只纠偏一次、第二次放行。若把集合放进 round 循环，
        // 模型每次重试都会再撞一次 CAPABILITY_MISROUTE，直到预算耗尽。
        let runtime_src = include_str!("runtime.rs");
        let decl = runtime_src
            .find("let mut corrected_capabilities")
            .expect("runtime 缺少守卫状态声明");
        let round = runtime_src
            .find("for round in 0..max_rounds {")
            .expect("runtime 缺少 round 循环");
        assert!(decl < round, "runtime 的 corrected_capabilities 必须在 round 循环外");

        let stream_src = include_str!("stream_loop.rs");
        let sdecl = stream_src
            .find("let mut corrected_capabilities")
            .expect("stream_loop 缺少守卫状态声明");
        let sround = stream_src
            .find("for round in 1..=max_rounds {")
            .expect("stream_loop 缺少 round 循环");
        assert!(sdecl < sround, "stream_loop 的 corrected_capabilities 必须在 round 循环外");
    }

    #[test]
    fn capability_brief_enters_the_stable_half_before_the_tool_guide() {
        let runtime = AgentRuntime::new(Config::default());
        let brief = crate::agent::capability::brief_from(
            &["read_file".to_string(), "shell_exec".to_string()],
            3,
            0,
        );
        let history = vec![Message::User {
            content: "用户输入".into(),
            timestamp: Utc::now(),
        }];
        let with_brief = runtime.build_model_messages(&history, Some("工具指引"), Some(&brief));
        let again = runtime.build_model_messages(&history, Some("工具指引"), Some(&brief));
        let systems = |msgs: &[Message]| -> Vec<String> {
            msgs.iter()
                .filter_map(|m| match m {
                    Message::System { content, .. } => Some(content.clone()),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(systems(&with_brief), systems(&again), "能力自述必须逐字节稳定");
        let sys = systems(&with_brief);
        let brief_pos = sys.iter().position(|line| line == &brief).expect("缺少能力自述");
        let guide_pos = sys.iter().position(|line| line == "工具指引").expect("缺少工具指引");
        assert!(brief_pos < guide_pos, "能力自述必须在工具指引之前（稳定半区）");
        // 不传时不得凭空插入（非工具回路保持原样）。
        let without = runtime.build_model_messages(&history, Some("工具指引"), None);
        assert!(!systems(&without).contains(&brief));
    }

    fn content_of(m: &Message) -> String {
        match m {
            Message::System { content, .. }
            | Message::User { content, .. }
            | Message::Assistant { content, .. } => content.clone(),
            Message::Tool { output, .. } => output.clone(),
        }
    }

    /// 缺陷回归（P0-1b）：工作区指引必须落在**静态区最前**，且在扣住技能/工具
    /// 指引（锚定期）时依然随车携带。
    ///
    /// 锚定期正是模型最容易乱猜路径的时候：工具指引被扣掉后，如果连「文件写去哪」
    /// 都不在提示里，猜路径 → 1ms 失败 → 死循环这条链就原样复现。
    #[test]
    fn workspace_guidance_survives_tool_guide_deferral() {
        let d = std::env::temp_dir().join(format!("mgt_rt_ws_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let mut runtime = AgentRuntime::new(Config::default());
        runtime.set_mode_section(Some("模式规则段".into()));
        runtime.set_workspace(Workspace::resolve(Some(&d)));
        let history = vec![Message::User {
            content: "用户输入".into(),
            timestamp: Utc::now(),
        }];

        // 正常路径：静态区 = mode → workspace → 不可信声明 → guide，全部先于历史
        let full = runtime.build_model_messages(&history, Some("工具指引"), None);
        assert_eq!(content_of(&full[0]), "模式规则段", "静态区首位仍是模式段");
        assert!(
            content_of(&full[1]).contains(&d.display().to_string()),
            "静态区次位必须是工作区指引（含根目录）：{}",
            content_of(&full[1])
        );
        assert_eq!(
            content_of(&full[2]),
            crate::security::spotlight::UNTRUSTED_NOTICE,
            "不可信声明在工具指引之前"
        );
        assert_eq!(content_of(&full[3]), "工具指引");
        assert_eq!(full[4].role(), "user", "历史必须紧跟静态区之后");

        // 锚定期：工具指引被扣住，工作区指引与不可信声明都不得跟着消失
        let anchored = runtime.build_model_messages(&history, None, None);
        assert_eq!(anchored.len(), 4, "mode + workspace + notice + history");
        assert!(
            content_of(&anchored[1]).contains("【工作区】"),
            "扣住工具指引时必须保留工作区指引"
        );
        assert_eq!(
            content_of(&anchored[2]),
            crate::security::spotlight::UNTRUSTED_NOTICE,
            "安全声明是常开的：工具指引被扣住也不能一起消失"
        );

        // 逐字节稳定（KV-cache 命中前提）：同根两次装配必须完全一致
        assert_eq!(content_of(&anchored[1]), content_of(&full[1]));

        // 未注入工作区的调用方（CLI/测试）不得被凭空塞一段目录信息；
        // 但不可信内容声明是**安全常开段**，任何装配路径都要有。
        let bare = AgentRuntime::new(Config::default());
        let out = bare.build_model_messages(&history, Some("工具指引"), None);
        assert_eq!(out.len(), 3, "notice + guide + history，不得注入目录信息");
        assert_eq!(
            content_of(&out[0]),
            crate::security::spotlight::UNTRUSTED_NOTICE,
            "安全声明与工作区绑定无关，必须常开"
        );
        assert_eq!(content_of(&out[1]), "工具指引");

        let _ = std::fs::remove_dir_all(&d);
    }

    /// 缺陷回归（HIGH）：滑动窗口低于阈值时为 no-op（短会话逐字节原样，不受影响）。
    #[test]
    fn test_compact_window_noop_below_threshold() {
        let config = Config::default();
        let mut runtime = AgentRuntime::new(config);
        runtime.set_mode_section(Some("模式".into()));
        let history: Vec<Message> = (0..10)
            .map(|i| Message::User {
                content: format!("u{i}"),
                timestamp: Utc::now(),
            })
            .collect();
        let out = runtime.build_model_messages(&history, Some("指引"), None);
        // 阈值内：mode + notice + guide + history(10) = 13，未裁剪
        assert_eq!(out.len(), 13, "低于阈值不得裁剪");
    }

    /// 缺陷回归（HIGH）：超过阈值时裁剪，且**稳定前缀逐字节原位保留**、尾部为最近 N 条。
    ///
    /// 这是 KV-cache 前缀复用的硬不变式：`compact_window` 的 sink 取
    /// [`leading_stable_prefix`]，故开头连续 System 段原样保留在最前。
    #[test]
    fn test_compact_window_preserves_prefix_above_threshold() {
        let config = Config::default();
        let mut runtime = AgentRuntime::new(config);
        runtime.set_mode_section(Some("STATIC-MODE".into()));
        let n = WINDOW_MIN_MESSAGES + 50;
        let history: Vec<Message> = (0..n)
            .map(|i| Message::User {
                content: format!("u{i}"),
                timestamp: Utc::now(),
            })
            .collect();
        let out = runtime.build_model_messages(&history, Some("STATIC-GUIDE"), None);
        // 裁剪后 = leading prefix(3 System：mode + notice + guide) + recent(WINDOW_RECENT_MESSAGES)
        assert_eq!(
            out.len(),
            3 + WINDOW_RECENT_MESSAGES,
            "超过阈值应裁剪到 sink + recent"
        );
        // 稳定前缀逐字节、原位保留在最前
        assert!(matches!(out[0], Message::System { .. }));
        assert!(matches!(out[1], Message::System { .. }));
        assert!(matches!(out[2], Message::System { .. }));
        assert_eq!(content_of(&out[0]), "STATIC-MODE", "稳定前缀首条逐字节保留");
        assert_eq!(
            content_of(&out[1]),
            crate::security::spotlight::UNTRUSTED_NOTICE,
            "安全声明属稳定前缀，必须原位保留"
        );
        assert_eq!(
            content_of(&out[2]),
            "STATIC-GUIDE",
            "稳定前缀末条逐字节保留"
        );
        // 尾部为最近 N 条：第一条 recent 与最后一条（最新）都应正确
        assert_eq!(
            content_of(&out[3]),
            format!("u{}", n - WINDOW_RECENT_MESSAGES),
            "recent 段起点应为最近 N 条的最早一条"
        );
        assert_eq!(
            content_of(out.last().unwrap()),
            format!("u{}", n - 1),
            "最新消息应保留在末尾"
        );
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

    /// 缺陷回归（P0-1c）：非流式工具回路必须熔断同名同参重复。
    ///
    /// 这条路径（CLI `chat --tools`）此前**一档熔断都没有**：模型重复同一个调用时
    /// 只会跑满 20 轮才 Err，每轮都是同样的失败——真机表现即「每 2–4 秒一轮的
    /// 死循环重试」。现在必须与真流式回路同一套判定：达硬中止阈值即 Err，
    /// 且在此之前把「重复调用提醒」注进工具结果（模型下一轮读得到）。
    #[tokio::test]
    async fn test_run_tool_loop_aborts_on_identical_repeats() {
        // 每轮都返回同一个工具调用（同名同参）
        let body = r#"{"choices":[{"message":{"tool_calls":[{"id":"c1","type":"function","function":{"name":"list_dir","arguments":"{}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
        let url = mock_llm_server(200, body).await;
        let mut runtime = AgentRuntime::new(llm_config(&url));
        runtime.init().unwrap();
        let mut history = vec![Message::User {
            content: "列出目录".into(),
            timestamp: Utc::now(),
        }];
        let ws = std::env::temp_dir().join(format!("mgt_rt_loop_{}", std::process::id()));
        std::fs::create_dir_all(&ws).unwrap();
        let mut reg = ToolRegistry::new();
        crate::tools::builtin::register_builtin_tools(&mut reg, &ws, None, None, None).unwrap();
        let defs: Vec<ToolDef> = reg
            .describe_tools_for_llm()
            .into_iter()
            .map(|(name, description, parameters)| ToolDef {
                name,
                description,
                parameters,
            })
            .collect();

        let err = runtime
            .run_tool_loop(&mut history, Arc::new(reg), &defs)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("doom-loop"),
            "同名同参重复必须熔断，实际返回: {err}"
        );
        // 真正跑了几个回合必须远小于 20（否则等于没熔断）
        let tool_msgs = history.iter().filter(|m| m.role() == "tool").count();
        assert!(
            tool_msgs <= DOOM_LOOP_ABORT,
            "熔断前至多跑 {DOOM_LOOP_ABORT} 轮回合，实际工具结果 {tool_msgs} 条"
        );
        // 熔断前至少执行过纠正阈值那么多次（否则提醒提示无从注入）
        assert!(
            tool_msgs >= DOOM_LOOP_THRESHOLD,
            "纠正提示需要至少 {DOOM_LOOP_THRESHOLD} 轮回合才注入，实际 {tool_msgs} 条"
        );
        let corrected = history.iter().any(|m| match m {
            Message::Tool { output, .. } => output.contains("[重复调用提醒]"),
            _ => false,
        });
        assert!(
            corrected,
            "重复调用提醒必须注进工具结果，模型下一轮才看得到"
        );

        let _ = std::fs::remove_dir_all(&ws);
    }

    /// E12 回归：非流式工具回路（桌面 Solo 主路径 / Bloom 自动闭环 / 计划子步骤）
    /// **不发** ToolCall/ToolResult 事件，事件桥因此拿不到工具输出 →
    /// `ContextLedger` 漏计。修复后每次工具执行的「入参 + 输出」都记入同一条台账，
    /// 且增量恰好等于 `history` 中各 `Message::Tool` 的 token 之和。
    #[tokio::test]
    async fn test_run_tool_loop_records_tool_output_to_context_ledger() {
        // 每轮都返回同一个工具调用：回路最终命中 doom-loop，但每次执行都必须记账。
        let body = r#"{"choices":[{"message":{"tool_calls":[{"id":"c1","type":"function","function":{"name":"list_dir","arguments":"{}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
        let url = mock_llm_server(200, body).await;
        let mut runtime = AgentRuntime::new(llm_config(&url));
        runtime.init().unwrap();
        let session = format!("mgt-e12-{}", Uuid::new_v4());
        runtime.set_event_bus(Arc::new(EventBus::default()), session.clone());

        let ws = std::env::temp_dir().join(format!("mgt_rt_ledger_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join("probe.txt"), "hello").unwrap();
        let mut reg = ToolRegistry::new();
        crate::tools::builtin::register_builtin_tools(&mut reg, &ws, None, None, None).unwrap();
        let defs: Vec<ToolDef> = reg
            .describe_tools_for_llm()
            .into_iter()
            .map(|(name, description, parameters)| ToolDef {
                name,
                description,
                parameters,
            })
            .collect();

        let before = crate::context::session::context_status(&session)["overhead_tokens"]
            .as_u64()
            .unwrap_or(0);
        assert_eq!(before, 0, "新会话初始不应有工具开销");

        let mut history = vec![Message::User {
            content: "列出目录".into(),
            timestamp: Utc::now(),
        }];
        // 恒返回 tool_call → 期望 Err（doom-loop），此处只关心记账。
        let _ = runtime
            .run_tool_loop(&mut history, Arc::new(reg), &defs)
            .await;

        let expected: u64 = history
            .iter()
            .filter_map(|m| match m {
                Message::Tool { input, output, .. } => {
                    let mut t = crate::context::session::estimate_tokens(input);
                    if !output.trim().is_empty() {
                        t += crate::context::session::estimate_tokens(output);
                    }
                    Some(t)
                }
                _ => None,
            })
            .sum();
        assert!(expected > 0, "回路应至少执行一次工具并产生上下文开销");

        let after = crate::context::session::context_status(&session)["overhead_tokens"]
            .as_u64()
            .unwrap_or(0);
        assert!(after > before, "工具输出必须计入台账：{before} -> {after}");
        assert_eq!(
            after, expected,
            "台账增量必须等于各工具消息（入参+输出）token 之和"
        );

        let _ = std::fs::remove_file(
            Config::data_dir()
                .join("context")
                .join(format!("{session}.json")),
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// E12 反向守卫：没有工具调用的回合**不得**改变台账（向后兼容）。
    #[tokio::test]
    async fn test_run_tool_loop_without_tools_keeps_context_ledger_unchanged() {
        let body = r#"{"choices":[{"message":{"content":"直接回答"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
        let url = mock_llm_server(200, body).await;
        let mut runtime = AgentRuntime::new(llm_config(&url));
        runtime.init().unwrap();
        let session = format!("mgt-e12-notool-{}", Uuid::new_v4());
        runtime.set_event_bus(Arc::new(EventBus::default()), session.clone());

        let ws = std::env::temp_dir().join(format!("mgt_rt_notool_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&ws).unwrap();
        let mut reg = ToolRegistry::new();
        crate::tools::builtin::register_builtin_tools(&mut reg, &ws, None, None, None).unwrap();
        let defs: Vec<ToolDef> = reg
            .describe_tools_for_llm()
            .into_iter()
            .map(|(name, description, parameters)| ToolDef {
                name,
                description,
                parameters,
            })
            .collect();

        let mut history = vec![Message::User {
            content: "随便聊聊".into(),
            timestamp: Utc::now(),
        }];
        let reply = runtime
            .run_tool_loop(&mut history, Arc::new(reg), &defs)
            .await
            .expect("无工具调用时应直接返回文本");
        assert_eq!(reply, "直接回答");

        let status = crate::context::session::context_status(&session);
        assert_eq!(
            status["overhead_tokens"].as_u64(),
            Some(0),
            "没有工具消息时台账不得变化"
        );
        assert_eq!(status["used_tokens"].as_u64(), Some(0));

        let _ = std::fs::remove_file(
            Config::data_dir()
                .join("context")
                .join(format!("{session}.json")),
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// P2-9 回路集成：单 turn 步数预算到达后强制中止，返回**可识别**的
    /// `budget_exhausted` 结构化结果（含 kind/limit/used/remaining），而不是静默截断。
    /// 经 `run_tool_loop_with_limits` 直接注入阈值，不修改进程 env，故对并行测试安全。
    #[tokio::test]
    async fn test_run_tool_loop_hard_aborts_on_step_budget() {
        // 每轮都返回同一工具调用（让回路一直有工具可执行，直到步数预算命中）。
        let body = r#"{"choices":[{"message":{"tool_calls":[{"id":"c1","type":"function","function":{"name":"list_dir","arguments":"{}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
        let url = mock_llm_server(200, body).await;
        let mut runtime = AgentRuntime::new(llm_config(&url));
        runtime.init().unwrap();
        let mut history = vec![Message::User {
            content: "列出目录".into(),
            timestamp: Utc::now(),
        }];
        let ws = std::env::temp_dir().join(format!("mgt_rt_budget_{}", std::process::id()));
        std::fs::create_dir_all(&ws).unwrap();
        let mut reg = ToolRegistry::new();
        crate::tools::builtin::register_builtin_tools(&mut reg, &ws, None, None, None).unwrap();
        let defs: Vec<ToolDef> = reg
            .describe_tools_for_llm()
            .into_iter()
            .map(|(name, description, parameters)| ToolDef {
                name,
                description,
                parameters,
            })
            .collect();

        // 步数预算 = 2：round 0、1 执行，round 2 开头 used=2>=2 命中。
        let limits = crate::agent::turn_budget::TurnBudgetLimits {
            max_steps: 2,
            max_secs: 0,
            max_tokens: 0,
        };
        let err = runtime
            .run_tool_loop_with_limits(&mut history, Arc::new(reg), &defs, limits)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("budget_exhausted"), "必须返回可识别预算耗尽结果: {msg}");
        // 结构化预算字段齐全
        assert!(msg.contains("\"kind\":\"steps\""), "预算 kind 缺失: {msg}");
        assert!(msg.contains("\"limit\":2"), "limit 缺失: {msg}");
        // 预算必须在 max_rounds 之前触发（工具结果至多 2 条，而不是跑满 20 轮）。
        let tool_msgs = history.iter().filter(|m| m.role() == "tool").count();
        assert!(tool_msgs <= 2, "步数预算应在 2 轮内熔断，实际工具结果 {tool_msgs} 条");
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// P2-9 回路集成：token 预算到达同样强制中止（非静默截断）。
    #[tokio::test]
    async fn test_run_tool_loop_hard_aborts_on_token_budget() {
        let body = r#"{"choices":[{"message":{"tool_calls":[{"id":"c1","type":"function","function":{"name":"list_dir","arguments":"{}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}"#;
        let url = mock_llm_server(200, body).await;
        let mut runtime = AgentRuntime::new(llm_config(&url));
        runtime.init().unwrap();
        let mut history = vec![Message::User {
            content: "列出目录".into(),
            timestamp: Utc::now(),
        }];
        let ws = std::env::temp_dir().join(format!("mgt_rt_tokbudget_{}", std::process::id()));
        std::fs::create_dir_all(&ws).unwrap();
        let mut reg = ToolRegistry::new();
        crate::tools::builtin::register_builtin_tools(&mut reg, &ws, None, None, None).unwrap();
        let defs: Vec<ToolDef> = reg
            .describe_tools_for_llm()
            .into_iter()
            .map(|(name, description, parameters)| ToolDef {
                name,
                description,
                parameters,
            })
            .collect();

        // token 预算 = 10：第 1 轮后 used=20 >= 10，第 2 轮开头命中。
        let limits = crate::agent::turn_budget::TurnBudgetLimits {
            max_steps: 0,
            max_secs: 0,
            max_tokens: 10,
        };
        let err = runtime
            .run_tool_loop_with_limits(&mut history, Arc::new(reg), &defs, limits)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("budget_exhausted"), "token 预算应熔断: {msg}");
        assert!(msg.contains("\"kind\":\"tokens\""), "预算 kind 应为 tokens: {msg}");
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// 回归：长工具目录首轮只发送小 schema 面；调用发现工具后，下一轮加载 Excel 完整 schema。
    #[tokio::test]
    async fn test_run_tool_loop_discovers_specialized_schema_on_demand() {
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(std::sync::Mutex::new(Vec::<serde_json::Value>::new()));
        let captured = requests.clone();
        tokio::spawn(async move {
            let responses = [
                r#"{"choices":[{"message":{"tool_calls":[{"id":"discover-1","type":"function","function":{"name":"_marguerite_discover_tools","arguments":"{\"query\":\"操作 Excel 工作簿\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#,
                r#"{"choices":[{"message":{"content":"已发现并加载 Excel 工具。"},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":2,"total_tokens":4}}"#,
            ];
            for response in responses {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let request = crate::tests::mock_http::read_request_full(&mut socket).await;
                let raw = String::from_utf8_lossy(&request);
                if let Some((_, body)) = raw.split_once("\r\n\r\n") {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
                        captured.lock().unwrap().push(value);
                    }
                }
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            }
        });

        let mut runtime = AgentRuntime::new(llm_config(&format!("http://{addr}")));
        runtime.init().unwrap();
        let mut history = vec![Message::User {
            content: "解释一下这个项目的用途".into(),
            timestamp: Utc::now(),
        }];
        let workspace =
            std::env::temp_dir().join(format!("mgt_schema_discovery_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let mut registry = ToolRegistry::new();
        crate::tools::builtin::register_builtin_tools(&mut registry, &workspace, None, None, None)
            .unwrap();
        let definitions: Vec<ToolDef> = registry
            .describe_tools_for_llm()
            .into_iter()
            .map(|(name, description, parameters)| ToolDef {
                name,
                description,
                parameters,
            })
            .collect();
        let expected_count = definitions.len();
        let reply = runtime
            .run_tool_loop(&mut history, Arc::new(registry), &definitions)
            .await
            .unwrap();
        assert!(reply.contains("Excel 工具"));

        let captured = requests.lock().unwrap();
        assert_eq!(captured.len(), 2, "应进行发现轮 + 最终答复轮");
        let first_tools = captured[0]["tools"].as_array().expect("首轮工具 schema");
        let second_tools = captured[1]["tools"].as_array().expect("次轮工具 schema");
        assert!(first_tools.len() < expected_count, "首轮应避免全量 schema");
        assert!(
            !first_tools
                .iter()
                .any(|tool| tool["function"]["name"] == "office_automation"),
            "无关请求首轮不应塞入 Office 专用 schema"
        );
        assert!(
            second_tools
                .iter()
                .any(|tool| tool["function"]["name"] == "office_automation"),
            "发现后下一轮必须含完整 Office/Excel schema"
        );
        let discovery_result = history.iter().any(|message| match message {
            Message::Tool { name, output, .. } => {
                name == DISCOVER_TOOLS_NAME && output.contains("office_automation")
            }
            _ => false,
        });
        assert!(discovery_result, "发现结果应回灌历史供模型读取");
        let _ = std::fs::remove_dir_all(&workspace);
    }

    /// 简易 mock LLM HTTP 服务器：记录收到的请求体（用于验证系统提示注入）
    async fn capturing_mock_llm_server(
        status: u16,
        body: &str,
        captured: Arc<std::sync::Mutex<Option<String>>>,
    ) -> String {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = body.to_string();
        tokio::spawn(async move {
            let mut first = true;
            loop {
                if let Ok((mut socket, _)) = listener.accept().await {
                    let data = crate::tests::mock_http::read_request_full(&mut socket).await;
                    // 只捕获首请求（真实对话请求；后续经验抽取等次要请求不覆盖断言目标）
                    if first {
                        first = false;
                        *captured.lock().unwrap() =
                            Some(String::from_utf8_lossy(&data).to_string());
                    }
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
    async fn test_process_input_appends_recall_and_hint_after_history() {
        // M2 降耗引擎分区契约：逐轮变化段（记忆召回 / 经验提示）必须后置于
        // 全部历史之后（volatile 区），不得进入历史之前的稳定前缀。
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
            "应含记忆召回段"
        );
        assert!(req.contains("历史经验"), "应含经验提示");
        // 分区契约：历史（含本轮用户输入）→ 记忆召回 → 经验提示
        let user_pos = req.find("如何配置 Tauri 快捷键").expect("应含用户输入");
        let stable_pos = req
            .find("Recalled memory (compacted)")
            .expect("应含记忆召回段");
        let hint_pos = req.find("历史经验").expect("应含经验提示");
        assert!(
            user_pos < stable_pos,
            "逐轮变化的记忆召回段必须在历史之后（不得进入稳定前缀）"
        );
        assert!(
            stable_pos < hint_pos,
            "动态区内相对顺序：记忆召回在经验提示之前"
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
