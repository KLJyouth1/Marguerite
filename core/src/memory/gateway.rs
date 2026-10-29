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
use crate::memory::lexical::{content_words, shared_content_words};
use crate::memory::long_term::LongTermMemory;
use crate::memory::recall::{KbBackedMemory, MemoryBackend, MemoryCategory};
use crate::memory::short_term::{max_sessions_from_config, ShortTermMemory};
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
        let mut m = ShortTermMemory::new(&Config::data_dir().join("marguerite_short_term.db"))
            .with_max_sessions(max_sessions_from_config());
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
    // 文件名编码统一由台账模块负责（键含 `:` 时旧口径 `{key}.json` 对不上盘上文件）。
    crate::context::session::read_session_context(session_key)
}

/// context 台账目录下的会话文件数（sensory 层的「会话数」统计）。
fn context_file_count() -> usize {
    let dir = crate::context::session::context_dir();
    match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
            .count(),
        Err(_) => 0,
    }
}

/// `memory.sensory.search` 的相关性判据：查询与条目的**实词重合数**（0 = 不相关）。
///
/// 抽成纯函数是为了让离线回归（`memory::recall_probe`）能直接对夹具求测 ——
/// 感知层的数据在盘上会话台账里，伪造一份台账来做测量既慢又脆。
///
/// ⚠️ 这里从来不该是「整条查询当子串」。实测 15 族 60 条查询命中率 **0.0%**：
/// 真实提问几乎不可能逐字复现台账里的原句，于是这一层看起来"接上了"其实永远返回空。
pub(crate) fn sensory_overlap(query: &str, text: &str) -> usize {
    shared_content_words(&content_words(query), &content_words(text))
}

/// `memory.sensory.search`：感知记忆检索（读当前会话上下文，按实词重合排序）。
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
    // 重合数 > 0 才可能相关；同分时**台账靠后的（更新的）在前** —— 感官层的定义
    // 就是「最近说过的话」，让旧条目抢位会把它做成一个过期的全文搜索。
    let mut scored: Vec<(usize, usize, &crate::context::session::ContextEntry)> = ctx
        .entries
        .iter()
        .enumerate()
        .map(|(pos, e)| (sensory_overlap(query, &e.text), pos, e))
        .filter(|(overlap, _, _)| *overlap > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let items: Vec<Value> = scored
        .into_iter()
        .take(limit.max(1))
        .map(|(overlap, _, e)| json!({ "text": truncate_chars(&e.text, 400), "overlap": overlap }))
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

/// `memory.tiered.status`：分层稀疏记忆（flyhash + tiered）只读观测面。
///
/// # 为什么需要它（W-O3）
///
/// `memory::flyhash` 与 `memory::tiered` 是**执行侧**真实在用的一层（第六路召回，
/// 见 [`crate::memory::inject`]），但此前**没有任何观测入口**：用户在界面上无法
/// 判断「这一层到底有没有数据、开关是否打开、索引与长时池是否一致」，出问题只能
/// 翻日志。本 RPC 只读、无副作用，暴露四类事实：
///
/// 1. **开关**：`memory.tiered_enabled`（默认 false，属静默注入，必须可查）；
/// 2. **计数**：三层条目数 + flyhash 索引条目数；
/// 3. **健全性**：`indexConsistent` = 索引条目数 == 长时池条目数
///    （两者只由 `insert`/`promote`/`trim_pool` 同时改动，不等即漏建/漏删）；
/// 4. **参数**：每层容量上限、弱证据标注线、截断规则 `(保留比例, 零重合可否单独成立)`、
///    FLY_DIM/WTA_ROUNDS（`memory.tiered_enabled` 打开后，flyhash 只作索引健全性观测与
///    写入侧聚类，**不参与打分**，见 `tiered` 模块头）。
///
/// 隐私红线：**只返回计数与参数**，不含任何记忆正文。
pub fn tiered_status(_params: &Value) -> Value {
    let enabled = match Config::load() {
        Ok(c) => c.memory.tiered_enabled,
        Err(_) => false,
    };
    let (sensory, short, long, fly) = crate::memory::inject::tier_counts();
    let (max_per_tier, weak_evidence) = crate::memory::inject::tier_limits();
    let (cutoff_ratio, semantic_alone) = crate::memory::inject::tier_cutoff_rule();
    json!({
        "ok": true,
        "enabled": enabled,
        "total": sensory + short + long,
        "tiers": {
            "sensory": sensory,
            "shortTerm": short,
            "longTerm": long,
        },
        "flyhash": {
            "indexed": fly,
            "dim": crate::memory::flyhash::FLY_DIM,
            "wtaRounds": crate::memory::flyhash::WTA_ROUNDS,
            // 索引与长时池必须一一对应；不相等就是漏建/漏删（trim/promote 是唯一改动点）
            "indexConsistent": fly == long,
            // 诚实上报：检索打分已不含 flyhash（实测 6/6 档全量兜底触发、候选覆盖 0–89%）
            "usedForScoring": false,
        },
        "maxPerTier": max_per_tier,
        "simThreshold": weak_evidence,
        "weakEvidence": weak_evidence,
        "cutoffRatio": cutoff_ratio,
        // 「共享 0 个实词的候选能不能只靠余弦进上下文」——默认后端为 false，
        // 实测依据见 memory::scoring 模块头与 memory::recall_probe 的常驻测量。
        "semanticAloneAllowed": semantic_alone,
    })
}

/// `memory.layers.status`：7 层记忆端到端审计视图（真实计数 + RPC 可达性）。
///
/// sensory / short_term / long_term / recall 四层返回真实计数（本轮补齐的 RPC）；
/// experience / behavior / cross_project 此前已暴露 RPC，此处标注可达性。
pub fn layers_status(_params: &Value) -> Value {
    // 自动捕获开关的真实生效值（关闭后：每轮抽取 / tiered remember / 长期短期沉淀全部停写，
    // 但显式 memory.save / memory_save 仍允许）。放在这里供设置页核对「开关确实生效」。
    let auto_capture_enabled = crate::persistence::config::Config::load()
        .map(|c| c.memory.auto_capture_enabled)
        .unwrap_or(true);
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
        "autoCaptureEnabled": auto_capture_enabled,
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

    /// 自动捕获开关必须出现在状态里（核对「关闭后是否真的停写」的唯一观测面）。
    #[test]
    fn layers_status_reports_auto_capture_flag() {
        let v = layers_status(&Value::Null);
        assert!(
            v["autoCaptureEnabled"].is_boolean(),
            "layers.status 必须回报 autoCaptureEnabled: {v}"
        );
    }

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

    /// 契约守卫：`memory.tiered.status` 字段齐全、`indexConsistent` 为布尔、
    /// 且**绝不泄露记忆正文**（只允许计数与参数）。
    #[test]
    fn tiered_status_is_privacy_safe() {
        let v = tiered_status(&Value::Null);
        for key in [
            "ok",
            "enabled",
            "total",
            "tiers",
            "flyhash",
            "maxPerTier",
            "simThreshold",
        ] {
            assert!(v.get(key).is_some(), "memory.tiered.status 缺少字段 `{key}`");
        }
        assert_eq!(v["ok"], true);
        for key in ["sensory", "shortTerm", "longTerm"] {
            assert!(v["tiers"][key].is_u64(), "tiers.{key} 必须是计数");
        }
        let fly = &v["flyhash"];
        assert_eq!(fly["dim"], crate::memory::flyhash::FLY_DIM);
        assert_eq!(fly["wtaRounds"], crate::memory::flyhash::WTA_ROUNDS);
        assert!(
            fly["indexConsistent"].is_boolean(),
            "indexConsistent 必须是布尔（前端据此染色）"
        );
        // 隐私红线：不得出现任何正文承载字段
        let dumped = v.to_string();
        for forbidden in ["\"text\"", "\"content\"", "\"snippet\""] {
            assert!(
                !dumped.contains(forbidden),
                "tiered.status 不得含字段 {forbidden}：{dumped}"
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
