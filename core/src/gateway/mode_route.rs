//! 模式路由（MSC-2：模式切换必须产生可观测的编排差异）。
//!
//! 历史缺陷：网关只在「运行时模式 == Bloom」时改走并行入口，其余模式
//! （Plan / Agent / Ensemble / OmniTranscend）**全部落回同一条单 Agent 路径**，
//! 导致用户「切换与不切换模式路由没什么区别」。
//!
//! 本模块把五档运行时模式（[`ModeKind`]）映射为编排内核的**入口形态**
//! [`ModeRoute`]，作为单一事实源：网关按它分流，编排内核按它偏置形态。
//! 五档映射**两两互异**，故同一输入在五档下的 `router_decision.strategy`
//! 不可能全部相同（≥4 档不同）。

use crate::domain::AgentMode;
use crate::mode::ModeKind;

/// 五档模式路由：运行时模式 → 编排入口形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeRoute {
    /// Plan：先计划后执行（只读探索优先），写类工具硬拒绝。
    PlanFirst,
    /// Agent：直接单 Agent 交互审批（标准助手基线，行为向后兼容）。
    Direct,
    /// Ensemble：多候选受控强编排（专家团队 / 集群）。
    MultiCandidate,
    /// Bloom：锚定深度并行（Captain + Copilot 双模型）与蜂群 / 自主闭环。
    DeepParallel,
    /// OmniTranscend：全谱超模（深度并行 + 蜂群 + 自主闭环，S1→S5 五段）。
    OmniSpectrum,
}

impl ModeRoute {
    /// 路由到编排内核的调用入口名（可观测性 / 前端只读展示用）。
    pub fn dispatch_entry(&self) -> &'static str {
        match self {
            ModeRoute::PlanFirst => "dispatch_plan_first",
            ModeRoute::Direct => "dispatch_direct",
            ModeRoute::MultiCandidate => "dispatch_ensemble",
            ModeRoute::DeepParallel => "dispatch_bloom",
            ModeRoute::OmniSpectrum => "dispatch_omni",
        }
    }

    /// 该路由对应的策略标签（写入 `router_decision.strategy`，供前端 / UI 观测模式
    /// 切换是否真实生效；五档互异）。
    pub fn strategy_label(&self) -> &'static str {
        match self {
            ModeRoute::PlanFirst => "PlanFirst",
            ModeRoute::Direct => "Direct",
            ModeRoute::MultiCandidate => "MultiCandidate",
            ModeRoute::DeepParallel => "DeepParallel",
            ModeRoute::OmniSpectrum => "OmniSpectrum",
        }
    }

    /// 该路由是否允许写类动作（仅 PlanFirst 拒绝；其余放行）。
    pub fn allows_write(&self) -> bool {
        !matches!(self, ModeRoute::PlanFirst)
    }

    /// 是否走 Bloom 并行入口（深度并行 / 全谱超模两档共用同一并行执行体）。
    pub fn uses_bloom_entry(&self) -> bool {
        matches!(self, ModeRoute::DeepParallel | ModeRoute::OmniSpectrum)
    }

    /// 形态偏置：把「分析得出的形态」按路由抬升到该模式应有的**形态下界**。
    ///
    /// - `PlanFirst` → 恒为 [`AgentMode::Solo`]（只读探索，写操作由门控拒绝）；
    /// - `Direct` → 保持分析结果（向后兼容基线）；
    /// - `MultiCandidate` → 至少 [`AgentMode::ExpertTeam`]（受控强编排）；
    /// - `DeepParallel` → [`AgentMode::DualLeader`]（双领队并行）；
    /// - `OmniSpectrum` → [`AgentMode::Cluster`]（大规模并行分发）。
    ///
    /// 交付流水线（[`AgentMode::Pipeline`]）优先于模式偏置，不被覆盖。
    pub fn bias_strategy(&self, analyzed: AgentMode) -> AgentMode {
        if matches!(analyzed, AgentMode::Pipeline) {
            return AgentMode::Pipeline;
        }
        match self {
            ModeRoute::PlanFirst => AgentMode::Solo,
            ModeRoute::Direct => analyzed,
            ModeRoute::MultiCandidate => AgentMode::ExpertTeam,
            ModeRoute::DeepParallel => AgentMode::DualLeader,
            ModeRoute::OmniSpectrum => AgentMode::Cluster,
        }
    }
}

/// 五档模式 → 路由（单一事实源，穷尽匹配保证新增模式必被覆盖）。
pub fn mode_route(kind: ModeKind) -> ModeRoute {
    match kind {
        ModeKind::Plan => ModeRoute::PlanFirst,
        ModeKind::Agent => ModeRoute::Direct,
        ModeKind::Ensemble => ModeRoute::MultiCandidate,
        ModeKind::Bloom => ModeRoute::DeepParallel,
        ModeKind::OmniTranscend => ModeRoute::OmniSpectrum,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_modes_map_to_distinct_routes() {
        let routes: Vec<ModeRoute> = ModeKind::ALL.iter().map(|k| mode_route(*k)).collect();
        // 五档两两互异
        for i in 0..routes.len() {
            for j in (i + 1)..routes.len() {
                assert_ne!(
                    routes[i], routes[j],
                    "模式 {:?} 与 {:?} 映射到同一路由",
                    ModeKind::ALL[i], ModeKind::ALL[j]
                );
            }
        }
        // 显式锚定
        assert_eq!(mode_route(ModeKind::Plan), ModeRoute::PlanFirst);
        assert_eq!(mode_route(ModeKind::Agent), ModeRoute::Direct);
        assert_eq!(mode_route(ModeKind::Ensemble), ModeRoute::MultiCandidate);
        assert_eq!(mode_route(ModeKind::Bloom), ModeRoute::DeepParallel);
        assert_eq!(
            mode_route(ModeKind::OmniTranscend),
            ModeRoute::OmniSpectrum
        );
    }

    #[test]
    fn five_routes_have_distinct_strategy_labels() {
        let labels: Vec<&str> = ModeKind::ALL
            .iter()
            .map(|k| mode_route(*k).strategy_label())
            .collect();
        let mut uniq = labels.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), 5, "五档策略标签必须互异，got {:?}", labels);
    }

    #[test]
    fn plan_route_forbids_write_others_allow() {
        assert!(!mode_route(ModeKind::Plan).allows_write());
        assert!(mode_route(ModeKind::Agent).allows_write());
        assert!(mode_route(ModeKind::Ensemble).allows_write());
        assert!(mode_route(ModeKind::Bloom).allows_write());
        assert!(mode_route(ModeKind::OmniTranscend).allows_write());
    }

    #[test]
    fn bias_strategy_lifts_form_per_route() {
        // PlanFirst：无论分析结果如何都收敛为 Solo（只读探索）
        assert_eq!(
            ModeRoute::PlanFirst.bias_strategy(AgentMode::Cluster),
            AgentMode::Solo
        );
        // Direct：保持分析结果（向后兼容）
        assert_eq!(
            ModeRoute::Direct.bias_strategy(AgentMode::Solo),
            AgentMode::Solo
        );
        assert_eq!(
            ModeRoute::Direct.bias_strategy(AgentMode::Cluster),
            AgentMode::Cluster
        );
        // MultiCandidate：至少 ExpertTeam
        assert_eq!(
            ModeRoute::MultiCandidate.bias_strategy(AgentMode::Solo),
            AgentMode::ExpertTeam
        );
        // DeepParallel / OmniSpectrum
        assert_eq!(
            ModeRoute::DeepParallel.bias_strategy(AgentMode::Solo),
            AgentMode::DualLeader
        );
        assert_eq!(
            ModeRoute::OmniSpectrum.bias_strategy(AgentMode::Solo),
            AgentMode::Cluster
        );
        // 交付流水线优先，不被模式偏置覆盖
        assert_eq!(
            ModeRoute::OmniSpectrum.bias_strategy(AgentMode::Pipeline),
            AgentMode::Pipeline
        );
    }

    #[test]
    fn bloom_entry_shared_by_deep_and_omni_only() {
        assert!(ModeRoute::DeepParallel.uses_bloom_entry());
        assert!(ModeRoute::OmniSpectrum.uses_bloom_entry());
        assert!(!ModeRoute::PlanFirst.uses_bloom_entry());
        assert!(!ModeRoute::Direct.uses_bloom_entry());
        assert!(!ModeRoute::MultiCandidate.uses_bloom_entry());
    }
}
