//! Bloom 特权模型（二次升级：吸收 GWWC MaxModePrivileges 概念，Rust 化）
//!
//! 对齐 MAX 模式 的 `MaxModePrivileges`（--dangerously-skip-permissions /
//! --enable-auto-mode / --allowedTools "*" 等），但不搬迁：权限仍受
//! 沙箱白名单约束（`sandbox::whitelist`），特权只决定「审批语义」——
//! 需要确认还是直接放行，绝不绕过白名单。
//!
//! | 字段                        | 语义（对齐 MAX）                                        |
//! |-----------------------------|----------------------------------------------------------|
//! | `auto_approve`              | enable-auto-mode：工具调用免确认                          |
//! | `skip_permissions`          | dangerously-skip-permissions：跳过预检（含系统级桌面控制放行） |
//! | `allowed_tools`             | 工具白名单模式（"*" = 全部，受沙箱约束）                 |
//! | `max_concurrent_tools`      | 并行工具上限（默认 10，全开 20）                          |
//! | `auto_retry_failures`       | 失败自动重试上限（默认 3，全开 5）                        |
//! | `unrestricted_filesystem`   | 不受限文件系统（仍受快照安全网约束）                      |
//! | `allow_network_all`         | 全网络放行                                                |
//! | `allow_shell_all`           | 全部 shell 命令（仍受白名单 DENY 约束）                   |

use serde::{Deserialize, Serialize};

/// Bloom 特权配置（声明式，可从 YAML `bloom.privileges` 段解析）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct BloomPrivileges {
    /// enable-auto-mode：工具调用免确认（Bloom 默认开）
    pub auto_approve: bool,
    /// dangerously-skip-permissions：跳过权限预检
    pub skip_permissions: bool,
    /// 工具白名单模式（"*" = 全部；逗号分隔列表）
    pub allowed_tools: String,
    /// 并行工具上限
    pub max_concurrent_tools: usize,
    /// 失败自动重试上限
    pub auto_retry_failures: usize,
    /// 不受限文件系统（仍受快照安全网约束）
    pub unrestricted_filesystem: bool,
    /// 全网络放行
    pub allow_network_all: bool,
    /// 全部 shell 命令（仍受白名单 DENY 约束）
    pub allow_shell_all: bool,
    /// DEEP 双模型深度：自动启用双模型协同（统一 Bloom 内化 DEEP）
    #[serde(default)]
    pub deep_dual_model: bool,
}

impl Default for BloomPrivileges {
    fn default() -> Self {
        // 温和默认：免确认 + 全工具 + 网络，但不跳过预检、不放开文件系统边界
        Self {
            auto_approve: true,
            skip_permissions: false,
            allowed_tools: "*".to_string(),
            max_concurrent_tools: 10,
            auto_retry_failures: 3,
            unrestricted_filesystem: false,
            allow_network_all: true,
            allow_shell_all: false,
            deep_dual_model: false,
        }
    }
}

/// EnsemblerEnsemble = BloomPrivileges 别名，兼容旧 spec 代码中引用
pub type EnsemblePrivileges = BloomPrivileges;

impl BloomPrivileges {
    /// 全量特权（对齐 `MaxModePrivileges.maximum()`）——NEXUS 级别
    pub fn maximum() -> Self {
        Self {
            auto_approve: true,
            skip_permissions: true,
            allowed_tools: "*".to_string(),
            max_concurrent_tools: 20,
            auto_retry_failures: 5,
            unrestricted_filesystem: true,
            allow_network_all: true,
            allow_shell_all: true,
            deep_dual_model: true,
        }
    }

    /// 从 CLI 风格 flag 构造（对齐 `MaxModePrivileges.from_flags`）
    pub fn from_flags(
        dangerously_skip_permissions: bool,
        enable_auto_mode: bool,
        allowed_tools: &str,
    ) -> Self {
        Self {
            auto_approve: enable_auto_mode,
            skip_permissions: dangerously_skip_permissions,
            allowed_tools: allowed_tools.to_string(),
            max_concurrent_tools: if enable_auto_mode { 20 } else { 10 },
            auto_retry_failures: if enable_auto_mode { 5 } else { 3 },
            unrestricted_filesystem: dangerously_skip_permissions,
            allow_network_all: dangerously_skip_permissions,
            allow_shell_all: dangerously_skip_permissions,
            deep_dual_model: true,
        }
    }

    /// 指定工具是否被允许（`"*"` = 全允许）
    pub fn tool_allowed(&self, tool: &str) -> bool {
        self.allowed_tools == "*" || self.allowed_tools.split(',').any(|t| t.trim() == tool)
    }

    /// 是否免确认自动执行（Bloom 语义核心）
    pub fn is_auto_approve(&self) -> bool {
        self.auto_approve
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_auto_approve_within_sandbox() {
        let p = BloomPrivileges::default();
        assert!(p.is_auto_approve());
        assert!(p.tool_allowed("edit_file"));
        assert!(p.tool_allowed("exec"));
        // 默认不跳过预检、不放开水 shell 全放行
        assert!(!p.skip_permissions);
        assert!(!p.allow_shell_all);
        assert!(p.allow_network_all);
    }

    #[test]
    fn test_maximum_preset() {
        let m = BloomPrivileges::maximum();
        assert!(m.auto_approve && m.skip_permissions);
        assert_eq!(m.max_concurrent_tools, 20);
        assert_eq!(m.auto_retry_failures, 5);
        assert!(m.unrestricted_filesystem && m.allow_network_all && m.allow_shell_all);
    }

    #[test]
    fn test_from_flags_maps_privileges() {
        let p = BloomPrivileges::from_flags(true, true, "*");
        assert!(p.skip_permissions && p.auto_approve);
        assert_eq!(p.max_concurrent_tools, 20);
        let mild = BloomPrivileges::from_flags(false, false, "edit_file,read_file");
        assert!(!mild.is_auto_approve());
        assert!(mild.tool_allowed("edit_file"));
        assert!(!mild.tool_allowed("exec"));
    }

    #[test]
    fn test_tool_allowed_wildcard_and_list() {
        let p = BloomPrivileges::default();
        assert!(p.tool_allowed("anything"));
        let list = BloomPrivileges {
            allowed_tools: "edit_file, git_commit".to_string(),
            ..Default::default()
        };
        assert!(list.tool_allowed("edit_file"));
        assert!(!list.tool_allowed("exec"));
    }
}
