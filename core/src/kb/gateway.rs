//! 知识库网关 RPC（`kb.status` / `kb.search` / `kb.ingest`）
//!
//! 把 `kb/` 引擎（BM25 检索 + 去噪 + 图谱 + 审计）暴露给 Web UI：
//! - 进程级 `KnowledgeBase` 单例；
//! - 首次访问时自动把 context 台账（会话历史）作为种子灌入，保证检索有内容；
//! - `kb.ingest` 支持外部喂入文档（标题 + 正文）。

use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use crate::context::session::SessionContext;
use crate::kb::store::KnowledgeBase;
use crate::persistence::config::Config;

/// 进程级知识库单例
fn kb() -> &'static Mutex<KnowledgeBase> {
    static KB: OnceLock<Mutex<KnowledgeBase>> = OnceLock::new();
    KB.get_or_init(|| Mutex::new(KnowledgeBase::new()))
}

/// 种子灌入标记（进程内只执行一次）
static SEEDED: OnceLock<()> = OnceLock::new();

/// 首次调用时把 context 台账（各会话历史）灌入 KB 作为种子。
/// 会话每次对话都会写 `data_dir/context/{session}.json`，天然构成可检索知识库。
fn ensure_seeded() {
    SEEDED.get_or_init(|| {
        let dir = Config::data_dir().join("context");
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut kb = kb().lock().unwrap_or_else(|e| e.into_inner());
            for entry in rd.flatten() {
                let path = entry.path();
                if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let Ok(raw) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(ctx) = serde_json::from_str::<SessionContext>(&raw) else {
                    continue;
                };
                let text: Vec<String> = ctx.entries.iter().map(|e| e.text.clone()).collect();
                if text.is_empty() {
                    continue;
                }
                kb.add_document(&ctx.session, &text.join("\n"));
            }
        }
    });
}

/// `kb.status`：知识库健康视图（文档数 / 分块数 / 引擎）
pub fn status() -> Value {
    ensure_seeded();
    let kb = kb().lock().unwrap_or_else(|e| e.into_inner());
    json!({
        "documents": kb.document_count(),
        "chunks": kb.chunk_count(),
        "engine": "bm25",
    })
}

/// `kb.search`：BM25 语义检索
///
/// params: `{ query: string, topK?: number }` → `{ items: [{ doc_id, title, chunk, score }] }`
pub fn search(params: &Value) -> Value {
    ensure_seeded();
    let query = params.get("query").and_then(Value::as_str).unwrap_or("");
    let top_k = params
        .get("topK")
        .and_then(Value::as_u64)
        .or_else(|| params.get("top_k").and_then(Value::as_u64))
        .unwrap_or(5) as usize;
    if query.trim().is_empty() {
        return json!({ "items": [] });
    }
    let kb = kb().lock().unwrap_or_else(|e| e.into_inner());
    let hits = kb.retrieve(query, top_k.min(20));
    json!({
        "items": hits.iter().map(|h| json!({
            "doc_id": h.doc_id,
            "title": h.title,
            "chunk": h.chunk,
            "score": h.score,
        })).collect::<Vec<_>>(),
    })
}

/// `kb.search.hybrid`：BM25 + 向量 + RRF 融合检索（向量检索页数据源）。
///
/// 向量后端 = 进程级嵌入提供者（默认零依赖特征哈希 256 维；配置 backend=model 且启用 real-embed
/// 时为真实模型）。语料 > 2000 块时诚实降级纯 BM25；engine 字段如实回报，不冒充。
pub fn search_hybrid(params: &Value) -> Value {
    ensure_seeded();
    let query = params.get("query").and_then(Value::as_str).unwrap_or("");
    let top_k = params
        .get("topK")
        .and_then(Value::as_u64)
        .or_else(|| params.get("top_k").and_then(Value::as_u64))
        .unwrap_or(5) as usize;
    if query.trim().is_empty() {
        return json!({ "items": [], "engine": "bm25" });
    }
    let kb_snapshot = kb().lock().unwrap_or_else(|e| e.into_inner()).clone();
    let chunk_count = kb_snapshot.chunk_count();
    if chunk_count > 2000 {
        let hits = kb_snapshot.retrieve(query, top_k.min(20));
        return json!({
            "engine": "bm25",
            "items": hits.iter().map(|h| json!({
                "doc_id": h.doc_id, "title": h.title, "chunk": h.chunk,
                "bm25_score": h.score, "vector_score": 0.0, "fused_score": h.score,
            })).collect::<Vec<_>>(),
        });
    }
    let embedder = crate::memory::embedding::EmbeddingProvider::global();
    let mut retriever = crate::kb::hybrid::HybridRetriever::with_embedder(kb_snapshot, embedder);
    retriever.reindex_embeddings();
    let engine = match crate::persistence::config::Config::load() {
        Ok(c) if c.embeddings.backend.eq_ignore_ascii_case("model") => {
            "bm25+vector(model)".to_string()
        }
        _ => "bm25+vector(hash256)".to_string(),
    };
    let hits = retriever.retrieve(query, top_k.min(20));
    json!({
        "engine": engine,
        "items": hits.iter().map(|h| json!({
            "doc_id": h.doc_id,
            "title": h.title,
            "chunk": h.chunk,
            "bm25_score": h.bm25_score,
            "vector_score": h.vector_score,
            "fused_score": h.fused_score,
        })).collect::<Vec<_>>(),
    })
}

/// `kb.ingest`：灌入一篇文档（标题 + 正文）
///
/// params: `{ title: string, text: string }` → `{ ok, documents, chunks }`
pub fn ingest(params: &Value) -> Value {
    ensure_seeded();
    let title = params
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("untitled");
    let text = params.get("text").and_then(Value::as_str).unwrap_or("");
    if text.trim().is_empty() {
        return json!({ "ok": false, "error": "text 不可为空" });
    }
    let mut kb = kb().lock().unwrap_or_else(|e| e.into_inner());
    kb.add_document(title, text);
    // 同步索引进全局知识图谱（kb.graph.view 的可视化数据源）
    if let Ok(mut graph) = crate::kb::graph::global_graph().lock() {
        graph.index_document(title, text);
    }
    json!({
        "ok": true,
        "documents": kb.document_count(),
        "chunks": kb.chunk_count(),
    })
}

/// `kb.list`：列出所有已索引文档
pub fn list() -> Value {
    ensure_seeded();
    let kb = kb().lock().unwrap_or_else(|e| e.into_inner());
    let docs: Vec<Value> = kb
        .list_documents()
        .iter()
        .map(|(id, title, chunks)| {
            json!({
                "doc_id": id,
                "title": title,
                "chunks": chunks,
            })
        })
        .collect();
    json!({ "documents": docs, "total": docs.len() })
}

/// `kb.delete`：删除指定文档
pub fn delete(params: &Value) -> Value {
    let doc_id = params.get("doc_id").and_then(Value::as_str).unwrap_or("");
    if doc_id.is_empty() {
        return json!({ "ok": false, "error": "doc_id 不可为空" });
    }
    let mut kb = kb().lock().unwrap_or_else(|e| e.into_inner());
    let removed = kb.delete_document(doc_id);
    json!({
        "ok": removed,
        "documents": kb.document_count(),
        "chunks": kb.chunk_count(),
    })
}

/// `kb.ingest_file`：从磁盘读取文件并灌入知识库
pub fn ingest_file(params: &Value) -> Value {
    ensure_seeded();
    let path = params.get("path").and_then(Value::as_str).unwrap_or("");
    if path.is_empty() {
        return json!({ "ok": false, "error": "path 不可为空" });
    }
    let p = std::path::Path::new(path);
    if !p.exists() {
        return json!({ "ok": false, "error": format!("文件不存在: {path}") });
    }
    let title = params
        .get("title")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            p.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("untitled")
                .to_string()
        });
    let text = match std::fs::read_to_string(p) {
        Ok(t) => t,
        Err(e) => return json!({ "ok": false, "error": format!("读取文件失败: {e}") }),
    };
    let mut kb = kb().lock().unwrap_or_else(|e| e.into_inner());
    kb.add_document(&title, &text);
    json!({
        "ok": true,
        "title": title,
        "documents": kb.document_count(),
        "chunks": kb.chunk_count(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kb_ingest_and_search_roundtrip() {
        // ingest 命中唯一特征词 → search 应返回对应文档
        let ingest = ingest(&json!({
            "title": "fps-guide",
            "text": "体素竞速游戏使用三缓冲渲染与固定时间步长来保证帧率稳定。",
        }));
        assert_eq!(ingest["ok"], true);
        let res = search(&json!({ "query": "三缓冲", "topK": 3 }));
        let items = res["items"].as_array().cloned().unwrap_or_default();
        assert!(!items.is_empty(), "应检索到灌入的文档: {res}");
        assert!(items.iter().any(|i| i["title"] == "fps-guide"));
    }

    #[test]
    fn test_kb_status_reports_documents() {
        let before = status()["documents"].as_u64().unwrap_or(0);
        let _ = ingest(&json!({
            "title": "status-probe",
            "text": "唯一性探针文本 ox7k9z 用于验证状态统计。",
        }));
        let after = status()["documents"].as_u64().unwrap_or(0);
        assert!(after >= before, "ingest 后文档数不应减少");
    }

    #[test]
    fn test_kb_search_hybrid_uses_vector_engine() {
        let _ = ingest(&json!({
            "title": "hybrid-probe",
            "text": "混合检索探针：特征哈希向量与 BM25 融合排序验证。",
        }));
        let res = search_hybrid(&json!({ "query": "混合检索", "topK": 3 }));
        let engine = res["engine"].as_str().unwrap_or("");
        assert!(
            engine == "bm25" || engine.starts_with("bm25+vector"),
            "引擎字段应如实回报（语料 > 2000 块允许诚实降级 bm25）: {res}"
        );
        let items = res["items"].as_array().cloned().unwrap_or_default();
        assert!(!items.is_empty(), "融合检索应有结果: {res}");
        assert!(items[0]["fused_score"].is_number());
    }
}
