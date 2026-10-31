//! 裁决：推荐哪条战术、为什么**不**推荐其余每条、以及先做一次「事前验尸」。
//!
//! 守正创新的两个硬约束：
//!
//! 1. **推荐必须是可被推翻的**。每条战术先过一组显式判据（失败率上限、token 预算、
//!    时限），判据不过就直接出局——不是算一个加权总分再取最大，那种做法永远无法
//!    回答「它凭什么赢」。
//! 2. **不推荐必须给理由**。每一条落选战术都要说明它是被哪条判据否掉，还是在可行
//!    集合里输给了谁、输多少。只说「综合评分较低」等于没说。
//!
//! 事前验尸（pre-mortem）不从模型自由发挥：失败模式全部来自推演自己的失败分类与
//! 参数余量，出现频率就是模拟里的真实计数。

use serde::Serialize;
use serde_json::Value;

use super::model::{Scenario, Topology};
use super::sim::TacticResult;

/// 判据：任何一条不过，战术直接出局。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Criteria {
    /// 可接受的失败概率上限
    pub max_fail_probability: f64,
    /// token 预算（默认取场景里的 `budgetTokens`）
    pub budget_tokens: Option<f64>,
    /// 时限（默认取场景里的 `deadlineMs`）
    pub deadline_ms: Option<f64>,
}

impl Default for Criteria {
    fn default() -> Self {
        Self {
            max_fail_probability: 0.15,
            budget_tokens: None,
            deadline_ms: None,
        }
    }
}

impl Criteria {
    pub fn from_params(v: &Value, scenario: &Scenario) -> Self {
        let mut c = Self::from_params_standalone(v);
        c.budget_tokens = scenario.budget_tokens;
        c.deadline_ms = scenario.deadline_ms;
        c
    }

    /// 没有场景可依时的解析（台账回放：run 本身没有声明预算与时限）。
    pub fn from_params_standalone(v: &Value) -> Self {
        let mut c = Self::default();
        if let Some(x) = v.get("maxFailProbability").and_then(Value::as_f64) {
            c.max_fail_probability = x.clamp(0.0, 1.0);
        }
        if let Some(x) = v.get("budgetTokens").and_then(Value::as_f64) {
            if x > 0.0 {
                c.budget_tokens = Some(x);
            }
        }
        if let Some(x) = v.get("deadlineMs").and_then(Value::as_f64) {
            if x > 0.0 {
                c.deadline_ms = Some(x);
            }
        }
        c
    }
}

/// 一条判据的裁决结果。
#[derive(Debug, Clone, Serialize)]
pub struct KillCheck {
    /// `fail-probability` | `budget-tokens` | `deadline`
    pub id: &'static str,
    /// `ratio`（0..1）或 `tokens` / `ms`
    pub unit: &'static str,
    pub threshold: f64,
    pub actual: f64,
    pub violated: bool,
}

/// 一条战术的完整裁决。
#[derive(Debug, Clone, Serialize)]
pub struct TacticVerdict {
    pub tactic: String,
    pub viable: bool,
    pub checks: Vec<KillCheck>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WhyNot {
    pub tactic: String,
    /// `killed` | `lower-success` | `costlier` | `slower`
    pub reason: &'static str,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rationale {
    pub recommended: Option<String>,
    /// 没有一条战术能同时满足判据 —— 这种情况必须显式说出来，而不是「矮子里拔将军」
    pub no_viable_tactic: bool,
    pub reason: String,
    pub verdicts: Vec<TacticVerdict>,
    pub why_not: Vec<WhyNot>,
}

/// 按判据裁决全部战术，并给出推荐。
pub fn evaluate(results: &[TacticResult], criteria: &Criteria) -> Rationale {
    let verdicts: Vec<TacticVerdict> = results
        .iter()
        .map(|r| {
            let mut checks = vec![KillCheck {
                id: "fail-probability",
                unit: "ratio",
                threshold: criteria.max_fail_probability,
                actual: r.fail_probability(),
                violated: r.fail_probability() > criteria.max_fail_probability,
            }];
            if let Some(b) = criteria.budget_tokens {
                checks.push(KillCheck {
                    id: "budget-tokens",
                    unit: "tokens",
                    threshold: b,
                    actual: r.p90_tokens,
                    violated: r.p90_tokens > b,
                });
            }
            if let Some(d) = criteria.deadline_ms {
                checks.push(KillCheck {
                    id: "deadline",
                    unit: "ms",
                    threshold: d,
                    actual: r.p90_ms,
                    violated: r.p90_ms > d,
                });
            }
            TacticVerdict {
                tactic: r.tactic.to_string(),
                viable: checks.iter().all(|c| !c.violated),
                checks,
            }
        })
        .collect();

    let viable: Vec<&TacticResult> = results
        .iter()
        .zip(&verdicts)
        .filter(|(_, v)| v.viable)
        .map(|(r, _)| r)
        .collect();

    let (winner, no_viable) = if viable.is_empty() {
        // 全军覆没：按「违反条数 → 成功率」挑一个最接近的，但**明确标记不可行**。
        let pick = results
            .iter()
            .zip(&verdicts)
            .min_by(|(a, va), (b, vb)| {
                let ca = va.checks.iter().filter(|c| c.violated).count();
                let cb = vb.checks.iter().filter(|c| c.violated).count();
                ca.cmp(&cb)
                    .then_with(|| b.success_rate.total_cmp(&a.success_rate))
            })
            .map(|(r, _)| r);
        (pick, true)
    } else {
        let pick = viable
            .iter()
            .copied()
            .max_by(|a, b| {
                a.success_rate
                    .total_cmp(&b.success_rate)
                    .then_with(|| b.p90_cost_usd.total_cmp(&a.p90_cost_usd))
                    .then_with(|| b.p90_ms.total_cmp(&a.p90_ms))
            });
        (pick, false)
    };

    let reason = match (winner, no_viable) {
        (Some(w), false) => format!(
            "在全部判据内，`{}` 的成功率最高（{:.1}%），p90 成本 ${:.2}、p90 耗时 {:.0}s。",
            w.tactic,
            w.success_rate * 100.0,
            w.p90_cost_usd,
            w.p90_ms / 1000.0
        ),
        (Some(w), true) => {
            let v = verdicts.iter().find(|v| v.tactic == w.tactic);
            let n = v.map_or(0, |v| v.checks.iter().filter(|c| c.violated).count());
            format!(
                "没有任何战术满足全部判据。最接近的是 `{}`，仍违反 {n} 条判据（成功率 {:.1}%）。\
                 这不是「选它」，而是「先改判据或改设计」。",
                w.tactic,
                w.success_rate * 100.0
            )
        }
        _ => "没有可用的战术结果。".to_string(),
    };

    let mut why_not = Vec::new();
    if let Some(w) = winner {
        for r in results {
            if r.tactic == w.tactic {
                continue;
            }
            let v = verdicts.iter().find(|v| v.tactic == r.tactic);
            let violated: Vec<&KillCheck> = v
                .map(|v| v.checks.iter().filter(|c| c.violated).collect())
                .unwrap_or_default();
            if !violated.is_empty() {
                let detail = violated
                    .iter()
                    .map(|c| format!("{} {:.2} 超过阈值 {:.2}", c.id, c.actual, c.threshold))
                    .collect::<Vec<_>>()
                    .join("；");
                why_not.push(WhyNot {
                    tactic: r.tactic.to_string(),
                    reason: "killed",
                    detail: format!("判据未通过：{detail}"),
                });
                continue;
            }
            let (reason, detail) = if r.success_rate < w.success_rate {
                (
                    "lower-success",
                    format!(
                        "成功率低 {:.1} 个百分点（{:.1}% vs {:.1}%）",
                        (w.success_rate - r.success_rate) * 100.0,
                        r.success_rate * 100.0,
                        w.success_rate * 100.0
                    ),
                )
            } else if r.p90_cost_usd > w.p90_cost_usd {
                (
                    "costlier",
                    format!(
                        "成功率相同但 p90 成本高 ${:.2}（{:.2} vs {:.2}）",
                        r.p90_cost_usd - w.p90_cost_usd,
                        r.p90_cost_usd,
                        w.p90_cost_usd
                    ),
                )
            } else {
                (
                    "slower",
                    format!(
                        "成功率与成本相同但 p90 慢 {:.0}s（{:.0}s vs {:.0}s）",
                        (r.p90_ms - w.p90_ms) / 1000.0,
                        r.p90_ms / 1000.0,
                        w.p90_ms / 1000.0
                    ),
                )
            };
            why_not.push(WhyNot {
                tactic: r.tactic.to_string(),
                reason,
                detail,
            });
        }
    }

    Rationale {
        recommended: winner.map(|w| w.tactic.to_string()),
        no_viable_tactic: no_viable,
        reason,
        verdicts,
        why_not,
    }
}

/// 事前验尸的一条结论。
#[derive(Debug, Clone, Serialize)]
pub struct PremortemItem {
    /// `gate-absent` | `failure-cause:{kind}` | `single-point` | `coordination-drag`
    /// | `budget-headroom` | `deadline-headroom` | `no-retry`
    pub id: String,
    /// 预计出现频率（0..1）
    pub frequency: f64,
    /// `high` | `medium` | `low`
    pub severity: &'static str,
    pub evidence: String,
    pub mitigation: String,
}

/// 事前验尸：**假设这次已经失败了**，倒推最可能的死法。
///
/// 每条结论都必须带一个可核对的数字（模拟计数 / 参数余量），否则就是编故事。
pub fn premortem(scenario: &Scenario, winner: &TacticResult) -> Vec<PremortemItem> {
    let trials = scenario.trials.max(1) as f64;
    let mut out = Vec::new();

    if !winner.gated {
        out.push(PremortemItem {
            id: "gate-absent".into(),
            frequency: winner.fail_probability(),
            severity: "high",
            evidence: format!(
                "`{}` 没有验证闸门：模拟里 {:.0}/{:.0} 次试跑以静默缺陷或未检出错误收场",
                winner.tactic,
                winner.fail_probability() * trials,
                trials
            ),
            mitigation: "换 `star-gated` / `panel`：把缺陷变成可重试的显式失败".into(),
        });
    }

    for (kind, count) in &winner.causes {
        out.push(PremortemItem {
            id: format!("failure-cause:{kind}"),
            frequency: round3(*count as f64 / trials),
            severity: if *count as f64 / trials >= 0.10 {
                "high"
            } else {
                "medium"
            },
            evidence: format!(
                "模拟中 {count} 次失败归因为 `{kind}`{}",
                winner
                    .failure_nodes
                    .first()
                    .map(|(n, c)| format!("（节点 `{n}` 命中 {c} 次）"))
                    .unwrap_or_default()
            ),
            mitigation: match kind.as_str() {
                "defect-propagated" | "gate-missed" => "加验证闸门并保留一次重试",
                "node-failed" => "提高重试次数或换更低失败的执行者",
                "coverage-gap" => "减小分片、让聚合器复核分片边界",
                "budget-tokens" => "先扩预算或缩小 DAG 宽度",
                "deadline" => "提高并行度或缩窄依赖链",
                _ => "复核该节点的先验",
            }
            .into(),
        });
    }

    if matches!(
        Topology::parse(winner.topology),
        Some(Topology::Solo) | Some(Topology::Star)
    ) {
        out.push(PremortemItem {
            id: "single-point".into(),
            frequency: winner.fail_probability().max(0.05),
            severity: "medium",
            evidence: format!(
                "`{}` 只有一个整合者：它的规划错误会同时污染全部 {} 个节点",
                winner.tactic,
                scenario.tasks.len()
            ),
            mitigation: "面板（多专家独立作答）或接力的专精分工".into(),
        });
    }

    if winner.coordination_share > 0.25 {
        out.push(PremortemItem {
            id: "coordination-drag".into(),
            frequency: winner.coordination_share,
            severity: "medium",
            evidence: format!(
                "协调/整合开销已占总 token 的 {:.0}%",
                winner.coordination_share * 100.0
            ),
            mitigation: "缩小并行宽度，或把可串行的节点收回单执行者".into(),
        });
    }

    if let Some(b) = scenario.budget_tokens {
        let util = winner.p90_tokens / b;
        if util > 0.8 {
            out.push(PremortemItem {
                id: "budget-headroom".into(),
                frequency: round3(util),
                severity: if util > 1.0 { "high" } else { "medium" },
                evidence: format!(
                    "p90 消耗 {:.0} token，占预算 {:.0} 的 {:.0}%",
                    winner.p90_tokens,
                    b,
                    util * 100.0
                ),
                mitigation: "扩预算或换更省 token 的战术".into(),
            });
        }
    }

    if let Some(d) = scenario.deadline_ms {
        let util = winner.p90_ms / d;
        if util > 0.8 {
            out.push(PremortemItem {
                id: "deadline-headroom".into(),
                frequency: round3(util),
                severity: if util > 1.0 { "high" } else { "medium" },
                evidence: format!(
                    "p90 耗时 {:.0}s，占时限 {:.0}s 的 {:.0}%",
                    winner.p90_ms / 1000.0,
                    d / 1000.0,
                    util * 100.0
                ),
                mitigation: "提高并行度或砍掉非关键路径上的节点".into(),
            });
        }
    }

    if winner.retries == 0 {
        out.push(PremortemItem {
            id: "no-retry".into(),
            frequency: winner.fail_probability(),
            severity: "medium",
            evidence: "重试次数为 0：任何一次可检出的失败都直接终止".into(),
            mitigation: "至少给 1 次重试（成本约为一次节点工作量的 1.15 倍）".into(),
        });
    }

    out.sort_by(|a, b| b.frequency.total_cmp(&a.frequency));
    out.truncate(6);
    out
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::super::model::{Model, TacticalTask};
    use super::super::sim::simulate;
    use super::*;

    fn chain(n: usize) -> Scenario {
        let tasks = (0..n)
            .map(|i| TacticalTask {
                id: format!("t{i}"),
                work_k: 20.0,
                depends_on: if i == 0 {
                    vec![]
                } else {
                    vec![format!("t{}", i - 1)]
                },
            })
            .collect();
        Scenario {
            tasks,
            trials: 400,
            seed: 11,
            budget_tokens: None,
            deadline_ms: None,
            model: Model::default(),
        }
    }

    fn results(s: &Scenario) -> Vec<TacticResult> {
        super::super::model::catalog()
            .iter()
            .map(|t| simulate(s, t))
            .collect()
    }

    #[test]
    fn every_loser_gets_a_reason() {
        let s = chain(5);
        let r = results(&s);
        let card = Criteria {
            max_fail_probability: 0.9,
            budget_tokens: None,
            deadline_ms: None,
        };
        let verdict = evaluate(&r, &card);
        assert_eq!(verdict.why_not.len(), r.len() - 1);
        assert!(verdict.recommended.is_some());
        assert!(!verdict.no_viable_tactic);
    }

    #[test]
    fn impossible_criteria_are_declared_not_hidden() {
        let s = chain(3);
        let r = results(&s);
        let card = Criteria {
            max_fail_probability: -0.1, // 不可能满足
            budget_tokens: None,
            deadline_ms: None,
        };
        let verdict = evaluate(&r, &card);
        assert!(verdict.no_viable_tactic);
        assert!(verdict.reason.contains("没有任何战术满足"));
    }

    #[test]
    fn budget_kills_expensive_tactics_by_name() {
        let s = chain(3);
        let r = results(&s);
        let card = Criteria {
            max_fail_probability: 0.99,
            budget_tokens: Some(1.0),
            deadline_ms: None,
        };
        let verdict = evaluate(&r, &card);
        assert!(verdict.no_viable_tactic);
        assert!(verdict
            .verdicts
            .iter()
            .all(|v| v.checks.iter().any(|c| c.id == "budget-tokens" && c.violated)));
    }

    #[test]
    fn premortem_flags_a_missing_gate() {
        let s = chain(5);
        let t = super::super::model::catalog()
            .into_iter()
            .find(|t| t.id == "solo")
            .unwrap();
        let r = simulate(&s, &t);
        let items = premortem(&s, &r);
        assert!(items.iter().any(|i| i.id == "gate-absent"));
        assert!(items.iter().any(|i| i.id == "no-retry"));
    }
}
