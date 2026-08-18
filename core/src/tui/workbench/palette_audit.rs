//! P0 视觉合规检测锚点（具名常量集中管理）。
//!
//! 这些颜色不是 UI 实际着色，而是 J5 全帧门禁中被「禁止 / 被要求」的检测锚点：
//! 紫粉色值必须不出现（P0-2），品牌蓝必须出现（P0 视觉达标）。
//! 集中为带用途注释的具名常量，避免渲染 / 断言代码散落匿名 RGB 字面量（P0-3）。

/// P0-2 禁用紫粉渐变色值锚点（任一命中即视为违规）。
pub const FORBIDDEN_VIOLET_PINK: &[(u8, u8, u8)] = &[
    (124, 58, 237),  // #7C3AED Violet
    (168, 85, 247),  // #A855F7 Magenta
    (236, 72, 153),  // #EC4899 Pink
    (99, 102, 241),  // #6366F1 Indigo
    (79, 70, 229),   // #4F46E5 SlateBlue
];

/// P0 品牌蓝锚点（#3B82F6），全帧渲染必须出现方视为视觉达标。
pub const REQUIRED_BRAND_BLUE: (u8, u8, u8) = (59, 130, 246);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_audit_constants_sanity() {
        assert_eq!(FORBIDDEN_VIOLET_PINK.len(), 5);
        assert!(FORBIDDEN_VIOLET_PINK.contains(&(236, 72, 153))); // Pink
        assert_eq!(REQUIRED_BRAND_BLUE, (59, 130, 246));
    }
}
