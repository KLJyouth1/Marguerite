//! ACP 会话路由层（差距② 薄壳）。
//!
//! `AcpSession::handle(msg)` 把原始消息经 `protocol::parse` 规范化后路由到内部 dispatch：
//!
//! - `ping`         → 健康检查探针
//! - `session.info` → 会话元信息（已注册方法 / 回环标志）
//! - `agent.invoke` → 经注入的 `AgentInvoker` 调用 agent（真实回环 sidecar 调用由上层提供）
//!
//! 约束（nexustar-api 设计 §7）：
//! - 仅回环：ACP 经 sidecar 暴露，禁止突破 127.0.0.1。
//! - ACP 仅做转发，不内联 agent 运行逻辑。

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use super::protocol::{
    make_error, make_result, parse, AcpError, AcpRequest, INTERNAL_ERROR, INVALID_PARAMS,
    INVALID_REQUEST, METHOD_NOT_FOUND,
};
use crate::agent::tool_cycle::ToolResult;
use crate::mcp::registry::ToolRegistry;
use crate::memory::recall::{MemoryBackend, MemoryCategory};

/// agent 调用转发接口（由上层 sidecar 真实实现；测试用 mock 取代）。
#[async_trait]
pub trait AgentInvoker: Send + Sync {
    /// 调用 agent 并返回结果文本。
    async fn invoke(&self, prompt: &str) -> Result<String, String>;

    /// 引擎健康/能力快照（供 sidecar 可观测性探针）。默认返回最小健康包。
    fn health(&self) -> Value {
        json!({"status": "ok"})
    }
}

/// 一个 ACP 会话：把 JSON-RPC 方法路由到内部 dispatch。
pub struct AcpSession {
    /// 会话 id
    pub session_id: String,
    /// agent 调用转发器（真实 sidecar 调用由上层注入）
    invoker: Arc<dyn AgentInvoker>,
    /// 工具注册表（混合检索引擎 ACP `tools.*` 方法复用；无则为 None）
    tool_registry: Option<Arc<ToolRegistry>>,
    /// 记忆后端（claude-mem 融合：`memory.search` / `memory.remember`；无则为 None）
    memory: Option<Arc<dyn MemoryBackend>>,
    /// 回环标志（文档约束：ACP 仅经 127.0.0.1 暴露）
    loopback_only: bool,
    /// sidecar 地址（仅文档/展示用）
    sidecar: String,
}

impl AcpSession {
    /// 构造会话（无工具回路，仅 agent 转发）。
    pub fn new(session_id: impl Into<String>, invoker: Arc<dyn AgentInvoker>) -> Self {
        Self {
            session_id: session_id.into(),
            invoker,
            tool_registry: None,
            memory: None,
            loopback_only: true,
            sidecar: "127.0.0.1:18181".to_string(),
        }
    }

    /// 构造会话并注入工具注册表（启用 混合检索引擎 ACP `tools.list` / `tools.invoke`）。
    pub fn new_with_tools(
        session_id: impl Into<String>,
        invoker: Arc<dyn AgentInvoker>,
        tool_registry: Arc<ToolRegistry>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            invoker,
            tool_registry: Some(tool_registry),
            memory: None,
            loopback_only: true,
            sidecar: "127.0.0.1:18181".to_string(),
        }
    }

    /// 构造会话并注入记忆后端（启用 claude-mem ACP `memory.search` / `memory.remember`）。
    pub fn new_with_memory(
        session_id: impl Into<String>,
        invoker: Arc<dyn AgentInvoker>,
        memory: Arc<dyn MemoryBackend>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            invoker,
            tool_registry: None,
            memory: Some(memory),
            loopback_only: true,
            sidecar: "127.0.0.1:18181".to_string(),
        }
    }

    /// 构造全功能会话（工具回路 + 记忆后端同时注入）。
    pub fn new_full(
        session_id: impl Into<String>,
        invoker: Arc<dyn AgentInvoker>,
        tool_registry: Arc<ToolRegistry>,
        memory: Arc<dyn MemoryBackend>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            invoker,
            tool_registry: Some(tool_registry),
            memory: Some(memory),
            loopback_only: true,
            sidecar: "127.0.0.1:18181".to_string(),
        }
    }

    /// 已注册方法名列表（混合检索引擎 ACP 方法面；tools.* 仅在注入注册表时出现）。
    pub fn methods(&self) -> Vec<&'static str> {
        let mut m = vec!["ping", "session.info", "agent.invoke", "health"];
        if self.tool_registry.is_some() {
            m.push("tools.list");
            m.push("tools.invoke");
        }
        m.push("permissions.list");
        m.push("auth.capabilities");
        m.push("events.list");
        m.push("session.list");
        if self.memory.is_some() {
            m.push("memory.search");
            m.push("memory.remember");
        }
        m
    }

    /// 引擎健康快照（转发给注入的 invoker）。
    pub fn health(&self) -> Value {
        self.invoker.health()
    }

    /// 处理一条原始消息，返回 JSON-RPC 响应 `Value`。
    ///
    /// - 解析层/调度层错误以 error 响应回包，不冒泡。
    /// - JSON-RPC 通知（无 id）返回 `None`（由网关回 204）。
    pub async fn handle(&self, msg: &str) -> Option<Value> {
        let req = match parse(msg) {
            Ok(r) => r,
            Err(e) => {
                return Some(make_error(&Value::Null, e.code, &e.message, e.data));
            }
        };
        let result = match self.dispatch(&req).await {
            Ok(v) => v,
            Err(e) => {
                let id = req.id.clone().unwrap_or(Value::Null);
                return Some(make_error(&id, e.code, &e.message, e.data));
            }
        };
        if req.is_notification() {
            None
        } else {
            let id = req.id.clone().unwrap_or(Value::Null);
            Some(make_result(&id, result))
        }
    }

    async fn dispatch(&self, req: &AcpRequest) -> Result<Value, AcpError> {
        match req.method.as_str() {
            "ping" => self.m_ping(),
            "session.info" => self.m_session_info(),
            "agent.invoke" => self.m_agent_invoke(req).await,
            "health" => Ok(self.health()),
            "tools.list" => self.m_tools_list(),
            "tools.invoke" => self.m_tools_invoke(req).await,
            "permissions.list" => self.m_permissions_list(),
            "auth.capabilities" => self.m_auth_capabilities(),
            "events.list" => self.m_events_list(),
            "session.list" => self.m_session_list(),
            "memory.search" => self.m_memory_search(req),
            "memory.remember" => self.m_memory_remember(req),
            other => Err(AcpError::new(
                METHOD_NOT_FOUND,
                format!("method not found: {other}"),
                None,
            )),
        }
    }

    fn m_ping(&self) -> Result<Value, AcpError> {
        Ok(json!({"pong": true, "session_id": self.session_id}))
    }

    fn m_session_info(&self) -> Result<Value, AcpError> {
        Ok(json!({
            "session_id": self.session_id,
            "methods": self.methods(),
            "loopback_only": self.loopback_only,
            "sidecar": self.sidecar,
        }))
    }

    async fn m_agent_invoke(&self, req: &AcpRequest) -> Result<Value, AcpError> {
        let prompt = req
            .params
            .get("prompt")
            .or_else(|| req.params.get("message"))
            .or_else(|| req.params.get("input"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let prompt = match prompt {
            Some(p) if !p.is_empty() => p,
            _ => {
                return Err(AcpError::new(
                    INVALID_PARAMS,
                    "agent.invoke requires a 'prompt' | 'message' | 'input' param",
                    None,
                ));
            }
        };
        match self.invoker.invoke(&prompt).await {
            Ok(result) => Ok(json!({
                "session_id": self.session_id,
                "sidecar": self.sidecar,
                "result": result,
            })),
            Err(e) => Err(AcpError::new(
                INTERNAL_ERROR,
                format!("agent invoke failed: {e}"),
                None,
            )),
        }
    }

    // ---- 混合检索引擎 ACP 方法面（Phase C-2）：tools / permissions / auth / events / session ----

    /// `tools.list`：枚举本地工具元信息（名称/描述/输入 schema）。
    fn m_tools_list(&self) -> Result<Value, AcpError> {
        match &self.tool_registry {
            Some(reg) => {
                let list: Vec<Value> = reg
                    .describe_tools_for_llm()
                    .into_iter()
                    .map(|(name, description, input_schema)| {
                        json!({
                            "name": name,
                            "description": description,
                            "input_schema": input_schema,
                        })
                    })
                    .collect();
                Ok(json!({ "tools": list, "count": list.len() }))
            }
            None => Err(AcpError::new(
                INVALID_REQUEST,
                "tools not configured on this session",
                None,
            )),
        }
    }

    /// `tools.invoke`：经 ToolRegistry 执行本地工具（复用既有的 ToolExecutor 回路）。
    async fn m_tools_invoke(&self, req: &AcpRequest) -> Result<Value, AcpError> {
        let reg = self.tool_registry.as_ref().ok_or_else(|| {
            AcpError::new(
                INVALID_REQUEST,
                "tools not configured on this session",
                None,
            )
        })?;
        let name = req
            .params
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let name = match name {
            Some(n) if !n.is_empty() => n,
            _ => {
                return Err(AcpError::new(
                    INVALID_PARAMS,
                    "tools.invoke requires a 'name' param",
                    None,
                ))
            }
        };
        let args = parse_tool_args(&req.params)?;
        // MCP 工具经 get_tool 命中后走其 execute；否则走本地工具回路。
        if let Some(tool) = reg.get_tool(&name) {
            match tool.execute(&args).await {
                Ok(ToolResult::Success { data, metadata }) => {
                    Ok(json!({ "ok": true, "data": data, "metadata": metadata }))
                }
                Ok(ToolResult::Error { message, code }) => Err(AcpError::new(
                    INTERNAL_ERROR,
                    format!("tool '{name}' failed [{code}]: {message}"),
                    None,
                )),
                Ok(ToolResult::Timeout { .. }) => Err(AcpError::new(
                    INTERNAL_ERROR,
                    format!("tool '{name}' timed out"),
                    None,
                )),
                Err(e) => Err(AcpError::new(
                    INTERNAL_ERROR,
                    format!("tool '{name}' error: {e}"),
                    None,
                )),
            }
        } else {
            match reg.execute_local_tool(&name, &args).await {
                Ok(ToolResult::Success { data, metadata }) => {
                    Ok(json!({ "ok": true, "data": data, "metadata": metadata }))
                }
                Ok(ToolResult::Error { message, code }) => Err(AcpError::new(
                    INTERNAL_ERROR,
                    format!("tool '{name}' failed [{code}]: {message}"),
                    None,
                )),
                Ok(ToolResult::Timeout { .. }) => Err(AcpError::new(
                    INTERNAL_ERROR,
                    format!("tool '{name}' timed out"),
                    None,
                )),
                Err(e) => Err(AcpError::new(
                    INTERNAL_ERROR,
                    format!("tool '{name}' error: {e}"),
                    None,
                )),
            }
        }
    }

    /// `permissions.list`：声明分层护栏能力（L1 命令/执行/SSRF、L2 恶意代码扫描、L3 操作护栏）。
    ///
    /// # 声明一致性（F-07）
    ///
    /// `enforced` 字段是对外的**安全保证**，必须与运行时事实严格一致。
    /// 曾出现 L2/L3 声明 `enforced: true` 但生产路径零调用点的"幻象防护"
    /// （虚假安全保证，OWASP A05 + A09 / STRIDE-Repudiation）。
    ///
    /// 现每层都附 `enforced_at`——真实拦截点的代码位置。改动护栏接线时
    /// **必须同步更新此处**；`session_declares_only_actually_enforced_layers`
    /// 测试会实际驱动各层拦截器验证声明为真，接线被摘除则测试失败。
    fn m_permissions_list(&self) -> Result<Value, AcpError> {
        Ok(json!({
            "model": "layered_guardrails",
            "transport": "loopback_only",
            "external_auth": false,
            // `enforced: true` 的语义严格限定为「`enforced_at` 所列调用点确实会拦截」，
            // 而非「本进程一切执行路径都受保护」。任何未列入 `enforced_at` 的执行面
            // 都必须在 `uncovered_surfaces` 里显式声明——宁可暴露缺口，
            // 也不给出覆盖不到的安全承诺（F-07 的教训：声明与接线必须一致）。
            "scope": "覆盖 ACP 工具管线（tools::builtin::*）、terminal::TerminalSession 与 \
                      gateway 的 sandbox.terminal.* 常驻 PTY 通道",
            // 2026-10-25：`tools::terminal` 早已接入 L1/L2/L3 + JSONL 审计
            // （write() 内的 security_check，见该模块文档），此前这里的
            // enforced:false 是过期声明，已按事实删除。
            "uncovered_surfaces": [],
            "layers": [
                {"id": "L1", "name": "command_guard", "enforced": true,
                 "enforced_at": ["security::exec::exec_with", "security::exec::sandboxed_exec",
                                 "terminal::TerminalSession::security_check"]},
                {"id": "L1", "name": "exec_sandbox", "enforced": true,
                 "enforced_at": ["tools::builtin::shell_exec",
                                 "tools::terminal::TerminalSession::security_check"]},
                {"id": "L1", "name": "ssrf_guard", "enforced": true,
                 "enforced_at": ["tools::builtin::web_fetch", "tools::builtin::http_request"]},
                {"id": "L2", "name": "malware_static_scan", "enforced": true,
                 "enforced_at": ["tools::builtin::write_file", "tools::builtin::append_file",
                                 "tools::builtin::edit_file", "tools::builtin::copy_file",
                                 "tools::builtin::shell_exec", "terminal::TerminalSession::security_check"]},
                {"id": "L3", "name": "operation_guardrail", "enforced": true,
                 "enforced_at": ["tools::builtin::write_file", "tools::builtin::append_file",
                                 "tools::builtin::edit_file", "tools::builtin::copy_file",
                                 "tools::builtin::move_file", "tools::builtin::delete_file",
                                 "tools::builtin::make_dir", "tools::builtin::shell_exec",
                                 "terminal::TerminalSession::security_check"]}
            ]
        }))
    }

    /// `auth.capabilities`：回环信任模型（无外部鉴权，会话作用域）。
    fn m_auth_capabilities(&self) -> Result<Value, AcpError> {
        Ok(json!({
            "model": "loopback_trust",
            "transport": "127.0.0.1",
            "external_auth_required": false,
            "session_scoped": true
        }))
    }

    /// `events.list`：引擎可发射事件目录（订阅能力暂未实现，诚实标注）。
    fn m_events_list(&self) -> Result<Value, AcpError> {
        Ok(json!({
            "catalog": [
                "agent.invoke.start", "agent.invoke.done",
                "tool.call", "tool.result",
                "permission.denied", "session.created", "session.closed", "error"
            ],
            "subscription": "not_implemented"
        }))
    }

    /// `session.list`：当前活跃会话（ACP 薄壳按 listener 单会话）。
    fn m_session_list(&self) -> Result<Value, AcpError> {
        Ok(json!({ "sessions": [self.session_id], "active": 1 }))
    }

    /// `memory.search`：按类别 + 查询召回 top_k 条记忆（claude-mem 融合）。
    fn m_memory_search(&self, req: &AcpRequest) -> Result<Value, AcpError> {
        let backend = self
            .memory
            .as_ref()
            .ok_or_else(|| AcpError::new(INVALID_REQUEST, "memory backend not configured", None))?;

        let category_str = req
            .params
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("project");
        let category = MemoryCategory::parse(category_str).ok_or_else(|| {
            AcpError::new(
                INVALID_PARAMS,
                format!(
                    "memory.search 'category' 必须是 user|project|feedback|auto，收到：{category_str}"
                ),
                None,
            )
        })?;

        let query = req
            .params
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let top_k = req
            .params
            .get("top_k")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(5);

        let hits = backend.recall(category, query, top_k);
        Ok(json!({
            "category": category.as_str(),
            "query": query,
            "count": hits.len(),
            "hits": hits.iter().map(|h| json!({
                "key": h.key,
                "snippet": h.snippet,
                "score": h.score
            })).collect::<Vec<_>>()
        }))
    }

    /// `memory.remember`：写入一条记忆（claude-mem 融合）。
    fn m_memory_remember(&self, req: &AcpRequest) -> Result<Value, AcpError> {
        let backend = self
            .memory
            .as_ref()
            .ok_or_else(|| AcpError::new(INVALID_REQUEST, "memory backend not configured", None))?;

        let category_str = req
            .params
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("project");
        let category = MemoryCategory::parse(category_str).ok_or_else(|| {
            AcpError::new(
                INVALID_PARAMS,
                format!(
                    "memory.remember 'category' 必须是 user|project|feedback|auto，收到：{category_str}"
                ),
                None,
            )
        })?;

        let key = req
            .params
            .get("key")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| AcpError::new(INVALID_PARAMS, "memory.remember 需要 'key'", None))?;
        let value = req
            .params
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match backend.remember(category, key, value) {
            Ok(()) => Ok(json!({
                "ok": true,
                "category": category.as_str(),
                "key": key,
                "stored": true
            })),
            Err(e) => Err(AcpError::new(
                INTERNAL_ERROR,
                format!("memory.remember failed: {e}"),
                None,
            )),
        }
    }
}

/// 从 ACP 参数解析 `args` 对象为 `HashMap<String, String>`（非字符串值 `.to_string()` 归一）。
fn parse_tool_args(params: &Map<String, Value>) -> Result<HashMap<String, String>, AcpError> {
    match params.get("args") {
        None | Some(Value::Null) => Ok(HashMap::new()),
        Some(Value::Object(m)) => Ok(m
            .iter()
            .map(|(k, v)| {
                let val = match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                (k.clone(), val)
            })
            .collect()),
        Some(_) => Err(AcpError::new(
            INVALID_PARAMS,
            "'args' must be a JSON object",
            None,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::agent::tool_cycle::ToolResult;
    use crate::mcp::registry::{ToolExecutor, ToolRegistry};
    use anyhow::Result;

    struct MockInvoker {
        calls: Arc<AtomicUsize>,
        reply: String,
    }

    #[async_trait]
    impl AgentInvoker for MockInvoker {
        async fn invoke(&self, _prompt: &str) -> Result<String, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.reply.clone())
        }
    }

    fn session() -> (AcpSession, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let invoker: Arc<dyn AgentInvoker> = Arc::new(MockInvoker {
            calls: calls.clone(),
            reply: "done".to_string(),
        });
        (AcpSession::new("s1", invoker), calls)
    }

    #[tokio::test]
    async fn ping_returns_pong() {
        let (s, _) = session();
        let resp = s
            .handle(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#)
            .await
            .unwrap();
        assert_eq!(resp["result"]["pong"], true);
        assert_eq!(resp["result"]["session_id"], "s1");
    }

    #[tokio::test]
    async fn session_info_lists_methods() {
        let (s, _) = session();
        let resp = s
            .handle(r#"{"jsonrpc":"2.0","id":2,"method":"session.info"}"#)
            .await
            .unwrap();
        let methods = resp["result"]["methods"].as_array().unwrap();
        assert!(methods.iter().any(|m| m == "agent.invoke"));
        assert_eq!(resp["result"]["loopback_only"], true);
    }

    #[tokio::test]
    async fn agent_invoke_forwards_to_invoker() {
        let (s, calls) = session();
        let resp = s
            .handle(r#"{"jsonrpc":"2.0","id":3,"method":"agent.invoke","params":{"prompt":"hi"}}"#)
            .await
            .unwrap();
        assert_eq!(resp["result"]["result"], "done");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn agent_invoke_rejects_missing_prompt() {
        let (s, _) = session();
        let resp = s
            .handle(r#"{"jsonrpc":"2.0","id":4,"method":"agent.invoke","params":{}}"#)
            .await
            .unwrap();
        assert_eq!(resp["error"]["code"], INVALID_PARAMS);
    }

    #[tokio::test]
    async fn unknown_method_returns_method_not_found() {
        let (s, _) = session();
        let resp = s
            .handle(r#"{"jsonrpc":"2.0","id":5,"method":"bogus"}"#)
            .await
            .unwrap();
        assert_eq!(resp["error"]["code"], METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn parse_error_returns_error_response() {
        let (s, _) = session();
        // 非法 JSON：应回 error 包而非 panic
        let resp = s.handle("not json at all").await.unwrap();
        assert_eq!(resp["error"]["code"], -32700);
    }

    #[tokio::test]
    async fn notification_returns_none() {
        let (s, _) = session();
        let r = s.handle(r#"{"jsonrpc":"2.0","method":"ping"}"#).await;
        assert!(r.is_none());
    }

    // ---- Phase C-2：混合检索引擎 ACP 方法面测试 ----

    struct MockTool;
    #[async_trait]
    impl ToolExecutor for MockTool {
        fn name(&self) -> &str {
            "echo_tool"
        }
        fn description(&self) -> &str {
            "echoes input"
        }
        async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
            let input = args.get("input").cloned().unwrap_or_default();
            Ok(ToolResult::Success {
                data: format!("echo:{input}"),
                metadata: HashMap::new(),
            })
        }
        fn input_schema(&self) -> Value {
            json!({"type": "object", "properties": {"input": {"type": "string"}}})
        }
    }

    fn session_with_tools() -> (AcpSession, Arc<ToolRegistry>) {
        let mut reg = ToolRegistry::new();
        reg.register_local(Box::new(MockTool));
        let reg = Arc::new(reg);
        let invoker: Arc<dyn AgentInvoker> = Arc::new(MockInvoker {
            calls: Arc::new(AtomicUsize::new(0)),
            reply: "done".to_string(),
        });
        (AcpSession::new_with_tools("s1", invoker, reg.clone()), reg)
    }

    #[tokio::test]
    async fn tools_list_returns_descriptors() {
        let (s, _) = session_with_tools();
        let resp = s
            .handle(r#"{"jsonrpc":"2.0","id":10,"method":"tools.list"}"#)
            .await
            .unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "echo_tool");
        assert_eq!(tools[0]["input_schema"]["type"], "object");
    }

    #[tokio::test]
    async fn tools_invoke_executes() {
        let (s, _) = session_with_tools();
        let resp = s
            .handle(r#"{"jsonrpc":"2.0","id":11,"method":"tools.invoke","params":{"name":"echo_tool","args":{"input":"hi"}}}"#)
            .await
            .unwrap();
        assert_eq!(resp["result"]["ok"], true);
        assert_eq!(resp["result"]["data"], "echo:hi");
    }

    #[tokio::test]
    async fn tools_invoke_missing_name() {
        let (s, _) = session_with_tools();
        let resp = s
            .handle(r#"{"jsonrpc":"2.0","id":12,"method":"tools.invoke","params":{}}"#)
            .await
            .unwrap();
        assert_eq!(resp["error"]["code"], INVALID_PARAMS);
    }

    #[tokio::test]
    async fn tools_invoke_unknown_tool() {
        let (s, _) = session_with_tools();
        let resp = s
            .handle(r#"{"jsonrpc":"2.0","id":13,"method":"tools.invoke","params":{"name":"nope"}}"#)
            .await
            .unwrap();
        assert_eq!(resp["error"]["code"], INTERNAL_ERROR);
    }

    #[tokio::test]
    async fn tools_not_configured_without_registry() {
        let (s, _) = session();
        let resp = s
            .handle(r#"{"jsonrpc":"2.0","id":14,"method":"tools.list"}"#)
            .await
            .unwrap();
        assert_eq!(resp["error"]["code"], INVALID_REQUEST);
    }

    // ---- Phase C-3：claude-mem ACP 方法面测试 ----

    fn session_with_memory() -> (AcpSession, Arc<crate::memory::recall::KbBackedMemory>) {
        use crate::memory::recall::KbBackedMemory;
        let mem = Arc::new(KbBackedMemory::new());
        let invoker: Arc<dyn AgentInvoker> = Arc::new(MockInvoker {
            calls: Arc::new(AtomicUsize::new(0)),
            reply: "done".to_string(),
        });
        (AcpSession::new_with_memory("s1", invoker, mem.clone()), mem)
    }

    #[tokio::test]
    async fn memory_remember_then_search() {
        let (s, _mem) = session_with_memory();
        // 写入一条 project 记忆
        let resp = s.handle(r#"{"jsonrpc":"2.0","id":20,"method":"memory.remember","params":{"category":"project","key":"deploy","value":"Kubernetes orchestrates containers at scale."}}"#).await.unwrap();
        assert_eq!(resp["result"]["ok"], true);
        assert_eq!(resp["result"]["stored"], true);

        // 召回
        let resp = s.handle(r#"{"jsonrpc":"2.0","id":21,"method":"memory.search","params":{"category":"project","query":"kubernetes orchestration","top_k":3}}"#).await.unwrap();
        assert_eq!(resp["result"]["category"], "project");
        let hits = resp["result"]["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["key"], "deploy");
        assert!(hits[0]["score"].as_f64().unwrap() > 0.0);
    }

    #[tokio::test]
    async fn memory_remember_user_and_recall() {
        let (s, _mem) = session_with_memory();
        let resp = s.handle(r#"{"jsonrpc":"2.0","id":22,"method":"memory.remember","params":{"category":"user","key":"pref","value":"喜欢用 Rust 写后端"}}"#).await.unwrap();
        assert_eq!(resp["result"]["ok"], true);

        let resp = s.handle(r#"{"jsonrpc":"2.0","id":23,"method":"memory.search","params":{"category":"user","query":"rust 后端","top_k":5}}"#).await.unwrap();
        let hits = resp["result"]["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["key"], "pref");
        assert!(hits[0]["score"].as_f64().unwrap() >= 1.0);
    }

    #[tokio::test]
    async fn memory_search_invalid_category() {
        let (s, _mem) = session_with_memory();
        let resp = s.handle(r#"{"jsonrpc":"2.0","id":24,"method":"memory.search","params":{"category":"nope","query":"x"}}"#).await.unwrap();
        assert_eq!(resp["error"]["code"], INVALID_PARAMS);
    }

    #[tokio::test]
    async fn memory_remember_missing_key() {
        let (s, _mem) = session_with_memory();
        let resp = s.handle(r#"{"jsonrpc":"2.0","id":25,"method":"memory.remember","params":{"category":"user","value":"v"}}"#).await.unwrap();
        assert_eq!(resp["error"]["code"], INVALID_PARAMS);
    }

    #[tokio::test]
    async fn memory_not_configured_without_backend() {
        let (s, _) = session();
        let resp = s.handle(r#"{"jsonrpc":"2.0","id":26,"method":"memory.search","params":{"category":"project","query":"x"}}"#).await.unwrap();
        assert_eq!(resp["error"]["code"], INVALID_REQUEST);
    }

    #[tokio::test]
    async fn permissions_list_returns_manifest() {
        let (s, _) = session();
        let resp = s
            .handle(r#"{"jsonrpc":"2.0","id":15,"method":"permissions.list"}"#)
            .await
            .unwrap();
        assert_eq!(resp["result"]["model"], "layered_guardrails");
        assert!(resp["result"]["layers"].as_array().unwrap().len() >= 5);
    }

    /// F-07 声明一致性守卫：`permissions.list` 里每个 `enforced: true` 的层，
    /// 都必须真的能拦下该层专属的危险输入。
    ///
    /// 这不是对 JSON 的同义反复断言——它**实际驱动**各层拦截器：
    /// 任何一层被从生产路径摘除（或退化为放行），本测试立即失败，
    /// 从而杜绝"声明 enforced 但运行时无人调用"的幻象防护重现。
    #[tokio::test]
    async fn session_declares_only_actually_enforced_layers() {
        use crate::security::{CommandGuard, Preflight, SsrfGuard};
        use std::path::Path;

        let (s, _) = session();
        let resp = s
            .handle(r#"{"jsonrpc":"2.0","id":115,"method":"permissions.list"}"#)
            .await
            .unwrap();
        let layers = resp["result"]["layers"].as_array().unwrap().clone();

        let ws = Path::new("/home/u/proj");
        let pf = Preflight::new();

        for layer in layers {
            let name = layer["name"].as_str().unwrap_or_default().to_string();
            let enforced = layer["enforced"].as_bool().unwrap_or(false);
            assert!(
                layer["enforced_at"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty()),
                "层 {name} 必须标注真实拦截点 enforced_at"
            );
            if !enforced {
                continue;
            }
            // 逐层用该层专属的危险输入验证「声明为真」
            let actually_blocks = match name.as_str() {
                "command_guard" | "exec_sandbox" => {
                    CommandGuard::new().check("sudo rm -rf /").is_err()
                }
                "ssrf_guard" => SsrfGuard::new()
                    .check("http://169.254.169.254/latest/meta-data/")
                    .is_err(),
                // L2 专属：CommandGuard 不认识 eval，只有 L2 内容扫描能拦
                "malware_static_scan" => {
                    !pf.check_content("result = eval(user_input)\n").is_allowed()
                }
                // L3 专属：safe_resolve 允许工作区内写 .ssh，CommandGuard 也不认识 chroot
                "operation_guardrail" => {
                    !pf.check_write(
                        Path::new("/home/u/proj/.ssh/authorized_keys"),
                        ws,
                        "ssh-rsa AAAA\n",
                    )
                    .is_allowed()
                        && !pf.check_exec("chroot /mnt /bin/sh", ws).is_allowed()
                }
                other => panic!("未知护栏层 `{other}`：新增层必须同时补上一致性验证"),
            };
            assert!(
                actually_blocks,
                "层 {name} 声明 enforced:true，但运行时并未真正拦截——\
                 要么补上接线，要么把 enforced 改成 false（禁止虚假安全保证）"
            );
        }
    }

    /// 声明必须与接线一致：`enforced: true` 只对 `enforced_at` 所列调用点成立。
    ///
    /// 2026-10-25：`tools::terminal` 已接入 L1/L2/L3 + 审计，故
    /// `uncovered_surfaces` 必须为空；若将来又出现无护栏通道，改回在
    /// `uncovered_surfaces` 里如实声明 `enforced:false` 并同步改本断言。
    #[tokio::test]
    async fn session_declares_no_uncovered_execution_surfaces() {
        let (s, _) = session();
        let resp = s
            .handle(r#"{"jsonrpc":"2.0","id":116,"method":"permissions.list"}"#)
            .await
            .unwrap();
        let result = &resp["result"];

        assert!(
            result["scope"].as_str().is_some_and(|s| s.contains("sandbox.terminal")),
            "scope 必须写明覆盖 sandbox.terminal.*（否则调用方会误以为它不受保护）"
        );

        let uncovered = result["uncovered_surfaces"]
            .as_array()
            .expect("必须存在 uncovered_surfaces 字段（可为空数组，但不能缺失）");
        assert!(
            uncovered.is_empty(),
            "终端通道已接入护栏，不应再声明无护栏执行面：{uncovered:?}"
        );

        // 缺口声明不得混进 layers（避免被误读成"已启用的一层"）
        for layer in result["layers"].as_array().unwrap() {
            assert_ne!(
                layer["enforced"].as_bool(),
                Some(false),
                "未接线的层不应出现在 layers 里，应放进 uncovered_surfaces"
            );
        }
    }

    #[tokio::test]
    async fn auth_capabilities_returns_trust_model() {
        let (s, _) = session();
        let resp = s
            .handle(r#"{"jsonrpc":"2.0","id":16,"method":"auth.capabilities"}"#)
            .await
            .unwrap();
        assert_eq!(resp["result"]["model"], "loopback_trust");
        assert_eq!(resp["result"]["external_auth_required"], false);
    }

    #[tokio::test]
    async fn events_list_returns_catalog() {
        let (s, _) = session();
        let resp = s
            .handle(r#"{"jsonrpc":"2.0","id":17,"method":"events.list"}"#)
            .await
            .unwrap();
        assert!(resp["result"]["catalog"].as_array().unwrap().len() >= 5);
        assert_eq!(resp["result"]["subscription"], "not_implemented");
    }

    #[tokio::test]
    async fn session_list_returns_active() {
        let (s, _) = session();
        let resp = s
            .handle(r#"{"jsonrpc":"2.0","id":18,"method":"session.list"}"#)
            .await
            .unwrap();
        assert_eq!(resp["result"]["active"], 1);
        assert_eq!(resp["result"]["sessions"][0], "s1");
    }

    #[tokio::test]
    async fn session_info_lists_new_methods() {
        let (s, _) = session_with_tools();
        let resp = s
            .handle(r#"{"jsonrpc":"2.0","id":19,"method":"session.info"}"#)
            .await
            .unwrap();
        let methods = resp["result"]["methods"].as_array().unwrap();
        assert!(methods.iter().any(|m| m == "tools.list"));
        assert!(methods.iter().any(|m| m == "tools.invoke"));
        assert!(methods.iter().any(|m| m == "permissions.list"));
        assert!(methods.iter().any(|m| m == "auth.capabilities"));
        assert!(methods.iter().any(|m| m == "events.list"));
        assert!(methods.iter().any(|m| m == "session.list"));
    }
}
