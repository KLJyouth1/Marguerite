//! 三栏工作台 TUI 入口
//!
//! 单向状态归约 + 事件驱动渲染 + 同步 poll 主循环 + 后台 tokio 多线程。
//! 复用已锁定的 ratatui 0.28 / crossterm 0.28，不升级、不引入新依赖。

pub mod command;
pub mod dsh_slots;
pub mod history;
pub mod input;
pub mod layout;
pub mod region;
pub mod registry;
pub mod shortcut;
pub mod slot;
pub mod state;
pub mod theme;
pub mod palette_audit;
pub mod workspace_watcher;

use std::collections::hash_map::DefaultHasher;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Terminal;
use serde_json::json;
use tokio::runtime::{Builder, Runtime};
use uuid::Uuid;

use crate::collab::session::CollabSession;
use crate::deploy::baota::{BaotaDeployOptions, RecordingBaotaTransport};
use crate::deploy::ssh::{DeployOptions, RecordingTransport, SshDeployer, SshTarget};
use crate::events::{Envelope, EventBus, EventType};
use crate::guard::gateway::ApprovalGate;
use crate::memory::{ExperienceKind, SelfEvolution};
use crate::mode::{ModeKind, ModeManager};
use crate::model::llm::{LLMClient, LLMConfig, LLMProvider};
use crate::model::selector::ModelSelector;
use crate::orchestration::Orchestrator;
use crate::persistence::config::Config;
use crate::sandbox::Sandbox;
use crate::tui::workbench::layout::{split, WorkbenchLayout};
use crate::tui::workbench::registry::register_builtins;
use crate::tui::workbench::slot::SlotRegistry;
use crate::tui::workbench::state::{
    ChatMessage, ChatRole, CollabOp, DeployKind, DeployOp, InteractionState, ModelOp, UiEffect,
    UiEvent, WorkbenchState,
};
use crate::tui::workbench::theme::Theme;
use crate::types::Message;
use crate::router::RouteDecision;

/// 引擎运行门面（屏蔽 ChainExecutor / AgentRuntime 借用差异；后端实现真实适配器）
pub trait EngineRunner: Send + Sync {
    /// 运行一轮对话（发布事件回流总线，返回自有结果）
    fn run_turn(
        &self,
        text: String,
        mode: ModeKind,
        session: String,
        bus: Arc<EventBus>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;
}

/// Mock 引擎（Phase B 自测用；真实引擎在 A4.4 后由后端以 `EngineRunner` 适配接入）
pub struct MockEngine;

impl EngineRunner for MockEngine {
    fn run_turn(
        &self,
        text: String,
        _mode: ModeKind,
        session: String,
        bus: Arc<EventBus>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>> {
        Box::pin(async move {
            bus.publish(Envelope::new(
                EventType::Thinking,
                json!({ "text": format!("> 思考：{text}") }),
                session.as_str(),
            ));
            bus.publish(Envelope::tool_call(
                session.as_str(),
                "read_file",
                &["path"],
            ));
            bus.publish(Envelope::tool_result(
                session.as_str(),
                "read_file",
                true,
                "已读取文件",
            ));
            bus.publish(Envelope::progress(session.as_str(), 100, "完成"));
            bus.publish(Envelope::new(
                EventType::Done,
                json!({ "summary": "mock 完成" }),
                session.as_str(),
            ));
            Ok(())
        })
    }
}

/// 真实引擎（A4.4）：适配 AgentRuntime 为 EngineRunner，发布真实事件流。
///
/// 工作台对话通常无文件上下文，故走 AgentRuntime 聊天路径
/// （Thinking → Done / Error 经总线回流 TUI）。文件上下文就绪后（Phase C）
/// 可优先走调用链（ChainExecutor 已埋点）。
pub struct RealEngine {
    /// 应用配置（克隆持有，供后台任务使用）
    config: Config,
    /// 自进化经验台账句柄（Phase F，可选）：设置后，每轮对话前注入相关经验、
    /// 完成后由 LLM 自主抽取踩坑/决策写入台账。TUI 默认不接（None）。
    evolution: Option<Arc<Mutex<SelfEvolution>>>,
    /// 跨项目记忆提示（可选）：run_turn 前检索本机各项目记忆，随系统提示注入。
    cross_project_hint: Option<String>,
    /// 记忆召回稳定前缀（可选，W-6）：跨项目记忆召回后渲染成的固定格式 System 消息，
    /// 作为编排器对话最前端的稳定前缀（先于经验提示），利于 KV-cache 前缀复用。
    stable_prefix: Option<Message>,
    /// Track A 路由决策（可选）：由网关/调用方注入，转发到编排器 lead 运行时。
    route_decision: Option<RouteDecision>,
    /// 项目工作区根目录（可选）：设置后工具/终端/文件沙箱根随项目目录切换。
    workspace: Option<PathBuf>,
    /// 模式提示段（可选）：显式注入时优先于「按当前模式规格取 prompt_section」
    /// （Plan 模式的 DSH 规则段走此通道）。
    mode_section: Option<String>,
}

impl RealEngine {
    /// 从配置构造
    pub fn from_config(config: Config) -> Self {
        Self {
            config,
            evolution: None,
            cross_project_hint: None,
            stable_prefix: None,
            route_decision: None,
            workspace: None,
            mode_section: None,
        }
    }

    /// 接入自进化经验台账（GUI 调用；TUI 保持默认 None）
    pub fn with_evolution(mut self, evo: Option<Arc<Mutex<SelfEvolution>>>) -> Self {
        self.evolution = evo;
        self
    }

    /// 注入跨项目记忆提示（run_turn 前检索所得；空串视为未注入）
    pub fn with_cross_project_hint(mut self, hint: String) -> Self {
        self.cross_project_hint = (!hint.is_empty()).then_some(hint);
        self
    }

    /// 注入记忆召回稳定前缀（W-6）：跨项目记忆渲染成的 System 消息（None 视为未注入）。
    /// 作为编排器对话最前端的稳定前缀，落进 P-7 stable 半区，利于 KV-cache 前缀复用。
    pub fn with_stable_prefix(mut self, prefix: Option<Message>) -> Self {
        self.stable_prefix = prefix;
        self
    }

    /// 注入项目工作区根目录（工具/终端/文件沙箱根随之切换；None 用默认）
    pub fn with_workspace(mut self, workspace: Option<PathBuf>) -> Self {
        self.workspace = workspace;
        self
    }

    /// 注入 Track A 路由决策（转发到编排器 lead 运行时，驱动思考预算 + prompt 策略注入）。
    pub fn with_route_decision(mut self, decision: Option<RouteDecision>) -> Self {
        self.route_decision = decision;
        self
    }

    /// 显式注入模式提示段（优先于按模式规格自动取 prompt_section）。
    pub fn with_mode_section(mut self, section: Option<String>) -> Self {
        self.mode_section = section.filter(|s| !s.is_empty());
        self
    }

    /// 运行一轮并返回编排结果（网关 / 桌面端复用；事件经总线回流）
    ///
    /// 与 [`EngineRunner::run_turn`] 共享同一执行体，额外返回 lead 综合后的答复与
    /// 自动编排形态，供网关封装 `session.event.done` 富终端收据 + `router_decision`
    /// 融合路由决策事件。
    pub async fn run_turn_reply(
        &self,
        text: String,
        session: String,
        bus: Arc<EventBus>,
    ) -> anyhow::Result<crate::orchestration::OrchestrationOutcome> {
        self.execute_turn(text, session, bus).await
    }

    /// 核心执行体：召回经验 → 自动编排 → 沉淀经验 → 返回编排结果。
    ///
    /// 自进化：执行前召回相关经验注入系统提示（越用越聪明）；执行后由 LLM
    /// 自主抽取踩坑 / 决策写入台账（best-effort，任何错误仅记录日志）。
    async fn execute_turn(
        &self,
        text: String,
        session: String,
        bus: Arc<EventBus>,
    ) -> anyhow::Result<crate::orchestration::OrchestrationOutcome> {
        let config = self.config.clone();
        let evolution = self.evolution.clone();
        // 经验抽取配置（在 config 被 move 进 Orchestrator 前取得）
        let extract_cfg = llm_config_for_extraction(&config);
        // 自进化：执行前召回相关经验，注入系统提示
        let hint = recall_hint(&evolution, &text);
        // 跨项目记忆：与自进化经验合并注入（同一系统提示片段）
        let hint = match &self.cross_project_hint {
            Some(cp) if !cp.is_empty() && !hint.is_empty() => format!("{hint}\n{cp}"),
            Some(cp) if !cp.is_empty() => cp.clone(),
            _ => hint,
        };
        // 自动编排器：每条输入由它离线分析并自动选定 Solo / MAX 双带队 / 专家团队 / 集群形态，
        // 用户（及本 TUI）不手动选择编排形态——形态只能由编排内核自动得出。
        let mut orch = match &self.workspace {
            Some(ws) => Orchestrator::new_with_workspace(config, ws.clone()),
            None => Orchestrator::new(config),
        };
        // 关键：把进程级运行时模式（前端 mode.switch 切换的 Plan/Agent/Bloom）
        // 应用到本次编排，否则模式切换形同虚设。
        orch.set_mode(crate::mode::runtime_mode());
        // 模式提示段（DSH 融合 v2 §4 接线收尾）：Plan 模式注入 plan.yaml 的规则段；
        // 显式注入（with_mode_section）优先，否则按当前模式规格取 prompt_section。
        let mode_section = self.mode_section.clone().or_else(|| {
            let kind = crate::mode::runtime_mode();
            crate::mode::ModeManager::new().spec(kind).prompt_section.clone()
        });
        orch.set_mode_section(mode_section);
        orch.set_event_bus(bus.clone(), session.clone());
        if !hint.is_empty() {
            orch.set_experience_hint(hint);
        }
        // W-6：记忆召回稳定前缀（最前端，先于经验提示），落进 P-7 stable 半区
        if let Some(p) = &self.stable_prefix {
            orch.set_stable_prefix(p.clone());
        }
        // Track A：路由决策（思考预算 + prompt 策略）注入 lead 运行时
        if let Some(rd) = &self.route_decision {
            orch.set_route_decision(rd.clone());
        }
        // 重启后上下文恢复：把本会话历史消息灌入 lead
        orch.load_history(crate::gateway::chat::session_history_messages(
            &session, &text,
        ));
        let outcome = orch.dispatch(&text).await?; // Orchestrator 已在失败时发布 Error 事件
                                                   // 自进化：本轮完成后由 LLM 自主抽取踩坑/决策写入台账（best-effort）
        if let Some(evo) = evolution {
            extract_experiences(evo, extract_cfg, text, outcome.lead_reply.clone()).await;
        }
        Ok(outcome)
    }
}

impl EngineRunner for RealEngine {
    fn run_turn(
        &self,
        text: String,
        _mode: ModeKind,
        session: String,
        bus: Arc<EventBus>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>> {
        let config = self.config.clone();
        let evolution = self.evolution.clone();
        let mode_section = self.mode_section.clone();
        Box::pin(async move {
            Self {
                config,
                evolution,
                cross_project_hint: None,
                stable_prefix: None,
                route_decision: None,
                workspace: None,
                mode_section,
            }
            .execute_turn(text, session, bus)
            .await
            .map(|_| ())
        })
    }
}

/// 从配置构造经验抽取用的 LLM 配置。
///
/// API Key 优先取环境变量 `MARGUERITE_API_KEY`（与 `plan_task` 一致），
/// 缺省回退到配置文件中的 key。抽取任务短小，限制 token 与温度以提速降噪。
fn llm_config_for_extraction(config: &Config) -> LLMConfig {
    let api_key =
        std::env::var("MARGUERITE_API_KEY").unwrap_or_else(|_| config.model.api_key.clone());
    let provider = match config.model.provider.as_str() {
        "openai" => LLMProvider::OpenAI,
        "deepseek" => LLMProvider::DeepSeek,
        _ => LLMProvider::Custom {
            base_url: config.model.base_url.clone(),
        },
    };
    LLMConfig {
        provider,
        model: config.model.model_name.clone(),
        api_key,
        base_url: config.model.base_url.clone(),
        max_tokens: 1024,
        temperature: 0.2,
        timeout_secs: 60,
        thinking: crate::model::thinking::ThinkingEffort::Off,
        context_window: crate::model::thinking::CONTEXT_WINDOW_DEFAULT,
    }
}

/// 召回与输入相关的经验，格式化为系统提示片段。
///
/// 无句柄 / 锁失败 / 召回失败 / 无相关经验时均返回空串（不阻塞对话）。
fn recall_hint(evolution: &Option<Arc<Mutex<SelfEvolution>>>, text: &str) -> String {
    let Some(evo) = evolution else {
        return String::new();
    };
    let Ok(guard) = evo.lock() else {
        return String::new();
    };
    let Ok(exps) = guard.recall(text, 5) else {
        return String::new();
    };
    if exps.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "以下是历史经验台账中与你当前请求相关的踩坑 / 决策 / 解法，请优先参考以避免重复犯错：\n",
    );
    for e in &exps {
        s.push_str(&format!(
            "- [{}] {}：{}\n",
            e.kind.as_str(),
            e.title,
            e.body
        ));
    }
    s
}

/// 自主抽取经验并写入台账（best-effort：任何错误仅记录日志，不影响本轮对话）。
///
/// 无 API Key 直接跳过；LLM 输出非 JSON 或解析失败则跳过。
async fn extract_experiences(
    evolution: Arc<Mutex<SelfEvolution>>,
    cfg: LLMConfig,
    user: String,
    assistant: String,
) {
    if cfg.api_key.is_empty() {
        return; // 无 API Key，跳过抽取
    }
    let client = LLMClient::new(cfg);
    let sys = Message::System {
        content: "你是 Marguerite 的经验抽取器。阅读一段「用户请求 + 助手回复」，抽取其中可复用的经验。\
            只输出严格 JSON 数组，元素形如：\
            {\"kind\":\"pitfall|decision|pattern\",\"title\":\"简短标题\",\"body\":\"根因/修法/影响（中文，≤120字）\",\"tags\":[\"关键词\"]}。\
            仅抽取明确的踩坑、决策或解法；若无有价值经验则输出 []。不要输出任何解释性文字，只输出 JSON。"
            .to_string(),
        timestamp: Utc::now(),
    };
    let usr = Message::User {
        content: format!("用户请求：{user}\n\n助手回复：{assistant}"),
        timestamp: Utc::now(),
    };
    let resp = match client.chat_complete(&[sys, usr]).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[self-evolution] 抽取 LLM 调用失败: {e}");
            return;
        }
    };
    let arr = match extract_json_array(&resp.content) {
        Some(v) => v,
        None => {
            eprintln!("[self-evolution] 抽取结果非 JSON，跳过");
            return;
        }
    };
    let Some(items) = arr.as_array() else {
        return;
    };
    let Ok(mut guard) = evolution.lock() else {
        return;
    };
    for item in items {
        let kind = item
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("pitfall");
        let kind = ExperienceKind::from_kind_str(kind);
        let title = item
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let body = item
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if title.is_empty() || body.is_empty() {
            continue;
        }
        let tags: Vec<String> = item
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        // 内容哈希作为幂等签名（相同内容不重复写入）
        let mut hasher = DefaultHasher::new();
        format!("{title}{body}").hash(&mut hasher);
        let sig = format!("auto:{:x}", hasher.finish());
        if let Err(e) = guard.ledger.record(kind, &title, &body, &tags, &sig) {
            eprintln!("[self-evolution] 记录经验失败: {e}");
        }
    }
}

/// 从可能夹杂解释文字的 LLM 输出中截取 JSON 数组。
fn extract_json_array(content: &str) -> Option<serde_json::Value> {
    let trimmed = content.trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if v.is_array() {
            return Some(v);
        }
    }
    let start = content.find('[')?;
    let end = content.rfind(']')?;
    if start < end {
        let slice = &content[start..=end];
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(slice) {
            if v.is_array() {
                return Some(v);
            }
        }
    }
    None
}

/// 工作台构造上下文
pub struct WorkbenchContext {
    /// 会话标识
    pub session: String,
    /// 模型名
    pub model: String,
    /// 配置目录（沙箱工作区）
    pub config_dir: PathBuf,
    /// 事件总线
    pub bus: Arc<EventBus>,
    /// 模式管理器（共享可变性：Arc<RwLock>）
    pub mode: Arc<RwLock<ModeManager>>,
    /// 审批门控
    pub gate: Arc<ApprovalGate>,
    /// 引擎门面
    pub engine: Arc<dyn EngineRunner>,
    /// 沙箱
    pub sandbox: Arc<Sandbox>,
    /// 模型选择器（共享 ModelSelector，与 CLI/GUI 共用同一份 models.json）
    pub selector: Arc<Mutex<ModelSelector>>,
    /// 协作会话（Phase I4，FR-15：多人实时事件流广播/回放）
    pub collab: Arc<Mutex<CollabSession>>,
}

/// 三栏工作台应用
pub struct WorkbenchApp {
    state: WorkbenchState,
    bus: Arc<EventBus>,
    bus_rx: tokio::sync::mpsc::Receiver<Envelope>,
    mode: Arc<RwLock<ModeManager>>,
    gate: Arc<ApprovalGate>,
    engine: Arc<dyn EngineRunner>,
    sandbox: Arc<Sandbox>,
    selector: Arc<Mutex<ModelSelector>>,
    /// 协作会话（Phase I4，FR-15）
    collab: Arc<Mutex<CollabSession>>,
    /// 本地成员 ID（用于 leave/send 标记）
    collab_self: String,
    rt: Runtime,
    workspace: PathBuf,
    slots: SlotRegistry,
    session: String,
    should_quit: bool,
}

impl WorkbenchApp {
    /// 从构造上下文创建应用
    pub fn new(ctx: WorkbenchContext) -> Result<Self> {
        let bus_rx = ctx.bus.subscribe(&format!("{}.*", ctx.session));
        let mut slots = SlotRegistry::new();
        register_builtins(&mut slots);
        let rt = Builder::new_multi_thread().enable_all().build()?;

        // Phase C：后台工作区文件监听（依赖无关轮询），变更经总线回流 TUI
        let watcher = workspace_watcher::WorkspaceWatcher::new(ctx.config_dir.clone())
            .with_interval(Duration::from_secs(2));
        let wbus = ctx.bus.clone();
        let wsession = ctx.session.clone();
        rt.spawn(async move {
            watcher.run(wbus, wsession).await;
        });

        let mut app = Self {
            state: WorkbenchState::new(ctx.session.clone(), ctx.model.clone()),
            bus: ctx.bus,
            bus_rx,
            mode: ctx.mode,
            gate: ctx.gate,
            engine: ctx.engine,
            sandbox: ctx.sandbox,
            selector: ctx.selector,
            collab: ctx.collab,
            collab_self: ctx.session.clone(),
            rt,
            workspace: ctx.config_dir,
            slots,
            session: ctx.session,
            should_quit: false,
        };

        // Phase 4 / AC-08：启动时加载跨会话命令历史（启用落盘）
        app.state.composer.history = crate::tui::workbench::history::CommandHistory::load();

        // H5：顶栏模型显示从选择器回写（主 + 副），保证与 models.json 一致
        if let Ok(sel) = app.selector.lock() {
            if let Some(p) = sel.registry().active_profile() {
                app.state.model = p.label.clone();
            }
            if let Some(p) = sel.registry().copilot_profile() {
                app.state.copilot_model = p.label.clone();
            }
        }
        Ok(app)
    }

    /// 从配置装配默认上下文（bus/mode/gate/sandbox 均为默认，engine 用 MockEngine）
    pub fn from_config(config: &Config) -> Result<Self> {
        let session = format!("ws.{}", Uuid::new_v4());
        let bus = Arc::new(EventBus::default());
        let mode = Arc::new(RwLock::new(ModeManager::new()));
        let gate = Arc::new(ApprovalGate::default());
        let sandbox = Arc::new(Sandbox::new(&Config::config_dir()));
        let engine: Arc<dyn EngineRunner> = Arc::new(RealEngine::from_config(config.clone()));
        // H5：加载模型选择器（与 CLI/GUI 共用同一份 models.json；缺失则种子默认值）
        let selector = Arc::new(Mutex::new(ModelSelector::load()?));
        // I4：协作会话（每工作台一个 CollabSession，复用核心 Envelope 事件流）
        let collab = Arc::new(Mutex::new(CollabSession::new(&session)));
        let ctx = WorkbenchContext {
            session,
            model: config.model.model_name.clone(),
            config_dir: Config::config_dir(),
            bus,
            mode,
            gate,
            engine,
            sandbox,
            selector,
            collab,
        };
        Self::new(ctx)
    }

    /// 主循环：同步 poll + 后台 spawn
    pub fn run(&mut self) -> Result<()> {
        enable_raw_mode()?;
        execute!(std::io::stdout(), EnterAlternateScreen)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
        install_panic_hook();

        while !self.should_quit {
            // (1) 非阻塞排空事件总线 → UiEvent::Bus
            while let Ok(env) = self.bus_rx.try_recv() {
                let effs = self.state.reduce(UiEvent::Bus(env));
                self.apply_effects(effs);
            }
            // (2) 重绘（渲染只读 self.state）
            terminal.draw(|f| self.render(f))?;
            // (3) 输入：200ms 轮询保活
            if event::poll(Duration::from_millis(200))? {
                match event::read()? {
                    Event::Key(k) => {
                        let e = self.state.reduce(UiEvent::Key(k));
                        self.apply_effects(e);
                    }
                    Event::Mouse(m) => {
                        let e = self.state.reduce(UiEvent::Mouse(m));
                        self.apply_effects(e);
                    }
                    Event::Resize(w, h) => {
                        self.state.reduce(UiEvent::Resize(w, h));
                    }
                    _ => {}
                }
            } else {
                self.state.reduce(UiEvent::Tick);
            }
        }

        disable_raw_mode()?;
        execute!(std::io::stdout(), LeaveAlternateScreen)?;
        Ok(())
    }

    /// 渲染：布局 → 五区域渲染（折叠区域按需跳过）
    fn render(&self, frame: &mut ratatui::Frame) {
        let collapse = self.state.collapse;
        let lay: WorkbenchLayout = split(frame.area(), &collapse);
        crate::tui::workbench::region::activity::render_activity(frame, lay.activity, &self.state);
        if let Some(r) = lay.sidebar {
            crate::tui::workbench::region::sidebar::render_sidebar(frame, r, &self.state);
        }
        crate::tui::workbench::region::main::render_main(frame, lay.main, &self.state);
        if let Some(r) = lay.rail {
            crate::tui::workbench::region::rail::render_rail(frame, r, &self.state);
        }
        crate::tui::workbench::region::status::render_status(frame, lay.status, &self.state);
        // H5：顶栏（mode + 主/副模型 + 提示 + 通知），与 GUI 顶栏选择器同构
        crate::tui::workbench::region::topbar::render_topbar(frame, lay.topbar, &self.state);

        if self.state.help_open {
            render_help_overlay(frame, frame.area());
        }
        let _ = &self.slots; // 注册表持有插槽实现（F1 契约），渲染走 free 函数
    }

    /// 副作用执行：需要 async / 改动外部状态的操作集中此处
    fn apply_effects(&mut self, effs: Vec<UiEffect>) {
        for eff in effs {
            match eff {
                UiEffect::SubmitPrompt(text) => self.spawn_turn(text),
                UiEffect::RunSandbox(argv) => self.spawn_sandbox(argv),
                UiEffect::SwitchMode(m) => {
                    if let Ok(mut g) = self.mode.write() {
                        let _ = g.switch(m);
                    }
                    self.state.mode = m;
                }
                UiEffect::ResolveApproval { id, outcome } => {
                    self.gate.resolve(&id, outcome);
                    self.state.approval = None;
                    self.state.interaction = InteractionState::Idle;
                }
                UiEffect::Quit => self.should_quit = true,
                UiEffect::ModelAction(op) => self.apply_model_action(op),
                UiEffect::DeployAction(op) => self.apply_deploy(op),
                UiEffect::CollabAction(op) => self.apply_collab(op),
                UiEffect::Render
                | UiEffect::ToggleSidebar
                | UiEffect::ToggleRail
                | UiEffect::SwitchMainTab(_)
                | UiEffect::SwitchRailTab(_)
                | UiEffect::SwitchActivity(_)
                | UiEffect::NewSession
                | UiEffect::RestoreSession
                | UiEffect::RequestApproval(_)
                | UiEffect::ShowHelp
                | UiEffect::OpenCommandPalette
                | UiEffect::PasteFile(_) => { /* 已在 state 生效或占位 */ }
            }
        }
    }

    /// 副作用执行：模型操作（H5 顶栏同构，与 GUI model_use/copilot/add/remove 语义一致）
    ///
    /// 锁定共享 `ModelSelector` 执行持久化，并将结果显示回写到顶栏状态
    /// （`state.model` / `state.copilot_model` / `state.notice`），`/model list` 回显到聊天。
    fn apply_model_action(&mut self, op: ModelOp) {
        let mut sel = match self.selector.lock() {
            Ok(g) => g,
            Err(_) => {
                self.state.notice = Some("模型选择器锁定失败".into());
                return;
            }
        };
        match op {
            ModelOp::UseActive(id) => match sel.use_active(&id) {
                Ok(_) => {
                    if let Some(p) = sel.registry().active_profile() {
                        self.state.model = p.label.clone();
                        self.state.notice = Some(format!("主模型已切换: {}", p.label));
                    }
                }
                Err(e) => self.state.notice = Some(format!("切换主模型失败: {e}")),
            },
            ModelOp::UseCopilot(opt) => {
                let label = match &opt {
                    Some(id) => sel
                        .registry()
                        .get(id)
                        .map(|p| p.label.clone())
                        .unwrap_or_default(),
                    None => String::new(),
                };
                match sel.use_copilot(opt) {
                    Ok(_) => {
                        self.state.copilot_model = label.clone();
                        self.state.notice = Some(if label.is_empty() {
                            "已关闭 Bloom 副模型".into()
                        } else {
                            format!("Bloom 副模型已设: {label}")
                        });
                    }
                    Err(e) => self.state.notice = Some(format!("设置副模型失败: {e}")),
                }
            }
            ModelOp::Add(p) => match sel.add(p.clone()) {
                Ok(_) => self.state.notice = Some(format!("已添加模型: {}", p.label)),
                Err(e) => self.state.notice = Some(format!("添加模型失败: {e}")),
            },
            ModelOp::Remove(id) => match sel.remove(&id) {
                Ok(p) => {
                    // 若删除的是当前副模型，清空顶栏副模型显示
                    if self.state.copilot_model == p.label {
                        self.state.copilot_model = String::new();
                    }
                    if let Some(active) = sel.registry().active_profile() {
                        self.state.model = active.label.clone();
                    }
                    self.state.notice = Some(format!("已删除模型: {}", p.label));
                }
                Err(e) => self.state.notice = Some(format!("删除模型失败: {e}")),
            },
            ModelOp::List => {
                let mut lines = String::from(
                    "可用模型（/model use <id> 切换，/model copilot <id|off> 设副模型）：\n",
                );
                for p in sel.registry().list() {
                    let tag = if Some(&p.id) == sel.registry().copilot.as_ref() {
                        "副"
                    } else if p.id == sel.registry().active {
                        "主"
                    } else {
                        "  "
                    };
                    lines.push_str(&format!(
                        "  [{}] {}  (id={}, provider={})\n",
                        tag, p.label, p.id, p.provider
                    ));
                }
                self.state.chat.messages.push(ChatMessage {
                    role: ChatRole::System,
                    content: lines,
                });
            }
        }
    }

    /// 副作用执行：部署演练（Phase I4，FR-16）
    ///
    /// TUI 侧默认 dry-run（不实际连接远端），用录制型传输跑一遍真实编排，
    /// 把将执行的远端步骤写入部署面板历史，供 Deploy 右栏呈现。
    fn apply_deploy(&mut self, op: DeployOp) {
        let DeployOp::Run {
            target,
            service,
            no_ssl,
        } = op;
        let service = service.unwrap_or_else(|| "marguerite".to_string());
        let mut run = crate::tui::workbench::state::DeployRunView {
            target: target.clone(),
            kind: DeployKind::Ssh,
            status: String::new(),
            detail: String::new(),
            sha256: None,
        };

        if target.starts_with("ssh://") {
            run.kind = DeployKind::Ssh;
            match self.dry_run_ssh(&target, &service) {
                Ok((detail, sha)) => {
                    run.status = "planned (dry-run)".into();
                    run.detail = detail;
                    run.sha256 = Some(sha);
                }
                Err(e) => {
                    run.status = format!("error: {e}");
                    run.detail = e.to_string();
                }
            }
        } else if target.starts_with("baota://") {
            run.kind = DeployKind::Baota;
            let alias = target.trim_start_matches("baota://").to_string();
            match self.dry_run_baota(&alias, &service, no_ssl) {
                Ok(detail) => {
                    run.status = "planned (dry-run)".into();
                    run.detail = detail;
                }
                Err(e) => {
                    run.status = format!("error: {e}");
                    run.detail = e.to_string();
                }
            }
        } else {
            run.status = "error: 未知目标前缀（须 ssh:// 或 baota://）".into();
            run.detail = target.clone();
        }

        // 写入部署面板历史（上限 20）
        self.state.deploy_panel.current_target = Some(target);
        self.state.deploy_panel.runs.push_back(run);
        while self.state.deploy_panel.runs.len() > 20 {
            self.state.deploy_panel.runs.pop_front();
        }

        // 聊天系统消息摘要
        let last_ok = self
            .state
            .deploy_panel
            .runs
            .back()
            .map(|r| !r.status.starts_with("error"))
            .unwrap_or(false);
        let summary = format!(
            "[部署演练] {} → {}",
            self.state
                .deploy_panel
                .current_target
                .as_deref()
                .unwrap_or(""),
            if last_ok {
                "已规划（dry-run）"
            } else {
                "失败"
            }
        );
        self.state.chat.messages.push(ChatMessage {
            role: ChatRole::System,
            content: summary,
        });
    }

    /// SSH dry-run：写临时占位二进制 → 录制型传输跑真实编排 → 收集远端步骤
    fn dry_run_ssh(&self, target: &str, service: &str) -> anyhow::Result<(String, String)> {
        let t = SshTarget::from_url(target)?;
        let opts = DeployOptions {
            service: service.to_string(),
            health_url: None,
            use_systemd: false,
            dry_run: true,
        };
        let bin = Self::write_temp_bin();
        let transport = RecordingTransport::new();
        let report = SshDeployer::deploy(&transport, &t, &opts, &bin)?;
        let _ = std::fs::remove_file(&bin);
        let mut steps = transport.exec_calls().clone();
        steps.push(format!("upload -> {service}  (scp)"));
        let detail = steps.join("\n");
        Ok((detail, report.binary_sha256))
    }

    /// 宝塔 dry-run：录制型传输跑真实编排（不解析面板密钥，不触网）
    fn dry_run_baota(&self, alias: &str, service: &str, no_ssl: bool) -> anyhow::Result<String> {
        let opts = BaotaDeployOptions {
            service: service.to_string(),
            site: Some(alias.to_string()),
            health_url: None,
            ssl: !no_ssl,
            dry_run: true,
        };
        let bin = Self::write_temp_bin();
        let transport = RecordingBaotaTransport::new();
        let _report = self
            .rt
            .block_on(crate::deploy::baota::deploy(&transport, &opts, &bin));
        let _ = std::fs::remove_file(&bin);
        let actions: Vec<String> = transport.calls().iter().map(|(a, _)| a.clone()).collect();
        Ok(actions.join("\n"))
    }

    /// 写临时占位二进制（供 dry-run 计算 sha256 / 路径），调用方负责删除
    fn write_temp_bin() -> std::path::PathBuf {
        let name = format!("mgt_dry_{}.bin", uuid::Uuid::new_v4());
        let p = std::env::temp_dir().join(name);
        let _ = std::fs::write(&p, b"marguerite-dry-run-binary");
        p
    }

    /// 副作用执行：协作会话操作（Phase I4，FR-15）
    ///
    /// 驱动共享 `CollabSession`：join/leave/broadcast/replay，并同步到 `state.collab` 视图。
    fn apply_collab(&mut self, op: CollabOp) {
        let mut guard = match self.collab.lock() {
            Ok(g) => g,
            Err(_) => {
                self.state
                    .collab
                    .log
                    .push_back("[错误] 协作会话锁定失败".into());
                return;
            }
        };
        match op {
            CollabOp::Join(name) => {
                let is_new = guard.join(&name);
                // 本地成员以该名标识，后续 leave/send 一致归因
                self.collab_self = name.clone();
                let msg = if is_new {
                    format!("成员 {name} 加入会话")
                } else {
                    format!("成员 {name} 已在线")
                };
                self.state.collab.log.push_back(msg);
            }
            CollabOp::Leave => {
                let was = guard.leave(&self.collab_self);
                self.state.collab.log.push_back(if was {
                    "你已离开协作会话".into()
                } else {
                    "你本就不在会话中".into()
                });
            }
            CollabOp::Send(text) => {
                guard.broadcast(Envelope::new(
                    EventType::Status,
                    serde_json::json!({ "kind": "collab.message", "from": self.collab_self, "text": text }),
                    &self.session,
                ));
                self.state.collab.log.push_back(format!("你: {text}"));
            }
            CollabOp::Replay => {
                let events = guard.replay();
                self.state.collab.log.clear();
                for e in &events {
                    let kind = e
                        .payload
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("-");
                    let summary = match kind {
                        "collab.message" => format!(
                            "{}: {}",
                            e.payload
                                .get("from")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?"),
                            e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("")
                        ),
                        "collab.member_joined" => format!(
                            "成员 {} 加入",
                            e.payload
                                .get("member")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?")
                        ),
                        _ => format!("[{:?}]", e.r#type),
                    };
                    self.state.collab.log.push_back(summary);
                }
                if events.is_empty() {
                    self.state.collab.log.push_back("（会话暂无事件）".into());
                }
            }
        }
        // 同步在线成员到视图
        self.state.collab.members = guard.members();
    }

    /// 引擎长任务放后台线程，事件经 EventBus 回流 UI（不阻塞主循环）
    fn spawn_turn(&self, text: String) {
        let engine = self.engine.clone();
        let bus = self.bus.clone();
        let mode = match self.mode.read() {
            Ok(g) => g.current(),
            Err(_) => ModeKind::Agent,
        };
        let session = self.session.clone();
        self.rt.spawn(async move {
            let _ = engine.run_turn(text, mode, session, bus).await;
        });
    }

    /// 沙箱命令放后台执行，结果以 terminal 事件回流总线
    fn spawn_sandbox(&self, argv: Vec<String>) {
        let sandbox = self.sandbox.clone();
        let bus = self.bus.clone();
        let mode = match self.mode.read() {
            Ok(g) => g.current(),
            Err(_) => ModeKind::Agent,
        };
        let session = self.session.clone();
        let cwd = self.workspace.clone();
        self.rt.spawn(async move {
            match sandbox.execute(&argv, &cwd, mode).await {
                Ok(r) => {
                    let ok = r.exit_code == 0;
                    let summary = if r.stdout.is_empty() {
                        r.stderr.clone()
                    } else {
                        r.stdout.clone()
                    };
                    bus.publish(Envelope::new(
                        EventType::ToolResult,
                        json!({ "kind": "terminal", "tool": "shell", "ok": ok, "summary": summary }),
                        session.as_str(),
                    ));
                }
                Err(e) => {
                    bus.publish(Envelope::error(session.as_str(), "sandbox", &format!("{e}")));
                }
            }
        });
    }
}

/// 帮助浮层（? 关闭）
fn render_help_overlay(frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
    use ratatui::layout::{Constraint, Layout};
    use ratatui::style::Style;

    let t = Theme::effective();
    let v = Layout::vertical([
        Constraint::Percentage(20),
        Constraint::Percentage(60),
        Constraint::Percentage(20),
    ])
    .split(area);
    let h = Layout::horizontal([
        Constraint::Percentage(20),
        Constraint::Percentage(60),
        Constraint::Percentage(20),
    ])
    .split(v[1]);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.brand))
        .title(" 帮助 (? 关闭) ");
    let p = Paragraph::new(command::help_text()).block(block);
    frame.render_widget(Clear, h[1]);
    frame.render_widget(p, h[1]);
}

/// 安装 panic hook：还原终端，避免卡死
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        original(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::config::Config;
    use crate::tui::workbench::layout::{compute_collapse, CollapseState};
    use crate::tui::workbench::region::activity::render_activity;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;

    #[test]
    fn test_extract_json_array_direct() {
        let v = extract_json_array("[{\"kind\":\"pitfall\",\"title\":\"t\",\"body\":\"b\"}]");
        assert!(v.is_some());
        assert!(v.unwrap().as_array().unwrap().len() == 1);
    }

    #[test]
    fn test_extract_json_array_with_surrounding_text() {
        let out =
            "好的，这是结果：\n[{\"kind\":\"decision\",\"title\":\"x\",\"body\":\"y\"}]\n以上。";
        let v = extract_json_array(out);
        assert!(v.is_some(), "应能截取出 JSON 数组");
        assert_eq!(v.unwrap().as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_extract_json_array_no_array() {
        assert!(extract_json_array("今天天气不错").is_none());
        assert!(extract_json_array("{}").is_none());
    }

    #[test]
    fn test_recall_hint_empty_without_evolution() {
        // 无经验台账句柄时返回空串（不阻塞对话）
        assert!(recall_hint(&None, "任何输入").is_empty());
    }

    #[test]
    fn test_from_config_builds_ok() {
        let app = WorkbenchApp::from_config(&Config::default());
        assert!(app.is_ok(), "from_config 应能构造 WorkbenchApp");
    }

    #[test]
    fn test_from_config_initial_state() {
        let app = WorkbenchApp::from_config(&Config::default()).unwrap();
        assert!(app.state.session.starts_with("ws."));
        assert_eq!(app.state.mode, ModeKind::Agent);
        assert_eq!(
            app.state.main_tab,
            crate::tui::workbench::state::MainTab::Chat
        );
    }

    #[tokio::test]
    async fn test_mock_engine_publishes_events() {
        let bus = Arc::new(EventBus::default());
        let mut rx = bus.subscribe("**");
        let engine = MockEngine;
        let res = engine
            .run_turn("hi".into(), ModeKind::Agent, "ws.test".into(), bus)
            .await;
        assert!(res.is_ok());
        let mut count = 0;
        while rx.try_recv().is_ok() {
            count += 1;
        }
        assert!(count >= 2, "mock 引擎应至少发布 Thinking + Done");
    }

    #[test]
    fn test_spawn_sandbox_constructs() {
        // 仅验证构造与字段，不真正运行终端
        let app = WorkbenchApp::from_config(&Config::default()).unwrap();
        assert!(app.slots.ids().len() >= 5, "应注册至少 5 个区域插槽");
    }

    // ---- A4.4 RealEngine 真实事件流 ----

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
    async fn test_real_engine_publishes_events() {
        let url = mock_llm_server(
            200,
            r#"{"choices":[{"message":{"content":"你好，Marguerite！"},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":3,"total_tokens":8}}"#,
        )
        .await;
        let bus = Arc::new(EventBus::default());
        let mut rx = bus.subscribe("**");
        let engine = RealEngine::from_config(llm_config(&url));
        let res = engine
            .run_turn("hi".into(), ModeKind::Agent, "ws.test".into(), bus)
            .await;
        assert!(res.is_ok(), "RealEngine 应成功运行一轮");
        // 至少应发布 Thinking（开始）与 Done（完成）
        let mut types = vec![];
        while let Ok(env) = rx.try_recv() {
            types.push(env.r#type);
        }
        assert!(
            types.contains(&EventType::Thinking),
            "RealEngine 应发布 Thinking 事件"
        );
        assert!(
            types.contains(&EventType::Done),
            "RealEngine 应发布 Done 事件"
        );
    }

    // ---- I4 部署面板 + 协作事件流（FR-16 / FR-15）----

    #[test]
    fn test_state_has_i4_views_initialized() {
        let app = WorkbenchApp::from_config(&Config::default()).unwrap();
        assert!(
            app.state.deploy_panel.runs.is_empty(),
            "部署面板历史初始为空"
        );
        assert!(app.state.collab.members.is_empty(), "协作成员初始为空");
        assert!(app.state.collab.log.is_empty(), "协作日志初始为空");
    }

    #[test]
    fn test_apply_deploy_ssh_dry_run_writes_panel() {
        let mut app = WorkbenchApp::from_config(&Config::default()).unwrap();
        app.apply_deploy(crate::tui::workbench::state::DeployOp::Run {
            target: "ssh://example.com".into(),
            service: None,
            no_ssl: false,
        });
        assert_eq!(app.state.deploy_panel.runs.len(), 1, "应写入一条部署演练");
        let run = app.state.deploy_panel.runs.back().unwrap();
        assert!(
            matches!(run.kind, crate::tui::workbench::state::DeployKind::Ssh),
            "ssh 目标应记为 Ssh 演练"
        );
        assert_eq!(
            app.state.deploy_panel.current_target.as_deref(),
            Some("ssh://example.com")
        );
    }

    #[test]
    fn test_apply_deploy_baota_dry_run_writes_panel() {
        let mut app = WorkbenchApp::from_config(&Config::default()).unwrap();
        app.apply_deploy(crate::tui::workbench::state::DeployOp::Run {
            target: "baota://mysite".into(),
            service: Some("marguerite".into()),
            no_ssl: true,
        });
        assert_eq!(app.state.deploy_panel.runs.len(), 1, "应写入一条部署演练");
        let run = app.state.deploy_panel.runs.back().unwrap();
        assert!(
            matches!(run.kind, crate::tui::workbench::state::DeployKind::Baota),
            "baota 目标应记为 Baota 演练"
        );
    }

    #[test]
    fn test_apply_deploy_unknown_target_errors() {
        let mut app = WorkbenchApp::from_config(&Config::default()).unwrap();
        app.apply_deploy(crate::tui::workbench::state::DeployOp::Run {
            target: "ftp://nope".into(),
            service: None,
            no_ssl: false,
        });
        let run = app.state.deploy_panel.runs.back().unwrap();
        assert!(
            run.status.starts_with("error: 未知目标前缀"),
            "未知前缀应报错而非静默"
        );
    }

    #[test]
    fn test_apply_collab_join_send_replay_leave() {
        let mut app = WorkbenchApp::from_config(&Config::default()).unwrap();
        app.apply_collab(crate::tui::workbench::state::CollabOp::Join("alice".into()));
        assert!(
            app.state.collab.members.iter().any(|m| m == "alice"),
            "join 后成员列表应含 alice"
        );
        app.apply_collab(crate::tui::workbench::state::CollabOp::Send("hello".into()));
        app.apply_collab(crate::tui::workbench::state::CollabOp::Replay);
        assert!(!app.state.collab.log.is_empty(), "replay 后协作日志非空");
        app.apply_collab(crate::tui::workbench::state::CollabOp::Leave);
        assert!(
            !app.state.collab.members.iter().any(|m| m == "alice"),
            "leave 后成员列表应移除 alice"
        );
    }

    /// I4 实机复核：隔离渲染右栏（Deploy / Collab），捕获真实 `render_rail` 输出。
    /// 等价于一帧真实 TUI 渲染（无显示器环境下最严格的面板渲染验证）。
    #[test]
    fn test_i4_render_deploy_and_collab_panels() {
        use crate::tui::workbench::region::rail::render_rail;
        use crate::tui::workbench::state::{CollabOp, DeployOp, RailTab, UiEffect};
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let norm = |s: &str| s.replace(' ', "");
        let collect = |term: &Terminal<TestBackend>| -> String {
            term.backend()
                .buffer()
                .content()
                .iter()
                .map(|c| c.symbol())
                .collect()
        };
        // 转储：非 ASCII 可打印字符标注为 U+XXXX，便于核对全角占位
        let dump_esc = |term: &Terminal<TestBackend>, path: &str| {
            let buf = term.backend().buffer();
            let w = buf.area.width as usize;
            let h = buf.area.height as usize;
            let content = buf.content();
            let mut out = String::new();
            for y in 0..h {
                let mut row = String::new();
                for x in 0..w {
                    let s = content[y * w + x].symbol();
                    for ch in s.chars() {
                        if ch == ' ' {
                            row.push(' ');
                        } else if ch.is_ascii_graphic() {
                            row.push(ch);
                        } else {
                            row.push_str(&format!("<U+{:04X}>", ch as u32));
                        }
                    }
                }
                out.push_str(&format!("{:2} |{}\n", y, row));
            }
            let _ = std::fs::write(path, out);
        };

        let mut app = WorkbenchApp::from_config(&Config::default()).unwrap();

        // (1) /deploy run ssh://deploy@203.0.113.10:22
        app.apply_effects(vec![UiEffect::DeployAction(DeployOp::Run {
            target: "ssh://deploy@203.0.113.10:22".into(),
            service: Some("marguerite".into()),
            no_ssl: false,
        })]);
        app.state.rail_tab = RailTab::Deploy;
        let mut t_dep = Terminal::new(TestBackend::new(34, 20)).unwrap();
        t_dep
            .draw(|f| render_rail(f, f.area(), &app.state))
            .unwrap();
        dump_esc(
            &t_dep,
            "D:/Downloads/Marguerite/mgt_tmp/render_deploy_dump.txt",
        );
        let d = collect(&t_dep);
        assert!(
            norm(&d).contains("部署面板"),
            "Deploy 面板应渲染标题「部署面板」"
        );
        assert!(
            d.contains("ssh://deploy@203.0.113.10:22"),
            "Deploy 面板应显示 ssh 目标"
        );
        assert!(d.contains("[SSH]"), "Deploy 面板应标 SSH 类型徽标");
        assert!(
            norm(&d).contains("planned(dry-run)"),
            "Deploy 面板应显示 dry-run 规划状态"
        );
        assert!(d.contains("sha256:"), "Deploy 面板应显示二进制 sha256 摘要");
        // 修复回归：34 宽右栏须完整显示 7 个 tab（含新增 Dep/Col），不得裁切
        for tab in ["Ag", "Tl", "Cx", "Mm", "Tk", "Dp", "Co"] {
            assert!(d.contains(tab), "tabbar 应显示 tab 缩写 {tab}");
        }

        // (2) /collab join alice
        app.apply_effects(vec![UiEffect::CollabAction(CollabOp::Join("alice".into()))]);
        app.state.rail_tab = RailTab::Collab;
        let mut t_col = Terminal::new(TestBackend::new(34, 20)).unwrap();
        t_col
            .draw(|f| render_rail(f, f.area(), &app.state))
            .unwrap();
        dump_esc(
            &t_col,
            "D:/Downloads/Marguerite/mgt_tmp/render_collab_dump.txt",
        );
        let c = collect(&t_col);
        assert!(
            norm(&c).contains("协作事件流"),
            "Collab 面板应渲染标题「协作事件流」"
        );
        assert!(c.contains("alice"), "Collab 面板应显示成员 alice");
        assert!(
            norm(&c).contains("1人在线"),
            "Collab 面板应显示在线人数统计"
        );
        assert!(
            norm(&c).contains("成员alice加入会话"),
            "Collab 面板应记录 join 事件到事件流"
        );
    }

    // ===== Phase J / J5 五源对齐像素复核 + P0 视觉门禁 =====
    //
    // J5 目标：以实机 TestBackend 渲染校验「TRAE 1:1 三栏」像素对齐——
    // 列宽映射 活动栏 2 列 ↔ 48px / 侧边栏 22 列 ↔ 236px / 右栏 34 列 ↔ 288px /
    // 顶栏·状态栏 1 行 ↔ 24px；并施加 P0 视觉门禁（禁 emoji / 禁紫粉渐变 / 品牌蓝锚点）。

    /// P0-1 emoji 检测：返回命中 emoji 字符（主题契约明确批准的语义状态点除外）。
    ///
    /// 允许列表 `▮✖◉·` 为 `theme.rs` 设计系统明确批准的 Unicode 几何状态点（非功能图标），
    /// 不属 P0-1 禁用范围（P0-1 禁的是「emoji 作为功能图标」）。
    fn j5_emoji_hits(term: &Terminal<TestBackend>) -> Vec<char> {
        const ALLOWED: &[char] = &['▮', '✖', '◉', '·'];
        let mut hits = Vec::new();
        for c in term.backend().buffer().content().iter() {
            for ch in c.symbol().chars() {
                if ALLOWED.contains(&ch) {
                    continue;
                }
                if is_emoji_char(ch) {
                    hits.push(ch);
                }
            }
        }
        hits
    }

    /// emoji 码点区间判定（对齐 P0 正则扫描规则）
    fn is_emoji_char(ch: char) -> bool {
        let cp = ch as u32;
        (0x1F300..=0x1F9FF).contains(&cp)
            || (0x2600..=0x26FF).contains(&cp)
            || (0x2700..=0x27BF).contains(&cp)
            || (0xFE00..=0xFE0F).contains(&cp)
            || (0x1F000..=0x1F02F).contains(&cp)
            || (0x1F0A0..=0x1F0FF).contains(&cp)
            || (0x1F100..=0x1F64F).contains(&cp)
            || (0x1F680..=0x1F6FF).contains(&cp)
            || (0x1FA00..=0x1FA6F).contains(&cp)
            || (0x1FA70..=0x1FAFF).contains(&cp)
            || (0x2300..=0x23FF).contains(&cp) // 杂项符号与箭头（计时类 emoji：沙漏/钟表等）
            || cp == 0x200D
            || cp == 0x20E3
            || (0xE0020..=0xE007F).contains(&cp)
    }

    /// P0-2 紫粉渐变检测：返回命中的禁用色值（Indigo/SlateBlue/Violet/Magenta/Pink）。
    fn j5_forbidden_color_present(term: &Terminal<TestBackend>) -> Vec<(u8, u8, u8)> {
        let mut hits = Vec::new();
        for c in term.backend().buffer().content().iter() {
            for col in [c.fg, c.bg] {
                if let ratatui::style::Color::Rgb(r, g, b) = col {
                    if palette_audit::FORBIDDEN_VIOLET_PINK.contains(&(r, g, b)) {
                        hits.push((r, g, b));
                    }
                }
            }
        }
        hits
    }

    /// 品牌蓝 #3B82F6 锚点检测：P0 视觉达标必须有蓝主色出现。
    fn j5_brand_blue_present(term: &Terminal<TestBackend>) -> bool {
        for c in term.backend().buffer().content().iter() {
            for col in [c.fg, c.bg] {
                if let ratatui::style::Color::Rgb(r, g, b) = col {
                    if (r, g, b) == palette_audit::REQUIRED_BRAND_BLUE {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// 收集全帧文本（逐 cell symbol 拼接）。
    fn j5_collect(term: &Terminal<TestBackend>) -> String {
        term.backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    /// J5 实机全帧：五 region 渲染 + P0 视觉门禁。等价一帧真实 TUI 渲染的最严格验证。
    #[test]
    fn test_j5_full_frame_five_regions_and_p0_gates() {
        let app = WorkbenchApp::from_config(&Config::default()).unwrap();
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| app.render(f)).unwrap();

        // 转储全帧供五源对齐人工核对（mgt_tmp 为 I4 已建临时目录）
        {
            let buf = term.backend().buffer();
            let w = buf.area.width as usize;
            let h = buf.area.height as usize;
            let content = buf.content();
            let mut out = String::new();
            for y in 0..h {
                let mut row = String::new();
                for x in 0..w {
                    let s = content[y * w + x].symbol();
                    row.push_str(if s == " " { " " } else { s });
                }
                out.push_str(&format!("{:2} |{}\n", y, row));
            }
            let _ = std::fs::write("D:/Downloads/Marguerite/mgt_tmp/j5_fullframe_dump.txt", out);
        }

        let d = j5_collect(&term);
        let norm = d.replace(' ', "");

        // J1 主区 TabBar：Chat / Code / Terminal
        assert!(d.contains("Chat"), "J1 主区 TabBar 应含 Chat");
        assert!(d.contains("Code"), "J1 主区 TabBar 应含 Code");
        assert!(d.contains("Terminal"), "J1 主区 TabBar 应含 Terminal");
        // J1 侧边栏（默认文件树面板）
        assert!(norm.contains("侧边栏"), "J1 侧边栏应渲染标题");
        assert!(d.contains("src/"), "J1 侧边栏文件树应含 src/");
        // J4 顶栏：模式 + 模型选择器同构显示
        assert!(
            d.contains("Agent") || d.contains("Bloom"),
            "J4 顶栏应显示模式标识"
        );
        // 注意：ratatui 全角 CJK 在 buffer 中以「字符+续格空格」存储，故用 norm 去空格匹配
        assert!(norm.contains("模型"), "J4 顶栏应显示模型标签");
        // J3 右栏 7 tab 全可见（Ag│Tl│Cx│Mm│Tk│Dp│Co）
        for tab in ["Ag", "Tl", "Cx", "Mm", "Tk", "Dp", "Co"] {
            assert!(d.contains(tab), "J3 右栏 tabbar 应显示 {tab}");
        }
        // J5 状态栏
        assert!(d.contains("tok"), "J5 状态栏应显示 token 用量");
        // 全角 CJK 在 buffer 中以「字符+续格空格」存储，故用 norm 去空格匹配
        assert!(
            norm.contains("就绪") || norm.contains("已连接") || norm.contains("已断开"),
            "J5 状态栏应显示连接态"
        );

        // P0-1 禁 emoji（允许主题语义状态点）
        let hits = j5_emoji_hits(&term);
        assert!(
            hits.is_empty(),
            "P0-1 全帧不得出现 emoji 功能图标，命中: {:?}",
            hits
        );
        // P0-2 禁紫粉渐变
        let vio = j5_forbidden_color_present(&term);
        assert!(
            vio.is_empty(),
            "P0-2 全帧不得出现紫/靛/粉色值，命中: {:?}",
            vio
        );
        // 品牌蓝 #3B82F6 必须出现（P0 视觉达标锚点）
        assert!(
            j5_brand_blue_present(&term),
            "品牌蓝 #3B82F6 必须出现在全帧渲染中"
        );
    }

    /// J5 活动栏单字符标签 + TRAE 1:1 列宽映射断言（48/236/288/24）。
    #[test]
    fn test_j5_activity_bar_glyphs_and_column_width() {
        let app = WorkbenchApp::from_config(&Config::default()).unwrap();
        // 活动栏宽 2，隔离渲染
        let mut term = Terminal::new(TestBackend::new(2, 10)).unwrap();
        term.draw(|f| render_activity(f, f.area(), &app.state))
            .unwrap();
        let d = j5_collect(&term);
        for g in ["F", "S", "K", "C", "O"] {
            assert!(d.contains(g), "活动栏应渲染单字符标签 {g}");
        }
        // 列宽映射：activity=2 列 ↔ 48px；sidebar=22 ↔ 236px；rail=34 ↔ 288px；
        // topbar/status 高 1 ↔ 24px（TRAE 1:1）。
        let area = Rect::new(0, 0, 120, 40);
        let lay = split(area, &CollapseState::default());
        assert_eq!(lay.activity.width, 2, "活动栏 2 列 ↔ TRAE GUI 48px");
        assert_eq!(
            lay.sidebar.expect("侧边栏应存在").width,
            22,
            "侧边栏 22 列 ↔ TRAE GUI 236px"
        );
        assert_eq!(
            lay.rail.expect("右栏应存在").width,
            34,
            "右栏 34 列 ↔ TRAE GUI 288px"
        );
        assert_eq!(lay.topbar.height, 1, "顶栏 1 行 ↔ TRAE GUI 24px");
        assert_eq!(lay.status.height, 1, "状态栏 1 行 ↔ TRAE GUI 24px");
        assert_eq!(lay.status.width, 120, "状态栏应全宽");
        // 顶栏位于内容区上方、主区在其下（垂直堆叠对齐）
        assert_eq!(lay.main.y, lay.topbar.y + 1, "主区应在顶栏正下方");
    }

    /// J5 响应式折叠：90 宽折右栏、70 宽折双栏，渲染均不崩且核心区完整。
    #[test]
    fn test_j5_collapse_90_and_70_render_no_crash() {
        // 90 宽：折右栏（rail=None），侧边栏仍在
        {
            let mut app = WorkbenchApp::from_config(&Config::default()).unwrap();
            app.state.on_resize(90); // 模拟真实事件循环收到 Resize 事件
            let mut term = Terminal::new(TestBackend::new(90, 40)).unwrap();
            term.draw(|f| app.render(f)).unwrap();
            let d = j5_collect(&term);
            let norm = d.replace(' ', "");
            for g in ["F", "S", "K", "C", "O"] {
                assert!(norm.contains(g), "90 宽活动栏应含 {g}");
            }
            assert!(
                d.contains("Chat") && d.contains("Code") && d.contains("Terminal"),
                "90 宽主区 TabBar 应全"
            );
            assert!(norm.contains("侧边栏"), "90 宽侧边栏仍应渲染");
            assert!(!d.contains("Dp"), "90 宽右栏已折叠，不应渲染 Deploy tab");
        }
        // 70 宽：双栏折叠（sidebar+rail=None）
        {
            let mut app = WorkbenchApp::from_config(&Config::default()).unwrap();
            app.state.on_resize(70); // 模拟真实事件循环收到 Resize 事件
            let mut term = Terminal::new(TestBackend::new(70, 40)).unwrap();
            term.draw(|f| app.render(f)).unwrap();
            let d = j5_collect(&term);
            let norm = d.replace(' ', "");
            for g in ["F", "S", "K", "C", "O"] {
                assert!(norm.contains(g), "70 宽活动栏应含 {g}");
            }
            assert!(
                d.contains("Chat") && d.contains("Code") && d.contains("Terminal"),
                "70 宽主区 TabBar 应全"
            );
            assert!(!d.contains("侧边栏"), "70 宽侧边栏已折叠");
            assert!(!d.contains("Dp"), "70 宽右栏已折叠");
        }
        // 折叠态由 layout 约束保证（防御性再断言）
        assert!(
            !compute_collapse(90).sidebar && compute_collapse(90).rail,
            "90 宽应仅折右栏"
        );
        assert!(
            compute_collapse(70).sidebar && compute_collapse(70).rail,
            "70 宽应折双栏"
        );
    }
}
