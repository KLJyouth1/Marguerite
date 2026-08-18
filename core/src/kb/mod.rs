//! 知识库模块（KB / RAG-lite + 质量层）
//!
//! - `store`：文档分块 + BM25 关键词检索（零外部模型依赖）。
//! - `graph`：实体-关系知识图谱（Exclusive 能力 `kb.graph`）。
//! - `denoiser`：索引前去噪（rag_denoiser）。
//! - `integrity`：索引后完整性审计（rag_integrity）。
//!
//! 三者构成「KB 2.0」质量闭环：去噪 → 索引 → 图谱增强 → 审计。

pub mod denoiser;
pub mod gateway;
pub mod graph;
pub mod hybrid;
pub mod integrity;
pub mod store;

pub use denoiser::{filter_documents, noise_reason, NoiseReport};
pub use graph::{Edge, KnowledgeGraph, RelationKind};
pub use hybrid::{HybridResult, HybridRetriever};
pub use integrity::{audit, IntegrityReport};
pub use store::{KnowledgeBase, RetrievalResult};

#[cfg(test)]
mod tests {
    use super::*;

    /// KB 2.0 质量闭环端到端：去噪 → 索引 → 图谱 → 审计。
    #[test]
    fn kb_enhancement_pipeline() {
        let raw = vec![
            (
                "rust".to_string(),
                "Rust and Kubernetes are both used for cloud native systems. Rust uses ownership."
                    .to_string(),
            ),
            (
                "go".to_string(),
                "Go is a simple concurrent language. Go uses goroutines for scheduling."
                    .to_string(),
            ),
            (
                "junk".to_string(),
                "the of and to in on for with is are the of and to in on for with".to_string(),
            ),
            (
                "dup".to_string(),
                "Rust and Kubernetes are both used for cloud native systems. Rust uses ownership."
                    .to_string(),
            ),
        ];

        // 1. 去噪：丢弃 junk（低信息）与 dup（近重复）
        let (clean, report) = filter_documents(&raw);
        assert!(
            report.dropped >= 2,
            "junk + near-dup should drop, report={:?}",
            report
        );
        assert_eq!(clean.len(), 2, "only rust+go kept");

        // 2. 索引 + 图谱增强
        let mut kb = KnowledgeBase::new();
        let mut graph = KnowledgeGraph::new();
        for (t, x) in &clean {
            kb.add_document(t, x);
            graph.index_document(t, x);
        }

        // 3. 图谱应连接 Rust 与 Kubernetes（同块共现）
        assert!(graph.entity_count() >= 2);
        let neighbors = graph.related("Rust");
        assert!(
            neighbors.contains(&"Kubernetes".to_string()),
            "Rust↔Kubernetes should co-occur, neighbors={:?}",
            neighbors
        );

        // 4. 完整性审计应通过（去噪后无噪声/重复）
        let integrity = audit(&kb);
        assert!(integrity.ok, "kb should be healthy, {:?}", integrity);
    }
}
