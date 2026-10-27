//! 网关共享工具函数（参数解析 / 信封构造 / OS 打开）。
//!
//! 本模块集中 `methods.rs` 跨方法族复用的小工具，避免每新增一个方法族就
//! 复制一遍 `param_str` 之类的样板代码。系统打开已迁到 `gateway::open_path`。

use serde_json::{json, Value};

/// 统一把 `Result<T, String>` 映射为 RPC 返回值：成功 `to_value`，失败 `{ok:false,error}`。
pub fn ok_or_err<T: serde::Serialize>(res: Result<T, String>) -> Value {
    match res {
        Ok(v) => serde_json::to_value(v).unwrap_or(json!({ "ok": false, "error": "序列化失败" })),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

/// 按 camelCase 优先、snake_case 兜底取字符串参数。
pub fn param_str<'a>(params: &'a Value, camel: &str, snake: &str, default: &'a str) -> &'a str {
    params
        .get(camel)
        .or_else(|| params.get(snake))
        .and_then(Value::as_str)
        .unwrap_or(default)
}

/// 按多个候选 key 依次取字符串参数（camel / snake / 别名皆可，首命中优先）。
///
/// 用于历史命名不一致的参数：例如前端发 `baseUrl`、部分调用方发 `baseUri`、
/// 持久化用 `base_url`，三者都要被接受，避免「字段名对不上」导致静默失败。
pub fn param_str_keys<'a>(params: &'a Value, keys: &[&str], default: &'a str) -> &'a str {
    for k in keys {
        if let Some(v) = params.get(k).and_then(Value::as_str) {
            return v;
        }
    }
    default
}

/// 按 camelCase 优先、snake_case 兜底取 i64 参数。
pub fn param_i64(params: &Value, camel: &str, snake: &str, default: i64) -> i64 {
    params
        .get(camel)
        .or_else(|| params.get(snake))
        .and_then(Value::as_i64)
        .unwrap_or(default)
}

// ───────────────────────────────────────────────────────────────────────────
// Epic A：产物台账 RPC 专用信封（Value 信封 `{code, message, data}`）
// 错误码：ARTIFACT_NOT_FOUND=1404 / ARTIFACT_OPEN_FAILED=1502 / SESSION_UNKNOWN=1401
// ───────────────────────────────────────────────────────────────────────────

/// 成功信封（code=0）。
pub(crate) fn rpc_ok(data: Value) -> Value {
    json!({ "code": 0, "message": "", "data": data })
}

/// 失败信封（携带业务错误码）。
pub(crate) fn rpc_err(code: u32, message: &str) -> Value {
    json!({ "code": code, "message": message, "data": Value::Null })
}

// 「用系统默认程序打开文件」已迁到 `gateway::open_path`（W3.2）：
// 旧实现在这里用 `cmd /c start <任意路径>`，既无工作区边界也无需可执行扩展名拦截。
// 新入口是 `open_path::open_path_checked`（校验）+ `open_path::launch_checked`（ShellExecuteW），
// 唯一调用点 `artifacts.open_in_system` 已切换；不要再在这里恢复 shell 版本。
