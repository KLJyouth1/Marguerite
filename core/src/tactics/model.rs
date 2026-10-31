//! 推演模型：把「协作战术」表达成**可计算的数据**，而不是散文式的偏好。
//!
//! 第一性原理只有三条，其余全部由它们推出来：
//!
//! 1. **任务是一个有依赖的活**（DAG）：节点没完成，后继不能开始。
//! 2. **执行者是有限资源**（吞吐 × 并发位）：并行只买时间，不买正确性。
//! 3. **错误只有在被检查时才存在**：没有验证闸门的战术里，失败节点的缺陷会
//!    静默传播；有闸门时，缺陷转成一次可重试的显式失败。
//!
//! 由此得到的结论不是抄来的分类法，而是可被推翻的：每一项先验都带 `n`（样本数）
//! 与 `source`；`n = 0` 的一律进 [`Model::unknowns`]，**绝不把「不知道」渲染成 0**
//! （与 `context::compact_state` 同一条设计红线）。

use serde::Serialize;
use serde_json::Value;

/// 协作拓扑。
///
/// 与 `domain::AgentMode` / `gateway::orchestrate::OrchestrateForm` **不是**同一层：
/// 那两个是「产品里可选的形态」，这里描述的是**资源调度方式**——同一形态在推演里
/// 可能对应不同拓扑，反之亦然。两套词表各自服务自己的消费者，不做强行合并。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Topology {
    /// 单执行者串行跑完整条 DAG
    Solo,
    /// 主智能体规划 + 子智能体并行执行 + 主智能体整合
    Star,
    /// 专家团：多个专家对同一节点独立作答，再经选择器合成
    Panel,
    /// 蜂群：大量分片并行，靠聚合器收口；分片边界会丢上下文
    Swarm,
    /// 流水线接力：每个节点交给专职执行者，串行但有专精增益
    Relay,
}

impl Topology {
    pub const ALL: &'static [Topology] = &[
        Topology::Solo,
        Topology::Star,
        Topology::Panel,
        Topology::Swarm,
        Topology::Relay,
    ];

    pub fn as_key(&self) -> &'static str {
        match self {
            Topology::Solo => "solo",
            Topology::Star => "star",
            Topology::Panel => "panel",
            Topology::Swarm => "swarm",
            Topology::Relay => "relay",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "solo" => Some(Topology::Solo),
            "star" => Some(Topology::Star),
            "panel" | "expert-team" | "expert" => Some(Topology::Panel),
            "swarm" => Some(Topology::Swarm),
            "relay" | "pipeline" => Some(Topology::Relay),
            _ => None,
        }
    }
}

/// 验证闸门：把「未被发现的缺陷」变成「可重试的显式失败」要付的代价。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gate {
    /// 缺陷检出率（0..1）：漏检的部分会继续传播
    pub detect: f64,
    /// 误报率：健康产物被判为不合格，触发一次返工
    pub false_positive: f64,
    /// 闸门自身的 token 开销（相对节点工作量的比例）
    pub token_overhead: f64,
    /// 闸门自身的耗时系数（1.15 = 多花 15%）
    pub latency_factor: f64,
}

/// 一条战术：拓扑 + 并行度 + 重试预算 + 闸门。
///
/// 战术是**声明**而不是代码路径：同一份引擎按数据解释它，于是「换战术」不需要
/// 再写一遍调度逻辑，也不会出现「文档里的战术」和「跑起来的战术」两套。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Tactic {
    pub id: &'static str,
    pub topology: Topology,
    /// 并行位（星型 = 子智能体数；面板 = 专家数；蜂群 = 分片数；其余 = 1）
    pub width: usize,
    /// 失败后可重试的次数（0 = 失败即终止）
    pub retries: u8,
    /// 无闸门 = 缺陷静默传播（这是大多数「多智能体事故」的物理来源）
    pub gate: Option<Gate>,
    /// 一句话说明这条战术在赌什么
    pub bet: &'static str,
}

const STRONG_GATE: Gate = Gate {
    detect: 0.85,
    false_positive: 0.05,
    token_overhead: 0.10,
    latency_factor: 1.15,
};

/// 内置战术目录。
///
/// 刻意把「同样的拓扑有/无闸门」各留一条：对比项不是装饰，而是这个系统唯一的
/// 卖点——**让「加不加验证」成为可被数据裁决的选择，而不是信仰**。
pub fn catalog() -> Vec<Tactic> {
    vec![
        Tactic {
            id: "solo",
            topology: Topology::Solo,
            width: 1,
            retries: 0,
            gate: None,
            bet: "任务简单到一次做对，省掉全部协调开销",
        },
        Tactic {
            id: "solo-gated",
            topology: Topology::Solo,
            width: 1,
            retries: 1,
            gate: Some(STRONG_GATE),
            bet: "多花一次检查，换「错误不会悄悄交出去」",
        },
        Tactic {
            id: "star",
            topology: Topology::Star,
            width: 3,
            retries: 1,
            gate: None,
            bet: "并行买时间；风险是没人检查整合结果",
        },
        Tactic {
            id: "star-gated",
            topology: Topology::Star,
            width: 3,
            retries: 1,
            gate: Some(STRONG_GATE),
            bet: "并行 + 整合后验收：速度与正确性都要",
        },
        Tactic {
            id: "panel",
            topology: Topology::Panel,
            width: 3,
            retries: 1,
            gate: Some(Gate {
                detect: 0.92,
                false_positive: 0.06,
                token_overhead: 0.12,
                latency_factor: 1.20,
            }),
            bet: "专家独立作答降低漏判；代价是同一个节点付三份钱",
        },
        Tactic {
            id: "swarm",
            topology: Topology::Swarm,
            width: 6,
            retries: 1,
            gate: Some(Gate {
                detect: 0.80,
                false_positive: 0.08,
                token_overhead: 0.08,
                latency_factor: 1.10,
            }),
            bet: "宽 DAG 靠分片压平时间；分片边界会丢上下文",
        },
        Tactic {
            id: "relay",
            topology: Topology::Relay,
            width: 1,
            retries: 1,
            gate: Some(Gate {
                detect: 0.88,
                false_positive: 0.04,
                token_overhead: 0.09,
                latency_factor: 1.12,
            }),
            bet: "专职接力降低单点失误；串行意味着时间换正确率",
        },
    ]
}

/// 先验的来源——决定这个数字能不能被当成事实使用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PriorSource {
    /// 来自真实台账（harness ledger）的实测
    Measured,
    /// 由调用方声明的经验值，未经本机实测
    Declared,
    /// 没有数据，也没有声明
    Unknown,
}

/// 一个带样本数的先验。`n = 0` 时 [`PriorSource`] 必为 `Declared` / `Unknown`。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Prior {
    pub value: f64,
    pub n: u64,
    pub source: PriorSource,
}

impl Prior {
    pub fn declared(value: f64) -> Self {
        Self {
            value,
            n: 0,
            source: PriorSource::Declared,
        }
    }

    pub fn measured(value: f64, n: u64) -> Self {
        Self {
            value,
            n,
            source: PriorSource::Measured,
        }
    }

    pub fn unknown() -> Self {
        Self {
            value: 0.0,
            n: 0,
            source: PriorSource::Unknown,
        }
    }

    /// 没有样本 = 这个数字**不构成证据**。
    pub fn is_evidence(&self) -> bool {
        self.n > 0
    }
}

/// 推演用到的全部先验。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    /// 单节点单次失败概率
    pub failure_rate: Prior,
    /// 单个执行者每分钟能处理多少千 token
    pub throughput_k_per_min: Prior,
    /// 每百万 token 的价格（美元）
    pub price_per_mtok_usd: Prior,
    /// 无闸门时执行者「自己发现出错」的概率
    pub self_detect: Prior,
    /// 重试的耗时放大（1.15 = 第二次多花 15%）
    pub retry_backoff: Prior,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            failure_rate: Prior::declared(0.08),
            throughput_k_per_min: Prior::declared(6.0),
            price_per_mtok_usd: Prior::declared(2.0),
            self_detect: Prior::declared(0.5),
            retry_backoff: Prior::declared(1.15),
        }
    }
}

/// 一条「未知」：哪个字段、为什么不知道、影响什么判断。
#[derive(Debug, Clone, Serialize)]
pub struct Unknown {
    pub field: &'static str,
    pub reason: String,
}

impl Model {
    /// 所有不构成证据的先验。**空数组才代表「全部有实测支撑」**。
    pub fn unknowns(&self) -> Vec<Unknown> {
        let mut out = Vec::new();
        let mut push = |field: &'static str, p: &Prior, impact: &str| {
            if !p.is_evidence() {
                out.push(Unknown {
                    field,
                    reason: format!(
                        "n=0（{}），本机没有实测样本，用的是声明值；{impact}",
                        match p.source {
                            PriorSource::Measured => "标记为实测却无样本",
                            PriorSource::Declared => "声明先验",
                            PriorSource::Unknown => "无数据",
                        }
                    ),
                });
            }
        };
        push(
            "failureRate",
            &self.failure_rate,
            "成功率因此只是先验条件下的估计，不是事实",
        );
        push(
            "throughputKPerMin",
            &self.throughput_k_per_min,
            "耗时排序可能失真，但战术之间的相对次序通常仍成立",
        );
        push("pricePerMTokUsd", &self.price_per_mtok_usd, "成本绝对值不可引用");
        push("selfDetect", &self.self_detect, "无闸门战术的失败传播率不确定");
        push("retryBackoff", &self.retry_backoff, "重试成本的量级不确定");
        out
    }

    /// 从 `priors` 参数覆盖默认值。未知字段静默保持默认（默认本身就是声明先验）。
    fn apply_overrides(&mut self, v: &Value) {
        if let Some(o) = v.as_object() {
            if let Some(x) = o.get("failureRate").and_then(Value::as_f64) {
                self.failure_rate = Prior::declared(x.clamp(0.0, 1.0));
            }
            if let Some(x) = o.get("throughputKPerMin").and_then(Value::as_f64) {
                if x > 0.0 {
                    self.throughput_k_per_min = Prior::declared(x);
                }
            }
            if let Some(x) = o.get("pricePerMTokUsd").and_then(Value::as_f64) {
                if x >= 0.0 {
                    self.price_per_mtok_usd = Prior::declared(x);
                }
            }
            if let Some(x) = o.get("selfDetect").and_then(Value::as_f64) {
                self.self_detect = Prior::declared(x.clamp(0.0, 1.0));
            }
            if let Some(x) = o.get("retryBackoff").and_then(Value::as_f64) {
                if x >= 1.0 {
                    self.retry_backoff = Prior::declared(x);
                }
            }
        }
    }
}

/// DAG 里的一个节点。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TacticalTask {
    pub id: String,
    /// 工作量（千 token）。这是**做这件事本身**要处理的信息量，不含协调开销。
    pub work_k: f64,
    pub depends_on: Vec<String>,
}

/// 一次推演的输入：任务 DAG + 预算 + 试跑次数 + 先验。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scenario {
    pub tasks: Vec<TacticalTask>,
    pub trials: usize,
    pub seed: u64,
    pub budget_tokens: Option<f64>,
    pub deadline_ms: Option<f64>,
    pub model: Model,
}

/// 单次试跑的默认次数：2000 次在毫秒级完成，且 p90 的抽样波动 < 1%。
pub const DEFAULT_TRIALS: usize = 2000;
/// 试跑次数上限：防止把 RPC 当算力矿机用。
pub const MAX_TRIALS: usize = 20_000;

impl Scenario {
    /// 从 RPC 参数解析。缺省值全部显式写在这里，前端不重复实现一遍默认值。
    pub fn from_params(v: &Value) -> Result<Self, String> {
        let tasks_v = v
            .get("tasks")
            .and_then(Value::as_array)
            .ok_or_else(|| "tasks 必须是数组".to_string())?;
        if tasks_v.is_empty() {
            return Err("tasks 不能为空".into());
        }
        if tasks_v.len() > 64 {
            return Err("tasks 最多 64 个（超过这个规模的推演没有决策价值）".into());
        }
        let mut tasks = Vec::with_capacity(tasks_v.len());
        for t in tasks_v {
            let id = t
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "每个 task 都要有非空 id".to_string())?;
            let work_k = t.get("workK").and_then(Value::as_f64).unwrap_or(40.0);
            if !(work_k.is_finite() && work_k > 0.0) {
                return Err(format!("task `{id}` 的 workK 必须是正数"));
            }
            let depends_on = t
                .get("dependsOn")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            tasks.push(TacticalTask {
                id: id.to_string(),
                work_k,
                depends_on,
            });
        }
        let trials = v
            .get("trials")
            .and_then(Value::as_u64)
            .map(|n| (n as usize).clamp(1, MAX_TRIALS))
            .unwrap_or(DEFAULT_TRIALS);
        let seed = v.get("seed").and_then(Value::as_u64).unwrap_or(42);
        let budget_tokens = v
            .get("budgetTokens")
            .and_then(Value::as_f64)
            .filter(|x| x.is_finite() && *x > 0.0);
        let deadline_ms = v
            .get("deadlineMs")
            .and_then(Value::as_f64)
            .filter(|x| x.is_finite() && *x > 0.0);
        let mut model = Model::default();
        if let Some(p) = v.get("priors") {
            model.apply_overrides(p);
        }
        let s = Self {
            tasks,
            trials,
            seed,
            budget_tokens,
            deadline_ms,
            model,
        };
        s.validate()?;
        Ok(s)
    }

    /// 结构校验：id 唯一、依赖存在、无环。
    pub fn validate(&self) -> Result<(), String> {
        let mut index = std::collections::BTreeMap::new();
        for (i, t) in self.tasks.iter().enumerate() {
            if index.insert(t.id.as_str(), i).is_some() {
                return Err(format!("task id 重复：`{}`", t.id));
            }
        }
        for t in &self.tasks {
            for d in &t.depends_on {
                if !index.contains_key(d.as_str()) {
                    return Err(format!("task `{}` 依赖了不存在的 `{d}`", t.id));
                }
                if d == &t.id {
                    return Err(format!("task `{}` 依赖了自己", t.id));
                }
            }
        }
        self.topo_order()?;
        Ok(())
    }

    /// Kahn 拓扑序；有环即报出环上的节点（而不是一句「有环」）。
    pub fn topo_order(&self) -> Result<Vec<usize>, String> {
        let n = self.tasks.len();
        let index: std::collections::BTreeMap<&str, usize> = self
            .tasks
            .iter()
            .enumerate()
            .map(|(i, t)| (t.id.as_str(), i))
            .collect();
        let mut indeg = vec![0usize; n];
        let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, t) in self.tasks.iter().enumerate() {
            for d in &t.depends_on {
                let Some(&j) = index.get(d.as_str()) else {
                    return Err(format!("task `{}` 依赖了不存在的 `{d}`", t.id));
                };
                indeg[i] += 1;
                out[j].push(i);
            }
        }
        let mut q: std::collections::VecDeque<usize> = (0..n).filter(|i| indeg[*i] == 0).collect();
        let mut order = Vec::with_capacity(n);
        while let Some(i) = q.pop_front() {
            order.push(i);
            for &k in &out[i] {
                indeg[k] -= 1;
                if indeg[k] == 0 {
                    q.push_back(k);
                }
            }
        }
        if order.len() != n {
            let stuck: Vec<&str> = (0..n)
                .filter(|i| indeg[*i] > 0)
                .map(|i| self.tasks[i].id.as_str())
                .collect();
            return Err(format!("任务依赖成环：{}", stuck.join(", ")));
        }
        Ok(order)
    }

    pub fn total_work_k(&self) -> f64 {
        self.tasks.iter().map(|t| t.work_k).sum()
    }

    /// 按拓扑序解析成下标依赖（仿真内部使用）。
    pub fn dep_indices(&self) -> Vec<Vec<usize>> {
        let index: std::collections::BTreeMap<&str, usize> = self
            .tasks
            .iter()
            .enumerate()
            .map(|(i, t)| (t.id.as_str(), i))
            .collect();
        self.tasks
            .iter()
            .map(|t| {
                t.depends_on
                    .iter()
                    .filter_map(|d| index.get(d.as_str()).copied())
                    .collect()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scenario(tasks: Vec<(&str, f64, Vec<&str>)>) -> Scenario {
        let tasks = tasks
            .into_iter()
            .map(|(id, work_k, deps)| TacticalTask {
                id: id.to_string(),
                work_k,
                depends_on: deps.into_iter().map(str::to_string).collect(),
            })
            .collect();
        Scenario {
            tasks,
            trials: 100,
            seed: 1,
            budget_tokens: None,
            deadline_ms: None,
            model: Model::default(),
        }
    }

    #[test]
    fn cycle_reports_the_nodes_on_it() {
        let s = scenario(vec![
            ("a", 1.0, vec!["c"]),
            ("b", 1.0, vec!["a"]),
            ("c", 1.0, vec!["b"]),
        ]);
        let err = s.validate().unwrap_err();
        assert!(err.contains("成环"), "{err}");
        assert!(err.contains('a') && err.contains('b') && err.contains('c'), "{err}");
    }

    #[test]
    fn missing_dependency_is_named() {
        let s = scenario(vec![("a", 1.0, vec!["ghost"])]);
        assert!(s.validate().unwrap_err().contains("ghost"));
    }

    #[test]
    fn topo_order_respects_dependencies() {
        let s = scenario(vec![
            ("c", 1.0, vec!["a", "b"]),
            ("a", 1.0, vec![]),
            ("b", 1.0, vec!["a"]),
        ]);
        let order = s.topo_order().unwrap();
        let pos = |id: &str| order.iter().position(|i| s.tasks[*i].id == id).unwrap();
        assert!(pos("a") < pos("b"));
        assert!(pos("b") < pos("c"));
    }

    #[test]
    fn zero_sample_priors_are_unknown_not_zero() {
        let model = Model::default();
        let unknowns = model.unknowns();
        assert_eq!(unknowns.len(), 5, "默认模型全部无实测样本");
        assert!(unknowns.iter().any(|u| u.field == "failureRate"));
    }

    #[test]
    fn measured_priors_leave_the_unknown_list() {
        let model = Model {
            failure_rate: Prior::measured(0.12, 30),
            ..Default::default()
        };
        let unknowns = model.unknowns();
        assert!(!unknowns.iter().any(|u| u.field == "failureRate"));
    }

    #[test]
    fn defaults_are_documented_once() {
        let v = serde_json::json!({ "tasks": [{ "id": "a" }] });
        let s = Scenario::from_params(&v).unwrap();
        assert_eq!(s.trials, DEFAULT_TRIALS);
        assert_eq!(s.seed, 42);
        assert_eq!(s.tasks[0].work_k, 40.0);
        assert!(s.budget_tokens.is_none());
    }

    #[test]
    fn trials_are_clamped_not_rejected() {
        let v = serde_json::json!({ "tasks": [{ "id": "a" }], "trials": 10_000_000u64 });
        assert_eq!(Scenario::from_params(&v).unwrap().trials, MAX_TRIALS);
    }
}
