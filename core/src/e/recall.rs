//! Track E — 知识图谱召回接口 + 关键词实现。
//!
//! 定义可插拔的 `Recall` trait 与一个纯关键词实现 `KeywordRecall`（用 `nlp` 原语做词重叠打分）。
//! 真实 embedding 召回（接 bge）为后续切片；本切片只给接口 + 关键词实现，证明召回链路可测。

use std::collections::HashMap;

use crate::e::nlp;

/// 一次召回命中的节点与得分。
#[derive(Debug, Clone, PartialEq)]
pub struct RecallHit {
    pub node: String,
    pub score: f64,
}

/// 可插拔召回接口：给定查询返回 top_k 命中（按 score 降序）。
pub trait Recall {
    fn recall(&self, query: &str, top_k: usize) -> Vec<RecallHit>;
}

/// 关键词召回：以 `node_id -> 节点文本` 的倒排/线性索引，用 Jaccard 词重叠打分。
#[derive(Debug, Clone, Default)]
pub struct KeywordRecall {
    /// node_id -> 节点文本（如 KG 节点描述 / 实体表层形式）。
    pub index: HashMap<String, String>,
}

impl KeywordRecall {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_index(index: HashMap<String, String>) -> Self {
        Self { index }
    }

    pub fn insert(&mut self, node: impl Into<String>, text: impl Into<String>) {
        self.index.insert(node.into(), text.into());
    }
}

impl Recall for KeywordRecall {
    fn recall(&self, query: &str, top_k: usize) -> Vec<RecallHit> {
        let q = nlp::tokenize_zh(query);
        let mut hits: Vec<RecallHit> = self
            .index
            .iter()
            .filter_map(|(node, text)| {
                let t = nlp::tokenize_zh(text);
                let score = nlp::jaccard(&q, &t);
                if score > 0.0 {
                    Some(RecallHit {
                        node: node.clone(),
                        score,
                    })
                } else {
                    None
                }
            })
            .collect();

        // 稳定按 score 降序（score 均来自 jaccard，有限非 NaN）。
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(top_k);
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> KeywordRecall {
        let mut r = KeywordRecall::new();
        r.insert("n1", "机器学习 模型");
        r.insert("n2", "天气预报 气温");
        r.insert("n3", "深度学习 神经网络");
        r
    }

    #[test]
    fn empty_index_returns_empty() {
        let r = KeywordRecall::new();
        assert!(r.recall("anything", 5).is_empty());
    }

    #[test]
    fn recall_top_k_ordered() {
        let r = sample();
        let hits = r.recall("机器学习 模型", 2);
        assert_eq!(hits.len(), 2);
        // n1 与查询几乎全等 -> 最高分且 ≈ 1.0
        assert_eq!(hits[0].node, "n1");
        assert!((hits[0].score - 1.0).abs() < 1e-9);
        // n3 仅有部分重叠（学/习），排第二；n2 零重叠被排除
        assert_eq!(hits[1].node, "n3");
        assert!(hits[1].score > 0.0 && hits[1].score < 1.0);
    }

    #[test]
    fn recall_excludes_zero_overlap() {
        let r = sample();
        let hits = r.recall("天气预报 气温", 10);
        // 只有 n2 命中，n1/n3 零重叠
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].node, "n2");
    }

    #[test]
    fn recall_truncates_to_top_k() {
        let r = sample();
        // 查询同时命中 n1 与 n3（部分重叠），top_k=1 只返回最佳
        let hits = r.recall("机器学习 深度学习", 1);
        assert_eq!(hits.len(), 1);
    }
}
