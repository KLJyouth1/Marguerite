//! 像素差异（Track B4，纯逻辑、零图像解码依赖）。
//!
//! 在**已解码**的 RGBA8 行优先缓冲上计算两图差异，本身不做任何解码：
//! 解码（PNG/JPEG/…）→ 内存缓冲 的环节留给后续切片，与本模块边界一致
//! （同 `vision_gate` / `grounding` 已定的「不在此处做 IO」契约）。
//!
//! 计算模型：逐像素比较 RGBA 四通道差绝对值之和，超过 [`CHANNEL_DIFF_SUM_THRESHOLD`]
//! 视为变化像素；把所有变化像素的外接矩形聚合成单个 [`DiffRegion`]
//! （MVP 最简实现：全图变化像素的最小外接矩形）。

/// 单处变化区域：变化像素的最小外接矩形 + 变化像素占比。
#[derive(Debug, Clone, PartialEq)]
pub struct DiffRegion {
    /// 归一化包围盒 `[x_min, y_min, x_max, y_max]`，坐标域 [0,1]×[0,1]。
    pub bbox: [f64; 4],
    /// 该区域变化像素数 / 总像素数 ∈ [0,1]。
    pub changed_ratio: f64,
}

/// 判定某像素为「变化」的阈值：四通道差绝对值之和超过此值即算变化。
///
/// 取 30 以忽略极轻微的压缩/量化噪声（如单通道 ±1 的舍入不会触发），
/// 同时保证明显的像素改动（单像素全通道翻转差和 ≈ 765）必然被检出。
const CHANNEL_DIFF_SUM_THRESHOLD: u32 = 30;

/// 计算两幅 RGBA8 缓冲的像素差异。
///
/// # 参数
/// - `a` / `b`：RGBA8 行优先缓冲，长度须恰为 `width * height * 4`。
/// - `width` / `height`：图像宽高（像素）。
///
/// # 返回
/// 变化像素的最小外接矩形聚合为单个 [`DiffRegion`]；无变化返回空 `Vec`。
///
/// # 边界（不 panic）
/// - `a` / `b` 长度与 `width * height * 4` 不符 → 返回空 `Vec`（调用方据需记 reason）。
/// - `width * height == 0`（任一为 0）→ 返回空 `Vec`。
pub fn pixel_diff(a: &[u8], b: &[u8], width: u32, height: u32) -> Vec<DiffRegion> {
    let w = width as usize;
    let h = height as usize;

    // 边界：尺寸为 0 或缓冲长度不符 → 直接返回空（不 panic）。
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let expected = w * h * 4;
    if a.len() != expected || b.len() != expected {
        return Vec::new();
    }

    let mut changed: usize = 0;
    let mut min_x: usize = w;
    let mut min_y: usize = h;
    let mut max_x: usize = 0;
    let mut max_y: usize = 0;

    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) * 4;
            let mut diff_sum: u32 = 0;
            for c in 0..4 {
                let av = a[idx + c] as i32;
                let bv = b[idx + c] as i32;
                diff_sum += (av - bv).abs() as u32;
            }
            if diff_sum > CHANNEL_DIFF_SUM_THRESHOLD {
                changed += 1;
                if x < min_x {
                    min_x = x;
                }
                if y < min_y {
                    min_y = y;
                }
                if x > max_x {
                    max_x = x;
                }
                if y > max_y {
                    max_y = y;
                }
            }
        }
    }

    // 无任何变化像素 → 空 Vec（正确反映「两图一致」）。
    if changed == 0 {
        return Vec::new();
    }

    let total = w * h;
    let changed_ratio = changed as f64 / total as f64;

    // 变化像素最小外接矩形，归一化到 [0,1]（像素格右/下边界取 x+1 以完整覆盖该格）。
    let bbox = [
        min_x as f64 / w as f64,
        min_y as f64 / h as f64,
        (max_x + 1) as f64 / w as f64,
        (max_y + 1) as f64 / h as f64,
    ];

    vec![DiffRegion {
        bbox,
        changed_ratio,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生成 `w*h` 的纯色 RGBA8 缓冲（默认全透明黑，alpha=255 便于 diff 稳定）。
    fn solid(w: u32, h: u32, r: u8, g: u8, b: u8, a: u8) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            v.extend_from_slice(&[r, g, b, a]);
        }
        v
    }

    #[test]
    fn identical_buffers_return_empty() {
        let buf = solid(8, 8, 10, 20, 30, 255);
        let out = pixel_diff(&buf, &buf, 8, 8);
        assert!(out.is_empty(), "完全相同的缓冲应返回空 Vec");
    }

    #[test]
    fn single_pixel_change_yields_one_region_tiny_ratio() {
        let a = solid(8, 8, 10, 20, 30, 255);
        let mut b = a.clone();
        // 翻转单个像素（第 0 行第 3 列）全部通道 → 差和 ≈ 765 > 阈值。
        let idx = 3 * 4; // 第 0 行第 3 列的 RGBA 起始索引
        b[idx..idx + 4].copy_from_slice(&[200, 200, 200, 255]);
        let out = pixel_diff(&a, &b, 8, 8);
        assert_eq!(out.len(), 1, "单像素变化 → 恰好 1 个 DiffRegion");
        let region = &out[0];
        // 该区域应覆盖第 0 行第 3 列这一格：x∈[3/8,(3+1)/8], y∈[0,1/8]。
        assert!(
            (region.bbox[0] - 3.0 / 8.0).abs() < 1e-9,
            "bbox x_min 应定位到变化像素列"
        );
        assert!(
            (region.bbox[2] - 4.0 / 8.0).abs() < 1e-9,
            "bbox x_max 应覆盖变化像素列"
        );
        assert!(
            (region.bbox[1] - 0.0).abs() < 1e-9 && (region.bbox[3] - 1.0 / 8.0).abs() < 1e-9,
            "bbox y 应覆盖变化像素行"
        );
        // 1/64 ≈ 0.0156，极小但非 0。
        assert!(
            region.changed_ratio > 0.0 && region.changed_ratio < 0.1,
            "changed_ratio 应极小（单像素 / 64）"
        );
    }

    #[test]
    fn fully_different_yields_whole_image_ratio_one() {
        let a = solid(4, 4, 0, 0, 0, 255);
        let b = solid(4, 4, 255, 255, 255, 255);
        let out = pixel_diff(&a, &b, 4, 4);
        assert_eq!(out.len(), 1, "全不同 → 1 个覆盖全图的 DiffRegion");
        let region = &out[0];
        // 覆盖全图：[0,0,1,1]。容差处理浮点边界。
        assert!(
            region.bbox[0].abs() < 1e-9
                && region.bbox[1].abs() < 1e-9
                && (region.bbox[2] - 1.0).abs() < 1e-9
                && (region.bbox[3] - 1.0).abs() < 1e-9,
            "bbox 应覆盖全图 [0,0,1,1]"
        );
        assert!(
            (region.changed_ratio - 1.0).abs() < 1e-9,
            "changed_ratio 应 ≈ 1.0"
        );
    }

    #[test]
    fn mismatched_length_returns_empty() {
        let a = solid(4, 4, 0, 0, 0, 255);
        let b = solid(2, 2, 255, 255, 255, 255); // 长度不符
        let out = pixel_diff(&a, &b, 4, 4);
        assert!(out.is_empty(), "长度不符应返回空 Vec（不 panic）");
    }

    #[test]
    fn zero_dimensions_return_empty() {
        let buf = solid(4, 4, 0, 0, 0, 255);
        assert!(pixel_diff(&buf, &buf, 0, 4).is_empty(), "width=0 → 空");
        assert!(pixel_diff(&buf, &buf, 4, 0).is_empty(), "height=0 → 空");
        assert!(pixel_diff(&[], &[], 0, 0).is_empty(), "全 0 → 空");
    }
}
