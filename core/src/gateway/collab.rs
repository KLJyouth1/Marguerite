//! 企业协同 SSH 方法族（collab.ssh.*）
//!
//! 管理 SSH 主机台账（list/add/remove），支持真实 ssh CLI 探测与远程执行。

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::gateway::_helpers::param_str;
use crate::persistence::config::Config;
use crate::proc::SilentCommand;

/// 企业协同 SSH 主机台账条目（不存密码；仅存 host/user/port/key 路径）。
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct CollabSshEntry {
    name: String,
    host: String,
    port: u16,
    user: String,
    key: Option<String>,
}

fn collab_ssh_path() -> PathBuf {
    Config::data_dir().join("collab_ssh.json")
}

fn collab_ssh_load() -> Vec<CollabSshEntry> {
    std::fs::read_to_string(collab_ssh_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<Vec<CollabSshEntry>>(&raw).ok())
        .unwrap_or_default()
}

fn collab_ssh_save(entries: &[CollabSshEntry]) {
    let p = collab_ssh_path();
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(
        &p,
        serde_json::to_string_pretty(entries).unwrap_or_default(),
    );
}

/// collab.ssh.list：企业协同 SSH 主机台账。
pub fn collab_ssh_list() -> Value {
    let entries = collab_ssh_load();
    json!({ "ok": true, "entries": entries })
}

/// collab.ssh.add：新增/替换 SSH 主机（name 唯一）。
pub fn collab_ssh_add(params: &Value) -> Value {
    let name = param_str(params, "name", "name", "").trim().to_string();
    let host = param_str(params, "host", "host", "").trim().to_string();
    let user = param_str(params, "user", "user", "").trim().to_string();
    if name.is_empty() || host.is_empty() || user.is_empty() {
        return json!({ "ok": false, "error": "name/host/user 均不可为空" });
    }
    let port = params.get("port").and_then(Value::as_u64).unwrap_or(22) as u16;
    let key = params
        .get("key")
        .and_then(Value::as_str)
        .map(String::from)
        .filter(|s| !s.is_empty());
    let mut entries = collab_ssh_load();
    entries.retain(|e| e.name != name);
    entries.push(CollabSshEntry {
        name,
        host,
        port,
        user,
        key,
    });
    collab_ssh_save(&entries);
    collab_ssh_list()
}

/// collab.ssh.remove：删除 SSH 主机。
pub fn collab_ssh_remove(params: &Value) -> Value {
    let name = param_str(params, "name", "name", "").trim();
    if name.is_empty() {
        return json!({ "ok": false, "error": "缺少 name" });
    }
    let mut entries = collab_ssh_load();
    entries.retain(|e| e.name != name);
    collab_ssh_save(&entries);
    collab_ssh_list()
}

/// collab.ssh.probe：真实 ssh CLI 探测（BatchMode 非交互，不弹密码）。
pub async fn collab_ssh_probe(params: &Value) -> Value {
    let name = param_str(params, "name", "name", "");
    let Some(entry) = collab_ssh_load().into_iter().find(|e| e.name == name) else {
        return json!({ "ok": false, "error": "主机不存在" });
    };
    let mut cmd = tokio::process::Command::new("ssh");
    cmd.silent()
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=4")
        .arg("-p")
        .arg(entry.port.to_string())
        .arg(format!("{}@{}", entry.user, entry.host))
        .arg("echo ok");
    if let Some(k) = &entry.key {
        cmd.arg("-i").arg(k);
    }
    match cmd.output().await {
        Ok(o) if o.status.success() => json!({ "ok": true, "message": "SSH 连接成功" }),
        Ok(o) => {
            json!({ "ok": false, "error": String::from_utf8_lossy(&o.stderr).trim().to_string() })
        }
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// collab.ssh.exec：在已登记主机上真实执行命令（BatchMode 非交互，60s 超时）。
pub async fn collab_ssh_exec(params: &Value) -> Value {
    let name = param_str(params, "name", "name", "");
    let command = param_str(params, "command", "command", "");
    if command.is_empty() {
        return json!({ "ok": false, "error": "collab.ssh.exec 需要 command" });
    }
    let Some(entry) = collab_ssh_load().into_iter().find(|e| e.name == name) else {
        return json!({ "ok": false, "error": "主机不存在" });
    };
    let mut cmd = tokio::process::Command::new("ssh");
    cmd.silent()
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=8")
        .arg("-p")
        .arg(entry.port.to_string())
        .arg(format!("{}@{}", entry.user, entry.host))
        .arg(command);
    if let Some(k) = &entry.key {
        cmd.arg("-i").arg(k);
    }
    let fut = cmd.output();
    match tokio::time::timeout(std::time::Duration::from_secs(60), fut).await {
        Err(_) => json!({ "ok": false, "error": "命令执行超时（60s）" }),
        Ok(Err(e)) => json!({ "ok": false, "error": e.to_string() }),
        Ok(Ok(o)) => json!({
            "ok": o.status.success(),
            "exitCode": o.status.code(),
            "stdout": String::from_utf8_lossy(&o.stdout).trim().to_string(),
            "stderr": String::from_utf8_lossy(&o.stderr).trim().to_string(),
        }),
    }
}
