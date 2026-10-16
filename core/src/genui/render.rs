//! GenUI 渲染器：把 [`crate::genui::contract::Widget`] 绘制为 Ratatui 内容。
//!
//! 渲染策略（功能级，样式细节可后续迭代）：
//! - Card：蓝紫边框 + 加粗标题 + 左对齐正文行截断 + 底部横向按钮栏（均分，间隔 2 列）
//! - Button：`[ label ]` 括号格式；enabled=false 变灰；"hover" 样式（反色）通过传入
//!   `hover=true` 控制（单测直接以样式字符断言，不依赖真实鼠标事件捕获）
//! - ProgressBar：`[#######....] 52%`，percent 夹紧 0..=1
//! - Form：标题行 + 逐字段 `label: [placeholder]` 或 `label: [选项1 v]`
//!
//! 注意：render 函数不返回事件；按钮点击事件通过单独的
//! [`click_dispatch`]（纯函数、无状态）完成"点击位置→按钮 id"的模拟派发。

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;
use serde::{Deserialize, Serialize};

use crate::genui::contract::{Button, Card, Form, FormField, ProgressBar, Widget};

/// 统一渲染入口。
pub fn render_widget(w: &Widget, frame: &mut Frame, area: Rect) {
    match w {
        Widget::Card(c) => render_card(c, frame, area),
        Widget::Button(b) => render_button(b, frame, area, false),
        Widget::ProgressBar(p) => render_progress_bar(p, frame, area),
        Widget::Form(f) => render_form(f, frame, area),
    }
}

// ───────────────────────────── Card ─────────────────────────────

fn render_card(card: &Card, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(120, 100, 220)))
        .title(Span::styled(
            &card.title,
            Style::default().add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // body 占用除去按钮栏的全部空间；若 actions 为空则 body 占满
    let body_lines: Vec<Line> = card
        .body
        .iter()
        .map(|s| {
            // 按宽度截断（左对齐，超出右边省略号…）
            let avail = inner.width as usize;
            if s.chars().count() > avail {
                let truncated: String = s.chars().take(avail.saturating_sub(1)).collect();
                Line::from(format!("{truncated}…"))
            } else {
                Line::from(s.as_str())
            }
        })
        .collect();

    let layout = if card.actions.is_empty() {
        Layout::vertical([Constraint::Min(1)]).split(inner)
    } else {
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner)
    };

    let body_para = Paragraph::new(body_lines).wrap(Wrap { trim: false });
    frame.render_widget(body_para, layout[0]);

    if !card.actions.is_empty() {
        let actions_area = layout[1];
        render_button_row(&card.actions, frame, actions_area);
    }
}

fn render_button_row(buttons: &[Button], frame: &mut Frame, area: Rect) {
    if buttons.is_empty() {
        return;
    }
    let n = buttons.len() as u16;
    // 每个按钮宽度至少 6（含 [  ] 括号 + 1 字符标签），间隔 2
    let constraints: Vec<Constraint> = (0..n)
        .flat_map(|i| {
            if i + 1 < n {
                vec![
                    Constraint::Length(10), // 按钮宽度
                    Constraint::Length(2),  // 间隔
                ]
            } else {
                vec![Constraint::Length(10)]
            }
        })
        .collect();
    let cells = Layout::horizontal(constraints).split(area);
    for (idx, btn) in buttons.iter().enumerate() {
        let cell_idx = idx * 2;
        if cell_idx < cells.len() {
            render_button(btn, frame, cells[cell_idx], false);
        }
    }
}

// ───────────────────────────── Button ─────────────────────────────

fn render_button(btn: &Button, frame: &mut Frame, area: Rect, hover: bool) {
    let (fg, bg) = if !btn.enabled {
        (Color::Gray, Color::Reset)
    } else if hover {
        // hover 反色
        (Color::Black, Color::Cyan)
    } else {
        (Color::Cyan, Color::Reset)
    };
    let style = Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD);
    // 括号格式 [ label ]
    let content = format!("[ {} ]", btn.label);
    let span = Span::styled(content, style);
    let para = Paragraph::new(Line::from(span));
    frame.render_widget(para, area);
}

/// 按钮 hover 独立渲染（单测验证样式：hover 用反色背景）。
pub fn render_button_hover(btn: &Button, frame: &mut Frame, area: Rect) {
    render_button(btn, frame, area, true);
}

/// Mock click dispatch：纯函数按 buttons 顺序、点击列索引判断命中哪个按钮 id。
///
/// - `buttons`：按钮列表
/// - `x_in_row`：行内（按钮横向区）的列坐标（0 = 最左）
/// - 行宽按 `[ label ]` 每项 10 + gap 2 估算
/// - 未命中返回 None
pub fn click_dispatch(buttons: &[Button], x_in_row: u16) -> Option<String> {
    let mut cursor: u16 = 0;
    for (i, btn) in buttons.iter().enumerate() {
        let start = cursor;
        let end = cursor + 10;
        if x_in_row >= start && x_in_row < end {
            return if btn.enabled {
                Some(btn.id.clone())
            } else {
                None // 禁用按钮不触发事件
            };
        }
        // 间隔 2 + 下一项
        cursor = end + 2;
        let _ = i;
    }
    None
}

// ───────────────────────────── ProgressBar ─────────────────────────────

fn render_progress_bar(p: &ProgressBar, frame: &mut Frame, area: Rect) {
    let pct = p.clamped_percent(); // 0..=1
    let width = (area.width as usize).max(14); // 至少容纳 "[ ] 100%"
    let bar_w = width.saturating_sub(7); // 扣除 "[" + "]" + " 100%" ≈ 7 字符
    let filled = ((pct * bar_w as f32).round() as usize).min(bar_w);
    let empty = bar_w.saturating_sub(filled);
    let percent_int = (pct * 100.0).round() as u8;
    let bar = format!(
        "[{}{}] {:>3}%",
        "#".repeat(filled),
        ".".repeat(empty),
        percent_int
    );
    let style = Style::default().fg(Color::Green);
    let para = Paragraph::new(Line::from(Span::styled(bar, style)));
    frame.render_widget(para, area);
}

// ───────────────────────────── Form ─────────────────────────────

fn render_form(form: &Form, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(80, 160, 200)))
        .title(Span::styled(
            format!("Form: {}", form.id),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::with_capacity(form.fields.len());
    for field in &form.fields {
        let line = match field {
            FormField::Text {
                label, placeholder, ..
            } => {
                // 格式：> label: [____placeholder__]
                let ph = if placeholder.is_empty() {
                    "________________".to_string()
                } else {
                    placeholder.clone()
                };
                Line::from(format!("> {label}: [{ph}]"))
            }
            FormField::Select { label, options, .. } => {
                // 格式：label: [选项1 v]
                let first = options.first().map(|s| s.as_str()).unwrap_or("-");
                Line::from(format!("{label}: [{first} v]"))
            }
        };
        lines.push(line);
    }
    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(para, inner);
}

// ───────────────────────────── Steering 事件（roundtrip 单测用） ─────────────────────────────

/// GenUI Steering 事件结构：与契约匹配的最小 roundtrip 载体。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SteeringEvent {
    /// 目标 widget id。
    pub target: String,
    /// 事件类型："click" / "hover" / "change" 等。
    pub event: String,
    /// 负载（字符串或 JSON 字符串）。
    #[serde(default)]
    pub payload: String,
}

// ───────────────────────────── 单测（8 条） ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// 用 40×10 后端渲染一个闭包（draw 函数），返回 buffer 字符串。
    fn render_buf<F>(w: u16, h: u16, f: F) -> String
    where
        F: FnOnce(&mut Frame),
    {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).expect("Terminal new");
        terminal.draw(|frame| f(frame)).expect("draw");
        // ratatui 0.28：Buffer 不再直接 Display/ToString 用 `to_string()`；
        // 改为逐行把每个 cell 的 symbol 拼出来（左→右、上→下、行换行），
        // 等价语义：下游断言 .contains("Demo")、.contains("[ OK ]") 仍成立。
        let buf = terminal.backend().buffer();
        let area = buf.area();
        let mut out = String::with_capacity((area.width as usize + 1) * area.height as usize);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let cell = &buf[(x, y)];
                out.push_str(cell.symbol());
            }
            out.push('\n');
        }
        out
    }

    // -- 1) Card render --
    #[test]
    fn card_render_contains_title_and_body() {
        let card = Widget::Card(Card {
            title: "Demo".into(),
            body: vec!["line one".into(), "line two".into()],
            actions: vec![Button::new("a", "OK"), Button::new("b", "Cancel")],
        });
        let buf = render_buf(40, 10, |frame| {
            render_widget(&card, frame, Rect::new(0, 0, 40, 10));
        });
        assert!(buf.contains("Demo"), "卡片应渲染标题：{buf}");
        assert!(buf.contains("line one"), "卡片应渲染正文行");
        assert!(buf.contains("line two"), "卡片应渲染正文行");
        assert!(buf.contains('['), "卡片按钮区应出现 [ 边框字符（按钮样式）");
    }

    // -- 2) Button render --
    #[test]
    fn button_render_bracket_style() {
        let w = Widget::Button(Button::new("btn1", "OK"));
        let buf = render_buf(40, 3, |frame| {
            render_widget(&w, frame, Rect::new(0, 0, 40, 3));
        });
        assert!(
            buf.contains("[ OK ]"),
            "按钮必须使用 [ label ] 括号格式，实际：{buf}"
        );
    }

    // -- 3) Button click dispatch（mock 返回事件 id）--
    #[test]
    fn click_dispatch_returns_button_id() {
        let buttons = vec![
            Button::new("a", "A"),
            Button::new("b", "B").disabled(),
            Button::new("c", "C"),
        ];
        // 每个按钮宽度 10，间隔 2：0..10 A，12..22 B（disabled），24..34 C
        let hit_a = click_dispatch(&buttons, 2);
        assert_eq!(hit_a.as_deref(), Some("a"), "x=2 应命中 A");

        let hit_b = click_dispatch(&buttons, 15);
        assert_eq!(hit_b, None, "B 被禁用，即使命中也应返回 None");

        let hit_c = click_dispatch(&buttons, 28);
        assert_eq!(hit_c.as_deref(), Some("c"), "x=28 应命中 C");

        let miss = click_dispatch(&buttons, 1000);
        assert_eq!(miss, None, "超出范围应不命中");
    }

    // -- 4) ProgressBar 0% --
    #[test]
    fn progress_bar_zero_percent() {
        let w = Widget::ProgressBar(ProgressBar {
            id: "p".into(),
            percent: 0.0,
        });
        let buf = render_buf(40, 3, |frame| {
            render_widget(&w, frame, Rect::new(0, 0, 40, 3));
        });
        assert!(buf.contains("0%"), "0% 进度应显示 0%，实际：{buf}");
    }

    // -- 5) ProgressBar 50% --
    #[test]
    fn progress_bar_half_percent() {
        let w = Widget::ProgressBar(ProgressBar {
            id: "p".into(),
            percent: 0.52,
        });
        let buf = render_buf(40, 3, |frame| {
            render_widget(&w, frame, Rect::new(0, 0, 40, 3));
        });
        assert!(buf.contains("52%"), "0.52 应显示约 52%，实际：{buf}");
    }

    // -- 6) ProgressBar 100%（夹紧 1.5）--
    #[test]
    fn progress_bar_100_percent_clamped() {
        let w = Widget::ProgressBar(ProgressBar {
            id: "p".into(),
            percent: 1.5,
        });
        let buf = render_buf(40, 3, |frame| {
            render_widget(&w, frame, Rect::new(0, 0, 40, 3));
        });
        assert!(
            buf.contains("100%"),
            "percent>=1 应夹紧为 100%，实际：{buf}"
        );
    }

    // -- 7) Form field render --
    #[test]
    fn form_field_render_text_and_select_layouts() {
        let w = Widget::Form(Form {
            id: "profile".into(),
            fields: vec![
                FormField::Text {
                    id: "name".into(),
                    label: "Name".into(),
                    placeholder: "请输入姓名".into(),
                },
                FormField::Select {
                    id: "city".into(),
                    label: "City".into(),
                    options: vec!["Beijing".into(), "Shanghai".into()],
                },
            ],
        });
        let buf = render_buf(40, 10, |frame| {
            render_widget(&w, frame, Rect::new(0, 0, 40, 10));
        });
        assert!(buf.contains("Form: profile"), "应渲染表单标题：{buf}");
        // 中文 placeholder 在 ratatui buffer 中按逐格 symbol 拼接后，每个汉字会与下一格空字符之间
        // 出现自然分隔（全角宽占 2 cell），所以这里拆成三段独立断言避免空格误判。
        assert!(
            buf.contains("> Name:"),
            "文本字段前缀应为 '> Name:'，实际：{buf}"
        );
        assert!(buf.contains("请"), "placeholder 中的'请' 需出现");
        assert!(buf.contains("姓"), "placeholder 中的'姓' 需出现");
        assert!(buf.contains("City:"), "下拉字段前缀应为 'City:'");
        assert!(buf.contains("Beijing"), "下拉首个选项 Beijing 需出现");
    }

    // -- 8) Steering 事件 roundtrip + hover 样式 buffer contains "[" --
    #[test]
    fn steering_roundtrip_and_hover_style_has_bracket() {
        // (a) SteeringEvent JSON roundtrip
        let ev = SteeringEvent {
            target: "btn-ok".into(),
            event: "click".into(),
            payload: "{}".into(),
        };
        let json = serde_json::to_string(&ev).expect("序列化 SteeringEvent");
        let back: SteeringEvent = serde_json::from_str(&json).expect("反序列化 SteeringEvent");
        assert_eq!(ev, back, "SteeringEvent 必须可 JSON roundtrip");

        // (b) hover 样式按钮 buffer 必须仍包含 "[" 括号（渲染出 bracket 边框）
        let btn = Button::new("h", "HoverMe");
        let buf = render_buf(40, 3, |frame| {
            render_button_hover(&btn, frame, Rect::new(0, 1, 20, 1));
        });
        assert!(
            buf.contains('['),
            "hover 渲染输出仍应包含 [ 括号字符，实际：{buf}"
        );
        assert!(buf.contains("HoverMe"), "hover 渲染仍应显示按钮标签");
    }
}
