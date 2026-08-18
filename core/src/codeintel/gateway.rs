//! 代码智能网关 RPC（`codeintel.status` / `.index` / `.search` / `.symbols`）
//!
//! 把 codeintel（项目索引 + 混合检索）暴露给 Web UI / 工具链。
//! workspace 缺省回落当前工作目录；全部路径离线、不触网。

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::codeintel::CodeIntel;

fn workspace_from(params: &Value) -> PathBuf {
    params
        .get("workspace")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// `codeintel.status`：索引健康视图（未索引时返回 0 计数，不报错）
pub fn status(params: &Value) -> Value {
    match CodeIntel::stats(&workspace_from(params)) {
        Ok((files, symbols, chunks)) => json!({
            "files": files,
            "symbols": symbols,
            "chunks": chunks,
        }),
        Err(e) => json!({ "files": 0, "symbols": 0, "chunks": 0, "error": e.to_string() }),
    }
}

/// `codeintel.index`：增量刷新索引
pub fn index(params: &Value) -> Value {
    match CodeIntel::index(&workspace_from(params)) {
        Ok(report) => json!({ "ok": true, "report": report }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `codeintel.search`：混合检索
pub fn search(params: &Value) -> Value {
    let query = params.get("query").and_then(Value::as_str).unwrap_or("");
    if query.trim().is_empty() {
        return json!({ "items": [] });
    }
    let top_k = params
        .get("topK")
        .and_then(Value::as_u64)
        .or_else(|| params.get("top_k").and_then(Value::as_u64))
        .unwrap_or(10) as usize;
    match CodeIntel::search(&workspace_from(params), query, top_k) {
        Ok(items) => json!({ "items": items }),
        Err(e) => json!({ "items": [], "error": e.to_string() }),
    }
}

/// `codeintel.symbols`：按名称查询符号
pub fn symbols(params: &Value) -> Value {
    let query = params.get("query").and_then(Value::as_str).unwrap_or("");
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
    match CodeIntel::symbols(&workspace_from(params), query, limit) {
        Ok(items) => json!({ "items": items }),
        Err(e) => json!({ "items": [], "error": e.to_string() }),
    }
}
