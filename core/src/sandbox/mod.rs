//! 终端沙箱（v2 工作台）
//!
//! 默认拒绝的命令执行层：白名单三分法 → 解释器逃逸拦截 → 路径守卫 →
//! 模式门控 → 资源限制执行 → JSONL 审计。
//! 借鉴 NexusStar sandbox-cli；执行层可注入（便于测试超时/大输出）。

pub mod audit;
pub mod policy;
pub mod policy_store;
pub mod rpc;
pub mod whitelist;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use async_trait::async_trait;
use uuid::Uuid;

use crate::mode::ModeKind;
use whitelist::{basename, classify_full, has_inline_payload, CommandClass};

/// 资源限制
#[derive(Debug, Clone, Copy)]
pub struct ResourceLimit {
    /// 命令超时（秒），默认 60
    pub timeout_sec: u64,
    /// 输出上限（字节），默认 1MB，超限截断
    pub max_output_bytes: usize,
    /// 最大子进程数（防 fork 炸弹，当前为执行层配额占位）
    pub max_children: u32,
}

impl Default for ResourceLimit {
    fn default() -> Self {
        Self {
            timeout_sec: 60,
            max_output_bytes: 1024 * 1024,
            max_children: 8,
        }
    }
}

/// 沙箱执行结果
#[derive(Debug, Clone)]
pub struct SandboxResult {
    /// 决策：allow / deny / path_denied / timeout / error
    pub decision: &'static str,
    /// 标准输出（截断后）
    pub stdout: String,
    /// 标准错误（截断后）
    pub stderr: String,
    /// 退出码（-1 = 未执行/超时/错误）
    pub exit_code: i32,
    /// 耗时（毫秒）
    pub duration_ms: u64,
    /// 输出是否被截断
    pub truncated: bool,
}

impl SandboxResult {
    fn denied(decision: &'static str, reason: impl Into<String>) -> Self {
        Self {
            decision,
            stdout: String::new(),
            stderr: reason.into(),
            exit_code: -1,
            duration_ms: 0,
            truncated: false,
        }
    }
}

/// 命令执行器输出
pub struct RunnerOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: i32,
    pub timed_out: bool,
}

/// 命令执行器 trait（测试可注入 mock）
#[async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(&self, argv: &[String], cwd: &Path, timeout: Duration) -> RunnerOutput;
}

/// 真实进程执行器（tokio::process；超时由沙箱层统一包裹，kill_on_drop 兜底杀进程）
pub struct ProcessRunner;

#[async_trait]
impl CommandRunner for ProcessRunner {
    async fn run(&self, argv: &[String], cwd: &Path, _timeout: Duration) -> RunnerOutput {
        use std::process::Stdio;
        use tokio::process::Command;

        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let Ok(child) = cmd.spawn() else {
            return RunnerOutput {
                stdout: Vec::new(),
                stderr: format!("无法启动进程: {}", argv[0]).into_bytes(),
                exit_code: -1,
                timed_out: false,
            };
        };
        match child.wait_with_output().await {
            Ok(out) => RunnerOutput {
                stdout: out.stdout,
                stderr: out.stderr,
                exit_code: out.status.code().unwrap_or(-1),
                timed_out: false,
            },
            Err(e) => RunnerOutput {
                stdout: Vec::new(),
                stderr: format!("执行错误: {e}").into_bytes(),
                exit_code: -1,
                timed_out: false,
            },
        }
    }
}

/// 终端沙箱
pub struct Sandbox {
    workspace: PathBuf,
    audit: audit::AuditSink,
    limits: ResourceLimit,
    interactive: Arc<AtomicBool>,
    runner: Box<dyn CommandRunner>,
}

impl Sandbox {
    /// 创建沙箱（工作区目录 + 默认资源限制 + 真实进程执行器）
    pub fn new(workspace: &Path) -> Self {
        Self::with_runner(workspace, Box::new(ProcessRunner))
    }

    /// 创建沙箱（注入执行器，测试用）
    pub fn with_runner(workspace: &Path, runner: Box<dyn CommandRunner>) -> Self {
        let audit_dir = audit::default_audit_dir(workspace);
        Self {
            workspace: workspace.to_path_buf(),
            audit: audit::AuditSink::new(audit_dir),
            limits: ResourceLimit::default(),
            interactive: Arc::new(AtomicBool::new(true)),
            runner,
        }
    }

    /// 设置是否交互（agent 模式写命令门控；非交互 fail-closed）
    pub fn set_interactive(&self, interactive: bool) {
        self.interactive.store(interactive, Ordering::SeqCst);
    }

    /// 设置资源限制
    pub fn set_limits(&mut self, limits: ResourceLimit) {
        self.limits = limits;
    }

    /// 当前资源限制
    pub fn limits(&self) -> ResourceLimit {
        self.limits
    }

    /// 工作区路径
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// 执行命令（完整流水线）
    pub async fn execute(
        &self,
        argv: &[String],
        cwd: &Path,
        mode: ModeKind,
    ) -> Result<SandboxResult> {
        let started = Instant::now();

        // 判定层（同步）
        let check = self.check(argv, cwd, mode);
        if let Some(denied) = check {
            self.audit.record(&self.to_record(&denied, argv, cwd));
            return Ok(denied);
        }

        // 资源限制执行（超时统一在沙箱层包裹，超时随 future drop 由执行器 kill）
        let cwd_resolved = resolve_cwd(cwd);
        let timeout = Duration::from_secs(self.limits.timeout_sec);
        let run_fut = self.runner.run(argv, &cwd_resolved, timeout);
        let runner_out = match tokio::time::timeout(timeout, run_fut).await {
            Ok(out) => out,
            Err(_) => RunnerOutput {
                stdout: Vec::new(),
                stderr: Vec::new(),
                exit_code: -1,
                timed_out: true,
            },
        };
        let duration_ms = started.elapsed().as_millis() as u64;

        let (stdout, truncated) = cap(&runner_out.stdout, self.limits.max_output_bytes);
        let (stderr, _) = cap(&runner_out.stderr, self.limits.max_output_bytes);
        let result = SandboxResult {
            decision: if runner_out.timed_out {
                "timeout"
            } else {
                "allow"
            },
            stdout,
            stderr,
            exit_code: runner_out.exit_code,
            duration_ms,
            truncated,
        };
        self.audit.record(&self.to_record(&result, argv, cwd));
        Ok(result)
    }

    /// 判定层：白名单 → 逃逸拦截 → 路径守卫 → 模式门控
    ///
    /// 返回 `Some` 表示命令被拒（含拒绝原因）。
    fn check(&self, argv: &[String], cwd: &Path, mode: ModeKind) -> Option<SandboxResult> {
        if argv.is_empty() {
            return Some(SandboxResult::denied("deny", "空命令"));
        }
        let base = basename(argv.first().map(String::as_str).unwrap_or(""));

        // 1) 白名单三分法 + git 子命令 + 参数复核
        match classify_full(argv) {
            CommandClass::Deny => {
                return Some(SandboxResult::denied("deny", whitelist::deny_reason(argv)));
            }
            CommandClass::ReadOnly => {}
            CommandClass::Write => {
                // 4) 模式门控
                match mode_gate(mode, &self.interactive) {
                    Ok(()) => {}
                    Err(reason) => {
                        return Some(SandboxResult::denied("deny", reason));
                    }
                }
            }
        }

        // 2) 解释器逃逸拦截（python -c / node -e / ...）
        if whitelist::INTERPRETERS.contains(&base) && has_inline_payload(base, &argv[1..]) {
            return Some(SandboxResult::denied(
                "deny",
                format!("解释器 '{base}' 内联执行被拦截"),
            ));
        }

        // 3) 路径守卫（cwd 必须位于工作区子树内）
        if !validate_cwd(cwd, &self.workspace) {
            return Some(SandboxResult::denied(
                "path_denied",
                format!("工作目录 {:?} 在工作区外", cwd),
            ));
        }

        None
    }

    /// 组装审计记录
    fn to_record(&self, result: &SandboxResult, argv: &[String], cwd: &Path) -> audit::AuditRecord {
        audit::AuditRecord {
            ts: chrono::Local::now().to_rfc3339(),
            trace_id: Uuid::new_v4().to_string(),
            decision: result.decision.to_string(),
            command: argv.first().cloned().unwrap_or_default(),
            args: argv.to_vec(),
            cwd: cwd.display().to_string(),
            exit_code: result.exit_code,
            duration_ms: result.duration_ms,
            stdout_bytes: result.stdout.len(),
            stderr_bytes: result.stderr.len(),
        }
    }
}

/// 模式门控：Write 类命令按模式判定
fn mode_gate(mode: ModeKind, interactive: &AtomicBool) -> Result<(), String> {
    match mode {
        ModeKind::Plan => Err("plan 模式禁止写命令".to_string()),
        ModeKind::Bloom => Ok(()),
        ModeKind::Agent => {
            if interactive.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err("写命令需审批（非交互默认拒绝）".to_string())
            }
        }
    }
}

/// 路径守卫：cwd 规范化后必须是工作区的子路径（组件级前缀匹配）
fn validate_cwd(cwd: &Path, workspace: &Path) -> bool {
    let cwd_c = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let ws_c = std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    cwd_c.starts_with(&ws_c)
}

/// 解析 cwd：canonicalize 失败则退回原值（判定层已保证在工作区内）
fn resolve_cwd(cwd: &Path) -> PathBuf {
    std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf())
}

/// 输出截断（上限 max 字节，超限标记 truncated）
fn cap(bytes: &[u8], max: usize) -> (String, bool) {
    if bytes.len() <= max {
        (String::from_utf8_lossy(bytes).into_owned(), false)
    } else {
        (String::from_utf8_lossy(&bytes[..max]).into_owned(), true)
    }
}

#[cfg(test)]
mod penetration_tests;
