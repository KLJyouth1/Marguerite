//! 长截图 OCR 拼接（Track B4，纯逻辑、零图像解码依赖）。
//!
//! 一张长页面常被切成 N 张纵向截图，每张截图各自 OCR 出若干 spans
//! （bbox 为相对该截图的局部归一化坐标 `[x_min, y_min, x_max, y_max]`，∈[0,1]）。
//! 本模块把第 i 张截图的 spans 整体沿 y 轴偏移 `offset_y_i` 后合并为一份
//! 全局坐标的 `OcrSpan` 列表，供下游（grounding / 决策）统一消费。
//!
//! 纯加性：不改动 `evidence.rs` 中 `OcrSpan` 的任何字段与语义，仅复用该类型。
//! 不引入任何图像解码库——IO 解码留给后续切片，与本模块边界一致。

use super::evidence::OcrSpan;

/// 单张截图的 OCR 结果 + 它在长图中的纵向偏移。
#[derive(Debug, Clone, PartialEq)]
pub struct ShotOcr {
    /// 该截图相对全局坐标的纵向偏移（可为负：表示上方截图；首图通常为 0）。
    pub offset_y: f64,
    /// 该截图局部坐标下的 OCR 跨度（bbox ∈ [0,1]）。
    pub spans: Vec<OcrSpan>,
}

/// 拼接报告：记录合并成功数量与被边界纪律拒绝的数量。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StitchReport {
    /// 成功合并进全局坐标的 span 数。
    pub merged: usize,
    /// 因越界（>1 / <0 / NaN / 负值宽高）被拒绝的 span 数。
    pub rejected: usize,
}

/// 单个 bbox 是否落在归一化坐标域 [0,1]×[0,1]（含边界），且宽高非负。
///
/// 沿用 `evidence.rs` 的 `bbox_in_unit` 边界纪律：NaN/越界一律判为无效；
/// 另补「负值宽高」校验（x_max ≥ x_min 且 y_max ≥ y_min）。
fn bbox_valid(b: &[f64; 4]) -> bool {
    for &v in b.iter() {
        // NaN 不满足任何比较，故 `!is_finite()` 显式拦截。
        if !v.is_finite() || v < 0.0 || v > 1.0 {
            return false;
        }
    }
    // 宽高非负：x_max ≥ x_min 且 y_max ≥ y_min。
    if b[2] < b[0] || b[3] < b[1] {
        return false;
    }
    true
}

/// 长截图 OCR 拼接（纯合并结果版）。
///
/// 对每个 shot，将其 spans 的 bbox **两个 y 分量整体加 `offset_y`**（x 不变，
/// `[x_min, y_min+offset_y, x_max, y_max+offset_y]`），输出合并后按 y 升序排列的
/// 全局 spans。越界 span 被静默丢弃。
///
/// 该版本不暴露被拒计数；若需审计拒绝数，请用 [`stitch_ocr_report`]。
pub fn stitch_ocr(shots: &[ShotOcr]) -> Vec<OcrSpan> {
    stitch_ocr_report(shots).0
}

/// 长截图 OCR 拼接（带报告版）。
///
/// 语义同 [`stitch_ocr`]，额外返回 [`StitchReport`]：
/// - `merged`：成功合并的 span 数；
/// - `rejected`：因边界纪律被拒的 span 数（越界 / NaN / 负值宽高）。
///
/// 合并结果按 `y_min` 升序排序（稳定），便于下游顺序消费。
pub fn stitch_ocr_report(shots: &[ShotOcr]) -> (Vec<OcrSpan>, StitchReport) {
    let mut merged: Vec<OcrSpan> = Vec::new();
    let mut rejected: usize = 0;

    for shot in shots {
        for span in &shot.spans {
            let mut b = span.bbox;
            // 仅 y 分量整体偏移；x 不变。offset_y 可为任意浮点（含负）。
            b[1] += shot.offset_y;
            b[3] += shot.offset_y;
            if !bbox_valid(&b) {
                rejected += 1;
                continue;
            }
            merged.push(OcrSpan {
                text: span.text.clone(),
                bbox: b,
                conf: span.conf,
            });
        }
    }

    // 按 y_min（bbox[1]）升序稳定排序；值均为有限浮点，partial_cmp 安全。
    merged.sort_by(|a, b| {
        a.bbox[1]
            .partial_cmp(&b.bbox[1])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let merged_len = merged.len();
    (
        merged,
        StitchReport {
            merged: merged_len,
            rejected,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(y_min: f64, y_max: f64) -> OcrSpan {
        OcrSpan {
            text: "t".into(),
            bbox: [0.1, y_min, 0.5, y_max],
            conf: 0.9,
        }
    }

    #[test]
    fn two_shots_offset_applied_and_merged() {
        // 两张截图：首图 offset 0，第二张整体下移 0.5。
        let shots = vec![
            ShotOcr {
                offset_y: 0.0,
                spans: vec![span(0.1, 0.3)],
            },
            ShotOcr {
                offset_y: 0.5,
                spans: vec![span(0.1, 0.3)],
            },
        ];
        let merged = stitch_ocr(&shots);
        assert_eq!(merged.len(), 2, "两张合法截图各 1 span → 合并 2 个");
        // 第二张的 y 整体 +0.5
        assert_eq!(merged[1].bbox, [0.1, 0.6, 0.5, 0.8], "第二张 y 应整体 +0.5");
        assert_eq!(merged[0].bbox, [0.1, 0.1, 0.5, 0.3], "首图 y 不变");
    }

    #[test]
    fn three_shots_with_negative_offset_merged_sorted() {
        // 三张截图含负 offset，验证归并后按全局 y 升序。
        let shots = vec![
            ShotOcr {
                // 上方截图：局部 y∈[0.1,0.3] + (-0.2) → 全局 [-0.1,0.1]（越界，应被拒）
                offset_y: -0.2,
                spans: vec![span(0.1, 0.3)],
            },
            ShotOcr {
                // 中间截图：局部 y∈[0.1,0.3] + 0.0 → 全局 [0.1,0.3]
                offset_y: 0.0,
                spans: vec![span(0.1, 0.3)],
            },
            ShotOcr {
                // 下方截图：局部 y∈[0.4,0.6] + 0.4 → 全局 [0.8,1.0]
                offset_y: 0.4,
                spans: vec![span(0.4, 0.6)],
            },
        ];
        let (merged, report) = stitch_ocr_report(&shots);
        assert_eq!(report.rejected, 1, "负偏移致越界的 1 个 span 应被拒");
        assert_eq!(report.merged, 2, "其余 2 个合并");
        // 按全局 y 升序：中间 [0.1,0.3] 在前，下方 [0.8,1.0] 在后
        assert_eq!(merged[0].bbox, [0.1, 0.1, 0.5, 0.3]);
        assert_eq!(merged[1].bbox, [0.1, 0.8, 0.5, 1.0]);
    }

    #[test]
    fn out_of_bounds_span_rejected_and_counted() {
        let shots = vec![
            ShotOcr {
                offset_y: 0.0,
                spans: vec![span(0.1, 0.3)], // 合法
            },
            ShotOcr {
                offset_y: 0.8,
                // 局部 y∈[0.5,0.7] + 0.8 → [1.3,1.5] 越界（>1）
                spans: vec![span(0.5, 0.7)],
            },
        ];
        let (merged, report) = stitch_ocr_report(&shots);
        assert_eq!(report.merged, 1, "仅 1 个合法");
        assert_eq!(report.rejected, 1, "越界 1 个计入 rejected");
        assert_eq!(merged[0].bbox, [0.1, 0.1, 0.5, 0.3]);
    }

    #[test]
    fn empty_input_returns_empty() {
        let merged = stitch_ocr(&[]);
        assert!(merged.is_empty(), "空输入返回空");
        let (m2, report) = stitch_ocr_report(&[]);
        assert!(m2.is_empty());
        assert_eq!(report, StitchReport { merged: 0, rejected: 0 });
    }

    #[test]
    fn nan_or_negative_dimension_rejected() {
        // bbox 含 NaN → 拒绝
        let nan_shot = ShotOcr {
            offset_y: 0.0,
            spans: vec![OcrSpan {
                text: "nan".into(),
                bbox: [0.1, f64::NAN, 0.5, 0.3],
                conf: 0.9,
            }],
        };
        // bbox 宽高为负（x_max < x_min）→ 拒绝
        let neg_dim = ShotOcr {
            offset_y: 0.0,
            spans: vec![OcrSpan {
                text: "neg".into(),
                bbox: [0.8, 0.1, 0.3, 0.3],
                conf: 0.9,
            }],
        };
        let (_, r1) = stitch_ocr_report(&[nan_shot]);
        assert_eq!(r1.rejected, 1);
        let (_, r2) = stitch_ocr_report(&[neg_dim]);
        assert_eq!(r2.rejected, 1);
    }
}
