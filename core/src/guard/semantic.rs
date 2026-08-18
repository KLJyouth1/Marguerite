//! 语义检查（护栏第二层）
//!
//! 基于 tree-sitter AST 进行轻量语义分析：未定义变量检测。
//! 预留 LSP 客户端 trait（借鉴 TUI 工作台 LSP 诊断），
//! 可扩展接入真实 LSP 获取类型不匹配等深度诊断。

use std::collections::HashSet;

use tree_sitter::{Node, Parser};

use super::{GuardIssue, GuardLayer, Severity, SourceRange};
use crate::guard::syntax::Language;

/// LSP 诊断客户端扩展点
///
/// 实现该 trait 可接入任意 LSP（typescript-language-server、pyright 等），
/// 返回额外的语义诊断。默认使用内置轻量分析，无需外部进程。
pub trait LspDiagnostics: Send + Sync {
    /// 语言标识
    fn language(&self) -> Language;
    /// 返回语义诊断问题
    fn diagnose(&self, code: &str) -> Vec<GuardIssue>;
}

/// 语义检查器
///
/// 内置分析：收集声明标识符与使用标识符，检测未定义变量。
pub struct SemanticChecker {
    lsp: Option<Box<dyn LspDiagnostics>>,
}

impl SemanticChecker {
    pub fn new() -> Self {
        Self { lsp: None }
    }

    /// 注入 LSP 诊断客户端
    pub fn with_lsp(mut self, lsp: Box<dyn LspDiagnostics>) -> Self {
        self.lsp = Some(lsp);
        self
    }

    /// 语义检查
    pub fn check(&self, code: &str, lang: Language) -> Vec<GuardIssue> {
        let mut issues = self.undefined_vars(code, lang);
        if let Some(lsp) = &self.lsp {
            if lsp.language() == lang {
                issues.extend(lsp.diagnose(code));
            }
        }
        issues
    }

    /// 未定义变量检测
    ///
    /// 遍历 AST：声明节点收集名称，引用节点收集标识符，
    /// 减去声明与全局内置后即为未定义。
    fn undefined_vars(&self, code: &str, lang: Language) -> Vec<GuardIssue> {
        let Some(mut parser) = parser_for(lang) else {
            return vec![];
        };
        let Some(tree) = parser.parse(code, None) else {
            return vec![];
        };

        let mut declared: HashSet<String> = HashSet::new();
        let mut used: Vec<(String, SourceRange)> = Vec::new();
        collect_symbols(tree.root_node(), code, &mut declared, &mut used);

        let globals = global_identifiers(lang);
        let mut issues = Vec::new();
        for (name, range) in used {
            if !declared.contains(&name) && !globals.contains(name.as_str()) && !is_reserved(&name)
            {
                issues.push(GuardIssue::new(
                    GuardLayer::Semantic,
                    Severity::Warning,
                    format!("未定义变量 `{name}`"),
                    Some(range),
                    Some(format!("先声明 `{name}` 再使用，或检查拼写")),
                ));
            }
        }
        issues
    }
}

impl Default for SemanticChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// 为语言创建 tree-sitter parser
fn parser_for(lang: Language) -> Option<Parser> {
    let lang_fn = match lang {
        Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
    };
    let mut parser = Parser::new();
    parser.set_language(&lang_fn).ok()?;
    Some(parser)
}

/// 递归收集声明标识符与使用标识符
fn collect_symbols(
    node: Node,
    code: &str,
    declared: &mut HashSet<String>,
    used: &mut Vec<(String, SourceRange)>,
) {
    let kind = node.kind();
    if is_declaration_node(kind) {
        collect_name_identifiers(node, code, declared);
    } else if kind == "identifier" && !is_property_name(node) {
        // 引用节点：排除对象属性名与成员访问
        if let Some(name) = node_text(node, code) {
            if !name.is_empty() {
                used.push((name, node_range(node)));
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_symbols(child, code, declared, used);
    }
}

/// 判断是否为声明节点（函数/类/变量/参数/导入）
fn is_declaration_node(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration"
            | "function_definition"
            | "class_declaration"
            | "class_definition"
            | "variable_declarator"
            | "let_declaration"
            | "const_declaration"
            | "var_declaration"
            | "short_var_declaration"
            | "formal_parameters"
            | "parameters"
            | "parameter_declaration"
            | "import_statement"
            | "import_from_statement"
            | "import_declaration"
            | "use_declaration"
    )
}

/// 判断 identifier 是否为属性名/成员访问（非变量引用）
fn is_property_name(node: Node) -> bool {
    node.parent()
        .map(|p| {
            matches!(
                p.kind(),
                "property_identifier"
                    | "shorthand_property_identifier"
                    | "member_expression"
                    | "method_definition"
                    | "field_definition"
                    | "pair"
            )
        })
        .unwrap_or(false)
}

/// 从声明节点中提取名称标识符（跳过字符串路径等非标识符）
fn collect_name_identifiers(node: Node, code: &str, declared: &mut HashSet<String>) {
    let kind = node.kind();
    // import 节点：名称嵌套在子节点中，递归收集
    if kind.contains("import") {
        collect_import_identifiers(node, code, declared);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            if let Some(text) = node_text(child, code) {
                declared.insert(text);
            }
        }
    }
}

/// 递归收集 import 语句中的绑定名（排除模块路径字符串）
fn collect_import_identifiers(node: Node, code: &str, declared: &mut HashSet<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                if let Some(text) = node_text(child, code) {
                    declared.insert(text);
                }
            }
            "string" | "string_literal" | "comment" => continue,
            _ => collect_import_identifiers(child, code, declared),
        }
    }
}

/// 节点文本
fn node_text(node: Node, code: &str) -> Option<String> {
    let bytes = code.as_bytes();
    if node.start_byte() >= node.end_byte() || node.end_byte() > bytes.len() {
        return None;
    }
    std::str::from_utf8(&bytes[node.start_byte()..node.end_byte()])
        .ok()
        .map(|s| s.to_string())
}

/// 节点位置 → SourceRange
fn node_range(node: Node) -> SourceRange {
    let start = node.start_position();
    let end = node.end_position();
    SourceRange {
        start_line: start.row,
        start_column: start.column,
        end_line: end.row,
        end_column: end.column,
    }
}

/// 语言全局内置标识符
fn global_identifiers(lang: Language) -> HashSet<&'static str> {
    let mut globals = HashSet::new();
    match lang {
        Language::JavaScript | Language::TypeScript => {
            globals.extend([
                "console",
                "window",
                "document",
                "process",
                "require",
                "module",
                "exports",
                "global",
                "globalThis",
                "fetch",
                "setTimeout",
                "clearTimeout",
                "setInterval",
                "clearInterval",
                "requestAnimationFrame",
                "cancelAnimationFrame",
                "Buffer",
                "JSON",
                "Math",
                "Date",
                "Promise",
                "Object",
                "Array",
                "String",
                "Number",
                "Boolean",
                "Symbol",
                "RegExp",
                "Error",
                "TypeError",
                "Map",
                "Set",
                "WeakMap",
                "WeakSet",
                "undefined",
                "null",
                "NaN",
                "Infinity",
                "URL",
                "URLSearchParams",
                "TextEncoder",
                "TextDecoder",
                "localStorage",
                "sessionStorage",
            ]);
        }
        Language::Python => {
            globals.extend([
                "print",
                "len",
                "range",
                "str",
                "int",
                "float",
                "bool",
                "list",
                "dict",
                "set",
                "tuple",
                "enumerate",
                "zip",
                "map",
                "filter",
                "sum",
                "min",
                "max",
                "abs",
                "type",
                "isinstance",
                "Exception",
                "ValueError",
                "TypeError",
                "open",
                "None",
                "True",
                "False",
                "self",
                "super",
                "object",
            ]);
        }
        Language::Rust => {
            globals.extend([
                "println",
                "print",
                "eprintln",
                "eprint",
                "format",
                "vec",
                "Some",
                "None",
                "Ok",
                "Err",
                "String",
                "Vec",
                "Box",
                "Option",
                "Result",
                "vec!",
                "format!",
                "todo",
                "unreachable",
                "assert",
                "assert_eq",
                "panic",
                "dbg",
                "Self",
                "Fn",
                "FnMut",
                "FnOnce",
                "Iterator",
                "Clone",
                "Copy",
                "Debug",
            ]);
        }
        Language::Go => {
            globals.extend([
                "fmt", "len", "cap", "append", "make", "new", "copy", "delete", "panic", "recover",
                "print", "println", "string", "int", "float64", "bool", "byte", "rune", "error",
                "nil", "true", "false", "iota", "uint", "int32", "int64", "float32", "uint8",
                "uint16", "uint32", "uint64",
            ]);
        }
    }
    globals
}

/// 保留关键字（避免误报）
fn is_reserved(name: &str) -> bool {
    matches!(
        name,
        "if" | "else"
            | "for"
            | "while"
            | "return"
            | "function"
            | "const"
            | "let"
            | "var"
            | "class"
            | "import"
            | "export"
            | "from"
            | "def"
            | "async"
            | "await"
            | "new"
            | "this"
            | "super"
            | "typeof"
            | "instanceof"
            | "in"
            | "of"
            | "do"
            | "switch"
            | "case"
            | "break"
            | "continue"
            | "try"
            | "catch"
            | "finally"
            | "throw"
            | "yield"
            | "delete"
            | "void"
            | "with"
            | "debugger"
            | "static"
            | "interface"
            | "type"
            | "extends"
            | "implements"
            | "private"
            | "public"
            | "protected"
            | "readonly"
            | "enum"
            | "namespace"
            | "declare"
            | "abstract"
            | "as"
            | "is"
            | "keyof"
            | "never"
            | "unknown"
            | "any"
            | "assert"
            | "fn"
            | "mut"
            | "pub"
            | "struct"
            | "impl"
            | "trait"
            | "mod"
            | "use"
            | "crate"
            | "self"
            | "where"
            | "move"
            | "ref"
            | "match"
            | "loop"
            | "package"
            | "func"
            | "map"
            | "chan"
            | "go"
            | "select"
            | "defer"
            | "range"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_undefined_var_detection() {
        let checker = SemanticChecker::new();
        let issues = checker.check(
            "function foo() {\n  return bar + 1;\n}\n",
            Language::JavaScript,
        );
        assert!(!issues.is_empty());
        assert!(issues.iter().any(|i| i.message.contains("bar")));
    }

    #[test]
    fn test_declared_var_no_issue() {
        let checker = SemanticChecker::new();
        let issues = checker.check(
            "const bar = 1;\nfunction foo() {\n  return bar + 1;\n}\n",
            Language::JavaScript,
        );
        assert!(issues.is_empty(), "expected no issues, got {issues:?}");
    }

    #[test]
    fn test_global_no_issue() {
        let checker = SemanticChecker::new();
        let issues = checker.check("console.log('hi');\n", Language::JavaScript);
        assert!(issues.is_empty(), "expected no issues, got {issues:?}");
    }

    #[test]
    fn test_python_undefined() {
        let checker = SemanticChecker::new();
        let issues = checker.check("def foo():\n    return bar\n", Language::Python);
        assert!(issues.iter().any(|i| i.message.contains("bar")));
    }
}
