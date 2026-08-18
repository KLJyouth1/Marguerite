//! ACP 消息协议层 —— JSON-RPC 2.0 信封（差距② 薄壳）。
//!
//! 职责仅限：解析入站 JSON-RPC 请求、规范化、构造响应。
//! 本模块**不涉及任何网络 / LLM / agent 调用**；agent 调用由 `session::AcpSession`
//! 通过注入的 `AgentInvoker` 转发。
//!
//! 约束（nexustar-api 设计 §7）：
//! - 回环优先：ACP 经本地 sidecar 暴露，禁止突破 127.0.0.1（由上层网络层保证）。
//! - 零新增依赖：仅用 serde_json。

use serde_json::{Map, Value};

/// JSON-RPC 协议版本（固定 "2.0"）。
pub const JSONRPC_VERSION: &str = "2.0";

// ---- JSON-RPC 标准错误码 ----
pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

/// ACP 协议层错误，携带 JSON-RPC 错误码。
#[derive(Debug, Clone)]
pub struct AcpError {
    /// JSON-RPC 标准错误码
    pub code: i64,
    /// 错误消息
    pub message: String,
    /// 可选附加数据
    pub data: Option<Value>,
}

impl std::fmt::Display for AcpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AcpError({}): {}", self.code, self.message)
    }
}

impl std::error::Error for AcpError {}

impl AcpError {
    /// 构造协议错误
    pub fn new(code: i64, message: impl Into<String>, data: Option<Value>) -> Self {
        Self {
            code,
            message: message.into(),
            data,
        }
    }
}

/// 规范化后的 JSON-RPC 请求。
#[derive(Debug, Clone)]
pub struct AcpRequest {
    /// 协议版本
    pub jsonrpc: String,
    /// 请求 id（无则为通知）
    pub id: Option<Value>,
    /// 方法名
    pub method: String,
    /// 参数对象
    pub params: Map<String, Value>,
}

impl AcpRequest {
    /// JSON-RPC 语义：无 `id` 视为通知（无需响应）。
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }
}

/// 从已解析的 JSON 值解析为 `AcpRequest`。
pub fn parse_value(raw: &Value) -> Result<AcpRequest, AcpError> {
    if !raw.is_object() {
        return Err(AcpError::new(
            INVALID_REQUEST,
            "request must be a JSON object",
            None,
        ));
    }
    let obj = raw.as_object().unwrap();
    let version = obj.get("jsonrpc").and_then(|v| v.as_str()).unwrap_or("");
    if version != JSONRPC_VERSION {
        return Err(AcpError::new(
            INVALID_REQUEST,
            "jsonrpc must be '2.0'",
            None,
        ));
    }
    let method = obj.get("method").and_then(|v| v.as_str()).unwrap_or("");
    if method.is_empty() {
        return Err(AcpError::new(
            INVALID_REQUEST,
            "method must be a non-empty string",
            None,
        ));
    }
    let id = obj.get("id").cloned();
    if let Some(id_val) = &id {
        if !id_val.is_string() && !id_val.is_number() && !id_val.is_boolean() && !id_val.is_null() {
            return Err(AcpError::new(
                INVALID_REQUEST,
                "id must be a JSON scalar or null",
                None,
            ));
        }
    }
    let params = match obj.get("params") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        // 允许位置式数组参数，统一包成 {"_args": [...]} 供 session 层按需取用
        Some(other) => {
            let mut m = Map::new();
            m.insert("_args".to_string(), other.clone());
            m
        }
    };
    Ok(AcpRequest {
        jsonrpc: JSONRPC_VERSION.to_string(),
        id,
        method: method.to_string(),
        params,
    })
}

/// 从 JSON 文本解析为 `AcpRequest`。
pub fn parse(raw: &str) -> Result<AcpRequest, AcpError> {
    if raw.trim().is_empty() {
        return Err(AcpError::new(PARSE_ERROR, "empty json payload", None));
    }
    let v: Value = serde_json::from_str(raw)
        .map_err(|e| AcpError::new(PARSE_ERROR, format!("invalid json: {e}"), None))?;
    parse_value(&v)
}

/// 从字节/字节数组解析为 `AcpRequest`。
pub fn parse_bytes(raw: &[u8]) -> Result<AcpRequest, AcpError> {
    let s = std::str::from_utf8(raw)
        .map_err(|e| AcpError::new(PARSE_ERROR, format!("invalid utf-8: {e}"), None))?;
    parse(s)
}

/// 构造 JSON-RPC 成功响应。
pub fn make_result(request_id: &Value, result: Value) -> Value {
    serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": request_id,
        "result": result,
    })
}

/// 构造 JSON-RPC 错误响应。
pub fn make_error(request_id: &Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut err = serde_json::Map::new();
    err.insert("code".to_string(), Value::from(code));
    err.insert("message".to_string(), Value::from(message));
    if let Some(d) = data {
        err.insert("data".to_string(), d);
    }
    serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": request_id,
        "error": Value::Object(err),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_request() {
        let v = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"ping","params":{}});
        let req = parse_value(&v).unwrap();
        assert_eq!(req.method, "ping");
        assert_eq!(req.id, Some(Value::from(1)));
        assert!(!req.is_notification());
    }

    #[test]
    fn parse_notification_has_no_id() {
        let v = serde_json::json!({"jsonrpc":"2.0","method":"notify"});
        let req = parse_value(&v).unwrap();
        assert!(req.is_notification());
    }

    #[test]
    fn parse_position_args_wrapped() {
        let v = serde_json::json!({"jsonrpc":"2.0","id":2,"method":"foo","params":[1,2,3]});
        let req = parse_value(&v).unwrap();
        assert!(req.params.contains_key("_args"));
    }

    #[test]
    fn parse_rejects_wrong_version() {
        let v = serde_json::json!({"jsonrpc":"1.0","method":"ping"});
        assert_eq!(parse_value(&v).unwrap_err().code, INVALID_REQUEST);
    }

    #[test]
    fn parse_rejects_missing_method() {
        let v = serde_json::json!({"jsonrpc":"2.0","id":1});
        assert_eq!(parse_value(&v).unwrap_err().code, INVALID_REQUEST);
    }

    #[test]
    fn parse_rejects_non_object() {
        let v = serde_json::json!([]);
        assert_eq!(parse_value(&v).unwrap_err().code, INVALID_REQUEST);
    }

    #[test]
    fn parse_str_and_bytes() {
        assert!(parse(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).is_ok());
        assert!(parse("   ").unwrap_err().code == PARSE_ERROR);
        assert!(parse_bytes(b"not json").unwrap_err().code == PARSE_ERROR);
    }

    #[test]
    fn make_result_and_error_shapes() {
        let ok = make_result(&Value::from(5), Value::from("pong"));
        assert_eq!(ok["id"], 5);
        assert_eq!(ok["result"], "pong");
        let err = make_error(&Value::from(5), METHOD_NOT_FOUND, "nope", None);
        assert_eq!(err["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(err["error"]["message"], "nope");
    }
}
