//! 图片二进制 → HTML 兜底方案（无多模态能力时 fallback）。
//!
//! 实现用户要求："图片的二进制编码转换成deepseek能看得见、读得懂的html代码"。
//! 当活跃模型不具备原生视觉能力时，将图片二进制打包为可被模型理解的 HTML：
//! - 图片元数据（格式、尺寸、字节大小）
//! - base64 编码嵌入为 `<img>` 标签，模型能在上下文里"看到"图片内容
//! - 提供提示词引导模型理解/描述图片
//!
//! 此方案零依赖（不要求能解码像素），纯 Rust，安全降级不影响既有行为。
//! 原生多模态可用时优先走原生视觉接口，此路径为 fallback。

use crate::vision::metadata::{image_metadata, ImageMeta};

/// 将图片字节转换成供模型阅读的完整 HTML 字符串（base64 嵌入 + 元信息 + 提示词）。
///
/// 用法：当模型无原生视觉能力时，用返回的 HTML 作为提示词一部分发给模型，
/// 使模型能通过 HTML 感知图片存在并描述内容。
pub fn binary_to_html(data: &[u8]) -> String {
    let meta = image_metadata(data);
    let b64 = base64_encode(data);
    let mime = &meta.mime;

    let dims = match (meta.width, meta.height) {
        (Some(w), Some(h)) => format!("{} × {}", w, h),
        _ => "未知".to_string(),
    };

    format!(
        "<!DOCTYPE html>\n\
         <html>\n\
         <head><meta charset=\"utf-8\"><title>图片</title></head>\n\
         <body>\n\
         <!--\n\
         图片元信息：\n\
         \u{20} 格式: {format}\n\
         \u{20} 尺寸: {dims}\n\
         \u{20} 字节数: {bytes}\n\
         \u{20} MIME: {mime}\n\
         -->\n\
         <h3>图片内容</h3>\n\
         <p>下方是嵌入的图片：</p>\n\
         <img src=\"data:{mime};base64,{b64}\" alt=\"用户上传图片\">\n\
         <p>请描述这张图片的内容。</p>\n\
         </body>\n\
         </html>\n",
        format = meta.format,
        dims = dims,
        bytes = meta.bytes,
        mime = mime,
        b64 = b64,
    )
}

/// 将图片字节转换成简化的提示词描述（不含完整 HTML，适合放在 prompt 里）。
pub fn binary_to_img_tag(data: &[u8]) -> String {
    let meta = image_metadata(data);
    let b64 = base64_encode(data);
    let mime = &meta.mime;

    let dims = match (meta.width, meta.height) {
        (Some(w), Some(h)) => format!("{}×{}", w, h),
        _ => String::new(),
    };

    format!(
        "<!-- 图片：格式={}, 大小={}字节 {} -->\n<img src=\"data:{mime};base64,{b64}\" alt=\"图片\">",
        meta.format, meta.bytes, dims
    )
}

/// 得到纯提示词（不含 base64，只有元信息）用于模型引导。
///
/// 当图片过大且无法嵌入时，只发元数据 + 请求模型基于上下文推断。
pub fn image_metadata_to_prompt(meta: &ImageMeta) -> String {
    let dims = match (meta.width, meta.height) {
        (Some(w), Some(h)) => format!("{} × {}", w, h),
        _ => "未知尺寸".to_string(),
    };
    format!(
        "这是一张图片。格式：{}，尺寸：{}，大小：{} 字节。\n\
         请根据我提供的上下文描述这张图片。",
        meta.format, dims, meta.bytes
    )
}

/// 最小 base64 标准字母表编码（RFC 4648，零依赖）。
fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        if chunk.len() >= 2 {
            out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
        }
        if chunk.len() >= 3 {
            out.push(ALPHABET[(n & 0x3F) as usize] as char);
        }
    }
    out
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

    #[test]
    fn binary_to_html_contains_meta_and_img() {
        let html = binary_to_html(&png());
        assert!(html.contains("png"), "应包含格式名称");
        assert!(html.contains("64 × 48"), "应包含尺寸");
        assert!(html.contains("<img"), "应包含 img 标签");
        assert!(
            html.contains("data:image/png;base64,"),
            "应包含正确 data URI"
        );
        assert!(html.len() > 100);
    }

    #[test]
    fn binary_to_img_tag_produces_compact_output() {
        let tag = binary_to_img_tag(&png());
        assert!(tag.starts_with("<!--"));
        assert!(tag.contains("<img src=\"data:image/png;base64,"));
        assert!(tag.len() < 500, "tag 应紧凑");
    }

    #[test]
    fn base64_encoding_produces_valid_prefix() {
        let b64 = base64_encode(b"hello world");
        assert!(b64.starts_with("aGVsbG8"), "base64 编码前缀正确");
    }

    #[test]
    fn metadata_to_prompt_contains_fields() {
        let meta = image_metadata(&png());
        let p = image_metadata_to_prompt(&meta);
        assert!(p.contains("png"), "提示词含格式");
        assert!(p.contains("字节"), "提示词含大小");
    }

    #[test]
    fn unknown_format_still_produces_output() {
        let html = binary_to_html(b"\x00\x01\x02\x03");
        assert!(html.contains("unknown"), "未知格式仍输出");
        assert!(html.len() > 50);
    }
}
