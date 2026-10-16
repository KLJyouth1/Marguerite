//! 通用 UIA 驱动（[`AppKind::Generic`]）：用 Windows UI Automation 操控**当前前台应用**的控件。
//!
//! 与 [`crate::desktop_automation::com`]（WPS 专用、late-bound IDispatch）互补：
//! 本驱动不绑定任何具体应用，凡是把控件暴露给 UIA 的程序都能被元素级读取与激活。
//! 这条「通用跨应用 UIA」路径此前在能力项 `auto.uia` 里被如实标注为**未接通**，
//! 现在由 [`WindowsUiaSource`] + 本驱动接通。
//!
//! # 动作契约（`UiAction.kind`）
//!
//! - `snapshot`：读前台窗口控件树，回执 `detail` 为 `{count, nodes:[{role,name,ref_id,x,y,w,h,clickable}]}`
//!   —— 只读，不落审计快照。
//! - `click`：按 `ref_id`（来自 `snapshot`）或 `name`（控件可见名）激活控件；落审计快照。
//!
//! 其余 kind 一律 [`DesktopError::UnsupportedAction`]，绝不假装支持。
//!
//! # 安全与审计
//!
//! 本驱动不自建闸门：`desktop.action` / `desktop_action` 已统一做 Policy 二次确认与注入扫描，
//! 所有真实动作再经 [`Recorder`] 落 `.marguerite/audit-snapshots/<session>/`。

#![cfg(windows)]

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{json, Value};
use windows::Win32::Foundation::{FALSE, HWND};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};

use crate::desktop_automation::a11y_tree::{A11yError, A11ySource, UiNode};
use crate::desktop_automation::driver::*;
use crate::desktop_automation::recorder::Recorder;
use crate::desktop_automation::windows_uia::WindowsUiaSource;

/// [`A11yError`] → [`DesktopError`]。映射保持语义：通道缺失≠找不到节点≠执行失败。
fn a11y_err(e: A11yError) -> DesktopError {
    match e {
        A11yError::Unavailable(m) => DesktopError::ComError(format!("无障碍通道不可用: {m}")),
        A11yError::NodeNotFound(id) => DesktopError::ElementNotFound(id),
        A11yError::OsFailure(m) => DesktopError::ComError(m),
    }
}

/// 控件 → JSON（显式列字段，避免给 a11y_tree 的稳定契约加 serde 依赖）。
fn node_json(n: &UiNode) -> Value {
    json!({
        "role": n.role,
        "name": n.name,
        "ref_id": n.ref_id,
        "x": n.bounds.0,
        "y": n.bounds.1,
        "w": n.bounds.2,
        "h": n.bounds.3,
        "clickable": n.clickable,
    })
}

/// 通用 UIA 驱动。
pub struct UiaDriver {
    source: WindowsUiaSource,
    /// handle_id → 前台窗口 HWND。
    ///
    /// 存 `isize` 而不是 `HWND`：`HWND` 是裸指针（不 `Send`），而 `DesktopAppDriver` 要求
    /// `Send + Sync`。指针值本身只是个整数句柄，序列化/搬运都是安全的。
    windows: Mutex<HashMap<String, isize>>,
}

impl Default for UiaDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl UiaDriver {
    pub fn new() -> Self {
        UiaDriver {
            source: WindowsUiaSource::new(),
            windows: Mutex::new(HashMap::new()),
        }
    }

    fn bind(&self, handle_id: &str, hwnd: HWND) {
        self.windows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(handle_id.to_string(), hwnd.0 as isize);
    }

    fn bound(&self, handle_id: &str) -> Option<HWND> {
        self.windows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(handle_id)
            .copied()
            .map(|v| HWND(v as *mut core::ffi::c_void))
    }

    /// 把 `click` 的参数解析成一个具体 `ref_id`。
    ///
    /// 优先用调用方给的 `ref_id`（来自上一次 `snapshot`，最精确）；
    /// 退化到 `name` 时**优先命中可点节点**，避免同名静态文本抢走点击目标。
    fn resolve_ref(&self, action: &UiAction) -> Result<String, DesktopError> {
        if let Some(r) = action.param_str("ref_id") {
            if !r.is_empty() {
                return Ok(r);
            }
        }
        let name = action.param_str("name").filter(|s| !s.is_empty()).ok_or_else(|| {
            DesktopError::UnsupportedAction("click 需要 ref_id（来自 snapshot）或 name".to_string())
        })?;
        let snap = self.source.snapshot().map_err(a11y_err)?;
        let hit = snap
            .nodes
            .iter()
            .find(|n| n.name == name && n.clickable)
            .or_else(|| snap.nodes.iter().find(|n| n.name == name))
            .ok_or_else(|| DesktopError::ElementNotFound(name.clone()))?;
        Ok(hit.ref_id.clone())
    }
}

impl DesktopAppDriver for UiaDriver {
    fn kind(&self) -> AppKind {
        AppKind::Generic
    }

    /// 「启动」对通用驱动没有意义——它是**附着**语义：绑定当前前台窗口并验证其可读。
    ///
    /// 为什么不顺手加个「按 exe 路径启动进程」：那是进程管理，属于 `tool.exec` / `system_control`
    /// 的职责；塞进这里只会多一条无闸门的任意程序执行面。
    fn launch(&self, _args: &Value) -> Result<AppHandle, DesktopError> {
        let hwnd: HWND = unsafe { GetForegroundWindow() };
        if hwnd.is_invalid() {
            return Err(DesktopError::ComError(
                "当前无前台窗口（会话未交互或已锁屏），无法附着".to_string(),
            ));
        }
        // 先确认这个窗口真的能被 UIA 读：读不到就不发句柄，免得上层拿到「看起来可用」的假句柄。
        self.source.snapshot().map_err(a11y_err)?;
        let handle = AppHandle::new(AppKind::Generic);
        self.bind(&handle.id, hwnd);
        Ok(handle)
    }

    fn activate(&self, h: &AppHandle) -> Result<(), DesktopError> {
        let hwnd = self.bound(&h.id).ok_or_else(|| {
            DesktopError::NotRunning(format!("句柄 {} 未绑定前台窗口（或已 quit）", h.id))
        })?;
        if unsafe { SetForegroundWindow(hwnd) } == FALSE {
            return Err(DesktopError::ComError(
                "SetForegroundWindow 被系统拒绝（前台锁定策略或目标窗口已销毁）".to_string(),
            ));
        }
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
            "snapshot" => {
                let snap = self.source.snapshot().map_err(a11y_err)?;
                let nodes: Vec<Value> = snap.nodes.iter().map(node_json).collect();
                let hwnd = self.bound(&h.id).map(|w| w.0 as isize).unwrap_or(0);
                Ok(ActionReceipt::ok(
                    &h.id,
                    "snapshot",
                    json!({ "count": nodes.len(), "window": hwnd, "nodes": nodes }),
                    None,
                ))
            }
            "click" => {
                let ref_id = self.resolve_ref(action)?;
                self.source.click(&ref_id).map_err(a11y_err)?;
                let snap = Recorder::record_step(session, action, None, None, "uia_click");
                Ok(ActionReceipt::ok(
                    &h.id,
                    "click",
                    json!({ "ref_id": ref_id }),
                    Some(snap),
                ))
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn driver() -> UiaDriver {
        UiaDriver::new()
    }

    fn action(kind: &str, params: Value) -> UiAction {
        UiAction {
            kind: kind.to_string(),
            params,
        }
    }

    #[test]
    fn kind_is_generic() {
        assert_eq!(driver().kind(), AppKind::Generic);
    }

    /// 未实现的动作必须明确报 UnsupportedAction，而不是静默成功。
    /// 该路径在碰 COM 之前就返回，因此不需要桌面会话。
    #[test]
    fn unknown_action_is_unsupported_without_touching_com() {
        let d = driver();
        let h = AppHandle::new(AppKind::Generic);
        let err = d
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
        let d = driver();
        let h = AppHandle::new(AppKind::Generic);
        let err = d
            .run_action(&h, &action("click", json!({})), "s")
            .expect_err("缺少定位参数必须失败");
        assert!(matches!(err, DesktopError::UnsupportedAction(_)), "实际: {err:?}");

        // 空字符串同样视为缺失，不能退化成「ref_id = ""」去撞 COM。
        let err2 = d
            .run_action(&h, &action("click", json!({ "ref_id": "", "name": "" })), "s")
            .expect_err("空字符串必须失败");
        assert!(matches!(err2, DesktopError::UnsupportedAction(_)), "实际: {err2:?}");
    }

    /// 未绑定窗口的句柄做 activate 必须诚实失败。
    #[test]
    fn activate_unbound_handle_is_not_running() {
        let d = driver();
        let h = AppHandle::new(AppKind::Generic);
        let err = d.activate(&h).expect_err("未绑定句柄必须失败");
        assert!(matches!(err, DesktopError::NotRunning(_)), "实际: {err:?}");
    }

    /// quit 是幂等的：解绑 + 再 quit 都不应报错。
    #[test]
    fn quit_is_idempotent() {
        let d = driver();
        let h = AppHandle::new(AppKind::Generic);
        assert!(d.quit(&h).is_ok());
        assert!(d.quit(&h).is_ok());
        assert!(d.bound(&h.id).is_none());
    }

    /// A11yError → DesktopError 的映射语义：找不到节点≠通道不可用≠执行失败。
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
    }
}
