//! 宝塔面板凭据方法族（baota.credentials.*）
//!
//! 管理宝塔面板地址/账号/密码的凭据存储与读取，供 btpanel-mcp 自动登录。

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::persistence::config::Config;

/// 宝塔面板凭据文件路径：`~/.marguerite/btpanel-credentials.json`
fn baota_credentials_path() -> PathBuf {
    Config::config_dir().join("btpanel-credentials.json")
}

/// `baota.credentials.set`：保存面板地址/账号/密码到凭据文件。
///
/// btpanel-mcp 启动或首次调用工具时读取该文件并自动登录，WebUI 由此实现
/// 「调用前配置宝塔地址与账号密码」。密码不回读（get 不暴露）。
pub fn baota_credentials_set(params: &Value) -> Value {
    let base_url = params
        .get("base_url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let username = params
        .get("username")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let password = params.get("password").and_then(Value::as_str).unwrap_or("");
    if base_url.is_empty() || username.is_empty() || password.is_empty() {
        return json!({ "ok": false, "msg": "base_url、username、password 均不能为空" });
    }
    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        return json!({ "ok": false, "msg": "base_url 需以 http:// 或 https:// 开头" });
    }
    let path = baota_credentials_path();
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            return json!({ "ok": false, "msg": format!("创建凭据目录失败: {e}") });
        }
    }
    let data = json!({
        "base_url": base_url,
        "username": username,
        "password": password,
        "cookies": {},
        "win_token": "",
        "linux_token": "",
        "linux_time": "",
    });
    // R13：原子写。凭据文件半截 = 解析失败 = 用户以为"保存成功"却要重填账号密码。
    match crate::persistence::write_json_atomic(
        &path,
        &serde_json::to_string(&data).unwrap_or_default(),
    ) {
        Ok(_) => json!({
            "ok": true,
            "msg": "凭据已保存，宝塔工具调用时将自动登录",
            "path": path.to_string_lossy(),
        }),
        Err(e) => json!({ "ok": false, "msg": format!("凭据保存失败: {e}") }),
    }
}

/// `baota.credentials.get`：凭据状态（不含密码），供 UI 展示是否已配置。
pub fn baota_credentials_get() -> Value {
    let path = baota_credentials_path();
    if !path.exists() {
        return json!({
            "configured": false,
            "base_url": "",
            "has_username": false,
            "path": path.to_string_lossy(),
        });
    }
    let v = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .unwrap_or(Value::Null);
    json!({
        "configured": true,
        "base_url": v.get("base_url").and_then(Value::as_str).unwrap_or(""),
        "has_username": !v.get("username").and_then(Value::as_str).unwrap_or("").is_empty(),
        "path": path.to_string_lossy(),
    })
}

/// `baota.credentials.clear`：删除凭据文件（登出）。
pub fn baota_credentials_clear() -> Value {
    let path = baota_credentials_path();
    match std::fs::remove_file(&path) {
        Ok(_) => json!({ "ok": true, "msg": "宝塔凭据已清除" }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            json!({ "ok": true, "msg": "宝塔凭据不存在（无需清除）" })
        }
        Err(e) => json!({ "ok": false, "msg": format!("清除凭据失败: {e}") }),
    }
}
