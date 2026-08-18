//! 聊天界面
//!
//! 显示消息列表，支持滚动、代码块高亮和思考过程显示。

use ratatui::{
    layout::{Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};

/// 聊天消息
#[derive(Debug, Clone)]
pub struct ChatMessage {
    /// 角色: "user", "assistant", "system", "tool"
    pub role: String,
    /// 消息内容
    pub content: String,
    /// 时间戳字符串
    pub timestamp: String,
    /// DeepSeek 思考过程
    pub thinking: Option<String>,
}

/// 聊天面板
#[derive(Debug)]
pub struct ChatPanel {
    /// 消息列表
    pub messages: Vec<ChatMessage>,
    /// 滚动偏移量
    pub scroll_offset: usize,
    /// 是否自动滚动到最新消息
    pub auto_scroll: bool,
}

impl ChatPanel {
    /// 创建新的聊天面板
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            scroll_offset: 0,
            auto_scroll: true,
        }
    }

    /// 添加消息
    pub fn add_message(&mut self, role: &str, content: &str) {
        let now = chrono::Local::now();
        self.messages.push(ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
            timestamp: now.format("%H:%M:%S").to_string(),
            thinking: None,
        });
        if self.auto_scroll {
            self.scroll_to_bottom();
        }
    }

    /// 添加带思考过程的消息
    pub fn add_message_with_thinking(
        &mut self,
        role: &str,
        content: &str,
        thinking: Option<String>,
    ) {
        let now = chrono::Local::now();
        self.messages.push(ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
            timestamp: now.format("%H:%M:%S").to_string(),
            thinking,
        });
        if self.auto_scroll {
            self.scroll_to_bottom();
        }
    }

    /// 滚动到底部
    fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    /// 向上滚动
    pub fn scroll_up(&mut self) {
        let max_scroll = self.content_height().saturating_sub(1);
        self.scroll_offset = self.scroll_offset.saturating_add(1).min(max_scroll);
        self.auto_scroll = false;
    }

    /// 向下滚动
    pub fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
        if self.scroll_offset == 0 {
            self.auto_scroll = true;
        }
    }

    /// 估算内容总行数
    fn content_height(&self) -> usize {
        self.messages
            .iter()
            .map(|m| {
                let mut lines = 2; // header + trailing blank
                if let Some(ref t) = m.thinking {
                    lines += t.lines().count();
                }
                lines += m.content.lines().count().max(1);
                lines
            })
            .sum()
    }

    /// 清空所有消息
    pub fn clear(&mut self) {
        self.messages.clear();
        self.scroll_offset = 0;
        self.auto_scroll = true;
    }

    /// 渲染聊天区域
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let mut lines: Vec<Line> = Vec::new();

        for msg in &self.messages {
            let (role_style, role_label) = match msg.role.as_str() {
                "user" => (Style::default().fg(Color::Blue), "You"),
                "assistant" => (Style::default().fg(Color::Green), "Assistant"),
                "system" => (Style::default().fg(Color::Gray), "System"),
                "tool" => (Style::default().fg(Color::Yellow), "Tool"),
                _ => (Style::default().fg(Color::White), msg.role.as_str()),
            };

            // 头部：角色 + 时间戳
            lines.push(Line::from(vec![
                Span::styled(format!("[{}] ", role_label), role_style),
                Span::styled(&msg.timestamp, Style::default().fg(Color::DarkGray)),
            ]));

            // 思考过程（灰色斜体）
            if let Some(ref thinking) = msg.thinking {
                for line in thinking.lines() {
                    lines.push(Line::from(vec![Span::styled(
                        format!("  {} ", line),
                        Style::default()
                            .fg(Color::Gray)
                            .add_modifier(Modifier::ITALIC),
                    )]));
                }
            }

            // 消息内容 — 代码块高亮
            let mut in_code = false;
            for content_line in msg.content.lines() {
                let trimmed = content_line.trim();
                if trimmed.starts_with("```") {
                    in_code = !in_code;
                    lines.push(Line::from(vec![Span::styled(
                        content_line.to_string(),
                        Style::default().fg(Color::Cyan),
                    )]));
                    continue;
                }

                if in_code {
                    lines.push(Line::from(vec![Span::styled(
                        format!("  {}", content_line),
                        Style::default().fg(Color::Cyan),
                    )]));
                } else {
                    lines.push(Line::from(vec![Span::raw(format!("  {}", content_line))]));
                }
            }

            // 消息间空行
            lines.push(Line::from(""));
        }

        // 如果没有消息，显示提示
        if lines.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                "  No messages yet. Start a conversation!",
                Style::default().fg(Color::DarkGray),
            )]));
        }

        let inner_area = area.inner(Margin {
            vertical: 1,
            horizontal: 1,
        });

        let paragraph = Paragraph::new(Text::from(lines))
            .block(Block::default().borders(Borders::ALL).title(" Chat "))
            .scroll((self.scroll_offset as u16, 0))
            .wrap(Wrap { trim: false });

        frame.render_widget(paragraph, area);

        // 滚动条
        let content_height = self.content_height();
        let visible_height = inner_area.height as usize;
        if content_height > visible_height {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓"));
            let max_scroll = content_height.saturating_sub(visible_height);
            let state =
                ScrollbarState::new(max_scroll).position(self.scroll_offset.min(max_scroll));
            frame.render_stateful_widget(scrollbar, area, &mut state.clone());
        }
    }
}

impl Default for ChatPanel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_message() {
        let mut panel = ChatPanel::new();
        panel.add_message("user", "hello");
        assert_eq!(panel.messages.len(), 1);
        assert_eq!(panel.messages[0].role, "user");
        assert_eq!(panel.messages[0].content, "hello");
        assert!(!panel.messages[0].timestamp.is_empty());
        assert!(panel.auto_scroll);
    }

    #[test]
    fn test_add_message_with_thinking() {
        let mut panel = ChatPanel::new();
        panel.add_message_with_thinking(
            "assistant",
            "response content",
            Some("thinking process".to_string()),
        );
        assert_eq!(panel.messages.len(), 1);
        assert_eq!(
            panel.messages[0].thinking,
            Some("thinking process".to_string())
        );
    }

    #[test]
    fn test_scroll() {
        let mut panel = ChatPanel::new();
        // 添加足够多的消息使内容可滚动
        for i in 0..20 {
            panel.add_message("user", &format!("message {}", i));
        }
        assert!(panel.auto_scroll);

        panel.scroll_up();
        assert!(!panel.auto_scroll);
        assert!(panel.scroll_offset > 0);

        panel.scroll_down();
        // 可能 scroll_offset 变为 0
        if panel.scroll_offset == 0 {
            assert!(panel.auto_scroll);
        }
    }

    #[test]
    fn test_clear() {
        let mut panel = ChatPanel::new();
        panel.add_message("user", "hello");
        panel.add_message("assistant", "world");
        assert_eq!(panel.messages.len(), 2);

        panel.clear();
        assert!(panel.messages.is_empty());
        assert_eq!(panel.scroll_offset, 0);
        assert!(panel.auto_scroll);
    }

    fn draw(panel: &ChatPanel, width: u16, height: u16) {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| panel.render(f, f.area())).unwrap();
    }

    #[test]
    fn test_render_empty() {
        draw(&ChatPanel::new(), 80, 24);
    }

    #[test]
    fn test_render_all_roles_and_code() {
        let mut panel = ChatPanel::new();
        panel.add_message("user", "hello");
        panel.add_message_with_thinking(
            "assistant",
            "answer\n```rust\nlet x = 1;\n```\ntail",
            Some("thinking...\nmore".to_string()),
        );
        panel.add_message("tool", "tool result");
        panel.add_message("system", "system note");
        panel.add_message("custom", "custom role");
        draw(&panel, 80, 24);
    }

    #[test]
    fn test_render_scroll_and_scrollbar() {
        let mut panel = ChatPanel::new();
        for i in 0..30 {
            panel.add_message("user", &format!("long message {i} {}", "x".repeat(40)));
        }
        panel.scroll_up();
        panel.scroll_up();
        draw(&panel, 60, 10);
    }

    #[test]
    fn test_content_height_multiline() {
        let mut panel = ChatPanel::new();
        panel.add_message_with_thinking("assistant", "line1\nline2", Some("think".to_string()));
        assert!(panel.content_height() >= 4);
    }
}
