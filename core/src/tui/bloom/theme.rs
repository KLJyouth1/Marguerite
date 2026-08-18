//! Bloom 品牌色板（终端原生，去 WebGL）
//!
//! Bloom 视觉身份 = 活体青/teal 辉光（非紫粉渐变 AI 模板套路）。
//! 严格遵循工程标准：仅经常量引用，禁止硬编码色值（除 `#fff`/`#000` 例外外），
//! 禁止 Indigo→Pink 渐变 + 发光边框 + 毛玻璃三位一体套路。
//! 粒子溶解隐喻用 Unicode 块字符 + 辉光梯度表达。

use ratatui::style::Color;

/// Bloom 主题（青/teal 辉光）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BloomTheme {
    /// 深底（近黑带青调）
    pub bg: Color,
    /// 面板底
    pub panel: Color,
    /// 卡片底
    pub card: Color,
    /// 边框
    pub border: Color,
    /// 主文本
    pub text_primary: Color,
    /// 次文本
    pub text_secondary: Color,
    /// 弱文本
    pub text_muted: Color,
    /// Bloom 品牌青
    pub bloom: Color,
    /// Bloom 辉光亮
    pub bloom_light: Color,
    /// Bloom 辉光暗
    pub bloom_dark: Color,
    /// 成功绿
    pub success: Color,
    /// 告警橙
    pub warning: Color,
    /// 错误红
    pub danger: Color,
}

impl BloomTheme {
    /// 默认 Bloom 主题
    pub const DEFAULT: BloomTheme = BloomTheme {
        bg: Color::Rgb(4, 10, 12),
        panel: Color::Rgb(9, 20, 24),
        card: Color::Rgb(14, 32, 38),
        border: Color::Rgb(22, 54, 64),
        text_primary: Color::Rgb(220, 240, 240),
        text_secondary: Color::Rgb(150, 178, 182),
        text_muted: Color::Rgb(86, 112, 116),
        bloom: Color::Rgb(34, 200, 200),
        bloom_light: Color::Rgb(120, 240, 230),
        bloom_dark: Color::Rgb(16, 110, 120),
        success: Color::Rgb(72, 214, 142),
        warning: Color::Rgb(255, 150, 70),
        danger: Color::Rgb(248, 113, 113),
    };

    /// 加载（当前无文件覆盖，直接回退默认）
    pub fn load() -> BloomTheme {
        BloomTheme::DEFAULT
    }

    /// 辉光梯度：按强度(0..1)从暗青 → 亮青。纯色阶跃，无渐变字符串。
    pub fn glow(intensity: f32) -> Color {
        let t = intensity.clamp(0.0, 1.0);
        if t < 0.25 {
            Color::Rgb(13, 60, 70)
        } else if t < 0.5 {
            Color::Rgb(20, 130, 140)
        } else if t < 0.75 {
            Color::Rgb(34, 200, 200)
        } else {
            Color::Rgb(120, 240, 230)
        }
    }

    /// 粒子溶解字符：按密度(0..1)从空 → 实心块。
    /// Unicode 几何块（非 emoji），表达 Bloom 粒子浓度。
    pub fn particle_glyph(density: f32) -> &'static str {
        let t = density.clamp(0.0, 1.0);
        if t < 0.08 {
            " "
        } else if t < 0.3 {
            "░"
        } else if t < 0.55 {
            "▒"
        } else if t < 0.8 {
            "▓"
        } else {
            "█"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bloom_palette_is_cyan_not_purple() {
        let t = BloomTheme::DEFAULT;
        // 品牌青应是青绿色系（G、B 高，R 低），而非紫粉（R、B 高 G 低）
        if let (Color::Rgb(r, g, b), Color::Rgb(br, bg, bb)) = (t.bloom, t.bloom_light) {
            assert!(g > r && b > r, "Bloom 应为青绿，实际 R{} G{} B{}", r, g, b);
            assert!(bg >= br && bb >= br, "辉光亮应比暗更亮");
        } else {
            panic!("Bloom 颜色应为 Rgb");
        }
    }

    #[test]
    fn test_glow_ramp_monotonic_brightness() {
        // 亮度随强度不减（粗略按 G+B 之和比较）
        let lum = |c: Color| -> i32 {
            if let Color::Rgb(r, g, b) = c {
                (g as i32) + (b as i32) - (r as i32)
            } else {
                0
            }
        };
        assert!(lum(BloomTheme::glow(0.9)) >= lum(BloomTheme::glow(0.1)));
        assert!(lum(BloomTheme::glow(0.5)) >= lum(BloomTheme::glow(0.0)));
    }

    #[test]
    fn test_particle_glyph_levels() {
        assert_eq!(BloomTheme::particle_glyph(0.0), " ");
        assert_eq!(BloomTheme::particle_glyph(0.1), "░");
        assert_eq!(BloomTheme::particle_glyph(0.4), "▒");
        assert_eq!(BloomTheme::particle_glyph(0.7), "▓");
        assert_eq!(BloomTheme::particle_glyph(1.0), "█");
    }

    #[test]
    fn test_load_returns_default() {
        assert_eq!(BloomTheme::load(), BloomTheme::DEFAULT);
    }
}
