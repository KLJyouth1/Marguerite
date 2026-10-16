//! 无障碍树（Accessibility Tree）UI 结构通道 —— WP3 抽象层。
//!
//! 目标（参照 GitHub lahfir/agent-desktop 的思路）：在现有「像素 computer_use」
//! 之外，提供一条「读 OS 原生 UI 树」的通道 —— 不猜像素坐标，而是直接枚举 OS
//! 暴露给辅助技术的控件树（role / name / bounds / 是否可点），让智能体按稳定的
//! `ref_id` 定位与点击控件。
//!
//! # 当前边界（重要）
//!
//! 本模块**只交付可测的抽象层 + 内存 Stub**：
//! - [`UiNode`] / [`UiSnapshot`] / [`A11ySource`] trait 是稳定契约；
//! - [`StubA11ySource`] 在内存里塞几个假节点，供单测与「尚未接入真实 UIA / AT-SPI」
//!   时的软失败路径使用（对齐现有 vision / uia 的「未接真实 OS 即明确报错、不 panic」风格）。
//!
//! **真实 OS 接入 = 后续 step，当前未做**：Windows UI Automation (UIA) COM 客户端、
//! Linux AT-SPI 桥接均未在此实现。后续 step 只需为某个具体 OS 实现 [`A11ySource`]
//! trait（例如 `WindowsUiaSource`），并把 `ref_id` 映射到 UIA element 运行时 ID /
//! native handle，无需改动本模块的数据契约。

use std::fmt;

/// 单个 UI 控件节点（OS 无障碍树中的一个元素）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiNode {
    /// 控件角色（OS 给出的语义类型，如 "button" / "edit" / "menu_item"）。
    pub role: String,
    /// 控件可见名称/标签。
    pub name: String,
    /// 稳定引用 ID：跨快照可复现的句柄（真实 OS 接入后映射到 UIA runtime ID 等）。
    pub ref_id: String,
    /// 屏幕包围盒 `(x, y, width, height)`。
    pub bounds: (i32, i32, i32, i32),
    /// 是否可交互（可点 / 可激活）。
    pub clickable: bool,
}

/// 一次完整的 UI 树快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiSnapshot {
    /// 平铺的节点列表（后续可按需加父子层级；当前保持最小可测结构）。
    pub nodes: Vec<UiNode>,
}

/// 无障碍通道错误。软失败：未接真实 OS 时返回 `Unavailable`，不 panic。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum A11yError {
    /// 无障碍通道不可用（未接入真实 OS / 无权限读取 UI 树）。
    /// 对齐现有 vision 通道的「unavailable 软失败」风格：明确告知能力缺失，而非 panic。
    Unavailable(String),
    /// 按 `ref_id` 找不到目标节点。
    NodeNotFound(String),
    /// 通道在读取/操控过程中失败（真实 OS 接入后承载 COM / IPC 错误）。
    OsFailure(String),
}

impl fmt::Display for A11yError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            A11yError::Unavailable(msg) => write!(f, "a11y channel unavailable: {msg}"),
            A11yError::NodeNotFound(id) => write!(f, "a11y node not found: {id}"),
            A11yError::OsFailure(msg) => write!(f, "a11y os failure: {msg}"),
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
/// 真实 OS 接入（Windows UIA COM / Linux AT-SPI）为后续 step；当前只有
/// [`StubA11ySource`] 提供内存实现。
pub trait A11ySource {
    /// 抓取当前 UI 树快照。
    fn snapshot(&self) -> Result<UiSnapshot, A11yError>;
    /// 按 `ref_id` 点击/激活控件。
    fn click(&self, ref_id: &str) -> Result<(), A11yError>;
}

/// 内存 Stub 无障碍源：塞几个假节点，供单测与「未接真实 OS」时的软失败演示。
///
/// 它**不代表**真实 UI 状态；存在的意义是让 trait / 数据契约在没有 UIA COM 的机器上
/// 也能被编译与单测，同时给上层一个「可用但为假数据」的降级实现。
#[derive(Debug, Clone)]
pub struct StubA11ySource {
    nodes: Vec<UiNode>,
    /// 若为 `Some`，`snapshot()` 直接返回该错误 —— 用于模拟通道不可用的软失败路径。
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
            UiNode {
                role: "button".into(),
                name: "确定".into(),
                ref_id: "btn-ok".into(),
                bounds: (100, 200, 80, 32),
                clickable: true,
            },
            UiNode {
                role: "button".into(),
                name: "取消".into(),
                ref_id: "btn-cancel".into(),
                bounds: (200, 200, 80, 32),
                clickable: true,
            },
            UiNode {
                role: "edit".into(),
                name: "用户名".into(),
                ref_id: "edit-user".into(),
                bounds: (100, 100, 200, 28),
                clickable: true,
            },
            UiNode {
                role: "text".into(),
                name: "标题栏".into(),
                ref_id: "static-title".into(),
                bounds: (0, 0, 400, 40),
                clickable: false,
            },
        ])
    }

    /// 让 Stub 进入「通道不可用」状态（模拟未接真实 UIA 时的软失败）。
    pub fn with_unavailable(mut self, reason: impl Into<String>) -> Self {
        self.unavailable = Some(reason.into());
        self
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
        // 真实 OS 接入后此处解析 ref_id -> UIA element 并 RaiseAutomationEvent / Invoke。
        // 当前 Stub：确认节点存在即视为「点击成功」，否则按契约返回 NodeNotFound。
        self.nodes
            .iter()
            .find(|n| n.ref_id == ref_id)
            .ok_or_else(|| A11yError::NodeNotFound(ref_id.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> UiSnapshot {
        UiSnapshot {
            nodes: vec![
                UiNode {
                    role: "button".into(),
                    name: "确定".into(),
                    ref_id: "btn-ok".into(),
                    bounds: (100, 200, 80, 32),
                    clickable: true,
                },
                UiNode {
                    role: "text".into(),
                    name: "标题".into(),
                    ref_id: "static-title".into(),
                    bounds: (0, 0, 400, 40),
                    clickable: false,
                },
                UiNode {
                    role: "edit".into(),
                    name: "用户名".into(),
                    ref_id: "edit-user".into(),
                    bounds: (100, 100, 200, 28),
                    clickable: true,
                },
            ],
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
        // static-title 不可点，应被过滤；btn-ok 与 edit-user 可点，应保留。
        assert_eq!(ids, vec!["btn-ok", "edit-user"]);
        assert!(interactive.iter().all(|n| n.clickable));
    }

    #[test]
    fn stub_snapshot_returns_demo_nodes() {
        let stub = StubA11ySource::demo();
        let snap = stub.snapshot().expect("stub snapshot should be ok");
        assert_eq!(snap.nodes.len(), 4);
        assert!(snap.find_by_ref("btn-ok").is_some());
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
    fn stub_unavailable_soft_fails_on_snapshot_and_click() {
        let stub = StubA11ySource::demo().with_unavailable("real UIA not wired");
        match stub.snapshot() {
            Err(A11yError::Unavailable(reason)) => assert_eq!(reason, "real UIA not wired"),
            other => panic!("expected Unavailable, got {other:?}"),
        }
        assert!(matches!(
            stub.click("btn-ok"),
            Err(A11yError::Unavailable(_))
        ));
    }
}
