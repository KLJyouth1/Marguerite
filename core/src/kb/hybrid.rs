//! 混合检索：BM25 + 向量嵌入 + Reciprocal Rank Fusion (RRF)
//!
//! 参考 混合检索引擎 的 SQLite+FTS5 混合检索与 外部参考 的多通道检索：
//! - BM25 擅长精确关键词匹配（零依赖，已有）
//! - 向量嵌入擅长语义相似（需要 embedding provider，可插拔）
//! - RRF 融合两路排名，无需调参分数归一化
//!
//! 当 embedding provider 不可用时，自动降级为纯 BM25。

use std::collections::HashMap;

use crate::kb::store::KnowledgeBase;
use crate::memory::embedding::Embedding;

/// RRF 默认 k 参数（标准值 60，来自 Cormack et al. 2009）
const RRF_K: f64 = 60.0;

/// 混合检索结果（含两路分数与融合分）
#[derive(Debug, Clone)]
pub struct HybridResult {
    pub doc_id: String,
    pub title: String,
    pub chunk: String,
    /// BM25 排名（1-based，0 = 未进入 BM25 top-k）
    pub bm25_rank: usize,
    /// 向量排名（1-based，0 = 未进入向量 top-k）
    pub vector_rank: usize,
    /// RRF 融合分数
    pub fused_score: f64,
    /// BM25 原始分
    pub bm25_score: f64,
    /// 向量余弦相似度
    pub vector_score: f64,
}

/// 混合检索器
pub struct HybridRetriever {
    kb: KnowledgeBase,
    /// 可选嵌入后端（None 时降级为纯 BM25）
    embedder: Option<std::sync::Arc<dyn Embedding>>,
    /// 文档块的嵌入缓存（chunk_index → embedding）
    embeddings: Vec<Vec<f64>>,
    /// BM25 权重
    bm25_weight: f64,
    /// 向量权重
    vector_weight: f64,
}

impl HybridRetriever {
    /// 创建纯 BM25 检索器（无嵌入后端）
    pub fn new(kb: KnowledgeBase) -> Self {
        Self {
            kb,
            embedder: None,
            embeddings: Vec::new(),
            bm25_weight: 1.0,
            vector_weight: 1.0,
        }
    }

    /// 创建带嵌入后端的混合检索器
    pub fn with_embedder(kb: KnowledgeBase, embedder: std::sync::Arc<dyn Embedding>) -> Self {
        Self {
            kb,
            embedder: Some(embedder),
            embeddings: Vec::new(),
            bm25_weight: 1.0,
            vector_weight: 1.0,
        }
    }

    /// 调整 BM25 / 向量权重（默认各 1.0）
    pub fn with_weights(mut self, bm25: f64, vector: f64) -> Self {
        self.bm25_weight = bm25;
        self.vector_weight = vector;
        self
    }

    /// 知识库引用
    pub fn kb(&self) -> &KnowledgeBase {
        &self.kb
    }

    /// 知识库可变引用
    pub fn kb_mut(&mut self) -> &mut KnowledgeBase {
        &mut self.kb
    }

    /// 添加文档并为其生成嵌入（如果有 embedder）
    pub fn add_document(&mut self, title: &str, text: &str) {
        let old_count = self.kb.chunk_count();
        self.kb.add_document(title, text);
        let new_count = self.kb.chunk_count();
        // 为新增的 chunk 生成嵌入
        if let Some(embedder) = &self.embedder {
            let chunks = self.kb.chunk_texts();
            for i in old_count..new_count {
                if let Some(chunk) = chunks.get(i) {
                    self.embeddings.push(embedder.embed(chunk));
                }
            }
        }
    }

    /// 重建全量向量索引（chunk_index -> embedding），供混合检索按需向量化既有语料。
    pub fn reindex_embeddings(&mut self) {
        self.embeddings.clear();
        let Some(embedder) = &self.embedder else {
            return;
        };
        for chunk in self.kb.chunk_texts() {
            self.embeddings.push(embedder.embed(&chunk));
        }
    }

    /// 混合检索：BM25 + 向量 + RRF 融合
    pub fn retrieve(&self, query: &str, top_k: usize) -> Vec<HybridResult> {
        // 1. BM25 检索
        let bm25_results = self.kb.retrieve(query, top_k);
        let bm25_ranked: HashMap<String, (usize, f64)> = bm25_results
            .iter()
            .enumerate()
            .map(|(i, r)| (r.doc_id.clone(), (i + 1, r.score)))
            .collect();

        // 2. 向量检索（如果有 embedder）
        let vector_ranked: HashMap<String, (usize, f64)> = if let Some(embedder) = &self.embedder {
            let query_emb = embedder.embed(query);
            let mut scored: Vec<(usize, f64)> = self
                .embeddings
                .iter()
                .enumerate()
                .filter(|(_, e)| !e.is_empty() && e.len() == query_emb.len())
                .map(|(i, e)| (i, cosine_sim_f64(&query_emb, e)))
                .collect();
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            scored
                .into_iter()
                .take(top_k)
                .enumerate()
                .map(|(rank, (chunk_idx, score))| (format!("chunk-{chunk_idx}"), (rank + 1, score)))
                .collect()
        } else {
            HashMap::new()
        };

        // 3. RRF 融合
        let chunks = self.kb.chunk_texts();
        let mut fused: Vec<HybridResult> = Vec::new();
        // 已并入 fused 的 chunk 索引：避免「BM25 已覆盖的 chunk」在向量回路里再被追加一次
        // （两个回路此前用不同口径的 key —— BM25 用 doc_id、向量用 chunk-N —— 永远判不重，
        // 于是每个向量 chunk 都成重复行）。
        let mut seen_chunks: std::collections::HashSet<usize> = std::collections::HashSet::new();

        // BM25 结果
        for r in &bm25_results {
            let (bm25_rank, bm25_score) = bm25_ranked.get(&r.doc_id).copied().unwrap_or((0, 0.0));
            // 找到该 BM25 命中**自己**对应的 chunk 索引，再取其对应的向量排名。
            // 历史缺陷：这里抓「第一个任意 `chunk-*` 条目」的排名赋给所有 BM25 命中 ——
            // 向量排名与命中无关，且完全取决于 HashMap 的迭代顺序（结果不确定）。
            let chunk_idx = chunks.iter().position(|c| c == &r.chunk);
            if let Some(idx) = chunk_idx {
                seen_chunks.insert(idx);
            }
            let (vector_rank, vector_score) = chunk_idx
                .and_then(|idx| vector_ranked.get(&format!("chunk-{idx}")))
                .copied()
                .unwrap_or((0, 0.0));
            let fused_score = self.rrf_score(bm25_rank, vector_rank);
            fused.push(HybridResult {
                doc_id: r.doc_id.clone(),
                title: r.title.clone(),
                chunk: r.chunk.clone(),
                bm25_rank,
                vector_rank,
                fused_score,
                bm25_score,
                vector_score,
            });
        }

        // 向量结果中 BM25 未覆盖的
        for (key, (vrank, vscore)) in &vector_ranked {
            let Some(idx_str) = key.strip_prefix("chunk-") else {
                continue;
            };
            let Ok(idx) = idx_str.parse::<usize>() else {
                continue;
            };
            if seen_chunks.contains(&idx) {
                continue;
            }
            if let Some(chunk) = chunks.get(idx) {
                let fused_score = self.rrf_score(0, *vrank);
                fused.push(HybridResult {
                    doc_id: key.clone(),
                    title: String::new(),
                    chunk: chunk.clone(),
                    bm25_rank: 0,
                    vector_rank: *vrank,
                    fused_score,
                    bm25_score: 0.0,
                    vector_score: *vscore,
                });
            }
        }

        fused.sort_by(|a, b| {
            b.fused_score
                .partial_cmp(&a.fused_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        fused.truncate(top_k);
        fused
    }

    /// RRF 分数：weight / (k + rank)，rank=0 表示该路未命中
    fn rrf_score(&self, bm25_rank: usize, vector_rank: usize) -> f64 {
        let mut score = 0.0;
        if bm25_rank > 0 {
            score += self.bm25_weight / (RRF_K + bm25_rank as f64);
        }
        if vector_rank > 0 {
            score += self.vector_weight / (RRF_K + vector_rank as f64);
        }
        score
    }
}

/// 余弦相似度（f64）
fn cosine_sim_f64(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i].powi(2);
        nb += b[i].powi(2);
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_sim() {
        let a = [1.0f64, 0.0, 0.0];
        let b = [1.0f64, 0.0, 0.0];
        assert!((cosine_sim_f64(&a, &b) - 1.0).abs() < 1e-6);

        let c = [0.0f64, 1.0, 0.0];
        assert!((cosine_sim_f64(&a, &c) - 0.0).abs() < 1e-6);

        let d = [1.0f64, 1.0, 0.0];
        let sim = cosine_sim_f64(&a, &d);
        assert!((sim - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.001);
    }

    #[test]
    fn test_rrf_score() {
        let retriever = HybridRetriever::new(KnowledgeBase::new());
        // 两路都命中
        let s1 = retriever.rrf_score(1, 1);
        // 只 BM25 命中
        let s2 = retriever.rrf_score(1, 0);
        // 只向量命中
        let s3 = retriever.rrf_score(0, 1);
        assert!(s1 > s2);
        assert!(s1 > s3);
        assert!(s2 > 0.0);
        assert!(s3 > 0.0);
        // 都未命中
        assert_eq!(retriever.rrf_score(0, 0), 0.0);
    }

    #[test]
    fn test_bm25_fallback_retrieval() {
        let mut kb = KnowledgeBase::new();
        kb.add_document(
            "rust",
            "Rust is a systems programming language with ownership.",
        );
        kb.add_document(
            "go",
            "Go is a concurrent programming language with goroutines.",
        );
        kb.add_document(
            "python",
            "Python is a scripting language popular for data science.",
        );

        let retriever = HybridRetriever::new(kb);
        let results = retriever.retrieve("systems programming", 2);
        assert!(!results.is_empty());
        // 无 embedder 时 vector_rank 全为 0
        assert!(results.iter().all(|r| r.vector_rank == 0));
        assert!(results[0].bm25_rank > 0);
    }

    /// 回归守卫：RRF 融合必须给每个命中**自己的**向量排名，且不得重复行。
    ///
    /// 历史缺陷：BM25 回路对每条命中都抓「第一个任意 `chunk-*` 条目」的排名 →
    /// 所有命中共享同一向量排名（且取决于 HashMap 迭代顺序）；向量回路又用
    /// `doc_id` 口径判重（与 `chunk-N` 永不相等）→ 每个向量 chunk 都成重复行。
    #[test]
    fn fusion_uses_each_hits_own_vector_rank_without_duplicates() {
        // 假嵌入器：含 "alpha" → [1,0]，否则 → [0,1]（已 L2 归一化）。
        struct FakeEmbed;
        impl Embedding for FakeEmbed {
            fn embed(&self, text: &str) -> Vec<f64> {
                if text.contains("alpha") {
                    vec![1.0, 0.0]
                } else {
                    vec![0.0, 1.0]
                }
            }
            fn dim(&self) -> usize {
                2
            }
        }

        let mut kb = KnowledgeBase::new();
        kb.add_document("A", "alpha alpha alpha");
        kb.add_document("B", "beta beta beta");
        let mut r = HybridRetriever::with_embedder(kb, std::sync::Arc::new(FakeEmbed));
        r.reindex_embeddings();

        // query 同时含 alpha 与 beta → BM25 命中 A、B；向量只与 A 的 chunk 相似。
        let results = r.retrieve("alpha beta", 10);
        let a = results.iter().find(|x| x.title == "A").expect("应命中 A");
        let b = results.iter().find(|x| x.title == "B").expect("应命中 B");

        // 各自用自己的向量排名：A 相似度最高排 1，B 排 2 —— 绝不共享同一个排名。
        assert_eq!(a.vector_rank, 1, "A 应是向量排名第一");
        assert_eq!(b.vector_rank, 2, "B 应取自己的向量排名，而非借用 A 的");
        assert_ne!(
            a.vector_rank, b.vector_rank,
            "两个命中不得共享同一个向量排名（旧缺陷）"
        );

        // 无重复 chunk（旧缺陷会产出重复行）。
        let mut chunks: Vec<&str> = results.iter().map(|x| x.chunk.as_str()).collect();
        chunks.sort();
        let before = chunks.len();
        chunks.dedup();
        assert_eq!(chunks.len(), before, "结果不得包含重复 chunk：{results:?}");
    }
}
