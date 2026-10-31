//! 战术推演：回答「蜂群 / 专家团 / 子智能体 / 主智能体该怎么组合」而不靠口号。
//!
//! # 为什么需要它
//!
//! 多智能体协作的选型长期靠两种东西：厂商给的分类法，和事后总结的轶事。两者都
//! 回答不了「换一种战术，这次会不会更好」。这个模块把**协作战术**当成可计算的
//! 对象：输入任务 DAG、执行者吞吐、失败先验，输出每条战术的成功率分布、p90 耗时、
//! p90 成本、失败归因，再用**显式判据**裁决——判据不过就是出局，不给加权总分留
//! 「综合考虑」的余地。
//!
//! # 三条物理规则（其余结论都由它们推出来）
//!
//! 1. **任务是一个有依赖的活**（DAG）。节点没完成，后继不能开始。
//! 2. **执行者是有限资源**（吞吐 × 并发位）。并行只买时间，不买正确性。
//! 3. **错误只有在被检查时才存在**。没有验证闸门时，失败节点的缺陷会静默传播，
//!    收口时表现为「莫名其妙的整体不对」；有闸门时，同一缺陷变成一次可重试的
//!    显式失败。这是「多智能体事故」最主要的物理来源，也是给战术目录配
//!    「有闸门 / 无闸门」对照唯一严肃的理由。
//!
//! # 诚实红线：未知 ≠ 0
//!
//! 每个先验都带样本数 `n` 与来源；`n = 0` 的一律进 [`Model::unknowns`]，随报告一起
//! 返回给前端并标注「这个数字没有实测支撑」。推演可以建立在声明先验上，但**不得把
//! 声明包装成事实**——这条与 `crate::context::compact_state` 是同一条红线。
//!
//! # 模块划分
//!
//! - [`model`]：数据模型、战术目录、参数解析与 DAG 校验。
//! - [`sim`]：确定性蒙特卡洛（同一 seed ⇒ 同一结果，便于反事实对照）。
//! - [`advise`]：判据裁决、推荐、每条落选者的理由、事前验尸。
//! - [`replay`]：把 harness 台账里的一次真实运行重放成场景，再做反事实对照与 5-Why。

pub mod advise;
pub mod model;
pub mod replay;
pub mod sim;

pub use advise::{evaluate, premortem, Criteria, KillCheck, PremortemItem, Rationale, TacticVerdict, WhyNot};
pub use model::{
    catalog, Model, Prior, PriorSource, Scenario, TacticalTask, Tactic, Topology, Unknown,
};
pub use replay::{
    replay, DeltaRow, MeasuredItem, ObservedRun, ReplayGap, ReplayReport, WhyKind, WhyLink,
};
pub use sim::{simulate, Failure, TacticResult, TrialOutcome};

use serde::Serialize;

/// 一次完整推演（RPC `tactics.simulate` 的返回体）。
///
/// 刻意把 `unknowns` 放在顶层而不是塞进 `model`：前端可以整块忽略 `model`，
/// 但**不能忽略「哪些数字是猜的」**——它就在推荐理由旁边。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    /// 按 [`catalog`] 顺序的推演结果；同一 seed 下顺序与内容都稳定
    pub results: Vec<TacticResult>,
    pub rationale: Rationale,
    /// 针对推荐战术的事前验尸；没有可行战术时为空
    pub premortem: Vec<PremortemItem>,
    /// 非空 = 本次推演里有先验没有实测支撑，结果只能当相对排序用
    pub unknowns: Vec<Unknown>,
    pub total_work_k: f64,
    pub trials: usize,
    pub seed: u64,
}

/// 跑完整流程：目录 → 逐条推演 → 判据裁决 → 事前验尸。
pub fn plan(scenario: &Scenario, criteria: &Criteria) -> Plan {
    let tactics = catalog();
    let results: Vec<TacticResult> = tactics
        .iter()
        .map(|t| simulate(scenario, t))
        .collect();
    let rationale = evaluate(&results, criteria);
    let premortem = rationale
        .recommended
        .as_ref()
        .and_then(|id| results.iter().find(|r| r.tactic == *id))
        .map(|winner| premortem(scenario, winner))
        .unwrap_or_default();
    Plan {
        results,
        rationale,
        premortem,
        unknowns: scenario.model.unknowns(),
        total_work_k: scenario.total_work_k(),
        trials: scenario.trials,
        seed: scenario.seed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plan_is_reproducible_for_the_same_seed() {
        let params = json!({
            "tasks": [
                {"id": "a", "workK": 30},
                {"id": "b", "workK": 20, "dependsOn": ["a"]},
                {"id": "c", "workK": 20, "dependsOn": ["a"]}
            ],
            "trials": 200,
            "seed": 7
        });
        let scenario = Scenario::from_params(&params).expect("参数应合法");
        let criteria = Criteria::from_params(&json!({}), &scenario);
        let one = plan(&scenario, &criteria);
        let two = plan(&scenario, &criteria);
        let a = serde_json::to_string(&one).unwrap();
        let b = serde_json::to_string(&two).unwrap();
        assert_eq!(a, b, "同一 seed 的完整推演必须逐字节一致");
    }

    #[test]
    fn plan_surfaces_unknown_priors_next_to_the_recommendation() {
        let params = json!({"tasks": [{"id": "only", "workK": 10}], "trials": 50});
        let scenario = Scenario::from_params(&params).expect("参数应合法");
        let criteria = Criteria::from_params(&json!({}), &scenario);
        let p = plan(&scenario, &criteria);
        assert!(p.rationale.recommended.is_some());
        assert!(
            !p.unknowns.is_empty(),
            "默认先验都没有实测样本，unknowns 不能为空"
        );
        assert!(
            p.unknowns.iter().any(|u| u.field == "failureRate"),
            "失败率先验没有样本时必须被点名"
        );
    }

    #[test]
    fn gate_contrast_exists_in_the_catalog() {
        let tactics = catalog();
        for topo in [Topology::Solo, Topology::Star] {
            let ungated = tactics
                .iter()
                .any(|t| t.topology == topo && t.gate.is_none());
            let gated = tactics.iter().any(|t| t.topology == topo && t.gate.is_some());
            assert!(
                ungated && gated,
                "拓扑 {} 必须有有/无闸门两条对照，否则无法裁决验证本身的价值",
                topo.as_key()
            );
        }
    }
}
