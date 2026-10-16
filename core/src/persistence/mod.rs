//! 持久化模块：配置管理、会话持久化与任务队列

use std::cell::RefCell;
use std::path::PathBuf;

// 测试用配置根目录覆盖（单测注入后 ~/.marguerite 不落盘，防止并发测试互踩）。
//
// 采用**线程局部**存储：并行测试各跑在独立配置目录、互不串扰。
// - 生产：未设置 → `Config::config_dir()` 走默认 ~/.marguerite
// - 单测：`Config::set_config_dir_override(temp)` 注入后「该线程」复用此目录，
//   且可重复覆盖（同一线程先后跑不同用例时各自生效，避免「OnceLock 设一次」导致
//   后续用例回落到真实 ~/.marguerite 而引发的 flaky / 跨用例串扰）。
thread_local! {
    pub(crate) static OVERRIDE_CONFIG_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

pub mod config;
pub mod safe_mode;
pub mod session;
pub mod task_queue;

pub use task_queue::{Task, TaskHandler, TaskQueue, TaskStatus};
