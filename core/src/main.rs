//! Marguerite CLI Workbench - 入口
//!
//! 创建 Program 实例并运行主循环，无参数时启动 TUI 模式。

use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() == 1 {
        // 无参数，启动 TUI 模式（加载真实配置）
        let config = marguerite_core::persistence::config::Config::load().unwrap_or_default();
        // 按配置初始化嵌入后端（默认 local；backend=model 加载真实模型，失败回退）
        marguerite_core::memory::EmbeddingProvider::configure(&config);
        if std::env::var("MARGUERITE_LEGACY_UI").is_ok() {
            let mut app = marguerite_core::tui::App::with_config(config);
            app.run()
        } else {
            let mut app = marguerite_core::tui::WorkbenchApp::from_config(&config)?;
            app.run()
        }
    } else {
        let mut program = marguerite_core::cli::Program::new();
        program.run()
    }
}
