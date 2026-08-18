//! 文件级冲突检测与解决（Phase I1）
//!
//! 协作编辑场景：多人基于同一基线快照并行改文件，提交时比对各文件哈希与基线，
//! 偏离基线即视为潜在冲突。复用 `session/rollback` 的快照思想，仅做内容哈希比对，
//! 不引入外部 CRDT 依赖。

use std::collections::HashMap;

/// 文件快照（路径 + 内容哈希）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSnapshot {
    /// 相对路径
    pub path: String,
    /// 内容 sha256（十六进制，由调用方计算）
    pub hash: String,
}

/// 冲突项
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// 冲突文件路径
    pub path: String,
    /// 基线哈希
    pub baseline_hash: String,
    /// 当前哈希（偏离基线）
    pub current_hash: String,
}

/// 冲突检测器：持有会话开始的基线快照
#[derive(Debug, Clone, Default)]
pub struct ConflictDetector {
    baseline: HashMap<String, String>,
}

impl ConflictDetector {
    /// 从基线快照构建
    pub fn new(baseline: &[FileSnapshot]) -> Self {
        let baseline = baseline
            .iter()
            .map(|s| (s.path.clone(), s.hash.clone()))
            .collect();
        Self { baseline }
    }

    /// 检测冲突：当前快照中相对基线被修改的文件
    pub fn detect(&self, current: &[FileSnapshot]) -> Vec<Conflict> {
        let mut out = Vec::new();
        for s in current {
            match self.baseline.get(&s.path) {
                Some(base) if base != &s.hash => out.push(Conflict {
                    path: s.path.clone(),
                    baseline_hash: base.clone(),
                    current_hash: s.hash.clone(),
                }),
                // 基线外的新文件不算冲突（视为新增）
                _ => {}
            }
        }
        out
    }

    /// 解决：将当前快照接受为新的基线（冲突清零）
    pub fn resolve(&mut self, current: &[FileSnapshot]) {
        self.baseline = current
            .iter()
            .map(|s| (s.path.clone(), s.hash.clone()))
            .collect();
    }

    /// 基线文件数
    pub fn baseline_len(&self) -> usize {
        self.baseline.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(path: &str, hash: &str) -> FileSnapshot {
        FileSnapshot {
            path: path.into(),
            hash: hash.into(),
        }
    }

    #[test]
    fn test_no_conflict_when_unchanged() {
        let det = ConflictDetector::new(&[snap("a.rs", "h1"), snap("b.rs", "h2")]);
        let conflicts = det.detect(&[snap("a.rs", "h1"), snap("b.rs", "h2")]);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn test_detect_modified_file_as_conflict() {
        let det = ConflictDetector::new(&[snap("a.rs", "h1"), snap("b.rs", "h2")]);
        let conflicts = det.detect(&[snap("a.rs", "hX"), snap("b.rs", "h2")]);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].path, "a.rs");
        assert_eq!(conflicts[0].baseline_hash, "h1");
        assert_eq!(conflicts[0].current_hash, "hX");
    }

    #[test]
    fn test_new_file_is_not_conflict() {
        let det = ConflictDetector::new(&[snap("a.rs", "h1")]);
        let conflicts = det.detect(&[snap("a.rs", "h1"), snap("c.rs", "h3")]);
        assert!(conflicts.is_empty(), "新增文件不应判为冲突");
    }

    #[test]
    fn test_resolve_clears_conflicts() {
        let mut det = ConflictDetector::new(&[snap("a.rs", "h1")]);
        let current = &[snap("a.rs", "hX")];
        assert_eq!(det.detect(current).len(), 1);
        det.resolve(current);
        assert!(det.detect(current).is_empty());
        assert_eq!(det.baseline_len(), 1);
    }
}
