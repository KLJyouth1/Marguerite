//! 插槽契约 + 36 槽注册表（跨形态契约唯一来源，F1 一致性门禁用）
//!
//! 说明：设计文档 `plan-phase-b-tui-workbench.md` 的 `SlotId` 枚举列出了 20 个显式槽位，
//! 但验收 B5 写明「36 槽」。本实现以 36 为权威契约总数：包含文档枚举的 20 个，并补充
//! 工作台实际存在的其余 16 个表面（git/problems/outline/bookmarks、preview/debug/search/
//! callstack、planner/trace/logs/eval、output/debugconsole/ports 等）。TUI 实现可渲染子集，
//! CLI/GUI 实现其余，三者共享同一组 `SlotId`。

use std::collections::HashMap;

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::tui::workbench::state::WorkbenchState;

/// 插槽 id（跨形态契约唯一来源）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotId {
    // ---- 左：活动栏 + 侧边栏（10）----
    LeftActivity,
    LeftSidebarFileTree,
    LeftSidebarSearch,
    LeftSidebarSkills,
    LeftSidebarSessions,
    LeftSidebarCommands,
    LeftSidebarGit,
    LeftSidebarProblems,
    LeftSidebarOutline,
    LeftSidebarBookmarks,
    // ---- 中：工作区（10）----
    MainTabs,
    MainChat,
    MainCode,
    MainTerminal,
    MainDiff,
    MainPreview,
    MainDebug,
    MainProblems,
    MainSearch,
    MainCallStack,
    // ---- 右：AI 面板（10）----
    RightChat,
    RightAgentStatus,
    RightAgentTools,
    RightAgentContext,
    RightAgentMemory,
    RightAgentTasks,
    RightAgentPlanner,
    RightAgentTrace,
    RightAgentLogs,
    RightAgentEval,
    // ---- 底：状态/输出（6）----
    BottomTerminal,
    BottomProblems,
    BottomStatusbar,
    BottomOutput,
    BottomDebugConsole,
    BottomPorts,
}

/// 插槽四态（三态内联：就地消化，错误绝不冒泡）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    /// 加载中
    Loading,
    /// 空
    Empty,
    /// 错误（内联危险红）
    Error,
    /// 有内容
    Content,
}

/// 插槽 trait：每个可渲染区域实现此 trait
pub trait Slot {
    /// 插槽 id
    fn id(&self) -> SlotId;
    /// 渲染（只读 `WorkbenchState`）
    fn render(&self, frame: &mut Frame, area: Rect, state: &WorkbenchState);
}

/// 全部 36 个契约槽位（顺序稳定，供 F1 diff 与单测比对）
pub fn all_slot_ids() -> Vec<SlotId> {
    use SlotId::*;
    vec![
        LeftActivity,
        LeftSidebarFileTree,
        LeftSidebarSearch,
        LeftSidebarSkills,
        LeftSidebarSessions,
        LeftSidebarCommands,
        LeftSidebarGit,
        LeftSidebarProblems,
        LeftSidebarOutline,
        LeftSidebarBookmarks,
        MainTabs,
        MainChat,
        MainCode,
        MainTerminal,
        MainDiff,
        MainPreview,
        MainDebug,
        MainProblems,
        MainSearch,
        MainCallStack,
        RightChat,
        RightAgentStatus,
        RightAgentTools,
        RightAgentContext,
        RightAgentMemory,
        RightAgentTasks,
        RightAgentPlanner,
        RightAgentTrace,
        RightAgentLogs,
        RightAgentEval,
        BottomTerminal,
        BottomProblems,
        BottomStatusbar,
        BottomOutput,
        BottomDebugConsole,
        BottomPorts,
    ]
}

/// 36 槽注册表（三形态共享同一组 id；TUI 实现其中可渲染子集）
pub struct SlotRegistry {
    slots: HashMap<SlotId, Box<dyn Slot>>,
}

impl SlotRegistry {
    /// 创建空注册表
    pub fn new() -> Self {
        Self {
            slots: HashMap::new(),
        }
    }

    /// 注册一个插槽实现
    pub fn register(&mut self, slot: Box<dyn Slot>) {
        self.slots.insert(slot.id(), slot);
    }

    /// 按 id 取插槽（不可变引用）
    pub fn get(&self, id: SlotId) -> Option<&dyn Slot> {
        self.slots.get(&id).map(|b| &**b)
    }

    /// 已注册插槽的 id 列表
    pub fn ids(&self) -> Vec<SlotId> {
        self.slots.keys().copied().collect()
    }

    /// 契约槽位数（恒为 36）
    pub fn contract_count() -> usize {
        all_slot_ids().len()
    }

    /// 契约全量 id（与 `all_slot_ids` 一致）
    pub fn contract_ids() -> Vec<SlotId> {
        all_slot_ids()
    }
}

impl Default for SlotRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contract_has_36_slots() {
        let ids = all_slot_ids();
        assert_eq!(ids.len(), 36, "跨形态契约应为 36 槽");
    }

    #[test]
    fn test_contract_ids_unique() {
        let ids = all_slot_ids();
        let mut seen = std::collections::HashSet::new();
        for id in &ids {
            assert!(seen.insert(*id), "重复槽位: {id:?}");
        }
    }

    #[test]
    fn test_registry_register_and_get() {
        struct Dummy;
        impl Slot for Dummy {
            fn id(&self) -> SlotId {
                SlotId::MainChat
            }
            fn render(&self, _f: &mut Frame, _a: Rect, _s: &WorkbenchState) {}
        }
        let mut reg = SlotRegistry::new();
        assert_eq!(reg.ids().len(), 0);
        reg.register(Box::new(Dummy));
        assert_eq!(reg.ids().len(), 1);
        assert!(reg.get(SlotId::MainChat).is_some());
        assert!(reg.get(SlotId::MainCode).is_none());
    }
}
