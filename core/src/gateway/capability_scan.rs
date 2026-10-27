//! P2·C SkillScan：skill / 工具加载前的静态**能力探针**（capability scan）
//!
//! 定位：在 `gateway::security_seam`（Policy / ApprovalGate / AuditStore）之上
//! 新增「能力层」静态扫描——聚焦 skill / 工具**能干什么**，而不是内容本身
//! 是否被既有 L2/L3 恶意扫描命中：
//! - 危险调用探针：`exec` / 破坏性 `rm` / 网络出口 / 凭证读取；
//! - 越权文件系统路径：越过工作区写敏感路径（`/etc/passwd`、`~/.ssh` 等）；
//! - 未声明副作用：写 / 删除 / 安装 / 网络操作但 manifest 未声明；
//!
//! 输出 capability 报告与风险等级（Low / Medium / High，fail-closed：
//! High 默认 `allow=false`，需要显式审批；与 `guard::trust` 信任库联动——
//! 已扫描且**签发**（trusted）的能力才允许自动执行。
//!
//! 纯静态、零外部依赖、确定性、可单测；不回显敏感片段原文（仅规则 ID + 行号）。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// 高风险被阻断时的机器可读错误码
pub const SKILL_SCAN_HIGH_RISK: &str = "SKILL_SCAN_HIGH_RISK";
/// 能力未在信任库签发（无法自动执行、需要显式授权）
pub const SKILL_SCAN_UNTRUSTED: &str = "SKILL_SCAN_UNTRUSTED";

/// 能力风险等级
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CapabilityRisk {
    #[default]
    Low,
    Medium,
    High,
}

/// 危险调用命中（脱敏：只带规则 ID / 行号 / 无害短片段）
#[derive(Debug, Clone)]
pub struct DangerousCall {
    pub rule: &'static str,
    pub line: usize,
}

/// 能力扫描报告
#[derive(Debug, Clone)]
pub struct CapabilityReport {
    /// 是否放行（fail-closed：High 阻断）
    pub allow: bool,
    /// 综合风险等级
    pub risk: CapabilityRisk,
    /// 命中的危险调用
    pub dangerous_calls: Vec<DangerousCall>,
    /// 越权文件系统路径
    pub unauthorized_paths: Vec<String>,
    /// 未声明副作用（写 / 删除 / 网络 / 安装）
    pub undeclared_side_effects: Vec<String>,
    /// 人类可读、已脱敏的判定理由
    pub reason: String,
}

impl CapabilityReport {
    pub fn is_allowed(&self) -> bool {
        self.allow
    }

    pub fn code(&self) -> &'static str {
        if !self.allow {
            SKILL_SCAN_HIGH_RISK
        } else {
            "OK"
        }
    }
}

/// 声明副作用种类（manifest 声明）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SideEffectKind {
    Write,
    Delete,
    Network,
    Install,
}

/// 高危险调用规则（命中即 High）
const HIGH_RULES: &[(&str, &[&str])] = &[
    (
        "exec",
        &[
            "exec(",
            "system(",
            "popen(",
            "subprocess",
            "Command::new",
            "os.system",
            "spawn(",
        ],
    ),
    (
        "destructive-rm",
        &["rm -rf", "rmtree", "remove_dir_all", "fs.rm(", "Remove-Item -Recurse -Force"],
    ),
    (
        "credential-read",
        &[
            ".env",
            "api_key",
            "api-key",
            "apikey",
            "password",
            "passwd",
            "secret",
            "credentials",
            "Bearer ",
            "Authorization",
        ],
    ),
];

/// 网络出口（中风险；同时计入副作用 Network）
const NETWORK_SIGNS: &[&str] = &[
    "curl ", "wget ", "reqwest", "http://", "https://", "TcpStream", "UdpSocket",
    "fetch(", "axios", "requests.", "send(",
];

/// 越权/敏感文件系统路径（命中即 High）
const UNAUTH_PATHS: &[&str] = &[
    "/etc/passwd",
    "/etc/shadow",
    "/root/",
    "/.ssh",
    "/home/",
    "C:\\Users\\",
    "/Users/",
    "~/.ssh",
    "~/.aws",
    "..\\..\\",
    "../../",
    "%APPDATA%",
    "%USERPROFILE%",
    "C:\\Windows",
];

/// 写副作用信号
const WRITE_SIGNS: &[&str] = &[
    "fs::write", "std::fs::write", "writeFile", "fs.writeFile", "open(.*\"w\"",
    "create_dir", "mkdir", "os.makedirs", "new FileWriter",
];
/// 删除副作用信号
const DELETE_SIGNS: &[&str] = &["delete", "remove", "unlink", "rmtree", "remove_dir", "fs.unlink"];
/// 安装副作用信号
const INSTALL_SIGNS: &[&str] = &[
    "pip install", "npm install", "cargo install", "apt install", "apt-get install", "yarn add",
];

/// SkillScan 能力探针
#[derive(Debug, Clone, Copy, Default)]
pub struct SkillScan;

impl SkillScan {
    pub fn new() -> Self {
        Self
    }

    /// 扫描 skill/工具文本，产出能力报告
    pub fn scan(&self, declared: &[SideEffectKind], content: &str) -> CapabilityReport {
        let mut dangerous_calls = Vec::new();
        let mut unauthorized_paths = Vec::new();
        let mut undeclared_side_effects: Vec<String> = Vec::new();
        let mut saw_network = false;
        let mut saw_write = false;
        let mut saw_delete = false;
        let mut saw_install = false;

        for (i, line) in content.lines().enumerate() {
            let line_no = i + 1;
            let lower = line.to_ascii_lowercase();

            // 危险调用（逐规则）
            for (rule, signs) in HIGH_RULES {
                if signs.iter().any(|s| lower.contains(s)) {
                    dangerous_calls.push(DangerousCall {
                        rule,
                        line: line_no,
                    });
                }
            }

            // 越权路径
            for p in UNAUTH_PATHS {
                if lower.contains(p) {
                    unauthorized_paths.push(p.to_string());
                }
            }

            // 副作用探测
            if NETWORK_SIGNS.iter().any(|s| lower.contains(s)) {
                saw_network = true;
            }
            if WRITE_SIGNS.iter().any(|s| lower.contains(s)) {
                saw_write = true;
            }
            if DELETE_SIGNS.iter().any(|s| lower.contains(s)) {
                saw_delete = true;
            }
            if INSTALL_SIGNS.iter().any(|s| lower.contains(s)) {
                saw_install = true;
            }
        }

        let declared_set: HashSet<SideEffectKind> = declared.iter().copied().collect();
        if saw_write && !declared_set.contains(&SideEffectKind::Write) {
            undeclared_side_effects.push("write".to_string());
        }
        if saw_delete && !declared_set.contains(&SideEffectKind::Delete) {
            undeclared_side_effects.push("delete".to_string());
        }
        if saw_network && !declared_set.contains(&SideEffectKind::Network) {
            undeclared_side_effects.push("network".to_string());
        }
        if saw_install && !declared_set.contains(&SideEffectKind::Install) {
            undeclared_side_effects.push("install".to_string());
        }

        // 风险定级：高危规则 / 越权路径 → High；网络出口 → Medium；否则 Low
        let has_high_call = dangerous_calls
            .iter()
            .any(|d| matches!(d.rule, "exec" | "destructive-rm" | "credential-read"));
        let risk = if has_high_call || !unauthorized_paths.is_empty() {
            CapabilityRisk::High
        } else if saw_network {
            CapabilityRisk::Medium
        } else {
            CapabilityRisk::Low
        };

        let allow = risk != CapabilityRisk::High;
        let reason = if !allow {
            format!(
                "能力风险等级 High：危险调用 {} 项、越权路径 {} 项（已脱敏，可查看 rule/line 审计）",
                dangerous_calls.len(),
                unauthorized_paths.len()
            )
        } else if !undeclared_side_effects.is_empty() {
            format!(
                "存在未声明副作用：{}（放行但需记录）",
                undeclared_side_effects.join(", ")
            )
        } else {
            "能力扫描通过".to_string()
        };

        CapabilityReport {
            allow,
            risk,
            dangerous_calls,
            unauthorized_paths,
            undeclared_side_effects,
            reason,
        }
    }

    /// 与 `guard::trust` 信任库联动：该能力是否已被签发
    pub fn is_signed(&self, tool: &str, mode: &str, session: &str) -> bool {
        let store = crate::guard::trust::global();
        store
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_trusted(tool, mode, session)
            .is_some()
    }

    /// 自动执行是否被允许：扫描通过 **且** 已签发（fail-closed）
    pub fn auto_execute_allowed(
        &self,
        report: &CapabilityReport,
        tool: &str,
        mode: &str,
        session: &str,
    ) -> bool {
        report.allow && self.is_signed(tool, mode, session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::trust::{TrustLevel, global};

    fn reset_trust() {
        let store = global();
        store.lock().unwrap_or_else(|e| e.into_inner()).revoke_all();
    }

    #[test]
    fn rm_rf_skill_is_flagged_high_risk() {
        reset_trust();
        let scanner = SkillScan::new();
        let skill = r#"---
name: cleanup
---
run: rm -rf /tmp/userdata && echo done
"#;
        let report = scanner.scan(&[], skill);
        assert_eq!(report.risk, CapabilityRisk::High);
        assert!(!report.allow, "含 rm -rf 的 skill 应被标高危并 fail-closed 阻断");
        assert!(
            report.dangerous_calls.iter().any(|d| d.rule == "destructive-rm"),
            "应命中 destructive-rm 规则"
        );
        assert_eq!(report.code(), SKILL_SCAN_HIGH_RISK);
    }

    #[test]
    fn exec_call_is_flagged_high_risk() {
        let scanner = SkillScan::new();
        let skill = "def run(task):\n    import subprocess\n    return subprocess.run(task)";
        let report = scanner.scan(&[], skill);
        assert_eq!(report.risk, CapabilityRisk::High);
        assert!(report.dangerous_calls.iter().any(|d| d.rule == "exec"));
    }

    #[test]
    fn undeclared_side_effects_are_captured() {
        let scanner = SkillScan::new();
        // 网络 + 安装副作用，但 manifest 未声明
        let skill = "download assets:\n  curl -s https://example.com/assets.tar -o out.tar\n  pip install requests";
        let report = scanner.scan(&[], skill);
        assert!(report.allow, "中风险放行（不需要审批门）");
        assert!(
            report.undeclared_side_effects.contains(&"network".to_string()),
            "应捕获未声明的 network 副作用"
        );
        assert!(
            report.undeclared_side_effects.contains(&"install".to_string()),
            "应捕获未声明的 install 副作用"
        );
    }

    #[test]
    fn declared_side_effects_are_not_flagged() {
        let scanner = SkillScan::new();
        let skill = "write results:\n  fs.writeFile('report.txt', data)\n  curl -s https://example.com/ping";
        let report = scanner.scan(
            &[SideEffectKind::Write, SideEffectKind::Network],
            skill,
        );
        assert!(report.allow);
        assert!(
            report.undeclared_side_effects.is_empty(),
            "已声明 Write/Network 后不应再报未声明，实际: {:?}",
            report.undeclared_side_effects
        );
    }

    #[test]
    fn unauthorized_path_detected() {
        let scanner = SkillScan::new();
        let skill = "read shadow:\n  cat /etc/passwd";
        let report = scanner.scan(&[], skill);
        assert_eq!(report.risk, CapabilityRisk::High);
        assert!(
            report.unauthorized_paths.contains(&"/etc/passwd".to_string()),
            "应捕获越权路径 /etc/passwd"
        );
        assert!(!report.allow);
    }

    #[test]
    fn benign_skill_allowed() {
        let scanner = SkillScan::new();
        let skill = "summarize notes:\n  read input.txt and summarize key points";
        let report = scanner.scan(&[], skill);
        assert!(report.allow);
        assert_eq!(report.risk, CapabilityRisk::Low);
        assert!(report.dangerous_calls.is_empty());
        assert!(report.undeclared_side_effects.is_empty());
    }

    #[test]
    fn trust_linked_auto_execute_requires_signature() {
        reset_trust();
        let scanner = SkillScan::new();
        let skill = "summarize notes:\n  read input.txt and summarize key points";
        let report = scanner.scan(&[], skill);
        assert!(report.allow);
        let tool = "skill:summarizer";
        // 未签发 → 禁止自动执行
        assert!(!scanner.auto_execute_allowed(&report, tool, "omni", "s1"));
        // 签发后 → 允许自动执行
        {
            let store = global();
            store
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .grant(TrustLevel::High, tool, Some("omni".to_string()), "s1");
        }
        assert!(scanner.auto_execute_allowed(&report, tool, "omni", "s1"));
        // 未签发工具仍被拒（fail-closed）
        assert!(!scanner.auto_execute_allowed(&report, "skill:other", "omni", "s1"));
        reset_trust();
    }
}