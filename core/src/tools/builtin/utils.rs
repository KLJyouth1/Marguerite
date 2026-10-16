//! Auto-extracted from builtin.rs — see parent module for shared infrastructure.
//!
//! Utility tools: json_query/diff/count_lines/read_env

use super::*;

pub(crate) fn json_query_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "json_query",
        "从 JSON 文本中按点分路径提取值（支持数组下标）。参数: json, path(如 a.b.0.c)",
        json!({"type":"object","properties":{"json":{"type":"string"},"path":{"type":"string"}}}),
        Arc::new(move |args| {
            Box::pin(async move {
                let js = args.get("json").ok_or_else(|| anyhow!("missing 'json'"))?;
                let path = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                match json_query(js, path) {
                    Ok(v) => ok(v),
                    Err(e) => err(e.to_string(), "QUERY_ERROR"),
                }
            })
        }),
    )
}

pub(crate) fn diff_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "diff",
        "计算两段文本的行级 diff。参数: old, new",
        json!({"type":"object","properties":{"old":{"type":"string"},"new":{"type":"string"}}}),
        Arc::new(move |args| {
            Box::pin(async move {
                let old = args.get("old").ok_or_else(|| anyhow!("missing 'old'"))?;
                let new = args.get("new").ok_or_else(|| anyhow!("missing 'new'"))?;
                ok(diff_lines(old, new))
            })
        }),
    )
}

pub(crate) fn count_lines_tool(workspace: PathBuf) -> Box<dyn ToolExecutor> {
    boxed(
        "count_lines",
        "统计文件行数。参数: path",
        json!({"type":"object","properties":{"path":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let p = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                let abs = safe_resolve(&ws, p)?;
                if !abs.is_file() {
                    return err(format!("not a file: {}", p), "NOT_FOUND");
                }
                let content = std::fs::read_to_string(&abs)?;
                ok(format!("{} lines in {}", content.lines().count(), p))
            })
        }),
    )
}

pub(crate) fn read_env_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "read_env",
        "读取指定环境变量的值（仅当前进程环境）。参数: name",
        json!({"type":"object","properties":{"name":{"type":"string"}}}),
        Arc::new(move |args| {
            Box::pin(async move {
                let name = args.get("name").ok_or_else(|| anyhow!("missing 'name'"))?;
                match std::env::var(name) {
                    Ok(v) => ok(v),
                    Err(_) => err(format!("env var not set: {}", name), "NOT_FOUND"),
                }
            })
        }),
    )
}

