//! 图片元数据解析（零依赖，手工解析常见格式头）。
//!
//! 支持 PNG / JPEG / GIF / BMP / WebP（VP8X）；尺寸缺失字段为 None。
//! 对齐 nexustar-api `vision.image_metadata`。

/// 图片元数据
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageMeta {
    /// 格式名（unknown 表示无法识别）
    pub format: &'static str,
    /// MIME 类型
    pub mime: &'static str,
    /// 字节数
    pub bytes: usize,
    /// 宽度（无法识别为 None）
    pub width: Option<u32>,
    /// 高度（无法识别为 None）
    pub height: Option<u32>,
}

// JPEG SOF 标记集合（各类帧起始）
const JPEG_SOF_MARKERS: &[u8] = &[
    0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF,
];

/// 解析图片格式 / 尺寸 / 字节数；无法识别时 format=unknown。
pub fn image_metadata(data: &[u8]) -> ImageMeta {
    let mut meta = ImageMeta {
        format: "unknown",
        mime: "application/octet-stream",
        bytes: data.len(),
        width: None,
        height: None,
    };

    if data.len() >= 24 && &data[0..8] == b"\x89PNG\r\n\x1a\n" {
        meta.format = "png";
        meta.mime = "image/png";
        meta.width = Some(u32::from_be_bytes([data[16], data[17], data[18], data[19]]));
        meta.height = Some(u32::from_be_bytes([data[20], data[21], data[22], data[23]]));
    } else if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xD8 {
        meta.format = "jpeg";
        meta.mime = "image/jpeg";
        // 顺序扫描段标记，定位 SOF 取宽高
        let mut i = 2;
        while i + 9 <= data.len() {
            if data[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = data[i + 1];
            if JPEG_SOF_MARKERS.contains(&marker) {
                // height 在 marker+5..+7，width 在 +7..+9
                meta.height = Some(u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32);
                meta.width = Some(u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32);
                break;
            }
            if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
                i += 2;
                continue;
            }
            if i + 4 > data.len() {
                break;
            }
            let seg_len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
            i += 2 + seg_len;
        }
    } else if data.len() >= 10 && (&data[0..6] == b"GIF87a" || &data[0..6] == b"GIF89a") {
        meta.format = "gif";
        meta.mime = "image/gif";
        meta.width = Some(u16::from_le_bytes([data[6], data[7]]) as u32);
        meta.height = Some(u16::from_le_bytes([data[8], data[9]]) as u32);
    } else if data.len() >= 26 && &data[0..2] == b"BM" {
        meta.format = "bmp";
        meta.mime = "image/bmp";
        let w = i32::from_le_bytes([data[18], data[19], data[20], data[21]]);
        let h = i32::from_le_bytes([data[22], data[23], data[24], data[25]]);
        meta.width = Some(w as u32);
        meta.height = Some(h.unsigned_abs());
    } else if data.len() >= 30 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        meta.format = "webp";
        meta.mime = "image/webp";
        if &data[12..16] == b"VP8X" {
            let w = u32::from_le_bytes([data[24], data[25], data[26], 0]) + 1;
            let h = u32::from_le_bytes([data[27], data[28], data[29], 0]) + 1;
            meta.width = Some(w);
            meta.height = Some(h);
        }
    }
    meta
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&[0, 0, 0, 13]); // IHDR length
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0]); // bit depth/color type
        v
    }

    fn jpeg(h: u16, w: u16) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&[3, 1, 0, 1, 1, 1, 0]); // 3 components
        v
    }

    fn gif(w: u16, h: u16) -> Vec<u8> {
        let mut v = b"GIF89a".to_vec();
        v.extend_from_slice(&w.to_le_bytes());
        v.extend_from_slice(&h.to_le_bytes());
        v.extend_from_slice(&[0; 4]);
        v
    }

    fn bmp(w: i32, h: i32) -> Vec<u8> {
        let mut v = b"BM".to_vec();
        v.extend_from_slice(&[0; 16]);
        v.extend_from_slice(&w.to_le_bytes());
        v.extend_from_slice(&h.to_le_bytes());
        v
    }

    fn webp(w: u32, h: u32) -> Vec<u8> {
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(b"WEBP");
        v.extend_from_slice(b"VP8X");
        v.extend_from_slice(&[0; 8]);
        let w = (w.saturating_sub(1)).to_le_bytes();
        let h = (h.saturating_sub(1)).to_le_bytes();
        v.extend_from_slice(&w[0..3]);
        v.extend_from_slice(&h[0..3]);
        v
    }

    #[test]
    fn png_metadata() {
        let m = image_metadata(&png(64, 48));
        assert_eq!(m.format, "png");
        assert_eq!(m.width, Some(64));
        assert_eq!(m.height, Some(48));
        assert_eq!(m.mime, "image/png");
    }

    #[test]
    fn jpeg_metadata() {
        let m = image_metadata(&jpeg(100, 200));
        assert_eq!(m.format, "jpeg");
        assert_eq!(m.height, Some(100));
        assert_eq!(m.width, Some(200));
    }

    #[test]
    fn gif_metadata() {
        let m = image_metadata(&gif(32, 16));
        assert_eq!(m.format, "gif");
        assert_eq!(m.width, Some(32));
        assert_eq!(m.height, Some(16));
    }

    #[test]
    fn bmp_metadata() {
        let m = image_metadata(&bmp(128, 64));
        assert_eq!(m.format, "bmp");
        assert_eq!(m.width, Some(128));
        assert_eq!(m.height, Some(64));
    }

    #[test]
    fn webp_metadata() {
        let m = image_metadata(&webp(300, 150));
        assert_eq!(m.format, "webp");
        assert_eq!(m.width, Some(300));
        assert_eq!(m.height, Some(150));
    }

    #[test]
    fn unknown_format() {
        let m = image_metadata(b"not an image");
        assert_eq!(m.format, "unknown");
        assert_eq!(m.width, None);
    }
}
