//! 恢复内核（统一 E 期模块名）
//!
//! 复用已验证的会话级快照 / 回滚引擎（`session::rollback::SideGit`，tar + sha256 完整性），
//! 在其上提供「命名检查点（named checkpoint）」能力——`SideGit` 本身只按轮次 / 消息快照，
//! 缺少人类可读的持久标签映射。本模块补齐这一差异点，且不重复实现快照 / 还原逻辑。

pub use crate::session::rollback::SideGit;

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// 命名检查点（标签 → 快照 ID 的持久映射）
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Checkpoint {
    label: String,
    snapshot_id: String,
    message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CheckpointIndex {
    map: HashMap<String, Checkpoint>,
}

/// 恢复管理器：SideGit（轮次快照）+ 命名检查点索引
pub struct RecoveryManager {
    engine: SideGit,
    index: CheckpointIndex,
    index_path: PathBuf,
}

impl RecoveryManager {
    /// 基于工作区与会话 ID 构造；检查点索引持久化到
    /// `$workspace/.marguerite-sidegit/<session>/recovery-index.json`
    pub fn new(workspace: &Path, session_id: &str) -> Result<Self> {
        let engine = SideGit::new(workspace, session_id)?;
        let index_path = workspace
            .join(".marguerite-sidegit")
            .join(session_id)
            .join("recovery-index.json");
        let index = if index_path.exists() {
            let raw = std::fs::read(&index_path)?;
            serde_json::from_slice(&raw).unwrap_or_default()
        } else {
            CheckpointIndex::default()
        };
        Ok(Self {
            engine,
            index,
            index_path,
        })
    }

    fn persist(&self) -> Result<()> {
        if let Some(parent) = self.index_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let raw = serde_json::to_vec(&self.index)?;
        let mut f = std::fs::File::create(&self.index_path)?;
        f.write_all(&raw)?;
        Ok(())
    }

    /// 打一个命名检查点（底层走 SideGit 增量快照）
    pub fn checkpoint(&mut self, label: &str, message: &str) -> Result<String> {
        let snap = self.engine.snapshot_before_turn(message)?;
        self.index.map.insert(
            label.to_string(),
            Checkpoint {
                label: label.to_string(),
                snapshot_id: snap.id.clone(),
                message: message.to_string(),
            },
        );
        self.persist()?;
        Ok(snap.id)
    }

    /// 按标签恢复到对应快照
    pub fn restore_by_label(&self, label: &str) -> Result<()> {
        let cp = self
            .index
            .map
            .get(label)
            .ok_or_else(|| anyhow!("checkpoint '{}' not found", label))?;
        self.engine.restore(&cp.snapshot_id)
    }

    /// 列出所有检查点标签
    pub fn labels(&self) -> Vec<String> {
        self.index.map.keys().cloned().collect()
    }

    /// 是否存在某标签
    pub fn has(&self, label: &str) -> bool {
        self.index.map.contains_key(label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_ws(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("marg_recovery_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn checkpoint_and_restore_by_label() {
        let ws = tmp_ws("ck");
        let mut mgr = RecoveryManager::new(&ws, "sess1").unwrap();
        let f = ws.join("code.rs");
        let mut fh = std::fs::File::create(&f).unwrap();
        fh.write_all(b"fn main() {}").unwrap();
        drop(fh);

        let id = mgr
            .checkpoint("before-refactor", "snapshot before refactor")
            .unwrap();
        assert!(!id.is_empty());
        assert!(mgr.has("before-refactor"));
        assert!(mgr.labels().contains(&"before-refactor".to_string()));

        // 篡改文件后按标签恢复，应成功（底层 SideGit 还原）
        let mut fh = std::fs::OpenOptions::new()
            .write(true)
            .open(ws.join("code.rs"))
            .unwrap();
        fh.write_all(b"fn main() { broken }").unwrap();
        drop(fh);
        assert!(mgr.restore_by_label("before-refactor").is_ok());
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn restore_unknown_label_errors() {
        let ws = tmp_ws("ck2");
        let mgr = RecoveryManager::new(&ws, "sess2").unwrap();
        assert!(mgr.restore_by_label("nope").is_err());
        let _ = std::fs::remove_dir_all(&ws);
    }
}
