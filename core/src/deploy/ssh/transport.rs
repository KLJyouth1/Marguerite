//! SSH 传输层：`SshTransport` 抽象、真实 `SystemSshTransport`（系统 ssh/scp）与批命令执行。
//!
//! 从 `deploy/ssh.rs` 拆分（M0.5，OD-003）。真实传输**密钥免密**
//! （`BatchMode=yes` 禁止密码交互，依赖 `~/.ssh` agent 或 identity 文件）。
use std::os::windows::process::CommandExt;
use std::path::Path;

use anyhow::{anyhow, bail, Result};

use super::target::SshTarget;

/// SSH 传输抽象（exec/upload），便于注入 mock / dry-run
pub trait SshTransport {
    /// 执行远端命令，远端非零退出视为失败（返回 Err）
    fn exec(&self, cmd: &str) -> Result<String>;
    /// 带幂等标记执行命令；缺省忽略旗标并委托 `exec`（向后兼容）。
    ///
    /// `idempotent = false` 表示该命令非幂等（写/启停/删除类），断线重连可能被重放，
    /// 实现方应记录警告标记（对齐 dsh-ssh「断线重连可能重放非幂等命令」审计语义）。
    fn exec_idempotent(&self, cmd: &str, idempotent: bool) -> Result<String> {
        let _ = idempotent;
        self.exec(cmd)
    }
    /// 上传本地文件到远端路径
    fn upload(&self, local: &Path, remote: &Path) -> Result<()>;
    /// 下载远端文件到本地路径
    fn download(&self, remote: &Path, local: &Path) -> Result<()>;
}

/// 退出码标签（保真：正常退出给出数字码，被信号终止时明确标注无退出码）。
fn exit_code_label(code: Option<i32>) -> String {
    match code {
        Some(c) => format!("退出码 {c}"),
        None => "被信号终止（无退出码）".to_string(),
    }
}

/// 真实传输：经系统 `ssh`/`scp`，密钥免密（BatchMode 禁止密码交互）
pub struct SystemSshTransport {
    target: SshTarget,
}

impl SystemSshTransport {
    pub fn new(target: &SshTarget) -> Self {
        Self {
            target: target.clone(),
        }
    }
}

impl SshTransport for SystemSshTransport {
    fn exec(&self, cmd: &str) -> Result<String> {
        let mut c = std::process::Command::new("ssh");
        c.creation_flags(0x08000000); // CREATE_NO_WINDOW：部署期不闪控制台
        c.arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("StrictHostKeyChecking=accept-new");
        if let Some(id) = &self.target.identity {
            c.arg("-i").arg(id);
        }
        c.arg("-p").arg(self.target.port.to_string());
        c.arg(format!("{}@{}", self.target.user, self.target.host));
        c.arg(cmd);
        let out = c
            .output()
            .map_err(|e| anyhow!("ssh 执行失败（请确认 ssh 客户端可用且已配置密钥）: {e}"))?;
        if !out.status.success() {
            bail!(
                "ssh 远端命令失败（{}）: {}",
                exit_code_label(out.status.code()),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    fn exec_idempotent(&self, cmd: &str, idempotent: bool) -> Result<String> {
        if let Some(w) = super::policy::idempotency_warning(cmd, idempotent) {
            tracing::warn!(command = %cmd, warning = %w, "非幂等命令执行告警");
        }
        self.exec(cmd)
    }

    fn upload(&self, local: &Path, remote: &Path) -> Result<()> {
        let mut c = std::process::Command::new("scp");
        c.creation_flags(0x08000000); // CREATE_NO_WINDOW：部署期不闪控制台
        c.arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("StrictHostKeyChecking=accept-new");
        if let Some(id) = &self.target.identity {
            c.arg("-i").arg(id);
        }
        c.arg("-P").arg(self.target.port.to_string());
        c.arg(local);
        c.arg(format!(
            "{}@{}:{}",
            self.target.user,
            self.target.host,
            remote.display()
        ));
        let out = c
            .output()
            .map_err(|e| anyhow!("scp 执行失败（请确认 scp 客户端可用且已配置密钥）: {e}"))?;
        if !out.status.success() {
            bail!(
                "scp 上传失败（{}）: {}",
                exit_code_label(out.status.code()),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }

    fn download(&self, remote: &Path, local: &Path) -> Result<()> {
        let mut c = std::process::Command::new("scp");
        c.creation_flags(0x08000000); // CREATE_NO_WINDOW：部署期不闪控制台
        c.arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("StrictHostKeyChecking=accept-new");
        if let Some(id) = &self.target.identity {
            c.arg("-i").arg(id);
        }
        c.arg("-P").arg(self.target.port.to_string());
        c.arg(format!(
            "{}@{}:{}",
            self.target.user,
            self.target.host,
            remote.display()
        ));
        c.arg(local);
        let out = c
            .output()
            .map_err(|e| anyhow!("scp 执行失败（请确认 scp 客户端可用且已配置密钥）: {e}"))?;
        if !out.status.success() {
            bail!(
                "scp 下载失败（{}）: {}",
                exit_code_label(out.status.code()),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }
}

/// 批量顺序执行（单连接、逐条保真收集、任一失败不吞并继续执行后续命令）。
///
/// `alias` 为连接别名，仅用于错误上下文标注（对齐 dsh-ssh 主机别名语义）。
/// 返回与 `commands` 一一对应的结果：每条独立 `Ok(stdout)` 或 `Err(带别名前缀的错误信息)`。
pub fn exec_batch(
    transport: &dyn SshTransport,
    alias: &str,
    commands: &[&str],
) -> Vec<Result<String, String>> {
    commands
        .iter()
        .map(|cmd| transport.exec(cmd).map_err(|e| format!("[{alias}] {e}")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::recording::RecordingTransport;
    use super::*;

    #[test]
    fn test_exec_batch_aligned_per_item() {
        let t = RecordingTransport::new().with_echo();
        let cmds = ["id", "uname -a", "pwd"];
        let results = exec_batch(&t, "web", &cmds);
        assert_eq!(results.len(), 3, "结果数须与命令数一一对应");
        assert_eq!(results[0].as_ref().unwrap(), "id");
        assert_eq!(results[1].as_ref().unwrap(), "uname -a");
        assert_eq!(results[2].as_ref().unwrap(), "pwd");
    }

    #[test]
    fn test_exec_batch_failure_not_swallowed() {
        let t = RecordingTransport::new().with_fail("bad");
        let cmds = ["ok1", "bad-cmd", "ok2"];
        let results = exec_batch(&t, "db", &cmds);
        assert_eq!(results.len(), 3);
        assert!(results[0].is_ok());
        assert!(results[1].is_err(), "失败命令应原样暴露，不吞");
        assert!(results[2].is_ok(), "任一失败不应阻断后续命令");
        let err = results[1].as_ref().unwrap_err();
        assert!(err.contains("[db]"), "错误信息应带别名前缀: {err}");
        // 逐条保真收集：三条均被执行且顺序一致
        assert_eq!(
            t.exec_calls(),
            vec!["ok1".to_string(), "bad-cmd".to_string(), "ok2".to_string()]
        );
    }

    #[test]
    fn test_exec_batch_empty() {
        let t = RecordingTransport::new();
        assert!(exec_batch(&t, "x", &[]).is_empty());
    }

    #[test]
    fn test_exit_code_label_fidelity() {
        assert_eq!(exit_code_label(Some(0)), "退出码 0");
        assert_eq!(exit_code_label(Some(127)), "退出码 127");
        assert!(
            exit_code_label(None).contains("信号"),
            "被信号终止应明确标注无退出码"
        );
    }
}
