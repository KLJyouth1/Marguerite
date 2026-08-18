//! KB 质量层 · 完整性审计（rag_integrity）
//!
//! 对索引后的知识库做健康检查：重复块、空块、检索分数异常。
//! 依赖 `KnowledgeBase::chunk_texts`（检索底座新增的访问器）。

use crate::kb::store::KnowledgeBase;
use std::collections::HashSet;

/// 完整性审计报告。
#[derive(Debug, Clone, Default)]
pub struct IntegrityReport {
    pub chunk_total: usize,
    pub duplicate_chunk_pairs: usize,
    pub empty_chunks: usize,
    pub score_anomalies: usize,
    pub ok: bool,
}

fn token_set(s: &str) -> HashSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_string())
        .collect()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    let inter = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        0.0
    } else {
        inter as f64 / union as f64
    }
}

/// 审计知识库健康度。
pub fn audit(kb: &KnowledgeBase) -> IntegrityReport {
    let texts = kb.chunk_texts();
    let mut report = IntegrityReport {
        chunk_total: texts.len(),
        ..Default::default()
    };

    for t in &texts {
        if t.trim().is_empty() {
            report.empty_chunks += 1;
        }
    }

    let sets: Vec<HashSet<String>> = texts.iter().map(|t| token_set(t)).collect();
    for i in 0..sets.len() {
        for j in (i + 1)..sets.len() {
            let exact = texts[i].trim() == texts[j].trim();
            let near = !exact && jaccard(&sets[i], &sets[j]) > 0.85;
            if exact || near {
                report.duplicate_chunk_pairs += 1;
            }
        }
    }

    if let Some(probe) = texts.first() {
        if !probe.trim().is_empty() {
            for r in kb.retrieve(probe, 5) {
                if !r.score.is_finite() || r.score < 0.0 {
                    report.score_anomalies += 1;
                }
            }
        }
    }

    report.ok = report.empty_chunks == 0 && report.score_anomalies == 0;
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_kb_passes() {
        let mut kb = KnowledgeBase::new();
        kb.add_document("a", "Rust owns memory safety via the borrow checker.");
        kb.add_document("b", "Go schedules goroutines across OS threads.");
        let r = audit(&kb);
        assert!(r.ok, "should be healthy, {:?}", r);
        assert_eq!(r.empty_chunks, 0);
        assert_eq!(r.score_anomalies, 0);
    }

    #[test]
    fn detects_empty_chunk() {
        let mut kb = KnowledgeBase::new();
        kb.add_document("x", "valid content about systems programming.");
        // 注入一个空块：通过空文本文档（chunk_texts 会包含空字符串）
        kb.add_document("y", "   ");
        let r = audit(&kb);
        assert!(r.empty_chunks >= 1, "empty chunk detected, {:?}", r);
        assert!(!r.ok);
    }

    #[test]
    fn detects_duplicate_chunks() {
        let mut kb = KnowledgeBase::new();
        let dup = "Kubernetes orchestrates containers at scale with pods.";
        kb.add_document("d1", dup);
        kb.add_document("d2", dup);
        let r = audit(&kb);
        assert!(r.duplicate_chunk_pairs >= 1, "dup pair found, {:?}", r);
    }
}
