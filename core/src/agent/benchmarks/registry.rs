//! 基准评分登记册：定义待测指标与诚实展示逻辑。
//!
//! 诚实纪律：本面板只呈现「真实测量」或「明确未验证」两种状态，禁止编造任何分数。
//! 在目标硬件（RTX 3050 Ti / 32GB）完成真实压测前，所有指标保持 `Unverified`，
//! 前端 / CLI 统一渲染为 “—”。

/// 一个基准指标的可信评分。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BenchScore {
    /// 已实测（真实测量值）。
    Measured(f64),
    /// 尚未实测，不可展示任何数字。
    Unverified,
}

/// 单个基准指标定义。
#[derive(Debug, Clone, Copy)]
pub struct Benchmark {
    pub id: &'static str,
    pub name: &'static str,
    /// 单位（如 "ms" / "tok/s" / "%"），展示时拼接。
    pub unit: &'static str,
    pub score: BenchScore,
    /// 出处 / 测量方法说明（诚实标注），例如“尚未在目标硬件实测”。
    pub note: &'static str,
}

/// 已登记的全部基准指标。
///
/// 当前**全部为 `Unverified`**：项目尚无在目标硬件上的真实压测数据。
/// 接入真实测量后，把对应项改为 `BenchScore::Measured(v)` 即可，面板自动显示数值。
pub const BENCHMARKS: &[Benchmark] = &[
    Benchmark {
        id: "first_token_latency",
        name: "首 token 时延",
        unit: "ms",
        score: BenchScore::Unverified,
        note: "尚未在 RTX 3050 Ti 实测",
    },
    Benchmark {
        id: "tok_rate_local",
        name: "本地推理吞吐",
        unit: "tok/s",
        score: BenchScore::Unverified,
        note: "取决于量化模型与显存，待实测",
    },
    Benchmark {
        id: "reflect_gate_overhead",
        name: "反思门禁开销",
        unit: "ms",
        score: BenchScore::Unverified,
        note: "确定性自检，开销可忽略，待实测确认",
    },
    Benchmark {
        id: "fanout_parallel_files",
        name: "文件级 fan-out 并行度",
        unit: "files",
        score: BenchScore::Unverified,
        note: "由运行时枚举条目数决定，待实测",
    },
    Benchmark {
        id: "task_completion_rate",
        name: "任务完成率",
        unit: "%",
        score: BenchScore::Unverified,
        note: "需真实评测集，禁止编造",
    },
];

/// 取已测评分；未验证返回 `None`（前端渲染 “—”）。
pub fn measured_score(id: &str) -> Option<f64> {
    BENCHMARKS
        .iter()
        .find(|b| b.id == id)
        .and_then(|b| match b.score {
            BenchScore::Measured(v) => Some(v),
            BenchScore::Unverified => None,
        })
}

/// 统一展示串：已测显示「数值+单位」，未验证显示 “—”（绝不显示伪造数字）。
pub fn display_score(b: &Benchmark) -> String {
    match b.score {
        BenchScore::Measured(v) => format!("{v}{}", unit_suffix(b.unit)),
        BenchScore::Unverified => "—".to_string(),
    }
}

fn unit_suffix(unit: &str) -> String {
    if unit.is_empty() {
        String::new()
    } else {
        format!(" {unit}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_benchmarks_start_unverified_no_fabrication() {
        // 诚实纪律：在没有真实测量前，不允许任何 Measured 分数存在。
        for b in BENCHMARKS {
            assert!(
                matches!(b.score, BenchScore::Unverified),
                "指标 {} 不得在未实测时标注为 Measured",
                b.id
            );
        }
    }

    #[test]
    fn display_unverified_shows_dash() {
        assert_eq!(display_score(&BENCHMARKS[0]), "—");
        assert!(measured_score(BENCHMARKS[0].id).is_none());
    }

    #[test]
    fn measured_shows_value_with_unit() {
        // display_score 对局部 Measured 基准正确渲染「数值+单位」
        let b = Benchmark {
            id: "x",
            name: "x",
            unit: "ms",
            score: BenchScore::Measured(12.5),
            note: "",
        };
        assert_eq!(display_score(&b), "12.5 ms");
    }

    #[test]
    fn measured_score_none_while_unverified() {
        // 全局登记册在未实测前，任何 id 的 measured_score 均为 None（绝不伪造）
        for b in BENCHMARKS {
            assert!(
                measured_score(b.id).is_none(),
                "指标 {} 未实测时不应有 Measured 值",
                b.id
            );
        }
    }
}
