//! 插件原生加载 RPC（MCP 子进程后端）。
//!
//! 方法族（全部 async-only，必须走 WS RPC）：
//! - `plugins.mcp.start`：按清单 `mcp` 声明启动插件进程（完成 initialize + tools/list）
//! - `plugins.mcp.stop`：kill 并回收进程
//! - `plugins.mcp.list`：清单声明 + 运行态快照（快照，不阻塞）
//! - `plugins.mcp.call`：调用插件工具（`tools/call`）
//!
//! 错误一律 `ok:false + error + code` 三段式；参数校验先于任何进程操作。

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::gateway::market::plugin_manifest_dir;
use crate::plugins::manifest::PluginManifest;
use crate::plugins::mcp_launch::{self, McpLaunchSpec};

fn invalid(msg: &str) -> Value {
    json!({ "ok": false, "error": msg, "code": "INVALID_REQUEST" })
}

fn param_name(params: &Value) -> Option<&str> {
    params
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// 在清单目录里按 `name` 找到清单（返回清单 + 所在目录）。
fn find_manifest(name: &str) -> Result<(PluginManifest, PathBuf), Value> {
    let dir = plugin_manifest_dir().ok_or_else(|| {
        json!({
            "ok": false,
            "error": "插件清单目录未解析到（检查 MARGUERITE_PLUGINS_DIR 或安装布局）",
            "code": "PLUGIN_MANIFESTS_UNAVAILABLE",
        })
    })?;
    let entries = std::fs::read_dir(&dir).map_err(|e| {
        json!({
            "ok": false,
            "error": format!("读取清单目录失败（{}）: {e}", dir.display()),
            "code": "PLUGIN_MANIFESTS_UNREADABLE",
        })
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_manifest = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(".plugin.json"))
            .unwrap_or(false);
        if !is_manifest {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<PluginManifest>(&text) else {
            continue;
        };
        if manifest.name == name {
            return Ok((manifest, path));
        }
    }
    Err(json!({
        "ok": false,
        "error": format!("插件 `{name}` 未在清单中声明"),
        "code": "PLUGIN_NOT_FOUND",
    }))
}

/// `plugins.mcp.start`
///
/// 允许 `{ name, command, args, env, cwd, timeoutMs }` 直接试跑（未写清单也能验证），
/// 此时不要求清单存在 —— 这是运维诊断口，不是安装入口。
pub async fn start(params: &Value) -> Value {
    let Some(name) = param_name(params) else {
        return invalid("name 不可为空");
    };
    let (spec, base) = if let Some(command) = params
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let spec = json!({
            "command": command,
            "args": params.get("args").cloned().unwrap_or(json!([])),
            "env": params.get("env").cloned().unwrap_or(json!({})),
            "cwd": params.get("cwd").cloned().unwrap_or(Value::Null),
            "timeoutMs": params.get("timeoutMs").cloned().unwrap_or(json!(20_000)),
        });
        match serde_json::from_value::<McpLaunchSpec>(spec) {
            Ok(s) => {
                let base = plugin_manifest_dir()
                    .and_then(|p| p.parent().map(PathBuf::from))
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                (s, base)
            }
            Err(e) => return invalid(&format!("启动声明不合法: {e}")),
        }
    } else {
        match find_manifest(name) {
            Ok((manifest, path)) => {
                let Some(spec) = manifest.mcp.clone() else {
                    return json!({
                        "ok": false,
                        "error": format!("插件 `{name}` 未声明 mcp 启动入口（清单里没有 mcp 字段）"),
                        "code": "PLUGIN_MCP_NOT_DECLARED",
                    });
                };
                let base = path.parent().map(PathBuf::from).unwrap_or_default();
                (spec, base)
            }
            Err(v) => return v,
        }
    };

    match mcp_launch::start(name, &spec, &base).await {
        Ok(v) => v,
        Err(e) => json!({
            "ok": false,
            "error": e,
            "code": "PLUGIN_MCP_START_FAILED",
        }),
    }
}

/// `plugins.mcp.stop`
pub async fn stop(params: &Value) -> Value {
    let Some(name) = param_name(params) else {
        return invalid("name 不可为空");
    };
    mcp_launch::stop(name).await
}

/// `plugins.mcp.list`
///
/// 返回「清单声明」与「运行态快照」的并集，而不是只回运行中条目：
/// - 清单里声明了 `mcp` 的插件即使未启动也列出（`declared/command/timeoutMs/running=false`），
///   否则前端无法对「已声明但未运行」的进程发起 start；
/// - 仅经 `plugins.mcp.start` 试跑、未写进清单的进程也列出（`declared=false`）；
/// - `tools` / `error` 来自最近一次 start/stop 快照（stop 后 tools 为空）。
pub fn list() -> Value {
    use std::collections::BTreeMap;

    let mut by_name: BTreeMap<String, Value> = BTreeMap::new();
    let running = mcp_launch::list();
    for mut item in running
        .get("plugins")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let Some(name) = item.get("name").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if let Some(obj) = item.as_object_mut() {
            obj.insert("declared".to_string(), json!(false));
            obj.entry("timeoutMs".to_string()).or_insert(Value::Null);
        }
        by_name.insert(name, item);
    }

    match plugin_manifest_dir() {
        Some(dir) => match PluginManifest::load_dir(&dir) {
            Ok(manifests) => {
                for manifest in manifests {
                    let Some(spec) = manifest.mcp.as_ref() else {
                        continue;
                    };
                    let name = manifest.name.clone();
                    let state = mcp_launch::state(&name);
                    by_name.insert(
                        name.clone(),
                        json!({
                            "name": name,
                            "declared": true,
                            "command": spec.command,
                            "args": spec.args,
                            "timeoutMs": spec.effective_timeout().as_millis() as u64,
                            "running": state.get("running").and_then(Value::as_bool).unwrap_or(false),
                            "tools": state.get("tools").cloned().unwrap_or(json!([])),
                            "error": state.get("error").cloned().unwrap_or(Value::Null),
                        }),
                    );
                }
            }
            Err(e) => {
                // 清单目录存在但装载失败：运行态仍要返回，失败原因不得静默。
                tracing::warn!(dir = %dir.display(), error = %e, "plugins.mcp.list: 清单装载失败，仅返回运行态快照");
            }
        },
        None => {
            tracing::warn!("plugins.mcp.list: 插件清单目录未解析到，仅返回运行态快照");
        }
    }

    let items: Vec<Value> = by_name.into_values().collect();
    let total = items.len();
    json!({ "ok": true, "plugins": items, "total": total })
}

/// `plugins.mcp.call`
pub async fn call(params: &Value) -> Value {
    let Some(name) = param_name(params) else {
        return invalid("name 不可为空");
    };
    let Some(tool) = params
        .get("tool")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return invalid("tool 不可为空");
    };
    // 兼容两种写法：MCP 原生 `arguments`（调用方直觉）与内部简写 `args`。
    let args = params
        .get("arguments")
        .or_else(|| params.get("args"))
        .cloned()
        .unwrap_or(json!({}));
    if !args.is_object() {
        return invalid("arguments/args 必须是对象");
    }
    match mcp_launch::call(name, tool, args).await {
        Ok(v) => v,
        Err(e) => json!({
            "ok": false,
            "error": e,
            "code": "PLUGIN_MCP_CALL_FAILED",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_name_and_tool() {
        assert_eq!(invalid("x")["code"], "INVALID_REQUEST");
        // 同步校验路径：name 为空不应触碰任何进程/磁盘
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let v = rt.block_on(call(&json!({ "name": "", "tool": "x" })));
        assert_eq!(v["code"], "INVALID_REQUEST");
        let v = rt.block_on(call(&json!({ "name": "p", "tool": "" })));
        assert_eq!(v["code"], "INVALID_REQUEST");
        let v = rt.block_on(call(&json!({ "name": "p", "tool": "t", "arguments": [] })));
        assert_eq!(v["code"], "INVALID_REQUEST");
    }

    #[test]
    fn list_is_available_without_running_processes() {
        let v = list();
        assert_eq!(v["ok"], true);
        assert!(v["plugins"].is_array());
    }
}
