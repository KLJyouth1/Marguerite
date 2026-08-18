//! 状态栏
//!
//! 底部单行状态栏，显示模型名、Agent 状态、Token 用量、会话时间和连接状态。

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{Paragraph, Wrap},
    Frame,
};

/// 状态栏
#[derive(Debug, Clone)]
pub struct StatusBar {
    /// 模型名称
    pub model_name: String,
    /// Token 用量
    pub token_usage: String,
    /// 会话时间
    pub session_time: String,
    /// Agent 状态
    pub agent_state: String,
    /// 连接状态
    pub connection_status: String,
}

impl StatusBar {
    /// 创建新的状态栏
    pub fn new() -> Self {
        Self {
            model_name: "deepseek-v4-pro".to_string(),
            token_usage: "0".to_string(),
            session_time: "00:00".to_string(),
            agent_state: "Ready".to_string(),
            connection_status: "●".to_string(),
        }
    }

    /// 渲染状态栏
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let left = format!(" {} | {} ", self.model_name, self.agent_state);
        let right = format!(
            " {} | {} | {} ",
            self.token_usage, self.session_time, self.connection_status
        );

        // 计算中间填充空格，使左右内容对齐
        let left_width = left.len() as u16;
        let right_width = right.len() as u16;
        let padding = area.width.saturating_sub(left_width + right_width + 2);
        let padding_str = " ".repeat(padding.saturating_sub(1) as usize);

        let full_line = format!("{}{}{}", left, padding_str, right);

        let line = Line::from(vec![Span::styled(
            full_line,
            Style::default().fg(Color::White).bg(Color::Black),
        )]);

        let paragraph = Paragraph::new(Text::from(vec![line]))
            .style(Style::default().bg(Color::Black))
            .wrap(Wrap { trim: false });

        frame.render_widget(paragraph, area);
    }

    /// 更新状态栏信息
    pub fn update(&mut self, model: &str, tokens: &str, time: &str, state: &str, conn: &str) {
        self.model_name = model.to_string();
        self.token_usage = tokens.to_string();
        self.session_time = time.to_string();
        self.agent_state = state.to_string();
        self.connection_status = conn.to_string();
    }
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let bar = StatusBar::new();
        assert_eq!(bar.model_name, "deepseek-v4-pro");
        assert_eq!(bar.token_usage, "0");
        assert_eq!(bar.session_time, "00:00");
        assert_eq!(bar.agent_state, "Ready");
        assert_eq!(bar.connection_status, "●");
    }

    #[test]
    fn test_update() {
        let mut bar = StatusBar::new();
        bar.update("gpt-4", "1234", "01:23", "Running", "✓");
        assert_eq!(bar.model_name, "gpt-4");
        assert_eq!(bar.token_usage, "1234");
        assert_eq!(bar.session_time, "01:23");
        assert_eq!(bar.agent_state, "Running");
        assert_eq!(bar.connection_status, "✓");
    }

    fn draw(bar: &StatusBar, width: u16, height: u16) {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| bar.render(f, f.area())).unwrap();
    }

    #[test]
    fn test_render() {
        let mut bar = StatusBar::new();
        bar.update("gpt-4", "1234", "01:23", "Running", "✓");
        draw(&bar, 80, 1);
    }

    #[test]
    fn test_render_narrow() {
        let bar = StatusBar::new();
        draw(&bar, 15, 1);
    }
}
