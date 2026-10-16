//! 内置基础文件工具（v2 工作台）
//!
//! 供调用链 CLI 集成使用的最小工具集：读文件 / 编辑文件 / 代码搜索 /
//! 测试运行 / 失败诊断。后两者为**真实执行**（按清单文件探测项目生态，
//! 带超时执行测试命令），支撑「改代码 → 跑测试 → 迭代」闭环的验证步。

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;

use crate::agent::tool_cycle::ToolResult;
use crate::mcp::registry::{ToolExecutor, ToolRegistry};
use crate::proc::SilentCommand;

/// 注册全部内置基础工具
pub fn register_builtin_tools(registry: &mut ToolRegistry) {
    for tool in [
        Box::new(ReadFileTool) as Box<dyn ToolExecutor>,
        Box::new(EditFileTool),
        Box::new(SearchCodeTool),
        Box::new(RunTestsTool::new()),
        Box::new(DebugTool::new()),
    ] {
        registry.register_local(tool);
    }
}

fn args_str<'a>(args: &'a HashMap<String, String>, key: &str) -> Result<&'a str> {
    args.get(key)
        .map(|s| s.as_str())
        .ok_or_else(|| anyhow::anyhow!("缺少参数: {key}"))
}

/// 读取文件内容
pub struct ReadFileTool;

#[async_trait]
impl ToolExecutor for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "读取指定路径的文件内容"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}})
    }
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let path = args_str(args, "path")?;
        match std::fs::read_to_string(path) {
            Ok(content) => Ok(ToolResult::Success {
                data: content,
                metadata: HashMap::new(),
            }),
            Err(e) => Ok(ToolResult::Error {
                message: format!("读取失败: {e}"),
                code: "IO_ERROR".to_string(),
            }),
        }
    }
}

/// 写入/覆盖文件内容
pub struct EditFileTool;

#[async_trait]
impl ToolExecutor for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }
    fn description(&self) -> &str {
        "将内容写入指定路径（覆盖写入）"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {
            "path": {"type": "string"}, "content": {"type": "string"}
        }})
    }
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let path = args_str(args, "path")?;
        let content = args.get("content").map(|s| s.as_str()).unwrap_or("");
        match std::fs::write(path, content) {
            Ok(()) => Ok(ToolResult::Success {
                data: format!("已写入 {}（{} 字节）", path, content.len()),
                metadata: HashMap::new(),
            }),
            Err(e) => Ok(ToolResult::Error {
                message: format!("写入失败: {e}"),
                code: "IO_ERROR".to_string(),
            }),
        }
    }
}

/// 在目录内递归搜索包含关键词的代码行
pub struct SearchCodeTool;

#[async_trait]
impl ToolExecutor for SearchCodeTool {
    fn name(&self) -> &str {
        "search_code"
    }
    fn description(&self) -> &str {
        "在目录内递归搜索包含关键词的代码行"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {
            "query": {"type": "string"}, "path": {"type": "string"}
        }})
    }
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let query = args_str(args, "query")?;
        let root = args.get("path").map(|s| s.as_str()).unwrap_or(".");
        let hits = search_dir(root, query, 0);
        if hits.is_empty() {
            return Ok(ToolResult::Success {
                data: format!("未找到包含 {:?} 的代码行", query),
                metadata: HashMap::new(),
            });
        }
        Ok(ToolResult::Success {
            data: hits.join("\n"),
            metadata: HashMap::new(),
        })
    }
}

/// 递归搜索（深度 ≤ 6，忽略隐藏目录与二进制目录）
fn search_dir(dir: &str, query: &str, depth: usize) -> Vec<String> {
    if depth > 6 {
        return Vec::new();
    }
    let mut hits = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return hits;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        if path.is_dir() {
            hits.extend(search_dir(&path.to_string_lossy(), query, depth + 1));
        } else if let Ok(content) = std::fs::read_to_string(&path) {
            for (idx, line) in content.lines().enumerate() {
                if line.contains(query) {
                    hits.push(format!("{}:{}: {}", path.display(), idx + 1, line.trim()));
                }
            }
        }
    }
    hits
}

// ─────────────────────────────────────────────────────────────────────────────
// 测试运行 / 失败诊断（真实执行）
//
// 历史缺陷：这两个工具是 `NOT_IMPLEMENTED` 占位，而 `refactor()` / `fix()` 调用链
// 以它们收尾 → 企业「改代码 → 跑测试 → 迭代」路径的**验证步恒失败**。
// 现按清单文件探测项目生态并真实执行测试命令（带超时、输出有界）。
// ─────────────────────────────────────────────────────────────────────────────

/// 测试命令默认超时（秒）。
pub(crate) const DEFAULT_TEST_TIMEOUT_SECS: u64 = 600;
/// 回传输出上限（字符）：避免把整份测试日志塞进上下文。
pub(crate) const MAX_TEST_OUTPUT_CHARS: usize = 8000;

/// 探测到的测试命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TestCommand {
    /// 项目根（探测到清单文件的目录）
    pub root: std::path::PathBuf,
    /// 可执行程序
    pub program: String,
    /// 固定参数
    pub args: Vec<String>,
    /// 生态名（人类可读）
    pub kind: &'static str,
}

impl TestCommand {
    /// 人类可读的完整命令（用于回显与诊断）。
    pub(crate) fn display(&self) -> String {
        let mut s = self.program.clone();
        for a in &self.args {
            s.push(' ');
            s.push_str(a);
        }
        s
    }
}

/// 从 `start` 起向上寻找项目根，按清单文件判定测试命令。
///
/// 支持：Cargo（Rust）/ npm·vitest·jest（Node）/ pytest（Python）/ go test（Go）。
/// 未识别到任何受支持生态时返回 `None`（调用方给出可操作错误，而非静默失败）。
pub(crate) fn detect_test_command(start: &std::path::Path) -> Option<TestCommand> {
    let mut dir: std::path::PathBuf = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };
    loop {
        if dir.join("Cargo.toml").is_file() {
            return Some(TestCommand {
                root: dir,
                program: "cargo".into(),
                args: vec!["test".into()],
                kind: "cargo",
            });
        }
        if dir.join("go.mod").is_file() {
            return Some(TestCommand {
                root: dir,
                program: "go".into(),
                args: vec!["test".into(), "./...".into()],
                kind: "go",
            });
        }
        if dir.join("pyproject.toml").is_file()
            || dir.join("pytest.ini").is_file()
            || dir.join("tox.ini").is_file()
        {
            return Some(TestCommand {
                root: dir,
                program: "python".into(),
                args: vec!["-m".into(), "pytest".into(), "-q".into()],
                kind: "pytest",
            });
        }
        if dir.join("package.json").is_file() {
            if let Some(cmd) = node_test_command(&dir) {
                return Some(cmd);
            }
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Node 生态：优先 `package.json` 的 `scripts.test`；否则按本地 vitest/jest 配置选择。
fn node_test_command(root: &std::path::Path) -> Option<TestCommand> {
    let raw = std::fs::read_to_string(root.join("package.json")).ok()?;
    let pkg: serde_json::Value = serde_json::from_str(&raw).ok()?;
    if let Some(script) = pkg
        .get("scripts")
        .and_then(|s| s.get("test"))
        .and_then(|t| t.as_str())
    {
        if !script.trim().is_empty() {
            return Some(TestCommand {
                root: root.to_path_buf(),
                program: "npm".into(),
                args: vec!["test".into()],
                kind: "npm",
            });
        }
    }
    // 无 test 脚本时按本地测试框架配置推断；`npx --no-install` 只用已装依赖、绝不联网拉包。
    for (cfg, args, kind) in [
        ("vitest.config.ts", vec!["--no-install", "vitest", "run"], "vitest"),
        ("vitest.config.js", vec!["--no-install", "vitest", "run"], "vitest"),
        ("vitest.config.mts", vec!["--no-install", "vitest", "run"], "vitest"),
        ("jest.config.js", vec!["--no-install", "jest"], "jest"),
        ("jest.config.ts", vec!["--no-install", "jest"], "jest"),
        ("jest.config.mjs", vec!["--no-install", "jest"], "jest"),
    ] {
        if root.join(cfg).is_file() {
            return Some(TestCommand {
                root: root.to_path_buf(),
                program: "npx".into(),
                args: args.into_iter().map(String::from).collect(),
                kind,
            });
        }
    }
    None
}

/// 测试执行结果。
#[derive(Debug, Clone)]
pub(crate) enum TestRunOutcome {
    Completed {
        success: bool,
        exit_code: i32,
        output: String,
    },
    Timeout,
    SpawnError(String),
}

/// 测试命令执行器抽象（便于测试注入 mock）。
#[async_trait]
pub(crate) trait TestRunner: Send + Sync {
    async fn run(&self, cmd: &TestCommand, timeout: std::time::Duration) -> TestRunOutcome;
}

/// 生产实现：真实派生进程，带超时。
pub(crate) struct SystemTestRunner;

#[async_trait]
impl TestRunner for SystemTestRunner {
    async fn run(&self, cmd: &TestCommand, timeout: std::time::Duration) -> TestRunOutcome {
        let mut command = build_process(cmd);
        command
            .current_dir(&cmd.root)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        match tokio::time::timeout(timeout, command.output()).await {
            Err(_) => TestRunOutcome::Timeout,
            Ok(Err(e)) => TestRunOutcome::SpawnError(format!("无法启动 `{}`: {e}", cmd.program)),
            Ok(Ok(out)) => {
                let mut text = String::from_utf8_lossy(&out.stdout).to_string();
                let err = String::from_utf8_lossy(&out.stderr);
                if !err.trim().is_empty() {
                    text.push_str("\n--- stderr ---\n");
                    text.push_str(&err);
                }
                TestRunOutcome::Completed {
                    success: out.status.success(),
                    exit_code: out.status.code().unwrap_or(-1),
                    output: text,
                }
            }
        }
    }
}

/// 构造进程命令。
///
/// Windows 上 `npm`/`npx`/`yarn`/`pnpm` 是 `.cmd` 包装脚本，而 `CreateProcess`
/// 不解析 `PATHEXT`，直接 `Command::new("npm")` 会失败 —— 需经 `cmd /C` 调用。
/// 参数均为本模块常量（非模型可控），无注入面。
fn build_process(cmd: &TestCommand) -> tokio::process::Command {
    #[cfg(windows)]
    {
        if matches!(cmd.program.as_str(), "npm" | "npx" | "yarn" | "pnpm") {
            let mut c = tokio::process::Command::new("cmd");
            c.silent().arg("/C").arg(&cmd.program).args(&cmd.args);
            return c;
        }
    }
    let mut c = tokio::process::Command::new(&cmd.program);
    c.silent().args(&cmd.args);
    c
}

/// 截断输出（按字符，UTF-8 安全）。
fn truncate_output(s: &str) -> String {
    let total = s.chars().count();
    if total <= MAX_TEST_OUTPUT_CHARS {
        return s.to_string();
    }
    let head: String = s.chars().take(MAX_TEST_OUTPUT_CHARS).collect();
    format!("{head}\n…（输出已截断，共 {total} 字符）")
}

/// 从测试输出中提取失败线索（失败用例 / 断言 / 错误行），有界。
pub(crate) fn extract_failure_hints(output: &str) -> Vec<String> {
    const MAX_HINTS: usize = 40;
    const MARKERS: &[&str] = &[
        "failed",
        "panicked",
        "assertion",
        "error:",
        "error[",
        "traceback",
        "failures:",
        "---- ",
    ];
    let mut hints = Vec::new();
    for line in output.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let lower = t.to_ascii_lowercase();
        if MARKERS.iter().any(|m| lower.contains(m)) {
            hints.push(t.to_string());
            if hints.len() >= MAX_HINTS {
                break;
            }
        }
    }
    hints
}

/// 解析 `file`/`path` 参数为探测起点（缺省为当前工作目录）。
fn detect_start(args: &HashMap<String, String>) -> std::path::PathBuf {
    match args.get("file").or_else(|| args.get("path")) {
        Some(p) if !p.trim().is_empty() => std::path::PathBuf::from(p),
        _ => std::env::current_dir().unwrap_or_default(),
    }
}

fn timeout_from(args: &HashMap<String, String>) -> std::time::Duration {
    std::time::Duration::from_secs(
        args.get("timeout_sec")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TEST_TIMEOUT_SECS),
    )
}

/// 运行测试（真实执行：探测项目生态 → 带超时执行 → 结构化回报）。
pub struct RunTestsTool {
    runner: std::sync::Arc<dyn TestRunner>,
}

impl RunTestsTool {
    pub fn new() -> Self {
        Self {
            runner: std::sync::Arc::new(SystemTestRunner),
        }
    }
    /// 注入自定义执行器（测试用）。
    #[cfg(test)]
    pub(crate) fn with_runner(runner: std::sync::Arc<dyn TestRunner>) -> Self {
        Self { runner }
    }
}

impl Default for RunTestsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolExecutor for RunTestsTool {
    fn name(&self) -> &str {
        "run_tests"
    }
    fn description(&self) -> &str {
        "运行项目测试：按清单文件探测生态（Cargo/npm/vitest/jest/pytest/go）并真实执行，返回通过/失败与输出摘要。参数: file(可选，项目内任一文件或目录), timeout_sec(可选)"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {
            "file": {"type": "string", "description": "项目内任一文件或目录（用于定位项目根）"},
            "timeout_sec": {"type": "number", "description": "超时秒数，默认 600"}
        }})
    }
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let start = detect_start(args);
        let Some(cmd) = detect_test_command(&start) else {
            return Ok(ToolResult::Error {
                message: format!(
                    "未识别到受支持的测试生态（在 {} 及其上级目录均未找到 Cargo.toml / package.json / pyproject.toml / go.mod）",
                    start.display()
                ),
                code: "NO_TEST_RUNNER".to_string(),
            });
        };
        let timeout = timeout_from(args);
        match self.runner.run(&cmd, timeout).await {
            TestRunOutcome::Timeout => Ok(ToolResult::Error {
                message: format!("测试超时（{}s）：{}", timeout.as_secs(), cmd.display()),
                code: "TEST_TIMEOUT".to_string(),
            }),
            TestRunOutcome::SpawnError(e) => Ok(ToolResult::Error {
                message: format!("{e}（请确认已安装 {}）", cmd.program),
                code: "TEST_SPAWN_ERROR".to_string(),
            }),
            TestRunOutcome::Completed {
                success,
                exit_code,
                output,
            } => {
                let body = truncate_output(output.trim());
                if success {
                    let mut metadata = HashMap::new();
                    metadata.insert("kind".to_string(), cmd.kind.to_string());
                    Ok(ToolResult::Success {
                        data: format!("[{}] 测试通过：{}\n{}", cmd.kind, cmd.display(), body),
                        metadata,
                    })
                } else {
                    Ok(ToolResult::Error {
                        message: format!(
                            "[{}] 测试失败（exit {}）：{}\n{}",
                            cmd.kind,
                            exit_code,
                            cmd.display(),
                            body
                        ),
                        code: "TEST_FAILED".to_string(),
                    })
                }
            }
        }
    }
}

/// 失败诊断：运行测试并**浓缩**失败线索（失败用例名、断言与错误行）。
///
/// 说明：这不是断点式调试器，而是「改代码 → 跑测试 → 迭代」闭环里的**失败后诊断步**——
/// 把冗长测试日志压缩成可读线索供修复环节直接使用。测试仍失败时返回错误（诚实：验证未过）。
pub struct DebugTool {
    runner: std::sync::Arc<dyn TestRunner>,
}

impl DebugTool {
    pub fn new() -> Self {
        Self {
            runner: std::sync::Arc::new(SystemTestRunner),
        }
    }
    /// 注入自定义执行器（测试用）。
    #[cfg(test)]
    pub(crate) fn with_runner(runner: std::sync::Arc<dyn TestRunner>) -> Self {
        Self { runner }
    }
}

impl Default for DebugTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolExecutor for DebugTool {
    fn name(&self) -> &str {
        "debug"
    }
    fn description(&self) -> &str {
        "测试失败诊断：运行项目测试并提取失败用例/断言/错误行（不是断点调试器）。参数: file(可选), timeout_sec(可选)"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {
            "file": {"type": "string", "description": "项目内任一文件或目录（用于定位项目根）"},
            "timeout_sec": {"type": "number", "description": "超时秒数，默认 600"}
        }})
    }
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let start = detect_start(args);
        let Some(cmd) = detect_test_command(&start) else {
            return Ok(ToolResult::Error {
                message: format!(
                    "未识别到受支持的测试生态（探测起点 {}）",
                    start.display()
                ),
                code: "NO_TEST_RUNNER".to_string(),
            });
        };
        let timeout = timeout_from(args);
        match self.runner.run(&cmd, timeout).await {
            TestRunOutcome::Timeout => Ok(ToolResult::Error {
                message: format!("测试超时（{}s）：{}", timeout.as_secs(), cmd.display()),
                code: "TEST_TIMEOUT".to_string(),
            }),
            TestRunOutcome::SpawnError(e) => Ok(ToolResult::Error {
                message: format!("{e}（请确认已安装 {}）", cmd.program),
                code: "TEST_SPAWN_ERROR".to_string(),
            }),
            TestRunOutcome::Completed {
                success,
                exit_code,
                output,
            } => {
                if success {
                    return Ok(ToolResult::Success {
                        data: format!("[{}] 测试通过，无需诊断：{}", cmd.kind, cmd.display()),
                        metadata: HashMap::new(),
                    });
                }
                let hints = extract_failure_hints(&output);
                let body = if hints.is_empty() {
                    truncate_output(output.trim())
                } else {
                    hints.join("\n")
                };
                Ok(ToolResult::Error {
                    message: format!(
                        "[{}] 测试失败（exit {}）：{}\n诊断线索（{} 条）：\n{}",
                        cmd.kind,
                        exit_code,
                        cmd.display(),
                        hints.len(),
                        body
                    ),
                    code: "TEST_FAILED".to_string(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_read_file_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("marguerite_builtin_{}", uuid::Uuid::new_v4()));
        std::fs::write(&tmp, "hello world").unwrap();
        let tool = ReadFileTool;
        let mut args = HashMap::new();
        args.insert("path".into(), tmp.to_string_lossy().to_string());
        let r = tool.execute(&args).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => assert_eq!(data, "hello world"),
            _ => panic!("应成功"),
        }
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn test_edit_file_write() {
        let tmp =
            std::env::temp_dir().join(format!("marguerite_builtin_w_{}", uuid::Uuid::new_v4()));
        let tool = EditFileTool;
        let mut args = HashMap::new();
        args.insert("path".into(), tmp.to_string_lossy().to_string());
        args.insert("content".into(), "new content".into());
        let r = tool.execute(&args).await.unwrap();
        assert!(matches!(r, ToolResult::Success { .. }));
        assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "new content");
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn test_read_missing_file_returns_error() {
        let tool = ReadFileTool;
        let mut args = HashMap::new();
        args.insert("path".into(), "/nonexistent/xxx.rs".into());
        let r = tool.execute(&args).await.unwrap();
        assert!(matches!(r, ToolResult::Error { .. }));
    }

    /// 探测：Cargo / Go / pytest / npm 各生态，未识别时返回 None。
    #[test]
    fn test_detect_test_command_by_manifest() {
        let base = std::env::temp_dir().join(format!("mgt_detect_{}", uuid::Uuid::new_v4()));

        let rust = base.join("rustproj");
        std::fs::create_dir_all(rust.join("src")).unwrap();
        std::fs::write(rust.join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let c = detect_test_command(&rust.join("src").join("lib.rs")).expect("应识别 cargo");
        assert_eq!(c.kind, "cargo");
        assert_eq!(c.program, "cargo");
        assert_eq!(c.root, rust);

        let go = base.join("goproj");
        std::fs::create_dir_all(&go).unwrap();
        std::fs::write(go.join("go.mod"), "module x\n").unwrap();
        let c = detect_test_command(&go).expect("应识别 go");
        assert_eq!(c.kind, "go");
        assert_eq!(c.args, vec!["test".to_string(), "./...".to_string()]);

        let py = base.join("pyproj");
        std::fs::create_dir_all(&py).unwrap();
        std::fs::write(py.join("pyproject.toml"), "[project]\nname=\"x\"\n").unwrap();
        assert_eq!(detect_test_command(&py).expect("应识别 pytest").kind, "pytest");

        let node = base.join("nodeproj");
        std::fs::create_dir_all(&node).unwrap();
        std::fs::write(
            node.join("package.json"),
            r#"{"scripts":{"test":"vitest run"}}"#,
        )
        .unwrap();
        let c = detect_test_command(&node).expect("应识别 npm");
        assert_eq!(c.kind, "npm");
        assert_eq!(c.program, "npm");

        let empty = base.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(detect_test_command(&empty).is_none(), "无清单应返回 None");

        let _ = std::fs::remove_dir_all(&base);
    }

    struct MockRunner {
        outcome: TestRunOutcome,
    }

    #[async_trait]
    impl TestRunner for MockRunner {
        async fn run(&self, _cmd: &TestCommand, _t: std::time::Duration) -> TestRunOutcome {
            self.outcome.clone()
        }
    }

    fn cargo_project() -> (std::path::PathBuf, std::path::PathBuf) {
        let base = std::env::temp_dir().join(format!("mgt_rt_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(base.join("src")).unwrap();
        std::fs::write(base.join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let file = base.join("src").join("lib.rs");
        (base, file)
    }

    /// 真实执行路径（注入 mock 执行器）：通过 → Success；失败 → Error/TEST_FAILED；超时 → TEST_TIMEOUT。
    #[tokio::test]
    async fn test_run_tests_reports_pass_fail_and_timeout() {
        let (base, file) = cargo_project();
        let mut args = HashMap::new();
        args.insert("file".into(), file.to_string_lossy().to_string());

        let ok = RunTestsTool::with_runner(Arc::new(MockRunner {
            outcome: TestRunOutcome::Completed {
                success: true,
                exit_code: 0,
                output: "test result: ok. 3 passed".into(),
            },
        }));
        match ok.execute(&args).await.unwrap() {
            ToolResult::Success { data, metadata } => {
                assert!(data.contains("测试通过"), "data={data}");
                assert_eq!(metadata.get("kind").map(String::as_str), Some("cargo"));
            }
            _ => panic!("通过用例应返回 Success"),
        }

        let bad = RunTestsTool::with_runner(Arc::new(MockRunner {
            outcome: TestRunOutcome::Completed {
                success: false,
                exit_code: 101,
                output: "FAILED".into(),
            },
        }));
        match bad.execute(&args).await.unwrap() {
            ToolResult::Error { code, message } => {
                assert_eq!(code, "TEST_FAILED");
                assert!(message.contains("exit 101"), "message={message}");
            }
            _ => panic!("失败用例应返回 Error"),
        }

        let to = RunTestsTool::with_runner(Arc::new(MockRunner {
            outcome: TestRunOutcome::Timeout,
        }));
        match to.execute(&args).await.unwrap() {
            ToolResult::Error { code, .. } => assert_eq!(code, "TEST_TIMEOUT"),
            _ => panic!("超时用例应返回 Error"),
        }

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 未识别生态 → 可操作错误 NO_TEST_RUNNER（而非静默失败）。
    #[tokio::test]
    async fn test_run_tests_without_manifest_reports_no_runner() {
        let tool = RunTestsTool::with_runner(Arc::new(MockRunner {
            outcome: TestRunOutcome::Timeout, // 不应被调用
        }));
        // 指向一个不存在的路径 → 其上级链无清单 → NO_TEST_RUNNER
        let mut args = HashMap::new();
        args.insert(
            "file".into(),
            std::env::temp_dir()
                .join(format!("mgt_none_{}", uuid::Uuid::new_v4()))
                .join("x.rs")
                .to_string_lossy()
                .to_string(),
        );
        match tool.execute(&args).await.unwrap() {
            ToolResult::Error { code, .. } => assert_eq!(code, "NO_TEST_RUNNER"),
            _ => panic!("应返回 NO_TEST_RUNNER"),
        }
    }

    /// debug：失败时提取失败用例与断言线索（有界）。
    #[tokio::test]
    async fn test_debug_extracts_failure_hints() {
        let (base, file) = cargo_project();
        let mut args = HashMap::new();
        args.insert("file".into(), file.to_string_lossy().to_string());
        let tool = DebugTool::with_runner(Arc::new(MockRunner {
            outcome: TestRunOutcome::Completed {
                success: false,
                exit_code: 101,
                output: "running 2 tests\ntest a ... ok\ntest b ... FAILED\n\
                         assertion `left == right` failed\nerror: test failed"
                    .into(),
            },
        }));
        match tool.execute(&args).await.unwrap() {
            ToolResult::Error { code, message } => {
                assert_eq!(code, "TEST_FAILED");
                assert!(message.contains("test b ... FAILED"), "message={message}");
                assert!(message.contains("assertion"), "应含断言线索: {message}");
            }
            _ => panic!("失败时 debug 应返回 Error（诚实：验证未过）"),
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_extract_failure_hints_is_bounded_and_selective() {
        let noisy: String = (0..500).map(|i| format!("line {i} FAILED\n")).collect();
        assert!(extract_failure_hints(&noisy).len() <= 40, "线索数应有界");
        assert!(extract_failure_hints("all good, nothing here").is_empty());
    }

    #[tokio::test]
    async fn test_register_builtin_tools() {
        let mut registry = ToolRegistry::new();
        register_builtin_tools(&mut registry);
        assert_eq!(registry.list_tools().len(), 5);
        assert!(registry.has_tool("read_file"));
        assert!(registry.has_tool("edit_file"));
        let _ = Arc::new(registry);
    }
}
