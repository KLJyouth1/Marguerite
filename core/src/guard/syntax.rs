//! 语法检查（护栏第一层）
//!
//! 基于 tree-sitter 的 AST 解析与语法错误检测，支持主流语言。
//! 借鉴 opencode 的语法校验思路，零依赖外部服务。

use std::collections::HashMap;
use std::sync::Mutex;

use tree_sitter::{Language as TsLanguage, Node, Parser, Point};

use super::{GuardIssue, GuardLayer, Severity, SourceRange};

/// 支持的语言
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    JavaScript,
    TypeScript,
    Python,
    Rust,
    Go,
}

impl Language {
    /// 从文件扩展名或名称解析语言
    pub fn from_name(name: &str) -> Option<Self> {
        let n = name.to_ascii_lowercase();
        match n.as_str() {
            "js" | "javascript" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "ts" | "typescript" | "tsx" => Some(Self::TypeScript),
            "py" | "python" => Some(Self::Python),
            "rs" | "rust" => Some(Self::Rust),
            "go" | "golang" => Some(Self::Go),
            _ => None,
        }
    }

    /// 从文件扩展名解析
    pub fn from_extension(ext: &str) -> Option<Self> {
        Self::from_name(ext)
    }

    /// 显示名称
    pub fn display(&self) -> &'static str {
        match self {
            Self::JavaScript => "JavaScript",
            Self::TypeScript => "TypeScript",
            Self::Python => "Python",
            Self::Rust => "Rust",
            Self::Go => "Go",
        }
    }
}

/// 语法检查器
///
/// 为每种语言维护一个 tree-sitter Parser，解析代码后遍历 AST，
/// 检测 ERROR / MISSING 节点并生成修复建议。
/// 使用 Mutex 包装 Parser，保证 `check(&self)` 线程安全且无需外部可变借用。
pub struct SyntaxChecker {
    parsers: HashMap<Language, Mutex<Parser>>,
}

impl SyntaxChecker {
    pub fn new() -> Self {
        let mut parsers = HashMap::new();
        let mut register = |lang: Language, lang_fn: TsLanguage| {
            let mut parser = Parser::new();
            if parser.set_language(&lang_fn).is_ok() {
                parsers.insert(lang, Mutex::new(parser));
            }
        };
        register(
            Language::JavaScript,
            tree_sitter_javascript::LANGUAGE.into(),
        );
        register(
            Language::TypeScript,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        );
        register(Language::Python, tree_sitter_python::LANGUAGE.into());
        register(Language::Rust, tree_sitter_rust::LANGUAGE.into());
        register(Language::Go, tree_sitter_go::LANGUAGE.into());
        Self { parsers }
    }

    /// 检查代码语法，返回语法问题列表
    pub fn check(&self, code: &str, lang: Language) -> Vec<GuardIssue> {
        let Some(parser) = self.parsers.get(&lang) else {
            return vec![];
        };
        let mut parser = parser.lock().unwrap();
        let Some(tree) = parser.parse(code, None) else {
            return vec![];
        };
        let root = tree.root_node();
        let mut issues = Vec::new();
        Self::walk(root, code, &mut issues);
        issues
    }

    /// 遍历 AST 收集错误节点
    fn walk(node: Node, code: &str, issues: &mut Vec<GuardIssue>) {
        if node.is_error() {
            issues.push(Self::error_issue(node, code));
        } else if node.is_missing() {
            issues.push(Self::missing_issue(node));
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk(child, code, issues);
        }
    }

    /// ERROR 节点 → 问题
    fn error_issue(node: Node, code: &str) -> GuardIssue {
        let range = node_range(node);
        let snippet = extract_snippet(code, node);
        let suggestion = suggest_fix(node);
        GuardIssue::new(
            GuardLayer::Syntax,
            Severity::Error,
            format!("语法错误{}", snippet_desc(snippet)),
            Some(range),
            suggestion,
        )
    }

    /// MISSING 节点 → 问题
    fn missing_issue(node: Node) -> GuardIssue {
        GuardIssue::new(
            GuardLayer::Syntax,
            Severity::Error,
            format!("缺少 token: {}", node.kind()),
            Some(node_range(node)),
            Some(format!(
                "在位置 {}:{} 补全 {}",
                node.start_position().row + 1,
                node.start_position().column + 1,
                node.kind()
            )),
        )
    }
}

impl Default for SyntaxChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// 节点位置 → SourceRange
fn node_range(node: Node) -> SourceRange {
    let start: Point = node.start_position();
    let end: Point = node.end_position();
    SourceRange {
        start_line: start.row,
        start_column: start.column,
        end_line: end.row,
        end_column: end.column,
    }
}

/// 提取错误位置附近的代码片段
fn extract_snippet(code: &str, node: Node) -> Option<String> {
    let bytes = code.as_bytes();
    let start = node.start_byte();
    let end = node.end_byte().min(bytes.len());
    if start >= end {
        return None;
    }
    let text = &code[start..end];
    let text = text.lines().next().unwrap_or(text);
    Some(text.chars().take(60).collect())
}

fn snippet_desc(snippet: Option<String>) -> String {
    snippet.map(|s| format!(": `{}`", s)).unwrap_or_default()
}

/// 根据错误节点类型生成通用修复建议
fn suggest_fix(node: Node) -> Option<String> {
    let kind = node.kind();
    let parent = node.parent().map(|p| p.kind());
    match (kind, parent) {
        ("ERROR", Some("statement_block")) => {
            Some("检查该语句是否完整（括号/分号/关键字拼写）".to_string())
        }
        ("ERROR", Some("expression_statement")) => {
            Some("检查表达式语法（括号配对、运算符）".to_string())
        }
        (_, Some("import_statement")) | (_, Some("export_statement")) => {
            Some("检查 import/export 语句语法".to_string())
        }
        _ => Some("检查该 token 附近的语法（括号配对、缺失符号）".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_from_name() {
        assert_eq!(Language::from_name("js"), Some(Language::JavaScript));
        assert_eq!(Language::from_name("TS"), Some(Language::TypeScript));
        assert_eq!(Language::from_name("py"), Some(Language::Python));
        assert_eq!(Language::from_name("rs"), Some(Language::Rust));
        assert_eq!(Language::from_name("go"), Some(Language::Go));
        assert_eq!(Language::from_name("java"), None);
    }

    #[test]
    fn test_valid_code_no_error() {
        let checker = SyntaxChecker::new();
        let issues = checker.check("const x = 1;\nconsole.log(x);\n", Language::JavaScript);
        assert!(issues.is_empty(), "expected no issues, got {issues:?}");
    }

    #[test]
    fn test_invalid_code_detected() {
        let checker = SyntaxChecker::new();
        // 缺少右括号
        let issues = checker.check("function foo() {\n  return 1;\n", Language::JavaScript);
        assert!(!issues.is_empty());
        assert!(issues.iter().all(|i| i.severity == Severity::Error));
    }

    #[test]
    fn test_python_syntax() {
        let checker = SyntaxChecker::new();
        assert!(checker
            .check("def foo(x):\n    return x\n", Language::Python)
            .is_empty());
        // 语法错误：缺少冒号
        assert!(!checker
            .check("def foo(x)\n    return x\n", Language::Python)
            .is_empty());
    }

    #[test]
    fn test_unsupported_lang_no_panic() {
        let checker = SyntaxChecker::new();
        // 不存在的语言不会 panic，返回空
        let _ = checker.check("x", Language::Go);
    }
}
