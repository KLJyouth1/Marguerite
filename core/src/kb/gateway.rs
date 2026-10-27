//! 知识库网关 RPC（`kb.status` / `kb.search` / `kb.ingest`）
//!
//! 把 `kb/` 引擎（BM25 检索 + 去噪 + 图谱 + 审计）暴露给 Web UI：
//! - 进程级 `KnowledgeBase` 单例，索引持久化到 `<data_dir>/kb/store.json`；
//! - 首次访问且索引为空时，把 context 台账（会话历史）作为种子灌入；
//! - `kb.ingest` 的入库关卡顺序固定：去噪 → 内容哈希去重 → 切块索引 → 图谱 → 审计。
//!
//! 失败一律**如实回报**（带稳定错误码），不静默丢弃：
//! `KB_QUOTA_EXCEEDED` / `KB_DOC_TOO_LARGE` / `KB_FILE_TOO_LARGE` /
//! `KB_INVALID_PARAMS` / `PATH_OUTSIDE_ROOTS` / `RBAC_FORBIDDEN`。

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

#[cfg(not(test))]
use crate::context::session::SessionContext;
use crate::kb::denoiser::filter_documents;
use crate::kb::integrity::audit;
use crate::kb::store::{document_hash, KnowledgeBase};
use crate::persistence::config::Config;
use crate::rbac::policy::Policy;
use crate::rbac::roles::Role;
use crate::tools::fs_path::safe_resolve_any;

/// 单文档文本上限：2 MiB。
pub const MAX_DOC_BYTES: usize = 2 * 1024 * 1024;
/// 单库文档数上限。
pub const MAX_DOCUMENTS: usize = 5_000;
/// `kb.ingest_file` 文件大小上限：5 MiB。
pub const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// 持久化 + 健康状态（`persisted` 如实反映最近一次加载/写盘结果）。
struct KbState {
    kb: KnowledgeBase,
    path: PathBuf,
    persisted: bool,
    last_error: Option<String>,
}

/// 索引文件路径（生产：`Config::data_dir()/kb/store.json`）。
///
/// 单测用独立临时目录：否则每个 `cargo test` 都会写用户真实数据目录，
/// 且用例间相互污染（上一次运行的文档会让本次的去重用例直接失败）。
fn index_path() -> PathBuf {
    #[cfg(test)]
    {
        static TEST_DIR: OnceLock<PathBuf> = OnceLock::new();
        let dir = TEST_DIR.get_or_init(|| {
            std::env::temp_dir().join(format!("mgt-kb-gw-{}", uuid::Uuid::new_v4()))
        });
        dir.join("kb").join("store.json")
    }
    #[cfg(not(test))]
    {
        Config::data_dir().join("kb").join("store.json")
    }
}

fn load_state(path: PathBuf) -> KbState {
    // 兼容旧路径：历史版本把索引写在 `index.json`。首次升级到 `store.json` 时
    // 先读旧文件并把内容迁移到新路径，避免「升级后知识库凭空清空」。
    let legacy = path.with_file_name("index.json");
    let read_path = if !path.exists() && legacy.is_file() {
        legacy
    } else {
        path.clone()
    };
    let migrating = read_path != path;
    match KnowledgeBase::load_from(&read_path) {
        Ok(kb) => {
            let mut st = KbState {
                kb,
                path,
                persisted: true,
                last_error: None,
            };
            if migrating {
                persist(&mut st);
            }
            st
        }
        Err(e) => {
            // 首次启动（文件尚不存在）不是持久化故障：空库 + 可写即视为健康。
            // 只有「文件存在但读不出来/损坏/超限」才如实标 persisted:false。
            let exists = read_path.exists();
            KbState {
                kb: KnowledgeBase::new(),
                path,
                persisted: !exists,
                last_error: exists.then_some(e),
            }
        }
    }
}

/// 进程级知识库单例
fn state() -> &'static Mutex<KbState> {
    static KB: OnceLock<Mutex<KbState>> = OnceLock::new();
    KB.get_or_init(|| Mutex::new(load_state(index_path())))
}

fn lock_state() -> std::sync::MutexGuard<'static, KbState> {
    state().lock().unwrap_or_else(|e| e.into_inner())
}

/// 原子写回索引，并把真实结果写进 `persisted` / `lastError`。
fn persist(st: &mut KbState) {
    match st.kb.save_to(&st.path) {
        Ok(()) => {
            st.persisted = true;
            st.last_error = None;
        }
        Err(e) => {
            st.persisted = false;
            st.last_error = Some(format!("写入索引失败: {e}"));
        }
    }
}

/// 种子灌入标记（进程内只执行一次）
#[cfg(not(test))]
static SEEDED: OnceLock<()> = OnceLock::new();

/// 首次调用时把 context 台账（各会话历史）灌入 KB 作为种子。
///
/// **只在索引为空时灌入**：已有持久化索引时再灌一次只会制造与去重无关的
/// 重复文档（会话历史本来就会变化），破坏「重启后库内容不变」的预期。
///
/// 单测**不做种子灌入**：否则会读开发者本机真实 `data_dir/context`（数百个会话），
/// 既污染用例的文档计数，又让每条入库都拖上全量审计的 O(n²) 开销。
fn ensure_seeded() {
    #[cfg(test)]
    return;
    #[cfg(not(test))]
    SEEDED.get_or_init(|| {
        let mut st = lock_state();
        if st.kb.document_count() > 0 {
            return;
        }
        let dir = Config::data_dir().join("context");
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for entry in rd.flatten() {
                // 种子同样受配额边界约束：超限就停，不把「首次启动」变成一次静默的无限膨胀。
                if st.kb.document_count() >= MAX_DOCUMENTS {
                    tracing::warn!(limit = MAX_DOCUMENTS, "KB 种子灌入达到文档上限，后续会话不再灌入");
                    break;
                }
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
                let body = text.join("\n");
                if body.len() > MAX_DOC_BYTES {
                    tracing::warn!(session = %ctx.session, bytes = body.len(), "会话种子超过单文档上限，跳过");
                    continue;
                }
                let hash = document_hash(&ctx.session, &body);
                if st.kb.find_by_hash(&hash).is_none() {
                    st.kb.insert_document(&ctx.session, &body, &hash);
                }
            }
        }
        if st.kb.document_count() > 0 {
            persist(&mut st);
        }
    });
}

/// 全量两两审计的规模上限。
///
/// `integrity::audit` 的分块重复检测是 O(n²)；语料大到这个量级时每次入库都跑一遍会
/// 让网关长时间不可用。超限时**如实标 skipped**（不是假装健康，也不是静默跳过）。
const MAX_AUDIT_CHUNKS: usize = 2_000;

/// 审计视图：结果同时给结构化指标与可读 `issues`（空数组 = 无问题）。
fn audit_view(kb: &KnowledgeBase) -> (Value, Vec<String>) {
    if kb.chunk_count() > MAX_AUDIT_CHUNKS {
        let issues = vec![format!(
            "audit_skipped:chunk_count>{}",
            MAX_AUDIT_CHUNKS
        )];
        return (
            json!({
                "ok": true,
                "skipped": true,
                "issues": issues,
                "chunkTotal": kb.chunk_count(),
            }),
            issues,
        );
    }
    let report = audit(kb);
    let mut issues: Vec<String> = Vec::new();
    if report.empty_chunks > 0 {
        issues.push(format!("empty_chunks:{}", report.empty_chunks));
    }
    if report.duplicate_chunk_pairs > 0 {
        issues.push(format!(
            "duplicate_chunk_pairs:{}",
            report.duplicate_chunk_pairs
        ));
    }
    if report.score_anomalies > 0 {
        issues.push(format!("score_anomalies:{}", report.score_anomalies));
    }
    (
        json!({
            "ok": report.ok,
            "issues": issues,
            "chunkTotal": report.chunk_total,
            "duplicateChunkPairs": report.duplicate_chunk_pairs,
            "emptyChunks": report.empty_chunks,
            "scoreAnomalies": report.score_anomalies,
        }),
        issues,
    )
}

/// 配额判定（纯函数，便于单测覆盖 5000 边界而不必真的灌 5000 篇）。
pub(crate) fn quota_violation(documents: usize, text_bytes: usize) -> Option<(&'static str, String)> {
    if text_bytes > MAX_DOC_BYTES {
        return Some((
            "KB_DOC_TOO_LARGE",
            format!(
                "单文档文本 {text_bytes} 字节超过上限 {MAX_DOC_BYTES} 字节（2MB，当前已索引 {documents} 篇）"
            ),
        ));
    }
    if documents >= MAX_DOCUMENTS {
        return Some((
            "KB_QUOTA_EXCEEDED",
            format!(
                "单库文档数已达上限 {MAX_DOCUMENTS}（当前 {documents} 篇），请先 kb.delete 再入库"
            ),
        ));
    }
    None
}

/// 写入口的 RBAC 校验（本地 owner 恒为 `Admin` → 恒通过，但校验真实执行）。
///
/// 说明：`SecuritySeam` 尚未接进 dispatch；在写函数入口直接调用 `Policy::check`
/// 是成本最低且可验证的接入方式 —— 未登记的方法会被 `requirement_for` 拒绝。
fn rbac_denied(method: &str) -> Option<Value> {
    match Policy.check(method, Role::Admin) {
        Ok(()) => None,
        Err(forbidden) => Some(json!({
            "ok": false,
            "code": forbidden.code,
            "method": method,
            "error": forbidden.message,
        })),
    }
}

/// `kb.status`：知识库健康 + 入库质量报告（全部取真实计数/真实文件大小）。
pub fn status() -> Value {
    ensure_seeded();
    let st = lock_state();
    let (audit_value, _) = audit_view(&st.kb);
    let stats = st.kb.stats();
    let size_bytes = std::fs::metadata(&st.path).map(|m| m.len()).unwrap_or(0);
    json!({
        "documents": st.kb.document_count(),
        "chunks": st.kb.chunk_count(),
        "engine": "bm25",
        "persisted": st.persisted,
        "storePath": st.path.display().to_string(),
        "duplicatesRejected": stats.duplicate_rejected,
        "denoisedDropped": stats.denoised_dropped,
        "rejected": stats.rejected,
        "lastIngestAt": stats.last_ingest_at,
        "sizeBytes": size_bytes,
        // 兼容旧前端/旧调用：`path` 与单数形式的质量计数保留为别名。
        "path": st.path.display().to_string(),
        "duplicateRejected": stats.duplicate_rejected,
        "denoisedRejected": stats.denoised_dropped,
        "audit": audit_value,
        "lastError": st.last_error,
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
    let st = lock_state();
    let hits = st.kb.retrieve(query, top_k.min(20));
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
    let kb_snapshot = lock_state().kb.clone();
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
/// params: `{ title: string, text: string }`
/// → `{ ok, ingested, doc_id, documents, chunks, quality:{rejectedByDenoiser, duplicate, audit} }`
pub fn ingest(params: &Value) -> Value {
    if let Some(denied) = rbac_denied("kb.ingest") {
        return denied;
    }
    let title = params
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("untitled")
        .trim()
        .to_string();
    let title = if title.is_empty() {
        "untitled".to_string()
    } else {
        title
    };
    let text = params.get("text").and_then(Value::as_str).unwrap_or("");
    ingest_gated(&title, text)
}

/// 段落级去噪：`filter_documents` 逐段判定（过短 / 模板 / 标点噪声 / 低信息量 / 段间近重复），
/// 返回（保留正文, 丢弃段落数, 丢弃原因）。
///
/// 这是「被丢弃的段落计数」的唯一来源：`denoisedDropped` 统计的是段落数，不是文档数。
fn denoise_paragraphs(title: &str, text: &str) -> (String, u64, Vec<String>) {
    let pairs: Vec<(String, String)> = text
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| (title.to_string(), p.to_string()))
        .collect();
    let (kept, report) = filter_documents(&pairs);
    let clean = kept
        .iter()
        .map(|(_, body)| body.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut reasons: Vec<String> = report
        .reasons
        .iter()
        .map(|(reason, count)| format!("{reason}:{count}"))
        .collect();
    reasons.sort();
    (clean, report.dropped as u64, reasons)
}

/// 入库关卡（`kb.ingest` / `kb.ingest_file` 共用，顺序固定）：
/// 1. 参数校验（空内容 / 单文档 > 2MB / 文档数配额）
/// 2. 去噪（`denoiser::filter_documents`，段落级，统计丢弃段落数）
/// 3. 内容哈希去重（`sha256(title + "\n" + 正文)`）
/// 4. 段落切块 + 索引
/// 5. 图谱索引
/// 6. 完整性审计 + 立即持久化
fn ingest_gated(title: &str, text: &str) -> Value {
    ensure_seeded();
    let mut st = lock_state();
    if text.trim().is_empty() {
        st.kb.stats_mut().rejected += 1;
        persist(&mut st);
        return json!({
            "ok": false,
            "code": "KB_INVALID_PARAMS",
            "error": "text 不可为空",
            "documents": st.kb.document_count(),
            "chunks": st.kb.chunk_count(),
        });
    }
    let documents = st.kb.document_count();
    if let Some((code, message)) = quota_violation(documents, text.len()) {
        st.kb.stats_mut().rejected += 1;
        persist(&mut st);
        return json!({
            "ok": false,
            "code": code,
            "error": message,
            "documents": documents,
            "chunks": st.kb.chunk_count(),
            "size": text.len(),
            "limits": { "maxDocumentBytes": MAX_DOC_BYTES, "maxDocuments": MAX_DOCUMENTS },
        });
    }

    // ① 去噪：逐段判定，保留合格段落
    let (clean, dropped, reasons) = denoise_paragraphs(title, text);
    if dropped > 0 {
        st.kb.stats_mut().denoised_dropped += dropped;
    }
    if clean.trim().is_empty() {
        st.kb.stats_mut().rejected += 1;
        persist(&mut st);
        let (audit_value, _) = audit_view(&st.kb);
        return json!({
            "ok": true,
            "ingested": false,
            "rejectedByDenoiser": true,
            "denoisedDropped": dropped,
            "reasons": reasons,
            "documents": st.kb.document_count(),
            "chunks": st.kb.chunk_count(),
            "quality": { "rejectedByDenoiser": true, "duplicate": false, "audit": audit_value },
        });
    }

    // ② 内容哈希去重：同一标题 + 正文只保留一篇（跨重启有效）
    let hash = document_hash(title, &clean);
    let duplicate_id = st.kb.find_by_hash(&hash).map(str::to_string);
    if let Some(doc_id) = duplicate_id {
        st.kb.stats_mut().duplicate_rejected += 1;
        persist(&mut st);
        let (audit_value, _) = audit_view(&st.kb);
        return json!({
            "ok": true,
            "ingested": false,
            "deduped": true,
            "duplicate": true,
            "doc_id": doc_id,
            "denoisedDropped": dropped,
            "documents": st.kb.document_count(),
            "chunks": st.kb.chunk_count(),
            "quality": { "rejectedByDenoiser": false, "duplicate": true, "audit": audit_value },
        });
    }

    // ③ 切块 + 索引
    let doc_id = st.kb.insert_document(title, &clean, &hash);
    st.kb.stats_mut().last_ingest_at = Some(chrono::Utc::now().to_rfc3339());
    // ④ 同步索引进全局知识图谱（kb.graph.view 的可视化数据源）
    if let Ok(mut graph) = crate::kb::graph::global_graph().lock() {
        graph.index_document(&doc_id, &clean);
    }
    // ⑤ 审计 + 立即持久化
    let (audit_value, _) = audit_view(&st.kb);
    persist(&mut st);
    json!({
        "ok": true,
        "ingested": true,
        "deduped": false,
        "duplicate": false,
        "doc_id": doc_id,
        "denoisedDropped": dropped,
        "documents": st.kb.document_count(),
        "chunks": st.kb.chunk_count(),
        "persisted": st.persisted,
        "quality": { "rejectedByDenoiser": false, "duplicate": false, "audit": audit_value },
    })
}

/// `kb.list`：列出所有已索引文档（含内容哈希与分块数）
pub fn list() -> Value {
    ensure_seeded();
    let st = lock_state();
    let docs: Vec<Value> = st
        .kb
        .documents()
        .iter()
        .map(|d| {
            json!({
                "doc_id": d.doc_id,
                "title": d.title,
                "chunks": d.chunks.len(),
                "hash": d.hash,
            })
        })
        .collect();
    json!({ "documents": docs, "total": docs.len() })
}

/// `kb.delete`：删除指定文档并立即写回索引
pub fn delete(params: &Value) -> Value {
    if let Some(denied) = rbac_denied("kb.delete") {
        return denied;
    }
    let doc_id = params.get("doc_id").and_then(Value::as_str).unwrap_or("");
    if doc_id.is_empty() {
        return json!({ "ok": false, "code": "KB_INVALID_PARAMS", "error": "doc_id 不可为空" });
    }
    let mut st = lock_state();
    let removed = st.kb.delete_document(doc_id);
    if removed {
        persist(&mut st);
    }
    json!({
        "ok": removed,
        "doc_id": doc_id,
        "documents": st.kb.document_count(),
        "chunks": st.kb.chunk_count(),
        "persisted": st.persisted,
    })
}

/// 允许 `kb.ingest_file` 读取的根：显式 workspace 根 / 会话工作区根 + 应用数据目录。
///
/// 显式根支持 `root` / `workspace` / `workspacePath`（前端不同入口的既有键名）；
/// 未显式传根时优先用 `sessionKey` 对应的真实会话工作区根，最后回退进程 cwd。
fn ingest_roots(params: &Value) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for key in ["root", "workspace", "workspacePath", "workspace_path"] {
        if let Some(raw) = params.get(key).and_then(Value::as_str) {
            let raw = raw.trim();
            if !raw.is_empty() {
                roots.push(PathBuf::from(raw));
            }
        }
    }
    // 会话工作区根（多根按主根优先）：前端未显式传 root 时用真实会话归属，
    // 而不是让调用方随便指定一个目录当白名单。
    if roots.is_empty() {
        if let Some(session) = params
            .get("sessionKey")
            .or_else(|| params.get("session_key"))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            roots.extend(crate::gateway::chat::session_project_roots(session));
        }
    }
    if roots.is_empty() {
        if let Ok(cwd) = std::env::current_dir() {
            roots.push(cwd);
        }
    }
    let data = Config::data_dir();
    if !roots.iter().any(|r| r == &data) {
        roots.push(data);
    }
    roots
}

/// 解析并校验 `kb.ingest_file` 的目标路径（越界 → `PATH_OUTSIDE_ROOTS`，不做任何读取）。
pub(crate) fn resolve_ingest_path(params: &Value) -> Result<PathBuf, Value> {
    let path = params.get("path").and_then(Value::as_str).unwrap_or("");
    if path.trim().is_empty() {
        return Err(json!({
            "ok": false,
            "code": "KB_INVALID_PARAMS",
            "error": "path 不可为空",
        }));
    }
    let roots = ingest_roots(params);
    match safe_resolve_any(&roots, path) {
        Ok(p) => Ok(p),
        Err(e) => Err(json!({
            "ok": false,
            "code": "PATH_OUTSIDE_ROOTS",
            "error": format!(
                "只允许读取会话工作区根或应用数据目录内的文件：{e}"
            ),
            "roots": roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>(),
        })),
    }
}

/// `kb.ingest_file`：从磁盘读取文件并灌入知识库。
///
/// 安全边界：路径必须落在 workspace 根或 `Config::data_dir()` 内，文件 ≤ 5 MiB。
pub fn ingest_file(params: &Value) -> Value {
    if let Some(denied) = rbac_denied("kb.ingest_file") {
        return denied;
    }
    ensure_seeded();
    let p = match resolve_ingest_path(params) {
        Ok(p) => p,
        Err(v) => return v,
    };
    if !p.is_file() {
        return json!({
            "ok": false,
            "code": "KB_INVALID_PARAMS",
            "error": format!("文件不存在或不是普通文件: {}", p.display()),
        });
    }
    let metadata = match std::fs::metadata(&p) {
        Ok(m) => m,
        Err(e) => {
            return json!({ "ok": false, "code": "KB_INVALID_PARAMS", "error": format!("读取文件元数据失败: {e}") });
        }
    };
    if metadata.len() > MAX_FILE_BYTES {
        return json!({
            "ok": false,
            "code": "KB_FILE_TOO_LARGE",
            "error": format!(
                "文件 {} 字节超过上限 {MAX_FILE_BYTES} 字节: {}",
                metadata.len(),
                p.display()
            ),
            "size": metadata.len(),
            "limit": MAX_FILE_BYTES,
        });
    }
    let title = params
        .get("title")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            p.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("untitled")
                .to_string()
        });
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) => {
            return json!({ "ok": false, "code": "KB_INVALID_PARAMS", "error": format!("读取文件失败: {e}") });
        }
    };
    let mut result = ingest_gated(&title, &text);
    if let Some(obj) = result.as_object_mut() {
        obj.insert("title".to_string(), json!(title));
        obj.insert("path".to_string(), json!(p.display().to_string()));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique(tag: &str) -> String {
        format!("{tag}-{}", uuid::Uuid::new_v4())
    }

    #[test]
    fn test_kb_ingest_and_search_roundtrip() {
        // ingest 命中唯一特征词 → search 应返回对应文档
        let title = unique("fps-guide");
        let token = format!("ox{}", uuid::Uuid::new_v4().simple());
        let ingest = ingest(&json!({
            "title": title,
            "text": format!("体素竞速游戏使用三缓冲渲染与固定时间步长来保证帧率稳定。{token}"),
        }));
        assert_eq!(ingest["ok"], true, "{ingest}");
        assert_eq!(ingest["ingested"], true, "{ingest}");
        let res = search(&json!({ "query": token, "topK": 3 }));
        let items = res["items"].as_array().cloned().unwrap_or_default();
        assert!(!items.is_empty(), "应检索到灌入的文档: {res}");
        assert!(items.iter().any(|i| i["title"] == title.as_str()));
    }

    #[test]
    fn test_kb_status_reports_documents() {
        let before = status()["documents"].as_u64().unwrap_or(0);
        let _ = ingest(&json!({
            "title": unique("status-probe"),
            "text": format!("唯一性探针文本 {} 用于验证状态统计。", uuid::Uuid::new_v4()),
        }));
        let after = status()["documents"].as_u64().unwrap_or(0);
        assert!(after >= before, "ingest 后文档数不应减少");
    }

    #[test]
    fn test_kb_search_hybrid_uses_vector_engine() {
        let _ = ingest(&json!({
            "title": unique("hybrid-probe"),
            "text": format!("混合检索探针：特征哈希向量与 BM25 融合排序验证。{}", uuid::Uuid::new_v4()),
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

    /// 关卡 2：同标题 + 同正文重复入库 → `duplicate:true` 且不新增。
    #[test]
    fn duplicate_ingest_is_rejected_without_growing_the_index() {
        let title = unique("dup-probe");
        let text = format!(
            "重复入库探针正文：同一份内容只应被索引一次，避免 BM25 语料被稀释。{}",
            uuid::Uuid::new_v4()
        );
        let first = ingest(&json!({ "title": title, "text": text }));
        assert_eq!(first["ingested"], true, "{first}");
        let doc_id = first["doc_id"].as_str().unwrap_or_default().to_string();
        let second = ingest(&json!({ "title": title, "text": text }));
        assert_eq!(second["ok"], true, "{second}");
        assert_eq!(second["duplicate"], true, "{second}");
        assert_eq!(second["doc_id"], doc_id, "必须回指已存在的 doc_id");
        // 进程级单例库会被其它用例并发写入，故按「同标题文档只有一篇」判定，不比较全局快照。
        let listed = list();
        let same_title = listed["documents"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|d| d["title"] == title.as_str())
            .count();
        assert_eq!(same_title, 1, "重复入库不得新增文档: {listed}");
        assert_eq!(second["quality"]["duplicate"], true);
    }

    /// 关卡 1：去噪拒绝（过短文本）→ `rejectedByDenoiser:true`，不新增。
    #[test]
    fn denoiser_rejects_noise_before_indexing() {
        let title = unique("noise");
        let res = ingest(&json!({ "title": title, "text": "hi" }));
        assert_eq!(res["ok"], true, "{res}");
        assert_eq!(res["ingested"], false, "{res}");
        assert_eq!(res["rejectedByDenoiser"], true, "{res}");
        assert_eq!(res["quality"]["rejectedByDenoiser"], true);
        // 单例库是进程级共享的（其它用例并发写入），因此按「该标题是否真的进了索引」判定，
        // 而不是比较全局文档数的前后快照。
        let listed = list();
        let hit = listed["documents"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .any(|d| d["title"] == title.as_str());
        assert!(!hit, "被去噪拒绝的文档不得进入索引: {listed}");
        assert!(status()["denoisedRejected"].as_u64().unwrap_or(0) >= 1);
    }

    /// 配额：5000 篇 / 2 MiB 边界由纯函数判定，超限必须报明确错误码。
    #[test]
    fn quota_rejects_when_document_count_or_size_exceeds_limit() {
        assert!(quota_violation(0, 1).is_none());
        assert!(quota_violation(MAX_DOCUMENTS - 1, MAX_DOC_BYTES).is_none());
        let (code, msg) = quota_violation(MAX_DOCUMENTS, 1).expect("达到 5000 篇必须超限");
        assert_eq!(code, "KB_QUOTA_EXCEEDED");
        assert!(msg.contains("5000"), "{msg}");
        let (code, msg) = quota_violation(0, MAX_DOC_BYTES + 1).expect("超过 2 MiB 必须超限");
        assert_eq!(code, "KB_DOC_TOO_LARGE");
        assert!(msg.contains("2097152"), "{msg}");
    }

    /// 配额真实拒绝路径：超 2 MiB 的正文由 `kb.ingest` 返回错误码，不静默截断。
    #[test]
    fn ingest_over_size_returns_doc_too_large_code() {
        let big = "a".repeat(MAX_DOC_BYTES + 1);
        let res = ingest(&json!({ "title": unique("too-big"), "text": big }));
        assert_eq!(res["ok"], false, "{}", res["error"]);
        assert_eq!(res["code"], "KB_DOC_TOO_LARGE");
    }

    /// 段落级去噪：混合文档只丢噪声段，合格正文仍入库；naive 计数如实累加。
    #[test]
    fn paragraph_denoise_drops_only_noisy_paragraphs() {
        let title = unique("para-denoise");
        let token = format!("pd{}", uuid::Uuid::new_v4().simple());
        let good = format!("本段是有效技术说明，描述三缓冲渲染与固定时间步长的实现细节。{token}");
        let noise = "hi";
        let res = ingest(&json!({
            "title": title,
            "text": format!("{noise}\n\n{good}"),
        }));
        assert_eq!(res["ok"], true, "{res}");
        assert_eq!(res["ingested"], true, "合格段落必须入库: {res}");
        assert!(
            res["denoisedDropped"].as_u64().unwrap_or(0) >= 1,
            "噪声段应被计数并丢弃: {res}"
        );
        let hits = search(&json!({ "query": token, "topK": 3 }));
        assert!(!hits["items"].as_array().unwrap().is_empty(), "{hits}");
    }

    /// 路径越界：只允许 workspace 根 / data_dir 内的文件。
    #[test]
    fn ingest_file_rejects_paths_outside_roots() {
        let base = std::env::temp_dir().join(format!("mgt_kb_deny_{}", uuid::Uuid::new_v4()));
        let root = base.join("ws");
        let outside = base.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "top secret content").unwrap();

        let denied = resolve_ingest_path(&json!({
            "path": outside.join("secret.txt").display().to_string(),
            "root": root.display().to_string(),
        }))
        .expect_err("根外文件必须拒绝");
        assert_eq!(denied["code"], "PATH_OUTSIDE_ROOTS", "{denied}");

        // 根内文件放行
        let inside = root.join("ok.txt");
        std::fs::write(&inside, "workspace 内部可读内容").unwrap();
        let ok = resolve_ingest_path(&json!({
            "path": inside.display().to_string(),
            "root": root.display().to_string(),
        }))
        .expect("根内文件应放行");
        assert_eq!(ok, inside);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 文件过大：`kb.ingest_file` 返回 KB_FILE_TOO_LARGE，而不是把 2 MiB 文本塞进索引。
    #[test]
    fn ingest_file_rejects_oversized_file() {
        let root = std::env::temp_dir().join(format!("mgt_kb_big_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("big.md");
        std::fs::write(&file, vec![b'a'; MAX_FILE_BYTES as usize + 1]).unwrap();
        let res = ingest_file(&json!({
            "path": file.display().to_string(),
            "root": root.display().to_string(),
        }));
        assert_eq!(res["ok"], false, "{res}");
        assert_eq!(res["code"], "KB_FILE_TOO_LARGE", "{res}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 状态契约：持久化标记 / 路径 / 质量计数 / 审计字段齐全。
    #[test]
    fn status_exposes_persistence_and_quality_fields() {
        let v = status();
        for key in [
            "documents",
            "chunks",
            "engine",
            "persisted",
            "storePath",
            "duplicatesRejected",
            "denoisedDropped",
            "rejected",
            "lastIngestAt",
            "sizeBytes",
            "audit",
        ] {
            assert!(v.get(key).is_some(), "status 缺少字段 {key}: {v}");
        }
        assert_eq!(v["engine"], "bm25");
        assert!(v["persisted"].is_boolean());
        assert!(v["sizeBytes"].is_number());
        assert!(v["audit"]["ok"].is_boolean());
        assert!(v["audit"]["issues"].is_array());
        assert!(
            v["storePath"]
                .as_str()
                .is_some_and(|p| p.ends_with("store.json")),
            "storePath 必须指向持久化文件: {v}"
        );
    }
}
