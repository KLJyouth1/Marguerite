//! TUI 主循环
//!
//! 终端初始化、事件循环、按键处理、布局渲染。

use std::io::stdout;
use std::path::Path;

use anyhow::Result;
use chrono::Local;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::Frame;
use ratatui::Terminal;

use crate::agent::runtime::AgentRuntime;
use crate::persistence::config::Config;
use crate::skills::loader::SkillLoader;

use crate::orchestration::Orchestrator;

use super::chat::ChatPanel;
use super::input::{Action, InputPanel};
use super::status::StatusBar;

/// TUI 应用主结构体
pub struct App {
    /// 聊天面板
    pub chat: ChatPanel,
    /// 输入面板
    pub input: InputPanel,
    /// 状态栏
    pub status: StatusBar,
    /// 是否退出
    pub should_quit: bool,
    /// 会话开始时间
    session_start: chrono::DateTime<Local>,
    /// Agent 运行时（配置了 API Key 时可用）
    runtime: Option<AgentRuntime>,
    /// tokio 运行时（同步桥接异步调用）
    tokio: Option<tokio::runtime::Runtime>,
    /// 应用配置
    config: Config,
}

impl App {
    /// 创建新的应用实例（使用默认配置）
    pub fn new() -> Self {
        Self::with_config(Config::default())
    }

    /// 使用指定配置创建应用实例
    ///
    /// 配置了 API Key 时初始化 AgentRuntime，普通消息将获得真实回复。
    pub fn with_config(config: Config) -> Self {
        let model_name = config.model.model_name.clone();
        let mut runtime = None;
        let mut tokio = None;
        if !config.model.api_key.is_empty() {
            let mut agent = AgentRuntime::new(config.clone());
            let _ = agent.init();
            runtime = Some(agent);
            tokio = Some(tokio::runtime::Runtime::new().expect("创建 tokio runtime 失败"));
        }
        let mut status = StatusBar::new();
        status.model_name = model_name;

        Self {
            chat: ChatPanel::new(),
            input: InputPanel::new(),
            status,
            should_quit: false,
            session_start: Local::now(),
            runtime,
            tokio,
            config,
        }
    }

    /// 主入口：初始化终端，进入事件循环
    pub fn run(&mut self) -> Result<()> {
        let mut terminal = self.setup_terminal()?;
        let result = self.event_loop(&mut terminal);
        self.restore_terminal()?;
        result
    }

    /// 初始化终端：启用 raw mode，进入 alternate screen
    fn setup_terminal(&self) -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout());
        let terminal = Terminal::new(backend)?;
        Ok(terminal)
    }

    /// 恢复终端：退出 alternate screen，禁用 raw mode
    fn restore_terminal(&self) -> Result<()> {
        disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;
        Ok(())
    }

    /// 事件循环
    fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        // 添加欢迎消息
        self.chat.add_message(
            "system",
            "Welcome to Marguerite! Type /help for available commands.",
        );

        while !self.should_quit {
            // 更新状态栏时间
            let elapsed = Local::now() - self.session_start;
            let minutes = elapsed.num_minutes();
            let seconds = elapsed.num_seconds() % 60;
            self.status.session_time = format!("{:02}:{:02}", minutes, seconds);

            // 绘制界面
            terminal.draw(|frame| {
                self.render(frame);
            })?;

            // 读取事件
            match event::read()? {
                Event::Key(key) => self.handle_key_event(key)?,
                Event::Resize(_w, _h) => self.resize_handler(),
                _ => {}
            }
        }

        Ok(())
    }

    /// 渲染界面布局
    fn render(&self, frame: &mut Frame) {
        let area = frame.area();
        let layout = Layout::vertical([
            Constraint::Min(1),    // 聊天区域
            Constraint::Length(4), // 输入区域（含边框和提示）
            Constraint::Length(1), // 状态栏
        ]);
        let areas = layout.split(area);

        self.chat.render(frame, areas[0]);
        self.input.render(frame, areas[1]);
        self.status.render(frame, areas[2]);
    }

    /// 处理按键事件
    fn handle_key_event(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            // 退出
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                self.should_quit = true;
            }
            KeyCode::Char('q') if !self.input.is_focused => {
                self.should_quit = true;
            }
            // 切换焦点
            KeyCode::Tab => {
                self.input.is_focused = !self.input.is_focused;
            }
            // 取消/返回
            KeyCode::Esc => {
                if !self.input.is_focused {
                    self.input.is_focused = true;
                }
            }
            // 滚动（聊天区聚焦时）
            KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => {
                if self.input.is_focused {
                    self.input.handle_key_event(key);
                } else {
                    match key.code {
                        KeyCode::Up => self.chat.scroll_up(),
                        KeyCode::Down => self.chat.scroll_down(),
                        KeyCode::PageUp => {
                            for _ in 0..10 {
                                self.chat.scroll_up();
                            }
                        }
                        KeyCode::PageDown => {
                            for _ in 0..10 {
                                self.chat.scroll_down();
                            }
                        }
                        _ => {}
                    }
                }
            }
            // 输入区按键
            _ => {
                if self.input.is_focused {
                    if let Action::Submit(content) = self.input.handle_key_event(key) {
                        self.on_submit(&content);
                    }
                }
            }
        }
        Ok(())
    }

    /// 处理提交的输入
    fn on_submit(&mut self, content: &str) {
        if content.is_empty() {
            return;
        }

        // 添加用户消息到聊天
        self.chat.add_message("user", content);

        // 处理 slash 命令
        if content.starts_with('/') {
            self.handle_slash_command(content);
            return;
        }

        // 普通消息 → 调用 AgentRuntime 获取真实回复
        match (&mut self.runtime, &self.tokio) {
            (Some(runtime), Some(rt)) => {
                let result = rt.block_on(runtime.process_input(content));
                match result {
                    Ok(resp) => {
                        let msg = resp.to_string();
                        self.chat.add_message("assistant", &msg);
                    }
                    Err(e) => {
                        self.chat
                            .add_message("system", &format!("调用模型失败: {e}"));
                    }
                }
            }
            _ => {
                // 离线模式：未配置 API Key。仍由引擎自动分析编排形态（只读展示），
                // 不调用任何 LLM，回显用户输入并标注自动编排判定（零手动编排 UI）。
                let orch = Orchestrator::new(self.config.clone());
                let a = orch.analyze(content);
                self.chat.add_message(
                    "assistant",
                    &format!(
                        "[离线·自动编排判定] 形态={} | {}\n回显: {}",
                        a.strategy.label(),
                        a.rationale,
                        content
                    ),
                );
            }
        }
    }

    /// 处理 slash 命令
    fn handle_slash_command(&mut self, cmd: &str) {
        match cmd {
            "/exit" | "/quit" => {
                self.should_quit = true;
            }
            "/clear" => {
                self.chat.clear();
            }
            "/new" => {
                self.chat.clear();
                if let Some(runtime) = &mut self.runtime {
                    runtime.reset();
                }
                self.chat.add_message("system", "已开始新会话");
            }
            "/help" => {
                self.chat.add_message(
                    "system",
                    "Available commands:\n\
                     \n\
                      /help     - Show this help message\n\
                      /clear    - Clear the chat\n\
                      /new      - Start a new conversation\n\
                      /model    - Show current model\n\
                      /skills   - List available skills\n\
                      /status   - Show current status\n\
                      /compress - Fold old messages to save tokens\n\
                      /exit     - Exit the application\n\
                      /quit     - Exit the application\n\
                     \n\
                     Keyboard shortcuts:\n\
                      Ctrl+C    - Exit\n\
                      Tab       - Toggle focus (input ↔ chat)\n\
                      Esc       - Return to input\n\
                      ↑/↓       - History navigation / chat scroll",
                );
            }
            "/model" => {
                let info = format!(
                    "当前模型: {} ({})\n路由规则: deepseek-v4-pro（默认/复杂推理）/ deepseek-v4-flash（轻任务，按意图自动切换）",
                    self.config.model.model_name, self.config.model.provider
                );
                self.chat.add_message("system", &info);
            }
            "/skills" => {
                let loader = SkillLoader::new(Path::new(&self.config.skills.skills_dir));
                match loader.load_all() {
                    Ok(skills) if skills.is_empty() => {
                        self.chat.add_message(
                            "system",
                            &format!("未找到技能（目录: {}）", self.config.skills.skills_dir),
                        );
                    }
                    Ok(skills) => {
                        let list: Vec<String> = skills
                            .iter()
                            .map(|s| format!("  {} v{} — {}", s.name, s.version, s.description))
                            .collect();
                        let msg = format!("已安装技能 ({}):\n{}", skills.len(), list.join("\n"));
                        self.chat.add_message("system", &msg);
                    }
                    Err(e) => {
                        self.chat
                            .add_message("system", &format!("加载技能失败: {e}"));
                    }
                }
            }
            "/status" => {
                let info = format!(
                    "Model: {} | State: {} | Tokens: {} | Session: {} | Connection: {}",
                    self.status.model_name,
                    self.status.agent_state,
                    self.status.token_usage,
                    self.status.session_time,
                    self.status.connection_status,
                );
                self.chat.add_message("system", &info);
            }
            "/compress" => {
                let total = self.chat.messages.len();
                if total <= 4 {
                    self.chat.add_message("system", "会话较短，无需压缩");
                    return;
                }
                // 保留最近 2 条消息，之前的折叠为摘要
                let folded = total - 2;
                let keep = self.chat.messages.split_off(folded);
                self.chat.messages.clear();
                self.chat
                    .add_message("system", &format!("已折叠 {folded} 条历史消息以节省 Token"));
                self.chat.messages.extend(keep);
            }
            _ => {
                self.chat.add_message(
                    "system",
                    &format!(
                        "Unknown command: {}. Type /help for available commands.",
                        cmd
                    ),
                );
            }
        }
    }

    /// 终端 resize 处理
    fn resize_handler(&mut self) {
        // ratatui 的 Terminal::draw 会自动处理 resize
        // 此处留空以备后续需要
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::backend::TestBackend;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_new() {
        let app = App::new();
        assert!(!app.should_quit);
        assert!(app.input.is_focused);
        assert_eq!(app.status.model_name, "deepseek-v4-pro");
    }

    #[test]
    fn test_ctrl_c_quits() {
        let mut app = App::new();
        let e = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        app.handle_key_event(e).unwrap();
        assert!(app.should_quit);
    }

    #[test]
    fn test_q_quits_only_when_input_not_focused() {
        let mut app = App::new();
        app.input.is_focused = false;
        app.handle_key_event(key(KeyCode::Char('q'))).unwrap();
        assert!(app.should_quit);

        let mut app = App::new();
        app.handle_key_event(key(KeyCode::Char('q'))).unwrap();
        assert!(!app.should_quit);
        assert_eq!(app.input.buffer, "q");
    }

    #[test]
    fn test_tab_toggles_focus() {
        let mut app = App::new();
        assert!(app.input.is_focused);
        app.handle_key_event(key(KeyCode::Tab)).unwrap();
        assert!(!app.input.is_focused);
        app.handle_key_event(key(KeyCode::Tab)).unwrap();
        assert!(app.input.is_focused);
    }

    #[test]
    fn test_esc_returns_to_input() {
        let mut app = App::new();
        app.input.is_focused = false;
        app.handle_key_event(key(KeyCode::Esc)).unwrap();
        assert!(app.input.is_focused);
    }

    #[test]
    fn test_scroll_keys_when_chat_focused() {
        let mut app = App::new();
        app.input.is_focused = false;
        for i in 0..20 {
            app.chat.add_message("user", &format!("msg {i}"));
        }
        app.handle_key_event(key(KeyCode::Up)).unwrap();
        assert!(app.chat.scroll_offset > 0);
        app.handle_key_event(key(KeyCode::Down)).unwrap();
        app.handle_key_event(key(KeyCode::PageUp)).unwrap();
        app.handle_key_event(key(KeyCode::PageDown)).unwrap();
    }

    #[test]
    fn test_scroll_keys_when_input_focused() {
        let mut app = App::new();
        app.handle_key_event(key(KeyCode::Up)).unwrap();
        app.handle_key_event(key(KeyCode::Down)).unwrap();
        app.handle_key_event(key(KeyCode::PageUp)).unwrap();
        app.handle_key_event(key(KeyCode::PageDown)).unwrap();
    }

    #[test]
    fn test_on_submit_flow() {
        // 无 API Key 时进入离线回显：由引擎自动分析编排形态（只读展示），
        // 不调用 LLM，回显用户输入并标注自动编排判定。
        let mut app = App::new();
        app.on_submit("hello world");
        assert_eq!(app.chat.messages.len(), 2);
        assert_eq!(app.chat.messages[0].role, "user");
        assert_eq!(app.chat.messages[0].content, "hello world");
        assert_eq!(app.chat.messages[1].role, "assistant");
        // 离线回显路径：标注自动编排判定，并回显原始输入
        assert!(app.chat.messages[1].content.contains("离线·自动编排判定"));
        assert!(app.chat.messages[1].content.contains("hello world"));
    }

    #[test]
    fn test_on_submit_empty_ignored() {
        let mut app = App::new();
        app.on_submit("");
        assert!(app.chat.messages.is_empty());
    }

    #[test]
    fn test_slash_commands() {
        let mut app = App::new();
        app.on_submit("/exit");
        assert!(app.should_quit);

        let mut app = App::new();
        app.on_submit("/clear");
        assert!(app.chat.messages.is_empty());

        let mut app = App::new();
        app.on_submit("/new");
        // 新会话包含系统提示
        assert_eq!(app.chat.messages.len(), 1);
        assert!(app.chat.messages[0].content.contains("新会话"));

        let mut app = App::new();
        app.on_submit("/help");
        assert!(app.chat.messages[1].content.contains("/compress"));

        let mut app = App::new();
        app.on_submit("/model");
        assert!(app.chat.messages[1].content.contains("deepseek-v4-pro"));

        let mut app = App::new();
        app.on_submit("/status");
        assert!(app.chat.messages[1].content.contains("Model:"));

        let mut app = App::new();
        app.on_submit("/unknown");
        assert!(app.chat.messages[1].content.contains("Unknown command"));
    }

    #[test]
    fn test_slash_skills_lists_skills() {
        let mut app = App::new();
        app.on_submit("/skills");
        // 默认技能目录存在时列出技能，否则提示未找到
        assert!(app.chat.messages[1].content.contains("技能"));
    }

    #[test]
    fn test_slash_skills_empty_dir() {
        let mut config = Config::default();
        config.skills.skills_dir = "__nonexistent_skills__".to_string();
        let mut app = App::with_config(config);
        app.on_submit("/skills");
        assert!(app.chat.messages[1].content.contains("未找到技能"));
    }

    #[test]
    fn test_slash_compress_folds_history() {
        let mut app = App::new();
        // 构造 6 条历史消息
        for i in 0..6 {
            app.chat.add_message("user", &format!("msg {i}"));
        }
        assert_eq!(app.chat.messages.len(), 6);
        app.on_submit("/compress");
        // 6 条历史 + /compress 命令本身 = 7 条，折叠 5 条，保留最近 2 条
        assert_eq!(app.chat.messages.len(), 3);
        assert!(app.chat.messages[0].content.contains("已折叠 5 条"));
        assert_eq!(app.chat.messages[1].content, "msg 5");
        assert_eq!(app.chat.messages[2].content, "/compress");
    }

    #[test]
    fn test_slash_compress_short_session() {
        let mut app = App::new();
        app.on_submit("/compress");
        assert!(app.chat.messages[1].content.contains("无需压缩"));
    }

    #[test]
    fn test_render_layout() {
        let mut app = App::new();
        app.chat.add_message("user", "hello");
        app.chat.add_message("assistant", "world");
        app.input.buffer = "typing...".to_string();
        app.status.session_time = "00:05".to_string();

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn test_resize_handler() {
        let mut app = App::new();
        app.resize_handler();
        assert!(!app.should_quit);
    }
}
