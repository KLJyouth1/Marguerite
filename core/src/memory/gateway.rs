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
///
/// ⚠️ 必须与**写入侧**同路径：对话运行时在 `gateway/chat.rs::memory_stack` 里写的是
/// `data_dir/marguerite_short_term.db`。历史缺陷：这里读的是
/// `data_dir/memory/short_term.db` —— 两个文件根本不是同一个，于是
/// `memory.short_term.search` / `layers_status` 永远返回空/0，运维会以为「没有记忆」。
fn short_term() -> &'static Mutex<ShortTermMemory> {
    static ST: OnceLock<Mutex<ShortTermMemory>> = OnceLock::new();
    ST.get_or_init(|| {
        let mut m = ShortTermMemory::new(&Config::data_dir().join("marguerite_short_term.db"));
        let _ = m.init();
        Mutex::new(m)
    })
}

/// 长期记忆进程单例（语义向量，首次访问时 init）。
///
/// ⚠️ 同 [`short_term`]：写入侧是 `data_dir/marguerite_ltm_gw.db`（见
/// `gateway/chat.rs::build_evolution` / `memory_stack`），读取侧必须一致。
fn long_term() -> &'static Mutex<LongTermMemory> {
    static LT: OnceLock<Mutex<LongTermMemory>> = OnceLock::new();
    LT.get_or_init(|| {
        let mut m = LongTermMemory::new(&Config::data_dir().join("marguerite_ltm_gw.db"));
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

/// `memory.synapse.status`：联想记忆网络（突触图）状态。
///
/// **隐私红线**：仅暴露节点/边计数、累计计数、平均权重与 top 边（≤10 条，形如
/// `{a,b,weight}`）；`a`/`b` 为**哈希 id**，**不含任何记忆正文**。
/// 字段名为 camelCase，与仓库既有返回风格一致。
pub fn synapse_status(_params: &Value) -> Value {
    let graph = crate::memory::synapse::global_graph();
    let s = graph
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .stats();
    let top_edges: Vec<Value> = s
        .top_edges
        .iter()
        .take(10)
        .map(|e| {
            json!({
                "a": e.a,
                "b": e.b,
                "weight": (e.weight * 10_000.0).round() / 10_000.0,
            })
        })
        .collect();
    json!({
        "ok": true,
        "enabled": crate::memory::synapse::synapse_enabled(),
        "nodes": s.nodes,
        "edges": s.edges,
        "reinforced": s.reinforced,
        "decayed": s.decayed,
        "activations": s.activations,
        "averageWeight": s.average_weight,
        "topEdges": top_edges,
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

    /// 前端契约守卫：`MemoryLayersPanel.vue` 要求 `layers` 是**数组**，每项含
    /// `id/name/module/reachable/rpc/count`，且 `count` 只能是数字或 null。
    ///
    /// 此前该 RPC 分派到一个把 `layers` 返回成「按 id 键控对象」的桩，前端
    /// `Array.isArray(data.layers)` 判假 → 七层记忆面板整块渲染为空。本测试锁住真实形状。
    #[test]
    fn layers_status_satisfies_memory_layers_panel_contract() {
        let v = layers_status(&Value::Null);
        let layers = v["layers"]
            .as_array()
            .expect("layers 必须是数组（前端 Array.isArray 判真才渲染）");
        assert!(!layers.is_empty(), "至少一层");
        for l in layers {
            for key in ["id", "name", "module", "reachable", "rpc", "count"] {
                assert!(
                    l.get(key).is_some(),
                    "层 {l} 缺少前端必需字段 `{key}`"
                );
            }
            assert!(l["id"].as_str().is_some_and(|s| !s.is_empty()), "id 非空");
            assert!(l["reachable"].is_boolean(), "reachable 必须是布尔");
            assert!(l["rpc"].is_array(), "rpc 必须是数组");
            // count 允许 null（无计数能力的层），但不允许是字符串等其它类型
            assert!(
                l["count"].is_null() || l["count"].is_u64(),
                "count 必须是数字或 null，实际 {}",
                l["count"]
            );
        }
        // 前端按 `settings.memoryLayers.layers.<id>` 取标签，故 7 个 id 必须齐全
        let ids: Vec<&str> = layers.iter().filter_map(|l| l["id"].as_str()).collect();
        for expected in [
            "sensory",
            "short_term",
            "long_term",
            "experience",
            "recall",
            "behavior",
            "cross_project",
        ] {
            assert!(ids.contains(&expected), "缺少层 id `{expected}`（前端标签键会落空）");
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

    /// 契约守卫：`memory.synapse.status` 字段齐全、camelCase、topEdges 仅含 id+权重
    /// （隐私红线：绝不泄露记忆正文）。
    #[test]
    fn synapse_status_contract_is_privacy_safe() {
        let v = synapse_status(&Value::Null);
        for key in [
            "ok",
            "enabled",
            "nodes",
            "edges",
            "reinforced",
            "decayed",
            "activations",
            "averageWeight",
            "topEdges",
        ] {
            assert!(v.get(key).is_some(), "memory.synapse.status 缺少字段 `{key}`");
        }
        assert_eq!(v["ok"], true);
        let top = v["topEdges"].as_array().expect("topEdges 必须是数组");
        assert!(top.len() <= 10, "topEdges 上限 10");
        for e in top {
            let obj = e.as_object().expect("边必须是对象");
            assert!(obj.contains_key("a") && obj.contains_key("b") && obj.contains_key("weight"));
            assert_eq!(obj.len(), 3, "边只含 a/b/weight，不得夹带记忆正文: {e}");
        }
    }
}
