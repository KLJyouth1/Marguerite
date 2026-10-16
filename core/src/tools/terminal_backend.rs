//! 终端后端切换/探测 RPC 入口（工具层）
//!
//! 与 `tools::terminal`（常驻 PTY 会话）不同：本模块暴露**一次性行执行**
//! 能力，供前端一键把后续命令切到 WSL 或 Docker 内执行（让 Windows 用户也能
//! 用上 Linux 工具链）。底层走 `terminal::backend` 的自动探测 + 护栏裁决。
//!
//! # RPC 方法
//!
//! - `terminal.backends.status`：列出全部后端的可用状态 + 当前解析结果
//! - `terminal.backend.exec {backend?, cwd?, cmd}`：在指定后端下执行一条命令，
//!   返回 stdout / stderr / exit_code / backend
//! - `terminal.backend.auto {cwd?, cmd}`：用当前机器最优后端执行（WSL > Docker > Native）
//!
//! # 安全
//!
//! 所有执行都过 `terminal::backend::run_command` 的分层护栏，不可信的 `cwd`
//! 参数（来自 RPC）先经 `safe_resolve` 收敛进工作区再传入。

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::terminal::backend::{
    run_command, status_all, ResolvedBackend, TerminalBackend, TerminalBackendConfig,
};
use crate::tools::fs_path::safe_resolve;

fn workspace() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn resolve_cwd(params: &Value) -> std::result::Result<PathBuf, Value> {
    let ws = workspace();
    match params
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        Some(raw) => safe_resolve(&ws, raw).map_err(|e| {
            json!({
                "ok": false,
                "code": "PATH_ESCAPES_WORKSPACE",
                "error": format!("cwd 越出工作区，已拒绝: {e}"),
            })
        }),
        None => Ok(ws),
    }
}

fn cfg_from_params(params: &Value) -> TerminalBackendConfig {
    let mut cfg = TerminalBackendConfig::default();
    if let Some(b) = params
        .get("backend")
        .and_then(|v| v.as_str())
        .and_then(TerminalBackend::from_str)
    {
        cfg = cfg.with_backend(b);
    }
    if let Some(d) = params.get("wslDistro").and_then(|v| v.as_str()) {
        cfg = cfg.with_distro(d);
    }
    if let Some(img) = params.get("dockerImage").and_then(|v| v.as_str()) {
        cfg = cfg.with_image(img);
    }
    cfg
}

/// `terminal.backends.status`：列出全部后端的可用状态。
pub fn backends_status(params: &Value) -> Value {
    let cfg = cfg_from_params(params);
    json!({ "ok": true, "status": status_all(&cfg) })
}

/// `terminal.backend.exec`：在指定（或用户指定）后端下执行一条命令。
pub fn backend_exec(params: &Value) -> Value {
    let cfg = cfg_from_params(params);
    let cwd = match resolve_cwd(params) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let cmd = params
        .get("cmd")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if cmd.is_empty() {
        return json!({ "ok": false, "error": "cmd 不可为空" });
    }
    let resolved = ResolvedBackend::resolve(&cfg);
    if let Err(reason) = &resolved.available {
        return json!({
            "ok": false,
            "code": "LINUX_BACKEND_UNAVAILABLE",
            "error": reason,
            "backend": resolved.backend.name(),
        });
    }
    match run_command(resolved.backend, &cfg, &cwd, &cmd) {
        Ok(res) => json!({
            "ok": true,
            "stdout": res.stdout,
            "stderr": res.stderr,
            "exitCode": res.exit_code,
            "backend": res.backend.name(),
            "durationMs": res.duration_ms,
            "warning": resolved.available.err(),
        }),
        Err(e) => json!({
            "ok": false,
            "error": e,
            "backend": resolved.backend.name(),
        }),
    }
}

/// `terminal.backend.auto`：用当前机器最优后端执行（自动探测，强制使用解析结果）。
pub fn backend_auto(params: &Value) -> Value {
    let mut cfg = cfg_from_params(params);
    cfg.backend = None; // 强制自动探测
    let cwd = match resolve_cwd(params) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let cmd = params
        .get("cmd")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if cmd.is_empty() {
        return json!({ "ok": false, "error": "cmd 不可为空" });
    }
    let resolved = ResolvedBackend::resolve(&cfg);
    if let Err(reason) = &resolved.available {
        return json!({
            "ok": false,
            "code": "LINUX_BACKEND_UNAVAILABLE",
            "error": reason,
            "backend": resolved.backend.name(),
        });
    }
    match run_command(resolved.backend, &cfg, &cwd, &cmd) {
        Ok(res) => json!({
            "ok": true,
            "stdout": res.stdout,
            "stderr": res.stderr,
            "exitCode": res.exit_code,
            "backend": res.backend.name(),
            "durationMs": res.duration_ms,
            "warning": resolved.available.err(),
        }),
        Err(e) => json!({
            "ok": false,
            "error": e,
            "backend": resolved.backend.name(),
        }),
    }
}
