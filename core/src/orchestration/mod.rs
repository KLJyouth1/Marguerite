//! 自动编排 dispatcher（C 期）
//!
//! 这是「比 标准 AI 助手 强数倍」的编排内核：给定用户一句话，agent **自动**
//! 决定用哪种编排形态（Solo / MAX 双带队 / 专家团队 / 集群），并把任务分解为
//! 角色专精的子 Agent 并行或顺序执行，最后由 lead 综合成最终答复。
//!
//! 设计约束（来自用户指令）：
//! - **零用户编排 UI**：前端不得提供任何让用户手动选择编排形态 / 操轮子 Agent 的界面；
//!   形态完全由本模块的 [`Orchestrator::analyze`] 规则引擎自动决策。
//! - 复用既有原语（[`crate::agent::runtime::AgentRuntime`] / [`crate::agent::subagent::SubAgentManager`] /
//!   [`crate::model::router::ModelRouter`] / [`crate::intent::classifier::Classifier`]），不重复造轮子。
//! - [`Orchestrator::analyze`] 为纯规则分析，**不调用 LLM**，可在离线 / 无 API Key 环境下运行，
//!   供前端只读展示「agent 将为这条输入选择何种编排」。

pub mod dag;

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::agent::runtime::{AgentRuntime, AgentState};
use crate::agent::stream_loop::AgentEvent;
use crate::agent::subagent::{SubAgentConfig, SubAgentManager};
use crate::agent::team::Team;
use crate::agent::tool_cycle::ToolResult;
use crate::automation::{Automation, AutomationManager, JobKind};
use crate::domain::AgentMode;
use crate::events::{EnsembleProgress, Envelope, EventBus, EventType};
use crate::governance::completion_gate::{
    CompletionClaim, CompletionGate, CompletionVerdict, RealShellRunner, ShellRunner,
};
use crate::governance::{ActionIntent, GateDecision, PlanGate, ProposedPlan};
use crate::intent::classifier::{Classifier, IntentType as ClassifierIntent};
use crate::mcp::registry::ToolRegistry;
use crate::mode::ModeKind;
use crate::model::llm::{LLMClient, LLMConfig, LLMProvider, ToolDef};
use crate::model::router::{Complexity, IntentType, ModelRouter};
use crate::model::StreamReceiver;
use crate::persistence::config::Config;
#[cfg(test)]
use crate::research::MockResearchLlm;
use crate::research::{ResearchCapability, ResearchLlm, ResearchPipeline};
#[cfg(test)]
use crate::serve::acp::listener::start_listener;
use crate::serve::acp::session::AgentInvoker;
#[cfg(test)]
use crate::serve::acp::AcpSession;
use crate::tools::fs_path::safe_resolve;
use crate::types::Message;
use crate::vision::{
    LlmVisionTransport, VisionCapability, VisionProviderResolver, VisionTransport,
};
#[cfg(test)]
use crate::vision::{MockVisionResolver, MockVisionTransport};

/// Complex 路径 fan-out 默认分片数（「上千 subagent」的可配置起点）。
/// 每个分片对应一个并行「编码实现」子代理；受 [`SubAgentConfig::max_concurrent`] 并发上限约束。
const DEFAULT_FANOUT: usize = 6;
/// Bloom 蜂群子模式：FlashSwarm 蜂群分片上限（bloom:swarm max_swarm_shards）。
const SWARM_SHARDS: usize = 11;
/// Bloom 蜂群子模式：ProExpert 专家团队子专家上限（bloom:swarm max_sub_experts）。
const SWARM_SUB_EXPERTS: usize = 5;
/// Bloom 自主闭环子模式：最大迭代轮数。
const AUTO_MAX_ITERATIONS: usize = 20;
/// Bloom 自主闭环子模式：自动修复上限。
const AUTO_MAX_FIXES: usize = 3;
/// 反思门禁失败标记：用于区分「DAG 不可用」与「反思自检未通过」，避免降级冒充成功。
const REFLECT_GATE_MARKER: &str = "[REFLECT_GATE]";

/// Bloom 自主闭环的确定性 QA 判定（离线可测）：非空、非过短、无失败信号 → 通过。
fn qa_pass(result: &str) -> bool {
    let t = result.trim();
    if t.chars().count() < 20 {
        return false;
    }
    let fail_markers = ["失败", "错误", "无法", "未能", "error", "failed", "未通过"];
    if fail_markers.iter().any(|m| t.contains(m)) && t.chars().count() < 80 {
        return false;
    }
    true
}

/// 一个子 Agent 的规格（角色人设 + 任务 + 路由意图/复杂度）。
#[derive(Debug, Clone)]
pub struct SubtaskSpec {
    /// 角色名（人类可读，用于事件与综合上下文）
    pub role: &'static str,
    /// 该子 Agent 的系统人设提示
    pub system_prompt: String,
    /// 具体子任务描述
    pub task: String,
    /// 模型路由意图
    pub intent: IntentType,
    /// 模型路由复杂度
    pub complexity: Complexity,
    /// 依赖的其它 subtask 的 role 名（由 `assign_dependencies` 离线确定性填充）。
    /// 空 = 无依赖，可并行；非空 = 须在其全部完成后才被唤醒 spawn。
    pub deps: Vec<String>,
}

/// 任务分析结果：自动编排的决策依据（纯规则，离线可用）。
#[derive(Debug, Clone)]
pub struct TaskAnalysis {
    /// 本地分类器识别的意图
    pub intent: ClassifierIntent,
    /// 评估出的复杂度
    pub complexity: Complexity,
    /// 是否需要 LLM 消歧
    pub needs_llm: bool,
    /// 自动选定的编排形态
    pub strategy: AgentMode,
    /// 分解出的子任务（Solo 时为空）
    pub subtasks: Vec<SubtaskSpec>,
    /// 决策理由（用于可观测性与前端只读展示）
    pub rationale: String,
}

/// 编排执行结果。
#[derive(Debug, Clone)]
pub struct OrchestrationOutcome {
    /// 实际采用的编排形态
    pub strategy: AgentMode,
    /// lead 综合后的最终答复
    pub lead_reply: String,
    /// 各子 Agent 的结论（按形态收集）
    pub subagent_summaries: Vec<String>,
    /// 本次决策的分析
    pub analysis: TaskAnalysis,
    /// 产出后完成门禁裁决（反幻觉 / 非空 / 时间压力 / 敷衍检测）
    pub completion_verdict: CompletionVerdict,
    /// 反思门禁裁决（仅 Reflect 形态有值；go=false 表示产出未被自检通过，不可信）
    pub reflect_verdict: Option<dag::Reflection>,
    /// Bloom 子模式（仅 Bloom 编排路径有值：deep/auto/swarm；其余路径为 None）。
    pub bloom_submode: Option<String>,
}

/// 自动化的只读画像（供可观测性与前端只读展示）。
#[derive(Debug, Clone)]
pub struct AutomationView {
    /// 唯一 ID
    pub id: String,
    /// 名称
    pub name: String,
    /// cron 表达式
    pub cron_expr: String,
    /// 任务类型名
    pub job: String,
    /// 是否启用
    pub enabled: bool,
    /// 上次运行时间
    pub last_run: Option<DateTime<Utc>>,
    /// 下次预计运行时间
    pub next_run: Option<DateTime<Utc>>,
}

/// 一次自动化运行的只读摘要。
#[derive(Debug, Clone)]
pub struct AutomationRunView {
    /// 运行 ID
    pub run_id: String,
    /// 状态
    pub status: String,
    /// 结果文本
    pub result: Option<String>,
}

/// 自动编排器：持有 lead 运行时 + 子 Agent 管理器 + 分类器 + 模型路由器。
pub struct Orchestrator {
    lead: AgentRuntime,
    subagents: SubAgentManager,
    classifier: Classifier,
    #[allow(dead_code)]
    model_router: Arc<ModelRouter>,
    #[allow(dead_code)]
    tool_registry: Arc<ToolRegistry>,
    /// 宪法前置裁决器（治理门）：自动 dispatch 执行前必须过此门。
    plan_gate: PlanGate,
    /// 完成门禁（产出后验证）：自动 dispatch 综合后过此门，防止「声称完成但未验证」。
    completion_gate: CompletionGate,
    /// 完成门禁用同步 shell 执行器（仅 CommandSucceeds 判据使用，运行时默认真实子进程）。
    completion_shell: Arc<dyn ShellRunner>,
    /// 工作区根（完成门禁核验 agent 自报文件是否真实落盘）。
    workspace: PathBuf,
    /// 一等深度研究能力（KB + 可注入 LLM）；编排器与内置 `research` 工具共享同一实例。
    research: Arc<ResearchCapability>,
    /// 一等多模态视觉能力（可注入解析器 + 转发器）；编排器与内置 `vision` 工具共享同一实例。
    vision: Arc<VisionCapability>,
    /// 一等自动化能力（cron 定时 + 长任务调度 + 台账）；编排器与内置 `automation` 工具共享同一实例。
    automation: Arc<Mutex<AutomationManager>>,
    events: Option<Arc<EventBus>>,
    session: Option<String>,
    /// 当前运行模式（Plan / Agent / Bloom）。
    ///
    /// [`dispatch`] 按此路由：`Bloom` → Captain+Copilot 双模型并行编排；
    /// 其余模式维持既有「分析 → 选形态 → 执行 → 综合」链路，行为不变。
    mode: ModeKind,
}

/// 模型名是否具备视觉（多模态）能力（启发式关键词匹配）。
fn is_vision_model(model: &str) -> bool {
    let m = model.to_lowercase();
    [
        "vision",
        "gpt-4o",
        "-4o",
        "gpt-4.1",
        "qwen-vl",
        "-vl",
        "glm-4v",
        "internvl",
        "deepseek-vl",
        "gemini",
        "claude-3-5-sonnet",
        "claude-sonnet-4",
        "claude-opus-4",
        "minicpm-v",
    ]
    .iter()
    .any(|k| m.contains(k))
}

/// 视觉模型解析器：声明当前模型具备视觉能力（与 transport 配对注入）。
struct VisionModelResolver(String);

#[async_trait]
impl VisionProviderResolver for VisionModelResolver {
    async fn resolve_vision_model(&self) -> Option<String> {
        Some(self.0.clone())
    }
}

/// 从配置构造视觉能力：
/// - 模型名命中视觉关键词 **且** 配置了 API Key → 注入真实 OpenAI 兼容多模态 transport
///   （`vision`/`vision_click` 全链路可用，对齐 分阶段智能体「截图→VLM 坐标」）；
/// - 否则离线降级（`describe_image` 返回元数据说明，`locate` 返回 None → 工具层明确报错），
///   绝不静默失败、绝不联网。
fn vision_from_config(config: &Config) -> Arc<VisionCapability> {
    let cap = Arc::new(VisionCapability::offline_default());
    let m = &config.model;
    if m.api_key.trim().is_empty() || !is_vision_model(&m.model_name) {
        return cap;
    }
    let llm = LLMConfig {
        provider: LLMProvider::Custom {
            base_url: m.base_url.clone(),
        },
        model: m.model_name.clone(),
        api_key: m.api_key.clone(),
        base_url: m.base_url.clone(),
        max_tokens: 2048,
        temperature: 0.2,
        timeout_secs: 30,
        thinking: crate::model::thinking::ThinkingEffort::Off,
        context_window: crate::model::thinking::CONTEXT_WINDOW_DEFAULT,
    };
    cap.set_transport(Box::new(LlmVisionTransport::new(llm)));
    cap.set_resolver(Box::new(VisionModelResolver(m.model_name.clone())));
    cap
}

/// 根据 `config.mcp.servers` 中带 `url` 的条目，建立 HTTP MCP 客户端并预拉取工具目录。
///
/// 在独立 OS 线程 + 独立 tokio runtime 中执行，避免污染调用方已有的 runtime / 阻塞事件循环。
/// 任何失败都 best-effort 跳过（仅打印告警），不影响引擎启动。
fn connect_configured_mcp_clients(
    config: &Config,
) -> Vec<std::sync::Arc<crate::mcp::client::McpClient>> {
    let mut clients = Vec::new();
    for srv in &config.mcp.servers {
        let url = match &srv.url {
            Some(u) if !u.trim().is_empty() => u.trim().to_string(),
            _ => continue,
        };
        let name = srv.name.clone();
        let handle = std::thread::spawn(move || {
            let rt = match ::tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    eprintln!("[mcp] 创建运行时失败({name}): {e}");
                    return None;
                }
            };
            rt.block_on(async move {
                let mut client = crate::mcp::client::McpClient::new(&url);
                if let Err(e) = client.connect().await {
                    eprintln!("[mcp] 连接 {name} ({url}) 失败: {e}");
                    return None;
                }
                match client.list_tools().await {
                    Ok(_) => {
                        eprintln!("[mcp] 已接入 MCP 服务器 {name} ({url})");
                        Some(std::sync::Arc::new(client))
                    }
                    Err(e) => {
                        eprintln!("[mcp] 拉取工具目录 {name} ({url}) 失败: {e}");
                        None
                    }
                }
            })
        });
        match handle.join() {
            Ok(Some(client)) => clients.push(client),
            Ok(None) => {}
            Err(_) => {}
        }
    }
    clients
}

/// 默认工作区根目录：优先应用数据目录，其次进程 cwd。
fn default_workspace() -> std::path::PathBuf {
    if Config::data_dir().exists() {
        Config::data_dir()
    } else {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    }
}

impl Orchestrator {
    /// 从配置构建编排器（消费 config：lead 运行时独占所有权）。
    pub fn new(config: Config) -> Self {
        Self::new_with_workspace(config, default_workspace())
    }

    /// 指定工作区根目录构建编排器（项目模式：工具/终端/文件沙箱根随项目目录切换）。
    pub fn new_with_workspace(config: Config, workspace: std::path::PathBuf) -> Self {
        // 先基于 &config 构造模型路由器（借用立即结束），再 move config 进 lead。
        let model_router = Arc::new(ModelRouter::new(&config));
        // D 期接入：把 20 个原生工具 + 命令护栏 + SSRF 防护注册进工具注册表，
        // 让自动编排（dispatch）下的 lead / 子 Agent 真正能调用工具（而非空注册）。
        let mut tool_registry = ToolRegistry::new();
        // 一等深度研究能力：默认离线安全（空 KB + Mock LLM），生产侧注入真实 LlmResearch。
        // 编排器与内置 `research` 工具共享同一 Arc<ResearchCapability> 实例。
        let research: Arc<ResearchCapability> = Arc::new(ResearchCapability::offline_default());
        // 一等多模态视觉能力：默认离线安全（无视觉模型，describe 优雅降级），
        // 编排器与内置 `vision` 工具共享同一 Arc<VisionCapability> 实例。
        // 模型具备视觉且配置了密钥时自动注入真实多模态 transport（Bloom 视觉通道可用）。
        let vision = vision_from_config(&config);
        // 一等自动化能力：内置 10 维审计轮转自动化就绪；台账落盘到工作区下的 automations/ 子目录，
        // 保证开发/CI 环境下可写（避开受限用户主目录）。编排器与内置 `automation` 工具共享同一实例。
        let mut automation_mgr = AutomationManager::new();
        automation_mgr.set_ledger_root(workspace.join("automations"));
        if let Err(e) = crate::automation::builtin::register_builtins(&mut automation_mgr) {
            eprintln!("[orchestration] 内置自动化注册失败：{e}");
        }
        let automation: Arc<Mutex<AutomationManager>> = Arc::new(Mutex::new(automation_mgr));
        if let Err(e) = crate::tools::register_builtin_tools(
            &mut tool_registry,
            &workspace,
            Some(research.clone()),
            Some(vision.clone()),
            Some(automation.clone()),
        ) {
            eprintln!("[orchestration] 内置工具注册部分失败（部分原生工具不可用）：{e}");
        }
        // MCP 接入：连接 config.mcp.servers 中带 url 的 HTTP MCP 服务器，
        // 将远端工具注册进工具注册表（best-effort，失败不影响引擎启动）。
        for client in connect_configured_mcp_clients(&config) {
            tool_registry.add_mcp_client(client);
        }
        // 技能工具：list_skills / use_skill / create_skill
        // 让 LLM 在工具循环中能发现、调用、创建技能（YAML prompt 模板）。
        let skills_dir = crate::tools::skill_tools::resolve_skills_dir(&config.skills.skills_dir);
        crate::tools::register_skill_tools(&mut tool_registry, skills_dir);
        // 治理接线：把已注册的原生工具名喂给 PlanGate（宪法前置裁决器）。
        // 自动编排身份为 orchestrator（非 user），因此推断出的破坏性动作一律未获批 ——
        // 即自动编排默认不能静默执行破坏性操作，须经治理门人工放行。
        let registered_names = tool_registry.list_tools();
        let plan_gate = PlanGate::default_with_tools(registered_names);
        let completion_gate = CompletionGate::new();
        // 完成门禁运行时的客观核验依赖：工作区根 + 同步 shell 执行器。
        let workspace_for_field = workspace.clone();
        let completion_shell: Arc<dyn ShellRunner> = Arc::new(RealShellRunner);
        let tool_registry = Arc::new(tool_registry);
        let subagents = SubAgentManager::new(
            SubAgentConfig::default(),
            tool_registry.clone(),
            model_router.clone(),
        );
        let lead = AgentRuntime::new(config);
        Self {
            lead,
            subagents,
            classifier: Classifier::new(),
            model_router,
            tool_registry,
            plan_gate,
            completion_gate,
            completion_shell,
            workspace: workspace_for_field,
            research,
            vision,
            automation,
            events: None,
            session: None,
            mode: ModeKind::Agent,
        }
    }

    /// 注册事件总线（lead 与子 Agent 管理器共用同一总线，事件流统一回流前端）。
    pub fn set_event_bus(&mut self, bus: Arc<EventBus>, session: String) {
        self.lead.set_event_bus(bus.clone(), session.clone());
        self.subagents.set_event_bus(bus.clone());
        self.events = Some(bus);
        self.session = Some(session);
    }

    /// 设置当前运行模式（决定 [`dispatch`] 的编排路由）。
    pub fn set_mode(&mut self, mode: ModeKind) {
        self.mode = mode;
    }

    /// 当前运行模式。
    pub fn mode(&self) -> ModeKind {
        self.mode
    }

    /// 注入自进化经验提示（透传到 lead 运行时）。
    pub fn set_experience_hint(&mut self, hint: String) {
        self.lead.set_experience_hint(hint);
    }

    /// 注入记忆召回稳定前缀（W-6）：透传到 lead 运行时，作为模型消息序列最前端的
    /// 稳定前缀（先于经验提示），落进 P-7 stable 半区，利于 KV-cache 前缀复用。
    pub fn set_stable_prefix(&mut self, prefix: Message) {
        self.lead.set_stable_prefix(prefix);
    }

    /// 注入 Track A 路由决策（透传到 lead 运行时）。
    pub fn set_route_decision(&mut self, d: crate::router::RouteDecision) {
        self.lead.set_route_decision(d);
    }

    /// 注入会话历史消息（重启后上下文恢复）：把之前轮次的 user/assistant 消息
    /// 灌入 lead 会话上下文，使模型能看到历史对话而非每轮从零开始。
    pub fn load_history(&mut self, messages: Vec<Message>) {
        for m in messages {
            self.lead.get_context_mut().add_message(m);
        }
    }

    /// 读取 Bloom 子模式（`config.bloom.submode`，空则默认 `deep`）。
    fn bloom_submode(&self) -> String {
        let s = self.lead.config().bloom.submode.trim().to_lowercase();
        if s.is_empty() {
            "deep".to_string()
        } else {
            s
        }
    }

    // ===== 一等深度研究能力（E 期 research/ 落地：编排器可直接触发） =====

    /// 注入真实研究 LLM（生产侧；默认 Mock 保证离线安全）。
    pub fn set_research_llm(&self, llm: Arc<dyn ResearchLlm>) {
        self.research.set_llm(llm);
    }

    /// 向研究知识库增量入库文档（支持运行时 ingestion）。
    pub fn ingest_research_doc(&self, title: &str, content: &str) {
        self.research.add_document(title, content);
    }

    /// 研究知识库已索引文档数（可观测性）。
    pub fn research_doc_count(&self) -> usize {
        self.research.document_count()
    }

    /// 触发深度研究 pipeline：本地 BM25 检索 + LLM 综合，返回结构化研究报告。
    ///
    /// 这是编排器对外的「深度研究」一等入口——区别于普通 chat，它做查询分解→
    /// 检索→提取→综合的完整研究流程，对齐「能力数倍强于 标准 AI 助手」的差异化目标。
    pub async fn research(&self, query: &str) -> ResearchPipeline {
        self.research.research(query, None).await
    }

    // ===== 一等多模态视觉能力（E 期 vision/ 真落地） =====

    /// 注入视觉模型解析器（生产侧；默认 Mock 无能力，describe 优雅降级）。
    pub fn set_vision_resolver(&self, r: Box<dyn VisionProviderResolver + Send + Sync>) {
        self.vision.set_resolver(r);
    }

    /// 注入视觉转发器（生产侧；默认 Mock 无能力，describe 优雅降级）。
    ///
    /// 真实实现传 [`crate::vision::LlmVisionTransport`]，走 OpenAI 兼容多模态接口。
    pub fn set_vision_transport(&self, t: Box<dyn VisionTransport + Send + Sync>) {
        self.vision.set_transport(t);
    }

    /// 多模态看图：读取工作区内图片文件，返回视觉模型描述（无能力/读取失败优雅降级）。
    ///
    /// 这是编排器对外的「看图」一等入口——让纯文本模型也能理解图片，对齐
    /// 「能力数倍强于 标准 AI 助手」的差异化目标。默认离线下返回明确降级文本，绝不联网。
    pub async fn describe_image(&self, path: &str) -> String {
        let abs = match safe_resolve(&self.workspace, path) {
            Ok(p) => p,
            Err(e) => {
                return format!("[视觉] 路径解析失败（可能越界工作区）: {e}");
            }
        };
        match std::fs::read(&abs) {
            Ok(bytes) => self.vision.describe_image(&bytes, "").await,
            Err(e) => format!("[视觉] 读取图片失败: {e}"),
        }
    }

    // ===== 一等自动化能力（E 期 automation/ 真落地） =====

    /// 列出所有已注册自动化（含内置 10 维审计轮转），返回只读画像，
    /// 供可观测性与前端只读展示「agent 当前排程 N 个自动化」。
    pub fn list_automations(&self) -> Vec<AutomationView> {
        let mgr = self.automation.lock().unwrap();
        mgr.list()
            .iter()
            .map(|a| AutomationView {
                id: a.id.clone(),
                name: a.name.clone(),
                cron_expr: a.cron_expr.clone(),
                job: a.job.name().to_string(),
                enabled: a.enabled,
                last_run: a.last_run,
                next_run: a.next_run,
            })
            .collect()
    }

    /// 注册一个自定义自动化（校验 cron 并计算下次运行）。
    pub fn register_automation(
        &self,
        id: &str,
        name: &str,
        cron_expr: &str,
        job: JobKind,
    ) -> Result<()> {
        let mut mgr = self.automation.lock().unwrap();
        mgr.register(Automation::new(id, name, cron_expr, job)?)
    }

    /// 手动触发一次自动化运行（真实落盘台账），返回运行记录摘要。
    pub fn run_automation(&self, id: &str) -> Result<AutomationRunView> {
        let mut mgr = self.automation.lock().unwrap();
        let rec = mgr.run(id)?;
        Ok(AutomationRunView {
            run_id: rec.run_id.clone(),
            status: rec.status.as_str().to_string(),
            result: rec.result.clone(),
        })
    }

    /// 启用 / 禁用某个自动化。
    pub fn set_automation_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let mut mgr = self.automation.lock().unwrap();
        if enabled {
            mgr.enable(id)
        } else {
            mgr.disable(id)
        }
    }

    /// 测试辅助：覆盖自动化台账根目录（让 run 落盘到可控临时目录）。
    #[cfg(test)]
    pub fn set_automation_ledger_root_for_test(&self, root: PathBuf) {
        let mut mgr = self.automation.lock().unwrap();
        mgr.set_ledger_root(root);
    }

    /// 已注册的原生工具数量（含命令护栏 / SSRF 防护 / 版本管理），
    /// 供可观测性与前端只读展示「agent 当前可调用 N 个原生工具」。
    pub fn tool_count(&self) -> usize {
        self.tool_registry.local_tool_count()
    }

    /// 构建 LLM function calling 所需的工具定义（本地 + 已接入 MCP）。
    ///
    /// 供 `dispatch_bloom` 等编排路径把内置工具（read_file / write_file / code_exec /
    /// browser / system_control / vision 等）真正暴露给模型并执行，而非空注册。
    fn tool_defs(&self) -> Vec<ToolDef> {
        self.tool_registry
            .describe_tools_for_llm()
            .into_iter()
            .map(|(name, description, parameters)| {
                // OpenAI/DeepSeek function name 规范：^[a-zA-Z0-9_-]+$
                // 不合法字符（点、空格、斜杠等）替换为下划线，避免 API 400 导致全部工具不可用
                let sanitized: String = name
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
                    .collect();
                if sanitized != name {
                    tracing::warn!(original = %name, sanitized = %sanitized, "工具名包含非法字符，已自动修正");
                }
                ToolDef {
                    name: sanitized,
                    description,
                    parameters,
                }
            })
            .collect()
    }

    /// 离线宪法前置裁决：分析输入 → 推断候选动作 → PlanGate 评估。
    ///
    /// **不调用 LLM**，可在无 API Key 环境运行。自动 [`dispatch`] 执行前必须先过此门：
    /// 任何破坏性 / 越权读取密钥等自动操作都会被宪法 C1 / C2 拦下并要求人工放行。
    /// 返回 [`GateDecision`]（approved / violations / requires_human_gate）。
    pub fn evaluate_dispatch(&self, text: &str) -> GateDecision {
        let analysis = self.analyze(text);
        let actions = infer_plan_actions(&analysis, text);
        let mut plan = ProposedPlan::new("dispatch", &analysis.rationale, "orchestrator");
        plan.actions.extend(actions);
        self.plan_gate.evaluate(&plan)
    }

    /// 确保 lead 处于可接收输入的状态（幂等）。
    fn ensure_ready(&mut self) {
        if matches!(*self.lead.get_state(), AgentState::Init) {
            let _ = self.lead.init();
        }
    }

    /// 纯规则分析：给定输入，自动决策编排形态与子任务分解（**不调用 LLM**）。
    ///
    /// 前端可离线调用此方法，只读展示「agent 将为这条输入选择何种编排」，
    /// 但**不允许**把选择结果回灌给用户手动覆盖——形态只能由本方法自动得出。
    pub fn analyze(&self, text: &str) -> TaskAnalysis {
        let c = self.classifier.classify(text);
        let complexity = map_router_complexity(c.intent.clone(), text);
        let (strategy, subtasks, rationale) = self.plan(text, c.intent.clone());
        TaskAnalysis {
            intent: c.intent.clone(),
            complexity,
            needs_llm: c.needs_llm,
            strategy,
            subtasks,
            rationale,
        }
    }

    /// 自动编排入口：分析 → 选形态 → 执行 → 综合。全程零用户介入。
    ///
    /// 模式路由：`ModeKind::Bloom` 走 [`dispatch_bloom`]（Captain + Copilot 双模型
    /// `tokio::join!` 真并行，过程经事件总线下发前端）；其余模式行为与既有实现一致。
    pub async fn dispatch(&mut self, text: &str) -> Result<OrchestrationOutcome> {
        // Bloom 模式：转交并行编排入口（其自带同一套治理门与完成门禁，
        // 因此这里在门禁之前分流，避免重复裁决）。
        if self.mode == ModeKind::Bloom {
            return self.dispatch_bloom(text).await;
        }
        // 宪法前置裁决：自动编排不得绕过治理门。未通过则阻断并要求人工放行，
        // 任何破坏性 / 越权读密钥等动作都不会被静默执行。
        let decision = self.evaluate_dispatch(text);
        if !decision.approved {
            let ids: Vec<String> = decision
                .violations
                .iter()
                .map(|v| v.rule_id.clone())
                .collect();
            self.emit_thinking(&format!(
                "> 治理门拦截：本次自动编排存在 {} 项宪法违规（{}），需人工放行后方可执行。",
                decision.violations.len(),
                ids.join(", ")
            ));
            return Err(anyhow!(
                "治理门裁决未通过（违反：{}），需人工放行。orchestrator 自动编排被宪法拦截，不得静默执行。",
                ids.join(", ")
            ));
        }
        self.ensure_ready();
        let analysis = self.analyze(text);
        self.emit_thinking(&format!(
            "> 自动编排决策：{} — {}",
            analysis.strategy.label(),
            analysis.rationale
        ));

        match analysis.strategy {
            AgentMode::Solo => {
                // 联网检索意图优先直接用内置 web 工具，保证能真正联网完成任务。
                if Self::is_web_search_intent(text) {
                    if let Some(reply) = self.try_builtin_web_search(text).await {
                        self.emit_thinking(
                            "> 检测到联网检索意图，已直接用内置 web_search/web_research 工具抓取并综合。",
                        );
                        return self.finish_dispatch(
                            text,
                            analysis,
                            AgentMode::Solo,
                            reply,
                            vec![],
                            None,
                        );
                    }
                }
                let reply = self.lead.process_input(text).await?.to_string();
                self.finish_dispatch(text, analysis, AgentMode::Solo, reply, vec![], None)
            }
            AgentMode::DualLeader => {
                // 顺序两阶段：规划者 → 执行者（含计划）→ 综合
                let planner = &analysis.subtasks[0];
                let plan = self.run_single(planner, text).await?;

                let mut executor = analysis.subtasks[1].clone();
                executor.task = format!("{}\n\n【规划者产出】\n{}", executor.task, plan);

                let impl_result = self.run_single(&executor, text).await?;
                let parts = vec![
                    format!("[规划者] {}", plan),
                    format!("[执行者] {}", impl_result),
                ];
                let reply = self.synthesize(text, &parts, &analysis).await?;
                self.finish_dispatch(text, analysis, AgentMode::DualLeader, reply, parts, None)
            }
            AgentMode::ExpertTeam | AgentMode::Cluster => {
                // 复用专家团队原语（agent::team::Team）：并行 spawn 全部角色成员、
                // 收集、聚合——与之前的内联实现行为一致，但把「团队」沉淀为
                // 可复用执行体（CLI / ACP / 上层团队调度均可直接复用）。
                // 复用专家团队原语（agent::team::Team）：依赖感知分波调度
                // （run_p2p 按 SubtaskSpec.deps 构建依赖图，无依赖时逐成员等价于 run）。
                let team = build_team(&analysis.subtasks);
                let outcome = team.run_p2p(&self.subagents, text).await;
                let summaries: Vec<String> =
                    outcome.member_results.into_iter().map(|(_, r)| r).collect();
                let parts: Vec<String> = summaries
                    .iter()
                    .enumerate()
                    .map(|(i, s)| format!("[子专家 {}] {}", i + 1, s))
                    .collect();
                let reply = self.synthesize(text, &parts, &analysis).await?;
                let strategy = analysis.strategy;
                self.finish_dispatch(text, analysis, strategy, reply, summaries, None)
            }
        }
    }

    /// Bloom 并行编排入口：Captain + Copilot 双模型 `tokio::join!` 并行推理，
    /// 再合并主答案与审校补充。与 [`dispatch`] 走同一治理门与完成门禁，
    /// 形态标记为 `DualLeader`（规划/执行双带队语义），前端 RouterFxStrip 可展示。
    ///
    /// 设计：Bloom 是推理层优化（并行/多轮），**不**放宽沙箱与工具权限；
    /// 工具调用仍走 AgentRuntime 白名单（见 mode/privileges.rs）。
    pub async fn dispatch_bloom(&mut self, text: &str) -> Result<OrchestrationOutcome> {
        let decision = self.evaluate_dispatch(text);
        if !decision.approved {
            let ids: Vec<String> = decision
                .violations
                .iter()
                .map(|v| v.rule_id.clone())
                .collect();
            self.emit_thinking(&format!(
                "> 治理门拦截：本次 Bloom 并行编排存在 {} 项宪法违规（{}），需人工放行后方可执行。",
                decision.violations.len(),
                ids.join(", ")
            ));
            return Err(anyhow!(
                "治理门裁决未通过（违反：{}），需人工放行。Bloom 并行编排被宪法拦截。",
                ids.join(", ")
            ));
        }
        self.ensure_ready();
        let mut analysis = self.analyze(text);
        analysis.strategy = AgentMode::DualLeader;
        self.emit_thinking(&format!(
            "> Bloom 并行编排：Captain + Copilot 双模型并行推理（{} — {}）",
            analysis.strategy.label(),
            analysis.rationale
        ));

        // 子模式分派：auto（自主闭环）/ swarm（双领队蜂群）走专属执行路径；
        // deep（默认）保持既有 complexity 路由，行为向后兼容。
        match self.bloom_submode().as_str() {
            "auto" => return self.dispatch_bloom_auto(text, analysis).await,
            "swarm" => return self.dispatch_bloom_swarm(text, analysis).await,
            _ => {}
        }

        // 联网检索快路径：搜索 / 新闻类意图且内置 web 工具可用 → 直接检索 + 综合，
        // 保证「搜索今天的 AI 新闻」这类任务一定用上内置联网工具，不再把简单检索
        // 拆成多个失败子专家，也不依赖模型是否主动选用工具。
        if Self::is_web_search_intent(text) {
            if let Some(reply) = self.try_builtin_web_search(text).await {
                self.emit_thinking(
                    "> 检测到联网检索意图，已直接用内置 web_search/web_research 工具抓取并综合。",
                );
                let mut analysis = self.analyze(text);
                analysis.strategy = AgentMode::Solo;
                return self.finish_dispatch(text, analysis, AgentMode::Solo, reply, vec![], None);
            }
        }

        // 构建工具定义（本地 + 已接入 MCP），让内置工具在 Bloom 模式下真正可达。
        // 按任务复杂度路由执行形态（token 消耗与质量权衡）：
        //   Simple  → 单工具回路（Captain 直接带工具，最省）
        //   Medium  → 双模型 + 工具（Captain 带工具执行 + Copilot 审校）
        //   Complex → 角色团队 DAG（调研/规划/实现/审校 子代理分层并行）
        // 无工具时退化为既有 Bloom 双模型文本推理，保持向后兼容。
        let complexity = analysis.complexity.clone();
        let tool_defs = self.tool_defs();
        let mut team_parts: Vec<String> = vec![];
        let mut team_reflect: Option<dag::Reflection> = None;
        let reply = if tool_defs.is_empty() {
            self.run_bloom_text(text).await?
        } else {
            match complexity {
                Complexity::Simple => self.run_tool_loop_or_bloom(text, &tool_defs).await?,
                Complexity::Medium => match self.run_dual_model_tools(text, &tool_defs).await {
                    Ok(r) => r,
                    Err(e) => {
                        self.emit_thinking(&format!(
                            "> 双模型+工具不可用（{e}），降级为单工具回路。"
                        ));
                        self.run_tool_loop_or_bloom(text, &tool_defs).await?
                    }
                },
                Complexity::Complex => {
                    // 路由：fan-out 信号（批量/枚举）→ 大规模并行（多文件/多模块，见 (A)）；
                    //        反思信号（自检/复盘/审校型）→ Reflect 门控审校闭环（见 (B)）；
                    //        否则经典构建闭环。
                    let kind = if fanout_signal(text) {
                        dag::DagKind::FanOut {
                            fanout: DEFAULT_FANOUT,
                        }
                    } else if reflect_signal(text) {
                        dag::DagKind::Reflect
                    } else {
                        dag::DagKind::Build
                    };
                    match self.run_team_dag(text, kind).await {
                        Ok((r, parts, verdict)) => {
                            team_parts = parts;
                            team_reflect = verdict;
                            r
                        }
                        Err(e) => {
                            // 反思门禁失败 = 产出不可信，必须如实上抛，不可降级冒充成功。
                            if e.to_string().starts_with(REFLECT_GATE_MARKER) {
                                return Err(e);
                            }
                            self.emit_thinking(&format!(
                                "> 角色团队 DAG 不可用（{e}），降级为多策略工具并行。"
                            ));
                            match self.run_multi_strategy_tools(text, &tool_defs).await {
                                Ok(r) => r,
                                Err(e2) => {
                                    self.emit_thinking(&format!(
                                        "> 多策略工具并行不可用（{e2}），降级为双模型+工具。"
                                    ));
                                    match self.run_dual_model_tools(text, &tool_defs).await {
                                        Ok(r) => r,
                                        Err(e3) => {
                                            self.emit_thinking(&format!(
                                                "> 双模型+工具不可用（{e3}），降级为单工具回路。"
                                            ));
                                            self.run_tool_loop_or_bloom(text, &tool_defs).await?
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        };
        self.finish_dispatch(
            text,
            analysis,
            AgentMode::DualLeader,
            reply,
            team_parts,
            team_reflect,
        )
    }

    /// 联网检索意图：命中即优先用内置 web_search / web_research 工具直接检索，
    /// 再把抓取结果综合进最终答复，保证「搜索 / 新闻」类任务一定用上内置联网工具，
    /// 不依赖模型是否主动选用工具。
    fn is_web_search_intent(text: &str) -> bool {
        let lower = text.to_lowercase();
        [
            "搜索",
            "search",
            "查一下",
            "查新闻",
            "新闻",
            "news",
            "最新",
            "today",
            "今天",
            "实时",
            "热搜",
            "资讯",
            "头条",
            "热点",
            "ai 新闻",
            "ai新闻",
            "资讯",
        ]
        .iter()
        .any(|k| lower.contains(&k.to_lowercase()))
    }

    /// 直接调用内置联网工具完成检索类任务（不依赖模型是否主动选用工具）。
    ///
    /// 优先 `web_research`（检索 + 自动深读正文 + 摘要），回退 `web_search`（标题/链接/摘要）；
    /// 任一可用即返回其结构化输出；工具不可用 / 无返回时返回 `None`，交由上层走常规编排。
    async fn try_builtin_web_search(&self, text: &str) -> Option<String> {
        let tool = if self.tool_registry.has_tool("web_research") {
            "web_research"
        } else if self.tool_registry.has_tool("web_search") {
            "web_search"
        } else {
            return None;
        };
        let mut args = HashMap::new();
        args.insert("q".to_string(), text.to_string());
        let res = match self.tool_registry.execute_local_tool(tool, &args).await {
            Ok(r) => r,
            Err(_) => return None,
        };
        let raw = match &res {
            ToolResult::Success { data, .. } => data.clone(),
            _ => return None,
        };
        if raw.trim().is_empty() {
            return None;
        }
        Some(raw)
    }

    /// Bloom 蜂群子模式：FlashSwarm 蜂群分片 + ProExpert 专家团队，双领队并行攻坚。
    async fn dispatch_bloom_swarm(
        &mut self,
        text: &str,
        analysis: TaskAnalysis,
    ) -> Result<OrchestrationOutcome> {
        self.emit_thinking(
            "> Bloom 蜂群子模式：FlashSwarm 蜂群分片 + ProExpert 专家团队，双领队并行。",
        );
        match self.run_bloom_swarm(text).await {
            Ok((reply, parts)) => {
                // 蜂群整体无可用产出（子专家全部失败 / 超时）→ 降级为 lead 带工具的回路，
                // 让 Captain 直接调用 web_search / web_research 等内置工具完成任务，而非交付空壳。
                let usable = !reply.trim().is_empty()
                    && !reply.contains("[子专家执行失败]")
                    && !reply.contains("[子专家无结果返回]")
                    && !reply.contains("[成员 spawn 失败]");
                if !usable {
                    self.emit_thinking(
                        "> 蜂群未能产出可用结果，降级为 lead 工具回路直接完成任务。",
                    );
                    let tool_defs = self.tool_defs();
                    let fallback = if tool_defs.is_empty() {
                        self.run_bloom_text(text).await?
                    } else {
                        self.run_tool_loop_or_bloom(text, &tool_defs).await?
                    };
                    return self.finish_dispatch(
                        text,
                        analysis,
                        AgentMode::Solo,
                        fallback,
                        vec![],
                        None,
                    );
                }
                self.finish_dispatch(text, analysis, AgentMode::Cluster, reply, parts, None)
            }
            Err(e) => {
                self.emit_thinking(&format!("> 蜂群编排不可用（{e}），降级为专家团队 DAG。"));
                let (reply, parts, verdict) = self.run_team_dag(text, dag::DagKind::Build).await?;
                self.finish_dispatch(text, analysis, AgentMode::ExpertTeam, reply, parts, verdict)
            }
        }
    }

    /// Bloom 自主闭环子模式：执行 → QA 判定 → 自动修复，直至 QA 全过或预算耗尽。
    async fn dispatch_bloom_auto(
        &mut self,
        text: &str,
        analysis: TaskAnalysis,
    ) -> Result<OrchestrationOutcome> {
        self.emit_thinking(
            "> Bloom 自主闭环子模式：执行 → QA 判定 → 自动修复，直至 QA 全过或预算耗尽。",
        );
        let reply = self.run_bloom_auto(text).await?;
        self.finish_dispatch(text, analysis, AgentMode::DualLeader, reply, vec![], None)
    }

    /// 双领队蜂群：FlashSwarm（蜂群分片）+ ProExpert（专家团队）双路成员并行 spawn。
    async fn run_bloom_swarm(&mut self, text: &str) -> Result<(String, Vec<String>)> {
        let mut team = Team::new();
        for s in cluster_specs(text).into_iter().take(SWARM_SHARDS) {
            team = team.member(
                s.role,
                &s.system_prompt,
                &s.task,
                s.intent.clone(),
                s.complexity.clone(),
            );
        }
        for s in team_specs(detect_roles(text))
            .into_iter()
            .take(SWARM_SUB_EXPERTS)
        {
            team = team.member(
                s.role,
                &s.system_prompt,
                &s.task,
                s.intent.clone(),
                s.complexity.clone(),
            );
        }
        let outcome = team.run(&self.subagents, text).await;
        let summaries: Vec<String> = outcome
            .member_results
            .iter()
            .map(|(role, r)| format!("[{role}] {r}"))
            .collect();
        Ok((outcome.aggregated, summaries))
    }

    /// 自主闭环：执行 → 确定性 QA 判定 → 失败自动修复（≤AUTO_MAX_FIXES），预算耗尽则停。
    async fn run_bloom_auto(&mut self, text: &str) -> Result<String> {
        let tool_defs = self.tool_defs();
        let mut history = vec![Message::User {
            content: text.to_string(),
            timestamp: Utc::now(),
        }];
        let mut fixes = 0usize;
        for round in 0..AUTO_MAX_ITERATIONS {
            let result = if tool_defs.is_empty() {
                self.run_bloom_text(text).await?
            } else {
                self.lead
                    .run_tool_loop(&mut history, self.tool_registry.clone(), &tool_defs)
                    .await?
            };
            if qa_pass(&result) {
                self.emit_thinking(&format!("> 自主闭环第 {} 轮：QA 通过，交付。", round + 1));
                return Ok(result);
            }
            if fixes >= AUTO_MAX_FIXES {
                self.emit_thinking(&format!(
                    "> 自主闭环：修复次数达到上限（{AUTO_MAX_FIXES}），交付当前结果。"
                ));
                return Ok(result);
            }
            fixes += 1;
            self.emit_thinking(&format!(
                "> 自主闭环第 {} 轮：QA 未过，自动修复（{fixes}/{AUTO_MAX_FIXES}）。",
                round + 1
            ));
            history.push(Message::User {
                content: "上一轮结果未通过质量检查，请修复问题并给出更完整、可验证的答案。"
                    .to_string(),
                timestamp: Utc::now(),
            });
        }
        anyhow::bail!("自主闭环迭代上限（{AUTO_MAX_ITERATIONS}）耗尽，仍未通过 QA")
    }

    /// 单工具回路（Captain 直接带工具，简单任务最省）；失败降级为 Bloom 双模型文本。
    /// 使用真流式回路：模型思考增量、正文增量、工具调用/结果实时经事件总线下发前端。
    async fn run_tool_loop_or_bloom(&self, text: &str, tool_defs: &[ToolDef]) -> Result<String> {
        let t0 = Utc::now().timestamp_millis();
        let captain_model = self.lead.config().model.model_name.clone();
        // 点亮编排卡片：Captain 工具回路（并行证据帧，对齐 Bloom ensemble_progress 契约）
        self.emit_ensemble(EnsembleProgress::proposer_start(0, "Captain"));
        let mut history = vec![Message::User {
            content: text.to_string(),
            timestamp: Utc::now(),
        }];
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
        let bus = self.events.clone();
        let sess = self.session.clone().unwrap_or_default();
        // 事件转发：AgentEvent → 事件总线 → WebSocket → 前端卡片/动效
        let forward = tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                let (etype, payload) = match ev {
                    AgentEvent::Thinking { text } => (EventType::Thinking, json!({ "text": text })),
                    AgentEvent::TextDelta { text } => (
                        EventType::TextDelta,
                        json!({
                            "text": text, "presentation": "answer",
                        }),
                    ),
                    AgentEvent::ToolCall { name, arguments } => {
                        let input: Value =
                            serde_json::from_str(&arguments).unwrap_or(json!(arguments));
                        (
                            EventType::ToolCall,
                            json!({
                                "name": name, "input": input,
                                "started_at": chrono::Utc::now().timestamp_millis(),
                            }),
                        )
                    }
                    AgentEvent::ToolResult { name, output, ok } => (
                        EventType::ToolResult,
                        json!({
                            "name": name, "result": output, "is_error": !ok,
                            "execution_status": { "status": if ok { "success" } else { "error" } },
                        }),
                    ),
                    AgentEvent::StepStart { round } => {
                        (EventType::StepStart, json!({ "round": round }))
                    }
                    AgentEvent::StepFinish { round, reason } => (
                        EventType::StepFinish,
                        json!({
                            "round": round, "reason": reason,
                        }),
                    ),
                    AgentEvent::Done { text } => (EventType::Done, json!({ "summary": text })),
                    AgentEvent::Error { message } => {
                        (EventType::Error, json!({ "message": message }))
                    }
                    AgentEvent::DoomLoop { tool, repeats } => (
                        EventType::Error,
                        json!({
                            "message": format!("工具 {tool} 连续调用 {repeats} 次，疑似死循环，已中断"),
                        }),
                    ),
                    AgentEvent::OutputSuppressed { repeated_line } => (
                        EventType::Thinking,
                        json!({
                            "text": format!("> 输出去冗余：抑制重复行「{}」", repeated_line),
                        }),
                    ),
                };
                if let Some(b) = &bus {
                    b.publish(Envelope::new(etype, payload, sess.clone()));
                }
            }
        });
        let result = self
            .lead
            .run_tool_loop_stream(&mut history, self.tool_registry.clone(), tool_defs, tx, 20)
            .await;
        // 确保转发任务结束（通道关闭后 rx.recv() 返回 None）
        let _ = forward.await;
        match result {
            Ok(r) => {
                self.emit_ensemble(EnsembleProgress::proposer_finish(
                    0,
                    "Captain",
                    &captain_model,
                    (Utc::now().timestamp_millis() - t0) as u64,
                ));
                Ok(r)
            }
            Err(e) => {
                self.emit_ensemble(EnsembleProgress::proposer_error(
                    0,
                    "Captain",
                    &e.to_string(),
                    (Utc::now().timestamp_millis() - t0) as u64,
                ));
                self.emit_thinking(&format!(
                    "> 工具回路不可用（{e}），降级为 Bloom 双模型文本推理。"
                ));
                self.run_bloom_text(text).await
            }
        }
    }

    /// 双模型 + 工具（中等任务）：Captain 用主模型带工具回路真实执行，Copilot 用副模型
    /// 对执行结果做审校补充。副模型未配置时退化为单 Captain 工具回路（不冒充双模型）。
    async fn run_dual_model_tools(&self, text: &str, tool_defs: &[ToolDef]) -> Result<String> {
        let t0 = Utc::now().timestamp_millis();
        let captain_model = self.lead.config().model.model_name.clone();
        // 点亮编排卡片：Captain + Copilot 双模型（对齐 Bloom ensemble_progress 契约）
        self.emit_ensemble(EnsembleProgress::proposer_start(0, "Captain"));
        let mut history = vec![Message::User {
            content: text.to_string(),
            timestamp: Utc::now(),
        }];
        let captain = match self
            .lead
            .run_tool_loop(&mut history, self.tool_registry.clone(), tool_defs)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                self.emit_ensemble(EnsembleProgress::proposer_error(
                    0,
                    "Captain",
                    &e.to_string(),
                    (Utc::now().timestamp_millis() - t0) as u64,
                ));
                return Err(e);
            }
        };

        self.emit_ensemble(EnsembleProgress::proposer_finish(
            0,
            "Captain",
            &captain_model,
            (Utc::now().timestamp_millis() - t0) as u64,
        ));
        let Some(cop_cfg) = LLMConfig::copilot_from_config(self.lead.config()) else {
            return Ok(captain);
        };
        let copilot_model = cop_cfg.model.clone();
        self.emit_ensemble(EnsembleProgress::proposer_start(1, "Copilot"));
        let copilot = LLMClient::new(cop_cfg);
        let review = copilot
            .chat_complete(&[
                Message::System {
                    content: "你是审校模型 Copilot。针对下方答案指出疏漏、边界缺陷与可改进点；若已足够好，只需一句确认。不要重复答案。".into(),
                    timestamp: Utc::now(),
                },
                Message::User {
                    content: format!("任务：{text}\n\nCaptain 答案：\n{captain}"),
                    timestamp: Utc::now(),
                },
            ])
            .await?;
        self.emit_ensemble(EnsembleProgress::proposer_finish(
            1,
            "Copilot",
            &copilot_model,
            (Utc::now().timestamp_millis() - t0) as u64,
        ));
        let review = review.content.trim().to_string();
        if review.is_empty() {
            Ok(captain)
        } else {
            Ok(format!("{captain}\n\n---\n审校补充（Copilot）：\n{review}"))
        }
    }

    /// 多策略工具并行（TUMIX 式）：直接/分步/批判三个角度并行跑带工具的多轮回路，
    /// 再用择优器在候选结果间挑最佳。工具回路真实调用内置工具（read_file / code_exec /
    /// browser / system_control / vision…），是 Bloom 模式「多解择优 + 工具落地」的执行路径。
    async fn run_multi_strategy_tools(&self, text: &str, tool_defs: &[ToolDef]) -> Result<String> {
        let router = self.model_router.as_ref().clone();
        let registry = self.tool_registry.clone();
        let defs = tool_defs.to_vec();

        let strategies = vec![
            crate::agent::tool_strategy(
                "direct",
                "你是执行者。直接、准确地完成用户任务；需要时调用工具（读文件/写代码/搜索/浏览器/系统控制）验证并落地，给出可用的最终结果。",
                router.clone(), registry.clone(), defs.clone(),
            ),
            crate::agent::tool_strategy(
                "stepwise",
                "你是执行者。分步推理：先规划步骤，逐步调用工具执行并验证每步结果，最后给出完整、可复现的结论。",
                router.clone(), registry.clone(), defs.clone(),
            ),
            crate::agent::tool_strategy(
                "critical",
                "你是执行者。先批判性审视任务与边界风险，再调用工具求解，给出严谨且经过验证的结论。",
                router.clone(), registry.clone(), defs.clone(),
            ),
        ];

        // 择优器：空工具定义 → 纯文本单轮，在多策略候选结果间挑最佳。
        let selector =
            crate::agent::ToolLoopBackend::new(router.clone(), registry.clone(), Vec::new());

        let result = crate::agent::run_multi_strategy(text, strategies, Some(selector)).await?;
        Ok(result.final_answer.unwrap_or_default())
    }

    /// Bloom 双模型文本推理（无工具时的退化路径）：Captain + Copilot 并行/四阶段打磨。
    ///
    /// 保留既有 `BloomOrchestrator` 的并行预取、事件回流与四阶段深度推理；作为
    /// `dispatch_bloom` 在「无可用工具」或「工具回路失败」时的兜底，保证 Bloom 始终可用。
    async fn run_bloom_text(&self, text: &str) -> Result<String> {
        let client = LLMClient::new(LLMConfig::from_model_config(self.lead.config()));
        let captain_model = self.lead.config().model.model_name.clone();
        let copilot_model = LLMConfig::copilot_from_config(self.lead.config())
            .map(|c| c.model)
            .unwrap_or_else(|| captain_model.clone());
        let orch = match LLMConfig::copilot_from_config(self.lead.config()) {
            Some(cop) => {
                let copilot = LLMClient::new(cop);
                crate::model::bloom::BloomOrchestrator::with_copilot(client, copilot)
                    .with_deep()
                    .with_model_names(&captain_model, &copilot_model)
            }
            None => crate::model::bloom::BloomOrchestrator::with_copilot(client.clone(), client)
                .with_model_names(&captain_model, &captain_model),
        };
        let mut orch = orch;
        if let (Some(bus), Some(sess)) = (&self.events, &self.session) {
            orch = orch.with_events(bus.clone(), sess.clone());
        }
        // 注入决策缓存：相同意图命中即复用既有编排决策，跳过重复的双模型调用。
        orch = orch.with_decision_cache(Arc::new(
            crate::agent::BloomDecisionCache::with_default_policy(),
        ));
        // 注入语义意图缓存（W-7a）：精确 miss 后高置信相似意图复用既有决策。
        orch = orch.with_semantic_cache(Arc::new(
            crate::agent::SemanticIntentCache::with_default_policy(),
        ));
        let result = orch.run(text).await?;
        Ok(result.merged)
    }

    /// 流式多轮对话（标准 REPL 风格 REPL / CLI 直接模式共用）。
    ///
    /// 委托给 lead 运行时，复用其模型路由与经验提示前置逻辑，
    /// 返回增量 token 流。不修改编排状态机，调用方负责维护多轮历史。
    pub async fn chat(&mut self, history: &[Message]) -> Result<StreamReceiver> {
        self.ensure_ready();
        self.lead.chat_stream(history).await
    }

    /// 运行单个子 Agent（spawn → wait → 取结果）。
    async fn run_single(&mut self, spec: &SubtaskSpec, context: &str) -> Result<String> {
        let id = self
            .subagents
            .spawn_with_spec(
                &spec.task,
                context,
                &spec.system_prompt,
                spec.intent.clone(),
                spec.complexity.clone(),
            )
            .await?;
        let _ = self.subagents.wait_all().await;
        self.subagents
            .get_result(&id)
            .await
            .ok_or_else(|| anyhow!("子专家「{}」无结果返回", spec.role))
    }

    /// DAG 分层编排：子代理「复杂任务自己拆、自己干」的真实入口。
    ///
    /// 流程：宪法门 → 模板分解为带依赖的任务 DAG（`dag::decompose`）→ 复用与
    /// `dispatch_team` 同源的「统一依赖执行体」`build_team` + [`Team::run_p2p`]
    /// （按依赖图分波真实 spawn、上游产出经共享信箱注入下游）→ `dag::aggregate`
    /// 聚合并（Reflect 形态）经 `dag::reflect_pass` 反思门控 → lead 综合 → 完成门禁。
    /// 所有子代理经 [`SubAgentManager::spawn_with_spec`] 真实执行（真实模型路由 +
    /// 真实工具注册表）；层级 / 节点状态经事件总线广播，GUI 卡片实时可见。
    /// 注：原 `dag::run_dag` 执行引擎已收敛为本路径的回滚锚（legacy fallback），不再被调用。
    pub async fn dispatch_team_dag(
        &mut self,
        text: &str,
        kind: dag::DagKind,
    ) -> Result<OrchestrationOutcome> {
        // 宪法前置裁决（与 dispatch 一致：破坏性 / 越权操作须人工放行）
        let decision = self.evaluate_dispatch(text);
        if !decision.approved {
            let ids: Vec<String> = decision
                .violations
                .iter()
                .map(|v| v.rule_id.clone())
                .collect();
            self.emit_thinking(&format!(
                "> 治理门拦截：DAG 编排存在 {} 项宪法违规（{}），需人工放行。",
                decision.violations.len(),
                ids.join(", ")
            ));
            return Err(anyhow!(
                "治理门裁决未通过（违反：{}），需人工放行。",
                ids.join(", ")
            ));
        }
        self.ensure_ready();
        let (reply, parts, verdict) = self.run_team_dag(text, kind).await?;
        self.finish_dispatch(
            text,
            self.analyze(text),
            AgentMode::ExpertTeam,
            reply,
            parts,
            verdict,
        )
    }

    /// 角色团队 DAG：把目标分解为带依赖的角色子代理（调研/规划/实现/审校），逐层并行
    /// 真实 spawn 执行，再由 lead 综合。返回 `(最终答复, 各子代理聚合摘要)`。
    async fn run_team_dag(
        &mut self,
        text: &str,
        kind: dag::DagKind,
    ) -> Result<(String, Vec<String>, Option<dag::Reflection>)> {
        let nodes = dag::decompose(text, kind);
        let t0 = Utc::now().timestamp_millis();
        let lead_model = self.lead.config().model.model_name.clone();
        // 点亮编排卡片：专家团队 DAG（对齐 Bloom ensemble_progress 契约）
        self.emit_ensemble(EnsembleProgress::proposer_start(0, "ExpertTeam"));
        self.emit_ensemble(EnsembleProgress::aggregator("start", None));
        self.emit_thinking(&format!(
            "> DAG 编排启动：{} 个子代理，{} 层依赖拓扑。",
            nodes.len(),
            dag::topo_layers(&nodes).len()
        ));
        // 统一依赖执行体（C6）：复用与 dispatch_team 同源的 build_team + Team::run_p2p，
        // 取代原 dag::run_dag 双套实现（依赖 id→role 映射见 `dag_team`）。
        let team = dag_team(&nodes);
        let outcome = team.run_p2p(&self.subagents, text).await;
        // run_p2p 按角色收集，但 member_results 顺序 == nodes 顺序（波次 FIFO 排空、不排序），
        // FanOut 的多个「编码实现」各自独立保留一条 —— 故可按 nodes 顺序重建 per-id 产出喂 aggregate。
        let mut outputs: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for (n, (_, r)) in nodes.iter().zip(outcome.member_results.iter()) {
            outputs.insert(n.id.clone(), r.clone());
        }
        // run_p2p 无事件总线形参：此处按层/节点重建等价 Thinking 事件，保证 GUI 卡片实时可见
        // （原 run_dag 在 orchestration.dag 源广播层级/节点状态；此处据 topo_layers + outputs 补发）。
        let layers = dag::topo_layers(&nodes);
        for (li, layer) in layers.iter().enumerate() {
            self.emit_thinking(&format!(
                "> 编排层级 {}/{}：{} 个子代理并行执行",
                li + 1,
                layers.len(),
                layer.len()
            ));
            for id in layer {
                if let Some(node) = nodes.iter().find(|n| n.id == *id) {
                    self.emit_thinking(&format!("> 子代理「{}」完成：{}", node.role, node.goal));
                }
            }
        }
        let parts = dag::aggregate(&nodes, &outputs);
        // (B) 反思门控：仅 Reflect 形态在综合交付前用确定性自检门控审校。
        //     任一阶段产出缺失/失败 → fail-closed 拒绝交付，绝不冒充成功。
        let reflect_verdict = if matches!(kind, dag::DagKind::Reflect) {
            let stage_texts: Vec<String> = nodes
                .iter()
                .map(|n| outputs.get(&n.id).cloned().unwrap_or_default())
                .collect();
            let reflection = dag::reflect_pass(&stage_texts);
            if !reflection.go {
                self.emit_thinking(&format!(
                    "> 反思门禁未通过：{} 个阶段缺失/失败（{}），产出不可信，拒绝交付。",
                    reflection.failed_stages.len(),
                    reflection.failed_stages.join("、")
                ));
                self.emit_ensemble(EnsembleProgress::proposer_error(
                    0,
                    "ExpertTeam",
                    "反思门禁未通过",
                    (Utc::now().timestamp_millis() - t0) as u64,
                ));
                return Err(anyhow!(
                    "{} 反思门禁未通过（失败阶段：{}），未交付。",
                    REFLECT_GATE_MARKER,
                    reflection.failed_stages.join("、")
                ));
            }
            self.emit_thinking("> 反思门禁通过：各阶段产出自洽，进入综合交付。");
            Some(reflection)
        } else {
            None
        };
        self.emit_thinking(&format!(
            "> {} 个子代理产出聚合完成，lead 开始综合。",
            parts.len()
        ));
        let reply = self
            .synthesize(
                text,
                &parts,
                &TaskAnalysis {
                    intent: self.classifier.classify(text).intent.clone(),
                    complexity: Complexity::Complex,
                    needs_llm: false,
                    strategy: AgentMode::ExpertTeam,
                    subtasks: vec![],
                    rationale: "DAG 分层编排：调研/规划/实现/审校依赖链".to_string(),
                },
            )
            .await?;
        self.emit_ensemble(EnsembleProgress::aggregator(
            "finish",
            Some((Utc::now().timestamp_millis() - t0) as u64),
        ));
        self.emit_ensemble(EnsembleProgress::proposer_finish(
            0,
            "ExpertTeam",
            &lead_model,
            (Utc::now().timestamp_millis() - t0) as u64,
        ));
        Ok((reply, parts, reflect_verdict))
    }

    /// 直接触发「专家团队并行 spawn」形态（用户显式团队命令 / 上层调度的底层执行体）。
    ///
    /// 与 [`dispatch`] 的 `ExpertTeam` 分支**同源**：经治理门前置裁决 → 复用
    /// [`Team::run_p2p`] 真实依赖感知分波 spawn 全部角色成员（无依赖时等价于 [`Team::run`]）
    /// → 收集聚合 → lead 综合 → 完成门禁。
    /// 区别：本方法**强制**走专家团队形态（不经自动策略路由 Solo/DualLeader/Cluster），
    /// 成员来源由规则角色检测得出（`detect_roles` 经 `analyze` 复用，离线安全）。若未检测
    /// 到任何专精角色（自动形态本会是 Solo），退化为单个通用「执行者」成员，保证「用户要
    /// 团队」至少有一次真实 spawn——不静默降级为单 Agent 对话、也不冒充成功。
    ///
    /// 这是「§6 团队 spawn 落地收尾」的核心：把 [`crate::agent::team::Team`] 这一真实执行体
    /// 暴露为用户可直接触发的表面，区别于 DAG 分层版 `subagent team`（`dispatch_team_dag`）。
    pub async fn dispatch_team(&mut self, goal: &str) -> Result<OrchestrationOutcome> {
        // 1) 治理门前置裁决（与 dispatch / dispatch_team_dag 一致：破坏性 / 越权须人工放行）
        let decision = self.evaluate_dispatch(goal);
        if !decision.approved {
            let ids: Vec<String> = decision
                .violations
                .iter()
                .map(|v| v.rule_id.clone())
                .collect();
            self.emit_thinking(&format!(
                "> 治理门拦截：专家团队存在 {} 项宪法违规（{}），需人工放行。",
                decision.violations.len(),
                ids.join(", ")
            ));
            return Err(anyhow!(
                "治理门裁决未通过（违反：{}），需人工放行。专家团队编排被宪法拦截，不得静默执行。",
                ids.join(", ")
            ));
        }
        self.ensure_ready();

        // 2) 复用自动角色检测（与 dispatch ExpertTeam 分支同源），强制团队形态
        let analysis = self.analyze(goal);
        let specs: Vec<SubtaskSpec> = if analysis.subtasks.is_empty() {
            // 未检测到专精角色：退化为单成员通用团队，至少有一次真实 spawn。
            vec![SubtaskSpec {
                role: "执行者",
                system_prompt: "你是通用执行者。聚焦把用户目标拆为可执行步骤，并交付明确结论。"
                    .to_string(),
                task: goal.to_string(),
                intent: IntentType::General,
                complexity: Complexity::Medium,
                deps: Vec::new(),
            }]
        } else {
            analysis.subtasks.clone()
        };
        self.emit_thinking(&format!(
            "> 专家团队（自动角色检测）：{} 名成员，依赖感知分波 spawn（Team::run_p2p）。",
            specs.len()
        ));

        // 3) 依赖感知分波调度并经 Team 真实执行（run_p2p：无依赖时等价于 run）
        let team = build_team(&specs);
        let outcome = team.run_p2p(&self.subagents, goal).await;
        let summaries: Vec<String> = outcome.member_results.into_iter().map(|(_, r)| r).collect();
        let parts: Vec<String> = summaries
            .iter()
            .enumerate()
            .map(|(i, s)| format!("[子专家 {}] {}", i + 1, s))
            .collect();

        // 4) lead 综合（复用同一份 analysis，离线幂等）+ 完成门禁
        let reply = self.synthesize(goal, &parts, &analysis).await?;
        self.finish_dispatch(
            goal,
            analysis,
            AgentMode::ExpertTeam,
            reply,
            summaries,
            None,
        )
    }

    /// 由 lead 综合子结论为最终答复。
    async fn synthesize(
        &mut self,
        original: &str,
        parts: &[String],
        analysis: &TaskAnalysis,
    ) -> Result<String> {
        let combined = parts.join("\n\n");
        let prompt = format!(
            "你是 Marguerite 的总编排者。下方的子专家结论由自动编排形态「{}」产生，\
             请综合、消歧、去重，并给出面向用户的最终答复（不要复述原始子结论，直接给结论）。\n\n\
             原始需求：{}\n\n子专家结论：\n{}\n\n请给出最终答复。",
            analysis.strategy.label(),
            original,
            combined
        );
        let reply = self.lead.process_input(&prompt).await?;
        Ok(reply.to_string())
    }

    /// 离线完成门禁（不依赖 LLM）：对综合后的产出跑四道检查
    /// （反幻觉客观核验 / 非空 / 时间压力 / 敷衍），返回裁决。
    ///
    /// **关键闭环**：完成门禁现在真正吃到自动验证结果——从综合产出里提取 agent
    /// 自报创建/修改的文件（`extract_claimed_artifacts`），逐一核验是否真实落盘
    /// （`verify_criteria`）。声称写了 `src/login.rs` 但磁盘上没有 → 反幻觉验证失败 →
    /// 拦截。无自报文件则不触发验证（不误伤正常解释性回复）。
    pub fn evaluate_completion(
        &self,
        text: &str,
        reply: &str,
        artifacts: &[&str],
        duration: Option<f64>,
    ) -> CompletionVerdict {
        let criteria = self.completion_gate.extract_claimed_artifacts(reply);
        let verification = if criteria.is_empty() {
            None
        } else {
            Some(self.completion_gate.verify_criteria(
                &criteria,
                &self.workspace,
                &self.completion_shell,
            ))
        };
        self.completion_gate.check_completion(CompletionClaim {
            task_description: text,
            execution_result: reply,
            artifacts,
            verification,
            verification_attempted: !criteria.is_empty(),
            execution_duration: duration,
        })
    }

    /// 收尾：跑完成门禁，若拦截则 emit 提示，并返回最终编排结果。
    fn finish_dispatch(
        &self,
        text: &str,
        analysis: TaskAnalysis,
        strategy: AgentMode,
        lead_reply: String,
        subagent_summaries: Vec<String>,
        reflect_verdict: Option<dag::Reflection>,
    ) -> Result<OrchestrationOutcome> {
        let verdict = self.evaluate_completion(text, &lead_reply, &[], None);
        if !verdict.allowed {
            self.emit_thinking(&format!(
                "> 完成门禁提示：本次产出存在 {} 项可疑（{}），产出可能未经验证，请人工复核。",
                verdict.blocked_by.len(),
                verdict.warnings.join("；")
            ));
        }
        Ok(OrchestrationOutcome {
            strategy,
            lead_reply,
            subagent_summaries,
            analysis,
            completion_verdict: verdict,
            reflect_verdict,
            bloom_submode: if self.mode == ModeKind::Bloom {
                Some(self.bloom_submode())
            } else {
                None
            },
        })
    }

    fn emit_thinking(&self, text: &str) {
        if let (Some(bus), Some(sess)) = (&self.events, &self.session) {
            bus.publish(Envelope::new(
                EventType::Thinking,
                json!({ "text": text }),
                sess.clone(),
            ));
        }
    }

    /// 向事件总线补发一帧并行编排进度（UI 编排卡片）。总线未注入时为空操作。
    /// 负载严格对齐 Web UI `EnsembleProgressPayload`（`useChatRouterDecisionRuntime` 按
    /// `proposer_index` 增量合并），故工具型子路径也能点亮非空编排卡片。
    fn emit_ensemble(&self, progress: EnsembleProgress) {
        if let (Some(bus), Some(sess)) = (&self.events, &self.session) {
            bus.publish(Envelope::ensemble_progress(sess, &progress));
        }
    }

    // ===== 自动决策核心 =====

    /// 规则引擎：根据意图 + 关键词信号，得出编排形态与子任务分解。
    fn plan(
        &self,
        text: &str,
        clf_intent: ClassifierIntent,
    ) -> (AgentMode, Vec<SubtaskSpec>, String) {
        let roles = detect_roles(text);
        let role_count = roles.len();
        let adversarial = adversarial_signal(&text.to_lowercase());
        let fanout = fanout_signal(text);

        let strategy = if fanout {
            AgentMode::Cluster
        } else if role_count >= 2 {
            AgentMode::ExpertTeam
        } else if adversarial {
            AgentMode::DualLeader
        } else {
            AgentMode::Solo
        };

        let mut subtasks = match strategy {
            AgentMode::Solo => vec![],
            AgentMode::DualLeader => dual_leader_specs(),
            AgentMode::ExpertTeam => team_specs(roles.clone()),
            AgentMode::Cluster => cluster_specs(text),
        };

        let rationale = match strategy {
            AgentMode::Solo => format!("单一意图（{:?}），单 Agent 循环即可覆盖（标准 AI 助手 基线等价）", clf_intent),
            AgentMode::DualLeader => "检测到「规划/执行」或「设计/评审」双侧张力，采用 MAX 双带队：规划者先行、执行者据计划落地".to_string(),
            AgentMode::ExpertTeam => format!("检测到 {} 个专精角色需求，采用专家团队并行：{}", role_count, role_labels(&roles)),
            AgentMode::Cluster => "检测到批量 / 枚举型 fan-out 任务，采用集群并行分发".to_string(),
        };

        // 离线、确定性地检测角色间依赖，填充各 subtask 的 deps（无依赖时自动并行）。
        assign_dependencies(&mut subtasks);
        (strategy, subtasks, rationale)
    }
}

/// 离线、确定性的角色间依赖检测：为各 subtask 填充 `deps`（其依赖的其它 subtask 的 role 名）。
///
/// 规则基于角色关键词，不调 LLM；若前置角色不存在于本次分解，则 deps 为空（自动降级并行，
/// 最安全的失败模式：最多「没串行化」，绝不会「出错」）。规则图天然无环，叠加 `run_p2p` 的
/// 环硬化，构成双重保险。
fn assign_dependencies(subtasks: &mut [SubtaskSpec]) {
    let roles: Vec<&str> = subtasks.iter().map(|s| s.role).collect();
    for s in subtasks.iter_mut() {
        let prereqs = prereq_keywords(s.role);
        s.deps = roles
            .iter()
            .filter(|&&other| other != s.role && prereqs.iter().any(|kw| other.contains(kw)))
            .map(|&r| r.to_string())
            .collect();
        s.deps.dedup();
    }
}

/// 角色 `role` 所「需要的前置角色」的关键词集合（其它 subtask 的 role 命中即视为前置）。
fn prereq_keywords(role: &str) -> &'static [&'static str] {
    if role.contains("测试")
        || role.contains("验证")
        || role.contains("test")
        || role.contains("review")
        || role.contains("审查")
        || role.contains("审校")
        || role.contains("audit")
    {
        &["编码实现", "实现", "开发", "代码", "code", "编码"]
    } else if role.contains("文档")
        || role.contains("doc")
        || role.contains("注释")
        || role.contains("readme")
    {
        &["架构设计", "设计", "编码实现", "实现"]
    } else if role.contains("架构设计")
        || role.contains("设计")
        || role.contains("design")
        || role.contains("architect")
        || role.contains("规划")
        || role.contains("plan")
    {
        &["调研", "需求", "分析"]
    } else {
        &[]
    }
}

/// 统一团队构建 seam：按 [`SubtaskSpec`]（含 deps）构造 [`Team`]，依赖感知成员走 `member_dep`。
///
/// 这是把 `Team::run` 切换为 `run_p2p` 的唯一改动点——编排层只换这一处 seam，
/// 不重写 `agent::team` 执行体。无依赖成员等价于 `run()`（向后兼容铁证）。
fn build_team(specs: &[SubtaskSpec]) -> Team {
    let mut team = Team::new();
    for s in specs {
        if s.deps.is_empty() {
            team = team.member(
                s.role,
                &s.system_prompt,
                &s.task,
                s.intent.clone(),
                s.complexity.clone(),
            );
        } else {
            let d: Vec<&str> = s.deps.iter().map(|x| x.as_str()).collect();
            team = team.member_dep(
                s.role,
                &s.system_prompt,
                &s.task,
                s.intent.clone(),
                s.complexity.clone(),
                &d,
            );
        }
    }
    team
}

/// C6 统一依赖执行体辅助：把 DAG 模板节点转换为依赖感知 [`Team`]（与 `dispatch_team`
/// 同源的 `member_dep` 分波调度）。
///
/// DAG 依赖以**节点 id** 描述（如 `t1`/`t3_1`），但 [`Team::run_p2p`] 以**角色名**调度并
/// 向 `"*"` 广播，故此处做 id→role 映射（唯一角色 kind 为 1:1 双射；FanOut 的多个实现节点
/// 折叠为单条「编码实现」，由 `run_p2p` 的波边界屏障保证 t4 在全部实现节点完成后才唤醒，
/// 等价原 `dag::run_dag` 的 id 级扇入）。
fn dag_team(nodes: &[dag::TaskNode]) -> Team {
    let id_to_role: std::collections::HashMap<&str, &str> =
        nodes.iter().map(|n| (n.id.as_str(), n.role)).collect();
    let mut team = Team::new();
    for n in nodes {
        let mut seen = std::collections::HashSet::new();
        let dep_roles: Vec<&str> = n
            .deps
            .iter()
            .filter_map(|d| id_to_role.get(d.as_str()).copied())
            .filter(|r| seen.insert(*r))
            .collect();
        team = team.member_dep(
            n.role,
            dag_role_system(n.role),
            &n.goal,
            n.intent.clone(),
            n.complexity.clone(),
            &dep_roles,
        );
    }
    team
}

/// 子 Agent 角色键（用于角色表查找与去重）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RoleKey {
    Design,
    Code,
    Test,
    Doc,
    Fix,
    Refactor,
    Security,
    Research,
}

/// 角色表：键 → （标签，系统人设，路由意图，复杂度）。
struct RoleDef {
    label: &'static str,
    prompt: &'static str,
    intent: IntentType,
    complexity: Complexity,
}

/// DAG 编排的角色 → 系统人设映射（与 [`role_table`] 共用同一套角色人设）。
fn dag_role_system(role: &str) -> &'static str {
    match role {
        "调研" => role_table(RoleKey::Research).prompt,
        "架构设计" => role_table(RoleKey::Design).prompt,
        "编码实现" => role_table(RoleKey::Code).prompt,
        "测试验证" => role_table(RoleKey::Test).prompt,
        "文档" => role_table(RoleKey::Doc).prompt,
        _ => role_table(RoleKey::Code).prompt,
    }
}

fn role_table(key: RoleKey) -> RoleDef {
    match key {
        RoleKey::Design => RoleDef {
            label: "架构设计",
            prompt: "你是资深系统架构师。聚焦接口契约、模块边界、可扩展性与技术选型权衡，给出清晰的设计方案，不写最终实现代码。",
            intent: IntentType::Reasoning,
            complexity: Complexity::Complex,
        },
        RoleKey::Code => RoleDef {
            label: "编码实现",
            prompt: "你是资深工程师。聚焦正确、可读、可维护的实现，遵循项目既有约定，交付可运行的代码或明确步骤。",
            intent: IntentType::Code,
            complexity: Complexity::Medium,
        },
        RoleKey::Test => RoleDef {
            label: "测试",
            prompt: "你是测试专家。聚焦覆盖关键路径与边界条件，设计可执行的测试用例，指出未覆盖风险。",
            intent: IntentType::Code,
            complexity: Complexity::Medium,
        },
        RoleKey::Doc => RoleDef {
            label: "文档",
            prompt: "你是技术文档专家。聚焦准确、结构化的说明文档与注释，面向目标读者。",
            intent: IntentType::Summarization,
            complexity: Complexity::Simple,
        },
        RoleKey::Fix => RoleDef {
            label: "缺陷修复",
            prompt: "你是调试专家。聚焦根因定位与最小修复，解释出错链路与修复依据。",
            intent: IntentType::Code,
            complexity: Complexity::Medium,
        },
        RoleKey::Refactor => RoleDef {
            label: "重构优化",
            prompt: "你是重构专家。聚焦在不改变外部行为的前提下消除重复、降低复杂度、提升性能，说明改动影响面。",
            intent: IntentType::Code,
            complexity: Complexity::Medium,
        },
        RoleKey::Security => RoleDef {
            label: "安全",
            prompt: "你是安全专家。聚焦漏洞识别、最小权限与输入校验，给出加固建议与风险评级。",
            intent: IntentType::Reasoning,
            complexity: Complexity::Complex,
        },
        RoleKey::Research => RoleDef {
            label: "调研",
            prompt: "你是调研分析师。聚焦事实检索、方案对比与权衡，给出有据可循的结论与出处。",
            intent: IntentType::Chat,
            complexity: Complexity::Medium,
        },
    }
}

/// 按关键词检测文本涉及的角色（去重、按固定顺序）。
fn detect_roles(text: &str) -> Vec<RoleKey> {
    let lower = text.to_lowercase();
    let rules: &[(RoleKey, &[&str])] = &[
        (
            RoleKey::Design,
            &["设计", "design", "架构", "architecture", "方案", "技术选型"],
        ),
        (
            RoleKey::Code,
            &[
                "实现",
                "implement",
                "代码",
                "code",
                "开发",
                "写",
                "函数",
                "模块",
                "功能",
                "编码",
            ],
        ),
        (
            RoleKey::Test,
            &["测试", "test", "单测", "单元测试", "覆盖", "coverage"],
        ),
        (
            RoleKey::Doc,
            &[
                "文档",
                "doc",
                "注释",
                "comment",
                "readme",
                "说明",
                "使用文档",
            ],
        ),
        (
            RoleKey::Fix,
            &["修复", "fix", "bug", "调试", "debug", "报错", "错误"],
        ),
        (
            RoleKey::Refactor,
            &["重构", "refactor", "优化", "improve", "清理", "clean"],
        ),
        (
            RoleKey::Security,
            &[
                "安全", "security", "扫描", "scan", "漏洞", "vuln", "风险", "加固",
            ],
        ),
        (
            RoleKey::Research,
            &[
                "调研",
                "research",
                "搜索",
                "search",
                "查找",
                "对比",
                "compare",
                "查一下",
            ],
        ),
    ];
    let mut out = Vec::new();
    for (key, kws) in rules {
        if kws.iter().any(|kw| lower.contains(&kw.to_lowercase())) {
            out.push(*key);
        }
    }
    out
}

/// 双侧张力信号：检测到「规划/执行」「设计/评审」「实现/验证」类结构。
fn adversarial_signal(lower: &str) -> bool {
    let triggers = [
        "评审",
        "review",
        "审查",
        "审阅",
        "验证",
        "verify",
        "对比",
        "权衡",
        "评估",
        "evaluate",
        "并评审",
        "并验证",
        "并审查",
        "设计",
        "design",
    ];
    triggers.iter().any(|t| lower.contains(t))
}

/// fan-out 信号：批量 / 枚举列表 / 多条目分发。
fn fanout_signal(text: &str) -> bool {
    let lower = text.to_lowercase();
    let keywords = [
        "批量",
        "分别",
        "每个",
        "全部",
        "逐一",
        "列表",
        "以下任务",
        "如下",
        "各自",
        "并发",
        "并行处理",
    ];
    if keywords.iter().any(|k| lower.contains(k)) {
        return true;
    }
    // 枚举列表（至少 2 项）
    extract_enumerated(text).len() >= 2
}

/// 反思信号：自检 / 复盘 / 审校型任务（需 s2_5 门控，先反思工作记忆再行动）。
fn reflect_signal(text: &str) -> bool {
    let lower = text.to_lowercase();
    let keywords = [
        "反思",
        "自检",
        "复盘",
        "review-self",
        "自我审查",
        "审校",
        "回顾",
        "retrospective",
        "重新检查",
        "核查一遍",
        "门控审校",
    ];
    keywords.iter().any(|k| lower.contains(k))
}

#[cfg(test)]
mod reflect_signal_tests {
    use super::*;

    #[test]
    fn detects_self_check_keywords() {
        assert!(reflect_signal("请反思一下这次实现的问题"));
        assert!(reflect_signal("对代码做一次自我审查"));
        assert!(reflect_signal("复盘整个项目并给出门控审校"));
        assert!(reflect_signal("review-self: 检查上述改动"));
    }

    #[test]
    fn ignores_non_reflect_tasks() {
        assert!(!reflect_signal("批量生成 10 个组件"));
        assert!(!reflect_signal("实现登录功能"));
        assert!(!reflect_signal("帮我写个排序算法"));
    }
}

/// 提取文本中的枚举条目（数字/符号列表项）。
pub(crate) fn extract_enumerated(text: &str) -> Vec<String> {
    let re = match regex::Regex::new(r"(?m)^\s*(?:\d+[.、)\-]|[-*•]\s+)\s*(.+?)\s*$") {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    re.captures_iter(text)
        .filter_map(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect()
}

/// MAX 双带队：规划者（Flash 式规划）+ 执行者（Pro 式落地）。
fn dual_leader_specs() -> Vec<SubtaskSpec> {
    let planner = RoleDef {
        label: "规划者",
        prompt: "你是规划者（Flash 式）。将用户需求拆解为清晰、可执行的实现计划与接口契约，明确步骤、依赖与验收标准。不要编写最终代码。",
        intent: IntentType::Reasoning,
        complexity: Complexity::Complex,
    };
    let executor = RoleDef {
        label: "执行者",
        prompt: "你是执行者（Pro 式）。基于规划者给出的计划实现需求，产出可交付的实现方案/代码/步骤，确保满足验收标准。",
        intent: IntentType::Code,
        complexity: Complexity::Complex,
    };
    vec![
        SubtaskSpec {
            role: planner.label,
            system_prompt: planner.prompt.to_string(),
            task: String::new(),
            intent: planner.intent,
            complexity: planner.complexity,
            deps: Vec::new(),
        },
        SubtaskSpec {
            role: executor.label,
            system_prompt: executor.prompt.to_string(),
            task: String::new(),
            intent: executor.intent,
            complexity: executor.complexity,
            deps: Vec::new(),
        },
    ]
}

/// 专家团队：按检测到的角色生成子 Agent（保证至少 2 个）。
fn team_specs(roles: Vec<RoleKey>) -> Vec<SubtaskSpec> {
    let mut roles = roles;
    if roles.is_empty() {
        roles = vec![RoleKey::Code, RoleKey::Test];
    }
    // 保证团队规模 >= 2：单角色时补一个校验角色
    if roles.len() == 1 {
        roles.push(RoleKey::Test);
    }
    roles
        .into_iter()
        .map(|k| {
            let d = role_table(k);
            // 行业领军人经验蒸馏：优先用专家库渲染的完整人设（含 lineage/mindset/
            // 专属技能/playbook），查不到才回退通用角色人设，保证团队不降级为占位。
            let system_prompt = crate::agent::expertise::expert_prompt(d.label)
                .unwrap_or_else(|| d.prompt.to_string());
            SubtaskSpec {
                role: d.label,
                system_prompt,
                task: String::new(),
                intent: d.intent,
                complexity: d.complexity,
                deps: Vec::new(),
            }
        })
        .collect()
}

/// 集群：把 fan-out 任务分片到并行 Worker（无枚举列表时退化为多视角并行）。
fn cluster_specs(text: &str) -> Vec<SubtaskSpec> {
    let items = extract_enumerated(text);
    if items.len() >= 2 {
        // 分片：每片至多 5 项（与 SubAgentConfig::max_concurrent 对齐）
        let per = items.len().div_ceil(5).max(1);
        let mut chunks: Vec<Vec<String>> = vec![];
        for chunk in items.chunks(per) {
            chunks.push(chunk.to_vec());
        }
        chunks
            .into_iter()
            .enumerate()
            .map(|(i, c)| SubtaskSpec {
                role: "Worker",
                system_prompt: "你是并行集群中的一个 Worker。独立、确定性地完成分配到的子任务，输出结构化结论，不依赖其他 Worker 的输出。".to_string(),
                task: format!("集群并行子任务（第 {} 批，共 {} 项）：\n- {}", i + 1, c.len(), c.join("\n- ")),
                intent: IntentType::General,
                complexity: Complexity::Simple,
                deps: Vec::new(),
            })
            .collect()
    } else {
        // 无枚举列表但有 fan-out 关键词：三个视角并行
        let views = [
            ("实现视角", "聚焦如何落地实现该任务。"),
            ("测试视角", "聚焦如何验证该任务的正确性与边界。"),
            ("文档视角", "聚焦如何为该任务产出清晰的使用/说明文档。"),
        ];
        views
            .iter()
            .map(|(label, focus)| SubtaskSpec {
                role: "Worker",
                system_prompt: format!(
                    "你是并行集群中的一个 Worker，从「{}」角度独立处理整个任务：{}",
                    label, focus
                ),
                task: text.to_string(),
                intent: IntentType::General,
                complexity: Complexity::Simple,
                deps: Vec::new(),
            })
            .collect()
    }
}

fn role_labels(roles: &[RoleKey]) -> String {
    roles
        .iter()
        .map(|k| role_table(*k).label)
        .collect::<Vec<_>>()
        .join(" + ")
}

/// 从编排分析结果推断「候选动作清单」，用于宪法前置裁决。
///
/// 这是**意图级**候选（基于策略 / 角色 / 文本信号，而非运行时实际工具调用），
/// 目的是在自动执行前拦住破坏性 / 越权动作，符合宪法 C1（破坏性须批准）/
/// C2（密钥仅限用户侧）/ C3（工具须登记）。
///
/// 自动编排身份为 `orchestrator`（非 `user`），因此推断出的破坏性动作一律
/// `approved=false` —— 即自动编排默认**不能**静默执行破坏性操作。
fn infer_plan_actions(analysis: &TaskAnalysis, text: &str) -> Vec<ActionIntent> {
    let lower = text.to_lowercase();
    let destructive_sig = [
        "rm -rf",
        "rm /",
        "del /",
        "删除",
        "delete",
        "覆盖",
        "overwrite",
        "force",
        "强制",
        "reset --hard",
        "drop ",
        "truncate",
        "格式化",
        "format",
        "shutdown",
        "重启",
        "reboot",
    ];
    let secret_sig = [
        "api key",
        "密钥",
        "secret",
        "token",
        "密码",
        "password",
        "凭据",
        "credential",
        "环境变量",
    ];
    let destructive = destructive_sig.iter().any(|s| lower.contains(s));
    let secret = secret_sig.iter().any(|s| lower.contains(s));

    let mut actions = Vec::new();
    for s in &analysis.subtasks {
        let (tool, op): (&str, &str) = match s.role {
            "编码实现" | "编码" => ("write_file", "写/改源码文件"),
            "测试" => ("shell_exec", "运行测试套件"),
            "文档" => ("write_file", "生成文档文件"),
            "重构优化" => ("edit_file", "就地重构源码"),
            "缺陷修复" => ("edit_file", "定位并修复缺陷"),
            "安全" => ("web_fetch", "拉取安全公告/扫描"),
            "调研" => ("web_fetch", "检索外部资料"),
            "架构设计" => ("read_file", "读取现状代码以设计"),
            "规划者" => ("write_file", "产出实现计划"),
            "执行者" => ("write_file", "落地实现"),
            _ => ("read_file", "读取上下文"),
        };
        let mut a = ActionIntent::new(tool, op);
        if destructive {
            a = a.destructive();
        }
        actions.push(a);
    }

    // 集群 / fan-out 隐含批量处理（默认破坏性，需人工放行）
    if matches!(analysis.strategy, AgentMode::Cluster) {
        actions.push(ActionIntent::new("shell_exec", "批量并行处理").destructive());
    }
    // Solo 默认读/处理（破坏性信号下同样标记为破坏性）
    if matches!(analysis.strategy, AgentMode::Solo) {
        let mut a = ActionIntent::new("read_file", "读取并处理输入");
        if destructive {
            a = a.destructive();
        }
        actions.push(a);
    }
    // 密钥读取信号 → 触发 C2（orchestrator 身份未获批）
    if secret {
        actions.push(ActionIntent::new("read_env", "读取环境变量/密钥"));
    }

    actions
}

// ===== 意图/复杂度映射（镜像 runtime.rs 的做法，但落到 model::router 枚举） =====

#[allow(dead_code)]
fn map_router_intent(clf: ClassifierIntent) -> IntentType {
    match clf {
        ClassifierIntent::Explain
        | ClassifierIntent::Refactor
        | ClassifierIntent::Fix
        | ClassifierIntent::Test => IntentType::Code,
        ClassifierIntent::Doc | ClassifierIntent::Commit => IntentType::Summarization,
        ClassifierIntent::Search | ClassifierIntent::Question => IntentType::Chat,
        ClassifierIntent::Unknown => IntentType::General,
    }
}

fn map_router_complexity(clf: ClassifierIntent, text: &str) -> Complexity {
    let base = match clf {
        ClassifierIntent::Refactor | ClassifierIntent::Fix | ClassifierIntent::Test => {
            Complexity::Medium
        }
        _ => Complexity::Simple,
    };
    // 长输入或含多角色关键词 → 升一档
    if text.chars().count() > 240 || detect_roles(text).len() >= 2 {
        Complexity::Complex
    } else {
        base
    }
}

#[cfg(test)]
impl Orchestrator {
    /// 测试专用：覆盖工作区根，使完成门禁可在受控临时目录里核验文件落盘。
    pub fn set_workspace_for_test(&mut self, p: PathBuf) {
        self.workspace = p;
    }
}

/// 把编排器暴露为 ACP `AgentInvoker`：sidecar 经回环端口调用 agent 时，
/// 真实路由到编排器的离线分析（不调 LLM）。供 `serve` 子命令与桌面 / TUI sidecar 复用。
///
/// 与 `serve/acp/session.rs` 的 `AgentInvoker` 契约一致；`health()` 提供引擎可观测性快照。
pub struct EngineInvoker {
    /// 共享的编排器（离线分析内核）
    orchestrator: Arc<Orchestrator>,
}

impl EngineInvoker {
    /// 构造：包裹一个编排器实例。
    pub fn new(orchestrator: Arc<Orchestrator>) -> Self {
        Self { orchestrator }
    }
}

#[async_trait::async_trait]
impl AgentInvoker for EngineInvoker {
    async fn invoke(&self, prompt: &str) -> Result<String, String> {
        let a = self.orchestrator.analyze(prompt);
        let v = json!({
            "intent": format!("{:?}", a.intent),
            "complexity": format!("{:?}", a.complexity),
            "needs_llm": a.needs_llm,
            "strategy": a.strategy.label(),
            "subtask_count": a.subtasks.len(),
            "rationale": a.rationale,
        });
        Ok(serde_json::to_string(&v).unwrap_or_else(|_| "{}".to_string()))
    }

    fn health(&self) -> Value {
        json!({
            "status": "ok",
            "loopback_only": true,
            "tools": self.orchestrator.tool_count(),
            "research_docs": self.orchestrator.research_doc_count(),
            "automations": self.orchestrator.list_automations().len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    #[test]
    fn vision_model_keyword_heuristics() {
        assert!(is_vision_model("gpt-4o"));
        assert!(is_vision_model("Qwen-VL-Max"));
        assert!(is_vision_model("gpt-4.1-mini"));
        assert!(is_vision_model("glm-4v-flash"));
        assert!(is_vision_model("deepseek-vl2"));
        assert!(is_vision_model("gemini-2.0-flash"));
        assert!(!is_vision_model("deepseek-chat"));
        assert!(!is_vision_model("DeepSeek-V4 Flash/Pro"));
        assert!(!is_vision_model("gpt-4"));
    }

    #[tokio::test]
    async fn vision_from_config_degrades_for_non_vision_model() {
        // deepseek-chat（无视觉）→ 不注入 transport，locate 返回 None（离线安全）
        let cfg = Config::default(); // model_name = deepseek-chat
        let cap = vision_from_config(&cfg);
        assert!(cap.locate(&png(), "按钮").await.is_none());
    }

    #[tokio::test]
    async fn vision_from_config_injects_for_vision_model() {
        // 视觉模型 + key → 注入 transport；locate 走真实 transport（无网络时优雅失败返回 None）
        let mut cfg = Config::default();
        cfg.model.model_name = "gpt-4o".into();
        cfg.model.api_key = "sk-test".into();
        cfg.model.base_url = "http://127.0.0.1:9".into(); // 不可达：请求立即失败，不发起外网
        let cap = vision_from_config(&cfg);
        // transport 已注入：resolver 声明 capable；网络失败 → locate 返回 None（不 panic）
        assert!(cap.locate(&png(), "按钮").await.is_none());
    }

    fn png() -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&[0, 0, 0, 13]);
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&64u32.to_be_bytes());
        v.extend_from_slice(&48u32.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0]);
        v
    }

    /// 把编排器暴露为 ACP `AgentInvoker`：sidecar 经回环端口调用 agent 时，
    /// 真实路由到编排器的离线分析（不调 LLM）。当前由集成测试驱动验证；
    /// 未来桌面 / TUI sidecar 接入时直接复用。
    struct EngineInvoker {
        orchestrator: Arc<Orchestrator>,
    }

    impl EngineInvoker {
        fn new(orchestrator: Arc<Orchestrator>) -> Self {
            Self { orchestrator }
        }
    }

    #[async_trait::async_trait]
    impl AgentInvoker for EngineInvoker {
        async fn invoke(&self, prompt: &str) -> Result<String, String> {
            let a = self.orchestrator.analyze(prompt);
            let v = serde_json::json!({
                "intent": format!("{:?}", a.intent),
                "complexity": format!("{:?}", a.complexity),
                "needs_llm": a.needs_llm,
                "strategy": a.strategy.label(),
                "subtask_count": a.subtasks.len(),
                "rationale": a.rationale,
            });
            Ok(serde_json::to_string(&v).unwrap_or_else(|_| "{}".to_string()))
        }

        fn health(&self) -> Value {
            serde_json::json!({
                "status": "ok",
                "loopback_only": true,
                "tools": self.orchestrator.tool_count(),
                "research_docs": self.orchestrator.research_doc_count(),
                "automations": self.orchestrator.list_automations().len(),
            })
        }
    }

    fn cfg() -> Config {
        Config::default()
    }

    #[test]
    fn test_analyze_solo_explain() {
        let o = Orchestrator::new(cfg());
        let a = o.analyze("解释一下闭包是什么");
        assert_eq!(a.strategy, AgentMode::Solo, "单意图解释应走 Solo");
        assert!(a.subtasks.is_empty());
    }

    #[test]
    fn test_analyze_solo_fix_single() {
        let o = Orchestrator::new(cfg());
        let a = o.analyze("修复这个登录崩溃的 bug");
        assert_eq!(
            a.strategy,
            AgentMode::Solo,
            "单一修复意图应走 Solo（基线等价）"
        );
    }

    #[test]
    fn test_analyze_expert_team_code_and_test() {
        let o = Orchestrator::new(cfg());
        let a = o.analyze("实现登录功能并编写单元测试");
        assert_eq!(a.strategy, AgentMode::ExpertTeam, "代码+测试应走专家团队");
        assert!(a.subtasks.len() >= 2, "专家团队至少 2 个子 Agent");
        // 子任务应角色感知（编码 + 测试）
        let roles: Vec<&str> = a.subtasks.iter().map(|s| s.role).collect();
        assert!(roles.contains(&"编码实现"), "应包含编码实现角色");
        assert!(roles.contains(&"测试"), "应包含测试角色");
    }

    #[test]
    fn test_analyze_expert_team_refactor_and_doc() {
        let o = Orchestrator::new(cfg());
        let a = o.analyze("重构这个模块并生成文档");
        assert_eq!(a.strategy, AgentMode::ExpertTeam);
        let roles: Vec<&str> = a.subtasks.iter().map(|s| s.role).collect();
        assert!(roles.contains(&"重构优化"), "应包含重构角色");
        assert!(roles.contains(&"文档"), "应包含文档角色");
    }

    #[test]
    fn test_analyze_dual_leader() {
        let o = Orchestrator::new(cfg());
        let a = o.analyze("设计系统架构并评审方案");
        assert_eq!(
            a.strategy,
            AgentMode::DualLeader,
            "设计+评审双侧张力应走 MAX 双带队"
        );
        assert_eq!(a.subtasks.len(), 2);
        assert_eq!(a.subtasks[0].role, "规划者");
        assert_eq!(a.subtasks[1].role, "执行者");
    }

    #[test]
    fn test_analyze_cluster_enumerated() {
        let o = Orchestrator::new(cfg());
        let a = o.analyze("分别处理以下任务：\n1. 重写登录模块\n2. 修复支付 bug\n3. 补充 API 文档");
        assert_eq!(a.strategy, AgentMode::Cluster, "枚举 fan-out 应走集群");
        assert!(a.subtasks.len() >= 2, "集群应分片为多个 Worker");
        assert!(a.subtasks.iter().all(|s| s.role == "Worker"));
    }

    #[test]
    fn test_analyze_cluster_keyword() {
        let o = Orchestrator::new(cfg());
        let a = o.analyze("批量处理这 50 个日志文件并提取错误");
        assert_eq!(a.strategy, AgentMode::Cluster);
    }

    #[test]
    fn test_analyze_offline_safe_no_llm() {
        // analyze 不得触发任何网络/LLM 调用：纯规则，离线必过。
        let o = Orchestrator::new(cfg());
        let inputs = [
            "解释闭包",
            "实现功能并测试",
            "设计架构并评审",
            "批量处理下列文件",
        ];
        for inp in inputs {
            let a = o.analyze(inp);
            assert!(!a.rationale.is_empty(), "自动决策应携带非空理由");
        }
    }

    #[test]
    fn test_role_table_unique_labels() {
        // 保证角色标签稳定可用于事件与综合上下文
        let keys = [
            RoleKey::Design,
            RoleKey::Code,
            RoleKey::Test,
            RoleKey::Doc,
            RoleKey::Fix,
            RoleKey::Refactor,
            RoleKey::Security,
            RoleKey::Research,
        ];
        let labels: Vec<&str> = keys.iter().map(|k| role_table(*k).label).collect();
        let mut sorted = labels.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), labels.len(), "角色标签应唯一");
    }

    #[test]
    fn test_builtin_tools_wired() {
        // D 期闭环验证：编排器构建后应接入 >=20 个原生工具（而非空注册表）。
        let o = Orchestrator::new(cfg());
        let n = o.tool_count();
        assert!(n >= 20, "编排器应接入至少 20 个原生工具，实际 {}", n);
    }

    // ===== E 期 research/ 真落地：一等深度研究能力接进编排器（离线端到端） =====

    #[tokio::test]
    async fn test_orchestrator_research_capability_wired() {
        // 证明编排器持有的一等深度研究能力端到端可用：入库文档 → 研究 pipeline →
        // 结构化报告。使用与生产线 LlmResearch 同一 ResearchLlm trait 的 Mock 实现，
        // 离线可测，且证明治理「mock 默认 + 真实注入」范式落地。
        let o = Orchestrator::new(cfg());
        o.ingest_research_doc(
            "Rust Async",
            "Rust 异步编程使用 tokio 与 async/await 模型。",
        );
        o.set_research_llm(Arc::new(MockResearchLlm));
        let p = o.research("Rust 异步编程是什么").await;
        assert_eq!(p.status, "done");
        assert!(!p.steps.is_empty(), "研究 pipeline 应产生步骤");
        assert!(p.final_report.contains("研究报告"), "应产出研究报告");
        // 研究知识库已索引文档可观测
        assert!(o.research_doc_count() >= 1);
        // 内置 research 工具也已随能力实例注册（编排器接入 21 个工具）
        assert!(
            o.tool_count() >= 21,
            "research 工具应已注册，实际 {}",
            o.tool_count()
        );
    }

    #[tokio::test]
    async fn test_research_tool_registered_and_callable() {
        // 证明内置 research 工具可被工具注册表真实调用，且走通研究 pipeline。
        let o = Orchestrator::new(cfg());
        o.ingest_research_doc("BM25", "BM25 是一种概率信息检索排序函数，用于知识库检索。");
        o.set_research_llm(Arc::new(MockResearchLlm));

        let mut args = std::collections::HashMap::new();
        args.insert("query".to_string(), "BM25 是什么".to_string());
        let reg = &o.tool_registry;
        let result = reg.execute_local_tool("research", &args).await.unwrap();
        match result {
            crate::agent::tool_cycle::ToolResult::Success { data, .. } => {
                assert!(data.contains("研究完成"), "research 工具应回显研究完成");
                assert!(data.contains("研究报告"), "research 工具应带回研究报告");
            }
            other => panic!("research 工具应成功，实际 {:?}", other.is_success()),
        }
    }

    // ===== E 期 vision/ 真落地：一等多模态视觉能力接进编排器（离线端到端） =====

    fn png_bytes() -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&[0, 0, 0, 13]);
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&64u32.to_be_bytes());
        v.extend_from_slice(&48u32.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0]);
        v
    }

    #[tokio::test]
    async fn test_orchestrator_vision_capability_wired() {
        // 证明编排器持有一等多模态能力，且离线默认走优雅降级（不联网、不静默失败）。
        let mut o = Orchestrator::new(cfg());
        let dir = std::env::temp_dir().join(format!("mar_vision_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cat.png"), png_bytes()).unwrap();
        o.set_workspace_for_test(dir.clone());

        // 离线默认：无视觉能力 → 明确降级文本 + 图片元数据
        let out = o.describe_image("cat.png").await;
        assert!(out.contains("[视觉降级]"), "离线应优雅降级，实际: {out}");
        assert!(
            out.contains("无可用视觉模型"),
            "应说明无视觉模型，实际: {out}"
        );
        assert!(out.contains("png"), "降级文本应含格式元数据，实际: {out}");

        // 注入可用视觉能力 → 真实描述路径走通
        o.set_vision_resolver(Box::new(MockVisionResolver { capable: true }));
        o.set_vision_transport(Box::new(MockVisionTransport {
            reply: "一只猫".to_string(),
        }));
        let out2 = o.describe_image("cat.png").await;
        assert_eq!(out2, "一只猫", "注入可用视觉能力后应返回描述");

        // 内置 vision 工具随能力实例注册（编排器接入 22 个工具）
        assert!(
            o.tool_count() >= 22,
            "vision 工具应已注册，实际 {}",
            o.tool_count()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_vision_tool_registered_and_callable() {
        // 证明内置 vision 工具可被工具注册表真实调用，离线默认走降级。
        // 注：工具的 workspace 在注册时绑定为 Config::data_dir()（与编排器一致），
        // 故写入编排器真实 workspace（o.workspace）供工具读取，避免污染临时目录。
        let o = Orchestrator::new(cfg());
        let ws = o.workspace.clone();
        std::fs::create_dir_all(&ws).ok();
        let png_path = ws.join("img.png");
        std::fs::write(&png_path, png_bytes()).unwrap();

        let mut args = std::collections::HashMap::new();
        args.insert("path".to_string(), "img.png".to_string());
        let reg = &o.tool_registry;
        let result = reg.execute_local_tool("vision", &args).await.unwrap();
        match result {
            crate::agent::tool_cycle::ToolResult::Success { data, .. } => {
                // 无视觉能力时走 HTML 兜底：返回「降级标记 + 可读 HTML（含 base64 图片）」，
                // 让无多模态模型也能据此「阅读」图片，而非仅返回元数据告警。
                assert!(
                    data.contains("[视觉降级·HTML 兜底]")
                        && data.contains("<img")
                        && data.contains("base64"),
                    "vision 工具离线应走 HTML 兜底（含 base64 图片），实际: {data}"
                );
            }
            other => panic!(
                "vision 工具应成功（降级也算成功），实际 {:?}",
                other.is_success()
            ),
        }
        let _ = std::fs::remove_file(&png_path);
    }

    // ===== E 期 automation/ 真落地：一等自动化能力接进编排器（离线端到端） =====

    #[tokio::test]
    async fn test_orchestrator_automation_wired() {
        // 证明编排器持有一等自动化能力：内置 10 维审计轮转就绪，可注册/查询/运行。
        // 用隔离临时目录作为台账根，避免写入受限用户主目录。
        let o = Orchestrator::new(cfg());
        let ledger_root = std::env::temp_dir().join(format!("mar_auto_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&ledger_root).unwrap();
        o.set_automation_ledger_root_for_test(ledger_root.clone());

        // 内置 10 维审计轮转自动化已就绪
        let list = o.list_automations();
        assert!(
            list.len() >= 10,
            "应至少注册 10 个内置自动化，实际 {}",
            list.len()
        );
        assert!(
            list.iter().any(|a| a.id == "audit-code-quality"),
            "应含内置审计自动化"
        );

        // 注册一个自定义自动化
        o.register_automation(
            "my-report",
            "我的周报",
            "0 9 * * 1",
            crate::automation::JobKind::Report,
        )
        .unwrap();
        let list2 = o.list_automations();
        assert!(
            list2.iter().any(|a| a.id == "my-report"),
            "应含刚注册的自动化"
        );

        // 运行一个内置自动化（真实落盘台账）
        let run = o.run_automation("audit-code-quality").unwrap();
        // F4 后审计为真实执行体：外部工具（clippy/cargo-audit/llvm-cov）缺失时
        // 会诚实降级为 WARN，而非冒充 DONE。两者均为非失败合法结果；
        // 仅当真实返回 FAILED 才是回归（说明审计本身出错或维度检查崩溃）。
        assert!(
            run.status == "DONE" || run.status == "WARN",
            "内置审计运行应成功(DONE)或不致命告警(WARN)，实际 {}",
            run.status
        );
        let ledger_path = ledger_root.join("audit-code-quality").join("memory.md");
        assert!(ledger_path.exists(), "台账应落盘到隔离目录");

        // 内置 automation 工具随能力实例注册（编排器接入 23 个工具）
        assert!(
            o.tool_count() >= 23,
            "automation 工具应已注册，实际 {}",
            o.tool_count()
        );
        let _ = std::fs::remove_dir_all(&ledger_root);
    }

    #[tokio::test]
    async fn test_automation_tool_registered_and_callable() {
        // 证明内置 automation 工具可被工具注册表真实调用，返回内置自动化清单。
        let o = Orchestrator::new(cfg());
        let mut args = std::collections::HashMap::new();
        args.insert("action".to_string(), "list".to_string());
        let reg = &o.tool_registry;
        let result = reg.execute_local_tool("automation", &args).await.unwrap();
        match result {
            crate::agent::tool_cycle::ToolResult::Success { data, .. } => {
                assert!(data.contains("已注册"), "automation 工具应回显已注册数量");
                assert!(data.contains("audit-code-quality"), "应含内置审计自动化");
            }
            other => panic!("automation 工具应成功，实际 {:?}", other.is_success()),
        }
    }

    #[tokio::test]
    async fn test_serve_loopback_listener_wired() {
        // 证明：编排器经 ACP 真实回环 listener 暴露，本地 sidecar 可连端口调用
        // ping / health / agent.invoke，且 agent.invoke 真实路由到离线分析（不调 LLM）。
        let o = Arc::new(Orchestrator::new(cfg()));
        let invoker: Arc<dyn AgentInvoker> = Arc::new(EngineInvoker::new(o.clone()));
        let session = Arc::new(AcpSession::new("srv", invoker));
        let handle = start_listener(session, 0).await.unwrap();
        let addr = handle.addr;
        assert_eq!(addr.ip().to_string(), "127.0.0.1", "ACP 仅回环暴露");

        // ping → pong
        {
            let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
            client
                .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n")
                .await
                .unwrap();
            let mut reader = tokio::io::BufReader::new(client);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            assert!(
                line.contains("\"pong\":true"),
                "ping 应回 pong，实际: {line}"
            );
            assert!(line.contains("\"session_id\":\"srv\""));
        }

        // health → 引擎状态快照
        {
            let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
            client
                .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"health\"}\n")
                .await
                .unwrap();
            let mut reader = tokio::io::BufReader::new(client);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            assert!(
                line.contains("\"status\":\"ok\""),
                "health 应 ok，实际: {line}"
            );
            assert!(line.contains("\"tools\":"), "health 应含 tools 数");
            assert!(line.contains("\"loopback_only\":true"));
        }

        // agent.invoke → 离线 analyze 结果
        {
            let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
            client
                .write_all(
                    b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"agent.invoke\",\"params\":{\"prompt\":\"explain closures\"}}\n",
                )
                .await
                .unwrap();
            let mut reader = tokio::io::BufReader::new(client);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            assert!(
                line.contains("Solo Agent"),
                "agent.invoke 应回离线分析策略(Solo Agent)，实际: {line}"
            );
        }
    }

    // ===== E 期治理接线：宪法前置裁决（离线，不依赖 LLM） =====

    #[test]
    fn test_evaluate_dispatch_safe_solo_passes() {
        let o = Orchestrator::new(cfg());
        let d = o.evaluate_dispatch("解释一下闭包是什么");
        assert!(d.approved, "安全 Solo 输入应过治理门");
        assert!(!d.requires_human_gate);
    }

    #[test]
    fn test_evaluate_dispatch_codetest_team_passes() {
        let o = Orchestrator::new(cfg());
        let d = o.evaluate_dispatch("实现登录功能并编写单元测试");
        assert!(d.approved, "普通编码+测试团队应过治理门（无破坏性/越权）");
    }

    #[test]
    fn test_evaluate_dispatch_destructive_blocked() {
        // 自动编排的破坏性操作必须被宪法 C1 拦截并要求人工放行
        let o = Orchestrator::new(cfg());
        let d = o.evaluate_dispatch("删除 /tmp 下所有文件并重建项目");
        assert!(!d.approved, "自动编排的破坏性操作须被宪法 C1 拦截");
        assert!(d.requires_human_gate, "破坏性违规需人工放行");
        assert!(d.violations.iter().any(|v| v.rule_id == "C1"));
    }

    #[test]
    fn test_evaluate_dispatch_secret_read_blocked() {
        // orchestrator 身份读取密钥须被 C2 拦截
        let o = Orchestrator::new(cfg());
        let d = o.evaluate_dispatch("读取我的 API key 环境变量");
        assert!(!d.approved, "orchestrator 读密钥须被 C2 拦截");
        assert!(d.violations.iter().any(|v| v.rule_id == "C2"));
    }

    // ===== E 期治理闭环：产出后完成门禁（离线，不依赖 LLM） =====

    #[test]
    fn test_evaluate_completion_placation_blocked() {
        // 完成门禁应抓出「声称完成但无实质内容」的敷衍产出
        let o = Orchestrator::new(cfg());
        let v = o.evaluate_completion("修复登录 bug", "已修复问题，请查看。", &[], None);
        assert!(!v.allowed, "敷衍话术无实质内容应被完成门禁拦截");
        assert!(v.blocked_by.iter().any(|b| matches!(
            b,
            crate::governance::completion_gate::BlockReason::PlacationDetected
        )));
    }

    #[test]
    fn test_evaluate_completion_clean_passes() {
        // 正常带代码块的完成报告应过门
        let o = Orchestrator::new(cfg());
        let v = o.evaluate_completion(
            "解释闭包",
            "闭包是捕获环境的匿名函数，示例：\n```rust\nlet f = |x| x + 1;\n```",
            &[],
            None,
        );
        assert!(v.allowed, "正常完成报告应过完成门禁");
    }

    #[test]
    fn test_completion_gate_blocks_missing_claimed_file() {
        // 让完成门禁真正核验 agent 自报文件：回复声称创建了 src/ghost.rs，
        // 但工作区里根本没有该文件 → 反幻觉验证失败 → 拦截。
        let mut o = Orchestrator::new(cfg());
        let dir = std::env::temp_dir().join(format!("bloom_cg_int_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        o.set_workspace_for_test(dir.clone());
        let v = o.evaluate_completion(
            "实现登录",
            "已实现登录逻辑，创建文件 src/ghost.rs 含完整实现。",
            &[],
            None,
        );
        assert!(
            !v.allowed,
            "声称创建但不存在的文件应被完成门禁拦截（反幻觉）"
        );
        assert!(
            v.blocked_by.iter().any(|b| matches!(
                b,
                crate::governance::completion_gate::BlockReason::VerificationFailed
            )),
            "应因验证失败被拦截"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_completion_gate_passes_existing_claimed_file() {
        // 回复声称创建的文件确实落盘 → 验证通过 → 允许（无幻觉）。
        let mut o = Orchestrator::new(cfg());
        let dir = std::env::temp_dir().join(format!("bloom_cg_int2_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("src/real.rs");
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        std::fs::write(&f, "pub fn login() {}").unwrap();
        o.set_workspace_for_test(dir.clone());
        let v = o.evaluate_completion("实现登录", "已实现登录，代码位于 src/real.rs。", &[], None);
        assert!(
            v.allowed,
            "声称创建且真实落盘的文件应通过完成门禁（无幻觉）"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ===== Bloom 模式路由：dispatch 必须走 BloomOrchestrator 并行路径 =====

    /// 离线配置：指向本地保留端口（连接立即被拒），保证测试不触网。
    /// provider 置为 custom 才会真正采用 base_url（deepseek/openai 走内置域名）。
    fn offline_cfg() -> Config {
        Config {
            model: crate::persistence::config::ModelConfig {
                provider: "custom".to_string(),
                model_name: "test-model".to_string(),
                api_key: "offline-test".to_string(),
                base_url: "http://127.0.0.1:1".to_string(),
                copilot_model: None,
            },
            ..Default::default()
        }
    }

    fn offline_cfg_with_submode(submode: &str) -> Config {
        Config {
            model: crate::persistence::config::ModelConfig {
                provider: "custom".to_string(),
                model_name: "test-model".to_string(),
                api_key: "offline-test".to_string(),
                base_url: "http://127.0.0.1:1".to_string(),
                copilot_model: None,
            },
            bloom: crate::persistence::config::BloomConfig {
                parallel_enabled: true,
                submode: submode.to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn drain_bus(rx: &mut tokio::sync::mpsc::Receiver<Envelope>) -> Vec<Envelope> {
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            out.push(e);
        }
        out
    }

    fn thinking_texts(events: &[Envelope]) -> Vec<String> {
        events
            .iter()
            .filter(|e| e.r#type == EventType::Thinking)
            .map(|e| e.payload["text"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    #[test]
    fn test_mode_defaults_to_agent_and_is_settable() {
        let mut o = Orchestrator::new(cfg());
        assert_eq!(o.mode(), ModeKind::Agent, "默认模式应为 Agent（行为不变）");
        o.set_mode(ModeKind::Bloom);
        assert_eq!(o.mode(), ModeKind::Bloom);
    }

    #[tokio::test]
    async fn test_dispatch_bloom_mode_routes_to_parallel_orchestrator() {
        // ModeKind::Bloom 下 dispatch 必须转入 BloomOrchestrator 并行路径，
        // 且 Captain / Copilot 双成员的并行起始帧经事件总线下发前端。
        let bus = Arc::new(EventBus::default());
        let mut rx = bus.subscribe("**");
        let mut o = Orchestrator::new(offline_cfg());
        o.set_event_bus(bus.clone(), "ws.bloom".to_string());
        o.set_mode(ModeKind::Bloom);

        // 模型不可达 → dispatch 最终返回 Err；本测试只关心「路由与事件」，
        // 因此加超时护栏，避免重试退避拖慢测试套件。
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            o.dispatch("解释一下闭包是什么"),
        )
        .await;

        let events = drain_bus(&mut rx);
        let thinking = thinking_texts(&events);
        assert!(
            thinking.iter().any(|t| t.contains("Bloom 并行编排")),
            "Bloom 模式应走并行编排入口，实际思考事件：{thinking:?}"
        );
        assert!(
            !thinking.iter().any(|t| t.contains("自动编排决策")),
            "Bloom 模式不应回落到 DualLeader/Solo 常规编排"
        );

        // 注：并行双成员（Captain / Copilot）的 EnsembleProgress「proposer_start」
        // 起始帧仅在 ensemble 真正 spawn proposer（需可达模型）时下发，离线环境无此帧。
        // 该 GUI 实时帧断言已抽离至 `test_dispatch_bloom_mode_emits_ensemble_progress_live`
        //（默认 #[ignore]，需手动 `cargo test -- --ignored` 并配置 MARGUERITE_API_KEY）。
    }

    #[tokio::test]
    #[ignore = "需要真实模型：EnsembleProgress proposer_start 帧仅在实际 spawn proposer 时下发，离线不可达"]
    async fn test_dispatch_bloom_mode_emits_ensemble_progress_live() {
        // 验证 Bloom 并行编排的 GUI 实时证据：Captain / Copilot 双成员的
        // EnsembleProgress「proposer_start」帧经事件总线下发前端（对话流呈现并行证据）。
        // 该帧仅在 ensemble 真正 spawn proposer（需可达模型）时产生，故默认忽略。
        let bus = Arc::new(EventBus::default());
        let mut rx = bus.subscribe("**");
        let cfg = crate::persistence::config::Config::load().expect("加载配置");
        assert!(
            !cfg.model.api_key.is_empty(),
            "未检测到 MARGUERITE_API_KEY：请在进程环境或 .env 中设置"
        );
        let mut o = Orchestrator::new(cfg);
        o.set_event_bus(bus.clone(), "ws.bloom".to_string());
        o.set_mode(ModeKind::Bloom);

        // 模型可达时 ensemble 真实 spawn proposer；加超时护栏避免异常长时阻塞。
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            o.dispatch("解释一下闭包是什么"),
        )
        .await;

        let events = drain_bus(&mut rx);
        let starts: Vec<String> = events
            .iter()
            .filter(|e| e.r#type == EventType::EnsembleProgress)
            .filter(|e| e.payload["event_type"] == "proposer_start")
            .map(|e| {
                e.payload["proposer_label"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
        assert_eq!(
            starts,
            vec!["Captain".to_string(), "Copilot".to_string()],
            "并行双成员的起始帧应下发前端（对话流呈现并行证据）"
        );
    }

    #[tokio::test]
    async fn test_bloom_submode_auto_routes_to_auto_loop() {
        let bus = Arc::new(EventBus::default());
        let mut rx = bus.subscribe("**");
        let mut o = Orchestrator::new(offline_cfg_with_submode("auto"));
        o.set_event_bus(bus.clone(), "ws.bloom".to_string());
        o.set_mode(ModeKind::Bloom);
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            o.dispatch("实现登录并写测试"),
        )
        .await;
        let events = drain_bus(&mut rx);
        let thinking = thinking_texts(&events);
        assert!(
            thinking.iter().any(|t| t.contains("自主闭环子模式")),
            "auto 子模式应走自主闭环，实际：{thinking:?}"
        );
    }

    #[tokio::test]
    async fn test_bloom_submode_swarm_routes_to_swarm() {
        let bus = Arc::new(EventBus::default());
        let mut rx = bus.subscribe("**");
        let mut o = Orchestrator::new(offline_cfg_with_submode("swarm"));
        o.set_event_bus(bus.clone(), "ws.bloom".to_string());
        o.set_mode(ModeKind::Bloom);
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            o.dispatch("实现登录并写测试"),
        )
        .await;
        let events = drain_bus(&mut rx);
        let thinking = thinking_texts(&events);
        assert!(
            thinking.iter().any(|t| t.contains("蜂群子模式")),
            "swarm 子模式应走双领队蜂群，实际：{thinking:?}"
        );
    }

    #[test]
    fn test_qa_pass_deterministic() {
        assert!(qa_pass(
            "这是一个足够长的、完整且可验证的答案，包含充分的实现细节与验证步骤说明。"
        ));
        assert!(!qa_pass("太短"));
        assert!(!qa_pass(""));
        assert!(!qa_pass("执行失败，无法完成任务"));
    }

    #[tokio::test]
    async fn test_dispatch_non_bloom_mode_behaviour_unchanged() {
        // 向后兼容：非 Bloom 模式不得进入并行路径、不得发 ensemble 事件。
        let bus = Arc::new(EventBus::default());
        let mut rx = bus.subscribe("**");
        let mut o = Orchestrator::new(offline_cfg());
        o.set_event_bus(bus.clone(), "ws.agent".to_string());

        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            o.dispatch("解释一下闭包是什么"),
        )
        .await;

        let events = drain_bus(&mut rx);
        let thinking = thinking_texts(&events);
        assert!(
            thinking.iter().any(|t| t.contains("自动编排决策")),
            "非 Bloom 模式应维持既有编排决策链路，实际：{thinking:?}"
        );
        assert!(
            !thinking.iter().any(|t| t.contains("Bloom 并行编排")),
            "非 Bloom 模式不得进入并行编排"
        );
        assert!(
            !events
                .iter()
                .any(|e| e.r#type == EventType::EnsembleProgress),
            "非 Bloom 模式不得发出 ensemble_progress 事件"
        );
    }

    // ===== 下一阶段：依赖感知团队执行体（统一依赖执行体）=====

    #[test]
    fn assign_dependencies_orders_prereqs() {
        // 离线、确定性的角色依赖检测：测试依赖实现、文档依赖设计与实现、设计者无前置。
        let mut subs = vec![
            SubtaskSpec {
                role: "架构设计",
                system_prompt: String::new(),
                task: String::new(),
                intent: IntentType::General,
                complexity: Complexity::Simple,
                deps: Vec::new(),
            },
            SubtaskSpec {
                role: "编码实现",
                system_prompt: String::new(),
                task: String::new(),
                intent: IntentType::General,
                complexity: Complexity::Simple,
                deps: Vec::new(),
            },
            SubtaskSpec {
                role: "测试验证",
                system_prompt: String::new(),
                task: String::new(),
                intent: IntentType::General,
                complexity: Complexity::Simple,
                deps: Vec::new(),
            },
            SubtaskSpec {
                role: "文档",
                system_prompt: String::new(),
                task: String::new(),
                intent: IntentType::General,
                complexity: Complexity::Simple,
                deps: Vec::new(),
            },
        ];
        assign_dependencies(&mut subs);
        let get = |r: &str| subs.iter().find(|s| s.role == r).unwrap().deps.clone();
        assert!(
            get("架构设计").is_empty(),
            "设计者前置（调研/需求）不存在时应无依赖"
        );
        assert!(get("编码实现").is_empty(), "实现者无前置");
        assert_eq!(
            get("测试验证"),
            vec!["编码实现".to_string()],
            "测试应依赖实现"
        );
        let doc = get("文档");
        assert!(doc.contains(&"架构设计".to_string()), "文档应依赖架构设计");
        assert!(doc.contains(&"编码实现".to_string()), "文档应依赖实现");
    }

    #[test]
    fn build_team_wires_deps_to_member_dep() {
        // build_team 把带 deps 的 spec 接到 member_dep（而非 member），无依赖走 member。
        let subs = vec![
            SubtaskSpec {
                role: "编码实现",
                system_prompt: String::new(),
                task: String::new(),
                intent: IntentType::General,
                complexity: Complexity::Simple,
                deps: vec![],
            },
            SubtaskSpec {
                role: "测试验证",
                system_prompt: String::new(),
                task: String::new(),
                intent: IntentType::General,
                complexity: Complexity::Simple,
                deps: vec!["编码实现".to_string()],
            },
        ];
        let team = build_team(&subs);
        assert_eq!(team.members.len(), 2);
        let test_m = team
            .members
            .iter()
            .find(|m| m.role == "测试验证")
            .expect("应有测试成员");
        assert_eq!(
            test_m.deps,
            vec!["编码实现".to_string()],
            "build_team 应把 deps 接到 member_dep"
        );
        let code_m = team
            .members
            .iter()
            .find(|m| m.role == "编码实现")
            .expect("应有实现成员");
        assert!(code_m.deps.is_empty(), "无依赖成员走 member()");
    }

    #[tokio::test]
    async fn analyze_assigns_role_deps_offline() {
        // 经真实 analyze 路径（规则引擎，离线）验证角色依赖检测已接入编排决策。
        let o = Orchestrator::new(cfg());
        let a = o.analyze("为登录功能实现代码并编写单元测试");
        let test_spec = a
            .subtasks
            .iter()
            .find(|s| s.role == "测试")
            .expect("analyze 应检测到测试角色");
        assert!(
            test_spec.deps.iter().any(|d| d == "编码实现"),
            "测试角色应依赖编码实现: {:?}",
            test_spec.deps
        );
    }

    // ===== C6：统一依赖执行体收敛（dispatch_team_dag 改走 build_team + run_p2p）=====

    /// 离线子 Agent 管理器：模型不可达（base_url 指向拒绝连接的本地端口），spawn 会失败，
    /// 但 run_p2p 的 spawn/wait/收集链路仍真实跑通——证明 DAG 统一执行体不依赖 LLM。
    fn offline_subagents() -> SubAgentManager {
        let cfg = crate::persistence::config::Config {
            model: crate::persistence::config::ModelConfig {
                provider: "custom".to_string(),
                model_name: "offline-model".to_string(),
                api_key: "offline".to_string(),
                base_url: "http://127.0.0.1:1".to_string(),
                copilot_model: None,
            },
            ..Default::default()
        };
        let router = std::sync::Arc::new(ModelRouter::new(&cfg));
        let reg = std::sync::Arc::new(ToolRegistry::new());
        SubAgentManager::new(SubAgentConfig::default(), reg, router)
    }

    #[test]
    fn dag_team_maps_id_deps_to_roles() {
        // C6：DAG 依赖以节点 id 描述，run_p2p 以角色名调度；dag_team 做 id→role 映射。
        // Build：t4 依赖 t3(id) → 角色「编码实现」。
        let build = dag::decompose("实现登录功能", dag::DagKind::Build);
        let build_team = dag_team(&build);
        let build_t4 = build_team
            .members
            .iter()
            .find(|m| m.role == "测试验证")
            .expect("应有测试成员");
        assert_eq!(
            build_t4.deps,
            vec!["编码实现".to_string()],
            "Build t4 依赖应映射为角色「编码实现」"
        );

        // FanOut（枚举 3 项 → 6 节点）：t4 依赖全部实现节点 id，应折叠为单条「编码实现」，
        // 但 3 个实现节点本身不得被角色键折叠。
        let fan = dag::decompose(
            "实现模块：\n1. auth\n2. db\n3. api",
            dag::DagKind::FanOut { fanout: 2 },
        );
        let fan_team = dag_team(&fan);
        assert_eq!(
            fan_team.members.len(),
            6,
            "FanOut 6 节点应各自成为成员，不被角色折叠"
        );
        let impls: Vec<_> = fan_team
            .members
            .iter()
            .filter(|m| m.role == "编码实现")
            .collect();
        assert_eq!(impls.len(), 3, "3 个枚举实现节点应保留为 3 个独立成员");
        let fan_t4 = fan_team
            .members
            .iter()
            .find(|m| m.role == "测试验证")
            .expect("应有测试成员");
        assert_eq!(
            fan_t4.deps,
            vec!["编码实现".to_string()],
            "FanOut t4 依赖应折叠为单条「编码实现」（波边界屏障保证全量扇入）"
        );
    }

    #[tokio::test]
    async fn dag_unified_executor_preserves_fanout_offline() {
        // C6 端到端（离线）：decompose → dag_team → run_p2p → per-id 重建 → aggregate。
        // 验证 FanOut 的多个同角色实现节点不被 run_p2p 的角色键折叠，且聚合按节点数产出。
        let subagents = offline_subagents();
        let nodes = dag::decompose(
            "实现模块：\n1. auth\n2. db\n3. api",
            dag::DagKind::FanOut { fanout: 2 },
        );
        let team = dag_team(&nodes);
        let outcome = team.run_p2p(&subagents, "实现模块").await;
        // 关键回归守卫：run_p2p 收集的成员数 == DAG 节点数（fan-out 3 个编码实现各自独立保留）
        assert_eq!(
            outcome.member_results.len(),
            nodes.len(),
            "FanOut 节点不应被 run_p2p 的角色键折叠"
        );
        // per-id 重建（与 run_team_dag 同源逻辑）：每个节点 id 唯一映射到一条产出
        let mut outputs: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for (n, (_, r)) in nodes.iter().zip(outcome.member_results.iter()) {
            outputs.insert(n.id.clone(), r.clone());
        }
        assert_eq!(
            outputs.len(),
            nodes.len(),
            "per-id 重建应产生与节点数一致的产出（无 fan-out 坍缩）"
        );
        let parts = dag::aggregate(&nodes, &outputs);
        assert_eq!(parts.len(), nodes.len(), "aggregate 应按节点数产出 parts");
    }
}

/// 实时 DeepSeek 集成测试（默认忽略，需手动 `cargo test -- --ignored` 并配置 MARGUERITE_API_KEY）。
///
/// 目的：证明「接真实 LLM 后端」已闭环——`dispatch` 经 `ModelRouter → LLMClient → DeepSeek`
/// 产出真实 `lead_reply`，而非离线占位。运行前确保在进程环境或项目根 `.env` 中设置
/// `MARGUERITE_API_KEY`（以及可选的 `MARGUERITE_MODEL`）。
#[cfg(test)]
mod live_llm_tests {
    use super::*;
    use crate::persistence::config::Config;

    #[tokio::test]
    #[ignore = "需要真实 DeepSeek API Key 与网络；手动运行以验证 LLM 后端闭环"]
    async fn live_dispatch_produces_real_reply() {
        // Config::load 会自动从环境变量 / 项目根 .env 注入 MARGUERITE_API_KEY。
        let config = Config::load().expect("加载配置");
        assert!(
            !config.model.api_key.is_empty(),
            "未检测到 MARGUERITE_API_KEY：请在进程环境或 .env 中设置"
        );

        let mut o = Orchestrator::new(config);
        let outcome = o
            .dispatch("用一句话向初学者解释 Rust 的所有权（ownership）是什么。")
            .await
            .expect("dispatch 不应返回错误");

        assert!(
            !outcome.lead_reply.trim().is_empty(),
            "lead_reply 为空：LLM 后端未产出真实回复"
        );
        println!(
            "[live] strategy={:?} reply_len={}",
            outcome.strategy,
            outcome.lead_reply.len()
        );
        println!(
            "[live] reply_head={}",
            &outcome.lead_reply.chars().take(80).collect::<String>()
        );
    }

    #[tokio::test]
    #[ignore = "需要真实 DeepSeek API Key 与网络；手动运行以验证统一依赖执行体 run_p2p 端到端"]
    async fn live_run_p2p_dependency_real() {
        // 真实 LLM 联调：直接驱动「统一依赖执行体」Team::run_p2p + member_dep，
        // 证明跨波依赖调度与上游产出注入在真实后端下端到端闭环（非离线占位）。
        let config = Config::load().expect("加载配置");
        assert!(
            !config.model.api_key.is_empty(),
            "未检测到 MARGUERITE_API_KEY：请在进程环境或 .env 中设置"
        );

        let router = Arc::new(ModelRouter::new(&config));
        let reg = Arc::new(ToolRegistry::new());
        let subagents = SubAgentManager::new(SubAgentConfig::default(), reg, router);

        // 两波依赖：调研（无依赖） → 编码实现（依赖「调研」角色）。
        let team = Team::new()
            .member(
                "调研",
                "你是严谨的技术调研员，输出简洁准确。",
                "用一句话向初学者解释 Rust 的所有权（ownership）是什么。",
                IntentType::General,
                Complexity::Medium,
            )
            .member_dep(
                "编码实现",
                "你是资深 Rust 工程师，给出可运行示例。",
                "基于上游【调研】结论，给出一个演示所有权的最小 Rust 函数示例，并配一行中文说明。",
                IntentType::Code,
                Complexity::Medium,
                &["调研"],
            );

        let outcome = team.run_p2p(&subagents, "实现一个所有权教学示例").await;

        // 1) 两个成员都应被真实调度并收集（依赖调度生效，非离线占位）。
        assert_eq!(outcome.member_results.len(), 2, "应收集到 2 个成员结论");
        let roles: Vec<&str> = outcome
            .member_results
            .iter()
            .map(|(r, _)| r.as_str())
            .collect();
        assert!(roles.contains(&"调研"), "应保留调研角色");
        assert!(roles.contains(&"编码实现"), "应保留编码实现角色");

        // 2) 真实后端产出：不得是执行失败 / 未完成 / spawn 失败的占位串。
        for (role, res) in &outcome.member_results {
            assert!(
                !res.starts_with("[子专家") && !res.starts_with("[成员 spawn"),
                "成员 {} 返回真实产出，而非错误占位：{}",
                role,
                &res.chars().take(60).collect::<String>()
            );
            assert!(
                res.trim().len() > 20,
                "成员 {} 产出应非空且有实质内容",
                role
            );
        }

        // 3) 依赖顺序：编码实现应在调研之后（跨波调度生效的实证）。
        let idx_research = roles.iter().position(|&r| r == "调研").unwrap();
        let idx_impl = roles.iter().position(|&r| r == "编码实现").unwrap();
        assert!(
            idx_impl > idx_research,
            "编码实现（依赖方）应在调研之后被收集"
        );

        println!(
            "[live][run_p2p] research_len={} impl_len={}",
            outcome.member_results[0].1.len(),
            outcome.member_results[1].1.len()
        );
        println!(
            "[live][run_p2p] research_head={}",
            &outcome.member_results[0]
                .1
                .chars()
                .take(80)
                .collect::<String>()
        );
        println!(
            "[live][run_p2p] impl_head={}",
            &outcome.member_results[1]
                .1
                .chars()
                .take(160)
                .collect::<String>()
        );
    }
}
