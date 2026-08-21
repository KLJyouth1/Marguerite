//! 顶栏（H5）— mode + 主/副模型同构显示，与 GUI 顶栏选择器一致
//!
//! 设计契约（P0 视觉）：蓝主色 `#3B82F6`、禁止 emoji 图标、禁止紫粉渐变、禁止硬编码色值。
//! 顶栏高为 1，背景用面板色铺底形成「条」，内容从左到右：模式 | 主模型 | (+ 副模型) | 提示 | 通知。

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::mode::ModeKind;
use crate::tui::workbench::state::WorkbenchState;
use crate::tui::workbench::theme::{truncate, Theme};

/// 顶栏渲染器（自由函数，渲染只读 `WorkbenchState`）
pub fn render_topbar(frame: &mut Frame, area: Rect, state: &WorkbenchState) {
    let t = Theme::effective();

    let mode_label = match state.mode {
        ModeKind::Bloom => "Bloom",
        ModeKind::Agent => "Agent",
        ModeKind::Plan => "Plan",
        ModeKind::Ensemble => "Ensemble",
    };

    let mut spans = Vec::new();
    // 模式标识（品牌色，区分于底部状态栏的单字符）
    spans.push(Span::styled(
        format!(" {mode_label} "),
        Style::default().fg(t.brand).bg(t.panel),
    ));
    spans.push(Span::styled(
        " | ",
        Style::default().fg(t.border).bg(t.panel),
    ));
    // 主模型（蓝主色标签）
    spans.push(Span::styled(
        format!("模型 {}", state.model),
        Style::default().fg(t.text_primary).bg(t.panel),
    ));
    // 副模型（若有）
    if !state.copilot_model.is_empty() {
        spans.push(Span::styled(
            " + ",
            Style::default().fg(t.border).bg(t.panel),
        ));
        spans.push(Span::styled(
            format!("副 {}", state.copilot_model),
            Style::default().fg(t.brand_light).bg(t.panel),
        ));
    }
    // 操作提示（弱文本，非 emoji）
    spans.push(Span::styled(
        "   输入 /model 管理模型",
        Style::default().fg(t.text_muted).bg(t.panel),
    ));
    // 瞬时通知（告警橙，覆盖式）
    if let Some(notice) = &state.notice {
        spans.push(Span::styled(
            format!("   {}", truncate(notice, 48)),
            Style::default().fg(t.warning).bg(t.panel),
        ));
    }

    let p = Paragraph::new(Line::from(spans)).style(Style::default().bg(t.panel));
    frame.render_widget(p, area);
}
