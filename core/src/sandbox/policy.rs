//! 沙箱策略分层（二次升级：吸收 TUI 工作台 SandboxPolicy 分层模型，Rust 化）
//!
//! 对齐 TUI 工作台 `crates/tui/src/sandbox/policy.rs` 的四层策略：
//! - [`SandboxPolicy::DangerFullAccess`]    → 无任何限制（配合快照安全网使用）
//! - [`SandboxPolicy::ReadOnly`]            → 全局只读
//! - [`SandboxPolicy::WorkspaceWrite`]      → 工作区可写 + 可选网络（默认/推荐）
//! - [`SandboxPolicy::ExternalSandbox`]     → 已在外部沙箱中运行，避免双重沙箱
//!
//! 与 [`crate::mode`] 的对应：
//! - plan  → `ReadOnly`
//! - agent → `WorkspaceWrite { network: false }`（写需审批）
//! - bloom → `WorkspaceWrite { network: true }` + 快照安全网（YOLO 语义）

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 命令执行沙箱策略
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SandboxPolicy {
    /// 无任何限制（配合快照安全网使用；对齐 danger-full-access）
    DangerFullAccess,
    /// 全局只读
    ReadOnly,
    /// 工作区可写 + 可选额外根目录 + 可选网络
    WorkspaceWrite {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        writable_roots: Vec<PathBuf>,
        #[serde(default)]
        network_access: bool,
        #[serde(default)]
        exclude_tmpdir: bool,
    },
    /// 已在外部沙箱中运行（避免双重沙箱）
    ExternalSandbox { network_access: bool },
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            network_access: false,
            exclude_tmpdir: false,
        }
    }
}

impl SandboxPolicy {
    /// 工作区可写 + 网络（Bloom 默认）
    pub fn workspace_with_network() -> Self {
        SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            network_access: true,
            exclude_tmpdir: false,
        }
    }

    /// 是否允许全盘写
    pub fn has_full_disk_write_access(&self) -> bool {
        matches!(
            self,
            SandboxPolicy::DangerFullAccess | SandboxPolicy::ExternalSandbox { .. }
        )
    }

    /// 是否允许出网
    pub fn has_network_access(&self) -> bool {
        match self {
            SandboxPolicy::DangerFullAccess => true,
            SandboxPolicy::ReadOnly => false,
            SandboxPolicy::WorkspaceWrite { network_access, .. } => *network_access,
            SandboxPolicy::ExternalSandbox { network_access } => *network_access,
        }
    }

    /// 是否为只读策略
    pub fn is_read_only(&self) -> bool {
        matches!(self, SandboxPolicy::ReadOnly)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_workspace_write_no_network() {
        let p = SandboxPolicy::default();
        assert!(matches!(
            p,
            SandboxPolicy::WorkspaceWrite {
                network_access: false,
                ..
            }
        ));
        assert!(!p.has_network_access());
        assert!(!p.is_read_only());
    }

    #[test]
    fn test_workspace_with_network() {
        let p = SandboxPolicy::workspace_with_network();
        assert!(p.has_network_access());
        assert!(!p.has_full_disk_write_access());
    }

    #[test]
    fn test_danger_and_readonly() {
        assert!(SandboxPolicy::DangerFullAccess.has_full_disk_write_access());
        assert!(SandboxPolicy::DangerFullAccess.has_network_access());
        assert!(SandboxPolicy::ReadOnly.is_read_only());
        assert!(!SandboxPolicy::ReadOnly.has_network_access());
    }

    #[test]
    fn test_external_sandbox_passthrough() {
        let p = SandboxPolicy::ExternalSandbox {
            network_access: true,
        };
        assert!(p.has_full_disk_write_access());
        assert!(p.has_network_access());
    }
}
