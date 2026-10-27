//! M0.4 WS JSON-RPC 回环测试夹具（IMPL-PLAN 任务 0.4）
//!
//! 提供同步 dispatch 直连 + 进程内事件订阅捕获，不启动真实 WS 服务器，
//! 不要求真实 SSH/主机。补 QA 指出的夹具缺口：confirm 流 / tail 推送 / 限流可测。
//!
//! 接缝契约（M1 生产化时的对齐点，事件名与方法语义在此固化）：
//! - confirm 流：复用生产 [`ApprovalGate`]（guard::gateway），受保护方法派发时
//!   先挂起审批并在事件总线发布 `ops.confirm.required`（EventType::Approval），
//!   批准前一律拒绝执行（fail-closed）；批准后方可执行放行。
//! - 限流：夹具令牌桶（M1.5 任务 1.5 令牌桶的测试接缝），超限返回
//!   JSON-RPC error（[`RpcError`]，code = RATE_LIMITED），非 HTTP 状态码。
#![cfg(test)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::events::envelope::Envelope;
use crate::events::{EventBus, EventType};
use crate::gateway::events::envelope_to_frame;
use crate::gateway::methods::{dispatch_sync, GatewayContext};
use crate::gateway::protocol::RpcError;
use crate::guard::gateway::{ApprovalGate, RiskLevel};
use crate::persistence::config::Config;

/// 夹具上下文：临时 config 目录（隔离真实 ~/.marguerite）+ 指定事件总线。
fn ctx_with_bus(bus: Arc<EventBus>) -> GatewayContext {
    use std::sync::{Mutex, OnceLock};
    static CONF: OnceLock<Mutex<std::path::PathBuf>> = OnceLock::new();
    let dir = CONF
        .get_or_init(|| {
            let d = std::env::temp_dir().join(format!("mgt-loopback-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&d);
            Mutex::new(d)
        })
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    Config::set_config_dir_override(dir);
    GatewayContext {
        config: Config::default(),
        token: "loopback".into(),
        started_at: Instant::now(),
        bus,
    }
}

/// 受二次确认保护的同步派发（confirm 流夹具）。
struct ConfirmLoopback {
    gate: Arc<ApprovalGate>,
    protected: &'static str,
    bus: Arc<EventBus>,
}

impl ConfirmLoopback {
    fn new(protected: &'static str, bus: Arc<EventBus>) -> Self {
        Self {
            gate: Arc::new(ApprovalGate::new(Duration::from_secs(60))),
            protected,
            bus,
        }
    }

    /// 发布 ops.confirm.required（M1.4 事件名契约，夹具先行固化）。
    fn publish_confirm_required(&self, method: &str, approval_id: &str) {
        self.bus.publish(Envelope::new(
            EventType::Approval,
            json!({
                "event": "ops.confirm.required",
                "method": method,
                "approval_id": approval_id,
                "risk": "high",
            }),
            "loopback",
        ));
    }

    /// 派发：受保护方法先过审批闸门；未批准一律拒绝（fail-closed）。
    fn dispatch(
        &self,
        ctx: &GatewayContext,
        method: &str,
        params: &Value,
    ) -> Result<Value, RpcError> {
        if method != self.protected {
            return dispatch_sync(ctx, method, params);
        }
        let pending = self
            .gate
            .request(method, params.to_string(), RiskLevel::High)
            .ok_or_else(|| RpcError::new("OPS_CONFIRM_BUSY", "已有挂起审批，先处理再发起新请求"))?;
        self.publish_confirm_required(method, &pending.id);
        Err(RpcError::new(
            "OPS_CONFIRM_REQUIRED",
            format!(
                "方法 {method} 需二次确认（approval_id={}），批准前不执行",
                pending.id
            ),
        ))
    }

    /// 批准指定审批后执行放行（confirm 流闭环后半段）。
    fn approve_and_execute(
        &self,
        ctx: &GatewayContext,
        approval_id: &str,
        params: &Value,
    ) -> Result<Value, RpcError> {
        let approved = self.gate.approve(approval_id);
        if !approved {
            return Err(RpcError::new(
                "OPS_CONFIRM_DENIED",
                format!("审批 {approval_id} 不存在、不匹配或已超时，拒绝执行"),
            ));
        }
        dispatch_sync(ctx, self.protected, params)
    }
}

/// 令牌桶夹具（M1.5 令牌桶限流的测试接缝：每桶固定容量 + 按时间线性补充）。
struct TokenBucket {
    capacity: f64,
    tokens: f64,
    refill_per_sec: f64,
    last: Instant,
}

impl TokenBucket {
    fn new(capacity: usize, refill_per_sec: f64) -> Self {
        Self {
            capacity: capacity as f64,
            tokens: capacity as f64,
            refill_per_sec,
            last: Instant::now(),
        }
    }

    fn take(&mut self) -> Result<(), RpcError> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            Err(RpcError::new(
                "RATE_LIMITED",
                "请求超过令牌桶限额（JSON-RPC error，非 HTTP 状态码）",
            )
            .with_retryable(true, Some(200)))
        }
    }
}

/// 限流派发夹具：超限直接拒绝，令牌可用才落到真实 dispatch_sync。
struct RateLimitedLoopback {
    bucket: Mutex<TokenBucket>,
    protected: &'static str,
}

impl RateLimitedLoopback {
    fn new(capacity: usize, refill_per_sec: f64, protected: &'static str) -> Self {
        Self {
            bucket: Mutex::new(TokenBucket::new(capacity, refill_per_sec)),
            protected,
        }
    }

    fn dispatch(
        &self,
        ctx: &GatewayContext,
        method: &str,
        params: &Value,
    ) -> Result<Value, RpcError> {
        if method == self.protected {
            self.bucket
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()?;
        }
        dispatch_sync(ctx, method, params)
    }
}

/// 订阅 `**` 并把到达的事件收进 Vec（进程内事件捕获）。
fn capture_events(bus: &EventBus) -> Arc<Mutex<Vec<Envelope>>> {
    let mut rx = bus.subscribe("**");
    let captured: Arc<Mutex<Vec<Envelope>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = captured.clone();
    tokio::spawn(async move {
        while let Some(env) = rx.recv().await {
            sink.lock().unwrap_or_else(|e| e.into_inner()).push(env);
        }
    });
    captured
}

async fn wait_for(captured: &Arc<Mutex<Vec<Envelope>>>, min_len: usize) -> Vec<Envelope> {
    for _ in 0..100 {
        let n = captured.lock().unwrap_or_else(|e| e.into_inner()).len();
        if n >= min_len {
            break;
        }
        // 用 tokio 睡眠让出执行权（当前线程运行时下，spawn 的捕获任务才有机会投递事件）
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    captured.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// confirm 流：受保护方法派发 -> ops.confirm.required 事件发出 -> 未批准不放行
/// -> 批准后执行放行。
#[tokio::test]
async fn confirm_flow_blocks_until_approved() {
    let bus = Arc::new(EventBus::default());
    let ctx = ctx_with_bus(bus.clone());
    let loopback = ConfirmLoopback::new("artifacts.list", bus.clone());
    let captured = capture_events(&bus);
    let params = json!({});

    // 第一次派发：被 confirm 闸门拦下，返回 JSON-RPC error，并发出确认事件
    let blocked = loopback
        .dispatch(&ctx, "artifacts.list", &params)
        .unwrap_err();
    assert_eq!(
        blocked.code, "OPS_CONFIRM_REQUIRED",
        "未批准必须被拦截: {blocked:?}"
    );
    let events = wait_for(&captured, 1).await;
    assert_eq!(
        events.len(),
        1,
        "应恰好发出一条 ops.confirm.required: {events:?}"
    );
    assert_eq!(events[0].r#type.as_str(), "approval");
    assert_eq!(events[0].payload["event"], "ops.confirm.required");
    assert_eq!(events[0].payload["method"], "artifacts.list");
    let approval_id = events[0].payload["approval_id"]
        .as_str()
        .unwrap()
        .to_string();
    // WS 帧契约映射：approval 事件帧名 = session.event.approval
    let frame = envelope_to_frame(&events[0], 1);
    assert_eq!(frame["event"], "session.event.approval");

    // 未批准期间重复派发仍然不放行
    let still = loopback.dispatch(&ctx, "artifacts.list", &params);
    assert!(still.is_err(), "批准前重复派发不得执行");

    // 用错误 ID 批准 -> 拒绝执行
    let bad = loopback.approve_and_execute(&ctx, "ap_wrong", &params);
    assert_eq!(bad.unwrap_err().code, "OPS_CONFIRM_DENIED");

    // 正确批准 -> 执行放行
    let ok = loopback
        .approve_and_execute(&ctx, &approval_id, &params)
        .expect("批准后应放行");
    assert!(
        ok["ok"] == true || ok.get("items").is_some() || ok.get("artifacts").is_some(),
        "执行结果应可用: {ok}"
    );
}

/// 限流：超过令牌桶阈值后返回 JSON-RPC error code（RATE_LIMITED），非 HTTP 状态码。
#[tokio::test]
async fn rate_limit_exceeded_returns_json_rpc_error() {
    let bus = Arc::new(EventBus::default());
    let ctx = ctx_with_bus(bus.clone());
    let limiter = RateLimitedLoopback::new(2, 0.0, "models.local.catalog");
    let params = json!({});

    assert!(limiter
        .dispatch(&ctx, "models.local.catalog", &params)
        .is_ok());
    assert!(limiter
        .dispatch(&ctx, "models.local.catalog", &params)
        .is_ok());
    let limited = limiter
        .dispatch(&ctx, "models.local.catalog", &params)
        .unwrap_err();
    assert_eq!(
        limited.code, "RATE_LIMITED",
        "超限应返回 JSON-RPC error code: {limited:?}"
    );
    assert_eq!(limited.retryable, Some(true), "限流错误应可重试");
    assert_eq!(limited.retry_after_ms, Some(200));

    // 不受限流保护的方法族不受影响（限流按方法粒度生效）
    assert!(limiter.dispatch(&ctx, "artifacts.list", &params).is_ok());
}


// ─────────────────────────────────────────────────────────────────────────────
// 会话推理等级（`session.thinking.*`）—— 输入框推理等级拖动条的服务端真源
// ─────────────────────────────────────────────────────────────────────────────

/// 三条契约一次验完：未设定返回 `null`（调用方据此保留本地默认，不能当 off）、
/// 设定后可读回、非法档位**响亮拒绝**（成本治理不允许 typo 静默升级）。
#[test]
fn session_thinking_get_set_roundtrip_and_rejects_unknown_effort() {
    let bus = Arc::new(EventBus::default());
    let ctx = ctx_with_bus(bus);
    let key = format!("loopback.thinking.{}", uuid::Uuid::new_v4());

    let fresh = dispatch_sync(&ctx, "session.thinking.get", &json!({ "sessionKey": key }))
        .expect("get 应可调用");
    assert!(
        fresh["effort"].is_null(),
        "从未设定过档位时必须回 null（前端据此保留本地默认值，不能当 off）: {fresh}"
    );

    let set = dispatch_sync(
        &ctx,
        "session.thinking.set",
        &json!({ "sessionKey": key, "effort": "deep" }),
    )
    .expect("set 应成功");
    assert_eq!(set["effort"], "deep", "{set}");
    assert_eq!(
        set["reasoningEffort"], "medium",
        "deep 必须映射到请求体 medium，否则档位到不了模型: {set}"
    );

    let read_back = dispatch_sync(&ctx, "session.thinking.get", &json!({ "sessionKey": key }))
        .expect("get 应可调用");
    assert_eq!(read_back["effort"], "deep", "落盘后必须读回同一档位: {read_back}");

    // 别名 + 大小写/空白宽容（与 chat.send 的 effort 解析同源）
    let alias = dispatch_sync(
        &ctx,
        "session.thinking.set",
        &json!({ "sessionKey": key, "effort": " HIGH " }),
    )
    .expect("别名 high 应被接受");
    assert_eq!(alias["effort"], "max", "high 是 max 的别名: {alias}");

    // off = 不注入 reasoning_effort（模型侧确实关闭思考）
    let off = dispatch_sync(
        &ctx,
        "session.thinking.set",
        &json!({ "sessionKey": key, "effort": "off" }),
    )
    .expect("off 应被接受");
    assert!(
        off["reasoningEffort"].is_null(),
        "off 不得向请求体注入 reasoning_effort: {off}"
    );

    // typo 必须报错，而不是静默取保守默认（Deep）
    let bad = dispatch_sync(
        &ctx,
        "session.thinking.set",
        &json!({ "sessionKey": key, "effort": "higg" }),
    )
    .expect_err("未知档位必须被拒绝");
    assert_eq!(bad.code, "INVALID_REQUEST", "{bad:?}");

    // 缺 sessionKey 也必须拒绝：否则会写出 `thinking/.json` 这种垃圾文件
    let no_key = dispatch_sync(&ctx, "session.thinking.get", &json!({}))
        .expect_err("缺 sessionKey 必须被拒绝");
    assert_eq!(no_key.code, "INVALID_REQUEST", "{no_key:?}");
}

#[path = "loopback_event_tests.rs"]
mod event_tests;
