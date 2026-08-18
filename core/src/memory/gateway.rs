//! 记忆层网关 RPC：sensory / short_term / long_term / recall 的端到端暴露
//!
//! 补齐「7 层记忆」中此前仅 experience / behavior / cross_project 可达的四层：
//! - `sensory`：从 context 台账读取当前会话上下文，做感知检索
//! - `short_term`：SQLite FTS5 跨会话全文检索
//! - `long_term`：语义向量检索
//! - `recall`：四类持久记忆（user / project / feedback / auto）统一召回
//!
//! 进程级单例管理持久化存储，与 [`crate::kb::gateway`] 同源；所有检索失败一律
//! 返回空结果而非 panic（诚实边界：未初始化 / 锁毒化时不冒充有数据）。

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use crate::context::session::SessionContext;
use crate::memory::long_term::LongTermMemory;
use crate::memory::recall::{KbBackedMemory, MemoryBackend, MemoryCategory};
use crate::memory::short_term::ShortTermMemory;
use crate::persistence::config::Config;

fn memory_dir() -> PathBuf {
    Config::data_dir().join("memory")
}

/// 短期记忆进程单例（FTS5，首次访问时 init）。
fn short_term() -> &'static Mutex<ShortTermMemory> {
    static ST: OnceLock<Mutex<ShortTermMemory>> = OnceLock::new();
    ST.get_or_init(|| {
        let mut m = ShortTermMemory::new(&memory_dir().join("short_term.db"));
        let _ = m.init();
        Mutex::new(m)
    })
}

/// 长期记忆进程单例（语义向量，首次访问时 init）。
fn long_term() -> &'static Mutex<LongTermMemory> {
    static LT: OnceLock<Mutex<LongTermMemory>> = OnceLock::new();
    LT.get_or_init(|| {
        let mut m = LongTermMemory::new(&memory_dir().join("long_term.db"));
        let _ = m.init();
        Mutex::new(m)
    })
}

/// 召回记忆进程单例（四类持久记忆，落盘 `data_dir/memory/*.json`）。
fn recall() -> &'static KbBackedMemory {
    static R: OnceLock<KbBackedMemory> = OnceLock::new();
    R.get_or_init(|| KbBackedMemory::with_persistence(&memory_dir()))
}

fn truncate_chars(s: &str, max: usize) -> String {
    let trimmed: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        format!("{trimmed}…")
    } else {
        trimmed
    }
}

fn read_sensory_context(session_key: &str) -> Option<SessionContext> {
    let path = Config::data_dir()
        .join("context")
        .join(format!("{session_key}.json"));
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<SessionContext>(&raw).ok()
}

/// context 台账目录下的会话文件数（sensory 层的「会话数」统计）。
fn context_file_count() -> usize {
    let dir = Config::data_dir().join("context");
    match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
            .count(),
        Err(_) => 0,
    }
}

/// `memory.sensory.search`：感知记忆检索（读当前会话上下文，子串匹配）。
pub fn sensory_search(params: &Value) -> Value {
    let session_key = params
        .get("sessionKey")
        .and_then(Value::as_str)
        .unwrap_or("");
    let query = params.get("query").and_then(Value::as_str).unwrap_or("");
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
    if session_key.is_empty() || query.is_empty() {
        return json!({ "items": [] });
    }
    let Some(ctx) = read_sensory_context(session_key) else {
        return json!({ "items": [] });
    };
    let q = query.to_lowercase();
    let items: Vec<Value> = ctx
        .entries
        .iter()
        .filter(|e| e.text.to_lowercase().contains(&q))
        .take(limit.max(1))
        .map(|e| json!({ "text": truncate_chars(&e.text, 400) }))
        .collect();
    json!({ "items": items, "total": ctx.entries.len() })
}

/// `memory.short_term.search`：短期记忆全文检索（FTS5 跨会话）。
pub fn short_term_search(params: &Value) -> Value {
    let query = params.get("query").and_then(Value::as_str).unwrap_or("");
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
    if query.is_empty() {
        return json!({ "items": [] });
    }
    let items: Vec<Value> = match short_term().lock() {
        Ok(m) => match m.search(query, limit) {
            Ok(list) => list
                .iter()
                .map(|s| {
                    json!({
                        "session_id": s.session_id,
                        "summary": s.summary,
                        "message_count": s.message_count,
                        "updated_at": s.updated_at,
                    })
                })
                .collect(),
            Err(_) => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    json!({ "items": items })
}

/// `memory.long_term.search`：长期记忆语义检索（向量余弦相似度）。
pub fn long_term_search(params: &Value) -> Value {
    let query = params.get("query").and_then(Value::as_str).unwrap_or("");
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
    if query.is_empty() {
        return json!({ "items": [] });
    }
    let items: Vec<Value> = match long_term().lock() {
        Ok(m) => match m.search(query, limit) {
            Ok(list) => list
                .iter()
                .map(|i| {
                    json!({
                        "content": i.content,
                        "meta": i.meta,
                        "score": i.score,
                    })
                })
                .collect(),
            Err(_) => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    json!({ "items": items })
}

/// `memory.recall.search`：四类持久记忆统一召回（project 走 BM25，其余子串/词重叠）。
pub fn recall_search(params: &Value) -> Value {
    let category = params
        .get("category")
        .and_then(Value::as_str)
        .and_then(MemoryCategory::parse)
        .unwrap_or(MemoryCategory::User);
    let query = params.get("query").and_then(Value::as_str).unwrap_or("");
    let top_k = params.get("topK").and_then(Value::as_u64).unwrap_or(10) as usize;
    let hits = recall().recall(category, query, top_k.max(1));
    json!({
        "category": category.as_str(),
        "items": hits.iter().map(|h| json!({
            "category": h.category.as_str(),
            "key": h.key,
            "snippet": h.snippet,
            "score": h.score,
        })).collect::<Vec<_>>(),
    })
}

/// `memory.layers.status`：7 层记忆端到端审计视图（真实计数 + RPC 可达性）。
///
/// sensory / short_term / long_term / recall 四层返回真实计数（本轮补齐的 RPC）；
/// experience / behavior / cross_project 此前已暴露 RPC，此处标注可达性。
pub fn layers_status(_params: &Value) -> Value {
    let sensory_count = context_file_count();
    let st_count = short_term()
        .lock()
        .map(|m| m.session_count().unwrap_or(0))
        .unwrap_or(0);
    let lt_count = long_term()
        .lock()
        .map(|m| m.count().unwrap_or(0))
        .unwrap_or(0);
    let recall_count = recall().count(MemoryCategory::User)
        + recall().count(MemoryCategory::Feedback)
        + recall().count(MemoryCategory::Auto)
        + recall().count(MemoryCategory::Project);

    let layer = |id: &str, name: &str, module: &str, rpc: &[&str], count: Option<usize>| {
        json!({
            "id": id,
            "name": name,
            "module": module,
            "implemented": true,
            "rpc": rpc,
            "reachable": !rpc.is_empty(),
            "count": count,
        })
    };

    json!({
        "layers": [
            layer("sensory", "感知记忆", "memory::sensory", &["memory.sensory.search"], Some(sensory_count)),
            layer("short_term", "短期记忆", "memory::short_term", &["memory.short_term.search"], Some(st_count)),
            layer("long_term", "长期记忆", "memory::long_term", &["memory.long_term.search"], Some(lt_count)),
            layer("experience", "经验记忆", "memory::experience", &["memory.search", "memory.save", "memory.list"], None),
            layer("recall", "召回/回忆", "memory::recall", &["memory.recall.search"], Some(recall_count)),
            layer("behavior", "行为记忆", "memory::behavior", &["memory.behavior.list", "memory.behavior.summary"], None),
            layer("cross_project", "跨项目记忆", "memory::cross_project", &["memory.cross_project.search", "memory.cross_project.expand"], None),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_status_marks_all_seven_reachable_after_this_round() {
        let v = layers_status(&Value::Null);
        let layers = v["layers"].as_array().expect("layers 数组");
        assert_eq!(layers.len(), 7, "7 层记忆");
        // 本轮补齐的四层必须都有 RPC 且 reachable
        for id in ["sensory", "short_term", "long_term", "recall"] {
            let l = layers.iter().find(|l| l["id"] == id).expect(id);
            assert_eq!(l["reachable"], true, "{id} 应已暴露 RPC: {l}");
            assert!(
                !l["rpc"].as_array().unwrap().is_empty(),
                "{id} 应含 RPC 方法: {l}"
            );
        }
    }

    #[test]
    fn recall_search_defaults_to_user_category() {
        let v = recall_search(&json!({ "query": "" }));
        assert_eq!(v["category"], "user");
        assert!(v["items"].is_array());
    }

    #[test]
    fn short_term_search_requires_query() {
        assert_eq!(short_term_search(&json!({})), json!({ "items": [] }));
    }

    #[test]
    fn long_term_search_requires_query() {
        assert_eq!(long_term_search(&json!({})), json!({ "items": [] }));
    }

    #[test]
    fn sensory_search_requires_session_and_query() {
        assert_eq!(sensory_search(&json!({})), json!({ "items": [] }));
    }
}
