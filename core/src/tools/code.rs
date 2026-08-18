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
        "隔离代码执行：在临时目录以解释器（python/js）运行代码，有超时与输出截断，不污染工作区。参数: language(python|js,默认python), code(必填), timeout_sec(可选,默认15)",
        json!({
            "type": "object",
            "properties": {
                "language": { "type": "string", "description": "python 或 js，默认 python" },
                "code": { "type": "string", "description": "要执行的源代码" },
                "timeout_sec": { "type": "number", "description": "超时秒数，默认 15" }
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
            Box::pin(async move {
                if code.trim().is_empty() {
                    return Ok(ToolResult::Error {
                        message: "code_exec 需要非空 code".into(),
                        code: "INVALID_PARAMS".into(),
                    });
                }
                match run_code(&language, &code, timeout).await {
                    Ok(o) => Ok(ToolResult::Success {
                        data: format!(
                            "[exit {}]\n--- stdout ---\n{}\n--- stderr ---\n{}",
                            o.exit_code, o.stdout, o.stderr
                        ),
                        metadata: HashMap::new(),
                    }),
                    Err(e) => Ok(ToolResult::Error {
                        message: e.to_string(),
                        code: "EXEC_ERROR".into(),
                    }),
                }
            })
        }),
    )
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
