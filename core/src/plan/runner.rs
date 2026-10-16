//! 复杂任务计划驱动长执行（Bloom 长续航核心）
//!
//! 聊天主路径检测到复杂任务（复杂度 Complex）时，不再跑单轮，而是进入
//! 「自动拆解 → 计划驱动分步执行」闭环：
//!
//! 1. `plans.analyze` 自动拆解 → 发布 `session.event.plan_revision`（前端
//!    PlanRunRibbon 出现计划步骤）；
//! 2. `plans.run` 启动运行 → 发布 `session.event.plan_run`（running）；
//! 3. 注册长时任务（心跳 + 检查点，跨会话续跑/恢复）；
//! 4. 逐步执行：每步由 Orchestrator 子任务驱动，进度与完成事件逐步发布，
//!    步骤间做上下文渐进压缩（防长窗口爆掉）、行为捕获；
//! 5. 步骤失败：发布错误并终止运行（terminal reason=blocked），已完成结论仍回传；
//! 6. 全部完成：运行置 completed，聚合各步结论作为 lead 收据返回。

use std::sync::{Arc, OnceLock};

use serde_json::{json, Value};

use crate::automation::long_running::{self, Stage};
use crate::domain::AgentMode;
use crate::events::{Envelope, EventBus, EventType};
use crate::gateway::mode_route::mode_route;
use crate::goal::runner_guard::{GoalGuardDecision, GoalRunnerGuard};
use crate::governance::completion_gate::{CompletionGate, RealShellRunner, ShellRunner};
use crate::mode::ModeKind;
use crate::model::router::Complexity;
use crate::orchestration::deliverable;
use crate::orchestration::{OrchestrationOutcome, Orchestrator};
use crate::persistence::config::Config;
use crate::plan;

/// 复杂任务判定与分析共享的分析器（纯规则、零 LLM、进程内缓存避免重复构造）。
fn analyzer() -> &'static Orchestrator {
    static R: OnceLock<Orchestrator> = OnceLock::new();
    R.get_or_init(|| Orchestrator::new(Config::default()))
}

/// 子步骤瞬时失败的最大重试次数（v9 完成性补强：商业级任务对瞬时错误有界重试）。
const STEP_MAX_RETRIES: usize = 3;

/// 单步执行的**总**墙钟预算（秒）：覆盖所有重试 + 退避的总耗时上限，而非每次重试
/// 各自计时。任何 LLM/工具调用若在此预算内仍无法返回，视为「远端无响应」并终止
/// 本步，确保「卡死子步骤」不会拖垮整个计划闭环（对应「任务执行效果垃圾」的真实
/// 根因——无界挂起）。采用「总预算递减」而非「每次重试独立超时」，避免重试次数
/// 放大等待时长。
///
/// v3.2：可配置（`plan.step_timeout_secs`），默认从 30s 提升到 **120s**。
/// 实测 DeepSeek v4 真实推理 + 工具回路单步常超 30s（尤其复杂拆解步骤），
/// 旧默认导致复杂任务第一步即失败（`plan_runner` 错误），整个自动拆解闭环报废。
fn step_total_timeout() -> u64 {
    let configured = cached_plan_config()
        .map(|c| c.plan.step_timeout_secs)
        .unwrap_or(0);
    if configured > 0 {
        configured
    } else {
        STEP_TOTAL_TIMEOUT_DEFAULT
    }
}

/// 进程级 `Config` 缓存（P2-12）：`step_total_timeout()` 每步都会调用，
/// 而 `Config::load()` 每次都要解析配置文件——旧实现还**静默**把解析失败
/// 回落成 0（→ 默认 120s），既不提示也无从排查。现首次 load 后冻结（进程是
/// 长驻网关，配置热变更另有通道），解析失败只 `warn!` 一次并缓存「无配置」标记。
static PLAN_CONFIG_CACHE: OnceLock<Option<Config>> = OnceLock::new();
static PLAN_CONFIG_WARNED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
static PLAN_CONFIG_LOADS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

fn cached_plan_config() -> Option<&'static Config> {
    use std::sync::atomic::Ordering;
    PLAN_CONFIG_CACHE
        .get_or_init(|| {
            #[cfg(test)]
            PLAN_CONFIG_LOADS.fetch_add(1, Ordering::Relaxed);
            match Config::load() {
                Ok(c) => Some(c),
                Err(e) => {
                    if !PLAN_CONFIG_WARNED.swap(true, Ordering::Relaxed) {
                        tracing::warn!(
                            error = %e,
                            "plan: Config::load() 失败，step_timeout 回落默认值（此告警仅出现一次）"
                        );
                    }
                    None
                }
            }
        })
        .as_ref()
}

/// 默认单步总预算（秒）。`plan.step_timeout_secs=0` 时生效。
const STEP_TOTAL_TIMEOUT_DEFAULT: u64 = 120;

/// N2：计划步骤 dispatch 期间挂上 `SESSION`（产物归属）与 `RUN_ID`（成本归因）
/// 两个 task-local。
///
/// 计划运行由独立的 `tokio::spawn`/`JoinSet` 执行，`SESSION` task-local **不会**
/// 自动继承到新任务；缺了它，步骤内的文件工具登记产物时 `current_session()` 为空，
/// 产物被记到默认会话、而非本次会话——于是「本次会话产物列表」永远为空
/// （真机现象：磁盘已有 `output.html`，右栏仍是空态）。
async fn with_step_scope<T, F>(session: &str, run_id: &str, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    crate::usage::SESSION
        .scope(
            session.to_string(),
            crate::usage::RUN_ID.scope(run_id.to_string(), fut),
        )
        .await
}

/// 计划子步骤的**固定输出结构约束**（P1-2）。
///
/// 步骤**标题**由 core 生成（`【计划子步骤 {sid}｜{title}】`，格式稳定），但子步骤的
/// **正文/结论**此前由 LLM 自由发挥 → 同一需求在不同会话里小节标题、表格列名、emoji
/// 都不一致（实测举例：`# step-1｜理解需求与约束` vs `# Step-1 结论：…`）。本约束以
/// **system 消息**随子步骤编排注入，固定小节标题与表格列名，使同类任务产出稳定格式。
///
/// 诚实边界：LLM 仍**可能**不完全遵守（提示约束非强保证）；此处只把「稳定格式」从
/// 「碰运气」提升为「明确要求」，并在下方给出可见的约束原文。
pub const PLAN_SUBSTEP_OUTPUT_CONTRACT: &str = "\
【计划子步骤输出结构（固定，必须严格遵守）】\n\
严格按以下固定小节输出，**不得**自创小节标题、序号或 emoji 前缀：\n\
## 结论\n\
用一句话给出本子步骤的最终结论（不要展开其它子步骤）。\n\
## 依据\n\
支撑结论的证据或来源，每条一行并标注来源（文件路径 / 链接 / 工具名）。\n\
## 产出\n\
本子步骤交付的文件 / 代码 / 数据；无则写「无」。\n\
## 风险与假设\n\
列出假设与未决项；无则写「无」。\n\
若需表格，一律使用固定中文列名：| 项目 | 内容 | 依据 |";

/// 步骤错误是否可重试（P1-5/T05）：裁决已收口到 `crate::errors`（结构化类别 +
/// provider 归类优先，子串仅作过渡兜底）。
pub use crate::errors::is_retryable_step_error;

/// 以已完成步骤的**真实**耗时估算剩余时长：返回 `(平均单步耗时, 剩余预计耗时)`。
///
/// 串行 runner 与并行批共用这一份口径——此前两处各写一遍均值公式，并行路径则
/// 完全没有 ETA；收口到纯函数后既消除重复，也让「无样本时不虚报」有单测锁定：
/// 无完成样本时返回 `(0, 0)`，前端降级为纯进度条（`PlanRunRibbon` 只在
/// `etaMs > 0` 时展示剩余时间）。
fn step_timing_estimate(samples: &[u64], remaining: usize) -> (u64, u64) {
    if samples.is_empty() {
        return (0, 0);
    }
    let avg = samples.iter().sum::<u64>() / samples.len() as u64;
    (avg, avg.saturating_mul(remaining as u64))
}

/// 项目型多阶段信号检测：不依赖复杂度分类器。
///
/// 覆盖「帮我做一个完善的FPS…」这类被分类器判为 Simple/Chat、但本质是多阶段
/// 交付型诉求的文本——命中「构建动作词 + 产物名/强化词」即视为复杂任务。
fn has_project_signal(text: &str) -> bool {
    let t = text.to_lowercase();
    const BUILD: &[&str] = &[
        "做一个",
        "做个",
        "构建",
        "开发",
        "搭建",
        "创建",
        "实现一个",
        "实现",
        "设计一个",
        "写一个",
        "完成一个",
        "做出一个",
        "完成",
    ];
    if !BUILD.iter().any(|w| t.contains(w)) {
        return false;
    }
    const PRODUCT: &[&str] = &[
        "游戏",
        "系统",
        "平台",
        "项目",
        "应用",
        "app",
        "工具",
        "网站",
        "网页",
        "软件",
        "编译器",
        "框架",
        "引擎",
        "客户端",
        "服务端",
        "数据库",
        "编辑器",
        "机器人",
        "模块",
        "组件",
    ];
    const STRENGTHEN: &[&str] = &["完善", "完整", "全套", "高级", "从零"];
    PRODUCT.iter().any(|w| t.contains(w)) || STRENGTHEN.iter().any(|w| t.contains(w))
}

/// 统一判定：是否应走「计划驱动长执行」（主路径判定与 runner 兜底复核共用）。
fn qualifies(config: &Config, text: &str) -> bool {
    if !config.plan.auto_decompose {
        return false;
    }
    let text = text.trim();
    let len = text.chars().count();
    // 极短/中短输入（闲聊式响应）不拆解；防过度
    if text.is_empty() || len < 12 {
        return false;
    }
    let a = analyzer().analyze(text);
    // 计划层第二条机制修复：复合任务（一句话含多个交付物）也必须触发自动拆解，
    // 否则会漏掉「搜索新闻并整理成网页再成PPT」这类明显需分解的诉求。
    a.complexity == Complexity::Complex
        || has_project_signal(text)
        || deliverable::detect(text).is_some()
}

/// 聊天主路径判定：该输入是否应走「计划驱动长执行」。
///
/// 开启 `plan.auto_decompose` 且满足以下任一条件时返回 true：
/// 1. 复杂度分类为 Complex（编程/科研等长任务）；
/// 2. 命中项目型信号（做/构建/开发 + 产物名/强化词，如「做一个完善的FPS射击游戏」）；
///
/// 简单/中等任务保持原单轮路径，行为完全向后兼容。
///
/// 门槛只做「闲聊过滤」：小于 12 字符的输入不拆解（避免"帮我修个bug"/"好"等
/// 简短诉求被切成多步噪音）。
pub fn should_auto_decompose(config: &Config, text: &str) -> bool {
    qualifies(config, text)
}

/// 从复合任务文本解析**契约声明的必需产物路径**（交付流水线 `target_path`）。
///
/// 计划路径（Plan/Agent/Ensemble/Bloom/Omni 共用）收尾完成门禁据此核验真实落盘：
/// 契约声明了 `output.html` / `presentation.pptx` 就必须真实存在，否则终态降为
/// `partial`——修复「4 秒显示已完成却无交付物」的真实根因（计划路径原先完全绕过产物门禁）。
pub fn required_artifacts_for(prompt: &str) -> Vec<String> {
    deliverable::detect(prompt)
        .map(|c| {
            c.deliverables
                .iter()
                .filter_map(|d| d.target_path.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// 从步骤标题里提取**产物目标路径**（形如 `output.html` / `presentation.pptx` /
/// `report.docx`）。
///
/// 步骤级完成核验用：标题声明了产物（含已知交付物扩展名）就必须真实落盘；不含任何
/// 产物路径的步骤（如「理解需求与约束」/「检索 …」）返回空，不施加约束（不误伤）。
pub fn step_target_paths(title: &str) -> Vec<String> {
    /// 已知交付物扩展名白名单（避免把散文里的 `1.5` / `v2.0` 误当文件）。
    const ARTIFACT_EXT: &[&str] = &[
        "html", "htm", "pptx", "ppt", "xlsx", "xls", "csv", "docx", "doc", "pdf", "md",
    ];
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"[A-Za-z0-9_./\\-]+\.[A-Za-z0-9]{1,6}").expect("步骤产物路径正则应可编译")
    });
    let mut out: Vec<String> = Vec::new();
    for m in re.find_iter(title) {
        let tok = m.as_str();
        let Some(ext) = std::path::Path::new(tok).extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if ARTIFACT_EXT.contains(&ext.to_ascii_lowercase().as_str()) && !out.iter().any(|p| p == tok)
        {
            out.push(tok.to_string());
        }
    }
    out
}

/// 停止原因 → 面向用户的文案。
///
/// 绝不把内部枚举（`not_armed` / `stale_revision` …）原样写进用户可见的回复：
/// 用户曾看到「计划执行结束（not_armed）。」这种只有实现者才读得懂的收尾语。
fn stopped_reason_text(reason: &str) -> &'static str {
    match reason {
        "completed" => "计划已执行完毕，但没有产生可复述的结论。",
        "paused" => "目标已被暂停，计划随之停止。恢复目标后可继续。",
        "blocked" => "目标已被标记为阻塞，计划随之停止。",
        "complete" => "目标已完成，计划随之停止。",
        "not_armed" => "目标当前未激活（重开后需重新激活目标），计划未继续执行。",
        "stale_revision" => "目标已被其他操作修改，为避免冲突计划已停止。",
        "max_goal_rounds" => "已达目标轮次上限，计划已停止。",
        "max_steps" => "已达单次运行的步骤上限，计划已停止。",
        _ => "计划已停止。",
    }
}

/// 计划驱动长执行入口。
///
/// 调用方保证已通过 [`should_auto_decompose`] 判定（本函数内部也会兜底复核）。
/// 事件经 `bus` 回流网关 → 前端；返回的 `OrchestrationOutcome` 由调用方包装
/// `session.event.router_decision` + `session.event.done` 富收据。
/// `experience_hint` 为可选的系统提示注入片段（如跨项目记忆），随每个子步骤
/// 编排器一并注入；空串时行为与旧版完全一致。
/// `cache` 为可选的共享前缀缓存（v9）：长任务各步骤复用同一缓存，可重复请求
/// 命中不重复预填充；None 时每步骤自建缓存。
/// `mode` 为当前**运行时模式**：计划路径据此经 [`mode_route`] 应用模式偏置
/// （MSC-2），使「切换模式 → 编排路径是否真实改变」在计划路径上同样可观测。
/// 计划运行期间的上下文台账订阅句柄。
///
/// 计划路径的事件同步发布到**网关总线**（见 `gateway::chat` 中 `run_autonomous` 调用点
/// 的注释），**不**经过 gateway 私总线桥 —— 那条桥只覆盖 bloom/单回合路径。因此计划路径的
/// 工具开销此前在令牌台账里**完全漏记**（`context_status` 偏低 → 压缩触发过晚）。
///
/// 这里在计划运行期间订阅本会话事件流补记；与 bloom 路径**互不重叠**（两者走不同总线，
/// 且同一回合只会走其中一条路径）。`Drop` 时 abort，避免任何早退路径泄漏后台任务。
struct LedgerTap(tokio::task::JoinHandle<()>);

impl LedgerTap {
    /// 订阅 `{session}.*`，把工具调用参数 / 工具结果计入令牌台账。
    fn spawn(bus: &Arc<EventBus>, session: &str) -> Self {
        let mut rx = bus.subscribe(&format!("{session}.*"));
        let sess = session.to_string();
        let handle = tokio::spawn(async move {
            while let Some(env) = rx.recv().await {
                let payload = &env.payload;
                let value = match env.r#type {
                    EventType::ToolCall => payload.get("input"),
                    EventType::ToolResult => payload.get("result"),
                    _ => None,
                };
                let Some(v) = value else { continue };
                if v.is_null() {
                    continue;
                }
                let body = match v.as_str() {
                    Some(s) => s.to_string(),
                    None => v.to_string(),
                };
                let tool = payload
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("tool");
                crate::context::session::record_tool_output(&sess, tool, &body);
            }
        });
        Self(handle)
    }
}

impl Drop for LedgerTap {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub async fn run_autonomous(
    config: Config,
    prompt: String,
    session: String,
    bus: Arc<EventBus>,
    experience_hint: String,
    cache: Option<Arc<crate::cache::PrefixCache>>,
    mode: ModeKind,
) -> anyhow::Result<OrchestrationOutcome> {
    if !qualifies(&config, &prompt) {
        return Err(anyhow::anyhow!(
            "run_autonomous 仅用于复杂/项目型任务，当前输入不满足"
        ));
    }
    // 模式路由偏置（MSC-2/Gap2）：计划路径也必须参考 mode_route 并应用 bias_strategy。
    // 原子子步骤的编排形态下界随模式抬升：Plan→Solo / Agent→Solo / Ensemble→ExpertTeam /
    // Bloom→DualLeader / Omni→Cluster。修复「计划路径完全无视运行时模式、五档事件流相同」。
    let route = mode_route(mode);
    let step_shape = route.bias_strategy(AgentMode::Solo);
    // 完成门禁核验用的工作区根：与各子步骤编排器（default_workspace）**同一**根目录，
    // 否则「文件真实落盘却核验不到」会误判为缺失。
    let workspace = crate::orchestration::default_workspace_root();
    let gate = CompletionGate::new();
    let shell: Arc<dyn ShellRunner> = Arc::new(RealShellRunner);
    let max_steps = config.plan.max_steps.max(1);
    let ckpt_every = config.plan.checkpoint_interval_steps.max(1);
    let heartbeat_s = config.plan.heartbeat_interval_s.max(5);

    // 1) 自动拆解 + 落库修订
    let created = plan::plans_create(&json!({ "prompt": prompt }));
    let Some(rev) = created.get("revision") else {
        let why = created
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("未知");
        return Err(anyhow::anyhow!("自动拆解失败：{why}"));
    };
    let mut revision_payload = json!({ "plan_revision": rev.clone() });
    // D-05/T08：同需求指纹命中「上次跑通」的计划时，随事件把复用候选一并下发
    // （不静默复用；前端 PlanCard 据此出现「继续上次计划」按钮）。
    if let Some(candidate) = created.get("reuse_candidate") {
        if !candidate.is_null() {
            revision_payload["reuse_candidate"] = candidate.clone();
        }
    }
    publish_plan(&bus, &session, EventType::PlanRevision, revision_payload);

    // 2) 启动运行实例
    let revision_id = rev["revisionId"].as_str().unwrap_or("").to_string();
    let started = plan::plans_run(&json!({ "revisionId": revision_id }));
    let Some(run) = started.get("run") else {
        return Err(anyhow::anyhow!(
            "计划运行启动失败：{}",
            started
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("未知")
        ));
    };
    let run_id = run["runId"].as_str().unwrap_or("").to_string();
    publish_plan(
        &bus,
        &session,
        EventType::PlanRun,
        json!({ "plan_run": run.clone() }),
    );

    // 3) 长时任务注册（心跳 + 检查点，跨会话续跑）
    let lrt_id = {
        let mut g = long_running::global()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        g.register(&prompt, heartbeat_s)
    };

    // 上下文令牌台账：补记 plan 路径的工具开销（详见 [`LedgerTap`]）。
    let _ledger_tap = LedgerTap::spawn(&bus, &session);

    let mut lead_replies: Vec<String> = Vec::new();
    let mut completed_steps = 0usize;
    let mut stopped_reason = "completed";
    let mut failed_step: Option<String> = None;
    // Goal 状态机是可选护栏：无 armed goal 时保持既有 plan runner 行为。
    // 有目标时每次 dispatch 前都必须先以 race-fenced revision 预留一轮。
    let mut goal_guard = GoalRunnerGuard::from_global();
    // 遥测（C1：任务执行质量）：累计成功 dispatch 的 LLM 调用数与耗时。
    // 失败/重试/超时的 dispatch 不纳入 total_elapsed_ms，但计划每步无论成败都有
    // LLM 往返；为保持「1 次 dispatch = 1 次 LLM 调用」的真实语义，仅统计进入成功
    // 分支的 dispatch（Solo 路径，单次调用）。两者在收尾处回填进 OrchestrationOutcome。
    let mut llm_call_count: u64 = 0;
    let mut total_elapsed_ms: u64 = 0;
    // C3 状态监控：记录每步真实耗时（毫秒），供平均单步耗时 → 剩余 ETA 估算。
    // run 启动但尚无完成步骤时 ETA 为 0（前端降级显示进度条，不虚报时间）。
    let mut step_elapsed_hist: Vec<u64> = Vec::new();

    // 4) 逐步执行
    while let Some((sid, title)) = current_step(&run_id) {
        match goal_guard.begin_global() {
            GoalGuardDecision::ContinueLegacy | GoalGuardDecision::Continue { .. } => {}
            GoalGuardDecision::Stopped { reason, .. } => {
                stopped_reason = reason.as_str();
                tracing::info!(session = %session, run_id = %run_id,
                    reason = stopped_reason, "自治目标终态护栏停止计划运行");
                // Close the plan run as cancelled so a Goal terminal state never
                // leaves an orphaned running plan behind.
                // C3：取消侧必须沿事件总线广播 PlanRun 终态，使 Web/GUI 状态流闭环。
                cancel_and_broadcast(&bus, &session, &run_id);
                break;
            }
        }
        completed_steps += 1;
        if completed_steps > max_steps {
            stopped_reason = "max_steps";
            tracing::warn!(session = %session, run_id = %run_id, steps = completed_steps,
                "计划执行达 max_steps 上限，自动停止");
            cancel_and_broadcast(&bus, &session, &run_id);
            break;
        }

        // 步骤间上下文渐进压缩（防长窗口爆掉）
        compact_if_needed(&session, &bus);

        // 子步骤任务文本：步骤标题 + 整体目标上下文
        let step_task = format!(
            "【计划子步骤 {sid}｜{title}】\n整体目标：{prompt}\n\n请完成上述子步骤，输出有依据的结论；不要展开其他子步骤。"
        );

        let mut orch =
            Orchestrator::new_with_credentials_and_cache(config.clone(), None, cache.clone());
        orch.set_event_bus(bus.clone(), session.clone());
        // 模式偏置（MSC-2/Gap2）：子步骤编排器的运行时模式 = 当前模式，使 mode_route 分派
        // 真实生效（Ensemble → 多 Agent 强编排；Bloom/Omni → 并行编排入口）。
        orch.set_mode(mode);
        // 仅当模式偏置后的子步骤形态为 Solo（Plan/Agent）时强制 Solo：原子任务无需升级为
        // 多 Agent 策略（浪费额度且易失控挂起）。Ensemble/Bloom/Omni 档保留 mode_route 分派，
        // 使子步骤真实走 ExpertTeam / 并行编排——这是「模式切换真实改变执行路径」的核心。
        orch.set_force_solo(matches!(step_shape, AgentMode::Solo));
        // 计划子步骤带工具执行（Gap1/Gap3）：Solo 步骤真实调用 write_file 等把产物落盘，
        // 而非只输出文本。Plan 档的写类工具由 tool_cycle 的运行时模式裁决硬拒绝
        // （产物不落盘 → 完成门禁阻断 → 终态 partial），非 Plan 档放行（产物真实落盘）。
        orch.set_solo_with_tools(true);
        // 计划子步骤：编排决策由方案卡一次性呈现，子步骤不再复述决策（A2/A3）。
        orch.set_plan_step(true);
        // v3.3 步骤开始进度事件：携带 stepIndex/stepsTotal/percent + taskKind 标签，
        // WebUI 据此渲染「第 n/N 步」进度条与任务类型徽章（编码/文档/对话…）。
        {
            let intent = crate::intent::Classifier::new().classify(&title).intent;
            let (idx, total) = plan::plan_run_progress(&run_id);
            // C3 状态监控：每阶段时间估计——以已完成步骤耗时的均值 × 剩余步数（含当前步）。
            // 无完成样本时 ETA=0，前端降级为纯进度条，避免虚报。
            let remaining_steps = total.saturating_sub(idx.saturating_sub(1));
            let (avg_ms, eta_ms) = step_timing_estimate(&step_elapsed_hist, remaining_steps);
            publish_plan(
                &bus,
                &session,
                EventType::Progress,
                json!({
                    "kind": "step_progress",
                    "stepId": sid,
                    "stepTitle": title,
                    "stepIndex": idx,
                    "stepsTotal": total,
                    "percent": if total > 0 { ((idx.saturating_sub(1)) as f64 / total as f64 * 100.0) as u32 } else { 0 },
                    "phase": "started",
                    "elapsedMs": total_elapsed_ms,
                    "etaMs": eta_ms,
                    "avgStepMs": avg_ms,
                    "taskKind": intent.task_label(),
                }),
            );
        }
        // 跨项目记忆等提示随子步骤编排注入（空串自动跳过）
        if !experience_hint.is_empty() {
            orch.set_experience_hint(experience_hint.clone());
        }
        // 步骤执行 + 瞬时失败退避重试（v 完成性补强）：
        // 治理门/宪法/反思门禁的拒绝是确定性裁决，绝不重试（重试无意义且烧额度）；
        // 网络/超时/5xx 等瞬时错误退避重试至 STEP_MAX_RETRIES 次。
        // 每一步包裹**总**墙钟预算（`STEP_TOTAL_TIMEOUT`）：用单调递减的剩余时间包装
        // 单次 dispatch，任何单步 LLM/工具调用若无限挂起（连接停滞、远端无响应），
        // 预算耗尽即判定为可重试瞬时错误并终止本步，绝不让一个卡死的子步骤拖垮整个
        // 计划闭环（对应「任务执行效果垃圾」的真实根因——无界挂起）。
        let mut last_err: Option<anyhow::Error> = None;
        let mut step_outcome = None;
        let step_budget = step_total_timeout();
        let step_deadline =
            tokio::time::Instant::now() + std::time::Duration::from_secs(step_budget);
        for attempt in 0..STEP_MAX_RETRIES {
            let remaining = step_deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                last_err = Some(anyhow::anyhow!(
                    "子步骤 {sid} 执行超出总预算 {step_budget}s，疑似远端无响应"
                ));
                break;
            }
            // C1 遥测：记录本步 dispatch 起始时刻，成功后据此统计耗时。
            // M3-D2 成本归因：dispatch 期间把 RUN_ID 挂进 task-local，让 usage::record
            // 把本步 LLM 用量按 run 归因（成本卡片可读 usage.taskCost）。
            let t0 = std::time::Instant::now();
            let dispatched = tokio::time::timeout(
                remaining,
                with_step_scope(&session, &run_id, orch.dispatch(&step_task)),
            )
            .await;
            match dispatched {
                Ok(Ok(outcome)) => {
                    let elapsed_ms = t0.elapsed().as_millis() as u64;
                    step_elapsed_hist.push(elapsed_ms);
                    llm_call_count += 1;
                    total_elapsed_ms += elapsed_ms;
                    step_outcome = Some(outcome);
                    break;
                }
                Ok(Err(e)) => {
                    let msg = e.to_string();
                    if !is_retryable_step_error(&e) || attempt + 1 >= STEP_MAX_RETRIES {
                        last_err = Some(e);
                        break;
                    }
                    tracing::warn!(
                        session = %session, run_id = %run_id, attempt = attempt + 1,
                        err = %msg, "步骤瞬时失败，退避重试"
                    );
                    tokio::time::sleep(std::time::Duration::from_secs((attempt + 1) as u64)).await;
                }
                Err(_elapsed) => {
                    // 墙钟预算耗尽：视为可重试瞬时错误（确定性超时与确定性裁决不同），
                    // 若仍有重试次数则退避后重试；否则本步失败并终止本轮（沿用既有语义）。
                    let msg = format!("子步骤 {sid} 执行超出总预算 {step_budget}s，疑似远端无响应");
                    tracing::warn!(
                        session = %session, run_id = %run_id, attempt = attempt + 1,
                        "步骤执行墙钟超时，退避重试"
                    );
                    if attempt + 1 >= STEP_MAX_RETRIES {
                        last_err = Some(anyhow::anyhow!(msg));
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs((attempt + 1) as u64)).await;
                }
            }
        }
        match step_outcome {
            Some(outcome) => {
                let reply = outcome.lead_reply;
                // 步骤级产物核验（P1-2/Gap3）：步骤标题声明的产物必须**真实落盘**，
                // 否则本步不算完成——绝不出现「步骤 completed + checklist 勾选却无产物」。
                let step_targets = step_target_paths(&title);
                let missing: Vec<String> = step_targets
                    .iter()
                    .filter(|p| {
                        !gate
                            .verify_required_artifacts(&[p.as_str()], &workspace)
                            .all_ok()
                    })
                    .cloned()
                    .collect();
                if !missing.is_empty() {
                    let msg = format!(
                        "计划子步骤 {sid}（{title}）未产出声明的产物（缺失: {}），不标记完成",
                        missing.join(", ")
                    );
                    tracing::warn!(session = %session, run_id = %run_id, %msg, "步骤产物缺失");
                    crate::memory::behavior::record_behavior(
                        &session,
                        "plan_step_artifact_missing",
                        &msg,
                    );
                    // R65/B1：错误正文改为「稳定码 + 结构化上下文」，人话化文案由前端
                    // 按码本地化（内部术语只留在 `msg` 的日志/行为记录里）。
                    bus.publish(plan_step_artifact_missing_error(&session, &sid, &title, &missing));
                    failed_step = Some(format!("{sid} {title}"));
                    stopped_reason = "failed";
                    // 关键：不调用 advance（不把该步标记 completed），并以取消广播把
                    // in_progress 步骤如实置为 blocked，清单勾选随步骤状态同步为未勾选。
                    cancel_and_broadcast(&bus, &session, &run_id);
                    break;
                }
                lead_replies.push(reply.clone());
                crate::context::session::record_turn(&session, "assistant", &reply);
                crate::memory::behavior::record_behavior(
                    &session,
                    "plan_step_done",
                    &format!("{sid} {title}"),
                );
                // 心跳 + 检查点（每 ckpt_every 步）
                let mut g = long_running::global()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let _ = g.heartbeat(&lrt_id);
                if completed_steps.is_multiple_of(ckpt_every) {
                    let _ = g.save_checkpoint(
                        &lrt_id,
                        Stage::Execute,
                        json!({
                            "runId": run_id,
                            "stepId": sid,
                            "completedSteps": completed_steps,
                            "maxSteps": max_steps,
                        }),
                    );
                }
                drop(g);
                // C3 状态监控：步骤完成时发布 phase:"finished"，带该步真实耗时
                // 与整体已用时长，前端据此展示单步耗时/ETA 余量，而非 UI 局部模拟。
                let (avg_step, _) = step_timing_estimate(&step_elapsed_hist, 0);
                if let Some(&step_ms) = step_elapsed_hist.last() {
                    publish_plan(
                        &bus,
                        &session,
                        EventType::Progress,
                        json!({
                            "kind": "step_progress",
                            "stepId": sid,
                            "stepTitle": title,
                            "phase": "finished",
                            "stepElapsedMs": step_ms,
                            "elapsedMs": total_elapsed_ms,
                            "avgStepMs": avg_step,
                        }),
                    );
                }
                // 推进运行：当前步骤 completed → 下一 pending
                let advanced =
                    plan::plans_run_advance(&json!({ "runId": run_id, "advance": true }));
                if let Some(r) = advanced.get("run") {
                    publish_plan(
                        &bus,
                        &session,
                        EventType::PlanRun,
                        json!({ "plan_run": r.clone() }),
                    );
                    if r["status"].as_str() == Some("completed") {
                        break;
                    }
                }
            }
            None => {
                let e = last_err.unwrap_or_else(|| anyhow::anyhow!("步骤执行失败"));
                failed_step = Some(format!("{sid} {title}"));
                stopped_reason = "failed";
                let msg = format!("计划子步骤 {sid} 执行失败：{e}");
                tracing::warn!(session = %session, run_id = %run_id, err = %e, "计划步骤失败");
                crate::memory::behavior::record_behavior(&session, "plan_step_failed", &msg);
                // R65/B1：内层已归类（如 AUTH_FAILED）则沿用其稳定码，否则 PLAN_STEP_FAILED。
                // 用户可见文案由前端按码 + `step_id`/`step_title` 组装，不再回显内部术语。
                bus.publish(plan_step_failed_error(&session, &sid, &title, &e));
                // 失败前落最终检查点（重试耗尽后的状态可被续跑机制消费）
                {
                    let mut g = long_running::global()
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    let _ = g.save_checkpoint(
                        &lrt_id,
                        Stage::Execute,
                        json!({
                            "runId": run_id,
                            "stepId": sid,
                            "completedSteps": completed_steps.saturating_sub(1),
                            "maxSteps": max_steps,
                            "failedStep": format!("{sid} {title}"),
                        }),
                    );
                }
                // 失败即终止本轮运行（重试已耗尽），已完成步骤结论保留回传
                // C3 状态监控：失败终止也广播 PlanRun 终态，避免多端悬挂 running。
                cancel_and_broadcast(&bus, &session, &run_id);
                break;
            }
        }
    }

    // 5) 收尾：长时任务置完成 + 运行终态事件 + 聚合结论
    // 5) 收尾：长时任务置完成 + 运行终态事件 + 聚合结论
    // 作用域块：`MutexGuard` 不是 `Send`，必须在本函数后续 `.await`（可选 LLM 验收）之前
    // 彻底出作用域 —— 单靠 `drop(g)` 不足以满足生成器的 Send 分析（其按作用域而非 drop 判定）。
    let latest = {
        let mut g = long_running::global()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = g.heartbeat(&lrt_id);
        let _ = g.complete(&lrt_id);
        long_running::task_view(g.get(&lrt_id).expect("长时任务应存在"))
    };
    if stopped_reason != "failed" {
        if let Some(r) = plan::plans_checklist(&json!({ "runId": run_id })).get("run") {
            publish_plan(
                &bus,
                &session,
                EventType::PlanRun,
                json!({ "plan_run": r.clone() }),
            );
        }
    }
    let mut lead_reply = if lead_replies.is_empty() {
        format!("计划执行结束：{}", stopped_reason_text(stopped_reason))
    } else if stopped_reason == "failed" {
        format!(
            "已完成 {} 个步骤，{} 失败后停止。已完成结论如下：\n\n{}",
            completed_steps.saturating_sub(1),
            failed_step.unwrap_or_default(),
            lead_replies.join("\n\n")
        )
    } else {
        lead_replies.join("\n\n")
    };
    let _ = latest;

    // 完成门禁（v9 完成性补强 + P1-2/Gap1 产物门禁）：聚合结论必须过四检查、且契约声明的
    // **必需产物**真实落盘才可标记 allowed——绝不硬编码放行；未放行时如实回传裁决与警告。
    let completion_verdict = if stopped_reason == "completed" {
        // 计划路径产物门禁：把契约声明的必需产物（如 output.html / presentation.pptx）
        // 作为强约束逐一核验真实落盘；缺失/为空 → MissingArtifact 阻断，终态降为 partial。
        // 这是「4 秒显示已完成却无交付物」的核心修复——此前此处硬编码 `artifacts: &[]`。
        let required = required_artifacts_for(&prompt);
        let required_refs: Vec<&str> = required.iter().map(String::as_str).collect();
        let mut verdict = gate.evaluate(&prompt, &lead_reply, &required_refs, &workspace, &shell, None);
        // 可选 LLM 验收（opt-in，默认关闭；fail-open）：确定性门禁放行之后，再由 LLM 复核
        // 「结果是否真的满足任务」。未启用 / 无 key / 调用失败 → 完全不影响本裁决。
        if verdict.allowed {
            if let Some(v) =
                crate::governance::verifier::verify_completion_if_enabled(&prompt, &lead_reply).await
            {
                if !v.satisfied {
                    verdict.allowed = false;
                    verdict
                        .blocked_by
                        .push(crate::governance::completion_gate::BlockReason::VerificationFailed);
                    verdict
                        .warnings
                        .push(format!("LLM 验收未通过：{}", v.reasoning));
                }
            }
        }
        // 可选自动评审（opt-in，默认关闭；fail-open）：仅阻断 blocking 级问题。
        if verdict.allowed {
            if let Some(rv) = crate::governance::review::review_completion_if_enabled(
                &prompt,
                &lead_reply,
                &workspace,
            )
            .await
            {
                if !rv.pass {
                    verdict.allowed = false;
                    verdict
                        .blocked_by
                        .push(crate::governance::completion_gate::BlockReason::VerificationFailed);
                    verdict.warnings.push(format!(
                        "自动评审未通过（{} 项阻断）：{}",
                        rv.blocking_count(),
                        rv.summary
                    ));
                }
            }
        }
        if !verdict.allowed {
            let warns = verdict.warnings.join("；");
            lead_reply = format!("{lead_reply}\n\n[完成门禁未放行] {warns}");
            // R65/B1：完成门禁未放行——稳定码 + 中性英文兜底；具体警告 `warns` 已并入
            // 上方 `lead_reply`（用户可在正文/日志查看），错误卡不再回显内部术语。
            bus.publish(plan_completion_blocked_error(&session));
        }
        verdict
    } else {
        // 失败/max_steps 终态：不允许标记完成，如实呈现
        crate::governance::completion_gate::CompletionVerdict {
            allowed: false,
            blocked_by: vec![],
            warnings: vec![format!("任务以 {stopped_reason} 结束")],
            evidence: std::collections::HashMap::new(),
        }
    };

    let analysis = analyzer().analyze(&prompt);
    // C1 遥测：把聚合的 LLM 调用数与总耗时回填进最终编排结果，并发布一帧遥测事件。
    // 事件总线 publish 属可观测性增强；签名与 emit_thinking 一致（kind 字段区分语义）。
    bus.publish(Envelope::new(
        EventType::Thinking,
        json!({
            "text": "计划执行质量遥测",
            "kind": "plan_telemetry",
            "llm_call_count": llm_call_count,
            "total_elapsed_ms": total_elapsed_ms,
            "completed_steps": completed_steps,
            "stopped_reason": stopped_reason,
        }),
        session.clone(),
    ));
    Ok(OrchestrationOutcome {
        // 模式偏置后的**子步骤编排形态**（MSC-2/Gap2）：计划路径逐原子步骤执行，
        // 其编排形态由 mode_route 抬升（Plan/Agent→Solo、Ensemble→ExpertTeam、
        // Bloom→DualLeader、Omni→Cluster），使 router_decision.strategy 随模式真实变化。
        strategy: step_shape,
        lead_reply,
        subagent_summaries: lead_replies,
        analysis,
        completion_verdict,
        reflect_verdict: None,
        llm_call_count,
        total_elapsed_ms,
    })
}

/// 方案 markdown → 步骤执行的「整体目标」上下文。
///
/// 剔除 `**编排形态**` / `**决策理由**` 等元信息行：这些行里的策略理由文本
/// （如「单 Agent 循环即可覆盖」）会被宪法前置裁决的破坏性关键词扫描误判为
/// C1 违规（「覆盖」命中），导致已批准实施的方案步骤被治理门拦截。
fn plan_step_context(markdown: &str) -> String {
    markdown
        .lines()
        .filter(|line| {
            let t = line.trim();
            !(t.starts_with("**编排形态**") || t.starts_with("**决策理由**"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 步骤执行的「整体目标」：计划标题（用户原话 / 编辑后标题）+ 剔除元信息后的正文。
///
/// A4 之后方案正文**只承载步骤列表**，不再重复写 `# 计划：…`（标题是唯一派生点，
/// 卡片 `<h3>` 呈现）。故这里以 `title` 补齐目标语义，避免实现路径丢失用户原始诉求。
fn plan_goal_context(title: &str, markdown: &str) -> String {
    let body = plan_step_context(markdown);
    let t = title.trim();
    if t.is_empty() {
        body
    } else if body.trim().is_empty() {
        t.to_string()
    } else {
        format!("{t}\n\n{body}")
    }
}

/// 按既有修订逐步执行（`plans.implement` 的后台执行体）。
///
/// 与 [`run_autonomous`] 的区别：不重新拆解，直接复用修订的标题与 markdown
/// 作为整体目标，按修订步骤逐条经 Orchestrator 真实执行；进度事件经 `bus`
/// 实时回流，`run_id` 由调用方先行创建（立即返回运行中卡片）。
pub async fn run_plan_revision_steps(
    config: Config,
    revision_id: String,
    run_id: String,
    session: String,
    bus: Arc<EventBus>,
    experience_hint: String,
) -> anyhow::Result<String> {
    let Some(rev_view) = plan::get_revision_view(&revision_id) else {
        anyhow::bail!("Plan revision not found: {revision_id}");
    };
    // 步骤「整体目标」用剔除元信息后的方案正文（标题先行补齐目标语义，见
    // `plan_goal_context`），避免决策理由文本（如「单 Agent 循环即可覆盖」）被
    // 宪法 C1 破坏性关键词扫描误判拦截。
    let title = rev_view["title"].as_str().unwrap_or("").to_string();
    let prompt = plan_goal_context(&title, rev_view["markdown"].as_str().unwrap_or(""));
    let max_steps = config.plan.max_steps.max(1);
    let ckpt_every = config.plan.checkpoint_interval_steps.max(1);
    let heartbeat_s = config.plan.heartbeat_interval_s.max(5);

    let lrt_id = {
        let mut g = long_running::global()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        g.register(&prompt, heartbeat_s)
    };

    let mut lead_replies: Vec<String> = Vec::new();
    let mut completed_steps = 0usize;
    let mut stopped_reason = "completed";
    let mut failed_step: Option<String> = None;
    let mut goal_guard = GoalRunnerGuard::from_global();

    while let Some((sid, step_title)) = current_step(&run_id) {
        match goal_guard.begin_global() {
            GoalGuardDecision::ContinueLegacy | GoalGuardDecision::Continue { .. } => {}
            GoalGuardDecision::Stopped { reason, .. } => {
                stopped_reason = reason.as_str();
                tracing::info!(session = %session, run_id = %run_id,
                    reason = stopped_reason, "自治目标终态护栏停止计划修订执行");
                break;
            }
        }
        completed_steps += 1;
        if completed_steps > max_steps {
            stopped_reason = "max_steps";
            tracing::warn!(session = %session, run_id = %run_id, steps = completed_steps,
                "计划执行达 max_steps 上限，自动停止");
            cancel_and_broadcast(&bus, &session, &run_id);
            break;
        }

        // 步骤间上下文渐进压缩（防长窗口爆掉）
        compact_if_needed(&session, &bus);

        // 子步骤任务文本：步骤标题 + 修订全文作整体目标
        let step_task = format!(
            "【计划子步骤 {sid}｜{step_title}】\n整体目标：{title}\n\n{prompt}\n\n请完成上述子步骤，输出有依据的结论；不要展开其他子步骤。"
        );

        let mut orch = Orchestrator::new(config.clone());
        orch.set_event_bus(bus.clone(), session.clone());
        // 原子子步骤强制 Solo：避免子步骤被升级为多 Agent 策略（浪费额度且易失控挂起）。
        orch.set_force_solo(true);
        // 计划子步骤：编排决策已由方案卡一次性呈现，子步骤只执行、不再复述决策（A2/A3）。
        orch.set_plan_step(true);
        // 跨项目记忆等提示随子步骤编排注入（空串自动跳过）
        if !experience_hint.is_empty() {
            orch.set_experience_hint(experience_hint.clone());
        }
        // M3-D2 成本归因：dispatch 期间挂 RUN_ID task-local，按 run 归因用量。
        let dispatch_outcome = with_step_scope(&session, &run_id, orch.dispatch(&step_task)).await;
        match dispatch_outcome {
            Ok(outcome) => {
                let reply = outcome.lead_reply;
                lead_replies.push(reply.clone());
                crate::context::session::record_turn(&session, "assistant", &reply);
                crate::memory::behavior::record_behavior(
                    &session,
                    "plan_step_done",
                    &format!("{sid} {step_title}"),
                );
                // 心跳 + 检查点（每 ckpt_every 步）
                let mut g = long_running::global()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let _ = g.heartbeat(&lrt_id);
                if completed_steps.is_multiple_of(ckpt_every) {
                    let _ = g.save_checkpoint(
                        &lrt_id,
                        Stage::Execute,
                        json!({
                            "runId": run_id,
                            "stepId": sid,
                            "completedSteps": completed_steps,
                            "maxSteps": max_steps,
                        }),
                    );
                }
                drop(g);
                // 推进运行：当前步骤 completed → 下一 pending
                let advanced =
                    plan::plans_run_advance(&json!({ "runId": run_id, "advance": true }));
                if let Some(r) = advanced.get("run") {
                    publish_plan(
                        &bus,
                        &session,
                        EventType::PlanRun,
                        json!({ "plan_run": r.clone() }),
                    );
                    if r["status"].as_str() == Some("completed") {
                        break;
                    }
                }
            }
            Err(e) => {
                failed_step = Some(format!("{sid} {step_title}"));
                stopped_reason = "failed";
                let msg = format!("计划子步骤 {sid} 执行失败：{e}");
                tracing::warn!(session = %session, run_id = %run_id, err = %e, "计划步骤失败");
                crate::memory::behavior::record_behavior(&session, "plan_step_failed", &msg);
                // R65/B1：稳定码 + 结构化上下文（同上一处；内部术语只留在日志）。
                bus.publish(plan_step_failed_error(&session, &sid, &step_title, &e));
                // 失败即终止本轮运行（不静默重试），已完成步骤结论保留回传
                // C3 状态监控：失败终止也广播 PlanRun 终态，避免多端悬挂 running。
                cancel_and_broadcast(&bus, &session, &run_id);
                break;
            }
        }
    }

    // 收尾：长时任务置完成 + 运行终态事件 + 聚合结论
    let mut g = long_running::global()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _ = g.heartbeat(&lrt_id);
    let _ = g.complete(&lrt_id);
    drop(g);
    if stopped_reason != "failed" {
        if let Some(r) = plan::plans_checklist(&json!({ "runId": run_id })).get("run") {
            publish_plan(
                &bus,
                &session,
                EventType::PlanRun,
                json!({ "plan_run": r.clone() }),
            );
        }
    }
    let lead_reply = if lead_replies.is_empty() {
        format!("计划执行结束：{}", stopped_reason_text(stopped_reason))
    } else if stopped_reason == "failed" {
        format!(
            "已完成 {} 个步骤，{} 失败后停止。已完成结论如下：\n\n{}",
            completed_steps.saturating_sub(1),
            failed_step.unwrap_or_default(),
            lead_replies.join("\n\n")
        )
    } else {
        lead_replies.join("\n\n")
    };
    // 把本会话上下文落盘（新任务实施时新建会话，sessions.list / chat.history 可见）
    crate::gateway::chat::persist_session(&session);
    Ok(lead_reply)
}

/// 并行步骤并发上限（Bloom 长续航策略：有界并发防瞬时打爆模型配额）。
const PARALLEL_STEP_CONCURRENCY: usize = 3;

/// 并行子步骤执行产物：终态 + **真实**耗时/尝试次数。
///
/// 此前并行批只回传 `(sid, title, ok, summary)`，于是前端拿不到单步耗时、
/// 也没有任何 ETA 依据（串行 runner 早已有 `stepElapsedMs`/`etaMs`）。补齐后
/// 并行批与串行批的进度契约一致，`PlanRunRibbon` 无需分叉渲染。
struct ParallelStepOutcome {
    step_id: String,
    title: String,
    ok: bool,
    summary: String,
    /// 本步真实墙钟耗时（含退避重试的等待）
    elapsed_ms: u64,
    /// 实际尝试次数（1 = 一次通过；>1 表示发生过退避重试）
    attempts: usize,
    /// 是否因耗尽单步总预算被判超时（与「远端明确报错」区分，便于诊断）
    timed_out: bool,
}

/// 派发一个并行子步骤任务（真执行，经 Orchestrator）。
///
/// 入参较多为并行调度上下文（set/config/bus + 会话/步骤字段），
/// 保持扁平签名便于调用方构造；clippy 风格 lint 局部豁免。
#[allow(clippy::too_many_arguments)]
fn spawn_step(
    set: &mut tokio::task::JoinSet<ParallelStepOutcome>,
    config: Config,
    bus: Arc<EventBus>,
    session: String,
    run_id: String,
    title_ctx: String,
    prompt_ctx: String,
    step: (String, String),
) {
    let (sid, step_title) = step;
    // N2：`session` 会被 set_event_bus 消费，先留一份给 SESSION task-local 作用域，
    // 否则 JoinSet 子任务里 `current_session()` 为空、产物归错会话。
    let session_scope = session.clone();
    set.spawn(async move {
        let step_task = format!(
            "【计划并行子步骤 {sid}｜{step_title}】\n整体目标：{title_ctx}\n\n{prompt_ctx}\n\n请完成上述子步骤，输出有依据的结论；不要展开其他子步骤。"
        );
        let mut orch = Orchestrator::new(config);
        orch.set_event_bus(bus, session);
        // 计划并行子步骤：编排决策由方案卡一次性呈现，子步骤不再复述决策（A2/A3）。
        orch.set_plan_step(true);
        // 单步**总**墙钟预算（与串行 runner 同一口径 `plan.step_timeout_secs`）：
        // 并行批此前完全没有超时保护——一个卡死的子步骤会永久占住并发额度，
        // 使整批永远收不了口（`join_next` 永不返回），前端进度条就此凝固。
        // 串行路径早已有这层保护，这里补齐对称性。
        let budget = step_total_timeout();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(budget);
        let t0 = std::time::Instant::now();
        let mut attempts = 0usize;
        let mut timed_out = false;
        let mut last_err = String::new();
        for attempt in 0..STEP_MAX_RETRIES {
            attempts = attempt + 1;
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                timed_out = true;
                last_err = format!("子步骤 {sid} 执行超出总预算 {budget}s，疑似远端无响应");
                break;
            }
            // M3-D2 成本归因 + N2 产物归属：dispatch 期间挂 RUN_ID / SESSION task-local。
            let dispatched = tokio::time::timeout(
                remaining,
                with_step_scope(&session_scope, &run_id, orch.dispatch(&step_task)),
            )
            .await;
            match dispatched {
                Ok(Ok(outcome)) => {
                    let summary: String = outcome.lead_reply.chars().take(120).collect();
                    return ParallelStepOutcome {
                        step_id: sid,
                        title: step_title,
                        ok: true,
                        summary,
                        elapsed_ms: t0.elapsed().as_millis() as u64,
                        attempts,
                        timed_out: false,
                    };
                }
                Ok(Err(e)) => {
                    last_err = e.to_string();
                    // 治理门/宪法/反思门禁是确定性裁决 → 绝不重试（重试无意义且烧额度）。
                    if !is_retryable_step_error(&e) || attempts >= STEP_MAX_RETRIES {
                        break;
                    }
                }
                Err(_elapsed) => {
                    timed_out = true;
                    last_err = format!("子步骤 {sid} 执行超出总预算 {budget}s，疑似远端无响应");
                    if attempts >= STEP_MAX_RETRIES {
                        break;
                    }
                }
            }
            tracing::warn!(
                run_id = %run_id, step = %sid, attempt = attempts, err = %last_err,
                "并行子步骤瞬时失败/超时，退避重试"
            );
            tokio::time::sleep(std::time::Duration::from_secs(attempts as u64)).await;
        }
        ParallelStepOutcome {
            step_id: sid,
            title: step_title,
            ok: false,
            summary: format!("执行失败：{last_err}"),
            elapsed_ms: t0.elapsed().as_millis() as u64,
            attempts,
            timed_out,
        }
    });
}

/// 并行子步骤终态 → 聚合结果 JSON（`plans.parallel.run` 的 `parallel_batch.results` 项）。
///
/// 抽出纯函数是为了让「回传真实耗时/尝试次数」这一前端契约有单测锁定——此前该
/// JSON 在异步执行体里内联构造，只能靠跑真实 LLM 才能验证，等于没有守卫。
fn parallel_result_json(
    step_id: &str,
    title: &str,
    ok: bool,
    summary: &str,
    elapsed_ms: u64,
    attempts: usize,
    timed_out: bool,
) -> Value {
    json!({
        "stepId": step_id,
        "title": title,
        "status": if ok { "completed" } else { "failed" },
        "summary": summary,
        // 真实耗时与尝试次数（前端可展示单步耗时/重试提示；无数据即不显示）
        "elapsedMs": elapsed_ms,
        "attempts": attempts,
        "timedOut": timed_out,
    })
}

/// 并行真执行（v19 真执行化）：`plans.parallel.run` 的后台执行体。
///
/// 对目标步骤（`step_ids` 缺省取全部 pending）**有界并发**经 Orchestrator 真实
/// 执行，逐步骤产出真实 summary；结果经 [`plan::parallel_apply`] 回填
/// （`executed=true`，绝不冒充），进度事件实时回流。失败步骤不中断兄弟任务
/// （与串行 runner 的「失败即终止」语义区分：并行扇出聚合全部结论后统一收口）。
pub async fn run_plan_steps_parallel(
    config: Config,
    run_id: String,
    session: String,
    bus: Arc<EventBus>,
    step_ids: Vec<String>,
) -> anyhow::Result<()> {
    use tokio::task::JoinSet;

    let Some(targets) = plan::parallel_snapshot(&run_id, &step_ids) else {
        anyhow::bail!("运行不存在：{run_id}");
    };
    if targets.is_empty() {
        anyhow::bail!("没有待并行执行的步骤");
    }
    // 整体目标上下文（与串行 runner 同源：剔除元信息的修订正文）
    let revision_id = plan::plans_checklist(&json!({ "runId": run_id }))
        .pointer("/run/planRevisionId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let (title, prompt) = plan::get_revision_view(&revision_id)
        .map(|rev| {
            let title = rev["title"].as_str().unwrap_or("").to_string();
            let prompt = plan_goal_context(&title, rev["markdown"].as_str().unwrap_or(""));
            (title, prompt)
        })
        .unwrap_or_default();

    // 运行级进度基数：并行批只覆盖部分步骤，因此进度必须以**整条运行**为分母，
    // 否则百分比会相对批次虚高。终态步数在批开始前取一次，批内每收口一步 +1。
    let (steps_total, mut steps_done) = {
        let view = plan::plans_checklist(&json!({ "runId": run_id }));
        let steps = view
            .pointer("/run/steps")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let done = steps
            .iter()
            .filter(|st| matches!(st["status"].as_str(), Some("completed" | "failed")))
            .count();
        (steps.len(), done)
    };
    let mut queue = targets.into_iter();
    let mut set: JoinSet<ParallelStepOutcome> = JoinSet::new();
    for _ in 0..PARALLEL_STEP_CONCURRENCY {
        if let Some(step) = queue.next() {
            spawn_step(
                &mut set,
                config.clone(),
                bus.clone(),
                session.clone(),
                run_id.clone(),
                title.clone(),
                prompt.clone(),
                step,
            );
        }
    }
    let mut results = Vec::new();
    let mut step_elapsed_hist: Vec<u64> = Vec::new();
    let batch_started = std::time::Instant::now();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(outcome) => {
                let ParallelStepOutcome {
                    step_id: sid,
                    title: step_title,
                    ok,
                    summary,
                    elapsed_ms,
                    attempts,
                    timed_out,
                } = outcome;
                crate::memory::behavior::record_behavior(
                    &session,
                    if ok {
                        "plan_parallel_step_done"
                    } else {
                        "plan_parallel_step_failed"
                    },
                    &format!("{sid} {step_title}"),
                );
                step_elapsed_hist.push(elapsed_ms);
                steps_done += 1;
                // C3 状态监控：并行批也发布**真实**进度/耗时/ETA（与串行同一事件契约，
                // 前端 PlanRunRibbon 无需分叉）。ETA 由已完成步骤实测耗时均值推算，
                // 无样本时为 0 → 前端降级为纯进度条，绝不虚报。
                let remaining = steps_total.saturating_sub(steps_done);
                let (avg_ms, eta_ms) = step_timing_estimate(&step_elapsed_hist, remaining);
                publish_plan(
                    &bus,
                    &session,
                    EventType::Progress,
                    json!({
                        "kind": "step_progress",
                        "stepId": sid,
                        "stepTitle": step_title,
                        "stepIndex": steps_done,
                        "stepsTotal": steps_total,
                        "percent": if steps_total > 0 { (steps_done as f64 / steps_total as f64 * 100.0) as u32 } else { 0 },
                        "phase": "finished",
                        "taskKind": crate::intent::Classifier::new()
                            .classify(&step_title)
                            .intent
                            .task_label(),
                        "stepElapsedMs": elapsed_ms,
                        "elapsedMs": batch_started.elapsed().as_millis() as u64,
                        "avgStepMs": avg_ms,
                        "etaMs": eta_ms,
                        "parallel": true,
                    }),
                );
                results.push(parallel_result_json(
                    &sid,
                    &step_title,
                    ok,
                    &summary,
                    elapsed_ms,
                    attempts,
                    timed_out,
                ));
            }
            Err(e) => tracing::warn!(run_id = %run_id, err = %e, "并行子步骤任务异常退出"),
        }
        if let Some(step) = queue.next() {
            spawn_step(
                &mut set,
                config.clone(),
                bus.clone(),
                session.clone(),
                run_id.clone(),
                title.clone(),
                prompt.clone(),
                step,
            );
        }
    }
    // 回填终态 + 广播（失败步骤不阻断聚合收口）
    if let Some(run) = plan::parallel_apply(&run_id, &results) {
        publish_plan(
            &bus,
            &session,
            EventType::PlanRun,
            json!({ "plan_run": run }),
        );
    }
    crate::gateway::chat::persist_session(&session);
    Ok(())
}

/// 取当前进行中（或下一个待执行）步骤的 (stepId, title)。
fn current_step(run_id: &str) -> Option<(String, String)> {
    let view = plan::plans_checklist(&json!({ "runId": run_id }));
    let run = view.get("run")?;
    let steps = run.get("steps")?.as_array()?;
    for st in steps {
        let status = st["status"].as_str().unwrap_or("");
        if status == "in_progress" {
            return Some((
                st["stepId"].as_str()?.to_string(),
                st["title"].as_str().unwrap_or("").to_string(),
            ));
        }
    }
    // 无 in_progress 时取第一个 pending（守护：状态机异常时仍可推进）
    for st in steps {
        if st["status"].as_str() == Some("pending") {
            return Some((
                st["stepId"].as_str()?.to_string(),
                st["title"].as_str().unwrap_or("").to_string(),
            ));
        }
    }
    None
}

/// 上下文压缩：assistant 回复已记录；告警「需压缩」时触发 4 层渐进压缩并广播事件。
fn compact_if_needed(session: &str, bus: &Arc<EventBus>) {
    if crate::context::session::context_status(session)["alert"] == "compact_required" {
        crate::context::session::context_compact(session, Some(bus));
    }
}

/// 发布 `session.event.plan_revision` / `session.event.plan_run`（payload 含
/// session_key/key，对齐 Web UI `payloadBelongsToSession` 归属判定）。
pub(crate) fn publish_plan(bus: &Arc<EventBus>, session: &str, ty: EventType, payload: Value) {
    let mut p = payload;
    if p.get("session_key").is_none() {
        p["session_key"] = json!(session);
        p["key"] = json!(session);
    }
    bus.publish(Envelope::new(ty, p, session));
}

/// C3 状态监控：取消/终止计划运行并沿事件总线广播 PlanRun 终态。
///
/// 任意触发（max_steps / goal 护栏 / 步骤失败）统一走此通道，保证计划终态
/// 一定通过 `session.event.plan_run` 广播，Web/GUI 状态流与 core 实际一致。
fn cancel_and_broadcast(bus: &Arc<EventBus>, session: &str, run_id: &str) {
    if let Some(r) = plan::plans_cancel(&json!({ "runId": run_id })).get("run") {
        publish_plan(
            bus,
            session,
            EventType::PlanRun,
            json!({ "plan_run": r.clone() }),
        );
    }
}

// ── R65/B1：计划失败的结构化错误帧 ───────────────────────────────────────────
// 这四处 `Envelope::error_structured` 调用点此前**没有任何测试覆盖**（变异自检证明：
// 把正文退回内部串、或退回 `Envelope::error`，`cargo test` 全绿）。这里把「稳定码 +
// 中性正文 + source/step_id/step_title 上下文」抽成三个**纯函数**，让调用点必须经过
// 它们，从而可被单测直接锁定语义（见 `b1_*` 用例）。

/// 步骤执行失败：内层已归类则沿用 provider 稳定码（如 `AUTH_FAILED`），否则回退
/// `PLAN_STEP_FAILED`；正文固定中性英文，**绝不回显内部术语**。
fn plan_step_failed_error(session: &str, sid: &str, title: &str, err: &anyhow::Error) -> Envelope {
    Envelope::error_structured(
        session,
        crate::model::error_taxonomy::plan_step_code(err),
        &format!("Plan step {sid} failed."),
        json!({ "source": "plan_runner", "step_id": sid, "step_title": title }),
    )
}

/// 步骤声明了产物却未真实落盘：稳定码 + 缺失清单。
fn plan_step_artifact_missing_error(
    session: &str,
    sid: &str,
    title: &str,
    missing: &[String],
) -> Envelope {
    Envelope::error_structured(
        session,
        crate::model::error_taxonomy::PLAN_STEP_ARTIFACT_MISSING,
        &format!("Plan step {sid} did not produce its declared artifact(s)."),
        json!({
            "source": "plan_runner",
            "step_id": sid,
            "step_title": title,
            "missing": missing,
        }),
    )
}

/// 完成门禁未放行：稳定码 + 中性正文（具体警告已并入正文，不再回显内部术语）。
fn plan_completion_blocked_error(session: &str) -> Envelope {
    Envelope::error_structured(
        session,
        crate::model::error_taxonomy::PLAN_COMPLETION_BLOCKED,
        "The plan finished but did not pass the completion gate.",
        json!({ "source": "plan_runner" }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBus;

    // ── R65/B1 守卫：计划失败的结构化错误帧 ──────────────────────────────────
    // 变异自检锚点：把 `plan_step_failed_error` 的正文退回内部串（如
    // `计划子步骤 {sid} 执行失败：{e}`）→ 本用例红；或把它退回 `Envelope::error`
    // （丢掉 source/step_id/step_title）→ 本用例红。

    /// 内层已归类 → 沿用 provider 码；正文不含任何内部术语；上下文齐全。
    #[test]
    fn b1_plan_step_failed_error_is_classified_and_term_free() {
        let err = anyhow::Error::new(
            crate::model::error_taxonomy::ProviderError::from_status(
                401,
                "AUTH_FAILED: The model provider rejected the API key (HTTP 401)",
            ),
        );
        let env = plan_step_failed_error("sess", "step-1", "理解需求与约束", &err);

        assert_eq!(env.r#type, EventType::Error);
        assert_eq!(env.payload["code"].as_str(), Some("AUTH_FAILED"));
        assert_eq!(env.payload["source"].as_str(), Some("plan_runner"));
        assert_eq!(env.payload["step_id"].as_str(), Some("step-1"));
        assert_eq!(env.payload["step_title"].as_str(), Some("理解需求与约束"));

        let msg = env.payload["message"].as_str().unwrap_or("");
        assert_eq!(msg, "Plan step step-1 failed.");
        for term in [
            "AUTH_FAILED",
            "401",
            "HTTP",
            "api key",
            "API key",
            "计划子步骤",
            "rejected",
        ] {
            assert!(
                !msg.contains(term),
                "用户可见正文不得回显内部术语 `{term}`：{msg}"
            );
        }
    }

    /// 未归类错误 → 回退 `PLAN_STEP_FAILED`，但仍携带结构化上下文（正文不回显内部细节）。
    #[test]
    fn b1_plan_step_failed_error_falls_back_and_keeps_context() {
        let err = anyhow::anyhow!("计划子步骤 step-2 执行失败：内部细节 secret-stack");
        let env = plan_step_failed_error("sess", "step-2", "实施与产出", &err);

        assert_eq!(env.payload["code"].as_str(), Some("PLAN_STEP_FAILED"));
        assert_eq!(env.payload["source"].as_str(), Some("plan_runner"));
        assert_eq!(env.payload["step_id"].as_str(), Some("step-2"));
        assert_eq!(env.payload["step_title"].as_str(), Some("实施与产出"));
        assert!(
            !env.payload["message"]
                .as_str()
                .unwrap_or("")
                .contains("secret-stack"),
            "正文不得回显内部错误串"
        );
    }

    /// 产物缺失 / 完成门禁阻断 → 稳定码 + 结构化上下文（正文中性）。
    #[test]
    fn b1_artifact_and_completion_gate_errors_are_structured() {
        let missing = vec!["output.html".to_string()];
        let env = plan_step_artifact_missing_error("sess", "step-3", "生成 HTML 网页", &missing);
        assert_eq!(
            env.payload["code"].as_str(),
            Some("PLAN_STEP_ARTIFACT_MISSING")
        );
        assert_eq!(env.payload["source"].as_str(), Some("plan_runner"));
        assert_eq!(env.payload["step_id"].as_str(), Some("step-3"));
        assert_eq!(env.payload["step_title"].as_str(), Some("生成 HTML 网页"));
        assert_eq!(env.payload["missing"][0].as_str(), Some("output.html"));
        assert_eq!(
            env.payload["message"].as_str(),
            Some("Plan step step-3 did not produce its declared artifact(s).")
        );

        let env2 = plan_completion_blocked_error("sess");
        assert_eq!(env2.payload["code"].as_str(), Some("PLAN_COMPLETION_BLOCKED"));
        assert_eq!(env2.payload["source"].as_str(), Some("plan_runner"));
        assert_eq!(
            env2.payload["message"].as_str(),
            Some("The plan finished but did not pass the completion gate.")
        );
    }

    /// P1-2：计划子步骤的固定输出结构约束必须包含全部固定小节标题与固定表格列名，
    /// 否则「稳定格式」无从保证（回归守卫）。
    #[test]
    fn plan_substep_output_contract_pins_sections_and_columns() {
        let c = PLAN_SUBSTEP_OUTPUT_CONTRACT;
        for section in ["## 结论", "## 依据", "## 产出", "## 风险与假设"] {
            assert!(c.contains(section), "固定输出结构缺少小节 {section}: {c}");
        }
        assert!(
            c.contains("| 项目 | 内容 | 依据 |"),
            "固定输出结构缺少固定表格列名: {c}"
        );
        // 明确禁止自创小节，避免「碰运气」的格式漂移。
        assert!(c.contains("不得"), "约束应显式禁止自创小节: {c}");
    }

    /// 停止原因文案：任何内部枚举都不得原样出现在用户可见的回复里
    /// （此前的「计划执行结束（not_armed）。」就是原样泄露）。
    #[test]
    fn stopped_reason_text_never_leaks_internal_enum() {
        use crate::goal::runner_guard::GoalStopReason;
        let reasons = [
            "completed",
            "failed",
            "max_steps",
            "unknown_reason",
            GoalStopReason::MissingGoal.as_str(),
            GoalStopReason::StaleRevision.as_str(),
            GoalStopReason::Paused.as_str(),
            GoalStopReason::Blocked.as_str(),
            GoalStopReason::Complete.as_str(),
            GoalStopReason::NotArmed.as_str(),
            GoalStopReason::MaxGoalRounds.as_str(),
        ];
        for reason in reasons {
            let text = stopped_reason_text(reason);
            assert!(text.ends_with('。'), "{reason} 应是完整句子：{text}");
            assert!(
                !text.contains(reason),
                "文案不得含内部枚举 {reason}：{text}"
            );
        }
    }

    /// C3 状态监控：多端共享总线下，PlanRun 状态从发布到另一端订阅者
    /// 收到的时间必须 <200ms（承接 UNIFIED-WORKBENCH 多端同步 200ms 断言）。
    #[tokio::test]
    async fn c3_plan_run_event_reaches_subscriber_under_200ms() {
        let bus = Arc::new(EventBus::default());
        // 订阅端模拟"另一端"（GUI/Web 任一），只订阅本 session 的 plan_run 路由
        // （EventBus route_key = `{session}.{type}` => `sess.plan_run`）。
        let mut rx = bus.subscribe("sess.plan_run");
        let t0 = std::time::Instant::now();
        publish_plan(
            &bus,
            "sess",
            EventType::PlanRun,
            json!({ "plan_run": { "runId": "r1", "status": "running" } }),
        );
        let env = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("应收到 PlanRun 事件")
            .expect("channel 不应关闭");
        assert!(
            t0.elapsed().as_millis() < 200,
            "发布→订阅延迟应 <200ms，实际 {}ms",
            t0.elapsed().as_millis()
        );
        assert_eq!(env.r#type, EventType::PlanRun);
        assert_eq!(
            env.payload["plan_run"]["status"].as_str(),
            Some("running")
        );
    }

    /// A 组剩余项回归：plan 路径的工具开销必须计入上下文令牌台账。
    ///
    /// 计划路径事件走**网关总线**、不经过 gateway 私总线桥，故此前完全漏记
    /// （`context_status` 偏低 → 压缩触发过晚）。
    #[tokio::test]
    async fn ledger_tap_records_plan_path_tool_overhead() {
        let bus = Arc::new(EventBus::default());
        let session = format!("sess-tap-{}", uuid::Uuid::new_v4());
        let _tap = LedgerTap::spawn(&bus, &session);

        let before = crate::context::session::context_status(&session)["overhead_tokens"]
            .as_u64()
            .unwrap_or(0);

        bus.publish(Envelope::new(
            EventType::ToolCall,
            json!({"name": "read_file", "input": {"path": "src/main.rs"}}),
            session.clone(),
        ));
        bus.publish(Envelope::new(
            EventType::ToolResult,
            json!({"name": "read_file", "result": "fn main() { /* 工具输出 */ }"}),
            session.clone(),
        ));

        // 有界轮询等待转发任务处理（避免脆弱的长 sleep）
        let mut after = before;
        for _ in 0..100 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            after = crate::context::session::context_status(&session)["overhead_tokens"]
                .as_u64()
                .unwrap_or(0);
            if after > before {
                break;
            }
        }
        assert!(
            after > before,
            "plan 路径工具开销应计入台账（before={before}, after={after}）"
        );
    }

    /// C3 状态监控：取消计划运行后，总线必须广播 PlanRun 终态（cancelled），
    /// 使未发起取消的另一端也能在 200ms 内收敛到一致状态。
    #[tokio::test]
    async fn c3_cancel_broadcasts_terminal_plan_run_event() {
        let bus = Arc::new(EventBus::default());
        let mut rx = bus.subscribe("sess.plan_run");
        // 走 RPC 同路径创建并启动 run（全局 store），随后通过
        // cancel_and_broadcast 模拟 max_steps/护栏/失败统一终止通道。
        let created = plan::plans_create(&json!({ "prompt": "帮我做一个完整的 FPS 射击游戏" }));
        let rev_id = created["revision"]["revisionId"]
            .as_str()
            .expect("应产出规划修订")
            .to_string();
        let started = plan::plans_run(&json!({ "revisionId": rev_id }));
        let run_id = started["run"]["runId"]
            .as_str()
            .expect("应产出运行实例")
            .to_string();
        // 消费掉 run 启动时可能混入 bus 的事件，仅统计取消广播这一跳。
        while tokio::time::timeout(
            std::time::Duration::from_millis(50),
            rx.recv(),
        )
        .await
        .map(|o| o.is_some())
        .unwrap_or(false)
        {}
        let t0 = std::time::Instant::now();
        cancel_and_broadcast(&bus, "sess", &run_id);
        let env = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("取消后应收到 PlanRun 终态事件")
            .expect("channel 不应关闭");
        assert!(
            t0.elapsed().as_millis() < 200,
            "取消状态同步延迟应 <200ms，实际 {}ms",
            t0.elapsed().as_millis()
        );
        assert_eq!(
            env.payload["plan_run"]["status"].as_str(),
            Some("cancelled")
        );
    }

    #[test]
    fn plan_step_context_strips_strategy_meta_lines() {
        // 用户实测触发治理门 C1 误判的方案 markdown：决策理由含「覆盖」。
        let md = "# 计划：帮我做一个fps射击游戏\n\n**编排形态**: Solo Agent\n\n**决策理由**: 单一意图（Unknown），单 Agent 循环即可覆盖（标准 AI 助手 基线等价）\n\n1. 理解需求与约束\n2. 实施与产出\n3. 验证与收尾\n";
        let ctx = plan_step_context(md);
        // 元信息行被剔除，且不再携带会命中宪法 C1 破坏性关键词的「覆盖」
        assert!(!ctx.contains("**编排形态**"));
        assert!(!ctx.contains("**决策理由**"));
        assert!(!ctx.contains("覆盖"));
        // 方案正文保留：标题 + 步骤列表
        assert!(ctx.contains("# 计划：帮我做一个fps射击游戏"));
        assert!(ctx.contains("1. 理解需求与约束"));
        assert!(ctx.contains("3. 验证与收尾"));
        // 破坏性关键词信号全部消失（C1 前置裁决据此放行）
        for keyword in [
            "删除",
            "覆盖",
            "force",
            "强制",
            "格式化",
            "format",
            "shutdown",
            "重启",
        ] {
            assert!(
                !ctx.to_lowercase().contains(keyword),
                "误命中破坏性关键词: {keyword}"
            );
        }
    }

    #[test]
    fn plan_step_context_keeps_plain_markdown_unchanged() {
        let md = "# 计划：测试\n\n1. 写代码\n2. 跑测试\n";
        let ctx = plan_step_context(md);
        assert_eq!(ctx.trim(), "# 计划：测试\n\n1. 写代码\n2. 跑测试");
    }

    #[test]
    fn short_input_never_decomposes() {
        let cfg = Config::default();
        assert!(!should_auto_decompose(&cfg, "你好"));
    }

    #[test]
    fn auto_decompose_off_disables() {
        let mut cfg = Config::default();
        cfg.plan.auto_decompose = false;
        assert!(!should_auto_decompose(&cfg, "请用 Rust 实现一个完整的 HTTP 服务器，包含路由、中间件、静态文件、TLS 与压力测试，并撰写架构文档与性能报告。"));
    }

    #[test]
    fn complex_coding_task_triggers_decompose() {
        let cfg = Config::default();
        // 编程类多阶段复杂任务（命中 Complex 分类）
        let text = "请从零构建一个 Rust 命令行编译器项目：先做词法分析、再做语法分析、语义检查、中间代码生成、目标代码优化，最后用基准测试验证正确性并撰写完整文档。";
        assert!(
            should_auto_decompose(&cfg, text),
            "复杂编程任务应触发自动拆解"
        );
    }

    #[test]
    fn short_project_request_triggers_decompose() {
        let cfg = Config::default();
        // 用户实测场景：17 字符的项目型诉求（此前被 20 字符门槛误拦）
        let text = "帮我做一个完善的FPS第一人称射击游戏";
        assert!(
            should_auto_decompose(&cfg, text),
            "短项目诉求（≥12 字符）应触发自动拆解"
        );
    }

    #[test]
    fn terse_fix_request_skips_decompose() {
        let cfg = Config::default();
        // 简短修复诉求（<12 字符）：不拆解，避免多步噪音
        let text = "帮我修个bug";
        assert!(
            !should_auto_decompose(&cfg, text),
            "简短修复诉求不应触发拆解"
        );
    }

    #[test]
    fn compound_deliverable_task_triggers_decompose() {
        // 计划层漏判修复：复合交付物（检索 → HTML → PPT）必须触发自动拆解。
        let cfg = Config::default();
        let text = "搜索今天的AI新闻，并整理成html网页再成PPT给我看";
        assert!(
            should_auto_decompose(&cfg, text),
            "复合交付物任务应触发自动拆解（修复计划层漏判）"
        );
    }

    #[test]
    fn publish_plan_attaches_session_identity() {
        let bus = Arc::new(EventBus::default());
        let session = "ws.plan-test";
        let mut rx = bus.subscribe(&format!("{session}.*"));
        publish_plan(
            &bus,
            session,
            EventType::PlanRevision,
            json!({ "plan_revision": { "revisionId": "rev-1", "planId": "plan-1", "title": "t", "markdown": "m", "steps": [], "current": true } }),
        );
        let env = rx.try_recv().expect("应收到事件");
        assert_eq!(env.r#type, EventType::PlanRevision);
        assert_eq!(env.payload["session_key"], "ws.plan-test");
        assert_eq!(env.payload["key"], "ws.plan-test");
    }

    #[tokio::test]
    async fn run_autonomous_rejects_simple_task() {
        let cfg = Config::default();
        let bus = Arc::new(EventBus::default());
        let err = run_autonomous(
            cfg,
            "简单问题".to_string(),
            "ws.x".to_string(),
            bus,
            String::new(),
            None,
            ModeKind::Agent,
        )
        .await
        .expect_err("简单任务应被拒绝");
        assert!(err.to_string().contains("仅用于复杂/项目型任务"));
    }

    #[test]
    fn retryable_error_classification_is_honest() {
        // 瞬时错误可重试（无 provider 归类时按 Unknown 放行）
        assert!(is_retryable_step_error(&anyhow::anyhow!("连接超时 timeout")));
        assert!(is_retryable_step_error(&anyhow::anyhow!("502 Bad Gateway")));
        assert!(is_retryable_step_error(&anyhow::anyhow!(
            "Chat completion failed: 网络错误"
        )));
        // 确定性裁决不可重试（重试无意义且烧额度）
        assert!(
            !is_retryable_step_error(&anyhow::anyhow!(
                "治理门裁决未通过（违反：C1），需人工放行"
            )),
            "治理门拦截不可重试"
        );
        assert!(!is_retryable_step_error(&anyhow::anyhow!(
            "宪法违规：破坏性操作"
        )));
        assert!(
            !is_retryable_step_error(&anyhow::anyhow!(format!(
                "{} 反思门禁失败",
                crate::orchestration::REFLECT_GATE_MARKER
            ))),
            "反思门禁失败不可重试"
        );
    }

    /// T05 行为终点（真实调用点 `is_retryable_step_error`）：外层 context 含「治理门」
    /// 字样的**瞬时网络错误**必须可重试——旧实现的子串嗅探（只看 e.to_string()）
    /// 会把最外层 context 里的「治理门」当成确定性裁决，误判不可重试。
    #[test]
    fn transient_error_mentioning_governance_gate_is_retryable() {
        let transient = anyhow::Error::new(
            crate::model::error_taxonomy::ProviderError::new(
                crate::model::error_taxonomy::ProviderErrorClass::Network,
                "raw transport failure",
            ),
        )
        .context("讨论「治理门」设计时网络中断（timeout）");
        assert!(transient.to_string().contains("治理门"));
        assert!(
            is_retryable_step_error(&transient),
            "含「治理门」字样的瞬时网络错误必须可重试（旧实现误判不可重试）"
        );
    }

    /// P2-12：`step_total_timeout` 的配置读取走进程级缓存——第二次调用不得再
    /// `Config::load()`（旧实现每步都解析配置文件）。
    #[test]
    fn step_total_timeout_reads_config_through_process_cache() {
        use std::sync::atomic::Ordering;
        let _ = step_total_timeout();
        let after_first = PLAN_CONFIG_LOADS.load(Ordering::Relaxed);
        let _ = step_total_timeout();
        let after_second = PLAN_CONFIG_LOADS.load(Ordering::Relaxed);
        assert_eq!(
            after_second, after_first,
            "第二次 step_total_timeout 不得再次 Config::load()（缓存未生效）"
        );
        // 返回值必须落在合法域内（配置 > 0 用配置，否则默认 120）。
        let t = step_total_timeout();
        assert!(t > 0, "step_total_timeout 必须为正的墙钟预算");
    }

    /// N2 行为终点：计划步骤 dispatch 的作用域必须把产物登记到**本次会话**。
    ///
    /// 以真实的登记调用形态（`record_artifact(current_session(), path, kind)`）+ 真实
    /// 台账读取（`list_artifacts`，即 `artifacts.list` 的数据源）验证：作用域内产出的
    /// 文件出现在该会话的产物列表里；作用域缺失时（旧实现）产物落到默认会话、列表为空。
    #[tokio::test]
    async fn with_step_scope_attributes_artifacts_to_the_run_session() {
        let session = format!("sess-n2-{}", uuid::Uuid::new_v4());
        let run_id = "run-n2";
        let dir = std::env::temp_dir().join(format!("marguerite-n2-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("output.html");
        std::fs::write(&file, b"<h1>ok</h1>").unwrap();
        let path = file.to_string_lossy().to_string();

        crate::tools::file_change::clear_artifacts(&session);
        crate::tools::file_change::clear_artifacts("");

        // 与 fs.rs 写文件后完全相同的登记形态：会话取自 SESSION task-local。
        with_step_scope(&session, run_id, async {
            let sess = crate::usage::SESSION
                .try_with(|s| s.clone())
                .unwrap_or_default();
            crate::tools::file_change::record_artifact(&sess, &path, "created");
        })
        .await;

        let (items, _, _) = crate::tools::file_change::list_artifacts(&session, None, 50);
        assert!(
            items.iter().any(|a| a.path == path),
            "计划步骤产出的文件必须登记到本次会话（N2），实际: {:?}",
            items.iter().map(|a| a.path.clone()).collect::<Vec<_>>()
        );

        crate::tools::file_change::clear_artifacts(&session);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ETA 口径守卫：无完成样本时**必须**为 (0,0)（前端据此降级为纯进度条，
    /// 绝不虚报剩余时间）；有样本时按真实均值 × 剩余步数推算。
    #[test]
    fn step_timing_estimate_never_fabricates_without_samples() {
        assert_eq!(step_timing_estimate(&[], 5), (0, 0), "无样本不得虚报 ETA");
        assert_eq!(step_timing_estimate(&[], 0), (0, 0));
    }

    #[test]
    fn step_timing_estimate_uses_real_average() {
        // 已完成两步 1s / 3s → 均值 2s；剩 3 步 → ETA 6s
        assert_eq!(step_timing_estimate(&[1000, 3000], 3), (2000, 6000));
        // 剩余 0 步 → ETA 归零，但均值仍如实给出（前端展示单步均值）
        assert_eq!(step_timing_estimate(&[1000, 3000], 0), (2000, 0));
    }

    /// 极端样本不得 panic（整数溢出在 release 下会回绕成荒谬的小 ETA）。
    #[test]
    fn step_timing_estimate_saturates_on_huge_samples() {
        let (avg, eta) = step_timing_estimate(&[u64::MAX], usize::MAX);
        assert_eq!(avg, u64::MAX);
        assert_eq!(eta, u64::MAX, "饱和而非回绕");
    }

    /// 前端契约守卫：并行聚合项必须带真实耗时/尝试次数/超时标记，
    /// 且 `ok` → status 的映射不得反向（失败绝不能显示成 completed）。
    #[test]
    fn parallel_result_carries_real_timing_and_honest_status() {
        let ok = parallel_result_json("s1", "编码", true, "已完成：编码", 1234, 1, false);
        assert_eq!(ok["status"], "completed");
        assert_eq!(ok["elapsedMs"], 1234);
        assert_eq!(ok["attempts"], 1);
        assert_eq!(ok["timedOut"], false);
        assert_eq!(ok["stepId"], "s1");

        let failed = parallel_result_json("s2", "测试", false, "执行失败：超时", 900, 3, true);
        assert_eq!(failed["status"], "failed", "失败不得被标成 completed");
        assert_eq!(failed["attempts"], 3, "重试次数须如实回传");
        assert_eq!(failed["timedOut"], true, "超时须与远端报错区分");
    }

    /// 每调用唯一的临时工作区（避免并行测试共享目录竞态）。
    fn tmp_workspace(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "mgt_plan_gate_{}_{}_{}",
            std::process::id(),
            tag,
            seq
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// 构造一个含必需条目的最小 `.pptx`（未压缩 ZIP）字节串。
    ///
    /// 完成门禁现在会校验办公交付物的**容器真实性**（缺陷 E1），因此测试里
    /// 不能再拿纯文本冒充 `presentation.pptx`。
    fn minimal_pptx_bytes() -> Vec<u8> {
        let names = ["[Content_Types].xml", "ppt/presentation.xml"];
        let mut out = Vec::new();
        let mut offsets = Vec::new();
        for name in names {
            offsets.push(out.len() as u32);
            out.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]); // local file header
            out.extend_from_slice(&20u16.to_le_bytes()); // version needed
            out.extend_from_slice(&0u16.to_le_bytes()); // flags
            out.extend_from_slice(&0u16.to_le_bytes()); // compression = stored
            out.extend_from_slice(&0u16.to_le_bytes()); // mod time
            out.extend_from_slice(&0u16.to_le_bytes()); // mod date
            out.extend_from_slice(&0u32.to_le_bytes()); // crc32
            out.extend_from_slice(&0u32.to_le_bytes()); // compressed size
            out.extend_from_slice(&0u32.to_le_bytes()); // uncompressed size
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // extra len
            out.extend_from_slice(name.as_bytes());
        }
        let cd_start = out.len() as u32;
        for (i, name) in names.iter().enumerate() {
            out.extend_from_slice(&[0x50, 0x4B, 0x01, 0x02]); // central directory
            out.extend_from_slice(&20u16.to_le_bytes()); // version made by
            out.extend_from_slice(&20u16.to_le_bytes()); // version needed
            out.extend_from_slice(&0u16.to_le_bytes()); // flags
            out.extend_from_slice(&0u16.to_le_bytes()); // compression
            out.extend_from_slice(&0u16.to_le_bytes()); // mod time
            out.extend_from_slice(&0u16.to_le_bytes()); // mod date
            out.extend_from_slice(&0u32.to_le_bytes()); // crc32
            out.extend_from_slice(&0u32.to_le_bytes()); // compressed size
            out.extend_from_slice(&0u32.to_le_bytes()); // uncompressed size
            out.extend_from_slice(&(name.len() as u16).to_le_bytes()); // name len
            out.extend_from_slice(&0u16.to_le_bytes()); // extra len
            out.extend_from_slice(&0u16.to_le_bytes()); // comment len
            out.extend_from_slice(&0u16.to_le_bytes()); // disk start
            out.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            out.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            out.extend_from_slice(&offsets[i].to_le_bytes()); // local header offset
            out.extend_from_slice(name.as_bytes());
        }
        let cd_size = out.len() as u32 - cd_start;
        out.extend_from_slice(&[0x50, 0x4B, 0x05, 0x06]); // EOCD
        out.extend_from_slice(&0u16.to_le_bytes()); // disk number
        out.extend_from_slice(&0u16.to_le_bytes()); // cd start disk
        out.extend_from_slice(&(names.len() as u16).to_le_bytes()); // entries this disk
        out.extend_from_slice(&(names.len() as u16).to_le_bytes()); // total entries
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_start.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // comment len
        out
    }

    /// Gap1：基准句解析出契约声明的必需产物（HTML + PPT）。
    #[test]
    fn required_artifacts_for_baseline_lists_html_and_pptx() {
        let required = required_artifacts_for("搜索今天的AI新闻，并整理成html网页再成PPT给我看");
        assert!(
            required.iter().any(|p| p == "output.html"),
            "必需产物应含 output.html，实际 {required:?}"
        );
        assert!(
            required.iter().any(|p| p == "presentation.pptx"),
            "必需产物应含 presentation.pptx，实际 {required:?}"
        );
        // 非复合任务无契约 → 无必需产物（不误伤普通复杂任务）
        assert!(required_artifacts_for("帮我做一个完善的FPS第一人称射击游戏").is_empty());
    }

    /// Gap3：步骤标题里声明的产物路径被精确提取（供步骤级完成核验）。
    #[test]
    fn step_target_paths_extracts_declared_artifacts() {
        let t = "[HTML] 生成 HTML 网页（output.html）。";
        let paths = step_target_paths(t);
        assert_eq!(paths, vec!["output.html".to_string()]);
        let t2 = "[PPT] 生成 PPT（presentation.pptx）。";
        assert_eq!(step_target_paths(t2), vec!["presentation.pptx".to_string()]);
        // 无产物路径的步骤（理解需求 / 检索）不施加约束
        assert!(step_target_paths("理解需求与约束").is_empty());
        assert!(step_target_paths("[检索] 围绕「今天的AI新闻」进行联网检索，汇总可信来源与要点。").is_empty());
        // 版本号类 token（无已知扩展名）不误当产物
        assert!(step_target_paths("升级到 v2.0 并兼容 1.5").is_empty());
    }

    /// Gap1：计划路径完成门禁对「契约声明的必需产物」做落盘强约束——
    /// 产物缺失 → MissingArtifact 阻断（终态 partial）；产物落盘 → 放行（completed）。
    #[test]
    fn plan_completion_gate_blocks_on_missing_required_artifact() {
        let ws = tmp_workspace("gate");
        let gate = CompletionGate::new();
        let shell: Arc<dyn ShellRunner> = Arc::new(RealShellRunner);
        let prompt = "搜索今天的AI新闻，并整理成html网页再成PPT给我看";
        // 足够长的聚合结论（避免 EmptyResult 干扰），但未声明任何相对路径产物。
        let reply = "第 1 步：已完成 AI 新闻检索，汇总了若干可信来源与要点，供后续整理使用。\
                     第 2 步：已整理为网页内容。第 3 步：已整理为演示内容。";
        let required = required_artifacts_for(prompt);
        let refs: Vec<&str> = required.iter().map(String::as_str).collect();
        let missing = gate.evaluate(prompt, reply, &refs, &ws, &shell, None);
        assert!(
            !missing.allowed,
            "必需产物缺失时不得放行（这正是「4 秒已完成却无交付物」的修复）"
        );
        assert!(
            missing.blocked_by.contains(
                &crate::governance::completion_gate::BlockReason::MissingArtifact
            ),
            "应以 MissingArtifact 阻断，实际 {:?}",
            missing.blocked_by
        );

        // 产物真实落盘（非空 + 合法容器）后 → 放行。
        std::fs::write(ws.join("output.html"), "<html>news</html>").unwrap();
        std::fs::write(ws.join("presentation.pptx"), minimal_pptx_bytes()).unwrap();
        let ok = gate.evaluate(prompt, reply, &refs, &ws, &shell, None);
        assert!(ok.allowed, "必需产物全部落盘且非空应放行：{:?}", ok.warnings);
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// Gap1/Gap3：步骤级产物核验——标题声明了产物但文件缺失 → 该步不得判完成。
    #[test]
    fn step_level_gate_rejects_missing_declared_artifact() {
        let ws = tmp_workspace("step");
        let gate = CompletionGate::new();
        let title = "[HTML] 生成 HTML 网页（output.html）。";
        let targets = step_target_paths(title);
        assert_eq!(targets.len(), 1);
        let missing: Vec<String> = targets
            .iter()
            .filter(|p| !gate.verify_required_artifacts(&[p.as_str()], &ws).all_ok())
            .cloned()
            .collect();
        assert_eq!(missing, vec!["output.html".to_string()], "缺失产物应被识别");
        // 落盘后不再算缺失 → 步骤可判完成
        std::fs::write(ws.join("output.html"), "<html>x</html>").unwrap();
        assert!(
            gate.verify_required_artifacts(&["output.html"], &ws).all_ok(),
            "产物落盘后步骤级核验应通过"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// Gap2：计划路径的子步骤编排形态由 mode_route 偏置，五档映射下
    /// `bias_strategy(Solo)` 至少产出 **4 个互异** 形态标签（Plan/Agent 同为 Solo，
    /// 但 Plan 档由写工具硬拒绝与 Agent 区分）。
    #[test]
    fn plan_step_shape_is_mode_biased_four_distinct() {
        let labels: Vec<&str> = ModeKind::ALL
            .iter()
            .map(|k| mode_route(*k).bias_strategy(AgentMode::Solo).label())
            .collect();
        let mut uniq = labels.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert!(
            uniq.len() >= 4,
            "五档模式的计划子步骤形态至少应有 4 个互异标签，实际 {labels:?}"
        );
        // 显式锚定每档
        assert_eq!(
            mode_route(ModeKind::Plan).bias_strategy(AgentMode::Solo),
            AgentMode::Solo
        );
        assert_eq!(
            mode_route(ModeKind::Agent).bias_strategy(AgentMode::Solo),
            AgentMode::Solo
        );
        assert_eq!(
            mode_route(ModeKind::Ensemble).bias_strategy(AgentMode::Solo),
            AgentMode::ExpertTeam,
            "Ensemble(MultiCandidate) 必须体现多 Agent 强编排"
        );
        assert_eq!(
            mode_route(ModeKind::Bloom).bias_strategy(AgentMode::Solo),
            AgentMode::DualLeader
        );
        assert_eq!(
            mode_route(ModeKind::OmniTranscend).bias_strategy(AgentMode::Solo),
            AgentMode::Cluster
        );
    }
}
