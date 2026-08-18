//! 代码契约（护栏加强层）
//!
//! 借鉴 NexusStar code_contract，从文档注释中提取契约
//! （@pre 前置条件 / @post 后置条件 / @invariant 不变量），
//! 校验实现是否满足声明。

use regex::Regex;

use super::{GuardIssue, GuardLayer, Severity};
use crate::guard::syntax::Language;

/// 契约类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractKind {
    Pre,
    Post,
    Invariant,
}

/// 提取到的契约条目
#[derive(Debug, Clone)]
pub struct ContractEntry {
    pub kind: ContractKind,
    pub condition: String,
    pub line: usize,
}

/// 代码契约检查器
///
/// 解析 `@pre`/`@post`/`@invariant` 注释，检查实现中的
/// 断言/校验语句是否覆盖契约条件。
pub struct ContractChecker {
    tag_re: Regex,
}

impl ContractChecker {
    pub fn new() -> Self {
        Self {
            tag_re: Regex::new(r"@(pre|post|invariant)\s+(.+)").unwrap(),
        }
    }

    /// 从代码中提取契约
    pub fn extract(&self, code: &str) -> Vec<ContractEntry> {
        let mut entries = Vec::new();
        for (idx, line) in code.lines().enumerate() {
            let stripped = line.trim();
            // 支持 `// @pre`、`/// @pre`、`# @pre`、`/* @pre */` 等注释风格
            let candidate = stripped
                .trim_start_matches("//")
                .trim_start_matches("///")
                .trim_start_matches('#')
                .trim_start_matches("/*")
                .trim_start_matches("*/")
                .trim_start_matches('*')
                .trim();
            if let Some(caps) = self.tag_re.captures(candidate) {
                let kind = match &caps[1] {
                    "pre" => ContractKind::Pre,
                    "post" => ContractKind::Post,
                    _ => ContractKind::Invariant,
                };
                entries.push(ContractEntry {
                    kind,
                    condition: caps[2].trim().to_string(),
                    line: idx,
                });
            }
        }
        entries
    }

    /// 检查契约是否满足
    pub fn check(&self, code: &str, _lang: Language) -> Vec<GuardIssue> {
        let entries = self.extract(code);
        let mut issues = Vec::new();
        for entry in entries {
            if !self.condition_guarded(code, &entry) {
                let (layer_desc, suggestion) = match entry.kind {
                    ContractKind::Pre => ("前置条件", "在函数入口添加参数校验"),
                    ContractKind::Post => ("后置条件", "在函数出口添加结果断言"),
                    ContractKind::Invariant => ("不变量", "在循环/临界点添加不变量断言"),
                };
                issues.push(GuardIssue::new(
                    GuardLayer::Contract,
                    Severity::Warning,
                    format!("{} `{}` 未被实现覆盖", layer_desc, entry.condition),
                    None,
                    Some(format!("{suggestion}：assert/检查 `{}`", entry.condition)),
                ));
            }
        }
        issues
    }

    /// 判断契约条件是否被断言/校验覆盖
    fn condition_guarded(&self, code: &str, entry: &ContractEntry) -> bool {
        let cond = &entry.condition;
        // 条件中的核心标识符（如 `x > 0` → `x`）
        let key = cond
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .find(|s| !s.is_empty() && !is_number(s))
            .unwrap_or(cond);
        // 检查断言类关键字 + 关键标识符是否同时出现
        let assert_keywords = [
            "assert",
            "debug_assert",
            "assert_eq",
            "if ",
            "require(",
            "check(",
            "guard ",
        ];
        let has_assert = assert_keywords.iter().any(|kw| code.contains(kw));
        if !has_assert {
            return false;
        }
        // 断言所在行附近应包含条件关键词
        let body: Vec<&str> = code.lines().collect();
        let window_start = entry.line.saturating_sub(3);
        let window_end = (entry.line + 8).min(body.len());
        body[window_start..window_end].join("\n").contains(key)
    }
}

impl Default for ContractChecker {
    fn default() -> Self {
        Self::new()
    }
}

fn is_number(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_contracts() {
        let checker = ContractChecker::new();
        let code = "\
// @pre x > 0
// @post result >= 0
fn abs(x: i32) -> i32 { x }
";
        let entries = checker.extract(code);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, ContractKind::Pre);
        assert_eq!(entries[0].condition, "x > 0");
    }

    #[test]
    fn test_unimplemented_contract_flagged() {
        let checker = ContractChecker::new();
        let code = "\
// @pre x > 0
fn abs(x: i32) -> i32 {
    x
}
";
        let issues = checker.check(code, Language::Rust);
        assert!(!issues.is_empty());
        assert!(issues[0].message.contains("前置条件"));
    }

    #[test]
    fn test_implemented_contract_ok() {
        let checker = ContractChecker::new();
        let code = "\
// @pre x > 0
fn abs(x: i32) -> i32 {
    assert!(x > 0);
    x
}
";
        let issues = checker.check(code, Language::Rust);
        assert!(issues.is_empty(), "expected no issues, got {issues:?}");
    }

    #[test]
    fn test_python_contract() {
        let checker = ContractChecker::new();
        let code = "\
# @pre count >= 0
def process(count):
    if count < 0:
        raise ValueError('count must be >= 0')
    return count
";
        let issues = checker.check(code, Language::Python);
        assert!(issues.is_empty(), "expected no issues, got {issues:?}");
    }
}
