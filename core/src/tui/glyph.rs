//! 双前端统一「批准字形」契约（SSOT）。
//!
//! 这些单色几何状态字形被 TUI（ratatui/TestBackend 逐 cell 扫描，见
//! `workbench::mod::j5_emoji_hits`）与桌面端（视觉红线扫描器
//! `scripts/check_visual_redlines.py`）共同认可为「非 emoji」的功能状态符号。
//!
//! 任何新加入 UI 的状态字形必须先从本集合选取，禁止引入 emoji（P0 视觉红线）。
//!
//! 桌面端的 Python 扫描器 `scripts/check_visual_redlines.py` 维护一份同步副本，
//! 并由该脚本的 `--self-test` 校验两份真相不漂移——本文件是唯一的权威来源。

/// 批准的几何状态字形（非 emoji）。
///
/// 取值说明：
/// - `▮` 阻塞 / 竖条状态点
/// - `✖` 失败 / 错误状态点
/// - `◉` 进行中 / 活跃状态点
/// - `·` 空闲 / 待命状态点
/// - `●` 完成 / 已就绪（桌面端 `main.rs` 能力点使用）
/// - `○` 未选中 / 空心
/// - `►▸›‹` 方向指示（列表展开 / 步骤导航）
/// - `█▓▒░` 密度块（进度 / 占位）
pub const APPROVED_GLYPHS: &[char] = &[
    '▮', '✖', '◉', '·', '●', '○', '►', '▸', '›', '‹', '█', '▓', '▒', '░',
];

/// 判断字符是否属于批准字形（非 emoji 功能符号）。
pub fn is_approved_glyph(ch: char) -> bool {
    APPROVED_GLYPHS.contains(&ch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approved_glyphs_all_recognized() {
        for &g in APPROVED_GLYPHS {
            assert!(is_approved_glyph(g), "{g:?} 必须在批准集合内");
        }
    }

    #[test]
    fn rejects_emoji_as_glyph() {
        // 这些是此前在生产路径抓到的真 emoji（P0 红线禁用）
        assert!(!is_approved_glyph('🚀'));
        assert!(!is_approved_glyph('⏳'));
        assert!(!is_approved_glyph('⚠'));
        assert!(!is_approved_glyph('✓'));
    }
}
