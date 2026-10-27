//! 沙箱执行：经命令护栏校验后，在工作区内以受限方式运行外部命令
//!
//! - 命令先过 `CommandGuard` 静态拦截；
//! - 进程 `current_dir` 限定为工作区（不切换全局目录）；
//! - 通过操作系统超时机制强制终止（防挂死），Windows 上**连同子进程树**终止；
//! - 输出有上限（默认 256KB），超出部分继续排空但不再累积，并标记 `truncated`；
//! - 可选 shell：`cmd` / `powershell` / `pwsh` / `bash` / `sh` / 平台默认。
//!
//! # 为什么补这三件事
//!
//! 旧实现只能 `cmd /C` 一条路、输出无上限、超时只 `kill_on_drop` 杀**直接子进程**。
//! 真实后果：`npm run dev` 超时后 `node` 仍在后台占端口；一次 `dir /s C:\` 的
//! 输出能把上下文与内存一起打爆；用户在 PowerShell 项目里写的命令语法
//! （`$env:FOO` / `Get-ChildItem`）在 `cmd` 下根本无法执行。

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use super::command_guard::CommandGuard;

/// 单次执行结果
#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    /// 输出被上限截断（stdout 或 stderr）
    pub truncated: bool,
    pub duration_ms: u64,
}

/// 默认命令执行超时
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// 默认输出上限（stdout / stderr 各自）
pub const DEFAULT_MAX_OUTPUT: usize = 256 * 1024;

/// 执行所用 shell。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecShell {
    /// 平台默认（Windows `cmd /C`，Unix `sh -c`）
    Default,
    Cmd,
    PowerShell,
    Pwsh,
    Bash,
    Sh,
}

impl ExecShell {
    /// 解析 shell id；`None` 表示未知 id（由调用方报错，不静默回落）。
    pub fn from_id(raw: &str) -> Option<Self> {
        Some(match raw.trim().to_ascii_lowercase().as_str() {
            "" | "default" | "auto" => ExecShell::Default,
            "cmd" | "cmd.exe" => ExecShell::Cmd,
            "powershell" | "windows-powershell" | "ps" | "ps5" => ExecShell::PowerShell,
            "pwsh" | "ps7" | "powershell7" => ExecShell::Pwsh,
            "bash" | "git-bash" | "gitbash" => ExecShell::Bash,
            "sh" | "posix" => ExecShell::Sh,
            _ => return None,
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            ExecShell::Default => "default",
            ExecShell::Cmd => "cmd",
            ExecShell::PowerShell => "powershell",
            ExecShell::Pwsh => "pwsh",
            ExecShell::Bash => "bash",
            ExecShell::Sh => "sh",
        }
    }
}

/// 一次执行的完整请求（超时 / shell / 输出上限都可控）。
#[derive(Debug, Clone)]
pub struct ExecRequest<'a> {
    pub cmd: &'a str,
    pub cwd: &'a Path,
    pub timeout: Duration,
    pub shell: ExecShell,
    pub max_output_bytes: usize,
}

impl<'a> ExecRequest<'a> {
    pub fn new(cmd: &'a str, cwd: &'a Path) -> Self {
        Self {
            cmd,
            cwd,
            timeout: DEFAULT_TIMEOUT,
            shell: ExecShell::Default,
            max_output_bytes: DEFAULT_MAX_OUTPUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_shell(mut self, shell: ExecShell) -> Self {
        self.shell = shell;
        self
    }

    pub fn with_max_output(mut self, bytes: usize) -> Self {
        self.max_output_bytes = bytes.max(1024);
        self
    }
}

/// 在沙箱中执行一条 shell 命令（兼容入口，使用平台默认 shell）。
pub async fn sandboxed_exec(
    cmd: &str,
    cwd: &Path,
    timeout: Duration,
    guard: &CommandGuard,
) -> Result<ExecOutput> {
    exec_with(&ExecRequest::new(cmd, cwd).with_timeout(timeout), guard).await
}

/// 按请求执行：护栏 → 构造 shell → 有界读取 → 超时树终止。
pub async fn exec_with(req: &ExecRequest<'_>, guard: &CommandGuard) -> Result<ExecOutput> {
    guard.check(req.cmd)?;

    let mut command = build_shell_command(req)?;
    command
        .current_dir(req.cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = command
        .spawn()
        .map_err(|e| anyhow!("failed to spawn `{}`: {}", req.shell.id(), e))?;
    let pid = child.id();

    let out_task = tokio::spawn(read_capped(child.stdout.take(), req.max_output_bytes));
    let err_task = tokio::spawn(read_capped(child.stderr.take(), req.max_output_bytes));
    let started = Instant::now();

    match tokio::time::timeout(req.timeout, child.wait()).await {
        Ok(Ok(status)) => {
            let (stdout, out_trunc) = out_task.await.unwrap_or_default();
            let (stderr, err_trunc) = err_task.await.unwrap_or_default();
            Ok(ExecOutput {
                stdout: String::from_utf8_lossy(&stdout).to_string(),
                stderr: String::from_utf8_lossy(&stderr).to_string(),
                exit_code: status.code().unwrap_or(-1),
                truncated: out_trunc || err_trunc,
                duration_ms: started.elapsed().as_millis() as u64,
            })
        }
        Ok(Err(e)) => Err(anyhow!("command execution error: {}", e)),
        Err(_) => {
            // 超时：直接子进程 + 子进程树一起终止（Windows 用 taskkill /T）。
            kill_process_tree(pid).await;
            // 收尸：避免 zombie，并给管道一个短暂窗口把已产生的输出排空。
            let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
            let _ = tokio::time::timeout(Duration::from_millis(500), out_task).await;
            let _ = tokio::time::timeout(Duration::from_millis(500), err_task).await;
            Err(anyhow!(
                "command timed out after {:?} (process tree terminated): {}",
                req.timeout,
                req.cmd
            ))
        }
    }
}

/// 识别「单条 powershell / pwsh 调用」并按 argv 直接构造命令；否则返回 `None`。
///
/// 为什么只做这一种形态：`cmd /C "<整串>"` 会把 Rust 传给它的 `\"` 原样交给子程序，
/// 于是 `-File "…"` / `-Command "…"` 里的引号变成字面字符。argv 直连没有这层再解析，
/// 与 `tools/computer_use.rs` / `tools/builtin/win_auto.rs` 的正确范式一致。
/// 含真实 shell 操作符（`&&`、`||`、`|`、重定向）的命令返回 `None`，仍交给 cmd 处理。
#[cfg(windows)]
fn direct_powershell_call(cmd: &str) -> Option<Command> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return None;
    }
    let argv = split_windows_argv(trimmed);
    let exe = argv.first()?.to_ascii_lowercase();
    let is_ps = matches!(
        exe.as_str(),
        "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe"
    );
    if !is_ps {
        return None;
    }
    // 操作符必须按「独立 token」判断：引号内的 `|` 属于脚本内容，不是管道。
    if argv.iter().any(|t| {
        matches!(t.as_str(), "&&" | "||" | "|" | ">" | ">>" | "<" | ";")
            || t.starts_with('>')
            || t.starts_with('<')
    }) {
        return None;
    }
    let has_script_arg = argv.iter().any(|t| {
        let low = t.to_ascii_lowercase();
        matches!(low.as_str(), "-file" | "-f" | "-command" | "-c" | "-encodedcommand")
    });
    if !has_script_arg {
        return None;
    }
    let mut c = Command::new(&argv[0]);
    c.args(&argv[1..]);
    Some(c)
}

/// 极简 Windows 命令行分词（仅供 [`direct_powershell_call`] 识别直连程序调用）。
///
/// 支持双引号包裹、`\"` 转义与空参数（`""`）；不处理 cmd 的 `^` 转义（那种命令
/// 本来就会命中外面的操作符判断、继续走 cmd）。
#[cfg(windows)]
fn split_windows_argv(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut started = false;
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                started = true;
            }
            '\\' if in_quotes && chars.peek() == Some(&'"') => {
                chars.next();
                cur.push('"');
                started = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if started || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// 按请求构造 `Command`：不同 shell 的参数语义完全不同，必须分别拼。
fn build_shell_command(req: &ExecRequest<'_>) -> Result<Command> {
    let cmd = req.cmd;
    // Windows 专属：`powershell/pwsh -File "路径"` 这类**单条程序调用**必须 argv 直连。
    //
    // 事故（真机截图）：模型生成 `powershell -ExecutionPolicy Bypass -File "C:\...\x.ps1"`
    // 交给 `cmd /C`，Rust 会把内部双引号转义成 `\"`，而 cmd 不认这种转义 →
    // PowerShell 收到带字面引号的路径 → `Illegal characters in path`，脚本根本没跑。
    // 仓库里 `tools/computer_use.rs:59-66` 与 `tools/builtin/win_auto.rs` 早已记录同一坑，
    // 但通用 shell 路径漏了这条。这里补上：只拦「单个 powershell 调用、无 shell 操作符」
    // 的形态，复合命令（&&、||、|、重定向）仍走 cmd，语义不变。
    #[cfg(windows)]
    if matches!(req.shell, ExecShell::Cmd | ExecShell::Default) {
        if let Some(c) = direct_powershell_call(cmd) {
            return Ok(c);
        }
    }
    // `mut` 只在 Unix 分支（process_group）需要；Windows 上没有可变调用。
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut command = match req.shell {
        ExecShell::Default => {
            if cfg!(windows) {
                let mut c = Command::new("cmd");
                // 先切 UTF-8 代码页，避免中文路径/输出乱码
                c.arg("/C").arg(format!("chcp 65001 >nul && {}", cmd));
                c
            } else {
                let mut c = Command::new("sh");
                c.arg("-c").arg(cmd);
                c
            }
        }
        ExecShell::Cmd => {
            if !cfg!(windows) {
                return Err(anyhow!("cmd 仅在 Windows 上可用"));
            }
            let mut c = Command::new("cmd");
            c.arg("/C").arg(format!("chcp 65001 >nul && {}", cmd));
            c
        }
        ExecShell::PowerShell => {
            if !cfg!(windows) {
                return Err(anyhow!("Windows PowerShell 仅在 Windows 上可用"));
            }
            let mut c = Command::new("powershell");
            c.args(["-NoLogo", "-NoProfile", "-Command"]).arg(cmd);
            c
        }
        ExecShell::Pwsh => {
            let mut c = Command::new("pwsh");
            c.args(["-NoLogo", "-NoProfile", "-Command"]).arg(cmd);
            c
        }
        ExecShell::Bash => {
            // `-lc` 让 .profile / .bashrc 里的 PATH 生效（nvm、conda 等）
            let mut c = Command::new("bash");
            c.args(["-lc", cmd]);
            c
        }
        ExecShell::Sh => {
            let mut c = Command::new("sh");
            c.arg("-c").arg(cmd);
            c
        }
    };
    #[cfg(unix)]
    {
        // 独立进程组：shell 被终止时不会把宿主一起带走（超时按组发信号）。
        // tokio 的 Command 在 Unix 上自带 `process_group`（无需 std 的 CommandExt）。
        command.process_group(0);
    }
    Ok(command)
}

/// 有界读取：超过 `cap` 后继续把管道读空（否则子进程会阻塞在写端），但不再累积。
async fn read_capped<R>(reader: Option<R>, cap: usize) -> (Vec<u8>, bool)
where
    R: AsyncRead + Unpin,
{
    let Some(mut reader) = reader else {
        return (Vec::new(), false);
    };
    let mut out: Vec<u8> = Vec::new();
    let mut truncated = false;
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                let space = cap.saturating_sub(out.len());
                if space > 0 {
                    out.extend_from_slice(&buf[..n.min(space)]);
                }
                if n > space {
                    truncated = true;
                }
            }
            Err(_) => break,
        }
    }
    (out, truncated)
}

/// 终止进程树。
///
/// Windows 用 `taskkill /T /F`：`kill_on_drop` 只杀直接子进程，`cmd /C npm run dev`
/// 的 `node` 孙进程会继续占端口。Unix 走进程组信号（找不到组时退回 `kill`）。
async fn kill_process_tree(pid: Option<u32>) {
    #[cfg(windows)]
    {
        if let Some(pid) = pid {
            let _ = tokio::task::spawn_blocking(move || {
                use crate::proc::SilentCommand;
                let mut c = std::process::Command::new("taskkill");
                c.silent()
                    .args(["/F", "/T", "/PID", &pid.to_string()])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
            })
            .await;
        }
    }
    #[cfg(unix)]
    {
        if let Some(pid) = pid {
            // 进程组组长（`process_group(0)`）→ 组内全部进程。
            let _ = tokio::task::spawn_blocking(move || unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            })
            .await;
        }
    }
    let _ = pid;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn ws() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("marguerite_exec_{}", uuid::Uuid::new_v4()))
    }

    #[tokio::test]
    async fn runs_benign_command() {
        let dir = ws();
        std::fs::create_dir_all(&dir).unwrap();
        let guard = Arc::new(CommandGuard::new());
        let out = sandboxed_exec("echo hello", &dir, DEFAULT_TIMEOUT, &guard)
            .await
            .unwrap();
        assert!(out.stdout.contains("hello"));
        assert_eq!(out.exit_code, 0);
        assert!(!out.truncated);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `powershell -File "带 空格 的路径"` 必须 argv 直连，不能经 `cmd /C`。
    ///
    /// 真机事故：经 cmd 时 Rust 的 `\"` 原样到达 PowerShell → `Illegal characters in path`，
    /// 脚本根本没执行（用户看到 13 分钟全是 `[exit -1]`）。
    #[cfg(windows)]
    #[tokio::test]
    async fn powershell_file_invocation_survives_quoting() {
        let dir = ws();
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("probe script.ps1"); // 故意带空格
        std::fs::write(&script, "Write-Output 'argv-ok'\r\n").unwrap();
        let cmd = format!(
            "powershell -NoProfile -ExecutionPolicy Bypass -File \"{}\"",
            script.display()
        );
        // 形态识别：必须走直连分支
        assert!(direct_powershell_call(&cmd).is_some());
        let guard = Arc::new(CommandGuard::new());
        let out = sandboxed_exec(&cmd, &dir, DEFAULT_TIMEOUT, &guard)
            .await
            .unwrap();
        assert!(
            out.stdout.contains("argv-ok"),
            "stdout={:?} stderr={:?}",
            out.stdout,
            out.stderr
        );
        assert!(
            !out.stderr.contains("Illegal characters"),
            "引号仍被 cmd 破坏: {}",
            out.stderr
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 复合命令与普通命令仍交给 cmd（不得被直连分支截胡）。
    #[cfg(windows)]
    #[test]
    fn compound_commands_stay_on_cmd() {
        assert!(direct_powershell_call("powershell -File \"C:\\x y\\a.ps1\" && echo done").is_none());
        assert!(direct_powershell_call("echo hi").is_none());
        assert!(direct_powershell_call("dir > out.txt").is_none());
        // 引号内的管道属于脚本内容，不是 shell 管道
        assert!(direct_powershell_call("powershell -c \"Get-Process | Select -First 2\"").is_some());
        assert_eq!(
            split_windows_argv("powershell -File \"C:\\a b\\x.ps1\""),
            vec!["powershell", "-File", "C:\\a b\\x.ps1"]
        );
    }

    #[tokio::test]
    async fn respects_requested_cwd() {
        let dir = ws();
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let guard = Arc::new(CommandGuard::new());
        let cmd = if cfg!(windows) { "cd" } else { "pwd" };
        let out = exec_with(&ExecRequest::new(cmd, &sub), &guard)
            .await
            .unwrap();
        assert!(
            out.stdout
                .to_lowercase()
                .contains(&sub.to_string_lossy().to_lowercase()),
            "cwd 应生效，实际: {:?}",
            out.stdout
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn truncates_oversized_output_but_keeps_exit_code() {
        let dir = ws();
        std::fs::create_dir_all(&dir).unwrap();
        let guard = Arc::new(CommandGuard::new());
        let cmd = if cfg!(windows) {
            "for /L %i in (1,1,2000) do @echo 0123456789012345678901234567890123456789"
        } else {
            "yes 0123456789012345678901234567890123456789 | head -n 2000"
        };
        let out = exec_with(
            &ExecRequest::new(cmd, &dir).with_max_output(4096),
            &guard,
        )
        .await
        .unwrap();
        assert!(out.truncated, "超限必须标记 truncated");
        assert!(out.stdout.len() <= 4096, "stdout 不得超过上限");
        assert_eq!(out.exit_code, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn blocks_dangerous_command() {
        let dir = ws();
        std::fs::create_dir_all(&dir).unwrap();
        let guard = Arc::new(CommandGuard::new());
        let res = sandboxed_exec("sudo rm -rf /", &dir, DEFAULT_TIMEOUT, &guard).await;
        assert!(res.is_err(), "dangerous command must be blocked pre-spawn");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn times_out_long_running() {
        let dir = ws();
        std::fs::create_dir_all(&dir).unwrap();
        let guard = Arc::new(CommandGuard::new());
        // 用真正会阻塞的命令（5s）配合 300ms 超时，验证强制终止
        let block = if cfg!(windows) {
            "ping 127.0.0.1 -n 5 >nul"
        } else {
            "sleep 5"
        };
        let res = sandboxed_exec(block, &dir, Duration::from_millis(300), &guard).await;
        assert!(res.is_err(), "should time out");
        let msg = res.unwrap_err().to_string();
        assert!(msg.contains("timed out"), "错误应说明超时: {msg}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shell_ids_parse_strictly() {
        assert_eq!(ExecShell::from_id("pwsh"), Some(ExecShell::Pwsh));
        assert_eq!(ExecShell::from_id("PowerShell"), Some(ExecShell::PowerShell));
        assert_eq!(ExecShell::from_id(""), Some(ExecShell::Default));
        assert_eq!(ExecShell::from_id("klingon"), None);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn powershell_shell_executes_cmdlet() {
        let dir = ws();
        std::fs::create_dir_all(&dir).unwrap();
        let guard = Arc::new(CommandGuard::new());
        let out = exec_with(
            &ExecRequest::new("Write-Output marguerite-powershell", &dir)
                .with_shell(ExecShell::PowerShell),
            &guard,
        )
        .await
        .unwrap();
        assert!(out.stdout.contains("marguerite-powershell"), "实际: {:?}", out.stdout);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
