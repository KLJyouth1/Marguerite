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

impl AppRegistry {
    pub fn new() -> Self {
        AppRegistry {
            drivers: Mutex::new(HashMap::new()),
            handles: Mutex::new(HashMap::new()),
        }
    }

    /// 进程级单例：启动时注册 WpsPpt（仅 Windows）。其余应用 C5 阶段接入。
    pub fn global() -> &'static AppRegistry {
        static R: OnceLock<AppRegistry> = OnceLock::new();
        R.get_or_init(|| {
            let reg = AppRegistry::new();
            #[cfg(windows)]
            reg.register(
                AppKind::WpsPpt,
                Box::new(crate::desktop_automation::com::ComDriver::new("Kwpp.Application")),
            );
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
    pub fn launch(&self, kind: AppKind, args: &serde_json::Value) -> Result<AppHandle, DesktopError> {
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
