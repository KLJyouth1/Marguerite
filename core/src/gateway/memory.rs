//! 记忆层 RPC（ExperienceLedger 直连）：语义检索 / 写入 / 列表。
//!
//! 数据源：`marguerite_experience.db`（SQLite，由 `ExperienceLedger` 托管）。
//! 所有方法在台账不可用时静默降级返回空结果，不阻塞前端。

use serde_json::{json, Value};

use crate::memory::{Experience, ExperienceKind, ExperienceLedger};
use crate::persistence::config::Config;

fn open_ledger() -> Option<ExperienceLedger> {
    let mut ledger = ExperienceLedger::new(&Config::data_dir().join("marguerite_experience.db"));
    match ledger.init() {
        Ok(()) => Some(ledger),
        Err(e) => {
            tracing::warn!(err = %e, "记忆台账打开失败");
            None
        }
    }
}

fn experience_view(e: &Experience) -> Value {
    json!({
        "id": e.id,
        "kind": e.kind.as_str(),
        "title": e.title,
        "body": e.body,
        "tags": e.tags,
        "signature": e.signature,
        "validated": e.validated,
        "created_at": e.created_at,
    })
}

/// `memory.search`：按 query 做经验台账检索（经验 = 坑 / 胜利 / 模式）。
pub fn memory_search(params: &Value) -> Value {
    let query = params.get("query").and_then(Value::as_str).unwrap_or("");
    if query.is_empty() {
        return json!({ "items": [] });
    }
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
    let items = match open_ledger() {
        Some(l) => match l.search(query, limit) {
            Ok(list) => list.iter().map(experience_view).collect::<Vec<_>>(),
            Err(e) => {
                tracing::warn!(err = %e, "memory.search 失败");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    json!({ "items": items })
}

/// `memory.save`：写入一条经验（支持自动 signature 防重复）。
pub fn memory_save(params: &Value) -> Value {
    let kind = ExperienceKind::from_kind_str(
        params
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("pitfall"),
    );
    let title = params
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let body = params
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if title.is_empty() || body.is_empty() {
        return json!({ "saved": false, "message": "title 与 body 不可为空" });
    }
    let tags: Vec<String> = params
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let signature = params
        .get("signature")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            format!("{title}{body}").hash(&mut h);
            format!("auto:{:x}", h.finish())
        });
    match open_ledger() {
        Some(mut l) => match l.record(kind, title, body, &tags, &signature) {
            Ok(id) => json!({ "saved": true, "id": id, "kind": kind.as_str() }),
            Err(e) => json!({ "saved": false, "message": format!("保存失败: {e}") }),
        },
        None => json!({ "saved": false, "message": "记忆台账不可用" }),
    }
}

/// `memory.list`：列出经验台账（按 kind 过滤，默认最近 50 条）。
pub fn memory_list(params: &Value) -> Value {
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
    let kind = params.get("kind").and_then(Value::as_str);
    let items = match open_ledger() {
        Some(l) => {
            let res = if let Some(k) = kind {
                l.list_by_kind(ExperienceKind::from_kind_str(k), limit)
            } else {
                l.list_recent(limit)
            };
            match res {
                Ok(list) => list.iter().map(experience_view).collect::<Vec<_>>(),
                Err(e) => {
                    tracing::warn!(err = %e, "memory.list 失败");
                    Vec::new()
                }
            }
        }
        None => Vec::new(),
    };
    json!({ "items": items })
}
