//! 工具模块：文件级版本快照 + 路径安全 + 内置原生工具集

pub mod baota;
pub mod baota_guard;
pub mod builtin;
pub mod code;
pub mod computer_use;
pub mod computer_use_loop;
pub mod file_change;
pub mod fs_path;
pub mod git;
pub mod skill_tools;
pub mod terminal;
pub mod versions;

pub use baota::register_baota_tools;
pub use builtin::{make_local_tool, register_builtin_tools};
pub use computer_use::register_computer_use_tools;
pub use computer_use_loop::{
    ui_action_to_dispatch_args, ActOutcome, Delayer, HumanizedTiming, LoopBackend, LoopPolicy,
    LoopReport, ObserveSnapshot, RealDelayer, RecordingDelayer, StepReport, UiAction, VisualLoop,
};
pub use file_change::{list_file_changes, record_file_change};
pub use fs_path::safe_resolve;
#[cfg(any(test, feature = "test-utils"))]
pub use git::RecordingGitRunner;
pub use git::{git_tool, GitRunner, SystemGitRunner};
pub use skill_tools::register_skill_tools;
