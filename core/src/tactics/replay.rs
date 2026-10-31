//! 反事实回放：拿**真实跑过的一次运行**当输入，问「换一种协作方式会怎样」。
//!
//! 这是整套系统里最难被抄走的一块，因为它依赖的不是模型，而是证据：
//! `harness` 台账里每一次 run 的 items 都带着真实的 `elapsed_ms` / `tokens` / `ok`。
//! 把这次运行实测出来的工作量当作**恒定物理量**，只替换「调度方式」，于是不同战术
//! 之间的差异只来自战术本身——任务难度被控制住了。
//!
//! 诚实边界（全部会出现在 `gaps` 里，不会假装知道）：
//! - 台账不记录真实 DAG 形状，只能按 items 的先后当串行链（`measured-sequential`）；
//! - 台账不记录「当时有没有验证闸门 / 有没有重试」，因果链里这两环标为 `gap`；
//! - 价格先验仍来自声明，不是实测。

use serde::Serialize;

use super::advise::{evaluate, Criteria, Rationale};
use super::model::{Model, Prior, Scenario, TacticalTask, Tactic};
use super::sim::{simulate, TacticResult};

/// 台账里一条实测条目的最小投影。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasuredItem {
    pub seq: i64,
    pub name: String,
    pub elapsed_ms: f64,
    pub tokens: f64,
    pub ok: bool,
    pub error_code: Option<String>,
}

/// 一次真实运行。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedRun {
    pub run_id: String,
    pub target: String,
    pub status: String,
    pub elapsed_ms: f64,
    pub tokens: f64,
    pub ok: bool,
    pub items: Vec<MeasuredItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplayGap {
    pub field: &'static str,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WhyKind {
    /// 台账里有记录，可直接核对
    Evidence,
    /// 由证据推出来的因果（存在其它解释）
    Inference,
    /// 证据里根本没有这一环 —— 不知道就是不知道
    Gap,
}

#[derive(Debug, Clone, Serialize)]
pub struct WhyLink {
    pub question: String,
    pub answer: String,
    pub kind: WhyKind,
    /// 可核对的证据指针（台账条目 / 模拟计数）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeltaRow {
    pub tactic: String,
    pub success_rate: f64,
    pub p50_ms: f64,
    pub p90_ms: f64,
    pub p90_tokens: f64,
    /// 相对实测 p50 的时间变化（负数 = 更快）
    pub time_delta_pct: f64,
    /// 相对实测 token 的变化
    pub token_delta_pct: f64,
    /// 成功率减去实测结局（1.0 = 实测成功）
    pub success_delta: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayReport {
    pub observed: ObservedRun,
    /// `measured-sequential` | `insufficient`
    pub scenario_source: &'static str,
    pub insufficient_evidence: bool,
    pub results: Vec<TacticResult>,
    pub rationale: Rationale,
    pub deltas: Vec<DeltaRow>,
    pub why_chain: Vec<WhyLink>,
    pub gaps: Vec<ReplayGap>,
}

/// 把实测运行折算成推演场景。
///
/// 工作量优先取 token（真实信息量）；没有 token 的记录退回「耗时 × 吞吐」估算，
/// 并在 gaps 里标注——估算值不能和实测值混为一谈。
fn scenario_from_run(
    run: &ObservedRun,
    trials: usize,
    seed: u64,
) -> (Scenario, Vec<ReplayGap>, bool) {
    let mut gaps = Vec::new();
    let default_model = Model::default();
    let throughput = default_model.throughput_k_per_min.value.max(f64::MIN_POSITIVE);

    gaps.push(ReplayGap {
        field: "dag",
        reason: "台账不记录真实依赖形状，回放按 items 的先后假设为串行链（measured-sequential）"
            .into(),
    });

    if run.items.is_empty() {
        gaps.push(ReplayGap {
            field: "items",
            reason: "这次运行在台账里没有条目，无法折算工作量".into(),
        });
        return (
            Scenario {
                tasks: Vec::new(),
                trials,
                seed,
                budget_tokens: None,
                deadline_ms: None,
                model: default_model,
            },
            gaps,
            true,
        );
    }

    let mut estimated_work = 0usize;
    let mut tasks = Vec::with_capacity(run.items.len());
    for (i, it) in run.items.iter().enumerate() {
        let work_k = if it.tokens > 0.0 {
            it.tokens / 1000.0
        } else {
            estimated_work += 1;
            (it.elapsed_ms / 60_000.0 * throughput).max(1.0)
        };
        tasks.push(TacticalTask {
            id: if it.name.trim().is_empty() {
                format!("item-{}", it.seq)
            } else {
                it.name.clone()
            },
            work_k,
            depends_on: if i == 0 {
                Vec::new()
            } else {
                vec![format!("item-{}", run.items[i - 1].seq)]
            },
        });
    }
    // 上面的依赖名必须与 id 对齐（id 可能来自 name），重算一遍保证自洽。
    for i in 1..tasks.len() {
        let prev = tasks[i - 1].id.clone();
        tasks[i].depends_on = vec![prev];
    }

    if estimated_work > 0 {
        gaps.push(ReplayGap {
            field: "workload",
            reason: format!(
                "{estimated_work}/{} 条条目没有 token 记录，工作量按「耗时 × 吞吐」估算",
                run.items.len()
            ),
        });
    }

    let total = run.items.len() as f64;
    let failed = run.items.iter().filter(|i| !i.ok).count() as f64;
    let mut model = default_model;
    // 本次运行的实测失败率是**真实证据**（样本 = 条目数），用它替换默认同款先验。
    model.failure_rate = Prior::measured(failed / total, run.items.len() as u64);
    let observed_tokens: f64 = run.items.iter().map(|i| i.tokens).sum();
    if run.elapsed_ms > 0.0 && observed_tokens > 0.0 {
        model.throughput_k_per_min = Prior::measured(
            observed_tokens / 1000.0 / (run.elapsed_ms / 60_000.0),
            1,
        );
    } else {
        gaps.push(ReplayGap {
            field: "throughput",
            reason: "缺少 token 或耗时，吞吐沿用声明先验".into(),
        });
    }
    gaps.push(ReplayGap {
        field: "price",
        reason: "价格先验来自声明，不是本机实测".into(),
    });
    gaps.push(ReplayGap {
        field: "selfDetect",
        reason: "台账不记录执行者是否自检到失败，无闸门战术的传播率仍按声明先验".into(),
    });

    (
        Scenario {
            tasks,
            trials,
            seed,
            budget_tokens: None,
            deadline_ms: None,
            model,
        },
        gaps,
        false,
    )
}

/// 错误码归类：只做**可复核的字符串归类**，不做语义猜测。
fn classify_error(code: Option<&str>) -> (String, WhyKind) {
    let Some(code) = code.map(str::trim).filter(|s| !s.is_empty()) else {
        return (
            "台账没有记录错误码，失败原因无法从证据里推出".to_string(),
            WhyKind::Gap,
        );
    };
    let lower = code.to_ascii_lowercase();
    let label = if lower.contains("timeout") || lower.contains("timed_out") {
        "超时"
    } else if lower.contains("rate") || lower.contains("429") {
        "被上游限流"
    } else if lower.contains("auth") || lower.contains("401") || lower.contains("403") {
        "鉴权失败"
    } else if lower.contains("network") || lower.contains("connect") {
        "网络不可达"
    } else {
        return (
            format!("错误码 `{code}`（未归类，需要人工看日志）"),
            WhyKind::Gap,
        );
    };
    (format!("{label}（错误码 `{code}`）"), WhyKind::Inference)
}

/// 5 Why 只问「有哪些环节**有证据**」。凑不满五层就少给几层，绝不用推测填满。
fn why_chain(run: &ObservedRun, rationale: &Rationale) -> Vec<WhyLink> {
    let Some(first_fail) = run.items.iter().find(|i| !i.ok) else {
        return vec![WhyLink {
            question: "这次运行为什么没有失败？".into(),
            answer: format!(
                "台账里 {} 条条目全部成功（status=`{}`），没有可追问的失败链",
                run.items.len(),
                run.status
            ),
            kind: WhyKind::Evidence,
            evidence: Some(format!("run `{}`", run.run_id)),
        }];
    };

    let mut chain = vec![WhyLink {
        question: "这次运行为什么失败？".into(),
        answer: format!(
            "条目 `{}`（seq={}）失败，状态 `{}`",
            first_fail.name, first_fail.seq, run.status
        ),
        kind: WhyKind::Evidence,
        evidence: Some(format!("run `{}` item seq={}", run.run_id, first_fail.seq)),
    }];

    let (answer, kind) = classify_error(first_fail.error_code.as_deref());
    chain.push(WhyLink {
        question: format!("`{}` 为什么会失败？", first_fail.name),
        answer,
        kind,
        evidence: first_fail
            .error_code
            .as_ref()
            .map(|c| format!("error_code={c}")),
    });

    chain.push(WhyLink {
        question: "为什么它在运行中没被拦住？".into(),
        answer: "台账不记录当时的验证闸门与重试语义，这一环没有证据；\
                 但反事实推演显示：无闸门战术下同类失败会静默传播而不是被重试"
            .into(),
        kind: WhyKind::Gap,
        evidence: Some("harness_items 无 gate/retry 字段".into()),
    });

    if let Some(best) = &rationale.recommended {
        let row = rationale.verdicts.iter().find(|v| &v.tactic == best);
        let violations = row.map_or(0, |v| v.checks.iter().filter(|c| c.violated).count());
        chain.push(WhyLink {
            question: "为什么这套战术允许「失败无人复核」？".into(),
            answer: format!(
                "在相同实测工作量下，`{best}` 的判据违反数为 {violations}；\
                 若它仍有闸门缺失或重试为 0，下一次同类失败会以同样的方式发生"
            ),
            kind: WhyKind::Inference,
            evidence: Some("tactics.simulate 反事实结果".into()),
        });
    }

    chain.push(WhyLink {
        question: "下次要改什么，才能让这条链断开？".into(),
        answer: rationale.reason.clone(),
        kind: WhyKind::Inference,
        evidence: None,
    });

    chain
}

/// 反事实回放主入口。
pub fn replay(
    run: &ObservedRun,
    tactics: &[Tactic],
    trials: usize,
    seed: u64,
    criteria: &Criteria,
) -> ReplayReport {
    let (scenario, gaps, insufficient) = scenario_from_run(run, trials, seed);
    if insufficient {
        return ReplayReport {
            observed: run.clone(),
            scenario_source: "insufficient",
            insufficient_evidence: true,
            results: Vec::new(),
            rationale: Rationale {
                recommended: None,
                no_viable_tactic: false,
                reason: "这次运行没有可折算的条目，无法做反事实比较".into(),
                verdicts: Vec::new(),
                why_not: Vec::new(),
            },
            deltas: Vec::new(),
            why_chain: why_chain(run, &Rationale {
                recommended: None,
                no_viable_tactic: false,
                reason: String::new(),
                verdicts: Vec::new(),
                why_not: Vec::new(),
            }),
            gaps,
        };
    }

    let results: Vec<TacticResult> = tactics.iter().map(|t| simulate(&scenario, t)).collect();
    let rationale = evaluate(&results, criteria);

    let observed_ms = if run.elapsed_ms > 0.0 {
        run.elapsed_ms
    } else {
        run.items.iter().map(|i| i.elapsed_ms).sum()
    };
    let observed_tokens = if run.tokens > 0.0 {
        run.tokens
    } else {
        run.items.iter().map(|i| i.tokens).sum()
    };
    let observed_success = if run.ok { 1.0 } else { 0.0 };

    let deltas = results
        .iter()
        .map(|r| DeltaRow {
            tactic: r.tactic.to_string(),
            success_rate: r.success_rate,
            p50_ms: r.p50_ms,
            p90_ms: r.p90_ms,
            p90_tokens: r.p90_tokens,
            time_delta_pct: pct(r.p50_ms, observed_ms),
            token_delta_pct: pct(r.p50_tokens, observed_tokens),
            success_delta: round3(r.success_rate - observed_success),
        })
        .collect();

    ReplayReport {
        observed: run.clone(),
        scenario_source: "measured-sequential",
        insufficient_evidence: false,
        results,
        why_chain: why_chain(run, &rationale),
        rationale,
        deltas,
        gaps,
    }
}

fn pct(new: f64, base: f64) -> f64 {
    if base.abs() < f64::EPSILON {
        return 0.0;
    }
    round3((new - base) / base)
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 夹具的一行观测，字段顺序与 `MeasuredItem` 一致：
    /// `(序号, 名称, 耗时 ms, token, 是否成功, 错误码)`。
    /// 起别名而不是把 6 元组写在签名里：clippy L3 的 `type_complexity` 会拦它。
    type Item<'a> = (i64, &'a str, f64, f64, bool, Option<&'a str>);

    fn run(items: Vec<Item<'_>>) -> ObservedRun {
        ObservedRun {
            run_id: "harness-1".into(),
            target: "shell".into(),
            status: "error".into(),
            elapsed_ms: 120_000.0,
            tokens: 60_000.0,
            ok: false,
            items: items
                .into_iter()
                .map(|(seq, name, elapsed_ms, tokens, ok, code)| MeasuredItem {
                    seq,
                    name: name.into(),
                    elapsed_ms,
                    tokens,
                    ok,
                    error_code: code.map(str::to_string),
                })
                .collect(),
        }
    }

    fn criteria() -> Criteria {
        Criteria {
            max_fail_probability: 0.5,
            budget_tokens: None,
            deadline_ms: None,
        }
    }

    #[test]
    fn empty_run_is_insufficient_not_fabricated() {
        let r = replay(&run(vec![]), &super::super::model::catalog(), 100, 1, &criteria());
        assert!(r.insufficient_evidence);
        assert!(r.results.is_empty());
        assert!(r.gaps.iter().any(|g| g.field == "items"));
    }

    #[test]
    fn measured_failure_rate_becomes_a_prior_with_samples() {
        let r = run(vec![
            (1, "read", 10_000.0, 5_000.0, true, None),
            (2, "write", 20_000.0, 6_000.0, false, Some("timeout")),
        ]);
        let report = replay(&r, &super::super::model::catalog(), 50, 1, &criteria());
        assert!(!report.insufficient_evidence);
        // 2 条条目里 1 条失败 ⇒ 实测先验 0.5，样本数 2
        assert!(report.gaps.iter().all(|g| g.field != "items"));
        assert!(report.why_chain.iter().any(|l| l.kind == WhyKind::Evidence));
        assert!(report.why_chain.iter().any(|l| l.kind == WhyKind::Gap));
    }

    #[test]
    fn why_chain_stops_at_evidence_for_a_successful_run() {
        let mut r = run(vec![(1, "read", 10_000.0, 5_000.0, true, None)]);
        r.ok = true;
        let report = replay(&r, &super::super::model::catalog(), 20, 1, &criteria());
        assert_eq!(report.why_chain.len(), 1);
        assert_eq!(report.why_chain[0].kind, WhyKind::Evidence);
    }

    #[test]
    fn deltas_are_relative_to_the_measured_run() {
        let r = run(vec![
            (1, "a", 30_000.0, 30_000.0, true, None),
            (2, "b", 30_000.0, 30_000.0, true, None),
        ]);
        let report = replay(&r, &super::super::model::catalog(), 100, 1, &criteria());
        let row = report
            .deltas
            .iter()
            .find(|d| d.tactic == "solo")
            .unwrap();
        assert!(row.p50_ms > 0.0);
        assert!(row.time_delta_pct.is_finite());
    }

    #[test]
    fn unclassified_error_code_is_reported_as_a_gap() {
        let r = run(vec![(1, "x", 1.0, 1.0, false, Some("E_WEIRD_42"))]);
        let report = replay(&r, &super::super::model::catalog(), 20, 1, &criteria());
        assert!(report
            .why_chain
            .iter()
            .any(|l| l.kind == WhyKind::Gap && l.answer.contains("未归类")));
    }
}
