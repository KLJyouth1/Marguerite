//! 内置审计维度（定义）
//!
//! 借鉴 Workbuddy 10 维轮转审计。此处定义各维度的结构化元数据
//! （ID / 名称 / cron / 描述），真实执行逻辑见 `audit::run_audit`。

use anyhow::Result;

use crate::automation::{Automation, AutomationManager, JobKind};

/// 单个内置审计维度的占位定义
pub struct BuiltinAudit {
    /// 唯一 ID
    pub id: &'static str,
    /// 名称
    pub name: &'static str,
    /// cron 表达式
    pub cron: &'static str,
    /// 描述
    pub description: &'static str,
}

/// 10 维审计轮转占位定义
pub const BUILTIN_AUDITS: &[BuiltinAudit] = &[
    BuiltinAudit {
        id: "audit-code-quality",
        name: "代码质量",
        cron: "0 */2 * * *",
        description: "clippy + format 检查",
    },
    BuiltinAudit {
        id: "audit-security",
        name: "安全扫描",
        cron: "0 */2 * * *",
        description: "cargo audit + 依赖检查",
    },
    BuiltinAudit {
        id: "audit-test-coverage",
        name: "测试覆盖率",
        cron: "0 */4 * * *",
        description: "llvm-cov 趋势",
    },
    BuiltinAudit {
        id: "audit-performance",
        name: "性能基线",
        cron: "0 */4 * * *",
        description: "关键路径耗时",
    },
    BuiltinAudit {
        id: "audit-memory",
        name: "记忆一致性",
        cron: "0 */6 * * *",
        description: "/memory verify",
    },
    BuiltinAudit {
        id: "audit-sandbox",
        name: "沙箱完整性",
        cron: "0 */2 * * *",
        description: "白名单回归",
    },
    BuiltinAudit {
        id: "audit-doc-sync",
        name: "文档同步",
        cron: "0 0 * * *",
        description: "文档 vs 代码",
    },
    BuiltinAudit {
        id: "audit-placeholders",
        name: "零占位门禁",
        cron: "0 0 * * *",
        description: "grep 占位符（每次 build 触发）",
    },
    BuiltinAudit {
        id: "audit-dependencies",
        name: "依赖新鲜度",
        cron: "0 0 * * *",
        description: "过期 crate 检查",
    },
    BuiltinAudit {
        id: "audit-config-drift",
        name: "配置漂移",
        cron: "0 0 * * *",
        description: "config.yaml 变更检测（每次启动触发）",
    },
];

/// 将全部内置审计维度注册到管理器
///
/// 由 build/startup 事件显式触发的两个维度（零占位门禁、配置漂移）默认禁用，
/// 其余按各自频率默认启用。
pub fn register_builtins(mgr: &mut AutomationManager) -> Result<usize> {
    let mut count = 0;
    for b in BUILTIN_AUDITS {
        let mut auto = Automation::new(b.id, b.name, b.cron, JobKind::Audit)?;
        // 这两个不由 cron 轮转，改由 build/startup 事件驱动
        auto.enabled = b.id != "audit-placeholders" && b.id != "audit-config-drift";
        mgr.register(auto)?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_builtins_defined() {
        assert_eq!(BUILTIN_AUDITS.len(), 10);
        // ID 唯一
        let mut ids = BUILTIN_AUDITS.iter().map(|b| b.id).collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 10);
    }

    #[test]
    fn builtins_have_valid_cron() {
        for b in BUILTIN_AUDITS {
            assert!(
                crate::automation::cron::CronScheduler::parse(b.cron).is_ok(),
                "内置 cron 非法: {} ({})",
                b.id,
                b.cron
            );
        }
    }

    #[test]
    fn register_builtins_all_valid() {
        let mut mgr = AutomationManager::new();
        let n = register_builtins(&mut mgr).unwrap();
        assert_eq!(n, 10);
        for b in BUILTIN_AUDITS {
            assert!(mgr.get(b.id).is_some(), "缺失内置: {}", b.id);
        }
    }
}
