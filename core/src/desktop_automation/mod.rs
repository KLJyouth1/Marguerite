//! 桌面自动化框架（Epic C）
//!
//! 统一抽象「启动 / 发现 / 操控 / 录制 / 闸门」：每个桌面应用由实现
//! [`driver::DesktopAppDriver`] 的适配器驱动。WPS 演示由 [`com::ComDriver`] 经
//! Windows COM `Kwpp.Application` 驱动。
//!
//! # 接通现状（如实，2026-10-26 核对）
//!
//! - **已实装**：
//!   1. [`com::ComDriver`] —— WPS Presentation（Windows COM/OLE，`Kwpp.Application`）。
//!   2. [`uia_driver::UiaDriver`] —— 通用跨应用元素级操作（Windows UI Automation），
//!      占 [`AppKind::Generic`]：动作矩阵见其模块头（`snapshot` / `find` / `click` /
//!      `invoke` / `set_value` / `type` / `toggle` / `select` / `expand` / `collapse` /
//!      `scroll` / `focus` / `wait` / `window_activate`），写动作带
//!      observe→act→verify 回执。
//!   3. [`windows`] —— 原生窗口枚举 / 按标题或进程名附着 / 激活（`desktop.windows`、
//!      `desktop_attach`、`desktop.action` 的 `window` 参数共用这一份事实源）。
//! - **未实装且已下线**：原「输入兜底（InputDriver）」C5 骨架已删除——它从未注册、
//!   没有任何 RPC/UI/工具触达，保留只会让「键鼠兜底」看似可得。
//! - **能力项**：`auto.uia` 已随 [`uia_driver`] 落地；键鼠级兜底仍走
//!   [`crate::tools::computer_use`]（`system_control`），不在本框架内（**结构化优先**）。
//!
//! 设计要点：
//! - 所有动作经 [`recorder::Recorder`] 落审计快照（`.marguerite/audit-snapshots/<session>/`），
//!   写动作额外落真实的 before/after 节点摘要。
//! - 敏感动作经 [`policy::Policy`] 二次确认闸门（前端弹确认 + 后端兜底拒绝）。
//! - 仅 Windows 编译 COM/UIA 真实后端；非 Windows 下两者都不注册，`desktop.launch` 返回 2401；
//!   `uia_driver` 的动作引擎本身是平台无关的，mock `A11ySource` 单测在任何平台都能跑。

pub mod a11y_tree;
pub mod com;
pub mod driver;
pub mod policy;
pub mod recorder;
pub mod registry;
pub mod uia_driver;
pub mod windows;
#[cfg(windows)]
pub mod windows_uia;

pub use a11y_tree::{
    A11yError, A11ySource, NodeQuery, NodeState, PatternOp, SnapshotOptions, UiNode, UiPattern,
    UiSnapshot, MAX_NODES,
};
pub use driver::{
    observe_effect, ActionReceipt, AppHandle, AppInfo, AppKind, Bounds, DesktopAppDriver,
    DesktopError, ObservationEffect, Snapshot, UiAction,
};
pub use policy::Policy;
pub use recorder::Recorder;
pub use registry::AppRegistry;
