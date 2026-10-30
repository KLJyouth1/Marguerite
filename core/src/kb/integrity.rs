//! KB 质量层 · 完整性审计（rag_integrity）
//!
//! 对索引后的知识库做健康检查：重复块、空块、检索分数异常。
//! 依赖 `KnowledgeBase::chunk_texts`（检索底座新增的访问器）。
//!
//! 另外提供**启动自检** [`audit_sources`]：把索引里的文件来源记录（mtime / 字节数）
//! 与磁盘现状逐条对账，不一致时明确给出「需重建」/「需移除」，绝不静默返回「一切正常」。

use crate::kb::store::{mtime_ms, KnowledgeBase};
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

// ───────────────────── 启动自检：索引记录 vs 磁盘 ─────────────────────

/// 单条源文件的对账结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceVerdict {
    /// 索引记录的 mtime / 字节数与磁盘一致，无需重建。
    UpToDate,
    /// 磁盘已变更（mtime 或字节数不符）：**该文件必须重建**。
    NeedsRebuild,
    /// 磁盘上文件已不存在：必须清掉它的节点与悬空边。
    Missing,
}

/// 一条漂移记录（附带可读原因，便于直接展示给用户）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDrift {
    pub path: String,
    pub doc_id: String,
    pub verdict: SourceVerdict,
    pub detail: String,
}

/// 源文件一致性自检报告。
#[derive(Debug, Clone, Default)]
pub struct SourceAuditReport {
    /// 对账过的来源记录数。
    pub checked: usize,
    /// 与磁盘一致（无需重建）的条数。
    pub up_to_date: usize,
    /// 需要重建 / 移除的漂移记录（空 = 无漂移）。
    pub drifts: Vec<SourceDrift>,
    /// 是否存在可对账的来源记录。`false` 表示「没有文件来源，结论不可校验」——
    /// 此时 `ok` 是没有覆盖度的，**不能当成「已校验通过」**。
    pub verified: bool,
    /// 无漂移为 `true`（与 `verified` 分开：无覆盖的 `ok` 不算证据）。
    pub ok: bool,
}

impl SourceAuditReport {
    /// 是否存在需要重建/移除的源文件（含「文件已消失」）。
    pub fn needs_rebuild(&self) -> bool {
        !self.drifts.is_empty()
    }
}

/// 启动自检：逐条比对索引里的文件来源记录与磁盘现状。
///
/// mtime 或字节数不符 → [`SourceVerdict::NeedsRebuild`]；文件不见了 → [`SourceVerdict::Missing`]。
/// 两者都会进 `drifts`，不会被静默吞掉；没有来源记录时 `verified = false`（结论「不可校验」）。
pub fn audit_sources(kb: &KnowledgeBase) -> SourceAuditReport {
    let mut report = SourceAuditReport::default();
    for src in kb.sources() {
        report.checked += 1;
        match std::fs::metadata(&src.path) {
            Err(e) => report.drifts.push(SourceDrift {
                path: src.path.clone(),
                doc_id: src.doc_id.clone(),
                verdict: SourceVerdict::Missing,
                detail: format!("文件已不存在: {e}"),
            }),
            Ok(meta) => {
                let size = meta.len();
                let mtime = mtime_ms(&meta);
                if size != src.size {
                    report.drifts.push(SourceDrift {
                        path: src.path.clone(),
                        doc_id: src.doc_id.clone(),
                        verdict: SourceVerdict::NeedsRebuild,
                        detail: format!("size 从 {} 变为 {size}", src.size),
                    });
                } else if mtime != src.mtime_ms {
                    report.drifts.push(SourceDrift {
                        path: src.path.clone(),
                        doc_id: src.doc_id.clone(),
                        verdict: SourceVerdict::NeedsRebuild,
                        detail: format!("mtime 从 {} 变为 {mtime}（毫秒）", src.mtime_ms),
                    });
                } else {
                    report.up_to_date += 1;
                }
            }
        }
    }
    report.verified = report.checked > 0;
    report.ok = report.drifts.is_empty();
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

    /// 启动自检：mtime 漂移（正文与字节数都没变）必须明确报「需重建」，不得静默返回空。
    #[test]
    fn source_audit_flags_mtime_drift_as_needs_rebuild() {
        use std::time::Duration;

        let dir = std::env::temp_dir().join(format!("mgt_kb_src_audit_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.md");
        std::fs::write(&path, "索引记录与磁盘 mtime/size 对账的源文件。").unwrap();

        let mut kb = KnowledgeBase::new();
        let report = kb.apply_changes(std::slice::from_ref(&path));
        assert_eq!(report.rebuilt.len(), 1, "{report:?}");

        let clean = audit_sources(&kb);
        assert!(clean.verified, "有来源记录即可校验: {clean:?}");
        assert!(clean.ok, "刚入库应无漂移: {clean:?}");
        assert_eq!(clean.up_to_date, 1);
        assert!(clean.drifts.is_empty());

        // 只改 mtime（正文与字节数保持不变）→ 自检必须报「需重建」
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        let later = std::time::SystemTime::now() + Duration::from_secs(5);
        file.set_times(std::fs::FileTimes::new().set_modified(later))
            .expect("设置 mtime");
        drop(file);

        let dirty = audit_sources(&kb);
        assert!(!dirty.ok, "mtime 漂移不得判为健康: {dirty:?}");
        assert!(dirty.needs_rebuild(), "{dirty:?}");
        assert_eq!(dirty.drifts.len(), 1, "{dirty:?}");
        assert_eq!(dirty.drifts[0].verdict, SourceVerdict::NeedsRebuild);
        assert!(
            dirty.drifts[0].detail.contains("mtime"),
            "{:?}",
            dirty.drifts[0]
        );
        assert_eq!(dirty.drifts[0].path, path.display().to_string());

        // 无来源记录 → 结论是「不可校验」，不是「健康」
        let empty = audit_sources(&KnowledgeBase::new());
        assert!(!empty.verified, "空库不得冒充已校验");
        assert_eq!(empty.checked, 0);
        assert_eq!(empty.up_to_date, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
