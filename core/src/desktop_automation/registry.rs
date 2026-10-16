//! 应用注册表：登记各 AppKind 的驱动，统一调度 launch / run_action / stop。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde_json::json;

use crate::desktop_automation::driver::*;

pub struct AppRegistry {
    drivers: Mutex<HashMap<AppKind, Box<dyn DesktopAppDriver>>>,
    // handle_id -> (kind, handle)
    handles: Mutex<HashMap<String, (AppKind, AppHandle)>>,
}

impl Default for AppRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AppRegistry {
    pub fn new() -> Self {
        AppRegistry {
            drivers: Mutex::new(HashMap::new()),
            handles: Mutex::new(HashMap::new()),
        }
    }

    /// 进程级单例：启动时注册**有真实实现**的驱动（仅 Windows）。
    ///
    /// 已注册：
    /// - `WpsPpt` → [`crate::desktop_automation::com::ComDriver`]（COM 生成 .pptx）；
    /// - `Generic` → [`crate::desktop_automation::uia_driver::UiaDriver`]（通用 UIA 元素级操作）。
    ///
    /// 其余 AppKind 保持未注册，`launch` 走 [`DesktopError::AppUnknown`]（网关映射 2401），
    /// 绝不注册返回「尚未实现」的桩驱动去假装能力存在。（InputDriver 等 C5 骨架已于 2026-10-14 下线。）
    pub fn global() -> &'static AppRegistry {
        static R: OnceLock<AppRegistry> = OnceLock::new();
        R.get_or_init(|| {
            let reg = AppRegistry::new();
            #[cfg(windows)]
            {
                reg.register(
                    AppKind::WpsPpt,
                    Box::new(crate::desktop_automation::com::ComDriver::new(
                        "Kwpp.Application",
                    )),
                );
                reg.register(
                    AppKind::Generic,
                    Box::new(crate::desktop_automation::uia_driver::UiaDriver::new()),
                );
            }
            reg
        })
    }

    pub fn register(&self, kind: AppKind, driver: Box<dyn DesktopAppDriver>) {
        self.drivers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(kind, driver);
    }

    /// 启动应用：取驱动 -> launch -> 记录句柄。
    pub fn launch(
        &self,
        kind: AppKind,
        args: &serde_json::Value,
    ) -> Result<AppHandle, DesktopError> {
        let handle = {
            let guard = self.drivers.lock().unwrap_or_else(|e| e.into_inner());
            let driver = guard
                .get(&kind)
                .ok_or_else(|| DesktopError::AppUnknown(kind.as_str().to_string()))?;
            driver.launch(args)?
        };
        self.handles
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(handle.id.clone(), (kind, handle.clone()));
        Ok(handle)
    }

    /// 执行动作：按句柄查驱动 -> run_action（含审计快照与产物登记）。
    pub fn run_action(
        &self,
        handle_id: &str,
        action: &UiAction,
        session: &str,
    ) -> Result<ActionReceipt, DesktopError> {
        let (kind, handle) = {
            let guard = self.handles.lock().unwrap_or_else(|e| e.into_inner());
            guard
                .get(handle_id)
                .cloned()
                .ok_or_else(|| DesktopError::NotRunning(handle_id.to_string()))?
        };
        let guard = self.drivers.lock().unwrap_or_else(|e| e.into_inner());
        let driver = guard
            .get(&kind)
            .ok_or_else(|| DesktopError::AppUnknown(kind.as_str().to_string()))?;
        driver.run_action(&handle, action, session)
    }

    /// 急停：对所有运行中句柄调用 quit（失败忽略），并清空句柄表。
    pub fn stop_all(&self) -> Result<(), DesktopError> {
        let snapshots: Vec<(AppKind, AppHandle)> = self
            .handles
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        let guard = self.drivers.lock().unwrap_or_else(|e| e.into_inner());
        for (kind, h) in &snapshots {
            if let Some(d) = guard.get(kind) {
                let _ = d.quit(h);
            }
        }
        self.handles
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        Ok(())
    }

    /// 已注册 + 运行中应用列表（前端 desktop.apps）。
    pub fn list_apps(&self) -> Vec<AppInfo> {
        let known = [
            AppKind::WpsPpt,
            AppKind::Browser,
            AppKind::Jianying,
            AppKind::ThreeD,
            AppKind::Generic,
        ];
        let drivers = self.drivers.lock().unwrap_or_else(|e| e.into_inner());
        let handles = self.handles.lock().unwrap_or_else(|e| e.into_inner());
        known
            .iter()
            .map(|k| AppInfo {
                kind: *k,
                name: kind_name(*k),
                registered: drivers.contains_key(k),
                running: handles.values().any(|(hk, _)| hk == k),
            })
            .collect()
    }
}

fn kind_name(k: AppKind) -> String {
    match k {
        AppKind::WpsPpt => "WPS Presentation",
        AppKind::Browser => "Browser",
        AppKind::Jianying => "Jianying",
        AppKind::ThreeD => "3D App",
        AppKind::Generic => "Generic App",
    }
    .to_string()
}

/// 仅用于 JSON 辅助（保持模块导出整洁）。
#[allow(dead_code)]
pub(crate) fn apps_json(apps: &[AppInfo]) -> serde_json::Value {
    json!(apps)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 注册表只允许注册**有真实实现**的驱动。
    ///
    /// 背景：C5 的 InputDriver 曾是「全部方法返回尚未实现」的桩，从未注册、从未被任何
    /// RPC/UI 触达，却让能力项看似可得。2026-10-14 已删除骨架，本测试锁死该决定：
    /// 万一将来有人重新注册桩驱动，这里必须红。
    ///
    /// 2026-10-15：`Generic` 重新出现在清单里**不是**回潮——它挂的是真实实现
    /// [`crate::desktop_automation::uia_driver::UiaDriver`]（Windows UI Automation，
    /// 见其模块头「动作契约」），与当年的空壳 UiaDriver 是两回事。
    #[test]
    fn global_registry_registers_only_real_drivers() {
        let reg = AppRegistry::global();
        let apps = reg.list_apps();
        let registered: Vec<AppKind> = apps
            .iter()
            .filter(|a| a.registered)
            .map(|a| a.kind)
            .collect();
        #[cfg(windows)]
        let expected = vec![AppKind::WpsPpt, AppKind::Generic];
        #[cfg(not(windows))]
        let expected: Vec<AppKind> = vec![];
        assert_eq!(
            registered, expected,
            "全局注册表只应注册有真实实现的驱动（Windows: WpsPpt + Generic(UIA)；其余平台: 无）"
        );
    }

    /// 未注册的应用种类必须是**诚实的 AppUnknown**，而不是静默成功或 panic。
    /// 前端 `desktop.list_apps` 也会因此如实显示 `registered: false`。
    #[test]
    fn unregistered_app_kind_fails_with_app_unknown() {
        let reg = AppRegistry::new();
        let err = reg
            .launch(AppKind::Generic, &json!({}))
            .expect_err("未注册的 Generic 必须失败");
        assert!(
            matches!(err, DesktopError::AppUnknown(_)),
            "未注册种类应返回 AppUnknown，实际: {err:?}"
        );
    }

    /// 对未启动句柄执行动作同样诚实失败（NotRunning），不 panicking。
    #[test]
    fn action_on_unknown_handle_fails_not_running() {
        let reg = AppRegistry::new();
        let action = UiAction {
            kind: "click".into(),
            params: json!({}),
        };
        let err = reg
            .run_action("no-such-handle", &action, "sess")
            .expect_err("未知句柄必须失败");
        assert!(
            matches!(err, DesktopError::NotRunning(_)),
            "未知句柄应返回 NotRunning，实际: {err:?}"
        );
    }
}
