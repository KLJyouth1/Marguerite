//! 桌面自动化框架（Epic C）
//!
//! 统一抽象「启动 / 发现 / 操控 / 录制 / 闸门」：每个桌面应用由实现
//! [`driver::DesktopAppDriver`] 的适配器驱动。WPS 演示由 [`com::ComDriver`] 经
//! Windows COM `Kwpp.Application` 驱动；通用 GUI（UiaDriver）与输入兜底（InputDriver）
//! 为后续 C5 阶段的骨架。
//!
//! 设计要点：
//! - 所有动作经 [`recorder::Recorder`] 落审计快照（`.marguerite/audit-snapshots/<session>/`）。
//! - 敏感动作经 [`policy::Policy`] 二次确认闸门（前端弹确认 + 后端兜底拒绝）。
//! - 仅 Windows 编译 COM 驱动；非 Windows 下 WpsPpt 不注册，`desktop.launch` 返回 2401。

pub mod com;
pub mod driver;
pub mod input;
pub mod policy;
pub mod recorder;
pub mod registry;
pub mod uia;

pub use driver::{
    ActionReceipt, AppHandle, AppInfo, AppKind, DesktopAppDriver, DesktopError, Snapshot, UiAction,
};
pub use policy::Policy;
pub use recorder::Recorder;
pub use registry::AppRegistry;
