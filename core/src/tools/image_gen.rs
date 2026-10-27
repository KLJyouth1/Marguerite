//! 生图工具（`image_generate`）：文本 → 图片。
//!
//! # 为什么需要（2026-10-23）
//!
//! 盘点运行实例的 279 个 UIA 节点与工具注册表后确认：系统此前**没有任何**
//! 图像生成工具（`vis.image` 只做「看图」）。用户接入一个能出图的模型后，
//! 主智能体完全不知道可以生图，只能回「我无法生成图片」。
//!
//! 本工具补的正是这一段：把「档案声明的 image_output 能力」变成一次真实
//! 的 `/v1/images/generations` 调用，并把结果**落盘到工作区**（而不是只回
//! 一个会过期的临时 URL）。
//!
//! # 诚实边界
//!
//! - 没有声明该能力的档案时**明确报错**并告诉用户去哪个页面勾哪个框，
//!   不假装成功、不返回占位图；
//! - 只支持 OpenAI 兼容的 `images/generations` 形状（这是绝大多数国内外
//!   服务商的共同子集）；非兼容形状请用档案里的「端点覆盖 + 自定义参数」，
//!   或改用 `http_request` 自行调用。

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use base64::Engine as _;
use serde_json::{json, Value};

use crate::agent::tool_cycle::ToolResult;
use crate::mcp::registry::ToolExecutor;
use crate::model::media_capability::{self, MediaEndpoint, MediaKind};
use crate::tools::file_change::{record_artifact, GENERATED_KIND};
use crate::tools::fs_path::RootSet;

/// 出图 HTTP 通道（可注入：单测用 mock 断言请求体，不真的联网）。
#[async_trait]
pub trait ImageTransport: Send + Sync {
    /// 发起生成请求，返回上游 JSON。
    async fn generate(&self, ep: &MediaEndpoint, body: &Value) -> Result<Value, String>;
    /// 下载图片 URL 的字节（上游只回 URL 时用；避免交付物依赖临时链接）。
    async fn download(&self, url: &str) -> Result<Vec<u8>, String>;
}

/// 生产实现：reqwest。
pub struct HttpImageTransport;

#[async_trait]
impl ImageTransport for HttpImageTransport {
    async fn generate(&self, ep: &MediaEndpoint, body: &Value) -> Result<Value, String> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .build()
            .map_err(|e| format!("构造 HTTP 客户端失败: {e}"))?;
        let mut req = client.post(&ep.url).json(body);
        if !ep.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", ep.api_key));
        }
        let resp = req.send().await.map_err(|e| format!("请求失败: {e}"))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("读取响应失败: {e}"))?;
        if !status.is_success() {
            return Err(format!(
                "上游返回 {}：{}",
                status.as_u16(),
                truncate(&text, 600)
            ));
        }
        serde_json::from_str::<Value>(&text)
            .map_err(|e| format!("上游返回非 JSON（{}）：{}", e, truncate(&text, 300)))
    }

    async fn download(&self, url: &str) -> Result<Vec<u8>, String> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| format!("构造 HTTP 客户端失败: {e}"))?;
        let resp = client.get(url).send().await.map_err(|e| format!("下载失败: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("下载返回 {}", resp.status().as_u16()));
        }
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("下载内容读取失败: {e}"))
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let head: String = s.chars().take(n).collect();
        format!("{head}…")
    }
}

/// 按文件头判定图片扩展名（上游有时不给 mime）。
fn sniff_ext(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        "png"
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        "jpg"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "webp"
    } else if bytes.starts_with(b"GIF8") {
        "gif"
    } else {
        "png"
    }
}

/// 从上游 JSON 里取出第 0 张图：优先 `b64_json`，其次 `url`。
fn first_image(v: &Value) -> Result<(Option<String>, Option<String>), String> {
    let arr = v
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("上游响应缺少 data 数组：{}", truncate(&v.to_string(), 300)))?;
    let item = arr
        .first()
        .ok_or_else(|| "上游返回 data 为空数组（未生成任何图片）".to_string())?;
    let b64 = item
        .get("b64_json")
        .and_then(Value::as_str)
        .map(str::to_string);
    let url = item
        .get("url")
        .and_then(Value::as_str)
        .map(str::to_string);
    if b64.is_none() && url.is_none() {
        return Err(format!(
            "上游响应既无 b64_json 也无 url：{}",
            truncate(&item.to_string(), 300)
        ));
    }
    Ok((b64, url))
}

/// 把档案里的自定义参数与工具显式参数合并进请求体。
///
/// 优先级（低 → 高）：内置保守默认 < 档案自定义参数 < 本次工具调用参数。
fn build_body(ep: &MediaEndpoint, prompt: &str, extra: &HashMap<String, String>) -> Value {
    let mut body = json!({
        "model": ep.model,
        "prompt": prompt,
        "n": 1,
        "size": "1024x1024",
        "response_format": "b64_json",
    });
    let merge = |body: &mut Value, src: &Value| {
        if let (Some(dst), Some(from)) = (body.as_object_mut(), src.as_object()) {
            for (k, v) in from {
                dst.insert(k.clone(), v.clone());
            }
        }
    };
    if let Some(p) = &ep.params {
        merge(&mut body, p);
    }
    // 工具显式参数：只接受白名单键，避免模型往请求体里塞任意字段。
    for key in ["size", "n", "quality", "style", "response_format", "negative_prompt"] {
        if let Some(v) = extra.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()) {
            body[key] = if key == "n" {
                v.parse::<u32>().map(Value::from).unwrap_or(json!(1))
            } else {
                json!(v)
            };
        }
    }
    body
}

/// `image_generate` 工具。
pub struct ImageGenerateTool {
    workspace: RootSet,
    transport: Arc<dyn ImageTransport>,
}

impl ImageGenerateTool {
    pub fn new(workspace: impl Into<RootSet>, transport: Arc<dyn ImageTransport>) -> Self {
        let workspace: RootSet = workspace.into();
        Self {
            workspace,
            transport,
        }
    }

    pub fn with_http(workspace: impl Into<RootSet>) -> Self {
        Self::new(workspace, Arc::new(HttpImageTransport))
    }
}

#[async_trait]
impl ToolExecutor for ImageGenerateTool {
    fn name(&self) -> &str {
        "image_generate"
    }

    fn description(&self) -> &str {
        "文本生成图片（多模态·图像输出）。使用当前档案声明的图像模型，\
         调用 OpenAI 兼容的 images/generations 端点，把结果保存到工作区并登记为产物。\
         prompt 必填；size/n/quality/style 可选；out 可指定文件名。\
         未声明「图像输出」能力时会报错并给出配置指引（不会假装生成）。"
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "prompt":{"type":"string","description":"图片描述（越具体越好：主体/风格/构图/光线/画幅）"},
            "size":{"type":"string","description":"尺寸，如 1024x1024 / 1536x1024 / 1024x1536"},
            "n":{"type":"number","description":"生成张数（默认 1，最大值由上游决定）"},
            "quality":{"type":"string","description":"质量档（如 standard / hd / high）"},
            "style":{"type":"string","description":"风格（如 vivid / natural）"},
            "negative_prompt":{"type":"string","description":"反向提示词（部分服务商支持）"},
            "out":{"type":"string","description":"输出文件名或相对路径（默认 generated/images/<时间戳>.png）"}
        },"required":["prompt"]})
    }

    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let prompt = args
            .get("prompt")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if prompt.is_empty() {
            return Ok(ToolResult::Error {
                message: "image_generate 需要非空的 prompt".to_string(),
                code: "INVALID_PARAMS".to_string(),
            });
        }
        let ep = match media_capability::resolve(MediaKind::Image) {
            Ok(ep) => ep,
            Err(e) => {
                return Ok(ToolResult::Error {
                    message: e,
                    code: "NO_IMAGE_CAPABILITY".to_string(),
                })
            }
        };

        let body = build_body(&ep, &prompt, args);
        let resp = match self.transport.generate(&ep, &body).await {
            Ok(v) => v,
            Err(e) => {
                return Ok(ToolResult::Error {
                    message: format!(
                        "图像生成失败（档案 {}，端点 {}）：{e}",
                        ep.profile_id, ep.url
                    ),
                    code: "IMAGE_GENERATE_FAILED".to_string(),
                })
            }
        };

        let (b64, url) = match first_image(&resp) {
            Ok(v) => v,
            Err(e) => {
                return Ok(ToolResult::Error {
                    message: e,
                    code: "IMAGE_GENERATE_BAD_RESPONSE".to_string(),
                })
            }
        };

        let bytes = if let Some(b64) = b64 {
            match base64::engine::general_purpose::STANDARD.decode(b64.trim()) {
                Ok(b) => b,
                Err(e) => {
                    return Ok(ToolResult::Error {
                        message: format!("上游 b64_json 解码失败: {e}"),
                        code: "IMAGE_GENERATE_BAD_RESPONSE".to_string(),
                    })
                }
            }
        } else {
            match self.transport.download(url.as_deref().unwrap_or_default()).await {
                Ok(b) => b,
                Err(e) => {
                    // 下载失败也要如实回传 URL，让用户至少能手动取图。
                    return Ok(ToolResult::Error {
                        message: format!("图片已生成但下载失败（{e}）；上游 URL：{url:?}"),
                        code: "IMAGE_DOWNLOAD_FAILED".to_string(),
                    });
                }
            }
        };

        let rel = args
            .get("out")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                format!(
                    "generated/images/{}.{}",
                    chrono::Local::now().format("%Y%m%d-%H%M%S"),
                    sniff_ext(&bytes)
                )
            });
        let target = match self.workspace.resolve(&rel) {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult::Error {
                    message: format!("输出路径非法（必须落在工作区内）: {e}"),
                    code: "INVALID_PATH".to_string(),
                })
            }
        };
        if let Some(dir) = target.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                return Ok(ToolResult::Error {
                    message: format!("创建输出目录失败: {e}"),
                    code: "IO_ERROR".to_string(),
                });
            }
        }
        if let Err(e) = std::fs::write(&target, &bytes) {
            return Ok(ToolResult::Error {
                message: format!("写入图片失败: {e}"),
                code: "IO_ERROR".to_string(),
            });
        }
        record_artifact("", &target.display().to_string(), GENERATED_KIND);

        Ok(ToolResult::Success {
            data: json!({
                "ok": true,
                "path": target.display().to_string(),
                "bytes": bytes.len(),
                "profile": ep.profile_id,
                "model": ep.model,
                "endpoint": ep.url,
                "prompt": prompt,
            })
            .to_string(),
            metadata: HashMap::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep() -> MediaEndpoint {
        MediaEndpoint {
            profile_id: "p".into(),
            provider: "openai".into(),
            model: "gpt-image-1".into(),
            url: "https://x/v1/images/generations".into(),
            api_key: "k".into(),
            params: None,
        }
    }

    #[test]
    fn body_defaults_and_whitelist() {
        let mut extra = HashMap::new();
        extra.insert("size".to_string(), "1536x1024".to_string());
        extra.insert("evil".to_string(), "x".to_string());
        let b = build_body(&ep(), "a cat", &extra);
        assert_eq!(b["size"], "1536x1024");
        assert_eq!(b["n"], 1);
        assert_eq!(b["model"], "gpt-image-1");
        assert!(b.get("evil").is_none(), "非白名单键不得进入请求体");
    }

    #[test]
    fn profile_params_override_defaults() {
        let mut e = ep();
        e.params = Some(json!({"size":"512x512","seed":7}));
        let b = build_body(&e, "x", &HashMap::new());
        assert_eq!(b["size"], "512x512");
        assert_eq!(b["seed"], 7);
    }

    #[test]
    fn tool_args_beat_profile_params() {
        let mut e = ep();
        e.params = Some(json!({"size":"512x512"}));
        let mut extra = HashMap::new();
        extra.insert("size".to_string(), "1024x1024".to_string());
        let b = build_body(&e, "x", &extra);
        assert_eq!(b["size"], "1024x1024");
    }

    #[test]
    fn first_image_prefers_b64_then_url() {
        let v = json!({"data":[{"b64_json":"AAA"}]});
        assert_eq!(first_image(&v).unwrap().0.as_deref(), Some("AAA"));
        let v = json!({"data":[{"url":"https://x/a.png"}]});
        assert_eq!(first_image(&v).unwrap().1.as_deref(), Some("https://x/a.png"));
        assert!(first_image(&json!({"data":[]})).is_err());
        assert!(first_image(&json!({})).is_err());
    }

    #[test]
    fn sniffs_extensions() {
        assert_eq!(sniff_ext(b"\x89PNG\r\n\x1a\n"), "png");
        assert_eq!(sniff_ext(&[0xFF, 0xD8, 0x00]), "jpg");
        assert_eq!(sniff_ext(b"RIFF0000WEBP"), "webp");
    }
}
