//! 视觉取证结构化证据（Track B1，对齐 modlens「贴图 → 结构化 JSON 证据」契约）。
//!
//! 把一张图片经视觉模型解析出的 OCR / 版面 / 语义结果收敛为可序列化、可观测、
//! 可审计的结构化证据。下游（grounding / pixel diff / 决策日志）消费此结构，
//! 而非原始图片字节。
//!
//! 安全纪律（P0）：`provenance.hash` 永远只存 `crate::cache::hash_text(model)` 的
//! 十六进制指纹，**绝不**存原始图片字节、绝不存明文 API key / token。

use serde::{Deserialize, Serialize};

/// 单条 OCR 文本跨度（包围盒 + 置信度）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrSpan {
    /// 识别出的文本
    pub text: String,
    /// 归一化包围盒 [x_min, y_min, x_max, y_max]，坐标域 [0,1]×[0,1]
    pub bbox: [f64; 4],
    /// 识别置信度 ∈ [0,1]
    pub conf: f64,
}

/// 版面区域（可递归嵌套子区域，构成版面树）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutRegion {
    /// 区域角色（如 "header" / "paragraph" / "button"）
    pub role: String,
    /// 归一化包围盒 [x_min, y_min, x_max, y_max]
    pub bbox: [f64; 4],
    /// 子区域（递归；叶子节点为空）
    pub children: Vec<LayoutRegion>,
}

/// 语义摘要（高层理解，非逐像素）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticSummary {
    /// 一段自由文本摘要
    pub summary: String,
    /// 关键元素列表（要点化）
    pub elements: Vec<String>,
}

/// 证据来源血统（provenance）：谁、何时、哪份数据产出本证据。
///
/// `hash` 字段只存模型名的指纹（FNV-1a 十六进制），不存任何敏感原值。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    /// 产证模型名（非密钥，可记录用于溯源）
    pub model: String,
    /// 采集时间戳（ISO-8601 字符串）
    pub ts: String,
    /// `format!("{:x}", crate::cache::hash_text(model))` —— 模型名指纹，绝不存密钥
    pub hash: String,
}

/// 一张图片的完整视觉取证证据。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisionEvidence {
    /// OCR 文本跨度列表
    pub ocr: Vec<OcrSpan>,
    /// 版面区域树
    pub layout: Vec<LayoutRegion>,
    /// 语义摘要
    pub semantics: SemanticSummary,
    /// 来源血统
    pub provenance: Provenance,
}

impl VisionEvidence {
    /// 构造证据，并按纪律自动派生 `provenance.hash`（模型名指纹，绝不暴露密钥）。
    ///
    /// `model` 仅用于生成指纹与溯源记录，不会以明文密钥形式落入 `hash`。
    pub fn new(
        model: &str,
        ts: &str,
        ocr: Vec<OcrSpan>,
        layout: Vec<LayoutRegion>,
        semantics: SemanticSummary,
    ) -> Self {
        let hash = format!("{:x}", crate::cache::hash_text(model));
        Self {
            ocr,
            layout,
            semantics,
            provenance: Provenance {
                model: model.to_string(),
                ts: ts.to_string(),
                hash,
            },
        }
    }

    /// 所有包围盒是否落在归一化坐标域 [0,1]×[0,1] 内（越界即视为取证无效）。
    ///
    /// 同时校验 OCR 跨度与整棵版面区域树。
    pub fn all_bboxes_in_unit(&self) -> bool {
        let ocr_ok = self.ocr.iter().all(|s| bbox_in_unit(&s.bbox));
        let layout_ok = self.layout.iter().all(region_in_unit);
        ocr_ok && layout_ok
    }
}

/// 单个 bbox 是否落在 [0,1]×[0,1]（含边界）；NaN 视为无效。
fn bbox_in_unit(b: &[f64; 4]) -> bool {
    b.iter().all(|&v| (0.0..=1.0).contains(&v))
}

/// 递归校验版面区域树的所有 bbox 是否落在 [0,1]×[0,1]。
fn region_in_unit(r: &LayoutRegion) -> bool {
    bbox_in_unit(&r.bbox) && r.children.iter().all(region_in_unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> VisionEvidence {
        VisionEvidence::new(
            "deepseek-vl",
            "2026-08-18T12:00:00Z",
            vec![OcrSpan {
                text: "Hello".into(),
                bbox: [0.1, 0.1, 0.5, 0.3],
                conf: 0.92,
            }],
            vec![LayoutRegion {
                role: "paragraph".into(),
                bbox: [0.0, 0.0, 1.0, 1.0],
                children: vec![LayoutRegion {
                    role: "line".into(),
                    bbox: [0.1, 0.2, 0.8, 0.25],
                    children: vec![],
                }],
            }],
            SemanticSummary {
                summary: "一段英文问候".into(),
                elements: vec!["greeting".into()],
            },
        )
    }

    #[test]
    fn serde_roundtrip() {
        let e = sample();
        let s = serde_json::to_string(&e).expect("序列化");
        let back: VisionEvidence = serde_json::from_str(&s).expect("反序列化");
        assert_eq!(e, back, "JSON 往返应一致");
    }

    #[test]
    fn provenance_hash_is_model_fingerprint() {
        let e = sample();
        let expected = format!("{:x}", crate::cache::hash_text("deepseek-vl"));
        assert_eq!(e.provenance.hash, expected, "hash 必须是模型名指纹");
        assert_ne!(e.provenance.hash, "deepseek-vl", "hash 不得是明文模型名");
        assert!(
            e.provenance.hash.chars().all(|c| c.is_ascii_hexdigit()),
            "hash 应为十六进制"
        );
    }

    #[test]
    fn bbox_in_unit_passes_for_valid() {
        assert!(sample().all_bboxes_in_unit(), "合法归一化 bbox 应通过");
    }

    #[test]
    fn bbox_in_unit_fails_for_out_of_range() {
        let mut e = sample();
        e.ocr[0].bbox = [0.1, 0.1, 1.5, 0.3]; // x_max 越界
        assert!(!e.all_bboxes_in_unit(), "越界 bbox 应判为无效");
    }

    #[test]
    fn bbox_in_unit_fails_for_negative() {
        let mut e = sample();
        e.layout[0].bbox = [-0.1, 0.0, 1.0, 1.0];
        assert!(!e.all_bboxes_in_unit(), "负值 bbox 应判为无效");
    }

    #[test]
    fn bbox_in_unit_fails_for_nan() {
        let mut e = sample();
        e.ocr[0].bbox = [0.1, 0.1, f64::NAN, 0.3];
        assert!(!e.all_bboxes_in_unit(), "NaN bbox 应判为无效");
    }

    #[test]
    fn nested_layout_region_checked_recursively() {
        // 父区域合法，但嵌套子区域越界 → 整体应判无效
        let mut e = sample();
        e.layout[0].children[0].bbox = [0.0, 0.0, 2.0, 0.5];
        assert!(
            !e.all_bboxes_in_unit(),
            "嵌套子区域越界也应判为无效"
        );
    }
}
