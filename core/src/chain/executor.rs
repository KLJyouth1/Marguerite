//! 调用链执行器（v2 工作台）
//!
//! 意图 → 有序/并行步骤链执行。每步：审批 → 分发（tool/skill/expert/guard）
//! → 超时 → 重试 → fallback；链后护栏汇总修复（≤2 轮）。
//! 借鉴 GWWC 调用链与 NexusStar 审批门控思路，复用 v1 工具/技能/子代理/护栏模块。

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use futures_util::future::join_all;

use super::schema::{ChainContext, ChainPlan, ChainStep, ChainStepType, ExecutionMode, StepOutput};
use crate::agent::subagent::{SubAgentManager, SubAgentStatus};
use crate::agent::tool_cycle::ToolResult;
use crate::cli::argv::Args;
use crate::events::{Envelope, EventBus};
use crate::guard::gateway::{ApprovalGate, ApprovalOutcome, RiskLevel};
use crate::guard::repair::{RepairConfig, RepairLoop};
use crate::guard::syntax::Language;
use crate::guard::{GuardGateway, Severity};
use crate::intent::classifier::IntentType;
use crate::intent::router::IntentContext;
use crate::mcp::registry::ToolRegistry;
use crate::mode::{GateDecision, ModeManager};
use crate::persistence::config::Config;
use crate::skills::engine::SkillEngine;

/// 步骤执行状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    /// 成功
    Succeeded,
    /// 失败
    Failed,
    /// 被 fallback 替代
    Skipped,
}

/// 单个步骤的执行结果
#[derive(Debug, Clone)]
pub struct StepOutcome {
    /// 步骤 ID
    pub step_id: String,
    /// 执行状态
    pub status: StepStatus,
    /// 结果摘要（截断）
    pub summary: String,
    /// 耗时（毫秒）
    pub duration_ms: u64,
    /// 错误信息
    pub error: Option<String>,
}

/// 调用链整体执行结果
#[derive(Debug, Clone)]
pub struct ChainOutcome {
    /// 链 ID
    pub chain_id: String,
    /// 各步骤结果
    pub steps: Vec<StepOutcome>,
    /// 链后护栏汇总报告
    pub guard_report: Option<crate::guard::GuardReport>,
    /// 是否触发过自动修复
    pub repaired: bool,
}

impl ChainOutcome {
    /// 是否全部步骤成功（无 Failed）
    pub fn succeeded(&self) -> bool {
        self.steps.iter().all(|s| s.status != StepStatus::Failed)
    }
}

/// 步骤内部执行记录（含写回上下文所需的值）
struct StepExec {
    step_id: String,
    status: StepStatus,
    summary: String,
    duration_ms: u64,
    error: Option<String>,
    value: Option<serde_json::Value>,
}

impl StepExec {
    fn succeeded(
        step_id: &str,
        summary: impl Into<String>,
        duration_ms: u64,
        value: serde_json::Value,
    ) -> Self {
        Self {
            step_id: step_id.to_string(),
            status: StepStatus::Succeeded,
            summary: summary.into(),
            duration_ms,
            error: None,
            value: Some(value),
        }
    }

    fn failed(step_id: &str, error: impl Into<String>, duration_ms: u64) -> Self {
        Self {
            step_id: step_id.to_string(),
            status: StepStatus::Failed,
            summary: String::new(),
            duration_ms,
            error: Some(error.into()),
            value: None,
        }
    }

    fn skipped(step_id: &str, reason: impl Into<String>) -> Self {
        Self {
            step_id: step_id.to_string(),
            status: StepStatus::Skipped,
            summary: reason.into(),
            duration_ms: 0,
            error: None,
            value: None,
        }
    }

    fn into_outcome(self) -> StepOutcome {
        StepOutcome {
            step_id: self.step_id,
            status: self.status,
            summary: self.summary,
            duration_ms: self.duration_ms,
            error: self.error,
        }
    }
}

/// 已插值完成的步骤（并发执行前准备好，避免并发共享 `&mut` 上下文）
struct PreparedStep {
    step: ChainStep,
    params: serde_json::Value,
    fallback: Option<Box<PreparedStep>>,
}

/// Bloom 快照安全网（YOLO 侧 git 快照）：链执行前快照，链失败自动回退
struct SafetyNet {
    git: std::sync::Mutex<crate::session::rollback::SideGit>,
    /// 失败后自动回退轮数上限
    auto_revert_max: usize,
}

/// 调用链执行器
pub struct ChainExecutor {
    tools: Arc<ToolRegistry>,
    skills: Arc<SkillEngine>,
    agents: Arc<SubAgentManager>,
    mode: Arc<ModeManager>,
    guards: Arc<GuardGateway>,
    repair: RepairLoop,
    max_concurrent: usize,
    backoff: Box<dyn Fn(u32) -> Duration + Send + Sync>,
    /// 事件总线（A4 事件流埋点；None 时无事件发布）
    events: Option<Arc<EventBus>>,
    /// 自进化经验提示（Phase F）：执行技能步骤前置到输入文本，让 Agent 越用越聪明
    experience_hint: Option<String>,
    /// Bloom 快照安全网（YOLO 语义：每链前快照 + 失败自动回退）
    safety_net: Option<SafetyNet>,
    /// 链级人设（persona）：由活跃智能体会话注入，技能步骤执行时透传给技能引擎
    /// （`skills.execute_with(.., self.persona)`），关闭"技能步不注入人设"的 C 类待办。
    /// 默认 `None`（纯技能模板，不叠加人设）。
    persona: Option<String>,
}

impl ChainExecutor {
    /// 创建执行器（内置默认护栏与修复循环，最多 2 轮）
    pub fn new(
        tools: Arc<ToolRegistry>,
        skills: Arc<SkillEngine>,
        agents: Arc<SubAgentManager>,
        mode: Arc<ModeManager>,
        events: Option<Arc<EventBus>>,
    ) -> Self {
        Self {
            tools,
            skills,
            agents,
            mode,
            guards: Arc::new(GuardGateway::new(Default::default())),
            repair: RepairLoop::new(
                GuardGateway::new(Default::default()),
                RepairConfig {
                    use_llm: false,
                    ..Default::default()
                },
            ),
            max_concurrent: 5,
            backoff: Box::new(|n| Duration::from_millis(500 * 2u64.pow(n.min(2)))),
            events,
            experience_hint: None,
            safety_net: None,
            persona: None,
        }
    }

    /// 启用 Bloom 快照安全网（YOLO：执行前快照，链失败自动回退）
    ///
    /// 独立于项目 `.git` 的侧 git 快照（`session::rollback::SideGit`），
    /// 不会污染用户仓库；仅当当前模式为 Bloom 且 `bloom.snapshot_enabled` 时
    /// `execute()` 才实际打快照与回退。
    pub fn enable_safety_net(
        &mut self,
        workspace: &Path,
        session_id: &str,
        auto_revert_max: usize,
    ) -> Result<()> {
        let git = crate::session::rollback::SideGit::new(workspace, session_id)?;
        self.safety_net = Some(SafetyNet {
            git: std::sync::Mutex::new(git),
            auto_revert_max: auto_revert_max.max(1),
        });
        Ok(())
    }

    /// 当前是否应启用快照安全网（Bloom + snapshot_enabled）
    fn safety_net_active(&self) -> bool {
        if self.safety_net.is_none() {
            return false;
        }
        if self.mode.current() != crate::mode::ModeKind::Bloom {
            return false;
        }
        self.mode.bloom_spec().snapshot_enabled
    }

    /// 注册事件总线（A4 事件流埋点接口）
    pub fn set_event_bus(&mut self, bus: Arc<EventBus>) {
        self.events = Some(bus);
    }

    /// 注入链级人设（persona）。技能步骤执行时会透传给技能引擎，使"自定义性格"
    /// 真实驱动技能产出（关闭 C 类待办）。`None` 表示不叠加人设。
    pub fn set_persona(&mut self, persona: Option<String>) {
        self.persona = persona;
    }

    /// 注入自进化经验提示（Phase F）：技能步骤执行时前置到输入文本
    pub fn set_experience_hint(&mut self, hint: String) {
        self.experience_hint = Some(hint);
    }

    /// 发布事件（若已注册事件总线）
    fn emit(&self, env: Envelope) {
        if let Some(bus) = &self.events {
            bus.publish(env);
        }
    }

    /// 设置并行组内最大并发数
    pub fn set_max_concurrent(&mut self, n: usize) {
        self.max_concurrent = n.max(1);
    }

    /// 设置重试退避策略（测试可注入零退避）
    pub fn set_backoff(&mut self, f: impl Fn(u32) -> Duration + Send + Sync + 'static) {
        self.backoff = Box::new(f);
    }

    /// 意图类型 → 链模板名（A2.4 接线：分类器意图 → 模板表）
    pub fn template_for_intent(intent: &IntentType) -> Option<&'static str> {
        match intent {
            IntentType::Explain => Some("explain"),
            IntentType::Refactor => Some("refactor"),
            IntentType::Fix => Some("fix"),
            IntentType::Test => Some("test"),
            IntentType::Doc => Some("doc"),
            IntentType::Commit => Some("commit"),
            IntentType::Search => Some("search"),
            IntentType::Question | IntentType::Unknown => None,
        }
    }

    /// 执行调用链
    pub async fn execute(&self, plan: ChainPlan, ctx: &mut ChainContext) -> Result<ChainOutcome> {
        let started = Instant::now();
        let session = plan.chain_id.clone();
        let mut outcome = ChainOutcome {
            chain_id: plan.chain_id.clone(),
            steps: Vec::new(),
            guard_report: None,
            repaired: false,
        };
        // 标记需链后护栏验证的步骤 ID
        let guarded: Vec<String> = plan
            .steps
            .iter()
            .filter(|s| s.guard)
            .map(|s| s.step_id.clone())
            .collect();

        // A4 埋点：链开始
        self.emit(Envelope::progress(&session, 0, "starting"));

        // Bloom 快照安全网：链执行前打快照（YOLO：每轮前快照）
        let mut snap_id: Option<String> = None;
        if self.safety_net_active() {
            let net = self
                .safety_net
                .as_ref()
                .expect("safety_net_active 保证 Some");
            match net
                .git
                .lock()
                .unwrap()
                .snapshot_before_turn(&format!("chain:{}", plan.chain_id))
            {
                Ok(s) => {
                    tracing::debug!(chain = %plan.chain_id, snap = %s.id, "Bloom 快照已打");
                    snap_id = Some(s.id);
                }
                Err(e) => {
                    tracing::warn!(err = ?e, "Bloom 快照失败（继续执行，无回退保障）");
                }
            }
        }

        match plan.execution_mode {
            ExecutionMode::Sequential => {
                for step in &plan.steps {
                    let prepared = prepare(step, ctx);
                    let (execs, fatal) = self.exec_prepared(prepared, &session).await;
                    for exec in execs {
                        write_back(ctx, &exec);
                        outcome.steps.push(exec.into_outcome());
                    }
                    if fatal {
                        break;
                    }
                }
            }
            ExecutionMode::Parallel => {
                for group in split_parallel_groups(&plan.steps) {
                    let prepared: Vec<PreparedStep> =
                        group.iter().map(|s| prepare(s, ctx)).collect();
                    let results = self.run_group(prepared, &session).await;
                    let mut fatal = false;
                    for (execs, f) in results {
                        for exec in execs {
                            write_back(ctx, &exec);
                            outcome.steps.push(exec.into_outcome());
                        }
                        if f {
                            fatal = true;
                        }
                    }
                    if fatal {
                        break;
                    }
                }
            }
        }

        // 链后护栏汇总：收集 guard 标记步骤的代码输出 → 检查 → 自动修复（≤2 轮）
        if !guarded.is_empty() {
            let mut code = String::new();
            for id in &guarded {
                if let Some(out) = ctx.outputs.get(id) {
                    if out.error.is_none() {
                        if let serde_json::Value::String(s) = &out.value {
                            code.push_str(s);
                            code.push('\n');
                        }
                    }
                }
            }
            if !code.trim().is_empty() {
                let lang = ctx
                    .vars
                    .get("lang")
                    .and_then(|v| v.as_str())
                    .and_then(Language::from_name)
                    .unwrap_or(Language::JavaScript);
                let result = self.repair.repair(&code, lang);
                outcome.repaired = result.attempts > 0;
                outcome.guard_report = Some(result.report);
            }
        }

        // A4 埋点：链完成
        let duration_ms = started.elapsed().as_millis() as u64;
        self.emit(Envelope::chain_done(
            &session,
            &plan.chain_id,
            duration_ms,
            outcome.repaired,
        ));

        // Bloom 快照安全网：链失败 → 恢复到链前快照（YOLO：失败回滚安全网）
        if let Some(sid) = snap_id {
            if outcome.steps.iter().any(|s| s.status == StepStatus::Failed) {
                let net = self
                    .safety_net
                    .as_ref()
                    .expect("快照已打则 safety_net 存在");
                let git = net.git.lock().unwrap();
                let mut ok = false;
                for attempt in 0..net.auto_revert_max {
                    match git.restore(&sid) {
                        Ok(()) => {
                            ok = true;
                            tracing::info!(
                                chain = %plan.chain_id,
                                snap = %sid,
                                attempt = attempt + 1,
                                "Bloom 快照安全网：链失败已回退到链前快照"
                            );
                            break;
                        }
                        Err(e) => {
                            tracing::warn!(err = ?e, "Bloom 回退到链前快照失败，继续尝试");
                        }
                    }
                }
                if ok {
                    self.emit(Envelope::progress(&session, 0, "bloom-safety-net:reverted"));
                }
            }
        }

        Ok(outcome)
    }

    /// 执行单个已插值步骤：审批 → 重试执行 → fallback
    ///
    /// 返回（步骤执行记录，是否致命错误）。致命错误会中断整条链。
    /// `session` 用于 A4 事件流埋点（路由键 `{session}.{type}`）。
    fn exec_prepared<'a>(
        &'a self,
        p: PreparedStep,
        session: &'a str,
    ) -> Pin<Box<dyn Future<Output = (Vec<StepExec>, bool)> + Send + 'a>> {
        Box::pin(async move {
            let step = &p.step;

            // 1) 审批门控
            match self.approval_decision(step) {
                GateDecision::Allow => {}
                GateDecision::Deny { reason } => {
                    let exec = StepExec::failed(&step.step_id, format!("审批被拒绝: {reason}"), 0);
                    self.emit(Envelope::tool_result(
                        session,
                        &step.target,
                        false,
                        "审批被拒绝",
                    ));
                    return (vec![exec], true);
                }
                GateDecision::Confirm { reason } => {
                    let gate = self.mode.approval_gate();
                    let args = serde_json::to_string(&p.params).unwrap_or_default();
                    let msg = match wait_approval(gate, &step.target, args).await {
                        ApprovalOutcome::Approved => None,
                        ApprovalOutcome::Denied => Some("审批被拒绝".to_string()),
                        ApprovalOutcome::Timeout => Some("审批超时".to_string()),
                        ApprovalOutcome::Pending => Some(format!("审批未通过: {reason}")),
                    };
                    if let Some(msg) = msg {
                        let exec = StepExec::failed(&step.step_id, msg.clone(), 0);
                        self.emit(Envelope::tool_result(session, &step.target, false, &msg));
                        return (vec![exec], true);
                    }
                }
            }

            // 2) 分发执行前发布 tool_call（A4 埋点）
            self.emit(Envelope::tool_call(session, &step.target, &[]));

            // 2) 分发执行（超时 + 重试 + 指数退避）
            let mut last_err = String::new();
            for attempt in 0..=step.max_retries {
                let started = Instant::now();
                match tokio::time::timeout(
                    Duration::from_secs(step.timeout_sec),
                    self.dispatch(step, &p.params),
                )
                .await
                {
                    Ok(Ok(value)) => {
                        let ms = started.elapsed().as_millis() as u64;
                        let summary = summarize(&value);
                        let exec = StepExec::succeeded(&step.step_id, summary.clone(), ms, value);
                        // A4 埋点：步骤结果（成功）
                        self.emit(Envelope::tool_result(session, &step.target, true, &summary));
                        return (vec![exec], false);
                    }
                    Ok(Err(e)) => last_err = format!("{e:#}"),
                    Err(_) => last_err = format!("步骤超时（{}s）", step.timeout_sec),
                }
                if attempt < step.max_retries {
                    tokio::time::sleep((self.backoff)(attempt)).await;
                }
            }

            // 3) fallback：原步骤标记 Skipped，执行回退步骤
            if let Some(fb) = p.fallback {
                let skipped = StepExec::skipped(
                    &step.step_id,
                    format!("失败后执行 fallback: {}", fb.step.step_id),
                );
                // A4 埋点：原步骤结果（失败，进入 fallback）
                self.emit(Envelope::tool_result(
                    session,
                    &step.target,
                    false,
                    &last_err,
                ));
                let (mut fb_execs, fb_fatal) = self.exec_prepared(*fb, session).await;
                fb_execs.insert(0, skipped);
                return (fb_execs, fb_fatal);
            }

            // A4 埋点：步骤结果（最终失败）
            let exec = StepExec::failed(&step.step_id, last_err.clone(), 0);
            self.emit(Envelope::tool_result(
                session,
                &step.target,
                false,
                &last_err,
            ));
            (vec![exec], false)
        })
    }

    /// 审批决策：步骤显式标记需要审批 → 强制确认；否则按模式门控
    fn approval_decision(&self, step: &ChainStep) -> GateDecision {
        let d = self.mode.gate(&step.target);
        if step.requires_approval {
            match d {
                GateDecision::Allow => GateDecision::Confirm {
                    reason: format!("步骤 {} 标记需要审批", step.step_id),
                },
                other => other,
            }
        } else {
            d
        }
    }

    /// 分发执行：按步骤类型路由到 tool / skill / expert / guard
    async fn dispatch(
        &self,
        step: &ChainStep,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        match step.step_type {
            ChainStepType::Tool => {
                let args = json_to_str_map(params);
                let data = self.exec_tool(&step.target, &args).await?;
                Ok(serde_json::json!(data))
            }
            ChainStepType::Skill => {
                let mut text = params.to_string();
                // 自进化：技能步骤前置相关历史经验，提升解法命中
                if let Some(hint) = &self.experience_hint {
                    if !hint.is_empty() {
                        text = format!("{hint}\n\n{text}");
                    }
                }
                let intent_ctx = IntentContext::new(text, Args::default(), Config::default());
                // 人设透传：活跃智能体会话注入的 persona 随技能调用下发给技能引擎，
                // 使"自定义性格 + 技能组合"的组合在链内也真实生效（关闭 C 类待办）。
                let out = self
                    .skills
                    .execute_with(&step.target, &intent_ctx, self.persona.as_deref())
                    .await
                    .map_err(|e| anyhow!("技能 '{}' 执行失败: {e:#}", step.target))?;
                if !out.success {
                    return Err(anyhow!("技能 '{}' 执行失败: {}", step.target, out.result));
                }
                Ok(serde_json::json!(out.result))
            }
            ChainStepType::Expert => {
                let task = params
                    .get("task")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&step.target)
                    .to_string();
                let context = params.to_string();
                let timeout = Duration::from_secs(step.timeout_sec);
                let id = self
                    .agents
                    .spawn_with_timeout(&task, &context, timeout)
                    .await
                    .map_err(|e| anyhow!("专家 '{}' 启动失败: {e:#}", step.target))?;
                let result = poll_agent(&self.agents, &id, timeout).await?;
                Ok(serde_json::json!(result))
            }
            ChainStepType::Guard => {
                let code = params
                    .get("code")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let lang = params
                    .get("lang")
                    .and_then(|v| v.as_str())
                    .and_then(Language::from_name)
                    .unwrap_or(Language::JavaScript);
                let report = self.guards.check(&code, lang);
                if report.has_error() {
                    let msgs: Vec<String> = report
                        .issues
                        .iter()
                        .filter(|i| i.severity == Severity::Error)
                        .map(|i| i.message.clone())
                        .collect();
                    return Err(anyhow!("护栏[{}]: {}", step.target, msgs.join("; ")));
                }
                Ok(serde_json::json!({"passed": true, "warnings": report.issues.len()}))
            }
        }
    }

    /// 执行工具：本地工具优先，MCP 工具兜底
    async fn exec_tool(&self, name: &str, args: &HashMap<String, String>) -> Result<String> {
        if self.tools.has_tool(name) {
            // 本地工具（get_tool 对本地返回 None）走 execute_local_tool
            if let Some(tool) = self.tools.get_tool(name) {
                return tool_result_to_string(tool.execute(args).await?);
            }
            return tool_result_to_string(self.tools.execute_local_tool(name, args).await?);
        }
        Err(anyhow!("工具 '{}' 不存在", name))
    }

    /// 并行执行一组已插值步骤（信号量限制并发）
    async fn run_group<'a>(
        &'a self,
        prepared: Vec<PreparedStep>,
        session: &'a str,
    ) -> Vec<(Vec<StepExec>, bool)> {
        let sem = Arc::new(tokio::sync::Semaphore::new(self.max_concurrent));
        let futs: Vec<_> = prepared
            .into_iter()
            .map(|p| {
                let sem = sem.clone();
                async move {
                    let _permit = sem.acquire_owned().await.expect("信号量不应被关闭");
                    self.exec_prepared(p, session).await
                }
            })
            .collect();
        join_all(futs).await
    }
}

/// 插值步骤参数（含 fallback 递归插值）
fn prepare(step: &ChainStep, ctx: &ChainContext) -> PreparedStep {
    let raw = serde_json::to_value(&step.params).unwrap_or(serde_json::Value::Null);
    let params = ctx.interpolate_value(&raw);
    let fallback = step
        .fallback
        .as_deref()
        .map(|fb| Box::new(prepare(fb, ctx)));
    PreparedStep {
        step: step.clone(),
        params,
        fallback,
    }
}

/// 将 JSON 参数扁平化为字符串参数表（工具执行接口）
fn json_to_str_map(v: &serde_json::Value) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let serde_json::Value::Object(obj) = v {
        for (k, val) in obj {
            map.insert(
                k.clone(),
                match val {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                },
            );
        }
    }
    map
}

/// 工具结果 → 文本 / 错误
fn tool_result_to_string(r: ToolResult) -> Result<String> {
    match r {
        ToolResult::Success { data, .. } => Ok(data),
        ToolResult::Error { message, code } => Err(anyhow!("[{code}] {message}")),
        ToolResult::Timeout { tool_name, .. } => Err(anyhow!("工具 '{tool_name}' 执行超时")),
    }
}

/// 等待审批结果（消费式轮询；非交互路径由 ApprovalGate 内部 fail-closed）
async fn wait_approval(gate: &ApprovalGate, tool: &str, args: String) -> ApprovalOutcome {
    let Some(pending) = gate.request(tool, args, RiskLevel::Medium) else {
        return ApprovalOutcome::Denied;
    };
    loop {
        match gate.outcome(&pending.id) {
            ApprovalOutcome::Pending => tokio::time::sleep(Duration::from_millis(50)).await,
            other => return other,
        }
    }
}

/// 轮询专家结果：完成返回结果；失败/超时快速返回错误
async fn poll_agent(agents: &SubAgentManager, id: &str, timeout: Duration) -> Result<String> {
    tokio::time::timeout(timeout, async {
        loop {
            if let Some(r) = agents.get_result(id).await {
                return Ok(r);
            }
            if let Some(status) = agents.get_status(id).await {
                if matches!(
                    status,
                    SubAgentStatus::Failed { .. } | SubAgentStatus::TimedOut
                ) {
                    return Err(anyhow!("专家执行失败或超时"));
                }
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .map_err(|_| anyhow!("专家执行超时"))?
}

/// 结果摘要（截断到 100 字符；字符串取原值，其余取紧凑 JSON）
fn summarize(v: &serde_json::Value) -> String {
    let s = match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let mut out: String = s.chars().take(100).collect();
    if s.chars().count() > 100 {
        out.push('…');
    }
    out
}

/// 将步骤执行记录写回链上下文（供后续步骤插值引用）
fn write_back(ctx: &mut ChainContext, exec: &StepExec) {
    match &exec.value {
        Some(v) => {
            ctx.outputs
                .insert(exec.step_id.clone(), StepOutput::success(v.clone()));
        }
        None => {
            ctx.outputs.insert(
                exec.step_id.clone(),
                StepOutput::failure(exec.error.clone().unwrap_or_default()),
            );
        }
    }
}

/// 并行组划分：无依赖（`{{sN.*}}` 引用）的相邻步骤合并为一组；含依赖步骤独占一组
fn split_parallel_groups(steps: &[ChainStep]) -> Vec<Vec<ChainStep>> {
    let mut groups: Vec<Vec<ChainStep>> = Vec::new();
    let mut batch: Vec<ChainStep> = Vec::new();
    for s in steps {
        if has_dependency(s) {
            if !batch.is_empty() {
                groups.push(std::mem::take(&mut batch));
            }
            groups.push(vec![s.clone()]);
        } else {
            batch.push(s.clone());
        }
    }
    if !batch.is_empty() {
        groups.push(batch);
    }
    groups
}

/// 步骤参数是否引用前序输出
fn has_dependency(s: &ChainStep) -> bool {
    s.params.values().any(|v| v.to_string().contains("{{s"))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;
    use crate::events::{EventBus, EventType};
    use crate::guard::gateway::ApprovalGate;
    use crate::intent::classifier::IntentType;
    use crate::mode::ModeKind;
    use crate::model::router::ModelRouter;
    use crate::skills::registry::{Skill, SkillRegistry, SkillSource};

    // ---------- 测试工具 ----------

    /// 可编程 mock 工具：失败次数 / 延迟 / 并发峰值 / 参数捕获
    struct MockTool {
        name: String,
        result: String,
        failures: Arc<AtomicUsize>,
        calls: Arc<AtomicUsize>,
        delay_ms: u64,
        current: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        last_args: Arc<Mutex<HashMap<String, String>>>,
    }

    #[derive(Default)]
    struct MockState {
        failures: Arc<AtomicUsize>,
        calls: Arc<AtomicUsize>,
        current: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        last_args: Arc<Mutex<HashMap<String, String>>>,
    }

    impl MockState {
        fn arg(&self, k: &str) -> String {
            self.last_args
                .lock()
                .unwrap()
                .get(k)
                .cloned()
                .unwrap_or_default()
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
        fn peak(&self) -> usize {
            self.peak.load(Ordering::SeqCst)
        }
    }

    impl MockTool {
        fn new(name: &str, result: &str, state: &Arc<MockState>) -> Self {
            Self {
                name: name.to_string(),
                result: result.to_string(),
                failures: state.failures.clone(),
                calls: state.calls.clone(),
                delay_ms: 0,
                current: state.current.clone(),
                peak: state.peak.clone(),
                last_args: state.last_args.clone(),
            }
        }
    }

    #[async_trait]
    impl crate::mcp::registry::ToolExecutor for MockTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "mock tool"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
            *self.last_args.lock().unwrap() = args.clone();
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            let cur = self.current.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(cur, Ordering::SeqCst);
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            self.current.fetch_sub(1, Ordering::SeqCst);
            if n < self.failures.load(Ordering::SeqCst) {
                return Ok(ToolResult::Error {
                    message: format!("{} 失败", self.name),
                    code: "MOCK_FAIL".to_string(),
                });
            }
            Ok(ToolResult::Success {
                data: self.result.clone(),
                metadata: HashMap::new(),
            })
        }
    }

    fn skill(name: &str) -> Skill {
        Skill {
            name: name.to_string(),
            description: format!("技能 {}", name),
            version: "1.0.0".to_string(),
            author: None,
            intent: vec![name.to_string()],
            tools: vec![],
            prompt_template: format!("技能 {} 模板渲染结果", name),
            dependencies: vec![],
            source: SkillSource::Builtin,
            rating: 0.0,
            uses: 0,
        }
    }

    struct Fixture {
        executor: ChainExecutor,
        state: Arc<MockState>,
        _approval: Arc<ApprovalGate>,
    }

    fn build(
        state: &Arc<MockState>,
        tools: Vec<Box<dyn crate::mcp::registry::ToolExecutor>>,
        skills: Vec<Skill>,
        mode: ModeManager,
    ) -> Fixture {
        let mut registry = ToolRegistry::new();
        for t in tools {
            registry.register_local(t);
        }
        let tools = Arc::new(registry);
        let mut skill_registry = SkillRegistry::new();
        for s in skills {
            skill_registry.register(s).unwrap();
        }
        let skills = Arc::new(SkillEngine::new(
            Arc::new(skill_registry),
            Arc::clone(&tools),
        ));
        let router = Arc::new(ModelRouter::new(&Config::default()));
        let agents = Arc::new(SubAgentManager::new(
            Default::default(),
            Arc::clone(&tools),
            router,
        ));
        let approval = mode.approval_gate().clone();
        let mut executor = ChainExecutor::new(tools, skills, agents, Arc::new(mode), None);
        executor.set_backoff(|_| Duration::ZERO);
        Fixture {
            executor,
            state: Arc::clone(state),
            _approval: approval,
        }
    }

    fn bloom_mode() -> ModeManager {
        let mut m = ModeManager::new();
        m.switch(ModeKind::Bloom).unwrap();
        m
    }

    fn agent_mode() -> ModeManager {
        ModeManager::new()
    }

    fn params_map(pairs: &[(&str, &str)]) -> HashMap<String, serde_json::Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), serde_json::json!(v)))
            .collect()
    }

    fn tool_step(id: &str, target: &str, params: HashMap<String, serde_json::Value>) -> ChainStep {
        ChainStep {
            step_id: id.to_string(),
            step_type: ChainStepType::Tool,
            target: target.to_string(),
            params,
            max_retries: 0,
            timeout_sec: 5,
            fallback: None,
            requires_approval: false,
            guard: false,
        }
    }

    fn skill_step(id: &str, target: &str) -> ChainStep {
        ChainStep {
            step_id: id.to_string(),
            step_type: ChainStepType::Skill,
            target: target.to_string(),
            params: HashMap::new(),
            max_retries: 0,
            timeout_sec: 5,
            fallback: None,
            requires_approval: false,
            guard: false,
        }
    }

    fn with_fallback(mut s: ChainStep, fb: ChainStep) -> ChainStep {
        s.fallback = Some(Box::new(fb));
        s
    }

    fn plan(steps: Vec<ChainStep>, vars: Vec<(&str, &str)>) -> ChainPlan {
        ChainPlan {
            chain_id: "c1".to_string(),
            intent: "test".to_string(),
            steps,
            execution_mode: ExecutionMode::Sequential,
            context: params_map(&vars),
        }
    }

    // ---------- C1：顺序链模板执行成功 ----------

    #[tokio::test]
    async fn c1_explain_chain_succeeds() {
        let state = Arc::new(MockState::default());
        let read = MockTool::new("read_file", "fn main() {}", &state);
        let fx = build(
            &state,
            vec![Box::new(read)],
            vec![skill("explain")],
            bloom_mode(),
        );
        let p = ChainPlan::from_template("c1", "explain", params_map(&[("file", "src/main.rs")]))
            .unwrap();
        let mut ctx = ChainContext::new(p.context.clone());
        let out = fx.executor.execute(p, &mut ctx).await.unwrap();
        assert!(out.succeeded(), "explain 链应全部成功: {:?}", out.steps);
        assert_eq!(out.steps.len(), 2);
        assert_eq!(out.steps[0].status, StepStatus::Succeeded);
        assert_eq!(out.steps[1].status, StepStatus::Succeeded);
        // 读文件步骤收到插值后的链变量
        assert_eq!(fx.state.arg("path"), "src/main.rs");
    }

    #[tokio::test]
    async fn c1_search_and_commit_succeed() {
        let state = Arc::new(MockState::default());
        let read = MockTool::new("read_file", "diff line", &state);
        let search = MockTool::new("search_code", "match", &state);
        let fx = build(
            &state,
            vec![Box::new(read), Box::new(search)],
            vec![skill("commit")],
            bloom_mode(),
        );

        let p = ChainPlan::from_template(
            "c1s",
            "search",
            params_map(&[("query", "foo"), ("file", "src")]),
        )
        .unwrap();
        let mut ctx = ChainContext::new(p.context.clone());
        let out = fx.executor.execute(p, &mut ctx).await.unwrap();
        assert!(out.succeeded());
        assert_eq!(fx.state.arg("query"), "foo");
        assert_eq!(fx.state.arg("path"), "src");

        let p =
            ChainPlan::from_template("c1c", "commit", params_map(&[("file", "diff.txt")])).unwrap();
        let mut ctx = ChainContext::new(p.context.clone());
        let out = fx.executor.execute(p, &mut ctx).await.unwrap();
        assert!(out.succeeded());
    }

    // ---------- C1b：技能步骤透传人设（关闭 C 类待办） ----------

    #[tokio::test]
    async fn c1b_skill_step_injects_persona() {
        let state = Arc::new(MockState::default());
        let mut fx = build(&state, vec![], vec![skill("explain")], bloom_mode());
        // 注入链级人设；技能步骤执行时透传到技能引擎的系统指令
        // （离线降级下 execute_with 把 persona 前置到结果，证明人设已透传而非被丢弃）。
        fx.executor
            .set_persona(Some("你是资深数据分析师".to_string()));

        let mut ctx = ChainContext::default();
        let out = fx
            .executor
            .execute(plan(vec![skill_step("s1", "explain")], vec![]), &mut ctx)
            .await
            .unwrap();
        assert!(out.succeeded(), "技能步骤应成功: {:?}", out.steps);
        assert_eq!(out.steps.len(), 1);
        assert!(
            out.steps[0].summary.contains("资深数据分析师"),
            "人设应随技能步骤透传并出现在产出中: {}",
            out.steps[0].summary
        );
        assert!(out.steps[0].summary.contains("模板渲染结果"));
    }

    // ---------- C2：失败 → 重试 → fallback ----------

    #[tokio::test]
    async fn c2_failure_falls_back() {
        let state = Arc::new(MockState::default());
        state.failures.store(2, Ordering::SeqCst); // 前 2 次失败
        let flaky = MockTool::new("flaky", "n/a", &state);
        let good = MockTool::new("good", "recovered", &state);
        let fx = build(
            &state,
            vec![Box::new(flaky), Box::new(good)],
            vec![],
            bloom_mode(),
        );
        // max_retries=1：第 1 次失败 → 重试（第 2 次失败）→ fallback
        let step = with_fallback(
            tool_step("s1", "flaky", HashMap::new()),
            tool_step("s2", "good", HashMap::new()),
        );
        let mut p = plan(vec![step], vec![]);
        p.steps[0].max_retries = 1;

        let mut ctx = ChainContext::new(p.context.clone());
        let out = fx.executor.execute(p, &mut ctx).await.unwrap();
        assert_eq!(out.steps.len(), 2);
        assert_eq!(out.steps[0].status, StepStatus::Skipped, "原步骤应 Skipped");
        assert_eq!(out.steps[1].status, StepStatus::Succeeded);
        assert_eq!(out.steps[1].summary, "recovered");
        assert!(out.succeeded(), "fallback 成功后整链不应有 Failed");
    }

    #[tokio::test]
    async fn c2_retry_then_success() {
        let state = Arc::new(MockState::default());
        state.failures.store(1, Ordering::SeqCst); // 仅第 1 次失败
        let flaky = MockTool::new("flaky", "ok", &state);
        let fx = build(&state, vec![Box::new(flaky)], vec![], bloom_mode());
        let mut p = plan(vec![tool_step("s1", "flaky", HashMap::new())], vec![]);
        p.steps[0].max_retries = 1;
        let mut ctx = ChainContext::new(p.context.clone());
        let out = fx.executor.execute(p, &mut ctx).await.unwrap();
        assert!(out.succeeded());
        assert_eq!(out.steps[0].status, StepStatus::Succeeded);
        assert_eq!(fx.state.calls(), 2, "应重试一次");
    }

    // ---------- C3：审批拒绝 → Failed 且不执行后续 ----------

    #[tokio::test]
    async fn c3_approval_denied_stops_chain() {
        let state = Arc::new(MockState::default());
        let read = MockTool::new("read_file", "ok", &state);
        let fx = build(&state, vec![Box::new(read)], vec![], agent_mode());
        // 非交互 → fail-closed：任何审批请求直接拒绝
        fx.executor.mode.approval_gate().set_interactive(false);

        let mut edit = tool_step("s1", "edit_file", HashMap::new());
        edit.requires_approval = true;
        let s2 = tool_step("s2", "read_file", HashMap::new());
        let mut ctx = ChainContext::default();
        let out = fx
            .executor
            .execute(plan(vec![edit, s2], vec![]), &mut ctx)
            .await
            .unwrap();

        assert_eq!(out.steps.len(), 1, "审批拒绝后不应执行 s2");
        assert_eq!(out.steps[0].status, StepStatus::Failed);
        let err = out.steps[0].error.clone().unwrap_or_default();
        assert!(err.contains("审批"), "错误应含审批原因: {err}");
    }

    #[tokio::test]
    async fn c3_approval_approved_continues() {
        let state = Arc::new(MockState::default());
        let edit = MockTool::new("edit_file", "written", &state);
        let fx = build(&state, vec![Box::new(edit)], vec![], agent_mode());
        // 自动批准：后台任务等挂起审批出现后立即批准
        let gate = fx.executor.mode.approval_gate().clone();
        tokio::spawn(async move {
            loop {
                if let Some(p) = gate.pending() {
                    gate.approve(&p.id);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });

        let mut edit_step = tool_step("s1", "edit_file", HashMap::new());
        edit_step.requires_approval = true;
        let mut ctx = ChainContext::default();
        let out = fx
            .executor
            .execute(plan(vec![edit_step], vec![]), &mut ctx)
            .await
            .unwrap();
        assert!(out.succeeded(), "批准后应成功: {:?}", out.steps);
    }

    // ---------- C4：并行组并发上限 ----------

    #[tokio::test]
    async fn c4_parallel_group_respects_max_concurrent() {
        let state = Arc::new(MockState::default());
        let mut t = MockTool::new("slow", "done", &state);
        t.delay_ms = 120;
        let mut fx = build(&state, vec![Box::new(t)], vec![], bloom_mode());
        fx.executor.set_max_concurrent(2);
        let steps: Vec<ChainStep> = (0..4)
            .map(|i| tool_step(&format!("s{i}"), "slow", HashMap::new()))
            .collect();
        let mut p = plan(steps, vec![]);
        p.execution_mode = ExecutionMode::Parallel;
        let mut ctx = ChainContext::default();
        let out = fx.executor.execute(p, &mut ctx).await.unwrap();
        assert!(out.succeeded());
        assert_eq!(out.steps.len(), 4);
        assert_eq!(fx.state.peak(), 2, "并发峰值应 ≤ 2");
    }

    // ---------- C5：链内变量插值 ----------

    #[tokio::test]
    async fn c5_step_output_interpolation() {
        let state = Arc::new(MockState::default());
        let read = MockTool::new("read_file", "hello", &state);
        let echo = MockTool::new("echo", "echoed", &state);
        let fx = build(
            &state,
            vec![Box::new(read), Box::new(echo)],
            vec![],
            bloom_mode(),
        );
        let s1 = tool_step("s1", "read_file", params_map(&[("path", "$file")]));
        let s2 = tool_step(
            "s2",
            "echo",
            params_map(&[("input", "{{s1.output}}"), ("ref", "$file")]),
        );
        let mut ctx = ChainContext::new(params_map(&[("file", "a.rs")]));
        let out = fx
            .executor
            .execute(plan(vec![s1, s2], vec![]), &mut ctx)
            .await
            .unwrap();
        assert!(out.succeeded());
        assert_eq!(
            fx.state.arg("input"),
            "hello",
            "{{s1.output}} 应替换为前序输出"
        );
        assert_eq!(fx.state.arg("ref"), "a.rs", "$file 应替换为链变量");
    }

    // ---------- C6：护栏汇总 → 自动修复 ----------

    #[tokio::test]
    async fn c6_guard_report_and_repair() {
        let state = Arc::new(MockState::default());
        // 返回缺右括号的 JS 代码 → 链后修复
        let bad = MockTool::new("gen", "function foo() {\n  return 1;\n", &state);
        let fx = build(&state, vec![Box::new(bad)], vec![], bloom_mode());
        let mut s1 = tool_step("s1", "gen", HashMap::new());
        s1.guard = true;
        let mut ctx = ChainContext::default();
        let out = fx
            .executor
            .execute(plan(vec![s1], vec![]), &mut ctx)
            .await
            .unwrap();
        assert!(out.repaired, "应触发自动修复");
        let report = out.guard_report.expect("应有护栏报告");
        assert!(
            !report.has_error(),
            "修复后不应有 Error 级问题: {:?}",
            report.issues
        );
    }

    #[tokio::test]
    async fn c6_clean_guard_no_repair() {
        let state = Arc::new(MockState::default());
        let good = MockTool::new("gen", "const x = 1;\n", &state);
        let fx = build(&state, vec![Box::new(good)], vec![], bloom_mode());
        let mut s1 = tool_step("s1", "gen", HashMap::new());
        s1.guard = true;
        let mut ctx = ChainContext::default();
        let out = fx
            .executor
            .execute(plan(vec![s1], vec![]), &mut ctx)
            .await
            .unwrap();
        assert!(!out.repaired);
        assert!(out.guard_report.as_ref().unwrap().passed);
    }

    // ---------- C7：致命错误中断链 ----------

    #[tokio::test]
    async fn c7_fatal_error_skips_rest() {
        let state = Arc::new(MockState::default());
        let read = MockTool::new("read_file", "ok", &state);
        let fx = build(&state, vec![Box::new(read)], vec![], agent_mode());
        fx.executor.mode.approval_gate().set_interactive(false);
        let mut edit = tool_step("s1", "edit_file", HashMap::new());
        edit.requires_approval = true;
        let s2 = tool_step("s2", "read_file", HashMap::new());
        let s3 = tool_step("s3", "read_file", HashMap::new());
        let mut ctx = ChainContext::default();
        let out = fx
            .executor
            .execute(plan(vec![edit, s2, s3], vec![]), &mut ctx)
            .await
            .unwrap();
        assert_eq!(out.steps.len(), 1, "s1 拒绝后 s2/s3 应跳过");
        assert_eq!(out.steps[0].status, StepStatus::Failed);
    }

    // ---------- 辅助 ----------

    #[test]
    fn test_template_for_intent_mapping() {
        assert_eq!(
            ChainExecutor::template_for_intent(&IntentType::Explain),
            Some("explain")
        );
        assert_eq!(
            ChainExecutor::template_for_intent(&IntentType::Fix),
            Some("fix")
        );
        assert_eq!(
            ChainExecutor::template_for_intent(&IntentType::Search),
            Some("search")
        );
        assert_eq!(
            ChainExecutor::template_for_intent(&IntentType::Question),
            None
        );
        assert_eq!(
            ChainExecutor::template_for_intent(&IntentType::Unknown),
            None
        );
    }

    #[test]
    fn test_split_parallel_groups() {
        use crate::chain::schema::ChainStepType;
        let mk = |id: &str, param: Option<&str>| {
            let mut s = tool_step(id, "t", HashMap::new());
            if let Some(v) = param {
                s.params = params_map(&[("code", v)]);
            }
            s
        };
        let _ = ChainStepType::Tool;
        let steps = vec![
            mk("s1", None),
            mk("s2", None),
            mk("s3", Some("{{s2.output}}")),
            mk("s4", None),
        ];
        let groups = split_parallel_groups(&steps);
        assert_eq!(groups.len(), 3, "依赖步骤应独占一组");
        assert_eq!(groups[0].len(), 2);
        assert_eq!(groups[1][0].step_id, "s3");
        assert_eq!(groups[2].len(), 1);
    }

    // ---------- C8：A4 事件流埋点（chain → bus）----------

    #[tokio::test]
    async fn c8_chain_publishes_events() {
        let state = Arc::new(MockState::default());
        let read = MockTool::new("read_file", "ok", &state);
        let mut fx = build(&state, vec![Box::new(read)], vec![], bloom_mode());
        let bus = Arc::new(EventBus::default());
        fx.executor.set_event_bus(Arc::clone(&bus));

        let since = std::time::Instant::now();
        let mut ctx = ChainContext::default();
        let out = fx
            .executor
            .execute(
                plan(vec![tool_step("s1", "read_file", HashMap::new())], vec![]),
                &mut ctx,
            )
            .await
            .unwrap();
        assert!(out.succeeded());

        // 回放应包含 progress(starting) / tool_call / tool_result(ok) / chain_done
        let replayed = bus.history(since);
        let types: Vec<EventType> = replayed.iter().map(|e| e.r#type).collect();
        assert!(
            types.contains(&EventType::Progress),
            "缺少 progress(starting)"
        );
        assert!(types.contains(&EventType::ToolCall), "缺少 tool_call");
        assert!(types.contains(&EventType::ToolResult), "缺少 tool_result");
        assert!(types.contains(&EventType::Done), "缺少 chain_done");
    }

    #[tokio::test]
    async fn c8_chain_publishes_failure_events() {
        let state = Arc::new(MockState::default());
        state.failures.store(5, Ordering::SeqCst); // 始终失败，无 fallback
        let bad = MockTool::new("flaky", "n/a", &state);
        let mut fx = build(&state, vec![Box::new(bad)], vec![], bloom_mode());
        let bus = Arc::new(EventBus::default());
        fx.executor.set_event_bus(Arc::clone(&bus));

        let mut p = plan(vec![tool_step("s1", "flaky", HashMap::new())], vec![]);
        p.steps[0].max_retries = 0;
        let since = std::time::Instant::now();
        let mut ctx = ChainContext::default();
        let out = fx.executor.execute(p, &mut ctx).await.unwrap();
        assert!(!out.succeeded());

        let replayed = bus.history(since);
        let ok_results = replayed
            .iter()
            .filter(|e| {
                e.r#type == EventType::ToolResult && e.payload["ok"] == serde_json::json!(true)
            })
            .count();
        assert_eq!(ok_results, 0, "失败链不应产生 ok=true 的 tool_result");
        assert!(
            replayed.iter().any(|e| e.r#type == EventType::ToolResult
                && e.payload["ok"] == serde_json::json!(false)),
            "失败步骤应产生 ok=false 的 tool_result"
        );
    }

    // ---------- C9：Bloom 快照安全网（YOLO 侧 git 快照 + 失败自动回退） ----------

    /// 执行时改写工作区文件并报错的工具（模拟执行中破坏）
    struct FileWritingTool {
        path: std::path::PathBuf,
    }

    #[async_trait]
    impl crate::mcp::registry::ToolExecutor for FileWritingTool {
        fn name(&self) -> &str {
            "file_writer"
        }
        fn description(&self) -> &str {
            "write file then fail"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, _args: &HashMap<String, String>) -> Result<ToolResult> {
            std::fs::write(&self.path, "mutated-by-agent").unwrap();
            Ok(ToolResult::Error {
                message: "boom".into(),
                code: "X".into(),
            })
        }
    }

    #[tokio::test]
    async fn c9_safety_net_snapshots_and_reverts_on_failure() {
        let tmp =
            std::env::temp_dir().join(format!("marguerite-safetynet-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let ws = std::fs::canonicalize(&tmp).unwrap_or(tmp);
        std::fs::write(ws.join("a.txt"), "base").unwrap();

        let state = Arc::new(MockState::default());
        let writer = FileWritingTool {
            path: ws.join("a.txt"),
        };
        let mut fx = build(&state, vec![Box::new(writer)], vec![], bloom_mode());
        fx.executor.enable_safety_net(&ws, "sess-safe", 2).unwrap();

        let p = plan(vec![tool_step("s1", "file_writer", HashMap::new())], vec![]);
        let mut ctx = ChainContext::default();
        let out = fx.executor.execute(p, &mut ctx).await.unwrap();
        assert!(!out.succeeded(), "写入并报错的工具应使链失败");

        // 失败后自动回退 → 文件恢复为执行前状态
        assert_eq!(
            std::fs::read_to_string(ws.join("a.txt")).unwrap(),
            "base",
            "Bloom 快照安全网应回退到链执行前"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[tokio::test]
    async fn c9_safety_net_inactive_in_agent_mode() {
        let tmp =
            std::env::temp_dir().join(format!("marguerite-safetynet-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let ws = std::fs::canonicalize(&tmp).unwrap_or(tmp);
        std::fs::write(ws.join("a.txt"), "base").unwrap();

        // Agent 模式（非 Bloom）：即使 enable_safety_net 已启用也不打快照
        let state = Arc::new(MockState::default());
        let writer = FileWritingTool {
            path: ws.join("a.txt"),
        };
        let mut fx = build(&state, vec![Box::new(writer)], vec![], agent_mode());
        fx.executor.enable_safety_net(&ws, "sess-safe2", 2).unwrap();

        let p = plan(vec![tool_step("s1", "file_writer", HashMap::new())], vec![]);
        let mut ctx = ChainContext::default();
        let out = fx.executor.execute(p, &mut ctx).await.unwrap();
        assert!(!out.succeeded());
        // 非 Bloom 模式不启用快照回退 → 文件保持被改写状态
        assert_eq!(
            std::fs::read_to_string(ws.join("a.txt")).unwrap(),
            "mutated-by-agent"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }
}
