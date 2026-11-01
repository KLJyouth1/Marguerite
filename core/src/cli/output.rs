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

/// 构造 `meta` 对象：先放契约字段，再并入调用方给的字段。
///
/// `extra` 不是 object 时**不合并** —— 把标量塞进 `meta` 会变成 `"meta": 42`，
/// 而下游一律假设 `meta` 是对象，「成功但解析崩」是最难查的一类故障。
/// 调用方传 `Value::Null` / 标量等于「没有额外字段」，不是错误。
pub fn meta_with(extra: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    let mut meta = serde_json::Map::new();
    meta.insert(
        "schema_version".to_string(),
        serde_json::Value::String("1.0".to_string()),
    );
    meta.insert(
        "cli".to_string(),
        serde_json::Value::String("marguerite".to_string()),
    );
    if let serde_json::Value::Object(more) = extra {
        for (k, v) in more {
            meta.insert(k, v);
        }
    }
    meta
}

/// 以 JSON 输出成功结果，并把调用方字段并入 `meta`（`{ok,data,meta}` 契约不变）。
///
/// 这是 [`print_json_ok`] 的**增量**版本：老调用点一个都不用改，
/// 只有需要往 `meta` 里带额外信息（例如逐轮 `cost_usd`、压缩统计）的地方换用它。
pub fn print_json_ok_with_meta(data: impl serde::Serialize, extra: serde_json::Value) {
    let payload = serde_json::json!({
        "ok": true,
        "data": data,
        "meta": serde_json::Value::Object(meta_with(extra))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_keeps_contract_fields() {
        let m = meta_with(serde_json::Value::Null);
        assert_eq!(m.get("schema_version").and_then(|v| v.as_str()), Some("1.0"));
        assert_eq!(m.get("cli").and_then(|v| v.as_str()), Some("marguerite"));
        assert_eq!(m.len(), 2, "没有额外字段时只有契约字段");
    }

    #[test]
    fn meta_merges_extra_object() {
        let m = meta_with(serde_json::json!({ "cost_usd": 0.42, "trace": "abc" }));
        assert_eq!(m.get("cost_usd").and_then(|v| v.as_f64()), Some(0.42));
        assert_eq!(m.get("trace").and_then(|v| v.as_str()), Some("abc"));
        assert_eq!(m.len(), 4);
    }

    #[test]
    fn meta_ignores_non_object_extra() {
        // 标量 / 数组 / null 都不进 meta：宁可少给，也不能让 meta 变成非对象
        for extra in [
            serde_json::json!(42),
            serde_json::json!("oops"),
            serde_json::json!([1, 2]),
            serde_json::Value::Null,
        ] {
            let m = meta_with(extra);
            assert_eq!(m.len(), 2, "非 object 不得合并任何键");
        }
    }

    #[test]
    fn meta_allows_caller_to_override_contract_fields() {
        // 现有行为：后写覆盖。记录在这里，避免以后有人「顺手加个保护」而悄悄改掉语义
        let m = meta_with(serde_json::json!({ "schema_version": "2.0" }));
        assert_eq!(m.get("schema_version").and_then(|v| v.as_str()), Some("2.0"));
    }
}
