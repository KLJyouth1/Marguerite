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

/// 受保护（禁止越界写/删）的系统目录前缀（绝对路径）
///
/// 判定不能只用 [`Path::starts_with`]：Windows 风格路径（`C:\Windows`）在非 Windows
/// 宿主上只是**一个组件名**，`starts_with` 永远不成立 —— 同一份守卫会在不同平台
/// 给出不同结论（CI 的 ubuntu 作业就是这么漏判的）。判定统一走
/// [`is_under_protected_prefix`]。
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

/// 引擎自身的凭证文件（按文件名，大小写不敏感）。
///
/// **为什么必须按文件名而不是路径前缀**：未绑定项目时工作区根回落到数据目录
/// `~/.marguerite`（见 `workspace::fallback`），于是这些文件与源码同处一个根内 ——
/// 只做「越界」判定的话它们全部算「根内合法目标」。
const ENGINE_CREDENTIAL_FILES: &[&str] = &[
    "gateway-token",
    "credentials.json",
    "vault.json",
    "model.key",
    "desktop-credential.json",
];

/// 不含密钥的环境变量模板（脚手架场景要读，放行）。
const ENV_TEMPLATES: &[&str] = &["example", "sample", "template", "dist"];

/// 是否为 `.env` 家族文件（`.env` / `.env.local` …；模板除外）。
fn is_env_file(path: &Path) -> bool {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()) else {
        return false;
    };
    if name == ".env" {
        return true;
    }
    match name.strip_prefix(".env.") {
        Some(rest) => !ENV_TEMPLATES.contains(&rest),
        None => false,
    }
}

/// 是否为引擎自身的凭证文件。
fn is_engine_credential_file(path: &Path) -> bool {
    path.file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|n| ENGINE_CREDENTIAL_FILES.contains(&n.as_str()))
}

/// 该路径是否承载凭据（`.env` 家族或引擎凭证文件）。
///
/// 供**批量扫描**类工具（grep / codebase_search）逐文件过滤：它们不按单个目标
/// 做裁决，但同样不能把 `.env` 的内容带进结果。
pub fn is_credential_file(path: &Path) -> bool {
    is_engine_credential_file(path) || is_env_file(path)
}

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

        // 2a) 读取凭证文件：读进上下文 = 密钥随下一次请求发往模型服务商，
        //     与写入同等不可接受，且在「默认工作区 = 数据目录」下极易发生。
        if op == OpKind::Read && is_credential_file(target) {
            return GuardrailVerdict::deny(&format!(
                "credential file must not be read by tools: {}",
                target.file_name().unwrap_or_default().to_string_lossy()
            ));
        }

        // 2b) 引擎凭证文件：写/删/移一律拒绝（防篡改 token 或凭据台账）。
        //     `.env` 的写入刻意放行 —— 脚手架/配置场景需要，且写入不泄露既有密钥。
        if writing && is_engine_credential_file(target) {
            return GuardrailVerdict::deny(&format!(
                "engine credential file is read-only to tools: {}",
                target.file_name().unwrap_or_default().to_string_lossy()
            ));
        }

        // 3) 凭证目录（按组件名）：.ssh/.aws/.gnupg 直接拒绝；.git 需审批
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

        // 4) 绝对系统目录保护
        if writing {
            for pfx in &self.system_prefixes {
                if is_under_protected_prefix(target, pfx) {
                    return GuardrailVerdict::deny(&format!(
                        "target under protected system path: {}",
                        pfx.display()
                    ));
                }
            }
        }

        // 5) 跨工作区 / 路径穿越 越界（仅对绝对目标做严格判定）
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

/// 判断 `target` 是否落在 `prefix` 之下（含自身），**与宿主平台无关**。
///
/// 规则：
/// 1. 先试 [`Path::starts_with`]（POSIX 上最准确，也最快）；
/// 2. 再按「分隔符归一 + 目录边界」比较字符串，于是 Windows 路径在 Linux 宿主上
///    仍能被判定（安全守卫不该因为跑在哪个 CI 上而改变结论）；
/// 3. 只对**带盘符**的前缀做大小写折叠（`C:\Windows` ≡ `c:\windows`）；
///    POSIX 路径大小写敏感，折叠会把 `/ETC` 也当成系统目录 —— 判定口径会变糊。
fn is_under_protected_prefix(target: &Path, prefix: &Path) -> bool {
    if target.starts_with(prefix) {
        return true;
    }
    let target_s = target.to_string_lossy().replace('\\', "/");
    let prefix_s = prefix.to_string_lossy().replace('\\', "/");
    if prefix_s.is_empty() {
        return false;
    }
    let windows_style = prefix_s.len() >= 2 && prefix_s.as_bytes()[1] == b':';
    let (target_s, prefix_s) = if windows_style {
        (target_s.to_lowercase(), prefix_s.to_lowercase())
    } else {
        (target_s, prefix_s)
    };
    target_s == prefix_s
        || target_s
            .strip_prefix(&prefix_s)
            .is_some_and(|rest| rest.starts_with('/'))
}

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

    /// 同一份判定必须**在 Linux 宿主上也成立**（CI 的 ubuntu 作业曾因此判红）：
    /// `C:\Windows` 在 POSIX 上只是一个组件名，`Path::starts_with` 永远不匹配。
    #[test]
    fn windows_system_prefix_matches_regardless_of_host() {
        assert!(is_under_protected_prefix(
            Path::new("C:\\Windows\\System32\\x.dll"),
            Path::new("C:\\Windows")
        ));
        // 盘符大小写不敏感；目录边界必须成立（`C:\Windows10` 不是 `C:\Windows`）
        assert!(is_under_protected_prefix(
            Path::new("c:\\windows\\system32\\x.dll"),
            Path::new("C:\\Windows")
        ));
        assert!(!is_under_protected_prefix(
            Path::new("C:\\Windows10\\x.dll"),
            Path::new("C:\\Windows")
        ));
        // POSIX 前缀保持大小写敏感（别把 `/ETC` 也当系统目录）
        assert!(is_under_protected_prefix(
            Path::new("/etc/passwd"),
            Path::new("/etc")
        ));
        assert!(!is_under_protected_prefix(
            Path::new("/ETC/passwd"),
            Path::new("/etc")
        ));
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

    /// 未绑定项目时工作区根就是数据目录，`.env` 与源码同级 —— 读取必须按文件名拦下，
    /// 否则密钥会直接进入模型上下文并随下一次请求发往服务商。
    #[test]
    fn denies_reading_env_file_even_inside_workspace() {
        let root = p("/home/u/.marguerite");
        for target in [
            "/home/u/.marguerite/.env",
            "/home/u/proj/.env.local",
            "/home/u/proj/.env.production",
        ] {
            let v = g().check(OpKind::Read, p(target), root);
            assert_eq!(v.severity, GuardrailSeverity::Deny, "应拒绝读取 {target}");
        }
    }

    /// 模板文件不含密钥，脚手架场景要读 —— 不能一并误杀。
    #[test]
    fn allows_reading_env_templates() {
        let root = p("/home/u/proj");
        for target in [
            "/home/u/proj/.env.example",
            "/home/u/proj/.env.sample",
            "/home/u/proj/.env.template",
            "/home/u/proj/.env.dist",
        ] {
            let v = g().check(OpKind::Read, p(target), root);
            assert!(v.allowed, "模板应放行: {target} -> {:?}", v.reason);
        }
    }

    /// 引擎凭证文件：读与写都不允许（防泄露 + 防篡改）。
    #[test]
    fn denies_engine_credential_files_for_read_and_write() {
        let root = p("/home/u/.marguerite");
        for name in [
            "gateway-token",
            "credentials.json",
            "vault.json",
            "model.key",
            "desktop-credential.json",
        ] {
            let target = PathBuf::from("/home/u/.marguerite").join(name);
            for op in [OpKind::Read, OpKind::WriteFile, OpKind::Delete] {
                let v = g().check(op, &target, root);
                assert_eq!(
                    v.severity,
                    GuardrailSeverity::Deny,
                    "{op:?} 应拒绝引擎凭证 {name}"
                );
            }
        }
    }

    /// `.env` 的**写入**刻意放行（脚手架要生成它），只拦读取。
    #[test]
    fn writing_env_file_stays_allowed_for_scaffolding() {
        let v = g().check(OpKind::WriteFile, p("/home/u/proj/.env"), p("/home/u/proj"));
        assert!(v.allowed, "写 .env 属正常脚手架: {:?}", v.reason);
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
