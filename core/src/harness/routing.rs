//! 动态模型路由门面
//!
//! 在既有 [`ModelRouter`] 之上加一层**策略维度**：调用方给出
//! `IntentType` + `Complexity`，再叠加 [`RoutingPolicy`]，得到子代理应使用的模型。
//!
//! 设计要点：本模块**不重新实现**选模型逻辑，而是把策略折算为「有效复杂度」
//! 后交给 [`ModelRouter::select_model`]，因此路由表始终是 `model::router` 那一份
//! （单一事实源）。策略语义：
//! - [`RoutingPolicy::QualityFirst`]：复杂度上调一档 → 倾向强推理模型；
//! - [`RoutingPolicy::CostFirst`] / [`RoutingPolicy::LatencyFirst`]：复杂度下调一档
//!   → 倾向轻量模型（成本与延迟在本仓路由表中同向，故折算一致，
//!   需要区分时用 [`HarnessRouter::with_override`] 显式钉住模型）；
//! - [`RoutingPolicy::Balanced`]：按原始复杂度直通。
//!
//! 诚实性说明：策略只能在**路由表已配置的模型**之间选择。若某意图缺少对应档位的
//! 规则，会回落到 `ModelRouter` 的兜底模型——此时不同策略可能得到同一模型，
//! 这是配置的真实结果，本层不伪造差异。

use std::collections::HashMap;
use std::sync::Arc;

use crate::model::router::{Complexity, IntentType, ModelRouter};

/// 路由策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoutingPolicy {
    /// 延迟优先：优先轻量模型
    LatencyFirst,
    /// 成本优先：优先轻量模型
    CostFirst,
    /// 质量优先：优先强推理模型
    QualityFirst,
    /// 均衡：按原始复杂度直通
    Balanced,
}

impl RoutingPolicy {
    /// 全部策略
    pub const ALL: [RoutingPolicy; 4] = [
        RoutingPolicy::LatencyFirst,
        RoutingPolicy::CostFirst,
        RoutingPolicy::QualityFirst,
        RoutingPolicy::Balanced,
    ];

    /// 稳定字符串标识
    pub fn as_str(&self) -> &'static str {
        match self {
            RoutingPolicy::LatencyFirst => "latency_first",
            RoutingPolicy::CostFirst => "cost_first",
            RoutingPolicy::QualityFirst => "quality_first",
            RoutingPolicy::Balanced => "balanced",
        }
    }
}

/// 复杂度上调一档
fn escalate(complexity: Complexity) -> Complexity {
    match complexity {
        Complexity::Simple => Complexity::Medium,
        // Medium / Complex 均封顶在 Complex
        _ => Complexity::Complex,
    }
}

/// 复杂度下调一档
fn deescalate(complexity: Complexity) -> Complexity {
    match complexity {
        Complexity::Complex => Complexity::Medium,
        // Medium / Simple 均落到 Simple
        _ => Complexity::Simple,
    }
}

/// 带策略的模型路由门面
pub struct HarnessRouter {
    /// 底层路由器（唯一路由表来源）
    router: Arc<ModelRouter>,
    /// 当前默认策略
    policy: RoutingPolicy,
    /// 策略级模型钉死（绕过复杂度折算，用于运维强制指定）
    overrides: HashMap<RoutingPolicy, String>,
}

impl HarnessRouter {
    /// 创建门面（默认 [`RoutingPolicy::Balanced`]）
    pub fn new(router: Arc<ModelRouter>) -> Self {
        Self {
            router,
            policy: RoutingPolicy::Balanced,
            overrides: HashMap::new(),
        }
    }

    /// 设置默认策略
    pub fn with_policy(mut self, policy: RoutingPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// 为某策略钉死模型名
    pub fn with_override(mut self, policy: RoutingPolicy, model: impl Into<String>) -> Self {
        self.overrides.insert(policy, model.into());
        self
    }

    /// 当前默认策略
    pub fn policy(&self) -> RoutingPolicy {
        self.policy
    }

    /// 切换默认策略
    pub fn set_policy(&mut self, policy: RoutingPolicy) {
        self.policy = policy;
    }

    /// 底层路由器引用（供需要直接发起调用的场景复用同一路由表）
    pub fn router(&self) -> &Arc<ModelRouter> {
        &self.router
    }

    /// 策略折算后的有效复杂度
    pub fn effective_complexity(policy: RoutingPolicy, complexity: Complexity) -> Complexity {
        match policy {
            RoutingPolicy::QualityFirst => escalate(complexity),
            RoutingPolicy::CostFirst | RoutingPolicy::LatencyFirst => deescalate(complexity),
            RoutingPolicy::Balanced => complexity,
        }
    }

    /// 按当前默认策略解析模型名
    pub fn resolve(&self, intent: &IntentType, complexity: Complexity) -> String {
        self.resolve_with(self.policy, intent, complexity)
    }

    /// 按指定策略解析模型名
    pub fn resolve_with(
        &self,
        policy: RoutingPolicy,
        intent: &IntentType,
        complexity: Complexity,
    ) -> String {
        if let Some(model) = self.overrides.get(&policy) {
            return model.clone();
        }
        let effective = Self::effective_complexity(policy, complexity);
        self.router
            .select_model(intent, effective)
            .config
            .model
            .clone()
    }

    /// 列出各策略下的选模型结果（可观测：CLI 可直接展示路由矩阵）
    pub fn explain(
        &self,
        intent: &IntentType,
        complexity: Complexity,
    ) -> Vec<(RoutingPolicy, String)> {
        RoutingPolicy::ALL
            .iter()
            .map(|p| (*p, self.resolve_with(*p, intent, complexity.clone())))
            .collect()
    }
}

impl std::fmt::Debug for HarnessRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HarnessRouter")
            .field("policy", &self.policy)
            .field("overrides", &self.overrides.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::config::Config;

    fn router() -> HarnessRouter {
        let model_router = Arc::new(ModelRouter::new(&Config::default()));
        HarnessRouter::new(model_router)
    }

    #[test]
    fn test_quality_first_differs_from_cost_first() {
        let r = router();
        // 默认路由表：Code+Complex → deepseek-v4-pro(优先级100)，Code+Medium → deepseek-v4-flash(80)
        let quality = r.resolve_with(
            RoutingPolicy::QualityFirst,
            &IntentType::Code,
            Complexity::Complex,
        );
        let cost = r.resolve_with(
            RoutingPolicy::CostFirst,
            &IntentType::Code,
            Complexity::Complex,
        );

        assert_ne!(
            quality, cost,
            "QualityFirst 与 CostFirst 必须选出不同模型（得到 {quality} / {cost}）"
        );
        assert_eq!(quality, "deepseek-v4-pro");
        assert_eq!(cost, "deepseek-v4-flash");
    }

    #[test]
    fn test_balanced_passes_complexity_through() {
        let r = router();
        assert_eq!(
            r.resolve_with(
                RoutingPolicy::Balanced,
                &IntentType::Code,
                Complexity::Medium
            ),
            "deepseek-v4-flash"
        );
        assert_eq!(
            r.resolve_with(
                RoutingPolicy::Balanced,
                &IntentType::Code,
                Complexity::Complex
            ),
            "deepseek-v4-pro"
        );
    }

    #[test]
    fn test_effective_complexity_folding() {
        assert_eq!(
            HarnessRouter::effective_complexity(RoutingPolicy::QualityFirst, Complexity::Simple),
            Complexity::Medium
        );
        assert_eq!(
            HarnessRouter::effective_complexity(RoutingPolicy::QualityFirst, Complexity::Complex),
            Complexity::Complex
        );
        assert_eq!(
            HarnessRouter::effective_complexity(RoutingPolicy::CostFirst, Complexity::Complex),
            Complexity::Medium
        );
        assert_eq!(
            HarnessRouter::effective_complexity(RoutingPolicy::LatencyFirst, Complexity::Medium),
            Complexity::Simple
        );
        assert_eq!(
            HarnessRouter::effective_complexity(RoutingPolicy::Balanced, Complexity::Medium),
            Complexity::Medium
        );
    }

    #[test]
    fn test_override_pins_model() {
        let r = router().with_override(RoutingPolicy::LatencyFirst, "my-edge-model");
        assert_eq!(
            r.resolve_with(
                RoutingPolicy::LatencyFirst,
                &IntentType::Code,
                Complexity::Complex
            ),
            "my-edge-model",
            "override 应绕过复杂度折算"
        );
        // 未被 override 的策略不受影响
        assert_eq!(
            r.resolve_with(
                RoutingPolicy::QualityFirst,
                &IntentType::Code,
                Complexity::Complex
            ),
            "deepseek-v4-pro"
        );
    }

    #[test]
    fn test_default_policy_and_switch() {
        let mut r = router().with_policy(RoutingPolicy::QualityFirst);
        assert_eq!(r.policy(), RoutingPolicy::QualityFirst);
        assert_eq!(
            r.resolve(&IntentType::Code, Complexity::Complex),
            "deepseek-v4-pro"
        );

        r.set_policy(RoutingPolicy::CostFirst);
        assert_eq!(r.policy(), RoutingPolicy::CostFirst);
        assert_eq!(
            r.resolve(&IntentType::Code, Complexity::Complex),
            "deepseek-v4-flash"
        );
    }

    #[test]
    fn test_explain_covers_all_policies() {
        let r = router();
        let matrix = r.explain(&IntentType::Code, Complexity::Complex);
        assert_eq!(matrix.len(), RoutingPolicy::ALL.len());
        assert!(matrix.iter().all(|(_, m)| !m.is_empty()));
    }

    #[test]
    fn test_policy_ids_unique() {
        let mut ids: Vec<&str> = RoutingPolicy::ALL.iter().map(|p| p.as_str()).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }
}
