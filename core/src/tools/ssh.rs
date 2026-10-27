//! 远程 SSH 工具（P2-10）：把「已登记主机」暴露成 Agent 可直接调用的原子工具。
//!
//! ## 为什么需要工具层
//!
//! `collab.ssh.*` 是前端 RPC：Agent 在长任务里遇到「远程构建 / 看远端日志 / 部署」
//! 时无法自行发起，只能让用户手动去 SSH 协同页复制粘贴。这里复用同一份主机台账
//! 与同一套 BatchMode 非交互执行逻辑，**不新增凭据存储**，只是把已有能力接到工具面。
//!
//! ## 诚实边界
//!
//! - 只向模型暴露 `name/host/user/port`，不返回私钥路径；
//! - 主机必须先在「SSH 协同」页登记，未登记一律显式报错，不猜默认主机；
//! - 结果保留 `exitCode/stdout/stderr`，超时与失败都不静默。

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use serde_json::{json, Value};

use crate::agent::tool_cycle::ToolResult;
use crate::mcp::registry::ToolRegistry;

fn ok(data: String) -> Result<ToolResult> {
    Ok(ToolResult::Success {
        data,
        metadata: HashMap::new(),
    })
}

fn err(message: String, code: &str) -> Result<ToolResult> {
    Ok(ToolResult::Error {
        message,
        code: code.to_string(),
    })
}

/// 工具入参（全字符串）→ RPC 入参（JSON），交给 `collab.ssh.exec` 复用同一套解析。
fn args_to_value(args: &HashMap<String, String>) -> Value {
    let mut map = serde_json::Map::new();
    for (k, v) in args {
        map.insert(k.clone(), Value::String(v.clone()));
    }
    Value::Object(map)
}

/// 注册远程 SSH 工具（`ssh_hosts` / `ssh_exec`）。
pub fn register_ssh_tools(registry: &mut ToolRegistry) {
    registry.register_local(crate::tools::make_local_tool(
        "ssh_hosts",
        "列出已登记的 SSH 主机（仅名称/主机/用户/端口，不含密钥路径）。远程操作前先用它确认可用主机名。无参数。",
        json!({"type":"object","properties":{}}),
        Arc::new(|_args| {
            Box::pin(async move {
                let list = crate::gateway::collab::collab_ssh_list();
                let entries = list
                    .get("entries")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if entries.is_empty() {
                    return ok(
                        "尚未登记 SSH 主机。请先在「SSH 协同」页添加主机（name/host/port/user/key）。"
                            .to_string(),
                    );
                }
                let lines: Vec<String> = entries
                    .iter()
                    .filter_map(|e| {
                        let name = e.get("name")?.as_str()?;
                        let host = e.get("host")?.as_str()?;
                        let user = e.get("user")?.as_str()?;
                        let port = e.get("port").and_then(Value::as_u64).unwrap_or(22);
                        Some(format!("- {name}: {user}@{host}:{port}"))
                    })
                    .collect();
                ok(format!(
                    "已登记 SSH 主机（{} 台）:\n{}",
                    lines.len(),
                    lines.join("\n")
                ))
            })
        }),
    ));

    registry.register_local(crate::tools::make_local_tool(
        "ssh_exec",
        "在已登记的 SSH 主机上执行命令（BatchMode 非交互，不会弹密码）。适合远程构建、查看日志、部署与巡检。\
         参数: name(主机名，必填，见 ssh_hosts), command(必填), timeoutSec(可选，5-1800)。\
         返回 exitCode / stdout / stderr；失败与超时都会显式说明。",
        json!({"type":"object","properties":{
            "name":{"type":"string","description":"已登记的主机名（先用 ssh_hosts 查看）"},
            "command":{"type":"string","description":"在远端执行的 shell 命令"},
            "timeoutSec":{"type":"integer","description":"超时秒数，默认按标准预算（300s），范围 5-1800"}
        },"required":["name","command"]}),
        Arc::new(|args| {
            let payload = args_to_value(args);
            Box::pin(async move {
                let name = payload
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if name.is_empty() {
                    return err("ssh_exec 需要非空 name（先用 ssh_hosts 查看可用主机）".into(), "INVALID_PARAMS");
                }
                let command = payload
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if command.is_empty() {
                    return err("ssh_exec 需要非空 command".into(), "INVALID_PARAMS");
                }
                let res = crate::gateway::collab::collab_ssh_exec(&payload).await;
                if res.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                    let exit = res
                        .get("exitCode")
                        .and_then(Value::as_i64)
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let stdout = res.get("stdout").and_then(Value::as_str).unwrap_or("");
                    let stderr = res.get("stderr").and_then(Value::as_str).unwrap_or("");
                    return ok(format!(
                        "SSH [{name}] 执行成功 (exit={exit})\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
                    ));
                }
                let reason = res
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("未知错误");
                err(format!("SSH [{name}] 执行失败：{reason}"), "SSH_EXEC_FAILED")
            })
        }),
    ));
}
