//! 桌面自动化框架（Epic C）
//!
//! 统一抽象「启动 / 发现 / 操控 / 录制 / 闸门」：每个桌面应用由实现
//! [`driver::DesktopAppDriver`] 的适配器驱动。WPS 演示由 [`com::ComDriver`] 经
//! Windows COM `Kwpp.Application` 驱动。
//!
//! # 接通现状（如实，2026-10-15 核对）
//!
//! - **已实装**：
//!   1. [`com::ComDriver`] —— WPS Presentation（Windows COM/OLE，`Kwpp.Application`）。
//!   2. [`uia_driver::UiaDriver`] —— 通用跨应用元素级操作（Windows UI Automation），
//!      占 [`AppKind::Generic`]：`snapshot` 读前台窗口控件树、`click` 按 `ref_id`/`name` 激活控件。
//! - **未实装且已下线**：原「输入兜底（InputDriver）」C5 骨架已删除——它从未注册、
//!   没有任何 RPC/UI/工具触达，保留只会让「键鼠兜底」看似可得。
//! - **能力项**：`auto.uia` 的「通用跨应用 UIA 未接通」表述已随 [`uia_driver`] 落地而更新；
//!   键鼠级兜底仍走 [`crate::tools::computer_use`]（`system_control`），不在本框架内。
//!
//! 设计要点：
//! - 所有动作经 [`recorder::Recorder`] 落审计快照（`.marguerite/audit-snapshots/<session>/`）。
//! - 敏感动作经 [`policy::Policy`] 二次确认闸门（前端弹确认 + 后端兜底拒绝）。
//! - 仅 Windows 编译 COM/UIA 驱动；非 Windows 下两者都不注册，`desktop.launch` 返回 2401。

pub mod com;
pub mod a11y_tree;
pub mod driver;
pub mod policy;
pub mod recorder;
pub mod registry;
#[cfg(windows)]
pub mod uia_driver;
#[cfg(windows)]
pub mod windows_uia;

pub use driver::{
    ActionReceipt, AppHandle, AppInfo, AppKind, DesktopAppDriver, DesktopError, Snapshot, UiAction,
};
pub use policy::Policy;
pub use recorder::Recorder;
pub use registry::AppRegistry;
