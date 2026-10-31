//! 确定性蒙特卡洛：把 [`Tactic`] 当成资源调度器，在给定 DAG 上跑 N 次。
//!
//! 三条物理规则（见 `model` 文档）在这里落地：
//!
//! - **并行只买时间**：并发位只影响调度，不影响单节点的失败概率；
//! - **协调要付钱**：星型/面板/蜂群各有一次性的协调 token 与整合时间，它们随
//!   规模增长，最终会吃掉并行带来的收益；
//! - **错误只在被检查时存在**：无闸门时失败节点有 `selfDetect` 的概率被自己发现，
//!   其余**静默传播**（试验失败）；有闸门时按 `detect` 转成可重试的显式失败。
//!
//! 随机数用仓库既有风格的手写 LCG（`games::poker` 同款），同一 seed 必然同一结果 ——
//! 这是「反事实回放」能作为证据的前提：不同战术之间共享同一串随机数，比较的才是
//! 战术本身，而不是抽样噪声。

use serde::Serialize;
use std::collections::BTreeMap;

use super::model::{Scenario, Tactic, Topology};

/// 蜂群每个节点的「分片丢上下文」概率。偏低是有意的：真实的蜂群事故也不是每片都丢，
/// 但一次丢片就足以让最终聚合结果是错的。
const SWARM_COVERAGE_MISS: f64 = 0.015;

/// 与 `games` 模块同款的手写 LCG：快、可复现、无外部依赖。
struct Lcg {
    state: u64,
}

impl Lcg {
    fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.state
    }

    /// [0, 1) 均匀分布（取高 53 位，避免低位相关性）
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// 失败原因。**「静默传播」与「显式失败」必须分开**：前者才是多智能体事故的真正形态
/// （结果看起来是成功的）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// 失败被发现且重试用尽
    NodeFailed { node: String },
    /// 无闸门战术里，缺陷没被任何人发现，直接进入交付物
    DefectPropagated { node: String },
    /// 有闸门但这次漏检了
    GateMissed { node: String },
    /// 蜂群分片边界丢了上下文
    CoverageGap { node: String },
    /// token 预算耗尽
    BudgetTokens,
    /// 超出时限
    DeadlineExceeded,
}

impl Failure {
    fn kind(&self) -> &'static str {
        match self {
            Failure::NodeFailed { .. } => "node-failed",
            Failure::DefectPropagated { .. } => "defect-propagated",
            Failure::GateMissed { .. } => "gate-missed",
            Failure::CoverageGap { .. } => "coverage-gap",
            Failure::BudgetTokens => "budget-tokens",
            Failure::DeadlineExceeded => "deadline",
        }
    }

    fn node(&self) -> Option<&str> {
        match self {
            Failure::NodeFailed { node }
            | Failure::DefectPropagated { node }
            | Failure::GateMissed { node }
            | Failure::CoverageGap { node } => Some(node),
            _ => None,
        }
    }
}

/// 一次试跑的结局。
#[derive(Debug, Clone)]
pub struct TrialOutcome {
    pub ok: bool,
    pub elapsed_ms: f64,
    pub tokens: f64,
    pub retries: u32,
    pub cause: Option<Failure>,
}

/// 一条战术在某个场景下的推演结论。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TacticResult {
    pub tactic: &'static str,
    pub topology: &'static str,
    pub width: usize,
    pub retries: u8,
    pub gated: bool,
    pub success_rate: f64,
    pub p50_ms: f64,
    pub p90_ms: f64,
    pub p50_tokens: f64,
    pub p90_tokens: f64,
    pub p90_cost_usd: f64,
    pub mean_retries: f64,
    /// 协调/整合开销占总 token 的比例
    pub coordination_share: f64,
    /// 失败原因分布（键见 [`Failure::kind`]）
    pub causes: BTreeMap<String, u64>,
    /// 最常出事的节点（最多 3 个）
    pub failure_nodes: Vec<(String, u64)>,
    /// 按期望耗时算出的关键路径
    pub critical_path: Vec<String>,
    /// 关键路径上耗时最长的节点
    pub bottleneck: String,
}

impl TacticResult {
    /// 期望失败概率（0..1）。kill 判据用它，而不是 `1 - success_rate` 的字面抄写，
    /// 方便以后换成置信上界而不改调用点。
    pub fn fail_probability(&self) -> f64 {
        (1.0 - self.success_rate).clamp(0.0, 1.0)
    }
}

struct Ctx<'a> {
    scenario: &'a Scenario,
    tactic: &'a Tactic,
    order: Vec<usize>,
    deps: Vec<Vec<usize>>,
    total_work_k: f64,
    throughput: f64,
    price: f64,
    retry_backoff: f64,
}

impl Ctx<'_> {
    fn node_base_ms(&self, work_k: f64) -> f64 {
        work_k / self.throughput * 60_000.0
    }

    fn gate_factor(&self) -> f64 {
        self.tactic
            .gate
            .map(|g| g.latency_factor)
            .unwrap_or(1.0)
    }

    fn gate_token_factor(&self) -> f64 {
        1.0 + self.tactic.gate.map(|g| g.token_overhead).unwrap_or(0.0)
    }

    /// 面板对同一节点跑 `width` 个专家 —— 每个专家都要读一遍上下文。
    fn duplication(&self) -> f64 {
        match self.tactic.topology {
            Topology::Panel => self.tactic.width as f64,
            _ => 1.0,
        }
    }

    fn handoff_ms(&self) -> f64 {
        match self.tactic.topology {
            Topology::Solo => 0.0,
            Topology::Star => 4_000.0,
            Topology::Panel => 2_000.0,
            Topology::Swarm => 1_500.0,
            Topology::Relay => 6_000.0,
        }
    }

    /// 一次性协调开销（相对总工作量的 token 比例）
    fn coordination_share(&self) -> f64 {
        match self.tactic.topology {
            Topology::Solo => 0.0,
            // 主智能体先规划、最后整合
            Topology::Star => 0.05 + 0.04,
            // 面板的额外开销已按节点计入 duplication，这里只剩合成胶水
            Topology::Panel => 0.02,
            Topology::Swarm => 0.12,
            Topology::Relay => 0.03,
        }
    }

    fn integration_ms(&self) -> f64 {
        let total_ms = self.node_base_ms(self.total_work_k);
        match self.tactic.topology {
            Topology::Solo => 0.0,
            Topology::Star => total_ms * 0.04,
            Topology::Panel => total_ms * 0.02,
            Topology::Swarm => total_ms * 0.06,
            Topology::Relay => 0.0,
        }
    }

    /// 单节点单次失败的等效概率。面板用「独立专家 + 相关性」的闭式解，
    /// 蜂群叠加分片丢上下文的概率，接力按专精折扣，星型按协调噪声折损。
    fn effective_failure(&self, p: f64) -> f64 {
        let t = self.tactic;
        match t.topology {
            Topology::Solo => p,
            Topology::Star => p * (1.0 + 0.10 * (t.width.saturating_sub(1) as f64) / t.width as f64),
            Topology::Panel => {
                // 相关性 0.35：三个专家不会真的完全独立，否则 p³ 会把成功率吹到 100%
                let corr = 0.35;
                let joint = p * (corr + (1.0 - corr) * p.powi(t.width.saturating_sub(1) as i32));
                // 选择器自身会引入新错
                (joint + 0.02).clamp(0.0, 1.0)
            }
            // 分片丢上下文的概率单独抽样（见 [`run_trial`]），这里只算协调噪声
            Topology::Swarm => {
                p * (1.0 + 0.25 * (t.width.saturating_sub(1) as f64) / t.width as f64)
            }
            Topology::Relay => p * 0.7,
        }
    }

    /// 该节点要占几个并发位（面板占满整组专家）。
    fn slots(&self) -> usize {
        match self.tactic.topology {
            Topology::Panel => self.tactic.width,
            _ => 1,
        }
    }
}

/// 对一条战术跑完全部试跑。
pub fn simulate(scenario: &Scenario, tactic: &Tactic) -> TacticResult {
    let ctx = Ctx {
        scenario,
        tactic,
        order: scenario.topo_order().unwrap_or_else(|_| (0..scenario.tasks.len()).collect()),
        deps: scenario.dep_indices(),
        total_work_k: scenario.total_work_k(),
        throughput: scenario.model.throughput_k_per_min.value.max(f64::MIN_POSITIVE),
        price: scenario.model.price_per_mtok_usd.value.max(0.0),
        retry_backoff: scenario.model.retry_backoff.value.max(1.0),
    };

    let mut trials = Vec::with_capacity(scenario.trials);
    for i in 0..scenario.trials {
        let mut rng = Lcg::new(
            scenario
                .seed
                .wrapping_add((i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)),
        );
        trials.push(run_trial(&ctx, &mut rng));
    }

    aggregate(&ctx, &trials)
}

fn run_trial(ctx: &Ctx, rng: &mut Lcg) -> TrialOutcome {
    let s = ctx.scenario;
    let t = ctx.tactic;
    let p = s.model.failure_rate.value.clamp(0.0, 1.0);
    let p_eff = ctx.effective_failure(p);
    let self_detect = s.model.self_detect.value.clamp(0.0, 1.0);
    let n = s.tasks.len();

    let mut free = vec![0.0_f64; t.width.max(1)];
    let mut finish = vec![0.0_f64; n];
    let mut tokens = 0.0_f64;
    let mut retries_used = 0u32;
    let mut cause: Option<Failure> = None;

    for &idx in &ctx.order {
        let node = &s.tasks[idx];
        let mut earliest = 0.0_f64;
        for &d in &ctx.deps[idx] {
            earliest = earliest.max(finish[d]);
        }
        let slots = ctx.slots().min(free.len()).max(1);
        // 面板占满整组专家，其余拓扑只占**一个空闲位**——占位必须在调度里记住，
        // 否则并行拓扑会全部挤到 0 号位上串行执行（时间维度直接失真）。
        let (start, used): (f64, Vec<usize>) = if slots >= free.len() {
            (
                earliest.max(free.iter().copied().fold(f64::MIN, f64::max)),
                (0..free.len()).collect(),
            )
        } else {
            let mut best = 0usize;
            for (i, f) in free.iter().enumerate() {
                if *f < free[best] {
                    best = i;
                }
            }
            (earliest.max(free[best]), vec![best])
        };

        let base_ms = ctx.node_base_ms(node.work_k);
        let attempt_tokens = node.work_k * 1000.0 * ctx.duplication() * ctx.gate_token_factor();
        let mut service = 0.0_f64;
        let mut ok_node = true;
        let mut attempt = 0u32;

        loop {
            attempt += 1;
            service += base_ms * ctx.gate_factor() * ctx.retry_backoff.powi(attempt as i32 - 1);
            tokens += attempt_tokens;

            // 蜂群特有：分片边界把上下文切丢了一截。它与「执行者做错」不是同一类失败，
            // 所以单独抽样、单独归因 —— 否则调参时会把分片问题误判成模型不可靠。
            let coverage_missed =
                t.topology == Topology::Swarm && rng.unit() < SWARM_COVERAGE_MISS;
            let failed = coverage_missed || rng.unit() < p_eff;
            if !failed {
                // 闸门误报：健康产物被打回，付一次半价返工
                if let Some(g) = t.gate {
                    if rng.unit() < g.false_positive {
                        tokens += attempt_tokens * 0.5;
                        service += base_ms * 0.5;
                    }
                }
                break;
            }
            let caught = match t.gate {
                Some(g) => rng.unit() < g.detect,
                None => rng.unit() < self_detect,
            };
            if !caught {
                ok_node = false;
                let node = node.id.clone();
                cause = Some(if coverage_missed {
                    Failure::CoverageGap { node }
                } else if t.gate.is_some() {
                    Failure::GateMissed { node }
                } else {
                    Failure::DefectPropagated { node }
                });
                break;
            }
            if attempt > u32::from(t.retries) {
                ok_node = false;
                cause = Some(Failure::NodeFailed {
                    node: node.id.clone(),
                });
                break;
            }
            retries_used += 1;
        }

        let end = start + service + ctx.handoff_ms();
        for slot in used {
            free[slot] = end;
        }
        finish[idx] = end;

        if s.budget_tokens.is_some_and(|b| tokens > b) {
            cause = Some(Failure::BudgetTokens);
            break;
        }
        if !ok_node {
            break;
        }
    }

    let coordination = ctx.total_work_k * 1000.0 * ctx.coordination_share();
    tokens += coordination;

    let mut elapsed_ms = finish.iter().copied().fold(0.0, f64::max);
    if cause.is_none() {
        elapsed_ms += ctx.integration_ms();
    }
    if cause.is_none() && s.deadline_ms.is_some_and(|d| elapsed_ms > d) {
        cause = Some(Failure::DeadlineExceeded);
    }

    TrialOutcome {
        ok: cause.is_none(),
        elapsed_ms,
        tokens,
        retries: retries_used,
        cause,
    }
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn aggregate(ctx: &Ctx, trials: &[TrialOutcome]) -> TacticResult {
    let n = trials.len().max(1) as f64;
    let ok = trials.iter().filter(|t| t.ok).count() as f64;

    let mut ms: Vec<f64> = trials.iter().map(|t| t.elapsed_ms).collect();
    let mut toks: Vec<f64> = trials.iter().map(|t| t.tokens).collect();
    ms.sort_by(f64::total_cmp);
    toks.sort_by(f64::total_cmp);

    let mut causes: BTreeMap<String, u64> = BTreeMap::new();
    let mut node_hits: BTreeMap<String, u64> = BTreeMap::new();
    let mut retries = 0.0_f64;
    for t in trials {
        if let Some(c) = &t.cause {
            *causes.entry(c.kind().to_string()).or_insert(0) += 1;
            if let Some(node) = c.node() {
                *node_hits.entry(node.to_string()).or_insert(0) += 1;
            }
        }
        retries += f64::from(t.retries);
    }

    let mut failure_nodes: Vec<(String, u64)> = node_hits.into_iter().collect();
    failure_nodes.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    failure_nodes.truncate(3);

    let p90_tokens = quantile(&toks, 0.90);
    let coordination_tokens = ctx.total_work_k * 1000.0 * ctx.coordination_share();
    let mean_tokens = trials.iter().map(|t| t.tokens).sum::<f64>() / n;
    let coordination_share = if mean_tokens > 0.0 {
        (coordination_tokens / mean_tokens).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let (critical_path, bottleneck) = critical_path(ctx);

    TacticResult {
        tactic: ctx.tactic.id,
        topology: ctx.tactic.topology.as_key(),
        width: ctx.tactic.width,
        retries: ctx.tactic.retries,
        gated: ctx.tactic.gate.is_some(),
        success_rate: round3(ok / n),
        p50_ms: quantile(&ms, 0.50).round(),
        p90_ms: quantile(&ms, 0.90).round(),
        p50_tokens: quantile(&toks, 0.50).round(),
        p90_tokens: p90_tokens.round(),
        p90_cost_usd: round4(p90_tokens / 1_000_000.0 * ctx.price),
        mean_retries: round2(retries / n),
        coordination_share: round3(coordination_share),
        causes,
        failure_nodes,
        critical_path,
        bottleneck,
    }
}

/// 按**期望**耗时（不含重试/闸门）回溯关键路径 —— 回答「这条链决定了最短完工时间」。
fn critical_path(ctx: &Ctx) -> (Vec<String>, String) {
    let n = ctx.scenario.tasks.len();
    let mut ef = vec![0.0_f64; n];
    let mut prev = vec![None; n];
    for &idx in &ctx.order {
        let mut best = 0.0_f64;
        let mut from = None;
        for &d in &ctx.deps[idx] {
            if ef[d] >= best {
                best = ef[d];
                from = Some(d);
            }
        }
        prev[idx] = from;
        ef[idx] = best + ctx.node_base_ms(ctx.scenario.tasks[idx].work_k) + ctx.handoff_ms();
    }
    let Some(end) = (0..n).max_by(|a, b| ef[*a].total_cmp(&ef[*b])) else {
        return (Vec::new(), String::new());
    };
    let mut path = Vec::new();
    let mut cur = Some(end);
    while let Some(i) = cur {
        path.push(ctx.scenario.tasks[i].id.clone());
        cur = prev[i];
    }
    path.reverse();
    let bottleneck = path
        .iter()
        .max_by(|a, b| {
            let wa = ctx
                .scenario
                .tasks
                .iter()
                .find(|t| &t.id == *a)
                .map_or(0.0, |t| t.work_k);
            let wb = ctx
                .scenario
                .tasks
                .iter()
                .find(|t| &t.id == *b)
                .map_or(0.0, |t| t.work_k);
            wa.total_cmp(&wb)
        })
        .cloned()
        .unwrap_or_default();
    (path, bottleneck)
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::super::model::{Model, TacticalTask};
    use super::*;

    fn scenario(tasks: Vec<(&str, f64, Vec<&str>)>) -> Scenario {
        Scenario {
            tasks: tasks
                .into_iter()
                .map(|(id, work_k, deps)| TacticalTask {
                    id: id.to_string(),
                    work_k,
                    depends_on: deps.into_iter().map(str::to_string).collect(),
                })
                .collect(),
            trials: 600,
            seed: 7,
            budget_tokens: None,
            deadline_ms: None,
            model: Model::default(),
        }
    }

    fn chain(n: usize) -> Scenario {
        let mut v: Vec<(String, f64, Vec<String>)> = Vec::new();
        for i in 0..n {
            let deps = if i == 0 {
                vec![]
            } else {
                vec![format!("t{}", i - 1)]
            };
            v.push((format!("t{i}"), 20.0, deps));
        }
        let tasks = v
            .into_iter()
            .map(|(id, work_k, depends_on)| TacticalTask {
                id,
                work_k,
                depends_on,
            })
            .collect();
        Scenario {
            tasks,
            trials: 600,
            seed: 7,
            budget_tokens: None,
            deadline_ms: None,
            model: Model::default(),
        }
    }

    #[test]
    fn same_seed_same_result() {
        let s = chain(5);
        let t = super::super::model::catalog()
            .into_iter()
            .find(|t| t.id == "star")
            .unwrap();
        let a = simulate(&s, &t);
        let b = simulate(&s, &t);
        assert_eq!(a.success_rate, b.success_rate);
        assert_eq!(a.p90_ms, b.p90_ms);
        assert_eq!(a.p90_tokens, b.p90_tokens);
    }

    #[test]
    fn different_seed_moves_the_distribution_a_little() {
        let mut s = chain(5);
        let t = super::super::model::catalog()
            .into_iter()
            .find(|t| t.id == "star")
            .unwrap();
        let a = simulate(&s, &t);
        s.seed = 8;
        let b = simulate(&s, &t);
        assert!((a.success_rate - b.success_rate).abs() < 0.15);
    }

    #[test]
    fn gate_never_lowers_success() {
        let s = chain(6);
        let tactics = super::super::model::catalog();
        let plain = tactics.iter().find(|t| t.id == "star").unwrap();
        let gated = tactics.iter().find(|t| t.id == "star-gated").unwrap();
        assert!(
            simulate(&s, gated).success_rate >= simulate(&s, plain).success_rate,
            "闸门把静默缺陷变成可重试失败，成功率不应下降"
        );
    }

    #[test]
    fn parallelism_buys_time_on_a_wide_dag() {
        let s = scenario(vec![
            ("a", 20.0, vec![]),
            ("b", 20.0, vec![]),
            ("c", 20.0, vec![]),
            ("d", 20.0, vec![]),
            ("e", 20.0, vec![]),
            ("f", 20.0, vec![]),
        ]);
        let tactics = super::super::model::catalog();
        let solo = tactics.iter().find(|t| t.id == "solo").unwrap();
        let star = tactics.iter().find(|t| t.id == "star").unwrap();
        assert!(simulate(&s, solo).p50_ms > simulate(&s, star).p50_ms);
    }

    #[test]
    fn panel_pays_tokens_for_reliability() {
        let s = chain(5);
        let tactics = super::super::model::catalog();
        let solo = simulate(&s, tactics.iter().find(|t| t.id == "solo-gated").unwrap());
        let panel = simulate(&s, tactics.iter().find(|t| t.id == "panel").unwrap());
        assert!(panel.p50_tokens > solo.p50_tokens);
        assert!(panel.success_rate >= solo.success_rate);
    }

    #[test]
    fn budget_exhaustion_is_explicit() {
        let mut s = chain(6);
        s.budget_tokens = Some(1.0);
        let t = super::super::model::catalog()
            .into_iter()
            .find(|t| t.id == "solo")
            .unwrap();
        let r = simulate(&s, &t);
        assert_eq!(r.success_rate, 0.0);
        assert_eq!(r.causes.get("budget-tokens").copied(), Some(600));
    }

    #[test]
    fn deadline_is_explicit() {
        let mut s = chain(3);
        s.deadline_ms = Some(1.0);
        let t = super::super::model::catalog()
            .into_iter()
            .find(|t| t.id == "solo")
            .unwrap();
        let r = simulate(&s, &t);
        assert_eq!(r.success_rate, 0.0);
        assert!(r.causes.contains_key("deadline"));
    }

    #[test]
    fn critical_path_follows_dependencies() {
        let s = scenario(vec![
            ("a", 10.0, vec![]),
            ("b", 90.0, vec!["a"]),
            ("c", 10.0, vec![]),
        ]);
        let t = super::super::model::catalog()
            .into_iter()
            .find(|t| t.id == "solo")
            .unwrap();
        let r = simulate(&s, &t);
        assert_eq!(r.critical_path, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(r.bottleneck, "b");
    }

    #[test]
    fn ungated_tactics_can_propagate_defects() {
        let mut s = chain(8);
        s.model.failure_rate = super::super::model::Prior::declared(0.3);
        s.model.self_detect = super::super::model::Prior::declared(0.0);
        let t = super::super::model::catalog()
            .into_iter()
            .find(|t| t.id == "star")
            .unwrap();
        let r = simulate(&s, &t);
        assert!(
            r.causes.contains_key("defect-propagated"),
            "无闸门 + 不自检 ⇒ 缺陷必须静默传播：{:?}",
            r.causes
        );
    }
}
