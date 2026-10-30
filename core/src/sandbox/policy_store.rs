//! 沙箱策略持久化（对齐 webui `SandboxPolicy` 契约，Rust-first 离线确定性可用）
//!
//! Python 侧车不可用时，`sandbox.policy.get / defaults / update` 由 Rust 网关
//! 直接提供默认策略并落盘到 `data_dir/sandbox-policy.json`，避免 NOT_IMPLEMENTED
//! 与侧车代理阻塞 WS 事件循环。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use crate::persistence::config::Config;

/// 策略版本冲突错误码（webui `currentPolicyFromConflict` 依赖）
pub const POLICY_VERSION_CONFLICT: &str = "POLICY_VERSION_CONFLICT";

/// 默认策略（对齐 webui `SandboxPolicy` 契约）
fn default_policy() -> Value {
    json!({
        "schemaVersion": 2,
        "policyVersion": 1,
        "files": {
            "customDenyWritePaths": [],
            "recursiveDeleteBackupEnabled": true,
            "backupQuotaBytes": 3221225472u64,
        },
        "commands": {
            "requireApprovalPrefixes": [],
            "autoAllowPrefixes": [],
            "systemTools": "auto",
        },
        "network": {
            "blockAllNetwork": false,
            "allowDomains": [],
            "denyDomains": [],
        },
        "runtimes": {
            "enabled": true,
            "python": true,
            "node": true,
            "gitBash": true,
        },
    })
}

/// Windows 常见敏感路径（写入需审批），对齐 Python `builtin_deny_write_paths`
fn builtin_deny_write_paths() -> Vec<String> {
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Users"));
    [
        ".ssh",
        ".aws",
        ".kube",
        "AppData/Local/Google/Chrome/User Data/Default/Cookies",
        "AppData/Local/Microsoft/Edge/User Data/Default/Cookies",
        "Documents/桌面.ini",
        "Desktop",
    ]
    .iter()
    .filter_map(|rel| {
        let p = home.join(rel);
        p.exists().then(|| p.to_string_lossy().replace('/', "\\"))
    })
    .collect()
}

/// 沙箱策略存储（进程单例，落盘 `data_dir/sandbox-policy.json`）
pub struct SandboxPolicyStore {
    path: PathBuf,
}

impl SandboxPolicyStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join("sandbox-policy.json"),
        }
    }

    fn load(&self) -> Value {
        match std::fs::read_to_string(&self.path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|_| default_policy()),
            Err(_) => default_policy(),
        }
    }

    fn save(&self, policy: &Value) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&self.path, serde_json::to_string_pretty(policy)?)
    }

    /// 读取当前策略（未落盘时返回默认并写盘）
    pub fn read(&self) -> Value {
        let policy = self.load();
        if !self.path.exists() {
            let _ = self.save(&policy);
        }
        policy
    }

    /// 无条件保存策略（返回新的 policyVersion）
    pub fn save_policy(&self, mut policy: Value) -> Value {
        let version = policy["policyVersion"].as_u64().unwrap_or(1) + 1;
        policy["policyVersion"] = json!(version);
        let _ = self.save(&policy);
        policy
    }

    /// 版本化更新：basePolicyVersion 不匹配 → POLICY_VERSION_CONFLICT（携带 currentPolicy）
    pub fn update(&self, base_version: u64, policy: Value) -> Result<Value, String> {
        let current = self.read();
        let current_version = current["policyVersion"].as_u64().unwrap_or(1);
        if current_version != base_version {
            return Err(format!(
                "{POLICY_VERSION_CONFLICT}|{}",
                serde_json::to_string(&current).unwrap_or_else(|_| "{}".into())
            ));
        }
        Ok(self.save_policy(policy))
    }

    /// defaults 视图（builtinDenyWritePaths / runtimeTarget / runtimeVersions）
    pub fn defaults_view(&self) -> Value {
        json!({
            "builtinDenyWritePaths": builtin_deny_write_paths(),
            "runtimeTarget": Value::Null,
            "runtimeVersions": {},
        })
    }
}

fn store() -> &'static Mutex<SandboxPolicyStore> {
    static STORE: OnceLock<Mutex<SandboxPolicyStore>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(SandboxPolicyStore::new(&Config::data_dir())))
}

/// `sandbox.policy.get`
pub fn policy_get() -> Value {
    store().lock().unwrap_or_else(|e| e.into_inner()).read()
}

/// `sandbox.policy.defaults`
pub fn policy_defaults() -> Value {
    store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .defaults_view()
}

/// `sandbox.policy.update`：成功返回新策略；版本冲突返回含 `POLICY_VERSION_CONFLICT` 前缀的 Err
pub fn policy_update(base_version: u64, policy: Value) -> Result<Value, String> {
    store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .update(base_version, policy)
}

/// `sandbox.run_mode.preference.get / set` 落盘（`data_dir/run-mode.json`）
pub fn run_mode_get() -> Value {
    let path = Config::data_dir().join("run-mode.json");
    let mode = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|v| v["runMode"].as_str().map(str::to_string))
        .unwrap_or_else(|| "full".into());
    json!({ "runMode": mode, "source": "config" })
}

pub fn run_mode_set(mode: &str) -> Value {
    let mode = if mode == "safe" { "safe" } else { "full" };
    let path = Config::data_dir().join("run-mode.json");
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, json!({ "runMode": mode }).to_string());
    json!({ "runMode": mode, "source": "config" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_policy_contract() {
        let p = default_policy();
        assert_eq!(p["schemaVersion"], 2);
        assert_eq!(p["policyVersion"], 1);
        for key in ["files", "commands", "network", "runtimes"] {
            assert!(p[key].is_object(), "{key} 应为对象");
        }
        assert!(p["files"]["customDenyWritePaths"].is_array());
        assert!(p["commands"]["systemTools"].is_string());
        assert!(p["network"]["blockAllNetwork"].is_boolean());
    }

    #[test]
    fn test_update_version_conflict_carries_current() {
        let dir = std::env::temp_dir().join(format!("mgt-pstore-{}", uuid::Uuid::new_v4()));
        let s = SandboxPolicyStore::new(&dir);
        let err = s.update(99, default_policy()).unwrap_err();
        assert!(err.starts_with(POLICY_VERSION_CONFLICT), "err: {err}");
        let current = err.split('|').nth(1).unwrap_or("{}");
        let v: Value = serde_json::from_str(current).unwrap();
        assert_eq!(v["policyVersion"], 1);
    }

    #[test]
    fn test_update_success_bumps_version() {
        let dir = std::env::temp_dir().join(format!("mgt-pstore2-{}", uuid::Uuid::new_v4()));
        let s = SandboxPolicyStore::new(&dir);
        let saved = s.update(1, default_policy()).unwrap();
        assert_eq!(saved["policyVersion"], 2);
        // 再读应持久化
        let s2 = SandboxPolicyStore::new(&dir);
        assert_eq!(s2.read()["policyVersion"], 2);
    }

    /// Windows 专属：`builtin_deny_write_paths()` 按 `USERPROFILE` 拼 Windows 相对路径，
    /// 且用 `exists()` 过滤 —— 在 Unix 上 USERPROFILE 不存在、这些路径也不存在，
    /// 函数合法地返回空表，因此**没有**可在 Unix 上断言的对应行为。
    #[cfg(windows)]
    #[test]
    fn test_builtin_deny_write_paths_non_empty_on_windows() {
        let paths = builtin_deny_write_paths();
        // Windows 下至少包含用户主目录下的敏感项
        assert!(paths
            .iter()
            .any(|p| p.contains(".ssh") || p.contains("Desktop")));
    }
}
