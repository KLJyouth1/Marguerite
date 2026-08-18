//! 内置 slot/command/shortcut 注册表装配

use crate::tui::workbench::region::activity::ActivityRail;
use crate::tui::workbench::region::main::MainArea;
use crate::tui::workbench::region::rail::RailPanel;
use crate::tui::workbench::region::sidebar::SidebarPanel;
use crate::tui::workbench::region::status::StatusBar;
use crate::tui::workbench::slot::SlotRegistry;

/// 装配全部内置插槽（五个区域渲染器）到注册表
pub fn register_builtins(reg: &mut SlotRegistry) {
    reg.register(Box::new(ActivityRail));
    reg.register(Box::new(SidebarPanel));
    reg.register(Box::new(MainArea));
    reg.register(Box::new(RailPanel));
    reg.register(Box::new(StatusBar));
}
