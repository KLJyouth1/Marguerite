//! 左活动栏（垂直图标列，单字符标签，禁用 emoji）

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::workbench::slot::{Slot, SlotId};
use crate::tui::workbench::state::{ActivityKind, WorkbenchState};
use crate::tui::workbench::theme::Theme;

/// 活动栏渲染器
pub struct ActivityRail;

impl Slot for ActivityRail {
    fn id(&self) -> SlotId {
        SlotId::LeftActivity
    }
    fn render(&self, frame: &mut Frame, area: Rect, state: &WorkbenchState) {
        render_activity(frame, area, state);
    }
}

/// 活动栏种类 → 单字符标签（F/S/K/C/O）
pub fn glyph(k: ActivityKind) -> &'static str {
    match k {
        ActivityKind::Files => "F",
        ActivityKind::Search => "S",
        ActivityKind::Skills => "K",
        ActivityKind::Sessions => "C",
        ActivityKind::Settings => "O",
    }
}

/// 渲染活动栏
pub fn render_activity(frame: &mut Frame, area: Rect, state: &WorkbenchState) {
    let t = Theme::effective();
    let kinds = [
        ActivityKind::Files,
        ActivityKind::Search,
        ActivityKind::Skills,
        ActivityKind::Sessions,
        ActivityKind::Settings,
    ];
    let mut lines: Vec<Line> = Vec::new();
    for k in kinds {
        let active = state.active_activity == k;
        let style = if active {
            Style::default().fg(t.brand).bg(t.panel)
        } else {
            Style::default().fg(t.text_muted)
        };
        lines.push(Line::from(vec![Span::styled(
            format!(" {} ", glyph(k)),
            style,
        )]));
    }
    // 活动栏宽 2，不放边框避免溢出
    let p = Paragraph::new(lines);
    frame.render_widget(p, area);
    let _ = Color::Black; // 保留引用，避免未使用导入告警
}
