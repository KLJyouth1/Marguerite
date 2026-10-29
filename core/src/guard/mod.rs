//! 幻觉护栏模块
//!
//! 三层护栏 + 代码契约，在代码写入/执行前拦截幻觉错误：
//! 1. 语法检查（tree-sitter AST）
//! 2. 语义检查（未定义变量，LSP 可扩展）
//! 3. 事实检查（本地知识库 + MCP 查询）
//! 4. 代码契约（@pre/@post/@invariant 验证）

pub mod approval_registry;
pub mod contract;
pub mod device_perms;
pub mod elevated;
pub mod escalation;
pub mod escalation_registry;
pub mod fact;
pub mod gateway;
pub mod permission_matrix;
pub mod repair;
pub mod semantic;
pub mod syntax;
pub mod tool_intent;
pub mod trust;

use serde::{Deserialize, Serialize};

use contract::ContractChecker;
use fact::FactChecker;
use semantic::SemanticChecker;
use syntax::{Language, SyntaxChecker};

/// 护栏层级
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardLayer {
    /// 语法层
    Syntax,
    /// 语义层
    Semantic,
    /// 事实层
    Fact,
    /// 契约层
    Contract,
}

/// 问题严重程度
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// 代码位置（0 起始行/列，与 tree-sitter Point 一致）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRange {
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

/// 统一的护栏问题
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardIssue {
    pub layer: GuardLayer,
    pub severity: Severity,
    pub message: String,
    pub range: Option<SourceRange>,
    pub suggestion: Option<String>,
}

impl GuardIssue {
    pub fn new(
        layer: GuardLayer,
        severity: Severity,
        message: impl Into<String>,
        range: Option<SourceRange>,
        suggestion: Option<String>,
    ) -> Self {
        Self {
            layer,
            severity,
            message: message.into(),
            range,
            suggestion,
        }
    }
}

/// 护栏检查报告
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuardReport {
    pub passed: bool,
    pub issues: Vec<GuardIssue>,
}

impl GuardReport {
    /// 汇总所有检查结果
    pub fn merge(mut self, other: GuardReport) -> Self {
        self.passed = self.passed && other.passed;
        self.issues.extend(other.issues);
        self
    }

    /// 是否无 Error 级问题
    pub fn has_error(&self) -> bool {
        self.issues.iter().any(|i| i.severity == Severity::Error)
    }
}

/// 护栏配置（可开关各层检查）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardConfig {
    /// 语法检查开关
    #[serde(default = "default_true")]
    pub syntax: bool,
    /// 语义检查开关
    #[serde(default = "default_true")]
    pub semantic: bool,
    /// 事实检查开关
    #[serde(default = "default_true")]
    pub fact: bool,
    /// 契约检查开关
    #[serde(default = "default_true")]
    pub contract: bool,
    /// v3.3.1：审批门超时（秒）。挂起审批超过该时长自动拒绝（fail-closed）。
    /// 0 = 使用默认 60s。此前硬编码 60s，长审批流（用户离开工位）常被误拒。
    #[serde(default)]
    pub approval_timeout_secs: u64,
}

fn default_true() -> bool {
    true
}

impl Default for GuardConfig {
    fn default() -> Self {
        Self {
            syntax: true,
            semantic: true,
            fact: true,
            contract: true,
            approval_timeout_secs: 0, // 0 = ApprovalGate 内部默认 60s
        }
    }
}

/// 护栏网关
///
/// 编排三层护栏 + 代码契约，一次调用完成全部校验。
/// 采用短路径设计：任一 Error 级问题立即中断后续层检查（节省计算）。
pub struct GuardGateway {
    syntax: SyntaxChecker,
    semantic: SemanticChecker,
    fact: FactChecker,
    contract: ContractChecker,
    config: GuardConfig,
}

impl GuardGateway {
    pub fn new(config: GuardConfig) -> Self {
        Self {
            syntax: SyntaxChecker::new(),
            semantic: SemanticChecker::new(),
            fact: FactChecker::new(),
            contract: ContractChecker::new(),
            config,
        }
    }

    /// 运行全部启用的检查
    ///
    /// 语法错误优先：存在语法错误时跳过语义/契约（AST 不可信）。
    pub fn check(&self, code: &str, lang: Language) -> GuardReport {
        let mut report = GuardReport {
            passed: true,
            issues: Vec::new(),
        };

        // 第一层：语法（最硬性的检查）
        if self.config.syntax {
            let issues = self.syntax.check(code, lang);
            if !issues.is_empty() {
                report.passed = false;
                report.issues.extend(issues);
                return report; // 语法错误 → 短路
            }
        }

        // 第二层：语义
        if self.config.semantic {
            let issues = self.semantic.check(code, lang);
            if !issues.is_empty() {
                report.issues.extend(issues);
            }
        }

        // 第三层：事实
        if self.config.fact {
            let issues = self.fact.check(code, lang);
            report.issues.extend(issues);
        }

        // 加强层：代码契约
        if self.config.contract {
            let issues = self.contract.check(code, lang);
            report.issues.extend(issues);
        }

        report.passed = !report.has_error();
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gateway_valid_code() {
        let gateway = GuardGateway::new(GuardConfig::default());
        let report = gateway.check("const x = 1;\nconsole.log(x);\n", Language::JavaScript);
        assert!(report.passed);
        assert!(report.issues.is_empty());
    }

    #[test]
    fn test_gateway_syntax_error_short_circuit() {
        let gateway = GuardGateway::new(GuardConfig::default());
        let report = gateway.check("function foo() {\n  return 1;\n", Language::JavaScript);
        assert!(!report.passed);
        assert!(report.has_error());
        assert!(report.issues.iter().all(|i| i.layer == GuardLayer::Syntax));
    }

    #[test]
    fn test_gateway_all_layers() {
        let gateway = GuardGateway::new(GuardConfig::default());
        let report = gateway.check("function foo() {\n  return bar;\n}\n", Language::JavaScript);
        // 语义警告不阻断，但应被报告
        assert!(report
            .issues
            .iter()
            .any(|i| i.layer == GuardLayer::Semantic));
        assert!(!report.has_error());
    }

    #[test]
    fn test_gateway_disabled_layer() {
        let config = GuardConfig {
            fact: false,
            ..Default::default()
        };
        let gateway = GuardGateway::new(config);
        let report = gateway.check("fetch('/api');\n", Language::JavaScript);
        // 事实检查被禁用 → 不再报告 fetch 提示
        assert!(report.issues.iter().all(|i| i.layer != GuardLayer::Fact));
    }
}
