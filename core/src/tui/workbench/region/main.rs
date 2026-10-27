//! 中工作区（TabBar + Chat/Code/Terminal）

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Tabs};
use ratatui::Frame;

use crate::tui::workbench::slot::{Slot, SlotId};
use crate::tui::workbench::state::{ChatRole, MainTab, WorkbenchState};
use crate::tui::workbench::theme::Theme;

/// 主区渲染器
pub struct MainArea;

impl Slot for MainArea {
    fn id(&self) -> SlotId {
        SlotId::MainTabs
    }
    fn render(&self, frame: &mut Frame, area: Rect, state: &WorkbenchState) {
        render_main(frame, area, state);
    }
}

/// 渲染主区
pub fn render_main(frame: &mut Frame, area: Rect, state: &WorkbenchState) {
    let v = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);
    render_tabbar(frame, v[0], state);
    match state.main_tab {
        MainTab::Chat => render_chat(frame, v[1], state),
        MainTab::Code => render_code(frame, v[1], state),
        MainTab::Terminal => render_terminal(frame, v[1], state),
    }
}

fn render_tabbar(frame: &mut Frame, area: Rect, state: &WorkbenchState) {
    let t = Theme::effective();
    let titles = vec![
        Line::from("Chat"),
        Line::from("Code"),
        Line::from("Terminal"),
    ];
    let idx = match state.main_tab {
        MainTab::Chat => 0,
        MainTab::Code => 1,
        MainTab::Terminal => 2,
    };
    let tabs = Tabs::new(titles)
        .select(idx)
        .style(Style::default().fg(t.text_muted))
        .highlight_style(Style::default().fg(t.brand).bold());
    frame.render_widget(tabs, area);
}

fn render_chat(frame: &mut Frame, area: Rect, state: &WorkbenchState) {
    let t = Theme::effective();
    let v = Layout::vertical([Constraint::Min(1), Constraint::Length(2)]).split(area);

    let mut lines: Vec<Line> = Vec::new();
    for m in &state.chat.messages {
        let tag = match m.role {
            ChatRole::User => "你",
            ChatRole::Assistant => "AI",
            ChatRole::System => "系统",
        };
        lines.push(Line::from(Span::styled(
            format!("{tag}: "),
            Style::default().fg(t.text_secondary),
        )));
        for ln in m.content.lines() {
            lines.push(Line::from(Span::styled(
                ln.to_string(),
                Style::default().fg(t.text_primary),
            )));
        }
    }
    // 思考过程（推理链）单独渲染、且用弱化色：它**不是**答复正文。
    // 此前 Thinking 被累积进正文缓冲，界面上把推理当答案显示。
    if let Some(tk) = &state.chat.thinking {
        if !tk.trim().is_empty() {
            lines.push(Line::from(Span::styled(
                "AI（思考）:",
                Style::default().fg(t.text_secondary),
            )));
            for ln in tk.lines() {
                lines.push(Line::from(Span::styled(
                    ln.to_string(),
                    Style::default().fg(t.text_secondary),
                )));
            }
        }
    }
    if let Some(s) = &state.chat.streaming {
        // 逐行渲染：答复含换行时，单条 `Line` 无法正确展示（会截断/串行）。
        lines.push(Line::from(Span::styled(
            "AI: ",
            Style::default().fg(t.text_secondary),
        )));
        for ln in s.lines() {
            lines.push(Line::from(Span::styled(
                ln.to_string(),
                Style::default().fg(t.brand_light),
            )));
        }
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), v[0]);

    // 反向搜索态显示查询串与当前候选；普通态显示缓冲（Phase 4 / AC-08）
    let composer_line = if state.composer.history.is_searching() {
        let q = state.composer.history.search_query();
        match state.composer.history.search_current() {
            Some(c) => format!("> (r) {q}  <<{c}>>"),
            None => format!("> (r) {q}  (无匹配)"),
        }
    } else {
        format!("> {}", state.composer.buffer)
    };
    let composer = Paragraph::new(Line::from(composer_line)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border)),
    );
    frame.render_widget(composer, v[1]);
}

fn render_code(frame: &mut Frame, area: Rect, _state: &WorkbenchState) {
    let t = Theme::effective();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border))
        .title(" 代码 ");
    let p = Paragraph::new(Line::from(Span::styled(
        "代码视图（由后端文件内容 + guard 诊断内联标记填充）",
        Style::default().fg(t.text_muted),
    )));
    frame.render_widget(p, area); // block 仅为示意；内容占位
    let _ = block;
}

fn render_terminal(frame: &mut Frame, area: Rect, state: &WorkbenchState) {
    let t = Theme::effective();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border))
        .title(" 终端 ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(Text::from(state.terminal_lines.clone())),
        inner,
    );
}
