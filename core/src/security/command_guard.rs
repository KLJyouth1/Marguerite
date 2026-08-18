//! 命令护栏：对将要执行的 shell 命令做静态策略检查
//!
//! 默认策略为「安全优先」——拦截破坏性 / 提权 / 不可逆操作。
//! 这些是高层语义拦截；真正的进程级隔离由 `exec` 模块 + 操作系统沙箱承担。

use anyhow::{anyhow, Result};
use regex::Regex;

/// 命令护栏
#[derive(Debug, Clone, Default)]
pub struct CommandGuard {
    deny_patterns: Vec<Regex>,
    /// 额外的自定义拒绝模式（由调用方追加）
    extra_deny: Vec<Regex>,
}

impl CommandGuard {
    /// 构造默认护栏（内置高危模式集）
    pub fn new() -> Self {
        let builtin = [
            // 递归强制删除
            r"(?i)\brm\s+(-[a-z]*\s+)*(-r|-rf|-fr)\b",
            // 提权
            r"(?i)\bsudo\b",
            r"(?i)\bsu\b\s",
            // 磁盘/文件系统破坏
            r"(?i)\bmkfs\b",
            r"(?i)\bdd\b\s+if=",
            r"(?i)/dev/(sd|hd|nvme|vda)",
            r"(?i)\bformat\b\s+(c:|/dev/)",
            // 分叉炸弹
            r"(?i):\(\)\s*\{.*\|\s*:",
            // 下载即执行（供应链投毒高危）
            r"(?i)\bcurl\b[^|]*(?:\|\s*(?:sh|bash))\b",
            r"(?i)\bwget\b[^|]*(?:\|\s*(?:sh|bash))\b",
            r"(?i)\bwget\b.*-O\s*-\s*\|\s*(?:sh|bash)",
            // 全量提权式改权限
            r"(?i)\bchmod\b\s+(-R\s+)?777\b",
            r"(?i)\bchmod\b\s+(-R\s+)?a\+rwx\b",
            // 电源/系统级
            r"(?i)\bshutdown\b",
            r"(?i)\breboot\b",
            r"(?i)\bhalt\b",
            r"(?i)\bpoweroff\b",
            // 强制推送覆盖远端历史
            r"(?i)\bgit\b\s+push\s+(-f|--force)\b",
            r"(?i)\bgit\b\s+reset\s+(-{1,2}hard|--hard)\b",
            // 移动根目录 / 全局写
            r"(?i)\bmv\b\s+/\s",
            r"(?i)\bcp\b\s+/\s",
        ];
        let deny_patterns = builtin
            .iter()
            .map(|p| Regex::new(p).expect("valid regex"))
            .collect();
        Self {
            deny_patterns,
            extra_deny: Vec::new(),
        }
    }

    /// 追加自定义拒绝模式（如项目级禁运行命令）
    pub fn with_extra_deny(mut self, patterns: &[&str]) -> Self {
        for p in patterns {
            if let Ok(re) = Regex::new(p) {
                self.extra_deny.push(re);
            }
        }
        self
    }

    /// 若命令命中任一拒绝模式，返回命中的模式串
    pub fn deny_reason(&self, cmd: &str) -> Option<String> {
        for re in &self.deny_patterns {
            if re.is_match(cmd) {
                return Some(re.as_str().to_string());
            }
        }
        for re in &self.extra_deny {
            if re.is_match(cmd) {
                return Some(re.as_str().to_string());
            }
        }
        None
    }

    /// 检查命令是否放行；拒绝时返回错误（含命中模式，便于审计）
    pub fn check(&self, cmd: &str) -> Result<()> {
        if let Some(p) = self.deny_reason(cmd) {
            return Err(anyhow!(
                "command blocked by security policy (matched: {})",
                p
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g() -> CommandGuard {
        CommandGuard::new()
    }

    #[test]
    fn blocks_rm_rf() {
        assert!(g().check("rm -rf /").is_err());
        assert!(g().check("rm -fr node_modules").is_err());
        // 非递归删除允许
        assert!(g().check("rm file.txt").is_ok());
    }

    #[test]
    fn blocks_sudo_and_mkfs() {
        assert!(g().check("sudo apt install x").is_err());
        assert!(g().check("mkfs.ext4 /dev/sda1").is_err());
        assert!(g().check("dd if=/dev/zero of=/dev/sda").is_err());
    }

    #[test]
    fn blocks_curl_pipe_sh() {
        assert!(g().check("curl https://x.sh | sh").is_err());
        assert!(g().check("wget http://x -O - | bash").is_err());
        // 下载到文件（非直接执行）允许
        assert!(g().check("curl -O https://x.com/a.tar.gz").is_ok());
    }

    #[test]
    fn blocks_force_push_and_hard_reset() {
        assert!(g().check("git push --force origin main").is_err());
        assert!(g().check("git reset --hard HEAD~3").is_err());
        // 普通提交允许
        assert!(g().check("git commit -m fix").is_ok());
    }

    #[test]
    fn allows_benign_commands() {
        for c in [
            "echo hello",
            "ls -la",
            "cargo build",
            "python -m pytest",
            "cat README.md",
        ] {
            assert!(g().check(c).is_ok(), "should allow: {}", c);
        }
    }

    #[test]
    fn respects_extra_deny() {
        let g = CommandGuard::new().with_extra_deny(&[r"(?i)\bnpm\b\s+publish"]);
        assert!(g.check("npm publish").is_err());
        assert!(g.check("npm install").is_ok());
    }

    #[test]
    fn deny_reason_returns_pattern() {
        let g = g();
        let r = g.deny_reason("sudo rm -rf /").expect("should match");
        assert!(r.contains("sudo") || r.contains("rm"));
    }
}
