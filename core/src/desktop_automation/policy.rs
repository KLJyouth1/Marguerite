//! 安全闸门（Epic C4）：全局开关 + 敏感动作二次确认 + 注入扫描。
//!
//! 完全开放模式下仍保留 kill-switch 与二次确认；前端在 `desktop.action` 调用前
//! 弹确认，后端也保留拒绝逻辑（未带 approved=true 则拒绝敏感动作）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use crate::desktop_automation::driver::UiAction;

static ENABLED: AtomicBool = AtomicBool::new(true);

pub struct Policy;

impl Policy {
    /// 全局开关注入点（kill-switch）。关闭后所有桌面自动化动作被拒。
    pub fn set_enabled(on: bool) {
        ENABLED.store(on, Ordering::SeqCst);
    }

    pub fn is_enabled() -> bool {
        ENABLED.load(Ordering::SeqCst)
    }

    /// 是否需要二次确认。默认对敏感动作返回 true；前端据此弹确认，
    /// 后端在 `desktop.action` 中若未带 `approved=true` 则拒绝。
    pub fn require_confirmation(action: &UiAction) -> bool {
        const SENSITIVE: &[&str] = &[
            "delete",
            "send",
            "pay",
            "quit",
            "close",
            "shutdown",
            "system_command",
            "inject",
        ];
        SENSITIVE.contains(&action.kind.as_str())
    }

    /// 注入扫描：检测动作参数中是否存在明显注入特征（命令拼接 / 路径逃逸）。
    /// 命中则返回告警理由，调用方应中断动作。
    pub fn scan_injection(action: &UiAction) -> Option<String> {
        let s = action.params.to_string();
        let patterns = [
            ";",
            "&&",
            "||",
            "|",
            "$(",
            "`",
            "..\\",
            "../",
            "cmd",
            "powershell",
            "regsvr32",
        ];
        for p in patterns {
            if s.contains(p) {
                return Some(format!("检测到潜在注入特征: {p}"));
            }
        }
        None
    }
}

/// 启动时在网关侧注册一个全局 Policy 实例引用（供未来扩展持久化开关）。
pub fn global_policy() -> &'static Policy {
    static P: OnceLock<Policy> = OnceLock::new();
    P.get_or_init(|| Policy)
}
