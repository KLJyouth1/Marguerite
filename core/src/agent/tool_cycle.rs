//! 工具循环
//!
//! 管理工具调用的生命周期，支持顺序执行和并行执行，
//! 提供超时控制、并发限制和结果收集能力。

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use tokio::sync::Semaphore;

use crate::agent::task_checkpoint::{CheckpointStore, TaskCheckpoint};
use crate::agent::tool_cache::ToolCache;
use crate::agent::tool_compress::ToolCompressor;
use crate::mcp::registry::ToolRegistry;

/// 工具执行结果
#[derive(Debug, Clone)]
pub enum ToolResult {
    /// 执行成功
    Success {
        /// 返回数据
        data: String,
        /// 元数据
        metadata: HashMap<String, String>,
    },
    /// 执行错误
    Error {
        /// 错误消息
        message: String,
        /// 错误码
        code: String,
    },
    /// 超时
    Timeout {
        /// 工具名称
        tool_name: String,
        /// 超时时间
        duration: Duration,
    },
}

impl ToolResult {
    /// 判断结果是否为成功
    pub fn is_success(&self) -> bool {
        matches!(self, ToolResult::Success { .. })
    }

    /// 判断结果是否为错误
    pub fn is_error(&self) -> bool {
        matches!(self, ToolResult::Error { .. })
    }

    /// 判断结果是否为超时
    pub fn is_timeout(&self) -> bool {
        matches!(self, ToolResult::Timeout { .. })
    }

    /// 获取成功数据（如果结果是成功）
    pub fn data(&self) -> Option<&str> {
        match self {
            ToolResult::Success { data, .. } => Some(data),
            _ => None,
        }
    }
}

/// 工具调用
#[derive(Debug, Clone)]
pub struct ToolCall {
    /// 工具名称
    pub name: String,
    /// 参数
    pub arguments: HashMap<String, String>,
    /// 超时时间
    pub timeout: Duration,
}

impl ToolCall {
    /// 创建新的工具调用
    pub fn new(name: &str, arguments: HashMap<String, String>) -> Self {
        // 模型显式声明的 timeout_sec 必须真的生效，否则「模型说给 300s，外层 30s
        // 就掐死」——用户看到的就是工具超时。长任务命令再按命令特征自动放宽。
        let timeout = declared_or_inferred_timeout(name, &arguments)
            .unwrap_or_else(|| default_timeout_for(name));
        Self {
            name: name.to_string(),
            arguments,
            timeout,
        }
    }

    /// 设置超时
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// 从调用参数里解析模型声明的超时，或按命令特征推断长任务预算。
///
/// - 模型写了 `timeout_sec`：以它为准，外加 30s 收尾余量（进程退出/日志读取），
///   硬上限 1800s；
/// - `shell_exec` 的常见长命令（构建、装依赖、跑测试、容器构建）：900s；
/// - 其它情况返回 `None`，走 [`default_timeout_for`] 的保守默认。
fn declared_or_inferred_timeout(
    name: &str,
    arguments: &HashMap<String, String>,
) -> Option<Duration> {
    const MARGIN_SECS: u64 = 30;
    const MAX_SECS: u64 = 1_800;
    if let Some(secs) = arguments
        .get("timeout_sec")
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
    {
        return Some(Duration::from_secs((secs + MARGIN_SECS).min(MAX_SECS)));
    }
    if name == "shell_exec" {
        let command = arguments.get("command")?.to_ascii_lowercase();
        const LONG_COMMANDS: &[&str] = &[
            "cargo build",
            "cargo test",
            "cargo clippy",
            "npm install",
            "npm ci",
            "npm run build",
            "npm run test",
            "pnpm install",
            "yarn install",
            "pip install",
            "pytest",
            "vitest",
            "cmake",
            "docker build",
            "dotnet build",
            "gradle",
            "mvn ",
            "go build",
            "go test",
            "playwright",
            "cypress",
            "make ",
        ];
        if LONG_COMMANDS.iter().any(|hint| command.contains(hint)) {
            return Some(Duration::from_secs(900));
        }
    }
    None
}

/// 按工具名给出默认超时（R14）。
///
/// 历史缺陷：所有工具统一 30s。对**长任务工具**（媒体转码、3D 渲染、Office 批处理）
/// 这不是「保护」而是「必杀」—— 一段 2 分钟的视频转码在 30s 被强杀，用户看到的是
/// 「工具超时」，而真实原因是超时预算给错了。这类工具的正确做法是给足预算，并让
/// 工具自己按操作类型再收紧（如 `media probe` 内部只用 30s）。
///
/// 只放宽**确定性长任务**；`shell_exec` 等任意命令执行仍保持默认值（它的风险不在
/// 耗时而在副作用，放宽超时只会扩大影响窗口）。
pub fn default_timeout_for(name: &str) -> Duration {
    match name {
        // 媒体转码/抽帧：540s 内部预算 + 外层余量
        "media" => Duration::from_secs(600),
        // 3D 渲染（Blender 桥接入后使用）
        "blender" => Duration::from_secs(900),
        // Office COM 批处理（大批量表格/幻灯片常见数十秒）
        "office_automation" => Duration::from_secs(180),
        // 云端生图：主流服务商 1024×1024 单张 10–60s，高清/多张会到分钟级。
        "image_generate" => Duration::from_secs(300),
        // 云端生视频：一段 5–10s 的视频常见 1–5 分钟，且上游会排队。
        "video_generate" => Duration::from_secs(600),
        // 本地生视频：逐帧渲染 + x264 编码，长代码/长时长会到分钟级。
        "code_video" | "web_video" => Duration::from_secs(600),
        // Manim 首次渲染要编译场景 + 排版，给足时间。
        "manim_video" => Duration::from_secs(900),
        // 云端 ASR/TTS：长音频转写常见分钟级。
        "audio_transcribe" | "audio_speak" => Duration::from_secs(360),
        // 操作回放：按录制时间轴逐事件重放，60s 的宏 + 循环会超过默认 30s。
        "auto_replay" => Duration::from_secs(600),
        // 录制只做启动/停止（真正的录制在后台进程里），无需放宽。
        _ => Duration::from_secs(30),
    }
}

// ---------------------------------------------------------------------------
// P3-1 长任务熔断：命中即「暂停 + 给出可回滚点」，不直接 abort 整个任务
// ---------------------------------------------------------------------------
//
// 三条判定（确定性、纯内存，不依赖模型/网络）：
// 1. 连续 5 次工具错误（`Error` 与 `Timeout` 都算失败）；
// 2. 60 秒窗口内错误率 > 15%（且窗口样本数 ≥ [`CIRCUIT_MIN_WINDOW_SAMPLES`]，
//    单点噪声不判熔断——连续错另有第 1 条的 5 连门槛）；
// 3. 同参数连续 3 步（参数指纹相同）判死循环。
//
// 命中后熔断器**只暂停**：返回 [`CircuitVerdict`]，其中 `checkpoint_id` 指向该任务的
// checkpoint（回滚点）；配合 [`CircuitBreaker::pause`] 把 checkpoint 状态置为
// `paused_for_rollback`，由操作员决定回滚 / 重放。已完成步骤不丢弃，任务不整单作废。

/// 连续工具错误触发阈值。
pub const CIRCUIT_MAX_CONSECUTIVE_ERRORS: usize = 5;
/// 错误率统计窗口（毫秒）。
pub const CIRCUIT_ERROR_WINDOW_MS: u64 = 60_000;
/// 窗口内错误率阈值（严格大于才触发）。
pub const CIRCUIT_ERROR_RATE: f64 = 0.15;
/// 窗口统计的最小样本数：低于此数不按错误率判定（太少样本的比率没有统计意义）。
pub const CIRCUIT_MIN_WINDOW_SAMPLES: usize = 8;
/// 同参数连续步数阈值。
pub const CIRCUIT_MAX_REPEAT_STEPS: usize = 3;

/// 熔断原因（三选一，可诊断、可审计）。
#[derive(Debug, Clone, PartialEq)]
pub enum CircuitReason {
    /// 连续工具错误
    ConsecutiveErrors {
        /// 连续错误数（≥ [`CIRCUIT_MAX_CONSECUTIVE_ERRORS`]）
        count: usize,
    },
    /// 窗口内错误率超阈值
    ErrorRate {
        /// 窗口内错误次数
        errors: usize,
        /// 窗口内调用次数
        calls: usize,
        /// 实际错误率
        rate: f64,
    },
    /// 同参数连续重复（疑似死循环）
    RepeatedParameters {
        /// 参数指纹
        fingerprint: String,
        /// 连续步数（≥ [`CIRCUIT_MAX_REPEAT_STEPS`]）
        steps: usize,
    },
}

impl std::fmt::Display for CircuitReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CircuitReason::ConsecutiveErrors { count } => {
                write!(f, "连续 {count} 次工具错误（阈值 {CIRCUIT_MAX_CONSECUTIVE_ERRORS}）")
            }
            CircuitReason::ErrorRate { errors, calls, rate } => write!(
                f,
                "60 秒窗口错误率 {:.2}%（{errors}/{calls}），阈值 {:.0}%",
                rate * 100.0,
                CIRCUIT_ERROR_RATE * 100.0
            ),
            CircuitReason::RepeatedParameters { fingerprint, steps } => write!(
                f,
                "同参数连续 {steps} 步（指纹 {fingerprint}），阈值 {CIRCUIT_MAX_REPEAT_STEPS}"
            ),
        }
    }
}

/// 熔断判定结果。
///
/// `tripped == true` 表示「暂停」：`checkpoint_id` 是可回滚点（任务 checkpoint 标识），
/// 调用方应中断当前回合、把控制权交回操作员；**不得**直接丢弃任务。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CircuitVerdict {
    /// 是否熔断
    pub tripped: bool,
    /// 熔断原因（未熔断时为 `None`）
    pub reason: Option<CircuitReason>,
    /// 可回滚点（任务 checkpoint 标识；未熔断时为 `None`）
    pub checkpoint_id: Option<String>,
}

impl CircuitVerdict {
    /// 未熔断。
    pub fn ok() -> Self {
        Self::default()
    }
}

/// 长任务熔断器（每条任务一个；观察工具回路的逐步结果）。
///
/// 用法（工具回路每步调用一次）：
/// ```ignore
/// let verdict = breaker.observe(&call, &result, task_checkpoint::now_ms());
/// if verdict.tripped {
///     breaker.pause(&store);          // 落盘可回滚点
///     // 暂停本任务、把 verdict 交给 UI / 操作员，而不是 abort
/// }
/// ```
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    task_id: String,
    consecutive_errors: usize,
    /// 窗口样本：`(发生时刻 ms, 是否成功)`
    window: VecDeque<(u64, bool)>,
    last_fingerprint: Option<String>,
    repeated_steps: usize,
    tripped: Option<CircuitReason>,
}

impl CircuitBreaker {
    /// 为某任务创建熔断器（`task_id` 同时是 checkpoint 标识）。
    pub fn new(task_id: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            consecutive_errors: 0,
            window: VecDeque::new(),
            last_fingerprint: None,
            repeated_steps: 0,
            tripped: None,
        }
    }

    /// 所属任务（= checkpoint 标识 / 可回滚点）。
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    /// 是否已熔断（暂停中）。
    pub fn is_tripped(&self) -> bool {
        self.tripped.is_some()
    }

    /// 记录一步工具调用结果并返回当前判定。
    ///
    /// `at_ms` 由调用方显式传入（一般取 `task_checkpoint::now_ms()`），便于离线单测
    /// 复现任意时间线，不读系统时钟。
    pub fn observe(
        &mut self,
        call: &ToolCall,
        result: &ToolResult,
        at_ms: u64,
    ) -> CircuitVerdict {
        let failed = result.is_error() || result.is_timeout();
        self.consecutive_errors = if failed { self.consecutive_errors + 1 } else { 0 };

        let fingerprint = tool_fingerprint(&call.name, &call.arguments);
        if self.last_fingerprint.as_deref() == Some(fingerprint.as_str()) {
            self.repeated_steps += 1;
        } else {
            self.last_fingerprint = Some(fingerprint);
            self.repeated_steps = 1;
        }

        self.window.push_back((at_ms, !failed));
        // 只保留最近 60 秒的样本：更早的错误不得计入当前错误率（否则一次历史抖动会永久拉高）。
        let cutoff = at_ms.saturating_sub(CIRCUIT_ERROR_WINDOW_MS);
        while let Some(&(ts, _)) = self.window.front() {
            if ts < cutoff {
                self.window.pop_front();
            } else {
                break;
            }
        }

        if self.tripped.is_none() {
            if self.consecutive_errors >= CIRCUIT_MAX_CONSECUTIVE_ERRORS {
                self.tripped = Some(CircuitReason::ConsecutiveErrors {
                    count: self.consecutive_errors,
                });
            } else {
                let calls = self.window.len();
                let errors = self.window.iter().filter(|(_, ok)| !ok).count();
                let rate = if calls == 0 {
                    0.0
                } else {
                    errors as f64 / calls as f64
                };
                if calls >= CIRCUIT_MIN_WINDOW_SAMPLES && rate > CIRCUIT_ERROR_RATE {
                    self.tripped = Some(CircuitReason::ErrorRate {
                        errors,
                        calls,
                        rate,
                    });
                } else if self.repeated_steps >= CIRCUIT_MAX_REPEAT_STEPS {
                    self.tripped = Some(CircuitReason::RepeatedParameters {
                        fingerprint: self.last_fingerprint.clone().unwrap_or_default(),
                        steps: self.repeated_steps,
                    });
                }
            }
        }

        self.verdict()
    }

    /// 当前判定（不推进状态）。
    pub fn verdict(&self) -> CircuitVerdict {
        match &self.tripped {
            Some(reason) => CircuitVerdict {
                tripped: true,
                reason: Some(reason.clone()),
                checkpoint_id: Some(self.task_id.clone()),
            },
            None => CircuitVerdict::ok(),
        }
    }

    /// 操作员确认回滚 / 重放后恢复：清空计数、窗口与熔断态，重新度量。
    pub fn resume(&mut self) {
        self.consecutive_errors = 0;
        self.window.clear();
        self.last_fingerprint = None;
        self.repeated_steps = 0;
        self.tripped = None;
    }

    /// 把回滚点落盘：checkpoint 状态置为 `paused_for_rollback`。
    ///
    /// 任务尚未建立 checkpoint 时返回 `None`（不凭空造一份，避免伪造回滚点）。
    pub fn pause(&self, store: &CheckpointStore) -> Option<TaskCheckpoint> {
        store.pause_for_rollback(&self.task_id)
    }
}

/// 工具参数指纹（确定性 FNV-1a 64，与进程/哈希种子无关）。
///
/// 键排序后拼接，故 `HashMap` 插入顺序不影响结果；同「工具名 + 参数集合」恒等。
/// 只用于判「同参数连续重复」，不承担安全职责（不替代审批/沙箱）。
pub fn tool_fingerprint(name: &str, arguments: &HashMap<String, String>) -> String {
    let mut pairs: Vec<(&str, &str)> = arguments
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    pairs.sort_unstable();
    let mut canonical = String::with_capacity(name.len() + 1);
    canonical.push_str(name);
    for (k, v) in pairs {
        canonical.push('\u{1e}');
        canonical.push_str(k);
        canonical.push('=');
        canonical.push_str(v);
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in canonical.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod timeout_policy_tests {
    use super::*;

    #[test]
    fn long_running_tools_get_a_budget_they_can_finish_in() {
        // 回归：统一 30s 会把真实转码/渲染/批处理强杀，用户看到「超时」而非真实原因。
        assert!(default_timeout_for("media") >= Duration::from_secs(300));
        assert!(default_timeout_for("blender") >= Duration::from_secs(600));
        assert!(default_timeout_for("office_automation") > Duration::from_secs(30));
    }

    #[test]
    fn ordinary_and_dangerous_tools_keep_the_short_default() {
        // shell_exec 的风险在副作用而非耗时，放宽超时只会扩大影响窗口。
        for name in ["shell_exec", "read_file", "grep", "web_fetch", "browser"] {
            assert_eq!(
                default_timeout_for(name),
                Duration::from_secs(30),
                "{name} 不应被放宽超时"
            );
        }
    }

    #[test]
    fn tool_call_new_uses_the_policy() {
        let call = ToolCall::new("media", HashMap::new());
        assert_eq!(call.timeout, default_timeout_for("media"));
        // 显式覆盖仍然优先（调用方说了算）
        let call = ToolCall::new("media", HashMap::new()).with_timeout(Duration::from_secs(5));
        assert_eq!(call.timeout, Duration::from_secs(5));
    }

    #[test]
    fn model_declared_timeout_sec_is_actually_honored() {
        // 回归：模型写 timeout_sec=600，外层却按 30s 掐死。
        let mut args = HashMap::new();
        args.insert("command".to_string(), "python slow.py".to_string());
        args.insert("timeout_sec".to_string(), "600".to_string());
        let call = ToolCall::new("shell_exec", args);
        assert!(call.timeout >= Duration::from_secs(600), "{:?}", call.timeout);
    }

    #[test]
    fn long_build_commands_get_a_longer_budget() {
        let mut args = HashMap::new();
        args.insert("command".to_string(), "cargo test --all".to_string());
        let call = ToolCall::new("shell_exec", args);
        assert!(call.timeout >= Duration::from_secs(600), "{:?}", call.timeout);

        // 普通命令仍走保守默认，不因为一次长任务把所有 shell 放宽。
        let mut quick = HashMap::new();
        quick.insert("command".to_string(), "ls -la".to_string());
        assert_eq!(
            ToolCall::new("shell_exec", quick).timeout,
            Duration::from_secs(30)
        );
    }
}

/// 工具循环默认工具缓存容量（FIFO 上限：超出仅淘汰最旧条目、不影响命中正确性）。
const DEFAULT_TOOL_CACHE_CAP: usize = 64;

/// 工具循环
///
/// 管理工具调用的执行生命周期，支持顺序和并行执行策略。
pub struct ToolCycle {
    /// 最大工具循环次数
    max_iterations: u32,
    /// 最大并行工具数
    max_concurrent: usize,
    /// 默认超时时间
    default_timeout: Duration,
    /// 工具注册表
    registry: Arc<ToolRegistry>,
    /// 所属会话 key（P2-3）：会话级提权按它查台账；`None` = 无会话上下文
    /// （CLI / 子智能体），此时提权一律不生效（fail-closed）。
    session_key: Option<String>,
    /// 幂等工具结果缓存：默认构造即注入（W-3，接 P-12 `ToolCache`）。注入后每次
    /// 工具调用同 (tool,args) 命中即返回缓存、不再重跑底层工具；非幂等（写/发/删/建）
    /// 与失败结果不入缓存（诚实，不掩盖瞬时失败）。跨轮/跨会话共享需显式 `with_tool_cache`
    /// 注入同一 `Arc<ToolCache>`。
    tool_cache: Option<Arc<ToolCache>>,
    /// 工具输出压缩器（W-4，接 P-12 `ToolCompressor`）：默认构造即注入。大体积工具输出
    /// （长日志/大 JSON/diff）经确定性压缩后进入上下文，缩小上下文、放大前缀缓存命中率；
    /// 原文按内容指纹存入内存 LRU，模型可经 `retrieve_full` 取回（CCR 可逆）。
    tool_compressor: Option<Arc<ToolCompressor>>,
}

impl ToolCycle {
    /// 创建新的工具循环（默认注入幂等工具结果缓存，W-3）。
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        // 单批工具调用上限不再写死 10：按自适应预算的 Standard 档取值
        // （24），既容得下「模型一次读完一组文件」的合法批量，又仍是有限值。
        // 调用方仍可用 [`ToolCycle::set_max_iterations`] 按实际任务收窄/放宽。
        let batch_cap = crate::agent::budget_policy::resolve("[[budget:standard]]", 1, 0)
            .tool_rounds
            .max(10);
        Self {
            max_iterations: batch_cap as u32,
            max_concurrent: 5,
            default_timeout: Duration::from_secs(30),
            registry,
            session_key: None,
            tool_cache: Some(Arc::new(ToolCache::with_default_policy(
                DEFAULT_TOOL_CACHE_CAP,
                None,
            ))),
            tool_compressor: Some(Arc::new(ToolCompressor::default())),
        }
    }

    /// 绑定会话 key（P2-3）：审批旁路按会话生效，必须先知道自己在哪个会话里。
    pub fn with_session(mut self, session: Option<String>) -> Self {
        self.session_key = session;
        self
    }

    /// 注入共享幂等工具结果缓存（覆盖默认实例，用于跨轮/跨会话复用同一缓存）。
    pub fn with_tool_cache(mut self, cache: Arc<ToolCache>) -> Self {
        self.tool_cache = Some(cache);
        self
    }

    /// 注入共享工具输出压缩器（覆盖默认实例，用于跨轮/跨会话复用同一压缩器与 CCR 原文库）。
    pub fn with_tool_compressor(mut self, compressor: Arc<ToolCompressor>) -> Self {
        self.tool_compressor = Some(compressor);
        self
    }

    /// 执行工具调用（默认并行执行）
    pub async fn execute(&self, tool_calls: Vec<ToolCall>) -> Result<Vec<ToolResult>> {
        self.execute_parallel(tool_calls).await
    }

    /// 顺序执行工具调用
    ///
    /// 按顺序依次执行每个工具调用，前一个完成后才执行下一个。
    pub async fn execute_sequential(&self, tool_calls: Vec<ToolCall>) -> Result<Vec<ToolResult>> {
        if tool_calls.len() > self.max_iterations as usize {
            return Err(anyhow!(
                "Too many tool calls: {} (max: {})",
                tool_calls.len(),
                self.max_iterations
            ));
        }

        let mut results = Vec::with_capacity(tool_calls.len());

        for call in tool_calls {
            let result = dispatch_cached(
                self.tool_cache.clone(),
                self.tool_compressor.clone(),
                self.registry.clone(),
                self.session_key.as_deref(),
                &call.name,
                &call.arguments,
                call.timeout,
            )
            .await;
            results.push(result);
        }

        Ok(results)
    }

    /// 并行执行工具调用
    ///
    /// 使用 tokio::spawn 并发执行所有工具调用，通过 Semaphore 控制并发数。
    pub async fn execute_parallel(&self, tool_calls: Vec<ToolCall>) -> Result<Vec<ToolResult>> {
        if tool_calls.len() > self.max_iterations as usize {
            return Err(anyhow!(
                "Too many tool calls: {} (max: {})",
                tool_calls.len(),
                self.max_iterations
            ));
        }

        let semaphore = Arc::new(Semaphore::new(self.max_concurrent));
        let mut handles = Vec::with_capacity(tool_calls.len());

        for call in tool_calls {
            let sem = semaphore.clone();
            let name = call.name.clone();
            let args = call.arguments.clone();
            let timeout = call.timeout;
            let registry = self.registry.clone();
            let cache = self.tool_cache.clone();
            let session_key = self.session_key.clone();

            let comp = self.tool_compressor.clone();
            let handle = tokio::spawn(async move {
                let _permit = sem.acquire().await;
                dispatch_cached(
                    cache,
                    comp,
                    registry,
                    session_key.as_deref(),
                    &name,
                    &args,
                    timeout,
                )
                .await
            });

            handles.push(handle);
        }

        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(e) => results.push(ToolResult::Error {
                    message: format!("Task join error: {}", e),
                    code: "JOIN_ERROR".to_string(),
                }),
            }
        }

        Ok(results)
    }

    /// 单工具调用（带超时）
    async fn call_tool_with_timeout(
        &self,
        name: &str,
        args: &HashMap<String, String>,
        timeout: Duration,
    ) -> ToolResult {
        dispatch_cached(
            self.tool_cache.clone(),
            self.tool_compressor.clone(),
            self.registry.clone(),
            self.session_key.as_deref(),
            name,
            args,
            timeout,
        )
        .await
    }

    /// 单工具调用（使用默认超时）
    ///
    /// 调用单个工具，带超时控制。
    pub async fn call_tool(&self, name: &str, args: &HashMap<String, String>) -> ToolResult {
        self.call_tool_with_timeout(name, args, self.default_timeout)
            .await
    }

    /// 设置最大循环次数
    pub fn set_max_iterations(&mut self, n: u32) {
        self.max_iterations = n;
    }

    /// 设置最大并行数
    pub fn set_max_concurrent(&mut self, n: usize) {
        self.max_concurrent = n;
    }

    /// 获取最大循环次数
    pub fn max_iterations(&self) -> u32 {
        self.max_iterations
    }

    /// 获取最大并行数
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// 获取默认超时时间
    pub fn default_timeout(&self) -> Duration {
        self.default_timeout
    }
}

/// 内部工具执行函数
///
/// 检查工具是否在注册表中，然后执行。
async fn execute_tool_internal(
    registry: &ToolRegistry,
    name: &str,
    args: &HashMap<String, String>,
) -> Result<ToolResult> {
    // 模式门控（MSC-2）：Plan（只读探索）模式在**工具执行链路上**硬拒绝写类工具，
    // 使 `mode.switch` 切到 Plan 后真实拒绝修改类动作，而非仅注入提示段。
    // 非 Plan 模式不受影响（读一次全局运行时模式，零行为回归）。
    if crate::mode::runtime_denies_tool(name) {
        return Ok(ToolResult::Error {
            message: format!("Plan 模式（只读探索）禁止执行写类工具：{name}"),
            code: "MODE_DENIED".to_string(),
        });
    }
    // 先检查本地工具
    if registry.has_tool(name) {
        // 本地工具经 get_tool 返回 None，走 execute_local_tool；
        // MCP 工具经 get_tool 命中后走其 execute（与 chain::executor 一致）。
        if let Some(tool) = registry.get_tool(name) {
            return tool.execute(args).await;
        }
        return registry.execute_local_tool(name, args).await;
    }

    Err(anyhow!("Tool '{}' not found in registry", name))
}

/// 经幂等工具缓存分派单次工具调用（W-3，接 P-12 `ToolCache`）。
///
/// - 注入缓存时：相同 (tool,args) 命中即返回缓存结果、不再重跑底层工具；未命中执行
///   `execute_tool_internal` 并按「幂等 + 成功」门控回填缓存（非幂等/失败不缓存，诚实）。
/// - 未注入缓存时：退化为直接超时包装执行，语义与旧 `call_tool` 一致。
/// 若注入了压缩器则压缩工具结果，否则原样返回（确定性、无副作用）。
#[allow(clippy::doc_lazy_continuation)] // 单行续写文档，非列表项
fn maybe_compress(
    compressor: &Option<Arc<ToolCompressor>>,
    name: &str,
    r: ToolResult,
) -> ToolResult {
    match compressor {
        Some(c) => c.compress(name, &r),
        None => r,
    }
}

async fn dispatch_cached(
    cache: Option<Arc<ToolCache>>,
    compressor: Option<Arc<ToolCompressor>>,
    registry: Arc<ToolRegistry>,
    session_key: Option<&str>,
    name: &str,
    args: &HashMap<String, String>,
    timeout: Duration,
) -> ToolResult {
    // 0) 高危闸门（P0-4）：**必须在超时包裹之外**。
    //
    // `execute_tool_internal` 被 `tokio::time::timeout(t, …)` 包着，而 t 是工具自身
    // 的超时（默认 30s），短于审批门的 60s 超时 —— 审批若放在里面，用户在第 45 秒
    // 点「允许」时工具早已被判 Timeout，批准永远到不了执行。故在超时之外先挂起审批。
    //
    // Plan 模式下写类工具会被 `execute_tool_internal` 硬拒，这里先跳过闸门，
    // 避免「先弹确认卡、批准后又被模式拒绝」这种浪费用户操作的顺序。
    if !crate::mode::runtime_denies_tool(name) {
        let gate = crate::guard::gateway::agent_gate();
        let host_present = crate::guard::gateway::agent_approval_host_present();
        if let Some(blocked) = suspend_for_high_risk(gate, host_present, session_key, name, args).await {
            return blocked;
        }
    }
    if let Some(cache) = &cache {
        // 缓存键用独立副本，执行闭包用另一副本，避免与 execute_cached 的 & 借用冲突。
        let key_tool = name.to_string();
        let key_args = args.clone();
        let exec_tool = key_tool.clone();
        let exec_args = key_args.clone();
        let reg = registry.clone();
        let t = timeout;
        let comp = compressor.clone();
        return cache
            .execute_cached(&key_tool, &key_args, move || {
                let reg = reg.clone();
                let exec_args = exec_args.clone();
                let exec_tool = exec_tool.clone();
                let comp = comp.clone();
                async move {
                    let raw = match tokio::time::timeout(
                        t,
                        execute_tool_internal(&reg, &exec_tool, &exec_args),
                    )
                    .await
                    {
                        Ok(Ok(result)) => result,
                        Ok(Err(e)) => ToolResult::Error {
                            message: e.to_string(),
                            code: "EXECUTION_ERROR".to_string(),
                        },
                        Err(_) => ToolResult::Timeout {
                            tool_name: exec_tool.clone(),
                            duration: t,
                        },
                    };
                    maybe_compress(&comp, &exec_tool, raw)
                }
            })
            .await;
    }
    // 无缓存：直接超时包装执行 + 压缩
    let raw =
        match tokio::time::timeout(timeout, execute_tool_internal(&registry, name, args)).await {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => ToolResult::Error {
                message: e.to_string(),
                code: "EXECUTION_ERROR".to_string(),
            },
            Err(_) => ToolResult::Timeout {
                tool_name: name.to_string(),
                duration: timeout,
            },
        };
    maybe_compress(&compressor, name, raw)
}

/// `tool_intent` 表按**工具名尾段 + 参数**判定，覆盖不到「动词在参数里」的系统级
/// 不可逆动作。`process_manage` 是唯一一例：尾段是 `manage`，`kill`/`stop` 由参数携带，
/// 于是它落在 `IntentKind::Unknown`（Medium、不拦）—— 而终止进程与桌面控制同属高影响
/// 动作（审计 P0-4 明确把「进程」列入高危面）。这里显式补齐，不靠概率。
const SYSTEM_IRREVERSIBLE_TOOLS: &[&str] = &["process_manage"];

/// 该 (工具, 参数) 是否**需要人工确认**；需要时返回结构化意图（纯函数，可单测）。
///
/// 判定面刻意保持窄：只拦「破坏性 / 不可逆 / 桌面驱动 / 系统级不可逆动作」。
/// `read_file`、普通 `write_file`（非覆盖）一律不拦 —— 否则每次写文件都要用户点确认，
/// 产品不可用。规则表复用 [`crate::guard::tool_intent::analyze`]（与确认卡渲染同源，
/// 不另造第二套判定）。
fn approval_required(
    name: &str,
    args: &HashMap<String, String>,
) -> Option<crate::guard::tool_intent::ToolIntention> {
    let intent = crate::guard::tool_intent::analyze(name, &args_json_of(args));
    let desktop = crate::mode::is_desktop_class(name);
    let system_irreversible = SYSTEM_IRREVERSIBLE_TOOLS.contains(&name);
    if intent.high_risk || intent.destructive || intent.irreversible || desktop
        || system_irreversible
    {
        Some(intent)
    } else {
        None
    }
}

/// 高危动作的审批闸门（P0-4）：命中即挂起审批，未批准不放行（fail-closed）。
///
/// # 为什么需要
///
/// 链式执行器（`chain::executor`）分发步骤前会走 `ModeManager::gate` + `ApprovalGate`，
/// 但智能体工具回路此前只有「Plan 模式硬拒」一道门 —— 同一条聊天回合里模型调
/// `shell_exec` 删目录、`delete_file_permanent` 永久删文件、`system_control` 驱动本机
/// UI，都不弹确认卡直接执行。两条执行路径的安全口径必须一致。
///
/// # 参数为何显式传入 gate / host_present
///
/// 生产路径传进程级单门 + [`crate::guard::gateway::agent_approval_host_present`]；
/// 单测传自己的短超时门 + 显式的宿主位。若在函数里直接读全局单例，测试就只能靠
/// 全局副作用造场景（同进程并行用例互相串扰，且要么白等 60s、要么无法验证「批准后
/// 真的执行」）。
///
/// # 返回
///
/// - `None`：无需审批，或审批通过 → 调用方继续执行；
/// - `Some(ToolResult::Error)`：拒绝 / 超时 / 无审批通道 → **不执行**。
async fn suspend_for_high_risk(
    gate: Arc<crate::guard::gateway::ApprovalGate>,
    host_present: bool,
    session_key: Option<&str>,
    name: &str,
    args: &HashMap<String, String>,
) -> Option<ToolResult> {
    suspend_with_risk(gate, host_present, session_key, name, args, false).await
}

/// W3.1：远程命令（`collab.ssh.exec`）的审批入口。
///
/// 复用与工具回路**完全相同**的一条链路（`ApprovalGate` → 全局注册表 →
/// 控制台审批卡 → 裁决），只是把「是否高危」显式置位：远程命令的破坏力
/// （在远端执行 `rm -rf`、改生产配置）无法用本机文件系统启发式判定。
pub(crate) async fn suspend_for_remote_command(
    gate: Arc<crate::guard::gateway::ApprovalGate>,
    host_present: bool,
    session_key: Option<&str>,
    name: &str,
    args: &HashMap<String, String>,
) -> Option<ToolResult> {
    suspend_with_risk(gate, host_present, session_key, name, args, true).await
}

/// `force_high = true` 时跳过「是否需要审批」的启发式判定，一律按高危挂起。
async fn suspend_with_risk(
    gate: Arc<crate::guard::gateway::ApprovalGate>,
    host_present: bool,
    session_key: Option<&str>,
    name: &str,
    args: &HashMap<String, String>,
    force_high: bool,
) -> Option<ToolResult> {
    let intent = if force_high {
        crate::guard::tool_intent::analyze(name, &args_json_of(args))
    } else {
        approval_required(name, args)?
    };

    // 无宿主（CLI / 未启动控制台）时没有人能裁决 → 立即 fail-closed，
    // 而不是让工具循环白等 60s 审批超时。
    gate.set_interactive(host_present);

    let risk = if force_high || intent.high_risk || intent.destructive || intent.irreversible {
        crate::guard::gateway::RiskLevel::High
    } else {
        crate::guard::gateway::RiskLevel::Medium
    };
    // `RiskLevel` 无 `Display`，事件里按 wire 值（`low`/`medium`/`high`）如实上报。
    let risk_str = match risk {
        crate::guard::gateway::RiskLevel::Low => "low",
        crate::guard::gateway::RiskLevel::Medium => "medium",
        crate::guard::gateway::RiskLevel::High => "high",
    };
    // P2-3：该会话已开启「审批旁路 / 完全提权」→ 跳过人工确认这一步，
    // 但仍发 Approval 事件（`elevated_skip: true`）留审计轨迹。
    // 边界：只跳过 Confirm；模式黑名单 / Plan 只读 / 沙箱策略在别处判定，不在此放宽。
    if let Some(session) = session_key {
        if crate::guard::elevated::global().skips_approval_for(Some(session)) {
            if let Some(bus) = crate::events::global_bus() {
                bus.publish(crate::events::Envelope::new(
                    crate::events::EventType::Approval,
                    serde_json::json!({
                        "elevated_skip": true,
                        "tool": name,
                        "tool_name": name,
                        "risk": risk_str,
                        "intent": serde_json::to_value(&intent).unwrap_or_default(),
                        "session_key": session,
                        "source": "agent.tool_cycle",
                        "reason": "会话已开启审批旁路，跳过人工确认",
                    }),
                    session,
                ));
            }
            return None;
        }
    }
    let pending = gate.request(name, args_json_of(args).to_string(), risk);
    let Some(pending) = pending else {
        return Some(ToolResult::Error {
            message: format!("已有其它待裁决的高危操作，{name} 本次不执行（fail-closed）"),
            code: "APPROVAL_BUSY".to_string(),
        });
    };
    // 注册进全局注册表：WebUI 每 2s 轮询 `/api/approvals` 据此渲染确认卡，
    // 裁决时经同一注册表定位到**本**门（与 chain::executor 抬升路径完全同源）。
    crate::guard::approval_registry::global_registry().register(gate.clone(), &pending);
    // 事件总线在线时额外推一条 approval 事件：TUI 的内联审批框靠它进入
    // AwaitingApproval 状态（`tui/workbench/state.rs` 的 EventType::Approval 分支）。
    if let Some(bus) = crate::events::global_bus() {
        bus.publish(crate::events::Envelope::new(
            crate::events::EventType::Approval,
            serde_json::json!({
                // 双键：TUI 读 `id`，链式执行器 / WebUI 读 `approval_id`
                "id": pending.id,
                "approval_id": pending.id,
                "tool": name,
                "tool_name": name,
                "args": pending.args,
                "risk": risk_str,
                "intent": serde_json::to_value(&intent).unwrap_or_default(),
                "reason": format!("高危操作需人工确认：{}", intent.scope),
                "source": "agent.tool_cycle",
            }),
            "agent",
        ));
    }

    let outcome = loop {
        match gate.outcome(&pending.id) {
            crate::guard::gateway::ApprovalOutcome::Pending => {
                tokio::time::sleep(Duration::from_millis(50)).await
            }
            other => break other,
        }
    };
    // 已消费 → 从注册表移除，防止陈旧 ID 被重复裁决（与 executor 同语义）
    crate::guard::approval_registry::global_registry().unregister(&pending.id);

    match outcome {
        crate::guard::gateway::ApprovalOutcome::Approved => None,
        crate::guard::gateway::ApprovalOutcome::Denied => Some(ToolResult::Error {
            message: format!(
                "高危操作被拒绝，未执行：{}（{}）",
                intent.scope,
                intent.side_effects.join("；")
            ),
            code: "APPROVAL_DENIED".to_string(),
        }),
        crate::guard::gateway::ApprovalOutcome::Timeout => Some(ToolResult::Error {
            message: format!("高危操作审批超时（fail-closed），未执行：{}", intent.scope),
            code: "APPROVAL_TIMEOUT".to_string(),
        }),
        crate::guard::gateway::ApprovalOutcome::Pending => Some(ToolResult::Error {
            message: format!("高危操作未获批准，未执行：{}", intent.scope),
            code: "APPROVAL_PENDING".to_string(),
        }),
    }
}

/// 把工具参数（`HashMap<String,String>`）渲染成意图分析 / 审批门要用的 JSON 对象。
///
/// 工具回路的参数是「字符串化的 JSON 值」，故这里统一按字符串装箱：`intent` 表里的
/// 布尔/数组信号（`overwrite`/`recursive`/`paths`）在字符串形态下也能命中
/// （`tool_intent::flag` 与 `batch_marker` 都接受字符串形态）。
fn args_json_of(args: &HashMap<String, String>) -> serde_json::Value {
    serde_json::Value::Object(
        args.iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::registry::{ToolExecutor, ToolRegistry};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct EchoTool;

    #[async_trait]
    impl ToolExecutor for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "Echo tool for testing"
        }

        async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
            let msg = args.get("message").cloned().unwrap_or_default();
            Ok(ToolResult::Success {
                data: format!("echo: {}", msg),
                metadata: HashMap::new(),
            })
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {"type": "string"}
                }
            })
        }
    }

    struct SlowTool;

    #[async_trait]
    impl ToolExecutor for SlowTool {
        fn name(&self) -> &str {
            "slow"
        }

        fn description(&self) -> &str {
            "Slow tool for testing timeout"
        }

        async fn execute(&self, _args: &HashMap<String, String>) -> Result<ToolResult> {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok(ToolResult::Success {
                data: "done".to_string(),
                metadata: HashMap::new(),
            })
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
    }

    struct FailingTool;

    #[async_trait]
    impl ToolExecutor for FailingTool {
        fn name(&self) -> &str {
            "fail"
        }

        fn description(&self) -> &str {
            "Tool that always fails"
        }

        async fn execute(&self, _args: &HashMap<String, String>) -> Result<ToolResult> {
            Ok(ToolResult::Error {
                message: "Intentional failure".to_string(),
                code: "TEST_FAIL".to_string(),
            })
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
    }

    fn setup_registry() -> Arc<ToolRegistry> {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(EchoTool));
        registry.register_local(Box::new(FailingTool));
        Arc::new(registry)
    }

    /// 写类工具（名字命中门控矩阵的 Write 类），用于验证 Plan 只读模式硬拒绝。
    struct WriteTool;

    #[async_trait]
    impl ToolExecutor for WriteTool {
        fn name(&self) -> &str {
            "edit_file"
        }

        fn description(&self) -> &str {
            "Write tool for Plan-mode denial test"
        }

        async fn execute(&self, _args: &HashMap<String, String>) -> Result<ToolResult> {
            Ok(ToolResult::Success {
                data: "written".to_string(),
                metadata: HashMap::new(),
            })
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
    }

    #[tokio::test]
    async fn test_sequential_execution() {
        let registry = setup_registry();
        let cycle = ToolCycle::new(registry);

        let calls = vec![
            ToolCall::new("echo", {
                let mut args = HashMap::new();
                args.insert("message".to_string(), "hello".to_string());
                args
            }),
            ToolCall::new("echo", {
                let mut args = HashMap::new();
                args.insert("message".to_string(), "world".to_string());
                args
            }),
        ];

        let results = cycle.execute_sequential(calls).await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].is_success());
        assert!(results[1].is_success());
        assert_eq!(results[0].data(), Some("echo: hello"));
        assert_eq!(results[1].data(), Some("echo: world"));
    }

    #[tokio::test]
    async fn test_parallel_execution() {
        let registry = setup_registry();
        let cycle = ToolCycle::new(registry);

        let calls = vec![
            ToolCall::new("echo", {
                let mut args = HashMap::new();
                args.insert("message".to_string(), "parallel1".to_string());
                args
            }),
            ToolCall::new("echo", {
                let mut args = HashMap::new();
                args.insert("message".to_string(), "parallel2".to_string());
                args
            }),
            ToolCall::new("echo", {
                let mut args = HashMap::new();
                args.insert("message".to_string(), "parallel3".to_string());
                args
            }),
        ];

        let results = cycle.execute_parallel(calls).await.unwrap();
        assert_eq!(results.len(), 3);
        for result in &results {
            assert!(result.is_success());
        }
    }

    #[tokio::test]
    async fn test_timeout_control() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(SlowTool));
        let registry = Arc::new(registry);

        let mut cycle = ToolCycle::new(registry);
        cycle.set_max_concurrent(1);

        let call = ToolCall::new("slow", HashMap::new()).with_timeout(Duration::from_millis(100));

        let results = cycle.execute_sequential(vec![call]).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].is_timeout());

        match &results[0] {
            ToolResult::Timeout {
                tool_name,
                duration,
            } => {
                assert_eq!(tool_name, "slow");
                assert!(duration.as_millis() <= 500);
            }
            _ => panic!("Expected Timeout"),
        }
    }

    #[tokio::test]
    async fn test_tool_error() {
        let registry = setup_registry();
        let cycle = ToolCycle::new(registry);

        let call = ToolCall::new("fail", HashMap::new());
        let results = cycle.execute_sequential(vec![call]).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].is_error());

        match &results[0] {
            ToolResult::Error { message, code } => {
                assert_eq!(message, "Intentional failure");
                assert_eq!(code, "TEST_FAIL");
            }
            _ => panic!("Expected Error"),
        }
    }

    #[tokio::test]
    async fn test_tool_not_found() {
        let registry = setup_registry();
        let cycle = ToolCycle::new(registry);

        let call = ToolCall::new("nonexistent", HashMap::new());
        let results = cycle.execute_sequential(vec![call]).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].is_error());
    }

    #[tokio::test]
    async fn test_execute_default_parallel() {
        let registry = setup_registry();
        let cycle = ToolCycle::new(registry);

        let calls = vec![ToolCall::new("echo", {
            let mut args = HashMap::new();
            args.insert("message".to_string(), "default".to_string());
            args
        })];

        let results = cycle.execute(calls).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].is_success());
        assert_eq!(results[0].data(), Some("echo: default"));
    }

    #[tokio::test]
    async fn test_call_tool_success() {
        let registry = setup_registry();
        let cycle = ToolCycle::new(registry);

        let mut args = HashMap::new();
        args.insert("message".to_string(), "direct".to_string());
        let result = cycle.call_tool("echo", &args).await;
        assert!(result.is_success());
        assert_eq!(result.data(), Some("echo: direct"));
    }

    #[tokio::test]
    async fn test_call_tool_error() {
        let registry = setup_registry();
        let cycle = ToolCycle::new(registry);

        let result = cycle.call_tool("fail", &HashMap::new()).await;
        assert!(result.is_error());
        match &result {
            ToolResult::Error { message, code } => {
                assert_eq!(message, "Intentional failure");
                assert_eq!(code, "TEST_FAIL");
            }
            _ => panic!("Expected Error"),
        }
    }

    #[test]
    fn test_plan_mode_denies_write_tool_at_execution() {
        // MSC-2 验收④：Plan（只读）模式在**真实工具执行链路**上拒绝写类工具；
        // 其他模式（Agent）放行——证明 mode.switch 到 Plan 后是「真拒绝」而非仅提示。
        //
        // 用同步 `#[test]` + `block_on`（而非 `#[tokio::test]`）：既满足「改写全局
        // runtime_mode 必须持有 test_runtime_mode_guard」的约束，又不把 MutexGuard
        // 跨 await 持有（clippy::await_holding_lock）。
        use crate::mode::{set_runtime_mode, test_runtime_mode_guard, ModeKind};
        let _g = test_runtime_mode_guard();
        let prev = crate::mode::runtime_mode();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(WriteTool));
        let cycle = ToolCycle::new(Arc::new(registry));

        // Plan：写类工具被硬拒绝（MODE_DENIED），且不触碰底层工具。
        set_runtime_mode(ModeKind::Plan);
        let denied = rt.block_on(cycle.call_tool("edit_file", &HashMap::new()));
        match &denied {
            ToolResult::Error { code, message } => {
                assert_eq!(code, "MODE_DENIED");
                assert!(message.contains("Plan"));
            }
            other => panic!("Plan 模式应拒绝写类工具，得到 {other:?}"),
        }

        // Agent：同一工具正常执行（无回归）。
        set_runtime_mode(ModeKind::Agent);
        let allowed = rt.block_on(cycle.call_tool("edit_file", &HashMap::new()));
        assert!(
            allowed.is_success(),
            "Agent 模式写类工具应放行，得到 {allowed:?}"
        );

        set_runtime_mode(prev);
    }

    #[tokio::test]
    async fn test_parallel_timeout() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(SlowTool));
        let registry = Arc::new(registry);

        let mut cycle = ToolCycle::new(registry);
        cycle.set_max_concurrent(1);

        let calls = vec![
            ToolCall::new("slow", HashMap::new()).with_timeout(Duration::from_millis(100)),
            ToolCall::new("slow", HashMap::new()).with_timeout(Duration::from_millis(100)),
        ];

        let results = cycle.execute_parallel(calls).await.unwrap();
        assert_eq!(results.len(), 2);
        for result in &results {
            assert!(result.is_timeout());
        }
        match &results[0] {
            ToolResult::Timeout {
                tool_name,
                duration,
            } => {
                assert_eq!(tool_name, "slow");
                assert!(duration.as_millis() <= 500);
            }
            _ => panic!("Expected Timeout"),
        }
    }

    #[tokio::test]
    async fn test_parallel_error_and_success_mixed() {
        let registry = setup_registry();
        let cycle = ToolCycle::new(registry);

        let calls = vec![
            ToolCall::new("echo", HashMap::new()),
            ToolCall::new("fail", HashMap::new()),
            ToolCall::new("missing", HashMap::new()),
        ];

        let results = cycle.execute_parallel(calls).await.unwrap();
        assert_eq!(results.len(), 3);
        assert!(results[0].is_success());
        assert!(results[1].is_error());
        assert!(results[2].is_error());
    }

    #[tokio::test]
    async fn test_max_iterations_exceeded() {
        let registry = setup_registry();
        let mut cycle = ToolCycle::new(registry);
        cycle.set_max_iterations(1);

        let calls = vec![
            ToolCall::new("echo", HashMap::new()),
            ToolCall::new("echo", HashMap::new()),
        ];

        // 顺序执行超出限制
        let result = cycle.execute_sequential(calls.clone()).await;
        assert!(result.is_err());

        // 并行执行超出限制
        let result = cycle.execute_parallel(calls).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_concurrent_limit() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(EchoTool));
        let registry = Arc::new(registry);

        let mut cycle = ToolCycle::new(registry);
        cycle.set_max_concurrent(2);

        let calls = (0..5)
            .map(|i| {
                ToolCall::new("echo", {
                    let mut args = HashMap::new();
                    args.insert("message".to_string(), format!("msg{}", i));
                    args
                })
            })
            .collect();

        let results = cycle.execute_parallel(calls).await.unwrap();
        assert_eq!(results.len(), 5);
        for result in &results {
            assert!(result.is_success());
        }
    }

    #[test]
    fn test_tool_result_is_success() {
        let success = ToolResult::Success {
            data: "ok".to_string(),
            metadata: HashMap::new(),
        };
        assert!(success.is_success());
        assert!(!success.is_error());
        assert!(!success.is_timeout());
        assert_eq!(success.data(), Some("ok"));
    }

    #[test]
    fn test_tool_result_is_error() {
        let error = ToolResult::Error {
            message: "fail".to_string(),
            code: "ERR".to_string(),
        };
        assert!(error.is_error());
        assert!(!error.is_success());
        assert!(!error.is_timeout());
        assert_eq!(error.data(), None);
    }

    #[test]
    fn test_tool_result_is_timeout() {
        let timeout = ToolResult::Timeout {
            tool_name: "test".to_string(),
            duration: Duration::from_secs(1),
        };
        assert!(timeout.is_timeout());
        assert!(!timeout.is_success());
        assert!(!timeout.is_error());
        assert_eq!(timeout.data(), None);
    }

    #[test]
    fn test_tool_call_with_timeout() {
        let call = ToolCall::new("test", HashMap::new()).with_timeout(Duration::from_secs(10));
        assert_eq!(call.timeout.as_secs(), 10);
        assert_eq!(call.name, "test");
    }

    #[test]
    fn test_tool_cycle_defaults() {
        let registry = Arc::new(ToolRegistry::new());
        let cycle = ToolCycle::new(registry);
        // 单批上限来自自适应预算（Standard 档 24），不再是硬编码 10；
        // 但必须始终有限且不小于历史值，避免一次批量把整轮拖死。
        assert!(
            (10..=crate::agent::budget_policy::MAX_TOOL_ROUNDS as u32)
                .contains(&cycle.max_iterations()),
            "unexpected batch cap: {}",
            cycle.max_iterations()
        );
        assert_eq!(cycle.max_concurrent(), 5);
        assert_eq!(cycle.default_timeout().as_secs(), 30);
    }

    #[test]
    fn test_tool_cycle_setters() {
        let registry = Arc::new(ToolRegistry::new());
        let mut cycle = ToolCycle::new(registry);
        cycle.set_max_iterations(20);
        cycle.set_max_concurrent(10);
        assert_eq!(cycle.max_iterations(), 20);
        assert_eq!(cycle.max_concurrent(), 10);
    }

    /// 计数工具：每次真实执行 +1，用于验证缓存是否跳过重复执行（W-3）。
    struct CountingTool(Arc<AtomicUsize>);

    #[async_trait]
    impl ToolExecutor for CountingTool {
        fn name(&self) -> &str {
            // 用**真实只读工具名**，才能被默认幂等策略（只读白名单）缓存 ——
            // 这正是这两个用例要验证的缓存机制。历史上用的是 `count`，而新策略
            // 只缓存已知只读工具，故改名以保持用例意图。
            "read_file"
        }
        fn description(&self) -> &str {
            "counts executions"
        }
        async fn execute(&self, _args: &HashMap<String, String>) -> Result<ToolResult> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(ToolResult::Success {
                data: "c".into(),
                metadata: HashMap::new(),
            })
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
    }

    #[tokio::test]
    async fn test_tool_cycle_caches_repeat_calls() {
        // W-3：注入 ToolCache 后，同一 (tool,args) 的两次 call_tool 仅执行底层工具一次
        // （第二次命中缓存，不再重跑）。
        let mut registry = ToolRegistry::new();
        let calls = Arc::new(AtomicUsize::new(0));
        registry.register_local(Box::new(CountingTool(calls.clone())));
        let registry = Arc::new(registry);
        let cache = Arc::new(ToolCache::with_default_policy(8, None));
        let cycle = ToolCycle::new(registry).with_tool_cache(cache.clone());
        let args = HashMap::from([("x".to_string(), "1".to_string())]);

        let _r1 = cycle.call_tool("read_file", &args).await;
        let _r2 = cycle.call_tool("read_file", &args).await;

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "相同入参第二次应命中工具缓存，底层工具仅执行 1 次"
        );
        assert!(
            cache.stats().hits >= 1,
            "工具缓存应记录命中，hits={}",
            cache.stats().hits
        );
    }

    #[tokio::test]
    async fn test_tool_cycle_default_injects_cache_and_caches() {
        // W-3 延伸：默认构造的 ToolCycle 即带工具缓存（生产默认）；execute_sequential
        // 重复同参应仅执行底层工具一次（第二次命中缓存）。
        let mut registry = ToolRegistry::new();
        let calls = Arc::new(AtomicUsize::new(0));
        registry.register_local(Box::new(CountingTool(calls.clone())));
        let registry = Arc::new(registry);
        let cycle = ToolCycle::new(registry);
        let cache = cycle
            .tool_cache
            .clone()
            .expect("ToolCycle::new 应默认注入 ToolCache（W-3 未完成时缺失）");

        let args = HashMap::from([("x".to_string(), "1".to_string())]);
        let calls_tc = vec![
            ToolCall::new("read_file", args.clone()),
            ToolCall::new("read_file", args),
        ];
        let _ = cycle.execute_sequential(calls_tc).await.unwrap();

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "相同入参顺序两次应仅执行底层工具 1 次"
        );
        assert!(
            cache.stats().hits >= 1,
            "默认工具缓存应记录命中，hits={}",
            cache.stats().hits
        );
    }

    /// 大 JSON 工具经 call_tool 返回时，应被确定性压缩并附指纹（W-4 接线验证）。
    struct BigJsonTool;

    #[async_trait]
    impl ToolExecutor for BigJsonTool {
        fn name(&self) -> &str {
            "bigjson"
        }
        fn description(&self) -> &str {
            "returns large json"
        }
        async fn execute(&self, _args: &HashMap<String, String>) -> Result<ToolResult> {
            // 构造含大数组的对象：超默认预算（2000），且数组截断后显著变短，
            // 验证 W-4 压缩器在 call_tool 链路上真正生效并附指纹。
            let items: Vec<String> = (0..300)
                .map(|i| {
                    format!(
                        "{{\"id\":{},\"name\":\"item number {} with a long descriptive label\"}}",
                        i, i
                    )
                })
                .collect();
            Ok(ToolResult::Success {
                data: format!("{{\"items\":[{}]}}", items.join(",")),
                metadata: HashMap::new(),
            })
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
    }

    #[tokio::test]
    async fn test_tool_cycle_compresses_large_tool_output() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(BigJsonTool));
        let registry = Arc::new(registry);
        let cycle = ToolCycle::new(registry);
        let comp = cycle
            .tool_compressor
            .clone()
            .expect("ToolCycle::new 应默认注入 ToolCompressor（W-4）");
        let result = cycle.call_tool("bigjson", &HashMap::new()).await;
        assert!(result.is_success());
        let data = result.data().expect("应有数据");
        assert!(
            data.contains("compressed sha256:"),
            "大输出应被压缩并附指纹: {}",
            data
        );
        assert!(comp.stats().compressed >= 1, "压缩器应记录一次压缩");
    }
}

/// 高危审批闸门（P0-4）的回归守卫。
///
/// 所有用例的 gate 与「宿主在场」标志都**显式注入**，不读进程级单例 ——
/// 否则同进程并行用例会互相串扰（一个用例把宿主点亮，另一个用例就白等审批超时）。
#[cfg(test)]
mod approval_gate_tests {
    use super::*;
    use crate::guard::gateway::{ApprovalGate, ApprovalOutcome, RiskLevel};
    use crate::mcp::registry::{ToolExecutor, ToolRegistry};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 计数工具：`shell_exec` 是真实 ExecWrite 工具名，会命中高危判定。
    struct SpyShell(Arc<AtomicUsize>);

    #[async_trait]
    impl ToolExecutor for SpyShell {
        fn name(&self) -> &str {
            "shell_exec"
        }
        fn description(&self) -> &str {
            "spy shell executor"
        }
        async fn execute(&self, _args: &HashMap<String, String>) -> Result<ToolResult> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(ToolResult::Success {
                data: "ran".into(),
                metadata: HashMap::new(),
            })
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
    }

    fn args_of(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    /// 判定面：只拦破坏性/不可逆/桌面/系统级不可逆；普通读写一律放行。
    ///
    /// 这条用例同时是「产品可用性」的守卫：若哪天把普通 `write_file` 也纳入确认，
    /// 每个写文件都要用户点一次按钮。
    #[test]
    fn approval_required_scopes_exactly_the_high_risk_surface() {
        // 必须拦：永久删除 / 删除 / 覆盖写 / 危险 shell / 终止进程 / 桌面控制
        let must_gate = [
            ("delete_file_permanent", args_of(&[("path", "D:/x/report.txt")])),
            ("delete_file", args_of(&[("path", "D:/x/a.txt")])),
            ("fs_delete", args_of(&[("path", "D:/x/a.txt")])),
            (
                "write_file",
                args_of(&[("path", "a.md"), ("content", "x"), ("overwrite", "true")]),
            ),
            ("shell_exec", args_of(&[("command", "rm -rf build")])),
            (
                "shell_exec",
                args_of(&[("command", "Remove-Item -Recurse D:\\old")]),
            ),
            (
                "process_manage",
                args_of(&[("action", "kill"), ("pid", "4321")]),
            ),
            ("system_control", args_of(&[("action", "click")])),
            ("office_automation", args_of(&[("action", "export_pdf")])),
            ("window_manage", args_of(&[("action", "focus")])),
            // 2026-10-23：回放会真实驱动本机键鼠，与 system_control 同属高危面。
            ("auto_replay", args_of(&[("name", "daily-report")])),
            ("auto_record", args_of(&[("action", "start"), ("name", "m")])),
        ];
        for (tool, args) in must_gate {
            assert!(
                approval_required(tool, &args).is_some(),
                "{tool}({args:?}) 属高危面，必须要求人工确认"
            );
        }

        // 必须放行：只读 / 普通写 / 无害命令 / 只读执行 / 出网
        let must_pass = [
            ("read_file", args_of(&[("path", "a.md")])),
            ("write_file", args_of(&[("path", "a.md"), ("content", "x")])),
            (
                "append_file",
                args_of(&[("path", "log.txt"), ("content", "x")]),
            ),
            ("make_dir", args_of(&[("path", "out")])),
            ("shell_exec", args_of(&[("command", "cargo test")])),
            ("run_tests", args_of(&[("command", "cargo test")])),
            ("git_status", args_of(&[])),
            ("web_search", args_of(&[("query", "rust")])),
            ("kb_search", args_of(&[("query", "q")])),
            // 多模态产物工具不驱动本机 UI，只读写工作区 → 不应每次点确认。
            ("image_generate", args_of(&[("prompt", "a cat")])),
            ("audio_transcribe", args_of(&[("path", "a.wav")])),
            ("audio_speak", args_of(&[("text", "你好")])),
        ];
        for (tool, args) in must_pass {
            assert!(
                approval_required(tool, &args).is_none(),
                "{tool}({args:?}) 不属高危面，不应打断用户（否则每次操作都要点确认）"
            );
        }
    }

    /// 无宿主（CLI / 未启动控制台）→ 立即 fail-closed，**且不白等审批超时**。
    #[tokio::test]
    async fn no_host_is_immediately_denied_without_waiting_for_timeout() {
        // 60s 门 + 无宿主：若实现退化成「等超时」，这里会挂满 60s
        let gate = Arc::new(ApprovalGate::new(Duration::from_secs(60)));
        let started = std::time::Instant::now();
        let blocked = suspend_for_high_risk(
            gate,
            false,
            None,
            "shell_exec",
            &args_of(&[("command", "rm -rf build")]),
        )
        .await;
        let blocked = blocked.expect("无宿主时必须 fail-closed");
        assert!(blocked.is_error(), "应是错误结果（未执行）: {blocked:?}");
        assert!(blocked.data().is_none(), "不得输出任何伪装成成功的内容");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "无宿主必须立即拒绝，不得白等审批超时（实测 {:?}）",
            started.elapsed()
        );
    }

    /// 宿主在场 + 批准 → 闸门放行（返回 None），调用方得以继续执行。
    #[tokio::test]
    async fn host_present_and_approved_releases_the_call() {
        let gate = Arc::new(ApprovalGate::new(Duration::from_secs(10)));
        let judge = gate.clone();
        let approver = tokio::spawn(async move {
            // 直接向**本门**轮询挂起项：走全局注册表会拿到并行用例的项，误伤他人。
            for _ in 0..500 {
                if let Some(p) = judge.pending() {
                    return judge.approve(&p.id);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            false
        });

        let out = suspend_for_high_risk(
            gate,
            true,
            None,
            "shell_exec",
            &args_of(&[("command", "rm -rf build")]),
        )
        .await;
        assert!(out.is_none(), "批准后应放行: {out:?}");
        assert!(approver.await.unwrap_or(false), "裁决方应真的批准到本门");
    }

    /// 宿主在场 + 拒绝 → 不执行，错误码可诊断。
    #[tokio::test]
    async fn host_present_and_denied_blocks_the_call() {
        let gate = Arc::new(ApprovalGate::new(Duration::from_secs(10)));
        let judge = gate.clone();
        tokio::spawn(async move {
            for _ in 0..500 {
                if let Some(p) = judge.pending() {
                    let _ = judge.deny(&p.id);
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });

        let out = suspend_for_high_risk(
            gate,
            true,
            None,
            "delete_file_permanent",
            &args_of(&[("path", "D:/x/report.txt")]),
        )
        .await;
        match out.expect("拒绝后必须阻断") {
            ToolResult::Error { code, .. } => {
                assert_eq!(code, "APPROVAL_DENIED", "错误码必须可诊断")
            }
            other => panic!("拒绝应是 Error，实际 {other:?}"),
        }
    }

    /// 同一时刻只允许一个挂起项：第二个高危调用必须 fail-closed，不得静默排队。
    #[tokio::test]
    async fn second_high_risk_call_fails_closed_while_one_is_pending() {
        let gate = Arc::new(ApprovalGate::new(Duration::from_secs(60)));
        gate.set_interactive(true);
        let held = gate
            .request("shell_exec", "{}".to_string(), RiskLevel::High)
            .expect("首个挂起应成功");

        let out = suspend_for_high_risk(
            gate.clone(),
            true,
            None,
            "shell_exec",
            &args_of(&[("command", "rm -rf build")]),
        )
        .await;
        match out.expect("已有挂起项时第二个应 fail-closed") {
            ToolResult::Error { code, .. } => {
                assert_eq!(code, "APPROVAL_BUSY", "应给出 APPROVAL_BUSY")
            }
            other => panic!("应是 Error，实际 {other:?}"),
        }
        assert_eq!(gate.outcome(&held.id), ApprovalOutcome::Pending);
    }

    /// P2-3：会话开启 bypass 后，高危工具**不再弹审批**；其它会话不受影响。
    ///
    /// 无宿主 + 60s 门是刻意的最坏场景：没跳过就会 fail-closed（或白等超时），
    /// 因此「立即返回 None 且没有挂起项」同时证明「真的跳过了」与「没有放宽别的门」。
    #[tokio::test]
    async fn elevated_bypass_skips_high_risk_confirmation_only_for_its_session() {
        let session = "agent:p2-3-bypass-test";
        let elevated = crate::guard::elevated::global();
        elevated.set(session, Some("bypass"));

        let gate = Arc::new(ApprovalGate::new(Duration::from_secs(60)));
        let started = std::time::Instant::now();
        let out = suspend_for_high_risk(
            gate.clone(),
            false,
            Some(session),
            "shell_exec",
            &args_of(&[("command", "rm -rf build")]),
        )
        .await;
        assert!(out.is_none(), "已提权会话应跳过人工确认: {out:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "跳过必须立即生效，不得白等审批超时"
        );
        assert!(gate.pending().is_none(), "跳过后不得产生挂起项");

        let other = suspend_for_high_risk(
            gate.clone(),
            false,
            Some("agent:p2-3-unrelated"),
            "shell_exec",
            &args_of(&[("command", "rm -rf build")]),
        )
        .await;
        assert!(other.is_some(), "未提权会话仍必须 fail-closed");

        elevated.set(session, None);
    }

    /// **接线守卫**：闸门必须真的接在 `dispatch_cached` 上，而不是个没人调用的辅助函数。
    ///
    /// 本用例走完整 `call_tool` 路径（进程级门 + 自动宿主判定）。逐案情况：
    /// - 无宿主（CLI/库调用）→ `APPROVAL_DENIED`；
    /// - 宿主在场（同进程网关用例注册过全局总线）→ 10ms 测试超时后 `APPROVAL_TIMEOUT`；
    /// - 同进程若有用例把全局模式留在 `Plan`，则是 `MODE_DENIED`（同样是 fail-closed）。
    ///
    /// 三条分支都满足要守的不变量：① 不假装成功；② `shell_exec` 底层实现一次都没执行。
    #[tokio::test]
    async fn dispatch_blocks_high_risk_tool_and_never_executes_it() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(SpyShell(calls.clone())));
        let cycle = ToolCycle::new(Arc::new(registry));

        let result = cycle
            .call_tool("shell_exec", &args_of(&[("command", "rm -rf build")]))
            .await;

        match result {
            ToolResult::Error { code, message } => {
                assert!(
                    code.starts_with("APPROVAL_") || code == "MODE_DENIED",
                    "必须是 fail-closed 类错误码，实际 {code}: {message}"
                );
            }
            other => panic!("高危工具未获批准时不得返回成功/超时: {other:?}"),
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "未获批准的 `rm -rf` 绝不能落到真实执行"
        );
    }
}

/// P3-1 长任务熔断（暂停 + 回滚点）的回归守卫。
///
/// 全部用例显式传入 `at_ms` 时间线，不读系统时钟：同一用例可复现任意时间序。
#[cfg(test)]
mod circuit_tests {
    use super::*;
    use crate::agent::task_checkpoint::{CheckpointStore, TaskCheckpoint, PAUSED_FOR_ROLLBACK};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    fn args(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn call(tool: &str, arg: &str) -> ToolCall {
        ToolCall::new(tool, args(&[("arg", arg)]))
    }

    fn ok() -> ToolResult {
        ToolResult::Success {
            data: "ok".to_string(),
            metadata: HashMap::new(),
        }
    }

    fn err() -> ToolResult {
        ToolResult::Error {
            message: "boom".to_string(),
            code: "E_TOOL".to_string(),
        }
    }

    static DIR_SEQ: AtomicU64 = AtomicU64::new(0);

    fn unique_dir() -> PathBuf {
        let n = DIR_SEQ.fetch_add(1, AtomicOrdering::SeqCst);
        std::env::temp_dir().join(format!("marg-circuit-{}-{n}", std::process::id()))
    }

    /// 连续 5 次工具错误 → 熔断（暂停）并给出可回滚点；4 连错不提前判。
    #[test]
    fn circuit_trips_after_five_consecutive_tool_errors() {
        let mut breaker = CircuitBreaker::new("task-five-errors");
        let mut last = CircuitVerdict::ok();
        for i in 0..CIRCUIT_MAX_CONSECUTIVE_ERRORS {
            last = breaker.observe(
                &call("read_file", &format!("file-{i}")),
                &err(),
                i as u64 * 100,
            );
        }
        assert!(last.tripped, "连续 5 次工具错误必须熔断（暂停），而非继续空转");
        match last.reason.clone() {
            Some(CircuitReason::ConsecutiveErrors { count }) => assert_eq!(count, 5),
            other => panic!("熔断原因应是连续错误，实际 {other:?}"),
        }
        assert_eq!(
            last.checkpoint_id.as_deref(),
            Some("task-five-errors"),
            "熔断必须返回可回滚点（checkpoint 标识）"
        );

        // 可回滚点真的落盘：checkpoint 进入 paused_for_rollback，已完成步骤不被丢弃。
        let store = CheckpointStore::open(unique_dir()).unwrap();
        store
            .record(TaskCheckpoint::new("task-five-errors", "sess", "长任务"))
            .unwrap();
        let paused = breaker.pause(&store).expect("已建 checkpoint 的任务应能暂停");
        assert_eq!(paused.status, PAUSED_FOR_ROLLBACK, "应进入可回滚暂停态");
        assert_eq!(
            store.list_interrupted().len(),
            0,
            "暂停态不是「进程中断的在飞任务」，不应混进中断清单"
        );

        // 四连错（窗口样本数不足）不得熔断：证据驱动，不反应过敏。
        let mut short = CircuitBreaker::new("task-four-errors");
        let mut v = CircuitVerdict::ok();
        for i in 0..4u64 {
            v = short.observe(&call("read_file", &format!("f{i}")), &err(), i * 100);
        }
        assert!(!v.tripped, "4 连错不应提前熔断，实际 {v:?}");
    }

    /// 60 秒窗口内错误率 > 15% → 熔断；窗口外的历史错误不得计入。
    #[test]
    fn circuit_trips_on_error_rate_inside_sixty_second_window() {
        let mut breaker = CircuitBreaker::new("task-error-rate");
        let mut verdict = CircuitVerdict::ok();
        for i in 0..14u64 {
            // 非连续错误：2、6、11，最长连续 1（不会命中 5 连门槛）
            let failed = matches!(i, 2 | 6 | 11);
            let result = if failed { err() } else { ok() };
            verdict = breaker.observe(&call("read_file", &format!("p{i}")), &result, i * 100);
        }
        assert!(verdict.tripped, "2/8 = 25% 错误率（非连续）应熔断");
        match verdict.reason.clone() {
            Some(CircuitReason::ErrorRate {
                errors,
                calls,
                rate,
            }) => {
                assert_eq!(errors, 2, "窗口内 2 次错误");
                assert_eq!(
                    calls, 8,
                    "第 2 次错误后的第 8 个窗口样本触发（2/8 = 25%），而非累计到人手"
                );
                assert!(rate > CIRCUIT_ERROR_RATE, "实际错误率 {rate} 应超阈值");
            }
            other => panic!("熔断原因应是错误率，实际 {other:?}"),
        }

        // 窗口剪枝：7 个旧样本（4 错 = 57%）因不足最小样本数不判熔断；
        // 120 秒后追加成功样本，旧错误必须被剪掉，否则 4/8 = 50% 会被误判熔断。
        let mut stale = CircuitBreaker::new("task-window-prune");
        let mut v = CircuitVerdict::ok();
        for i in 0..7u64 {
            let result = if i < 4 { err() } else { ok() };
            v = stale.observe(&call("read_file", &format!("old{i}")), &result, i * 100);
        }
        assert!(!v.tripped, "样本数不足最小窗口样本，不得按错误率熔断");
        for i in 0..8u64 {
            v = stale.observe(&call("read_file", &format!("fresh{i}")), &ok(), 120_000 + i * 10);
        }
        assert!(!v.tripped, "60 秒窗口外的历史错误不得计入当前错误率");
    }

    /// 同参数连续 3 步 → 判循环并熔断；2 步不判；参数变化打断循环；回滚后恢复。
    #[test]
    fn circuit_trips_when_same_parameters_repeat_three_steps() {
        let same = args(&[("path", "D:/w/report.md"), ("limit", "100")]);
        let mut breaker = CircuitBreaker::new("task-same-params");
        let mut verdict = CircuitVerdict::ok();
        for i in 0..CIRCUIT_MAX_REPEAT_STEPS {
            verdict = breaker.observe(
                &ToolCall::new("read_file", same.clone()),
                &ok(),
                i as u64 * 50,
            );
        }
        assert!(verdict.tripped, "同参数连续 3 步应判循环并熔断");
        match verdict.reason.clone() {
            Some(CircuitReason::RepeatedParameters {
                fingerprint,
                steps,
            }) => {
                assert_eq!(steps, 3);
                assert_eq!(fingerprint, tool_fingerprint("read_file", &same));
            }
            other => panic!("熔断原因应是同参数循环，实际 {other:?}"),
        }
        assert_eq!(verdict.checkpoint_id.as_deref(), Some("task-same-params"));

        // 指纹与 HashMap 插入顺序无关（确定性、跨进程稳定）。
        let mut reversed = HashMap::new();
        reversed.insert("limit".to_string(), "100".to_string());
        reversed.insert("path".to_string(), "D:/w/report.md".to_string());
        assert_eq!(
            tool_fingerprint("read_file", &same),
            tool_fingerprint("read_file", &reversed),
            "参数指纹必须与插入顺序无关"
        );

        // 同参数 2 步不判循环；第 3 步换参数即打断连续计数。
        let mut two = CircuitBreaker::new("task-two-steps");
        let mut v = CircuitVerdict::ok();
        for i in 0..2u64 {
            v = two.observe(&ToolCall::new("read_file", same.clone()), &ok(), i * 50);
        }
        assert!(!v.tripped, "同参数 2 步不应判循环");
        v = two.observe(
            &ToolCall::new("read_file", args(&[("path", "D:/w/other.md")])),
            &ok(),
            150,
        );
        assert!(!v.tripped, "参数变化应打断连续计数");

        // 操作员确认回滚后恢复度量。
        breaker.resume();
        assert!(!breaker.verdict().tripped, "resume 后应回到未熔断");
    }
}
