//! 统一网关 RPC 协议层
//!
//! 对齐 Web UI 客户端（`webui/src/lib/rpc.ts`）的 Marguerite 协议契约：
//! - 握手：服务端发 `connect.challenge` 事件 → 客户端回 `connect` req（携带 token）→
//!   服务端回带 `protocol` 字段的 HelloOk res 帧。
//! - 常规调用：客户端 `req`（id/method/params）→ 服务端 `res`（ok/payload | error）。
//! - 事件流：`event` 帧（event/payload/meta/seq），seq 从 1 递增，客户端按连续校验，
//!   断档触发重连（`_noteIncomingFrame`）；`ping` → `pong` 心跳；`tick` 事件保活。
//! - 认证：仅回环本地 token（`connect.params.auth.token` 与服务端一致才放行）。

use serde::Serialize;
use serde_json::{json, Value};

/// 协议版本（客户端 `minProtocol/maxProtocol = 3`，HelloOk 回写同一值）
pub const PROTOCOL_VERSION: u32 = 3;

/// tick 心跳间隔（毫秒，HelloOk `policy.tick_interval_ms` 与推送周期共用）
pub const TICK_INTERVAL_MS: u64 = 30_000;

/// 支持的方法清单（HelloOk `features.methods`，UI 按 `supportsMethod` 探测）
pub const SUPPORTED_METHODS: &[&str] = &[
    // 状态 / 只读
    "health",
    "status",
    "doctor.status",
    "onboarding.router.configure",
    "onboarding.ensemble.configure",
    "onboarding.provider.probe",
    "onboarding.provider.configure",
    "config.get",
    "sandbox.status",
    "sessions.list",
    "sessions.messages.subscribe",
    "sessions.messages.snapshot",
    "sessions.messages.hydrate",
    "chat.history",
    "tools.catalog",
    // 宝塔面板凭据（baota_login 落盘目标）
    "baota.credentials.set",
    "baota.credentials.get",
    "baota.credentials.clear",
    "agents.list",
    "agents.get",
    "agents.create",
    "agents.update",
    "agents.delete",
    // 专家团队（行业领军人经验蒸馏档案库）+ 用户画像抽取
    "experts.list",
    "agent.profile.analyze",
    "cron.list",
    "usage.status",
    "usage.query",
    // 运行时模式（mode.get 只读 / mode.switch 异步切换）
    "mode.get",
    "mode.switch",
    "mode.status",
    // Bloom 会话（异步）
    "chat.send",
    "chat.abort",
    // 技能
    "skills.list",
    "skills.get",
    "skills.reload",
    "skills.search",
    "skills.install",
    "skills.uninstall",
    "skills.create",
    // 记忆层
    "memory.search",
    "memory.save",
    "memory.list",
    "memory.layers.status",
    "memory.sensory.search",
    "memory.short_term.search",
    "memory.long_term.search",
    "memory.recall.search",
    "memory.cross_project.search",
    "memory.cross_project.expand",
    "memory.behavior.list",
    "memory.behavior.summary",
    // 上下文管理 / 压缩（compact 走 async LLM 分支；rollback 恢复预压缩快照）
    "context.status",
    "context.compact",
    "context.rollback",
    // 任务拆解规划
    "plans.analyze",
    "plans.create",
    "plans.list",
    "plans.run",
    "plans.cancel",
    "plans.implement",
    "plans.revise",
    "plans.setMode",
    "plans.capabilities",
    // 规划升级（B4）：执行前清单 / 动态重规划 / 并行子任务
    "plans.checklist",
    "plans.run.advance",
    "plans.replan",
    "plans.parallel.run",
    // 渠道
    "channels.status",
    "channels.pairings",
    "channels.list",
    // 沙箱策略（Rust-first，侧车不可用时确定性可用）
    "sandbox.policy.get",
    "sandbox.policy.defaults",
    "sandbox.policy.update",
    "sandbox.run_mode.preference.get",
    "sandbox.run_mode.preference.set",
    "sandbox.capability.status",
    "sandbox.setup.status",
    // 项目工作区目录浏览 / 选择 / 新建（Rust 本地文件系统实现）
    "sandbox.path.pick",
    "sandbox.path.list",
    "sandbox.path.create-directory",
    // AI 智能融合路由
    "router.decision",
    "router.selflearning.status",
    // 项目工作区（新建任务引导打开/创建项目文件夹；会话绑定项目根目录）
    "workspaces.list",
    "workspaces.open",
    "workspace.file_changes.list",
    "workspace.file_changes.revert",
    "artifacts.list",
    // Epic A：产物台账（Rust 原生，不代理 Python 侧车）
    "artifacts.get",
    "artifacts.register",
    "artifacts.open_in_system",
    "artifacts.edit_inline",
    // Epic C：桌面自动化（Rust 原生：启动 / 动作 / 录制 / 闸门）
    "desktop.list_apps",
    "desktop.launch",
    "desktop.action",
    "desktop.snapshots",
    "desktop.stop",
    "workspaces.update",
    "workspaces.pin",
    "workspaces.remove",
    "workspaces.history.delete",
    // 画像导入（用户/记忆/导入物 三类目标 + preview/apply/undo 闭环）
    "memory.import.info",
    "memory.import.start",
    "memory.import.status",
    "memory.import.cancel",
    "memory.import.retry",
    "memory.import.apply",
    "memory.import.undo",
    "memory.import.discard",
    // 知识库（KB 2.0：BM25 检索 / 状态 / 灌入）
    "kb.status",
    "kb.search",
    "kb.ingest",
    "kb.list",
    "kb.delete",
    "kb.ingest_file",
    // 代码智能（CodeIntel：项目索引 + 符号/文本混合检索）
    "codeintel.status",
    "codeintel.index",
    "codeintel.search",
    "codeintel.symbols",
    // 内置浏览器 RPC 直通（browser.*：真实 Chromium/CDP 桥暴露给前端）
    "browser.status",
    "browser.open",
    "browser.navigate",
    "browser.evaluate",
    "browser.screenshot",
    "browser.a11y",
    "browser.click",
    "browser.close",
    // 引导目录 / 状态（Rust 离线回退：侧车不可用时返回内置 provider 目录）
    "onboarding.catalog",
    "onboarding.status",
    // 电脑操控（三层架构：屏幕感知/键鼠控制/GUI定位）
    "computer_use.action",
    // 模型舰队（多角色路由）
    "fleet.list",
    "fleet.route",
    // 自进化历史
    "evolve.history",
    // 插件槽位
    "plugins.list",
];

/// 支持的事件名清单（HelloOk `features.events`，一期为系统/会话事件）
pub const SUPPORTED_EVENTS: &[&str] = &[
    "tick",
    "sessions.changed",
    "artifact.updated",
    "desktop.action_done",
    "session.event.*",
];

/// RPC 错误（res.error 结构，对齐客户端 RpcErrorDetail）
#[derive(Debug, Clone, Serialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted: Option<bool>,
}

/// 超时错误的建议退避间隔（毫秒），供 webui 退避重试参考
const TIMEOUT_RETRY_AFTER_MS: u64 = 1000;

impl RpcError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
            retryable: None,
            retry_after_ms: None,
            accepted: None,
        }
    }

    /// 方法未实现：UI 层 `markMethodUnavailable` 感知（Phase 3 逐个落地）
    pub fn not_implemented(method: &str) -> Self {
        Self::new(
            "NOT_IMPLEMENTED",
            format!("方法 `{method}` 尚未在 Rust 网关实现（Phase 3 引擎桥接入）"),
        )
    }

    pub fn unauthorized() -> Self {
        Self::new("UNAUTHORIZED", "本地网关 token 校验失败")
    }

    /// 附加结构化 details（webui `currentPolicyFromConflict` 等消费）
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn internal(msg: &str) -> Self {
        Self::new("INTERNAL_ERROR", msg.to_string())
    }

    /// 标记为可重试（瞬时故障语义：侧车超时、网关过载等），
    /// 供 webui `useRpcCall`/上层按 retryable 决定是否退避重试。
    pub fn with_retryable(mut self, retryable: bool, retry_after_ms: Option<u64>) -> Self {
        self.retryable = Some(retryable);
        if let Some(ms) = retry_after_ms {
            self.retry_after_ms = Some(ms);
        }
        self
    }

    /// 调用超时（区别于用户中止/上游取消）：UI 应退避重试。
    /// `waited_ms` 为实际等待时长（毫秒），`retry_after_ms` 建议退避 1s。
    pub fn timeout(method: &str, waited_ms: u64) -> Self {
        Self::new(
            "TIMEOUT",
            format!("调用 `{method}` 超时（已等待 {waited_ms}ms，Python 侧车未响应）"),
        )
        .with_retryable(true, Some(TIMEOUT_RETRY_AFTER_MS))
    }

    /// 用户/上层主动中止（例如用户点击取消）：语义上**不应**退避重试。
    pub fn aborted(reason: &str) -> Self {
        Self::new("ABORTED", format!("调用已中止: {reason}")).with_retryable(false, None)
    }

    /// 上游信道/依赖被取消（例如连接断开、任务被取消）：不可重试。
    pub fn cancelled(reason: &str) -> Self {
        Self::new("CANCELLED", format!("调用已取消: {reason}")).with_retryable(false, None)
    }
}

/// 入站帧（客户端 → 网关），宽松解析未知字段
#[derive(Debug, Clone)]
pub struct InboundFrame {
    pub r#type: String,
    pub id: Option<String>,
    pub method: Option<String>,
    pub params: Value,
}

/// 握手参数（`connect` req 的 `params`）
#[derive(Debug, Clone)]
pub struct ConnectParams {
    pub min_protocol: u32,
    pub max_protocol: u32,
    pub token: Option<String>,
    pub guest_session_key: Option<String>,
}

pub fn parse_inbound(text: &str) -> Result<InboundFrame, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("JSON 解析失败: {e}"))?;
    let r#type = v
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let id = v.get("id").and_then(Value::as_str).map(str::to_string);
    let method = v.get("method").and_then(Value::as_str).map(str::to_string);
    let params = v.get("params").cloned().unwrap_or(Value::Null);
    Ok(InboundFrame {
        r#type,
        id,
        method,
        params,
    })
}

pub fn parse_connect_params(params: &Value) -> ConnectParams {
    let auth = params.get("auth").unwrap_or(&Value::Null);
    let num = |key: &str| -> u32 { params.get(key).and_then(Value::as_u64).unwrap_or(0) as u32 };
    ConnectParams {
        min_protocol: num("minProtocol"),
        max_protocol: num("maxProtocol"),
        token: auth
            .get("token")
            .and_then(Value::as_str)
            .map(str::to_string),
        guest_session_key: auth
            .get("guestSessionKey")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

// ---- 出站帧构造 ----

/// `connect.challenge` 事件（握手第一步，不带 seq）
pub fn challenge_frame() -> Value {
    json!({
        "type": "event",
        "event": "connect.challenge",
        "nonce": uuid::Uuid::new_v4().to_string(),
    })
}

/// HelloOk res 帧：携带 protocol/policy/auth/features，客户端据此判定连接成功
pub fn hello_frame(id: &str, methods: &[&str], events: &[&str]) -> Value {
    json!({
        "type": "res",
        "id": id,
        "ok": true,
        "protocol": PROTOCOL_VERSION,
        "policy": {
            "tick_interval_ms": TICK_INTERVAL_MS,
            "max_frame_bytes": 16 * 1024 * 1024,
        },
        "auth": {
            // 本地回环 owner：UI 的 `isLocalOwner` 依赖 principal.isOwner
            "principal": { "isOwner": true, "kind": "local" },
        },
        "features": {
            "methods": methods,
            "events": events,
        },
    })
}

/// 成功 res 帧
pub fn ok_frame(id: &str, payload: Value) -> Value {
    json!({ "type": "res", "id": id, "ok": true, "payload": payload })
}

/// 失败 res 帧（error 对象对齐客户端 RpcErrorDetail）
pub fn err_frame(id: &str, err: &RpcError) -> Value {
    json!({ "type": "res", "id": id, "ok": false, "error": err })
}

/// 事件帧（带递增 seq）
pub fn event_frame(event: &str, payload: Value, meta: Value, seq: u64) -> Value {
    json!({
        "type": "event",
        "event": event,
        "payload": payload,
        "meta": meta,
        "seq": seq,
    })
}

/// tick 保活事件帧
pub fn tick_frame(seq: u64) -> Value {
    event_frame(
        "tick",
        json!({ "ts": chrono::Utc::now().to_rfc3339() }),
        json!({}),
        seq,
    )
}

/// pong 心跳回帧
pub fn pong_frame() -> Value {
    json!({ "type": "pong" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_inbound_req() {
        let f = parse_inbound(r#"{"type":"req","id":"7","method":"health","params":{}}"#).unwrap();
        assert_eq!(f.r#type, "req");
        assert_eq!(f.id.as_deref(), Some("7"));
        assert_eq!(f.method.as_deref(), Some("health"));
    }

    #[test]
    fn test_parse_inbound_ping_and_malformed() {
        let f = parse_inbound(r#"{"type":"ping"}"#).unwrap();
        assert_eq!(f.r#type, "ping");
        assert!(f.method.is_none());
        assert!(parse_inbound("not json").is_err());
    }

    #[test]
    fn test_parse_connect_params_auth() {
        let p = parse_connect_params(&json!({
            "minProtocol": 3,
            "maxProtocol": 3,
            "client": { "name": "opensquilla-web" },
            "auth": { "token": "t1", "guestSessionKey": "osqg_x" },
        }));
        assert_eq!(p.min_protocol, 3);
        assert_eq!(p.max_protocol, 3);
        assert_eq!(p.token.as_deref(), Some("t1"));
        assert_eq!(p.guest_session_key.as_deref(), Some("osqg_x"));
    }

    #[test]
    fn test_hello_frame_contract() {
        let h = hello_frame("1", SUPPORTED_METHODS, SUPPORTED_EVENTS);
        assert_eq!(h["type"], "res");
        assert_eq!(h["ok"], true);
        assert_eq!(h["protocol"], PROTOCOL_VERSION);
        assert_eq!(h["auth"]["principal"]["isOwner"], true);
        assert!(h["features"]["methods"].is_array());
    }

    #[test]
    fn test_event_frame_seq_and_tick() {
        let e = event_frame("sessions.changed", json!({}), json!({}), 5);
        assert_eq!(e["seq"], 5);
        assert_eq!(e["event"], "sessions.changed");
        let t = tick_frame(6);
        assert_eq!(t["event"], "tick");
        assert_eq!(t["seq"], 6);
    }

    #[test]
    fn test_err_frame_roundtrip_shape() {
        let e = RpcError::unauthorized();
        let f = err_frame("9", &e);
        assert_eq!(f["ok"], false);
        assert_eq!(f["error"]["code"], "UNAUTHORIZED");
        assert_eq!(f["error"]["message"], "本地网关 token 校验失败");
    }

    #[test]
    fn test_rpc_error_timeout_is_retryable() {
        let e = RpcError::timeout("health", 60_000);
        assert_eq!(e.code, "TIMEOUT");
        assert_eq!(e.retryable, Some(true));
        assert!(e.retry_after_ms.is_some());
    }

    #[test]
    fn test_rpc_error_aborted_not_retryable() {
        let e = RpcError::aborted("用户取消");
        assert_eq!(e.code, "ABORTED");
        assert_eq!(e.retryable, Some(false));
    }

    #[test]
    fn test_rpc_error_cancelled_distinct_from_aborted() {
        let a = RpcError::aborted("x");
        let c = RpcError::cancelled("y");
        assert_ne!(a.code, c.code);
        assert_eq!(a.retryable, Some(false));
        assert_eq!(c.retryable, Some(false));
        assert_eq!(c.code, "CANCELLED");
    }
}
