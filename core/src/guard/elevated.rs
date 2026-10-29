//! 会话级提权（P2-3）：让 WebUI 的「审批旁路 / 完全提权」开关在 Rust 侧有真实语义。
//!
//! # 为什么需要
//!
//! 前端 `useChatElevatedMode.ts` 一直 `POST /api/elevated-mode`，但 Rust 网关从未
//! 注册该路由：侧车在线时被 `/api/{*path}` 代理接住（pyengine 有实现），侧车离线
//! 时回 501，前端只能把开关回退成「不可用」。审计实测（`code-audit` P1-3）确认
//! **纯 Rust 托管下审批旁路完全不可用**，而聊天工具回路恰恰跑在 Rust 里。
//!
//! # 语义
//!
//! - `on`：只记录「提权已开启」，不改变审批口径；
//! - `bypass` / `full`：跳过**人工确认**这一步（高危工具、链式步骤的 Confirm 门）。
//!
//! 安全边界：只跳过「弹卡等人点允许」，**不放宽**模式黑名单、Plan 只读、沙箱策略与
//! 参数级判定；每次跳过都发一条 `Approval` 事件（`elevated_skip: true`）留审计轨迹。
//!
//! # 存储
//!
//! 按 **session key** 落盘到 `Config::config_dir()/elevated-mode.json`，网关重启、
//! 会话重开后保持。文件很小（会话数 × 一个短字符串），读失败一律按「没有提权」处理
//! （fail-closed，绝不因台账损坏而默认放行）。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// 合法模式白名单（与前端 `normalizeElevatedMode` / pyengine 契约同集）。
pub const ELEVATED_MODES: [&str; 3] = ["on", "bypass", "full"];

/// 解析请求里的 `mode`。
///
/// `Ok(None)` = 关闭（`off` / 空串）；`Err` = 非法值，调用方应回 400。
/// 大小写与空白宽容，因为 REST 调用方是浏览器而不是内核。
pub fn parse_mode(raw: &str) -> Result<Option<&'static str>, String> {
    let value = raw.trim().to_ascii_lowercase();
    match value.as_str() {
        "" | "off" | "none" => Ok(None),
        "on" => Ok(Some("on")),
        "bypass" => Ok(Some("bypass")),
        "full" => Ok(Some("full")),
        _ => Err(format!(
            "mode 必须是 off / on / bypass / full（收到 {raw}）"
        )),
    }
}

/// 该模式是否跳过人工审批确认。`on` 只是提权标记，不扩大自动放行面。
pub fn skips_approval(mode: Option<&str>) -> bool {
    matches!(mode, Some("bypass") | Some("full"))
}

#[derive(Debug, Default)]
struct Inner {
    loaded: bool,
    modes: BTreeMap<String, String>,
}

/// 提权模式台账。生产实例落盘；单测用 [`ElevatedStore::in_memory`] 隔离。
#[derive(Debug)]
pub struct ElevatedStore {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
}

impl ElevatedStore {
    pub fn in_memory() -> Self {
        Self {
            path: None,
            inner: Mutex::new(Inner::default()),
        }
    }

    pub fn on_disk(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            inner: Mutex::new(Inner::default()),
        }
    }

    fn with_inner<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !inner.loaded {
            inner.loaded = true;
            if let Some(path) = &self.path {
                match std::fs::read_to_string(path) {
                    Ok(raw) => match serde_json::from_str::<serde_json::Value>(&raw) {
                        Ok(value) => {
                            if let Some(map) = value.get("sessions").and_then(|v| v.as_object()) {
                                for (key, mode) in map {
                                    let Some(mode) = mode.as_str() else { continue };
                                    if ELEVATED_MODES.contains(&mode) {
                                        inner.modes.insert(key.clone(), mode.to_string());
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            // 台账损坏：按「没有提权」处理，而不是猜一个放行状态。
                            tracing::warn!(err = %e, path = %path.display(), "elevated-mode.json 解析失败，按无提权处理");
                        }
                    },
                    // 文件不存在是正常状态（从未开过提权），不告警。
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        tracing::warn!(err = %e, path = %path.display(), "elevated-mode.json 读取失败，按无提权处理");
                    }
                    Err(_) => {}
                }
            }
        }
        f(&mut inner)
    }

    /// 读取会话当前模式；未知会话 / 空 key 一律 `None`。
    pub fn mode(&self, session: &str) -> Option<String> {
        let key = session.trim();
        if key.is_empty() {
            return None;
        }
        self.with_inner(|inner| inner.modes.get(key).cloned())
    }

    /// 写入（`None` = 清除）并落盘；空 session key 静默忽略（调用方已做 400）。
    pub fn set(&self, session: &str, mode: Option<&str>) {
        let key = session.trim();
        if key.is_empty() {
            return;
        }
        self.with_inner(|inner| match mode {
            Some(mode) => {
                inner.modes.insert(key.to_string(), mode.to_string());
            }
            None => {
                inner.modes.remove(key);
            }
        });
        self.persist();
    }

    /// 该会话是否跳过人工审批确认。
    pub fn skips_approval_for(&self, session: Option<&str>) -> bool {
        let mode = session.and_then(|key| self.mode(key));
        skips_approval(mode.as_deref())
    }

    fn persist(&self) {
        let Some(path) = &self.path else { return };
        let sessions = self.with_inner(|inner| inner.modes.clone());
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_vec_pretty(&serde_json::json!({ "sessions": sessions })) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(path, bytes) {
                    tracing::warn!(
                        err = %e,
                        path = %path.display(),
                        "elevated-mode.json 落盘失败（本次仍在本进程生效）"
                    );
                }
            }
            Err(e) => tracing::warn!(err = %e, "elevated-mode.json 序列化失败"),
        }
    }

    /// 测试辅助：清空内存台账（生产全局单例在 cfg(test) 下同样走内存实现）。
    #[cfg(test)]
    pub fn clear(&self) {
        self.with_inner(|inner| inner.modes.clear());
    }
}

/// 进程级单例。
///
/// 单测编译期使用**内存**台账：`cargo test` 不应该在开发者真实
/// `~/.marguerite/elevated-mode.json` 里留状态，也不该受并发用例干扰。
pub fn global() -> &'static ElevatedStore {
    #[cfg(test)]
    {
        static STORE: OnceLock<ElevatedStore> = OnceLock::new();
        STORE.get_or_init(ElevatedStore::in_memory)
    }
    #[cfg(not(test))]
    {
        static STORE: OnceLock<ElevatedStore> = OnceLock::new();
        STORE.get_or_init(|| {
            ElevatedStore::on_disk(
                crate::persistence::config::Config::config_dir().join("elevated-mode.json"),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mode_accepts_contract_values_and_rejects_typos() {
        assert_eq!(parse_mode("off").unwrap(), None);
        assert_eq!(parse_mode("").unwrap(), None);
        assert_eq!(parse_mode("OFF").unwrap(), None);
        assert_eq!(parse_mode(" on ").unwrap(), Some("on"));
        assert_eq!(parse_mode("bypass").unwrap(), Some("bypass"));
        assert_eq!(parse_mode("Full").unwrap(), Some("full"));
        assert!(parse_mode("yolo").is_err());
        assert!(parse_mode("true").is_err());
    }

    #[test]
    fn only_bypass_and_full_skip_approval() {
        assert!(!skips_approval(None));
        assert!(!skips_approval(Some("on")));
        assert!(skips_approval(Some("bypass")));
        assert!(skips_approval(Some("full")));
    }

    #[test]
    fn set_mode_is_session_scoped_and_clearable() {
        let store = ElevatedStore::in_memory();
        assert!(!store.skips_approval_for(Some("agent:a")));
        store.set("agent:a", Some("bypass"));
        store.set("agent:b", Some("on"));
        assert!(store.skips_approval_for(Some("agent:a")));
        assert!(!store.skips_approval_for(Some("agent:b")));
        assert!(!store.skips_approval_for(None));
        assert!(!store.skips_approval_for(Some("agent:c")));

        store.set("agent:a", None);
        assert!(!store.skips_approval_for(Some("agent:a")));
        store.set("   ", Some("full"));
        assert!(!store.skips_approval_for(Some("")));
    }

    #[test]
    fn on_disk_store_round_trips_and_fails_closed_on_corrupt_json() {
        let dir = std::env::temp_dir().join(format!(
            "marguerite-elevated-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let path = dir.join("elevated-mode.json");
        std::fs::create_dir_all(&dir).unwrap();

        let store = ElevatedStore::on_disk(path.clone());
        store.set("agent:persist", Some("bypass"));
        assert!(path.is_file(), "set 后必须落盘");

        // 新实例（模拟网关重启）应从同一文件恢复。
        let reopened = ElevatedStore::on_disk(path.clone());
        assert!(reopened.skips_approval_for(Some("agent:persist")));

        // 损坏台账 → fail-closed（绝不因为读不出来就默认放行）。
        std::fs::write(&path, b"{not json").unwrap();
        let corrupt = ElevatedStore::on_disk(path);
        assert!(!corrupt.skips_approval_for(Some("agent:persist")));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
