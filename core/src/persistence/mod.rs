//! 持久化模块：配置管理、会话持久化与任务队列

use std::path::PathBuf;
use std::sync::OnceLock;

/// 测试用配置根目录覆盖（单测注入后 ~/.marguerite 不落盘，防止并发测试互踩）
///
/// - 生产：未设置 → `Config::config_dir()` 走默认 ~/.marguerite
/// - 单测：`Config::set_config_dir_override(temp)` 注入后复用该目录
pub(crate) static OVERRIDE_CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();

pub mod config;
pub mod session;
pub mod task_queue;

pub use task_queue::{Task, TaskHandler, TaskQueue, TaskStatus};
