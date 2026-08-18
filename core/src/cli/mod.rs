//! CLI 路由模块：自研轻量级命令行路由系统
//!
//! 包含参数解析 (argv)、命令路由 (route) 和主程序 (program) 三个子模块。

pub mod argv;
pub mod output;
pub mod program;
pub mod repl;
pub mod route;

pub use argv::{parse_args, Args, CliMode, SUBCOMMANDS};
pub use output::{exit_with, print_json_err, print_json_ok, ExitCode};
pub use program::{print_top_help, Program, CLI_VERSION};
pub use route::{Route, RouteTable};
