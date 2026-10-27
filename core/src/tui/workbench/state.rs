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
    /// 模型档案面板（与 CLI / GUI 共用 models.json）
    Models,
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

/// 三档模型分层 + 会话预算的只读快照（供状态栏展示，来源 `SessionState`）。
///
/// 为什么单独抽出来：状态栏是纯渲染层，不应在 render 里读磁盘；
/// Tick 里刷新一次快照，渲染只读字段，避免每帧 I/O 与不可重入锁。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TierLedger {
    /// 生效档位（flagship / balanced / turbo）
    pub effective: String,
    /// 用户锁定档位（空 = 自动）
    pub locked: String,
    /// 预算状态机（healthy / degraded / emergency / depleted）
    pub budget_state: String,
    /// 剩余比例（0..1）
    pub remaining_ratio: f32,
    /// 已用 token
    pub tokens_used: u64,
    /// token 上限（0 = 不按 token 熔断）
    pub tokens_cap: u64,
}

impl TierLedger {
    /// 状态栏单行文案：`balanced(自) 预算78%`。
    pub fn label(&self) -> String {
        if self.effective.is_empty() {
            return String::new();
        }
        let lock = if self.locked.is_empty() { "自" } else { "锁" };
        if self.tokens_cap == 0 {
            format!("{}({}) 预算{}%·仅记量", self.effective, lock,
                    (self.remaining_ratio * 100.0).round() as i32)
        } else {
            format!("{}({}) {}/{} tok·剩{}%", self.effective, lock,
                    self.tokens_used, self.tokens_cap,
                    (self.remaining_ratio * 100.0).round() as i32)
        }
    }
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
    /// 可见答复的流式缓冲（`TextDelta` 累积，`Done` 时落盘为 Assistant 消息）
    pub streaming: Option<String>,
    /// 模型思考过程缓冲（`Thinking` 累积）。
    ///
    /// 与 [`ChatModel::streaming`] **刻意分开**：`Thinking` 是推理链、`TextDelta` 是答复，
    /// 混进同一个缓冲就等于把思考过程当正文展示；而回合结束（`Done`）时它必须被丢弃，
    /// 否则下一轮会拿着上一轮的思考续写。
    pub thinking: Option<String>,
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

/// 模型面板单行（渲染期只读投影，零磁盘 IO）
#[derive(Debug, Clone)]
pub struct ModelItem {
    /// 档案 id（激活时按此定位，`label` 会重名）
    pub id: String,
    /// 展示名
    pub label: String,
    /// 提供商标识
    pub provider: String,
    /// 接入点（空 = 走 provider 默认）
    pub base_url: String,
    /// 是否内置种子档案
    pub builtin: bool,
    /// 是否已配置密钥（只看有没有，不接触明文）
    pub key_ready: bool,
}

/// 模型档案面板视图（右栏 Models tab）
///
/// 为什么要有这一层：`models.json` 的读取 + JSON 解析不能放进渲染函数。
/// 主循环每 200ms 重绘一帧，逐帧 `ModelSelector::load()` 等于把文件 IO 塞进
/// 绘制路径（掉帧 + 与其它进程写档案互斥）。改为「事件驱动刷新 → 状态缓存 → 渲染只读」。
#[derive(Debug, Clone, Default)]
pub struct ModelPanelView {
    /// 档案列表（按 id 排序，顺序稳定，光标才有意义）
    pub items: Vec<ModelItem>,
    /// 当前激活档案 id
    pub active_id: String,
    /// 光标位置
    pub cursor: usize,
    /// 读取错误（None = 正常；Some = 面板必须显示错误态而不是空态）
    pub error: Option<String>,
}

impl ModelPanelView {
    /// 当前光标所指档案
    pub fn selected(&self) -> Option<&ModelItem> {
        self.items.get(self.cursor)
    }

    /// 光标下移（越界钳制，不绕回：避免长按 ↓ 从末项跳回首项而误激活）
    pub fn move_down(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.cursor = (self.cursor + 1).min(self.items.len() - 1);
    }

    /// 光标上移
    pub fn move_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }
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
    /// 侧边栏待聚焦/展开的路径（`/open <path>` 设置，文件树消费）
    pub sidebar_open_path: Option<PathBuf>,
    /// 主区 Tab
    pub main_tab: MainTab,
    /// 右栏 Tab
    pub rail_tab: RailTab,
    /// 模型面板视图（右栏 Models tab，事件驱动刷新）
    pub model_panel: ModelPanelView,
    /// 模式（来自 ModeManager::current 的缓存）
    pub mode: ModeKind,
    /// 模型名（主模型展示标签）
    pub model: String,
    /// Bloom 副模型名（空 = 未启用双模型）
    pub copilot_model: String,
    /// Token 用量
    pub token_usage: TokenUsage,
    /// 档位 + 会话预算快照（状态栏展示）
    pub tier_ledger: TierLedger,
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
    /// 鲸鱼娘桌宠 PetV2 共享状态（Review Advisory-B：解决 PetWhaleSlot 硬编码 None，
    /// 让 affinity / energy / 6 帧 ASCII 精灵真正参与到工作台渲染闭环）。
    /// None 时 Slot 自动回退 PetState / idle 默认值。
    pub pet_v2: Option<crate::workbench::pet_v2::PetV2>,
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
            sidebar_open_path: None,
            main_tab: MainTab::Chat,
            rail_tab: RailTab::Agent,
            model_panel: ModelPanelView::default(),
            mode: ModeKind::Agent,
            model,
            copilot_model: String::new(),
            token_usage: TokenUsage::default(),
            tier_ledger: TierLedger::default(),
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
            // 桌宠 PetV2 默认初始化：idle、affinity=0、energy=80（与 Slot 原 fallback 一致）。
            // 后续若接入 /pet save/load 持久化，则在 state reduce 里替换为磁盘加载值。
            pet_v2: Some(crate::workbench::pet_v2::PetV2::default()),
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
                self.refresh_tier_ledger();
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

    /// 刷新档位 + 会话预算快照（三档分层 / 台账在 TUI 的展示口径）。
    ///
    /// 与 CLI `marguerite model tier` / 网关 `model.tier.get` 同源：
    /// 都读 `SessionState`，避免三端各算一套、数字对不上。
    ///
    /// 预算的**已用/剩余**额外经 `usage` 台账实测：`SessionState.budget` 的 used 字段
    /// 全仓无人写入（恒 0），只读它会永远显示「剩余 100%」。
    pub fn refresh_tier_ledger(&mut self) {
        use crate::model::tier::SessionState;
        let s = SessionState::load();
        let budget = match crate::usage::tightest_budget_snapshot() {
            Some(b) => s.budget.clone().with_measured_snapshot(
                b.tokens_cap,
                b.usd_cap,
                b.tokens_used,
                b.usd_used,
            ),
            None => s.budget.clone(),
        };
        self.tier_ledger = TierLedger {
            effective: s.effective_tier().as_str().to_string(),
            locked: s.locked_tier.map(|t| t.as_str().to_string()).unwrap_or_default(),
            budget_state: budget.state().as_str().to_string(),
            remaining_ratio: budget.remaining_ratio() as f32,
            tokens_used: budget.tokens_used,
            tokens_cap: budget.tokens_cap,
        };
    }

    /// 重新读取 `models.json` 并投影到模型面板视图。
    ///
    /// 调用时机只有三类：打开模型面板、一次模型操作结束后、启动首帧。
    /// 渲染函数只读 `self.model_panel`，不做任何文件 IO。
    pub fn refresh_model_panel(&mut self) {
        match crate::model::selector::ModelSelector::load() {
            Ok(sel) => {
                let reg = sel.registry();
                let mut items: Vec<ModelItem> = reg
                    .list()
                    .into_iter()
                    .map(|p| ModelItem {
                        id: p.id.clone(),
                        label: p.label.clone(),
                        provider: p.provider.clone(),
                        base_url: p.base_url.clone(),
                        builtin: p.builtin,
                        // 本地/回环端点免密钥，与 `byok::is_keyless_endpoint` 同判据；
                        // 这里只判断"能不能直接用"，绝不读取密钥明文。
                        key_ready: !p.api_key.trim().is_empty()
                            || !p.api_key_env.trim().is_empty()
                            || crate::model::byok::is_keyless_endpoint(&p.provider, &p.base_url),
                    })
                    .collect();
                items.sort_by(|a, b| a.id.cmp(&b.id));
                let active_id = reg.active.clone();
                let cursor = items
                    .iter()
                    .position(|i| i.id == active_id)
                    .unwrap_or_default();
                // 顶栏主/副模型标签同步到 models.json 的**当前**激活项：
                // 否则从面板激活后，顶栏还写着旧模型（"切了没反应"的典型观感）。
                self.model = items
                    .iter()
                    .find(|i| i.id == active_id)
                    .map(|i| i.label.clone())
                    .unwrap_or_default();
                self.copilot_model = reg
                    .copilot
                    .as_ref()
                    .and_then(|cid| items.iter().find(|i| &i.id == cid))
                    .map(|i| i.label.clone())
                    .unwrap_or_default();
                self.model_panel = ModelPanelView {
                    items,
                    active_id,
                    cursor,
                    error: None,
                };
            }
            Err(e) => {
                // 读取失败 = 错误态，**不是**空态：空态会让用户以为"档案被删光了"。
                self.model_panel = ModelPanelView {
                    items: Vec::new(),
                    active_id: String::new(),
                    cursor: 0,
                    error: Some(e.to_string()),
                };
            }
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
                KeyCode::Char('m') => {
                    // Ctrl+M：打开/重读模型面板（重读是错误态的显式重试出口）
                    self.rail_tab = RailTab::Models;
                    self.refresh_model_panel();
                    return vec![UiEffect::SwitchRailTab(RailTab::Models)];
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

        // 模型面板：↑↓ 移动光标，Enter 激活。只在面板可见时接管方向键，
        // 否则会把聊天页的历史召回（↑/↓）抢走。
        if self.rail_tab == RailTab::Models {
            match k.code {
                KeyCode::Up => {
                    self.model_panel.move_up();
                    return vec![UiEffect::Render];
                }
                KeyCode::Down => {
                    self.model_panel.move_down();
                    return vec![UiEffect::Render];
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    // 走 `ModelOp::Activate`（与 /model activate、桌面 models.activate 同一条
                    // 持久化链）。绝不在前端另写一份 models.json。
                    if let Some(item) = self.model_panel.selected() {
                        let id = item.id.clone();
                        if id != self.model_panel.active_id {
                            return vec![UiEffect::ModelAction(ModelOp::Activate(id))];
                        }
                    }
                    return vec![UiEffect::Render];
                }
                _ => {}
            }
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
                // 思考过程单独累积（不是答复正文）：见 [`ChatModel::thinking`]。
                let text = str_of(&env.payload, "text").to_string();
                self.chat
                    .thinking
                    .get_or_insert_with(String::new)
                    .push_str(&text);
            }
            EventType::TextDelta => {
                // 可见答复的增量（编排器把 `AgentEvent::TextDelta` 映射为
                // `{text, presentation}`）。此前 TUI 直接忽略它，于是**非流式兜底
                // 之外一个字都看不到**：真实引擎只发 Thinking + Done，
                // 而 Done 的 `summary` 又被丢弃 → 界面上永远没有 AI 答复。
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
                // 收口本轮答复：优先用流式增量累积的正文；引擎不流式时回落
                // `Done.summary`（`AgentRuntime` 与编排器的 `AgentEvent::Done { text }`
                // 都是映射到 `summary`）。两条路都为空时**不编造**空消息。
                let streamed = self.chat.streaming.take().unwrap_or_default();
                let final_text = if streamed.trim().is_empty() {
                    str_of(&env.payload, "summary").to_string()
                } else {
                    streamed
                };
                if !final_text.trim().is_empty() {
                    self.chat.messages.push(ChatMessage {
                        role: ChatRole::Assistant,
                        content: final_text,
                    });
                }
                // 思考过程属过程态：回合结束即清，绝不带入下一轮当正文续写。
                self.chat.thinking = None;
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
            EventType::StepStart | EventType::StepFinish => {
                // 步骤边界的耗时/轮次在 Web UI 用于卡片分组；TUI 无对应视图状态。
                // （此前这里连同 `TextDelta` 一起被丢弃，正文因此整轮不可见。）
            }
            // 网关专属事件：用量实时刷新（session.event.usage）与会话运行态变更
            // （sessions.changed）。TUI 不经网关总线，忽略即可，仅保证穷尽匹配可编译。
            EventType::Usage | EventType::SessionsChanged => {}
            // 产物登记/更新（Epic A R2）：TUI 不经网关总线，忽略。
            EventType::ArtifactUpdated => {}
            // 桌面动作完成（Epic C）：TUI 忽略，仅 Web UI 渲染审计回执。
            EventType::DesktopActionDone => {}
            // 记忆召回（联想记忆网络）：TUI 不经网关总线，忽略（Web UI 据此构建筑）。
            EventType::MemoryRecall => {}
            // 回合保活心跳：只证明「这一轮还活着」，不携带任何视图状态。
            // TUI 的存活判据是事件流本身（每帧刷新 last_event_at），故此处无副作用。
            EventType::RunHeartbeat => {}
            // 升级路径（M2-C4）：护栏决定「停下问人」，四场景结构化 escalation。
            // TUI 复用审批视图（AwaitingApproval），展示触发场景 + 原因，fail-closed 等待确认。
            EventType::Escalation => {
                let id = str_of(&env.payload, "id").to_string();
                let trigger = str_of(&env.payload, "trigger").to_string();
                let reason = str_of(&env.payload, "reason").to_string();
                let operation = env
                    .payload
                    .get("context")
                    .and_then(|c| c.get("operation"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                self.approval = Some(PendingApprovalView {
                    id,
                    tool: format!("escalation:{trigger}"),
                    args: if operation.is_empty() {
                        reason.clone()
                    } else {
                        format!("{operation} · {reason}")
                    },
                    risk: trigger,
                });
                self.interaction = InteractionState::AwaitingApproval;
            }
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
    Add(Box<ModelProfile>),
    /// 删除档案
    Remove(String),
    /// 列出全部档案（回显到聊天系统消息）
    List,
    /// 部署本地模型目录项为路由档案（P2·A：复用 gateway local_deploy，按需激活）
    Deploy(String, bool),
    /// 部署真实存在的 Ollama tag（P2·A：复用 gateway local_ollama_deploy，按需激活）
    OllamaDeploy(String, bool),
    /// 按档案 id 激活（P2·A：activate_profile_by_id + reconcile_routing_candidates）
    Activate(String),
    /// 查看 / 设置路由模式（P2·A：empty = get；否则为 set <mode>）
    Routing(Option<String>),
    /// 查看主机真实系统规格（P2·A：local_system）
    System,
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
    fn tier_ledger_label_shows_lock_and_budget() {
        let auto = TierLedger {
            effective: "balanced".into(),
            locked: String::new(),
            budget_state: "healthy".into(),
            remaining_ratio: 0.78,
            tokens_used: 44_000,
            tokens_cap: 200_000,
        };
        assert_eq!(auto.label(), "balanced(自) 44000/200000 tok·剩78%");

        let locked = TierLedger { locked: "flagship".into(), ..auto };
        assert_eq!(locked.label(), "balanced(锁) 44000/200000 tok·剩78%");

        let meter_only = TierLedger { tokens_cap: 0, ..locked };
        assert_eq!(meter_only.label(), "balanced(锁) 预算78%·仅记量");
        assert_eq!(TierLedger::default().label(), "");
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

    /// `TextDelta`（正文增量）在 `Done` 时落盘为 Assistant 消息。
    #[test]
    fn test_ingest_text_delta_then_done_flushes_message() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.ingest(Envelope::new(
            EventType::TextDelta,
            json!({"text":"你好","presentation":"answer"}),
            "s1",
        ));
        assert!(s.chat.streaming.is_some(), "正文增量应进入流式缓冲");
        s.ingest(Envelope::new(
            EventType::TextDelta,
            json!({"text":"，Marguerite"}),
            "s1",
        ));
        s.ingest(Envelope::new(EventType::Done, json!({}), "s1"));
        assert!(s.chat.streaming.is_none());
        assert_eq!(s.chat.messages.len(), 1);
        assert_eq!(s.chat.messages[0].content, "你好，Marguerite");
        assert_eq!(s.interaction, InteractionState::Idle);
    }

    /// 引擎不流式时，`Done.summary` 必须被展示（此前被整段丢弃 → 界面无答复）。
    #[test]
    fn test_done_summary_is_shown_when_no_stream() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.ingest(Envelope::new(
            EventType::Done,
            json!({"summary":"引擎未流式时的答复"}),
            "s1",
        ));
        assert_eq!(s.chat.messages.len(), 1, "非流式答复必须落盘");
        assert_eq!(s.chat.messages[0].content, "引擎未流式时的答复");
    }

    /// 思考过程与答复分开：思考不进正文，且回合结束即清空。
    #[test]
    fn test_thinking_is_separate_from_answer_and_cleared_on_done() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.ingest(Envelope::new(
            EventType::Thinking,
            json!({"text":"先看看用户想干什么…"}),
            "s1",
        ));
        assert!(
            s.chat.streaming.is_none(),
            "思考不得混入正交流式缓冲（否则推理链会被当作答案展示）"
        );
        assert!(s.chat.thinking.is_some());

        s.ingest(Envelope::new(EventType::TextDelta, json!({"text":"答复"}), "s1"));
        s.ingest(Envelope::new(EventType::Done, json!({}), "s1"));
        assert!(s.chat.thinking.is_none(), "回合结束必须丢弃思考过程");
        assert_eq!(s.chat.messages.len(), 1);
        assert_eq!(s.chat.messages[0].content, "答复", "落盘的只能是答复正文");
    }

    /// 空答复不得编造一条空 Assistant 消息。
    #[test]
    fn test_done_without_any_text_pushes_nothing() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.ingest(Envelope::new(EventType::Done, json!({}), "s1"));
        s.ingest(Envelope::new(EventType::Done, json!({"summary":"   "}), "s1"));
        assert!(s.chat.messages.is_empty(), "无正文时不得塞空消息");
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
