//! 桌面应用驱动抽象与共享类型。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::desktop_automation::a11y_tree::NodeState;

/// 受支持的桌面应用种类（对齐 SPEC §6 DesktopAppDriver）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppKind {
    WpsPpt,
    Browser,
    Jianying,
    ThreeD,
    Generic,
}

impl AppKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AppKind::WpsPpt => "wps_ppt",
            AppKind::Browser => "browser",
            AppKind::Jianying => "jianying",
            AppKind::ThreeD => "three_d",
            AppKind::Generic => "generic",
        }
    }

    /// 从字符串解析（接受 snake_case 或原始名），未知返回 None。
    #[allow(clippy::should_implement_trait)] // 命名仿 FromStr 但返回 Option，保留为解析辅助
    pub fn from_str(s: &str) -> Option<AppKind> {
        match s {
            "wps_ppt" | "WpsPpt" => Some(AppKind::WpsPpt),
            "browser" | "Browser" => Some(AppKind::Browser),
            "jianying" | "Jianying" => Some(AppKind::Jianying),
            "three_d" | "ThreeD" => Some(AppKind::ThreeD),
            "generic" | "Generic" => Some(AppKind::Generic),
            _ => None,
        }
    }
}

/// 框架级错误（映射到网关 RPC 错误码，见 protocol.rs）。
#[derive(Debug)]
pub enum DesktopError {
    AppUnknown(String),
    ActionDenied(String),
    ComError(String),
    UnsupportedAction(String),
    NotRunning(String),
    /// 元素级定位失败：应用在跑、通道也通，但按 `ref_id` / `name` 找不到那个控件。
    ///
    /// 单列一个变体是为了不把「找不到控件」混进 `ComError`（那是通道/OS 层失败）或
    /// `NotRunning`（那是应用层失败）——UIA 驱动的 `click` 恰恰最容易命中这一种。
    ElementNotFound(String),
    /// 窗口级定位失败：没有前台窗口，或按标题/进程名找不到目标窗口。
    /// 网关映射为 `WINDOW_NOT_FOUND`（与元素级 `ELEMENT_NOT_FOUND` 区分）。
    WindowNotFound(String),
    /// 等待超时（`wait` 动作轮询到截止时间仍未满足条件）。
    Timeout(String),
}

impl std::fmt::Display for DesktopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DesktopError::AppUnknown(s) => write!(f, "未知应用: {s}"),
            DesktopError::ActionDenied(s) => write!(f, "动作被安全闸门拒绝: {s}"),
            DesktopError::ComError(s) => write!(f, "COM 错误: {s}"),
            DesktopError::UnsupportedAction(s) => write!(f, "不支持的桌面动作: {s}"),
            DesktopError::NotRunning(s) => write!(f, "应用未运行: {s}"),
            DesktopError::ElementNotFound(s) => write!(f, "控件未找到: {s}"),
            DesktopError::WindowNotFound(s) => write!(f, "窗口未找到: {s}"),
            DesktopError::Timeout(s) => write!(f, "动作超时: {s}"),
        }
    }
}

impl std::error::Error for DesktopError {}

/// 所有驱动**声称支持**的 `UiAction.kind`（网关与工具在碰 UI 之前用它做未知动作拦截）。
///
/// 这张表是「未知动作立刻返回结构化错误」的唯一事实源；新增动作必须同时在这里、
/// 驱动 `run_action` 的 match 臂、以及工具描述里出现，否则会被判为不支持。
pub const KNOWN_ACTION_KINDS: &[&str] = &[
    // 通用 UIA 驱动（AppKind::Generic）
    "snapshot",
    "find",
    "click",
    "invoke",
    "set_value",
    "type",
    "toggle",
    "select",
    "expand",
    "collapse",
    "scroll",
    "focus",
    "wait",
    "window_activate",
    // WPS COM 驱动（AppKind::WpsPpt）
    "generate_presentation",
];

/// `kind` 是否是已登记的动作（大小写敏感，与驱动 match 臂一致）。
pub fn is_known_action_kind(kind: &str) -> bool {
    KNOWN_ACTION_KINDS.contains(&kind)
}

/// 矩形（网关/审计统一用 `{x,y,width,height}`，而不是四元组数组）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl From<(i32, i32, i32, i32)> for Bounds {
    fn from(b: (i32, i32, i32, i32)) -> Self {
        Bounds {
            x: b.0,
            y: b.1,
            width: b.2,
            height: b.3,
        }
    }
}

/// 已启动应用的句柄（进程内标识）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppHandle {
    pub id: String,
    pub kind: AppKind,
    pub launched_at: i64,
}

impl AppHandle {
    pub fn new(kind: AppKind) -> Self {
        AppHandle {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            launched_at: chrono::Utc::now().timestamp(),
        }
    }
}

/// 应用注册信息（前端「已注册 + 运行中」展示）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub kind: AppKind,
    pub name: String,
    pub registered: bool,
    pub running: bool,
}

/// 单次 UI 动作（驱动无关，供 Recorder 落盘 + Policy 裁决）。
///
/// # 序列化契约（稳定，网关/前端/模型共用）
///
/// ```json
/// { "kind": "set_value", "params": { "ref_id": "42.1", "value": "hello" } }
/// ```
///
/// - `kind` 必须在 [`KNOWN_ACTION_KINDS`] 里（[`is_known_action_kind`] 是唯一判定入口）；
/// - `params` 缺省为 `null`（`#[serde(default)]`），驱动侧按「未提供」处理，不 panic；
/// - 新增字段只允许**可选**语义，否则旧调用方会被反序列化拒绝。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiAction {
    /// 动作类型（snapshot / find / invoke / set_value / generate_presentation ...）。
    pub kind: String,
    #[serde(default)]
    pub params: Value,
}

impl UiAction {
    /// 按 `kind` + `params` 构造（网关/工具/单测共用的唯一构造入口）。
    pub fn new(kind: impl Into<String>, params: Value) -> Self {
        UiAction {
            kind: kind.into(),
            params,
        }
    }

    pub fn param_str(&self, key: &str) -> Option<String> {
        self.params
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }
    pub fn param_bool(&self, key: &str) -> bool {
        self.params
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
    pub fn param_i64(&self, key: &str, default: i64) -> i64 {
        self.params
            .get(key)
            .and_then(Value::as_i64)
            .unwrap_or(default)
    }
    /// 空字符串视为「未提供」（模型/前端很容易传 `""`）。
    pub fn param_str_nonempty(&self, key: &str) -> Option<String> {
        self.param_str(key).filter(|s| !s.trim().is_empty())
    }
}

/// observe→act→verify 的判定结果（写进 `ActionReceipt.effect`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationEffect {
    ValueChanged,
    NodeRemoved,
    StateChanged,
    NoObservableChange,
}

impl ObservationEffect {
    pub fn as_str(&self) -> &'static str {
        match self {
            ObservationEffect::ValueChanged => "value_changed",
            ObservationEffect::NodeRemoved => "node_removed",
            ObservationEffect::StateChanged => "state_changed",
            ObservationEffect::NoObservableChange => "no_observable_change",
        }
    }

    /// `no_observable_change` 不算「已验证」——动作可能成功，但回执必须如实说没有观测到变化。
    pub fn verified(&self) -> bool {
        !matches!(self, ObservationEffect::NoObservableChange)
    }
}

/// 纯函数：比较动作前后的节点状态，得出可观测效果。
///
/// 语义（与 W6.3 契约一一对应）：
/// - 前有后无 → `node_removed`（例如 invoke 关闭了对话框）；
/// - 前无后有 → `state_changed`（节点出现/换了一个）；
/// - value 变化 → `value_changed`；
/// - 其它摘要变化 → `state_changed`；
/// - 两边相同（或都读不到）→ `no_observable_change`。
pub fn observe_effect(before: Option<&NodeState>, after: Option<&NodeState>) -> ObservationEffect {
    match (before, after) {
        (Some(_), None) => ObservationEffect::NodeRemoved,
        (None, Some(_)) => ObservationEffect::StateChanged,
        (Some(b), Some(a)) => {
            if b.value != a.value {
                ObservationEffect::ValueChanged
            } else if b.digest() != a.digest() {
                ObservationEffect::StateChanged
            } else {
                ObservationEffect::NoObservableChange
            }
        }
        (None, None) => ObservationEffect::NoObservableChange,
    }
}

/// 动作执行回执（每步经 Recorder 落盘；敏感动作先查 Policy）。
///
/// # 序列化契约（稳定，网关 `desktop.action` 直接回给前端/模型）
///
/// ```json
/// {
///   "handle": "…", "action": "set_value", "ok": true,
///   "detail": { "ref_id": "42.1", "effect": "value_changed", "verified": true },
///   "snapshot_path": "…/audit-snapshots/<session>/<ts>.json",
///   "before": { "node_digest": "…", "value": "old", "enabled": true, "focused": true },
///   "after":  { "node_digest": "…", "value": "new", "enabled": true, "focused": true },
///   "verified": true, "effect": "value_changed", "ts": 1761264000
/// }
/// ```
///
/// - `before` / `after` / `effect` / `snapshot_path`：`None` 时**整个键消失**
///   （`skip_serializing_if`）。只读动作（snapshot / find / wait）恒为 `None`。
/// - `verified` **总是出现**（`#[serde(default)]`，缺省 `false`）：客户端不能把「键不存在」
///   读成「已验证」。
/// - `ok` 与 `verified` 正交：动作可能成功但无可观测变化（`ok=true, verified=false`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionReceipt {
    pub handle: String,
    pub action: String,
    pub ok: bool,
    pub detail: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_path: Option<String>,
    /// 动作前的目标摘要（`{ref_id, role, name, node_digest, value, enabled, focused, …}`）。
    ///
    /// 只读动作（snapshot / find / wait）为 `None`；写动作尽量给出，读不到目标时也为 `None`
    /// （诚实：不是「没变化」，而是「没观测到」）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
    /// 是否观测到预期效果。`false` **不**改变 `ok`（动作可能成功但无可见变化）。
    #[serde(default)]
    pub verified: bool,
    /// "value_changed" | "node_removed" | "state_changed" | "no_observable_change"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
    pub ts: i64,
}

impl ActionReceipt {
    pub fn ok(handle: &str, action: &str, detail: Value, snapshot: Option<PathBuf>) -> Self {
        ActionReceipt {
            handle: handle.to_string(),
            action: action.to_string(),
            ok: true,
            detail,
            snapshot_path: snapshot.map(|p| p.to_string_lossy().to_string()),
            before: None,
            after: None,
            verified: false,
            effect: None,
            ts: chrono::Utc::now().timestamp(),
        }
    }
    pub fn failed(handle: &str, action: &str, detail: Value) -> Self {
        ActionReceipt {
            handle: handle.to_string(),
            action: action.to_string(),
            ok: false,
            detail,
            snapshot_path: None,
            before: None,
            after: None,
            verified: false,
            effect: None,
            ts: chrono::Utc::now().timestamp(),
        }
    }

    /// 附上 observe→act→verify 的观测载荷。
    pub fn with_observation(
        mut self,
        before: Option<Value>,
        after: Option<Value>,
        effect: ObservationEffect,
    ) -> Self {
        self.before = before;
        self.after = after;
        self.verified = effect.verified();
        self.effect = Some(effect.as_str().to_string());
        self
    }
}

/// 按 `kind` + `params` 执行一次动作的**薄入口**（网关 `desktop.action`、模型工具、
/// 单测共用同一签名，避免各自拼 [`UiAction`] 时漏字段或拼错键）。
///
/// 这是 [`DesktopAppDriver::run_action`] 的便利包装：`UiAction` 由 `kind`/`params` 构造，
/// 其余语义（超时、审计、observe→act→verify）完全由驱动决定，本函数不加任何隐式行为。
pub fn execute_ui_action(
    driver: &dyn DesktopAppDriver,
    handle: &AppHandle,
    kind: &str,
    params: Value,
    session: &str,
) -> Result<ActionReceipt, DesktopError> {
    driver.run_action(handle, &UiAction::new(kind, params), session)
}

/// 审计快照元信息（目录 `.marguerite/audit-snapshots/<session>/`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub session: String,
    pub ts: i64,
    pub action_kind: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_snapshot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_snapshot: Option<String>,
}

/// 桌面应用驱动接口：启动 / 激活 / 执行动作 / 退出。
///
/// 设计为同步（COM / 进程操作均为阻塞 IO），便于在网关 RPC 内直接调用。
/// `run_action` 额外接收 `session` 用于审计快照落盘与产物登记归属。
pub trait DesktopAppDriver: Send + Sync {
    fn kind(&self) -> AppKind;
    fn launch(&self, args: &Value) -> Result<AppHandle, DesktopError>;
    fn activate(&self, h: &AppHandle) -> Result<(), DesktopError>;
    fn run_action(
        &self,
        h: &AppHandle,
        action: &UiAction,
        session: &str,
    ) -> Result<ActionReceipt, DesktopError>;
    fn quit(&self, h: &AppHandle) -> Result<(), DesktopError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_automation::a11y_tree::NodeState;
    use serde_json::json;

    fn state(value: Option<&str>, enabled: bool, focused: bool) -> NodeState {
        NodeState {
            ref_id: "edit-1".into(),
            role: "edit".into(),
            name: "输入框".into(),
            automation_id: "Edit1".into(),
            value: value.map(str::to_string),
            enabled,
            focused,
            toggle_state: None,
            expand_collapse_state: None,
            selected: None,
            scroll_percent: None,
            bounds: (0, 0, 100, 20),
        }
    }

    #[test]
    fn value_change_is_value_changed_and_verified() {
        let before = state(Some("a"), true, true);
        let after = state(Some("ab"), true, true);
        let effect = observe_effect(Some(&before), Some(&after));
        assert_eq!(effect, ObservationEffect::ValueChanged);
        assert_eq!(effect.as_str(), "value_changed");
        assert!(effect.verified());
    }

    #[test]
    fn disappearing_node_is_node_removed_and_verified() {
        let before = state(None, true, false);
        let effect = observe_effect(Some(&before), None);
        assert_eq!(effect, ObservationEffect::NodeRemoved);
        assert!(effect.verified());
    }

    #[test]
    fn identical_states_are_no_observable_change_and_not_verified() {
        let before = state(Some("a"), true, true);
        let after = state(Some("a"), true, true);
        let effect = observe_effect(Some(&before), Some(&after));
        assert_eq!(effect, ObservationEffect::NoObservableChange);
        assert!(!effect.verified(), "无变化不得声称已验证");
        // 两边都读不到也是「没观测到」，而不是「删掉了」。
        assert_eq!(
            observe_effect(None, None),
            ObservationEffect::NoObservableChange
        );
        // 出现新节点是状态变化，不是删除。
        assert_eq!(
            observe_effect(None, Some(&after)),
            ObservationEffect::StateChanged
        );
    }

    #[test]
    fn non_value_state_change_is_state_changed() {
        let before = state(None, true, false);
        let after = state(None, true, true); // 焦点变了
        assert_eq!(
            observe_effect(Some(&before), Some(&after)),
            ObservationEffect::StateChanged
        );
    }

    #[test]
    fn receipt_with_observation_keeps_ok_true_even_when_unverified() {
        let receipt = ActionReceipt::ok("h", "click", json!({}), None)
            .with_observation(None, None, ObservationEffect::NoObservableChange);
        assert!(receipt.ok, "无可观测变化不改变 ok");
        assert!(!receipt.verified);
        assert_eq!(receipt.effect.as_deref(), Some("no_observable_change"));
    }

    #[test]
    fn unknown_action_kind_is_rejected_by_the_shared_table() {
        for kind in KNOWN_ACTION_KINDS {
            assert!(is_known_action_kind(kind), "{kind} 必须在已知动作表里");
        }
        assert!(!is_known_action_kind("no_such_kind"));
        assert!(!is_known_action_kind(""));
    }

    #[test]
    fn action_param_helpers_treat_blank_strings_as_absent() {
        let a = UiAction {
            kind: "click".into(),
            params: json!({ "name": "  ", "timeout_ms": 1500 }),
        };
        assert_eq!(a.param_str_nonempty("name"), None);
        assert_eq!(a.param_i64("timeout_ms", 0), 1500);
        assert_eq!(a.param_i64("missing", 7), 7);
    }

    /// `UiAction` 的 serde 形状是网关契约：`params` 可省略，`kind` 必填。
    #[test]
    fn ui_action_serde_shape_is_stable() {
        let a: UiAction = serde_json::from_value(json!({ "kind": "snapshot" })).expect("params 可省略");
        assert_eq!(a.kind, "snapshot");
        assert!(a.params.is_null(), "缺省 params 必须是 null（驱动按未提供处理）");

        let built = UiAction::new("focus", json!({ "ref_id": "1.2" }));
        assert_eq!(built.kind, "focus");
        assert_eq!(built.params["ref_id"], "1.2");
        let back: UiAction = serde_json::from_value(serde_json::to_value(&built).unwrap()).unwrap();
        assert_eq!(back.kind, built.kind);
        assert_eq!(back.params, built.params);
    }

    /// 回执的序列化契约：缺席的观测字段整键消失，`verified` 永远出现。
    ///
    /// 这条守卫给网关/前端用：它们只能靠 `verified` 的**值**判断，不能靠「键在不在」。
    #[test]
    fn receipt_serde_shape_is_stable_and_omits_absent_observations() {
        let plain = ActionReceipt::ok("h", "find", json!({}), None);
        let v = serde_json::to_value(&plain).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["verified"], false, "verified 必须始终出现");
        for absent in ["before", "after", "effect", "snapshot_path"] {
            assert!(v.get(absent).is_none(), "{absent} 无值时必须整键消失");
        }

        let observed = plain.with_observation(
            Some(json!({ "value": "a" })),
            Some(json!({ "value": "b" })),
            ObservationEffect::ValueChanged,
        );
        let v = serde_json::to_value(&observed).unwrap();
        assert_eq!(v["verified"], true);
        assert_eq!(v["effect"], "value_changed");
        assert_eq!(v["before"]["value"], "a");
        assert_eq!(v["after"]["value"], "b");

        // 旧回执（没有 verified 键）必须读成 false —— 绝不能默认成「已验证」。
        let legacy: ActionReceipt = serde_json::from_value(json!({
            "handle": "h", "action": "click", "ok": true, "detail": {}, "ts": 1
        }))
        .expect("旧回执必须仍可反序列化");
        assert!(!legacy.verified);
        assert!(legacy.before.is_none());
        assert!(legacy.after.is_none());
        assert!(legacy.effect.is_none());
    }

    /// kind+params 的薄入口：不得改写 kind/params，也不得凭空造观测载荷。
    #[test]
    fn execute_ui_action_passes_kind_and_params_through_unchanged() {
        use std::sync::Mutex;

        #[derive(Default)]
        struct EchoDriver {
            seen: Mutex<Vec<(String, Value)>>,
        }

        impl DesktopAppDriver for EchoDriver {
            fn kind(&self) -> AppKind {
                AppKind::Generic
            }
            fn launch(&self, _args: &Value) -> Result<AppHandle, DesktopError> {
                Ok(AppHandle::new(AppKind::Generic))
            }
            fn activate(&self, _h: &AppHandle) -> Result<(), DesktopError> {
                Ok(())
            }
            fn run_action(
                &self,
                h: &AppHandle,
                action: &UiAction,
                _session: &str,
            ) -> Result<ActionReceipt, DesktopError> {
                self.seen
                    .lock()
                    .unwrap()
                    .push((action.kind.clone(), action.params.clone()));
                Ok(ActionReceipt::ok(&h.id, &action.kind, json!({ "echo": true }), None))
            }
            fn quit(&self, _h: &AppHandle) -> Result<(), DesktopError> {
                Ok(())
            }
        }

        let driver = EchoDriver::default();
        let h = AppHandle::new(AppKind::Generic);
        let receipt = execute_ui_action(
            &driver,
            &h,
            "set_value",
            json!({ "ref_id": "1.2", "value": "x" }),
            "sess-echo",
        )
        .expect("薄入口不应改变结果");
        assert!(receipt.ok);
        assert_eq!(receipt.action, "set_value");
        assert!(receipt.before.is_none() && receipt.after.is_none());
        assert!(receipt.snapshot_path.is_none());

        let seen = driver.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "set_value");
        assert_eq!(seen[0].1["ref_id"], "1.2");
        assert_eq!(seen[0].1["value"], "x");
    }
}
