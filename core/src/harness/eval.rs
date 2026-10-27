//! 白盒评估层（Omni-Agent 质量可见）：任务集 × 输出 → 量化分数 / 命中率 / 错误率。
//!
//! 对齐「拟建设计」：内置量化仪表盘——莱文斯坦相似度测质量、命中率与错误率可见；
//! 供自愈环（crate::harness::distill 的 diagnose / mutate_prompt / EvolutionGate）
//! 消费，形成「评估 → 诊断 → 改进 → 再评估」数据飞轮。

use std::cmp;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalTask {
    pub id: String,
    pub prompt: String,
    pub expected: String,
}

#[derive(Debug, Clone)]
pub struct EvalEntry {
    pub id: String,
    pub score: f64,
    pub exact: bool,
    pub length_ratio: f64,
}

#[derive(Debug, Clone)]
pub struct EvalSummary {
    pub total: usize,
    pub avg_score: f64,
    pub exact_count: usize,
    pub hit_rate: f64,
    pub error_rate: f64,
    pub min_score: f64,
    pub max_score: f64,
}

pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = cmp::min(cmp::min(curr[j] + 1, prev[j + 1] + 1), prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

pub fn similarity(expected: &str, actual: &str) -> f64 {
    let max_len = expected.chars().count().max(actual.chars().count());
    if max_len == 0 {
        return 1.0;
    }
    let dist = levenshtein(expected, actual);
    (1.0 - dist as f64 / max_len as f64).clamp(0.0, 1.0)
}

pub fn score_for(expected: &str, actual: &str) -> f64 {
    similarity(expected, actual)
}

pub fn eval_suite(tasks: &[EvalTask], actuals: &[(String, String)]) -> Vec<EvalEntry> {
    tasks
        .iter()
        .map(|t| {
            let actual = actuals
                .iter()
                .find(|(id, _)| id == &t.id)
                .map(|(_, a)| a.as_str())
                .unwrap_or("");
            let score = similarity(&t.expected, actual);
            let elen = t.expected.chars().count();
            EvalEntry {
                id: t.id.clone(),
                score,
                exact: t.expected == actual,
                length_ratio: if elen == 0 {
                    1.0
                } else {
                    actual.chars().count() as f64 / elen as f64
                },
            }
        })
        .collect()
}

pub fn summarize(entries: &[EvalEntry]) -> EvalSummary {
    let total = entries.len();
    if total == 0 {
        return EvalSummary {
            total: 0,
            avg_score: 0.0,
            exact_count: 0,
            hit_rate: 0.0,
            error_rate: 0.0,
            min_score: 0.0,
            max_score: 0.0,
        };
    }
    let sum: f64 = entries.iter().map(|e| e.score).sum();
    let mut min = f64::MAX;
    let mut max = f64::MIN;
    let mut exact = 0;
    let mut hits = 0;
    for e in entries {
        min = min.min(e.score);
        max = max.max(e.score);
        if e.exact {
            exact += 1;
        }
        if e.score >= 0.8 {
            hits += 1;
        }
    }
    let hit_rate = hits as f64 / total as f64;
    EvalSummary {
        total,
        avg_score: sum / total as f64,
        exact_count: exact,
        hit_rate,
        error_rate: 1.0 - hit_rate,
        min_score: min,
        max_score: max,
    }
}

/// 单轮自愈飞轮结果。
#[derive(Debug, Clone)]
pub struct FlywheelResult {
    pub accepted: bool,
    pub before: f64,
    pub after: f64,
    pub mutated_prompt: Option<String>,
}

/// 自愈飞轮单轮：评估(before) → 诊断薄弱面 → 变异基础提示 → 门控(score_after>=score_before 且 >=threshold) → 落地变异。
/// 直接接入 crate::harness::distill::run_self_evolve（评测→诊断→变异→门控同一语义）。
pub fn eval_flywheel(
    tasks: &[EvalTask],
    actuals_before: &[(String, String)],
    actuals_after: &[(String, String)],
    base_prompt: &str,
    threshold: f64,
) -> FlywheelResult {
    let entries_before = eval_suite(tasks, actuals_before);
    let entries_after = eval_suite(tasks, actuals_after);
    let before = summarize(&entries_before).avg_score;
    let after = summarize(&entries_after).avg_score;
    let weak: Vec<String> = entries_before
        .iter()
        .filter(|e| e.score < 0.8)
        .map(|e| format!("task:{}", e.id))
        .collect();
    let aspects: Vec<&str> = weak.iter().map(|s| s.as_str()).collect();
    let diagnosis = crate::harness::distill::diagnose(&aspects);
    let mutated =
        crate::harness::distill::run_self_evolve(base_prompt, &diagnosis, before, after, threshold);
    FlywheelResult {
        accepted: mutated.is_some(),
        before,
        after,
        mutated_prompt: mutated,
    }
}

/// 多轮飞轮：consecutive 轮次输出两两比较（round[0]=基线），返回每化解轮结果。
pub fn loop_flywheel(
    tasks: &[EvalTask],
    rounds: &[Vec<(String, String)>],
    base_prompt: &str,
    threshold: f64,
    max_rounds: usize,
) -> Vec<FlywheelResult> {
    let n = rounds.len().min(max_rounds.saturating_add(1));
    let mut results = Vec::with_capacity(n.saturating_sub(1));
    for i in 1..n {
        results.push(eval_flywheel(
            tasks,
            &rounds[i - 1],
            &rounds[i],
            base_prompt,
            threshold,
        ));
    }
    results
}

// ───────────────────────────────────────────────────────────────────────────
// 加速度报告 —— 区分「迭代」与「递归」
// ───────────────────────────────────────────────────────────────────────────

/// 一轮自改进的加速度报告。
///
/// RSI 的判据不是「这轮涨了多少」，而是「这轮有没有让下一轮更容易涨」：
/// - `delta`（速度）= 本轮相对上轮的提升
/// - `delta_of_delta`（加速度）= 速度的变化；只有它 > 0 才谈得上递归
/// - `reuse_rate`（复用率）= 上轮沉淀的知识，本轮实际被调用的比例
///
/// `reuse_rate` 是最关键的一列：长期为 0 说明系统只是每轮重新学，
/// 沉淀没有进入下一轮 —— 这正是「看起来在进步、其实没复利」的机器判据。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccelerationReport {
    /// 轮次（从 1 开始）
    pub round: usize,
    /// 本轮得分
    pub score: f64,
    /// 相对上轮的提升（速度）
    pub delta: f64,
    /// 速度的变化（加速度）；首轮无历史时为 0
    pub delta_of_delta: f64,
    /// 每单位提升消耗的 token；提升 <= 0 时为 `None`
    ///
    /// 刻意用 `Option` 而不是 `f64::INFINITY`：后者无法被 `serde_json` 序列化，
    /// 会在落盘或回传时直接报错 —— 这类错误只在真正落盘那天才暴露。
    pub cost_per_gain: Option<f64>,
    /// 上轮沉淀被本轮复用的比例（0..=1）
    pub reuse_rate: f64,
}

impl AccelerationReport {
    /// 是否具备「递归」特征：加速度为正 **且** 上轮沉淀确实被复用。
    ///
    /// 只满足其一的都不是递归：加速度正但复用 0，提升来自本轮运气；
    /// 复用高但加速度 <= 0，是在吃老本。
    pub fn is_compounding(&self) -> bool {
        self.delta_of_delta > 0.0 && self.reuse_rate > 0.0
    }
}

/// 计算一轮的加速度报告。
///
/// `prev_delta` 传上一轮报告的 `delta`；首轮传 `None`（加速度记 0）。
/// `reused` / `available` 是**上轮沉淀**的知识条数中被本轮调用的数量与总数。
pub fn acceleration_report(
    round: usize,
    score: f64,
    prev_score: f64,
    prev_delta: Option<f64>,
    tokens: u64,
    reused: usize,
    available: usize,
) -> AccelerationReport {
    let delta = score - prev_score;
    let delta_of_delta = match prev_delta {
        Some(d) => delta - d,
        None => 0.0,
    };
    let cost_per_gain = if delta > 0.0 {
        Some(tokens as f64 / delta)
    } else {
        None
    };
    let reuse_rate = if available == 0 {
        0.0
    } else {
        (reused.min(available) as f64) / (available as f64)
    };
    AccelerationReport {
        round,
        score,
        delta,
        delta_of_delta,
        cost_per_gain,
        reuse_rate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acceleration_distinguishes_recursive_from_iterative() {
        // 首轮：无历史，加速度记 0，不可能是递归
        let r1 = acceleration_report(1, 0.5, 0.0, None, 1_000, 0, 0);
        assert!((r1.delta - 0.5).abs() < 1e-9);
        assert_eq!(r1.delta_of_delta, 0.0);
        assert_eq!(r1.reuse_rate, 0.0);
        assert!(!r1.is_compounding(), "首轮不可能是递归");

        // 第二轮：涨 0.2，慢于上轮的 0.5 → 加速度为负
        let r2 = acceleration_report(2, 0.7, 0.5, Some(r1.delta), 1_000, 3, 10);
        assert!((r2.delta - 0.2).abs() < 1e-9);
        assert!(r2.delta_of_delta < 0.0, "速度变慢，加速度应为负");
        assert!((r2.reuse_rate - 0.3).abs() < 1e-9);
        assert!(!r2.is_compounding(), "加速度为负不算递归");

        // 第三轮：涨 0.25，快于上轮的 0.2，且复用了沉淀 → 递归
        let r3 = acceleration_report(3, 0.95, 0.7, Some(r2.delta), 1_000, 8, 10);
        assert!((r3.delta - 0.25).abs() < 1e-9);
        assert!(r3.delta_of_delta > 0.0);
        assert!((r3.reuse_rate - 0.8).abs() < 1e-9);
        assert!(r3.is_compounding(), "加速度正 + 有复用 = 递归");
    }

    #[test]
    fn acceleration_requires_both_acceleration_and_reuse() {
        // 加速度正但零复用 → 提升来自本轮运气，不算递归
        let a = acceleration_report(2, 0.9, 0.5, Some(0.1), 100, 0, 10);
        assert!(a.delta_of_delta > 0.0);
        assert_eq!(a.reuse_rate, 0.0);
        assert!(!a.is_compounding());

        // 复用高但加速度为负 → 吃老本，也不算递归
        let b = acceleration_report(2, 0.55, 0.5, Some(0.5), 100, 10, 10);
        assert!(b.delta_of_delta < 0.0);
        assert!((b.reuse_rate - 1.0).abs() < 1e-9);
        assert!(!b.is_compounding());
    }

    #[test]
    fn acceleration_report_is_json_safe() {
        // 无提升时不得用 Infinity（serde_json 会直接报错），必须是 null
        let flat = acceleration_report(1, 0.5, 0.5, Some(0.0), 100, 0, 0);
        assert!(flat.cost_per_gain.is_none(), "无提升时不得用 Infinity");
        let s = serde_json::to_string(&flat).expect("必须可序列化");
        assert!(s.contains("\"cost_per_gain\":null"), "实际：{s}");

        // 有提升时给出可读的每单位成本
        let up = acceleration_report(1, 0.8, 0.5, None, 1_500, 0, 0);
        let c = up.cost_per_gain.expect("有提升应有成本");
        assert!((c - 5000.0).abs() < 1e-6, "1500 / 0.3 = 5000，实际 {c}");
    }

    #[test]
    fn levenshtein_basics() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", "abc"), 0);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("abc", "abd"), 1);
        assert_eq!(levenshtein("", "abc"), 3);
    }

    #[test]
    fn similarity_ranges() {
        assert_eq!(similarity("hello", "hello"), 1.0);
        assert_eq!(similarity("", ""), 1.0);
        let near = similarity("fn main()", "fn main() {");
        assert!(near > 0.8, "接近应高分: {near}");
        let far = similarity("aaaa", "bbbb");
        assert!(far < 0.2, "无关应低分: {far}");
    }

    #[test]
    fn suite_and_summary_quantifies_quality() {
        let tasks = vec![
            EvalTask {
                id: "t1".into(),
                prompt: "写 hello".into(),
                expected: "hello".into(),
            },
            EvalTask {
                id: "t2".into(),
                prompt: "写 world".into(),
                expected: "world".into(),
            },
            EvalTask {
                id: "t3".into(),
                prompt: "写 xyz".into(),
                expected: "xyzxyzxyz".into(),
            },
        ];
        let actuals = vec![
            ("t1".to_string(), "hello".to_string()),
            ("t2".to_string(), "wor1d".to_string()),
            ("t3".to_string(), "xyz".to_string()),
        ];
        let entries = eval_suite(&tasks, &actuals);
        let s = summarize(&entries);
        assert_eq!(s.total, 3);
        assert_eq!(s.exact_count, 1);
        assert!(
            s.hit_rate >= 2.0 / 3.0 - 1e-9,
            "t1 满分+t2 近似应命中: {}",
            s.hit_rate
        );
        assert!(s.avg_score > 0.5, "均值应合理: {}", s.avg_score);
        assert!(
            (s.min_score - 1.0 / 3.0).abs() < 1e-9,
            "t3 最差: {}",
            s.min_score
        );
        assert_eq!(s.max_score, 1.0);
        assert!((s.error_rate + s.hit_rate - 1.0).abs() < 1e-9);
    }

    #[test]
    fn summarize_empty_is_zero() {
        let s = summarize(&[]);
        assert_eq!(s.total, 0);
        assert_eq!(s.avg_score, 0.0);
    }

    #[test]
    fn eval_flywheel_accepts_progression() {
        let tasks = vec![EvalTask {
            id: "t1".into(),
            prompt: "p".into(),
            expected: "hello".into(),
        }];
        let before = vec![("t1".to_string(), "xxxxx".to_string())];
        let after = vec![("t1".to_string(), "hello".to_string())];
        let fw = eval_flywheel(&tasks, &before, &after, "base", 0.6);
        assert!(fw.before < 0.5, "基线应低分: {}", fw.before);
        assert_eq!(fw.after, 1.0);
        assert!(fw.accepted, "改良应被采纳");
        assert!(fw
            .mutated_prompt
            .as_deref()
            .unwrap()
            .contains("[自演化变异]"));
    }

    #[test]
    fn eval_flywheel_rejects_regression() {
        let tasks = vec![EvalTask {
            id: "t1".into(),
            prompt: "p".into(),
            expected: "hello".into(),
        }];
        let before = vec![("t1".to_string(), "hello".to_string())];
        let after = vec![("t1".to_string(), "zzzzz".to_string())];
        let fw = eval_flywheel(&tasks, &before, &after, "base", 0.6);
        assert!(!fw.accepted, "退化应被拒");
        assert!(fw.mutated_prompt.is_none());
    }

    #[test]
    fn loop_flywheel_returns_per_round() {
        let tasks = vec![EvalTask {
            id: "t1".into(),
            prompt: "p".into(),
            expected: "hello world".into(),
        }];
        let rounds = vec![
            vec![("t1".to_string(), "aaaa".to_string())],
            vec![("t1".to_string(), "hello worl".to_string())],
            vec![("t1".to_string(), "hello world".to_string())],
        ];
        let results = loop_flywheel(&tasks, &rounds, "base", 0.6, 2);
        assert_eq!(results.len(), 2, "两轮化解");
        assert!(results[1].accepted, "末轮高分应落地");
        assert!(results[1].after >= results[1].before);
    }
}
