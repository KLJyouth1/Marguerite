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

/// collab.ssh.exec：在已登记主机上真实执行命令（BatchMode 非交互；
/// 超时默认取自适应预算 Standard 档，调用方可传 `timeoutSec` 覆盖）。
pub async fn collab_ssh_exec(params: &Value) -> Value {
    let name = param_str(params, "name", "name", "");
    let command = param_str(params, "command", "command", "");
    if command.is_empty() {
        return json!({ "ok": false, "error": "collab.ssh.exec 需要 command" });
    }
    let Some(entry) = collab_ssh_load().into_iter().find(|e| e.name == name) else {
        return json!({ "ok": false, "error": "主机不存在" });
    };
    exec_ssh_entry(&entry, command, params).await
}

/// 执行单个台账条目（审批 → spawn ssh）。抽成独立函数便于单测验证
/// 「审批未通过时绝不 spawn」——测试不必写真实用户数据目录的台账文件。
async fn exec_ssh_entry(entry: &CollabSshEntry, command: &str, params: &Value) -> Value {
    // W3.1：远程命令一律先过审批 —— 与工具回路共用同一条链路
    // （ApprovalGate → 全局注册表 → 控制台审批卡 → 裁决）。无宿主 / 拒绝 /
    // 超时都 fail-closed：**审批通过之前绝不 spawn ssh**。
    let mut approval_args = std::collections::HashMap::new();
    approval_args.insert("host".to_string(), entry.host.clone());
    approval_args.insert("user".to_string(), entry.user.clone());
    approval_args.insert("command".to_string(), command.to_string());
    if let Some(blocked) = crate::agent::tool_cycle::suspend_for_remote_command(
        crate::guard::gateway::agent_gate(),
        crate::guard::gateway::agent_approval_host_present(),
        None,
        "collab.ssh.exec",
        &approval_args,
    )
    .await
    {
        return match blocked {
            crate::agent::tool_cycle::ToolResult::Error { message, code } => {
                json!({ "ok": false, "code": code, "error": message })
            }
            _ => json!({
                "ok": false,
                "code": "APPROVAL_BLOCKED",
                "error": "审批未通过，未执行远程命令",
            }),
        };
    }
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
    // 超时可由调用方声明（模型给出的 `timeoutSec` / `timeout_sec`），
    // 未声明时按自适应预算的 Standard 档（300s）；仍钳制在 5s..1800s。
    // 历史值固定 60s：远程编译/打包类命令必然超时。
    let declared = ["timeoutSec", "timeout_sec", "timeout"]
        .iter()
        .find_map(|key| params.get(*key))
        .and_then(|value| match value {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.trim().parse::<u64>().ok(),
            _ => None,
        });
    let secs = declared
        .unwrap_or_else(|| {
            crate::agent::budget_policy::resolve("[[budget:standard]]", 1, 0)
                .step_timeout
                .as_secs()
        })
        .clamp(5, crate::agent::budget_policy::MAX_STEP_SECS);
    match tokio::time::timeout(std::time::Duration::from_secs(secs), fut).await {
        Err(_) => json!({ "ok": false, "error": format!("命令执行超时（{secs}s）") }),
        Ok(Err(e)) => json!({ "ok": false, "error": e.to_string() }),
        Ok(Ok(o)) => json!({
            "ok": o.status.success(),
            "exitCode": o.status.code(),
            "stdout": String::from_utf8_lossy(&o.stdout).trim().to_string(),
            "stderr": String::from_utf8_lossy(&o.stderr).trim().to_string(),
        }),
    }
}

#[cfg(test)]
mod approval_guard_tests {
    use super::*;
    use serde_json::json;

    fn fake_entry() -> CollabSshEntry {
        CollabSshEntry {
            name: "probe".into(),
            host: "127.0.0.1".into(),
            port: 22,
            user: "nobody".into(),
            key: None,
        }
    }

    /// W3.1：远程命令在批准之前**绝不 spawn ssh**。
    /// 单元测试环境没有审批宿主：要么直接 fail-closed（APPROVAL_BUSY），
    /// 要么走 10ms 短门超时（APPROVAL_TIMEOUT，见 `agent_gate()` 的 cfg(test) 分支）。
    #[tokio::test]
    async fn remote_exec_is_blocked_before_ssh_spawn_without_approval() {
        let out = exec_ssh_entry(&fake_entry(), "echo should-not-run", &json!({})).await;
        assert_eq!(out["ok"], false, "{out}");
        let code = out["code"].as_str().unwrap_or("");
        assert!(
            ["APPROVAL_BUSY", "APPROVAL_TIMEOUT", "APPROVAL_DENIED", "APPROVAL_PENDING"]
                .contains(&code),
            "必须由审批分支拒绝，实际: {out}"
        );
        // 批准前不得执行：没有 ssh 的 stdout/exitCode，也没有「命令执行超时」文案。
        assert!(out.get("stdout").is_none(), "{out}");
        assert!(out.get("exitCode").is_none(), "{out}");
        assert!(
            !out["error"].as_str().unwrap_or("").contains("命令执行超时"),
            "不应走到 spawn 后的超时分支: {out}"
        );
    }
}
