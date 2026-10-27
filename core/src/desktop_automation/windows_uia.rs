//! Windows UI Automation (UIA) 后端 —— **真实 COM 接入**（B1/B2 已接通）。
//!
//! 本模块为 [`crate::desktop_automation::a11y_tree::A11ySource`] 提供 Windows 后端
//! [`WindowsUiaSource`]：用 `windows` crate 的 UI Automation COM 客户端枚举**前台窗口**
//! 的控件树（role / name / automation_id / bounds / enabled / patterns），并按 `ref_id`
//! 找回元素后执行 pattern 级操作（Invoke / Toggle / SelectionItem / Value /
//! ExpandCollapse / Scroll / SetFocus）。
//!
//! 数据契约与「哪一个动作对应哪一个 pattern」的映射定义在
//! [`crate::desktop_automation::a11y_tree`]；本文件只做 COM 实现。
//!
//! # 线程模型（关键设计，务必保持）
//!
//! UIA 的 `IUIAutomation` / `IUIAutomationElement` **不是 `Send`/`Sync`**（COM 接口天然线程亲和），
//! 而 `A11ySource` 是在任意线程上同步调用的普通 trait。直接把这些 COM 指针塞进结构体会因
//! 缺少 `Send` 而无法编译，强行 `unsafe impl Send` 又是真实的内存安全风险（跨公寓调用 COM 是
//! 未定义行为）。因此本模块采用：
//!
//! - 每次调用都在**新建的 STA 线程**里 `CoInitializeEx(COINIT_APARTMENTTHREADED)`
//!   → `CoCreateInstance(CUIAutomation)` → 干活 → 线程结束（COM 随之解绑）；
//! - 结果经 `mpsc` 通道回传，配合 `recv_timeout` 兜底「目标窗口无响应」——绝不让调用方永久挂死；
//! - [`WindowsUiaSource`] 自身保持**零状态**（`Default + Clone`），因此天然 `Send + Sync`。
//!
//! 代价是每次快照约 1–3 ms 的线程创建开销；换来的是「无跨公寓 UB + 无残留 COM 状态 + 超时可控」。
//!
//! # 失败语义（沿用仓内 vision 通道惯例）
//!
//! 绝不编造控件树：读不到就返回 [`A11yError::Unavailable`] 或 [`A11yError::OsFailure`]，
//! 找不到 `ref_id` 就返回 [`A11yError::NodeNotFound`]，控件不支持目标 pattern 就返回
//! [`A11yError::Unsupported`]。空树是合法结果（真的没有可读控件），它不等于「假数据」。

#![cfg(windows)]

use std::sync::mpsc;
use std::time::Duration;

use windows::Win32::Foundation::{FALSE, HWND};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED, SAFEARRAY,
};
use windows::Win32::System::Ole::{
    SafeArrayDestroy, SafeArrayGetElement, SafeArrayGetLBound, SafeArrayGetUBound,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationExpandCollapsePattern,
    IUIAutomationInvokePattern, IUIAutomationLegacyIAccessiblePattern,
    IUIAutomationScrollPattern, IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern,
    IUIAutomationTreeWalker, IUIAutomationValuePattern, ExpandCollapseState_Collapsed,
    ExpandCollapseState_Expanded, ExpandCollapseState_LeafNode,
    ExpandCollapseState_PartiallyExpanded, ScrollAmount_LargeDecrement,
    ScrollAmount_LargeIncrement, ScrollAmount_NoAmount, ScrollAmount_SmallDecrement,
    ScrollAmount_SmallIncrement, ToggleState_Indeterminate, ToggleState_Off, ToggleState_On,
    UIA_AppBarControlTypeId, UIA_ButtonControlTypeId, UIA_CalendarControlTypeId,
    UIA_CheckBoxControlTypeId, UIA_ComboBoxControlTypeId, UIA_CONTROLTYPE_ID,
    UIA_CustomControlTypeId, UIA_DataGridControlTypeId, UIA_DataItemControlTypeId,
    UIA_DocumentControlTypeId, UIA_EditControlTypeId, UIA_ExpandCollapsePatternId,
    UIA_GroupControlTypeId, UIA_HeaderControlTypeId, UIA_HeaderItemControlTypeId,
    UIA_HyperlinkControlTypeId, UIA_ImageControlTypeId, UIA_InvokePatternId,
    UIA_LegacyIAccessiblePatternId, UIA_ListControlTypeId, UIA_ListItemControlTypeId,
    UIA_MenuBarControlTypeId, UIA_MenuControlTypeId, UIA_MenuItemControlTypeId,
    UIA_PaneControlTypeId, UIA_ProgressBarControlTypeId, UIA_RadioButtonControlTypeId,
    UIA_ScrollBarControlTypeId, UIA_ScrollPatternId, UIA_SelectionItemPatternId,
    UIA_SemanticZoomControlTypeId, UIA_SeparatorControlTypeId, UIA_SliderControlTypeId,
    UIA_SpinnerControlTypeId, UIA_SplitButtonControlTypeId, UIA_StatusBarControlTypeId,
    UIA_TabControlTypeId, UIA_TabItemControlTypeId, UIA_TableControlTypeId, UIA_TextControlTypeId,
    UIA_ThumbControlTypeId, UIA_TitleBarControlTypeId, UIA_TogglePatternId,
    UIA_ToolBarControlTypeId, UIA_ToolTipControlTypeId, UIA_TreeControlTypeId,
    UIA_TreeItemControlTypeId, UIA_ValuePatternId, UIA_WindowControlTypeId,
};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

use crate::desktop_automation::a11y_tree::{
    A11yError, A11ySource, NodeState, PatternOp, SnapshotOptions, UiNode, UiPattern, UiSnapshot,
    MAX_DEPTH, MAX_NODES,
};

/// 单次 UIA 调用的硬超时。COM 调用可能因目标进程无响应而永久阻塞（UIA 有跨进程 RPC），
/// 超时后调用方拿到明确的 `Unavailable`，而不是整个工作台卡死。
const UIA_CALL_TIMEOUT: Duration = Duration::from_secs(10);

/// 遍历上限（防退化 UI 树把内存/时间吃光）。超过即截断——快照本就是「采样」。
const MAX_VISITED: usize = 2000;

// ─────────────────────────────────────────────── 阶段 2（B2）：纯映射，可脱离 COM 单测

/// UIA 控件类型 → 稳定 role 字符串。
///
/// 为什么用查表而不是 `match`：`UIA_CONTROLTYPE_ID` 是 newtype，常量匹配依赖结构性相等；
/// 查表既回避该约束，又便于单测校验「无重复 role」。
const ROLE_TABLE: &[(UIA_CONTROLTYPE_ID, &str)] = &[
    (UIA_AppBarControlTypeId, "app_bar"),
    (UIA_ButtonControlTypeId, "button"),
    (UIA_CalendarControlTypeId, "calendar"),
    (UIA_CheckBoxControlTypeId, "check_box"),
    (UIA_ComboBoxControlTypeId, "combo_box"),
    (UIA_CustomControlTypeId, "custom"),
    (UIA_DataGridControlTypeId, "data_grid"),
    (UIA_DataItemControlTypeId, "data_item"),
    (UIA_DocumentControlTypeId, "document"),
    (UIA_EditControlTypeId, "edit"),
    (UIA_GroupControlTypeId, "group"),
    (UIA_HeaderControlTypeId, "header"),
    (UIA_HeaderItemControlTypeId, "header_item"),
    (UIA_HyperlinkControlTypeId, "hyperlink"),
    (UIA_ImageControlTypeId, "image"),
    (UIA_ListControlTypeId, "list"),
    (UIA_ListItemControlTypeId, "list_item"),
    (UIA_MenuBarControlTypeId, "menu_bar"),
    (UIA_MenuControlTypeId, "menu"),
    (UIA_MenuItemControlTypeId, "menu_item"),
    (UIA_PaneControlTypeId, "pane"),
    (UIA_ProgressBarControlTypeId, "progress_bar"),
    (UIA_RadioButtonControlTypeId, "radio_button"),
    (UIA_ScrollBarControlTypeId, "scroll_bar"),
    (UIA_SemanticZoomControlTypeId, "semantic_zoom"),
    (UIA_SeparatorControlTypeId, "separator"),
    (UIA_SliderControlTypeId, "slider"),
    (UIA_SpinnerControlTypeId, "spinner"),
    (UIA_SplitButtonControlTypeId, "split_button"),
    (UIA_StatusBarControlTypeId, "status_bar"),
    (UIA_TabControlTypeId, "tab"),
    (UIA_TabItemControlTypeId, "tab_item"),
    (UIA_TableControlTypeId, "table"),
    (UIA_TextControlTypeId, "text"),
    (UIA_ThumbControlTypeId, "thumb"),
    (UIA_TitleBarControlTypeId, "title_bar"),
    (UIA_ToolBarControlTypeId, "tool_bar"),
    (UIA_ToolTipControlTypeId, "tool_tip"),
    (UIA_TreeControlTypeId, "tree"),
    (UIA_TreeItemControlTypeId, "tree_item"),
    (UIA_WindowControlTypeId, "window"),
];

/// 未登记的控件类型统一落到 `"other"`（诚实降级，不猜语义）。
fn control_type_role(ct: UIA_CONTROLTYPE_ID) -> &'static str {
    ROLE_TABLE
        .iter()
        .find(|(id, _)| *id == ct)
        .map(|(_, role)| *role)
        .unwrap_or("other")
}

/// 把 UIA runtime ID（i32 数组）编码成稳定的 `ref_id`。
///
/// UIA 的 runtime ID 在同一元素的生存期内稳定，跨快照可复现，所以它能作为 `ref_id`：
/// 快照里拿到的 id，下一次动作时重新走一遍树就能找回同一个元素。
fn format_runtime_id(ids: &[i32]) -> String {
    ids.iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(".")
}

/// [`format_runtime_id`] 的逆运算。任何非数字/空段都判为不合法（返回 `None`），
/// 保证动作收到畸形 ref 时能立刻给出 `NodeNotFound` 而不是去做一次无意义的遍历。
fn parse_runtime_id(s: &str) -> Option<Vec<i32>> {
    if s.is_empty() {
        return None;
    }
    let parts: Vec<&str> = s.split('.').collect();
    let mut out = Vec::with_capacity(parts.len());
    for p in parts {
        out.push(p.parse::<i32>().ok()?);
    }
    Some(out)
}

/// 取元素包围盒 `(x, y, width, height)`，宽高非正即视为不可见/无面积。
fn rect_to_bounds(r: windows::Win32::Foundation::RECT) -> Option<(i32, i32, i32, i32)> {
    let w = r.right - r.left;
    let h = r.bottom - r.top;
    if w <= 0 || h <= 0 {
        return None;
    }
    Some((r.left, r.top, w, h))
}

/// 一次遍历的边界（避免 `visit` 参数爆炸）。
#[derive(Debug, Clone, Copy)]
struct Limits {
    max_depth: usize,
    max_nodes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_depth: MAX_DEPTH,
            max_nodes: MAX_NODES,
        }
    }
}

// ─────────────────────────────────────────────── 阶段 1（B2）：COM 侧

/// `GetRuntimeId` 返回的 SAFEARRAY 的释放守卫。
///
/// 为什么要守卫：读数组时任何一步 `?` 提前返回都会漏掉 `SafeArrayDestroy`，
/// 每次失败调用泄漏一小块 COM 内存；快照是高频动作，必须结构性地保证释放。
struct SafeArrayGuard(*mut SAFEARRAY);

impl Drop for SafeArrayGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = SafeArrayDestroy(self.0);
        }
    }
}

/// 读元素的 runtime ID（一维 i32 SAFEARRAY）。
fn read_runtime_id(el: &IUIAutomationElement) -> Result<Vec<i32>, A11yError> {
    unsafe {
        let psa = el
            .GetRuntimeId()
            .map_err(|e| A11yError::OsFailure(format!("GetRuntimeId 失败: {e}")))?;
        if psa.is_null() {
            return Err(A11yError::OsFailure("GetRuntimeId 返回空数组".into()));
        }
        let _guard = SafeArrayGuard(psa);
        let lb = SafeArrayGetLBound(psa, 1)
            .map_err(|e| A11yError::OsFailure(format!("SafeArrayGetLBound 失败: {e}")))?;
        let ub = SafeArrayGetUBound(psa, 1)
            .map_err(|e| A11yError::OsFailure(format!("SafeArrayGetUBound 失败: {e}")))?;
        let mut out = Vec::with_capacity((ub - lb + 1).max(0) as usize);
        for i in lb..=ub {
            let mut v: i32 = 0;
            SafeArrayGetElement(psa, &i as *const i32, &mut v as *mut i32 as *mut core::ffi::c_void)
                .map_err(|e| A11yError::OsFailure(format!("SafeArrayGetElement 失败: {e}")))?;
            out.push(v);
        }
        Ok(out)
    }
}

/// 前台窗口的 UIA 元素。取不到（锁屏 / 无交互会话 / 应用未实现 UIA）时**明确报错**。
fn foreground_root(uia: &IUIAutomation) -> Result<IUIAutomationElement, A11yError> {
    let hwnd: HWND = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return Err(A11yError::Unavailable(
            "当前无前台窗口（会话未交互或已锁屏）".into(),
        ));
    }
    unsafe { uia.ElementFromHandle(hwnd) }.map_err(|e| {
        A11yError::OsFailure(format!("前台窗口未暴露 UIA 元素（该应用可能未实现 UIA）: {e}"))
    })
}

/// 深度优先遍历**控件视图**树，收集 `(ref_id, element)`。
///
/// 用 `ControlViewWalker` 而非 RawView：控件视图已经剔除了纯布局容器，
/// 信噪比高得多，正是给「智能体读界面」用的那一层。
fn visit(
    walker: &IUIAutomationTreeWalker,
    root: &IUIAutomationElement,
    out: &mut Vec<(String, IUIAutomationElement)>,
    depth: usize,
    limits: Limits,
) {
    unsafe {
        if depth > limits.max_depth || out.len() >= MAX_VISITED || out.len() >= limits.max_nodes {
            return;
        }
        let mut child = walker.GetFirstChildElement(root).ok();
        while let Some(c) = child {
            if out.len() >= MAX_VISITED || out.len() >= limits.max_nodes {
                return;
            }
            if let Ok(ids) = read_runtime_id(&c) {
                out.push((format_runtime_id(&ids), c.clone()));
            }
            visit(walker, &c, out, depth + 1, limits);
            child = walker.GetNextSiblingElement(&c).ok();
        }
    }
}

/// 探测元素支持的 pattern（`GetCurrentPatternAs` 成功即视为可用）。
///
/// 这是 W6.1 的 pattern 分发的**发现侧**：控制台/模型先知道「这个控件能做什么」，
/// 再决定用 invoke 还是 set_value，而不是盲试。
fn probe_patterns(el: &IUIAutomationElement) -> Vec<UiPattern> {
    let mut out = Vec::new();
    unsafe {
        if el
            .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
            .is_ok()
        {
            out.push(UiPattern::Invoke);
        }
        if el
            .GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
            .is_ok()
        {
            out.push(UiPattern::Toggle);
        }
        if el
            .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
            .is_ok()
        {
            out.push(UiPattern::SelectionItem);
        }
        if el
            .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .is_ok()
        {
            out.push(UiPattern::Value);
        }
        if el
            .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
            .is_ok()
        {
            out.push(UiPattern::ExpandCollapse);
        }
        if el
            .GetCurrentPatternAs::<IUIAutomationScrollPattern>(UIA_ScrollPatternId)
            .is_ok()
        {
            out.push(UiPattern::Scroll);
        }
        // SetFocus 不是独立 pattern（是 element 方法）：只在元素**可获焦**时才声明支持，
        // 否则 `focus` 动作会在静态文本上「看起来支持、实际失败」。
        let focusable = el
            .CurrentIsKeyboardFocusable()
            .map(|b| b != FALSE)
            .unwrap_or(false);
        if focusable {
            out.push(UiPattern::SetFocus);
        }
    }
    out
}

/// 元素 → [`UiNode`]。宽高非正或已离屏的元素直接判为不可见，不进快照。
fn build_node(el: &IUIAutomationElement, ref_id: String) -> Option<UiNode> {
    unsafe {
        let ct = el.CurrentControlType().ok()?;
        if el.CurrentIsOffscreen().map(|b| b != FALSE).unwrap_or(false) {
            return None;
        }
        let bounds = rect_to_bounds(el.CurrentBoundingRectangle().ok()?)?;
        let name = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
        let automation_id = el
            .CurrentAutomationId()
            .map(|b| b.to_string())
            .unwrap_or_default();
        let enabled = el.CurrentIsEnabled().map(|b| b != FALSE).unwrap_or(false);
        let focused = el
            .CurrentHasKeyboardFocus()
            .map(|b| b != FALSE)
            .unwrap_or(false);
        let focusable = el
            .CurrentIsKeyboardFocusable()
            .map(|b| b != FALSE)
            .unwrap_or(false);
        let patterns = probe_patterns(el);
        Some(UiNode {
            role: control_type_role(ct).to_string(),
            name,
            ref_id,
            automation_id,
            bounds,
            clickable: enabled && focusable,
            enabled,
            focused,
            patterns,
        })
    }
}

/// 激活一个元素：按「最语义化 → 最退化」依次尝试。
///
/// Invoke（按钮/链接）→ Toggle（复选框）→ Select（列表项/标签页）→ SetFocus（输入框等）。
/// 四条都失败才报错：这条链覆盖了绝大多数真实控件，且**从不谎报成功**。
fn activate_element(el: &IUIAutomationElement) -> Result<(), A11yError> {
    unsafe {
        if let Ok(p) = el.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) {
            if p.Invoke().is_ok() {
                return Ok(());
            }
        }
        if let Ok(p) = el.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) {
            if p.Toggle().is_ok() {
                return Ok(());
            }
        }
        if let Ok(p) =
            el.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
        {
            if p.Select().is_ok() {
                return Ok(());
            }
        }
        if el.SetFocus().is_ok() {
            return Ok(());
        }
        Err(A11yError::OsFailure(
            "目标控件不支持 Invoke/Toggle/Select，SetFocus 亦失败".into(),
        ))
    }
}

fn pattern_unavailable(op: &PatternOp, e: windows::core::Error) -> A11yError {
    A11yError::Unsupported(format!(
        "{} pattern 不可用（{}）: {e}",
        op.pattern().as_str(),
        op.as_str()
    ))
}

/// 读元素当前值（ValuePattern）；不支持时 `None`。
fn read_value(el: &IUIAutomationElement) -> Option<String> {
    unsafe {
        el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .ok()
            .and_then(|p| p.CurrentValue().ok())
            .map(|b| b.to_string())
    }
}

/// 写值：ValuePattern 优先，退化到 LegacyIAccessiblePattern.SetValue。
fn set_element_value(el: &IUIAutomationElement, value: &str) -> Result<(), A11yError> {
    unsafe {
        if let Ok(p) = el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
            return p
                .SetValue(&windows::core::BSTR::from(value))
                .map_err(|e| A11yError::OsFailure(format!("ValuePattern.SetValue 失败: {e}")));
        }
        if let Ok(p) =
            el.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(
                UIA_LegacyIAccessiblePatternId,
            )
        {
            let wide = windows::core::HSTRING::from(value);
            return p
                .SetValue(&wide)
                .map_err(|e| A11yError::OsFailure(format!("LegacyIAccessible.SetValue 失败: {e}")));
        }
        Err(A11yError::Unsupported(
            "目标控件既无 ValuePattern 也无 LegacyIAccessiblePattern，无法写值".into(),
        ))
    }
}

/// 滚动格数 → UIA `ScrollAmount`（|delta| > 1 视为大增量，否则小增量）。
fn scroll_amount(delta: i32) -> windows::Win32::UI::Accessibility::ScrollAmount {
    if delta > 1 {
        ScrollAmount_LargeIncrement
    } else if delta > 0 {
        ScrollAmount_SmallIncrement
    } else if delta < -1 {
        ScrollAmount_LargeDecrement
    } else if delta < 0 {
        ScrollAmount_SmallDecrement
    } else {
        ScrollAmount_NoAmount
    }
}

fn toggle_state_name(state: &windows::Win32::UI::Accessibility::ToggleState) -> &'static str {
    if *state == ToggleState_On {
        "on"
    } else if *state == ToggleState_Off {
        "off"
    } else if *state == ToggleState_Indeterminate {
        "indeterminate"
    } else {
        "unknown"
    }
}

fn expand_state_name(
    state: &windows::Win32::UI::Accessibility::ExpandCollapseState,
) -> &'static str {
    if *state == ExpandCollapseState_Expanded {
        "expanded"
    } else if *state == ExpandCollapseState_Collapsed {
        "collapsed"
    } else if *state == ExpandCollapseState_PartiallyExpanded {
        "partially_expanded"
    } else if *state == ExpandCollapseState_LeafNode {
        "leaf"
    } else {
        "unknown"
    }
}

/// **pattern 分发**（W6.1 核心）：把跨平台 [`PatternOp`] 落到具体 COM pattern。
///
/// 只做精确映射，不做「猜一个能用的 pattern」：目标不支持就返回
/// [`A11yError::Unsupported`]，让上层如实告诉用户/模型。
fn dispatch_pattern(el: &IUIAutomationElement, op: &PatternOp) -> Result<(), A11yError> {
    unsafe {
        match op {
            PatternOp::Invoke => {
                let p = el
                    .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                    .map_err(|e| pattern_unavailable(op, e))?;
                p.Invoke()
                    .map_err(|e| A11yError::OsFailure(format!("Invoke 失败: {e}")))
            }
            PatternOp::Toggle => {
                let p = el
                    .GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
                    .map_err(|e| pattern_unavailable(op, e))?;
                p.Toggle()
                    .map_err(|e| A11yError::OsFailure(format!("Toggle 失败: {e}")))
            }
            PatternOp::Select => {
                let p = el
                    .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                        UIA_SelectionItemPatternId,
                    )
                    .map_err(|e| pattern_unavailable(op, e))?;
                p.Select()
                    .map_err(|e| A11yError::OsFailure(format!("SelectionItem.Select 失败: {e}")))
            }
            PatternOp::Expand => {
                let p = el
                    .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(
                        UIA_ExpandCollapsePatternId,
                    )
                    .map_err(|e| pattern_unavailable(op, e))?;
                p.Expand()
                    .map_err(|e| A11yError::OsFailure(format!("Expand 失败: {e}")))
            }
            PatternOp::Collapse => {
                let p = el
                    .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(
                        UIA_ExpandCollapsePatternId,
                    )
                    .map_err(|e| pattern_unavailable(op, e))?;
                p.Collapse()
                    .map_err(|e| A11yError::OsFailure(format!("Collapse 失败: {e}")))
            }
            PatternOp::Focus => el
                .SetFocus()
                .map_err(|e| A11yError::OsFailure(format!("SetFocus 失败: {e}"))),
            PatternOp::SetValue(value) => set_element_value(el, value),
            PatternOp::TypeText { text, clear } => {
                let next = if *clear {
                    text.clone()
                } else {
                    let mut current = read_value(el).unwrap_or_default();
                    current.push_str(text);
                    current
                };
                set_element_value(el, &next)
            }
            PatternOp::Scroll {
                horizontal,
                vertical,
            } => {
                let p = el
                    .GetCurrentPatternAs::<IUIAutomationScrollPattern>(UIA_ScrollPatternId)
                    .map_err(|e| pattern_unavailable(op, e))?;
                p.Scroll(scroll_amount(*horizontal), scroll_amount(*vertical))
                    .map_err(|e| A11yError::OsFailure(format!("Scroll 失败: {e}")))
            }
        }
    }
}

/// 读元素的完整可观测状态（observe→act→verify 的观测面）。
fn read_element_state(el: &IUIAutomationElement) -> Result<NodeState, A11yError> {
    unsafe {
        let ids = read_runtime_id(el)?;
        let role = el
            .CurrentControlType()
            .map(control_type_role)
            .unwrap_or("other")
            .to_string();
        let name = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
        let automation_id = el
            .CurrentAutomationId()
            .map(|b| b.to_string())
            .unwrap_or_default();
        let enabled = el.CurrentIsEnabled().map(|b| b != FALSE).unwrap_or(false);
        let focused = el
            .CurrentHasKeyboardFocus()
            .map(|b| b != FALSE)
            .unwrap_or(false);
        let bounds = el
            .CurrentBoundingRectangle()
            .ok()
            .and_then(rect_to_bounds)
            .unwrap_or((0, 0, 0, 0));

        let toggle_state = el
            .GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
            .ok()
            .and_then(|p| p.CurrentToggleState().ok())
            .map(|s| toggle_state_name(&s).to_string());
        let expand_collapse_state = el
            .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
            .ok()
            .and_then(|p| p.CurrentExpandCollapseState().ok())
            .map(|s| expand_state_name(&s).to_string());
        let selected = el
            .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                UIA_SelectionItemPatternId,
            )
            .ok()
            .and_then(|p| p.CurrentIsSelected().ok())
            .map(|b| b != FALSE);
        let scroll_percent = el
            .GetCurrentPatternAs::<IUIAutomationScrollPattern>(UIA_ScrollPatternId)
            .ok()
            .and_then(|p| {
                let h = p.CurrentHorizontalScrollPercent().ok()?;
                let v = p.CurrentVerticalScrollPercent().ok()?;
                // UIA_ScrollPatternNoScroll = -1.0：表示该方向不可滚动，不是「0%」。
                (h >= 0.0 && v >= 0.0).then_some((h, v))
            });

        Ok(NodeState {
            ref_id: format_runtime_id(&ids),
            role,
            name,
            automation_id,
            value: read_value(el),
            enabled,
            focused,
            toggle_state,
            expand_collapse_state,
            selected,
            scroll_percent,
            bounds,
        })
    }
}

/// 在独立 STA 线程上执行 UIA 工作。返回值经通道回传并受 [`UIA_CALL_TIMEOUT`] 约束。
fn on_uia_thread<T, F>(f: F) -> Result<T, A11yError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, A11yError> + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<Result<T, A11yError>>();
    std::thread::Builder::new()
        .name("marguerite-uia".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            // 线程结束即丢弃 send 端；若 f panic，recv 端会看到 Disconnected 而不是挂死。
            let _ = tx.send(f());
        })
        .map_err(|e| A11yError::OsFailure(format!("无法创建 UIA 工作线程: {e}")))?;

    match rx.recv_timeout(UIA_CALL_TIMEOUT) {
        Ok(r) => r,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(A11yError::Unavailable(format!(
            "UIA 调用超时（{}s）：目标窗口无响应",
            UIA_CALL_TIMEOUT.as_secs()
        ))),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err(A11yError::OsFailure("UIA 工作线程异常退出".into()))
        }
    }
}

/// 建 COM 公寓 + `IUIAutomation`，把实例交给 `f`。实例绝不出线程。
fn with_uia<T, F>(f: F) -> Result<T, A11yError>
where
    T: Send + 'static,
    F: FnOnce(&IUIAutomation) -> Result<T, A11yError> + Send + 'static,
{
    on_uia_thread(move || {
        unsafe {
            // CoInitializeEx 返回 RPC_E_CHANGED_MODE 等也已由系统保证线程可继续用 COM，
            // 因此忽略返回值；真正致命的是下面的 CoCreateInstance。
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let uia: IUIAutomation = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_ALL) }
            .map_err(|e| {
                A11yError::Unavailable(format!(
                    "无法创建 IUIAutomation（{e}）：系统 UI Automation 服务不可用"
                ))
            })?;
        f(&uia)
    })
}

/// 在前台窗口控件树里按 `ref_id` 找回元素。
fn locate(uia: &IUIAutomation, ref_id: &str) -> Result<IUIAutomationElement, A11yError> {
    let ids = parse_runtime_id(ref_id).ok_or_else(|| A11yError::NodeNotFound(ref_id.to_string()))?;
    let want = format_runtime_id(&ids);
    let root = foreground_root(uia)?;
    let walker = unsafe { uia.ControlViewWalker() }
        .map_err(|e| A11yError::OsFailure(format!("取 ControlViewWalker 失败: {e}")))?;
    let mut raw: Vec<(String, IUIAutomationElement)> = Vec::new();
    visit(&walker, &root, &mut raw, 0, Limits::default());
    raw.iter()
        .find(|(id, _)| *id == want)
        .map(|(_, e)| e.clone())
        .ok_or_else(|| A11yError::NodeNotFound(ref_id.to_string()))
}

/// 枚举前台窗口的控件树。
fn enumerate_foreground(limits: Limits) -> Result<UiSnapshot, A11yError> {
    with_uia(move |uia| {
        let root = foreground_root(uia)?;
        let walker = unsafe { uia.ControlViewWalker() }
            .map_err(|e| A11yError::OsFailure(format!("取 ControlViewWalker 失败: {e}")))?;
        let mut raw: Vec<(String, IUIAutomationElement)> = Vec::new();
        visit(&walker, &root, &mut raw, 0, limits);
        let mut nodes = Vec::new();
        for (ref_id, el) in &raw {
            if nodes.len() >= limits.max_nodes {
                break;
            }
            if let Some(n) = build_node(el, ref_id.clone()) {
                nodes.push(n);
            }
        }
        Ok(UiSnapshot { nodes })
    })
}

/// 按 `ref_id` 找回元素并激活（click 的退化链）。
fn click_ref(ref_id: &str) -> Result<(), A11yError> {
    // 闭包要送进 'static 线程：错误文案用的 ref_id 必须提前拷成 owned String。
    let not_found = ref_id.to_string();
    with_uia(move |uia| {
        let el = locate(uia, &not_found)?;
        activate_element(&el)
    })
}

/// 按 `ref_id` 读节点状态。
fn state_by_ref(ref_id: &str) -> Result<NodeState, A11yError> {
    let not_found = ref_id.to_string();
    with_uia(move |uia| {
        let el = locate(uia, &not_found)?;
        read_element_state(&el)
    })
}

/// 按 `ref_id` 执行 pattern 操作。
fn perform_by_ref(ref_id: &str, op: &PatternOp) -> Result<(), A11yError> {
    let not_found = ref_id.to_string();
    let op = op.clone();
    with_uia(move |uia| {
        let el = locate(uia, &not_found)?;
        dispatch_pattern(&el, &op)
    })
}

// ─────────────────────────────────────────────── 对外契约实现

/// Windows UIA 无障碍源（[`A11ySource`] 的 Windows 后端）。
///
/// 零状态：每次调用现开 STA 线程、现建 COM 对象（见模块头「线程模型」）。
/// 因此本结构体天然 `Send + Sync`，可安全放进 [`crate::desktop_automation::registry::AppRegistry`]。
#[derive(Debug, Default, Clone)]
pub struct WindowsUiaSource {
    /// 占位字段，防止外部以结构体字面量构造出「假装持有 COM 指针」的实例。
    /// 真实 COM 对象只存在于每次调用的线程内，不落在结构体上。
    _private: (),
}

impl WindowsUiaSource {
    /// 构造一个 Windows UIA 源。
    pub fn new() -> Self {
        Self::default()
    }
}

impl A11ySource for WindowsUiaSource {
    fn snapshot(&self) -> Result<UiSnapshot, A11yError> {
        enumerate_foreground(Limits::default())
    }

    fn snapshot_with(&self, opts: &SnapshotOptions) -> Result<UiSnapshot, A11yError> {
        let (max_depth, max_nodes) = opts.bounded();
        enumerate_foreground(Limits {
            max_depth,
            max_nodes,
        })
    }

    fn click(&self, ref_id: &str) -> Result<(), A11yError> {
        click_ref(ref_id)
    }

    fn node_state(&self, ref_id: &str) -> Result<NodeState, A11yError> {
        state_by_ref(ref_id)
    }

    fn perform(&self, ref_id: &str, op: &PatternOp) -> Result<(), A11yError> {
        perform_by_ref(ref_id, op)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 纯函数层：不碰 COM，任何会话下都可稳定运行 ──

    #[test]
    fn role_table_maps_known_control_types() {
        assert_eq!(control_type_role(UIA_ButtonControlTypeId), "button");
        assert_eq!(control_type_role(UIA_EditControlTypeId), "edit");
        assert_eq!(control_type_role(UIA_TextControlTypeId), "text");
        assert_eq!(control_type_role(UIA_WindowControlTypeId), "window");
        assert_eq!(control_type_role(UIA_MenuItemControlTypeId), "menu_item");
    }

    #[test]
    fn role_table_has_no_duplicate_roles_or_ids() {
        let mut roles: Vec<&str> = ROLE_TABLE.iter().map(|(_, r)| *r).collect();
        let n = roles.len();
        roles.sort_unstable();
        roles.dedup();
        assert_eq!(roles.len(), n, "role 字符串必须唯一");

        let mut ids: Vec<i32> = ROLE_TABLE.iter().map(|(id, _)| id.0).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "控件类型 ID 不得重复登记");
    }

    #[test]
    fn runtime_id_round_trips() {
        let ids = vec![42, -1001, 7];
        assert_eq!(parse_runtime_id(&format_runtime_id(&ids)), Some(ids));
    }

    #[test]
    fn runtime_id_rejects_malformed_input() {
        assert_eq!(parse_runtime_id(""), None);
        assert_eq!(parse_runtime_id("a.b"), None);
        assert_eq!(parse_runtime_id("1..2"), None);
        assert_eq!(parse_runtime_id("1.2."), None);
        assert_eq!(parse_runtime_id("1.2"), Some(vec![1, 2]));
    }

    #[test]
    fn rect_to_bounds_drops_zero_or_negative_area() {
        use windows::Win32::Foundation::RECT;
        let ok = RECT {
            left: 10,
            top: 20,
            right: 110,
            bottom: 60,
        };
        assert_eq!(rect_to_bounds(ok), Some((10, 20, 100, 40)));

        for degenerate in [
            RECT { left: 10, top: 20, right: 10, bottom: 60 }, // 零宽
            RECT { left: 10, top: 20, right: 110, bottom: 20 }, // 零高
            RECT { left: 10, top: 20, right: 5, bottom: 60 },   // 负宽
        ] {
            assert_eq!(rect_to_bounds(degenerate), None, "{degenerate:?} 应被丢弃");
        }
    }

    // ── pattern 分发的纯映射层（不需要 COM） ──

    #[test]
    fn scroll_amount_maps_sign_and_magnitude() {
        assert_eq!(scroll_amount(0), ScrollAmount_NoAmount);
        assert_eq!(scroll_amount(1), ScrollAmount_SmallIncrement);
        assert_eq!(scroll_amount(5), ScrollAmount_LargeIncrement);
        assert_eq!(scroll_amount(-1), ScrollAmount_SmallDecrement);
        assert_eq!(scroll_amount(-5), ScrollAmount_LargeDecrement);
    }

    #[test]
    fn toggle_and_expand_state_names_are_stable() {
        assert_eq!(toggle_state_name(&ToggleState_On), "on");
        assert_eq!(toggle_state_name(&ToggleState_Off), "off");
        assert_eq!(toggle_state_name(&ToggleState_Indeterminate), "indeterminate");
        assert_eq!(
            expand_state_name(&ExpandCollapseState_Expanded),
            "expanded"
        );
        assert_eq!(
            expand_state_name(&ExpandCollapseState_Collapsed),
            "collapsed"
        );
        assert_eq!(
            expand_state_name(&ExpandCollapseState_PartiallyExpanded),
            "partially_expanded"
        );
        assert_eq!(expand_state_name(&ExpandCollapseState_LeafNode), "leaf");
    }

    /// 每个 [`PatternOp`] 都必须在 COM 分发里有对应分支（源码级守卫：新增 op 忘了接线会红）。
    #[test]
    fn every_pattern_op_is_handled_by_dispatch_pattern() {
        let src = include_str!("windows_uia.rs");
        let dispatch_start = src
            .find("fn dispatch_pattern(")
            .expect("dispatch_pattern 必须存在");
        let body = &src[dispatch_start..];
        for arm in [
            "PatternOp::Invoke =>",
            "PatternOp::Toggle =>",
            "PatternOp::Select =>",
            "PatternOp::Expand =>",
            "PatternOp::Collapse =>",
            "PatternOp::Focus =>",
            "PatternOp::SetValue(value) =>",
            "PatternOp::TypeText { text, clear } =>",
            "PatternOp::Scroll {",
        ] {
            assert!(body.contains(arm), "dispatch_pattern 缺少分支: {arm}");
        }
    }

    /// 「单次 UIA 调用 10s 硬超时」不是文档口号：每个 COM 入口都必须经 `with_uia`/`on_uia_thread`。
    ///
    /// 源码级守卫：将来新增一个直接 `CoCreateInstance` 干活的入口，会在这里红。
    #[test]
    fn every_com_entry_point_goes_through_the_timeout_wrapper() {
        assert_eq!(
            UIA_CALL_TIMEOUT,
            Duration::from_secs(10),
            "10s 是 W6.1 的既有约束，不得悄悄改小/改大"
        );
        let src = include_str!("windows_uia.rs");
        for entry in [
            "fn click_ref(",
            "fn state_by_ref(",
            "fn perform_by_ref(",
            "fn enumerate_foreground(",
        ] {
            let start = src.find(entry).unwrap_or_else(|| panic!("{entry} 必须存在"));
            let rest = &src[start..];
            let body = &rest[..rest.find("\n}\n").unwrap_or(rest.len())];
            assert!(
                body.contains("with_uia("),
                "{entry} 未经 with_uia/on_uia_thread，会绕过 10s 超时"
            );
        }
    }

    /// 遍历上限必须与共享契约同源（`MAX_NODES` / `MAX_DEPTH`），不允许后端私设更松的边界。
    #[test]
    fn default_limits_match_the_shared_bounds() {
        let limits = Limits::default();
        assert_eq!(limits.max_nodes, MAX_NODES, "节点上限必须是共享的 MAX_NODES");
        assert_eq!(limits.max_depth, MAX_DEPTH, "深度上限必须是共享的 MAX_DEPTH");
        const {
            assert!(
                MAX_VISITED >= MAX_NODES,
                "MAX_VISITED 不得比 MAX_NODES 更紧（否则节点上限被遍历上限悄悄压低）"
            );
        }
    }

    // ── 会话相关层：断言「诚实」，不假设一定有桌面 ──

    #[test]
    fn malformed_ref_is_node_not_found_without_touching_com() {
        // 畸形 ref 在进 COM 之前就被 parse 拦下：结果必须精确且确定。
        let src = WindowsUiaSource::new();
        assert_eq!(
            src.click("not-a-runtime-id"),
            Err(A11yError::NodeNotFound("not-a-runtime-id".into()))
        );
        assert_eq!(
            src.node_state("not-a-runtime-id"),
            Err(A11yError::NodeNotFound("not-a-runtime-id".into()))
        );
        assert_eq!(
            src.perform("not-a-runtime-id", &PatternOp::Invoke),
            Err(A11yError::NodeNotFound("not-a-runtime-id".into()))
        );
    }

    #[test]
    fn snapshot_is_honest_never_fakes_a_tree() {
        let src = WindowsUiaSource::new();
        match src.snapshot() {
            Ok(snap) => {
                // 有结果就必须是自洽的真实数据：role 非空、ref_id 可解析、包围盒为正。
                for n in &snap.nodes {
                    assert!(!n.role.is_empty(), "role 不得为空");
                    assert!(
                        parse_runtime_id(&n.ref_id).is_some(),
                        "ref_id 必须是可解析的运行时 ID: {}",
                        n.ref_id
                    );
                    assert!(n.bounds.2 > 0 && n.bounds.3 > 0, "包围盒必须为正: {:?}", n.bounds);
                    assert!(
                        n.patterns.iter().all(|p| UiPattern::all().contains(p)),
                        "patterns 只能来自已知枚举: {:?}",
                        n.patterns
                    );
                    // 可点节点必然可获焦（clickable = enabled && focusable），所以它必须声明 SetFocus；
                    // 反过来不成立（可获焦但被禁用）。
                    if n.clickable {
                        assert!(
                            n.patterns.contains(&UiPattern::SetFocus),
                            "可点节点必须声明 SetFocus: {:?}",
                            n.patterns
                        );
                    }
                }
            }
            // 无桌面会话 / UIA 不可用 / 前台窗口不响应：都必须是明确错误。
            Err(A11yError::Unavailable(_)) | Err(A11yError::OsFailure(_)) => {}
            Err(e @ A11yError::NodeNotFound(_)) => {
                panic!("snapshot 不应返回 NodeNotFound（那是 click 的语义）: {e:?}")
            }
            Err(e @ A11yError::Unsupported(_)) => {
                panic!("snapshot 不应返回 Unsupported（那是 pattern 的语义）: {e:?}")
            }
        }
    }

    #[test]
    fn snapshot_with_respects_bounds() {
        let src = WindowsUiaSource::new();
        let opts = SnapshotOptions {
            max_depth: Some(1),
            max_nodes: Some(3),
        };
        match src.snapshot_with(&opts) {
            Ok(snap) => assert!(snap.nodes.len() <= 3, "max_nodes 必须生效: {}", snap.nodes.len()),
            Err(A11yError::Unavailable(_)) | Err(A11yError::OsFailure(_)) => {}
            Err(e) => panic!("snapshot_with 失败了: {e:?}"),
        }
    }

    #[test]
    fn click_on_unknown_ref_never_reports_success() {
        let src = WindowsUiaSource::new();
        match src.click("999999999.1") {
            Ok(()) => panic!("不存在的 ref_id 不得返回成功"),
            Err(A11yError::Unavailable(m)) | Err(A11yError::OsFailure(m)) => {
                assert!(!m.is_empty(), "错误文案不得为空");
            }
            Err(A11yError::NodeNotFound(_)) => {}
            Err(A11yError::Unsupported(_)) => {}
        }
    }

    #[test]
    fn source_is_default_clonable_and_debuggable() {
        let a = WindowsUiaSource::new();
        let b = a.clone();
        let c = WindowsUiaSource::default();
        // 三者都必须是零状态值：Debug 输出一致即证明没夹带会话相关的内部状态。
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
        assert_eq!(format!("{a:?}"), format!("{c:?}"));
    }
}
