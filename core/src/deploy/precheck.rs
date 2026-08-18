//! 上传前语法预检（共享模块）
//!
//! 按扩展名路由到系统解释器做语法校验，未命中的扩展名直接放行：
//! - `.py`             → `python -m py_compile <file>`
//! - `.js/.mjs/.cjs/.ts` → `node --check <file>`
//!
//! 用 `std::process::Command` 直接执行解释器（不走 shell，免疫注入），
//! 带超时与输出截断。返回结构化结果 `PrecheckOutcome`，供各部署后端在
//! 上传前统一拦截语法错误的代码产物。

use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

/// 语法预检超时
const TIMEOUT: Duration = Duration::from_secs(60);
/// 错误输出截断长度
const MAX_OUTPUT: usize = 2048;

/// 预检结果
#[derive(Debug, Clone)]
pub struct PrecheckOutcome {
    /// 被检查文件的展示路径
    pub file: String,
    /// 是否通过（语法正确、未知扩展名、或解释器可用但校验通过）
    pub ok: bool,
    /// 失败原因（ok 为 false 时非空）
    pub error: Option<String>,
}

/// 报文扩展名到解释器命令的路由（程序 + 固定前缀参数）。
/// 返回 None 表示该扩展名无需校验（直接放行）。
fn checker_for(ext: &str) -> Option<(&'static str, &'static [&'static str])> {
    match ext {
        "py" => Some(("python", &["-m", "py_compile"][..])),
        "js" | "mjs" | "cjs" | "ts" => Some(("node", &["--check"][..])),
        _ => None,
    }
}

/// 截断过长的输出，避免把整段日志塞进错误信息。
fn truncate(s: &str) -> String {
    if s.len() <= MAX_OUTPUT {
        s.to_string()
    } else {
        format!("{}…（已截断，共 {} 字符）", &s[..MAX_OUTPUT], s.len())
    }
}

/// 运行命令并等待，带超时。超时后终止子进程并返回超时错误。
fn run_with_timeout(command: &mut Command, timeout: Duration) -> std::io::Result<Output> {
    let mut child = command.spawn()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait()? {
            Some(_) => return child.wait_with_output(),
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("语法预检超时（>{timeout:?}）"),
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// 对单个文件做语法预检。未知扩展名直接返回 ok。
pub fn precheck(path: &Path) -> PrecheckOutcome {
    let file = path.display().to_string();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let Some((program, prefix)) = checker_for(&ext) else {
        return PrecheckOutcome {
            file,
            ok: true,
            error: None,
        };
    };

    let mut command = Command::new(program);
    command.args(prefix).arg(path);
    let outcome = match run_with_timeout(&mut command, TIMEOUT) {
        Err(e) => PrecheckOutcome {
            file,
            ok: false,
            error: Some(format!("语法预检无法执行: {e}")),
        },
        Ok(out) => {
            if out.status.success() {
                PrecheckOutcome {
                    file,
                    ok: true,
                    error: None,
                }
            } else {
                let msg = String::from_utf8_lossy(&out.stderr);
                let msg = if msg.trim().is_empty() {
                    String::from_utf8_lossy(&out.stdout).to_string()
                } else {
                    msg.to_string()
                };
                PrecheckOutcome {
                    file,
                    ok: false,
                    error: Some(truncate(msg.trim())),
                }
            }
        }
    };
    outcome
}

/// 批量预检：任一文件失败即整体拒绝，返回首个失败结果。
pub fn precheck_many(paths: &[&Path]) -> Result<(), PrecheckOutcome> {
    for p in paths {
        let r = precheck(p);
        if !r.ok {
            return Err(r);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, content: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mgt_precheck_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    /// 含语法错误的 .py 应被拒绝，并报告语法错误
    #[test]
    fn rejects_broken_python() {
        let p = temp_file("broken.py", b"def foo(x)\n    return x\n");
        let r = precheck(&p);
        assert!(!r.ok, "语法错误的 .py 应被拒绝");
        assert!(r.error.is_some(), "应报告语法错误信息");
        let _ = std::fs::remove_file(&p);
    }

    /// 合法的 .py 应通过
    #[test]
    fn accepts_valid_python() {
        let p = temp_file("ok.py", b"def foo(x):\n    return x\nprint(foo(1))\n");
        let r = precheck(&p);
        assert!(r.ok, "合法 .py 应通过: {:?}", r.error);
        let _ = std::fs::remove_file(&p);
    }

    /// 含语法错误的 .js 应被拒绝
    #[test]
    fn rejects_broken_js() {
        let p = temp_file("broken.js", b"function foo( {\n  return 1;\n}\n");
        let r = precheck(&p);
        assert!(!r.ok, "语法错误的 .js 应被拒绝");
        assert!(r.error.is_some());
        let _ = std::fs::remove_file(&p);
    }

    /// 合法的 .js 应通过
    #[test]
    fn accepts_valid_js() {
        let p = temp_file(
            "ok.js",
            b"function foo() { return 1; }\nconsole.log(foo());\n",
        );
        let r = precheck(&p);
        assert!(r.ok, "合法 .js 应通过: {:?}", r.error);
        let _ = std::fs::remove_file(&p);
    }

    /// 未知扩展名直接通过（不校验）
    #[test]
    fn unknown_extension_passes() {
        let p = temp_file("app.bin", b"\x7f\x45\x4c\x46\x00\xff");
        let r = precheck(&p);
        assert!(r.ok, "未知扩展名应直接放行");
        let _ = std::fs::remove_file(&p);
    }

    /// 批量预检：任一失败即整体拒绝
    #[test]
    fn precheck_many_rejects_on_first_failure() {
        let good = temp_file("good.py", b"x = 1\n");
        let bad = temp_file("bad.py", b"def f(\n");
        let err = precheck_many(&[&good, &bad]).unwrap_err();
        assert!(!err.ok);
        assert_eq!(err.file, bad.display().to_string());
        let _ = std::fs::remove_file(&good);
        let _ = std::fs::remove_file(&bad);
    }
}
