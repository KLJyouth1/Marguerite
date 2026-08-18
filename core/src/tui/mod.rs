//! TUI 交互界面模块
//!
//! 基于 Ratatui 的终端用户界面，包含主循环、聊天界面、输入处理和状态栏。

mod app;
pub mod bloom;
mod chat;
pub mod glyph;
mod input;
mod status;
pub mod workbench;

pub use app::App;
pub use chat::{ChatMessage, ChatPanel};
pub use input::{Action, InputPanel};
pub use status::StatusBar;
pub use workbench::WorkbenchApp;
