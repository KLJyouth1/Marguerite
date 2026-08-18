//! 原生能力注册表（单一事实源）
//!
//! `/harness/capabilities` 的唯一数据来源。每项能力记录其**蒸馏来源**
//! （从哪个业界 harness 提炼出的工程语义）与**成熟度**，避免能力清单
//! 在 CLI / HTTP / 文档三处硬编码而互相漂移。
//!
//! 诚实性约束：`module_path` 必须指向本仓真实存在的实现模块；
//! `maturity` 反映实测状态（有单测覆盖且路径打通才算 `Stable`），
//! 不为「看起来完整」而虚标。

use serde::Serialize;

/// 原生能力标识
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeCapability {
    /// 子代理隔离：独立 token 预算 + 独立 sidechain 转录 + 并发闸门
    SubagentIsolation,
    /// 生命周期钩子：工具调用 / 子代理 / 错误五阶段可插拔
    LifecycleHooks,
    /// 动态模型路由：按意图 + 复杂度 + 策略选模型
    DynamicRouting,
    /// 上下文预算：token 记账、告警与压缩触发
    ContextBudget,
    /// git 检查点：创建可回滚快照并恢复
    GitCheckpoint,
    /// 沙箱分级：按风险等级约束命令与文件访问
    SandboxGrading,
    /// 智能体团队：多角色子代理协同编排（复用 agent::team 的 TeamMember/run）
    AgentTeams,
}

impl NativeCapability {
    /// 全部能力（与 [`CAPABILITIES`] 表一一对应）
    pub const ALL: [NativeCapability; 7] = [
        NativeCapability::SubagentIsolation,
        NativeCapability::LifecycleHooks,
        NativeCapability::DynamicRouting,
        NativeCapability::ContextBudget,
        NativeCapability::GitCheckpoint,
        NativeCapability::SandboxGrading,
        NativeCapability::AgentTeams,
    ];

    /// 稳定字符串标识（CLI / HTTP 输出用，勿随意更名）
    pub fn as_str(&self) -> &'static str {
        match self {
            NativeCapability::SubagentIsolation => "subagent_isolation",
            NativeCapability::LifecycleHooks => "lifecycle_hooks",
            NativeCapability::DynamicRouting => "dynamic_routing",
            NativeCapability::ContextBudget => "context_budget",
            NativeCapability::GitCheckpoint => "git_checkpoint",
            NativeCapability::SandboxGrading => "sandbox_grading",
            NativeCapability::AgentTeams => "agent_teams",
        }
    }
}

/// 能力成熟度
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Maturity {
    /// 实验：接口可能变更
    Experimental,
    /// 公测：接口基本稳定，边界场景待补
    Beta,
    /// 稳定：有单测覆盖且已接线到主流程
    Stable,
}

impl Maturity {
    /// 稳定字符串标识
    pub fn as_str(&self) -> &'static str {
        match self {
            Maturity::Experimental => "experimental",
            Maturity::Beta => "beta",
            Maturity::Stable => "stable",
        }
    }
}

/// 能力元数据
#[derive(Debug, Clone, Copy, Serialize)]
pub struct CapabilityInfo {
    /// 能力标识
    pub capability: NativeCapability,
    /// 一句话说明
    pub summary: &'static str,
    /// 蒸馏来源：该工程语义提炼自哪个业界 harness
    pub distilled_from: &'static str,
    /// 成熟度
    pub maturity: Maturity,
    /// 本仓实现模块路径（必须真实存在）
    pub module_path: &'static str,
}

/// 能力表（单一事实源）
const CAPABILITIES: [CapabilityInfo; 7] = [
    CapabilityInfo {
        capability: NativeCapability::SubagentIsolation,
        summary: "子代理独立预算与 sidechain 转录，并发受信号量闸门限制",
        distilled_from: "业界领先终端工作台实践",
        maturity: Maturity::Stable,
        module_path: "harness::subagent",
    },
    CapabilityInfo {
        capability: NativeCapability::LifecycleHooks,
        summary: "PreToolCall/PostToolCall/PreSubagent/PostSubagent/OnError 五阶段钩子",
        distilled_from: "业界领先终端工作台实践",
        maturity: Maturity::Stable,
        module_path: "harness::hooks",
    },
    CapabilityInfo {
        capability: NativeCapability::DynamicRouting,
        summary: "按意图 + 复杂度 + 路由策略动态选择子代理模型",
        distilled_from: "开源智能体 CLI 实践",
        maturity: Maturity::Stable,
        module_path: "harness::routing",
    },
    CapabilityInfo {
        capability: NativeCapability::ContextBudget,
        summary: "每子代理独立 token 记账，超阈值触发分页压缩",
        distilled_from: "开源结对编程实践",
        maturity: Maturity::Stable,
        module_path: "harness::subagent + context::budget",
    },
    CapabilityInfo {
        capability: NativeCapability::GitCheckpoint,
        summary: "commit-tree 悬挂提交快照与 checkout 回滚，全程免 shell",
        distilled_from: "开源结对编程实践",
        maturity: Maturity::Beta,
        module_path: "harness::checkpoint",
    },
    CapabilityInfo {
        capability: NativeCapability::SandboxGrading,
        summary: "按风险等级分级约束命令执行与文件访问",
        distilled_from: "主流 CLI 工作台实践",
        maturity: Maturity::Beta,
        module_path: "sandbox",
    },
    CapabilityInfo {
        capability: NativeCapability::AgentTeams,
        summary: "多角色子代理协同编排，复用 agent::team 的 TeamMember/run 扇出任务",
        distilled_from: "开源智能体团队协作实践",
        maturity: Maturity::Beta,
        module_path: "agent::team",
    },
];

/// 能力注册表
///
/// 无状态查询门面：所有能力元数据来自编译期常量表，避免运行时构造漂移。
#[derive(Debug, Clone, Copy, Default)]
pub struct CapabilityRegistry;

impl CapabilityRegistry {
    /// 创建注册表
    pub fn new() -> Self {
        Self
    }

    /// 全部能力元数据
    pub fn all(&self) -> &'static [CapabilityInfo] {
        &CAPABILITIES
    }

    /// 查询单项能力元数据
    pub fn get(&self, capability: NativeCapability) -> Option<&'static CapabilityInfo> {
        CAPABILITIES.iter().find(|c| c.capability == capability)
    }

    /// 是否支持该能力
    pub fn supports(&self, capability: NativeCapability) -> bool {
        self.get(capability).is_some()
    }

    /// 能力总数
    pub fn len(&self) -> usize {
        CAPABILITIES.len()
    }

    /// 是否为空（恒为 false，满足 clippy 对 `len` 的配对要求）
    pub fn is_empty(&self) -> bool {
        CAPABILITIES.is_empty()
    }

    /// 按成熟度筛选
    pub fn by_maturity(&self, maturity: Maturity) -> Vec<&'static CapabilityInfo> {
        CAPABILITIES
            .iter()
            .filter(|c| c.maturity == maturity)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_covers_every_capability() {
        let reg = CapabilityRegistry::new();
        assert_eq!(reg.len(), NativeCapability::ALL.len());
        for cap in NativeCapability::ALL {
            assert!(reg.supports(cap), "能力 {} 缺少元数据", cap.as_str());
        }
    }

    #[test]
    fn test_capability_ids_unique_and_stable() {
        let mut ids: Vec<&str> = NativeCapability::ALL.iter().map(|c| c.as_str()).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "能力标识必须唯一");
    }

    #[test]
    fn test_metadata_is_non_empty() {
        let reg = CapabilityRegistry::new();
        for info in reg.all() {
            assert!(!info.summary.is_empty(), "summary 不可为空");
            assert!(!info.distilled_from.is_empty(), "蒸馏来源不可为空");
            assert!(!info.module_path.is_empty(), "module_path 不可为空");
        }
    }

    #[test]
    fn test_by_maturity_partitions_table() {
        let reg = CapabilityRegistry::new();
        let total = reg.by_maturity(Maturity::Stable).len()
            + reg.by_maturity(Maturity::Beta).len()
            + reg.by_maturity(Maturity::Experimental).len();
        assert_eq!(total, reg.len(), "成熟度筛选应覆盖全表");
    }

    #[test]
    fn test_capability_serializes_to_snake_case() {
        let json = serde_json::to_string(&NativeCapability::SubagentIsolation).unwrap();
        assert_eq!(json, "\"subagent_isolation\"");
        let json = serde_json::to_string(&Maturity::Stable).unwrap();
        assert_eq!(json, "\"stable\"");
    }
}
