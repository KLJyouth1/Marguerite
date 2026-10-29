//! 安全层：命令护栏 + SSRF 防护 + 沙箱执行 + 分层静态扫描
//!
//! 这是 Marguerite 相比 标准 AI 助手 的差异化能力之一——所有外部执行（shell / 网络）
//! 在真正落地前必须经过可审计的安全策略拦截，默认拒绝高危操作，而非信任模型输出。
//!
//! 分层：
//! - L1（命令级语义拦截）：`command_guard` / `ssrf_guard` / `exec` 沙箱
//! - L2（静态代码/内容扫描）：`scanner_l2` 恶意模式启发式扫描（对标 `sec.malware`）
//! - L3（操作护栏/权限边界）：`scanner_l3` 越界/逃逸裁决（对标 `sec.guardrail`）

pub mod command_guard;
pub mod exec;
pub mod os_sandbox;
pub mod preflight;
pub mod scanner_l2;
pub mod scanner_l3;
pub mod skill_scan;
pub mod spotlight;
pub mod ssrf_guard;

pub use command_guard::CommandGuard;
pub use exec::{
    exec_with, sandboxed_exec, ExecOutput, ExecRequest, ExecShell, DEFAULT_MAX_OUTPUT,
    DEFAULT_TIMEOUT,
};
pub use os_sandbox::{
    apply_for_current_platform, kernel_isolation_available, platform_sandbox, IsolationLevel,
    OsSandbox, OsSandboxVerdict,
};
pub use preflight::{
    normalize_workspace, Preflight, PreflightDecision, CODE_APPROVAL_REQUIRED,
    CODE_GUARDRAIL_DENIED, CODE_MALWARE_BLOCKED,
};
pub use scanner_l2::{MalwareScanner, ScanCategory, ScanFinding, ScanReport, Severity};
pub use scanner_l3::{GuardrailScanner, GuardrailSeverity, GuardrailVerdict, OpKind};
pub use skill_scan::{SkillInstallScanner, SkillScanVerdict, SKILL_MALWARE_BLOCKED};
pub use ssrf_guard::SsrfGuard;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// 分层安全集成链：模拟"生成含 eval 的代码 + 试图写 /etc/passwd"的危险场景。
    /// L2 应命中 critical 并 blocked；L3 应 deny 越界写。
    #[test]
    fn security_layered_chain_blocks_malicious_pipeline() {
        // —— L2：扫描生成的代码 ——
        let generated = "def run(cmd):\n    return eval(cmd)\n";
        let l2 = MalwareScanner::new().scan_text(generated);
        assert!(
            l2.blocked,
            "L2 应阻断含 eval 的生成代码，risk_score={}",
            l2.risk_score
        );
        assert!(l2.has_severity_at_least(Severity::Critical));

        // —— L3：裁决"写 /etc/passwd"操作 ——
        let l3 = GuardrailScanner::new().check(
            OpKind::WriteFile,
            Path::new("/etc/passwd"),
            Path::new("/home/user/proj"),
        );
        assert_eq!(l3.severity, GuardrailSeverity::Deny);
        assert!(!l3.allowed);

        // 组合裁决：任一阻挡即整体拒绝
        let overall_blocked = l2.blocked || !l3.allowed;
        assert!(overall_blocked, "分层扫描叠加应整体拒绝危险管线");
    }

    /// 良性场景：工作区内写普通文件 + 无害代码，应全流程放行。
    #[test]
    fn security_layered_chain_allows_benign() {
        let code = "def add(a, b):\n    return a + b\n";
        let l2 = MalwareScanner::new().scan_text(code);
        assert!(!l2.blocked);

        let l3 = GuardrailScanner::new().check(
            OpKind::WriteFile,
            Path::new("/home/user/proj/src/main.rs"),
            Path::new("/home/user/proj"),
        );
        assert!(l3.allowed);
    }
}
