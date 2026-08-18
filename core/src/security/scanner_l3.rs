//! L3 分层安全扫描：操作护栏 / 权限边界层
//!
//! 对标 `capabilities.rs` 中的 `sec.guardrail`（操作护栏，Enhanced 能力）。
//! L1（command_guard/ssrf_guard/exec 沙箱）是"命令级语义拦截"，本层在此之上
//! 做"权限边界"裁决——即便命令本身合法，若其目标越界（写系统目录、跨工作区、
//! 覆写系统文件）或含沙箱逃逸意图，仍须 `requires_approval` 甚至 `deny`。
//!
//! 设计原则：
//! - 纯静态、零外部依赖、不调 LLM，可在无 API Key 环境运行。
//! - 接收结构化的操作意图（操作类型 + 目标路径 + 当前工作区），返回裁决。
//! - 默认拒绝越界，需显式授权才放行——与 L1 "默认拒绝高危" 一脉相承。

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// 操作种类
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpKind {
    /// 写文件
    WriteFile,
    /// 删除
    Delete,
    /// 执行命令
    Exec,
    /// 读取
    Read,
    /// 移动/重命名
    Move,
}

impl OpKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            OpKind::WriteFile => "write_file",
            OpKind::Delete => "delete",
            OpKind::Exec => "exec",
            OpKind::Read => "read",
            OpKind::Move => "move",
        }
    }
}

/// 裁决严重度
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GuardrailSeverity {
    Info = 0,
    Warn = 1,
    Deny = 2,
}

/// L3 裁决结果
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardrailVerdict {
    /// 是否直接允许（无需审批）
    pub allowed: bool,
    /// 是否需要人工/显式审批后才放行
    pub requires_approval: bool,
    /// 裁决严重度
    pub severity: GuardrailSeverity,
    /// 理由（便于审计）
    pub reason: Option<String>,
}

impl GuardrailVerdict {
    pub fn allow() -> Self {
        Self {
            allowed: true,
            requires_approval: false,
            severity: GuardrailSeverity::Info,
            reason: None,
        }
    }

    pub fn approve(reason: &str) -> Self {
        Self {
            allowed: false,
            requires_approval: true,
            severity: GuardrailSeverity::Warn,
            reason: Some(reason.to_string()),
        }
    }

    pub fn deny(reason: &str) -> Self {
        Self {
            allowed: false,
            requires_approval: false,
            severity: GuardrailSeverity::Deny,
            reason: Some(reason.to_string()),
        }
    }
}

/// 受保护（禁止越界写/删）的系统目录前缀（绝对路径，用 `Path::starts_with` 判定）
const SYSTEM_PREFIXES: &[&str] = &[
    "/etc",
    "/usr",
    "/System",
    "/Library",
    "/bin",
    "/sbin",
    "/boot",
    "/sys",
    "/proc",
    "C:\\Windows",
    "C:\\Program Files",
    "C:\\ProgramData",
];

/// 直接拒绝的凭证目录（按路径组件名判定，兼容绝对/相对路径）
const CREDENTIAL_DIRS: &[&str] = &[".ssh", ".aws", ".gnupg"];

/// L3 操作护栏扫描器
#[derive(Debug, Clone, Default)]
pub struct GuardrailScanner {
    system_prefixes: Vec<PathBuf>,
}

impl GuardrailScanner {
    /// 构造默认护栏（内置系统保护目录 + 逃逸意图模式）
    pub fn new() -> Self {
        let system_prefixes: Vec<PathBuf> = SYSTEM_PREFIXES.iter().map(PathBuf::from).collect();
        Self { system_prefixes }
    }

    /// 检查一次操作是否满足权限边界
    ///
    /// - `op`：操作类型
    /// - `target`：操作目标路径（相对或绝对）
    /// - `cwd`：当前授权工作区根（操作目标须落在其内或明确授权）
    pub fn check(&self, op: OpKind, target: &Path, cwd: &Path) -> GuardrailVerdict {
        // 1) 沙箱逃逸意图（仅 Exec 类操作检查命令/参数）
        if op == OpKind::Exec {
            let t = target.to_string_lossy().to_lowercase();
            for pat in ESCAPE_PATTERNS {
                // 按「词边界」匹配而非裸子串：否则 `mount` 会误伤
                // `git commit -m "mountain view"` / `paramount` 等良性命令。
                if contains_token(&t, pat) {
                    return GuardrailVerdict::deny(&format!(
                        "sandbox-escape intent detected: `{}`",
                        pat
                    ));
                }
            }
        }

        let writing = matches!(op, OpKind::WriteFile | OpKind::Delete | OpKind::Move);

        // 2) 凭证目录（按组件名）：.ssh/.aws/.gnupg 直接拒绝；.git 需审批
        if writing {
            for comp in target.components() {
                let name = comp.as_os_str().to_string_lossy();
                if CREDENTIAL_DIRS.contains(&name.as_ref()) {
                    return GuardrailVerdict::deny(&format!(
                        "target under credential dir: {}",
                        name
                    ));
                }
                if name == ".git" {
                    return GuardrailVerdict::approve(
                        "writing inside .git workspace metadata requires approval",
                    );
                }
            }
        }

        // 3) 绝对系统目录保护
        if writing {
            for pfx in &self.system_prefixes {
                if target.starts_with(pfx) {
                    return GuardrailVerdict::deny(&format!(
                        "target under protected system path: {}",
                        pfx.display()
                    ));
                }
            }
        }

        // 4) 跨工作区 / 路径穿越 越界（仅对绝对目标做严格判定）
        if target.has_root() {
            if target
                .components()
                .any(|c| matches!(c, Component::ParentDir))
            {
                return GuardrailVerdict::approve(
                    "path contains '..' traversal; explicit approval required",
                );
            }
            if !target.starts_with(cwd) {
                return GuardrailVerdict::approve(&format!(
                    "operation targets path outside workspace `{}`; explicit approval required",
                    cwd.display()
                ));
            }
        }

        // 相对目标视为工作区内（cwd-relative），放行
        GuardrailVerdict::allow()
    }

    /// 批量检查，返回最严格的裁决
    pub fn check_all(&self, items: &[(OpKind, &Path, &Path)]) -> GuardrailVerdict {
        let mut worst = GuardrailVerdict::allow();
        for (op, target, cwd) in items {
            let v = self.check(*op, target, cwd);
            if v.severity > worst.severity {
                worst = v;
            }
        }
        worst
    }
}

/// 标识符字符（用于词边界判定）：字母/数字/下划线
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// 以「词边界」方式判断 `haystack` 是否包含 `needle`。
///
/// 与裸 `str::contains` 的区别：要求命中处两侧不是标识符字符，
/// 从而避免 `mount` 命中 `paramount` / `unmount` / `mountain`，
/// 同时仍能命中 `/usr/sbin/chroot`、`sudo unshare -r`、`x;nsenter` 等真实逃逸写法
/// （`/`、空格、`;` 均非标识符字符）。
fn contains_token(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut from = 0usize;
    while let Some(rel) = haystack[from..].find(needle) {
        let start = from + rel;
        let end = start + needle.len();
        let left_ok = start == 0
            || !haystack[..start]
                .chars()
                .next_back()
                .is_some_and(is_word_char);
        let right_ok =
            end >= haystack.len() || !haystack[end..].chars().next().is_some_and(is_word_char);
        if left_ok && right_ok {
            return true;
        }
        // 继续向后找下一处（按字符边界前进，避免切在多字节中间）
        from = match haystack[start..].char_indices().nth(1) {
            Some((off, _)) => start + off,
            None => break,
        };
    }
    false
}

/// 沙箱逃逸意图匹配模式（命令级）
const ESCAPE_PATTERNS: &[&str] = &[
    "chroot",
    "mount",
    "umount",
    "modprobe",
    "insmod",
    "setcap",
    "setuid",
    "pkexec",
    "unshare",
    "nsenter",
    "docker run --privileged",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn g() -> GuardrailScanner {
        GuardrailScanner::new()
    }

    fn p(s: &str) -> &Path {
        Path::new(s)
    }

    #[test]
    fn denies_write_to_etc() {
        let v = g().check(OpKind::WriteFile, p("/etc/passwd"), p("/home/user/proj"));
        assert!(!v.allowed);
        assert!(!v.requires_approval);
        assert_eq!(v.severity, GuardrailSeverity::Deny);
        assert!(v.reason.unwrap().contains("/etc"));
    }

    #[test]
    fn denies_delete_in_windows_system() {
        let v = g().check(
            OpKind::Delete,
            p("C:\\Windows\\System32\\x.dll"),
            p("D:\\proj"),
        );
        assert_eq!(v.severity, GuardrailSeverity::Deny);
    }

    #[test]
    fn denies_ssh_dir_write() {
        let v = g().check(
            OpKind::WriteFile,
            p("/home/u/.ssh/authorized_keys"),
            p("/home/u/proj"),
        );
        assert_eq!(v.severity, GuardrailSeverity::Deny);
    }

    #[test]
    fn denies_aws_dir_write() {
        let v = g().check(
            OpKind::WriteFile,
            p("/home/u/.aws/credentials"),
            p("/home/u/proj"),
        );
        assert_eq!(v.severity, GuardrailSeverity::Deny);
    }

    #[test]
    fn denies_sandbox_escape_exec() {
        let v = g().check(OpKind::Exec, p("chroot /mnt /bin/sh"), p("/home/u/proj"));
        assert_eq!(v.severity, GuardrailSeverity::Deny);
        assert!(v.reason.unwrap().contains("sandbox-escape"));
    }

    #[test]
    fn requires_approval_for_outside_workspace() {
        let v = g().check(
            OpKind::WriteFile,
            p("/home/other/file.txt"),
            p("/home/u/proj"),
        );
        assert!(!v.allowed);
        assert!(v.requires_approval);
    }

    #[test]
    fn allows_write_inside_workspace() {
        let v = g().check(
            OpKind::WriteFile,
            p("/home/u/proj/src/main.rs"),
            p("/home/u/proj"),
        );
        assert!(v.allowed);
        assert!(!v.requires_approval);
    }

    #[test]
    fn requires_approval_for_traversal_back() {
        // 含 .. 的越界路径 → 需审批
        let v = g().check(
            OpKind::WriteFile,
            p("/home/u/proj/../../etc/x"),
            p("/home/u/proj"),
        );
        assert!(v.requires_approval);
    }

    #[test]
    fn read_is_permissive_in_workspace() {
        let v = g().check(OpKind::Read, p("/home/u/proj/README.md"), p("/home/u/proj"));
        assert!(v.allowed);
    }

    #[test]
    fn check_all_returns_strictest() {
        let items = [
            (
                OpKind::WriteFile,
                p("/home/u/proj/a.txt"),
                p("/home/u/proj"),
            ),
            (OpKind::WriteFile, p("/etc/passwd"), p("/home/u/proj")),
        ];
        let v = g().check_all(&items);
        assert_eq!(v.severity, GuardrailSeverity::Deny);
    }

    #[test]
    fn escape_patterns_match_on_word_boundary_only() {
        // 真实逃逸写法仍必须命中（含被路径/分隔符包裹的形式）
        for cmd in [
            "chroot /mnt /bin/sh",
            "/usr/sbin/chroot /jail sh",
            "sudo unshare -r /bin/bash",
            "foo; nsenter -t 1 -m",
            "mount -o bind / /mnt",
            "docker run --privileged ubuntu",
            "pkexec /bin/sh",
        ] {
            let v = g().check(OpKind::Exec, p(cmd), p("/home/u/proj"));
            assert_eq!(
                v.severity,
                GuardrailSeverity::Deny,
                "逃逸命令应被拒绝: {cmd}"
            );
        }
    }

    #[test]
    fn escape_patterns_do_not_false_positive_on_benign_words() {
        // 回归防护：这些良性命令内含 mount/setuid 等子串，但不是独立词，不得误杀
        for cmd in [
            "git commit -m \"add mountain view page\"",
            "echo paramount",
            "cargo test --test dismount_helper",
            "npm run build:amounts",
        ] {
            let v = g().check(OpKind::Exec, p(cmd), p("/home/u/proj"));
            assert!(v.allowed, "良性命令不应被判逃逸: {cmd} -> {:?}", v.reason);
        }
    }

    #[test]
    fn git_metadata_requires_approval() {
        let v = g().check(
            OpKind::WriteFile,
            p("/home/u/proj/.git/config"),
            p("/home/u/proj"),
        );
        assert!(v.requires_approval);
    }
}
