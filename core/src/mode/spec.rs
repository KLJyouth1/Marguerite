//! 模式规格（声明式 YAML）
//!
//! 每种模式一份 YAML：工具白/黑名单、模型参数、反模式、审批策略。
//! 内置（plan/agent/bloom/bloom_auto/bloom_swarm），可通过 `~/.marguerite/modes/*.yaml` 覆盖。

use serde::{Deserialize, Serialize};

use crate::guard::gateway::RiskLevel;
use crate::mode::privileges::BloomPrivileges;
use crate::sandbox::policy::SandboxPolicy;

/// 审批动作
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalAction {
    /// 直接允许
    Allow,
    /// 需确认
    Confirm,
}

/// 审批策略（按工具类别）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalPolicy {
    /// 写操作（edit_file/write_code/...）
    #[serde(default = "default_confirm")]
    pub mutating: ApprovalAction,
    /// 写类命令执行（exec/terminal）
    #[serde(default = "default_confirm")]
    pub exec_write: ApprovalAction,
    /// 出网操作（web_fetch/web_search/...）
    #[serde(default = "default_confirm")]
    pub network: ApprovalAction,
    /// 桌面控制
    #[serde(default = "default_confirm")]
    pub desktop: ApprovalAction,
    /// 审批超时（秒）
    #[serde(default = "default_timeout")]
    pub fallback_timeout_sec: u64,
}

fn default_confirm() -> ApprovalAction {
    ApprovalAction::Confirm
}

fn default_timeout() -> u64 {
    60
}

impl Default for ApprovalPolicy {
    fn default() -> Self {
        Self {
            mutating: ApprovalAction::Confirm,
            exec_write: ApprovalAction::Confirm,
            network: ApprovalAction::Confirm,
            desktop: ApprovalAction::Confirm,
            fallback_timeout_sec: 60,
        }
    }
}

/// 模型参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelParams {
    /// 采样温度
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    /// 思考深度
    #[serde(default = "default_reasoning")]
    pub reasoning_effort: String,
}

fn default_temperature() -> f64 {
    0.7
}

fn default_reasoning() -> String {
    "medium".to_string()
}

impl Default for ModelParams {
    fn default() -> Self {
        Self {
            temperature: default_temperature(),
            reasoning_effort: default_reasoning(),
        }
    }
}

/// 模式规格
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeSpec {
    /// 模式名
    pub name: String,
    /// 描述
    #[serde(default)]
    pub description: String,
    /// 允许的工具列表（空 = 全部，受沙箱约束）
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// 禁止的工具列表
    #[serde(default)]
    pub denied_tools: Vec<String>,
    /// 模型参数
    #[serde(default)]
    pub model_params: ModelParams,
    /// 反模式（提示词约束）
    #[serde(default)]
    pub anti_patterns: Vec<String>,
    /// 风险等级
    #[serde(default)]
    pub risk_level: RiskLevel,
    /// 审批策略
    #[serde(default)]
    pub approval_policy: ApprovalPolicy,
    /// Bloom 增强段（`bloom:` YAML 键）：MAX+YOLO 融合配置，仅 Bloom 系列模式使用
    #[serde(default)]
    pub bloom: Option<BloomSpec>,
}

/// Bloom 增强规格（二次升级）：MAX（4 阶段推理 + 特权）+ YOLO（沙箱分层 + 快照安全网）融合配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BloomSpec {
    /// 子模式标识：deep（4 阶段推理）/ auto（自主闭环）/ swarm（双领队蜂群）
    pub submode: String,
    /// 特权（对齐 MaxModePrivileges）
    pub privileges: BloomPrivileges,
    /// 沙箱策略（对齐 TUI 工作台 SandboxPolicy）
    pub sandbox: SandboxPolicy,
    /// 4 阶段推理深度（Generation→Critique→Refinement×N→Verification）
    pub thinking_depth: usize,
    /// 验证轮数
    pub verification_passes: usize,
    /// 思考强度（low/medium/high/max）
    pub reasoning_effort: String,
    /// 采样温度（MAX 默认 0.05~0.1，收敛更稳）
    pub temperature: f64,
    /// 是否启用侧 git 快照安全网（YOLO：每轮前快照 + 失败自动回退）
    pub snapshot_enabled: bool,
    /// 失败后自动回退轮数上限
    pub auto_revert_max: usize,
    /// 蜂群分片上限（submode=swarm 时生效）
    pub max_swarm_shards: usize,
    /// 专家团队子专家上限（submode=swarm 时生效）
    #[serde(default = "default_max_sub_experts")]
    pub max_sub_experts: usize,
    /// 自动重试失败工具上限
    pub auto_retry_failures: usize,
    /// 自主闭环 token 预算（submode=auto）
    #[serde(default = "default_budget_tokens")]
    pub budget_tokens: usize,
    /// 自主闭环最大迭代轮数（submode=auto）
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    /// 自主闭环自动修复上限（submode=auto）
    #[serde(default = "default_max_fixes")]
    pub max_fixes: usize,
}

fn default_max_sub_experts() -> usize {
    5
}

fn default_budget_tokens() -> usize {
    100_000
}

fn default_max_iterations() -> usize {
    20
}

fn default_max_fixes() -> usize {
    3
}

impl Default for BloomSpec {
    fn default() -> Self {
        Self {
            submode: "deep".to_string(),
            privileges: BloomPrivileges::default(),
            sandbox: SandboxPolicy::workspace_with_network(),
            thinking_depth: 3,
            verification_passes: 3,
            reasoning_effort: "max".to_string(),
            temperature: 0.1,
            snapshot_enabled: true,
            auto_revert_max: 2,
            max_swarm_shards: 11,
            max_sub_experts: 5,
            auto_retry_failures: 3,
            budget_tokens: 100_000,
            max_iterations: 20,
            max_fixes: 3,
        }
    }
}

impl ModeSpec {
    /// 从 YAML 解析
    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }
}

/// 内置模式规格（include_str! 嵌入，零运行时文件依赖）
pub mod builtin {
    use super::ModeSpec;

    pub fn plan() -> ModeSpec {
        ModeSpec::from_yaml(include_str!("modes/plan.yaml")).expect("内置 plan.yaml 必须合法")
    }

    pub fn agent() -> ModeSpec {
        ModeSpec::from_yaml(include_str!("modes/agent.yaml")).expect("内置 agent.yaml 必须合法")
    }

    pub fn bloom() -> ModeSpec {
        ModeSpec::from_yaml(include_str!("modes/bloom.yaml")).expect("内置 bloom.yaml 必须合法")
    }

    /// Bloom 自主闭环子模式（bloom:auto）：预算驱动，QA 全过才停
    pub fn bloom_auto() -> ModeSpec {
        ModeSpec::from_yaml(include_str!("modes/bloom_auto.yaml"))
            .expect("内置 bloom_auto.yaml 必须合法")
    }

    /// Bloom 双领队蜂群子模式（bloom:swarm）：FlashSwarm + ProExpert + 蜂群/专家团队
    pub fn bloom_swarm() -> ModeSpec {
        ModeSpec::from_yaml(include_str!("modes/bloom_swarm.yaml"))
            .expect("内置 bloom_swarm.yaml 必须合法")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_specs_parse() {
        let plan = builtin::plan();
        assert_eq!(plan.name, "plan");
        assert_eq!(plan.risk_level, RiskLevel::Low);
        assert!(plan.denied_tools.contains(&"edit_file".to_string()));

        let agent = builtin::agent();
        assert_eq!(agent.name, "agent");
        assert_eq!(agent.approval_policy.mutating, ApprovalAction::Confirm);

        let bloom = builtin::bloom();
        assert_eq!(bloom.name, "bloom");
        assert_eq!(bloom.approval_policy.mutating, ApprovalAction::Allow);
    }

    #[test]
    fn test_spec_from_yaml_missing_fields_defaults() {
        let yaml = r#"name: custom
description: "自定义模式"
"#;
        let spec = ModeSpec::from_yaml(yaml).unwrap();
        assert!(spec.allowed_tools.is_empty());
        assert!(spec.denied_tools.is_empty());
        assert_eq!(spec.model_params.temperature, 0.7);
        assert_eq!(spec.risk_level, RiskLevel::Low);
        assert_eq!(spec.approval_policy.fallback_timeout_sec, 60);
    }

    #[test]
    fn test_spec_from_yaml_invalid() {
        assert!(ModeSpec::from_yaml("not: [valid").is_err());
    }

    #[test]
    fn test_model_params_custom() {
        let yaml = r#"name: custom
model_params: { temperature: 0.3, reasoning_effort: high }
risk_level: high
approval_policy: { mutating: allow, exec_write: allow, network: allow, desktop: allow, fallback_timeout_sec: 30 }
"#;
        let spec = ModeSpec::from_yaml(yaml).unwrap();
        assert_eq!(spec.model_params.temperature, 0.3);
        assert_eq!(spec.model_params.reasoning_effort, "high");
        assert_eq!(spec.risk_level, RiskLevel::High);
        assert_eq!(spec.approval_policy.mutating, ApprovalAction::Allow);
        assert_eq!(spec.approval_policy.fallback_timeout_sec, 30);
    }

    #[test]
    fn test_builtin_bloom_parses_bloom_spec() {
        // bloom.yaml 的 bloom: 段（MAX+YOLO 融合配置）应完整解析
        let spec = builtin::bloom();
        let b = spec.bloom.as_ref().expect("bloom.yaml 必须含 bloom: 段");
        assert_eq!(b.submode, "deep");
        assert!(b.privileges.is_auto_approve());
        assert!(b.privileges.tool_allowed("exec"));
        assert_eq!(b.privileges.max_concurrent_tools, 20);
        assert!(b.sandbox.has_network_access());
        assert!(b.snapshot_enabled);
        assert_eq!(b.thinking_depth, 3);
        assert_eq!(b.verification_passes, 3);
        assert_eq!(b.reasoning_effort, "max");
    }

    #[test]
    fn test_bloom_spec_yaml_roundtrip_defaults() {
        // 最小 bloom: 段 → 其余字段走默认
        let yaml = r#"name: bloom
bloom:
  submode: deep
"#;
        let spec = ModeSpec::from_yaml(yaml).unwrap();
        let b = spec.bloom.unwrap();
        assert_eq!(b.submode, "deep");
        assert!(b.privileges.is_auto_approve());
        assert!(b.sandbox.has_network_access());
        assert_eq!(b.thinking_depth, 3);
        assert!(b.snapshot_enabled);
    }

    #[test]
    fn test_bloom_spec_privileges_restriction() {
        // 指定 allowed_tools 列表 → 白名单外工具被特权拒绝
        let yaml = r#"name: bloom
bloom:
  privileges:
    auto_approve: true
    allowed_tools: "edit_file, read_file"
"#;
        let spec = ModeSpec::from_yaml(yaml).unwrap();
        let b = spec.bloom.unwrap();
        assert!(b.privileges.tool_allowed("edit_file"));
        assert!(!b.privileges.tool_allowed("exec"));
    }

    #[test]
    fn test_bloom_swarm_spec_parses_swarm_fields() {
        let spec = builtin::bloom_swarm();
        let b = spec
            .bloom
            .as_ref()
            .expect("bloom_swarm.yaml 必须含 bloom: 段");
        assert_eq!(b.submode, "swarm");
        assert_eq!(b.max_swarm_shards, 11);
        assert_eq!(b.max_sub_experts, 5);
    }

    #[test]
    fn test_bloom_auto_spec_parses_auto_fields() {
        let spec = builtin::bloom_auto();
        let b = spec
            .bloom
            .as_ref()
            .expect("bloom_auto.yaml 必须含 bloom: 段");
        assert_eq!(b.submode, "auto");
        assert_eq!(b.budget_tokens, 100_000);
        assert_eq!(b.max_iterations, 20);
        assert_eq!(b.max_fixes, 3);
    }

    #[test]
    fn test_bloom_spec_auto_swarm_fields_default() {
        // 最小 bloom: 段 → auto/swarm 专属字段走默认值
        let yaml = r#"name: bloom
bloom:
  submode: deep
"#;
        let spec = ModeSpec::from_yaml(yaml).unwrap();
        let b = spec.bloom.unwrap();
        assert_eq!(b.max_sub_experts, 5);
        assert_eq!(b.budget_tokens, 100_000);
        assert_eq!(b.max_iterations, 20);
        assert_eq!(b.max_fixes, 3);
    }
}
