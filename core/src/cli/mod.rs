//! CLI 路由模块：自研轻量级命令行路由系统
//!
//! 包含参数解析 (argv)、命令路由 (route) 和主程序 (program) 三个子模块。
//! 以及 DSH 二创斜杠命令解析器 (dslash)、`--pipe` 有界标准输入读取 (pipe_input)。

pub mod argv;
pub mod dslash;
pub mod output;
pub mod pipe_input;
pub mod program;
pub mod repl;
pub mod route;

pub use argv::{parse_args, Args, CliMode, SUBCOMMANDS};
pub use dslash::{
    parse_dslash, publish_dslash_event, DshSlashCommand, DshSlashKind, DshSlashPayload, GamesCmd,
    MarketCmd, ModlensMode, PetAction, RouterCmd, RouterMode, ScheduleCmd, SidebarCmd,
    SlashParseError, UsageScope,
};
pub use output::{exit_with, print_json_err, print_json_ok, ExitCode};
pub use program::{print_top_help, Program, CLI_VERSION};
pub use route::{Route, RouteTable};
