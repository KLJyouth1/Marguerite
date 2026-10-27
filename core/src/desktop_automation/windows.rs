//! 原生窗口枚举 / 查找 / 激活（Windows）。
//!
//! 为什么单独一个文件：W6.2 要求「附着已打开的应用」——附着的前提是**能按标题或
//! 进程名找到窗口**。此前 `computer_use.window_list` 是 PowerShell 脚本（`pilot`），
//! 桌面自动化框架里则完全没有窗口枚举；本模块用 `EnumWindows` 直连，给
//! `desktop.windows` RPC、`desktop.action` 的 `window` 参数与 `desktop_attach` 工具
//! 提供同一份事实源。
//!
//! 非 Windows 平台：所有函数返回 [`DesktopError::UnsupportedAction`]，绝不返回
//! 「空列表」冒充「没有窗口」。

use serde::Serialize;

use crate::desktop_automation::driver::{Bounds, DesktopError};

/// 一个可见顶层窗口。
#[derive(Debug, Clone, Serialize)]
pub struct WindowInfo {
    pub hwnd: i64,
    pub title: String,
    /// 可执行文件名（如 `notepad.exe`；读不到时为空串，不编造）。
    pub process_name: String,
    pub pid: u32,
    pub foreground: bool,
    /// 最小化（还原后才能可靠 SetForegroundWindow）。
    pub iconic: bool,
    pub bounds: Bounds,
}

impl WindowInfo {
    /// 匹配查询串：标题/进程名大小写不敏感子串。
    pub fn matches(&self, needle_lower: &str) -> bool {
        self.title.to_lowercase().contains(needle_lower)
            || self.process_name.to_lowercase().contains(needle_lower)
    }
}

/// 列出所有可见顶层窗口（前台窗口排第一，其余按标题排序）。
pub fn list_windows() -> Result<Vec<WindowInfo>, DesktopError> {
    #[cfg(windows)]
    {
        Ok(platform::list_visible_windows())
    }
    #[cfg(not(windows))]
    {
        Err(DesktopError::UnsupportedAction(
            "窗口枚举/附着仅支持 Windows".into(),
        ))
    }
}

/// 当前前台窗口。
pub fn foreground_window() -> Result<WindowInfo, DesktopError> {
    #[cfg(windows)]
    {
        platform::foreground()
    }
    #[cfg(not(windows))]
    {
        Err(DesktopError::UnsupportedAction(
            "前台窗口读取仅支持 Windows".into(),
        ))
    }
}

/// 按标题或进程名附着窗口（空串视为调用方错误 → `WINDOW_NOT_FOUND`）。
///
/// 匹配优先级：标题完全相等 → 标题子串 → 进程名子串；平票时前台窗口优先。
pub fn find_window(query: &str) -> Result<WindowInfo, DesktopError> {
    let needle = query.trim();
    if needle.is_empty() {
        return Err(DesktopError::WindowNotFound(
            "窗口标题/进程名为空（请给出已打开窗口的标题或进程名）".into(),
        ));
    }
    let windows = list_windows()?;
    let lower = needle.to_lowercase();
    let exact = windows
        .iter()
        .find(|w| w.title.to_lowercase() == lower)
        .or_else(|| windows.iter().find(|w| w.matches(&lower) && w.foreground))
        .or_else(|| windows.iter().find(|w| w.matches(&lower)));
    exact.cloned().ok_or_else(|| {
        DesktopError::WindowNotFound(format!(
            "找不到标题/进程名含「{needle}」的可见窗口（共 {} 个可见窗口）",
            windows.len()
        ))
    })
}

/// 按 hwnd 激活窗口并返回它的最新信息。
pub fn activate_window(hwnd: i64) -> Result<WindowInfo, DesktopError> {
    #[cfg(windows)]
    {
        platform::activate(hwnd)
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        Err(DesktopError::UnsupportedAction(
            "窗口激活仅支持 Windows".into(),
        ))
    }
}

#[cfg(windows)]
mod platform {
    use super::{Bounds, DesktopError, WindowInfo};
    use windows::core::{BOOL, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetForegroundWindow, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
        GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, SetForegroundWindow,
        ShowWindow, SW_RESTORE,
    };

    fn hwnd_of(raw: i64) -> HWND {
        HWND(raw as *mut core::ffi::c_void)
    }

    fn wide_to_string(buf: &[u16], len: usize) -> String {
        String::from_utf16_lossy(&buf[..len.min(buf.len())])
    }

    fn window_title(hwnd: HWND) -> String {
        unsafe {
            let len = GetWindowTextLengthW(hwnd);
            if len <= 0 {
                return String::new();
            }
            let mut buf = vec![0u16; len as usize + 1];
            let copied = GetWindowTextW(hwnd, &mut buf);
            if copied <= 0 {
                return String::new();
            }
            wide_to_string(&buf, copied as usize)
        }
    }

    /// 可执行文件名。读不到时返回空串（不猜、不编造）。
    fn process_name(pid: u32) -> String {
        if pid == 0 {
            return String::new();
        }
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return String::new();
            };
            let mut buf = [0u16; 1024];
            let mut size = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut size,
            )
            .is_ok();
            let _ = CloseHandle(handle);
            if !ok {
                return String::new();
            }
            let full = wide_to_string(&buf, size as usize);
            full.rsplit(['\\', '/'])
                .next()
                .unwrap_or(full.as_str())
                .to_string()
        }
    }

    fn bounds(hwnd: HWND) -> Bounds {
        unsafe {
            let mut rect = windows::Win32::Foundation::RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_ok() {
                Bounds {
                    x: rect.left,
                    y: rect.top,
                    width: (rect.right - rect.left).max(0),
                    height: (rect.bottom - rect.top).max(0),
                }
            } else {
                Bounds {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                }
            }
        }
    }

    /// 采集单个窗口；不合格（不可见 / 无标题 / 零面积）返回 `None`。
    fn collect(hwnd: HWND, foreground: HWND) -> Option<WindowInfo> {
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() {
                return None;
            }
            if !IsWindowVisible(hwnd).as_bool() {
                return None;
            }
            let title = window_title(hwnd);
            if title.trim().is_empty() {
                return None;
            }
            let b = bounds(hwnd);
            if b.width <= 0 || b.height <= 0 {
                return None;
            }
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            Some(WindowInfo {
                hwnd: hwnd.0 as i64,
                title,
                process_name: process_name(pid),
                pid,
                foreground: hwnd == foreground,
                iconic: IsIconic(hwnd).as_bool(),
                bounds: b,
            })
        }
    }

    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let out = &mut *(lparam.0 as *mut (Vec<WindowInfo>, HWND));
        if let Some(info) = collect(hwnd, out.1) {
            out.0.push(info);
        }
        true.into()
    }

    pub(super) fn list_visible_windows() -> Vec<WindowInfo> {
        let foreground = unsafe { GetForegroundWindow() };
        let mut out: (Vec<WindowInfo>, HWND) = (Vec::new(), foreground);
        unsafe {
            let _ = EnumWindows(
                Some(enum_cb),
                LPARAM(&mut out as *mut (Vec<WindowInfo>, HWND) as isize),
            );
        }
        let mut windows = out.0;
        windows.sort_by(|a, b| {
            b.foreground
                .cmp(&a.foreground)
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });
        windows
    }

    pub(super) fn foreground() -> Result<WindowInfo, DesktopError> {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_invalid() {
            return Err(DesktopError::WindowNotFound(
                "当前无前台窗口（会话未交互或已锁屏）".into(),
            ));
        }
        // 前台窗口一定可见，但标题可能为空（UIA 仍可读）——所以这里不用 collect 的标题过滤。
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        Ok(WindowInfo {
            hwnd: hwnd.0 as i64,
            title: window_title(hwnd),
            process_name: process_name(pid),
            pid,
            foreground: true,
            iconic: unsafe { IsIconic(hwnd).as_bool() },
            bounds: bounds(hwnd),
        })
    }

    pub(super) fn activate(hwnd_raw: i64) -> Result<WindowInfo, DesktopError> {
        let hwnd = hwnd_of(hwnd_raw);
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() {
                return Err(DesktopError::WindowNotFound(format!(
                    "hwnd {hwnd_raw} 不是有效窗口（可能已关闭）"
                )));
            }
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            if !SetForegroundWindow(hwnd).as_bool() {
                return Err(DesktopError::ComError(
                    "SetForegroundWindow 被系统拒绝（前台锁定策略或目标窗口已销毁）".into(),
                ));
            }
        }
        // 激活后重新采集一次：标题/前台标记都取最新值。
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        Ok(WindowInfo {
            hwnd: hwnd_raw,
            title: window_title(hwnd),
            process_name: process_name(pid),
            pid,
            foreground: true,
            iconic: unsafe { IsIconic(hwnd).as_bool() },
            bounds: bounds(hwnd),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 空查询必须立刻失败（W6.2：空标题 → WINDOW_NOT_FOUND），且不依赖桌面会话。
    #[test]
    fn empty_query_is_window_not_found_without_touching_the_desktop() {
        for q in ["", "   ", "\t"] {
            match find_window(q) {
                Err(DesktopError::WindowNotFound(msg)) => {
                    assert!(!msg.is_empty(), "错误文案不得为空");
                }
                other => panic!("空查询应返回 WindowNotFound，实际: {other:?}"),
            }
        }
    }

    /// 枚举是诚实的：要么给出真实窗口（hwnd 非零、标题非空、尺寸为正），
    /// 要么在无桌面会话下给出空列表 —— 绝不编造假窗口。
    #[test]
    fn list_windows_is_honest_or_empty() {
        match list_windows() {
            Ok(windows) => {
                for w in &windows {
                    assert_ne!(w.hwnd, 0, "hwnd 不得为 0");
                    assert!(!w.title.trim().is_empty(), "列表里的窗口必须有标题");
                    assert!(w.bounds.width > 0 && w.bounds.height > 0);
                }
            }
            Err(DesktopError::UnsupportedAction(_)) => {}
            Err(e) => panic!("list_windows 只允许诚实失败，实际: {e:?}"),
        }
    }

    /// 不存在的 hwnd 激活必须失败，不得假装成功。
    #[test]
    fn activating_a_bogus_handle_fails() {
        match activate_window(1) {
            Err(DesktopError::WindowNotFound(_))
            | Err(DesktopError::ComError(_))
            | Err(DesktopError::UnsupportedAction(_)) => {}
            Err(e) => panic!("意外错误类型: {e:?}"),
            Ok(w) => panic!("hwnd=1 不可能被激活，却返回 {w:?}"),
        }
    }
}
