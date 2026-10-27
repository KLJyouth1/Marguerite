//! 无障碍树（Accessibility Tree）跨平台抽象层。
//!
//! **本文件只定义抽象与内存 Stub，不含任何 OS 调用。** 真实的 UIA 实现在
//! `crate::desktop_automation::windows_uia::WindowsUiaSource`（Windows，COM/UI Automation；
//! 该模块仅 `cfg(windows)` 编译），由 [`crate::desktop_automation::uia_driver::UiaDriver`]
//! 与 `desktop.a11y` RPC 共用：
//!
//! - [`UiNode`] / [`UiSnapshot`] / [`NodeState`] / [`PatternOp`] 是稳定数据契约（跨平台）；
//! - [`A11ySource`] 是平台后端必须实现的 trait（`WindowsUiaSource` 已接通真实 COM）；
//! - [`StubA11ySource`] 是内存假数据源，只供单测与「没有桌面会话」时的软失败路径使用。
//!
//! # 观测面（observe→act→verify）
//!
//! 动作执行前/后各读一次 [`NodeState`]：`digest()` 是稳定的内容摘要，用来判断
//! 「value 变了 / 节点消失了 / 状态变了 / 无可观测变化」。摘要算法是自实现的
//! FNV-1a，不依赖 `DefaultHasher`（后者跨 Rust 版本不保证稳定）。

use std::fmt;

/// 单次快照的节点上限（跨平台统一口径；Windows 后端同样遵守）。
pub const MAX_NODES: usize = 400;
/// 单次快照的递归深度上限。
pub const MAX_DEPTH: usize = 14;
/// `find` 默认返回条数（对齐 W6.1「最多 20」）。
pub const MAX_FIND_RESULTS: usize = 20;

/// UIA / AT-SPI 的 pattern 语义（跨平台抽象，不是 COM 接口名）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiPattern {
    Invoke,
    Toggle,
    SelectionItem,
    Value,
    ExpandCollapse,
    Scroll,
    SetFocus,
}

impl UiPattern {
    pub fn as_str(&self) -> &'static str {
        match self {
            UiPattern::Invoke => "invoke",
            UiPattern::Toggle => "toggle",
            UiPattern::SelectionItem => "selection_item",
            UiPattern::Value => "value",
            UiPattern::ExpandCollapse => "expand_collapse",
            UiPattern::Scroll => "scroll",
            UiPattern::SetFocus => "set_focus",
        }
    }

    pub fn all() -> &'static [UiPattern] {
        &[
            UiPattern::Invoke,
            UiPattern::Toggle,
            UiPattern::SelectionItem,
            UiPattern::Value,
            UiPattern::ExpandCollapse,
            UiPattern::Scroll,
            UiPattern::SetFocus,
        ]
    }
}

/// 对单个节点执行的 pattern 级操作（`A11ySource::perform` 的入参）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternOp {
    Invoke,
    Toggle,
    Select,
    Expand,
    Collapse,
    Focus,
    SetValue(String),
    /// `clear=true` 表示替换整段文本，否则在现有值后追加。
    TypeText { text: String, clear: bool },
    /// `horizontal` / `vertical` 是滚动格数（正数向前，负数向后）。
    Scroll { horizontal: i32, vertical: i32 },
}

impl PatternOp {
    /// 该操作依赖哪个 [`UiPattern`]。Windows 后端据此做 `GetCurrentPatternAs` 分发。
    pub fn pattern(&self) -> UiPattern {
        match self {
            PatternOp::Invoke => UiPattern::Invoke,
            PatternOp::Toggle => UiPattern::Toggle,
            PatternOp::Select => UiPattern::SelectionItem,
            PatternOp::Expand | PatternOp::Collapse => UiPattern::ExpandCollapse,
            PatternOp::Focus => UiPattern::SetFocus,
            PatternOp::SetValue(_) | PatternOp::TypeText { .. } => UiPattern::Value,
            PatternOp::Scroll { .. } => UiPattern::Scroll,
        }
    }

    /// 人类可读的动作标签（写进审计 `params`，便于事后回放）。
    pub fn as_str(&self) -> &'static str {
        match self {
            PatternOp::Invoke => "invoke",
            PatternOp::Toggle => "toggle",
            PatternOp::Select => "select",
            PatternOp::Expand => "expand",
            PatternOp::Collapse => "collapse",
            PatternOp::Focus => "focus",
            PatternOp::SetValue(_) => "set_value",
            PatternOp::TypeText { .. } => "type",
            PatternOp::Scroll { .. } => "scroll",
        }
    }
}

/// 单个 UI 控件节点（OS 无障碍树中的一个元素）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiNode {
    /// 控件角色（OS 给出的语义类型，如 "button" / "edit" / "menu_item"）。
    pub role: String,
    /// 控件可见名称/标签。
    pub name: String,
    /// 稳定引用 ID：跨快照可复现的句柄（Windows 上是 UIA runtime ID）。
    pub ref_id: String,
    /// OS 声明的自动化 ID（UIA `AutomationId`；很多控件为空）。
    pub automation_id: String,
    /// 屏幕包围盒 `(x, y, width, height)`。
    pub bounds: (i32, i32, i32, i32),
    /// 是否可交互（可点 / 可激活）。
    pub clickable: bool,
    /// 是否启用（UIA `IsEnabled`）。
    pub enabled: bool,
    /// 当前是否持有键盘焦点（UIA `HasKeyboardFocus`）。
    pub focused: bool,
    /// 该节点支持哪些 pattern（真实后端逐个探测；Stub 可用 [`UiNode::with_patterns`] 预置）。
    pub patterns: Vec<UiPattern>,
}

impl UiNode {
    /// 最小构造：只填位置/名称这类必填项，其余取安全的默认值。
    pub fn new(
        role: impl Into<String>,
        name: impl Into<String>,
        ref_id: impl Into<String>,
        bounds: (i32, i32, i32, i32),
        clickable: bool,
    ) -> Self {
        UiNode {
            role: role.into(),
            name: name.into(),
            ref_id: ref_id.into(),
            automation_id: String::new(),
            bounds,
            clickable,
            enabled: clickable,
            focused: false,
            patterns: Vec::new(),
        }
    }

    pub fn with_automation_id(mut self, id: impl Into<String>) -> Self {
        self.automation_id = id.into();
        self
    }

    pub fn with_state(mut self, enabled: bool, focused: bool) -> Self {
        self.enabled = enabled;
        self.focused = focused;
        self
    }

    pub fn with_patterns(mut self, patterns: Vec<UiPattern>) -> Self {
        self.patterns = patterns;
        self
    }
}

/// 一次完整的 UI 树快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiSnapshot {
    /// 平铺的节点列表（按遍历顺序；根窗口本身不在列表里）。
    pub nodes: Vec<UiNode>,
}

/// 快照的边界参数（深度 / 节点数）。真实后端必须遵守，Stub 可以忽略。
#[derive(Debug, Clone, Copy, Default)]
pub struct SnapshotOptions {
    pub max_depth: Option<usize>,
    pub max_nodes: Option<usize>,
}

impl SnapshotOptions {
    /// 夹到合法范围：深度 1..=[`MAX_DEPTH`]，节点 1..=[`MAX_NODES`]。
    pub fn bounded(&self) -> (usize, usize) {
        (
            self.max_depth.unwrap_or(MAX_DEPTH).clamp(1, MAX_DEPTH),
            self.max_nodes.unwrap_or(MAX_NODES).clamp(1, MAX_NODES),
        )
    }
}

/// 按条件查找节点。空条件视为调用方错误（不做全树 dump）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeQuery {
    pub role: Option<String>,
    pub name: Option<String>,
    pub automation_id: Option<String>,
}

impl NodeQuery {
    pub fn is_empty(&self) -> bool {
        self.role.as_deref().map(str::trim).unwrap_or("").is_empty()
            && self.name.as_deref().map(str::trim).unwrap_or("").is_empty()
            && self
                .automation_id
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
    }

    /// 角色/自动化 ID 精确匹配（大小写不敏感），名称做大小写不敏感子串匹配。
    pub fn matches(&self, node: &UiNode) -> bool {
        let role = self.role.as_deref().unwrap_or("").trim();
        if !role.is_empty() && !node.role.eq_ignore_ascii_case(role) {
            return false;
        }
        let automation_id = self.automation_id.as_deref().unwrap_or("").trim();
        if !automation_id.is_empty() && node.automation_id != automation_id {
            return false;
        }
        let name = self.name.as_deref().unwrap_or("").trim();
        if !name.is_empty() && !node.name.to_lowercase().contains(&name.to_lowercase()) {
            return false;
        }
        true
    }
}

/// 单个节点的可观测状态（observe→act→verify 的观测载荷）。
///
/// 只放「比较前后是否变化」需要的字段；不含父子结构，避免摘要被遍历顺序影响。
#[derive(Debug, Clone, PartialEq)]
pub struct NodeState {
    pub ref_id: String,
    pub role: String,
    pub name: String,
    pub automation_id: String,
    pub value: Option<String>,
    pub enabled: bool,
    pub focused: bool,
    /// "on" / "off" / "indeterminate"（TogglePattern）
    pub toggle_state: Option<String>,
    /// "expanded" / "collapsed" / "partially_expanded" / "leaf"（ExpandCollapsePattern）
    pub expand_collapse_state: Option<String>,
    /// SelectionItemPattern.CurrentIsSelected
    pub selected: Option<bool>,
    /// ScrollPattern 的 (horizontal, vertical) 百分比
    pub scroll_percent: Option<(f64, f64)>,
    pub bounds: (i32, i32, i32, i32),
}

impl NodeState {
    /// 稳定内容摘要（FNV-1a 64 位，16 进制）。同一状态永远得到同一串。
    ///
    /// 不包含 `focused` 以外的瞬态信息之外的任何时间戳/顺序，因此「动作前后各读一次」
    /// 可以直接比较。
    pub fn digest(&self) -> String {
        let mut canon = String::new();
        canon.push_str(&self.ref_id);
        canon.push('\u{1f}');
        canon.push_str(&self.role);
        canon.push('\u{1f}');
        canon.push_str(&self.name);
        canon.push('\u{1f}');
        canon.push_str(&self.automation_id);
        canon.push('\u{1f}');
        canon.push_str(self.value.as_deref().unwrap_or(""));
        canon.push('\u{1f}');
        canon.push_str(if self.enabled { "1" } else { "0" });
        canon.push('\u{1f}');
        canon.push_str(if self.focused { "1" } else { "0" });
        canon.push('\u{1f}');
        canon.push_str(self.toggle_state.as_deref().unwrap_or(""));
        canon.push('\u{1f}');
        canon.push_str(self.expand_collapse_state.as_deref().unwrap_or(""));
        canon.push('\u{1f}');
        canon.push_str(match self.selected {
            Some(true) => "1",
            Some(false) => "0",
            None => "",
        });
        canon.push('\u{1f}');
        if let Some((h, v)) = self.scroll_percent {
            canon.push_str(&format!("{h:.2}/{v:.2}"));
        }
        canon.push('\u{1f}');
        canon.push_str(&format!(
            "{},{},{},{}",
            self.bounds.0, self.bounds.1, self.bounds.2, self.bounds.3
        ));
        format!("{:016x}", fnv1a64(canon.as_bytes()))
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 无障碍通道错误。软失败：未接真实 OS 时返回 `Unavailable`，不 panic。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum A11yError {
    /// 无障碍通道不可用（未接入真实 OS / 无权限读取 UI 树）。
    Unavailable(String),
    /// 按 `ref_id` 找不到目标节点。
    NodeNotFound(String),
    /// 通道在读取/操控过程中失败（真实 OS 接入后承载 COM / IPC 错误）。
    OsFailure(String),
    /// 目标节点不支持该 pattern（例如对纯文本调 SetValue）。
    Unsupported(String),
}

impl fmt::Display for A11yError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            A11yError::Unavailable(msg) => write!(f, "a11y channel unavailable: {msg}"),
            A11yError::NodeNotFound(id) => write!(f, "a11y node not found: {id}"),
            A11yError::OsFailure(msg) => write!(f, "a11y os failure: {msg}"),
            A11yError::Unsupported(msg) => write!(f, "a11y unsupported: {msg}"),
        }
    }
}

impl std::error::Error for A11yError {}

impl UiSnapshot {
    /// 按 `ref_id` 查找节点（命中返回引用，未命中返回 `None`）。
    pub fn find_by_ref(&self, ref_id: &str) -> Option<&UiNode> {
        self.nodes.iter().find(|n| n.ref_id == ref_id)
    }

    /// 返回所有可交互（`clickable == true`）节点。
    pub fn interactive_nodes(&self) -> Vec<&UiNode> {
        self.nodes.iter().filter(|n| n.clickable).collect()
    }
}

/// 无障碍树通道抽象：读取 UI 结构并按稳定 `ref_id` 操控控件。
///
/// Windows 后端是 [`crate::desktop_automation::windows_uia::WindowsUiaSource`]；
/// 单测用内存 mock / [`StubA11ySource`]。
pub trait A11ySource: Send + Sync {
    /// 抓取当前 UI 树快照。
    fn snapshot(&self) -> Result<UiSnapshot, A11yError>;

    /// 带边界参数抓取快照。默认实现忽略边界（Stub 用）；真实后端必须遵守，
    /// 否则「大 UI 树拖死调用方」的防护就只存在于文档里。
    fn snapshot_with(&self, opts: &SnapshotOptions) -> Result<UiSnapshot, A11yError> {
        let _ = opts;
        self.snapshot()
    }

    /// 按 `ref_id` 点击/激活控件（Invoke → Toggle → Select → SetFocus 的退化链）。
    fn click(&self, ref_id: &str) -> Result<(), A11yError>;

    /// 读单个节点的当前状态。节点不存在时**必须**返回 [`A11yError::NodeNotFound`]
    /// —— 上层据此判定 `node_removed`。
    fn node_state(&self, ref_id: &str) -> Result<NodeState, A11yError>;

    /// 对节点执行一个 pattern 级操作。不支持时返回 [`A11yError::Unsupported`]。
    fn perform(&self, ref_id: &str, op: &PatternOp) -> Result<(), A11yError>;

    /// 按条件查找节点（最多 `limit` 条）。默认实现从快照过滤，够用且实现成本低。
    fn find(&self, query: &NodeQuery, limit: usize) -> Result<Vec<UiNode>, A11yError> {
        if query.is_empty() {
            return Err(A11yError::OsFailure(
                "find 需要 role / name / automation_id 中至少一个条件".into(),
            ));
        }
        let cap = limit.clamp(1, MAX_NODES);
        Ok(self
            .snapshot()?
            .nodes
            .into_iter()
            .filter(|n| query.matches(n))
            .take(cap)
            .collect())
    }
}

/// 内存 Stub 无障碍源：塞几个假节点，供单测与「未接真实 OS」时的软失败演示。
///
/// 它**不代表**真实 UI 状态；`perform` 只在该节点**声明支持**对应 pattern 时才返回成功，
/// 否则如实报 [`A11yError::Unsupported`]（不假装执行成功）。
#[derive(Debug, Clone)]
pub struct StubA11ySource {
    nodes: Vec<UiNode>,
    /// 若为 `Some`，所有调用直接返回该错误 —— 用于模拟通道不可用的软失败路径。
    unavailable: Option<String>,
}

impl StubA11ySource {
    /// 构造一个带假节点的 Stub。
    pub fn new(nodes: Vec<UiNode>) -> Self {
        Self {
            nodes,
            unavailable: None,
        }
    }

    /// 构造一个预置了几个典型控件的 Stub（按钮 / 输入框 / 静态文本），便于演示与测试。
    pub fn demo() -> Self {
        Self::new(vec![
            UiNode::new("button", "确定", "btn-ok", (100, 200, 80, 32), true)
                .with_automation_id("OkButton")
                .with_patterns(vec![UiPattern::Invoke]),
            UiNode::new("button", "取消", "btn-cancel", (200, 200, 80, 32), true)
                .with_automation_id("CancelButton")
                .with_patterns(vec![UiPattern::Invoke]),
            UiNode::new("edit", "用户名", "edit-user", (100, 100, 200, 28), true)
                .with_automation_id("UserEdit")
                .with_patterns(vec![UiPattern::Value, UiPattern::SetFocus]),
            UiNode::new("text", "标题栏", "static-title", (0, 0, 400, 40), false),
        ])
    }

    /// 让 Stub 进入「通道不可用」状态（模拟未接真实 UIA 时的软失败）。
    pub fn with_unavailable(mut self, reason: impl Into<String>) -> Self {
        self.unavailable = Some(reason.into());
        self
    }

    fn node(&self, ref_id: &str) -> Result<&UiNode, A11yError> {
        self.nodes
            .iter()
            .find(|n| n.ref_id == ref_id)
            .ok_or_else(|| A11yError::NodeNotFound(ref_id.to_string()))
    }
}

impl A11ySource for StubA11ySource {
    fn snapshot(&self) -> Result<UiSnapshot, A11yError> {
        if let Some(reason) = &self.unavailable {
            return Err(A11yError::Unavailable(reason.clone()));
        }
        Ok(UiSnapshot {
            nodes: self.nodes.clone(),
        })
    }

    fn click(&self, ref_id: &str) -> Result<(), A11yError> {
        if let Some(reason) = &self.unavailable {
            return Err(A11yError::Unavailable(reason.clone()));
        }
        self.node(ref_id)?;
        Ok(())
    }

    fn node_state(&self, ref_id: &str) -> Result<NodeState, A11yError> {
        if let Some(reason) = &self.unavailable {
            return Err(A11yError::Unavailable(reason.clone()));
        }
        let n = self.node(ref_id)?;
        Ok(NodeState {
            ref_id: n.ref_id.clone(),
            role: n.role.clone(),
            name: n.name.clone(),
            automation_id: n.automation_id.clone(),
            value: None,
            enabled: n.enabled,
            focused: n.focused,
            toggle_state: None,
            expand_collapse_state: None,
            selected: None,
            scroll_percent: None,
            bounds: n.bounds,
        })
    }

    fn perform(&self, ref_id: &str, op: &PatternOp) -> Result<(), A11yError> {
        if let Some(reason) = &self.unavailable {
            return Err(A11yError::Unavailable(reason.clone()));
        }
        let n = self.node(ref_id)?;
        let want = op.pattern();
        if n.patterns.contains(&want) {
            Ok(())
        } else {
            Err(A11yError::Unsupported(format!(
                "节点 {}（{}）不支持 {} pattern",
                n.ref_id,
                n.role,
                want.as_str()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> UiSnapshot {
        UiSnapshot {
            nodes: vec![
                UiNode::new("button", "确定", "btn-ok", (100, 200, 80, 32), true),
                UiNode::new("text", "标题", "static-title", (0, 0, 400, 40), false),
                UiNode::new("edit", "用户名", "edit-user", (100, 100, 200, 28), true),
            ],
        }
    }

    fn state(value: Option<&str>, enabled: bool, focused: bool) -> NodeState {
        NodeState {
            ref_id: "edit-user".into(),
            role: "edit".into(),
            name: "用户名".into(),
            automation_id: "UserEdit".into(),
            value: value.map(str::to_string),
            enabled,
            focused,
            toggle_state: None,
            expand_collapse_state: None,
            selected: None,
            scroll_percent: None,
            bounds: (100, 100, 200, 28),
        }
    }

    #[test]
    fn find_by_ref_hits_existing_node() {
        let snap = sample();
        let node = snap.find_by_ref("btn-ok").expect("should find btn-ok");
        assert_eq!(node.name, "确定");
        assert_eq!(node.role, "button");
        assert_eq!(node.bounds, (100, 200, 80, 32));
        assert!(node.clickable);
    }

    #[test]
    fn find_by_ref_missing_returns_none() {
        let snap = sample();
        assert!(snap.find_by_ref("does-not-exist").is_none());
    }

    #[test]
    fn interactive_nodes_filters_clickable_only() {
        let snap = sample();
        let interactive = snap.interactive_nodes();
        let ids: Vec<&str> = interactive.iter().map(|n| n.ref_id.as_str()).collect();
        assert_eq!(ids, vec!["btn-ok", "edit-user"]);
        assert!(interactive.iter().all(|n| n.clickable));
    }

    #[test]
    fn stub_snapshot_returns_demo_nodes() {
        let stub = StubA11ySource::demo();
        let snap = stub.snapshot().expect("stub snapshot should be ok");
        assert_eq!(snap.nodes.len(), 4);
        assert!(snap.find_by_ref("btn-ok").is_some());
        // pattern 预置必须可读（W6.1 的 pattern 分发依赖它）。
        let edit = snap.find_by_ref("edit-user").unwrap();
        assert!(edit.patterns.contains(&UiPattern::Value));
        assert!(!edit.patterns.contains(&UiPattern::Invoke));
    }

    #[test]
    fn stub_click_existing_ref_succeeds() {
        let stub = StubA11ySource::demo();
        assert!(stub.click("btn-ok").is_ok());
    }

    #[test]
    fn stub_click_unknown_ref_is_node_not_found() {
        let stub = StubA11ySource::demo();
        let err = stub.click("nope").unwrap_err();
        assert_eq!(err, A11yError::NodeNotFound("nope".to_string()));
    }

    #[test]
    fn stub_unavailable_soft_fails_on_every_entry_point() {
        let stub = StubA11ySource::demo().with_unavailable("real UIA not wired");
        assert!(matches!(stub.snapshot(), Err(A11yError::Unavailable(_))));
        assert!(matches!(stub.click("btn-ok"), Err(A11yError::Unavailable(_))));
        assert!(matches!(
            stub.node_state("btn-ok"),
            Err(A11yError::Unavailable(_))
        ));
        assert!(matches!(
            stub.perform("btn-ok", &PatternOp::Invoke),
            Err(A11yError::Unavailable(_))
        ));
    }

    #[test]
    fn stub_perform_honours_declared_patterns() {
        let stub = StubA11ySource::demo();
        assert!(stub.perform("btn-ok", &PatternOp::Invoke).is_ok());
        let err = stub
            .perform("static-title", &PatternOp::SetValue("x".into()))
            .expect_err("静态文本不支持 ValuePattern");
        assert!(matches!(err, A11yError::Unsupported(_)), "实际: {err:?}");
        let missing = stub
            .perform("nope", &PatternOp::Toggle)
            .expect_err("不存在的节点必须报 NodeNotFound");
        assert_eq!(missing, A11yError::NodeNotFound("nope".into()));
    }

    #[test]
    fn node_query_requires_a_condition_and_matches_fuzzy_name() {
        let empty = NodeQuery::default();
        assert!(empty.is_empty());
        let by_name = NodeQuery {
            name: Some("确定".into()),
            ..Default::default()
        };
        assert!(by_name.matches(&UiNode::new(
            "button",
            "确定并继续",
            "b",
            (0, 0, 1, 1),
            true
        )));
        let by_role = NodeQuery {
            role: Some("EDIT".into()),
            ..Default::default()
        };
        assert!(by_role.matches(&UiNode::new("edit", "x", "e", (0, 0, 1, 1), true)));
        let by_aid = NodeQuery {
            automation_id: Some("UserEdit".into()),
            ..Default::default()
        };
        assert!(by_aid.matches(
            &UiNode::new("edit", "x", "e", (0, 0, 1, 1), true).with_automation_id("UserEdit")
        ));
        assert!(!by_aid.matches(&UiNode::new("edit", "x", "e", (0, 0, 1, 1), true)));
    }

    #[test]
    fn find_default_impl_filters_and_caps() {
        let stub = StubA11ySource::demo();
        let q = NodeQuery {
            role: Some("button".into()),
            ..Default::default()
        };
        let hits = stub.find(&q, 1).expect("find");
        assert_eq!(hits.len(), 1, "limit 必须生效");
        let all = stub.find(&q, 20).expect("find");
        assert_eq!(all.len(), 2, "两个 button");
        let err = stub.find(&NodeQuery::default(), 5).unwrap_err();
        assert!(matches!(err, A11yError::OsFailure(_)));
    }

    #[test]
    fn node_state_digest_is_stable_and_distinguishes_fields() {
        let a = state(Some("abc"), true, false);
        let b = state(Some("abc"), true, false);
        assert_eq!(a.digest(), b.digest(), "同一状态必须得到同一摘要");
        assert_eq!(a.digest().len(), 16);

        let value_changed = state(Some("abd"), true, false);
        assert_ne!(a.digest(), value_changed.digest());

        let focus_changed = state(Some("abc"), true, true);
        assert_ne!(a.digest(), focus_changed.digest());

        let enabled_changed = state(Some("abc"), false, false);
        assert_ne!(a.digest(), enabled_changed.digest());
    }

    #[test]
    fn pattern_op_maps_to_one_pattern_each() {
        assert_eq!(PatternOp::Invoke.pattern(), UiPattern::Invoke);
        assert_eq!(PatternOp::Toggle.pattern(), UiPattern::Toggle);
        assert_eq!(PatternOp::Select.pattern(), UiPattern::SelectionItem);
        assert_eq!(PatternOp::Expand.pattern(), UiPattern::ExpandCollapse);
        assert_eq!(PatternOp::Collapse.pattern(), UiPattern::ExpandCollapse);
        assert_eq!(PatternOp::Focus.pattern(), UiPattern::SetFocus);
        assert_eq!(PatternOp::SetValue("x".into()).pattern(), UiPattern::Value);
        assert_eq!(
            PatternOp::TypeText {
                text: "x".into(),
                clear: true
            }
            .pattern(),
            UiPattern::Value
        );
        assert_eq!(
            PatternOp::Scroll {
                horizontal: 0,
                vertical: 3
            }
            .pattern(),
            UiPattern::Scroll
        );
        // 每个 pattern 都必须有操作可达（反方向自证没有死枚举）。
        for p in UiPattern::all() {
            let ops = [
                PatternOp::Invoke,
                PatternOp::Toggle,
                PatternOp::Select,
                PatternOp::Expand,
                PatternOp::Collapse,
                PatternOp::Focus,
                PatternOp::SetValue(String::new()),
                PatternOp::TypeText {
                    text: String::new(),
                    clear: false,
                },
                PatternOp::Scroll {
                    horizontal: 0,
                    vertical: 0,
                },
            ];
            assert!(ops.iter().any(|o| o.pattern() == *p), "{p:?} 无操作可达");
        }
    }

    #[test]
    fn snapshot_options_are_clamped_to_hard_limits() {
        let (d, n) = SnapshotOptions {
            max_depth: Some(9999),
            max_nodes: Some(0),
        }
        .bounded();
        assert_eq!(d, MAX_DEPTH);
        assert_eq!(n, 1, "0 必须夹到 1，而不是返回空树");
        let (d, n) = SnapshotOptions::default().bounded();
        assert_eq!((d, n), (MAX_DEPTH, MAX_NODES));
    }
}
