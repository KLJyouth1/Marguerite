//! 插件原生加载后端 v1：MCP 子进程。
//!
//! ## 选型结论（ADR 见 `docs/plugin-dynamicload-adr.md`）
//!
//! 在「abi_stable 原生 DLL / wasmtime / MCP 子进程」三者中选 **MCP 子进程**：
//! - **稳定**：插件在独立进程里，崩溃/死循环不会拖垮宿主；协议是行分隔 JSON-RPC，
//!   无 ABI 耦合，不受 Rust 编译器版本影响。
//! - **好用**：任意语言都能写（示例插件是纯 Python 标准库），第三方 MCP server 可直接用。
//! - **可热重载**：stop = kill 子进程，start = 重新拉起，是真热替换。
//! - **体积**：不引入 wasmtime/abi_stable 等重依赖（用户明确表示体积不是约束，
//!   但仍优先选择协议成熟、失败模式更少的方案）。
//!
//! 代价（如实登记）：插件只能提供 MCP 工具，不能 `provide` 宿主内部 Rust 服务；
//! 每个插件进程有独立内存开销。
//!
//! ## 安全边界
//! - **不经过 shell**：`Command` 直接 spawn，`args` 原样传参 → 无命令注入面。
//! - `env` 只透传显式声明的键（不继承额外凭据注入）。
//! - 每次请求都有超时（默认 20s，钳制到 1..=120s）；超时后调用方拿到结构化错误。
//! - 进程在 `Drop` 时 `start_kill`，避免退出后残留子进程。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// 插件清单里的 `mcp` 启动声明。
///
/// 字段用 camelCase（与 WebUI/JSON 生态一致），同时接受 snake_case 别名 ——
/// 历史清单可能写成 `timeout_ms`，静默回落默认值会让"我设了 8s 却是 20s"无法解释。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpLaunchSpec {
    /// 可执行文件：不含路径分隔符时按 PATH 查找；相对路径按清单目录解析。
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// 工作目录（相对清单目录）；缺省 = 清单目录。
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default = "default_timeout", alias = "timeout_ms")]
    pub timeout_ms: u64,
}

fn default_timeout() -> u64 {
    20_000
}

/// 单次请求的最大超时（防止清单写一个离谱值把 RPC 挂死）。
pub const MAX_TIMEOUT_MS: u64 = 120_000;

impl McpLaunchSpec {
    pub fn effective_timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms.clamp(1_000, MAX_TIMEOUT_MS))
    }
}

/// 一个运行中的 MCP 插件进程。
struct Proc {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    seq: u64,
    tools: Vec<Value>,
    timeout: Duration,
}

impl Proc {
    /// 发一条请求并等待**同 id** 的响应；通知与非本 id 的响应会被跳过。
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.seq += 1;
        let id = self.seq;
        let frame = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.stdin
            .write_all(format!("{frame}\n").as_bytes())
            .await
            .map_err(|e| format!("写入插件 stdin 失败: {e}"))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| format!("flush 插件 stdin 失败: {e}"))?;

        let deadline = tokio::time::Instant::now() + self.timeout;
        loop {
            let mut buf = String::new();
            let read = tokio::time::timeout_at(deadline, self.stdout.read_line(&mut buf))
                .await
                .map_err(|_| {
                    format!(
                        "等待 `{method}` 响应超时（{}ms）",
                        self.timeout.as_millis()
                    )
                })?
                .map_err(|e| format!("读取插件 stdout 失败: {e}"))?;
            if read == 0 {
                return Err("插件进程已退出（stdout EOF）".to_string());
            }
            let Ok(value) = serde_json::from_str::<Value>(buf.trim()) else {
                continue; // 非 JSON 行（日志混入）不视为错误
            };
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue; // 通知或过期响应
            }
            if let Some(err) = value.get("error") {
                return Err(format!("插件返回错误: {err}"));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let frame = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        self.stdin
            .write_all(format!("{frame}\n").as_bytes())
            .await
            .map_err(|e| format!("写入插件 stdin 失败: {e}"))?;
        self.stdin.flush().await.map_err(|e| e.to_string())
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        // 同步 kill：Drop 不能 await；start_kill 只发信号，不留僵尸等待。
        let _ = self.child.start_kill();
    }
}

static PROCS: OnceLock<tokio::sync::Mutex<HashMap<String, Proc>>> = OnceLock::new();
/// 供**同步** `plugins.coverage` 读取的最近状态快照（非实时，start/stop/call 时刷新）。
static SNAPSHOT: OnceLock<Mutex<HashMap<String, Value>>> = OnceLock::new();

fn procs() -> &'static tokio::sync::Mutex<HashMap<String, Proc>> {
    PROCS.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

fn snapshot() -> &'static Mutex<HashMap<String, Value>> {
    SNAPSHOT.get_or_init(|| Mutex::new(HashMap::new()))
}

fn set_snapshot(name: &str, value: Value) {
    let mut guard = snapshot().lock().unwrap_or_else(|e| e.into_inner());
    guard.insert(name.to_string(), value);
}

/// 解析可执行路径：含分隔符的相对路径按 `base` 解析，否则交给 PATH。
fn resolve_command(command: &str, base: &Path) -> PathBuf {
    let p = Path::new(command);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    if command.contains('/') || command.contains('\\') {
        return base.join(p);
    }
    p.to_path_buf()
}

fn resolve_cwd(cwd: Option<&str>, base: &Path) -> PathBuf {
    match cwd.map(str::trim).filter(|s| !s.is_empty()) {
        Some(rel) => {
            let p = Path::new(rel);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                base.join(p)
            }
        }
        None => base.to_path_buf(),
    }
}

/// 启动插件 MCP 进程并完成 `initialize` + `tools/list`。
///
/// `base` 是清单所在目录（相对路径基准）。
pub async fn start(
    name: &str,
    spec: &McpLaunchSpec,
    base: &Path,
) -> Result<Value, String> {
    if name.trim().is_empty() {
        return Err("插件名不可为空".to_string());
    }
    // 同名已在跑：先停，保证"启动"是幂等的替换语义。
    stop(name).await;

    let exe = resolve_command(&spec.command, base);
    let cwd = resolve_cwd(spec.cwd.as_deref(), base);
    if !cwd.is_dir() {
        return Err(format!("插件工作目录不存在: {}", cwd.display()));
    }
    let mut cmd = Command::new(&exe);
    cmd.args(&spec.args)
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动插件进程失败（{}）: {e}", exe.display()))?;
    let stdin = child.stdin.take().ok_or("插件进程 stdin 不可用")?;
    let stdout = child.stdout.take().ok_or("插件进程 stdout 不可用")?;
    let mut proc = Proc {
        child,
        stdin,
        stdout: BufReader::new(stdout),
        seq: 0,
        tools: Vec::new(),
        timeout: spec.effective_timeout(),
    };

    let init = proc
        .request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "marguerite", "version": env!("CARGO_PKG_VERSION") }
            }),
        )
        .await;
    if let Err(e) = init {
        return Err(format!("MCP initialize 失败: {e}"));
    }
    // 通知失败不应阻断（部分实现宽松处理）；但工具清单失败必须响亮。
    let _ = proc.notify("notifications/initialized", json!({})).await;
    let listed = proc.request("tools/list", json!({})).await?;
    proc.tools = listed
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let tool_names: Vec<String> = proc
        .tools
        .iter()
        .filter_map(|t| t.get("name").and_then(Value::as_str).map(str::to_string))
        .collect();
    // 展示用规范化路径：`manifests/../examples/x` 虽然能跑，但读起来像配置错误。
    let cwd_display = cwd.canonicalize().unwrap_or_else(|_| cwd.clone());

    set_snapshot(
        name,
        json!({
            "running": true,
            "tools": tool_names,
            "error": Value::Null,
            "command": spec.command,
        }),
    );
    procs().lock().await.insert(name.to_string(), proc);
    Ok(json!({
        "ok": true,
        "name": name,
        "running": true,
        "tools": tool_names,
        "command": spec.command,
        "args": spec.args,
        "cwd": cwd_display.display().to_string(),
        "timeoutMs": spec.effective_timeout().as_millis() as u64,
    }))
}

/// 停止并回收插件进程（kill + drop）。
pub async fn stop(name: &str) -> Value {
    let removed = procs().lock().await.remove(name);
    let was_running = removed.is_some();
    drop(removed);
    set_snapshot(
        name,
        json!({ "running": false, "tools": [], "error": Value::Null }),
    );
    json!({ "ok": true, "name": name, "stopped": was_running })
}

/// 运行中插件与工具（来自快照，不阻塞）。
///
/// 只返回 `running = true` 的条目：方法语义是"当前运行中的插件"。
/// 已停止的插件仍保留在快照里供 `state()`/coverage 解释「曾经启动过」。
pub fn list() -> Value {
    let guard = snapshot().lock().unwrap_or_else(|e| e.into_inner());
    let mut items: Vec<Value> = guard
        .iter()
        .filter(|(_, state)| {
            state
                .get("running")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .map(|(name, state)| {
            let mut v = state.clone();
            if let Some(obj) = v.as_object_mut() {
                obj.insert("name".to_string(), json!(name));
            }
            v
        })
        .collect();
    items.sort_by(|a, b| {
        a.get("name")
            .and_then(Value::as_str)
            .cmp(&b.get("name").and_then(Value::as_str))
    });
    json!({ "ok": true, "plugins": items, "total": items.len() })
}

/// 调用插件工具。
pub async fn call(name: &str, tool: &str, args: Value) -> Result<Value, String> {
    let mut guard = procs().lock().await;
    let proc = guard
        .get_mut(name)
        .ok_or_else(|| format!("插件 `{name}` 未运行（先 plugins.mcp.start）"))?;
    let result = proc
        .request("tools/call", json!({ "name": tool, "arguments": args }))
        .await?;
    drop(guard);
    Ok(json!({ "ok": true, "name": name, "tool": tool, "result": result }))
}

/// 某个插件是否在运行（给 coverage 用，读快照）。
pub fn is_running(name: &str) -> bool {
    snapshot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(name)
        .and_then(|v| v.get("running"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// 某个插件的运行态（快照）：`{running, tools, error}`；未知插件返回 `running=false`。
pub fn state(name: &str) -> Value {
    snapshot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(name)
        .cloned()
        .unwrap_or_else(|| json!({ "running": false, "tools": [], "error": Value::Null }))
}

/// 关闭全部插件进程（进程退出前的兜底；网关正常退出时调用）。
pub async fn stop_all() -> usize {
    let mut guard = procs().lock().await;
    let names: Vec<String> = guard.keys().cloned().collect();
    for name in &names {
        set_snapshot(name, json!({ "running": false, "tools": [], "error": Value::Null }));
    }
    guard.clear();
    names.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_parses_camel_case_and_snake_case_timeout() {
        let camel: McpLaunchSpec = serde_json::from_str(
            r#"{"command":"python","args":["server.py"],"cwd":".","timeoutMs":8000}"#,
        )
        .unwrap();
        assert_eq!(camel.timeout_ms, 8_000, "camelCase 必须生效（不能静默回默认值）");
        assert_eq!(camel.effective_timeout(), Duration::from_millis(8_000));

        let snake: McpLaunchSpec = serde_json::from_str(
            r#"{"command":"python","timeout_ms":3000}"#,
        )
        .unwrap();
        assert_eq!(snake.timeout_ms, 3_000, "snake_case 别名必须兼容");
    }

    #[test]
    fn timeout_is_clamped_to_safe_range() {
        let mk = |ms| McpLaunchSpec {
            command: "x".into(),
            args: vec![],
            env: HashMap::new(),
            cwd: None,
            timeout_ms: ms,
        };
        assert_eq!(mk(0).effective_timeout(), Duration::from_millis(1_000));
        assert_eq!(mk(10).effective_timeout(), Duration::from_millis(1_000));
        assert_eq!(
            mk(u64::MAX).effective_timeout(),
            Duration::from_millis(MAX_TIMEOUT_MS)
        );
        assert_eq!(mk(5_000).effective_timeout(), Duration::from_millis(5_000));
    }

    #[test]
    fn command_resolution_uses_base_only_for_paths() {
        let base = Path::new("C:/plugins/echo");
        assert_eq!(resolve_command("python", base), PathBuf::from("python"));
        assert_eq!(
            resolve_command("./bin/server", base),
            base.join("./bin/server")
        );
        assert_eq!(
            resolve_command("C:/abs/server.exe", base),
            PathBuf::from("C:/abs/server.exe")
        );
    }

    #[test]
    fn cwd_defaults_to_base() {
        let base = Path::new("C:/plugins/echo");
        assert_eq!(resolve_cwd(None, base), base.to_path_buf());
        assert_eq!(resolve_cwd(Some("  "), base), base.to_path_buf());
        assert_eq!(resolve_cwd(Some("sub"), base), base.join("sub"));
    }

    #[tokio::test]
    async fn missing_executable_returns_structured_error() {
        let spec = McpLaunchSpec {
            command: "definitely-not-a-real-binary-xyz".into(),
            args: vec![],
            env: HashMap::new(),
            cwd: None,
            timeout_ms: 2_000,
        };
        let err = start("ghost", &spec, &std::env::temp_dir())
            .await
            .expect_err("不存在的可执行文件必须失败");
        assert!(err.contains("启动插件进程失败"), "错误必须可读: {err}");
    }

    #[tokio::test]
    async fn start_rejects_missing_workdir() {
        let spec = McpLaunchSpec {
            command: "python".into(),
            args: vec![],
            env: HashMap::new(),
            cwd: Some("no-such-dir-xyz".into()),
            timeout_ms: 2_000,
        };
        let err = start("ghost", &spec, &std::env::temp_dir())
            .await
            .expect_err("工作目录不存在必须失败");
        assert!(err.contains("工作目录不存在"), "错误必须可读: {err}");
    }

    #[tokio::test]
    async fn call_on_stopped_plugin_is_rejected() {
        let err = call("never-started", "echo", json!({}))
            .await
            .expect_err("未运行必须失败");
        assert!(err.contains("未运行"), "错误必须可读: {err}");
    }

    #[test]
    fn list_is_stable_when_empty() {
        let v = list();
        assert_eq!(v["ok"], true);
        assert!(v["plugins"].is_array());
    }
}
