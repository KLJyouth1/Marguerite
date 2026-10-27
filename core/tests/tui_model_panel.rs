//! TUI 模型面板契约测试（P3 矩阵第 10 行「TUI 模型面板」的自动化证据）。
//!
//! 这组用例锁的是**行为**而不是渲染：
//!   1. 面板打开时 ↑/↓ 只移动光标，且**不绕回**（绕回会让长按 ↓ 从末项跳回首项，
//!      再按 Enter 就激活错档案）；
//!   2. Enter 只在"光标项 ≠ 当前激活项"时产生 `ModelOp::Activate`，且 id 取自列表，
//!      不重新读盘；
//!   3. 空列表时 Enter 不得产生任何模型操作（否则会用空 id 去打网关）；
//!   4. 非模型面板时 ↑/↓ 仍归历史召回（不得被面板抢走）。
//!
//! 面板数据来自 `WorkbenchState::model_panel`（事件驱动投影），测试直接构造它，
//! 不依赖开发机上的 `models.json` —— 否则这组用例会在别人的机器上飘。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use marguerite_core::tui::workbench::state::{
    ModelItem, ModelPanelView, RailTab, UiEffect, UiEvent, WorkbenchState,
};

fn item(id: &str, active: bool) -> ModelItem {
    ModelItem {
        id: id.to_string(),
        label: format!("label-{id}"),
        provider: "deepseek".into(),
        base_url: "https://api.deepseek.com".into(),
        builtin: false,
        key_ready: active,
    }
}

fn state_with_panel(ids: &[&str], active: &str) -> WorkbenchState {
    let mut st = WorkbenchState::new("s".into(), String::new());
    st.rail_tab = RailTab::Models;
    let items: Vec<ModelItem> = ids.iter().map(|id| item(id, *id == active)).collect();
    let cursor = items
        .iter()
        .position(|i| i.id == active)
        .unwrap_or_default();
    st.model_panel = ModelPanelView {
        items,
        active_id: active.to_string(),
        cursor,
        error: None,
    };
    st
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn cursor_opens_on_active_and_clamps_at_both_ends() {
    let mut st = state_with_panel(&["a", "b", "c"], "b");
    assert_eq!(st.model_panel.cursor, 1, "打开时应停在当前激活项");

    st.reduce(UiEvent::Key(key(KeyCode::Down)));
    assert_eq!(st.model_panel.cursor, 2);
    // 已经在末项：再按不得绕回 0
    st.reduce(UiEvent::Key(key(KeyCode::Down)));
    assert_eq!(st.model_panel.cursor, 2, "末项不得绕回");

    st.reduce(UiEvent::Key(key(KeyCode::Up)));
    st.reduce(UiEvent::Key(key(KeyCode::Up)));
    st.reduce(UiEvent::Key(key(KeyCode::Up)));
    assert_eq!(st.model_panel.cursor, 0, "首项不得下溢");
}

#[test]
fn enter_on_other_profile_emits_activate_with_that_id() {
    let mut st = state_with_panel(&["a", "b", "c"], "a");
    st.reduce(UiEvent::Key(key(KeyCode::Down)));
    let effects = st.reduce(UiEvent::Key(key(KeyCode::Enter)));
    let activated = effects.iter().find_map(|e| match e {
        UiEffect::ModelAction(op) => Some(format!("{op:?}")),
        _ => None,
    });
    let activated = activated.expect("Enter 必须产生 ModelAction");
    assert!(
        activated.contains("Activate(\"b\")"),
        "应按光标项 id 激活，实际: {activated}"
    );
}

#[test]
fn enter_on_active_profile_is_a_noop() {
    let mut st = state_with_panel(&["a", "b"], "a");
    // 光标就在激活项：重复激活只会多打一次磁盘写 + 一次事件广播
    let effects = st.reduce(UiEvent::Key(key(KeyCode::Enter)));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, UiEffect::ModelAction(_))),
        "对当前激活项按 Enter 不应产生写入: {effects:?}"
    );
}

#[test]
fn enter_on_empty_panel_never_emits_activate() {
    let mut st = WorkbenchState::new("s".into(), String::new());
    st.rail_tab = RailTab::Models;
    let effects = st.reduce(UiEvent::Key(key(KeyCode::Enter)));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, UiEffect::ModelAction(_))),
        "空面板不得用空 id 调网关: {effects:?}"
    );
}

#[test]
fn arrow_keys_still_recall_history_outside_model_panel() {
    let mut st = WorkbenchState::new("s".into(), String::new());
    st.rail_tab = RailTab::Agent;
    st.composer.history.push("first prompt");
    st.composer.buffer.clear();
    st.reduce(UiEvent::Key(key(KeyCode::Up)));
    assert_eq!(
        st.composer.buffer, "first prompt",
        "非模型面板时 ↑ 必须仍走历史召回"
    );
}
