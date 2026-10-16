//! 视觉 grounding（Track B 切片2 / B3）：把「语义定位锚点」落到结构化证据的真实几何。
//!
//! 纯逻辑、零 LLM 开销、可单测。对齐 GUI_Agent `vision_adapter.py` 的 `SemanticLocator`
//! 定位语义，但**修正其 UIA 路径的已知缺陷**：
//!
//! > 原 Python 实现在真实 UIAutomationProvider 节点不回填 `rect`（确定性管线只抽文本、
//! > 不点击）时，直接 `return None` 静默降级 VLM。这属于「沉默逻辑错误」——调用方拿到
//! > 空坐标却无从分辨是「真的没匹配」还是「管道丢了几何」。
//!
//! 本 Rust 实现**绝不**产生静默 `None`：
//! - bbox **只能**来自证据的真实几何（`ocr.bbox` / `layout.bbox`）；
//! - 匹配成功必带非空 `reason` 说明取了哪个几何；
//! - 无匹配时 `matched=false` + `bbox=None`，但 `reason` 必为非空字符串说明为何无匹配；
//! - 越界 / 负值 / NaN 的 bbox 一律拒绝，绝不回填无效坐标。

use crate::vision::evidence::{LayoutRegion, OcrSpan, VisionEvidence};
use serde::{Deserialize, Serialize};

/// 定位目标（语义锚点的一等公民，移植自 `SemanticLocator`）。
///
/// 优先用 `text`（OCR 文本）或 `role`（版面角色）做语义匹配，绝不用裸像素坐标
/// 做初始定位。`index_hint` 仅作歧义消解的次级提示（在多个同分候选中按出现序定位）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroundingTarget {
    /// OCR 文本匹配（精确 / 子串）
    pub text: Option<String>,
    /// 版面区域角色匹配（如 "header" / "button"）
    pub role: Option<String>,
    /// 歧义消解提示：多个同分候选时，偏好第 index_hint 个（按出现序，从 0 起）
    pub index_hint: Option<usize>,
}

/// grounding 结果。
///
/// 不变量（P0）：`reason` 永不为空串——无论命中还是未命中，都必须解释清楚。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroundingResult {
    /// 回显定位目标，便于审计与链路追踪
    pub target: GroundingTarget,
    /// 是否成功把目标落到真实几何
    pub matched: bool,
    /// 命中的真实包围盒 `[x_min, y_min, x_max, y_max]`（归一化域 [0,1]）；未命中为 None
    pub bbox: Option<[f64; 4]>,
    /// 命中置信度 ∈ [0,1]（OCR 取识别置信度 + 匹配质量；版面取固定权重）
    pub score: f64,
    /// 命中/未命中的解释，**绝不空串**
    pub reason: String,
}

/// 内部候选结构：收集所有匹配项，最后一刀切取最高分。
#[derive(Clone)]
struct Candidate {
    bbox: [f64; 4],
    score: f64,
    /// 出现顺序（DFS 序），用于歧义消解时的确定性兜底
    ordinal: usize,
}

/// 单个 bbox 是否可安全使用：有限（非 NaN/Inf）且落在 [0,1]×[0,1]（含边界）。
///
/// 与 `evidence.rs::bbox_in_unit` 保持同一判定纪律，但此处为私有复制以避免跨模块
/// 改动既有符号；NaN 经 `is_finite()` 直接判无效。
fn bbox_valid(b: &[f64; 4]) -> bool {
    b.iter().all(|&v| v.is_finite() && (0.0..=1.0).contains(&v))
}

/// 分数近似相等（避免浮点尾差引发歧义判定抖动）。
fn score_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// 从候选集中取最优：先按 score 降序，再按 ordinal 升序（确定性）。
///
/// 若存在多个并列最高分，且 `index_hint` 指向其中某个候选，则优先采用该候选
/// （歧义消解）；否则采用排序后的第一个（即最高分 + 最早出现）。
fn pick_best(mut cands: Vec<Candidate>, index_hint: Option<usize>) -> Option<Candidate> {
    if cands.is_empty() {
        return None;
    }
    cands.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.ordinal.cmp(&b.ordinal))
    });
    let top = cands[0].score;
    if let Some(hint) = index_hint {
        if let Some(picked) = cands
            .iter()
            .find(|c| c.ordinal == hint && score_eq(c.score, top))
        {
            return Some(picked.clone());
        }
    }
    Some(cands[0].clone())
}

/// 递归收集版面树中所有 `role` 精确匹配且 bbox 合法的候选（前序 DFS）。
fn collect_role_candidates(regions: &[LayoutRegion], role: &str, out: &mut Vec<Candidate>) {
    for r in regions {
        if r.role == role && bbox_valid(&r.bbox) {
            out.push(Candidate {
                bbox: r.bbox,
                score: 0.7,
                ordinal: out.len(),
            });
        }
        collect_role_candidates(&r.children, role, out);
    }
}

/// 组装无匹配时的非空原因（绝不返回空串）。
fn no_match_reason(target: &GroundingTarget) -> String {
    if target.text.is_none() && target.role.is_none() && target.index_hint.is_none() {
        return "未提供任何定位目标（text / role / index_hint 均为空），无法执行 grounding".into();
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = &target.text {
        parts.push(format!("OCR 文本中无任何片段匹配「{}」", t));
    }
    if let Some(r) = &target.role {
        parts.push(format!("版面树中无角色为「{}」的区域", r));
    }
    if parts.is_empty() {
        parts.push("仅提供 index_hint，但需配合 text 或 role 才能定位（已忽略索引提示）".into());
    }
    format!("未匹配到目标：{}", parts.join("；"))
}

/// 视觉 grounding 主入口：把 `target` 落到 `evidence` 的真实几何。
///
/// 优先级：① `text` 在 `evidence.ocr` 中精确/子串匹配（取其 bbox）→
/// ② `role` 在 `evidence.layout` 中精确匹配（递归遍历版面树取其 bbox）。
/// 歧义（多匹配）按 `score` 取最高，并在 `reason` 中说明取舍；
/// 无匹配返回 `matched=false` + `bbox=None` + 非空 `reason`（**绝不静默 None**）。
pub fn locate(evidence: &VisionEvidence, target: &GroundingTarget) -> GroundingResult {
    // ── 优先级①：OCR 文本匹配 ──────────────────────────────────────────────
    if let Some(text) = &target.text {
        let mut cands: Vec<Candidate> = Vec::new();
        for span in &evidence.ocr {
            // 精确匹配优先于子串匹配（分数更高，确定性消解歧义）
            let exact = &span.text == text;
            let substring = !exact && span.text.contains(text);
            if exact || substring {
                // UIA rect=Null 缺陷的对立面：无效几何直接拒绝，绝不回填
                if !bbox_valid(&span.bbox) {
                    continue;
                }
                let score = if exact {
                    0.8 + 0.2 * span.conf // ∈ [0.8, 1.0]
                } else {
                    0.4 + 0.2 * span.conf // ∈ [0.4, 0.6]，恒低于精确匹配
                };
                cands.push(Candidate {
                    bbox: span.bbox,
                    score,
                    ordinal: cands.len(),
                });
            }
        }
        if let Some(best) = pick_best(cands, target.index_hint) {
            let reason = if count_top(&evidence.ocr, text) > 1 {
                format!(
                    "OCR 文本中匹配到「{}」的候选多处，按匹配质量（精确/置信度）取最高分命中真实包围盒",
                    text
                )
            } else {
                format!("在 OCR 文本中命中「{}」，取得真实几何包围盒", text)
            };
            return GroundingResult {
                target: target.clone(),
                matched: true,
                bbox: Some(best.bbox),
                score: best.score,
                reason,
            };
        }
    }

    // ── 优先级②：版面角色匹配（递归） ─────────────────────────────────────
    if let Some(role) = &target.role {
        let mut cands: Vec<Candidate> = Vec::new();
        collect_role_candidates(&evidence.layout, role, &mut cands);
        let role_hit_count = cands.len();
        if let Some(best) = pick_best(cands, target.index_hint) {
            let reason = if role_hit_count > 1 {
                format!(
                    "版面树中角色「{}」命中多处区域，按歧义消解规则取其一真实包围盒",
                    role
                )
            } else {
                format!("在版面树中按角色「{}」命中区域，取得真实几何包围盒", role)
            };
            return GroundingResult {
                target: target.clone(),
                matched: true,
                bbox: Some(best.bbox),
                score: best.score,
                reason,
            };
        }
    }

    // ── 无匹配：明确原因，绝不静默 ─────────────────────────────────────────
    GroundingResult {
        target: target.clone(),
        matched: false,
        bbox: None,
        score: 0.0,
        reason: no_match_reason(target),
    }
}

/// 统计 OCR 中能匹配 `text`（精确或子串）的片段数，用于 reason 说明歧义规模。
fn count_top(ocr: &[OcrSpan], text: &str) -> usize {
    ocr.iter()
        .filter(|s| s.text == text || s.text.contains(text))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vision::evidence::{LayoutRegion, OcrSpan, SemanticSummary, VisionEvidence};

    fn empty_semantics() -> SemanticSummary {
        SemanticSummary {
            summary: String::new(),
            elements: vec![],
        }
    }

    fn ev_with_ocr(spans: Vec<OcrSpan>) -> VisionEvidence {
        VisionEvidence::new(
            "deepseek-vl",
            "2026-08-18T12:00:00Z",
            spans,
            vec![],
            empty_semantics(),
        )
    }

    fn ev_with_layout(regions: Vec<LayoutRegion>) -> VisionEvidence {
        VisionEvidence::new(
            "deepseek-vl",
            "2026-08-18T12:00:00Z",
            vec![],
            regions,
            empty_semantics(),
        )
    }

    // ① 按 text 精确命中返回真实 bbox
    #[test]
    fn text_exact_hit_returns_real_bbox() {
        let e = ev_with_ocr(vec![OcrSpan {
            text: "Submit".into(),
            bbox: [0.2, 0.2, 0.8, 0.4],
            conf: 0.95,
        }]);
        let r = locate(
            &e,
            &GroundingTarget {
                text: Some("Submit".into()),
                role: None,
                index_hint: None,
            },
        );
        assert!(r.matched, "应命中");
        assert_eq!(r.bbox, Some([0.2, 0.2, 0.8, 0.4]), "应返回 OCR 真实 bbox");
        assert!(!r.reason.is_empty(), "reason 不得为空");
    }

    // ② 按 role 命中 layout 区域
    #[test]
    fn role_hit_returns_layout_bbox() {
        let e = ev_with_layout(vec![LayoutRegion {
            role: "button".into(),
            bbox: [0.1, 0.1, 0.3, 0.3],
            children: vec![],
        }]);
        let r = locate(
            &e,
            &GroundingTarget {
                text: None,
                role: Some("button".into()),
                index_hint: None,
            },
        );
        assert!(r.matched, "应按 role 命中");
        assert_eq!(
            r.bbox,
            Some([0.1, 0.1, 0.3, 0.3]),
            "应返回 layout 真实 bbox"
        );
        assert!(!r.reason.is_empty());
    }

    // ③ 歧义多匹配按 score 取最高（精确匹配分数更高）
    #[test]
    fn ambiguity_picks_highest_score() {
        let e = ev_with_ocr(vec![
            OcrSpan {
                text: "Submit Form".into(), // 子串匹配，分数较低
                bbox: [0.0, 0.0, 0.5, 0.1],
                conf: 0.9,
            },
            OcrSpan {
                text: "Submit".into(), // 精确匹配，分数较高
                bbox: [0.2, 0.2, 0.8, 0.4],
                conf: 0.9,
            },
        ]);
        let r = locate(
            &e,
            &GroundingTarget {
                text: Some("Submit".into()),
                role: None,
                index_hint: None,
            },
        );
        assert!(r.matched);
        assert_eq!(
            r.bbox,
            Some([0.2, 0.2, 0.8, 0.4]),
            "应取精确匹配（分数更高）的真实 bbox"
        );
        assert!(r.score > 0.8, "精确匹配分数应高于子串（>0.8）");
        assert!(r.reason.contains("多处"), "歧义应说明取舍: {}", r.reason);
    }

    // ④ 无匹配 matched=false 且 bbox=None 且 reason 非空
    #[test]
    fn no_match_is_explicit_not_silent() {
        let e = ev_with_ocr(vec![]);
        let r = locate(
            &e,
            &GroundingTarget {
                text: Some("Nope".into()),
                role: None,
                index_hint: None,
            },
        );
        assert!(!r.matched, "无匹配应 matched=false");
        assert_eq!(r.bbox, None, "无匹配 bbox 必须为 None");
        assert!(
            !r.reason.is_empty(),
            "UIA 式静默 None 缺陷已修复：reason 必须非空"
        );
    }

    // ⑤ bbox 含负值 / NaN 被拒绝返回无匹配（不回填无效几何）
    #[test]
    fn invalid_bbox_negative_rejected() {
        let e = ev_with_ocr(vec![OcrSpan {
            text: "X".into(),
            bbox: [-0.1, 0.0, 0.5, 0.5], // 负值越界
            conf: 0.9,
        }]);
        let r = locate(
            &e,
            &GroundingTarget {
                text: Some("X".into()),
                role: None,
                index_hint: None,
            },
        );
        assert!(!r.matched, "无效(负值) bbox 应被拒绝");
        assert_eq!(r.bbox, None, "不得回填越界几何");
        assert!(!r.reason.is_empty());
    }

    #[test]
    fn invalid_bbox_nan_rejected() {
        let e = ev_with_ocr(vec![OcrSpan {
            text: "Y".into(),
            bbox: [0.1, 0.1, f64::NAN, 0.5], // NaN
            conf: 0.9,
        }]);
        let r = locate(
            &e,
            &GroundingTarget {
                text: Some("Y".into()),
                role: None,
                index_hint: None,
            },
        );
        assert!(!r.matched, "NaN bbox 应被拒绝");
        assert_eq!(r.bbox, None);
        assert!(!r.reason.is_empty());
    }
}
