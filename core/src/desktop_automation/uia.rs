//! 通用 GUI 驱动骨架（C5 阶段接入 uiautomation）。
//!
//! MVP 仅保留可编译骨架：未注册到注册表，[`crate::desktop_automation::driver::DesktopAppDriver`]
//! 方法返回明确的「未实现」错误，避免 `todo!()` 在生产路径上 panic。

use serde_json::Value;

use crate::desktop_automation::driver::*;

pub struct UiaDriver;

impl DesktopAppDriver for UiaDriver {
    fn kind(&self) -> AppKind {
        AppKind::Generic
    }

    fn launch(&self, _args: &Value) -> Result<AppHandle, DesktopError> {
        Err(DesktopError::UnsupportedAction(
            "UiaDriver 尚未实现（C5 阶段接入 uiautomation）".into(),
        ))
    }

    fn activate(&self, _h: &AppHandle) -> Result<(), DesktopError> {
        Err(DesktopError::UnsupportedAction("UiaDriver 尚未实现".into()))
    }

    fn run_action(
        &self,
        _h: &AppHandle,
        action: &UiAction,
        _session: &str,
    ) -> Result<ActionReceipt, DesktopError> {
        Err(DesktopError::UnsupportedAction(format!(
            "UiaDriver 尚未实现: {}",
            action.kind
        )))
    }

    fn quit(&self, _h: &AppHandle) -> Result<(), DesktopError> {
        Err(DesktopError::UnsupportedAction("UiaDriver 尚未实现".into()))
    }
}
