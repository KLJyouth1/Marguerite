//! 隔离代码执行工具（code_exec）：code interpreter 沙箱
//!
//! 对齐主流工作台（CLI 工作台 / OpenAI Code Interpreter / 沙箱代码执行）的
//! 「隔离代码解释器」能力，与 `shell_exec`（OS 级 shell）互补：
//!
//! - **无 shell**：直接用 `Command::new(interpreter)` 跑代码文件，代码/UUID 文件名
//!   作为参数，天然免疫 shell 注入（不走 `sh -c` / `cmd /C`）。
//! - **临时目录隔离**：代码写到系统临时目录，运行 `current_dir` 固定为临时目录，
//!   不污染工作区、不访问工作区文件。
//! - **资源限制**：超时强制终止（`kill_on_drop`），stdout/stderr 长度截断防 token 膨胀。
//! - **依赖白名单**：fail-closed——仅允许列表内解释器二进制（python/python3/node），
//!   未装解释器时返回明确错误而非静默失败。

use std::collections::HashMap;
#[cfg(test)]
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::json;
use tokio::process::Command;

use crate::agent::tool_cycle::ToolResult;

/// 单次代码执行结果
#[derive(Debug, Clone)]
pub struct CodeOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// 默认超时
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
/// 单流输出截断上限（防 token 膨胀）
const MAX_OUTPUT_CHARS: usize = 16_000;

/// 语言 → 解释器候选（按优先级；Windows 常见 `python`，Unix 常见 `python3`）
fn interpreters(language: &str) -> &'static [&'static str] {
    match language {
        "js" | "javascript" | "node" => &["node"],
        _ => &["python", "python3"],
    }
}

fn file_ext(language: &str) -> &'static str {
    match language {
        "js" | "javascript" | "node" => "js",
        _ => "py",
    }
}

fn truncate(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push_str("… [已截断]");
    }
    out
}

/// 在临时目录中执行解释器代码，返回结构化输出。
async fn run_code(language: &str, code: &str, timeout: Duration) -> Result<CodeOutput> {
    let interps = interpreters(language);
    let ext = file_ext(language);

    // 临时目录隔离：代码与运行目录都在系统临时区，不触碰工作区。
    let dir = std::env::temp_dir().join(format!("marguerite_code_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).map_err(|e| anyhow!("创建临时目录失败: {e}"))?;
    let file = dir.join(format!("main.{ext}"));
    std::fs::write(&file, code).map_err(|e| anyhow!("写入代码文件失败: {e}"))?;

    let mut last_err = None;
    for interp in interps {
        // 直接 `Command::new`，无 shell 参与 → 免疫 shell 注入。
        let mut cmd = Command::new(interp);
        cmd.arg(&file)
            .current_dir(&dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true);
        // Python: 强制 UTF-8 输出，避免中文乱码
        if interp.starts_with("python") {
            cmd.env("PYTHONIOENCODING", "utf-8");
            cmd.env("PYTHONUTF8", "1");
        }
        #[cfg(windows)]
        {
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                last_err = Some(anyhow!("无法启动解释器 `{interp}`: {e}"));
                continue;
            }
        };
        let out = match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(anyhow!("解释器执行失败: {e}"));
            }
            Err(_) => {
                // kill_on_drop 自动终止超时子进程
                let _ = std::fs::remove_dir_all(&dir);
                return Err(anyhow!("代码执行超时（>{timeout:?}），已强制终止"));
            }
        };
        let _ = std::fs::remove_dir_all(&dir);
        return Ok(CodeOutput {
            stdout: truncate(&String::from_utf8_lossy(&out.stdout), MAX_OUTPUT_CHARS),
            stderr: truncate(&String::from_utf8_lossy(&out.stderr), MAX_OUTPUT_CHARS),
            exit_code: out.status.code().unwrap_or(-1),
        });
    }
    let _ = std::fs::remove_dir_all(&dir);
    Err(last_err.unwrap_or_else(|| anyhow!("没有可用的解释器（{interps:?}）")))
}

/// 构造 `code_exec` 工具（供 `register_builtin_tools` 挂载）。
pub fn code_exec_tool() -> Box<dyn crate::mcp::registry::ToolExecutor> {
    crate::tools::builtin::make_local_tool(
        "code_exec",
        "隔离代码执行：在临时目录以解释器（python/js）运行代码，有超时与输出截断，不污染工作区。\
         执行前会做静态风险扫描：命中高危调用（os.system / subprocess / shutil.rmtree / eval / 网络 / JS child_process 等）\
         默认拒绝并返回 SANDBOX_DENIED；需要预演时设 dry_run=true（只扫描不执行）；确认无误要放行时设 allow_unsafe=true。\
         参数: language(python|js,默认python), code(必填), timeout_sec(可选,默认15), dry_run(可选), allow_unsafe(可选)",
        json!({
            "type": "object",
            "properties": {
                "language": { "type": "string", "description": "python 或 js，默认 python" },
                "code": { "type": "string", "description": "要执行的源代码" },
                "timeout_sec": { "type": "number", "description": "超时秒数，默认 15" },
                "dry_run": { "type": "boolean", "description": "true=只做风险扫描、不执行代码（默认 false）" },
                "allow_unsafe": { "type": "boolean", "description": "true=高危调用也执行（默认 false，即 fail-closed）" }
            },
            "required": ["code"]
        }),
        Arc::new(move |args: &HashMap<String, String>| {
            let code = args.get("code").cloned().unwrap_or_default();
            let language = args.get("language").cloned().unwrap_or_else(|| "python".into());
            let timeout = match args.get("timeout_sec").and_then(|t| t.parse::<u64>().ok()) {
                Some(s) => Duration::from_secs(s.min(120)),
                None => DEFAULT_TIMEOUT,
            };
            let dry_run = parse_bool_flag(args.get("dry_run"));
            let allow_unsafe = parse_bool_flag(args.get("allow_unsafe"));
            Box::pin(async move {
                if code.trim().is_empty() {
                    return Ok(ToolResult::Error {
                        message: "code_exec 需要非空 code".into(),
                        code: "INVALID_PARAMS".into(),
                    });
                }

                // ── 静态风险扫描（P4·T2 受控层）──────────────────────────────
                // 为什么放在执行**之前**而不是靠沙箱兜底：临时目录 + 无 shell 只能保证
                // "不污染工作区"，挡不住 `shutil.rmtree('/')`、`os.system('curl …')`、
                // 死循环拉满 CPU 这类**内部**破坏。静态扫描是唯一能在进程启动前刹车的位置。
                let signals = scan_risk(&language, &code);
                let blocked: Vec<&RiskSignal> =
                    signals.iter().filter(|s| s.severity == Severity::High).collect();

                if dry_run {
                    return Ok(ToolResult::Success {
                        data: render_scan_report(&signals, true, 0),
                        metadata: HashMap::new(),
                    });
                }
                if !blocked.is_empty() && !allow_unsafe {
                    // fail-closed：默认拒绝。错误码独立，前端/上层可区分"被安全策略拒绝"
                    // 与"执行失败"，而不是笼统报错。
                    return Ok(ToolResult::Error {
                        message: format!(
                            "代码包含 {} 处高危调用，已拒绝执行（fail-closed）。\n{}\n\
                             如确需执行，显式传 allow_unsafe=true；只想看风险可传 dry_run=true。",
                            blocked.len(),
                            render_scan_report(&signals, false, 0)
                        ),
                        code: "SANDBOX_DENIED".into(),
                    });
                }

                match run_code(&language, &code, timeout).await {
                    Ok(o) => {
                        let mut data = String::new();
                        if !signals.is_empty() {
                            // 已放行但仍有风险信号：把警告留在输出里，便于事后审计
                            data.push_str(&render_scan_report(&signals, false, 0));
                            data.push_str("\n---\n");
                        }
                        data.push_str(&format!(
                            "[exit {}]\n--- stdout ---\n{}\n--- stderr ---\n{}",
                            o.exit_code, o.stdout, o.stderr
                        ));
                        Ok(ToolResult::Success {
                            data,
                            metadata: HashMap::new(),
                        })
                    }
                    Err(e) => Ok(ToolResult::Error {
                        message: e.to_string(),
                        code: "EXEC_ERROR".into(),
                    }),
                }
            })
        }),
    )
}

/// 宽松解析布尔旗标：工具参数是以字符串形式传进来的，`"true"`/`"1"`/`"yes"` 都算真。
///
/// 为什么不只认 `"true"`：不同模型/前端序列化布尔的方式不一致（`1`/`true`/`True`），
/// 只认一种会让 `dry_run` 静默失效——而"静默失效的安全开关"比没有开关更危险。
fn parse_bool_flag(raw: Option<&String>) -> bool {
    match raw.map(|s| s.trim().to_ascii_lowercase()) {
        Some(v) => matches!(v.as_str(), "true" | "1" | "yes" | "on"),
        None => false,
    }
}

/// 风险等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// 高危：默认拒绝执行（破坏文件系统 / 起子进程 / 动态执行 / 外联）
    High,
    /// 中危：放行但记录（可能的副作用，需要人看一眼）
    Medium,
}

/// 一条静态风险信号。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiskSignal {
    /// 命中的源码行号（1-based），便于模型/人直接定位
    pub line: usize,
    /// 命中的模式（如 `os.system`）
    pub pattern: &'static str,
    pub severity: Severity,
    /// 为什么危险（一句话，说后果而不是说规则）
    pub why: &'static str,
}

impl RiskSignal {
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "line": self.line,
            "pattern": self.pattern,
            "severity": match self.severity { Severity::High => "high", Severity::Medium => "medium" },
            "why": self.why,
        })
    }
}

/// 高危模式表（顺序即报告顺序；先匹配到的行只记一次最高级）。
///
/// 判据取舍：宁可**少拦**也不误伤——例如 `open(` 只记中危，因为读文件是代码解释器的
/// 正常用法；而 `os.remove` / `shutil.rmtree` 一律高危，因为解释器场景下没有正当理由。
const HIGH_RISK_PY: &[(&str, &str)] = &[
    ("os.system", "起子进程执行系统命令，可绕过沙箱做任意操作"),
    ("os.popen", "起子进程并读取输出，等同任意命令执行"),
    ("subprocess", "起子进程执行外部程序"),
    ("pty.spawn", "起伪终端执行交互命令"),
    ("shutil.rmtree", "递归删除目录，且不可恢复"),
    ("os.remove", "删除文件"),
    ("os.unlink", "删除文件"),
    ("os.rmdir", "删除目录"),
    ("os.removedirs", "递归删除空目录树"),
    ("eval(", "动态求值字符串代码，静态分析无法审计其行为"),
    ("exec(", "动态执行字符串代码，静态分析无法审计其行为"),
    ("__import__(", "动态导入模块，可绕过导入白名单"),
    ("importlib.import_module", "动态导入模块，可绕过导入白名单"),
    ("socket.", "直接开网络连接（外联）"),
    ("urllib.request", "发起 HTTP 请求（外联）"),
    ("requests.", "发起 HTTP 请求（外联）"),
    ("httpx.", "发起 HTTP 请求（外联）"),
    ("paramiko", "SSH 远程操作"),
    ("ctypes.", "调用本地库，可绕过所有 Python 层限制"),
];

const MEDIUM_RISK_PY: &[(&str, &str)] = &[
    ("open(", "文件读写（在临时目录内是正常的，但会改动磁盘）"),
    ("while True", "死循环风险（有超时兜底，但会占满单核）"),
    ("input(", "等待标准输入；此处 stdin 为空，会挂起到超时"),
    ("pickle.loads", "反序列化不可信数据可导致任意代码执行"),
];

const HIGH_RISK_JS: &[(&str, &str)] = &[
    ("child_process", "起子进程执行外部程序"),
    ("execSync", "同步执行 shell 命令"),
    ("spawnSync", "起子进程"),
    ("fs.rmSync", "删除文件/目录，且不可恢复"),
    ("fs.unlinkSync", "删除文件"),
    ("fs.rmdirSync", "删除目录"),
    ("eval(", "动态求值字符串代码"),
    ("new Function(", "动态构造函数，静态分析无法审计"),
    ("process.exit", "强制结束进程"),
    ("require('net')", "直接开网络连接（外联）"),
    ("require(\"net\")", "直接开网络连接（外联）"),
    ("require('http')", "发起 HTTP 请求（外联）"),
    ("require(\"http\")", "发起 HTTP 请求（外联）"),
];

const MEDIUM_RISK_JS: &[(&str, &str)] = &[
    ("fs.writeFile", "写文件（临时目录内正常，但会改动磁盘）"),
    ("fs.readFile", "读文件"),
    ("while (true)", "死循环风险"),
];

/// 风险模式表：每项为 `(标记, 说明)`。
///
/// 单独取别名是因为 `scan_risk` 里同时持有「高风险表 + 中风险表」两个切片，
/// 内联写出来是 `(&[(&str, &str)], &[(&str, &str)])` —— 两层元组嵌套，
/// 读的人要先在心里展开才看得懂在选什么。
type RiskPatterns = &'static [(&'static str, &'static str)];

/// 静态扫描：把源码按行扫一遍，返回命中的风险信号。
///
/// 一行可能命中多个模式：只保留**最高级**（High 优先），避免同一行刷出 5 条噪声。
pub fn scan_risk(language: &str, code: &str) -> Vec<RiskSignal> {
    let is_js = matches!(language, "js" | "javascript" | "node");
    let (high, medium): (RiskPatterns, RiskPatterns) = if is_js {
        (HIGH_RISK_JS, MEDIUM_RISK_JS)
    } else {
        (HIGH_RISK_PY, MEDIUM_RISK_PY)
    };

    let mut out = Vec::new();
    for (idx, line) in code.lines().enumerate() {
        // 跳过纯注释行：注释里提到 `os.system` 不算风险（否则文档示例全被拦）
        let trimmed = line.trim_start();
        let is_comment = trimmed.starts_with('#') || trimmed.starts_with("//");
        if is_comment {
            continue;
        }
        if let Some((pattern, why)) = high.iter().find(|(p, _)| line.contains(p)) {
            out.push(RiskSignal {
                line: idx + 1,
                pattern,
                severity: Severity::High,
                why,
            });
            continue;
        }
        if let Some((pattern, why)) = medium.iter().find(|(p, _)| line.contains(p)) {
            out.push(RiskSignal {
                line: idx + 1,
                pattern,
                severity: Severity::Medium,
                why,
            });
        }
    }
    out
}

/// 渲染扫描报告（给模型/人看的固定格式）。
fn render_scan_report(signals: &[RiskSignal], dry_run: bool, _executed: i32) -> String {
    if signals.is_empty() {
        return if dry_run {
            "风险扫描: 未发现高危/中危调用（dry_run=true，代码未执行）".to_string()
        } else {
            String::new()
        };
    }
    let high = signals.iter().filter(|s| s.severity == Severity::High).count();
    let medium = signals.len() - high;
    let mut s = if dry_run {
        format!(
            "风险扫描: 命中 {} 条（高危 {high} / 中危 {medium}）｜dry_run=true，代码未执行",
            signals.len()
        )
    } else {
        format!(
            "风险扫描: 命中 {} 条（高危 {high} / 中危 {medium}）",
            signals.len()
        )
    };
    for sig in signals {
        let tag = match sig.severity {
            Severity::High => "HIGH",
            Severity::Medium => "MED",
        };
        s.push_str(&format!(
            "\n  [{}] line {} `{}` — {}",
            tag, sig.line, sig.pattern, sig.why
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::registry::ToolRegistry;
    use crate::tools::builtin::register_builtin_tools;

    fn setup() -> (ToolRegistry, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("marguerite_code_tool_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut reg = ToolRegistry::new();
        register_builtin_tools(&mut reg, &dir, None, None, None).unwrap();
        (reg, dir)
    }

    async fn exec(reg: &ToolRegistry, language: &str, code: &str) -> ToolResult {
        let mut m = HashMap::new();
        m.insert("language".into(), language.into());
        m.insert("code".into(), code.into());
        reg.execute_local_tool("code_exec", &m).await.unwrap()
    }

    // ── P4·T2 受控层 ─────────────────────────────────────────────────────

    #[test]
    fn scan_flags_high_risk_python() {
        let code = "import shutil\nshutil.rmtree('/')\n";
        let s = scan_risk("python", code);
        assert_eq!(s.len(), 1, "{s:?}");
        assert_eq!(s[0].pattern, "shutil.rmtree");
        assert_eq!(s[0].severity, Severity::High);
        assert_eq!(s[0].line, 2, "行号必须是 1-based 且指向真实行");
    }

    #[test]
    fn scan_allows_pure_computation() {
        // 最常见也最合法的用法：纯计算。不许误报。
        assert!(scan_risk("python", "print(6*7)\nsum(range(10))\n").is_empty());
        assert!(scan_risk("js", "console.log(6*7)\n").is_empty());
    }

    #[test]
    fn scan_skips_comment_lines() {
        // 注释里提到危险函数名（文档/示例）不该被拦，否则教模型的代码全红
        let code = "# 不要用 os.system('x')\nprint(1)\n";
        assert!(scan_risk("python", code).is_empty(), "注释行应跳过");
    }

    #[test]
    fn scan_reports_only_highest_severity_per_line() {
        // 同一行同时含 open( 与 eval(：只报 High，避免一行刷出多条噪声
        let code = "eval(open('x').read())\n";
        let s = scan_risk("python", code);
        assert_eq!(s.len(), 1, "{s:?}");
        assert_eq!(s[0].severity, Severity::High);
    }

    #[test]
    fn scan_covers_js_high_risk() {
        let s = scan_risk("js", "const cp = require('child_process')\n");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].severity, Severity::High);
    }

    #[test]
    fn scan_flags_network_and_dynamic_import() {
        let s = scan_risk("python", "import urllib.request\n__import__('os')\n");
        assert_eq!(s.len(), 2, "{s:?}");
        assert!(s.iter().all(|x| x.severity == Severity::High));
    }

    #[test]
    fn bool_flag_accepts_common_serializations() {
        // 不同前端/模型序列化布尔的方式不同；只认 "true" 会让安全开关静默失效
        for v in ["true", "TRUE", "1", "yes", "on"] {
            assert!(parse_bool_flag(Some(&v.to_string())), "{v} 应判真");
        }
        for v in ["false", "0", "no", "", "off"] {
            assert!(!parse_bool_flag(Some(&v.to_string())), "{v} 应判假");
        }
        assert!(!parse_bool_flag(None));
    }

    #[tokio::test]
    async fn high_risk_code_is_denied_by_default() {
        let (reg, dir) = setup();
        let mut m = HashMap::new();
        m.insert("language".into(), "python".into());
        // 用 shutil.rmtree 而不是 os.system：前者不依赖外部程序，判定稳定
        m.insert("code".into(), "import shutil\nshutil.rmtree('x')\n".into());
        let r = reg.execute_local_tool("code_exec", &m).await.unwrap();
        assert!(
            matches!(&r, ToolResult::Error { code, .. } if code == "SANDBOX_DENIED"),
            "高危代码必须 fail-closed: {r:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn dry_run_does_not_execute_and_reports_risk() {
        let (reg, dir) = setup();
        let mut m = HashMap::new();
        m.insert("language".into(), "python".into());
        m.insert("dry_run".into(), "true".into());
        // 这行代码若真执行会创建文件；dry_run 下必须不存在
        m.insert(
            "code".into(),
            "import shutil\nopen('SHOULD_NOT_EXIST','w').write('x')\nshutil.rmtree('y')\n"
                .into(),
        );
        let r = reg.execute_local_tool("code_exec", &m).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                assert!(data.contains("dry_run=true，代码未执行"), "{data}");
                assert!(data.contains("HIGH"), "应报出高危: {data}");
                // 行号必须指向真实行：本用例里 open( 在第 2 行、rmtree 在第 3 行
                assert!(data.contains("line 2"), "应带 open( 的行号: {data}");
                assert!(data.contains("line 3"), "应带 rmtree 的行号: {data}");
            }
            other => panic!("dry_run 应成功返回扫描报告: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn allow_unsafe_bypasses_gate_but_keeps_warning() {
        // 放行时警告必须留在输出里（审计需要），不能静默执行
        let (reg, dir) = setup();
        let mut m = HashMap::new();
        m.insert("language".into(), "python".into());
        m.insert("allow_unsafe".into(), "1".into());
        m.insert("timeout_sec".into(), "10".into());
        // 必须真的含高危模式才会产生警告：`import shutil` 本身无害（不拦），
        // 用 `eval('1+1')` —— HIGH 级但执行结果无害，适合验证"放行 + 留痕"。
        m.insert("code".into(), "print(eval('1+1'))\n".into());
        let r = reg.execute_local_tool("code_exec", &m).await.unwrap();
        // 本机可能没装 python（Error/Timeout）：三种终态都可接受，
        // 但**不能**是 SANDBOX_DENIED —— 那说明 allow_unsafe 没生效。
        match &r {
            ToolResult::Error { code, .. } => {
                assert_ne!(*code, "SANDBOX_DENIED", "allow_unsafe 后不应再被安全层拒绝");
            }
            ToolResult::Success { data, .. } => {
                assert!(data.contains("风险扫描"), "放行时仍应保留风险警告: {data}");
            }
            ToolResult::Timeout { .. } => {}
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn registers_code_exec() {
        let (reg, dir) = setup();
        assert!(reg.has_tool("code_exec"), "code_exec 应已注册");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn requires_code() {
        let (reg, dir) = setup();
        let m = HashMap::new();
        let r = reg.execute_local_tool("code_exec", &m).await.unwrap();
        assert!(matches!(r, ToolResult::Error { code, .. } if code == "INVALID_PARAMS"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    #[ignore = "依赖本机 python/node；手动运行以验证真实解释器执行"]
    async fn live_python_and_js() {
        let (reg, dir) = setup();
        // Python 真实执行
        let r = exec(
            &reg,
            "python",
            "import sys\nprint('PY', sys.version_info[:2])\nprint(6*7)",
        )
        .await;
        if let ToolResult::Success { data, .. } = r {
            assert!(data.contains("PY"), "python 输出异常: {data}");
            assert!(data.contains("42"), "python 计算异常: {data}");
        } else {
            panic!("python 应成功: {r:?}");
        }
        // JS 真实执行
        let r = exec(&reg, "js", "console.log('JS', 6*7)").await;
        if let ToolResult::Success { data, .. } = r {
            assert!(data.contains("JS"), "js 输出异常: {data}");
            assert!(data.contains("42"), "js 计算异常: {data}");
        } else {
            panic!("js 应成功: {r:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    #[ignore = "依赖本机解释器；手动运行验证超时强制终止"]
    async fn live_timeout_kills() {
        let (reg, dir) = setup();
        let mut m = HashMap::new();
        m.insert("language".into(), "python".into());
        m.insert("timeout_sec".into(), "1".into());
        m.insert(
            "code".into(),
            "import time\ntime.sleep(30)\nprint('never')".into(),
        );
        let r = reg.execute_local_tool("code_exec", &m).await.unwrap();
        assert!(
            matches!(&r, ToolResult::Error { code, .. } if code == "EXEC_ERROR"),
            "应超时返回 EXEC_ERROR: {r:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
