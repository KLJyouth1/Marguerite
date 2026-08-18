//! Bloom 原生能力面板（ratatui Widget，可渲染到 Buffer 做视觉门禁）
//!
//! 把引擎差异化能力（46 工具 / 沙箱 / KB / 治理门）以 Bloom 终端美学呈现，
//! 背景叠一层 Bloom 粒子场（粒子溶解隐喻）。渲染层只依赖 ratatui，
//! 可在单测中 `render` 到 `Buffer` 并断言文本内容（无头环境可测视觉）。
//!
//! 严格遵循：无 emoji 图标（用 Unicode 几何块 `▮`/`✖`）、无硬编码色（经 BloomTheme）、
//! 无紫粉渐变。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;

use super::capabilities::CapabilitySnapshot;
use super::particles::ParticleField;
use super::theme::BloomTheme;

/// Bloom 原生能力面板
pub struct CapabilityPanel {
    snap: CapabilitySnapshot,
    field: ParticleField,
    theme: BloomTheme,
}

impl CapabilityPanel {
    pub fn new(snap: CapabilitySnapshot, field: ParticleField) -> Self {
        Self {
            snap,
            field,
            theme: BloomTheme::DEFAULT,
        }
    }

    fn status_glyph(on: bool) -> &'static str {
        if on {
            "▮"
        } else {
            "✖"
        }
    }

    fn status_color(on: bool, t: &BloomTheme) -> Color {
        if on {
            t.success
        } else {
            t.danger
        }
    }
}

impl Widget for CapabilityPanel {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.theme;
        // 背景填充
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_style(Style::default().bg(t.bg).fg(t.text_primary));
                }
            }
        }
        if area.height < 6 || area.width < 30 {
            return; // 太小不渲染（避免越界）
        }
        let mut row = area.top();
        // 标题
        buf.set_string(
            area.left() + 1,
            row,
            self.snap.brand,
            Style::default().fg(t.bloom_light).bg(t.bg),
        );
        row += 1;
        // 分隔线
        let sep: String =
            std::iter::repeat_n('─', (area.width as usize).saturating_sub(2)).collect();
        buf.set_string(
            area.left() + 1,
            row,
            &sep,
            Style::default().fg(t.border).bg(t.bg),
        );
        row += 1;

        // 能力行
        let lines: Vec<(String, bool)> = vec![
            (format!("Native Tools      : {}", self.snap.tools), true),
            (
                format!(
                    "OS Sandbox        : {}",
                    Self::status_glyph(self.snap.sandbox)
                ),
                self.snap.sandbox,
            ),
            (
                format!("KB / RAG (BM25)   : {}", Self::status_glyph(self.snap.kb)),
                self.snap.kb,
            ),
            (
                format!(
                    "Governance Gate   : {}",
                    Self::status_glyph(self.snap.governance)
                ),
                self.snap.governance,
            ),
        ];
        for (text, on) in lines {
            let color = if text.starts_with("Native Tools") {
                t.bloom
            } else {
                Self::status_color(on, &t)
            };
            buf.set_string(
                area.left() + 1,
                row,
                &text,
                Style::default().fg(color).bg(t.bg),
            );
            row += 1;
        }
        row += 1; // 留白

        // 粒子场（剩余区域）
        let field_top = row;
        let field_bottom = area.bottom().saturating_sub(1);
        if field_bottom > field_top {
            let fw = (area.width.saturating_sub(2)).min(self.field.w);
            let fh = (field_bottom - field_top).min(self.field.h);
            for gy in 0..fh {
                for gx in 0..fw {
                    let d = self.field.density_at(gx, gy);
                    let ch = BloomTheme::particle_glyph(d);
                    if ch != " " {
                        let color = BloomTheme::glow(d);
                        buf.set_string(
                            area.left() + 1 + gx,
                            field_top + gy,
                            ch,
                            Style::default().fg(color).bg(t.bg),
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::bloom::capabilities::CapabilityProvider;

    fn sample_snapshot() -> CapabilitySnapshot {
        struct Mock;
        impl CapabilityProvider for Mock {
            fn tool_count(&self) -> usize {
                23
            }
        }
        CapabilitySnapshot::from_provider(&Mock)
    }

    #[test]
    fn test_panel_renders_brand_and_capabilities() {
        let snap = sample_snapshot();
        let mut field = ParticleField::new(40, 12, 5);
        field.step(0.2, 0.9); // 注入粒子
        let panel = CapabilityPanel::new(snap, field);
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        panel.render(area, &mut buf);
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("MARGUERITE"), "应包含品牌标识");
        assert!(content.contains("Native Tools"), "应包含工具行");
        assert!(
            content.contains(&snap.tools.to_string()),
            "应显示工具数 {}",
            snap.tools
        );
        assert!(content.contains("OS Sandbox"), "应包含沙箱行");
        assert!(content.contains("Governance Gate"), "应包含治理门行");
        assert!(content.contains("▮"), "启用项应用几何块状态点（非 emoji）");
    }

    #[test]
    fn test_panel_too_small_skips_render() {
        let snap = sample_snapshot();
        let field = ParticleField::new(10, 5, 1);
        let panel = CapabilityPanel::new(snap, field);
        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);
        panel.render(area, &mut buf);
        // 不 panic 即可（极小区域直接返回）
    }

    #[test]
    fn test_panel_no_emoji_glyphs() {
        // 视觉门禁：渲染结果不得含 emoji 功能图标
        // （允许本设计明确使用的 Unicode 几何块/状态点，非 emoji）
        let emoji_ranges: &[(u32, u32)] = &[
            (0x1F300, 0x1FAFF),
            (0x1F000, 0x1F02F),
            (0x1F0A0, 0x1F0FF),
            (0x1F100, 0x1F1FF),
            (0x1F600, 0x1F64F),
            (0x1F680, 0x1F6FF),
            (0x1F900, 0x1F9FF),
            (0x2600, 0x26FF),
            (0x2700, 0x27BF),
        ];
        let whitelist: &[char] = &['▮', '✖', '◉', '·', '█', '▓', '▒', '░'];
        let snap = sample_snapshot();
        let mut field = ParticleField::new(30, 10, 2);
        field.step(0.3, 1.0);
        let panel = CapabilityPanel::new(snap, field);
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        panel.render(area, &mut buf);
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        for c in content.chars() {
            let cp = c as u32;
            let in_emoji = emoji_ranges.iter().any(|(lo, hi)| cp >= *lo && cp <= *hi);
            assert!(
                !in_emoji || whitelist.contains(&c),
                "渲染含 emoji 字符 U+{:X}",
                cp
            );
        }
    }
}
