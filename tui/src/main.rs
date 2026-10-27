//! Marguerite 终端前端（ratatui）。
//!
//! 与桌面端（egui）共享同一 Rust 引擎 `marguerite_core`：每条输入由编排内核
//! **自动**决定用 Solo / MAX 双带队 / 专家团队 / 集群 形态，前端不提供任何让用户
//! 手动选择编排形态 / 操轮子 Agent 的控件（形态只能由 agent 自动得出）。
//! 未配置 API Key 时优雅降级为离线回显（仍展示自动编排判定）。
//!
//! 顶层 `--help` / `--version` 行为与 core 二进制保持一致；默认启动 WorkbenchApp，
//! 仅当显式设置 `MARGUERITE_LEGACY_UI=1` 时回退到旧版单栏界面（仅排障）。

use anyhow::Result;
use marguerite_core::persistence::config::Config;
use marguerite_core::tui::{App, WorkbenchApp};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // 顶层发现（与 core 二进制一致，先于 TUI 启动）
    if args.iter().any(|a| a == "--help" || a == "-h") {
        marguerite_core::cli::print_top_help();
        std::process::exit(0);
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("marguerite {}", marguerite_core::cli::CLI_VERSION);
        std::process::exit(0);
    }

    // 会话入口叠加「用户选择的模型」（`models.json` active 档案，`model use` 写的就是它）：
    // 执行侧只认 `config.model.*`，不叠加则「切换了模型、TUI 里仍走旧 provider」。
    let config = marguerite_core::model::selector::apply_effective_model(
        &Config::load().unwrap_or_default(),
    );
    if std::env::var("MARGUERITE_LEGACY_UI").is_ok() {
        let mut app = App::with_config(config);
        app.run()
    } else {
        let mut app = WorkbenchApp::from_config(&config)?;
        app.run()
    }
}
