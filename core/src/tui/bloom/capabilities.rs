//! 引擎能力快照（Bloom 能力面板的数据源）
//!
//! 把核心引擎的差异化能力（原生工具 / OS 沙箱 / KB RAG / 治理门）
//! 聚合成终端可渲染的快照。通过 [`CapabilityProvider`] trait 解耦，
//! 既可由 `Orchestrator` 提供真实数据，也可在单测中用 Mock 注入。
//!
//! 注意：能力「是否启用」一律查 [`crate::domain::capabilities::registry`] 真实成熟度，
//! 不再硬编码恒真——详见 [`CapabilityProvider`] 默认实现。

use crate::domain::capabilities::registry;
use crate::domain::CapabilityId;
use crate::orchestration::Orchestrator;

/// 查真实能力注册表，判断某能力是否已落地。
///
/// `Real` / `Partial` 视为可用，`Narrative`（纯叙事、零实现）视为未落地。
/// 注册表为进程级 `OnceLock` 单例，首次访问自动构建，无副作用、不会 panic。
fn cap_implemented(id: &str) -> bool {
    registry()
        .iter()
        .find(|c| c.id == CapabilityId(id.to_string()))
        .map(|c| c.maturity.is_implemented())
        .unwrap_or(false)
}

/// 能力提供方（核心引擎实现）
///
/// 默认实现直接从 [`crate::domain::capabilities::registry`] 读取真实成熟度，
/// 根除「硬编码恒真」的虚假能力面板——任何标记为 `Narrative`（零实现）的能力
/// 都会在此自动呈现为「未启用」，与 `domain::capabilities` 的单一事实源保持一致。
pub trait CapabilityProvider {
    /// 已注册原生工具数量
    fn tool_count(&self) -> usize;
    /// OS 级命令护栏 + SSRF + 超时沙箱是否启用（查真实注册表 `sec.os_sandbox` 成熟度）
    fn sandbox_enabled(&self) -> bool {
        cap_implemented("sec.os_sandbox")
    }
    /// KB RAG（BM25）是否启用（查真实注册表 `kb.rag` 成熟度）
    fn kb_enabled(&self) -> bool {
        cap_implemented("kb.rag")
    }
    /// 治理门（宪法计划门）是否接入编排（查真实注册表 `gov.plan_gate` 成熟度）
    fn governance_enabled(&self) -> bool {
        cap_implemented("gov.plan_gate")
    }
}

impl CapabilityProvider for Orchestrator {
    fn tool_count(&self) -> usize {
        Orchestrator::tool_count(self)
    }
}

/// 能力快照（终端渲染用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilitySnapshot {
    /// 原生工具数
    pub tools: usize,
    /// OS 沙箱
    pub sandbox: bool,
    /// KB RAG
    pub kb: bool,
    /// 治理门
    pub governance: bool,
    /// 品牌标识
    pub brand: &'static str,
}

impl CapabilitySnapshot {
    /// 从提供方聚合
    pub fn from_provider(p: &dyn CapabilityProvider) -> Self {
        Self {
            tools: p.tool_count(),
            sandbox: p.sandbox_enabled(),
            kb: p.kb_enabled(),
            governance: p.governance_enabled(),
            brand: "MARGUERITE · BLOOM",
        }
    }

    /// 能力计数（用于进度/占比展示）
    pub fn enabled_count(&self) -> usize {
        let mut n = 1; // tools 恒存在
        if self.sandbox {
            n += 1;
        }
        if self.kb {
            n += 1;
        }
        if self.governance {
            n += 1;
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockProvider {
        tools: usize,
    }
    impl CapabilityProvider for MockProvider {
        fn tool_count(&self) -> usize {
            self.tools
        }
    }

    #[test]
    fn test_snapshot_from_mock() {
        let m = MockProvider { tools: 46 };
        let s = CapabilitySnapshot::from_provider(&m);
        assert_eq!(s.tools, 46);
        assert!(s.sandbox && s.kb && s.governance);
        assert_eq!(s.brand, "MARGUERITE · BLOOM");
        assert_eq!(s.enabled_count(), 4);
    }

    #[test]
    fn test_snapshot_without_optional() {
        // 用自定义结构体关闭可选能力
        struct Partial;
        impl CapabilityProvider for Partial {
            fn tool_count(&self) -> usize {
                20
            }
            fn sandbox_enabled(&self) -> bool {
                false
            }
            fn kb_enabled(&self) -> bool {
                false
            }
            fn governance_enabled(&self) -> bool {
                false
            }
        }
        let s = CapabilitySnapshot::from_provider(&Partial);
        assert_eq!(s.tools, 20);
        assert_eq!(s.enabled_count(), 1);
    }
}
