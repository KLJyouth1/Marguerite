//! Auto-extracted from builtin.rs — see parent module for shared infrastructure.
//!
//! Memory tools: user_memory_dir + memory_save/search/list

use super::*;

pub(crate) fn user_memory_dir() -> PathBuf {
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        PathBuf::from(local).join("Marguerite")
    } else {
        std::env::temp_dir().join("marguerite-memory")
    }
}

pub(crate) fn memory_tools(db_path: PathBuf) -> Vec<Box<dyn ToolExecutor>> {
    let save_path = db_path.clone();
    let search_path = db_path.clone();
    let list_path = db_path.clone();
    vec![
        boxed(
            "memory_save",
            "将经验/知识/踩坑沉淀到长期记忆库（SQLite 持久化，同 signature 幂等更新）。参数: body(必填), title, kind=pattern|decision|pitfall, tags(逗号分隔), signature(可选)",
            json!({"type":"object","properties":{
                "body":{"type":"string"},
                "title":{"type":"string"},
                "kind":{"type":"string","enum":["pattern","decision","pitfall"]},
                "tags":{"type":"string"},
                "signature":{"type":"string"}
            },"required":["body"]}),
            Arc::new(move |args| {
                let db_path = save_path.clone();
                Box::pin(async move {
                    if let Some(p) = db_path.parent() {
                        std::fs::create_dir_all(p).ok();
                    }
                    let mut ledger = ExperienceLedger::new(&db_path);
                    ledger.init().map_err(|e| anyhow!("memory init: {e}"))?;
                    let body = args.get("body").cloned().unwrap_or_default();
                    if body.trim().is_empty() {
                        return err("memory_save 需要非空 body".into(), "INVALID_PARAMS");
                    }
                    let kind = ExperienceKind::from_kind_str(args.get("kind").map(String::as_str).unwrap_or("pattern"));
                    let title = args.get("title").cloned().unwrap_or_else(|| "未命名经验".into());
                    let tags: Vec<String> = args
                        .get("tags")
                        .cloned()
                        .unwrap_or_default()
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    let sig = args.get("signature").cloned().unwrap_or_default();
                    let id = ledger.record(kind, &title, &body, &tags, &sig)?;
                    ok(format!("已沉淀记忆条目 #{id}（{}）：{title}", kind.as_str()))
                })
            }),
        ),
        boxed(
            "memory_search",
            "语义检索长期记忆库（向量相似度，低于阈值视为无相关）。参数: query(必填), limit(默认5)",
            json!({"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"number"}},"required":["query"]}),
            Arc::new(move |args| {
                let db_path = search_path.clone();
                Box::pin(async move {
                    if let Some(p) = db_path.parent() {
                        std::fs::create_dir_all(p).ok();
                    }
                    let mut ledger = ExperienceLedger::new(&db_path);
                    ledger.init().map_err(|e| anyhow!("memory init: {e}"))?;
                    let query = args.get("query").cloned().unwrap_or_default();
                    if query.trim().is_empty() {
                        return err("memory_search 需要非空 query".into(), "INVALID_PARAMS");
                    }
                    let limit: usize = args.get("limit").and_then(|l| l.parse().ok()).unwrap_or(5);
                    let hits = ledger.search(&query, limit)?;
                    if hits.is_empty() {
                        return ok("未检索到相关记忆".into());
                    }
                    let out = hits
                        .iter()
                        .map(|e| {
                            let tags = if e.tags.is_empty() { "-".to_string() } else { e.tags.join(",") };
                            format!("[{}] {} ({})\n  {}", e.kind.as_str(), e.title, e.id, e.body)
                                + &format!("\n  tags: {tags}")
                        })
                        .collect::<Vec<_>>()
                        .join("\n---\n");
                    ok(out)
                })
            }),
        ),
        boxed(
            "memory_list",
            "按类型列出长期记忆（最新在前）。参数: kind=pattern|decision|pitfall(必填), limit(默认20)",
            json!({"type":"object","properties":{"kind":{"type":"string","enum":["pattern","decision","pitfall"]},"limit":{"type":"number"}},"required":["kind"]}),
            Arc::new(move |args| {
                let db_path = list_path.clone();
                Box::pin(async move {
                    if let Some(p) = db_path.parent() {
                        std::fs::create_dir_all(p).ok();
                    }
                    let mut ledger = ExperienceLedger::new(&db_path);
                    ledger.init().map_err(|e| anyhow!("memory init: {e}"))?;
                    let kind = ExperienceKind::from_kind_str(args.get("kind").map(String::as_str).unwrap_or("pattern"));
                    let limit: usize = args.get("limit").and_then(|l| l.parse().ok()).unwrap_or(20);
                    let items = ledger.list_by_kind(kind, limit)?;
                    if items.is_empty() {
                        return ok(format!("暂无 {} 类型记忆", kind.as_str()));
                    }
                    let out = items
                        .iter()
                        .map(|e| format!("[{}] {} | {} (validated={})", e.kind.as_str(), e.title, e.body, e.validated))
                        .collect::<Vec<_>>()
                        .join("\n");
                    ok(out)
                })
            }),
        ),
    ]
}
