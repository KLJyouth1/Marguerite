//! Track E — 自学习 提升/闸门 策略（对译 opensquilla `self_learning/gates.py` +
//! `promotion.py` 的决策逻辑）。
//!
//! **透明标注**：真实 `lgbm_*.bin` / `bge_onnx` 推理 **不在本切片加载**（无 ONNX/torch）。
//! 生产接入点：候选 `score` 由训练好的 lgbm 给出、`embedding` 由 bge 给出；本切片只固化
//! "何时提升 / 何时拦截" 的判定面，使其可单测、无副作用。

/// 提升所需的最低质量分（AND-gate 之一）。对译 opensquilla 训练就绪链里的质量门槛。
pub const PROMOTE_THRESHOLD: f64 = 0.75;
/// 提升所需的最小观测样本数（seen）。样本不足则统计不可信，不提升。
pub const MIN_SAMPLES: u64 = 30;
/// 闸门地板：低于此分（或非有限值）视为信号过弱 / 异常波动，须拦截。
pub const GATE_FLOOR: f64 = 0.30;

/// 一个待决策的自学习候选。
#[derive(Debug, Clone, PartialEq)]
pub struct LearningCandidate {
    pub id: String,
    pub score: f64,
    pub seen: u64,
    pub promoted: bool,
}

/// 是否应提升该候选进入缓存/回模型。
///
/// AND-gate（镜像 opensquilla "every gate must pass" 的训练就绪链）：
/// 质量分过阈值 **且** 观测样本达标 **且** 尚未提升。三者任一不满足即不提升。
#[must_use]
pub fn should_promote(c: &LearningCandidate) -> bool {
    c.score >= PROMOTE_THRESHOLD && c.seen >= MIN_SAMPLES && !c.promoted
}

/// 是否应闸门拦截（不进缓存 / 不回模型）。
///
/// 拦截条件：`score` 过低（< `GATE_FLOOR`）**或** 非有限值（NaN/±inf，即损坏或剧烈波动的异常信号）。
#[must_use]
pub fn should_gate(c: &LearningCandidate) -> bool {
    !c.score.is_finite() || c.score < GATE_FLOOR
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(score: f64, seen: u64, promoted: bool) -> LearningCandidate {
        LearningCandidate {
            id: "c1".into(),
            score,
            seen,
            promoted,
        }
    }

    #[test]
    fn promote_requires_all_three_conditions() {
        // 边界：score == 阈值、seen == 最小样本、未提升 -> 提升
        assert!(should_promote(&cand(0.75, 30, false)));
        // score 差一档
        assert!(!should_promote(&cand(0.74, 30, false)));
        // seen 差一个
        assert!(!should_promote(&cand(0.75, 29, false)));
        // 已提升
        assert!(!should_promote(&cand(0.75, 30, true)));
    }

    #[test]
    fn promote_all_good_passes() {
        assert!(should_promote(&cand(0.99, 1000, false)));
    }

    #[test]
    fn mid_score_neither_promoted_nor_gated() {
        // 0.5 在 [GATE_FLOOR, PROMOTE_THRESHOLD) 区间：不被拦截，也不提升（正常中间态）。
        let c = cand(0.5, 100, false);
        assert!(!should_gate(&c));
        assert!(!should_promote(&c));
    }

    #[test]
    fn gate_on_low_score_and_non_finite() {
        assert!(should_gate(&cand(0.29, 100, false))); // 过低
        assert!(!should_gate(&cand(0.30, 100, false))); // 恰好地板，不拦截
        assert!(should_gate(&cand(f64::NAN, 100, false))); // 损坏
        assert!(should_gate(&cand(f64::INFINITY, 100, false))); // 波动
        assert!(should_gate(&cand(f64::NEG_INFINITY, 100, false)));
    }
}
