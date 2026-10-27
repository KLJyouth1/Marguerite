//! Auto-extracted from builtin.rs — see parent module for shared infrastructure.
//!
//! Code intelligence tool: incremental index + symbol/text hybrid search

use super::*;

pub(crate) fn code_intel_tool(workspace: impl Into<RootSet>) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "code_intel",
        "项目代码智能：增量索引 + 符号/文本混合检索。action: index(建索引)/symbols(按名查符号)/search(混合检索)。参数: action, query(可选), workspace(可选,缺省主根), max(可选)",
        json!({"type":"object","properties":{
            "action":{"type":"string"},
            "query":{"type":"string"},
            "workspace":{"type":"string"},
            "max":{"type":"number"}
        }}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let action = args.get("action").map(|s| s.as_str()).unwrap_or("search");
                let ws_path = match args.get("workspace").filter(|s| !s.trim().is_empty()) {
                    Some(p) => ws.resolve(p)?,
                    None => ws.primary().to_path_buf(),
                };
                let max: usize = args.get("max").and_then(|m| m.parse().ok()).unwrap_or(20);
                let query = args.get("query").cloned().unwrap_or_default();
                match action {
                    "index" => match crate::codeintel::CodeIntel::index(&ws_path) {
                        Ok(report) => ok(format!(
                            "索引完成: {} 文件(更新 {} / 未变 {} / 移除 {} / 跳过 {})，符号 {}",
                            report.files_total,
                            report.files_updated,
                            report.files_unchanged,
                            report.files_removed,
                            report.skipped,
                            report.symbols_total
                        )),
                        Err(e) => err(format!("索引失败: {e}"), "INDEX_ERROR"),
                    },
                    "symbols" => match crate::codeintel::CodeIntel::symbols(&ws_path, &query, max) {
                        Ok(items) => {
                            if items.is_empty() {
                                return ok("0 symbol(s)".into());
                            }
                            let lines: Vec<String> = items
                                .iter()
                                .map(|s| format!("{}\t{}\t{}:{}", s.kind, s.name, s.path, s.line))
                                .collect();
                            ok(format!("{} symbol(s):\n{}", items.len(), lines.join("\n")))
                        }
                        Err(e) => err(format!("符号查询失败: {e}"), "SYMBOL_ERROR"),
                    },
                    _ => match crate::codeintel::CodeIntel::search(&ws_path, &query, max) {
                        Ok(items) => {
                            if items.is_empty() {
                                return ok("0 hit(s)".into());
                            }
                            let lines: Vec<String> = items
                                .iter()
                                .map(|h| match h.line {
                                    Some(l) => format!("[{}] {}:{} {}", h.source, h.path, l, h.snippet),
                                    None => format!("[{}] {} {}", h.source, h.path, h.snippet),
                                })
                                .collect();
                            ok(format!("{} hit(s):\n{}", items.len(), lines.join("\n")))
                        }
                        Err(e) => err(format!("检索失败: {e}"), "SEARCH_ERROR"),
                    },
                }
            })
        }),
    )
}
