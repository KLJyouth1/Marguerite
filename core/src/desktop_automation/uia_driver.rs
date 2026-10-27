//! 通用 UIA 驱动（[`AppKind::Generic`]）：用 Windows UI Automation 操控**已打开应用**的控件。
//!
//! 与 [`crate::desktop_automation::com`]（WPS 专用、late-bound IDispatch）互补：
//! 本驱动不绑定任何具体应用，凡是把控件暴露给 UIA 的程序都能被元素级读取与操控。
//! 真实 COM 后端在 [`crate::desktop_automation::windows_uia`]；本文件是**平台无关**的
//! 动作引擎 + 策略层，因此 mock `A11ySource` 的单测在任何平台都能跑。
//!
//! # 动作矩阵（`UiAction.kind`，W6.1）
//!
//! | kind | params | 说明 |
//! |---|---|---|
//! | `snapshot` | `window?`, `max_depth?`, `max_nodes?` | 读控件树（只读，不落审计） |
//! | `find` | `role?`, `name?`, `automation_id?`, `limit?` | 按条件找节点（**最多 20**，只读） |
//! | `click` | `ref_id` 或 `name` | Invoke→Toggle→Select→SetFocus 退化链 |
//! | `invoke` | `ref_id` / `name` | InvokePattern |
//! | `set_value` | `ref_id`/`name`, `value` | ValuePattern.SetValue |
//! | `type` | `ref_id`/`name`, `text`, `clear?`（默认 true） | ValuePattern（追加或替换） |
//! | `toggle` | `ref_id`/`name` | TogglePattern |
//! | `select` | `ref_id`/`name` | SelectionItemPattern |
//! | `expand` / `collapse` | `ref_id`/`name` | ExpandCollapsePattern |
//! | `scroll` | `ref_id`/`name`, `vertical?`（默认 true）, `horizontal?`, `amount?` | ScrollPattern |
//! | `focus` | `ref_id`/`name` | SetFocus |
//! | `wait` | `ref_id`/`name`, `state`（exists/enabled/focused/value_non_empty）, `timeout_ms?` | 轮询属性（只读） |
//! | `window_activate` | `title`/`window` 或 `hwnd` | SetForegroundWindow |
//!
//! 其余 kind 一律 [`DesktopError::UnsupportedAction`]，绝不假装支持。`type` 只有
//! ValuePattern（含 LegacyIAccessible 兜底）一条实现，**没有** SendKeys 兜底：键盘注入属于
//! `computer_use` 的像素轨道，不做进元素级驱动（否则「结构化优先」就没意义了）。
//!
//! # 节点定位
//!
//! `ref_id`（来自 `snapshot` / `find`）优先且最精确；退化到 `name` / `automation_id` /
//! `role` 时先查可点节点，避免同名静态文本抢走目标。找不到 → [`DesktopError::ElementNotFound`]
//! （网关码 `ELEMENT_NOT_FOUND`；本仓的「NodeNotFound」就叫这个名字，**不 panic**）。
//! 每次查找都受 [`MAX_FIND_RESULTS`]（20）约束，`find` 的 `limit` 只能往下压。
//!
//! # observe→act→verify（W6.3）
//!
//! 每个**写动作**在 act 前/后各读一次目标 [`NodeState`]（`window_activate` 读窗口摘要），
//! 回执带 `before` / `after` / `verified` / `effect`，并把这些真实载荷落进审计快照。
//! `verified=false` **不**改变 `ok`（动作可能成功但无可观测变化）。
//!
//! **只读动作**（`snapshot` / `find` / `wait`）的 `before` / `after` 恒为 `None`，也不落审计：
//! 它们每次调用都读一遍界面，落盘只会把审计目录刷满，而审计要回答的是「谁动了界面」。
//! `wait` 的 `verified` 语义是「等到了条件」（回执 `detail.hit = true`），超时则返回
//! [`DesktopError::Timeout`]（网关码 `ACTION_TIMEOUT`）——命中与超时都能被调用方区分。
//!
//! # 安全与审计
//!
//! 本驱动不自建闸门：`desktop.action` / `desktop_action` 已统一做 Policy 二次确认与注入扫描，
//! 所有真实动作再经 [`Recorder`] 落 `.marguerite/audit-snapshots/<session>/`。
//!
//! # 动作超时与节点上限
//!
//! 单次 UIA 调用由后端强制 10s 硬超时（`windows_uia::UIA_CALL_TIMEOUT`），节点数受
//! [`MAX_NODES`] 与 [`MAX_FIND_RESULTS`] 约束；`wait` 的总预算由 `timeout_ms` 决定
//! （默认 5s，上限 60s），不会无限轮询。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::desktop_automation::a11y_tree::{
    A11yError, A11ySource, NodeQuery, NodeState, PatternOp, SnapshotOptions, UiNode, UiSnapshot,
    MAX_FIND_RESULTS, MAX_NODES,
};
use crate::desktop_automation::driver::*;
use crate::desktop_automation::recorder::Recorder;
use crate::desktop_automation::windows;
#[cfg(windows)]
use crate::desktop_automation::windows_uia::WindowsUiaSource;

/// `wait` 的默认预算与硬上限（这是轮询总时长，不是单次 UIA 调用的 10s 超时）。
pub const DEFAULT_WAIT_MS: u64 = 5_000;
pub const MAX_WAIT_MS: u64 = 60_000;
/// 轮询间隔。
const WAIT_POLL: Duration = Duration::from_millis(100);

/// [`A11yError`] → [`DesktopError`]。映射保持语义：通道缺失≠找不到节点≠不支持≠执行失败。
fn a11y_err(e: A11yError) -> DesktopError {
    match e {
        A11yError::Unavailable(m) => DesktopError::ComError(format!("无障碍通道不可用: {m}")),
        A11yError::NodeNotFound(id) => DesktopError::ElementNotFound(id),
        A11yError::OsFailure(m) => DesktopError::ComError(m),
        A11yError::Unsupported(m) => DesktopError::UnsupportedAction(m),
    }
}

/// 控件 → JSON（显式列字段，避免给 a11y_tree 的稳定契约加 serde 依赖）。
///
/// `pub(crate)`：`desktop.a11y` RPC 与本驱动的 `snapshot`/`find` 共用同一份字段口径。
pub(crate) fn node_json(n: &UiNode) -> Value {
    json!({
        "role": n.role,
        "name": n.name,
        "ref_id": n.ref_id,
        "automation_id": n.automation_id,
        "bounds": Bounds::from(n.bounds),
        "enabled": n.enabled,
        "focused": n.focused,
        "patterns": n.patterns.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
        // 旧字段保留：已有调用方按 x/y/w/h 读。
        "x": n.bounds.0,
        "y": n.bounds.1,
        "w": n.bounds.2,
        "h": n.bounds.3,
        "clickable": n.clickable,
    })
}

/// 节点状态 → 回执/审计里的摘要 JSON（W6.3 契约：node_digest/value/enabled/focused）。
fn state_json(s: &NodeState) -> Value {
    json!({
        "ref_id": s.ref_id,
        "role": s.role,
        "name": s.name,
        "automation_id": s.automation_id,
        "node_digest": s.digest(),
        "value": s.value,
        "enabled": s.enabled,
        "focused": s.focused,
        "toggle_state": s.toggle_state,
        "expand_collapse_state": s.expand_collapse_state,
        "selected": s.selected,
        "scroll_percent": s.scroll_percent.map(|(h, v)| json!({ "horizontal": h, "vertical": v })),
        "bounds": Bounds::from(s.bounds),
    })
}

/// 通用 UIA 驱动。
pub struct UiaDriver {
    source: Arc<dyn A11ySource>,
    /// handle_id → 目标窗口 HWND。
    ///
    /// 存 `i64` 而不是 `HWND`：`HWND` 是裸指针（不 `Send`），而 `DesktopAppDriver` 要求
    /// `Send + Sync`。指针值本身只是个整数句柄，搬运是安全的。
    windows: Mutex<HashMap<String, i64>>,
}

impl UiaDriver {
    /// 用注入的 [`A11ySource`] 构造（单测 / 自定义后端）。
    pub fn with_source(source: Arc<dyn A11ySource>) -> Self {
        UiaDriver {
            source,
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// 生产构造：真实 Windows UIA 后端。非 Windows 平台不提供（注册表也不会注册 Generic）。
    #[cfg(windows)]
    pub fn new() -> Self {
        Self::with_source(Arc::new(WindowsUiaSource::new()))
    }

    fn bind(&self, handle_id: &str, hwnd: i64) {
        self.windows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(handle_id.to_string(), hwnd);
    }

    fn bound(&self, handle_id: &str) -> Option<i64> {
        self.windows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(handle_id)
            .copied()
    }

    /// 动作里的窗口查询串：`window` 优先，其次 `title`。
    fn window_query(action: &UiAction) -> Option<String> {
        action
            .param_str_nonempty("window")
            .or_else(|| action.param_str_nonempty("title"))
    }

    /// 动作里的定位条件（不含 `ref_id`）。
    fn target_query(action: &UiAction) -> NodeQuery {
        NodeQuery {
            role: action.param_str_nonempty("role"),
            name: action.param_str_nonempty("name"),
            automation_id: action.param_str_nonempty("automation_id"),
        }
    }

    /// 按条件查一次（**先查可点节点**，避免同名静态文本抢走目标）。
    ///
    /// 返回 `Ok(None)` 表示「这一轮没找到」——`wait` 要能等一个**尚未出现**的节点，
    /// 所以「没找到」在查找层不是错误，是否算失败由调用方决定。
    fn find_target(&self, query: &NodeQuery) -> Result<Option<String>, DesktopError> {
        let mut hits = self
            .source
            .find(query, MAX_FIND_RESULTS)
            .map_err(a11y_err)?;
        // 稳定排序：同一条件内保持原始遍历顺序，只把可点节点提前。
        hits.sort_by_key(|n| !n.clickable);
        Ok(hits.first().map(|n| n.ref_id.clone()))
    }

    /// 把动作解析成一个具体 `ref_id`；找不到时 `Ok(None)`（只有 `wait` 会用这个语义）。
    ///
    /// 优先用调用方给的 `ref_id`（来自 `snapshot` / `find`，最精确）——注意这里**不**校验其存在性：
    /// 写动作会立刻 `read_state`，读不到就报 `ELEMENT_NOT_FOUND`；`wait` 则继续轮询。
    fn resolve_optional(&self, action: &UiAction) -> Result<Option<String>, DesktopError> {
        if let Some(r) = action.param_str_nonempty("ref_id") {
            return Ok(Some(r));
        }
        let query = Self::target_query(action);
        if query.is_empty() {
            return Err(DesktopError::UnsupportedAction(format!(
                "{} 需要 ref_id（来自 snapshot/find）或 name / automation_id / role",
                action.kind
            )));
        }
        self.find_target(&query)
    }

    /// 写动作的定位：找不到节点是**错误**（`ELEMENT_NOT_FOUND`），不 panic，也不盲目点击。
    fn resolve_ref(&self, action: &UiAction) -> Result<String, DesktopError> {
        let query = Self::target_query(action);
        self.resolve_optional(action)?.ok_or_else(|| {
            DesktopError::ElementNotFound(format!(
                "找不到匹配节点（role={:?}, name={:?}, automation_id={:?}）",
                query.role, query.name, query.automation_id
            ))
        })
    }

    /// 目标的人类可读描述（错误文案里用；不含任何界面内容）。
    fn describe_target(action: &UiAction) -> String {
        if let Some(r) = action.param_str_nonempty("ref_id") {
            return format!("ref_id={r}");
        }
        let query = Self::target_query(action);
        let mut parts = Vec::new();
        if let Some(v) = &query.role {
            parts.push(format!("role={v}"));
        }
        if let Some(v) = &query.name {
            parts.push(format!("name={v}"));
        }
        if let Some(v) = &query.automation_id {
            parts.push(format!("automation_id={v}"));
        }
        if parts.is_empty() {
            "<未指定目标>".to_string()
        } else {
            parts.join(", ")
        }
    }

    /// 读状态：节点不存在返回 `Ok(None)`（用于判定 `node_removed`），其它错误照常上抛。
    fn read_state(&self, ref_id: &str) -> Result<Option<NodeState>, DesktopError> {
        match self.source.node_state(ref_id) {
            Ok(s) => Ok(Some(s)),
            Err(A11yError::NodeNotFound(_)) => Ok(None),
            Err(e) => Err(a11y_err(e)),
        }
    }

    /// 附着并前置一个窗口：`query` 有值时按标题/进程名找，否则取前台窗口。
    fn attach_window(&self, query: Option<&str>) -> Result<windows::WindowInfo, DesktopError> {
        let found = match query.map(str::trim).filter(|q| !q.is_empty()) {
            Some(q) => windows::find_window(q)?,
            None => windows::foreground_window()?,
        };
        // 后端按「前台窗口」读控件树，所以附着 = 前置 + 记住 hwnd。
        windows::activate_window(found.hwnd)
    }

    /// 写动作：观察 → 执行 → 再观察 → 落审计。
    fn observed_action(
        &self,
        handle: &AppHandle,
        action: &UiAction,
        session: &str,
        context: &str,
    ) -> Result<ActionReceipt, DesktopError> {
        let ref_id = self.resolve_ref(action)?;
        let before = self.read_state(&ref_id)?.ok_or_else(|| {
            DesktopError::ElementNotFound(format!("动作前读不到目标节点: {ref_id}"))
        })?;
        let op = op_for_action(action)?;
        match op {
            Some(op) => self.source.perform(&ref_id, &op).map_err(a11y_err)?,
            // `click` 走「最语义化 → 最退化」的激活链（Invoke/Toggle/Select/SetFocus）。
            None => self.source.click(&ref_id).map_err(a11y_err)?,
        }
        let after = self.read_state(&ref_id)?;
        let effect = observe_effect(Some(&before), after.as_ref());
        let before_json = state_json(&before);
        let after_json = after.as_ref().map(state_json);
        let snapshot = Recorder::record_step(
            session,
            action,
            Some(&before_json),
            after_json.as_ref(),
            context,
        );
        let detail = json!({
            "ref_id": ref_id,
            "role": before.role,
            "name": before.name,
            "effect": effect.as_str(),
            "verified": effect.verified(),
            "value": after.as_ref().and_then(|s| s.value.clone()),
        });
        Ok(
            ActionReceipt::ok(&handle.id, &action.kind, detail, Some(snapshot))
                .with_observation(Some(before_json), after_json, effect),
        )
    }

    /// 只读快照。
    ///
    /// ⚠️ 唯一的副作用：`window` 给出时**会前置该窗口**（UIA 后端只读前台窗口，见
    /// `windows_uia::foreground_root`）。这条前置不落审计；不想动前台就不要传 `window`。
    fn snapshot_action(
        &self,
        handle: &AppHandle,
        action: &UiAction,
    ) -> Result<ActionReceipt, DesktopError> {
        let query = Self::window_query(action);
        let window = match query.as_deref() {
            Some(q) => Some(self.attach_window(Some(q))?),
            None => windows::foreground_window().ok(),
        };
        let max_depth = action
            .param_i64("max_depth", action.param_i64("maxDepth", 0))
            .max(0) as usize;
        let max_nodes = action
            .param_i64("max_nodes", action.param_i64("maxNodes", 0))
            .max(0) as usize;
        // 双重夹紧：`SnapshotOptions::bounded` 会再夹一次，这里先夹是为了让 `MAX_NODES`
        // 这个上限在**驱动侧**也可见（不把边界只寄存在后端里）。
        let max_nodes = if max_nodes > 0 {
            max_nodes.min(MAX_NODES)
        } else {
            0
        };
        let opts = SnapshotOptions {
            max_depth: (max_depth > 0).then_some(max_depth),
            max_nodes: (max_nodes > 0).then_some(max_nodes),
        };
        let snap = self.source.snapshot_with(&opts).map_err(a11y_err)?;
        let nodes: Vec<Value> = snap.nodes.iter().map(node_json).collect();
        let mut detail = json!({ "count": nodes.len(), "nodes": nodes });
        if let Some(w) = &window {
            detail["window"] = serde_json::to_value(w).unwrap_or(Value::Null);
        } else if let Some(hwnd) = self.bound(&handle.id) {
            detail["window"] = json!({ "hwnd": hwnd });
        }
        Ok(ActionReceipt::ok(&handle.id, "snapshot", detail, None))
    }

    /// 只读查找（单次快照，最多 [`MAX_FIND_RESULTS`] 条）。
    ///
    /// 为什么这里不接受 `timeout_ms`：`find` 是**单次**查询，需要「等它出现」请用 `wait`
    /// （`wait` 才有轮询预算）。给 `find` 加等待只会让两个动作的语义重叠。
    fn find_action(
        &self,
        handle: &AppHandle,
        action: &UiAction,
    ) -> Result<ActionReceipt, DesktopError> {
        let query = Self::target_query(action);
        if query.is_empty() {
            return Err(DesktopError::UnsupportedAction(
                "find 需要 role / name / automation_id 中至少一个条件".into(),
            ));
        }
        let limit = find_limit(action);
        let nodes = self.source.find(&query, limit).map_err(a11y_err)?;
        let nodes: Vec<Value> = nodes.iter().map(node_json).collect();
        Ok(ActionReceipt::ok(
            &handle.id,
            "find",
            json!({ "count": nodes.len(), "limit": limit, "nodes": nodes }),
            None,
        ))
    }

    /// 等待条件成立（只读）。
    ///
    /// - 命中：`ok=true, verified=true, detail.hit=true`，不落审计、`before/after` 为 `None`；
    /// - 超时：返回 [`DesktopError::Timeout`]（网关码 `ACTION_TIMEOUT`），错误文案带目标与预算。
    ///
    /// 目标节点**允许一开始不存在**：每轮都重新解析（`ref_id` 直接读，条件查询每轮查一次），
    /// 这样 `wait {name:"保存", state:"exists"}` 才能等到一个刚弹出的对话框。
    fn wait_action(
        &self,
        handle: &AppHandle,
        action: &UiAction,
        _session: &str,
    ) -> Result<ActionReceipt, DesktopError> {
        let wanted = action
            .param_str_nonempty("state")
            .unwrap_or_else(|| "exists".to_string());
        // 先校验 state：未知条件不该先跑一轮轮询再报错。
        if !matches!(
            wanted.as_str(),
            "exists" | "enabled" | "focused" | "value_non_empty"
        ) {
            return Err(DesktopError::UnsupportedAction(format!(
                "wait 不支持 state={wanted}（支持 exists/enabled/focused/value_non_empty）"
            )));
        }
        let budget = action
            .param_i64("timeout_ms", DEFAULT_WAIT_MS as i64)
            .clamp(0, MAX_WAIT_MS as i64) as u64;
        let started = Instant::now();
        let deadline = started + Duration::from_millis(budget);
        loop {
            let resolved = self.resolve_optional(action)?;
            let current = match &resolved {
                Some(id) => self.read_state(id)?,
                None => None,
            };
            let satisfied = match wanted.as_str() {
                "exists" => current.is_some(),
                "enabled" => current.as_ref().map(|s| s.enabled).unwrap_or(false),
                "focused" => current.as_ref().map(|s| s.focused).unwrap_or(false),
                // 上面已校验过，这里只剩 value_non_empty。
                _ => current
                    .as_ref()
                    .and_then(|s| s.value.as_deref())
                    .map(|v| !v.is_empty())
                    .unwrap_or(false),
            };
            if satisfied {
                let mut receipt = ActionReceipt::ok(
                    &handle.id,
                    "wait",
                    json!({
                        "ref_id": resolved,
                        "state": wanted,
                        "hit": true,
                        "waited_ms": started.elapsed().as_millis() as u64,
                    }),
                    None,
                );
                // `wait` 是只读动作：没有「动作前后的目标」可比（before/after 恒为 None），
                // verified 的语义是「等到了条件」，与写动作的「观测到变化」不同。
                receipt.verified = true;
                return Ok(receipt);
            }
            if Instant::now() >= deadline {
                return Err(DesktopError::Timeout(format!(
                    "等待 {} 进入 {} 超时（{}ms）",
                    Self::describe_target(action),
                    wanted,
                    budget
                )));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            std::thread::sleep(WAIT_POLL.min(remaining));
        }
    }

    /// 激活窗口并把句柄重绑到该窗口（后续动作都作用在同一窗口上）。
    ///
    /// 这是**写动作**：它改变前台窗口。因此同样做 observe→act→verify ——
    /// 动作前抓当前前台窗口摘要，动作后抓目标窗口摘要，`hwnd`/`title` 变化即 `state_changed`。
    fn window_activate_action(
        &self,
        handle: &AppHandle,
        action: &UiAction,
        session: &str,
    ) -> Result<ActionReceipt, DesktopError> {
        let hwnd = if let Some(h) = action
            .param_i64("hwnd", 0)
            .checked_abs()
            .filter(|v| *v > 0)
        {
            h
        } else if let Some(q) = Self::window_query(action) {
            windows::find_window(&q)?.hwnd
        } else {
            self.bound(&handle.id).ok_or_else(|| {
                DesktopError::WindowNotFound(format!(
                    "句柄 {} 未绑定窗口（或已 quit），且动作未给出 title/window/hwnd",
                    handle.id
                ))
            })?
        };
        // 无前台窗口时 `before = None`（诚实：没观测到「之前」），不阻断动作。
        let before = windows::foreground_window()
            .ok()
            .map(|w| window_summary(&w));
        let info = windows::activate_window(hwnd)?;
        self.bind(&handle.id, info.hwnd);
        let hwnd_out = info.hwnd;
        let after = window_summary(&info);
        let effect = window_effect(before.as_ref(), &after);
        let snapshot = Recorder::record_step(
            session,
            action,
            before.as_ref(),
            Some(&after),
            "uia_window_activate",
        );
        Ok(ActionReceipt::ok(
            &handle.id,
            "window_activate",
            json!({ "hwnd": hwnd_out, "window": info }),
            Some(snapshot),
        )
        .with_observation(before, Some(after), effect))
    }
}

/// `find` 的返回上限：**最多 20**（[`MAX_FIND_RESULTS`]）。参数只能往下压、不能往上抬。
fn find_limit(action: &UiAction) -> usize {
    action
        .param_i64("limit", action.param_i64("max", MAX_FIND_RESULTS as i64))
        .clamp(1, MAX_FIND_RESULTS as i64) as usize
}

/// 窗口摘要（observe→act→verify 的观测载荷；只放比较需要的字段）。
fn window_summary(w: &windows::WindowInfo) -> Value {
    json!({
        "hwnd": w.hwnd,
        "title": w.title,
        "process_name": w.process_name,
        "pid": w.pid,
        "foreground": w.foreground,
        "iconic": w.iconic,
    })
}

/// 纯函数：窗口前置的效果判定（无窗口可 mock，因此单独抽出来做单测）。
///
/// 比较**整份摘要**（hwnd/title/process/pid/foreground/iconic）：只比 `hwnd` 会漏掉
/// 「从最小化还原」这种真实变化，只比 `title` 会把换窗口漏成「没有变化」。
fn window_effect(before: Option<&Value>, after: &Value) -> ObservationEffect {
    match before {
        Some(b) if b == after => ObservationEffect::NoObservableChange,
        // 换了窗口 / 标题变了 / 还原了最小化 / 之前根本没有前台窗口：都是可观测的状态变化。
        _ => ObservationEffect::StateChanged,
    }
}

/// 文本载荷参数：接受字符串，也接受数字/布尔（模型常把值写成 `42` / `true`）。
///
/// 只用于 `set_value` / `type` 的**载荷**。定位参数仍走 `param_str_nonempty`
/// ——空串在那里是「未提供」，在载荷里是「清空」，两者语义相反，不能混用。
fn param_text(action: &UiAction, key: &str) -> Option<String> {
    match action.params.get(key) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::Bool(b)) => Some(b.to_string()),
        _ => None,
    }
}

/// 动作 → pattern 操作。`Ok(None)` 表示该 kind 使用 `click` 的退化链。
fn op_for_action(action: &UiAction) -> Result<Option<PatternOp>, DesktopError> {
    let op = match action.kind.as_str() {
        "click" => return Ok(None),
        "invoke" => PatternOp::Invoke,
        "toggle" => PatternOp::Toggle,
        "select" => PatternOp::Select,
        "expand" => PatternOp::Expand,
        "collapse" => PatternOp::Collapse,
        "focus" => PatternOp::Focus,
        "set_value" => PatternOp::SetValue(
            param_text(action, "value")
                .ok_or_else(|| DesktopError::UnsupportedAction("set_value 需要 value".into()))?,
        ),
        "type" => {
            let text = param_text(action, "text")
                .ok_or_else(|| DesktopError::UnsupportedAction("type 需要 text".into()))?;
            let clear = action
                .params
                .get("clear")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            PatternOp::TypeText { text, clear }
        }
        "scroll" => {
            let amount = action.param_i64("amount", 1).clamp(-50, 50) as i32;
            let vertical = action
                .params
                .get("vertical")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let horizontal = action
                .params
                .get("horizontal")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            PatternOp::Scroll {
                horizontal: if horizontal { amount } else { 0 },
                vertical: if vertical { amount } else { 0 },
            }
        }
        other => {
            return Err(DesktopError::UnsupportedAction(other.to_string()));
        }
    };
    Ok(Some(op))
}

/// 生产默认构造（真实 UIA 后端）。仅 Windows：非 Windows 没有真实源，不提供「默认假源」。
#[cfg(windows)]
impl Default for UiaDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopAppDriver for UiaDriver {
    fn kind(&self) -> AppKind {
        AppKind::Generic
    }

    /// 「启动」对通用驱动没有意义——它是**附着**语义：绑定目标窗口（默认前台）并验证其可读。
    ///
    /// `args.window` 给出标题/进程名时附着该窗口（W6.2）；否则附着当前前台窗口。
    /// 为什么不顺手加个「按 exe 路径启动进程」：那是进程管理，属于 `tool.exec` /
    /// `system_control` 的职责；塞进这里只会多一条无闸门的任意程序执行面。
    fn launch(&self, args: &Value) -> Result<AppHandle, DesktopError> {
        let query = args
            .get("window")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let info = self.attach_window(query)?;
        // 先确认这个窗口真的能被 UIA 读：读不到就不发句柄，免得上层拿到「看起来可用」的假句柄。
        self.source.snapshot().map_err(a11y_err)?;
        let handle = AppHandle::new(AppKind::Generic);
        self.bind(&handle.id, info.hwnd);
        Ok(handle)
    }

    fn activate(&self, h: &AppHandle) -> Result<(), DesktopError> {
        let hwnd = self.bound(&h.id).ok_or_else(|| {
            DesktopError::NotRunning(format!("句柄 {} 未绑定窗口（或已 quit）", h.id))
        })?;
        windows::activate_window(hwnd)?;
        Ok(())
    }

    fn run_action(
        &self,
        h: &AppHandle,
        action: &UiAction,
        session: &str,
    ) -> Result<ActionReceipt, DesktopError> {
        match action.kind.as_str() {
            // 只读：不落审计快照（否则轮询 snapshot 会把审计目录刷满），但回执如实带节点。
            "snapshot" => self.snapshot_action(h, action),
            "find" => self.find_action(h, action),
            "click" | "invoke" | "toggle" | "select" | "expand" | "collapse" | "focus"
            | "set_value" | "type" | "scroll" => self.observed_action(
                h,
                action,
                session,
                &format!("uia_{}", action.kind),
            ),
            "wait" => self.wait_action(h, action, session),
            "window_activate" => self.window_activate_action(h, action, session),
            other => Err(DesktopError::UnsupportedAction(other.to_string())),
        }
    }

    fn quit(&self, h: &AppHandle) -> Result<(), DesktopError> {
        self.windows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&h.id);
        // 「退出」只解除绑定，不去关别人的窗口——关窗是破坏性动作，不在通用驱动职责内。
        Ok(())
    }
}

/// `desktop.a11y` RPC 的共享实现：可选先附着窗口，再读控件树。
///
/// 返回 `(窗口信息, 快照)`；窗口信息在无前台窗口时为 `None`。
#[cfg(windows)]
pub fn a11y_snapshot_for_rpc(
    window: Option<&str>,
    max_depth: Option<usize>,
) -> Result<(Option<windows::WindowInfo>, UiSnapshot), DesktopError> {
    let info = match window.map(str::trim).filter(|s| !s.is_empty()) {
        Some(q) => Some(windows::activate_window(windows::find_window(q)?.hwnd)?),
        None => windows::foreground_window().ok(),
    };
    let snap = WindowsUiaSource::new()
        .snapshot_with(&SnapshotOptions {
            max_depth,
            max_nodes: None,
        })
        .map_err(a11y_err)?;
    Ok((info, snap))
}

/// 非 Windows：UIA 不存在，必须诚实报「不支持」，绝不返回空树冒充「没有控件」。
#[cfg(not(windows))]
pub fn a11y_snapshot_for_rpc(
    _window: Option<&str>,
    _max_depth: Option<usize>,
) -> Result<(Option<windows::WindowInfo>, UiSnapshot), DesktopError> {
    Err(DesktopError::UnsupportedAction(
        "桌面 UIA 控件树仅支持 Windows".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_automation::a11y_tree::UiPattern;

    /// 单测行为：动作如何改变节点状态（模拟真实控件的可见效果）。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Behavior {
        SetValue,
        ToggleState,
        RemoveOnInvoke,
        Noop,
    }

    struct MockSource {
        nodes: Mutex<Vec<UiNode>>,
        states: Mutex<HashMap<String, NodeState>>,
        ops: Mutex<Vec<(String, PatternOp)>>,
        clicks: Mutex<Vec<String>>,
        /// 每次 `find` 收到的 `limit`（用来钉住「驱动最多只要 20 条」）。
        find_limits: Mutex<Vec<usize>>,
        behavior: Behavior,
    }

    impl MockSource {
        fn new(node: UiNode, state: NodeState, behavior: Behavior) -> Self {
            let mut states = HashMap::new();
            states.insert(state.ref_id.clone(), state);
            MockSource {
                nodes: Mutex::new(vec![node]),
                states: Mutex::new(states),
                ops: Mutex::new(Vec::new()),
                clicks: Mutex::new(Vec::new()),
                find_limits: Mutex::new(Vec::new()),
                behavior,
            }
        }

        fn ops(&self) -> Vec<(String, PatternOp)> {
            self.ops.lock().unwrap().clone()
        }

        fn click_count(&self) -> usize {
            self.clicks.lock().unwrap().len()
        }

        fn find_limits(&self) -> Vec<usize> {
            self.find_limits.lock().unwrap().clone()
        }
    }

    impl A11ySource for MockSource {
        fn snapshot(&self) -> Result<UiSnapshot, A11yError> {
            Ok(UiSnapshot {
                nodes: self.nodes.lock().unwrap().clone(),
            })
        }

        fn find(&self, query: &NodeQuery, limit: usize) -> Result<Vec<UiNode>, A11yError> {
            self.find_limits.lock().unwrap().push(limit);
            if query.is_empty() {
                return Err(A11yError::OsFailure(
                    "find 需要 role / name / automation_id 中至少一个条件".into(),
                ));
            }
            let cap = limit.clamp(1, MAX_NODES);
            Ok(self
                .nodes
                .lock()
                .unwrap()
                .iter()
                .filter(|n| query.matches(n))
                .take(cap)
                .cloned()
                .collect())
        }

        fn click(&self, ref_id: &str) -> Result<(), A11yError> {
            self.clicks.lock().unwrap().push(ref_id.to_string());
            if self.states.lock().unwrap().contains_key(ref_id) {
                Ok(())
            } else {
                Err(A11yError::NodeNotFound(ref_id.to_string()))
            }
        }

        fn node_state(&self, ref_id: &str) -> Result<NodeState, A11yError> {
            self.states
                .lock()
                .unwrap()
                .get(ref_id)
                .cloned()
                .ok_or_else(|| A11yError::NodeNotFound(ref_id.to_string()))
        }

        fn perform(&self, ref_id: &str, op: &PatternOp) -> Result<(), A11yError> {
            {
                let mut states = self.states.lock().unwrap();
                let state = states
                    .get_mut(ref_id)
                    .ok_or_else(|| A11yError::NodeNotFound(ref_id.to_string()))?;
                self.ops
                    .lock()
                    .unwrap()
                    .push((ref_id.to_string(), op.clone()));
                match (self.behavior, op) {
                    (Behavior::SetValue, PatternOp::SetValue(v)) => {
                        state.value = Some(v.clone());
                    }
                    (Behavior::SetValue, PatternOp::TypeText { text, clear }) => {
                        let mut next = if *clear {
                            String::new()
                        } else {
                            state.value.clone().unwrap_or_default()
                        };
                        next.push_str(text);
                        state.value = Some(next);
                    }
                    (Behavior::ToggleState, PatternOp::Toggle) => {
                        let next = match state.toggle_state.as_deref() {
                            Some("on") => "off",
                            _ => "on",
                        };
                        state.toggle_state = Some(next.to_string());
                    }
                    (Behavior::RemoveOnInvoke, PatternOp::Invoke) => {}
                    _ => {}
                }
            }
            if self.behavior == Behavior::RemoveOnInvoke && matches!(op, PatternOp::Invoke) {
                self.nodes.lock().unwrap().retain(|n| n.ref_id != ref_id);
                self.states.lock().unwrap().remove(ref_id);
            }
            Ok(())
        }
    }

    fn node(role: &str, name: &str, ref_id: &str, patterns: Vec<UiPattern>) -> UiNode {
        UiNode::new(role, name, ref_id, (10, 20, 100, 30), true).with_patterns(patterns)
    }

    fn state(ref_id: &str, value: Option<&str>, enabled: bool, focused: bool) -> NodeState {
        NodeState {
            ref_id: ref_id.to_string(),
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
            bounds: (10, 20, 100, 30),
        }
    }

    struct Fixture {
        driver: UiaDriver,
        source: Arc<MockSource>,
        audit_root: std::path::PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.audit_root);
            Recorder::set_root_override(None);
        }
    }

    fn fixture(node: UiNode, state: NodeState, behavior: Behavior) -> Fixture {
        let audit_root = std::env::temp_dir().join(format!(
            "mgt-uia-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        Recorder::set_root_override(Some(audit_root.clone()));
        let source = Arc::new(MockSource::new(node, state, behavior));
        Fixture {
            driver: UiaDriver::with_source(source.clone()),
            source,
            audit_root,
        }
    }

    fn action(kind: &str, params: Value) -> UiAction {
        UiAction {
            kind: kind.to_string(),
            params,
        }
    }

    fn read_audit(receipt: &ActionReceipt) -> Value {
        let path = receipt
            .snapshot_path
            .as_ref()
            .expect("写动作必须落审计快照");
        let text = std::fs::read_to_string(path).expect("审计文件必须存在");
        serde_json::from_str(&text).expect("审计文件必须是合法 JSON")
    }

    #[test]
    fn kind_is_generic() {
        let f = fixture(
            node("edit", "用户名", "edit-1", vec![UiPattern::Value]),
            state("edit-1", Some("old"), true, true),
            Behavior::Noop,
        );
        assert_eq!(f.driver.kind(), AppKind::Generic);
    }

    /// 未实现的动作必须明确报 UnsupportedAction，而不是静默成功。
    #[test]
    fn unknown_action_is_unsupported() {
        let f = fixture(
            node("edit", "用户名", "edit-1", vec![]),
            state("edit-1", None, true, true),
            Behavior::Noop,
        );
        let h = AppHandle::new(AppKind::Generic);
        let err = f
            .driver
            .run_action(&h, &action("generate_presentation", json!({})), "s")
            .expect_err("未实现动作必须失败");
        assert!(
            matches!(err, DesktopError::UnsupportedAction(ref k) if k == "generate_presentation"),
            "实际: {err:?}"
        );
    }

    /// `click` 缺少定位参数时必须立刻失败（不得先做一次无意义的全树遍历）。
    #[test]
    fn click_without_target_is_rejected_before_snapshot() {
        let f = fixture(
            node("edit", "用户名", "edit-1", vec![]),
            state("edit-1", None, true, true),
            Behavior::Noop,
        );
        let h = AppHandle::new(AppKind::Generic);
        let err = f
            .driver
            .run_action(&h, &action("click", json!({})), "s")
            .expect_err("缺少定位参数必须失败");
        assert!(matches!(err, DesktopError::UnsupportedAction(_)), "实际: {err:?}");
        assert_eq!(f.source.click_count(), 0, "不得触碰后端");

        let err2 = f
            .driver
            .run_action(&h, &action("click", json!({ "ref_id": "", "name": "" })), "s")
            .expect_err("空字符串必须失败");
        assert!(matches!(err2, DesktopError::UnsupportedAction(_)), "实际: {err2:?}");
    }

    /// 找不到节点 → ElementNotFound（不 panic，也不假装成功）。
    #[test]
    fn missing_node_is_element_not_found() {
        let f = fixture(
            node("button", "确定", "btn-1", vec![UiPattern::Invoke]),
            state("btn-1", None, true, true),
            Behavior::Noop,
        );
        let h = AppHandle::new(AppKind::Generic);
        let err = f
            .driver
            .run_action(&h, &action("invoke", json!({ "name": "不存在的按钮" })), "s")
            .expect_err("找不到节点必须失败");
        assert!(matches!(err, DesktopError::ElementNotFound(_)), "实际: {err:?}");
        assert!(f.source.ops().is_empty(), "找不到节点时不得执行任何 pattern");
    }

    /// 未绑定窗口的句柄做 activate 必须诚实失败。
    #[test]
    fn activate_unbound_handle_is_not_running() {
        let f = fixture(
            node("edit", "用户名", "edit-1", vec![]),
            state("edit-1", None, true, true),
            Behavior::Noop,
        );
        let h = AppHandle::new(AppKind::Generic);
        let err = f.driver.activate(&h).expect_err("未绑定句柄必须失败");
        assert!(matches!(err, DesktopError::NotRunning(_)), "实际: {err:?}");
    }

    /// quit 是幂等的：解绑 + 再 quit 都不应报错。
    #[test]
    fn quit_is_idempotent() {
        let f = fixture(
            node("edit", "用户名", "edit-1", vec![]),
            state("edit-1", None, true, true),
            Behavior::Noop,
        );
        let h = AppHandle::new(AppKind::Generic);
        assert!(f.driver.quit(&h).is_ok());
        assert!(f.driver.quit(&h).is_ok());
        assert!(f.driver.bound(&h.id).is_none());
    }

    // ── W6.3：observe→act→verify 三态 ──

    #[test]
    fn set_value_reports_value_changed_and_audits_before_after() {
        let f = fixture(
            node("edit", "用户名", "edit-1", vec![UiPattern::Value]),
            state("edit-1", Some("old"), true, true),
            Behavior::SetValue,
        );
        let h = AppHandle::new(AppKind::Generic);
        let receipt = f
            .driver
            .run_action(
                &h,
                &action("set_value", json!({ "ref_id": "edit-1", "value": "new" })),
                "sess-value",
            )
            .expect("set_value 应成功");

        assert!(receipt.ok);
        assert_eq!(receipt.effect.as_deref(), Some("value_changed"));
        assert!(receipt.verified, "值真的变了，必须 verified=true");
        assert_eq!(receipt.before.as_ref().unwrap()["value"], "old");
        assert_eq!(receipt.after.as_ref().unwrap()["value"], "new");
        assert!(
            receipt.before.as_ref().unwrap()["node_digest"].is_string(),
            "before 必须带 node_digest"
        );

        let audit = read_audit(&receipt);
        assert_eq!(audit["action"], "set_value");
        assert_eq!(audit["before"]["value"], "old", "审计必须含真实 before");
        assert_eq!(audit["after"]["value"], "new", "审计必须含真实 after");
        assert_eq!(audit["before"]["enabled"], true);
        assert_eq!(audit["before"]["focused"], true);
        assert!(audit["before_snapshot"].is_null(), "屏幕快照仍无产出");
    }

    #[test]
    fn invoke_that_removes_the_node_reports_node_removed() {
        let f = fixture(
            node("button", "保存", "btn-save", vec![UiPattern::Invoke]),
            state("btn-save", None, true, true),
            Behavior::RemoveOnInvoke,
        );
        let h = AppHandle::new(AppKind::Generic);
        let receipt = f
            .driver
            .run_action(&h, &action("invoke", json!({ "name": "保存" })), "sess-remove")
            .expect("invoke 应成功");

        assert_eq!(receipt.effect.as_deref(), Some("node_removed"));
        assert!(receipt.verified);
        assert!(receipt.before.is_some(), "删除前必须读到节点");
        assert!(receipt.after.is_none(), "删除后不能再读到节点");

        let audit = read_audit(&receipt);
        assert!(audit["before"].is_object());
        assert!(audit["after"].is_null(), "after 必须如实为 null");
    }

    #[test]
    fn action_without_visible_effect_is_unverified_but_ok() {
        let f = fixture(
            node("button", "确定", "btn-ok", vec![UiPattern::SetFocus]),
            state("btn-ok", None, true, true),
            Behavior::Noop,
        );
        let h = AppHandle::new(AppKind::Generic);
        let receipt = f
            .driver
            .run_action(&h, &action("focus", json!({ "ref_id": "btn-ok" })), "sess-noop")
            .expect("focus 应成功");

        assert!(receipt.ok, "无可观测变化不改变 ok");
        assert_eq!(receipt.effect.as_deref(), Some("no_observable_change"));
        assert!(!receipt.verified, "没有变化就不得声称已验证");
        let audit = read_audit(&receipt);
        assert_eq!(audit["before"], audit["after"], "前后摘要应完全相同");
    }

    /// `toggle` 属于 `state_changed`：value 没变、开关状态变了，回执与审计都必须如实说明。
    #[test]
    fn toggle_reports_state_changed_not_value_changed() {
        let mut before_state = state("chk-1", None, true, true);
        before_state.toggle_state = Some("off".into());
        let f = fixture(
            node("check_box", "启用", "chk-1", vec![UiPattern::Toggle]),
            before_state,
            Behavior::ToggleState,
        );
        let h = AppHandle::new(AppKind::Generic);
        let receipt = f
            .driver
            .run_action(&h, &action("toggle", json!({ "ref_id": "chk-1" })), "sess-toggle")
            .expect("toggle 应成功");
        assert!(receipt.ok);
        assert_eq!(receipt.effect.as_deref(), Some("state_changed"));
        assert!(receipt.verified);
        assert_eq!(receipt.before.as_ref().unwrap()["toggle_state"], "off");
        assert_eq!(receipt.after.as_ref().unwrap()["toggle_state"], "on");

        let audit = read_audit(&receipt);
        assert_eq!(audit["before"]["toggle_state"], "off");
        assert_eq!(audit["after"]["toggle_state"], "on");
    }

    // ── W6.1：动作 kind → pattern 分发（mock source，无桌面也能跑） ──

    #[test]
    fn every_pattern_kind_dispatches_the_expected_op() {
        let cases: Vec<(&str, Value, PatternOp)> = vec![
            ("invoke", json!({ "ref_id": "n1" }), PatternOp::Invoke),
            ("toggle", json!({ "ref_id": "n1" }), PatternOp::Toggle),
            ("select", json!({ "ref_id": "n1" }), PatternOp::Select),
            ("expand", json!({ "ref_id": "n1" }), PatternOp::Expand),
            ("collapse", json!({ "ref_id": "n1" }), PatternOp::Collapse),
            ("focus", json!({ "ref_id": "n1" }), PatternOp::Focus),
            (
                "set_value",
                json!({ "ref_id": "n1", "value": "hello" }),
                PatternOp::SetValue("hello".into()),
            ),
            (
                "type",
                json!({ "ref_id": "n1", "text": "hi" }),
                PatternOp::TypeText {
                    text: "hi".into(),
                    clear: true,
                },
            ),
            (
                "type",
                json!({ "ref_id": "n1", "text": "!", "clear": false }),
                PatternOp::TypeText {
                    text: "!".into(),
                    clear: false,
                },
            ),
            (
                "scroll",
                json!({ "ref_id": "n1", "amount": 2 }),
                PatternOp::Scroll {
                    horizontal: 0,
                    vertical: 2,
                },
            ),
            (
                "scroll",
                json!({ "ref_id": "n1", "amount": 1, "horizontal": true, "vertical": false }),
                PatternOp::Scroll {
                    horizontal: 1,
                    vertical: 0,
                },
            ),
        ];
        for (kind, params, expected) in cases {
            let f = fixture(
                node(
                    "edit",
                    "输入",
                    "n1",
                    vec![
                        UiPattern::Invoke,
                        UiPattern::Toggle,
                        UiPattern::SelectionItem,
                        UiPattern::Value,
                        UiPattern::ExpandCollapse,
                        UiPattern::Scroll,
                        UiPattern::SetFocus,
                    ],
                ),
                state("n1", Some("old"), true, true),
                Behavior::Noop,
            );
            let h = AppHandle::new(AppKind::Generic);
            f.driver
                .run_action(&h, &action(kind, params.clone()), "s")
                .unwrap_or_else(|e| panic!("{kind} {params} 应成功，实际: {e:?}"));
            let ops = f.source.ops();
            assert_eq!(ops.len(), 1, "{kind} 应只执行一次 pattern: {ops:?}");
            assert_eq!(ops[0].1, expected, "{kind} 分发的 pattern 不对");
        }
    }

    #[test]
    fn click_uses_the_fallback_chain_not_a_specific_pattern() {
        let f = fixture(
            node("button", "确定", "btn-ok", vec![UiPattern::Invoke]),
            state("btn-ok", None, true, true),
            Behavior::Noop,
        );
        let h = AppHandle::new(AppKind::Generic);
        f.driver
            .run_action(&h, &action("click", json!({ "name": "确定" })), "s")
            .expect("click 应成功");
        assert_eq!(f.source.click_count(), 1, "click 必须走 click 退化链");
        assert!(f.source.ops().is_empty(), "click 不应直接调 pattern");
    }

    #[test]
    fn set_value_without_value_is_rejected() {
        let f = fixture(
            node("edit", "用户名", "edit-1", vec![UiPattern::Value]),
            state("edit-1", Some("old"), true, true),
            Behavior::Noop,
        );
        let h = AppHandle::new(AppKind::Generic);
        let err = f
            .driver
            .run_action(&h, &action("set_value", json!({ "ref_id": "edit-1" })), "s")
            .expect_err("缺 value 必须失败");
        assert!(matches!(err, DesktopError::UnsupportedAction(_)), "实际: {err:?}");
    }

    #[test]
    fn validate_value_action_reports_value_changed() {
        let f = fixture(
            node("edit", "用户名", "edit-1", vec![UiPattern::Value]),
            state("edit-1", Some("old"), true, true),
            Behavior::SetValue,
        );
        let h = AppHandle::new(AppKind::Generic);
        // type（默认 clear=true）同样属于 ValuePattern 写动作。
        let receipt = f
            .driver
            .run_action(
                &h,
                &action("type", json!({ "ref_id": "edit-1", "text": "new" })),
                "sess-type",
            )
            .expect("type 应成功");
        assert_eq!(receipt.effect.as_deref(), Some("value_changed"));
        assert_eq!(receipt.after.as_ref().unwrap()["value"], "new");
    }

    // ── 只读动作 / find / wait ──

    #[test]
    fn snapshot_is_read_only_and_carries_automation_id_and_patterns() {
        let f = fixture(
            node("edit", "用户名", "edit-1", vec![UiPattern::Value])
                .with_automation_id("UserEdit"),
            state("edit-1", Some("v"), true, true),
            Behavior::Noop,
        );
        let h = AppHandle::new(AppKind::Generic);
        let receipt = f
            .driver
            .run_action(&h, &action("snapshot", json!({})), "s")
            .expect("snapshot 应成功");
        assert!(receipt.snapshot_path.is_none(), "只读动作不落审计");
        assert!(!receipt.verified, "只读动作没有 verify 语义");
        let nodes = receipt.detail["nodes"].as_array().expect("nodes");
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0]["automation_id"], "UserEdit");
        assert_eq!(nodes[0]["patterns"][0], "value");
        assert_eq!(nodes[0]["bounds"]["width"], 100);
    }

    #[test]
    fn find_rejects_empty_query_and_honours_limit() {
        let f = fixture(
            node("button", "确定", "btn-ok", vec![UiPattern::Invoke])
                .with_automation_id("OkButton"),
            state("btn-ok", None, true, true),
            Behavior::Noop,
        );
        let h = AppHandle::new(AppKind::Generic);
        let err = f
            .driver
            .run_action(&h, &action("find", json!({})), "s")
            .expect_err("空条件必须失败");
        assert!(matches!(err, DesktopError::UnsupportedAction(_)));

        let receipt = f
            .driver
            .run_action(
                &h,
                &action("find", json!({ "automation_id": "OkButton", "limit": 1 })),
                "s",
            )
            .expect("find 应成功");
        assert_eq!(receipt.detail["count"], 1);
        assert_eq!(receipt.detail["limit"], 1);
        assert_eq!(receipt.detail["nodes"][0]["ref_id"], "btn-ok");
        // 只读：不落审计，也不带观测载荷。
        assert!(receipt.snapshot_path.is_none(), "find 不落审计");
        assert!(receipt.before.is_none() && receipt.after.is_none());
        assert!(receipt.effect.is_none());

        // 上限只能往下压：limit=999 必须被夹到 20 再下发给后端。
        let wide = f
            .driver
            .run_action(
                &h,
                &action("find", json!({ "automation_id": "OkButton", "limit": 999 })),
                "s",
            )
            .expect("find 应成功");
        assert_eq!(wide.detail["limit"], 20, "find 最多 20 条");
        assert_eq!(
            f.source.find_limits(),
            vec![1, 20],
            "驱动下发给后端的 limit 必须先夹紧"
        );
    }

    /// `find` 的上限是硬契约（W6.1「最多 20」），不接受参数抬升。
    #[test]
    fn find_limit_is_capped_at_twenty_and_only_shrinks() {
        assert_eq!(MAX_FIND_RESULTS, 20, "W6.1 契约：find 最多 20 条");
        assert_eq!(find_limit(&action("find", json!({}))), 20);
        assert_eq!(find_limit(&action("find", json!({ "limit": 999 }))), 20);
        assert_eq!(find_limit(&action("find", json!({ "limit": 0 }))), 1);
        assert_eq!(find_limit(&action("find", json!({ "limit": -5 }))), 1);
        assert_eq!(find_limit(&action("find", json!({ "limit": 5 }))), 5);
        assert_eq!(find_limit(&action("find", json!({ "max": 3 }))), 3);
    }

    #[test]
    fn wait_succeeds_for_an_already_satisfied_state() {
        let f = fixture(
            node("edit", "用户名", "edit-1", vec![UiPattern::Value]),
            state("edit-1", Some("v"), true, true),
            Behavior::Noop,
        );
        let h = AppHandle::new(AppKind::Generic);
        let receipt = f
            .driver
            .run_action(
                &h,
                &action("wait", json!({ "ref_id": "edit-1", "state": "enabled", "timeout_ms": 50 })),
                "sess-wait-hit",
            )
            .expect("条件已成立应立即返回");
        assert!(receipt.ok);
        assert!(receipt.verified, "命中条件必须 verified=true");
        assert_eq!(receipt.detail["state"], "enabled");
        assert_eq!(receipt.detail["hit"], true, "命中必须如实报告 hit");
        assert_eq!(receipt.detail["ref_id"], "edit-1");
        // wait 是只读动作：不落审计、不带观测载荷、不声明「发生了什么变化」。
        assert!(receipt.snapshot_path.is_none(), "wait 不落审计");
        assert!(receipt.before.is_none() && receipt.after.is_none());
        assert!(receipt.effect.is_none());
        assert!(
            Recorder::list_snapshots("sess-wait-hit").is_empty(),
            "只读 wait 不得在审计目录留下文件"
        );
    }

    /// `wait` 支持按 `name` 定位：每轮重新查，命中后回执里带上真正解析到的 `ref_id`。
    #[test]
    fn wait_resolves_by_name_and_reports_the_resolved_ref() {
        let f = fixture(
            node("edit", "用户名", "edit-1", vec![UiPattern::Value]),
            state("edit-1", Some("v"), true, true),
            Behavior::Noop,
        );
        let h = AppHandle::new(AppKind::Generic);
        let receipt = f
            .driver
            .run_action(
                &h,
                &action("wait", json!({ "name": "用户名", "state": "exists", "timeout_ms": 50 })),
                "sess-wait-name",
            )
            .expect("按 name 应能命中");
        assert!(receipt.verified);
        assert_eq!(receipt.detail["hit"], true);
        assert_eq!(receipt.detail["ref_id"], "edit-1");
        assert!(receipt.snapshot_path.is_none());
    }

    /// `wait` 能等一个**还不存在**的节点：不立刻报 ELEMENT_NOT_FOUND，而是轮询到预算耗尽。
    #[test]
    fn wait_for_a_node_that_never_appears_times_out_without_audit() {
        let f = fixture(
            node("edit", "用户名", "edit-1", vec![UiPattern::Value]),
            state("edit-1", Some("v"), true, true),
            Behavior::Noop,
        );
        let h = AppHandle::new(AppKind::Generic);
        let start = Instant::now();
        let err = f
            .driver
            .run_action(
                &h,
                &action(
                    "wait",
                    json!({ "ref_id": "ghost", "state": "exists", "timeout_ms": 150 }),
                ),
                "sess-wait-miss",
            )
            .expect_err("永不出现的节点必须超时");
        assert!(matches!(err, DesktopError::Timeout(_)), "实际: {err:?}");
        assert!(
            start.elapsed() >= Duration::from_millis(120),
            "必须真的轮询到预算附近（实际 {:?}）",
            start.elapsed()
        );
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(
            Recorder::list_snapshots("sess-wait-miss").is_empty(),
            "超时的只读 wait 也不得落审计"
        );
    }

    #[test]
    fn wait_times_out_with_a_structured_error() {
        let f = fixture(
            node("edit", "用户名", "edit-1", vec![UiPattern::Value]),
            state("edit-1", Some("v"), true, false),
            Behavior::Noop,
        );
        let h = AppHandle::new(AppKind::Generic);
        let start = Instant::now();
        let err = f
            .driver
            .run_action(
                &h,
                &action("wait", json!({ "ref_id": "edit-1", "state": "focused", "timeout_ms": 150 })),
                "s",
            )
            .expect_err("条件永不成立必须超时");
        assert!(matches!(err, DesktopError::Timeout(_)), "实际: {err:?}");
        assert!(start.elapsed() < Duration::from_secs(3), "必须在预算附近返回");
    }

    #[test]
    fn wait_rejects_unknown_state_without_polling() {
        let f = fixture(
            node("edit", "用户名", "edit-1", vec![]),
            state("edit-1", Some("v"), true, true),
            Behavior::Noop,
        );
        let h = AppHandle::new(AppKind::Generic);
        let err = f
            .driver
            .run_action(
                &h,
                &action("wait", json!({ "ref_id": "edit-1", "state": "蓝" })),
                "s",
            )
            .expect_err("未知 state 必须失败");
        assert!(matches!(err, DesktopError::UnsupportedAction(_)), "实际: {err:?}");
    }

    /// 显式 `ref_id` 指向不存在的节点：动作前观测失败 → ELEMENT_NOT_FOUND，
    /// 不 panic、不执行任何 pattern（也不会被误当成 node_removed）。
    #[test]
    fn missing_ref_id_is_element_not_found_without_touching_the_backend() {
        let f = fixture(
            node("edit", "用户名", "edit-1", vec![UiPattern::Value]),
            state("edit-1", Some("old"), true, true),
            Behavior::SetValue,
        );
        let h = AppHandle::new(AppKind::Generic);
        for kind in ["set_value", "invoke", "toggle"] {
            let err = f
                .driver
                .run_action(
                    &h,
                    &action(kind, json!({ "ref_id": "ghost", "value": "x" })),
                    "sess-missing",
                )
                .expect_err("不存在的 ref_id 必须失败");
            assert!(
                matches!(err, DesktopError::ElementNotFound(_)),
                "{kind} 实际: {err:?}"
            );
        }
        assert!(f.source.ops().is_empty(), "找不到节点时不得执行 pattern");
        assert!(
            Recorder::list_snapshots("sess-missing").is_empty(),
            "动作没发生就不该留下审计文件"
        );
    }

    /// `value` / `text` 接受非字符串载荷（模型常把值写成数字），空串仍是合法载荷（清空）。
    #[test]
    fn text_payloads_accept_numbers_and_empty_strings_but_reject_null() {
        let f = fixture(
            node("edit", "用户名", "edit-1", vec![UiPattern::Value]),
            state("edit-1", Some("old"), true, true),
            Behavior::SetValue,
        );
        let h = AppHandle::new(AppKind::Generic);

        let number = f
            .driver
            .run_action(
                &h,
                &action("set_value", json!({ "ref_id": "edit-1", "value": 42 })),
                "sess-num",
            )
            .expect("数字载荷应被写成字符串");
        assert_eq!(number.after.as_ref().unwrap()["value"], "42");

        let cleared = f
            .driver
            .run_action(
                &h,
                &action("set_value", json!({ "ref_id": "edit-1", "value": "" })),
                "sess-empty",
            )
            .expect("空串是「清空」而不是「未提供」");
        assert_eq!(cleared.effect.as_deref(), Some("value_changed"));
        assert_eq!(cleared.after.as_ref().unwrap()["value"], "");

        // 缺 key / null 仍然是调用方错误（不能猜成空串）。
        let err = f
            .driver
            .run_action(
                &h,
                &action("set_value", json!({ "ref_id": "edit-1", "value": null })),
                "sess-null",
            )
            .expect_err("null 载荷必须失败");
        assert!(matches!(err, DesktopError::UnsupportedAction(_)), "实际: {err:?}");
    }

    /// 窗口前置的效果判定是纯函数：同窗口=无可观测变化，换窗口/换标题/无前置=state_changed。
    #[test]
    fn window_effect_is_honest_about_activation() {
        let foreground = json!({ "hwnd": 100, "title": "记事本" });
        let other = json!({ "hwnd": 200, "title": "计算器" });
        let renamed = json!({ "hwnd": 100, "title": "未命名 - 记事本" });

        assert_eq!(
            window_effect(Some(&foreground), &foreground),
            ObservationEffect::NoObservableChange,
            "本来就在前台就是没有可观测变化"
        );
        assert_eq!(
            window_effect(Some(&foreground), &other),
            ObservationEffect::StateChanged
        );
        assert_eq!(
            window_effect(Some(&foreground), &renamed),
            ObservationEffect::StateChanged
        );
        assert_eq!(
            window_effect(None, &other),
            ObservationEffect::StateChanged,
            "没观测到「之前」时不声称没有变化"
        );
        // 摘要必须带句柄（W6.1：window_activate 的回执要能拿到窗口句柄）。
        let info = windows::WindowInfo {
            hwnd: 4242,
            title: "记事本".into(),
            process_name: "notepad.exe".into(),
            pid: 7,
            foreground: true,
            iconic: false,
            bounds: Bounds {
                x: 0,
                y: 0,
                width: 100,
                height: 60,
            },
        };
        let summary = window_summary(&info);
        assert_eq!(summary["hwnd"], 4242);
        assert_eq!(summary["title"], "记事本");
        assert_eq!(summary["process_name"], "notepad.exe");
        // 摘要只放比较需要的字段，不得夹带屏幕截图之类的大载荷。
        assert!(summary.get("screenshot").is_none());
        // 同一份摘要（真实形状）必须判成「没有变化」，否则每个 window_activate 都会假报 state_changed。
        assert_eq!(
            window_effect(Some(&summary), &summary),
            ObservationEffect::NoObservableChange
        );
    }

    /// A11yError → DesktopError 的映射语义：找不到节点≠通道不可用≠不支持≠执行失败。
    #[test]
    fn a11y_error_mapping_keeps_semantics() {
        assert!(matches!(
            a11y_err(A11yError::NodeNotFound("42.1".into())),
            DesktopError::ElementNotFound(id) if id == "42.1"
        ));
        assert!(matches!(
            a11y_err(A11yError::Unavailable("no uia".into())),
            DesktopError::ComError(m) if m.contains("无障碍通道不可用")
        ));
        assert!(matches!(
            a11y_err(A11yError::OsFailure("boom".into())),
            DesktopError::ComError(m) if m == "boom"
        ));
        assert!(matches!(
            a11y_err(A11yError::Unsupported("no value pattern".into())),
            DesktopError::UnsupportedAction(_)
        ));
    }

    #[test]
    fn action_surface_matches_the_documented_matrix() {
        // 每一个已登记 kind 都必须被 run_action 或 op_for_action 接住（防止「文档写了、代码没接」）。
        let handled = [
            "snapshot",
            "find",
            "click",
            "invoke",
            "toggle",
            "select",
            "expand",
            "collapse",
            "focus",
            "set_value",
            "type",
            "scroll",
            "wait",
            "window_activate",
        ];
        for kind in handled {
            assert!(KNOWN_ACTION_KINDS.contains(&kind), "{kind} 未登记");
        }
        assert!(KNOWN_ACTION_KINDS.contains(&"generate_presentation"));
        assert!(!is_known_action_kind("attach"));
    }
}
