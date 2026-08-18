//! 原生 harness 层：子代理隔离 / 生命周期钩子 / 动态路由 / 检查点
//!
//! 本模块把散落在业界 agent harness 中的几项关键工程语义，蒸馏为本仓原生能力，
//! 并**复用**既有模块实现，不另起一套平行体系：
//!
//! | 能力 | 复用的既有模块 | 本层新增 |
//! |------|----------------|----------|
//! | 子代理隔离 | [`crate::agent::subagent`] | 并发闸门、独立预算、sidechain 转录 |
//! | 上下文预算 | [`crate::context::budget`] / [`crate::context::paging`] | 按子代理隔离 + 超限压缩 |
//! | 模型路由 | [`crate::model::router`] | [`RoutingPolicy`] 策略维度 |
//! | 事件埋点 | [`crate::events`] | 复用 `subagent_start/end` 信封 |
//! | 路径安全 | [`crate::tools::fs_path`] | 钩子层工作区守卫 |
//!
//! [`Harness`] 是对外单一入口：`capabilities()` 暴露能力清单，
//! `run_isolated()` 把钩子、预算、并发、路由四者串成一次完整的隔离执行。

pub mod checkpoint;
pub mod distill;
pub mod evolve;
pub mod hooks;
pub mod registry;
pub mod routing;
pub mod subagent;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};

use crate::agent::subagent::{SubAgentConfig, SubAgentManager};
use crate::context::budget::BudgetAlert;
use crate::events::EventBus;
use crate::mcp::registry::ToolRegistry;
use crate::model::router::{Complexity, IntentType, ModelRouter};

pub use checkpoint::{Checkpoint, CheckpointId, GitCheckpoints};
pub use evolve::{EvolutionRecord, EvolutionRunner};
pub use hooks::{HookContext, HookDecision, HookFn, HookOutcome, HookPhase, HookRunner};
pub use registry::{CapabilityInfo, CapabilityRegistry, Maturity, NativeCapability};
pub use routing::{HarnessRouter, RoutingPolicy};
pub use subagent::{
    ConcurrencyGate, IsolatedOutcome, IsolatedSpec, IsolatedSubagents, SidechainTranscript,
    SubagentBudget,
};

/// harness 配置
#[derive(Debug, Clone)]
pub struct HarnessConfig {
    /// 工作区根（钩子路径守卫的基准）
    pub workspace: PathBuf,
    /// 数据目录（sidechain 转录落盘根）
    pub data_dir: PathBuf,
    /// 子代理并发上限
    pub max_concurrent: usize,
    /// 每个子代理的 token 预算
    pub budget_per_subagent: u64,
    /// 默认路由策略
    pub policy: RoutingPolicy,
    /// 子代理单次运行超时
    pub subagent_timeout: Duration,
}

impl Default for HarnessConfig {
    fn default() -> Self {
        Self {
            workspace: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            data_dir: std::env::temp_dir().join("marguerite"),
            max_concurrent: 5,
            budget_per_subagent: 32_000,
            policy: RoutingPolicy::Balanced,
            subagent_timeout: Duration::from_secs(120),
        }
    }
}

/// 一次 [`Harness::run_isolated`] 的结果
#[derive(Debug, Clone)]
pub struct HarnessRun {
    /// 子代理 ID
    pub subagent_id: String,
    /// 路由选中的模型
    pub model: String,
    /// 是否成功
    pub ok: bool,
    /// 输出（失败时为错误摘要）
    pub output: String,
    /// 入参 token 估算
    pub prompt_tokens: u64,
    /// 输出 token 估算
    pub output_tokens: u64,
    /// 预算告警级别
    pub alert: BudgetAlert,
    /// 上下文是否被压缩
    pub compacted: bool,
    /// sidechain 转录路径
    pub transcript_path: PathBuf,
}

/// 原生 harness 门面
pub struct Harness {
    subagents: IsolatedSubagents,
    hooks: Arc<HookRunner>,
    router: HarnessRouter,
    capabilities: CapabilityRegistry,
    workspace: PathBuf,
    subagent_timeout: Duration,
}

impl Harness {
    /// 创建 harness
    ///
    /// 自动注册工作区守卫钩子（`PreToolCall`），使越界写入默认即被拦截。
    pub fn new(
        config: HarnessConfig,
        tool_registry: Arc<ToolRegistry>,
        model_router: Arc<ModelRouter>,
    ) -> Self {
        let manager = Arc::new(SubAgentManager::new(
            SubAgentConfig {
                max_concurrent: config.max_concurrent,
                default_timeout: config.subagent_timeout,
                model: None,
            },
            tool_registry,
            Arc::clone(&model_router),
        ));

        let hooks = Arc::new(HookRunner::new());
        hooks.register_workspace_guard(config.workspace.clone());

        Self {
            subagents: IsolatedSubagents::new(
                manager,
                config.data_dir.clone(),
                config.budget_per_subagent,
            ),
            hooks,
            router: HarnessRouter::new(model_router).with_policy(config.policy),
            capabilities: CapabilityRegistry::new(),
            workspace: config.workspace,
            subagent_timeout: config.subagent_timeout,
        }
    }

    /// 注册事件总线（子代理 start/end 埋点）
    pub fn with_event_bus(mut self, bus: Arc<EventBus>) -> Self {
        self.subagents = self.subagents.with_event_bus(bus);
        self
    }

    /// 能力清单（`/harness/capabilities` 的唯一来源）
    pub fn capabilities(&self) -> &'static [CapabilityInfo] {
        self.capabilities.all()
    }

    /// 钩子运行器（可继续注册自定义钩子）
    pub fn hooks(&self) -> &Arc<HookRunner> {
        &self.hooks
    }

    /// 路由门面
    pub fn router(&self) -> &HarnessRouter {
        &self.router
    }

    /// 可变路由门面（切换策略）
    pub fn router_mut(&mut self) -> &mut HarnessRouter {
        &mut self.router
    }

    /// 隔离子代理运行时
    pub fn subagents(&self) -> &IsolatedSubagents {
        &self.subagents
    }

    /// 工作区根
    pub fn workspace(&self) -> &PathBuf {
        &self.workspace
    }

    /// 执行一次隔离子代理任务
    ///
    /// 串联四项能力：
    /// 1. **钩子**：`PreSubagent` 可阻断（阻断则直接返回错误，不启动子代理）；
    /// 2. **路由**：按 `intent` + `complexity` + 当前策略选模型；
    /// 3. **预算 + 并发 + 转录**：委托 [`IsolatedSubagents::run`]；
    /// 4. **钩子**：`PostSubagent` 观测（其失败不影响主结果）。
    pub async fn run_isolated(
        &self,
        task: &str,
        role: &str,
        intent: IntentType,
        complexity: Complexity,
    ) -> Result<HarnessRun> {
        // 1) 前置钩子
        let pre = HookContext::new(HookPhase::PreSubagent, self.workspace.clone())
            .with_tool("subagent")
            .with_arg("task", task)
            .with_arg("role", role);
        let approved = match self.hooks.run_phase(pre)? {
            HookOutcome::Proceed(ctx) => ctx,
            HookOutcome::Aborted { by, reason } => {
                return Err(anyhow!("钩子 '{by}' 阻止子代理启动：{reason}"));
            }
        };

        // 2) 动态路由（钩子可能改写了 task/role）
        let task = approved
            .args
            .get("task")
            .map(String::as_str)
            .unwrap_or(task);
        let role = approved
            .args
            .get("role")
            .map(String::as_str)
            .unwrap_or(role);
        let model = self.router.resolve(&intent, complexity.clone());

        // 3) 预算 + 并发 + 转录
        let spec = IsolatedSpec::new(task, role)
            .with_context(format!("routed_model={model}"))
            .with_routing(intent, complexity)
            .with_timeout(self.subagent_timeout);
        let outcome = self.subagents.run(spec).await?;

        // 4) 后置钩子（观测阶段，故障不掩盖主结果）
        let post = HookContext::new(HookPhase::PostSubagent, self.workspace.clone())
            .with_tool("subagent")
            .with_subagent(&outcome.subagent_id)
            .with_arg("ok", &outcome.ok.to_string());
        let _ = self.hooks.run_phase(post);

        Ok(HarnessRun {
            subagent_id: outcome.subagent_id,
            model,
            ok: outcome.ok,
            output: outcome.output,
            prompt_tokens: outcome.prompt_tokens,
            output_tokens: outcome.output_tokens,
            alert: outcome.alert,
            compacted: outcome.compacted,
            transcript_path: outcome.transcript_path,
        })
    }
}

impl std::fmt::Debug for Harness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Harness")
            .field("workspace", &self.workspace)
            .field("capabilities", &self.capabilities.len())
            .field("policy", &self.router.policy())
            .field("gate_limit", &self.subagents.gate().limit())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::config::Config;
    use uuid::Uuid;

    fn harness(max_concurrent: usize) -> (Harness, PathBuf) {
        let dir = std::env::temp_dir().join(format!("marguerite_harness_mod_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = HarnessConfig {
            workspace: dir.clone(),
            data_dir: dir.clone(),
            max_concurrent,
            budget_per_subagent: 100_000,
            policy: RoutingPolicy::Balanced,
            subagent_timeout: Duration::from_millis(200),
        };
        let h = Harness::new(
            config,
            Arc::new(ToolRegistry::new()),
            Arc::new(ModelRouter::new(&Config::default())),
        );
        (h, dir)
    }

    #[test]
    fn test_capabilities_exposed_from_registry() {
        let (h, dir) = harness(2);
        let caps = h.capabilities();
        assert_eq!(caps.len(), NativeCapability::ALL.len());
        assert!(caps
            .iter()
            .any(|c| c.capability == NativeCapability::SubagentIsolation));
        assert!(caps
            .iter()
            .any(|c| c.capability == NativeCapability::GitCheckpoint));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_workspace_guard_registered_by_default() {
        let (h, dir) = harness(2);
        assert!(
            h.hooks()
                .hook_names(HookPhase::PreToolCall)
                .contains(&"workspace_guard".to_string()),
            "构造时应默认注册工作区守卫"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_routing_policy_switch_changes_model() {
        let (mut h, dir) = harness(2);
        h.router_mut().set_policy(RoutingPolicy::QualityFirst);
        let quality = h.router().resolve(&IntentType::Code, Complexity::Complex);
        h.router_mut().set_policy(RoutingPolicy::CostFirst);
        let cost = h.router().resolve(&IntentType::Code, Complexity::Complex);
        assert_ne!(quality, cost, "策略切换应改变选模型");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_run_isolated_wires_hooks_budget_routing() {
        let (h, dir) = harness(2);
        let run = h
            .run_isolated(
                "分析模块依赖",
                "你是架构分析专家",
                IntentType::Code,
                Complexity::Complex,
            )
            .await
            .expect("隔离运行不应报错");

        // 路由生效（Balanced + Code + Complex → 强推理模型）
        assert_eq!(run.model, "deepseek-v4-pro");
        // 预算生效
        assert!(run.prompt_tokens > 0);
        // 转录生效
        assert!(run.transcript_path.exists(), "应落 sidechain 转录文件");
        assert!(!run.subagent_id.is_empty());
        // 许可归还
        assert_eq!(h.subagents().gate().available(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_pre_subagent_hook_can_abort_run() {
        let (h, dir) = harness(2);
        h.hooks()
            .register_fn(HookPhase::PreSubagent, "deny_all", |_| {
                Ok(HookDecision::Abort("演练：禁止启动子代理".to_string()))
            });

        let err = h
            .run_isolated(
                "任意任务",
                "任意角色",
                IntentType::General,
                Complexity::Simple,
            )
            .await
            .expect_err("前置钩子阻断时应返回错误")
            .to_string();
        assert!(err.contains("deny_all"), "错误应指明阻断者：{err}");
        // 未启动子代理：闸门许可未被占用
        assert_eq!(h.subagents().gate().available(), 2);
        assert_eq!(h.subagents().tracked_count(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_default_config_is_sane() {
        let c = HarnessConfig::default();
        assert!(c.max_concurrent >= 1);
        assert!(c.budget_per_subagent > 0);
        assert_eq!(c.policy, RoutingPolicy::Balanced);
    }
}
