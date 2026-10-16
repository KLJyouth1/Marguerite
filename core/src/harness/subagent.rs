//! 原生子代理隔离运行时
//!
//! 在既有 [`SubAgentManager`] 之上补齐三件「隔离」能力：
//!
//! 1. **并发闸门**（[`ConcurrencyGate`]）：以 `tokio::sync::Semaphore` 真正限制
//!    同时在跑的子代理数。许可在任务开始前获取、任务结束后释放。
//! 2. **上下文预算隔离**（[`SubagentBudget`]）：每个子代理一份独立
//!    [`TokenBudget`]，累计消息 token 估算；超阈值时用
//!    [`crate::context::paging`] 折叠压缩，而不是无声截断。
//! 3. **独立 sidechain 转录**（[`SidechainTranscript`]）：每个子代理的轮次
//!    追加到 `<data_dir>/harness/subagents/<id>.jsonl`，一个子代理一个文件、
//!    一把文件级锁，互不串写。
//!
//! Token 估算复用 [`crate::context::session::estimate_tokens`]（区分 CJK 的
//! 高精度启发式），不另造一套估算逻辑。

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde_json::json;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

use crate::agent::subagent::{SubAgentManager, SubAgentStatus};
use crate::agent::worktree::{WorktreeHandle, WorktreeManager};
use crate::context::budget::{BudgetAlert, TokenBudget, TokenBudgetError};
use crate::context::paging::{PagingEngine, PagingOptions};
use crate::context::session::estimate_tokens;
use crate::events::{Envelope, EventBus};
use crate::model::router::{Complexity, IntentType};
use crate::types::Message;

/// 并发闸门
///
/// 对 `Semaphore` 的极薄封装：许可以 `OwnedSemaphorePermit` 形式交出，
/// 便于 `move` 进 `tokio::spawn` 的任务体内、随任务结束自动释放。
#[derive(Debug, Clone)]
pub struct ConcurrencyGate {
    sem: Arc<Semaphore>,
    limit: usize,
}

impl ConcurrencyGate {
    /// 创建闸门（`limit` 至少为 1，0 会被抬到 1 以避免永久死锁）
    pub fn new(limit: usize) -> Self {
        let limit = limit.max(1);
        Self {
            sem: Arc::new(Semaphore::new(limit)),
            limit,
        }
    }

    /// 并发上限
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// 当前可用许可数
    pub fn available(&self) -> usize {
        self.sem.available_permits()
    }

    /// 获取许可（无许可时挂起等待，形成背压）
    pub async fn acquire(&self) -> Result<OwnedSemaphorePermit> {
        self.sem
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| anyhow!("并发闸门已关闭：{e}"))
    }

    /// 尝试获取许可（不等待）
    pub fn try_acquire(&self) -> Option<OwnedSemaphorePermit> {
        self.sem.clone().try_acquire_owned().ok()
    }
}

/// 子代理上下文预算（隔离）
///
/// 每个子代理独占一份，父代理与兄弟子代理的用量互不影响。
#[derive(Debug, Clone)]
pub struct SubagentBudget {
    budget: TokenBudget,
    messages: u64,
}

impl SubagentBudget {
    /// 创建预算
    pub fn new(max_tokens: u64) -> Self {
        Self {
            budget: TokenBudget::new(max_tokens),
            messages: 0,
        }
    }

    /// 预算上限
    pub fn max_tokens(&self) -> u64 {
        self.budget.max_tokens
    }

    /// 已用 token
    pub fn used(&self) -> u64 {
        self.budget.used
    }

    /// 剩余 token
    pub fn remaining(&self) -> u64 {
        self.budget.remaining()
    }

    /// 已记账消息数
    pub fn message_count(&self) -> u64 {
        self.messages
    }

    /// 当前告警级别
    pub fn alert(&self) -> BudgetAlert {
        self.budget.alert()
    }

    /// 再追加 `extra_tokens` 是否会超预算
    pub fn would_exceed(&self, extra_tokens: u64) -> bool {
        self.budget.max_tokens > 0
            && self.budget.used.saturating_add(extra_tokens) > self.budget.max_tokens
    }

    /// 文本的 token 估算（复用全局估算器）
    pub fn estimate(text: &str) -> u64 {
        estimate_tokens(text)
    }

    /// 记录一条消息
    ///
    /// 返回本条消息的 token 估算。超预算**不报错**（用量已计入），
    /// 由 [`Self::alert`] / [`Self::needs_compaction`] 暴露，交调用方决定压缩或降级；
    /// 仅「预算配置为 0」这类配置错误才上抛。
    pub fn record_text(&mut self, text: &str) -> Result<u64> {
        let tokens = estimate_tokens(text);
        match self.budget.record(tokens) {
            Ok(()) => {}
            Err(TokenBudgetError::OverBudget) => {}
            Err(e) => return Err(anyhow!("子代理预算记账失败：{e}")),
        }
        self.messages += 1;
        Ok(tokens)
    }

    /// 是否需要压缩（接近或超过预算）
    pub fn needs_compaction(&self) -> bool {
        !matches!(self.budget.alert(), BudgetAlert::Normal)
    }

    /// 重置用量
    pub fn reset(&mut self) {
        self.budget.reset();
        self.messages = 0;
    }
}

/// 隔离 sidechain 转录
///
/// 目录布局：`<data_dir>/harness/subagents/<id>.jsonl`，每行一个 JSON 对象。
/// 追加写用 `OpenOptions::append(true).create(true)`；同一文件由一把
/// 文件级 `Mutex` 串行化，避免并发追加交错。
pub struct SidechainTranscript {
    root: PathBuf,
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl SidechainTranscript {
    /// 以 `data_dir` 为根创建转录器（实际写入其 `harness/subagents` 子目录）
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            root: data_dir.into().join("harness").join("subagents"),
            locks: Mutex::new(HashMap::new()),
        }
    }

    /// 转录根目录
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 某子代理的转录文件路径
    ///
    /// 用 [`crate::tools::fs_path::safe_resolve`] 兜底校验：即便 `subagent_id`
    /// 被污染成 `../x`，也不会写到转录根之外。
    pub fn path_for(&self, subagent_id: &str) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.root)
            .with_context(|| format!("无法创建转录目录：{}", self.root.display()))?;
        let name = format!("{subagent_id}.jsonl");
        crate::tools::fs_path::safe_resolve(&self.root, &name)
            .with_context(|| format!("非法子代理转录路径：{name}"))
    }

    /// 取（或懒建）某文件的专属锁
    fn lock_for(&self, subagent_id: &str) -> Arc<Mutex<()>> {
        let mut map = self.locks.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(subagent_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// 追加一条结构化记录
    pub fn append_value(&self, subagent_id: &str, value: &serde_json::Value) -> Result<()> {
        let path = self.path_for(subagent_id)?;
        let line = serde_json::to_string(value).context("转录记录序列化失败")?;

        let file_lock = self.lock_for(subagent_id);
        let _held = file_lock.lock().unwrap_or_else(|e| e.into_inner());

        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .with_context(|| format!("无法打开转录文件：{}", path.display()))?;
        writeln!(file, "{line}")
            .with_context(|| format!("写入转录文件失败：{}", path.display()))?;
        Ok(())
    }

    /// 追加一轮对话
    pub fn append_turn(
        &self,
        subagent_id: &str,
        role: &str,
        content: &str,
        tokens: u64,
    ) -> Result<()> {
        self.append_value(
            subagent_id,
            &json!({
                "ts": Utc::now().to_rfc3339(),
                "subagent_id": subagent_id,
                "role": role,
                "content": content,
                "tokens": tokens,
            }),
        )
    }

    /// 读取某子代理的全部转录行
    pub fn read_lines(&self, subagent_id: &str) -> Result<Vec<String>> {
        let path = self.path_for(subagent_id)?;
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("读取转录文件失败：{}", path.display()))?;
        Ok(text.lines().map(str::to_string).collect())
    }
}

impl std::fmt::Debug for SidechainTranscript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SidechainTranscript")
            .field("root", &self.root)
            .finish()
    }
}

/// 一次隔离运行的规格
#[derive(Debug, Clone)]
pub struct IsolatedSpec {
    /// 任务描述
    pub task: String,
    /// 全局上下文
    pub context: String,
    /// 角色人设
    pub role_prompt: String,
    /// 意图（用于模型路由）
    pub intent: IntentType,
    /// 复杂度（用于模型路由）
    pub complexity: Complexity,
    /// 单次运行超时
    pub timeout: Duration,
}

impl IsolatedSpec {
    /// 以任务与角色构造（默认 `General` / `Medium` / 120s）
    pub fn new(task: impl Into<String>, role_prompt: impl Into<String>) -> Self {
        Self {
            task: task.into(),
            context: String::new(),
            role_prompt: role_prompt.into(),
            intent: IntentType::General,
            complexity: Complexity::Medium,
            timeout: Duration::from_secs(120),
        }
    }

    /// 设置全局上下文
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = context.into();
        self
    }

    /// 设置路由维度
    pub fn with_routing(mut self, intent: IntentType, complexity: Complexity) -> Self {
        self.intent = intent;
        self.complexity = complexity;
        self
    }

    /// 设置超时
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// 一次隔离运行的结果
#[derive(Debug, Clone)]
pub struct IsolatedOutcome {
    /// 子代理 ID
    pub subagent_id: String,
    /// 是否成功
    pub ok: bool,
    /// 输出（失败时为错误摘要）
    pub output: String,
    /// 入参 token 估算
    pub prompt_tokens: u64,
    /// 输出 token 估算
    pub output_tokens: u64,
    /// 运行结束时的预算告警级别
    pub alert: BudgetAlert,
    /// 上下文是否被压缩过
    pub compacted: bool,
    /// 该子代理的转录文件路径
    pub transcript_path: PathBuf,
}

/// 隔离子代理运行时
///
/// 组合既有 [`SubAgentManager`]（真正执行）+ 闸门 + 预算 + 转录（隔离保障）。
/// 不复制 manager 的执行逻辑，只在其外围补隔离语义。
pub struct IsolatedSubagents {
    manager: Arc<SubAgentManager>,
    gate: ConcurrencyGate,
    transcript: SidechainTranscript,
    budget_per_agent: u64,
    budgets: Mutex<HashMap<String, SubagentBudget>>,
    events: Option<Arc<EventBus>>,
    /// 是否为每个子代理分配独立 git worktree（默认 false，向后兼容）。
    ///
    /// 开启后每次 `run` 先 [`WorktreeManager::alloc`] 一棵隔离工作树，把其路径
    /// 注入子代理上下文（作为 cwd 语义），运行结束随 [`WorktreeHandle`] 的 Drop
    /// 自动清理。多代理并行改代码时互不踩工作区。
    use_worktree: bool,
    /// worktree 管理器（`use_worktree=true` 时才创建；绑定主仓库根）
    worktrees: Option<WorktreeManager>,
}

impl IsolatedSubagents {
    /// 创建运行时
    ///
    /// 闸门上限取 `manager.max_concurrent()`，与 [`crate::agent::subagent::SubAgentConfig`]
    /// 保持同一事实源。
    pub fn new(
        manager: Arc<SubAgentManager>,
        data_dir: impl Into<PathBuf>,
        budget_per_agent: u64,
    ) -> Self {
        let limit = manager.max_concurrent();
        Self {
            manager,
            gate: ConcurrencyGate::new(limit),
            transcript: SidechainTranscript::new(data_dir),
            budget_per_agent,
            budgets: Mutex::new(HashMap::new()),
            events: None,
            use_worktree: false,
            worktrees: None,
        }
    }

    /// 开启 git worktree 隔离（为每个子代理分配独立工作树）
    ///
    /// `repo` 为主仓库根（须为 git 工作区，否则返回错误）；`session_id` 用作
    /// 分支命名空间（`marguerite/<session_id>/<name>`）。开启后 `run` 会在
    /// 启动子代理前 alloc 工作树、结束后自动清理。
    ///
    /// 默认关闭（向后兼容）：不调用本方法时行为与既有完全一致。
    pub fn with_worktree(mut self, repo: impl Into<PathBuf>, session_id: &str) -> Result<Self> {
        let mgr = WorktreeManager::new(repo.into(), session_id)
            .context("worktree 隔离初始化失败（仓库须为 git 工作区）")?;
        self.worktrees = Some(mgr);
        self.use_worktree = true;
        Ok(self)
    }

    /// 是否启用了 worktree 隔离
    pub fn use_worktree(&self) -> bool {
        self.use_worktree
    }

    /// worktree 管理器（未启用时为 None）
    pub fn worktree_manager(&self) -> Option<&WorktreeManager> {
        self.worktrees.as_ref()
    }

    /// 注册事件总线
    ///
    /// 注意：若同一总线也通过 [`SubAgentManager::set_event_bus`] 挂在了 manager 上，
    /// `subagent_start/end` 会各发一次（manager 一次、本层一次）。推荐二者只挂其一。
    pub fn with_event_bus(mut self, bus: Arc<EventBus>) -> Self {
        self.events = Some(bus);
        self
    }

    /// 并发闸门（可观测）
    pub fn gate(&self) -> &ConcurrencyGate {
        &self.gate
    }

    /// 转录器
    pub fn transcript(&self) -> &SidechainTranscript {
        &self.transcript
    }

    /// 底层管理器
    pub fn manager(&self) -> &Arc<SubAgentManager> {
        &self.manager
    }

    /// 某子代理的预算快照
    pub fn budget_of(&self, subagent_id: &str) -> Option<SubagentBudget> {
        let map = self.budgets.lock().unwrap_or_else(|e| e.into_inner());
        map.get(subagent_id).cloned()
    }

    /// 已建账的子代理数
    pub fn tracked_count(&self) -> usize {
        let map = self.budgets.lock().unwrap_or_else(|e| e.into_inner());
        map.len()
    }

    /// 发布事件（未注册总线时为空操作）
    fn emit(&self, env: Envelope) {
        if let Some(bus) = &self.events {
            bus.publish(env);
        }
    }

    /// 按预算压缩上下文
    ///
    /// 复用 [`PagingEngine`]：超长上下文会被切成摘要页（`expanded=false`），
    /// `effective()` 即取摘要，从而真实缩短。返回（压缩后文本，是否发生压缩）。
    fn compact_context(context: &str, budget_tokens: u64) -> (String, bool) {
        // token → 字符的保守折算：拉丁约 4 字符/token，留一半余量给任务与角色
        let max_chars = ((budget_tokens / 2) * 4).max(512) as usize;
        if context.chars().count() <= max_chars {
            return (context.to_string(), false);
        }
        let engine = PagingEngine::new(PagingOptions {
            max_chars,
            semantic_split: true,
        });
        let messages = vec![Message::User {
            content: context.to_string(),
            timestamp: Utc::now(),
        }];
        let mut pages = engine.paginate(&messages);
        engine.collapse_excess(&mut pages, max_chars);
        let joined = pages
            .iter()
            .map(|p| p.effective())
            .collect::<Vec<_>>()
            .join("\n");
        if joined.len() < context.len() {
            (joined, true)
        } else {
            // 分页无法缩短（无摘要可用）→ 硬截断并显式标注，绝不静默丢内容
            let head: String = context.chars().take(max_chars).collect();
            (format!("{head}\n[上下文已截断以适配子代理预算]"), true)
        }
    }

    /// 执行一次隔离运行
    ///
    /// 时序：预算核算（必要时压缩）→ 取并发许可 → **（可选）alloc worktree** →
    /// `subagent_start` → 转录入参 → 委托 manager 执行并等待终态 → 记账输出 →
    /// 转录输出 → `subagent_end` → 释放许可。
    ///
    /// 许可在**整个运行期间**持有（而非仅 spawn 瞬间），因此闸门真正限制的是
    /// 同时在跑的子代理数。
    pub async fn run(&self, spec: IsolatedSpec) -> Result<IsolatedOutcome> {
        let mut budget = SubagentBudget::new(self.budget_per_agent);

        // 1) 可选：为本子代理分配独立 git worktree（隔离工作区）
        //
        // alloc 是阻塞 git 调用，卸载到 spawn_blocking；句柄随 run 结束 Drop
        // 自动清理（失败路径同样自动收尾，见 WorktreeManager::alloc）。
        let worktree: Option<WorktreeHandle> = if self.use_worktree {
            let mgr = self
                .worktrees
                .as_ref()
                .ok_or_else(|| anyhow!("worktree 已启用但管理器缺失（内部不一致）"))?;
            let name = format!("task-{}", Uuid::new_v4().simple());
            let mgr2 = mgr.clone();
            Some(
                tokio::task::spawn_blocking(move || mgr2.alloc(&name))
                    .await
                    .map_err(|e| anyhow!("worktree 分配任务异常：{e}"))??,
            )
        } else {
            None
        };

        // 2) 预算核算：入参过大先压缩上下文（worktree 路径并入 context）
        let raw_context = if let Some(wt) = &worktree {
            format!(
                "{}\n\n[worktree] 你的独立工作树（cwd）：{}\n分支：{}",
                spec.context,
                wt.path.display(),
                wt.branch
            )
        } else {
            spec.context.clone()
        };
        let prompt_probe = format!("{}{}", spec.role_prompt, spec.task);
        let probe_tokens = SubagentBudget::estimate(&prompt_probe);
        let (context, compacted) =
            if budget.would_exceed(probe_tokens + SubagentBudget::estimate(&raw_context)) {
                Self::compact_context(&raw_context, self.budget_per_agent)
            } else {
                (raw_context, false)
            };

        let prompt = format!("{}\n{}\n{}", spec.role_prompt, spec.task, context);
        let prompt_tokens = budget.record_text(&prompt)?;
        if budget.would_exceed(0) {
            return Err(anyhow!(
                "子代理入参 {} tokens 超出隔离预算 {} tokens（压缩后仍不足）",
                prompt_tokens,
                self.budget_per_agent
            ));
        }

        // 3) 并发闸门：无许可则在此背压等待
        let _permit = self.gate.acquire().await;

        // 4) 委托既有 manager 执行（角色 + 意图 + 复杂度路由）
        let subagent_id = self
            .manager
            .spawn_spec_with_timeout(
                &spec.task,
                &context,
                &spec.role_prompt,
                spec.intent.clone(),
                spec.complexity.clone(),
                spec.timeout,
            )
            .await
            .context("子代理启动失败")?;

        self.emit(Envelope::subagent_start(&subagent_id, &spec.task));
        self.transcript
            .append_turn(&subagent_id, "prompt", &prompt, prompt_tokens)?;

        // 4) 等待终态
        let status = self.wait_for(&subagent_id, spec.timeout).await;
        let (ok, output) = match status {
            Some(SubAgentStatus::Completed { result }) => (true, result),
            Some(SubAgentStatus::Failed { error }) => (false, error),
            Some(SubAgentStatus::TimedOut) => (false, "子代理执行超时".to_string()),
            // 超过等待窗口仍未落终态：如实报告，不谎称成功
            Some(other) => (false, format!("子代理未落终态：{other:?}")),
            None => (false, "子代理状态丢失".to_string()),
        };

        // 5) 输出记账 + 转录
        let output_tokens = budget.record_text(&output)?;
        self.transcript
            .append_turn(&subagent_id, "output", &output, output_tokens)?;

        let alert = budget.alert();
        {
            let mut map = self.budgets.lock().unwrap_or_else(|e| e.into_inner());
            map.insert(subagent_id.clone(), budget);
        }

        self.emit(Envelope::subagent_end(&subagent_id, ok, &output));

        Ok(IsolatedOutcome {
            transcript_path: self.transcript.path_for(&subagent_id)?,
            subagent_id,
            ok,
            output,
            prompt_tokens,
            output_tokens,
            alert,
            compacted,
        })
        // `_permit` 在此处 drop，释放闸门许可
    }

    /// 轮询等待某子代理落终态
    ///
    /// manager 的完成广播端未对外暴露，故以轮询实现；等待窗口为
    /// `timeout` 加一段宽限，避免恰好卡在 manager 自身超时边界上。
    async fn wait_for(&self, subagent_id: &str, timeout: Duration) -> Option<SubAgentStatus> {
        let deadline = Instant::now() + timeout + Duration::from_secs(2);
        loop {
            let status = self.manager.get_status(subagent_id).await;
            match &status {
                Some(SubAgentStatus::Completed { .. })
                | Some(SubAgentStatus::Failed { .. })
                | Some(SubAgentStatus::TimedOut) => return status,
                _ => {}
            }
            if Instant::now() >= deadline {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

impl std::fmt::Debug for IsolatedSubagents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IsolatedSubagents")
            .field("gate_limit", &self.gate.limit())
            .field("budget_per_agent", &self.budget_per_agent)
            .field("transcript_root", &self.transcript.root())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::agent::subagent::SubAgentConfig;
    use crate::mcp::registry::ToolRegistry;
    use crate::model::router::ModelRouter;
    use crate::persistence::config::Config;

    fn temp_data_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("marguerite_harness_{tag}_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn manager(max_concurrent: usize, timeout: Duration) -> Arc<SubAgentManager> {
        Arc::new(SubAgentManager::new(
            SubAgentConfig {
                max_concurrent,
                default_timeout: timeout,
                model: None,
            },
            Arc::new(ToolRegistry::new()),
            Arc::new(ModelRouter::new(&Config::default())),
        ))
    }

    // ---- 并发闸门 ----

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_concurrency_gate_caps_peak_parallelism() {
        let gate = ConcurrencyGate::new(2);
        let current = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..8 {
            let gate = gate.clone();
            let current = current.clone();
            let peak = peak.clone();
            handles.push(tokio::spawn(async move {
                let _permit = gate.acquire().await.unwrap();
                let now = current.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(30)).await;
                current.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        // 无闸门时峰值会是 8；闸门生效则恒 ≤ 2
        assert_eq!(
            peak.load(Ordering::SeqCst),
            2,
            "8 个任务在 limit=2 的闸门下峰值并发必须恰为 2"
        );
        assert_eq!(gate.available(), 2, "任务结束后许可应全部归还");
    }

    #[tokio::test]
    async fn test_gate_blocks_when_exhausted() {
        let gate = ConcurrencyGate::new(1);
        let held = gate.acquire().await.unwrap();
        assert_eq!(gate.available(), 0);
        assert!(gate.try_acquire().is_none(), "许可耗尽时不应拿到新许可");
        drop(held);
        assert_eq!(gate.available(), 1);
        assert!(gate.try_acquire().is_some());
    }

    #[test]
    fn test_gate_zero_limit_raised_to_one() {
        let gate = ConcurrencyGate::new(0);
        assert_eq!(gate.limit(), 1, "0 上限会永久死锁，应抬到 1");
        assert_eq!(gate.available(), 1);
    }

    // ---- 预算隔离 ----

    #[test]
    fn test_budgets_are_isolated_between_subagents() {
        let mut a = SubagentBudget::new(1000);
        let mut b = SubagentBudget::new(1000);
        a.record_text("这是子代理 A 的一段较长上下文内容").unwrap();
        assert!(a.used() > 0);
        assert_eq!(b.used(), 0, "A 的用量不得影响 B");
        b.record_text("short").unwrap();
        assert_ne!(a.used(), b.used());
    }

    #[test]
    fn test_would_exceed_predicts_overflow() {
        let mut budget = SubagentBudget::new(100);
        assert!(!budget.would_exceed(50));
        assert!(budget.would_exceed(101));
        // 估算器约 4 拉丁字符/token，须用足量文本真正越过预算（而非假设 1 字符=1 token）
        let payload = "x".repeat(800); // ≈ 200 tokens > 预算 100
        let est = SubagentBudget::estimate(&payload);
        assert!(
            est > budget.max_tokens(),
            "payload 须真的越过预算，实际估算 {est} tokens"
        );
        budget.record_text(&payload).unwrap();
        assert!(
            budget.used() > budget.max_tokens(),
            "记录后 used 应已超预算，实际 used={} max={}",
            budget.used(),
            budget.max_tokens()
        );
        assert!(budget.would_exceed(0), "已超预算时 would_exceed(0) 应为真");
    }

    #[test]
    fn test_record_text_tracks_alert_and_count() {
        let mut budget = SubagentBudget::new(100);
        assert_eq!(budget.alert(), BudgetAlert::Normal);
        assert!(!budget.needs_compaction());

        budget.record_text(&"word ".repeat(80)).unwrap();
        assert_eq!(budget.message_count(), 1);
        assert!(
            budget.needs_compaction(),
            "接近/超出预算应提示压缩，实际 used={} max={}",
            budget.used(),
            budget.max_tokens()
        );

        budget.reset();
        assert_eq!(budget.used(), 0);
        assert_eq!(budget.message_count(), 0);
        assert_eq!(budget.alert(), BudgetAlert::Normal);
    }

    #[test]
    fn test_remaining_never_underflows() {
        let mut budget = SubagentBudget::new(10);
        budget.record_text(&"a".repeat(500)).unwrap();
        assert_eq!(budget.remaining(), 0, "超预算后剩余量应饱和到 0");
    }

    // ---- sidechain 转录 ----

    #[test]
    fn test_transcript_writes_one_file_per_subagent() {
        let dir = temp_data_dir("transcript");
        let t = SidechainTranscript::new(&dir);

        t.append_turn("agent-a", "prompt", "任务 A", 10).unwrap();
        t.append_turn("agent-a", "output", "结果 A", 20).unwrap();
        t.append_turn("agent-b", "prompt", "任务 B", 30).unwrap();

        assert_eq!(t.read_lines("agent-a").unwrap().len(), 2);
        assert_eq!(t.read_lines("agent-b").unwrap().len(), 1);
        assert!(t.read_lines("agent-missing").unwrap().is_empty());

        // 落盘位置符合约定：<data_dir>/harness/subagents/<id>.jsonl
        let path = t.path_for("agent-a").unwrap();
        assert!(path.ends_with("agent-a.jsonl"));
        assert!(path.starts_with(std::fs::canonicalize(&dir).unwrap_or(dir.clone())));

        // 每行都是合法 JSON 且字段完整
        for line in t.read_lines("agent-a").unwrap() {
            let v: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(v["subagent_id"], "agent-a");
            assert!(v["ts"].is_string());
            assert!(v["tokens"].is_number());
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_transcript_rejects_path_traversal_id() {
        let dir = temp_data_dir("traversal");
        let t = SidechainTranscript::new(&dir);
        assert!(
            t.path_for("../../escape").is_err(),
            "被污染的子代理 ID 不得写到转录根之外"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_transcript_concurrent_appends_do_not_interleave() {
        let dir = temp_data_dir("concurrent");
        let t = Arc::new(SidechainTranscript::new(&dir));

        let mut handles = Vec::new();
        for w in 0..8 {
            let t = t.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..10 {
                    t.append_turn("shared", "prompt", &format!("w{w}-i{i}"), 1)
                        .unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let lines = t.read_lines("shared").unwrap();
        assert_eq!(lines.len(), 80, "8 线程 ×10 行应无丢失");
        for line in &lines {
            serde_json::from_str::<serde_json::Value>(line)
                .expect("并发追加不得产生半行/交错的坏 JSON");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- 上下文压缩 ----

    #[test]
    fn test_compact_context_shrinks_oversized_input() {
        let long = "这是一段需要被压缩的超长上下文。".repeat(2000);
        let (compacted, changed) = IsolatedSubagents::compact_context(&long, 1000);
        assert!(changed, "超长上下文应被压缩");
        assert!(
            compacted.len() < long.len(),
            "压缩后长度应下降：{} → {}",
            long.len(),
            compacted.len()
        );
    }

    #[test]
    fn test_compact_context_leaves_small_input_untouched() {
        let short = "短上下文";
        let (out, changed) = IsolatedSubagents::compact_context(short, 100_000);
        assert!(!changed);
        assert_eq!(out, short);
    }

    // ---- 端到端隔离运行（离线：无 API key 时子代理必然失败/超时，但隔离设施须完整生效）----

    #[tokio::test]
    async fn test_isolated_run_records_transcript_and_budget() {
        let dir = temp_data_dir("run");
        let mgr = manager(2, Duration::from_millis(200));
        let runtime = IsolatedSubagents::new(mgr, &dir, 100_000);
        assert_eq!(runtime.gate().limit(), 2, "闸门上限应取自 SubAgentConfig");

        let spec = IsolatedSpec::new("统计仓库文件数", "你是代码分析专家")
            .with_routing(IntentType::Code, Complexity::Medium)
            .with_timeout(Duration::from_millis(200));
        let outcome = runtime.run(spec).await.expect("隔离运行本身不应报错");

        assert!(!outcome.subagent_id.is_empty());
        assert!(outcome.prompt_tokens > 0, "入参应被记账");

        // 转录：入参 + 输出各一行
        let lines = runtime
            .transcript()
            .read_lines(&outcome.subagent_id)
            .unwrap();
        assert_eq!(lines.len(), 2, "应记录 prompt 与 output 两轮");
        assert!(outcome.transcript_path.exists());

        // 预算：该子代理独立建账
        let budget = runtime
            .budget_of(&outcome.subagent_id)
            .expect("应留下预算记录");
        assert_eq!(budget.message_count(), 2);
        assert_eq!(runtime.tracked_count(), 1);

        // 许可归还：运行结束后闸门恢复满额
        assert_eq!(runtime.gate().available(), 2, "运行结束应归还许可");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_isolated_run_rejects_prompt_over_budget() {
        let dir = temp_data_dir("overbudget");
        let mgr = manager(1, Duration::from_millis(100));
        // 预算极小：入参必然超限，应在启动子代理前就被拦下
        let runtime = IsolatedSubagents::new(mgr, &dir, 8);

        let spec = IsolatedSpec::new("x".repeat(4000), "你是专家")
            .with_timeout(Duration::from_millis(100));
        let err = runtime
            .run(spec)
            .await
            .expect_err("超预算应拒绝执行")
            .to_string();
        assert!(err.contains("预算"), "错误应指明预算原因：{err}");
        // 未启动任何子代理，许可未泄漏
        assert_eq!(runtime.gate().available(), 1);
        assert_eq!(runtime.tracked_count(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
