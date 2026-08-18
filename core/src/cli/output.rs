//! CLI 输出与错误契约（Phase 1：AC-03 / AC-04）
//!
//! 统一机器可读（JSON）与文本输出，错误带结构化 hint，分层退出码。
//! 约定：stdout = 业务数据；stderr = 错误/日志；JSON 结构稳定：
//! - 成功：`{"ok":true,"data":...,"meta":{...}}`
//! - 错误：`{"ok":false,"error":{"code","message","hint","retryable"},"meta":{...}}`

use std::process;

/// 分层退出码（AC-04 配套，与 `man 8 sysexits`/常见 CLI 约定对齐）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    /// 成功
    Success = 0,
    /// 用法错误（未知选项 / 未知子命令 / 缺必选参数）
    Usage = 2,
    /// 认证失败（API Key 缺失或无效）
    Auth = 3,
    /// 资源未找到
    NotFound = 4,
    /// 临时失败（网络 / 限流，可重试）
    Temporary = 5,
    /// 冲突（并发 / 状态不一致）
    Conflict = 9,
}

impl ExitCode {
    /// 退出码整数值
    pub fn code(self) -> i32 {
        self as i32
    }
}

/// 以 JSON 输出成功结果：`{"ok":true,"data":...,"meta":{...}}`
pub fn print_json_ok(data: impl serde::Serialize) {
    let payload = serde_json::json!({
        "ok": true,
        "data": data,
        "meta": { "schema_version": "1.0", "cli": "marguerite" }
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{\"ok\":true}".to_string())
    );
}

/// 以 JSON 输出错误到 stderr：`{"ok":false,"error":{...},"meta":{...}}`
pub fn print_json_err(code: ExitCode, message: &str, hint: Option<&str>, retryable: bool) {
    let payload = serde_json::json!({
        "ok": false,
        "error": {
            "code": code.code(),
            "message": message,
            "hint": hint,
            "retryable": retryable
        },
        "meta": { "schema_version": "1.0", "cli": "marguerite" }
    });
    eprintln!(
        "{}",
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );
}

/// 以指定退出码结束进程（用于调用方显式控制退出语义）
pub fn exit_with(code: ExitCode) -> ! {
    process::exit(code.code());
}
