//! Fleet 多角色模型路由（参考 CodeWhale Fleet 概念）
//!
//! 不同角色绑定不同的 provider/model/reasoning level：
//! - **architect**：强推理模型，负责方案设计和拆解
//! - **builder**：快速模型，负责代码生成和工具调用
//! - **reviewer**：中等模型，负责审查和验证
//! - **cheap-commander**：便宜模型指挥贵模型（CodeWhale 核心模式）
//!
//! Fleet 与现有 `dual.rs`（规划/执行双模型）互补：
//! dual 是固定的 2 角色，Fleet 支持 N 角色 + 角色间消息传递。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::model::llm::LLMConfig;

/// Fleet 中的角色标识
pub type RoleId = String;

/// 角色类型（内置角色 + 自定义）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RoleKind {
    /// 架构师：方案设计、任务拆解（强推理）
    Architect,
    /// 构建者：代码生成、工具调用（快速）
    Builder,
    /// 审查者：代码审查、验证（中等）
    Reviewer,
    /// 便宜指挥者：用便宜模型决定是否需要贵模型
    CheapCommander,
    /// 自定义角色
    Custom(String),
}

impl RoleKind {
    pub fn as_str(&self) -> &str {
        match self {
            RoleKind::Architect => "architect",
            RoleKind::Builder => "builder",
            RoleKind::Reviewer => "reviewer",
            RoleKind::CheapCommander => "cheap_commander",
            RoleKind::Custom(s) => s.as_str(),
        }
    }
}

impl std::fmt::Display for RoleKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 推理级别
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningLevel {
    /// 不使用推理
    None,
    /// 快速推理（最少思考 token）
    Fast,
    /// 标准推理
    #[default]
    Standard,
    /// 深度推理（最多思考 token）
    Deep,
}

/// 单个角色的模型绑定配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleBinding {
    /// 角色类型
    pub kind: RoleKind,
    /// 显示名称
    pub label: String,
    /// 该角色使用的 LLM 配置（provider/model/api_key/base_url）
    pub config: LLMConfig,
    /// 推理级别
    #[serde(default)]
    pub reasoning: ReasoningLevel,
    /// 该角色允许使用的工具名列表（空 = 全部允许）
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// 系统提示词前缀（角色人格）
    #[serde(default)]
    pub system_prompt: String,
    /// 最大并发数（同时处理的任务数）
    #[serde(default = "default_concurrency")]
    pub max_concurrency: usize,
}

fn default_concurrency() -> usize {
    1
}

/// Fleet 配置：一组角色绑定 + 路由规则
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FleetConfig {
    /// 是否启用 Fleet 模式
    #[serde(default)]
    pub enabled: bool,
    /// 角色绑定表（role_id → binding）
    #[serde(default)]
    pub roles: HashMap<RoleId, RoleBinding>,
    /// 默认角色 ID（未指定角色时使用）
    #[serde(default)]
    pub default_role: Option<RoleId>,
    /// 故障转移链：角色失败时依次尝试的角色 ID 列表
    #[serde(default)]
    pub failover_chain: Vec<RoleId>,
}

impl FleetConfig {
    /// 创建默认 Fleet 配置（基于主配置派生）
    pub fn with_defaults(main_config: &LLMConfig) -> Self {
        let mut roles = HashMap::new();

        // architect：同 provider 但优先用 reasoner 模型
        let mut architect_config = main_config.clone();
        if architect_config.model.contains("chat") {
            architect_config.model = architect_config.model.replace("chat", "reasoner");
        }
        roles.insert(
            "architect".to_string(),
            RoleBinding {
                kind: RoleKind::Architect,
                label: "架构师".to_string(),
                config: architect_config,
                reasoning: ReasoningLevel::Deep,
                allowed_tools: vec![],
                system_prompt: "你是架构师，负责分析需求、设计方案、拆解任务。输出结构化计划。"
                    .to_string(),
                max_concurrency: 1,
            },
        );

        // builder：主配置，快速执行
        roles.insert(
            "builder".to_string(),
            RoleBinding {
                kind: RoleKind::Builder,
                label: "构建者".to_string(),
                config: main_config.clone(),
                reasoning: ReasoningLevel::Fast,
                allowed_tools: vec![],
                system_prompt: "你是构建者，负责按计划执行任务、调用工具、编写代码。简洁高效。"
                    .to_string(),
                max_concurrency: 3,
            },
        );

        // reviewer：同配置，标准推理
        roles.insert(
            "reviewer".to_string(),
            RoleBinding {
                kind: RoleKind::Reviewer,
                label: "审查者".to_string(),
                config: main_config.clone(),
                reasoning: ReasoningLevel::Standard,
                allowed_tools: vec![],
                system_prompt: "你是审查者，负责验证结果、发现问题、给出改进建议。严格但建设性。"
                    .to_string(),
                max_concurrency: 1,
            },
        );

        Self {
            enabled: false,
            roles,
            default_role: Some("builder".to_string()),
            failover_chain: vec!["builder".to_string(), "architect".to_string()],
        }
    }

    /// 获取角色绑定
    pub fn get(&self, role_id: &str) -> Option<&RoleBinding> {
        self.roles.get(role_id)
    }

    /// 获取默认角色
    pub fn default_binding(&self) -> Option<&RoleBinding> {
        self.default_role.as_ref().and_then(|id| self.roles.get(id))
    }

    /// 根据任务类型推荐角色
    pub fn recommend_role(&self, task_description: &str) -> &RoleId {
        let lower = task_description.to_lowercase();
        // 设计/规划/架构类任务 → architect
        if lower.contains("设计")
            || lower.contains("规划")
            || lower.contains("架构")
            || lower.contains("plan")
            || lower.contains("design")
            || lower.contains("architect")
        {
            static ARCHITECT: std::sync::LazyLock<String> =
                std::sync::LazyLock::new(|| "architect".to_string());
            return &ARCHITECT;
        }
        // 审查/验证/review 类任务 → reviewer
        if lower.contains("审查")
            || lower.contains("review")
            || lower.contains("验证")
            || lower.contains("检查")
            || lower.contains("audit")
        {
            static REVIEWER: std::sync::LazyLock<String> =
                std::sync::LazyLock::new(|| "reviewer".to_string());
            return &REVIEWER;
        }
        // 默认 → builder
        self.default_role.as_ref().unwrap_or(&BUILDER_FALLBACK)
    }

    /// 添加或更新角色绑定
    pub fn set_role(&mut self, role_id: impl Into<String>, binding: RoleBinding) {
        self.roles.insert(role_id.into(), binding);
    }

    /// 移除角色
    pub fn remove_role(&mut self, role_id: &str) -> Option<RoleBinding> {
        self.roles.remove(role_id)
    }
}

static BUILDER_FALLBACK: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| "builder".to_string());

/// Fleet 路由决策结果
#[derive(Debug, Clone)]
pub struct FleetDecision {
    /// 选中的角色 ID
    pub role_id: RoleId,
    /// 选中的角色绑定
    pub binding: RoleBinding,
    /// 决策来源
    pub source: FleetDecisionSource,
}

/// 决策来源
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FleetDecisionSource {
    /// 显式指定
    Explicit,
    /// 按任务类型推荐
    Recommended,
    /// 默认角色
    Default,
    /// 故障转移
    Failover { from: RoleId, attempt: usize },
}

/// Fleet 路由器：根据角色/任务类型选择模型配置
pub struct FleetRouter {
    config: FleetConfig,
}

impl FleetRouter {
    pub fn new(config: FleetConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &FleetConfig {
        &self.config
    }

    /// 显式指定角色路由
    pub fn route_by_role(&self, role_id: &str) -> Option<FleetDecision> {
        self.config.roles.get(role_id).map(|binding| FleetDecision {
            role_id: role_id.to_string(),
            binding: binding.clone(),
            source: FleetDecisionSource::Explicit,
        })
    }

    /// 按任务描述自动路由
    pub fn route_by_task(&self, task_description: &str) -> Option<FleetDecision> {
        let role_id = self.config.recommend_role(task_description).clone();
        self.config
            .roles
            .get(&role_id)
            .map(|binding| FleetDecision {
                role_id,
                binding: binding.clone(),
                source: FleetDecisionSource::Recommended,
            })
    }

    /// 获取默认路由
    pub fn route_default(&self) -> Option<FleetDecision> {
        let role_id = self.config.default_role.clone()?;
        self.config
            .roles
            .get(&role_id)
            .map(|binding| FleetDecision {
                role_id,
                binding: binding.clone(),
                source: FleetDecisionSource::Default,
            })
    }

    /// 故障转移：获取下一个可用角色
    pub fn route_failover(&self, failed_role: &str) -> Option<FleetDecision> {
        let chain = &self.config.failover_chain;
        let pos = chain.iter().position(|r| r == failed_role)?;
        chain.iter().skip(pos + 1).find_map(|next_id| {
            self.config.roles.get(next_id).map(|binding| FleetDecision {
                role_id: next_id.clone(),
                binding: binding.clone(),
                source: FleetDecisionSource::Failover {
                    from: failed_role.to_string(),
                    attempt: pos + 1,
                },
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> LLMConfig {
        LLMConfig {
            provider: crate::model::llm::LLMProvider::DeepSeek,
            model: "deepseek-chat".to_string(),
            api_key: "test-key".to_string(),
            base_url: "https://api.deepseek.com/v1".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_default_fleet_config() {
        let main = sample_config();
        let fleet = FleetConfig::with_defaults(&main);
        assert!(!fleet.enabled);
        assert_eq!(fleet.roles.len(), 3);
        assert!(fleet.roles.contains_key("architect"));
        assert!(fleet.roles.contains_key("builder"));
        assert!(fleet.roles.contains_key("reviewer"));
        // architect 应使用 reasoner 模型
        assert_eq!(fleet.roles["architect"].config.model, "deepseek-reasoner");
        assert_eq!(fleet.roles["builder"].config.model, "deepseek-chat");
    }

    #[test]
    fn test_route_by_role() {
        let fleet = FleetConfig::with_defaults(&sample_config());
        let router = FleetRouter::new(fleet);
        let decision = router.route_by_role("architect").unwrap();
        assert_eq!(decision.role_id, "architect");
        assert_eq!(decision.binding.kind, RoleKind::Architect);
        assert_eq!(decision.source, FleetDecisionSource::Explicit);
    }

    #[test]
    fn test_route_by_task() {
        let fleet = FleetConfig::with_defaults(&sample_config());
        let router = FleetRouter::new(fleet);
        let d = router.route_by_task("请设计一个微服务架构方案").unwrap();
        assert_eq!(d.role_id, "architect");
        let d = router.route_by_task("帮我审查这段代码").unwrap();
        assert_eq!(d.role_id, "reviewer");
        let d = router.route_by_task("写一个排序函数").unwrap();
        assert_eq!(d.role_id, "builder");
    }

    #[test]
    fn test_failover() {
        let fleet = FleetConfig::with_defaults(&sample_config());
        let router = FleetRouter::new(fleet);
        let fo = router.route_failover("builder").unwrap();
        assert_eq!(fo.role_id, "architect");
        assert!(matches!(fo.source, FleetDecisionSource::Failover { .. }));
    }

    #[test]
    fn test_custom_role() {
        let mut fleet = FleetConfig::with_defaults(&sample_config());
        fleet.set_role(
            "translator",
            RoleBinding {
                kind: RoleKind::Custom("translator".to_string()),
                label: "翻译官".to_string(),
                config: sample_config(),
                reasoning: ReasoningLevel::None,
                allowed_tools: vec![],
                system_prompt: "你是翻译官".to_string(),
                max_concurrency: 2,
            },
        );
        assert_eq!(fleet.roles.len(), 4);
        assert!(fleet.get("translator").is_some());
    }
}
