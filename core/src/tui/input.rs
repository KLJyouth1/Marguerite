//! 输入处理
//!
//! 管理输入缓冲区、光标位置、历史记录和 slash 命令补全。

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

/// 输入操作结果
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// 提交输入内容
    Submit(String),
    /// Tab 补全
    TabComplete,
    /// 历史记录上翻
    HistoryUp,
    /// 历史记录下翻
    HistoryDown,
    /// 无操作
    None,
}

/// 输入面板
#[derive(Debug)]
pub struct InputPanel {
    /// 输入缓冲区
    pub buffer: String,
    /// 光标位置
    pub cursor_position: usize,
    /// 历史记录
    pub history: Vec<String>,
    /// 当前历史记录索引
    pub history_index: Option<usize>,
    /// 是否聚焦
    pub is_focused: bool,
    /// slash 命令列表
    pub slash_commands: Vec<String>,
}

impl InputPanel {
    /// 创建新的输入面板
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor_position: 0,
            history: Vec::new(),
            history_index: None,
            is_focused: true,
            slash_commands: vec![
                "/help".to_string(),
                "/model".to_string(),
                "/skills".to_string(),
                "/new".to_string(),
                "/compress".to_string(),
                "/status".to_string(),
                "/exit".to_string(),
            ],
        }
    }

    /// 渲染输入区域
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let title = if self.is_focused {
            " Input (focused) "
        } else {
            " Input "
        };
        let block = Block::default().borders(Borders::ALL).title(title);

        let inner = block.inner(area);

        // 提示符 + 输入内容
        let prompt = Span::styled("> ", Style::default().fg(Color::Green));
        let input_text = if self.buffer.is_empty() && self.is_focused {
            Span::styled("Type a message...", Style::default().fg(Color::DarkGray))
        } else {
            Span::raw(&self.buffer)
        };

        let line = Line::from(vec![prompt, input_text]);
        let paragraph = Paragraph::new(Text::from(vec![line]))
            .block(block)
            .wrap(Wrap { trim: false });

        frame.render_widget(paragraph, area);

        // 光标位置
        if self.is_focused {
            let cursor_x = inner.x + 2 + self.cursor_position as u16; // 2 for "> "
            let cursor_y = inner.y;
            frame.set_cursor_position((cursor_x.min(inner.right().saturating_sub(1)), cursor_y));
        }

        // slash 命令补全提示
        if self.buffer.starts_with('/') && !self.is_exact_slash_match() {
            let matches: Vec<&String> = self
                .slash_commands
                .iter()
                .filter(|cmd| cmd.starts_with(&self.buffer))
                .collect();
            if !matches.is_empty() {
                let hint = format!(
                    "  {}",
                    matches
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join("  ")
                );
                let hint_span = Span::styled(hint, Style::default().fg(Color::DarkGray));
                // 显示提示在输入区域下方（但在 block 内）
                let hint_paragraph = Paragraph::new(Text::from(vec![Line::from(hint_span)]));
                if area.height > 2 {
                    let hint_area = Rect {
                        x: area.x + 1,
                        y: inner.y + 1,
                        width: area.width.saturating_sub(2),
                        height: 1,
                    };
                    frame.render_widget(hint_paragraph, hint_area);
                }
            }
        }
    }

    /// 检查当前输入是否为精确的 slash 命令匹配
    fn is_exact_slash_match(&self) -> bool {
        self.slash_commands.iter().any(|cmd| cmd == &self.buffer)
    }

    /// 插入字符
    pub fn insert_char(&mut self, c: char) {
        if self.cursor_position >= self.buffer.len() {
            self.buffer.push(c);
        } else {
            self.buffer.insert(self.cursor_position, c);
        }
        self.cursor_position += c.len_utf8();
        self.history_index = None;
    }

    /// 删除字符（Backspace）
    pub fn delete_char(&mut self) {
        if self.cursor_position > 0 && !self.buffer.is_empty() {
            let prev = self.cursor_position - 1;
            self.buffer.remove(prev);
            self.cursor_position = prev;
        }
    }

    /// 光标左移
    pub fn cursor_left(&mut self) {
        if self.cursor_position > 0 {
            self.cursor_position -= 1;
        }
    }

    /// 光标右移
    pub fn cursor_right(&mut self) {
        if self.cursor_position < self.buffer.len() {
            self.cursor_position += 1;
        }
    }

    /// 提交输入，返回内容并清空缓冲区
    pub fn submit(&mut self) -> String {
        let content = self.buffer.clone();
        if !content.is_empty() {
            self.history.push(content.clone());
        }
        self.buffer.clear();
        self.cursor_position = 0;
        self.history_index = None;
        content
    }

    /// 历史记录上翻
    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.history_index {
            Some(i) => {
                if i > 0 {
                    i - 1
                } else {
                    return;
                }
            }
            None => self.history.len() - 1,
        };
        self.history_index = Some(idx);
        self.buffer = self.history[idx].clone();
        self.cursor_position = self.buffer.len();
    }

    /// 历史记录下翻
    pub fn history_down(&mut self) {
        if let Some(i) = self.history_index {
            if i + 1 < self.history.len() {
                let idx = i + 1;
                self.history_index = Some(idx);
                self.buffer = self.history[idx].clone();
            } else {
                self.history_index = None;
                self.buffer.clear();
            }
            self.cursor_position = self.buffer.len();
        }
    }

    /// 获取当前输入文本
    pub fn get_input_text(&self) -> &str {
        &self.buffer
    }

    /// 处理输入按键事件
    pub fn handle_key_event(&mut self, key: KeyEvent) -> Action {
        if !self.is_focused {
            return Action::None;
        }

        match key.code {
            KeyCode::Enter => {
                let content = self.submit();
                if content.is_empty() {
                    Action::None
                } else {
                    Action::Submit(content)
                }
            }
            KeyCode::Char(c) => {
                self.insert_char(c);
                Action::None
            }
            KeyCode::Backspace => {
                self.delete_char();
                Action::None
            }
            KeyCode::Left => {
                self.cursor_left();
                Action::None
            }
            KeyCode::Right => {
                self.cursor_right();
                Action::None
            }
            KeyCode::Up => {
                self.history_up();
                Action::HistoryUp
            }
            KeyCode::Down => {
                self.history_down();
                Action::HistoryDown
            }
            KeyCode::Tab => Action::TabComplete,
            KeyCode::Esc => Action::None,
            _ => Action::None,
        }
    }
}

impl Default for InputPanel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn test_insert_char() {
        let mut panel = InputPanel::new();
        panel.insert_char('h');
        panel.insert_char('i');
        assert_eq!(panel.buffer, "hi");
        assert_eq!(panel.cursor_position, 2);
    }

    #[test]
    fn test_delete_char() {
        let mut panel = InputPanel::new();
        panel.insert_char('h');
        panel.insert_char('i');
        panel.delete_char();
        assert_eq!(panel.buffer, "h");
        assert_eq!(panel.cursor_position, 1);
    }

    #[test]
    fn test_cursor_movement() {
        let mut panel = InputPanel::new();
        panel.insert_char('a');
        panel.insert_char('b');
        panel.insert_char('c');
        assert_eq!(panel.cursor_position, 3);

        panel.cursor_left();
        assert_eq!(panel.cursor_position, 2);

        panel.cursor_right();
        assert_eq!(panel.cursor_position, 3);

        // 边界情况
        panel.cursor_right();
        assert_eq!(panel.cursor_position, 3); // 不应超出

        panel.cursor_left();
        panel.cursor_left();
        panel.cursor_left();
        assert_eq!(panel.cursor_position, 0);

        panel.cursor_left();
        assert_eq!(panel.cursor_position, 0); // 不应低于 0
    }

    #[test]
    fn test_submit() {
        let mut panel = InputPanel::new();
        panel.insert_char('h');
        panel.insert_char('i');

        let content = panel.submit();
        assert_eq!(content, "hi");
        assert!(panel.buffer.is_empty());
        assert_eq!(panel.cursor_position, 0);
        assert_eq!(panel.history.len(), 1);
        assert_eq!(panel.history[0], "hi");
    }

    #[test]
    fn test_submit_empty() {
        let mut panel = InputPanel::new();
        let content = panel.submit();
        assert_eq!(content, "");
        assert!(panel.history.is_empty());
    }

    #[test]
    fn test_history() {
        let mut panel = InputPanel::new();
        panel.insert_char('a');
        panel.submit();
        panel.insert_char('b');
        panel.submit();
        panel.insert_char('c');
        panel.submit();

        assert_eq!(panel.history.len(), 3);

        // 上翻
        panel.history_up();
        assert_eq!(panel.buffer, "c");

        panel.history_up();
        assert_eq!(panel.buffer, "b");

        panel.history_up();
        assert_eq!(panel.buffer, "a");

        // 边界：不能再上翻
        panel.history_up();
        assert_eq!(panel.buffer, "a");

        // 下翻
        panel.history_down();
        assert_eq!(panel.buffer, "b");

        panel.history_down();
        assert_eq!(panel.buffer, "c");

        panel.history_down();
        assert!(panel.buffer.is_empty());
        assert!(panel.history_index.is_none());
    }

    #[test]
    fn test_slash_commands() {
        let panel = InputPanel::new();
        assert!(panel.slash_commands.contains(&"/help".to_string()));
        assert!(panel.slash_commands.contains(&"/exit".to_string()));
        assert_eq!(panel.slash_commands.len(), 7);
    }

    #[test]
    fn test_handle_key_event() {
        let mut panel = InputPanel::new();

        // 输入字符
        let key = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE);
        assert_eq!(panel.handle_key_event(key), Action::None);
        assert_eq!(panel.buffer, "h");

        // 回车提交
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        match panel.handle_key_event(key) {
            Action::Submit(content) => assert_eq!(content, "h"),
            _ => panic!("Expected Submit"),
        }
        assert!(panel.buffer.is_empty());
    }

    fn draw(panel: &InputPanel, width: u16, height: u16) {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| panel.render(f, f.area())).unwrap();
    }

    #[test]
    fn test_render_focused_empty() {
        draw(&InputPanel::new(), 80, 4);
    }

    #[test]
    fn test_render_unfocused_with_text() {
        let mut panel = InputPanel::new();
        panel.is_focused = false;
        panel.buffer = "hello".to_string();
        panel.cursor_position = 5;
        draw(&panel, 80, 4);
    }

    #[test]
    fn test_render_slash_hint() {
        let mut panel = InputPanel::new();
        panel.buffer = "/h".to_string();
        panel.cursor_position = 2;
        draw(&panel, 80, 4);
    }

    #[test]
    fn test_render_slash_no_match() {
        let mut panel = InputPanel::new();
        panel.buffer = "/zz".to_string();
        draw(&panel, 80, 4);
    }

    #[test]
    fn test_render_exact_slash_no_hint() {
        let mut panel = InputPanel::new();
        panel.buffer = "/help".to_string();
        draw(&panel, 80, 4);
    }

    #[test]
    fn test_render_tiny_area() {
        draw(&InputPanel::new(), 10, 1);
    }

    #[test]
    fn test_insert_char_middle() {
        let mut panel = InputPanel::new();
        panel.buffer = "ab".to_string();
        panel.cursor_position = 1;
        panel.insert_char('X');
        assert_eq!(panel.buffer, "aXb");
        assert_eq!(panel.cursor_position, 2);
    }

    #[test]
    fn test_delete_char_at_start() {
        let mut panel = InputPanel::new();
        panel.insert_char('a');
        panel.cursor_position = 0;
        panel.delete_char();
        assert_eq!(panel.buffer, "a");
    }

    #[test]
    fn test_backspace_and_left_right_keys() {
        let mut panel = InputPanel::new();
        panel.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        panel.handle_key_event(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(panel.cursor_position, 0);
        panel.handle_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(panel.cursor_position, 1);
        panel.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert!(panel.buffer.is_empty());
    }

    #[test]
    fn test_history_up_down_keys() {
        let mut panel = InputPanel::new();
        panel.handle_key_event(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        panel.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        panel.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(panel.buffer, "a");
        panel.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert!(panel.buffer.is_empty());
    }

    #[test]
    fn test_tab_and_unfocused_keys() {
        let mut panel = InputPanel::new();
        assert_eq!(
            panel.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            Action::TabComplete
        );
        panel.is_focused = false;
        assert_eq!(
            panel.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
            Action::None
        );
        assert!(panel.buffer.is_empty());
    }

    #[test]
    fn test_history_up_empty() {
        let mut panel = InputPanel::new();
        panel.history_up();
        assert!(panel.buffer.is_empty());
    }
}
