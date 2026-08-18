//! 全局视图模型 + 单向归约核心（渲染只读）
//!
//! 设计：所有外部刺激（按键 / 事件总线 / 定时器 / 审批）统一为 [`UiEvent`]，
//! 经 [`WorkbenchState::reduce`] 产生视图变更（直接改 state）与副作用（[`UiEffect`] 交
//! `WorkbenchApp::apply_effects` 执行）。`WorkbenchState` 是不可变快照，渲染只读它。

use std::collections::VecDeque;
use std::path::PathBuf;

use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::text::Line;
use serde_json::Value;

use crate::events::{Envelope, EventType};
use crate::guard::gateway::ApprovalOutcome;
use crate::mode::ModeKind;
use crate::model::registry::ModelProfile;
use crate::tui::workbench::history::CommandHistory;
use crate::tui::workbench::input::Composer;
use crate::tui::workbench::layout::{compute_collapse, CollapseState, WorkbenchRegion};
use crate::tui::workbench::slot::SlotId;

/// 占位默认会话预算（token）。
///
/// 当前系统未在任何配置中定义「会话级 Token 预算」，故状态栏分母先用占位值。
/// 正式实现应从模型上下文窗口 / 用户设置读取（见二次开发方案 F5-b）。
const DEFAULT_TOKEN_BUDGET: u32 = 200_000;

/// 交互状态机（B4）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionState {
    /// 空闲
    Idle,
    /// 引擎处理中
    Processing,
    /// 等待审批
    AwaitingApproval,
    /// 完成
    Done,
    /// 自愈修复中
    Repairing,
    /// 中断中
    Interrupting,
}

/// 主区 Tab
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainTab {
    /// 聊天
    Chat,
    /// 代码
    Code,
    /// 终端
    Terminal,
}

/// 右栏 Tab
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailTab {
    /// 智能体状态
    Agent,
    /// 工具日志
    Tools,
    /// 上下文
    Context,
    /// 记忆
    Memory,
    /// 任务
    Tasks,
    /// 一键部署面板（Phase I4，FR-16）
    Deploy,
    /// 协作事件流（Phase I4，FR-15）
    Collab,
}

/// 活动栏种类（图标标签 F/S/K/C/O）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    /// 文件
    Files,
    /// 搜索
    Search,
    /// 技能
    Skills,
    /// 会话
    Sessions,
    /// 设置
    Settings,
}

/// 连接状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// 已就绪
    Ready,
    /// 已连接
    Connected,
    /// 已断开
    Disconnected,
}

/// Token 用量
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsage {
    /// 已用
    pub used: u32,
    /// 预算
    pub budget: u32,
}

// ---- 视图模型子结构 ----

/// 聊天角色
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    /// 用户
    User,
    /// 助手
    Assistant,
    /// 系统
    System,
}

/// 聊天消息
#[derive(Debug, Clone)]
pub struct ChatMessage {
    /// 角色
    pub role: ChatRole,
    /// 内容
    pub content: String,
}

/// 聊天模型（含流式游标）
#[derive(Debug, Clone, Default)]
pub struct ChatModel {
    /// 消息流
    pub messages: Vec<ChatMessage>,
    /// 流式输出缓冲（Thinking 累积，Done 落盘为 Assistant 消息）
    pub streaming: Option<String>,
}

/// 文件引用（@file 解析结果）
#[derive(Debug, Clone)]
pub struct FileRef {
    /// 路径
    pub path: PathBuf,
    /// 摘录（可选）
    pub excerpt: Option<String>,
}

/// 输入缓冲状态
#[derive(Debug, Clone)]
pub struct ComposerState {
    /// 文本缓冲
    pub buffer: String,
    /// 光标位置
    pub cursor: usize,
    /// 引用列表
    pub references: Vec<FileRef>,
    /// 待粘贴文件（落盘后）
    pub pending_paste: Option<PathBuf>,
    /// 命令历史（跨会话持久化 + 上/下召回 + Ctrl+R 搜索，Phase 4 / AC-08）
    pub history: CommandHistory,
}

impl ComposerState {
    /// 创建空输入状态（历史默认不落盘，由 WorkbenchApp::new 调 load 启用）
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
            references: Vec::new(),
            pending_paste: None,
            history: CommandHistory::new(),
        }
    }
}

impl Default for ComposerState {
    fn default() -> Self {
        Self::new()
    }
}

/// 内联审批视图
#[derive(Debug, Clone)]
pub struct PendingApprovalView {
    /// 审批 ID
    pub id: String,
    /// 目标工具
    pub tool: String,
    /// 参数
    pub args: String,
    /// 风险等级
    pub risk: String,
}

/// 问题项（底部问题面板）
#[derive(Debug, Clone)]
pub struct ProblemItem {
    /// 严重度
    pub severity: String,
    /// 消息
    pub message: String,
}

/// 上下文快照
#[derive(Debug, Clone, Copy, Default)]
pub struct ContextSnapshot {
    /// 已用 token
    pub used: u32,
    /// 预算
    pub budget: u32,
    /// 缓存命中率 0..1
    pub cache_hit: f32,
    /// 是否压缩
    pub compressed: bool,
}

/// 任务项（右栏任务队列）
#[derive(Debug, Clone)]
pub struct TaskItem {
    /// 任务 ID
    pub id: String,
    /// 标签
    pub label: String,
    /// 状态
    pub status: String,
}

/// 工具调用日志条目（右栏 Tools）
#[derive(Debug, Clone)]
pub struct ToolCallEntry {
    /// 工具名
    pub tool: String,
    /// 参数键
    pub args: Vec<String>,
    /// 结果（None=进行中）
    pub ok: Option<bool>,
    /// 摘要
    pub summary: String,
}

/// 记忆引用条目（右栏 Memory）
#[derive(Debug, Clone)]
pub struct MemoryRefEntry {
    /// 键
    pub key: String,
    /// 置信度
    pub confidence: f32,
}

/// 部署种类
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployKind {
    /// SSH 推送（I2）
    Ssh,
    /// 宝塔面板（I3）
    Baota,
}

/// 单次部署运行视图（部署面板历史）
#[derive(Debug, Clone)]
pub struct DeployRunView {
    /// 目标（ssh://host 或 baota://alias）
    pub target: String,
    /// 部署种类
    pub kind: DeployKind,
    /// 状态文本（planned (dry-run) / success / rolled_back / error:...）
    pub status: String,
    /// 明细（远端将执行步骤 / 错误信息）
    pub detail: String,
    /// 二进制 sha256（dry-run 也为真实计算）
    pub sha256: Option<String>,
}

/// 部署面板视图（Phase I4，FR-16）
#[derive(Debug, Clone, Default)]
pub struct DeployPanelView {
    /// 当前聚焦目标
    pub current_target: Option<String>,
    /// 部署运行历史（上限 20）
    pub runs: std::collections::VecDeque<DeployRunView>,
}

/// 协作视图（Phase I4，FR-15）：在线成员 + 事件流日志
#[derive(Debug, Clone, Default)]
pub struct CollabView {
    /// 在线成员 ID
    pub members: Vec<String>,
    /// 事件流日志（最近在上，上限 200）
    pub log: std::collections::VecDeque<String>,
}

/// 全局视图模型（不可变快照，渲染只读）
#[derive(Debug, Clone)]
pub struct WorkbenchState {
    /// 会话标识
    pub session: String,
    /// 当前聚焦 region
    pub focus: WorkbenchRegion,
    /// 当前活动栏种类
    pub active_activity: ActivityKind,
    /// 当前侧边面板
    pub active_sidebar: SlotId,
    /// 主区 Tab
    pub main_tab: MainTab,
    /// 右栏 Tab
    pub rail_tab: RailTab,
    /// 模式（来自 ModeManager::current 的缓存）
    pub mode: ModeKind,
    /// 模型名（主模型展示标签）
    pub model: String,
    /// Bloom 副模型名（空 = 未启用双模型）
    pub copilot_model: String,
    /// Token 用量
    pub token_usage: TokenUsage,
    /// 会话计时文本
    pub session_time: String,
    /// 连接状态
    pub connection: ConnectionStatus,
    /// 交互状态
    pub interaction: InteractionState,
    /// 最近事件（右栏消费，上限 500）
    pub events: VecDeque<Envelope>,
    /// 聊天模型
    pub chat: ChatModel,
    /// 输入缓冲
    pub composer: ComposerState,
    /// 内联审批框
    pub approval: Option<PendingApprovalView>,
    /// 终端面板输出
    pub terminal_lines: Vec<Line<'static>>,
    /// 问题面板
    pub problems: Vec<ProblemItem>,
    /// 上下文快照
    pub context_snapshot: Option<ContextSnapshot>,
    /// 任务队列
    pub tasks: Vec<TaskItem>,
    /// 折叠状态（响应式 + 手动）
    pub collapse: CollapseState,
    /// 手动侧边栏折叠覆盖
    pub manual_sidebar: Option<bool>,
    /// 手动右栏折叠覆盖
    pub manual_rail: Option<bool>,
    /// 工具日志
    pub tools: Vec<ToolCallEntry>,
    /// 记忆引用
    pub memory_refs: Vec<MemoryRefEntry>,
    /// 进度百分比
    pub progress: Option<u8>,
    /// 顶栏瞬时通知（模型切换 / 添加 / 删除反馈，覆盖式）
    pub notice: Option<String>,
    /// 帮助浮层是否打开
    pub help_open: bool,
    /// 命令面板是否打开
    pub palette_open: bool,
    /// 部署面板视图（Phase I4，FR-16）
    pub deploy_panel: DeployPanelView,
    /// 协作事件流视图（Phase I4，FR-15）
    pub collab: CollabView,
}

impl WorkbenchState {
    /// 创建初始状态
    pub fn new(session: String, model: String) -> Self {
        let collapse = compute_collapse(120);
        Self {
            session,
            focus: WorkbenchRegion::Activity,
            active_activity: ActivityKind::Files,
            active_sidebar: SlotId::LeftSidebarFileTree,
            main_tab: MainTab::Chat,
            rail_tab: RailTab::Agent,
            mode: ModeKind::Agent,
            model,
            copilot_model: String::new(),
            token_usage: TokenUsage::default(),
            session_time: "00:00".into(),
            connection: ConnectionStatus::Ready,
            interaction: InteractionState::Idle,
            events: VecDeque::new(),
            chat: ChatModel::default(),
            composer: ComposerState::new(),
            approval: None,
            terminal_lines: Vec::new(),
            problems: Vec::new(),
            context_snapshot: None,
            tasks: Vec::new(),
            collapse,
            manual_sidebar: None,
            manual_rail: None,
            tools: Vec::new(),
            memory_refs: Vec::new(),
            progress: None,
            notice: None,
            help_open: false,
            palette_open: false,
            deploy_panel: DeployPanelView::default(),
            collab: CollabView::default(),
        }
    }

    /// 切换侧边栏折叠（手动优先）
    pub fn toggle_sidebar(&mut self) {
        let cur = self.manual_sidebar.unwrap_or(self.collapse.sidebar);
        self.manual_sidebar = Some(!cur);
        self.collapse.sidebar = !cur;
    }

    /// 切换右栏折叠（手动优先）
    pub fn toggle_rail(&mut self) {
        let cur = self.manual_rail.unwrap_or(self.collapse.rail);
        self.manual_rail = Some(!cur);
        self.collapse.rail = !cur;
    }

    /// 响应式：按宽度重算折叠（保留手动覆盖）
    pub fn on_resize(&mut self, width: u16) {
        let r = compute_collapse(width);
        self.collapse.sidebar = self.manual_sidebar.unwrap_or(r.sidebar);
        self.collapse.rail = self.manual_rail.unwrap_or(r.rail);
    }

    /// 单向归约：返回副作用列表
    pub fn reduce(&mut self, ev: UiEvent) -> Vec<UiEffect> {
        match ev {
            UiEvent::Key(k) => self.reduce_key(k),
            UiEvent::Mouse(_) => vec![UiEffect::Render],
            UiEvent::Tick => {
                self.refresh_token_usage();
                vec![]
            }
            UiEvent::Resize(w, _h) => {
                self.on_resize(w);
                vec![UiEffect::Render]
            }
            UiEvent::Bus(env) => {
                self.ingest(env);
                vec![UiEffect::Render]
            }
        }
    }

    /// 实时刷新 Token 用量（修复 F5：此前 TUI 面板恒显 0/0，用量台账从未接通）。
    ///
    /// - `usage::init()` 幂等打开进程级台账；此前仅网关会调用它，纯 TUI 模式下面板无数据源。
    /// - 读取**全局**聚合（记录全部落在 `session=""`，因 `usage::SESSION` 从未被 set），
    ///   故不按 sessionKey 过滤，直接取 `total_tokens` 真值填入 `used`。
    /// - 主循环每轮 `terminal.draw` 都会重绘，因此 Tick 内更新即可实时可见。
    pub fn refresh_token_usage(&mut self) {
        crate::usage::init();
        let status = crate::usage::status(&Value::Null);
        let used = status
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        self.token_usage.used = (used.min(u32::MAX as u64)) as u32;
        // 预算分母：系统暂无配置，用占位默认；非零即视为已设置，保持稳定。
        if self.token_usage.budget == 0 {
            self.token_usage.budget = DEFAULT_TOKEN_BUDGET;
        }
    }

    fn reduce_key(&mut self, k: KeyEvent) -> Vec<UiEffect> {
        use crossterm::event::{KeyCode, KeyModifiers};

        // 内联审批框：y/n 裁决
        if self.interaction == InteractionState::AwaitingApproval {
            if let KeyCode::Char(c) = k.code {
                let id = self
                    .approval
                    .as_ref()
                    .map(|a| a.id.clone())
                    .unwrap_or_default();
                if c == 'y' || c == 'Y' {
                    return vec![UiEffect::ResolveApproval {
                        id,
                        outcome: ApprovalOutcome::Approved,
                    }];
                }
                if c == 'n' || c == 'N' {
                    return vec![UiEffect::ResolveApproval {
                        id,
                        outcome: ApprovalOutcome::Denied,
                    }];
                }
            }
        }

        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);

        if ctrl {
            match k.code {
                KeyCode::Char('b') if shift => {
                    self.toggle_rail();
                    return vec![UiEffect::ToggleRail];
                }
                KeyCode::Char('b') => {
                    self.toggle_sidebar();
                    return vec![UiEffect::ToggleSidebar];
                }
                KeyCode::Char('1') => {
                    self.main_tab = MainTab::Chat;
                    return vec![UiEffect::SwitchMainTab(MainTab::Chat)];
                }
                KeyCode::Char('2') => {
                    self.main_tab = MainTab::Code;
                    return vec![UiEffect::SwitchMainTab(MainTab::Code)];
                }
                KeyCode::Char('3') => {
                    self.main_tab = MainTab::Terminal;
                    return vec![UiEffect::SwitchMainTab(MainTab::Terminal)];
                }
                KeyCode::Char('u') => {
                    self.rail_tab = RailTab::Agent;
                    return vec![UiEffect::SwitchRailTab(RailTab::Agent)];
                }
                KeyCode::Char('5') => {
                    self.rail_tab = RailTab::Deploy;
                    return vec![UiEffect::SwitchRailTab(RailTab::Deploy)];
                }
                KeyCode::Char('6') => {
                    self.rail_tab = RailTab::Collab;
                    return vec![UiEffect::SwitchRailTab(RailTab::Collab)];
                }
                KeyCode::Char('n') => return vec![UiEffect::NewSession],
                KeyCode::Char('l') => {
                    self.active_activity = ActivityKind::Sessions;
                    self.active_sidebar = SlotId::LeftSidebarSessions;
                    return vec![UiEffect::SwitchActivity(ActivityKind::Sessions)];
                }
                KeyCode::Char('c') => return vec![UiEffect::Quit],
                KeyCode::Char('r') => {
                    // Ctrl+R：进入反向搜索；搜索中再次按则跳到下一条匹配
                    if self.composer.history.is_searching() {
                        self.composer.history.search_next();
                    } else {
                        self.composer.history.start_search();
                    }
                    return vec![UiEffect::Render];
                }
                _ => {}
            }
        }

        if k.modifiers.contains(KeyModifiers::ALT) && k.code == KeyCode::Char('v') {
            return vec![UiEffect::Render]; // 全屏查看（占位）
        }

        if k.code == KeyCode::Esc {
            if self.help_open {
                self.help_open = false;
            }
            if self.palette_open {
                self.palette_open = false;
            }
            if self.composer.history.is_searching() {
                self.composer.history.exit_search(false);
            }
            return vec![UiEffect::Render];
        }

        // 反向搜索态：按键路由到搜索状态机（Phase 4 / AC-08）
        if self.composer.history.is_searching() {
            match k.code {
                KeyCode::Char(c) => self.composer.history.search_type(c),
                KeyCode::Backspace => self.composer.history.search_backspace(),
                KeyCode::Enter => {
                    if let Some(t) = self.composer.history.exit_search(true) {
                        self.composer.buffer = t;
                    }
                    return vec![UiEffect::Render];
                }
                KeyCode::Up => self.composer.history.search_prev(),
                KeyCode::Down => self.composer.history.search_next(),
                _ => {}
            }
            return vec![UiEffect::Render];
        }

        // 上/下召回历史（非搜索态）
        match k.code {
            KeyCode::Up => {
                if let Some(t) = self.composer.history.recall_up(&self.composer.buffer) {
                    self.composer.buffer = t;
                }
                return vec![UiEffect::Render];
            }
            KeyCode::Down => {
                if let Some(t) = self.composer.history.recall_down() {
                    self.composer.buffer = t;
                }
                return vec![UiEffect::Render];
            }
            _ => {}
        }

        // 普通文本输入 → composer 缓冲
        match k.code {
            KeyCode::Char(c) => {
                self.composer.buffer.push(c);
            }
            KeyCode::Backspace => {
                self.composer.buffer.pop();
            }
            KeyCode::Enter => {
                return Composer::submit(self);
            }
            _ => {}
        }
        vec![UiEffect::Render]
    }

    /// 消费事件总线回流的信封，更新视图模型
    fn ingest(&mut self, env: Envelope) {
        // 事件队列（上限 500）
        self.events.push_back(env.clone());
        while self.events.len() > 500 {
            self.events.pop_front();
        }

        match env.r#type {
            EventType::Thinking => {
                let text = str_of(&env.payload, "text").to_string();
                self.chat
                    .streaming
                    .get_or_insert_with(String::new)
                    .push_str(&text);
            }
            EventType::ToolCall => {
                let tool = str_of(&env.payload, "tool").to_string();
                let args = arr_of(&env.payload, "args");
                self.tools.push(ToolCallEntry {
                    tool,
                    args,
                    ok: None,
                    summary: String::new(),
                });
            }
            EventType::ToolResult => {
                let is_terminal = str_of(&env.payload, "kind") == "terminal";
                let ok = env.payload.get("ok").and_then(|v| v.as_bool());
                let summary = str_of(&env.payload, "summary").to_string();
                if is_terminal {
                    for line in summary.lines() {
                        self.terminal_lines.push(Line::from(line.to_string()));
                    }
                } else if let Some(last) = self.tools.last_mut() {
                    last.ok = ok;
                    last.summary = summary;
                } else {
                    self.tools.push(ToolCallEntry {
                        tool: "shell".into(),
                        args: vec![],
                        ok,
                        summary,
                    });
                }
            }
            EventType::Plan => {
                let summary = str_of(&env.payload, "summary");
                let summary = if summary.is_empty() {
                    "计划已生成"
                } else {
                    summary
                };
                self.chat.messages.push(ChatMessage {
                    role: ChatRole::System,
                    content: summary.to_string(),
                });
            }
            EventType::Approval => {
                let id = str_of(&env.payload, "id").to_string();
                let tool = str_of(&env.payload, "tool").to_string();
                let args = str_of(&env.payload, "args").to_string();
                let risk = str_of(&env.payload, "risk").to_string();
                self.approval = Some(PendingApprovalView {
                    id,
                    tool,
                    args,
                    risk,
                });
                self.interaction = InteractionState::AwaitingApproval;
            }
            EventType::Progress => {
                let p = env
                    .payload
                    .get("percent")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u8;
                self.progress = Some(p);
            }
            EventType::MemoryRef => {
                let key = str_of(&env.payload, "key").to_string();
                let conf = env
                    .payload
                    .get("confidence")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as f32;
                self.memory_refs.push(MemoryRefEntry {
                    key,
                    confidence: conf,
                });
            }
            EventType::Error => {
                let msg = str_of(&env.payload, "message").to_string();
                self.problems.push(ProblemItem {
                    severity: "error".into(),
                    message: msg,
                });
                self.interaction = InteractionState::Idle;
            }
            EventType::Done => {
                if let Some(s) = self.chat.streaming.take() {
                    if !s.is_empty() {
                        self.chat.messages.push(ChatMessage {
                            role: ChatRole::Assistant,
                            content: s,
                        });
                    }
                }
                self.progress = None;
                self.interaction = InteractionState::Idle;
            }
            EventType::Status => {
                if str_of(&env.payload, "kind") == "mode.changed" {
                    if let Some(to) = env.payload.get("to").and_then(|v| v.as_str()) {
                        if let Some(mk) = ModeKind::from_name(to) {
                            self.mode = mk;
                        }
                    }
                } else if let Some(conn) = env.payload.get("connection").and_then(|v| v.as_str()) {
                    self.connection = match conn {
                        "connected" => ConnectionStatus::Connected,
                        "disconnected" => ConnectionStatus::Disconnected,
                        _ => ConnectionStatus::Ready,
                    };
                }
            }
            // AI 融合路由决策：工作台侧当前仅记录事件（Web UI 经 router_decision 帧渲染路由条）
            EventType::RouterDecision => {}
            // 上下文压缩生命周期：工作台侧仅作状态回执（压缩完成提示）
            EventType::Compaction => {
                if let Some(status) = env.payload.get("status").and_then(|v| v.as_str()) {
                    if status == "completed" {
                        self.progress = None;
                    }
                }
            }
            // 计划修订/运行快照：工作台侧仅记录计划摘要（Web UI 经 plan_revision/plan_run 帧渲染 Ribbon）
            EventType::PlanRevision | EventType::PlanRun => {
                let title = str_of(&env.payload, "title");
                if !title.is_empty() {
                    self.chat.messages.push(ChatMessage {
                        role: ChatRole::System,
                        content: format!("[计划] {title}"),
                    });
                }
            }
            // Bloom 并行编排进度（Captain+Copilot 双模型）：工作台侧仅作状态回执，
            // 详细进度由 Web UI 的 RouterFxStrip / ensemble 徽章渲染。
            EventType::EnsembleProgress => {
                if let Some(status) = env.payload.get("status").and_then(|v| v.as_str()) {
                    if status == "completed" {
                        self.progress = None;
                    }
                }
            }
            EventType::TextDelta | EventType::StepStart | EventType::StepFinish => {
                // TUI 侧暂不渲染流式增量/步骤边界（Web UI 卡片化展示）
            }
            // 网关专属事件：用量实时刷新（session.event.usage）与会话运行态变更
            // （sessions.changed）。TUI 不经网关总线，忽略即可，仅保证穷尽匹配可编译。
            EventType::Usage | EventType::SessionsChanged => {}
            // 产物登记/更新（Epic A R2）：TUI 不经网关总线，忽略。
            EventType::ArtifactUpdated => {}
            // 桌面动作完成（Epic C）：TUI 忽略，仅 Web UI 渲染审计回执。
            EventType::DesktopActionDone => {}
        }
    }
}

/// 从 JSON 取值为字符串（缺失返回空）
fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("")
}

/// 从 JSON 数组取字符串列表
fn arr_of(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// 归约输入（统一所有外部刺激）
#[derive(Debug, Clone)]
pub enum UiEvent {
    /// 按键
    Key(KeyEvent),
    /// 鼠标
    Mouse(MouseEvent),
    /// 定时器心跳
    Tick,
    /// 终端尺寸变化
    Resize(u16, u16),
    /// 事件总线回流
    Bus(Envelope),
}

/// 归约输出：视图变更（已在 state 生效）+ 副作用（交 apply_effects）
#[derive(Debug, Clone)]
pub enum UiEffect {
    /// 重绘
    Render,
    /// 提交提示（spawn 引擎轮次）
    SubmitPrompt(String),
    /// 经沙箱执行（!cmd）
    RunSandbox(Vec<String>),
    /// 切换模式
    SwitchMode(ModeKind),
    /// 新会话
    NewSession,
    /// 恢复会话
    RestoreSession,
    /// 请求审批
    RequestApproval(PendingApprovalView),
    /// 裁定审批
    ResolveApproval {
        /// 审批 ID
        id: String,
        /// 裁定结果
        outcome: ApprovalOutcome,
    },
    /// 切换主区 Tab
    SwitchMainTab(MainTab),
    /// 切换右栏 Tab
    SwitchRailTab(RailTab),
    /// 切换活动栏
    SwitchActivity(ActivityKind),
    /// 折叠侧边栏
    ToggleSidebar,
    /// 折叠右栏
    ToggleRail,
    /// 显示帮助
    ShowHelp,
    /// 打开命令面板
    OpenCommandPalette,
    /// 粘贴文件引用
    PasteFile(PathBuf),
    /// 模型操作（H5：TUI 接入 ModelSelector，与 GUI 顶栏同构）
    ModelAction(ModelOp),
    /// 部署操作（Phase I4，FR-16：TUI 部署面板触发一次 dry-run 演练）
    DeployAction(DeployOp),
    /// 协作操作（Phase I4，FR-15：加入/离开/广播/回放协作会话）
    CollabAction(CollabOp),
    /// 退出
    Quit,
}

/// 部署操作（交 `WorkbenchApp::apply_effects` 执行 dry-run 演练 + 写入部署面板）
#[derive(Debug, Clone)]
pub enum DeployOp {
    /// 对一个目标执行部署演练（dry-run，不实际连接）
    ///
    /// `target` 为 `ssh://[user@]host[:port][/dir]` 或 `baota://<别名>`；
    /// `service` 覆盖服务名；`no_ssl` 对宝塔禁用 SSL。
    Run {
        /// 目标地址
        target: String,
        /// 服务名覆盖（None = marguerite）
        service: Option<String>,
        /// 宝塔：禁用 SSL
        no_ssl: bool,
    },
}

/// 协作操作（交 `WorkbenchApp::apply_effects` 驱动共享 `CollabSession`）
#[derive(Debug, Clone)]
pub enum CollabOp {
    /// 以指定成员名加入协作会话（幂等）
    Join(String),
    /// 本地成员离开
    Leave,
    /// 广播一条协作消息（经事件总线回流，其他成员可见）
    Send(String),
    /// 拉取并回放会话事件流到本地日志
    Replay,
}

/// 模型操作（交 `WorkbenchApp::apply_effects` 执行 ModelSelector 持久化 + 状态回写）
///
/// 与 GUI 的 `model_use` / `model_copilot` / `model_add` / `model_remove` 语义一致，
/// 共用同一份 `~/.marguerite/models.json`，保证三形态选择器行为同构。
#[derive(Debug, Clone)]
pub enum ModelOp {
    /// 切换主模型
    UseActive(String),
    /// 设置 / 关闭 Bloom 副模型（None = 关闭双模型）
    UseCopilot(Option<String>),
    /// 新增档案
    Add(ModelProfile),
    /// 删除档案
    Remove(String),
    /// 列出全部档案（回显到聊天系统消息）
    List,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Envelope;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use serde_json::json;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn test_reduce_typing_appends_to_composer() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.reduce(UiEvent::Key(key(KeyCode::Char('h'), KeyModifiers::NONE)));
        s.reduce(UiEvent::Key(key(KeyCode::Char('i'), KeyModifiers::NONE)));
        assert_eq!(s.composer.buffer, "hi");
    }

    #[test]
    fn test_reduce_backspace() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.reduce(UiEvent::Key(key(KeyCode::Char('a'), KeyModifiers::NONE)));
        s.reduce(UiEvent::Key(key(KeyCode::Char('b'), KeyModifiers::NONE)));
        s.reduce(UiEvent::Key(key(KeyCode::Backspace, KeyModifiers::NONE)));
        assert_eq!(s.composer.buffer, "a");
    }

    #[test]
    fn test_reduce_ctrl_b_toggles_sidebar() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let before = s.collapse.sidebar;
        s.reduce(UiEvent::Key(key(KeyCode::Char('b'), KeyModifiers::CONTROL)));
        assert_ne!(s.collapse.sidebar, before);
    }

    #[test]
    fn test_reduce_ctrl_shift_b_toggles_rail() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let before = s.collapse.rail;
        s.reduce(UiEvent::Key(key(
            KeyCode::Char('b'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        )));
        assert_ne!(s.collapse.rail, before);
    }

    #[test]
    fn test_reduce_ctrl_1_switches_main_tab() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.reduce(UiEvent::Key(key(KeyCode::Char('2'), KeyModifiers::CONTROL)));
        assert_eq!(s.main_tab, MainTab::Code);
        s.reduce(UiEvent::Key(key(KeyCode::Char('3'), KeyModifiers::CONTROL)));
        assert_eq!(s.main_tab, MainTab::Terminal);
    }

    #[test]
    fn test_reduce_on_resize_folds_narrow() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.reduce(UiEvent::Resize(70, 40));
        assert!(s.collapse.sidebar && s.collapse.rail);
        s.reduce(UiEvent::Resize(120, 40));
        assert!(!s.collapse.sidebar && !s.collapse.rail);
    }

    #[test]
    fn test_reduce_manual_override_persists_resize() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.reduce(UiEvent::Key(key(KeyCode::Char('b'), KeyModifiers::CONTROL))); // 折叠侧边栏
        assert!(s.collapse.sidebar);
        s.reduce(UiEvent::Resize(120, 40)); // 宽屏默认展开，但手动覆盖保留
        assert!(s.collapse.sidebar, "手动折叠应覆盖响应式展开");
    }

    #[test]
    fn test_ingest_thinking_then_done_flushes_message() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.ingest(Envelope::new(
            EventType::Thinking,
            json!({"text":"你好"}),
            "s1",
        ));
        assert!(s.chat.streaming.is_some());
        s.ingest(Envelope::new(EventType::Done, json!({}), "s1"));
        assert!(s.chat.streaming.is_none());
        assert_eq!(s.chat.messages.len(), 1);
        assert_eq!(s.chat.messages[0].content, "你好");
        assert_eq!(s.interaction, InteractionState::Idle);
    }

    #[test]
    fn test_ingest_tool_call_and_result() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.ingest(Envelope::new(
            EventType::ToolCall,
            json!({"tool":"edit_file","args":["a.rs"]}),
            "s1",
        ));
        assert_eq!(s.tools.len(), 1);
        assert_eq!(s.tools[0].ok, None);
        s.ingest(Envelope::new(
            EventType::ToolResult,
            json!({"tool":"edit_file","ok":true,"summary":"ok"}),
            "s1",
        ));
        assert_eq!(s.tools[0].ok, Some(true));
        assert_eq!(s.tools[0].summary, "ok");
    }

    #[test]
    fn test_ingest_terminal_output_to_lines() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.ingest(Envelope::new(
            EventType::ToolResult,
            json!({"kind":"terminal","tool":"shell","ok":true,"summary":"line1\nline2"}),
            "s1",
        ));
        assert_eq!(s.terminal_lines.len(), 2);
    }

    #[test]
    fn test_ingest_approval_sets_awaiting() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.ingest(Envelope::new(
            EventType::Approval,
            json!({"id":"ap1","tool":"edit_file","args":"{}","risk":"medium"}),
            "s1",
        ));
        assert_eq!(s.interaction, InteractionState::AwaitingApproval);
        assert_eq!(s.approval.as_ref().unwrap().id, "ap1");
    }

    #[test]
    fn test_reduce_y_n_resolves_approval() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.ingest(Envelope::new(
            EventType::Approval,
            json!({"id":"ap1","tool":"edit_file","args":"{}","risk":"medium"}),
            "s1",
        ));
        let eff = s.reduce(UiEvent::Key(key(KeyCode::Char('y'), KeyModifiers::NONE)));
        assert!(matches!(
            eff.as_slice(),
            [UiEffect::ResolveApproval {
                outcome: ApprovalOutcome::Approved,
                ..
            }]
        ));
    }

    #[test]
    fn test_ingest_mode_changed_updates_state() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.ingest(Envelope::mode_changed("s1", "agent", "bloom"));
        assert_eq!(s.mode, ModeKind::Bloom);
    }

    #[test]
    fn test_events_queue_capped_at_500() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        for i in 0..600u32 {
            s.ingest(Envelope::new(EventType::Status, json!({"n":i}), "s1"));
        }
        assert_eq!(s.events.len(), 500);
    }
}
