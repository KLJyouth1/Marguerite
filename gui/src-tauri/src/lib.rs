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
use marguerite_core::guard::approval_registry::{global_registry, DecisionResult};
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
    /// P0 S3：磁盘 `plugins/manifests/*.plugin.json` 真源快照（setup 时只读装载一次）。
    ///
    /// 供 `plugins_list` / `doctor_check` 消费 —— 取代原先「从能力注册表假投影」的写法。
    pub plugin_manifests: Arc<Mutex<Vec<marguerite_core::plugins::manifest::PluginManifest>>>,
    /// P0 S3：patch 层只读装载摘要（含诊断）；未装载时为 `None`。
    pub plugin_patch_summary: Arc<Mutex<Option<marguerite_core::plugins::patch::PatchSummary>>>,
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
        //
        // 配置必须来自**磁盘 + 生效模型叠加**，不能用 `Config::default()`：后者
        // api_key / provider / base_url 全为空，一旦前端经 `run_turn` 走这条内嵌引擎，
        // 就会在「已配置密钥」的机器上必然鉴权失败（真实用户看到的是一句 401）。
        // 叠加 `models.json` active 档案，与 CLI / TUI / 网关看到同一个模型。
        let boot_config = marguerite_core::model::selector::apply_effective_model(
            &Config::load().unwrap_or_default(),
        );
        let engine: Arc<dyn EngineRunner> = Arc::new(
            RealEngine::from_config(boot_config).with_evolution(Some(evolution.clone())),
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
            plugin_manifests: Arc::new(Mutex::new(Vec::new())),
            plugin_patch_summary: Arc::new(Mutex::new(None)),
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

// ─────────────────────────── 开机自启动（HKCU Run 键，零依赖） ───────────────────────────
//
// 为什么不用 tauri-plugin-autostart：那需要联网拉新依赖，而本机到 crates.io 未必可达
// （实测 huggingface 已被拦）。这里直接用 Windows 自带的 `reg.exe` 读写 HKCU 的 Run 键：
// 零新依赖、不触网、对安装后的 exe 同样有效。
//
// ⚠ 安装器必须同步：开启后会在 HKCU 写入一条名为 `Marguerite` 的值。卸载流程若不删除它，
// 会残留一条指向已删除 exe 的自启动项——所以 wix/nsis 的卸载步骤要一并清掉（见安装器配置）。
#[cfg(windows)]
const AUTOSTART_VALUE_NAME: &str = "Marguerite";
#[cfg(windows)]
const AUTOSTART_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// 开机自启动是否已开启。
///
/// 判定口径：Run 键里**存在** `Marguerite` 这个值即算开（不比对路径）。
/// 为什么不比对路径：便携版/多份安装并存时 exe 路径会变，按"值是否在册"判定更贴合
/// 用户对这个开关的意图；比对路径反而会把"用户已开启"误报成"已关闭"。
#[tauri::command]
fn autostart_status() -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new("reg")
            .args(["query", AUTOSTART_KEY, "/v", AUTOSTART_VALUE_NAME])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        // 非 Windows 平台不提供该能力：如实返回 false，而不是编造一个"已开启"。
        false
    }
}

/// 开启/关闭开机自启动。
///
/// 失败必须回 Err 让前端如实提示（例如组策略限制 Run 键），**不要假装成功**——
/// 否则用户会以为下次开机应用会自动起来，实际不会。
#[tauri::command]
fn autostart_set(enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe()
            .map_err(|e| format!("取当前程序路径失败：{e}"))?
            .to_string_lossy()
            .to_string();
        let out = if enabled {
            std::process::Command::new("reg")
                .args([
                    "add", AUTOSTART_KEY, "/v", AUTOSTART_VALUE_NAME, "/t", "REG_SZ", "/d", &exe, "/f",
                ])
                .output()
        } else {
            std::process::Command::new("reg")
                .args(["delete", AUTOSTART_KEY, "/v", AUTOSTART_VALUE_NAME, "/f"])
                .output()
        };
        match out {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => Err(format!("调用 reg.exe 失败：{e}")),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
        Err("开机自启动目前仅支持 Windows".to_string())
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
        // P0-1：崩溃循环熔断——UI 据此提示「侧车反复崩溃，已停止自动重启」。
        PyEngineStatus::Degraded => "degraded",
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

/// 侧车日志目录（Runtime 面板展示 + 排障入口）。
///
/// 与 `core/src/pyengine/sidecar.rs` 的 `sidecar_log_dir()` **同源**：
/// 不让前端自己拼 `%TEMP%/marguerite-sidecar`，否则一旦上游改了布局
/// （见该函数注释里的 `MARGUERITE_SIDECAR_LOG_DIR` 覆盖与迁移取舍），
/// 面板会指向一个不存在的路径 —— 那正是"看起来有功能、实际点不动"。
#[tauri::command]
fn sidecar_log_dir() -> String {
    marguerite_core::pyengine::sidecar::sidecar_log_dir()
        .display()
        .to_string()
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

// ───────────── M2-C1：GUI 审批裁决桥（Intent Preview 动作确认卡）─────────────
// GUI 端事件经 `mgt:{session}` 转发 core 的 `approval` Envelope（含结构化
// tool_intent::ToolIntention），前端渲染 ActionCard；批准/拒绝走本命令，
// 经全局审批注册表定位持有挂起审批的 gate 完成裁决（fail-closed 语义由
// gate::resolve 保证：超时 / 已消费 / ID 不匹配一律不执行）。

#[derive(serde::Serialize)]
struct ApprovalDecisionView {
    /// 裁决是否真正消费了挂起审批（false = 不存在/已超时/已消费）
    resolved: bool,
}

fn approval_decide(id: String, approve: bool) -> ApprovalDecisionView {
    let resolved = match global_registry().decide(&id, approve) {
        DecisionResult::Decided(_) => true,
        DecisionResult::NotFound => false,
    };
    ApprovalDecisionView { resolved }
}

/// 批准一条挂起审批（M2-C1）
#[tauri::command]
fn approval_approve(approval_id: String) -> ApprovalDecisionView {
    approval_decide(approval_id, true)
}

/// 拒绝一条挂起审批（M2-C1）
#[tauri::command]
fn approval_deny(approval_id: String) -> ApprovalDecisionView {
    approval_decide(approval_id, false)
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
    // 与构造 `state.engine` 时用的是**同一份**生效配置：若这里只看 config.yaml，
    // 而引擎用的是叠加后的档案密钥，两处判据会分叉（一处说「未配置」、另一处却真发请求）。
    let config = marguerite_core::model::selector::apply_effective_model(
        &Config::load().unwrap_or_default(),
    );
    if config.model.api_key.is_empty() {
        let event_name = format!("mgt:{session}");
        let _ = app.emit(&event_name, serde_json::json!({
            "event": "error",
            "payload": {
                "kind": "configuration",
                "code": "api_key_missing",
                "message": "API Key 未配置",
                "detail": "当前没有配置任何大模型 API Key，无法发送消息。请先配置 API Key 后再试。",
                "action": {
                    "label": "去配置",
                    "target": "settings/providers"
                },
                "recoverable": true
            }
        }));
        return Ok(());
    }

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

// ───────────── V2.1-P2：桌面桌宠悬浮窗（pet-overlay）─────────────
//
// 设计约束（R1/R3 诚实透明 + 回环红线）：
//   · 悬浮窗 label=pet-overlay，decorations=false transparent=true always_on_top=true，
//     尺寸 360×420，缺省位置屏幕右下；
//   · URL 指向 /control/pet-overlay（由统一网关托管的 Vue3 页面，见 WebUI PetOverlayView），
//     复用 pet.* RPC 全套契约，不新增 Tauri→Core 通道；
//   · 桌面交互（拖拽/双击抚摸/右键菜单）全部由 Tauri Window 级 API 完成，
//     不要求 webview 捕获系统事件（WebGL 画布透明时 click-through 会失效，走菜单兜底）。

const PET_OVERLAY_LABEL: &str = "pet-overlay";

/// 桌宠悬浮窗 URL。**端口取自单一事实源** `marguerite_core::gateway::DEFAULT_PORT`
/// （`core/src/gateway/mod.rs`），不硬编码 —— 避免「改端口要改 N 处」的静默漂移。
fn pet_overlay_url() -> String {
    format!("http://localhost:{DEFAULT_PORT}/control/pet-overlay")
}
const PET_OVERLAY_W: f64 = 360.0;
const PET_OVERLAY_H: f64 = 420.0;

/// 计算桌宠悬浮窗缺省位置：屏幕右下角（有工作区则避开任务栏 + 24px 边距）。
/// 纯函数（可单测）：`workarea = (x, y, width, height)`；无显示器信息时回退 (640, 480)。
fn pet_overlay_default_position(
    workarea: Option<(f64, f64, f64, f64)>,
    w: f64,
    h: f64,
) -> (f64, f64) {
    match workarea {
        Some((x, y, ww, wh)) => (x + ww - w - 24.0, y + wh - h - 24.0),
        None => (640.0, 480.0),
    }
}

/// 创建 / 显示桌宠悬浮窗（幂等：已存在则 show + 置前）。
///
/// 返回窗口句柄信息（是否新建），供前端托盘按钮判断是否需要刷新皮肤。
#[derive(serde::Serialize)]
struct PetOverlayOp {
    created: bool,
    label: String,
}

#[tauri::command]
fn pet_overlay_show(app: tauri::AppHandle) -> Result<PetOverlayOp, String> {
    use tauri::Manager;
    use tauri::WebviewWindowBuilder;

    if let Some(win) = app.get_webview_window(PET_OVERLAY_LABEL) {
        let _ = win.show();
        let _ = win.set_always_on_top(true);
        return Ok(PetOverlayOp { created: false, label: PET_OVERLAY_LABEL.into() });
    }

    // 计算默认位置：屏幕右下角（有工作区则避开任务栏）。
    let work = match app.primary_monitor() {
        Ok(Some(monitor)) => {
            let a = monitor.work_area();
            Some((
                a.position.x as f64,
                a.position.y as f64,
                a.size.width as f64,
                a.size.height as f64,
            ))
        }
        _ => None,
    };
    let (x, y) = pet_overlay_default_position(work, PET_OVERLAY_W, PET_OVERLAY_H);

    // 窗口创建后由 Tauri 按 label 管理生命周期，无需本地持有句柄（前缀 _ 避免 unused 告警）
    let _win = WebviewWindowBuilder::new(&app, PET_OVERLAY_LABEL, tauri::WebviewUrl::External(pet_overlay_url().parse().map_err(|e| format!("非法桌宠 URL: {e}"))?))
        .title("Marguerite Pet")
        .inner_size(PET_OVERLAY_W, PET_OVERLAY_H)
        .position(x, y)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .visible(true)
        .skip_taskbar(true)
        .build()
        .map_err(|e| format!("桌宠悬浮窗创建失败: {e}"))?;

    // Win11 需要再显式 set_ignore_cursor_events(false) 才能穿透透明区域，此处保持默认
    // （让用户能右键菜单、拖拽标题区）。

    Ok(PetOverlayOp { created: true, label: PET_OVERLAY_LABEL.into() })
}

/// 隐藏桌宠悬浮窗（存在则 hide，不存在静默跳过，保持幂等）。
#[tauri::command]
fn pet_overlay_hide(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    if let Some(win) = app.get_webview_window(PET_OVERLAY_LABEL) {
        let _ = win.hide();
    }
    Ok(())
}

/// 切换显示/隐藏（托盘右键菜单快捷入口）。
#[tauri::command]
fn pet_overlay_toggle(app: tauri::AppHandle) -> Result<PetOverlayOp, String> {
    use tauri::Manager;
    match app.get_webview_window(PET_OVERLAY_LABEL) {
        Some(win) if win.is_visible().unwrap_or(false) => {
            let _ = win.hide();
            Ok(PetOverlayOp { created: false, label: PET_OVERLAY_LABEL.into() })
        }
        _ => pet_overlay_show(app),
    }
}

/// 桌宠悬浮窗：拖拽到新屏幕坐标（x,y 为窗口左上角，由前端 `dragend` 上报）。
#[tauri::command]
fn pet_overlay_move(app: tauri::AppHandle, x: f64, y: f64) -> Result<(), String> {
    use tauri::Manager;
    if let Some(win) = app.get_webview_window(PET_OVERLAY_LABEL) {
        let _ = win.set_position(tauri::LogicalPosition { x, y });
    }
    Ok(())
}

// ───────────── P0-4：任务悬浮球（task-overlay）─────────────
//
// 与 pet-overlay 同源同风格：同样由网关托管的 Vue 页面渲染（/control/task-overlay），
// 窗口配置、生命周期与拖拽命令完全复用 pet-overlay 的写法，不新增窗口管理层。

const TASK_OVERLAY_LABEL: &str = "task-overlay";
const TASK_OVERLAY_W: f64 = 120.0;
const TASK_OVERLAY_H: f64 = 120.0;

/// 任务悬浮球 URL：端口同样取自 `marguerite_core::gateway::DEFAULT_PORT`。
fn task_overlay_url() -> String {
    format!("http://localhost:{DEFAULT_PORT}/control/task-overlay")
}

/// 任务悬浮球缺省位置：屏幕右下角，但落在桌宠悬浮窗顶部之上（12px 间距），
/// 避免两者在缺省位置重叠。纯函数（可单测）。
fn task_overlay_default_position(
    workarea: Option<(f64, f64, f64, f64)>,
    w: f64,
    h: f64,
) -> (f64, f64) {
    match workarea {
        Some((x, y, ww, wh)) => (
            x + ww - w - 24.0,
            y + wh - PET_OVERLAY_H - 24.0 - h - 12.0,
        ),
        None => (640.0, 120.0),
    }
}

/// 创建 / 显示任务悬浮球（幂等：已存在则 show + 置前）。
#[tauri::command]
fn task_overlay_show(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    use tauri::WebviewWindowBuilder;

    if let Some(win) = app.get_webview_window(TASK_OVERLAY_LABEL) {
        let _ = win.show();
        let _ = win.set_always_on_top(true);
        return Ok(());
    }

    let work = match app.primary_monitor() {
        Ok(Some(monitor)) => {
            let a = monitor.work_area();
            Some((
                a.position.x as f64,
                a.position.y as f64,
                a.size.width as f64,
                a.size.height as f64,
            ))
        }
        _ => None,
    };
    let (x, y) = task_overlay_default_position(work, TASK_OVERLAY_W, TASK_OVERLAY_H);

    let _win = WebviewWindowBuilder::new(&app, TASK_OVERLAY_LABEL, tauri::WebviewUrl::External(task_overlay_url().parse().map_err(|e| format!("非法任务悬浮球 URL: {e}"))?))
        .title("Marguerite Tasks")
        .inner_size(TASK_OVERLAY_W, TASK_OVERLAY_H)
        .position(x, y)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .visible(true)
        .skip_taskbar(true)
        .build()
        .map_err(|e| format!("任务悬浮球创建失败: {e}"))?;

    Ok(())
}

/// 隐藏任务悬浮球（存在则 hide，不存在静默跳过，保持幂等）。
#[tauri::command]
fn task_overlay_hide(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    if let Some(win) = app.get_webview_window(TASK_OVERLAY_LABEL) {
        let _ = win.hide();
    }
    Ok(())
}

/// 切换显示/隐藏（托盘菜单快捷入口）。
#[tauri::command]
fn task_overlay_toggle(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    match app.get_webview_window(TASK_OVERLAY_LABEL) {
        Some(win) if win.is_visible().unwrap_or(false) => {
            let _ = win.hide();
            Ok(())
        }
        _ => task_overlay_show(app),
    }
}

/// 任务悬浮球：拖拽到新屏幕坐标（x,y 为窗口左上角，由前端上报）。
#[tauri::command]
fn task_overlay_move(app: tauri::AppHandle, x: f64, y: f64) -> Result<(), String> {
    use tauri::Manager;
    if let Some(win) = app.get_webview_window(TASK_OVERLAY_LABEL) {
        let _ = win.set_position(tauri::LogicalPosition { x, y });
    }
    Ok(())
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
        // GUI 的「添加接入点」表单只收**密钥本体**，不收集环境变量名，故这里显式留空。
        // 留空 = 该档案不声明 env 来源，运行期只用上面的加密密钥；
        // 设置页里选「使用环境变量」走的是 `llmProfile.activate` 的另一条路径。
        api_key_env: String::new(),
        model: model.trim().to_string(),
        kind: match kind.as_str() {
            "local" => ProfileKind::Local,
            "custom" => ProfileKind::Custom,
            _ => ProfileKind::Cloud,
        },
        builtin: false,
        temperature: 0.7,
        max_tokens: 4096,
        tier: marguerite_core::model::tier::ModelTier::Balanced,
        tier_caps: marguerite_core::model::tier::TierCaps::default(),
        // GUI 里手工添加的档案**没有**声明过档位能力（caps 是 Default 填充的），
        // 故必须显式给 false：路由准入据此区分「未声明（信任档位、标记未验证）」
        // 与「已声明（用 caps 严格卡线）」。若给 true，会把默认均衡档当成作者的
        // 严格声明，把该档案错误地卡在旗舰候选链之外。
        tier_caps_declared: false,
        // GUI「添加接入点」表单没有多模态勾选项：显式写入 `Default`（全 false + declared=false），
        // 语义是「用户未声明」→ 运行期按 provider/model 名做保守自动识别（见 registry::recognize）；
        // 若这里省略字段，`ModelProfile` 的构造会直接编译失败（真机 2026-10-23 构建阻断）。
        capabilities: marguerite_core::model::registry::ModelCapabilities::default(),
        // GUI「添加接入点」表单暂不提供协议选择，默认 chat。
        // 需要 responses 的用户走设置页 / `marguerite model add --wire-api responses`，
        // 那两条路径都会把这个字段写进同一个 models.json。
        wire_api: marguerite_core::model::wire::WireApi::Chat,
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
        // 协议随 `config.yaml` 的 `[model] wire_api`；缺省 Chat。
        wire_api: m.wire_api,
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

/// 轻量运行时状态（供前端首次加载时判断是否需要引导配置）
#[tauri::command]
fn runtime_status() -> Result<serde_json::Value, String> {
    let config = Config::load().unwrap_or_default();
    let key_ok = !config.model.api_key.is_empty();
    let config_exists = Config::config_path().exists();
    Ok(serde_json::json!({
        "api_key_configured": key_ok,
        "model_provider": format!("{:?}", config.model.provider),
        "model_name": config.model.model_name,
        "first_run": !config_exists,
        "gateway_ready": true,
    }))
}

/// 系统诊断（对齐 `marguerite doctor`）
#[tauri::command]
fn doctor_check(state: State<'_, Arc<AppState>>) -> DoctorReportView {
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

    // ── P0 S3：插件真源组（让「声明了但没实现」在 doctor 里可见）──
    let manifests = snapshot_plugin_manifests(&state);
    let declarative = manifests.iter().filter(|m| m.is_declarative_only()).count();
    let native = manifests.len() - declarative;
    let manifest_diags: Vec<marguerite_core::plugins::ManifestDiagnostic> =
        manifests.iter().flat_map(|m| m.validate()).collect();
    let manifest_ok = !manifests.is_empty() && manifest_diags.is_empty();
    checks.push(DoctorCheckView {
        name: "plugin.manifest".into(),
        // detail 口径与 S3 测试/UI 一致：先报「N 份中多少 declarative-only」，再报诊断。
        detail: format!(
            "{} 份清单：{} declarative-only / {} native；validate 诊断 {} 条",
            manifests.len(),
            declarative,
            native,
            manifest_diags.len()
        ),
        ok: manifest_ok,
        hint: if manifests.is_empty() {
            "未装载到任何 plugin manifest（plugins/manifests 目录缺失？）".into()
        } else if manifest_diags.is_empty() {
            format!(
                "{declarative} 份清单均为纯声明式（无 entry / permissions），原生实现尚未接入"
            )
        } else {
            manifest_diags
                .iter()
                .map(|d| format!("[{}] {}: {}", d.code, d.manifest, d.message))
                .collect::<Vec<_>>()
                .join("; ")
        },
    });

    // patch 层摘要（S2 只读装载结果）
    let patch_summary = state
        .plugin_patch_summary
        .lock()
        .ok()
        .and_then(|g| g.clone());
    let patch_diags = patch_summary
        .as_ref()
        .map(|s| s.diagnostics.len())
        .unwrap_or(0);
    checks.push(DoctorCheckView {
        name: "plugin.patch".into(),
        ok: patch_diags == 0,
        detail: match &patch_summary {
            Some(s) => format!("{} 条 patch；诊断 {} 条", s.total, s.diagnostics.len()),
            None => "未装载（未找到 plugins/patches）".into(),
        },
        hint: match (&patch_summary, patch_diags) {
            (None, _) => "patch 层未装载（plugins/patches 目录缺失或未启用）".into(),
            (Some(_), 0) => "patch 层装载无诊断".into(),
            (Some(s), _) => s
                .diagnostics
                .iter()
                .map(|d| format!("[{}] {}: {}", d.code, d.target, d.message))
                .collect::<Vec<_>>()
                .join("; "),
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
#[allow(clippy::too_many_arguments)] // 输入动作参数天然较多，Tauri 命令签名直传
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

// ───────────── 自动化 / 定时任务（automation 面板的真实后端）─────────────
//
// 为什么需要这一组（2026-10-21 审计）：桌面端 nav 里的「自动化」此前是
// `PlaceholderPanel`（"该模块正在开发中"），而 core 早已有完整的
// `crate::schedule::ScheduleStore`（Reminder CRUD + 持久化到 SQLite），
// WebUI 也有整套 `cron.*` RPC + CronView。也就是说**能力早就存在，只是桌面端没接**。
//
// 与网关的分工遵循既有约定：Tauri 做 dispatch + 序列化，`ScheduleStore` 持有状态。
// 这里刻意**复用同一个 store**（`open_default`），而不是另建一张表——
// 否则桌面端建的任务 WebUI/CLI 看不到，形成又一处"双配置源"。

/// 列表：全部定时任务（不按会话过滤；桌面端是单用户视角，需要看到全部）。
#[tauri::command]
fn cron_list() -> Result<serde_json::Value, String> {
    let store = marguerite_core::schedule::ScheduleStore::open_default()?;
    let items: Vec<serde_json::Value> = store
        .all()
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "session": r.session,
                "message": r.message,
                "trigger": trigger_to_json(&r.trigger),
                "created_at_ms": r.created_at_ms,
                "next_fire_ms": r.next_fire_ms,
                "fired": r.fired,
            })
        })
        .collect();
    Ok(serde_json::json!({ "items": items }))
}

fn trigger_to_json(t: &marguerite_core::schedule::ReminderTrigger) -> serde_json::Value {
    use marguerite_core::schedule::ReminderTrigger;
    match t {
        ReminderTrigger::After { seconds } => {
            serde_json::json!({ "kind": "after", "value": seconds })
        }
        ReminderTrigger::At { ts_ms } => serde_json::json!({ "kind": "at", "value": ts_ms }),
        ReminderTrigger::Every { interval_seconds } => {
            serde_json::json!({ "kind": "every", "value": interval_seconds })
        }
    }
}

/// 把前端传来的 `{kind, value}` 解析成触发方式。
///
/// 校验放在这里而不是只信前端：`every` 有 ≥300 秒下限（防误填 1 秒把库打爆），
/// `at` 必须是未来时刻。错误原样回传，不吞成"创建成功"。
fn parse_trigger(kind: &str, value: i64) -> Result<marguerite_core::schedule::ReminderTrigger, String> {
    use marguerite_core::schedule::ReminderTrigger;
    match kind {
        "after" => {
            if value <= 0 {
                return Err("延迟秒数必须大于 0".into());
            }
            Ok(ReminderTrigger::After {
                seconds: value as u64,
            })
        }
        "at" => Ok(ReminderTrigger::At { ts_ms: value }),
        "every" => {
            if value < 300 {
                return Err("间隔不得小于 300 秒（防误填导致频繁触发）".into());
            }
            Ok(ReminderTrigger::Every {
                interval_seconds: value as u64,
            })
        }
        other => Err(format!("未知触发类型：{other}（可选 after / at / every）")),
    }
}

/// 当前毫秒时间戳。
///
/// 刻意用 `std::time` 而不是引入 `chrono`：gui crate 没有 chrono 依赖，
/// 为一个时间戳加依赖不值得。时钟回拨时返回 0（而非 panic），
/// `ScheduleStore::register` 会据此报 `NotFuture`，错误可见。
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 新建定时任务。
#[tauri::command]
fn cron_create(
    session: String,
    message: String,
    kind: String,
    value: i64,
) -> Result<serde_json::Value, String> {
    let trigger = parse_trigger(&kind, value)?;
    let mut store = marguerite_core::schedule::ScheduleStore::open_default()?;
    let now = now_ms();
    let r = store
        .register(&session, &message, trigger, now)
        .map_err(|e| format!("{e:?}"))?;
    Ok(serde_json::json!({
        "ok": true,
        "item": {
            "id": r.id,
            "session": r.session,
            "message": r.message,
            "trigger": trigger_to_json(&r.trigger),
            "created_at_ms": r.created_at_ms,
            "next_fire_ms": r.next_fire_ms,
            "fired": r.fired,
        }
    }))
}

/// 删除定时任务。返回是否真的删掉了（id 不存在 → false，不谎报成功）。
#[tauri::command]
fn cron_remove(id: String) -> Result<serde_json::Value, String> {
    let mut store = marguerite_core::schedule::ScheduleStore::open_default()?;
    let removed = store.cancel(&id);
    Ok(serde_json::json!({ "ok": removed, "id": id }))
}

/// 立即触发一次（不等计划时刻）。
///
/// 语义选择：**一次性任务触发后即删除**（after/at 的语义就是"到点提醒一次"），
/// 周期任务（every）只推进 `next_fire_ms` 而保留。这样"立即运行"不会在列表里
/// 留下一条永远不会再触发的死任务。
#[tauri::command]
fn cron_run(id: String) -> Result<serde_json::Value, String> {
    use marguerite_core::schedule::ReminderTrigger;
    let mut store = marguerite_core::schedule::ScheduleStore::open_default()?;
    let target = store.all().into_iter().find(|r| r.id == id);
    let Some(r) = target else {
        return Err(format!("未找到任务：{id}"));
    };
    let message = r.message.clone();
    let recurring = matches!(r.trigger, ReminderTrigger::Every { .. });
    if !recurring {
        store.cancel(&id);
    }
    Ok(serde_json::json!({
        "ok": true,
        "id": id,
        "message": message,
        "recurring": recurring,
        // 桌面端据此把提醒作为系统消息投进当前会话，而不是"静默成功"
        "delivered": true,
    }))
}

// ───────────── 回合结束通知（turn-ended）─────────────
//
// 为什么需要：长任务（深度推理、批量执行）动辄几分钟，用户会切去干别的，
// 回来才发现早跑完了 —— 或者更糟，一直等着。ChatGPT / Codex 桌面端都有这条。
//
// 为什么放在 Rust 侧发、而不是 webui 里直接调插件 JS：
//   webui 是纯浏览器包，**没有** `@tauri-apps/api`（静态 import 会让 vite build 失败，
//   见 `webui/src/utils/tauriBridge.ts` 的注释）。它只能用 `__TAURI_INTERNALS__.invoke`
//   调 command。所以这里提供一个 command，把通知能力封装在壳内。
//
// 为什么不在前端用 Web Notification API：WebView2 / WKWebView 上行为不一致，
// 窗口最小化时可能静默失败，且拿不到系统通知中心的历史。
#[tauri::command]
fn notify_turn_ended(
    app: tauri::AppHandle,
    title: String,
    body: String,
) -> Result<bool, String> {
    use tauri_plugin_notification::NotificationExt;
    // 空标题的通知在 Windows 上会被静默丢弃；显式拒绝，让前端知道没发出去。
    if title.trim().is_empty() {
        return Err("通知标题不能为空".into());
    }
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| format!("通知发送失败：{e}"))?;
    Ok(true)
}

// ───────────── 三档模型分层 + 会话预算（四端同源展示）─────────────

/// 档位决策 + 预算台账 + 各档候选链。
///
/// 与 CLI `marguerite model tier` / TUI 状态栏 / WebUI `model.tier.get` 同源，
/// 都读 `SessionState`；桌面端不能再自算一套，否则四端数字会对不上。
#[tauri::command]
fn model_tier_get() -> Result<serde_json::Value, String> {
    use marguerite_core::model::tier::{ModelTier, SessionState, TierRequest, TierRules};
    let s = SessionState::load();
    let rules = TierRules::from_embedded();
    // 预算已用/剩余取 `usage` 台账实测：会话态里的 used 字段全仓无人写入（恒 0），
    // 直接读会给出「剩余 100%」假读数（与 WebUI/CLI/TUI 的诚实性要求一致）。
    let snap = marguerite_core::usage::tightest_budget_snapshot();
    let budget = match &snap {
        Some(b) => s.budget.clone().with_measured_snapshot(
            b.tokens_cap,
            b.usd_cap,
            b.tokens_used,
            b.usd_used,
        ),
        None => s.budget.clone(),
    };
    let decision = s.resolve_tier_with(TierRequest::default(), snap.as_ref().map(|b| b.remaining_ratio));
    Ok(serde_json::json!({
        "locked_tier": s.locked_tier.map(|t| t.as_str()),
        "thinking_effort": s.thinking_effort.reasoning_effort(),
        "effective_tier": decision.tier.as_str(),
        "tier_reasons": decision.why,
        "budget": {
            "tokens_cap": budget.tokens_cap,
            "usd_cap": budget.usd_cap,
            "tokens_used": budget.tokens_used,
            "usd_used": budget.usd_used,
            "remaining_ratio": budget.remaining_ratio(),
            "state": budget.state().as_str(),
            "budget_source": if snap.is_some() { "usage_ledger" } else { "session_state" },
        },
        "tiers": {
            "flagship": rules.chain_for(ModelTier::Flagship),
            "balanced": rules.chain_for(ModelTier::Balanced),
            "turbo": rules.chain_for(ModelTier::Turbo),
        }
    }))
}

/// 锁定档位（auto/none = 解除锁定）。返回最新档位，避免前端再读一次。
#[tauri::command]
fn model_tier_set(tier: String) -> Result<serde_json::Value, String> {
    use marguerite_core::model::tier::{ModelTier, SessionState};
    let mut s = SessionState::load();
    s.locked_tier = match tier.trim() {
        "" | "auto" | "none" => None,
        other => Some(ModelTier::parse(other)),
    };
    s.save().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "ok": true,
        "locked_tier": s.locked_tier.map(|t| t.as_str()),
        "effective_tier": s.effective_tier().as_str(),
    }))
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
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    std::fs::write(&path, content).map_err(|e| format!("写入失败: {e}"))
}

/// 创建目录（含父目录），支持中文路径
#[tauri::command]
fn fs_mkdir(path: String) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("路径不可为空".into());
    }
    std::fs::create_dir_all(&path).map_err(|e| format!("创建目录失败: {e}"))
}

/// 删除文件或目录（目录递归删除）
#[tauri::command]
fn fs_delete(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err("路径不存在".into());
    }
    if p.is_dir() {
        std::fs::remove_dir_all(p).map_err(|e| format!("删除目录失败: {e}"))
    } else {
        std::fs::remove_file(p).map_err(|e| format!("删除文件失败: {e}"))
    }
}

/// 在系统默认浏览器打开链接（仅 http/https）
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("仅支持 http/https 链接".into());
    }
    #[cfg(target_os = "windows")]
    {
        use marguerite_core::proc::SilentCommand;
        std::process::Command::new("cmd")
            .silent()
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
struct PluginDiagnosticView {
    /// 机器可读诊断码，如 `manifest.hot_without_entry`
    code: String,
    /// 产生该诊断的清单 name
    manifest: String,
    /// 人类可读说明
    message: String,
}

/// 插件槽视图（P0 S3）。
///
/// ⚠️ **向后兼容硬约束**：`name` / `kind` / `description` / `version` 四个字段是
/// 既有契约（`gui/src/panels/PluginsPanel.tsx` 在消费），**只增不改**；`mode` /
/// `entry` / `activation` / `permissions` / `diagnostics` 为 S3 纯增量字段。
#[derive(serde::Serialize)]
struct PluginSlotView {
    // ── 既有字段（名与类型保持不变）──
    name: String,
    kind: String,
    description: String,
    version: String,
    // ── P0 S3 新增字段 ──
    /// `"declarative-only"`（纯声明式，无原生实现）| `"native"`（声明了 entry 或权限）
    mode: String,
    /// 原生入口（相对 manifest 目录），无则 None
    entry: Option<String>,
    /// `"restart"` | `"hot"`
    activation: String,
    /// 已声明权限条数
    permissions: usize,
    /// 清单自洽性诊断（`PluginManifest::validate()`）
    diagnostics: Vec<PluginDiagnosticView>,
}

/// 插件清单列表（P0 S3：读**磁盘真源** `plugins/manifests/*.plugin.json`）。
///
/// 取代旧实现「从 `domain::capabilities::registry()` 假投影」——旧写法从不读磁盘清单，
/// 导致「装了不动」在 UI 上完全不可见。现按真源输出，并把 `validate()` 诊断一并带出，
/// 使「声明了但没实现」在同一张卡片上可见。
#[tauri::command]
fn plugins_list(state: State<'_, Arc<AppState>>) -> Vec<PluginSlotView> {
    use marguerite_core::plugins::Activation;

    snapshot_plugin_manifests(&state)
        .into_iter()
        .map(|m| {
            let diagnostics: Vec<PluginDiagnosticView> = m
                .validate()
                .into_iter()
                .map(|d| PluginDiagnosticView {
                    code: d.code.to_string(),
                    manifest: d.manifest,
                    message: d.message,
                })
                .collect();
            PluginSlotView {
                name: m.name.clone(),
                kind: "plugin".to_string(),
                description: m.description.clone(),
                version: m.version.clone(),
                mode: if m.is_declarative_only() {
                    "declarative-only".to_string()
                } else {
                    "native".to_string()
                },
                entry: m.entry.clone(),
                activation: match m.activation {
                    Activation::Restart => "restart".to_string(),
                    Activation::Hot => "hot".to_string(),
                },
                permissions: m.permissions.len(),
                diagnostics,
            }
        })
        .collect()
}

/// 构建系统托盘与右键原生菜单
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示工作台", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "隐藏到托盘", true, None::<&str>)?;
    // V2.1-P2：桌宠悬浮窗入口，缺省为可见状态（启动后未创建时点击会创建）。
    let pet_toggle = MenuItem::with_id(app, "pet_toggle", "桌宠悬浮窗 · 切换显示", true, Some("CmdOrCtrl+Shift+P"))?;
    let pet_hide = MenuItem::with_id(app, "pet_hide", "桌宠悬浮窗 · 隐藏", true, None::<&str>)?;
    // P0-4：任务悬浮球入口（与桌宠同构，缺省未创建时点击会创建）。
    let task_toggle = MenuItem::with_id(app, "task_toggle", "任务悬浮球 · 切换显示", true, None::<&str>)?;
    let task_hide = MenuItem::with_id(app, "task_hide", "任务悬浮球 · 隐藏", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 Marguerite", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &show,
            &hide,
            &PredefinedMenuItem::separator(app)?,
            &pet_toggle,
            &pet_hide,
            &task_toggle,
            &task_hide,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
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
            "pet_toggle" => {
                let _ = pet_overlay_toggle(app.clone());
            }
            "pet_hide" => {
                let _ = pet_overlay_hide(app.clone());
            }
            "task_toggle" => {
                let _ = task_overlay_toggle(app.clone());
            }
            "task_hide" => {
                let _ = task_overlay_hide(app.clone());
            }
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

/// 插件清单目录候选（`MARGUERITE_PLUGINS_DIR` > cwd > exe 同目录/上溯 > 安装资源布局）。
///
/// 与 core 的 `gateway::market::plugin_manifest_dir()` 同构 —— 该函数是 `pub(crate)`，
/// 跨 crate 无法复用，故在此复刻候选项与「目录必须存在」的口径。
fn plugin_manifest_dir_local() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("MARGUERITE_PLUGINS_DIR") {
        if !p.trim().is_empty() {
            let path = std::path::PathBuf::from(p);
            if path.is_dir() {
                return Some(path);
            }
        }
    }
    let cwd = std::env::current_dir().ok()?;
    let exe = std::env::current_exe().ok()?;
    let mut candidates = vec![
        cwd.join("plugins").join("manifests"),
        exe.parent()?.join("plugins").join("manifests"),
        exe.parent()?.parent()?.join("plugins").join("manifests"),
        exe.parent()?
            .join("resources")
            .join("plugins")
            .join("manifests"),
    ];
    candidates.retain(|p| p.is_dir());
    candidates.into_iter().next()
}

/// P0 S3：按需回读磁盘 `plugins/manifests/*.plugin.json` 真源。
///
/// 与 setup 内 S2 只读装载块同口径（读失败/解析失败**按文件告警后跳过**，不让启动失败），
/// 但按**文件路径排序**以保证顺序确定。仅在 `AppState` 快照为空时被
/// `snapshot_plugin_manifests()` 调用（如 setup 尚未跑、或清单目录当时不存在）。
fn load_plugin_manifests_local() -> Vec<marguerite_core::plugins::manifest::PluginManifest> {
    use marguerite_core::plugins::manifest::PluginManifest;

    let Some(dir) = plugin_manifest_dir_local() else {
        eprintln!("[plugin] 未找到 plugins/manifests 目录（MARGUERITE_PLUGINS_DIR / cwd / exe 上溯）");
        return Vec::new();
    };
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) => {
            eprintln!("[plugin] 清单目录 {} 不可读: {e}", dir.display());
            return Vec::new();
        }
    };

    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        let is_manifest = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(".plugin.json"))
            .unwrap_or(false);
        if is_manifest {
            paths.push(path);
        }
    }
    // read_dir 顺序由文件系统决定；排序后与平台无关，且 plugins_list 输出稳定。
    paths.sort();

    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        match std::fs::read_to_string(&path) {
            Ok(text) => match PluginManifest::from_json(&text) {
                Ok(m) => out.push(m),
                Err(e) => eprintln!("[plugin] 清单 {} 解析失败，已跳过: {e}", path.display()),
            },
            Err(e) => eprintln!("[plugin] 清单 {} 读取失败，已跳过: {e}", path.display()),
        }
    }
    out
}

/// 取插件真源快照：优先 `AppState`（setup 装入），快照为空时按需回读磁盘。
///
/// **不回写** `AppState`，避免在只读消费路径上引入写锁竞争。
fn snapshot_plugin_manifests(
    state: &State<'_, Arc<AppState>>,
) -> Vec<marguerite_core::plugins::manifest::PluginManifest> {
    if let Ok(guard) = state.plugin_manifests.lock() {
        if !guard.is_empty() {
            return guard.clone();
        }
    }
    load_plugin_manifests_local()
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
    webui_dir_candidates(app)
        .into_iter()
        .find(|p| p.join("index.html").is_file())
}

/// 可执行文件所在目录（Windows 上必须用 `current_exe()` 兜底）。
///
/// `tauri::path::PathResolver::executable_dir()` 在 Tauri 2.11 的文档里明确写着
/// **Windows: Not supported**（内部 `dirs::executable_dir()` 在 Windows 返回 `None`
/// → `Err(UnknownPath)`）。因此原 `if let Ok(exe) = app.path().executable_dir()`
/// 这条腿在 Windows 上**恒不成立**，「exe 同目录 + 上溯 6 级」是死代码——实测裸 exe
/// 只探测到 resource_dir 与 cwd 两个候选。此处回退到 `std::env::current_exe()`，
/// 让该腿真正生效（双击 `target\release\marguerite-gui.exe` → 上溯找到仓库 `webui/dist`）。
fn exe_dir(app: &tauri::App) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    if let Ok(dir) = app.path().executable_dir() {
        return Some(dir);
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

/// `resolve_webui_dir` 的全部候选路径（按探测顺序）。
///
/// 单独抽出供**诊断输出**使用：产物缺失时必须能让用户看到「到底找过哪些路径」，
/// 而不是静默白屏（P0-D 缺陷 1：裸 exe 出仓库 → `/control/` 404 白屏且日志谎报就绪）。
fn webui_dir_candidates(app: &tauri::App) -> Vec<std::path::PathBuf> {
    use tauri::Manager;
    let mut candidates = Vec::new();
    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join("webui/dist"));
    }
    if let Some(exe) = exe_dir(app) {
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
        handle.dialog().message(msg).title(title).show(move |_| {});
    });
}

// ---------------------------------------------------------------------------
// 内嵌网关「界面就绪」判定（P0-D 缺陷 1）
//
// 历史缺陷：就绪判定只做一次 TCP `connect_timeout`——网关在监听即打印
// 「内嵌网关已在 127.0.0.1:18790 就绪」，但 `/control/` 实际返回 404
// （裸 exe 复制到仓库外、`webui/dist` 缺失时），窗口全白且日志谎报成功。
// 现在改为真实 `GET /control/` 并要求 HTTP 200 + 非兜底诊断页。
// ---------------------------------------------------------------------------

/// `/control/` 探测结论
#[derive(Debug, Clone, PartialEq, Eq)]
enum ControlProbe {
    /// HTTP 200 且返回真实控制台入口（非兜底诊断页）
    Ready,
    /// 网关在监听，但返回兜底诊断页（core 侧 `webui/dist` 缺失）
    FallbackPage { status: u16 },
    /// 其它 HTTP 状态码（期望 200）
    HttpStatus { status: u16 },
    /// 连接 / 读取失败
    Unreachable { reason: String },
}

/// 从原始 HTTP 响应判定 `/control/` 是否真的可用（纯函数，便于单测）。
///
/// 规则：
/// - `200` 且**不含**兜底诊断页标记 → [`ControlProbe::Ready`]；
/// - 含标记（core 的产物缺失诊断页，固定 503）→ [`ControlProbe::FallbackPage`]；
/// - 无合法状态行 → [`ControlProbe::Unreachable`]；
/// - 其它状态码 → [`ControlProbe::HttpStatus`]（同样视为未就绪）。
fn classify_control_response(raw: &str) -> ControlProbe {
    let status = raw
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    let is_fallback = raw.contains(marguerite_core::gateway::server::WEBUI_MISSING_MARKER);
    match (status, is_fallback) {
        (200, false) => ControlProbe::Ready,
        (_, true) => ControlProbe::FallbackPage { status },
        (0, false) => ControlProbe::Unreachable {
            reason: "响应缺少合法状态行".to_string(),
        },
        _ => ControlProbe::HttpStatus { status },
    }
}

/// 对 `127.0.0.1:{DEFAULT_PORT}/control/` 发一次真实 HTTP/1.1 GET（手写请求，
/// 不引入新依赖、不走任何第三方 HTTP 客户端），返回原始响应文本。
fn fetch_control_page(timeout: std::time::Duration) -> Result<String, String> {
    use std::io::{Read, Write};
    let addr: std::net::SocketAddr = ([127u8, 0, 0, 1], DEFAULT_PORT).into();
    let mut stream = std::net::TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| format!("连接 127.0.0.1:{DEFAULT_PORT} 失败: {e}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| format!("设置读超时失败: {e}"))?;
    let _ = stream.set_write_timeout(Some(timeout));
    stream
        .write_all(
            b"GET /control/ HTTP/1.1\r\nHost: 127.0.0.1\r\nUser-Agent: marguerite-gui-readiness\r\nConnection: close\r\n\r\n",
        )
        .map_err(|e| format!("发送 GET /control/ 失败: {e}"))?;
    let _ = stream.flush();

    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > 512 * 1024 {
                    break;
                }
            }
            // 读超时：已读到的部分照常判定（不因末尾无 FIN 而丢弃整段响应）
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                break
            }
            Err(e) => return Err(format!("读取 /control/ 响应失败: {e}")),
        }
    }
    if buf.is_empty() {
        return Err("连接被立即关闭（无任何响应字节）".to_string());
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// 真实探测一次 `/control/` 可用性。
fn probe_control_page(timeout: std::time::Duration) -> ControlProbe {
    match fetch_control_page(timeout) {
        Ok(raw) => classify_control_response(&raw),
        Err(reason) => ControlProbe::Unreachable { reason },
    }
}

// ---------------------------------------------------------------------------
// 端口占用者判定（R14）：区分「本产品网关」与「别的程序」
// ---------------------------------------------------------------------------
//
// 历史缺陷：端口被占 → 一律弹「Marguerite 已在运行」并 exit(1)。实测本机一个
// 残留 `marguerite-core.exe` 占着 18790 时，桌面端因此**完全无法启动**，
// 而桌面上并没有任何窗口 —— 用户被告知了一个不成立的事实，也没有恢复路径。
//
// 判定依据用 `/api/health` 的**结构化字段**（`engine == "rust"`），而不是
// 「TCP 能连上」：后者任何监听进程都满足，等于没判。

/// 端口占用者分类
#[derive(Debug, Clone, PartialEq, Eq)]
enum PortOwner {
    /// 端口空闲，可以自起网关
    Free,
    /// 本产品的 Rust 网关已在跑（可附着复用）
    OurGateway { version: String },
    /// 别的程序占着（不可附着）
    Foreign,
}

/// 从 `/api/health` 响应体判定占用者身份。
///
/// 纯函数，便于单测：输入是原始 HTTP 响应（含状态行），输出分类。
/// 判定必须同时满足 `status == "ok"` 与 `engine == "rust"`——只认其中一个
/// 会把恰好返回 `{"status":"ok"}` 的第三方服务误认成本产品网关。
fn classify_port_owner(raw: &str) -> PortOwner {
    let Some(body_start) = raw.find("\r\n\r\n").map(|i| i + 4) else {
        return PortOwner::Foreign;
    };
    // 状态行必须是 200；别的状态码说明该端口上不是健康的本产品网关
    if !raw.starts_with("HTTP/1.1 200") && !raw.starts_with("HTTP/1.0 200") {
        return PortOwner::Foreign;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw[body_start..]) else {
        return PortOwner::Foreign;
    };
    if v.get("engine").and_then(|x| x.as_str()) != Some("rust")
        || v.get("status").and_then(|x| x.as_str()) != Some("ok")
    {
        return PortOwner::Foreign;
    }
    PortOwner::OurGateway {
        version: v
            .get("version")
            .and_then(|x| x.as_str())
            .unwrap_or("unknown")
            .to_string(),
    }
}

/// 探测 18790 端口当前归属（Free / 本产品网关 / 别的程序）。
fn probe_port_owner() -> PortOwner {
    // 先试 bind：成功即空闲（guard 立刻释放，交给网关线程去绑）。
    match std::net::TcpListener::bind(std::net::SocketAddr::from(([127u8, 0, 0, 1], DEFAULT_PORT))) {
        Ok(_guard) => return PortOwner::Free,
        Err(e)
            if e.kind() != std::io::ErrorKind::AddrInUse
                && e.kind() != std::io::ErrorKind::PermissionDenied =>
        {
            // 不是「被占用」而是别的绑定失败（如权限）：同样按不可用处理，
            // 交给后续 alert_fatal 报准确原因，而不是在这里猜。
            return PortOwner::Foreign;
        }
        Err(_) => {}
    }
    match fetch_health_probe(std::time::Duration::from_millis(1200)) {
        Some(raw) => classify_port_owner(&raw),
        None => PortOwner::Foreign,
    }
}

/// `GET /api/health` 原始响应（供端口归属判定）。失败返回 None。
fn fetch_health_probe(timeout: std::time::Duration) -> Option<String> {
    use std::io::{Read, Write};
    let addr = std::net::SocketAddr::from(([127u8, 0, 0, 1], DEFAULT_PORT));
    let mut stream = std::net::TcpStream::connect_timeout(&addr, timeout).ok()?;
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    stream
        .write_all(
            format!(
                "GET /api/health HTTP/1.1\r\nHost: 127.0.0.1:{DEFAULT_PORT}\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > 64 * 1024 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    (!buf.is_empty()).then(|| String::from_utf8_lossy(&buf).into_owned())
}

/// 冷启动超时后的**后台续探 + 自动导航**（R13，2026-10-18）。
///
/// 为什么需要：首轮就绪等待有截止时间（20s）。一旦超时，窗口停在 core 的
/// `/control/` 诊断页（503）或 WebView 的连接错误页，而**原实现没有任何恢复
/// 路径**——用户只能关掉重开，重开时网关早已就绪，于是表现为「重启一下就好了」
/// 这种最难复现、也最难解释的缺陷。
///
/// 本函数起一个 detached 线程，最多再等 [`LATE_READY_BUDGET`]；一旦 `/control/`
/// 真的就绪，就把主窗口导航回控制台。三项设计约束：
/// 1. **不阻塞启动**：全程后台，主线程该干嘛干嘛；
/// 2. **不重复建窗**：只对既有 `main` 窗口做 `navigate`，不新开窗口、不改变
///    用户已手动导航到别处之外的会话状态（`navigate` 仅重载控制台入口）；
/// 3. **失败静默**：再次超时只记一行 stderr，不再弹第二个对话框（避免叠加骚扰）。
fn spawn_late_readiness_recovery(app: tauri::AppHandle) {
    /// 首轮 20s 之后，再给 90s（覆盖首次运行解压 pyengine / 杀软全盘扫描）。
    const LATE_READY_BUDGET: std::time::Duration = std::time::Duration::from_secs(90);
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + LATE_READY_BUDGET;
        while std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(500));
            if matches!(
                probe_control_page(std::time::Duration::from_millis(1200)),
                ControlProbe::Ready
            ) {
                let url = control_url();
                if let Some(win) = app.get_webview_window("main") {
                    // `navigate` 同时覆盖「诊断页」与「WebView 连接错误页」两种状态
                    // （后者是浏览器内部页，`location.reload()` 无法从 Rust 侧触发）。
                    match win.navigate(url.parse().unwrap_or_else(|_| {
                        unreachable!("control_url 为编译期固定回环地址")
                    })) {
                        Ok(()) => eprintln!("[gateway] 延迟就绪：已自动把主窗口导航回控制台"),
                        Err(e) => eprintln!("[gateway] 延迟就绪但导航失败：{e}"),
                    }
                }
                return;
            }
        }
        eprintln!(
            "[gateway] 延迟续探 {}s 仍未就绪，停止自动恢复（请查看上方诊断按提示修复）",
            LATE_READY_BUDGET.as_secs()
        );
    });
}

/// 控制台入口地址（单一定义，供窗口配置校验与延迟导航共用）。
fn control_url() -> String {
    format!("http://localhost:{DEFAULT_PORT}/control/")
}

/// 把探测结论转成「可执行的诊断」（摘要 + 修复动作），供 stderr 与原生对话框共用。
///
/// 纯函数：`webui_missing` 为 `resolve_webui_dir` 的结果，`candidates` 为探测过的
/// 全部候选路径——用户据此能自己把产物放对位置，而不是面对白屏猜。
fn diagnose_control_failure(
    probe: &ControlProbe,
    webui_missing: bool,
    candidates: &[std::path::PathBuf],
) -> (String, String) {
    let summary = match probe {
        ControlProbe::Ready => format!(
            "内嵌网关已在 127.0.0.1:{DEFAULT_PORT} 就绪（GET /control/ → HTTP 200）"
        ),
        ControlProbe::FallbackPage { status } => format!(
            "界面未就绪：GET /control/ 返回 HTTP {status}（兜底诊断页 = WebUI 产物缺失），窗口显示的是诊断页而非控制台"
        ),
        ControlProbe::HttpStatus { status } => format!(
            "界面未就绪：GET /control/ 返回 HTTP {status}（期望 200），窗口可能是错误页"
        ),
        ControlProbe::Unreachable { reason } => format!(
            "界面未就绪：无法访问 http://127.0.0.1:{DEFAULT_PORT}/control/（{reason}）"
        ),
    };

    let mut hint = String::new();
    let artifacts_missing =
        webui_missing || matches!(probe, ControlProbe::FallbackPage { .. });
    if artifacts_missing {
        hint.push_str(&format!(
            "未找到 webui/dist（已探测 {} 个候选路径，均无 index.html）：",
            candidates.len()
        ));
        for c in candidates {
            hint.push_str(&format!("\n  - {}", c.display()));
        }
        hint.push_str(&format!(
            "\n修复（任选其一）：1) 把 webui/dist 放到 exe 同级目录；2) 设置环境变量 MARGUERITE_WEBUI_DIST=<绝对路径>；3) 在源码目录运行 npm run build（webui/）后重启本程序。\n窗口内已显示同一份诊断页（HTTP {DEFAULT_PORT}/control/ 返回 503 诊断页，非空白页）。"
        ));
    } else {
        hint.push_str(&format!(
            "网关进程可能未起来或端口被占用：确认 127.0.0.1:{DEFAULT_PORT} 无其它 Marguerite 实例占用，再重启本程序。"
        ));
    }
    (summary, hint)
}

// ───────────── 桌宠悬浮窗的运行时 ACL 能力（2026-10-25）─────────────
//
// 真机缺陷：「桌宠拖不动、拖不出窗体」。
//
// 根因不在前端坐标换算，而在**窗口级权限**：`capabilities/default.json` 的
// `windows` 只有 `["main"]`，而 pet-overlay 是另建的独立 webview（加载
// `http://localhost:PORT/control/pet-overlay`，属于 remote origin）。Tauri 2 的
// `RuntimeAuthority::resolve_access` 要求「命令在白名单内 **且** 窗口标签匹配」，
// 于是该窗口内的所有 IPC —— 原生拖拽 `plugin:window|start_dragging`、
// 以及自定义的 `pet_overlay_move` / `pet_overlay_hide` —— 在安装版（release）
// 全部被拒；`debug` 构建不落拒绝分支，所以本地开发看不出来，只有装包后复现。
//
// 这里在 setup 阶段为该窗口补一份最小能力集（不放松 main 窗口的权限，也不改
// capabilities/default.json 的既有范围）。
fn grant_pet_overlay_capability(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::ipc::CapabilityBuilder;
    use tauri::Manager;

    app.add_capability(
        CapabilityBuilder::new("pet-overlay-runtime")
            .window(PET_OVERLAY_LABEL)
            // 悬浮窗由内嵌网关托管（remote origin），与 default.json 的 remote 段同源
            .remote(format!("http://localhost:{DEFAULT_PORT}/*"))
            .permission("core:window:allow-start-dragging")
            .permission("allow-pet-overlay-move")
            .permission("allow-pet-overlay-hide"),
    )
}

/// 应用入口
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        // 回合结束通知（2026-10-21）：长任务跑完时用户往往不在看屏幕。
        // 用官方通知插件而不是 Web Notification API：后者在 WebView2 /
        // WKWebView 上行为不一致（部分平台静默失败），且无法在窗口最小化时弹出。
        .plugin(tauri_plugin_notification::init())
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

            // 桌宠悬浮窗是独立 webview，不在 capabilities/default.json 的 windows 名单里；
            // 不补这份运行时能力，安装版的拖拽 / 隐藏会被 ACL 静默拒绝。
            if let Err(e) = grant_pet_overlay_capability(&handle) {
                eprintln!("[pet-overlay] 运行时 ACL 能力注册失败（拖拽/隐藏可能不可用）: {e}");
            }

            // ── 单实例 / 端口占用守卫（127.0.0.1:DEFAULT_PORT）──
            //
            // R14（2026-10-19）修：原实现把「端口被占用」**一律**当成「另一个 Marguerite
            // 正在运行」，弹窗后 `exit(1)`。但实测（2026-10-18 本机）一个残留的
            // `marguerite-core.exe`（CLI/服务进程）占着 18790 时，桌面端会**拒绝启动**并
            // 谎称「Marguerite 已在运行」——而桌面上根本没有窗口，用户唯一能做的只有
            // 重启机器。这是把「端口冲突」误判成「单实例」的语义错误。
            //
            // 现在先**问清楚占用者是谁**（`GET /api/health` 的 `engine=rust` + `version`）：
            // - 本产品网关 → **附着复用**（不再自起一个），窗口直接连它；版本不一致时
            //   如实弹一次提示（不静默连旧版，避免界面出现无法解释的契约错误）；
            // - 别的程序 → 给出可执行的处置指引（查 PID / 结束进程 / 改端口），不装作
            //   「另一个 Marguerite 在跑」。
            let port_owner = probe_port_owner();
            if let PortOwner::Foreign = port_owner {
                alert_fatal(
                    &handle,
                    "端口被其它程序占用",
                    &format!(
                        "127.0.0.1:{DEFAULT_PORT} 已被**非 Marguerite** 的程序占用，桌面端无法启动内嵌网关。\n\n\
                         Marguerite 的网关特征：GET http://127.0.0.1:{DEFAULT_PORT}/api/health 应返回 \
                         {{\"engine\":\"rust\"}}。当前该端口没有这个响应，说明占用者不是本程序。\n\n\
                         处置（任选）：\n\
                         1) 查占用者：netstat -ano | findstr :{DEFAULT_PORT}\n\
                         2) 结束它：taskkill /PID <上一步的 PID> /F\n\
                         3) 若是别的开发服务，请先停掉再启动 Marguerite。",
                        DEFAULT_PORT = DEFAULT_PORT
                    ),
                );
            }
            let attached_gateway = match &port_owner {
                PortOwner::OurGateway { version } => {
                    eprintln!(
                        "[gateway] 端口 {DEFAULT_PORT} 上已有 Marguerite 网关（version={version}），进入附着模式：复用现有网关，不再自起一个"
                    );
                    if version != env!("CARGO_PKG_VERSION") {
                        alert_async(
                            &handle,
                            "已连接到另一个版本的网关",
                            &format!(
                                "127.0.0.1:{DEFAULT_PORT} 上运行的网关版本为 {version}，当前桌面端为 {}。\n\
                                 界面由该网关托管，若出现功能缺失或契约错误，请先关闭那个进程（可能是 CLI/TUI 或旧版桌面端）再重启。",
                                env!("CARGO_PKG_VERSION")
                            ),
                        );
                    }
                    true
                }
                _ => false,
            };

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
            let webui_candidates = webui_dir_candidates(app);
            let webui_dir = resolve_webui_dir(app);
            let webui_missing = webui_dir.is_none();
            if webui_missing {
                eprintln!(
                    "[gateway] 未找到 webui/dist（探测 {} 个候选路径）：/control/ 将回诊断页而非控制台",
                    webui_candidates.len()
                );
            }
            let bundled_skills = resolve_skills_dir(app);
            let gw_app = app.handle().clone();
            std::thread::spawn(move || {
                // R14：附着模式下复用已在跑的网关，**不再自起一个**（否则必然 bind 失败，
                // 原实现会在这里二次弹「Marguerite 已在运行」并退出，等于把可用的启动路径
                // 变成死路）。放在闭包首行而非外面包 `if`，是为了不重排 200+ 行既有装配逻辑。
                if attached_gateway {
                    eprintln!(
                        "[gateway] 附着模式：复用 127.0.0.1:{DEFAULT_PORT} 上已有的网关，跳过内嵌网关启动"
                    );
                    return;
                }
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
                    // 会话入口叠加「用户选择的模型」：网关的 `ctx.config` 是 chat 发信的
                    // 密钥/模型判据（`gateway::chat` 的 api_key 门禁、`system.health` 上报的
                    // provider/model），若这里只读 config.yaml，则会与 `models.json` 里
                    // 用户选中的档案分叉——表现为「界面显示 A 模型，实际跑 B」。
                    let mut config = marguerite_core::model::selector::apply_effective_model(
                        &marguerite_core::persistence::config::Config::load().unwrap_or_default(),
                    );
                    // 安装版：把打包内嵌的技能目录（resource_dir/skills 等）注入
                    // config.skills.skills_dir，使 WebUI skills.list 能枚举到内置技能；
                    // 开发期 src-tauri/skills 未复制时 bundled_skills 为 None，保留默认
                    // 相对 "skills"（cwd 上溯）以兼容本地源码运行。仅改内存快照，不落盘，
                    // 避免安装路径变更后残留绝对路径失效。
                    if let Some(skills) = bundled_skills {
                        config.skills.skills_dir = skills.to_string_lossy().into_owned();
                    }
                    // ── P0 S2：插件 patch 层**只读装载**（解析 + 校验 + 排序 + 摘要日志）──
                    // 现状：「装了不动」在本项目是**完全静默**的（清单只被当数据读）。
                    // 本切片只加载一次并打一行摘要；**不 apply 到槽位**（见后续切片 S4），
                    // 不改 GatewayState、不新增 RPC。装载失败仅告警，绝不让启动失败。
                    {
                        use marguerite_core::plugins::manifest::PluginManifest;
                        use marguerite_core::plugins::patch;

                        let known: Vec<PluginManifest> =
                            match plugin_manifest_dir_local() {
                                Some(dir) => match std::fs::read_dir(&dir) {
                                    Ok(entries) => {
                                        let mut out = Vec::new();
                                        for entry in entries.flatten() {
                                            let path = entry.path();
                                            let is_manifest = path
                                                .file_name()
                                                .and_then(|n| n.to_str())
                                                .map(|n| n.ends_with(".plugin.json"))
                                                .unwrap_or(false);
                                            if !is_manifest {
                                                continue;
                                            }
                                            match std::fs::read_to_string(&path) {
                                                Ok(text) => match PluginManifest::from_json(&text) {
                                                    Ok(m) => out.push(m),
                                                    Err(e) => eprintln!(
                                                        "[patch] 清单 {} 解析失败，已跳过: {e}",
                                                        path.display()
                                                    ),
                                                },
                                                Err(e) => eprintln!(
                                                    "[patch] 清单 {} 读取失败，已跳过: {e}",
                                                    path.display()
                                                ),
                                            }
                                        }
                                        out
                                    }
                                    Err(e) => {
                                        eprintln!(
                                            "[patch] 清单目录 {} 不可读: {e}",
                                            dir.display()
                                        );
                                        Vec::new()
                                    }
                                },
                                None => {
                                    eprintln!(
                                        "[patch] 未找到 plugins/manifests 目录，patch 校验将全部报 orphan_target"
                                    );
                                    Vec::new()
                                }
                            };

                        // P0 S3：把真源清单快照存进 AppState（供 plugins_list / doctor_check 读取）。
                        // 只加字段，不改 GatewayState、不改本块既有校验语义。
                        if let Some(state) = gw_app.try_state::<Arc<AppState>>() {
                            if let Ok(mut guard) = state.plugin_manifests.lock() {
                                *guard = known.clone();
                            }
                        }

                        match patch::patches_dir() {
                            Some(dir) => match patch::PatchLayer::load_dir(&dir) {
                                Ok(layers) => {
                                    let summary = patch::summarize(&layers, &known);
                                    // P0 S3：patch 摘要（含诊断）同样存入 AppState，供 doctor 消费。
                                    if let Some(state) = gw_app.try_state::<Arc<AppState>>() {
                                        if let Ok(mut guard) = state.plugin_patch_summary.lock() {
                                            *guard = Some(summary.clone());
                                        }
                                    }
                                    let profiles = summary
                                        .by_profile
                                        .iter()
                                        .map(|(p, n)| format!("{p}={n}"))
                                        .collect::<Vec<_>>()
                                        .join(",");
                                    eprintln!(
                                        "[patch] 装载 {} 条 patch（profile: {}），诊断 {} 条",
                                        summary.total,
                                        if profiles.is_empty() {
                                            "无".to_string()
                                        } else {
                                            profiles
                                        },
                                        summary.diagnostics.len()
                                    );
                                    for d in &summary.diagnostics {
                                        eprintln!(
                                            "[patch] [{}] target={} {}",
                                            d.code, d.target, d.message
                                        );
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[patch] patch 目录 {} 装载失败: {e:#}", dir.display())
                                }
                            },
                            None => eprintln!(
                                "[patch] 未找到 patch 目录（MARGUERITE_PATCHES_DIR / plugins/patches）"
                            ),
                        }
                    }

                    match GatewayState::new(config, webui_dir) {
                        Ok(state) => {
                            let state = std::sync::Arc::new(state);
                            if let Err(e) = gateway::run(state, DEFAULT_PORT).await {
                                // 走到这里说明：启动前的探测判定「端口空闲」，但真正 bind 时
                                // 已被别的进程抢走（竞态）。如实报「端口冲突」，不再断言
                                // 「Marguerite 已在运行」——那句话在没有第二个 Marguerite 时是假信息。
                                alert_fatal(
                                    &gw_app,
                                    "端口冲突，内嵌网关无法启动",
                                    &format!(
                                        "内嵌网关未能在 127.0.0.1:{DEFAULT_PORT} 启动（{e}）。\n\
                                         该端口在启动瞬间被其它进程占用（竞态）。请重试一次；\
                                         若持续失败，用 netstat -ano | findstr :{DEFAULT_PORT} 查占用者。",
                                        DEFAULT_PORT = DEFAULT_PORT
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

            // ── 网关就绪等待（窗口加载 /control/ 前确保**界面**真的可用）──
            // 判定依据是真实 `GET /control/` 的 HTTP 200（且非兜底诊断页），
            // 不是 TCP connect：后者曾在产物缺失时谎报「就绪」（P0-D 缺陷 1）。
            //
            // R13（2026-10-18）两处修正，依据是**真实安装包启动日志**：
            //   18:27:49 `gateway.startup_phase … startup_elapsed_ms=7051`
            // ① 首轮截止从 8s 提到 20s。实测冷启动就要 7.05s，8s 只剩 0.95s 余量；
            //    磁盘冷缓存 / 首次解压 pyengine / 杀软扫描任一发生就会超时（Windows
            //     Defender 首次扫描新装的 240MB 程序集是常态，不是异常）。
            // ② 超时不再"放弃"：起一个**后台续探**线程，最多再等 90s，一旦就绪立刻
            //    把主窗口导航到 `/control/`。原实现超时后只弹对话框，窗口停在 core
            //    的 503 诊断页，**没有任何自动恢复路径** —— 用户唯一选择是关掉重开
            //    （而重开时网关早已就绪，于是"重启就好了"这类最难复现的体验缺陷）。
            //
            // 【2026-10-23 修复·首帧】该等待**不能**留在 setup 里同步跑：`setup` 在主线程
            // 且早于事件循环启动，同步 sleep 多久窗口就多久不绘制 —— 启动动画会被自己的
            // 等待逻辑挡住，表现为「白屏几秒」，与"第一帧就看到动画"正好相反。
            // 改为后台线程：setup 立即返回 → 窗口立刻渲染 splash → 就绪后由它 navigate。
            {
                let handle = handle.clone();
                let webui_candidates = webui_candidates.clone();
                let splash_started = std::time::Instant::now();
                std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
                // 循环体内必定赋值后才可能 break，故此处无需初值（避免 unused_assignments）。
                let mut last: ControlProbe;
                loop {
                    last = probe_control_page(std::time::Duration::from_millis(1500));
                    if matches!(last, ControlProbe::Ready)
                        || std::time::Instant::now() >= deadline
                    {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(150));
                }

                if matches!(last, ControlProbe::Ready) {
                    eprintln!(
                        "[gateway] 内嵌网关已在 127.0.0.1:{DEFAULT_PORT} 就绪（GET /control/ → HTTP 200，真实控制台入口）"
                    );
                    // 【2026-10-23 修复：首屏是本地启动动画，就绪后必须主动导航】
                    //
                    // tauri.conf 的窗口 url 已改为 `splash.html`（随包内嵌，秒开），
                    // 目的是不让用户在网关冷启动期间看到 WebView2 的
                    // 「无法连接到 127.0.0.1」内部错误页。
                    // 但 splash 自己不探测网关（CSP 的 connect-src 不含回环地址），
                    // 所以"就绪 → 进控制台"这一步只能由 Rust 完成。
                    // 旧实现只打日志不导航 —— 实测会永远停在启动动画上。
                    // 【2026-10-23】启动动画至少展示 MIN_SPLASH_MS 再跳转。
                    //
                    // 热启动时网关 1–2s 就绪，动画会"一闪而过"——观感像闪屏，用户
                    // 根本看不清品牌素材（用户口径：启动界面和视频放久一点）。
                    // 冷启动本来就 >6s，这段不会额外增加等待。
                    const MIN_SPLASH_MS: u64 = 6000;
                    let shown = splash_started.elapsed().as_millis() as u64;
                    if shown < MIN_SPLASH_MS {
                        std::thread::sleep(std::time::Duration::from_millis(
                            MIN_SPLASH_MS - shown,
                        ));
                    }
                    if let Some(win) = handle.get_webview_window("main") {
                        match win.navigate(control_url().parse().unwrap_or_else(|_| {
                            unreachable!("control_url 为编译期固定回环地址")
                        })) {
                            Ok(()) => eprintln!("[gateway] 已把主窗口从启动动画导航到控制台"),
                            Err(e) => eprintln!("[gateway] 就绪但导航失败：{e}"),
                        }
                    }
                } else {
                    // 未就绪：打印可执行诊断（探测过的路径 + 修复动作），并用原生对话框
                    // 告知用户（非致命，不阻塞启动）。窗口本身会渲染 core 侧的 503
                    // 诊断页，因此不会停在空白页。
                    let (summary, hint) =
                        diagnose_control_failure(&last, webui_missing, &webui_candidates);
                    eprintln!("[gateway] {summary}");
                    for line in hint.lines() {
                        eprintln!("[gateway] {line}");
                    }
                    alert_async(
                        &handle,
                        "内嵌网关界面未就绪",
                        &format!("{summary}\n\n{hint}"),
                    );
                    // 后台续探：就绪后自动把窗口带回控制台（用户无需重启）。
                    // 只读 argv 里的窗口句柄不成立，故经 AppHandle 回主线程取窗口。
                    spawn_late_readiness_recovery(handle.clone());
                }
                });
            }
            Ok(())
        })
        .manage(Arc::new(AppState::new()))
        .invoke_handler(tauri::generate_handler![
            guard_check,
            // 开机自启动（HKCU Run 键，零依赖；见文件上方说明与安装器同步要求）
            autostart_status,
            autostart_set,
            repair_code,
            plan_task,
            run_turn,
            approval_approve,
            approval_deny,
            toggle_window,
            show_window,
            hide_window,
            // V2.1-P2：桌宠悬浮窗（独立命令组，契约与托盘菜单同步）
            pet_overlay_show,
            pet_overlay_hide,
            pet_overlay_toggle,
            pet_overlay_move,
            // P0-4：任务悬浮球（与桌宠同构的独立命令组）
            task_overlay_show,
            task_overlay_hide,
            task_overlay_toggle,
            task_overlay_move,
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
            sidecar_log_dir,
            sidecar_start,
            sidecar_stop,
            doctor_check,
            runtime_status,
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
            cron_list,
            cron_create,
            cron_remove,
            cron_run,
            notify_turn_ended,
            model_tier_get,
            model_tier_set,
            pet_status,
            pet_action,
            fs_list_dir,
            fs_read_file,
            fs_write_file,
            fs_mkdir,
            fs_delete,
            open_url
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ───────────── V2.2-Tauri 联调：桌宠悬浮窗纯逻辑单测 ─────────────
//
// 真实 `cargo tauri dev` 需要桌面会话 + WebView2，无法在无头 CI 自动断言。
// 此处把窗口决策的纯逻辑抽成可测函数（缺省位置 / 幂等标签），在 lib 层单测覆盖；
// 命令本体（WebviewWindowBuilder 配置）由 `cargo check -p marguerite-gui` 编译门禁
// + 人工 `cargo tauri dev` 冒烟清单（见 docs/development.md）保证。
#[cfg(test)]
mod pet_overlay_tests {
    use super::*;

    #[test]
    fn default_position_uses_workarea_bottom_right() {
        let (x, y) = pet_overlay_default_position(Some((0.0, 0.0, 1920.0, 1040.0)), 360.0, 420.0);
        assert_eq!(x, 1920.0 - 360.0 - 24.0);
        assert_eq!(y, 1040.0 - 420.0 - 24.0);
    }

    #[test]
    fn default_position_offsets_from_non_zero_workarea_origin() {
        let (x, y) = pet_overlay_default_position(Some((80.0, 40.0, 1000.0, 800.0)), 360.0, 420.0);
        assert_eq!(x, 80.0 + 1000.0 - 360.0 - 24.0);
        assert_eq!(y, 40.0 + 800.0 - 420.0 - 24.0);
    }

    #[test]
    fn default_position_falls_back_without_monitor() {
        let (x, y) = pet_overlay_default_position(None, 360.0, 420.0);
        assert_eq!((x, y), (640.0, 480.0));
    }

    #[test]
    fn overlay_window_constants_are_consistent() {
        // 契约卫生：URL 指向回环 DEFAULT_PORT 的 /control/pet-overlay（WebUI PetOverlayView 路由）
        let url = pet_overlay_url();
        assert!(
            url.starts_with(&format!("http://localhost:{DEFAULT_PORT}/control/pet-overlay")),
            "got: {url}"
        );
        assert_eq!(PET_OVERLAY_LABEL, "pet-overlay");
        const { assert!(PET_OVERLAY_W > 0.0 && PET_OVERLAY_H > 0.0) };
    }
}

/// P0-4：任务悬浮球纯逻辑单测（与 pet_overlay_tests 同构）。
#[cfg(test)]
mod task_overlay_tests {
    use super::*;

    #[test]
    fn default_position_sits_above_pet_overlay() {
        let (x, y) = task_overlay_default_position(
            Some((0.0, 0.0, 1920.0, 1040.0)),
            TASK_OVERLAY_W,
            TASK_OVERLAY_H,
        );
        assert_eq!(x, 1920.0 - TASK_OVERLAY_W - 24.0);
        let pet_top = 1040.0 - PET_OVERLAY_H - 24.0;
        assert_eq!(y, pet_top - TASK_OVERLAY_H - 12.0);
        // 与桌宠缺省矩形不重叠
        assert!(y + TASK_OVERLAY_H <= pet_top);
    }

    #[test]
    fn default_position_falls_back_without_monitor() {
        let (x, y) = task_overlay_default_position(None, TASK_OVERLAY_W, TASK_OVERLAY_H);
        assert_eq!((x, y), (640.0, 120.0));
    }

    #[test]
    fn overlay_window_constants_are_consistent() {
        let url = task_overlay_url();
        assert!(
            url.starts_with(&format!("http://localhost:{DEFAULT_PORT}/control/task-overlay")),
            "got: {url}"
        );
        assert_eq!(TASK_OVERLAY_LABEL, "task-overlay");
        assert_eq!((TASK_OVERLAY_W, TASK_OVERLAY_H), (120.0, 120.0));
    }
}

/// P0-D 缺陷 1：内嵌网关「界面就绪」判定的单测。
///
/// 原缺陷是「TCP connect 成功即判就绪」——产物缺失时 `/control/` 实为 404 白屏，
/// 日志却打印就绪。这些测试把判定钉在真实 HTTP 状态 + 内容标记上。
#[cfg(test)]
mod gateway_readiness_tests {
    use super::*;
    const REAL_INDEX: &str =
        "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\n\r\n<!doctype html><html><body><div id=\"app\"></div></body></html>";

    #[test]
    fn ready_only_for_200_without_fallback_marker() {
        assert_eq!(
            classify_control_response(REAL_INDEX),
            ControlProbe::Ready,
            "200 + 真实入口应判就绪"
        );
    }

    #[test]
    fn missing_webui_fallback_page_is_never_ready() {
        // core 兜底诊断页：503 + 标记（标记常量与 core 同源，防两处字面量漂移）
        let page = format!(
            "HTTP/1.1 503 Service Unavailable\r\ncontent-type: text/html\r\nx-marguerite-webui: missing\r\n\r\n<!doctype html><html><body {}>未找到 webui/dist</body></html>",
            marguerite_core::gateway::server::WEBUI_MISSING_MARKER
        );
        assert_eq!(
            classify_control_response(&page),
            ControlProbe::FallbackPage { status: 503 },
            "兜底诊断页不得判就绪（这正是原缺陷的假阳性）"
        );
    }

    #[test]
    fn status_200_with_marker_still_treated_as_fallback() {
        // 防御：万一诊断页状态码被改成 200，标记仍能拦住假就绪
        let page = format!(
            "HTTP/1.1 200 OK\r\n\r\n<html><body {}></body></html>",
            marguerite_core::gateway::server::WEBUI_MISSING_MARKER
        );
        assert_eq!(
            classify_control_response(&page),
            ControlProbe::FallbackPage { status: 200 }
        );
    }

    #[test]
    fn blank_404_and_other_statuses_are_not_ready() {
        // 实测的原缺陷响应：404 且 content-length: 0（空白窗）
        let blank = "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n";
        assert_eq!(
            classify_control_response(blank),
            ControlProbe::HttpStatus { status: 404 }
        );
        assert_eq!(
            classify_control_response("HTTP/1.1 500 Internal Server Error\r\n\r\nx"),
            ControlProbe::HttpStatus { status: 500 }
        );
        assert!(matches!(
            classify_control_response("not-http-at-all"),
            ControlProbe::Unreachable { .. }
        ));
    }

    #[test]
    fn diagnostics_list_probed_paths_and_fix_actions() {
        let candidates = vec![
            std::path::PathBuf::from(r"C:\app\resources\webui\dist"),
            std::path::PathBuf::from(r"C:\app\webui\dist"),
        ];
        let (summary, hint) = diagnose_control_failure(
            &ControlProbe::FallbackPage { status: 503 },
            true,
            &candidates,
        );
        assert!(summary.contains("HTTP 503"), "摘要: {summary}");
        assert!(summary.contains("未就绪"), "摘要: {summary}");
        for c in &candidates {
            assert!(
                hint.contains(&c.display().to_string()),
                "诊断须列出已探测路径: {hint}"
            );
        }
        assert!(hint.contains("npm run build"), "诊断须给构建指引: {hint}");
        assert!(hint.contains("MARGUERITE_WEBUI_DIST"), "诊断须给环境变量: {hint}");
        assert!(hint.contains("exe 同级目录"), "诊断须给放置指引: {hint}");

        // 产物存在但网关不可达：给端口占用指引，而不是误报产物缺失
        let (summary, hint) = diagnose_control_failure(
            &ControlProbe::Unreachable {
                reason: "连接失败: 拒绝".to_string(),
            },
            false,
            &candidates,
        );
        assert!(summary.contains("无法访问"), "摘要: {summary}");
        assert!(hint.contains("18790"), "端口指引: {hint}");
        assert!(!hint.contains("npm run build"), "不该误报产物缺失: {hint}");
    }

    // ── R14：端口占用者判定 ──────────────────────────────────────────
    // 缺陷背景：端口被占时一律弹「Marguerite 已在运行」并 exit(1)。实测本机
    // 一个残留 marguerite-core.exe 占着 18790 就让桌面端完全无法启动。

    #[test]
    fn port_owner_recognises_own_gateway_with_version() {
        let raw = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n\
                   {\"status\":\"ok\",\"engine\":\"rust\",\"version\":\"0.2.13\",\"uptime_ms\":42}";
        assert_eq!(
            classify_port_owner(raw),
            PortOwner::OurGateway {
                version: "0.2.13".to_string()
            }
        );
    }

    #[test]
    fn port_owner_treats_third_party_ok_as_foreign() {
        // 第三方服务也可能回 {"status":"ok"}：缺 engine=rust 必须判为外部占用，
        // 否则会「附着」到一个根本不是本产品的端口上，界面全错。
        let raw = "HTTP/1.1 200 OK\r\n\r\n{\"status\":\"ok\"}";
        assert_eq!(classify_port_owner(raw), PortOwner::Foreign);
        // engine 存在但不是 rust（如 Python 侧车）同样不是可附着的网关
        let py = "HTTP/1.1 200 OK\r\n\r\n{\"status\":\"ok\",\"engine\":\"python\"}";
        assert_eq!(classify_port_owner(py), PortOwner::Foreign);
    }

    #[test]
    fn port_owner_rejects_non_200_and_non_json() {
        // 状态码非 200：即便 body 像健康检查也不认（可能是网关的错误页）
        let raw = "HTTP/1.1 503 Service Unavailable\r\n\r\n\
                   {\"status\":\"ok\",\"engine\":\"rust\",\"version\":\"0.2.13\"}";
        assert_eq!(classify_port_owner(raw), PortOwner::Foreign);
        // 纯文本（如 `upstream connect failed …`）不是 JSON
        let text = "HTTP/1.1 502 Bad Gateway\r\n\r\nupstream connect failed";
        assert_eq!(classify_port_owner(text), PortOwner::Foreign);
        // 没有 header/body 分隔符
        assert_eq!(classify_port_owner("HTTP/1.1 200 OK"), PortOwner::Foreign);
    }

    #[test]
    fn port_owner_accepts_health_without_version_field() {
        // 旧版网关（本次改动前）不带 version：仍应可附着，版本记为 unknown，
        // 由调用方决定是否提示「版本未知」——不能因为缺字段就判定成外部程序。
        let raw = "HTTP/1.1 200 OK\r\n\r\n{\"status\":\"ok\",\"engine\":\"rust\"}";
        assert_eq!(
            classify_port_owner(raw),
            PortOwner::OurGateway {
                version: "unknown".to_string()
            }
        );
    }
}
