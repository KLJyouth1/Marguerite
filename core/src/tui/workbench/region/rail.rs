//! 右 AI 面板（Agent/Tools/Context/Memory/Tasks 五 tab，事件驱动）

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::workbench::slot::{Slot, SlotId};
use crate::tui::workbench::state::{DeployKind, RailTab, WorkbenchState};
use crate::tui::workbench::theme::{truncate, StatusDot, Theme};

/// 右栏渲染器
pub struct RailPanel;

impl Slot for RailPanel {
    fn id(&self) -> SlotId {
        SlotId::RightAgentStatus
    }
    fn render(&self, frame: &mut Frame, area: Rect, state: &WorkbenchState) {
        render_rail(frame, area, state);
    }
}

/// 渲染右栏
pub fn render_rail(frame: &mut Frame, area: Rect, state: &WorkbenchState) {
    let v = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);
    render_tabbar(frame, v[0], state);
    match state.rail_tab {
        RailTab::Agent => render_agent(frame, v[1], state),
        RailTab::Models => render_models(frame, v[1], state),
        RailTab::Tools => render_tools(frame, v[1], state),
        RailTab::Context => render_context(frame, v[1], state),
        RailTab::Memory => render_memory(frame, v[1], state),
        RailTab::Tasks => render_tasks(frame, v[1], state),
        RailTab::Deploy => render_deploy(frame, v[1], state),
        RailTab::Collab => render_collab(frame, v[1], state),
    }
}

fn render_tabbar(frame: &mut Frame, area: Rect, state: &WorkbenchState) {
    use ratatui::layout::Alignment;
    let t = Theme::effective();
    // 右栏保持紧凑：模型面板使用 Md，其余 tab 使用 2 字符缩写。
    // 缩写: Ag=Agent Md=Models Tl=Tools Cx=Context Mm=Memory Tk=Tasks Dp=Deploy Co=Collab
    let labels = ["Ag", "Md", "Tl", "Cx", "Mm", "Tk", "Dp", "Co"];
    let idx = match state.rail_tab {
        RailTab::Agent => 0,
        RailTab::Models => 1,
        RailTab::Tools => 2,
        RailTab::Context => 3,
        RailTab::Memory => 4,
        RailTab::Tasks => 5,
        RailTab::Deploy => 6,
        RailTab::Collab => 7,
    };
    let mut spans: Vec<Span> = Vec::with_capacity(labels.len() * 2);
    for (i, lab) in labels.iter().enumerate() {
        let active = i == idx;
        spans.push(Span::styled(
            *lab,
            if active {
                Style::default().fg(t.brand).bold()
            } else {
                Style::default().fg(t.text_muted)
            },
        ));
        if i < labels.len() - 1 {
            spans.push(Span::styled("│", Style::default().fg(t.text_muted)));
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Left),
        area,
    );
}

fn render_agent(frame: &mut Frame, area: Rect, state: &WorkbenchState) {
    let t = Theme::effective();
    let dot = match state.interaction {
        crate::tui::workbench::state::InteractionState::Processing => StatusDot::Running,
        crate::tui::workbench::state::InteractionState::AwaitingApproval => StatusDot::Waiting,
        crate::tui::workbench::state::InteractionState::Idle => StatusDot::Idle,
        _ => StatusDot::Idle,
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{} ", Theme::status_glyph(dot)),
                Style::default().fg(t.success),
            ),
            Span::styled("智能体", Style::default().fg(t.text_secondary)),
        ]),
        Line::from(vec![
            Span::styled("模式: ", Style::default().fg(t.text_muted)),
            Span::styled(state.mode.as_str(), Style::default().fg(t.brand)),
        ]),
        Line::from(vec![
            Span::styled("模型: ", Style::default().fg(t.text_muted)),
            Span::styled(state.model.clone(), Style::default().fg(t.text_primary)),
        ]),
        Line::from(vec![
            Span::styled("Token: ", Style::default().fg(t.text_muted)),
            Span::styled(
                format!("{}/{}", state.token_usage.used, state.token_usage.budget),
                Style::default().fg(t.text_primary),
            ),
        ]),
    ];
    if let Some(p) = state.progress {
        lines.push(Line::from(vec![
            Span::styled("进度: ", Style::default().fg(t.text_muted)),
            Span::styled(format!("{p}%"), Style::default().fg(t.brand_light)),
        ]));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn render_models(frame: &mut Frame, area: Rect, state: &WorkbenchState) {
    let t = Theme::effective();
    let panel = &state.model_panel;
    let mut lines = vec![Line::from(vec![
        Span::styled("模型档案 ", Style::default().fg(t.brand).bold()),
        Span::styled(
            format!("{} 个", panel.items.len()),
            Style::default().fg(t.text_secondary),
        ),
    ])];

    if let Some(err) = &panel.error {
        // 错误态与空态必须分开：读取失败时显示"暂无档案"会把用户引向错误操作。
        lines.push(Line::from(Span::styled(
            "读取失败",
            Style::default().fg(t.danger).bold(),
        )));
        for ln in err.lines().take(3) {
            lines.push(Line::from(Span::styled(
                ln.to_string(),
                Style::default().fg(t.danger),
            )));
        }
        lines.push(Line::from(Span::styled(
            "Ctrl+M 重试",
            Style::default().fg(t.text_muted),
        )));
    } else if panel.items.is_empty() {
        lines.push(Line::from(Span::styled(
            "暂无档案：/model add 新增，或到设置页配置服务商",
            Style::default().fg(t.text_muted),
        )));
    } else {
        for (index, item) in panel.items.iter().enumerate() {
            let is_active = item.id == panel.active_id;
            let is_cursor = index == panel.cursor;
            // 标记语义：● 当前激活 + 光标，◆ 当前激活，▶ 光标，○ 普通
            let marker = match (is_cursor, is_active) {
                (true, true) => "●",
                (true, false) => "▶",
                (false, true) => "◆",
                (false, false) => "○",
            };
            let base = if is_active {
                Style::default().fg(t.brand).bold()
            } else {
                Style::default().fg(t.text_primary)
            };
            let style = if is_cursor { base.bg(t.panel) } else { base };
            lines.push(Line::from(vec![
                Span::styled(format!("{marker} "), style),
                Span::styled(truncate(&item.label, 18), style),
                Span::styled(
                    format!(
                        " {}{}",
                        if is_active { "当前 " } else { "" },
                        if item.key_ready { "" } else { "缺密钥" }
                    ),
                    Style::default().fg(if item.key_ready { t.success } else { t.warning }),
                ),
            ]));
            // 第二行放 provider + 接入点：右栏窄，同一行必然被截断成噪声。
            lines.push(Line::from(Span::styled(
                format!("   {} · {}", item.provider, item.base_url),
                Style::default().fg(t.text_muted),
            )));
        }
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn render_tools(frame: &mut Frame, area: Rect, state: &WorkbenchState) {
    let t = Theme::effective();
    let mut lines: Vec<Line> = Vec::new();
    if state.tools.is_empty() {
        lines.push(Line::from(Span::styled(
            "（无工具调用）",
            Style::default().fg(t.text_muted),
        )));
    }
    for e in &state.tools {
        let (glyph, color) = match e.ok {
            Some(true) => (Theme::status_glyph(StatusDot::Running), t.success),
            Some(false) => (Theme::status_glyph(StatusDot::Error), t.danger),
            None => (Theme::status_glyph(StatusDot::Waiting), t.warning),
        };
        let args = e.args.join(" ");
        lines.push(Line::from(vec![
            Span::styled(format!("{glyph} "), Style::default().fg(color)),
            Span::styled(
                format!("{} {}", e.tool, args),
                Style::default().fg(t.text_primary),
            ),
            Span::styled(
                format!("  {}", e.summary),
                Style::default().fg(t.text_muted),
            ),
        ]));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn render_context(frame: &mut Frame, area: Rect, state: &WorkbenchState) {
    let t = Theme::effective();
    let lines = match state.context_snapshot {
        Some(cs) => vec![
            Line::from(vec![
                Span::styled("已用: ", Style::default().fg(t.text_muted)),
                Span::styled(
                    format!("{}/{}", cs.used, cs.budget),
                    Style::default().fg(t.text_primary),
                ),
            ]),
            Line::from(vec![
                Span::styled("缓存命中: ", Style::default().fg(t.text_muted)),
                Span::styled(
                    format!("{:.0}%", cs.cache_hit * 100.0),
                    Style::default().fg(t.text_primary),
                ),
            ]),
            Line::from(vec![
                Span::styled("压缩: ", Style::default().fg(t.text_muted)),
                Span::styled(
                    if cs.compressed { "是" } else { "否" },
                    Style::default().fg(t.text_primary),
                ),
            ]),
        ],
        None => vec![Line::from(Span::styled(
            "（上下文快照待填充）",
            Style::default().fg(t.text_muted),
        ))],
    };
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn render_memory(frame: &mut Frame, area: Rect, state: &WorkbenchState) {
    let t = Theme::effective();
    let mut lines: Vec<Line> = Vec::new();
    if state.memory_refs.is_empty() {
        lines.push(Line::from(Span::styled(
            "（无记忆引用）",
            Style::default().fg(t.text_muted),
        )));
    }
    for m in &state.memory_refs {
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", m.key), Style::default().fg(t.text_primary)),
            Span::styled(
                format!("({:.2})", m.confidence),
                Style::default().fg(t.text_muted),
            ),
        ]));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn render_tasks(frame: &mut Frame, area: Rect, state: &WorkbenchState) {
    let t = Theme::effective();
    let mut lines: Vec<Line> = Vec::new();
    if state.tasks.is_empty() {
        lines.push(Line::from(Span::styled(
            "（无任务）",
            Style::default().fg(t.text_muted),
        )));
    }
    for tk in &state.tasks {
        lines.push(Line::from(vec![
            Span::styled(
                format!("▸ {} ", tk.label),
                Style::default().fg(t.text_primary),
            ),
            Span::styled(
                format!("[{}]", tk.status),
                Style::default().fg(t.text_muted),
            ),
        ]));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

/// 部署面板（Phase I4，FR-16）：当前目标 + 最近一次 dry-run 结果 + 历史
fn render_deploy(frame: &mut Frame, area: Rect, state: &WorkbenchState) {
    let t = Theme::effective();
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("部署面板", Style::default().fg(t.brand).bold()),
        Span::styled("  (FR-16)", Style::default().fg(t.text_muted)),
    ]));
    match &state.deploy_panel.current_target {
        Some(tgt) => lines.push(Line::from(vec![
            Span::styled("目标: ", Style::default().fg(t.text_muted)),
            Span::styled(tgt.clone(), Style::default().fg(t.text_primary)),
        ])),
        None => lines.push(Line::from(Span::styled(
            "（未选择目标，/deploy run <ssh://host|baota://别名>）",
            Style::default().fg(t.text_muted),
        ))),
    }
    if let Some(run) = state.deploy_panel.runs.back() {
        let kind = match run.kind {
            DeployKind::Ssh => "SSH",
            DeployKind::Baota => "宝塔",
        };
        let (glyph, color) = if run.status.starts_with("error") {
            (Theme::status_glyph(StatusDot::Error), t.danger)
        } else if run.status.starts_with("rolled_back") {
            (Theme::status_glyph(StatusDot::Waiting), t.warning)
        } else {
            (Theme::status_glyph(StatusDot::Running), t.success)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{glyph} "), Style::default().fg(color)),
            Span::styled(
                format!("[{kind}] {}", run.status),
                Style::default().fg(color),
            ),
        ]));
        if let Some(h) = &run.sha256 {
            let head = if h.len() >= 8 { &h[..8] } else { h.as_str() };
            lines.push(Line::from(vec![
                Span::styled("sha256: ", Style::default().fg(t.text_muted)),
                Span::styled(format!("{head}…"), Style::default().fg(t.text_secondary)),
            ]));
        }
    }
    if state.deploy_panel.runs.len() > 1 {
        lines.push(Line::from(Span::styled(
            "历史:",
            Style::default().fg(t.text_muted),
        )));
        for run in state.deploy_panel.runs.iter().rev().skip(1).take(5) {
            let kind = match run.kind {
                DeployKind::Ssh => "SSH",
                DeployKind::Baota => "宝塔",
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  · [{kind}] {} ", run.target),
                    Style::default().fg(t.text_secondary),
                ),
                Span::styled(run.status.clone(), Style::default().fg(t.text_muted)),
            ]));
        }
    }
    if state.deploy_panel.runs.is_empty() {
        lines.push(Line::from(Span::styled(
            "（尚无部署记录）",
            Style::default().fg(t.text_muted),
        )));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

/// 协作事件流（Phase I4，FR-15）：在线成员 + 事件日志
fn render_collab(frame: &mut Frame, area: Rect, state: &WorkbenchState) {
    let t = Theme::effective();
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("协作事件流", Style::default().fg(t.brand).bold()),
        Span::styled(
            format!("  ({} 人在线)", state.collab.members.len()),
            Style::default().fg(t.text_muted),
        ),
    ]));
    if state.collab.members.is_empty() {
        lines.push(Line::from(Span::styled(
            "（无在线成员，/collab join <名> 加入）",
            Style::default().fg(t.text_muted),
        )));
    } else {
        for m in &state.collab.members {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} ", Theme::status_glyph(StatusDot::Running)),
                    Style::default().fg(t.success),
                ),
                Span::styled(m.clone(), Style::default().fg(t.text_primary)),
            ]));
        }
    }
    lines.push(Line::from(Span::styled(
        "事件:",
        Style::default().fg(t.text_muted),
    )));
    if state.collab.log.is_empty() {
        lines.push(Line::from(Span::styled(
            "（暂无事件）",
            Style::default().fg(t.text_muted),
        )));
    } else {
        for entry in state.collab.log.iter().rev().take(12) {
            lines.push(Line::from(Span::styled(
                entry.clone(),
                Style::default().fg(t.text_secondary),
            )));
        }
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}
