//! 视觉调用与转发（E 期 vision/）。
//!
//! - `VisionProviderResolver`：判定当前是否有可用视觉模型（由上层据 provider/模型名启发式提供）。
//! - `VisionTransport`：把图片转发到视觉模型取描述（真实实现走 OpenAI 兼容多模态接口）。
//! - `describe_image`：元数据解析 → 能力判定 → 转发；全程优雅降级。

use async_trait::async_trait;
use serde_json::Value;

use super::metadata::{image_metadata, ImageMeta};
use crate::model::llm::LLMConfig;

/// 默认描述提示词
const DEFAULT_HINT: &str = "请详细描述这张图片的内容，包括主要对象、文字与布局。";

/// 视觉能力解析器（可注入；测试用 mock 取代）
#[async_trait]
pub trait VisionProviderResolver: Send + Sync {
    /// 返回当前可用视觉模型的名称（None 表示无可用视觉能力）
    async fn resolve_vision_model(&self) -> Option<String>;
}

/// 视觉模型转发器（可注入；测试用 mock 取代真实 HTTP）
#[async_trait]
pub trait VisionTransport: Send + Sync {
    /// 调用视觉模型返回图片描述；失败返回 Err（由调用方降级）
    async fn describe(&self, data: &[u8], mime: &str, hint: &str) -> Result<String, String>;

    /// 视觉定位：截图 → 元素中心坐标（相对截图像素）+ 置信度。
    /// 默认返回 None（transport 不支持结构化定位时，调用方降级为纯描述）。
    async fn locate(&self, data: &[u8], mime: &str, hint: &str) -> Option<LocateResult> {
        let _ = (data, mime, hint);
        None
    }
}

/// 视觉定位结果：元素中心坐标（相对截图左上角，像素）+ 置信度 0..1
#[derive(Debug, Clone, PartialEq)]
pub struct LocateResult {
    pub x: f64,
    pub y: f64,
    pub confidence: f64,
}

/// 定位提示词：要求模型仅输出坐标 JSON（供 `LlmVisionTransport::locate` 使用）
const LOCATE_HINT: &str = r#"你是 GUI 元素定位器。根据截图与对目标元素的描述，找出该元素中心的坐标。仅输出一个 JSON 对象，不要任何其他文字，格式：{"x": 123.4, "y": 456.7, "confidence": 0.95}。confidence 为 0..1 的置信度。"#;

/// 从模型输出中解析定位 JSON（容忍前后缀与 ```json 围栏）
fn parse_locate_json(raw: &str) -> Option<LocateResult> {
    let trimmed = raw.trim();
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end <= start {
        return None;
    }
    let body = &trimmed[start..=end];
    let v: Value = serde_json::from_str(body).ok()?;
    let x = v.get("x")?.as_f64()?;
    let y = v.get("y")?.as_f64()?;
    let confidence = v.get("confidence").and_then(|c| c.as_f64()).unwrap_or(0.0);
    Some(LocateResult { x, y, confidence })
}

/// 构造降级文本：明确原因 + 图片元数据 + 修复指引。
fn degraded(meta: &ImageMeta, reason: &str) -> String {
    let dims = match (meta.width, meta.height) {
        (Some(w), Some(h)) => format!("{w}x{h}"),
        _ => "未知".to_string(),
    };
    format!(
        "[视觉降级] {reason}，本次无法生成图像内容描述。\n\
         图片元数据: 格式={}, 尺寸={}, 大小={} 字节。\n\
         修复方式: 为支持视觉的 provider 标记能力，并用备用视觉 provider 路由。",
        meta.format, dims, meta.bytes
    )
}

/// 分析图片并返回文字描述（核心入口）。
///
/// 流程：元数据解析 → 能力判定 → 转发；无可用视觉能力 / 上游失败返回降级文本，
/// 绝不静默吞错。
pub async fn describe_image(
    data: &[u8],
    resolver: &(dyn VisionProviderResolver + Send + Sync),
    transport: &(dyn VisionTransport + Send + Sync),
    hint: &str,
) -> String {
    let meta = image_metadata(data);
    if meta.format == "unknown" {
        return degraded(&meta, "无法识别图片格式（支持 PNG/JPEG/GIF/BMP/WebP）");
    }
    match resolver.resolve_vision_model().await {
        None => degraded(
            &meta,
            "当前无可用视觉模型（激活 provider 与备用视觉 provider 均不具备视觉能力）",
        ),
        Some(_model) => match transport.describe(data, meta.mime, hint).await {
            Ok(t) => t,
            Err(e) => degraded(&meta, &format!("视觉模型调用失败: {e}")),
        },
    }
}

/// 最小 base64 标准字母表编码（零依赖）
fn b64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// 真实视觉转发：复用 OpenAI 兼容 `/v1/chat/completions`（多模态 image_url）。
pub struct LlmVisionTransport {
    /// LLM 配置（base_url / model / api_key / max_tokens / temperature）
    pub config: LLMConfig,
}

impl LlmVisionTransport {
    /// 构造
    pub fn new(config: LLMConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl VisionTransport for LlmVisionTransport {
    async fn describe(&self, data: &[u8], mime: &str, hint: &str) -> Result<String, String> {
        let b64 = b64_encode(data);
        let prompt = if hint.is_empty() { DEFAULT_HINT } else { hint };
        self.send_multimodal(&b64, mime, prompt, self.config.max_tokens)
            .await
    }

    async fn locate(&self, data: &[u8], mime: &str, hint: &str) -> Option<LocateResult> {
        let b64 = b64_encode(data);
        let prompt = if hint.trim().is_empty() {
            LOCATE_HINT.to_string()
        } else {
            format!("{LOCATE_HINT}\n目标元素描述：{hint}")
        };
        let raw = self
            .send_multimodal(&b64, mime, &prompt, self.config.max_tokens.min(512))
            .await
            .ok()?;
        parse_locate_json(&raw)
    }
}

impl LlmVisionTransport {
    /// 公共多模态请求：发送 text + image_url 到 OpenAI 兼容接口，返回文本内容。
    async fn send_multimodal(
        &self,
        b64: &str,
        mime: &str,
        prompt: &str,
        max_tokens: u32,
    ) -> Result<String, String> {
        let body = serde_json::json!({
            "model": self.config.model,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": prompt},
                    {"type": "image_url", "image_url": {"url": format!("data:{};base64,{}", mime, b64)}}
                ]
            }],
            "max_tokens": max_tokens,
            "temperature": self.config.temperature,
            "stream": false
        });
        let url = format!(
            "{}/v1/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("status {}", resp.status().as_u16()));
        }
        let v: Value = resp.json().await.map_err(|e| format!("bad json: {e}"))?;
        let content = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if content.trim().is_empty() {
            Err("视觉模型返回为空".to_string())
        } else {
            Ok(content)
        }
    }
}

// ---------------------------------------------------------------------------
// Mock 实现（测试用）
// ---------------------------------------------------------------------------

/// mock 解析器：固定返回一个视觉模型名
pub struct MockVisionResolver {
    /// 是否具备视觉能力
    pub capable: bool,
}

#[async_trait]
impl VisionProviderResolver for MockVisionResolver {
    async fn resolve_vision_model(&self) -> Option<String> {
        if self.capable {
            Some("mock-vision-model".to_string())
        } else {
            None
        }
    }
}

/// mock 转发器：返回固定描述
pub struct MockVisionTransport {
    /// 返回的描述文本
    pub reply: String,
}

#[async_trait]
impl VisionTransport for MockVisionTransport {
    async fn describe(&self, _data: &[u8], _mime: &str, _hint: &str) -> Result<String, String> {
        Ok(self.reply.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&[0, 0, 0, 13]);
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&64u32.to_be_bytes());
        v.extend_from_slice(&48u32.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0]);
        v
    }

    #[tokio::test]
    async fn describes_when_capable() {
        let data = png();
        let r = MockVisionResolver { capable: true };
        let t = MockVisionTransport {
            reply: "一只猫".to_string(),
        };
        let out = describe_image(&data, &r, &t, "").await;
        assert_eq!(out, "一只猫");
    }

    #[tokio::test]
    async fn degrades_when_no_capability() {
        let data = png();
        let r = MockVisionResolver { capable: false };
        let t = MockVisionTransport {
            reply: "x".to_string(),
        };
        let out = describe_image(&data, &r, &t, "").await;
        assert!(out.contains("[视觉降级]"));
        assert!(out.contains("无可用视觉模型"));
    }

    #[tokio::test]
    async fn degrades_on_unknown_format() {
        let r = MockVisionResolver { capable: true };
        let t = MockVisionTransport {
            reply: "x".to_string(),
        };
        let out = describe_image(b"garbage", &r, &t, "").await;
        assert!(out.contains("无法识别图片格式"));
    }

    #[tokio::test]
    async fn degrades_on_transport_error() {
        struct FailTransport;
        #[async_trait]
        impl VisionTransport for FailTransport {
            async fn describe(&self, _: &[u8], _: &str, _: &str) -> Result<String, String> {
                Err("boom".to_string())
            }
        }
        let r = MockVisionResolver { capable: true };
        let out = describe_image(&png(), &r, &FailTransport, "").await;
        assert!(out.contains("视觉模型调用失败: boom"));
    }

    #[test]
    fn b64_encoder_basic() {
        // "Man" -> "TWFu"
        assert_eq!(b64_encode(b"Man"), "TWFu");
        // "Ma" -> "TWE="
        assert_eq!(b64_encode(b"Ma"), "TWE=");
        // "M" -> "TQ=="
        assert_eq!(b64_encode(b"M"), "TQ==");
    }

    #[test]
    fn parse_locate_plain_json() {
        let out = parse_locate_json(r#"{"x": 123.5, "y": 456.25, "confidence": 0.92}"#).unwrap();
        assert_eq!(
            out,
            LocateResult {
                x: 123.5,
                y: 456.25,
                confidence: 0.92
            }
        );
    }

    #[test]
    fn parse_locate_tolerates_fences_and_prose() {
        let out = parse_locate_json(
            "好的，坐标如下：\n```json\n{\"x\":10,\"y\":20,\"confidence\":0.8}\n```\n",
        )
        .unwrap();
        assert_eq!(out.x, 10.0);
        assert_eq!(out.y, 20.0);
        assert!((out.confidence - 0.8).abs() < 1e-9);
    }

    #[test]
    fn parse_locate_rejects_garbage() {
        assert!(parse_locate_json("抱歉，我无法定位该元素。").is_none());
        assert!(parse_locate_json("{}").is_none());
    }

    #[tokio::test]
    async fn locate_defaults_to_none_for_mock_transport() {
        // 未实现 locate 的 transport（mock）默认返回 None → 调用方降级为纯描述
        let r = MockVisionResolver { capable: true };
        let t = MockVisionTransport {
            reply: "一只猫".to_string(),
        };
        let out = describe_image(&png(), &r, &t, "").await;
        assert_eq!(out, "一只猫");
    }
}
