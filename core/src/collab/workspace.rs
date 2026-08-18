//! 共享工作区同步原语（Phase I1，轻量）
//!
//! 中心服务（Marguerite Relay）可选；小团队可用「共享仓库 + 事件流导出」的对等模式。
//! 本模块提供文件级同步状态追踪（最后写入者 + 内容哈希），供冲突解决与增量同步使用。

use std::collections::HashMap;

/// 单文件同步状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncState {
    /// 最后写入成员 ID
    pub last_writer: String,
    /// 内容哈希
    pub hash: String,
}

/// 工作区同步追踪器
#[derive(Debug, Clone, Default)]
pub struct WorkspaceSync {
    states: HashMap<String, SyncState>,
}

impl WorkspaceSync {
    /// 记录一次同步（成员写入某文件）
    pub fn mark_synced(&mut self, path: &str, member: &str, hash: &str) {
        self.states.insert(
            path.into(),
            SyncState {
                last_writer: member.into(),
                hash: hash.into(),
            },
        );
    }

    /// 是否需要同步：路径不存在，或哈希与记录不一致
    pub fn needs_sync(&self, path: &str, hash: &str) -> bool {
        match self.states.get(path) {
            Some(s) => s.hash != hash,
            None => true,
        }
    }

    /// 取某文件同步状态
    pub fn state_of(&self, path: &str) -> Option<&SyncState> {
        self.states.get(path)
    }

    /// 已追踪文件数
    pub fn tracked(&self) -> usize {
        self.states.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mark_and_needs_sync() {
        let mut w = WorkspaceSync::default();
        assert!(w.needs_sync("a.rs", "h1")); // 未追踪 → 需同步
        w.mark_synced("a.rs", "u1", "h1");
        assert!(!w.needs_sync("a.rs", "h1"));
        assert!(w.needs_sync("a.rs", "h2")); // 哈希变化 → 需同步
        assert_eq!(w.state_of("a.rs").unwrap().last_writer, "u1");
        assert_eq!(w.tracked(), 1);
    }
}
