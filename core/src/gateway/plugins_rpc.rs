//! 插件热启停与覆盖率 RPC。
//!
//! 方法族：
//! - `plugins.coverage`：只读统一清单（注册表 + managed + manifests + patches + 红线）
//! - `plugins.setEnabled`：热启用/停用（持久化 + 重跑 DI，失败回滚）
//! - `plugins.reload`：热重载单个插件
//!
//! 错误统一 `ok:false + error + code`；参数错误先于任何运行时访问被拒绝。

use serde_json::{json, Value};

use crate::gateway::plugins_runtime;

fn invalid(msg: &str) -> Value {
    json!({ "ok": false, "error": msg, "code": "INVALID_REQUEST" })
}

/// 解析 `name`：去空白后非空。
fn parse_name(params: &Value) -> Option<&str> {
    params
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// `plugins.coverage`：统一覆盖率（只读，不触发装配/写盘）。
pub fn plugins_coverage() -> Value {
    let rt = plugins_runtime::global()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    rt.coverage()
}

/// `plugins.setEnabled`：热启用/停用。
pub fn plugins_set_enabled(params: &Value) -> Value {
    let Some(name) = parse_name(params) else {
        return invalid("name 不可为空");
    };
    let Some(enabled) = params.get("enabled").and_then(Value::as_bool) else {
        return invalid("enabled 必须是布尔值");
    };
    let mut rt = plugins_runtime::global()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match rt.set_enabled(name, enabled) {
        Ok(outcome) => outcome.to_json(),
        Err(e) => json!({ "ok": false, "error": e.message(), "code": e.code() }),
    }
}

/// `plugins.reload`：热重载单个插件（不改变启用状态）。
pub fn plugins_reload(params: &Value) -> Value {
    let Some(name) = parse_name(params) else {
        return invalid("name 不可为空");
    };
    let mut rt = plugins_runtime::global()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match rt.reload(name) {
        Ok(outcome) => outcome.to_json(),
        Err(e) => json!({ "ok": false, "error": e.message(), "code": e.code() }),
    }
}

/// 插件名安全校验：只允许单段文件名（拒绝路径穿越）。
fn safe_plugin_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// 目标清单是否可写（只读安装包资源会返回 false，UI 据此禁用删除）。
fn manifest_writable(path: &std::path::Path) -> bool {
    match std::fs::OpenOptions::new().write(true).open(path) {
        Ok(_) => true,
        Err(_) => {
            // 文件不存在时看目录是否可写（新建场景）
            path.parent()
                .map(|dir| {
                    let probe = dir.join(".marguerite-write-probe");
                    let ok = std::fs::write(&probe, b"probe").is_ok();
                    let _ = std::fs::remove_file(&probe);
                    ok
                })
                .unwrap_or(false)
        }
    }
}

/// `plugins.detail`：单个插件的完整详情（清单原文 + 注册表状态 + 适配映射）。
pub fn plugins_detail(params: &Value) -> Value {
    let Some(name) = parse_name(params) else {
        return invalid("name 不可为空");
    };
    if !safe_plugin_name(name) {
        return invalid("name 含非法路径字符");
    }
    let manifest_dir = crate::gateway::market::plugin_manifest_dir();
    let manifest_path = manifest_dir
        .as_ref()
        .map(|dir| dir.join(format!("{name}.plugin.json")));
    let raw = manifest_path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok());
    let manifest = raw
        .as_ref()
        .and_then(|text| crate::plugins::manifest::PluginManifest::from_json(text).ok());
    let rt = plugins_runtime::global()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let entry = rt.loader.registry.entries.get(name);
    if raw.is_none() && entry.is_none() {
        return json!({
            "ok": false,
            "code": "PLUGIN_NOT_FOUND",
            "error": format!("插件不存在：{name}"),
        });
    }
    let manifest_json = manifest
        .as_ref()
        .and_then(|m| serde_json::to_value(m).ok())
        .unwrap_or(Value::Null);
    let diagnostics = manifest
        .as_ref()
        .map(|m| {
            m.validate()
                .into_iter()
                .map(|d| json!({ "code": d.code, "message": d.message }))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "ok": true,
        "name": name,
        "manifestPath": manifest_path.as_ref().map(|p| p.display().to_string()),
        "manifestDir": manifest_dir.as_ref().map(|d| d.display().to_string()),
        "writable": manifest_path.as_ref().map(|p| manifest_writable(p)).unwrap_or(false),
        "hasManifest": raw.is_some(),
        "raw": raw,
        "manifest": manifest_json,
        "diagnostics": diagnostics,
        "registry": entry.map(|e| json!({
            "enabled": e.enabled,
            "declarative": e.declarative,
            "entry": e.entry.display().to_string(),
            "dependsOn": e.depends_on,
            "source": e.source,
        })),
        "adaptation": crate::plugins::adaptation::detail_for(name),
    })
}

/// `plugins.delete`：删除插件清单 + 注册表条目（带依赖检查与失败回滚）。
pub fn plugins_delete(params: &Value) -> Value {
    let Some(name) = parse_name(params) else {
        return invalid("name 不可为空");
    };
    if !safe_plugin_name(name) {
        return invalid("name 含非法路径字符");
    }
    let manifest_dir = crate::gateway::market::plugin_manifest_dir();
    let mut rt = plugins_runtime::global()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match rt.remove(name, manifest_dir.as_deref()) {
        Ok(outcome) => json!({
            "ok": true,
            "name": outcome.name,
            "removedManifest": outcome.removed_manifest,
            "removedRegistry": outcome.removed_registry,
            "order": outcome.order,
            "note": "已删除清单与注册表条目并重跑 DI；如清单位于安装包资源目录，升级后可能重新出现",
        }),
        Err(e) => json!({ "ok": false, "error": e.message(), "code": e.code() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 参数校验必须在触碰全局运行时之前完成（否则单测会读用户真实注册表）。
    #[test]
    fn set_enabled_rejects_empty_name_and_missing_bool() {
        let v = plugins_set_enabled(&json!({ "name": "  ", "enabled": true }));
        assert_eq!(v["code"], "INVALID_REQUEST");
        let v = plugins_set_enabled(&json!({ "name": "x" }));
        assert_eq!(v["code"], "INVALID_REQUEST");
        let v = plugins_set_enabled(&json!({ "name": "x", "enabled": "yes" }));
        assert_eq!(v["code"], "INVALID_REQUEST");
    }

    #[test]
    fn reload_rejects_empty_name() {
        let v = plugins_reload(&json!({ "name": "" }));
        assert_eq!(v["code"], "INVALID_REQUEST");
    }
}
