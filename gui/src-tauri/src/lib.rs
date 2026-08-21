//! Marguerite GUI 壳 — Tauri 后端
//!
//! Phase C 桌面能力：系统托盘 + 全局快捷键 + 原生菜单。
//! Phase 6 Fusion：切换为唯一 GUI（egui crate 从 workspace 移除），并接入双引擎桥
//! —— PySidecar / 能力注册表 SSOT / 子代理事件流 / 侧车健康。
//! 壳层只做薄封装，业务逻辑全在 core（PPA 架构：枢·辙·脉·件·盾）。

use std::sync::{Arc, Mutex};

use marguerite_core::domain::capabilities::{
    self, Capability, CapabilityGroup, CapabilityMaturity,
};
use marguerite_core::events::EventBus;
use marguerite_core::gateway::{self, GatewayState, DEFAULT_PORT};
use marguerite_core::guard::repair::{RepairConfig, RepairLoop, RepairResult};
use marguerite_core::guard::syntax::Language;
use marguerite_core::guard::{GuardConfig, GuardGateway, GuardReport};
use marguerite_core::memory::{ExperienceKind, ExperienceLedger, LongTermMemory, SelfEvolution};
use marguerite_core::mode::ModeKind;
use marguerite_core::model::byok;
use marguerite_core::model::dual::DualModelCoordinator;
use marguerite_core::model::registry::{ModelProfile, ProfileKind};
use marguerite_core::model::selector::ModelSelector;
use marguerite_core::persistence::config::Config;
use marguerite_core::pyengine::{PyEngineSidecar, PyEngineStatus, SidecarConfig};
use marguerite_core::tui::workbench::{EngineRunner, RealEngine};
use marguerite_core::vision::VisionCapability;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, State,
};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// 应用状态：共享的护栏网关、修复循环、对话引擎、自进化经验台账
/// + 双引擎桥：PySidecar 运行时
pub struct AppState {
    pub gateway: GuardGateway,
    pub repair: RepairLoop,
    /// 对话引擎门面（RealEngine：真实事件流，非 Mock）
    pub engine: Arc<dyn EngineRunner>,
    /// 自进化经验台账（Phase E）：沉淀 + 召回闭环
    /// 内部持有 SQLite Connection（Send 非 Sync），故以 Mutex 包裹满足 Send+Sync
    pub evolution: Arc<Mutex<SelfEvolution>>,
    /// 模型选择器（Phase H4）：多命名档案 + 主/副模型选区，落盘 `models.json`
    pub model_selector: Arc<Mutex<ModelSelector>>,
    /// Marguerite Python 侧车（Phase 3 双引擎桥）：按需启动，首次调用前为 None
    pub py_sidecar: Arc<tokio::sync::Mutex<Option<Arc<PyEngineSidecar>>>>,
    /// 共享 token（与 Python 侧车 home 目录对齐）；启动侧车前用 auth::write_python_home_token 写入
    pub py_shared_token: String,
    /// 视觉能力（modlens 结构化取证）：可注入 resolver/transport，默认离线降级
    pub vision: Arc<VisionCapability>,
    /// 扑克游戏大厅（Game Hall）：进程内房间管理器，支持多人本地对战 + AI 座位
    pub poker: Arc<Mutex<marguerite_core::poker::PokerHub>>,
    /// 知识图谱（KB 2.0 质量层 · kb.graph）：实体-关系共现/定型边，内存图
    pub kb_graph: Arc<Mutex<marguerite_core::kb::KnowledgeGraph>>,
}

impl AppState {
    pub fn new() -> Self {
        let gateway = GuardGateway::new(GuardConfig::default());
        let repair = RepairLoop::new(
            GuardGateway::new(GuardConfig::default()),
            RepairConfig::default(),
        );

        // Phase E/F：自进化经验台账，持久化到应用数据目录（非临时目录，跨会话保留）
        let data_dir = Config::data_dir();
        let _ = std::fs::create_dir_all(&data_dir);
        let mut ledger = ExperienceLedger::new(&data_dir.join("marguerite_experience.db"));
        let _ = ledger.init();
        let mut long_term = LongTermMemory::new(&data_dir.join("marguerite_ltm_gui.db"));
        let _ = long_term.init();
        let evolution = Arc::new(Mutex::new(SelfEvolution::new(ledger, long_term)));

        // Phase D/F：接入真实引擎，并注入自进化经验台账
        let engine: Arc<dyn EngineRunner> = Arc::new(
            RealEngine::from_config(Config::default()).with_evolution(Some(evolution.clone())),
        );

        // Phase H4：模型选择器（加载 models.json，首次运行落盘种子默认值）
        let selector = match ModelSelector::load() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[h4] 模型选择器加载失败，回退路径加载: {e}");
                ModelSelector::with_path(marguerite_core::model::selector::models_path())
                    .expect("[h4] ModelSelector 初始化失败")
            }
        };
        let _ = selector.save();

        // 双引擎桥共享 token：16 字节 hex，启动侧车前写入到 `data_dir/pyengine-home/gateway-token`
        use rand::RngCore;
        let mut token_bytes = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut token_bytes);
        let py_shared_token = token_bytes
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>();

        Self {
            gateway,
            repair,
            engine,
            evolution,
            model_selector: Arc::new(Mutex::new(selector)),
            py_sidecar: Arc::new(tokio::sync::Mutex::new(None)),
            py_shared_token,
            vision: Arc::new(VisionCapability::offline_default()),
            poker: Arc::new(Mutex::new(marguerite_core::poker::PokerHub::new())),
            kb_graph: Arc::new(Mutex::new(marguerite_core::kb::KnowledgeGraph::new())),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// ───────────── 能力注册表 SSOT（给前端「能力」页和欢迎向导使用）─────────────

#[derive(serde::Serialize)]
struct CapabilityView {
    id: String,
    name: String,
    group: String,
    tier: String,
    maturity: String,
    description: String,
    supersedes: Option<String>,
}

#[derive(serde::Serialize)]
struct CapabilityGroupView {
    group: String,
    items: Vec<CapabilityView>,
}

// 孤儿规则：不能在 marguerite-gui crate 内给 core::capabilities::CapabilityGroup 加 inherent impl。
// 幸运的是 core 已经提供了 `CapabilityGroup::label(&self)`，直接复用，避免变体名漂移。
fn group_label(g: CapabilityGroup) -> &'static str {
    g.label()
}

fn capability_to_view(c: &Capability) -> CapabilityView {
    CapabilityView {
        id: c.id.0.clone(),
        name: c.name.clone(),
        group: group_label(c.group).to_string(),
        tier: c.tier.label().to_string(),
        maturity: match c.maturity {
            CapabilityMaturity::Real => "Real",
            CapabilityMaturity::Partial => "Partial",
            CapabilityMaturity::Narrative => "Narrative",
        }
        .to_string(),
        description: c.description.clone(),
        supersedes: c.supersedes.clone(),
    }
}

/// 列出全量能力（按组分，和 core/domain/capabilities::by_group() 顺序一致）
#[tauri::command]
fn capabilities_groups() -> Vec<CapabilityGroupView> {
    capabilities::by_group()
        .into_iter()
        .map(|(g, items)| CapabilityGroupView {
            group: group_label(g).to_string(),
            items: items.into_iter().map(capability_to_view).collect(),
        })
        .collect()
}

/// 能力总数 / 分层 / 真货占比（用于导航角标和向导 Page 2）
#[tauri::command]
fn capabilities_stats() -> CapabilityStatsView {
    let s = capabilities::stats();
    CapabilityStatsView {
        total: s.total,
        exclusive: s.exclusive,
        enhanced: s.enhanced,
        baseline: s.baseline,
        implemented: s.implemented,
        narrative: s.narrative,
    }
}

/// 原生 harness 能力注册表（单一事实源）。
///
/// 与 Python 侧车 `/harness/capabilities` 同源，但走 Rust 原生路径，
/// 供前端 HarnessPanel 在不依赖 Python 侧车的情况下读取能力清单
/// （呼应「把能力原生吸收、全变成我们自己的」）。
#[tauri::command]
fn harness_capabilities() -> Vec<marguerite_core::harness::CapabilityInfo> {
    marguerite_core::harness::CapabilityRegistry::new()
        .all()
        .to_vec()
}

#[derive(serde::Serialize)]
struct CapabilityStatsView {
    total: usize,
    exclusive: usize,
    enhanced: usize,
    baseline: usize,
    implemented: usize,
    narrative: usize,
}

// ───────────── 双引擎桥：Marguerite 侧车状态与启停 ─────────────

#[derive(serde::Serialize)]
struct SidecarStatusView {
    status: &'static str,
    port: Option<u16>,
    python: Option<String>,
    base_url: Option<String>,
}

fn py_status_to_str(s: PyEngineStatus) -> &'static str {
    match s {
        PyEngineStatus::Stopped => "stopped",
        PyEngineStatus::Starting => "starting",
        PyEngineStatus::Running => "running",
        PyEngineStatus::Restarting => "restarting",
        PyEngineStatus::Failed => "failed",
    }
}

/// Tauri async 命令：如果参数里带引用（State<'_, T>），则返回值必须是 Result。
#[tauri::command]
async fn sidecar_status(state: State<'_, Arc<AppState>>) -> Result<SidecarStatusView, String> {
    let guard = state.py_sidecar.lock().await;
    Ok(match &*guard {
        Some(sc) => SidecarStatusView {
            status: py_status_to_str(sc.status()),
            port: Some(sc.config.port),
            python: Some(sc.config.python_exe.to_string_lossy().to_string()),
            base_url: Some(sc.base_url()),
        },
        None => SidecarStatusView {
            status: "idle",
            port: None,
            python: None,
            base_url: None,
        },
    })
}

/// 首次启动 Python 侧车（懒启动）：detect → start；如果已运行则复用。
#[tauri::command]
async fn sidecar_start(state: State<'_, Arc<AppState>>) -> Result<SidecarStatusView, String> {
    let mut guard = state.py_sidecar.lock().await;
    if guard.is_some() {
        drop(guard);
        return sidecar_status(state).await;
    }
    // 1. detect 内部已经：找 python、找 pyengine_src、算 py_home、写 token
    let cfg = SidecarConfig::detect(state.py_shared_token.clone()).map_err(|e| e.to_string())?;

    // 2. 异步启动（含首次健康探测 + 守护任务后台 spawn）
    let sc = PyEngineSidecar::start(cfg)
        .await
        .map_err(|e| e.to_string())?;
    let sc = Arc::new(sc);
    let view = SidecarStatusView {
        status: py_status_to_str(sc.status()),
        port: Some(sc.config.port),
        python: Some(sc.config.python_exe.to_string_lossy().to_string()),
        base_url: Some(sc.base_url()),
    };
    *guard = Some(sc);
    Ok(view)
}

/// 停止侧车（清理子进程 + 释放锁）
#[tauri::command]
async fn sidecar_stop(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let mut guard = state.py_sidecar.lock().await;
    if let Some(sc) = guard.take() {
        sc.stop().await;
    }
    Ok(())
}

/// 护栏检查命令
#[tauri::command]
fn guard_check(state: State<Arc<AppState>>, code: String, lang: String) -> GuardReport {
    let language = Language::from_name(&lang).unwrap_or(Language::JavaScript);
    state.gateway.check(&code, language)
}

/// 自动修复命令
#[tauri::command]
fn repair_code(state: State<Arc<AppState>>, code: String, lang: String) -> RepairResult {
    let language = Language::from_name(&lang).unwrap_or(Language::JavaScript);
    state.repair.repair(&code, language)
}

/// 双模型规划（演示用，需配置 API Key）
#[tauri::command]
async fn plan_task(task: String) -> Result<Vec<String>, String> {
    if std::env::var("MARGUERITE_API_KEY").is_err() {
        return Ok(vec![]);
    }
    let cfg = marguerite_core::model::llm::LLMConfig {
        model: "deepseek-reasoner".to_string(),
        ..Default::default()
    };
    let planner = marguerite_core::model::llm::LLMClient::new(cfg);
    let executor = marguerite_core::model::llm::LLMClient::new(Default::default());
    let coordinator = DualModelCoordinator::new(planner, executor);
    let plan = coordinator.plan(&task).await.map_err(|e| e.to_string())?;
    Ok(plan.steps.into_iter().map(|s| s.description).collect())
}

/// 切换主窗口可见性（托盘左键 / 全局快捷键共用）
#[tauri::command]
fn toggle_window(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
        } else {
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
}

/// 运行一轮对话（真实引擎；事件经总线转发到前端事件通道）
///
/// 前端通过 `listen("mgt:{session}", ...)` 订阅本次会话的全部事件
/// （thinking / tool_call / tool_result / progress / done / error / status），
/// 按 `payload` 分流渲染。会话隔离由事件名中的 session 保证。
#[tauri::command]
async fn run_turn(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    text: String,
    session: String,
    mode: String,
) -> Result<(), String> {
    let mode_kind = ModeKind::from_name(&mode).unwrap_or(ModeKind::Bloom);
    let bus = Arc::new(EventBus::default());
    // 订阅全部事件并转发到前端（事件名含 session，避免多会话串台）
    let mut rx = bus.subscribe("**");
    let app_clone = app.clone();
    let event_name = format!("mgt:{session}");
    tokio::spawn(async move {
        while let Some(env) = rx.recv().await {
            if let Ok(payload) = serde_json::to_value(&env) {
                let _ = app_clone.emit(&event_name, payload);
            }
        }
    });

    let engine = state.engine.clone();
    engine
        .run_turn(text, mode_kind, session, bus)
        .await
        .map_err(|e| e.to_string())
}

/// 显示主窗口
#[tauri::command]
fn show_window(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// 隐藏主窗口到托盘
#[tauri::command]
fn hide_window(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

/// 经验视图（序列化给前端）
#[derive(serde::Serialize)]
struct ExperienceView {
    id: i64,
    kind: String,
    title: String,
    body: String,
    tags: Vec<String>,
    validated: bool,
}

/// 记录一条经验（踩坑 / 决策 / 解法），供自进化沉淀
#[tauri::command]
fn record_experience(
    state: State<'_, Arc<AppState>>,
    kind: String,
    title: String,
    body: String,
    tags: Vec<String>,
    signature: String,
) -> Result<i64, String> {
    let mut ev = state.evolution.lock().map_err(|e| e.to_string())?;
    let k = ExperienceKind::from_kind_str(&kind);
    ev.ledger
        .record(k, &title, &body, &tags, &signature)
        .map_err(|e| e.to_string())
}

/// 语义召回相关经验（自进化：执行前先读历史）
#[tauri::command]
fn search_experience(
    state: State<'_, Arc<AppState>>,
    query: String,
    limit: usize,
) -> Result<Vec<ExperienceView>, String> {
    let ev = state.evolution.lock().map_err(|e| e.to_string())?;
    let list = ev.ledger.search(&query, limit).map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .map(|e| ExperienceView {
            id: e.id,
            kind: e.kind.as_str().to_string(),
            title: e.title,
            body: e.body,
            tags: e.tags,
            validated: e.validated,
        })
        .collect())
}

/// 模型档案视图（序列化给前端）
#[derive(serde::Serialize)]
struct ModelProfileView {
    id: String,
    label: String,
    provider: String,
    model: String,
    kind: String,
    is_active: bool,
    is_copilot: bool,
}

/// 模型选择器整体状态视图
#[derive(serde::Serialize)]
struct ModelStateView {
    active: Option<String>,
    copilot: Option<String>,
    profiles: Vec<ModelProfileView>,
}

/// 列出全部模型档案（标注主/副模型）—— 供顶栏选择器渲染
#[tauri::command]
fn model_list(state: State<Arc<AppState>>) -> Result<ModelStateView, String> {
    let sel = state.model_selector.lock().map_err(|e| e.to_string())?;
    let reg = sel.registry();
    let profiles = reg
        .list()
        .iter()
        .map(|p| ModelProfileView {
            id: p.id.clone(),
            label: p.label.clone(),
            provider: p.provider.clone(),
            model: p.model.clone(),
            kind: match p.kind {
                ProfileKind::Cloud => "cloud",
                ProfileKind::Local => "local",
                ProfileKind::Custom => "custom",
            }
            .to_string(),
            is_active: reg.active == p.id,
            is_copilot: reg.copilot.as_deref() == Some(&p.id),
        })
        .collect();
    Ok(ModelStateView {
        active: Some(reg.active.clone()),
        copilot: reg.copilot.clone(),
        profiles,
    })
}

/// 切换主模型（即时生效，写入 models.json）
#[tauri::command]
fn model_use(state: State<Arc<AppState>>, id: String) -> Result<(), String> {
    let mut sel = state.model_selector.lock().map_err(|e| e.to_string())?;
    sel.use_active(&id).map_err(|e| e.to_string())
}

/// 设置 / 关闭 Bloom 副模型
#[tauri::command]
fn model_copilot(state: State<Arc<AppState>>, id: Option<String>) -> Result<(), String> {
    let mut sel = state.model_selector.lock().map_err(|e| e.to_string())?;
    sel.use_copilot(id).map_err(|e| e.to_string())
}

/// 删除模型档案（至少保留一个）
#[tauri::command]
fn model_remove(state: State<Arc<AppState>>, id: String) -> Result<(), String> {
    let mut sel = state.model_selector.lock().map_err(|e| e.to_string())?;
    sel.remove(&id).map(|_| ()).map_err(|e| e.to_string())
}

/// 新增自定义 API 接入点（api_key 非空前自动 AES-256-GCM 加密落盘）
#[tauri::command]
#[allow(clippy::too_many_arguments)] // 参数由前端 model_add 调用契约固定
fn model_add(
    state: State<Arc<AppState>>,
    id: String,
    label: String,
    provider: String,
    base_url: String,
    model: String,
    api_key: String,
    kind: String,
) -> Result<(), String> {
    if id.trim().is_empty() {
        return Err("接入点 ID 不可为空".into());
    }
    if base_url.trim().is_empty() || model.trim().is_empty() {
        return Err("基址(base_url)与模型名(model)不可为空".into());
    }
    let stored_key = if api_key.is_empty() {
        String::new()
    } else if byok::is_encrypted(&api_key) {
        api_key
    } else {
        byok::encrypt_api_key(&api_key).map_err(|e| e.to_string())?
    };
    let profile = ModelProfile {
        id: id.clone(),
        label: if label.trim().is_empty() {
            id.clone()
        } else {
            label.trim().to_string()
        },
        provider: if provider.trim().is_empty() {
            "custom".into()
        } else {
            provider.trim().to_string()
        },
        base_url: base_url.trim().to_string(),
        api_key: stored_key,
        model: model.trim().to_string(),
        kind: match kind.as_str() {
            "local" => ProfileKind::Local,
            "custom" => ProfileKind::Custom,
            _ => ProfileKind::Cloud,
        },
        builtin: false,
        temperature: 0.7,
        max_tokens: 4096,
    };
    let mut sel = state.model_selector.lock().map_err(|e| e.to_string())?;
    sel.add(profile).map_err(|e| e.to_string())
}

/// 返回 models.json 绝对路径（供前端展示/调试）
#[tauri::command]
fn model_path() -> String {
    marguerite_core::model::selector::models_path()
        .to_string_lossy()
        .to_string()
}

// ───────────── P2/GUI 对齐：doctor / mcp / skills / session / exec / sandbox ─────────────

/// 将持久化 ModelConfig 转换为 LLMConfig（供 Fleet 等模块使用）
fn model_config_to_llm(
    m: &marguerite_core::persistence::config::ModelConfig,
) -> marguerite_core::model::llm::LLMConfig {
    use marguerite_core::model::llm::{LLMConfig, LLMProvider};
    use marguerite_core::model::thinking::{ThinkingEffort, CONTEXT_WINDOW_DEFAULT};
    let provider = match m.provider.to_lowercase().as_str() {
        "openai" => LLMProvider::OpenAI,
        "deepseek" => LLMProvider::DeepSeek,
        _ => LLMProvider::Custom {
            base_url: m.base_url.clone(),
        },
    };
    LLMConfig {
        provider,
        model: m.model_name.clone(),
        api_key: m.api_key.clone(),
        base_url: m.base_url.clone(),
        max_tokens: 4096,
        temperature: 0.7,
        timeout_secs: 120,
        thinking: ThinkingEffort::Off,
        context_window: CONTEXT_WINDOW_DEFAULT,
    }
}

#[derive(serde::Serialize)]
struct DoctorCheckView {
    name: String,
    ok: bool,
    detail: String,
    hint: String,
}

#[derive(serde::Serialize)]
struct DoctorReportView {
    os: String,
    arch: String,
    config_dir: String,
    model_name: String,
    model_provider: String,
    api_key_configured: bool,
    mcp_servers: usize,
    skills_dir: String,
    checks: Vec<DoctorCheckView>,
    total: usize,
    passed: usize,
    failed: usize,
    status: String,
}

/// 系统诊断（对齐 `marguerite doctor`）
#[tauri::command]
fn doctor_check() -> DoctorReportView {
    let config = Config::load().unwrap_or_default();
    let config_dir = Config::config_dir();
    let key_ok = !config.model.api_key.is_empty();

    let mut checks: Vec<DoctorCheckView> = Vec::new();

    // 配置目录
    checks.push(DoctorCheckView {
        name: "config".into(),
        ok: config_dir.exists(),
        detail: config_dir.display().to_string(),
        hint: if config_dir.exists() {
            String::new()
        } else {
            "配置目录不存在".into()
        },
    });

    // API Key
    checks.push(DoctorCheckView {
        name: "api_key".into(),
        ok: key_ok,
        detail: if key_ok {
            "已配置".into()
        } else {
            "未配置".into()
        },
        hint: if key_ok {
            String::new()
        } else {
            "运行 marguerite model use 配置 API Key".into()
        },
    });

    // MCP
    checks.push(DoctorCheckView {
        name: "mcp".into(),
        ok: true,
        detail: format!("{} 个 MCP 服务器", config.mcp.servers.len()),
        hint: String::new(),
    });

    // 技能目录
    let skills_path = std::path::Path::new(&config.skills.skills_dir);
    let skills_ok = skills_path.exists();
    checks.push(DoctorCheckView {
        name: "skills".into(),
        ok: skills_ok,
        detail: format!(
            "{} ({})",
            config.skills.skills_dir,
            if skills_ok { "存在" } else { "不存在" }
        ),
        hint: if skills_ok {
            String::new()
        } else {
            "技能目录不存在，将使用内置技能".into()
        },
    });

    // 沙箱
    checks.push(DoctorCheckView {
        name: "sandbox".into(),
        ok: true,
        detail: match std::env::consts::OS {
            "linux" => "Landlock 已支持".into(),
            "macos" => "Seatbelt 已支持".into(),
            _ => "当前平台只读降级（符合安全约束）".into(),
        },
        hint: String::new(),
    });

    // 引擎
    let engine = marguerite_core::orchestration::Orchestrator::new(config.clone());
    let engine_ok = engine.tool_count() > 0;
    checks.push(DoctorCheckView {
        name: "engine".into(),
        ok: engine_ok,
        detail: format!(
            "tools={} research_docs={} automations={}",
            engine.tool_count(),
            engine.research_doc_count(),
            engine.list_automations().len()
        ),
        hint: if engine_ok {
            String::new()
        } else {
            "引擎未注册任何工具".into()
        },
    });

    let total = checks.len();
    let passed = checks.iter().filter(|c| c.ok).count();
    let failed = total - passed;
    let status = if failed == 0 { "ok" } else { "degraded" }.to_string();

    DoctorReportView {
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        config_dir: config_dir.display().to_string(),
        model_name: config.model.model_name.clone(),
        model_provider: format!("{:?}", config.model.provider),
        api_key_configured: key_ok,
        mcp_servers: config.mcp.servers.len(),
        skills_dir: config.skills.skills_dir.clone(),
        checks,
        total,
        passed,
        failed,
        status,
    }
}

#[derive(serde::Serialize)]
struct McpServerView {
    name: String,
    command: String,
    args: Vec<String>,
}

/// MCP 服务器列表（对齐 `marguerite mcp list`）
#[tauri::command]
fn mcp_list() -> Vec<McpServerView> {
    let config = Config::load().unwrap_or_default();
    config
        .mcp
        .servers
        .iter()
        .map(|s| McpServerView {
            name: s.name.clone(),
            command: s.command.clone(),
            args: s.args.clone(),
        })
        .collect()
}

#[derive(serde::Serialize)]
struct SkillView {
    name: String,
    description: String,
    version: String,
    author: String,
}

/// 技能列表（对齐 `marguerite skills list`）
#[tauri::command]
fn skills_list() -> Result<Vec<SkillView>, String> {
    let config = Config::load().unwrap_or_default();
    let loader = marguerite_core::skills::loader::SkillLoader::new(std::path::Path::new(
        &config.skills.skills_dir,
    ));
    let skills = loader.load_all().map_err(|e| e.to_string())?;
    Ok(skills
        .iter()
        .map(|s| SkillView {
            name: s.name.clone(),
            description: s.description.clone(),
            version: s.version.clone(),
            author: s.author.clone().unwrap_or_default(),
        })
        .collect())
}

/// 安装技能（对齐 `marguerite skills install <url>`）
#[tauri::command]
async fn skills_install(url: String) -> Result<SkillView, String> {
    let config = Config::load().unwrap_or_default();
    let loader = marguerite_core::skills::loader::SkillLoader::new(std::path::Path::new(
        &config.skills.skills_dir,
    ));
    let skill = loader
        .download_and_register(&url)
        .await
        .map_err(|e| e.to_string())?;
    Ok(SkillView {
        name: skill.name,
        description: skill.description,
        version: skill.version,
        author: skill.author.unwrap_or_default(),
    })
}

#[derive(serde::Serialize)]
struct SessionView {
    session_id: String,
    summary: String,
    message_count: usize,
    updated_at: String,
}

/// 会话列表（对齐 `marguerite session list`）
#[tauri::command]
fn session_list() -> Result<Vec<SessionView>, String> {
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = marguerite_core::persistence::session::SessionManager::new(&db_path);
    manager.init().map_err(|e| e.to_string())?;
    let sessions = manager.list_sessions(50).map_err(|e| e.to_string())?;
    Ok(sessions
        .into_iter()
        .map(|s| SessionView {
            session_id: s.session_id,
            summary: s.summary,
            message_count: s.message_count,
            updated_at: s.updated_at,
        })
        .collect())
}

/// 删除会话（对齐 `marguerite session delete <id>`）
#[tauri::command]
fn session_delete(id: String) -> Result<(), String> {
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = marguerite_core::persistence::session::SessionManager::new(&db_path);
    manager.init().map_err(|e| e.to_string())?;
    manager.delete_session(&id).map_err(|e| e.to_string())
}

/// 创建工作区子目录（用于"新建会话/项目"的落盘目录）
/// 红线修复：此前无后端命令 → 前端无法直接创建文件夹开始对话。
#[tauri::command]
fn create_workspace_folder(name: String) -> Result<String, String> {
    if name.trim().is_empty() {
        return Err("目录名不可为空".into());
    }
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err("目录名不可包含路径分隔符或 ..".into());
    }
    let base = Config::data_dir().join("workspaces");
    let path = base.join(name.trim());
    std::fs::create_dir_all(&path)
        .map_err(|e| format!("创建工作区目录失败({}): {}", path.display(), e))?;
    Ok(path.display().to_string())
}

/// 保存模型路由配置（前端路由切换器写入 → config.yaml.model_routing）
/// 红线修复：此前无后端命令 → 前端"保存路由"必失败。
#[tauri::command]
fn routing_save(json: String) -> Result<(), String> {
    let routing: marguerite_core::persistence::config::ModelRoutingConfig =
        serde_json::from_str(&json).map_err(|e| format!("解析路由配置失败: {}", e))?;
    let mut cfg = Config::load().map_err(|e| e.to_string())?;
    cfg.model_routing = routing;
    cfg.save().map_err(|e| e.to_string())
}

/// modlens 视觉取证：图片(base64) → 结构化 JSON 文档（文字/阅读顺序/语义/版面）。
/// 无视觉模型时降级为元数据兜底（绝不静默失败，返回 source=metadata-fallback）。
#[tauri::command]
async fn modlens_analyze(
    state: State<'_, Arc<AppState>>,
    image_base64: String,
    hint: String,
) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(image_base64.trim())
        .map_err(|e| format!("图片 base64 解码失败: {e}"))?;
    let doc = state.vision.analyze_document(&bytes, &hint).await;
    Ok(doc.to_json())
}

// ───────────── 扑克游戏大厅（Game Hall / 赌 Token）─────────────

/// 创建扑克房间，返回 room_id
#[tauri::command]
fn poker_create_room(
    state: State<'_, Arc<AppState>>,
    name: String,
    small_blind: i64,
    big_blind: i64,
) -> Result<String, String> {
    let mut hub = state.poker.lock().map_err(|e| e.to_string())?;
    Ok(hub.create_room(name, small_blind, big_blind))
}

/// 加入房间（人/AI），返回座位号
#[tauri::command]
fn poker_join_room(
    state: State<'_, Arc<AppState>>,
    room_id: String,
    player_name: String,
    tokens: i64,
    is_human: bool,
) -> Result<u32, String> {
    let mut hub = state.poker.lock().map_err(|e| e.to_string())?;
    hub.join_room(&room_id, player_name, tokens, is_human)
}

/// 开局（发牌 + 盲注）
#[tauri::command]
fn poker_start_room(state: State<'_, Arc<AppState>>, room_id: String) -> Result<(), String> {
    let mut hub = state.poker.lock().map_err(|e| e.to_string())?;
    hub.start_room(&room_id)
}

/// 玩家行动。action: "fold"|"check"|"call"|"raise"|"all_in"（amount 仅 raise 使用）
#[tauri::command]
fn poker_action(
    state: State<'_, Arc<AppState>>,
    room_id: String,
    seat: u32,
    action: String,
    amount: Option<i64>,
) -> Result<(), String> {
    use marguerite_core::poker::ActionKind;
    let kind = match action.as_str() {
        "fold" => ActionKind::Fold,
        "check" => ActionKind::Check,
        "call" => ActionKind::Call,
        "raise" => ActionKind::Raise,
        "all_in" => ActionKind::AllIn,
        other => return Err(format!("未知动作: {other}")),
    };
    let mut hub = state.poker.lock().map_err(|e| e.to_string())?;
    hub.act(&room_id, seat, kind, amount)
}

/// 房间状态快照（供 UI 轮询/刷新）
#[tauri::command]
fn poker_state(
    state: State<'_, Arc<AppState>>,
    room_id: String,
) -> Result<marguerite_core::poker::PokerRoom, String> {
    let hub = state.poker.lock().map_err(|e| e.to_string())?;
    hub.room(&room_id).cloned().ok_or_else(|| "房间不存在".into())
}

/// AI 座位自动行动：循环到轮到人类座位或牌局结束。
/// 简单策略：无注可跟时过牌（有对子以上则最小加注）；注额超过剩余 Token 时好牌全下否则弃牌；其余跟注。
#[tauri::command]
fn poker_auto(state: State<'_, Arc<AppState>>, room_id: String) -> Result<(), String> {
    use marguerite_core::poker::{evaluate_7, ActionKind};
    let mut hub = state.poker.lock().map_err(|e| e.to_string())?;
    for _ in 0..64 {
        let (hand_over, cur, is_human, to_call, tokens, has_pair, street_high, min_raise) = {
            let room = hub.room(&room_id).ok_or("房间不存在")?;
            let idx = room.current_seat as usize;
            let p = &room.players[idx];
            let has_pair = match (p.hole[0], p.hole[1]) {
                (Some(a), Some(b)) => evaluate_7([a, b], &room.board).category >= 1,
                _ => false,
            };
            (
                room.hand_over,
                room.current_seat,
                p.is_human,
                room.street_high - p.street_committed,
                p.tokens,
                has_pair,
                room.street_high,
                room.min_raise,
            )
        };
        if hand_over {
            return Ok(());
        }
        if is_human {
            return Ok(());
        }
        let (action, amount) = if to_call <= 0 {
            if has_pair && tokens > min_raise * 2 {
                (ActionKind::Raise, Some(street_high + min_raise))
            } else {
                (ActionKind::Check, None)
            }
        } else if to_call >= tokens {
            if has_pair {
                (ActionKind::AllIn, None)
            } else {
                (ActionKind::Fold, None)
            }
        } else if has_pair {
            (ActionKind::Call, None)
        } else {
            (ActionKind::Call, None)
        };
        hub.act(&room_id, cur, action, amount)?;
    }
    Ok(())
}

#[derive(serde::Serialize)]
struct ExecResultView {
    stdout: String,
    stderr: String,
    exit_code: i32,
    decision: String,
    truncated: bool,
}

/// 沙箱命令执行（对齐 `marguerite exec -- <cmd>`）
#[tauri::command]
async fn exec_command(argv: Vec<String>) -> Result<ExecResultView, String> {
    use marguerite_core::mode::ModeKind;
    use marguerite_core::sandbox::Sandbox;
    let sandbox = Sandbox::new(std::path::Path::new("."));
    sandbox.set_interactive(false);
    let result = sandbox
        .execute(&argv, std::path::Path::new("."), ModeKind::Agent)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ExecResultView {
        stdout: result.stdout,
        stderr: result.stderr,
        exit_code: result.exit_code,
        decision: result.decision.to_string(),
        truncated: result.truncated,
    })
}

#[derive(serde::Serialize)]
struct SandboxInfoView {
    os: String,
    platform_support: String,
    workspace: String,
}

/// 沙箱状态信息（对齐 `marguerite sandbox workspace`）
#[tauri::command]
fn sandbox_info() -> SandboxInfoView {
    let support = match std::env::consts::OS {
        "linux" => "Landlock (完整沙箱)",
        "macos" => "Seatbelt (完整沙箱)",
        _ => "只读降级 (Windows)",
    };
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".into());
    SandboxInfoView {
        os: std::env::consts::OS.into(),
        platform_support: support.into(),
        workspace: cwd,
    }
}

// ───────────── P2/GUI 对齐：computer_use / kb / fleet / evolve / plugins ─────────────

/// 计算机操控（对齐 TUI/CLI computer_use 工具，三层架构）
#[tauri::command]
async fn computer_use_action(
    action: String,
    x: Option<i64>,
    y: Option<i64>,
    x2: Option<i64>,
    y2: Option<i64>,
    dx: Option<i64>,
    dy: Option<i64>,
    text: Option<String>,
    key: Option<String>,
    mode: Option<String>,
    max_edge: Option<u32>,
) -> Result<String, String> {
    use marguerite_core::tools::computer_use::dispatch;
    use std::collections::HashMap;
    let mut args: HashMap<String, String> = HashMap::new();
    args.insert("action".into(), action);
    if let Some(v) = x {
        args.insert("x".into(), v.to_string());
    }
    if let Some(v) = y {
        args.insert("y".into(), v.to_string());
    }
    if let Some(v) = x2 {
        args.insert("x2".into(), v.to_string());
    }
    if let Some(v) = y2 {
        args.insert("y2".into(), v.to_string());
    }
    if let Some(v) = dx {
        args.insert("dx".into(), v.to_string());
    }
    if let Some(v) = dy {
        args.insert("dy".into(), v.to_string());
    }
    if let Some(v) = text {
        args.insert("text".into(), v);
    }
    if let Some(v) = key {
        args.insert("key".into(), v);
    }
    if let Some(v) = mode {
        args.insert("mode".into(), v);
    }
    if let Some(v) = max_edge {
        args.insert("max_edge".into(), v.to_string());
    }

    let result = dispatch(args, None).await.map_err(|e| e.to_string())?;
    match result {
        marguerite_core::agent::tool_cycle::ToolResult::Success { data, .. } => Ok(data),
        marguerite_core::agent::tool_cycle::ToolResult::Error { message, .. } => Err(message),
        _ => Err("未知结果类型".into()),
    }
}

#[derive(serde::Serialize)]
struct KbSearchResultView {
    doc_id: String,
    score: f64,
    text_preview: String,
}

/// 知识库检索（BM25 混合检索）
#[tauri::command]
fn kb_search(query: String, top_k: Option<usize>) -> Result<Vec<KbSearchResultView>, String> {
    use marguerite_core::kb::hybrid::HybridRetriever;
    use marguerite_core::kb::store::KnowledgeBase;
    // 从数据目录加载持久化 KB（若存在），否则返回空结果
    let data_dir = Config::data_dir();
    let kb_path = data_dir.join("kb_store.json");
    let mut kb = KnowledgeBase::new();
    if kb_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&kb_path) {
            if let Ok(docs) = serde_json::from_str::<Vec<(String, String)>>(&content) {
                for (title, text) in docs {
                    kb.add_document(&title, &text);
                }
            }
        }
    }
    let retriever = HybridRetriever::new(kb);
    let results = retriever.retrieve(&query, top_k.unwrap_or(5));
    Ok(results
        .into_iter()
        .map(|r| KbSearchResultView {
            doc_id: r.doc_id,
            score: r.fused_score,
            text_preview: r.chunk.chars().take(200).collect(),
        })
        .collect())
}

/// 知识库添加文档（同时索引进知识图谱，供 kb_graph_view 可视化）
#[tauri::command]
fn kb_add(state: State<'_, Arc<AppState>>, title: String, text: String) -> Result<(), String> {
    let data_dir = Config::data_dir();
    let _ = std::fs::create_dir_all(&data_dir);
    let kb_path = data_dir.join("kb_store.json");
    let mut docs: Vec<(String, String)> = Vec::new();
    if kb_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&kb_path) {
            if let Ok(d) = serde_json::from_str(&content) {
                docs = d;
            }
        }
    }
    docs.push((title.clone(), text.clone()));
    let json = serde_json::to_string_pretty(&docs).map_err(|e| e.to_string())?;
    std::fs::write(&kb_path, json).map_err(|e| e.to_string())?;
    // 同步索引进内存知识图谱
    let mut graph = state.kb_graph.lock().map_err(|e| e.to_string())?;
    graph.index_document(&title, &text);
    Ok(())
}

#[derive(serde::Serialize)]
struct KbGraphNodeView {
    id: String,
    degree: usize,
}

#[derive(serde::Serialize)]
struct KbGraphEdgeView {
    from: String,
    to: String,
    kind: String,
    weight: f64,
}

#[derive(serde::Serialize)]
struct KbGraphView {
    nodes: Vec<KbGraphNodeView>,
    edges: Vec<KbGraphEdgeView>,
    entity_count: usize,
    edge_count: usize,
}

/// 知识图谱全量视图（节点 + 边），供前端 SVG 图谱面板渲染。
/// 懒加载：把持久化 KB（kb_store.json）中尚未索引的文档补进内存图谱。
#[tauri::command]
fn kb_graph_view(state: State<'_, Arc<AppState>>) -> Result<KbGraphView, String> {
    let mut graph = state.kb_graph.lock().map_err(|e| e.to_string())?;

    // 懒加载持久化 KB 文档（仅补索引尚未进图的 doc_id）
    let data_dir = Config::data_dir();
    let kb_path = data_dir.join("kb_store.json");
    if kb_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&kb_path) {
            if let Ok(docs) = serde_json::from_str::<Vec<(String, String)>>(&content) {
                for (title, text) in docs {
                    if graph.entities_in_document(&title).is_empty() {
                        graph.index_document(&title, &text);
                    }
                }
            }
        }
    }

    // 计算度数（出/入边计数）
    let edges = graph.edges();
    let mut degree: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for e in edges {
        *degree.entry(e.from.clone()).or_insert(0) += 1;
        *degree.entry(e.to.clone()).or_insert(0) += 1;
    }
    let nodes: Vec<KbGraphNodeView> = graph
        .entities()
        .into_iter()
        .map(|id| KbGraphNodeView {
            degree: degree.get(id).copied().unwrap_or(0),
            id: id.to_string(),
        })
        .collect();
    let edges_view: Vec<KbGraphEdgeView> = edges
        .iter()
        .map(|e| KbGraphEdgeView {
            from: e.from.clone(),
            to: e.to.clone(),
            kind: e.kind.label().to_string(),
            weight: e.weight,
        })
        .collect();
    Ok(KbGraphView {
        entity_count: graph.entity_count(),
        edge_count: graph.edge_count(),
        nodes,
        edges: edges_view,
    })
}

/// 查询与某实体直接相连的邻居
#[tauri::command]
fn kb_graph_related(
    state: State<'_, Arc<AppState>>,
    entity: String,
) -> Result<Vec<String>, String> {
    let graph = state.kb_graph.lock().map_err(|e| e.to_string())?;
    Ok(graph.neighbors(&entity))
}

// ───────────── Daily Usage（Token 消费统计，原生内置）─────────────

/// 用量聚合（总量/输入/输出/会话/调用次数/分模型明细）。
/// 台账为进程级单例（`usage::init` 幂等），数据落盘 `~/.marguerite/usage.db`。
#[tauri::command]
fn usage_summary() -> Result<serde_json::Value, String> {
    marguerite_core::usage::init();
    Ok(marguerite_core::usage::status(&serde_json::json!({})))
}

/// 最近 N 天按日聚合（热力图/月度开销数据源），UTC 日边界。
#[tauri::command]
fn usage_daily(days: u32) -> Result<serde_json::Value, String> {
    marguerite_core::usage::init();
    Ok(marguerite_core::usage::daily(days))
}

// ───────────── 鲸鱼娘桌宠（原生状态机，好感度持久化）─────────────

#[derive(serde::Serialize)]
struct PetStatusView {
    mood: String,
    energy: u8,
    affection: u32,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PetStore {
    mood: String,
    energy: u8,
    affection: u32,
    last_ms: u64,
}

fn pet_store_path() -> std::path::PathBuf {
    Config::data_dir().join("pet_state.json")
}

fn pet_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn load_pet_store() -> PetStore {
    match std::fs::read_to_string(pet_store_path()) {
        Ok(s) => serde_json::from_str(&s).unwrap_or(PetStore {
            mood: "idle".into(),
            energy: 100,
            affection: 0,
            last_ms: pet_now_ms(),
        }),
        Err(_) => PetStore {
            mood: "idle".into(),
            energy: 100,
            affection: 0,
            last_ms: pet_now_ms(),
        },
    }
}

fn save_pet_store(p: &PetStore) {
    if let Ok(json) = serde_json::to_string(p) {
        let _ = std::fs::write(pet_store_path(), json);
    }
}

fn pet_mood_from_str(s: &str) -> marguerite_core::workbench::pet::PetMood {
    match s {
        "happy" => marguerite_core::workbench::pet::PetMood::Happy,
        "sleepy" => marguerite_core::workbench::pet::PetMood::Sleepy,
        "working" => marguerite_core::workbench::pet::PetMood::Working,
        _ => marguerite_core::workbench::pet::PetMood::Idle,
    }
}

fn pet_mood_str(m: marguerite_core::workbench::pet::PetMood) -> &'static str {
    match m {
        marguerite_core::workbench::pet::PetMood::Idle => "idle",
        marguerite_core::workbench::pet::PetMood::Happy => "happy",
        marguerite_core::workbench::pet::PetMood::Sleepy => "sleepy",
        marguerite_core::workbench::pet::PetMood::Working => "working",
    }
}

/// 桌宠状态（读取时按流逝时间做能量衰减 tick）
#[tauri::command]
fn pet_status() -> Result<PetStatusView, String> {
    use marguerite_core::workbench::pet::{tick, PetState};
    let mut store = load_pet_store();
    let now = pet_now_ms();
    let dt_secs = now.saturating_sub(store.last_ms) / 1000;
    let cur = PetState {
        mood: pet_mood_from_str(&store.mood),
        energy: store.energy,
    };
    let next = tick(&cur, dt_secs);
    store.energy = next.energy;
    store.mood = pet_mood_str(next.mood).to_string();
    store.last_ms = now;
    save_pet_store(&store);
    Ok(PetStatusView {
        mood: store.mood.clone(),
        energy: store.energy,
        affection: store.affection,
    })
}

/// 桌宠互动：pet（摸头）/ feed（投喂小鱼干），回能升 Happy 且好感度 +1
#[tauri::command]
fn pet_action(action: String) -> Result<PetStatusView, String> {
    use marguerite_core::workbench::pet::{feed, pet, PetState};
    let mut store = load_pet_store();
    let cur = PetState {
        mood: pet_mood_from_str(&store.mood),
        energy: store.energy,
    };
    let next = match action.as_str() {
        "pet" => pet(&cur),
        "feed" => feed(&cur),
        other => return Err(format!("未知互动: {other}")),
    };
    store.energy = next.energy;
    store.mood = pet_mood_str(next.mood).to_string();
    store.affection = store.affection.saturating_add(1);
    store.last_ms = pet_now_ms();
    save_pet_store(&store);
    Ok(PetStatusView {
        mood: store.mood.clone(),
        energy: store.energy,
        affection: store.affection,
    })
}

// ───────────── Better Sidebar：文件树 / 编辑器 / 浏览器 ─────────────

#[derive(serde::Serialize)]
struct FsEntryView {
    name: String,
    path: String,
    is_dir: bool,
    size: u64,
}

/// 列目录。空路径默认到工作区根（data_dir/workspaces）。
#[tauri::command]
fn fs_list_dir(path: String) -> Result<Vec<FsEntryView>, String> {
    let base = Config::data_dir();
    let p = if path.trim().is_empty() {
        base.join("workspaces")
    } else {
        std::path::PathBuf::from(&path)
    };
    let _ = std::fs::create_dir_all(&p);
    let canon = p
        .canonicalize()
        .map_err(|e| format!("无法访问目录 {}: {}", p.display(), e))?;
    let mut entries: Vec<FsEntryView> = Vec::new();
    let rd = std::fs::read_dir(&canon).map_err(|e| format!("读取目录失败: {e}"))?;
    for item in rd.flatten() {
        let name = item.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue; // 隐藏文件/目录不展示，保持树干净
        }
        let is_dir = item.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let size = if is_dir {
            0
        } else {
            item.metadata().map(|m| m.len()).unwrap_or(0)
        };
        entries.push(FsEntryView {
            name,
            path: item.path().to_string_lossy().to_string(),
            is_dir,
            size,
        });
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(entries)
}

/// 读文本文件（>512KB 拒绝，防止 UI 卡死）
#[tauri::command]
fn fs_read_file(path: String) -> Result<String, String> {
    let meta = std::fs::metadata(&path).map_err(|e| format!("无法访问文件: {e}"))?;
    if meta.len() > 512 * 1024 {
        return Err("文件超过 512KB，请用外部编辑器打开".into());
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("读取失败: {e}"))?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

/// 写文本文件（自动创建父目录）
#[tauri::command]
fn fs_write_file(path: String, content: String) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, content).map_err(|e| format!("写入失败: {e}"))
}

/// 在系统默认浏览器打开链接（仅 http/https）
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("仅支持 http/https 链接".into());
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &url])
            .spawn()
            .map_err(|e| format!("打开浏览器失败: {e}"))?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("打开浏览器失败: {e}"))?;
    }
    Ok(())
}

#[derive(serde::Serialize)]
struct FleetRoleView {
    role_id: String,
    provider: String,
    model: String,
    reasoning: String,
}

/// Fleet 角色列表（多模型并行路由）
#[tauri::command]
fn fleet_list() -> Vec<FleetRoleView> {
    let config = Config::load().unwrap_or_default();
    let llm = model_config_to_llm(&config.model);
    let fleet = marguerite_core::model::fleet::FleetConfig::with_defaults(&llm);
    fleet
        .roles
        .iter()
        .map(|(id, b)| FleetRoleView {
            role_id: id.clone(),
            provider: format!("{:?}", b.config.provider),
            model: b.config.model.clone(),
            reasoning: format!("{:?}", b.reasoning),
        })
        .collect()
}

/// Fleet 路由推荐（按任务描述推荐角色）
#[tauri::command]
fn fleet_route(task: String) -> Result<FleetRoleView, String> {
    let config = Config::load().unwrap_or_default();
    let llm = model_config_to_llm(&config.model);
    let fleet = marguerite_core::model::fleet::FleetConfig::with_defaults(&llm);
    let router = marguerite_core::model::fleet::FleetRouter::new(fleet);
    let decision = router
        .route_by_task(&task)
        .ok_or_else(|| "无可用路由".to_string())?;
    Ok(FleetRoleView {
        role_id: decision.role_id,
        provider: format!("{:?}", decision.binding.config.provider),
        model: decision.binding.config.model,
        reasoning: format!("{:?}", decision.binding.reasoning),
    })
}

#[derive(serde::Serialize)]
struct EvolutionRecordView {
    round: usize,
    accepted: bool,
    score: f64,
    timestamp: String,
    summary: String,
}

/// 自进化历史（对齐 harness evolve）
#[tauri::command]
fn evolve_history() -> Result<Vec<EvolutionRecordView>, String> {
    let data_dir = Config::data_dir();
    let runner = marguerite_core::harness::evolve::EvolutionRunner::new(&data_dir, "");
    let history = runner.load_history().map_err(|e| e.to_string())?;
    Ok(history
        .into_iter()
        .map(|r| EvolutionRecordView {
            round: r.round,
            accepted: r.accepted,
            score: r.score_after,
            timestamp: r.timestamp,
            summary: r.diagnosis.chars().take(200).collect(),
        })
        .collect())
}

#[derive(serde::Serialize)]
struct PluginSlotView {
    name: String,
    kind: String,
    description: String,
    version: String,
}

/// 插件槽列表（对齐 PluginSlot 统一注册）
#[tauri::command]
fn plugins_list() -> Vec<PluginSlotView> {
    // 返回内置工具/技能槽位信息（从能力注册表派生）
    use marguerite_core::domain::capabilities;
    capabilities::registry()
        .iter()
        .filter(|c| c.maturity.is_implemented())
        .take(50)
        .map(|c| PluginSlotView {
            name: c.id.0.clone(),
            kind: format!("{:?}", c.group),
            description: c.description.clone(),
            version: "0.1.0".into(),
        })
        .collect()
}

/// 构建系统托盘与右键原生菜单
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示工作台", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "隐藏到托盘", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 Marguerite", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&show, &hide, &PredefinedMenuItem::separator(app)?, &quit],
    )?;

    let icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))?;

    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("Marguerite 工作台")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_window(app.clone()),
            "hide" => hide_window(app.clone()),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_window(tray.app_handle().clone());
            }
        })
        .build(app)?;

    Ok(())
}

/// 全局快捷键备选链：主键被系统占用时依次尝试后备组合
const SHORTCUT_CANDIDATES: &[&str] = &[
    "CmdOrCtrl+Shift+Space",
    "CmdOrCtrl+Shift+M",
    "CmdOrCtrl+Alt+M",
    "CmdOrCtrl+Shift+Comma",
];

/// 注册全局快捷键（切换主窗口）。若主键已被系统占用，自动降级到备选组合，
/// 避免「HotKey already registered」报错导致功能不可用。
fn register_shortcut(app: &tauri::AppHandle) {
    for spec in SHORTCUT_CANDIDATES {
        let shortcut: Shortcut = match spec.parse() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[phase-c] 快捷键解析失败 ({spec}): {e}");
                continue;
            }
        };
        match app
            .global_shortcut()
            .on_shortcut(shortcut, |app, _shortcut, event| {
                if event.state == ShortcutState::Pressed {
                    toggle_window(app.clone());
                }
            }) {
            Ok(_) => {
                eprintln!("[phase-c] 全局快捷键已注册: {spec}");
                return;
            }
            Err(e) => {
                eprintln!("[phase-c] 全局快捷键注册失败 ({spec}): {e}");
            }
        }
    }
    eprintln!("[phase-c] 警告: 全部快捷键组合均被占用，切换窗口请用托盘");
}

/// 解析 Marguerite Web 界面产物目录（打包资源 > exe 同目录/上溯 > 工作目录），
/// 供内嵌网关托管 `/control/` 控制台。
///
/// 找不到时返回 `None`，由 core 的 `GatewayState::new` 继续走
/// `MARGUERITE_WEBUI_DIST` 环境变量与 `detect_webui_dir()`（含 cwd 上溯
/// `../webui/dist`）——绝不能回退成相对路径 `webui/dist` 去短路探测，
/// 否则双击裸 exe（如 `target\release`）时网关会 serve 不存在的目录，
/// UI 加载旧实例或 404，新改动不可见。
fn resolve_webui_dir(app: &tauri::App) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    let mut candidates = Vec::new();
    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join("webui/dist"));
    }
    if let Ok(exe) = app.path().executable_dir() {
        candidates.push(exe.join("webui/dist"));
        // 与 pyengine 侧车探测一致：从 exe 目录向上逐级查找 `webui/dist`，
        // 覆盖 `target\release\marguerite-gui.exe` 双击 → `marguerite\webui\dist`。
        let mut base = exe;
        for _ in 0..6 {
            match base.parent() {
                Some(p) => {
                    candidates.push(p.join("webui/dist"));
                    base = p.to_path_buf();
                }
                None => break,
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("webui/dist"));
    }
    candidates
        .into_iter()
        .find(|p| p.join("index.html").is_file())
}

/// 解析内嵌技能目录（打包资源），供 `setup` 注入 `config.skills.skills_dir`。
///
/// 与 `resolve_webui_dir` 同构：优先 `resource_dir()/skills`，其次 `executable_dir()/skills`
/// 及逐级上溯，最后 cwd 直连。仅当该目录真实存在才返回 `Some`，否则返回 `None`，
/// 由 core 继续走默认相对 `"skills"`（cwd 上溯）——保证开发期未复制技能时不破坏。
fn resolve_skills_dir(app: &tauri::App) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    let mut candidates = Vec::new();
    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join("skills"));
    }
    if let Ok(exe) = app.path().executable_dir() {
        candidates.push(exe.join("skills"));
        // 从 exe 目录向上逐级查找 `skills`，覆盖 `target\release` 双击等场景。
        let mut base = exe;
        for _ in 0..6 {
            match base.parent() {
                Some(p) => {
                    candidates.push(p.join("skills"));
                    base = p.to_path_buf();
                }
                None => break,
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("skills"));
    }
    candidates.into_iter().find(|p| p.is_dir())
}

/// 阻塞原生对话框后退出进程（用于致命错误，如端口被其它实例占用）。
///
/// 仅在启动期或网关线程等无法继续运行的场景调用；调用后进程终止，不会返回。
fn alert_fatal(app: &tauri::AppHandle, title: &str, msg: &str) -> ! {
    let _ = app.dialog().message(msg).title(title).blocking_show();
    std::process::exit(1);
}

/// 非阻塞原生对话框（用于非致命告警，如托盘不可用、网关未就绪）。
///
/// 在 Tauri 异步运行时上派生任务展示，不阻塞启动主流程。
fn alert_async(app: &tauri::AppHandle, title: &str, msg: &str) {
    let handle: tauri::AppHandle = app.app_handle().clone();
    let title = title.to_string();
    let msg = msg.to_string();
    tauri::async_runtime::spawn(async move {
        let _ = handle.dialog().message(msg).title(title).show(move |_| {});
    });
}

/// 应用入口
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // 第二实例启动：聚焦已存在的 “main” 窗口，而非再开一个进程。
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();
            }
        }))
        .setup(|app| {
            let handle = app.handle().clone();

            // ── 单实例端口守卫（127.0.0.1:DEFAULT_PORT）──
            // 若端口已被其它 Marguerite 实例（或残留进程）占用，说明已有实例在运行。
            // 直接弹窗告知并退出，而非静默降级到陈旧实例。
            match std::net::TcpListener::bind(std::net::SocketAddr::from(([127u8, 0, 0, 1], DEFAULT_PORT))) {
                Ok(_guard) => { /* 端口空闲：立即释放，交由网关于线程内绑定 */ }
                Err(e)
                    if e.kind() == std::io::ErrorKind::AddrInUse
                        || e.kind() == std::io::ErrorKind::PermissionDenied =>
                {
                    alert_fatal(
                        &handle,
                        "Marguerite 已在运行",
                        &format!(
                            "检测到端口 127.0.0.1:{} 已被占用，另一个 Marguerite 实例可能正在运行。\n请先关闭已运行的实例后再启动本程序。",
                            DEFAULT_PORT
                        ),
                    );
                }
                Err(e) => {
                    alert_fatal(
                        &handle,
                        "无法启动内嵌网关",
                        &format!("内嵌网关端口 {} 绑定失败：{e}", DEFAULT_PORT),
                    );
                }
            }

            if let Err(e) = build_tray(&handle) {
                // 托盘非致命：用原生对话框告知，但继续启动（不退出）。
                alert_async(
                    &handle,
                    "系统托盘不可用",
                    &format!("托盘初始化失败，部分快捷操作将不可用：{e}"),
                );
            }
            register_shortcut(&handle);

            // ── 内嵌统一网关（双引擎桥）：桌面端承载 Marguerite Web 界面 ──
            // 同进程启动 18790 回环网关并托管 webui/dist 控制台，Web 界面经
            // `ws://localhost:18790/ws` 走 RPC v3 全量方法（渠道/技能/记忆/工具）。
            // 端口占用已在 setup 入口的单实例守卫处拦截（弹窗并退出）；此处网关节点
            // 若仍 bind 失败，同样走 alert_fatal 弹窗退出，不再静默降级到陈旧实例。
            let webui_dir = resolve_webui_dir(app);
            let bundled_skills = resolve_skills_dir(app);
            let gw_app = app.handle().clone();
            std::thread::spawn(move || {
                let rt = match tokio::runtime::Runtime::new() {
                    Ok(rt) => rt,
                    Err(e) => {
                        // 运行时起不来则无法承载网关：弹窗并退出，而非静默返回。
                        alert_fatal(
                            &gw_app,
                            "无法启动内嵌网关",
                            &format!("Tokio 运行时初始化失败：{e}"),
                        );
                    }
                };
                rt.block_on(async move {
                    let mut config = marguerite_core::persistence::config::Config::load()
                        .unwrap_or_default();
                    // 安装版：把打包内嵌的技能目录（resource_dir/skills 等）注入
                    // config.skills.skills_dir，使 WebUI skills.list 能枚举到内置技能；
                    // 开发期 src-tauri/skills 未复制时 bundled_skills 为 None，保留默认
                    // 相对 "skills"（cwd 上溯）以兼容本地源码运行。仅改内存快照，不落盘，
                    // 避免安装路径变更后残留绝对路径失效。
                    if let Some(skills) = bundled_skills {
                        config.skills.skills_dir = skills.to_string_lossy().into_owned();
                    }
                    match GatewayState::new(config, webui_dir) {
                        Ok(state) => {
                            let state = std::sync::Arc::new(state);
                            if let Err(e) = gateway::run(state, DEFAULT_PORT).await {
                                // 端口被占用（如另一 GUI/CLI 实例已占 18790）：bind 失败即视为
                                // 已有实例在运行，弹窗告知并退出，避免「静默降级到陈旧实例」。
                                alert_fatal(
                                    &gw_app,
                                    "Marguerite 已在运行",
                                    &format!(
                                        "内嵌网关未能在 127.0.0.1:{} 启动（{e}）。\n端口可能已被另一个 Marguerite 实例占用，请先关闭它再启动本程序。",
                                        DEFAULT_PORT
                                    ),
                                );
                            }
                        }
                        Err(e) => alert_fatal(
                            &gw_app,
                            "无法启动内嵌网关",
                            &format!("网关状态初始化失败：{e}"),
                        ),
                    }
                });
            });

            // ── 网关就绪等待（窗口加载 /control/ 前确保网关已监听）──
            {
                let addr: std::net::SocketAddr = ([127, 0, 0, 1], DEFAULT_PORT).into();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
                let mut ready = false;
                while std::time::Instant::now() < deadline {
                    if std::net::TcpStream::connect_timeout(
                        &addr,
                        std::time::Duration::from_millis(200),
                    )
                    .is_ok()
                    {
                        ready = true;
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(150));
                }
                if ready {
                    eprintln!("[gateway] 内嵌网关已在 127.0.0.1:{DEFAULT_PORT} 就绪");
                } else {
                    // 网关未就绪：窗口可能显示错误页。用原生对话框告知用户（非致命，
                    // 不阻塞启动），便于排查端口被占用等环境问题。
                    alert_async(
                        &handle,
                        "内嵌网关未就绪",
                        &format!(
                            "8 秒内内嵌网关未在 127.0.0.1:{} 就绪，主窗口可能显示错误页。\n若已有其它 Marguerite 实例占用该端口，请先关闭它。",
                            DEFAULT_PORT
                        ),
                    );
                }
            }
            Ok(())
        })
        .manage(Arc::new(AppState::new()))
        .invoke_handler(tauri::generate_handler![
            guard_check,
            repair_code,
            plan_task,
            run_turn,
            toggle_window,
            show_window,
            hide_window,
            record_experience,
            search_experience,
            model_list,
            model_use,
            model_copilot,
            model_remove,
            model_add,
            model_path,
            capabilities_groups,
            capabilities_stats,
            harness_capabilities,
            sidecar_status,
            sidecar_start,
            sidecar_stop,
            doctor_check,
            mcp_list,
            skills_list,
            skills_install,
            session_list,
            session_delete,
            create_workspace_folder,
            routing_save,
            modlens_analyze,
            exec_command,
            sandbox_info,
            computer_use_action,
            kb_search,
            kb_add,
            fleet_list,
            fleet_route,
            evolve_history,
            plugins_list,
            poker_create_room,
            poker_join_room,
            poker_start_room,
            poker_action,
            poker_state,
            poker_auto,
            kb_graph_view,
            kb_graph_related,
            usage_summary,
            usage_daily,
            pet_status,
            pet_action,
            fs_list_dir,
            fs_read_file,
            fs_write_file,
            open_url
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
