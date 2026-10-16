//! 模式规格（声明式 YAML）
//!
//! 每种模式一份 YAML：工具白/黑名单、模型参数、反模式、审批策略。
//! 内置（plan/agent/bloom），可通过 `~/.marguerite/modes/*.yaml` 覆盖。

use serde::{Deserialize, Serialize};

use crate::guard::gateway::RiskLevel;
use crate::mode::privileges::BloomPrivileges;
use crate::sandbox::policy::SandboxPolicy;

/// Bloom 锚定冷启动规格（dsh-forge 社区验证版 + PTC 合并）
///
/// 首轮走「锚定」：仅暴露 2 个最小工具（bash_persistent +
/// str_replace_editor）；首块文本经 `prefix_gate` 门控（deepseek 系
/// "we" 且无 "let me" → 晋升；四步兜底；无工具首轮自动晋升）。
/// 晋升后进入 PTC 阶段：单一 `ptc_tool`（run_code）；之后再按需
/// 单调扩展到 Bloom 全工具面（中途不回退、不替换）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BloomAnchoringSpec {
    /// 是否启用锚定冷启动（统一 Bloom 默认开启）
    pub enabled: bool,
    /// 前缀门控家族（当前支持 "deepseek"；其他家族走通用门控）
    pub prefix_gate: String,
    /// 锚定期最小工具集（默认 bash_persistent + str_replace_editor）
    pub minimal_tools: Vec<String>,
    /// PTC 阶段工具（默认 run_code）
    pub ptc_tool: String,
    /// 兜底晋升步数（默认 4，对齐 FALLBACK_STEPS）
    pub fallback_steps: u32,
    /// 锚定期消息源白名单（默认 user + goal）：非白名单来源不进模型
    /// （对齐梁神 issue #578：过滤后的 /goal 自动轮不得触发晋升分支）
    #[serde(default = "default_message_sources")]
    pub message_sources: Vec<String>,
    /// 首轮请求输出预算（token；None = 不设限）。梁神用 1024 压住「We need」
    /// 触发窗口，晋升后由装配层剥离，绝不残留进后续请求。
    #[serde(default)]
    pub bootstrap_max_tokens: Option<u32>,
    /// 晋升后延迟注入的来源（默认 agent-instructions + skill-catalog）：
    /// workspace 指令与技能目录晚一步注入，先让 PTC 单工具轨迹稳定。
    #[serde(default = "default_deferred_sources")]
    pub deferred_sources: Vec<String>,
    /// 晋升后延迟步数（默认 1）
    #[serde(default = "default_deferred_grace_steps")]
    pub deferred_grace_steps: u32,
    /// 无工具首轮自动晋升（默认 true，向后兼容既有行为）
    #[serde(default = "default_promote_after_first_response")]
    pub promote_after_first_response: bool,
}

fn default_message_sources() -> Vec<String> {
    vec!["user".to_string(), "goal".to_string()]
}

fn default_deferred_sources() -> Vec<String> {
    vec![
        "agent-instructions".to_string(),
        "skill-catalog".to_string(),
    ]
}

fn default_deferred_grace_steps() -> u32 {
    1
}

fn default_promote_after_first_response() -> bool {
    true
}

impl Default for BloomAnchoringSpec {
    fn default() -> Self {
        Self {
            enabled: true,
            prefix_gate: "deepseek".to_string(),
            minimal_tools: vec![
                "bash_persistent".to_string(),
                "str_replace_editor".to_string(),
            ],
            ptc_tool: "run_code".to_string(),
            fallback_steps: 4,
            message_sources: default_message_sources(),
            bootstrap_max_tokens: None,
            deferred_sources: default_deferred_sources(),
            deferred_grace_steps: default_deferred_grace_steps(),
            promote_after_first_response: default_promote_after_first_response(),
        }
    }
}

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
    /// 模式提示段（可选）：作为该模式的静态前缀段注入模型消息装配
    /// （仅追加、不重写历史——红线 #1）。plan 模式携带 DSH plan-mode 同款规则段。
    #[serde(default)]
    pub prompt_section: Option<String>,
    /// Bloom 增强段（`bloom:` YAML 键）：MAX+YOLO 融合配置，仅 Bloom 系列模式使用
    #[serde(default)]
    pub bloom: Option<BloomSpec>,
}

/// Bloom 增强规格（二次升级）：MAX（4 阶段推理 + 特权）+ YOLO（沙箱分层 + 快照安全网）融合配置。
///
/// 统一 Bloom 模式（不再有 submode 子模式开关）：deep 双模型并行、auto 自主闭环、
/// swarm 双领队蜂群、PTC 程序化工具调用、锚定冷启动全部内建，按任务分析自动选用。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BloomSpec {
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
    /// 蜂群分片上限（统一 Bloom 内并行攻坚时生效）
    pub max_swarm_shards: usize,
    /// 专家团队子专家上限（统一 Bloom 内并行攻坚时生效）
    #[serde(default = "default_max_sub_experts")]
    pub max_sub_experts: usize,
    /// 自动重试失败工具上限
    pub auto_retry_failures: usize,
    /// 自主闭环 token 预算（统一 Bloom 自主闭环生效）
    #[serde(default = "default_budget_tokens")]
    pub budget_tokens: usize,
    /// 自主闭环最大迭代轮数（统一 Bloom 自主闭环生效）
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    /// 自主闭环自动修复上限（统一 Bloom 自主闭环生效）
    #[serde(default = "default_max_fixes")]
    pub max_fixes: usize,
    /// 锚定冷启动规格：首轮锚定 + 工具收窄 → 晋升到 PTC。统一 Bloom 默认开启。
    #[serde(default)]
    pub anchoring: BloomAnchoringSpec,
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
            anchoring: BloomAnchoringSpec::default(),
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
    use crate::mode::ModeKind;

    pub fn plan() -> ModeSpec {
        ModeSpec::from_yaml(include_str!("modes/plan.yaml")).expect("内置 plan.yaml 必须合法")
    }

    pub fn agent() -> ModeSpec {
        ModeSpec::from_yaml(include_str!("modes/agent.yaml")).expect("内置 agent.yaml 必须合法")
    }

    pub fn bloom() -> ModeSpec {
        ModeSpec::from_yaml(include_str!("modes/bloom.yaml")).expect("内置 bloom.yaml 必须合法")
    }

    /// Ensemble 扁平强编排（不做锚定三阶段晋升）
    ///
    /// 历史缺陷：`modes/ensemble.yaml` 从未被 `include_str!` 引用，是纯死文件；
    /// `mode.spec` 遇到 `ModeKind::Ensemble` 时冒用 `bloom()` 规格，导致
    /// 「Ensemble 与 Bloom 参数完全一致」的假象。此处接回真实规格。
    pub fn ensemble() -> ModeSpec {
        ModeSpec::from_yaml(include_str!("modes/ensemble.yaml"))
            .expect("内置 ensemble.yaml 必须合法")
    }

    /// 全谱超模 Omni-Transcend（超星 x）
    pub fn omni_transcend() -> ModeSpec {
        ModeSpec::from_yaml(include_str!("modes/omni_transcend.yaml"))
            .expect("内置 omni_transcend.yaml 必须合法")
    }

    /// 按模式类型取内置规格（单一事实源；网关 / CLI / TUI 共用）
    pub fn for_kind(kind: ModeKind) -> ModeSpec {
        match kind {
            ModeKind::Plan => plan(),
            ModeKind::Agent => agent(),
            ModeKind::Ensemble => ensemble(),
            ModeKind::Bloom => bloom(),
            ModeKind::OmniTranscend => omni_transcend(),
        }
    }

    /// 全部内置规格（顺序对齐 [`ModeKind::ALL`]）
    pub fn all() -> Vec<ModeSpec> {
        ModeKind::ALL.iter().copied().map(for_kind).collect()
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

    /// 契约守卫：`ModeKind::ALL` 的每一种模式都必须有可解析的内置规格，
    /// 且 name 字段与 `as_str()` 严格一致（防止 YAML 与枚举漂移）。
    #[test]
    fn test_every_mode_kind_has_a_parsable_builtin_spec() {
        use crate::mode::ModeKind;
        assert_eq!(ModeKind::ALL.len(), 5, "模式总数应为 5");
        for kind in ModeKind::ALL {
            let spec = builtin::for_kind(*kind);
            assert_eq!(
                spec.name,
                kind.as_str(),
                "{} 的 YAML name 字段与 ModeKind::as_str() 不一致",
                kind.as_str()
            );
            assert!(
                !spec.description.trim().is_empty(),
                "{} 缺少 description（模式选择器需要展示）",
                kind.as_str()
            );
            assert!(
                ModeKind::from_name(kind.as_str()) == Some(*kind),
                "{} 无法通过 from_name 往返解析",
                kind.as_str()
            );
            assert!(
                !kind.label().trim().is_empty(),
                "{} 缺少 label",
                kind.as_str()
            );
        }
        assert_eq!(builtin::all().len(), 5);
    }

    /// 契约守卫：ensemble.yaml 必须真正解析出 BloomSpec（历史上它是死文件，
    /// `mode.spec` 冒用 bloom() 规格），且以 anchoring 关闭区别于 Bloom。
    #[test]
    fn test_ensemble_spec_is_live_and_distinct_from_bloom() {
        let e = builtin::ensemble();
        assert_eq!(e.name, "ensemble");
        assert_eq!(e.risk_level, RiskLevel::High);
        assert_eq!(e.approval_policy.mutating, ApprovalAction::Allow);
        let eb = e
            .bloom
            .as_ref()
            .expect("ensemble.yaml 的 bloom: 块必须解析成功，否则又变回死文件");
        // 原文件声明的数值逐字保留
        assert_eq!(eb.thinking_depth, 3);
        assert_eq!(eb.verification_passes, 3);
        assert_eq!(eb.reasoning_effort, "max");
        assert_eq!(eb.max_swarm_shards, 11);
        assert_eq!(eb.auto_retry_failures, 5);
        assert_eq!(eb.privileges.max_concurrent_tools, 20);
        assert!(eb.snapshot_enabled);
        assert_eq!(eb.auto_revert_max, 2);
        // 与 Bloom 的实质差异：Ensemble 扁平编排，不做锚定三阶段晋升
        assert!(!eb.anchoring.enabled, "Ensemble 不应启用锚定冷启动");
        let b = builtin::bloom();
        let bb = b.bloom.as_ref().expect("bloom 应有 BloomSpec");
        assert!(bb.anchoring.enabled, "Bloom 必须启用锚定冷启动");
    }

    #[test]
    fn test_omni_transcend_spec_parses_with_liangshen_anchoring() {
        let s = builtin::omni_transcend();
        assert_eq!(s.name, "omni-transcend");
        assert_eq!(s.risk_level, RiskLevel::High);
        assert_eq!(s.approval_policy.mutating, ApprovalAction::Allow);
        // 梁神两阶段锚定：最小工具 2 件（bash_persistent + str_replace_editor）
        let bloom = s.bloom.as_ref().expect("omni-transcend 应配置 bloom");
        assert_eq!(bloom.anchoring.minimal_tools.len(), 2);
        assert_eq!(bloom.anchoring.minimal_tools[0], "bash_persistent");
        assert_eq!(bloom.anchoring.minimal_tools[1], "str_replace_editor");
        assert!(bloom.anchoring.promote_after_first_response);
        assert_eq!(bloom.anchoring.bootstrap_max_tokens, Some(1024));
        // MAX 强化：thinking_depth 4 + 验证 4 遍 + 推理 max
        assert_eq!(bloom.thinking_depth, 4);
        assert_eq!(bloom.verification_passes, 4);
        assert_eq!(bloom.reasoning_effort, "max");
        // YOLO：免确认 + 快照安全网
        assert!(bloom.privileges.auto_approve);
        assert!(bloom.snapshot_enabled);
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
  privileges:
    auto_approve: true
"#;
        let spec = ModeSpec::from_yaml(yaml).unwrap();
        let b = spec.bloom.unwrap();
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
    fn test_bloom_spec_auto_swarm_fields_default() {
        // 最小 bloom: 段 → auto/swarm 专属字段走默认值
        let yaml = r#"name: bloom
bloom:
  privileges:
    auto_approve: true
"#;
        let spec = ModeSpec::from_yaml(yaml).unwrap();
        let b = spec.bloom.unwrap();
        assert_eq!(b.max_sub_experts, 5);
        assert_eq!(b.budget_tokens, 100_000);
        assert_eq!(b.max_iterations, 20);
        assert_eq!(b.max_fixes, 3);
    }

    #[test]
    fn test_bloom_anchoring_defaults_enable_deepseek_prefix_gate() {
        // 最小 bloom: 段 → anchoring 默认开启、前缀门控为 deepseek、最小工具 2 件
        let yaml = r#"name: bloom
bloom:
  privileges:
    auto_approve: true
"#;
        let spec = ModeSpec::from_yaml(yaml).unwrap();
        let b = spec.bloom.unwrap();
        assert!(b.anchoring.enabled);
        assert_eq!(b.anchoring.prefix_gate, "deepseek");
        assert_eq!(b.anchoring.minimal_tools.len(), 2);
        assert!(b
            .anchoring
            .minimal_tools
            .iter()
            .any(|t| t == "bash_persistent"));
        assert!(b
            .anchoring
            .minimal_tools
            .iter()
            .any(|t| t == "str_replace_editor"));
        assert_eq!(b.anchoring.ptc_tool, "run_code");
        assert_eq!(b.anchoring.fallback_steps, 4);
    }

    #[test]
    fn test_bloom_anchoring_roundtrip_overrides() {
        let yaml = r#"name: bloom
bloom:
  privileges:
    auto_approve: true
  anchoring:
    enabled: true
    prefix_gate: deepseek
    minimal_tools: [a, b]
    ptc_tool: c
    fallback_steps: 5
"#;
        let spec = ModeSpec::from_yaml(yaml).unwrap();
        let b = spec.bloom.unwrap();
        assert_eq!(b.anchoring.minimal_tools, vec!["a", "b"]);
        assert_eq!(b.anchoring.ptc_tool, "c");
        assert_eq!(b.anchoring.fallback_steps, 5);
    }

    #[test]
    fn test_bloom_deep_dual_model_flag() {
        // 内置 bloom.yaml：deep_dual_model=true（DEEP 内化）
        let spec = builtin::bloom();
        let b = spec.bloom.as_ref().unwrap();
        assert!(b.privileges.deep_dual_model);
    }

    /// 端到端守卫：声明的思考深度必须真的换算出更多推理轮数。
    ///
    /// 此前 `thinking_depth` / `verification_passes` 在执行层零消费，改多改少都无差别；
    /// 现由 `DeepPlan::from_mode_spec` 接线。本测试盯住「OmniTranscend 必须比 Bloom 更深」，
    /// 若日后有人把 omni_transcend.yaml 的深度改回与 bloom 相同，这里会立刻失败。
    #[test]
    fn test_declared_depth_translates_to_more_reasoning_rounds() {
        use crate::model::ensemble_deep::DeepPlan;

        let bloom = builtin::bloom().bloom.expect("bloom.yaml 必须带 bloom 段");
        let omni = builtin::omni_transcend()
            .bloom
            .expect("omni_transcend.yaml 必须带 bloom 段");

        let p_bloom = DeepPlan::from_mode_spec(bloom.thinking_depth, bloom.verification_passes);
        let p_omni = DeepPlan::from_mode_spec(omni.thinking_depth, omni.verification_passes);

        assert!(
            omni.thinking_depth > bloom.thinking_depth,
            "OmniTranscend 声明的推理深度应高于 Bloom"
        );
        assert!(
            p_omni.max_llm_calls() > p_bloom.max_llm_calls(),
            "深度差异必须落到实际轮数：omni={} bloom={}",
            p_omni.max_llm_calls(),
            p_bloom.max_llm_calls()
        );
        // 任何带 bloom 段的模式，轮数下限都不得退化成 0
        for kind in crate::mode::ModeKind::ALL {
            if let Some(b) = builtin::for_kind(*kind).bloom {
                let p = DeepPlan::from_mode_spec(b.thinking_depth, b.verification_passes);
                assert!(
                    p.refine_rounds >= 1 && p.verify_rounds >= 1,
                    "模式 {} 的轮数不得为 0",
                    kind.as_str()
                );
            }
        }
    }
}
