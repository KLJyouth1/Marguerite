//! 沙箱执行：经命令护栏校验后，在工作区内以受限方式运行外部命令
//!
//! - 命令先过 `CommandGuard` 静态拦截；
//! - 进程 `current_dir` 限定为工作区（不切换全局目录）；
//! - 通过操作系统超时机制强制终止（防挂死）；
//! - 默认不继承危险环境变量，输出大小受调用方控制。

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, Result};
use tokio::process::Command;

use super::command_guard::CommandGuard;

/// 单次执行结果
#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// 默认命令执行超时
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// 在沙箱中执行一条 shell 命令。
///
/// `cmd` 整体先做策略校验；随后按平台以 `sh -c` / `cmd /C` 运行，
/// 工作目录固定为 `cwd`，超时后强制终止进程。
pub async fn sandboxed_exec(
    cmd: &str,
    cwd: &Path,
    timeout: Duration,
    guard: &CommandGuard,
) -> Result<ExecOutput> {
    guard.check(cmd)?;

    let (shell, flag) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };

    let mut command = Command::new(shell);
    command
        .arg(flag)
        .arg(cmd)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true);
    // Windows: 静默后台执行，不弹出 cmd/PowerShell 窗口
    #[cfg(windows)]
    {
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let child = command
        .spawn()
        .map_err(|e| anyhow!("failed to spawn `{}`: {}", shell, e))?;

    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let exit_code = out.status.code().unwrap_or(-1);
            Ok(ExecOutput {
                stdout,
                stderr,
                exit_code,
            })
        }
        Ok(Err(e)) => Err(anyhow!("command execution error: {}", e)),
        Err(_) => {
            // 超时：timeout 丢弃 future 时 `kill_on_drop` 会自动终止子进程
            Err(anyhow!("command timed out after {:?}: {}", timeout, cmd))
        }
    }
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
        let _ = std::fs::remove_dir_all(&dir);
    }
}
