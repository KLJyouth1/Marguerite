//! 记录型传输：`RecordingTransport`（纯记录调用、不实际执行，可注入失败）。
//!
//! 用于单测与 `--dry-run` 演练。从 `deploy/ssh.rs` 拆分（M0.5，OD-003）。
use std::cell::RefCell;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use super::transport::SshTransport;

/// 记录型传输：纯记录调用、不实际执行，可注入失败（用于单测与 --dry-run）
pub struct RecordingTransport {
    inner: RefCell<RecordingInner>,
    fail_on: Option<String>,
    echo: bool,
}

struct RecordingInner {
    execs: Vec<String>,
    execs_idem: Vec<(String, bool)>,
    uploads: Vec<(PathBuf, String)>,
    downloads: Vec<(String, PathBuf)>,
}

impl Default for RecordingTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingTransport {
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(RecordingInner {
                execs: Vec::new(),
                execs_idem: Vec::new(),
                uploads: Vec::new(),
                downloads: Vec::new(),
            }),
            fail_on: None,
            echo: false,
        }
    }

    /// 配置：远端命令包含该子串时返回 Err（模拟命令失败，典型用于健康探活失败）
    pub fn with_fail(mut self, substring: &str) -> Self {
        self.fail_on = Some(substring.to_string());
        self
    }

    /// 配置：`exec` 返回命令本身作为 stdout（便于批命令逐条对齐测试）；缺省返回 "ok"。
    pub fn with_echo(mut self) -> Self {
        self.echo = true;
        self
    }

    pub fn exec_calls(&self) -> Vec<String> {
        self.inner.borrow().execs.clone()
    }

    /// 经 `exec_idempotent` 执行的命令及其幂等旗标（按执行顺序）。
    pub fn exec_idempotent_calls(&self) -> Vec<(String, bool)> {
        self.inner.borrow().execs_idem.clone()
    }

    pub fn upload_calls(&self) -> Vec<(PathBuf, String)> {
        self.inner.borrow().uploads.clone()
    }

    pub fn download_calls(&self) -> Vec<(String, PathBuf)> {
        self.inner.borrow().downloads.clone()
    }
}

impl SshTransport for RecordingTransport {
    fn exec(&self, cmd: &str) -> Result<String> {
        self.inner.borrow_mut().execs.push(cmd.to_string());
        if let Some(needle) = &self.fail_on {
            if cmd.contains(needle.as_str()) {
                bail!("（mock）远端命令失败: {cmd}");
            }
        }
        Ok(if self.echo {
            cmd.to_string()
        } else {
            "ok".to_string()
        })
    }

    fn exec_idempotent(&self, cmd: &str, idempotent: bool) -> Result<String> {
        self.inner
            .borrow_mut()
            .execs_idem
            .push((cmd.to_string(), idempotent));
        self.exec(cmd)
    }

    fn upload(&self, local: &Path, remote: &Path) -> Result<()> {
        self.inner
            .borrow_mut()
            .uploads
            .push((local.to_path_buf(), remote.to_string_lossy().to_string()));
        Ok(())
    }

    fn download(&self, remote: &Path, local: &Path) -> Result<()> {
        self.inner
            .borrow_mut()
            .downloads
            .push((remote.to_string_lossy().to_string(), local.to_path_buf()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recording_download_records() {
        let t = RecordingTransport::new();
        t.download(Path::new("/remote/log.txt"), Path::new("local.txt"))
            .unwrap();
        assert_eq!(
            t.download_calls(),
            vec![("/remote/log.txt".to_string(), PathBuf::from("local.txt"))]
        );
    }

    #[test]
    fn test_exec_idempotent_records_flag() {
        let t = RecordingTransport::new();
        t.exec_idempotent("echo hi", true).unwrap();
        t.exec_idempotent("rm -rf /tmp/x", false).unwrap();
        let calls = t.exec_idempotent_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], ("echo hi".to_string(), true));
        assert_eq!(calls[1], ("rm -rf /tmp/x".to_string(), false));
    }
}
