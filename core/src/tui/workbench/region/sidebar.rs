//! 左侧边栏（文件树/搜索/技能/会话/命令 五面板，随活动栏切换）

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::tui::workbench::slot::{Slot, SlotId};
use crate::tui::workbench::state::WorkbenchState;
use crate::tui::workbench::theme::Theme;

/// 侧边栏渲染器
pub struct SidebarPanel;

impl Slot for SidebarPanel {
    fn id(&self) -> SlotId {
        // 侧边栏 id 随活动面板动态变化
        // 注：Slot::id 用于注册表索引；此处返回当前激活面板 id 以反映实际渲染目标
        SlotId::LeftSidebarFileTree
    }
    fn render(&self, frame: &mut Frame, area: Rect, state: &WorkbenchState) {
        render_sidebar(frame, area, state);
    }
}

/// 渲染侧边栏（按当前激活面板分发）
pub fn render_sidebar(frame: &mut Frame, area: Rect, state: &WorkbenchState) {
    let t = Theme::effective();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border))
        .title(" 侧边栏 ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    match state.active_sidebar {
        SlotId::LeftSidebarFileTree => render_file_tree(frame, inner, state),
        SlotId::LeftSidebarSearch => render_search(frame, inner, state),
        SlotId::LeftSidebarSkills => render_skills(frame, inner, state),
        SlotId::LeftSidebarSessions => render_sessions(frame, inner, state),
        SlotId::LeftSidebarCommands => render_commands(frame, inner, state),
        _ => {
            let p = Paragraph::new(Line::from("未实现面板"));
            frame.render_widget(p, inner);
        }
    }
}

fn title_style() -> Style {
    Style::default().fg(Theme::effective().brand)
}

fn render_file_tree(frame: &mut Frame, area: Rect, _state: &WorkbenchState) {
    let t = Theme::effective();
    // 文件树示意：用几何块/文本标记而非 emoji 图标（P0 规则）；实际文件树由后端填充
    let entries = [
        "▾ src/",
        "  ▾ core/",
        "  ▾ gui/",
        "· README.md",
        "· Cargo.toml",
    ];
    let lines: Vec<Line> = entries
        .iter()
        .map(|e| {
            let (icon, label) = e.split_once(' ').unwrap_or(("", e));
            Line::from(vec![
                Span::styled(icon.to_string(), Style::default().fg(t.text_muted)),
                Span::styled(format!(" {label}"), Style::default().fg(t.text_primary)),
            ])
        })
        .collect();
    let p = Paragraph::new(lines);
    frame.render_widget(p, area);
}

fn render_search(frame: &mut Frame, area: Rect, _state: &WorkbenchState) {
    let t = Theme::effective();
    let p = Paragraph::new(Line::from(vec![
        Span::styled("搜索 ", title_style()),
        Span::styled(
            "输入关键字以检索代码与记忆",
            Style::default().fg(t.text_muted),
        ),
    ]));
    frame.render_widget(p, area);
}

fn render_skills(frame: &mut Frame, area: Rect, _state: &WorkbenchState) {
    let t = Theme::effective();
    let p = Paragraph::new(Line::from(Span::styled(
        "（无已安装技能）",
        Style::default().fg(t.text_muted),
    )));
    frame.render_widget(p, area);
}

fn render_sessions(frame: &mut Frame, area: Rect, _state: &WorkbenchState) {
    let t = Theme::effective();
    let p = Paragraph::new(Line::from(Span::styled(
        "（无历史会话）",
        Style::default().fg(t.text_muted),
    )));
    frame.render_widget(p, area);
}

fn render_commands(frame: &mut Frame, area: Rect, _state: &WorkbenchState) {
    let t = Theme::effective();
    let p = Paragraph::new(Line::from(vec![
        Span::styled("/help ", title_style()),
        Span::styled("查看全部命令", Style::default().fg(t.text_muted)),
    ]));
    frame.render_widget(p, area);
}
