//! 预执行安全闸口：把 L2（恶意内容扫描）与 L3（操作护栏）合并成一次裁决
//!
//! 背景：L2 `MalwareScanner` / L3 `GuardrailScanner` 此前只有单测覆盖，
//! 生产路径零调用点，而 `acp/session.rs` 却对外声明 `enforced: true`——
//! 属"幻象防护"（虚假安全保证）。本模块是把两层真正接入运行时的统一入口。
//!
//! 定位：
//! - L1（`command_guard` / `ssrf_guard` / `exec` 沙箱）：命令字符串级语义拦截；
//! - L2（本模块 `check_content`）：**内容级**——落盘/执行前扫描恶意模式；
//! - L3（本模块 `check_path` / `check_exec`）：**权限边界级**——即便命令合法、
//!   路径也在工作区内，仍要拦住"写 .ssh""写 .git""chroot 逃逸"这类越界意图。
//!
//! L3 补的正是 L1 覆盖不到的洞：`safe_resolve` 只保证"不逃出工作区"，
//! 但工作区内写 `.ssh/authorized_keys` 它照样放行；`CommandGuard` 的正则表里
//! 没有 chroot / unshare / nsenter / pkexec 等逃逸原语。
//!
//! 设计原则：
//! - 纯静态、零外部依赖、不调 LLM，离线可测；
//! - **fail-closed**：`requires_approval` 在无审批通道的自动化路径上按拒绝处理
//!   （与 `guard::gateway::ApprovalGate` 「非交互即拒绝」的既有约定一致）；
//! - **不回显敏感内容**：拒绝理由只带规则 ID / 行号 / 规则说明，
//!   绝不把命中的原文片段（可能是密钥本身）写进错误信息或审计日志。

use std::path::{Path, PathBuf};

use super::scanner_l2::MalwareScanner;
use super::scanner_l3::{GuardrailScanner, OpKind};

/// 预执行裁决结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightDecision {
    /// 放行
    Allow,
    /// 阻断（附层级 / 机器可读错误码 / 人类可读理由）
    Deny {
        /// 命中层级："L2" | "L3"
        layer: &'static str,
        /// 机器可读错误码，直接用作 `ToolResult::Error.code`
        code: &'static str,
        /// 审计理由（已脱敏，不含命中原文）
        reason: String,
    },
}

impl PreflightDecision {
    /// 是否放行
    pub fn is_allowed(&self) -> bool {
        matches!(self, PreflightDecision::Allow)
    }

    /// 拒绝理由（放行时为 None）
    pub fn reason(&self) -> Option<&str> {
        match self {
            PreflightDecision::Allow => None,
            PreflightDecision::Deny { reason, .. } => Some(reason.as_str()),
        }
    }

    /// 命中层级（放行时为 None）
    pub fn layer(&self) -> Option<&'static str> {
        match self {
            PreflightDecision::Allow => None,
            PreflightDecision::Deny { layer, .. } => Some(layer),
        }
    }

    /// 机器可读错误码（放行时为 `"OK"`）
    pub fn code(&self) -> &'static str {
        match self {
            PreflightDecision::Allow => "OK",
            PreflightDecision::Deny { code, .. } => code,
        }
    }
}

/// L3 判定为"需审批"时使用的错误码。
///
/// 自动化 / 非交互路径没有人可以点"同意"，因此按拒绝处理（fail-closed），
/// 但用独立错误码与"硬拒绝"区分，便于上层将来接交互审批通道时改判。
pub const CODE_APPROVAL_REQUIRED: &str = "APPROVAL_REQUIRED";
/// L3 硬拒绝错误码
pub const CODE_GUARDRAIL_DENIED: &str = "GUARDRAIL_DENIED";
/// L2 恶意内容阻断错误码
pub const CODE_MALWARE_BLOCKED: &str = "MALWARE_BLOCKED";

/// L2 + L3 统一预执行闸口
///
/// 构造成本较高（L2 会编译约 20 条正则），调用方应复用单例
/// （见 `tools::builtin` 中的 `LazyLock<Preflight>`）。
#[derive(Debug, Clone)]
pub struct Preflight {
    guardrail: GuardrailScanner,
    malware: MalwareScanner,
    /// 是否启用 L2 内容扫描（默认 true）
    content_scan: bool,
}

impl Default for Preflight {
    fn default() -> Self {
        Self::new()
    }
}

impl Preflight {
    /// 构造全量闸口（L2 + L3 均启用）
    pub fn new() -> Self {
        Self {
            guardrail: GuardrailScanner::new(),
            malware: MalwareScanner::new(),
            content_scan: true,
        }
    }

    /// 仅启用 L3 路径护栏，关闭 L2 内容扫描。
    ///
    /// 供「只做路径裁决、内容由调用方另行处理」的场景使用（如 `delete` / `mkdir`
    /// 本就没有内容）。**注意**：这是构造期的显式选择，不提供运行时环境变量开关——
    /// 避免出现"改个环境变量就能静默关掉安全层"的旁路。
    pub fn path_only() -> Self {
        Self {
            guardrail: GuardrailScanner::new(),
            malware: MalwareScanner::new(),
            content_scan: false,
        }
    }

    /// L3：路径 / 操作边界裁决
    pub fn check_path(&self, op: OpKind, target: &Path, cwd: &Path) -> PreflightDecision {
        let v = self.guardrail.check(op, target, cwd);
        if v.allowed {
            return PreflightDecision::Allow;
        }
        let reason = v
            .reason
            .unwrap_or_else(|| "operation rejected by guardrail".to_string());
        if v.requires_approval {
            // fail-closed：无审批通道即拒绝，但保留独立错误码供上层改判。
            PreflightDecision::Deny {
                layer: "L3",
                code: CODE_APPROVAL_REQUIRED,
                reason: format!("{reason}（自动化路径无审批通道，按拒绝处理）"),
            }
        } else {
            PreflightDecision::Deny {
                layer: "L3",
                code: CODE_GUARDRAIL_DENIED,
                reason,
            }
        }
    }

    /// L3 多根版本（P2-9 多项目工作区）：目标落在**任一**登记根内即放行。
    ///
    /// 逐个根裁决而不是「先拼一个大边界」：每个根都要走软链/凭证目录等既有规则，
    /// 合并边界会绕过单一根上的检查。
    pub fn check_path_multi(
        &self,
        op: OpKind,
        target: &Path,
        roots: &[PathBuf],
    ) -> PreflightDecision {
        let mut last_deny: Option<PreflightDecision> = None;
        for root in roots {
            match self.check_path(op, target, root) {
                PreflightDecision::Allow => return PreflightDecision::Allow,
                deny => last_deny = Some(deny),
            }
        }
        last_deny.unwrap_or(PreflightDecision::Deny {
            layer: "L3",
            code: CODE_GUARDRAIL_DENIED,
            reason: "没有可用的工作区根，按拒绝处理".to_string(),
        })
    }

    /// 写入类多根裁决：路径过任一登记根 + 内容扫描
    pub fn check_write_multi(
        &self,
        target: &Path,
        roots: &[PathBuf],
        content: &str,
    ) -> PreflightDecision {
        let p = self.check_path_multi(OpKind::WriteFile, target, roots);
        if !p.is_allowed() {
            return p;
        }
        self.check_content(content)
    }

    /// 读取类多根裁决：凭证文件与越界路径一律拒绝。
    ///
    /// 不做内容扫描 —— 读取不产生落盘，L2 的「恶意内容」判据在这里没有意义；
    /// 但 L3 的凭证文件名判定必须生效（见 `scanner_l3::is_credential_file`）。
    pub fn check_read_multi(&self, target: &Path, roots: &[PathBuf]) -> PreflightDecision {
        self.check_path_multi(OpKind::Read, target, roots)
    }

    /// 执行类多根裁决：命令逃逸意图过任一登记根 + 命令文本扫描
    pub fn check_exec_multi(&self, command: &str, roots: &[PathBuf]) -> PreflightDecision {
        let p = self.check_path_multi(OpKind::Exec, Path::new(command), roots);
        if !p.is_allowed() {
            return p;
        }
        self.check_content(command)
    }

    /// L2：内容恶意模式扫描（落盘 / 执行前）
    pub fn check_content(&self, content: &str) -> PreflightDecision {
        if !self.content_scan {
            return PreflightDecision::Allow;
        }
        let report = self.malware.scan_text(content);
        if !report.blocked {
            return PreflightDecision::Allow;
        }
        // 只汇总规则 ID / 行号 / 规则说明——绝不回显 snippet（可能含密钥原文）
        let mut hits: Vec<String> = report
            .findings
            .iter()
            .filter(|f| f.severity >= super::scanner_l2::Severity::High)
            .map(|f| format!("{}@L{} ({})", f.rule_id, f.line, f.message))
            .collect();
        hits.dedup();
        hits.truncate(5);
        PreflightDecision::Deny {
            layer: "L2",
            code: CODE_MALWARE_BLOCKED,
            reason: format!(
                "内容命中恶意模式（risk_score={}）: {}",
                report.risk_score,
                hits.join("; ")
            ),
        }
    }

    /// 写入类操作的组合裁决：先 L3 路径边界，再 L2 内容扫描
    pub fn check_write(&self, target: &Path, cwd: &Path, content: &str) -> PreflightDecision {
        let p = self.check_path(OpKind::WriteFile, target, cwd);
        if !p.is_allowed() {
            return p;
        }
        self.check_content(content)
    }

    /// 执行类操作的组合裁决：L3 逃逸意图 + L2 命令文本扫描
    pub fn check_exec(&self, command: &str, cwd: &Path) -> PreflightDecision {
        // L3 的 Exec 分支把「命令串」当作 target 传入（见 scanner_l3::check）
        let p = self.check_path(OpKind::Exec, Path::new(command), cwd);
        if !p.is_allowed() {
            return p;
        }
        self.check_content(command)
    }
}

/// 把工作区路径归一化为可与 `safe_resolve` 输出直接比较的形式。
///
/// `safe_resolve` 内部以 `canonicalize(root)` 为基准拼装结果，
/// 若这里传未归一化的 workspace 作为 `cwd`，`Path::starts_with` 会误判为
/// "越出工作区"→ 触发 `requires_approval` → 把**所有正常写入**全部拒掉。
/// 这是接线时最容易踩的坑，故统一走此函数。
pub fn normalize_workspace(ws: &Path) -> PathBuf {
    std::fs::canonicalize(ws).unwrap_or_else(|_| ws.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pf() -> Preflight {
        Preflight::new()
    }

    #[test]
    fn allows_benign_write_inside_workspace() {
        let d = pf().check_write(
            Path::new("/home/u/proj/src/main.rs"),
            Path::new("/home/u/proj"),
            "fn main() { println!(\"hi\"); }\n",
        );
        assert!(d.is_allowed(), "良性写入不应被拦: {:?}", d.reason());
    }

    #[test]
    fn denies_credential_dir_write_even_inside_workspace() {
        // 关键：这条正是 L1 safe_resolve 覆盖不到的洞——路径确实在工作区内
        let d = pf().check_write(
            Path::new("/home/u/proj/.ssh/authorized_keys"),
            Path::new("/home/u/proj"),
            "ssh-rsa AAAA...\n",
        );
        assert!(!d.is_allowed());
        assert_eq!(d.code(), CODE_GUARDRAIL_DENIED);
        assert_eq!(d.layer(), Some("L3"));
    }

    #[test]
    fn denies_system_path_write() {
        let d = pf().check_path(
            OpKind::WriteFile,
            Path::new("/etc/passwd"),
            Path::new("/home/u/proj"),
        );
        assert_eq!(d.code(), CODE_GUARDRAIL_DENIED);
    }

    #[test]
    fn approval_required_maps_to_fail_closed_deny() {
        // .git 元数据写入 → L3 判 requires_approval → 自动化路径按拒绝
        let d = pf().check_path(
            OpKind::WriteFile,
            Path::new("/home/u/proj/.git/config"),
            Path::new("/home/u/proj"),
        );
        assert!(!d.is_allowed());
        assert_eq!(d.code(), CODE_APPROVAL_REQUIRED);
    }

    #[test]
    fn denies_malicious_content_on_write() {
        let d = pf().check_write(
            Path::new("/home/u/proj/x.py"),
            Path::new("/home/u/proj"),
            "def run(c):\n    return eval(c)\n",
        );
        assert!(!d.is_allowed());
        assert_eq!(d.code(), CODE_MALWARE_BLOCKED);
        assert_eq!(d.layer(), Some("L2"));
    }

    #[test]
    fn deny_reason_never_leaks_secret_material() {
        // 命中 AWS Key 规则时，理由里只能出现规则 ID，不能出现密钥本身
        let secret = "AKIAIOSFODNN7EXAMPLE";
        let content = format!("key = \"{secret}\"\n");
        let d = pf().check_write(
            Path::new("/home/u/proj/cfg.py"),
            Path::new("/home/u/proj"),
            &content,
        );
        assert!(!d.is_allowed());
        let reason = d.reason().unwrap();
        assert!(reason.contains("L2-AWS-KEY"), "应标注规则 ID: {reason}");
        assert!(
            !reason.contains(secret),
            "拒绝理由绝不能回显密钥原文: {reason}"
        );
    }

    #[test]
    fn denies_sandbox_escape_exec() {
        let d = pf().check_exec("chroot /mnt /bin/sh", Path::new("/home/u/proj"));
        assert!(!d.is_allowed());
        assert_eq!(d.layer(), Some("L3"));
    }

    #[test]
    fn allows_benign_exec() {
        for cmd in ["echo hello", "cargo build", "ls -la", "git commit -m fix"] {
            let d = pf().check_exec(cmd, Path::new("/home/u/proj"));
            assert!(
                d.is_allowed(),
                "良性命令不应被拦: {cmd} -> {:?}",
                d.reason()
            );
        }
    }

    #[test]
    fn path_only_skips_content_scan() {
        let d = Preflight::path_only().check_write(
            Path::new("/home/u/proj/x.py"),
            Path::new("/home/u/proj"),
            "eval(x)\n",
        );
        assert!(d.is_allowed(), "path_only 模式不做内容扫描");
    }

    #[test]
    fn normalize_workspace_is_idempotent_for_missing_path() {
        let p = Path::new("/definitely/not/existing/xyz123");
        assert_eq!(normalize_workspace(p), p.to_path_buf());
    }
}
