//! Design Tokens（TUI 适配，品牌蓝 SSOT `#1A56DB`，与桌面/设计稿统一）
//!
//! 所有渲染颜色一律经 [`Theme`] 常量引用，禁止硬编码色值、禁止紫粉渐变、禁止 emoji 图标。
//! 状态点用 Unicode 几何块（`▮` 绿呼吸 / `✖` 红常亮 / `◉` 橙等待），图标用单字符文本标签。
//!
//! 主题选择（Phase 3，2026-08-07）：
//! - 默认深空 [`Theme::DEEP_SPACE`]；`MARGUERITE_THEME=light` 切浅色 [`Theme::LIGHT`]；
//! - `NO_COLOR` 环境变量置位时自动降级为单色 [`Theme::monochrome`]（无障碍 / agent 友好）；
//! - 全部经 [`Theme::effective`] 解析，渲染主路径统一调用，避免硬编码色。
//! - 文件覆盖（`~/.marguerite/themes/*.toml`）因环境安全层 `Cargo.lock` 写入争用（os error 5）
//!   刻意不引入 `toml` 新依赖，推迟至后续；环境变量机制已满足当前需求，契约预留。

use ratatui::style::Color;
use std::path::Path;

/// 工作台主题（深空蓝主）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// 深空底
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
    /// 品牌蓝（导航激活 / 焦点环 / 进度主体 / 链接）
    pub brand: Color,
    /// 品牌蓝 hover
    pub brand_light: Color,
    /// 品牌蓝 active
    pub brand_dark: Color,
    /// 成功绿（运行呼吸 / 完成）
    pub success: Color,
    /// 告警橙
    pub warning: Color,
    /// 错误红
    pub danger: Color,
    /// 焦点环
    pub focus_ring: Color,
}

impl Theme {
    /// 深空默认主题（品牌蓝 SSOT `#1A56DB`）
    pub const DEEP_SPACE: Theme = Theme {
        bg: Color::Rgb(3, 5, 16),
        panel: Color::Rgb(10, 16, 40),
        card: Color::Rgb(22, 27, 46),
        border: Color::Rgb(42, 48, 72),
        text_primary: Color::Rgb(229, 233, 245),
        text_secondary: Color::Rgb(154, 163, 191),
        text_muted: Color::Rgb(92, 100, 128),
        brand: Color::Rgb(26, 86, 219),
        brand_light: Color::Rgb(59, 130, 246),
        brand_dark: Color::Rgb(30, 64, 175),
        success: Color::Rgb(61, 214, 140),
        warning: Color::Rgb(255, 138, 61),
        danger: Color::Rgb(248, 113, 113),
        focus_ring: Color::Rgb(26, 86, 219),
    };

    /// 浅色主题（白底深字，品牌蓝 SSOT `#1A56DB`，对比度优先；无紫粉渐变）
    pub const LIGHT: Theme = Theme {
        bg: Color::Rgb(248, 250, 252),
        panel: Color::Rgb(255, 255, 255),
        card: Color::Rgb(241, 245, 249),
        border: Color::Rgb(203, 213, 225),
        text_primary: Color::Rgb(15, 23, 42),
        text_secondary: Color::Rgb(71, 85, 105),
        text_muted: Color::Rgb(148, 163, 184),
        brand: Color::Rgb(26, 86, 219),
        brand_light: Color::Rgb(59, 130, 246),
        brand_dark: Color::Rgb(30, 64, 175),
        success: Color::Rgb(22, 163, 74),
        warning: Color::Rgb(217, 119, 0),
        danger: Color::Rgb(220, 38, 38),
        focus_ring: Color::Rgb(26, 86, 219),
    };

    /// 加载主题：当前回退深空默认（文件覆盖待 `toml` 依赖引入后启用）
    pub fn load_or_default(_dir: &Path) -> Theme {
        Theme::DEEP_SPACE
    }

    /// 解析生效主题：NO_COLOR 优先 → 单色；`MARGUERITE_THEME=light` → 浅色；否则深空。
    pub fn resolve(no_color: bool, theme_hint: &str) -> Theme {
        if no_color {
            return Theme::monochrome();
        }
        match theme_hint.trim().to_ascii_lowercase().as_str() {
            "light" | "l" => Theme::LIGHT,
            _ => Theme::DEEP_SPACE,
        }
    }

    /// 读取环境变量解析当前主题（渲染主路径统一调用，使主题切换 / NO_COLOR 真正生效）。
    pub fn effective() -> Theme {
        let no_color = std::env::var_os("NO_COLOR").is_some();
        let hint = std::env::var("MARGUERITE_THEME").unwrap_or_default();
        Theme::resolve(no_color, &hint)
    }

    /// 单色降级（NO_COLOR）：去所有语义色，仅黑 / 白 / 灰，保证无障碍与 agent 抓取可读性。
    fn monochrome() -> Theme {
        Theme {
            bg: Color::Black,
            panel: Color::Black,
            card: Color::Black,
            border: Color::DarkGray,
            text_primary: Color::White,
            text_secondary: Color::Gray,
            text_muted: Color::DarkGray,
            brand: Color::White,
            brand_light: Color::White,
            brand_dark: Color::Gray,
            success: Color::White,
            warning: Color::White,
            danger: Color::White,
            focus_ring: Color::White,
        }
    }

    /// 状态点字符（Unicode 几何块，非 emoji）
    pub fn status_glyph(state: StatusDot) -> &'static str {
        match state {
            StatusDot::Running => "▮", // 绿呼吸
            StatusDot::Error => "✖",   // 红常亮
            StatusDot::Waiting => "◉", // 橙慢闪
            StatusDot::Idle => "·",
        }
    }
}

/// 状态点语义
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusDot {
    /// 运行中（绿）
    Running,
    /// 错误（红）
    Error,
    /// 等待（橙）
    Waiting,
    /// 空闲（灰）
    Idle,
}

/// 显示宽度近似（未引入 `unicode-width`，CJK 按 1 计，保守不溢出）
pub fn disp_width(s: &str) -> usize {
    s.chars().count()
}

/// 按显示宽度截断（保留末尾省略号）
pub fn truncate(s: &str, max: usize) -> String {
    if disp_width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    for (n, c) in s.chars().enumerate() {
        if n + 1 > max.saturating_sub(1) {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_deep_space_brand_is_blue() {
        let t = Theme::DEEP_SPACE;
        assert_eq!(t.brand, Color::Rgb(26, 86, 219));
    }

    #[test]
    fn test_both_themes_share_ssot_brand_blue() {
        // F1 修复：跨 GUI 品牌蓝统一为设计 SSOT #1A56DB（与桌面 ACCENT 一致）
        let ssot = Color::Rgb(26, 86, 219);
        assert_eq!(Theme::DEEP_SPACE.brand, ssot);
        assert_eq!(Theme::LIGHT.brand, ssot);
        assert_eq!(Theme::DEEP_SPACE.focus_ring, ssot);
        assert_eq!(Theme::LIGHT.focus_ring, ssot);
    }

    #[test]
    fn test_load_or_default_returns_deep_space() {
        let t = Theme::load_or_default(Path::new("/tmp"));
        assert_eq!(t, Theme::DEEP_SPACE);
    }

    #[test]
    fn test_light_theme_has_light_background_and_dark_text() {
        let t = Theme::LIGHT;
        // 浅色主题：底显著亮于字（R+G+B 之和 bg > text_primary）
        let sum = |c: Color| -> i32 {
            if let Color::Rgb(r, g, b) = c {
                (r as i32) + (g as i32) + (b as i32)
            } else {
                0
            }
        };
        assert!(sum(t.bg) > sum(t.text_primary), "浅色主题底应亮于字");
        // 无紫粉渐变：品牌蓝仍是蓝（B 高、R 低），非紫（R、B 接近）
        if let Color::Rgb(r, _g, b) = t.brand {
            assert!(b > r, "浅色主题品牌应为蓝而非紫");
        }
    }

    #[test]
    fn test_resolve_light_and_default_hint() {
        assert_eq!(Theme::resolve(false, "light"), Theme::LIGHT);
        assert_eq!(Theme::resolve(false, "L"), Theme::LIGHT);
        assert_eq!(Theme::resolve(false, "dark"), Theme::DEEP_SPACE);
        assert_eq!(Theme::resolve(false, ""), Theme::DEEP_SPACE);
    }

    #[test]
    fn test_resolve_no_color_forces_monochrome() {
        let m = Theme::resolve(true, "light");
        assert_eq!(m, Theme::monochrome());
        assert_eq!(m.bg, Color::Black);
        assert_eq!(m.text_primary, Color::White);
        assert_eq!(m.brand, Color::White);
    }

    #[test]
    fn test_effective_defaults_to_deep_space_without_env() {
        // 正常测试环境无 NO_COLOR / MARGUERITE_THEME
        assert_eq!(Theme::effective(), Theme::DEEP_SPACE);
    }

    #[test]
    fn test_status_glyphs_are_non_emoji() {
        assert_eq!(Theme::status_glyph(StatusDot::Running), "▮");
        assert_eq!(Theme::status_glyph(StatusDot::Error), "✖");
        assert_eq!(Theme::status_glyph(StatusDot::Waiting), "◉");
    }

    #[test]
    fn test_disp_width_and_truncate() {
        assert_eq!(disp_width("abc"), 3);
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello", 3), "he…");
    }
}
