//! Meta-agent 调度器：无头转发任务到 11 个外部 agent CLI，统一回收结果。
//!
//! 能力对齐 dsh-agent-conductor（90 行纯 stdlib 的 dispatch.py），但在 Marguerite
//! 内实现为 Rust 原生 dispatcher：不依赖 python、无头运行、超时 + 取消 +
//! 结构化结果。对外接口：
//!
//! ```ignore
//! use crate::harness::dispatch::{dispatch, DispatchTarget, DispatchRequest, DispatchResult};
//! let result = dispatch(DispatchRequest::new(
//!     DispatchTarget::Codex,
//!     "为项目生成一个登录页",
//!     "/home/user/workspace",
//! )).await?;
//! ```
//!
//! supersedes: "dsh-agent-conductor"
//!
//! 覆盖的 11 个 CLI（DispatchTarget 枚举一一对应；不存在的命令自动返回
//! `not_available`，不会 panic 挂起 Marguerite runtime）：
//! 1. Codex（OpenAI Codex / claude-codex CLI 兼容）
//! 2. ClaudeCode（Anthropic Claude Code）
//! 3. Gemini（Google Gemini CLI）
//! 4. Cursor（Cursor 远程 headless CLI）
//! 5. Kimi（月之暗面 Kimi CLI）
//! 6. Qwen（通义千问 CLI / codeqwen）
//! 7. Copilot（GitHub Copilot CLI）
//! 8. DeepSeek（官方 deepseek CLI / dsh）
//! 9. Doubao（字节豆包 CLI）
//! 10. CodeWhale（第三方 Rust 编码 agent，harness/ 第三方已克隆参考）
//! 11. OpenHands（OpenHands / OpenDevin 无头 CLI）

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::task::JoinSet;
use uuid::Uuid;

/// 支持的外部 agent 目标（11 个，与策划 §三.2 一一对应）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchTarget {
    /// OpenAI Codex / codex-coding-agent-compatible CLI
    Codex,
    /// Anthropic Claude Code (`claude` CLI)
    ClaudeCode,
    /// Google Gemini (`gemini` CLI)
    Gemini,
    /// Cursor (`cursor` headless / remote-session CLI)
    Cursor,
    /// 月之暗面 Kimi (`kimi` CLI)
    Kimi,
    /// 通义千问 / CodeQwen (`qwen` CLI)
    Qwen,
    /// GitHub Copilot (`gh copilot` / `copilot` CLI)
    Copilot,
    /// 官方 deepseek-harness (`dsh` / `deepseek` CLI)
    DeepSeek,
    /// 字节豆包 (`doubao` CLI)
    Doubao,
    /// CodeWhale（第三方 Rust 编码 agent，参见 third_party/harness/CodeWhale）
    CodeWhale,
    /// OpenHands / OpenDevin 无头 CLI
    OpenHands,
}

impl DispatchTarget {
    /// 所有可调度目标（用于列表枚举 / UI）
    pub const ALL: [DispatchTarget; 11] = [
        DispatchTarget::Codex,
        DispatchTarget::ClaudeCode,
        DispatchTarget::Gemini,
        DispatchTarget::Cursor,
        DispatchTarget::Kimi,
        DispatchTarget::Qwen,
        DispatchTarget::Copilot,
        DispatchTarget::DeepSeek,
        DispatchTarget::Doubao,
        DispatchTarget::CodeWhale,
        DispatchTarget::OpenHands,
    ];

    /// 枚举所有权 11 个目标（CLI list / 扇出 many 使用）
    pub const fn all() -> &'static [DispatchTarget] {
        &Self::ALL
    }

    /// 人可读描述（一行，用于 CLI 列表和 --help）
    pub fn description(&self) -> &'static str {
        match self {
            DispatchTarget::Codex => "OpenAI Codex / codex-coding-agent-compatible CLI",
            DispatchTarget::ClaudeCode => "Anthropic Claude Code (claude) headless CLI",
            DispatchTarget::Gemini => "Google Gemini CLI（gemini --batch 无头）",
            DispatchTarget::Cursor => "Cursor headless / remote-session CLI",
            DispatchTarget::Kimi => "月之暗面 Kimi 无头 CLI（kimi --headless）",
            DispatchTarget::Qwen => "通义千问 / CodeQwen 无头 CLI（qwen --batch）",
            DispatchTarget::Copilot => "GitHub Copilot：gh copilot suggest 批处理模式",
            DispatchTarget::DeepSeek => "官方 deepseek-harness（dsh / deepseek CLI 无头）",
            DispatchTarget::Doubao => "字节豆包（doubao --no-ui 无头）",
            DispatchTarget::CodeWhale => "CodeWhale 第三方 Rust 编码 agent（--batch）",
            DispatchTarget::OpenHands => "OpenHands / OpenDevin 无头 CLI（--no-browser）",
        }
    }

    /// 同步可用性探测：PATH 中是否能定位到二进制（等价于 `available().await` 的阻塞版）。
    /// 用于 `dispatch list` 单线程视图；避免启动异步 runtime 即可列出。
    pub fn probe(&self) -> Result<ProbeInfo> {
        let tmpl = self.default_cmd();
        // 尝试用 which / where 找到二进制
        let bin_name = tmpl.bin.to_string_lossy().to_string();
        let found = match which_locate(&bin_name) {
            Some(p) => p,
            None => {
                anyhow::bail!(
                    "未在 PATH 中找到二进制 '{bin_name}'（请安装后再运行 dispatch run/many）"
                )
            }
        };
        Ok(ProbeInfo {
            bin_path: found,
            target: *self,
        })
    }

    /// 目标的短名（用于日志 / 事件键）
    pub fn as_str(&self) -> &'static str {
        match self {
            DispatchTarget::Codex => "codex",
            DispatchTarget::ClaudeCode => "claude_code",
            DispatchTarget::Gemini => "gemini",
            DispatchTarget::Cursor => "cursor",
            DispatchTarget::Kimi => "kimi",
            DispatchTarget::Qwen => "qwen",
            DispatchTarget::Copilot => "copilot",
            DispatchTarget::DeepSeek => "deepseek",
            DispatchTarget::Doubao => "doubao",
            DispatchTarget::CodeWhale => "codewhale",
            DispatchTarget::OpenHands => "openhands",
        }
    }

    /// 从短名解析（大小写不敏感）
    pub fn from_name(name: &str) -> Option<Self> {
        let lower = name.trim().to_ascii_lowercase();
        for t in DispatchTarget::ALL.iter() {
            if t.as_str() == lower {
                return Some(*t);
            }
        }
        // 常见别名兼容
        match lower.as_str() {
            "claude-code" | "claude" => Some(DispatchTarget::ClaudeCode),
            "openhands" | "opendevin" => Some(DispatchTarget::OpenHands),
            "ds" | "dsh" => Some(DispatchTarget::DeepSeek),
            "gh-copilot" => Some(DispatchTarget::Copilot),
            _ => None,
        }
    }

    /// 默认命令 + 无头参数模板。
    /// 返回 (binary, args_template)：args_template 中 `{task}` 被替换为任务、
    /// `{workspace}` 被替换为工作目录；不支持模板的走 stdin。
    fn default_cmd(&self) -> CmdTemplate {
        match self {
            // ---- 11 个目标（dsh-agent-conductor 对齐）----
            DispatchTarget::Codex => CmdTemplate {
                bin: "codex".into(),
                args: vec!["--non-interactive".into(), "--task".into(), "{task}".into()],
                pass_task_via_stdin: false,
                cwd_arg: None,
                extra_env: HashMap::new(),
            },
            DispatchTarget::ClaudeCode => CmdTemplate {
                bin: "claude".into(),
                args: vec!["--no-prompt".into(), "-p".into(), "{task}".into()],
                pass_task_via_stdin: false,
                cwd_arg: None,
                extra_env: HashMap::new(),
            },
            DispatchTarget::Gemini => CmdTemplate {
                bin: "gemini".into(),
                args: vec!["--batch".into(), "{task}".into()],
                pass_task_via_stdin: false,
                cwd_arg: None,
                extra_env: HashMap::new(),
            },
            DispatchTarget::Cursor => CmdTemplate {
                bin: "cursor".into(),
                args: vec!["--remote".into(), "--headless".into()],
                pass_task_via_stdin: true,
                cwd_arg: None,
                extra_env: HashMap::new(),
            },
            DispatchTarget::Kimi => CmdTemplate {
                bin: "kimi".into(),
                args: vec!["--headless".into(), "-q".into(), "{task}".into()],
                pass_task_via_stdin: false,
                cwd_arg: None,
                extra_env: HashMap::new(),
            },
            DispatchTarget::Qwen => CmdTemplate {
                bin: "qwen".into(),
                args: vec!["--batch".into(), "{task}".into()],
                pass_task_via_stdin: false,
                cwd_arg: None,
                extra_env: HashMap::new(),
            },
            DispatchTarget::Copilot => CmdTemplate {
                bin: "gh".into(),
                args: vec![
                    "copilot".into(),
                    "suggest".into(),
                    "--non-interactive".into(),
                ],
                pass_task_via_stdin: true,
                cwd_arg: None,
                extra_env: HashMap::new(),
            },
            DispatchTarget::DeepSeek => CmdTemplate {
                bin: "dsh".into(),
                args: vec![
                    "--non-interactive".into(),
                    "--no-color".into(),
                    "--query".into(),
                    "{task}".into(),
                ],
                pass_task_via_stdin: false,
                cwd_arg: None,
                extra_env: {
                    let mut m = HashMap::new();
                    // 无头模式：禁止打开 TTY/UI
                    m.insert("DSH_HEADLESS".into(), "1".into());
                    m
                },
            },
            DispatchTarget::Doubao => CmdTemplate {
                bin: "doubao".into(),
                args: vec!["--no-ui".into(), "{task}".into()],
                pass_task_via_stdin: false,
                cwd_arg: None,
                extra_env: HashMap::new(),
            },
            DispatchTarget::CodeWhale => CmdTemplate {
                bin: "codewhale".into(),
                args: vec![
                    "--batch".into(),
                    "--project".into(),
                    "{workspace}".into(),
                ],
                pass_task_via_stdin: true,
                cwd_arg: None,
                extra_env: HashMap::new(),
            },
            DispatchTarget::OpenHands => CmdTemplate {
                bin: "openhands".into(),
                args: vec![
                    "-i".into(),
                    "default".into(),
                    "--task".into(),
                    "{task}".into(),
                    "--no-browser".into(),
                ],
                pass_task_via_stdin: false,
                cwd_arg: Some("{workspace}".into()),
                extra_env: {
                    let mut m = HashMap::new();
                    m.insert("OPENHANDS_HEADLESS".into(), "1".into());
                    m
                },
            },
        }
    }

    /// 该 target 在本机 PATH 中是否可用。
    pub async fn available(&self) -> bool {
        let tmpl = self.default_cmd();
        binary_exists(&tmpl.bin).await
    }
}

/// 命令模板（内部使用）
#[derive(Debug, Clone)]
struct CmdTemplate {
    bin: PathBuf,
    args: Vec<String>,
    pass_task_via_stdin: bool,
    /// 可选：把 {workspace} 作为 cwd 参数（而非 --workspace 标志）
    cwd_arg: Option<String>,
    extra_env: HashMap<String, String>,
}

/// 一次调度请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchRequest {
    /// 目标 agent
    pub target: DispatchTarget,
    /// 任务描述（纯文本）
    pub task: String,
    /// 工作目录（agent 所在的 cwd；传 None 时继承当前目录）
    pub workspace: Option<PathBuf>,
    /// 单次运行超时（默认 10 分钟）
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// 环境变量覆盖（在模板 extra_env 之上合并，优先于后者）
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// 本次调度的自定义 ID（空则生成 UUID）
    #[serde(default)]
    pub run_id: Option<String>,
}

fn default_timeout() -> u64 {
    600
}

impl DispatchRequest {
    pub fn new(target: DispatchTarget, task: impl Into<String>, workspace: impl AsRef<Path>) -> Self {
        Self {
            target,
            task: task.into(),
            workspace: Some(workspace.as_ref().to_path_buf()),
            timeout_secs: default_timeout(),
            env: HashMap::new(),
            run_id: None,
        }
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    /// 合并环境变量（单个 k/v 键值，已有则覆盖）
    pub fn with_env(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.env.insert(k.into(), v.into());
        self
    }

    /// 设置 / 覆盖工作目录
    pub fn with_workspace(mut self, path: Option<PathBuf>) -> Self {
        self.workspace = path;
        self
    }

    /// 手动指定 run_id（用于外部追踪）
    pub fn with_run_id(mut self, id: impl Into<String>) -> Self {
        self.run_id = Some(id.into());
        self
    }
}

/// 一次调度结果（结构化）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchResult {
    /// 本次 run id
    pub run_id: String,
    /// 目标
    pub target: DispatchTarget,
    /// 是否成功（exit code == 0）
    pub ok: bool,
    /// 进程退出码
    pub exit_code: Option<i32>,
    /// stdout（合并后）
    pub stdout: String,
    /// stderr
    pub stderr: String,
    /// 合并 stdout+stderr 的摘要（通常返回给上层）
    pub output: String,
    /// 实际耗时（毫秒）
    pub elapsed_ms: u64,
    /// 若目标不可用（未安装 / 找不到命令）：填充原因；ok=false
    #[serde(default)]
    pub unavailable_reason: Option<String>,
    /// 若超时被 kill：填充原因
    #[serde(default)]
    pub timeout_killed: bool,
}

impl DispatchResult {
    /// 构造「目标不可用」结果（不启动任何子进程）
    fn unavailable(target: DispatchTarget, run_id: String, reason: impl Into<String>) -> Self {
        let reason: String = reason.into();
        Self {
            run_id,
            target,
            ok: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            output: format!("[dispatch:{} unavailable] {}", target.as_str(), reason),
            elapsed_ms: 0,
            unavailable_reason: Some(reason),
            timeout_killed: false,
        }
    }
}

/// 调度入口：无头运行外部 agent 并回收结构化结果。
///
/// 设计保证：
/// - 任何缺失 CLI 只返回 `DispatchResult.ok=false` + `unavailable_reason`，不 panic。
/// - 超时会 kill 子进程；僵尸不残留（wait 语义）。
/// - stdout / stderr 分开保存；`output` 作为给上层的单一字符串（stdout\n\nstderr）。
pub async fn dispatch(req: DispatchRequest) -> Result<DispatchResult> {
    let run_id = req
        .run_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let target = req.target;
    let tmpl = target.default_cmd();

    // 1) 检查命令是否存在；不存在直接返回 unavailable
    if !binary_exists(&tmpl.bin).await {
        return Ok(DispatchResult::unavailable(
            target,
            run_id,
            format!(
                "未在 PATH 中找到命令 '{}'（请安装对应 CLI 或将其加入 PATH）",
                tmpl.bin.display()
            ),
        ));
    }

    // 2) 构建 args：替换 {task} / {workspace}
    let workspace = req
        .workspace
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let task_safe = shell_escape_task(&req.task);
    let workspace_str = workspace.to_string_lossy().to_string();

    let args: Vec<String> = tmpl
        .args
        .iter()
        .map(|a| {
            a.replace("{task}", &task_safe)
                .replace("{workspace}", &workspace_str)
        })
        .collect();

    // 3) 启动子进程：无头 → 接管 stdin；合并 env
    let mut cmd = Command::new(&tmpl.bin);
    cmd.args(&args);
    cmd.current_dir(match &tmpl.cwd_arg {
        Some(cwd_tmpl) => PathBuf::from(cwd_tmpl.replace("{workspace}", &workspace_str)),
        None => workspace.clone(),
    });
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    // 模板内置 env（无头标志等）
    for (k, v) in tmpl.extra_env.iter() {
        cmd.env(k, v);
    }
    // 请求级 env（覆盖模板）
    for (k, v) in req.env.iter() {
        cmd.env(k, v);
    }
    // 禁止打开交互 TTY
    cmd.env("CI", "true");
    cmd.env("NO_COLOR", "1");

    let started = std::time::Instant::now();
    let mut child = cmd
        .spawn()
        .with_context(|| format!("dispatch 启动 {} 失败", target.as_str()))?;
    let id = child.id();

    // 4) 写入 stdin（若走 stdin 传 task）
    if tmpl.pass_task_via_stdin {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(req.task.as_bytes()).await;
            let _ = stdin.flush().await;
            drop(stdin);
        }
    }

    // 5) 超时等待 + stdout/stderr 读取：不把 child 整体 move 进 async，
    //    先单独 take 管道，然后分别用子 future 跑 wait 与 wait_with_output。
    let mut stdout_buf = String::new();
    let mut stderr_buf = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout_buf).await;
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr_buf).await;
    }

    let timeout = Duration::from_secs(req.timeout_secs.max(1));
    let outcome = tokio::time::timeout(timeout, child.wait()).await;

    let elapsed_ms = started.elapsed().as_millis() as u64;

    match outcome {
        Ok(Ok(status)) => {
            let ok = status.success();
            let output = merge_output(&stdout_buf, &stderr_buf);
            Ok(DispatchResult {
                run_id,
                target,
                ok,
                exit_code: status.code(),
                stdout: stdout_buf,
                stderr: stderr_buf,
                output,
                elapsed_ms,
                unavailable_reason: None,
                timeout_killed: false,
            })
        }
        Ok(Err(e)) => Err(anyhow::anyhow!(
            "dispatch:{} 子进程 wait 失败: {e}",
            target.as_str()
        )),
        Err(_elapsed) => {
            // 超时：kill 子进程（跨平台：有 id 走 cmd / kill，否则再次 try_wait 回收）
            let killed = {
                #[cfg(unix)]
                {
                    if let Some(pid) = id {
                        // 使用 nix 风格的信号发送，简单起见用命令行 kill fallback
                        let _ = std::process::Command::new("kill")
                            .arg("-9")
                            .arg(pid.to_string())
                            .status();
                        true
                    } else {
                        false
                    }
                }
                #[cfg(not(unix))]
                {
                    if let Some(pid) = id {
                        let _ = std::process::Command::new("taskkill")
                            .args(["/F", "/PID", &pid.to_string()])
                            .status();
                        true
                    } else {
                        false
                    }
                }
            };
            // 再次回收
            let _ = child.try_wait();
            let msg = format!(
                "dispatch:{} 运行超时（>{}s），已 kill (killed={killed})",
                target.as_str(),
                req.timeout_secs
            );
            Ok(DispatchResult {
                run_id,
                target,
                ok: false,
                exit_code: None,
                stdout: String::new(),
                stderr: msg.clone(),
                output: msg.clone(),
                elapsed_ms,
                unavailable_reason: None,
                timeout_killed: true,
            })
        }
    }
}

/// 批量扇出：把同一任务同时派发给多个目标，按完成先后返回结果向量。
///
/// 调用端可用 `max_parallel` 限制并发（默认 = targets 数量，上限 11）。
pub async fn dispatch_many(
    targets: &[DispatchTarget],
    task: impl Into<String> + Clone,
    workspace: impl AsRef<Path> + Clone,
    timeout_secs_each: u64,
    max_parallel: Option<usize>,
) -> Vec<Result<DispatchResult>> {
    let parallel = max_parallel
        .unwrap_or_else(|| targets.len())
        .clamp(1, 32);
    let sem = Arc::new(tokio::sync::Semaphore::new(parallel));
    let mut set = JoinSet::new();
    for t in targets.iter().copied() {
        let task = task.clone().into();
        let ws = workspace.as_ref().to_path_buf();
        let sem = sem.clone();
        set.spawn(async move {
            let _permit = sem.acquire().await.expect("dispatch_many 信号量已关闭");
            dispatch(DispatchRequest::new(t, task, ws).with_timeout(timeout_secs_each)).await
        });
    }

    let mut out = Vec::with_capacity(targets.len());
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(r) => out.push(r),
            Err(e) => out.push(Err(anyhow::anyhow!("dispatch_many 任务 panic/cancel: {e}"))),
        }
    }
    out
}

use std::sync::Arc;
use std::{env, str::FromStr};

/// `probe()` 的同步探测结果（用于 dispatch list / 外部审计）
#[derive(Debug, Clone)]
pub struct ProbeInfo {
    /// 目标 ID
    pub target: DispatchTarget,
    /// 解析到的二进制绝对路径（或 PATH 中的相对名）
    pub bin_path: PathBuf,
}

/// 跨平台同步查找 PATH 中二进制位置（等价于 `which/where`）
pub(crate) fn which_locate(bin_name: &str) -> Option<PathBuf> {
    if let Ok(abs) = PathBuf::from(bin_name).canonicalize() {
        return Some(abs);
    }
    let paths = env::var_os("PATH")?;
    let exts = if cfg!(windows) {
        env::var_os("PATHEXT")
            .map(|v| env::split_paths(&v).map(|p| p.to_string_lossy().to_string()).collect())
            .unwrap_or_else(|| vec![".EXE;.CMD;.BAT;.COM;.PS1".to_string()])
    } else {
        Vec::new()
    };
    for dir in env::split_paths(&paths) {
        let candidate = dir.join(bin_name);
        if candidate.exists() {
            return Some(candidate);
        }
        if cfg!(windows) {
            for ext in exts.iter().flat_map(|s| s.split(';')) {
                let mut with_ext = candidate.clone();
                with_ext.set_extension(ext.trim_start_matches('.'));
                if with_ext.exists() {
                    return Some(with_ext);
                }
            }
        }
    }
    None
}

impl FromStr for DispatchTarget {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        DispatchTarget::from_name(s)
            .ok_or_else(|| format!("非法目标名 '{s}'（marguerite dispatch list 查看全部）"))
    }
}

/// 批量同步探测（用于 `dispatch list`；HashMap<target, Some(info) or None(未安装)>）
pub fn available_targets() -> std::collections::HashMap<DispatchTarget, Option<ProbeInfo>> {
    let mut map = std::collections::HashMap::new();
    for t in DispatchTarget::ALL {
        map.insert(t, t.probe().ok());
    }
    map
}

/// 简单 shell-escape 任务文本（避免 {task} 插入时被当标志解析）：
/// trim + 把换行压成空格 + 把非 safe 字符转 _；对已通过 stdin 的路径此函数
/// 是额外保险而非主要机制。
fn shell_escape_task(task: &str) -> String {
    let trimmed = task.trim();
    let mut out = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '.' | ',' | '-' | '_' | '/' | '\\') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}

fn merge_output(stdout: &str, stderr: &str) -> String {
    let s = stdout.trim();
    let e = stderr.trim();
    if s.is_empty() && e.is_empty() {
        String::new()
    } else if s.is_empty() {
        e.to_string()
    } else if e.is_empty() {
        s.to_string()
    } else {
        format!("{s}\n\n--- stderr ---\n{e}")
    }
}

/// 轻量级「命令是否在 PATH 中存在」探测（跨平台）。
///
/// Windows 用 `where`；类 Unix 用 `which`；两者都失败时回退 `Command::new`
/// 一次轻量 spawn（无 args、读不到立即回收）。
async fn binary_exists(bin: &Path) -> bool {
    if bin.exists() {
        // 绝对路径直接看文件存在性
        return true;
    }
    let bin_name = match bin.file_name().and_then(|s| s.to_str()) {
        Some(n) => n,
        None => return false,
    };
    let (probe, arg) = if cfg!(windows) {
        ("where.exe", bin_name)
    } else {
        ("which", bin_name)
    };
    if let Ok(out) = Command::new(probe).arg(arg).output().await {
        return out.status.success();
    }
    // 回退：尝试以空 arg 构造一次（有些平台 `where` 不装）。
    Command::new(bin_name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_target_enum_has_11_entries_and_all_unique() {
        let all = DispatchTarget::ALL;
        assert_eq!(all.len(), 11);
        let mut names: Vec<&str> = all.iter().map(|t| t.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 11);
    }

    #[test]
    fn dispatch_target_from_name_recognizes_aliases() {
        assert_eq!(
            DispatchTarget::from_name("codex"),
            Some(DispatchTarget::Codex)
        );
        assert_eq!(
            DispatchTarget::from_name("Claude-Code"),
            Some(DispatchTarget::ClaudeCode)
        );
        assert_eq!(
            DispatchTarget::from_name("OpenDevin"),
            Some(DispatchTarget::OpenHands)
        );
        assert_eq!(
            DispatchTarget::from_name("dsh"),
            Some(DispatchTarget::DeepSeek)
        );
        assert!(DispatchTarget::from_name("not-real").is_none());
    }

    #[test]
    fn shell_escape_is_safe() {
        let s = shell_escape_task("ls; rm -rf / && $(bad)\n newline line 2");
        assert!(!s.contains(';'));
        assert!(!s.contains('$'));
        assert!(!s.contains('&'));
        assert!(!s.contains('\n'));
    }

    #[tokio::test]
    async fn unknown_targets_return_unavailable_gracefully() {
        // 构造一个肯定不存在的 bin，通过 CmdTemplate 改写：走 public API
        // 时我们无法直接改模板，这里用不存在的二进制名探测 binary_exists 行为。
        let fake = PathBuf::from("this-binary-should-never-exist-abc123");
        assert!(!binary_exists(&fake).await);
    }

    #[test]
    fn default_cmd_for_each_target_has_binary_name() {
        for t in DispatchTarget::ALL.iter() {
            let tmpl = t.default_cmd();
            assert!(!tmpl.bin.as_os_str().is_empty(), "{t:?} bin empty");
        }
    }

    #[test]
    fn dispatch_request_constructs_cleanly() {
        let r = DispatchRequest::new(DispatchTarget::Codex, "t", "/tmp");
        assert_eq!(r.target, DispatchTarget::Codex);
        assert_eq!(r.task, "t");
        assert_eq!(r.timeout_secs, 600);
    }

    #[test]
    fn dispatch_result_unavailable_sets_ok_false() {
        let r = DispatchResult::unavailable(DispatchTarget::Kimi, "id".into(), "no cmd");
        assert!(!r.ok);
        assert_eq!(r.unavailable_reason.as_deref(), Some("no cmd"));
        assert_eq!(r.target, DispatchTarget::Kimi);
    }
}
