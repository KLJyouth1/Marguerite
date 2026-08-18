//! 输入模拟兜底驱动骨架（C5 阶段接入 enigo：坐标 / 热键）。
//!
//! MVP 仅保留可编译骨架：未注册到注册表，方法返回明确的「未实现」错误。

use serde_json::Value;

use crate::desktop_automation::driver::*;

pub struct InputDriver;

impl DesktopAppDriver for InputDriver {
    fn kind(&self) -> AppKind {
        AppKind::Generic
    }

    fn launch(&self, _args: &Value) -> Result<AppHandle, DesktopError> {
        Err(DesktopError::UnsupportedAction(
            "InputDriver 尚未实现（C5 阶段接入 enigo）".into(),
        ))
    }

    fn activate(&self, _h: &AppHandle) -> Result<(), DesktopError> {
        Err(DesktopError::UnsupportedAction("InputDriver 尚未实现".into()))
    }

    fn run_action(
        &self,
        _h: &AppHandle,
        action: &UiAction,
        _session: &str,
    ) -> Result<ActionReceipt, DesktopError> {
        Err(DesktopError::UnsupportedAction(format!(
            "InputDriver 尚未实现: {}",
            action.kind
        )))
    }

    fn quit(&self, _h: &AppHandle) -> Result<(), DesktopError> {
        Err(DesktopError::UnsupportedAction("InputDriver 尚未实现".into()))
    }
}
