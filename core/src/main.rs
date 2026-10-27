//! Marguerite CLI Workbench - 入口
//!
//! 创建 Program 实例并运行主循环，无参数时启动 TUI 模式。

use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Safe Mode（S10）：必须在**任何 Config 读取之前**注入隔离目录 ——
    // 一旦 `Config::load()` 先跑过，用户的 config.yaml 就已经被读进来了。
    // 三个目录（config/data/sidecar-log）必须一起换，只换其一仍会漏读用户数据。
    if marguerite_core::persistence::safe_mode::requested(&args) {
        match marguerite_core::persistence::safe_mode::activate() {
            Ok(paths) => {
                eprintln!(
                    "[safe-mode] 已启用隔离环境（不读写用户数据）: {}",
                    paths.root.display()
                );
            }
            Err(e) => {
                // 无法隔离 → 绝不降级为普通启动：那会让用户以为数据被保护了。
                eprintln!("[safe-mode] 启用失败，已中止：{e:#}");
                std::process::exit(2);
            }
        }
    }

    if args.len() == 1 {
        // 无参数，启动 TUI 模式（加载真实配置）
        //
        // 「用户选了哪个模型」由 `models.json` 的 active 档案决定（`model use` / 控制台
        // `llmProfile.activate` 写的就是它），而执行侧只认 `config.model.*`。不叠加则
        // 会出现「切换了模型、TUI 里回答仍来自旧 provider」——在**会话入口**叠加一次，
        // 让三端（CLI / TUI / 网关）看到同一个生效模型。
        let config = marguerite_core::model::selector::apply_effective_model(
            &marguerite_core::persistence::config::Config::load().unwrap_or_default(),
        );
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
