//! modlens —— 视觉取证的结构化 JSON 输出
//!
//! 把一张图组织成可被下游（文本模型 / 呈现层）直接消费的结构化文档：
//! - **文字**：OCR 文本块（[`VisionBlock::text`]）
//! - **阅读顺序**：[`VisionDocument::reading_order`]（块 ID 自左到右、自上到下）
//! - **语义**：[`VisionBlock::semantics`] + [`VisionDocument::summary`]
//! - **版面**：[`VisionLayout`]（列数 / 分区）+ [`VisionBlock::bbox`]（像素包围盒）
//!
//! 这是「路由 → modlens 取证 → 内容呈现 → web 追踪」协同链路的**取证源头**：
//! 上游把图片交给 modlens，得到一份 JSON，下游据此呈现。
//!
//! 设计原则（与 [`super::VisionCapability`] 对齐）：
//! - 复用既有 [`VisionTransport`]（OpenAI 兼容多模态接口），不新增 trait；
//! - 视觉模型不可用 / 返回非 JSON 时，**降级为元数据兜底文档**，绝不静默失败；
//! - 纯数据契约 + 确定性解析，可单测、离线可测。

use serde::{Deserialize, Serialize};

use super::client::{VisionProviderResolver, VisionTransport};
use super::metadata::image_metadata;

/// 版面包围盒（相对图片左上角，像素坐标）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct BBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// 文本块类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Title,
    Heading,
    Paragraph,
    List,
    Table,
    Form,
    Code,
    Image,
    Caption,
    Other,
}

impl BlockKind {
    /// 蛇形命名（与 serde 输出一致）。
    pub fn as_str(&self) -> &'static str {
        match self {
            BlockKind::Title => "title",
            BlockKind::Heading => "heading",
            BlockKind::Paragraph => "paragraph",
            BlockKind::List => "list",
            BlockKind::Table => "table",
            BlockKind::Form => "form",
            BlockKind::Code => "code",
            BlockKind::Image => "image",
            BlockKind::Caption => "caption",
            BlockKind::Other => "other",
        }
    }
}

/// 单个文本块（OCR 文字 + 语义角色 + 版面位置）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisionBlock {
    pub id: String,
    pub kind: BlockKind,
    pub text: String,
    #[serde(default)]
    pub semantics: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<BBox>,
}

/// 版面结构（列数 + 自上而下、自左到右的语义分区）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct VisionLayout {
    #[serde(default)]
    pub columns: u32,
    #[serde(default)]
    pub regions: Vec<String>,
}

/// 结构化视觉文档（modlens 输出契约）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisionDocument {
    /// 一句话语义概述
    pub summary: String,
    /// 内容来源：`vision-model`（视觉模型提取）/ `metadata-fallback`（离线降级）
    pub source: String,
    /// 阅读顺序（块 ID 序列）
    #[serde(default)]
    pub reading_order: Vec<String>,
    /// 文本块
    #[serde(default)]
    pub blocks: Vec<VisionBlock>,
    /// 版面结构
    #[serde(default)]
    pub layout: VisionLayout,
}

impl VisionDocument {
    /// 序列化为 JSON 字符串（永不 panic）。
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// 从 JSON 反序列化（失败返回 None）。
    pub fn from_json(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }
}

/// 要求视觉模型返回固定 JSON schema 的结构化提示词。
const STRUCTURED_PROMPT: &str = r#"请把这张图解析成结构化 JSON，字段如下：
{
  "summary": "一句话语义概述",
  "reading_order": ["块ID，按自上而下、自左到右的阅读顺序排列"],
  "blocks": [
    {"id":"b0","kind":"title|heading|paragraph|list|table|form|code|image|caption|other","text":"OCR 文本","semantics":"该块的语义角色","bbox":{"x":0,"y":0,"width":0,"height":0}}
  ],
  "layout": {"columns": 1, "regions": ["版面分区，自上而下自左到右"]}
}
只输出 JSON，不要额外解释。"#;

/// modlens 分析器：图片 → 结构化 JSON 文档。
pub struct ModLens;

impl ModLens {
    /// 把一张图转成结构化 JSON 文档。
    ///
    /// 优先走视觉模型结构化提取；视觉模型不可用 / 返回非 JSON 时，
    /// 降级为元数据兜底（[`ModLens::metadata_fallback`]），绝不静默失败。
    pub async fn analyze(
        resolver: &dyn VisionProviderResolver,
        transport: &dyn VisionTransport,
        data: &[u8],
        hint: &str,
    ) -> VisionDocument {
        if resolver.resolve_vision_model().await.is_some() {
            let meta = image_metadata(data);
            let prompt = format!("{STRUCTURED_PROMPT}\n用户意图：{hint}");
            if let Ok(reply) = transport.describe(data, meta.mime, &prompt).await {
                if let Some(doc) = extract_json_document(&reply) {
                    return doc;
                }
            }
        }
        Self::metadata_fallback(data)
    }

    /// 元数据兜底文档：无视觉模型时，只含格式 / 尺寸，明确标注降级来源。
    pub fn metadata_fallback(data: &[u8]) -> VisionDocument {
        let meta = image_metadata(data);
        let block = VisionBlock {
            id: "b0".to_string(),
            kind: BlockKind::Other,
            text: format!(
                "[视觉降级] 无法识别内容：{} 图片 {}×{}",
                meta.format,
                meta.width.unwrap_or(0),
                meta.height.unwrap_or(0)
            ),
            semantics: "metadata_fallback".to_string(),
            bbox: meta
                .width
                .zip(meta.height)
                .map(|(w, h)| BBox {
                    x: 0.0,
                    y: 0.0,
                    width: w as f32,
                    height: h as f32,
                }),
        };
        VisionDocument {
            summary: format!("{} 图片（离线降级，未调用视觉模型）", meta.format),
            source: "metadata-fallback".to_string(),
            reading_order: vec!["b0".to_string()],
            blocks: vec![block],
            layout: VisionLayout {
                columns: 1,
                regions: Vec::new(),
            },
        }
    }
}

/// 从视觉模型回复里提取 JSON 文档，容忍 markdown 代码块包裹。
pub fn extract_json_document(reply: &str) -> Option<VisionDocument> {
    let s = reply.trim();
    if let Some(d) = VisionDocument::from_json(s) {
        return Some(d);
    }
    // 剥离 ```json ... ``` 或 ``` ... ``` 包裹后重试
    let stripped = s
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    VisionDocument::from_json(stripped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vision::client::{MockVisionResolver, MockVisionTransport};

    fn png() -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&[0, 0, 0, 13]);
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&64u32.to_be_bytes());
        v.extend_from_slice(&48u32.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0]);
        v
    }

    #[test]
    fn metadata_fallback_has_expected_shape() {
        let doc = ModLens::metadata_fallback(&png());
        assert_eq!(doc.source, "metadata-fallback");
        assert_eq!(doc.reading_order, vec!["b0"]);
        assert_eq!(doc.blocks.len(), 1);
        assert_eq!(doc.blocks[0].bbox.unwrap().width, 64.0);
        let json = doc.to_json();
        assert!(json.contains("\"source\":\"metadata-fallback\""));
        assert!(json.contains("\"reading_order\""));
        assert!(json.contains("\"layout\""));
    }

    #[test]
    fn document_json_roundtrip() {
        let doc = VisionDocument {
            summary: "一张截图".into(),
            source: "vision-model".into(),
            reading_order: vec!["b0".into(), "b1".into()],
            blocks: vec![
                VisionBlock {
                    id: "b0".into(),
                    kind: BlockKind::Title,
                    text: "标题".into(),
                    semantics: "页面标题".into(),
                    bbox: Some(BBox { x: 0.0, y: 0.0, width: 64.0, height: 8.0 }),
                },
                VisionBlock {
                    id: "b1".into(),
                    kind: BlockKind::Paragraph,
                    text: "正文".into(),
                    semantics: "说明文字".into(),
                    bbox: None,
                },
            ],
            layout: VisionLayout { columns: 1, regions: vec!["header".into(), "body".into()] },
        };
        let json = doc.to_json();
        let back = VisionDocument::from_json(&json).expect("应可反序列化");
        assert_eq!(back.summary, "一张截图");
        assert_eq!(back.blocks.len(), 2);
        assert_eq!(back.blocks[0].kind, BlockKind::Title);
    }

    #[test]
    fn extract_parses_markdown_fenced_json() {
        let reply = r#"```json
{"summary":"含标题和表格","source":"vision-model","reading_order":["b0","b1"],"blocks":[{"id":"b0","kind":"table","text":"列A|列B","semantics":"数据表"}],"layout":{"columns":2}}
```"#;
        let doc = extract_json_document(reply).expect("应解析出文档");
        assert_eq!(doc.source, "vision-model");
        assert_eq!(doc.layout.columns, 2);
        assert_eq!(doc.blocks[0].kind, BlockKind::Table);
    }

    #[test]
    fn extract_parses_plain_json() {
        let reply = r#"{"summary":"x","source":"vision-model","blocks":[],"layout":{}}"#;
        assert!(extract_json_document(reply).is_some());
    }

    #[test]
    fn extract_returns_none_on_garbage() {
        assert!(extract_json_document("这不是 JSON").is_none());
    }

    #[tokio::test]
    async fn analyze_uses_vision_model_when_capable() {
        // 视觉模型返回结构化 JSON → 直接解析
        let resolver = MockVisionResolver { capable: true };
        let transport = MockVisionTransport {
            reply: r#"{"summary":"一张登录框","source":"vision-model","reading_order":["b0"],"blocks":[{"id":"b0","kind":"form","text":"用户名","semantics":"输入框"}],"layout":{"columns":1}}"#
                .into(),
        };
        let doc = ModLens::analyze(&resolver, &transport, &png(), "识别表单").await;
        assert_eq!(doc.source, "vision-model");
        assert_eq!(doc.summary, "一张登录框");
    }

    #[tokio::test]
    async fn analyze_falls_back_when_no_vision_model() {
        let resolver = MockVisionResolver { capable: false };
        let transport = MockVisionTransport { reply: "".into() };
        let doc = ModLens::analyze(&resolver, &transport, &png(), "x").await;
        assert_eq!(doc.source, "metadata-fallback");
        assert!(doc.summary.contains("png"));
    }

    #[tokio::test]
    async fn analyze_falls_back_when_reply_is_not_json() {
        let resolver = MockVisionResolver { capable: true };
        let transport = MockVisionTransport { reply: "我不懂你说什么".into() };
        let doc = ModLens::analyze(&resolver, &transport, &png(), "x").await;
        assert_eq!(doc.source, "metadata-fallback");
    }
}
