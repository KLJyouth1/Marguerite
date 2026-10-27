//! 底部状态栏（模型 | 模式 | Token | 会话时间 | 连接）

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::workbench::slot::{Slot, SlotId};
use crate::tui::workbench::state::ConnectionStatus;
use crate::tui::workbench::state::WorkbenchState;
use crate::tui::workbench::theme::Theme;

/// 状态栏渲染器
pub struct StatusBar;

impl Slot for StatusBar {
    fn id(&self) -> SlotId {
        SlotId::BottomStatusbar
    }
    fn render(&self, frame: &mut Frame, area: Rect, state: &WorkbenchState) {
        render_status(frame, area, state);
    }
}

/// 渲染状态栏（固定顺序：模型 | 模式 | Token | 会话时间 | 连接）
pub fn render_status(frame: &mut Frame, area: Rect, state: &WorkbenchState) {
    let t = Theme::effective();

    let mode_char = state.mode.as_str().chars().next().unwrap_or('?'); // P / A / Y

    let (conn_text, conn_color) = match state.connection {
        ConnectionStatus::Ready => ("就绪", t.text_muted),
        ConnectionStatus::Connected => ("已连接", t.success),
        ConnectionStatus::Disconnected => ("已断开", t.danger),
    };

    // 档位颜色跟随预算状态机：健康绿 / 吃紧橙 / 紧急·耗尽红。
    let tier_color = match state.tier_ledger.budget_state.as_str() {
        "healthy" => t.success,
        "degraded" => t.warning,
        "emergency" | "depleted" => t.danger,
        _ => t.brand,
    };
    let tier_label = state.tier_ledger.label();

    let line = Line::from(vec![
        Span::styled(format!(" {} ", state.model), Style::default().fg(t.brand)),
        Span::styled(" | ", Style::default().fg(t.border)),
        Span::styled(format!(" {tier_label} "), Style::default().fg(tier_color)),
        Span::styled(" | ", Style::default().fg(t.border)),
        Span::styled(format!(" {mode_char} "), Style::default().fg(t.brand_light)), // 模式标识
        Span::styled(" | ", Style::default().fg(t.border)),
        Span::styled(
            format!(
                " {}/{} tok ",
                state.token_usage.used, state.token_usage.budget
            ),
            Style::default().fg(t.text_secondary),
        ),
        Span::styled(" | ", Style::default().fg(t.border)),
        Span::styled(
            format!(" {} ", state.session_time),
            Style::default().fg(t.text_secondary),
        ),
        Span::styled(" | ", Style::default().fg(t.border)),
        Span::styled(format!(" {} ", conn_text), Style::default().fg(conn_color)),
    ]);
    // 状态栏高 1，直接渲染（无边框避免溢出）
    frame.render_widget(Paragraph::new(line), area);
}
