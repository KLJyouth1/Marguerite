//! 工具模块：文件级版本快照 + 路径安全 + 内置原生工具集

pub mod baota;
pub mod blender;
pub mod baota_guard;
pub mod builtin;
pub mod audio;
pub mod code;
pub mod code_video;
pub mod computer_use;
pub mod computer_use_loop;
pub mod file_change;
pub mod ffmpeg_bin;
pub mod fs_path;
pub mod git;
pub mod image_gen;
pub mod macros;
pub mod manim_video;
pub mod media;
pub mod ooxml;
pub mod patch;
pub mod skill_tools;
pub mod ssh;
pub mod structured_output;
pub mod terminal;
pub mod terminal_backend;
pub mod trash;
pub mod versions;
pub mod video_gen;
/// 工作台面板薄工具（P0-1b）：只包既有 gateway RPC，不新增后端契约。
pub mod workbench;
pub mod web_video;

pub use baota::register_baota_tools;
pub use builtin::{make_local_tool, register_builtin_tools, register_builtin_tools_with_roots};
pub use computer_use::register_computer_use_tools;
pub use computer_use_loop::{
    ui_action_to_dispatch_args, ActOutcome, Delayer, HumanizedTiming, LoopBackend, LoopPolicy,
    LoopReport, ObserveSnapshot, RealDelayer, RecordingDelayer, StepReport, UiAction, VisualLoop,
};
pub use file_change::{list_file_changes, record_file_change};
pub use fs_path::safe_resolve;
#[cfg(any(test, feature = "test-utils"))]
pub use git::RecordingGitRunner;
pub use git::{
    git_tool, GitRunner, SystemGitRunner,
};
pub use blender::{discover_blender_binary, BlenderRunner, BlenderTool, SystemBlenderRunner};
pub use media::{MediaRunner, MediaTool, SystemMediaRunner};
pub use skill_tools::register_skill_tools;
pub use ssh::register_ssh_tools;
