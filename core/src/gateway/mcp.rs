//! MCP 服务器管理方法族
//!
//! 提供 MCP 服务器的增删查改与探测能力，配置持久化到 config.yaml。

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Value};

use crate::gateway::_helpers::param_str;
use crate::gateway::protocol::RpcError;
use crate::persistence::config::Config;

/// 单服务器探测超时。
///
/// MCP 服务器（尤其 stdio 子进程）可能永不响应；没有这层上限，一次探测就能挂住
/// 整个 RPC，前端「探测」按钮永远转圈。串行 runner 的步骤预算、并行子步骤的
/// 单步预算都是同一类保护，这里补齐。
const MCP_PROBE_TIMEOUT_SECS: u64 = 8;

/// 工具清单缓存 TTL。
///
/// `mcp.tools.list` 会被聊天渲染路径读取（工具来源徽标），绝不能每次渲染都去连
/// 一遍服务器；60s 内复用上一次结果，过期后由下一次调用刷新。
const MCP_TOOLS_TTL_SECS: u64 = 60;

/// `mcp.servers.list`：列出已配置的 MCP 服务器。
pub fn mcp_servers_list() -> Value {
    match Config::load() {
        Ok(cfg) => json!({
            "ok": true,
            "servers": cfg.mcp.servers.iter().map(|s| json!({
                "name": s.name,
                "command": s.command,
                "args": s.args,
                "env": s.env.keys().collect::<Vec<_>>(),
                "url": s.url,
                "enabled": s.enabled,
            })).collect::<Vec<_>>(),
        }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `mcp.list`：Rust 离线回退——MCP 服务器目录（含工具计数占位）。
///
/// 此前该方法仅由 Python 侧车提供，侧车离线时返回 NOT_IMPLEMENTED，
/// 前端 MCP 面板空白。现在回退 Rust 实现：数据源与 `mcp.servers.list`
/// 同一 config.yaml；`tools` 字段如实置空（真实工具目录需 probe，
/// 由 `mcp.servers.probe` 异步提供，不做假数据）。
pub fn mcp_list() -> Value {
    let mut servers = match mcp_servers_list() {
        v if v["ok"].as_bool().unwrap_or(false) => v["servers"].clone(),
        _ => json!([]),
    };
    if let Some(arr) = servers.as_array_mut() {
        for s in arr.iter_mut() {
            s["status"] = json!("configured");
            s["tools"] = json!([]);
            s["toolCount"] = json!(0);
        }
    }
    json!({
        "ok": true,
        "source": "rust",
        "servers": servers,
        "note": "工具目录需经 mcp.servers.probe 探测获取",
    })
}

/// 读取可选字符串参数：键缺省时返回 `None`（用于区分「不修改」与「清空」）。
fn optional_str<'a>(params: &'a Value, camel: &str, snake: &str) -> Option<&'a str> {
    params
        .get(camel)
        .or_else(|| params.get(snake))
        .and_then(Value::as_str)
}

/// 解析 `args`：必须是字符串数组；键缺省返回 `None`（表示不修改）。
fn read_args(params: &Value) -> std::result::Result<Option<Vec<String>>, String> {
    match params.get("args") {
        None => Ok(None),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "args 必须是字符串数组".to_string())
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err("args 必须是字符串数组".to_string()),
    }
}

/// 解析 `env`：必须是「字符串 → 字符串」对象；键缺省返回 `None`（表示不修改）。
fn read_env(
    params: &Value,
) -> std::result::Result<Option<std::collections::HashMap<String, String>>, String> {
    match params.get("env") {
        None => Ok(None),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| {
                v.as_str()
                    .map(|s| (k.clone(), s.to_string()))
                    .ok_or_else(|| format!("env.{k} 必须是字符串"))
            })
            .collect::<std::result::Result<std::collections::HashMap<_, _>, _>>()
            .map(Some),
        Some(_) => Err("env 必须是键值对象".to_string()),
    }
}

/// 写入服务器配置的共用实现（add / update 同一路径，避免两处逻辑漂移）。
///
/// - `must_exist = true`（update）：name 必须已存在，否则返回 `MCP_SERVER_NOT_FOUND`；
/// - `args` / `env` 一旦提供就**全量替换**该服务器的配置（不是合并）；
/// - `url` / `command` 缺省表示保留原值，提供则覆盖。
fn save_server(params: &Value, must_exist: bool) -> Value {
    let name = param_str(params, "name", "name", "").trim().to_string();
    if name.is_empty() {
        return json!({ "ok": false, "error": "缺少 name" });
    }
    let args = match read_args(params) {
        Ok(v) => v,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    let env = match read_env(params) {
        Ok(v) => v,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    let mut cfg = match Config::load() {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let idx = cfg.mcp.servers.iter().position(|s| s.name == name);
    if must_exist && idx.is_none() {
        return json!({
            "ok": false,
            "error": format!("MCP 服务器不存在: {name}"),
            "code": "MCP_SERVER_NOT_FOUND",
        });
    }
    let mut srv = match idx {
        Some(i) => cfg.mcp.servers[i].clone(),
        None => crate::persistence::config::McpServerConfig {
            name: name.clone(),
            command: String::new(),
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            url: None,
            enabled: true,
        },
    };
    srv.name = name;
    if let Some(enabled) = params.get("enabled").and_then(Value::as_bool) {
        srv.enabled = enabled;
    }
    if let Some(url) = optional_str(params, "url", "url") {
        let url = url.trim();
        srv.url = if url.is_empty() {
            None
        } else {
            Some(url.to_string())
        };
    }
    if let Some(command) = optional_str(params, "command", "command") {
        srv.command = command.trim().to_string();
    }
    if let Some(args) = args {
        srv.args = args;
    }
    if let Some(env) = env {
        srv.env = env;
    }
    let has_url = srv
        .url
        .as_deref()
        .map(str::trim)
        .map(|u| !u.is_empty())
        .unwrap_or(false);
    if !has_url && srv.command.is_empty() {
        return json!({ "ok": false, "error": "url 与 command 至少提供一项" });
    }
    match idx {
        Some(i) => cfg.mcp.servers[i] = srv,
        None => cfg.mcp.servers.push(srv),
    }
    match cfg.save() {
        Ok(()) => mcp_servers_list(),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `mcp.servers.add`：新增/替换 MCP 服务器配置（name 唯一；url 或 command 至少一项）。
///
/// 同时支持 `args`（字符串数组）与 `env`（键值对象），与 `mcp.servers.update` 共用写入路径。
pub fn mcp_servers_add(params: &Value) -> Value {
    save_server(params, false)
}

/// `mcp.servers.update`：修改已存在的 MCP 服务器配置。
///
/// 入参 `{name, url?, command?, args?, env?}`；name 必须已存在。
/// `args` / `env` 提供时全量替换；返回同 `mcp.servers.list`。
pub fn mcp_servers_update(params: &Value) -> Value {
    save_server(params, true)
}

/// `mcp.servers.remove`：删除 MCP 服务器配置。
pub fn mcp_servers_remove(params: &Value) -> Value {
    let name = param_str(params, "name", "name", "").trim();
    if name.is_empty() {
        return json!({ "ok": false, "error": "缺少 name" });
    }
    match Config::load().and_then(|mut cfg| {
        cfg.mcp.servers.retain(|s| s.name != name);
        cfg.save()
    }) {
        Ok(()) => mcp_servers_list(),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// 连接一个已配置的 MCP 服务器并拉取工具名（带超时）。
///
/// 抽出共用实现，避免 `mcp.servers.probe` 与 `mcp.tools.list` 各写一遍连接逻辑
/// 而慢慢漂移（本项目已多次被「同一件事两处实现」咬到）。
async fn connect_and_list_tools(
    srv: &crate::persistence::config::McpServerConfig,
) -> std::result::Result<Vec<crate::mcp::client::McpTool>, String> {
    let url = srv.url.clone().unwrap_or_default();
    let command = srv.command.trim().to_string();
    let mut client = if !url.is_empty() {
        crate::mcp::client::McpClient::new(&url)
    } else if !command.is_empty() {
        crate::mcp::client::McpClient::new_stdio(&command, &srv.args, srv.env.clone())
    } else {
        return Err("服务器未配置 url 或 command".to_string());
    };
    let budget = Duration::from_secs(MCP_PROBE_TIMEOUT_SECS);
    let fut = async {
        client.connect().await.map_err(|e| e.to_string())?;
        client.list_tools().await.map_err(|e| e.to_string())
    };
    match tokio::time::timeout(budget, fut).await {
        Ok(Ok(tools)) => Ok(tools),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(format!(
            "探测超时（{}s 内未响应，疑似服务器无响应）",
            MCP_PROBE_TIMEOUT_SECS
        )),
    }
}

/// `mcp.servers.setEnabled`：启用/停用一个已配置的 MCP 服务器。
///
/// 停用只把 `enabled` 写成 false 并保留配置：运行期加载（工具注册 / 对话注入 /
/// 批量工具清单）会跳过它；探测与编辑仍可用，方便排查后再启用。
pub fn mcp_servers_set_enabled(params: &Value) -> Value {
    let name = param_str(params, "name", "name", "").trim();
    if name.is_empty() {
        return json!({ "ok": false, "error": "缺少 name" });
    }
    let Some(enabled) = params.get("enabled").and_then(Value::as_bool) else {
        return json!({ "ok": false, "error": "enabled 必须是布尔值", "code": "INVALID_REQUEST" });
    };
    let mut cfg = match Config::load() {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let Some(srv) = cfg.mcp.servers.iter_mut().find(|s| s.name == name) else {
        return json!({
            "ok": false,
            "error": format!("MCP 服务器不存在: {name}"),
            "code": "MCP_SERVER_NOT_FOUND",
        });
    };
    srv.enabled = enabled;
    match cfg.save() {
        Ok(()) => mcp_servers_list(),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `mcp.servers.probe`：连接 MCP 服务器并拉取工具目录。
pub async fn mcp_servers_probe(params: &Value) -> Result<Value, RpcError> {
    let name = param_str(params, "name", "name", "");
    let cfg = Config::load().map_err(|e| RpcError::new("CONFIG_LOAD_FAILED", e.to_string()))?;
    let srv = cfg
        .mcp
        .servers
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| {
            RpcError::new("MCP_SERVER_NOT_FOUND", format!("MCP 服务器不存在: {name}"))
        })?;
    let tools = connect_and_list_tools(srv).await.map_err(|e| {
        RpcError::new("MCP_PROBE_FAILED", format!("探测 {name} 失败：{e}"))
    })?;
    Ok(json!({
        "ok": true,
        "server": name,
        "url": srv.url.clone().unwrap_or_default(),
        "command": srv.command.trim(),
        "tools": tools.iter().map(|t| json!({
            "name": t.name,
            "description": t.description,
        })).collect::<Vec<_>>(),
    }))
}

/// 单个服务器的探测结果（失败被隔离：一个服务器连不上不影响其它服务器）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerToolProbe {
    pub server: String,
    pub tools: Vec<String>,
    pub error: Option<String>,
}

/// 纯函数：把各服务器探测结果聚合成 RPC 载荷。
///
/// - `tools` 去重（同名工具可能被多个服务器提供）并按名排序，供前端做来源徽标；
/// - `servers` 逐服务器如实报告成功/失败与计数——**失败不隐藏**，也不把失败
///   服务器的 `count` 填成 0 冒充「没有工具」；
/// - 无服务器时 `ok: true`（配置为空是正常状态，不是错误）。
pub fn aggregate_tool_list(results: &[ServerToolProbe], age_ms: u64, cached: bool) -> Value {
    // BTreeMap 一次给出「去重 + 按名排序 + 首次出现的来源胜出」，
    // 输出顺序与服务器配置顺序无关（前端渲染稳定，不随配置抖动）。
    let mut by_name: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for r in results {
        if r.error.is_some() {
            continue;
        }
        for t in &r.tools {
            by_name
                .entry(t.clone())
                .or_insert_with(|| r.server.clone());
        }
    }
    let tools: Vec<Value> = by_name
        .iter()
        .map(|(name, server)| json!({ "name": name, "server": server }))
        .collect();
    let total = by_name.len();
    let servers: Vec<Value> = results
        .iter()
        .map(|r| {
            json!({
                "name": r.server,
                "ok": r.error.is_none(),
                "count": if r.error.is_none() { Some(r.tools.len()) } else { None },
                "error": r.error,
            })
        })
        .collect();
    let all_ok = results.iter().all(|r| r.error.is_none());
    json!({
        "ok": all_ok,
        "tools": tools,
        "servers": servers,
        "total": total,
        "cached": cached,
        "ageMs": age_ms,
    })
}

/// 工具清单缓存（进程级；`at` 为空表示从未探测过）。
#[derive(Default)]
struct ToolsCache {
    at: Option<Instant>,
    value: Option<Value>,
}

impl ToolsCache {
    /// 取缓存值；超过 `ttl` 视为过期（返回 None）。
    fn get_at(&self, now: Instant, ttl: Duration) -> Option<Value> {
        let at = self.at?;
        if now.duration_since(at) >= ttl {
            return None;
        }
        self.value.clone()
    }
    fn put_at(&mut self, now: Instant, value: Value) {
        self.at = Some(now);
        self.value = Some(value);
    }
}

fn tools_cache() -> &'static Mutex<ToolsCache> {
    static C: OnceLock<Mutex<ToolsCache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(ToolsCache::default()))
}

/// 只读模式（`cachedOnly`）的响应构造（纯函数，便于确定性测试）。
///
/// 有陈旧缓存就给缓存并标注 `stale: true`；从未探测过则给空清单 + 说明，
/// **绝不**因为「没数据」就悄悄去探测（渲染路径不该有网络副作用）。
fn stale_view(stale: Option<Value>) -> Value {
    match stale {
        Some(mut v) => {
            if let Some(obj) = v.as_object_mut() {
                obj.insert("cached".to_string(), json!(true));
                obj.insert("stale".to_string(), json!(true));
                obj.insert("ageMs".to_string(), Value::Null);
            }
            v
        }
        None => json!({
            "ok": true,
            "tools": [],
            "servers": [],
            "total": 0,
            "cached": false,
            "stale": true,
            "ageMs": Value::Null,
            "note": "尚未探测过 MCP 工具；显式调用 mcp.tools.list（不带 cachedOnly）触发探测",
        }),
    }
}

/// `mcp.tools.list`：列出所有已配置 MCP 服务器提供的工具（带 TTL 缓存）。
///
/// 存在意义：`tools.catalog` 只覆盖**本地**工具，前端因此无法判断某个工具是否
/// 来自 MCP 服务器（聊天工具卡片的来源徽标恒不显示）。本方法补上这份清单。
///
/// 探测本身是网络/子进程 IO，故：① 逐服务器 8s 超时，绝不挂住 RPC；
/// ② 结果缓存 60s，重复调用直接复用（响应里 `cached`/`ageMs` 如实标注）。
///
/// `cached_only = true` 时**只读缓存、绝不探测**（无论是否过期）：聊天渲染路径
/// 用它做来源查询，渲染过程中不该产生网络/子进程副作用；缓存为空则如实返回
/// `stale: true` 的空清单，由显式探测（`cached_only = false`）去填。
pub async fn mcp_tools_list(cached_only: bool) -> Result<Value, RpcError> {
    let ttl = Duration::from_secs(MCP_TOOLS_TTL_SECS);
    let now = Instant::now();
    let cached = {
        let g = tools_cache().lock().unwrap_or_else(|e| e.into_inner());
        g.get_at(now, ttl).map(|v| {
            let age = g
                .at
                .map(|at| now.duration_since(at).as_millis() as u64)
                .unwrap_or(0);
            (v, age)
        })
    };
    if let Some((mut hit, age_ms)) = cached {
        if let Some(obj) = hit.as_object_mut() {
            obj.insert("cached".to_string(), json!(true));
            obj.insert("ageMs".to_string(), json!(age_ms));
        }
        return Ok(hit);
    }
    if cached_only {
        // 只读模式：给出陈旧缓存（若有）或空清单，绝不触发探测。
        let stale = tools_cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .value
            .clone();
        return Ok(stale_view(stale));
    }

    let cfg = Config::load().map_err(|e| RpcError::new("CONFIG_LOAD_FAILED", e.to_string()))?;
    let mut results: Vec<ServerToolProbe> = Vec::with_capacity(cfg.mcp.servers.len());
    let mut disabled: Vec<String> = Vec::new();
    for srv in &cfg.mcp.servers {
        // 停用的服务器不参与批量探测：它们不在运行期工具面里，
        // 连接失败也不该把整体 `ok` 拉黑。
        if !srv.enabled {
            disabled.push(srv.name.clone());
            continue;
        }
        match connect_and_list_tools(srv).await {
            Ok(tools) => results.push(ServerToolProbe {
                server: srv.name.clone(),
                tools: tools.into_iter().map(|t| t.name).collect(),
                error: None,
            }),
            Err(e) => results.push(ServerToolProbe {
                server: srv.name.clone(),
                tools: Vec::new(),
                error: Some(e),
            }),
        }
    }
    let mut value = aggregate_tool_list(&results, 0, false);
    if let Some(obj) = value.as_object_mut() {
        obj.insert("disabled".to_string(), json!(disabled));
    }
    tools_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .put_at(now, value.clone());
    Ok(value)
}

/// 清空工具清单缓存（单测隔离用）。
#[cfg(test)]
pub fn reset_tools_cache_for_test() {
    *tools_cache().lock().unwrap_or_else(|e| e.into_inner()) = ToolsCache::default();
}

#[cfg(test)]
mod server_write_tests {
    use super::*;

    #[test]
    fn read_args_rejects_non_string_entries() {
        assert_eq!(read_args(&json!({})).unwrap(), None);
        assert_eq!(
            read_args(&json!({ "args": ["--port", "8080"] })).unwrap(),
            Some(vec!["--port".to_string(), "8080".to_string()])
        );
        assert!(read_args(&json!({ "args": ["ok", 1] })).is_err());
        assert!(read_args(&json!({ "args": "--port 8080" })).is_err());
    }

    #[test]
    fn read_env_rejects_non_object_and_non_string_values() {
        assert_eq!(read_env(&json!({})).unwrap(), None);
        let env = read_env(&json!({ "env": { "API_KEY": "secret" } }))
            .unwrap()
            .unwrap();
        assert_eq!(env.get("API_KEY").map(String::as_str), Some("secret"));
        assert!(read_env(&json!({ "env": ["API_KEY=secret"] })).is_err());
        assert!(read_env(&json!({ "env": { "PORT": 8080 } })).is_err());
    }
}

#[cfg(test)]
mod tool_list_tests {
    use super::*;

    fn probe(server: &str, tools: &[&str]) -> ServerToolProbe {
        ServerToolProbe {
            server: server.to_string(),
            tools: tools.iter().map(|t| t.to_string()).collect(),
            error: None,
        }
    }

    #[test]
    fn aggregate_dedupes_and_sorts_tools_across_servers() {
        let v = aggregate_tool_list(
            &[
                probe("baota", &["baota_system_info", "shared_tool"]),
                probe("other", &["alpha", "shared_tool"]),
            ],
            0,
            false,
        );
        assert_eq!(v["ok"], true);
        assert_eq!(v["total"], 3, "同名工具只应出现一次");
        let names: Vec<&str> = v["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["alpha", "baota_system_info", "shared_tool"]);
        // 来源必须带上：前端靠它显示 MCP 徽标
        let shared = v["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "shared_tool")
            .unwrap();
        assert_eq!(shared["server"], "baota");
    }

    /// 一个服务器失败不得污染其它服务器的结果，也不得把失败伪装成「零工具」。
    #[test]
    fn aggregate_isolates_failures_and_never_fakes_zero_tools() {
        let v = aggregate_tool_list(
            &[
                probe("ok-server", &["t1"]),
                ServerToolProbe {
                    server: "dead-server".to_string(),
                    tools: Vec::new(),
                    error: Some("连接被拒绝".to_string()),
                },
            ],
            0,
            false,
        );
        assert_eq!(v["ok"], false, "存在失败服务器时整体不得报 ok");
        assert_eq!(v["total"], 1, "失败服务器的工具不得计入");
        let servers = v["servers"].as_array().unwrap();
        let dead = servers.iter().find(|s| s["name"] == "dead-server").unwrap();
        assert_eq!(dead["ok"], false);
        assert_eq!(dead["error"], "连接被拒绝");
        assert!(
            dead["count"].is_null(),
            "失败服务器的 count 必须为 null，不得填 0 冒充「没有工具」"
        );
        let alive = servers.iter().find(|s| s["name"] == "ok-server").unwrap();
        assert_eq!(alive["count"], 1);
    }

    #[test]
    fn aggregate_with_no_servers_is_ok_not_an_error() {
        let v = aggregate_tool_list(&[], 0, false);
        assert_eq!(v["ok"], true, "未配置服务器是正常状态");
        assert_eq!(v["total"], 0);
        assert_eq!(v["servers"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn cache_honours_ttl_and_reports_age() {
        let mut c = ToolsCache::default();
        let t0 = Instant::now();
        assert!(c.get_at(t0, Duration::from_secs(60)).is_none(), "未探测过应无缓存");

        c.put_at(t0, json!({ "total": 2 }));
        assert_eq!(c.get_at(t0, Duration::from_secs(60)).unwrap()["total"], 2);
        // TTL 边界：恰好到期即视为过期（避免用过期数据冒充新鲜）
        assert!(c
            .get_at(t0 + Duration::from_secs(60), Duration::from_secs(60))
            .is_none());
        assert!(c
            .get_at(t0 + Duration::from_secs(59), Duration::from_secs(60))
            .is_some());
    }

    #[test]
    fn reset_clears_cache() {
        let t0 = Instant::now();
        tools_cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .put_at(t0, json!({ "total": 1 }));
        assert!(tools_cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_at(t0, Duration::from_secs(60))
            .is_some());
        reset_tools_cache_for_test();
        assert!(tools_cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_at(Instant::now(), Duration::from_secs(60))
            .is_none());
    }

    /// 只读模式：从未探测过时必须给**空清单 + 说明**（而不是偷偷去探测），
    /// 否则聊天渲染就会产生网络/子进程副作用。
    #[test]
    fn stale_view_without_cache_is_empty_and_explains_why() {
        let v = stale_view(None);
        assert_eq!(v["ok"], true, "没探测过不是错误");
        assert_eq!(v["total"], 0);
        assert_eq!(v["cached"], false);
        assert_eq!(v["stale"], true);
        assert!(
            v["note"].as_str().is_some_and(|n| n.contains("cachedOnly")),
            "须说明如何触发真实探测"
        );
    }

    #[test]
    fn stale_view_returns_stale_cache_marked_stale() {
        let v = stale_view(Some(json!({ "total": 3, "tools": [{ "name": "x", "server": "s" }] })));
        assert_eq!(v["total"], 3, "陈旧缓存仍然有用，不该丢弃");
        assert_eq!(v["stale"], true, "须如实标注是陈旧数据");
        assert!(v["ageMs"].is_null(), "陈旧数据不得报一个假的年龄");
    }
}

