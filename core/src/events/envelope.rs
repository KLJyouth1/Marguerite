//! 事件信封（Envelope）
//!
//! 所有事件流经统一信封：版本化字段 `v` 保证前向兼容，
//! `to_sse` / `from_sse` 提供 SSE（`data: {json}\n\n`）编解码。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::Value;
use uuid::Uuid;

/// 当前事件信封版本
pub const CURRENT_EVENT_VERSION: u8 = 1;

/// 事件信封解析/版本错误
#[derive(Debug, thiserror::Error)]
pub enum EventError {
    /// 收到超出当前支持版本的事件（消费方应忽略并告警）
    #[error("事件版本 {0} 超出当前支持版本 {1}，已忽略")]
    UnsupportedVersion(u8, u8),
    /// JSON 反序列化失败
    #[error("事件 JSON 解析失败: {0}")]
    Json(#[from] serde_json::Error),
}

impl PartialEq for EventError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (EventError::UnsupportedVersion(a, b), EventError::UnsupportedVersion(c, d)) => {
                a == c && b == d
            }
            // 仅比较变体种类，不比较内部错误内容（serde_json::Error 未实现 PartialEq）
            (EventError::Json(_), EventError::Json(_)) => true,
            _ => false,
        }
    }
}

/// 事件类型（11 种全集）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    /// 模型思考片段
    Thinking,
    /// 工具调用请求
    ToolCall,
    /// 工具返回结果
    ToolResult,
    /// 计划/链结构
    Plan,
    /// 审批请求/结果
    Approval,
    /// 进度播报
    Progress,
    /// 记忆台账引用
    MemoryRef,
    /// 错误
    Error,
    /// 完成
    Done,
    /// 状态变更（模式切换等）
    Status,
    /// AI 融合路由决策（Bloom：编排形态/路由理由；对齐 Web UI `session.event.router_decision`）
    RouterDecision,
    /// 上下文压缩生命周期（对齐 Web UI `session.event.compaction`：started/completed/skipped）
    Compaction,
    /// 计划修订快照（对齐 Web UI `session.event.plan_revision`：自动拆解产物）
    PlanRevision,
    /// 计划运行快照（对齐 Web UI `session.event.plan_run`：分步执行进度）
    PlanRun,
    /// 并行集成推理进度（Bloom Captain/Copilot 双模型并行；对齐 Web UI
    /// `session.event.ensemble_progress`：proposer/aggregator 的 start/finish，
    /// 使对话流在终态 `done` 之前就能增量呈现并行成员）
    EnsembleProgress,
    /// 正文增量（真流式：模型逐块输出文本）
    TextDelta,
    /// 工具回路步骤开始（第几轮工具调用）
    StepStart,
    /// 工具回路步骤结束
    StepFinish,
    /// 用量实时事件（对齐 Web UI `session.event.usage`：每次 LLM 调用后即时推送前端刷新）
    Usage,
    /// 会话运行态变更（对齐 Web UI `sessions.changed`：turn 开始/结束触发会话列表刷新）
    SessionsChanged,
    /// 产物登记/更新（Epic A R2：对齐 Web UI 顶层事件 `artifact.updated`）
    ArtifactUpdated,
    /// 桌面动作完成（Epic C：对齐 Web UI 顶层事件 `desktop.action_done`）
    DesktopActionDone,
    /// 升级路径（M2-C4：护栏决定「停下问人」——高影响副作用/意图歧义/超时/成本超预算，
    /// 输出结构化 escalation；对齐 Web UI `session.event.escalation`）
    Escalation,
    /// 记忆召回（联想记忆网络：对齐 Web UI `session.event.memory_recall`；
    /// 仅在有命中时发布，前端据「有无事件」决定是否构建筑）
    MemoryRecall,
    /// 回合保活心跳（Rust 引擎侧；对齐 Web UI `session.event.run_heartbeat`）
    RunHeartbeat,
}

impl EventType {
    /// 类型对应的主题段（snake_case）
    pub fn as_str(&self) -> &'static str {
        match self {
            EventType::Thinking => "thinking",
            EventType::ToolCall => "tool_call",
            EventType::ToolResult => "tool_result",
            EventType::Plan => "plan",
            EventType::Approval => "approval",
            EventType::Progress => "progress",
            EventType::MemoryRef => "memory_ref",
            EventType::Error => "error",
            EventType::Done => "done",
            EventType::Status => "status",
            EventType::RouterDecision => "router_decision",
            EventType::Compaction => "compaction",
            EventType::PlanRevision => "plan_revision",
            EventType::PlanRun => "plan_run",
            EventType::EnsembleProgress => "ensemble_progress",
            EventType::TextDelta => "text_delta",
            EventType::StepStart => "step_start",
            EventType::StepFinish => "step_finish",
            EventType::Usage => "usage",
            EventType::SessionsChanged => "sessions_changed",
            EventType::ArtifactUpdated => "artifact_updated",
            EventType::DesktopActionDone => "desktop_action_done",
            EventType::Escalation => "escalation",
            EventType::MemoryRecall => "memory_recall",
            EventType::RunHeartbeat => "run_heartbeat",
        }
    }
}

/// 并行集成推理进度负载（严格对齐 Web UI `EnsembleProgressPayload`）
///
/// 语义：每个并行「提议者」（proposer，如 Bloom 的 Captain / Copilot）在开始与
/// 结束时各发一帧；聚合阶段（aggregator）在合并前后各发一帧。前端
/// `useChatRouterDecisionRuntime` 按 `proposer_index` 增量合并成员条，
/// 因此 `proposer_start` 可只带 label，`proposer_finish` 再补齐 model / 用量。
///
/// 所有可选字段为 `None` 时不参与序列化，保证与前端 optional 字段契约一致。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct EnsembleProgress {
    /// 阶段：`proposer_start` / `proposer_finish` / `aggregator_start` / `aggregator_finish`
    pub event_type: String,
    /// 提议者序号（Captain=0、Copilot=1）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposer_index: Option<u32>,
    /// 提议者展示名（Captain / Copilot）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposer_label: Option<String>,
    /// 实际模型名（结束帧回填）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposer_model: Option<String>,
    /// 供应商标识
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposer_provider: Option<String>,
    /// 采样序号（多采样时使用）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_index: Option<u32>,
    /// 该阶段耗时（毫秒）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    /// 输入 token
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,
    /// 输出 token
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,
    /// 估算成本（美元）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// 该提议者失败时的错误摘要
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl EnsembleProgress {
    /// 提议者开始（并行发起时立即发，前端据此点亮成员条）
    pub fn proposer_start(index: u32, label: &str) -> Self {
        Self {
            event_type: "proposer_start".to_string(),
            proposer_index: Some(index),
            proposer_label: Some(label.to_string()),
            ..Default::default()
        }
    }

    /// 提议者开始（带模型名，前端立即显示模型而非等 finish 帧）
    pub fn proposer_start_with_model(index: u32, label: &str, model: &str, provider: &str) -> Self {
        Self {
            event_type: "proposer_start".to_string(),
            proposer_index: Some(index),
            proposer_label: Some(label.to_string()),
            proposer_model: Some(model.to_string()),
            proposer_provider: Some(provider.to_string()),
            ..Default::default()
        }
    }

    /// 提议者结束（回填模型名 / 用量 / 耗时）
    pub fn proposer_finish(index: u32, label: &str, model: &str, elapsed_ms: u64) -> Self {
        Self {
            event_type: "proposer_finish".to_string(),
            proposer_index: Some(index),
            proposer_label: Some(label.to_string()),
            proposer_model: Some(model.to_string()),
            elapsed_ms: Some(elapsed_ms),
            ..Default::default()
        }
    }

    /// 提议者失败（错误摘要随帧下发，前端标红该成员而不中断整轮）
    pub fn proposer_error(index: u32, label: &str, error: &str, elapsed_ms: u64) -> Self {
        Self {
            event_type: "proposer_finish".to_string(),
            proposer_index: Some(index),
            proposer_label: Some(label.to_string()),
            elapsed_ms: Some(elapsed_ms),
            error: Some(error.to_string()),
            ..Default::default()
        }
    }

    /// 聚合阶段（`start` / `finish`）
    pub fn aggregator(stage: &str, elapsed_ms: Option<u64>) -> Self {
        Self {
            event_type: format!("aggregator_{stage}"),
            elapsed_ms,
            ..Default::default()
        }
    }

    /// 附加 token 用量（链式）
    pub fn with_usage(mut self, input_tokens: u32, output_tokens: u32) -> Self {
        self.input_tokens = Some(input_tokens);
        self.output_tokens = Some(output_tokens);
        self
    }

    /// 附加供应商标识（链式）
    pub fn with_provider(mut self, provider: &str) -> Self {
        self.proposer_provider = Some(provider.to_string());
        self
    }
}

fn default_version() -> u8 {
    CURRENT_EVENT_VERSION
}

/// 事件信封
///
/// `v` 缺失时反序列化默认视为 [`CURRENT_EVENT_VERSION`]；
/// `v` 大于当前版本时 [`Envelope::from_sse`] 返回 [`EventError::UnsupportedVersion`]，
/// 消费方据此忽略并告警（前向兼容）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Envelope {
    /// 事件信封版本（默认 1，缺失视为 1）
    #[serde(default = "default_version")]
    pub v: u8,
    /// 事件类型
    pub r#type: EventType,
    /// 事件负载（任意 JSON）
    pub payload: serde_json::Value,
    /// 时间戳（UTC）
    pub ts: DateTime<Utc>,
    /// 会话标识（路由键第一段）
    pub session: String,
    /// 链路追踪 ID
    pub trace_id: String,
}

impl Envelope {
    /// 构造信封（版本=当前、时间戳=now、trace_id=随机）
    pub fn new(r#type: EventType, payload: serde_json::Value, session: impl Into<String>) -> Self {
        Self {
            v: CURRENT_EVENT_VERSION,
            r#type,
            payload,
            ts: Utc::now(),
            session: session.into(),
            trace_id: Uuid::new_v4().to_string(),
        }
    }

    /// 路由键：`{session}.{type}`，用于总线主题匹配
    pub fn route_key(&self) -> String {
        format!("{}.{}", self.session, self.r#type.as_str())
    }

    /// 序列化为 SSE 帧：`data: {json}\n\n`
    pub fn to_sse(&self) -> String {
        match serde_json::to_string(self) {
            Ok(body) => format!("data: {body}\n\n"),
            Err(_) => String::new(),
        }
    }

    /// 从 SSE 帧（或裸 JSON）反序列化
    ///
    /// 自动剥离 `data:` 前缀；`v` 缺失视为 1；
    /// `v` 超出当前版本返回 [`EventError::UnsupportedVersion`]。
    pub fn from_sse(s: &str) -> Result<Envelope, EventError> {
        let trimmed = s.trim();
        let body = trimmed
            .strip_prefix("data:")
            .map(str::trim)
            .unwrap_or(trimmed);
        let env: Envelope = serde_json::from_str(body)?;
        if env.v > CURRENT_EVENT_VERSION {
            tracing::warn!(
                v = env.v,
                current = CURRENT_EVENT_VERSION,
                "收到未来版本事件，已忽略（前向兼容）"
            );
            return Err(EventError::UnsupportedVersion(env.v, CURRENT_EVENT_VERSION));
        }
        Ok(env)
    }
}

/// 埋点构建器：为 mode / chain / sandbox 提供统一的事件发布接口
impl Envelope {
    /// 模式切换事件
    pub fn mode_changed(session: &str, from: &str, to: &str) -> Self {
        Envelope::new(
            EventType::Status,
            json!({ "kind": "mode.changed", "from": from, "to": to }),
            session,
        )
    }

    /// 模式切换事件（带权限预设溯源）：`permission_origin` 标注新模式的
    /// 权限预设来源（default / selection / inferred，对齐上游 PR #2608）。
    pub fn mode_changed_with_origin(session: &str, from: &str, to: &str, origin: &str) -> Self {
        Envelope::new(
            EventType::Status,
            json!({
                "kind": "mode.changed",
                "from": from,
                "to": to,
                "permission_origin": origin
            }),
            session,
        )
    }

    /// 工具调用事件
    pub fn tool_call(session: &str, tool: &str, arg_keys: &[&str]) -> Self {
        Envelope::new(
            EventType::ToolCall,
            json!({ "tool": tool, "args": arg_keys }),
            session,
        )
    }

    /// 子代理启动事件（携带任务描述，供前端卡片渲染）
    ///
    /// 事件类型沿用 ToolCall、工具名为 `expert`，`session` 即子代理 ID，
    /// 额外携带 `task` 字段用于卡片标题与角色推断。
    pub fn subagent_start(session: &str, task: &str) -> Self {
        Envelope::new(
            EventType::ToolCall,
            json!({ "tool": "expert", "task": task }),
            session,
        )
    }

    /// 子代理结束事件
    ///
    /// `ok` 标记成败，`summary` 为结果摘要；`session` 即子代理 ID。
    pub fn subagent_end(session: &str, ok: bool, summary: &str) -> Self {
        Envelope::new(
            EventType::ToolResult,
            json!({ "tool": "expert", "ok": ok, "summary": summary }),
            session,
        )
    }

    /// 工具结果事件
    pub fn tool_result(session: &str, tool: &str, ok: bool, summary: &str) -> Self {
        Envelope::new(
            EventType::ToolResult,
            json!({ "tool": tool, "ok": ok, "summary": summary }),
            session,
        )
    }

    /// 调用链完成事件
    pub fn chain_done(session: &str, chain_id: &str, duration_ms: u64, repaired: bool) -> Self {
        Envelope::new(
            EventType::Done,
            json!({ "chain_id": chain_id, "duration_ms": duration_ms, "repaired": repaired }),
            session,
        )
    }

    /// 进度播报事件
    pub fn progress(session: &str, percent: u8, message: &str) -> Self {
        Envelope::new(
            EventType::Progress,
            json!({ "percent": percent, "message": message }),
            session,
        )
    }

    /// 错误事件
    pub fn error(session: &str, code: &str, message: &str) -> Self {
        Envelope::new(
            EventType::Error,
            json!({ "code": code, "message": message }),
            session,
        )
    }

    /// 结构化错误事件（R65/B1）
    ///
    /// 在 [`Envelope::error`] 之上附加**可本地化插值**的结构化上下文字段
    /// （如计划步骤的 `step_id` / `step_title`）。前端按稳定 `code` 选文案模板、
    /// 用上下文插值，从而既人话化又不把内部术语拼进后端文案。
    ///
    /// `context` 必须是 JSON 对象；其中的键会并入 payload 顶层（不覆盖 `code` /
    /// `message`）。非对象 context 被忽略（保持载荷仍是合法错误帧）。
    pub fn error_structured(
        session: &str,
        code: &str,
        message: &str,
        context: serde_json::Value,
    ) -> Self {
        let mut payload = json!({ "code": code, "message": message });
        if let (Some(target), Some(extra)) = (payload.as_object_mut(), context.as_object()) {
            for (key, value) in extra {
                // 保护权威字段：code / message 由签名决定，上下文不得覆写。
                if key == "code" || key == "message" {
                    continue;
                }
                target.insert(key.clone(), value.clone());
            }
        }
        Envelope::new(EventType::Error, payload, session)
    }

    /// 并行集成推理进度事件（Bloom 双模型并行 → Web UI `session.event.ensemble_progress`）
    ///
    /// 负载序列化失败时降级为空对象，保证事件仍然抵达前端（不静默丢帧）。
    pub fn ensemble_progress(session: &str, progress: &EnsembleProgress) -> Self {
        let payload = serde_json::to_value(progress).unwrap_or_else(|_| json!({}));
        Envelope::new(EventType::EnsembleProgress, payload, session)
    }

    /// 用量实时事件（对齐 Web UI `session.event.usage`：每次 LLM 调用后即时推送，
    /// 替代前端 15s 轮询）。负载携带 model 与 token 用量。
    pub fn usage(
        session: &str,
        model: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
        total_tokens: u32,
    ) -> Self {
        Envelope::new(
            EventType::Usage,
            json!({
                "model": model,
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": total_tokens,
            }),
            session,
        )
    }

    /// 会话运行态变更事件（对齐 Web UI `sessions.changed`：turn 开始/结束触发列表刷新）。
    ///
    /// 负载携带 `run_status`（`running`/`idle`/`error`）与 `occasion`（`turn_start`/`turn_end`），
    /// 前端据此重算 `runStatus`/`lastActivityAt`/idle 并重排会话列表。
    pub fn sessions_changed(session: &str, run_status: &str, occasion: &str) -> Self {
        Envelope::new(
            EventType::SessionsChanged,
            json!({
                "session_key": session,
                "key": session,
                "run_status": run_status,
                "occasion": occasion,
            }),
            session,
        )
    }

    /// 回合保活心跳：长回合期间周期性发布，证明「这一轮还活着」。
    ///
    /// 为什么必须在 Rust 侧发：前端的 `session.event.run_heartbeat` 消费与
    /// `DEFAULT_STREAM_IDLE_TIMEOUT_MS`（630s 收不到任何帧即判死并掐断本轮）是配套设计，
    /// 而此事件此前**只有 Python 侧车发**。侧车离线/未安装时由 Rust 跑回合，长工具执行或
    /// 长思考期间一个帧都不发 → 健康的回合在 10.5 分钟后被前端误杀。
    ///
    /// `phase` 取 `"turn"`：相位无关的保活（不是阶段跃迁），前端只据此刷新存活时间，
    /// **不覆盖**当前活动文案。
    pub fn run_heartbeat(session: &str, task_id: &str, phase: &str) -> Self {
        Envelope::new(
            EventType::RunHeartbeat,
            json!({
                "session_key": session,
                "task_id": task_id,
                "phase": phase,
            }),
            session,
        )
    }

    /// 产物登记/更新事件（Epic A R2：Web UI 顶层事件 `artifact.updated`）。
    ///
    /// 负载 `{id, session_key, kind, path, ts}` 与 `file_change::record_artifact_with_session`
    /// 的登记回执对齐，前端据此刷新「文件更改卡片 / 产物面板」。
    pub fn artifact_updated(
        session: &str,
        id: &str,
        session_key: &str,
        kind: &str,
        path: &str,
        ts: i64,
    ) -> Self {
        Envelope::new(
            EventType::ArtifactUpdated,
            json!({
                "id": id,
                "session_key": session_key,
                "kind": kind,
                "path": path,
                "ts": ts,
            }),
            session,
        )
    }

    /// 桌面动作完成事件（Epic C：Web UI 顶层事件 `desktop.action_done`）。
    ///
    /// 负载 `{handle, action, receipt, ts}`，`receipt` 为动作回执（每步经 Recorder 落盘）。
    pub fn desktop_action_done(
        session: &str,
        handle: &str,
        action: &str,
        receipt: &Value,
        ts: i64,
    ) -> Self {
        Envelope::new(
            EventType::DesktopActionDone,
            json!({
                "handle": handle,
                "action": action,
                "receipt": receipt,
                "ts": ts,
            }),
            session,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn sample() -> Envelope {
        Envelope::new(
            EventType::ToolResult,
            json!({ "tool": "edit_file", "ok": true, "nested": { "a": [1, 2, 3] } }),
            "ws.s1",
        )
    }

    #[test]
    fn test_e1_envelope_roundtrip_sse() {
        let env = sample();
        let sse = env.to_sse();
        let back = Envelope::from_sse(&sse).expect("roundtrip");
        assert_eq!(env, back);
    }

    #[test]
    fn test_e1b_roundtrip_json() {
        let env = sample();
        let s = serde_json::to_string(&env).unwrap();
        let back: Envelope = serde_json::from_str(&s).unwrap();
        assert_eq!(env, back);
    }

    /// R65/B1：结构化错误帧把上下文并入 payload 顶层，且不改写 code / message。
    #[test]
    fn test_error_structured_merges_context_without_clobbering() {
        let env = Envelope::error_structured(
            "ws.s1",
            "PLAN_STEP_FAILED",
            "Plan step step-2 failed.",
            json!({ "step_id": "step-2", "step_title": "生成 HTML", "code": "IGNORED" }),
        );
        assert_eq!(env.r#type, EventType::Error);
        let p = &env.payload;
        assert_eq!(p["code"], "PLAN_STEP_FAILED");
        assert_eq!(p["message"], "Plan step step-2 failed.");
        assert_eq!(p["step_id"], "step-2");
        assert_eq!(p["step_title"], "生成 HTML");
    }

    /// 非对象 context 被安全忽略（载荷仍为合法错误帧，不 panic）。
    #[test]
    fn test_error_structured_ignores_non_object_context() {
        let env = Envelope::error_structured("s", "OTHER", "msg", json!("not-an-object"));
        assert_eq!(env.payload["code"], "OTHER");
        assert_eq!(env.payload["message"], "msg");
    }

    #[test]
    fn test_e2_v_missing_defaults_to_current() {
        let json = r#"{"type":"status","payload":{},"ts":"2026-08-04T00:00:00Z","session":"core","trace_id":"t1"}"#;
        let env = Envelope::from_sse(json).expect("v 缺失应默认 1");
        assert_eq!(env.v, CURRENT_EVENT_VERSION);
    }

    #[test]
    fn test_e2b_v_future_returns_error() {
        let json = r#"{"v":99,"type":"status","payload":{},"ts":"2026-08-04T00:00:00Z","session":"core","trace_id":"t1"}"#;
        let err = Envelope::from_sse(json).expect_err("未来版本应报错");
        assert_eq!(
            err,
            EventError::UnsupportedVersion(99, CURRENT_EVENT_VERSION)
        );
    }

    #[test]
    fn test_e2c_v_current_ok() {
        let json = r#"{"v":1,"type":"status","payload":{},"ts":"2026-08-04T00:00:00Z","session":"core","trace_id":"t1"}"#;
        let env = Envelope::from_sse(json).expect("当前版本应成功");
        assert_eq!(env.v, 1);
    }

    #[test]
    fn test_to_sse_format() {
        let sse = sample().to_sse();
        assert!(sse.starts_with("data: "));
        assert!(sse.ends_with("\n\n"));
        // 帧体可被独立解析
        let body = sse.trim_start_matches("data:").trim();
        let _: Value = serde_json::from_str(body).unwrap();
    }

    #[test]
    fn test_from_sse_strips_prefix_and_raw() {
        let env = sample();
        let framed = env.to_sse();
        let back = Envelope::from_sse(&framed).unwrap();
        assert_eq!(env, back);

        // 裸 JSON（无 data: 前缀）也能解析
        let raw = serde_json::to_string(&env).unwrap();
        let back2 = Envelope::from_sse(&raw).unwrap();
        assert_eq!(env, back2);
    }

    #[test]
    fn test_event_type_serde_snake_case() {
        let s = serde_json::to_string(&EventType::ToolResult).unwrap();
        assert_eq!(s, "\"tool_result\"");
        let back: EventType = serde_json::from_str("\"tool_result\"").unwrap();
        assert_eq!(back, EventType::ToolResult);
        let all = [
            (EventType::Thinking, "thinking"),
            (EventType::ToolCall, "tool_call"),
            (EventType::ToolResult, "tool_result"),
            (EventType::Plan, "plan"),
            (EventType::Approval, "approval"),
            (EventType::Progress, "progress"),
            (EventType::MemoryRef, "memory_ref"),
            (EventType::Error, "error"),
            (EventType::Done, "done"),
            (EventType::Status, "status"),
            (EventType::RouterDecision, "router_decision"),
            (EventType::Compaction, "compaction"),
            (EventType::PlanRevision, "plan_revision"),
            (EventType::PlanRun, "plan_run"),
            (EventType::EnsembleProgress, "ensemble_progress"),
            (EventType::TextDelta, "text_delta"),
            (EventType::StepStart, "step_start"),
            (EventType::StepFinish, "step_finish"),
            (EventType::Usage, "usage"),
            (EventType::SessionsChanged, "sessions_changed"),
            (EventType::ArtifactUpdated, "artifact_updated"),
            (EventType::DesktopActionDone, "desktop_action_done"),
            (EventType::MemoryRecall, "memory_recall"),
            (EventType::RunHeartbeat, "run_heartbeat"),
        ];
        for (ty, name) in all {
            assert_eq!(ty.as_str(), name);
            assert_eq!(serde_json::to_string(&ty).unwrap(), format!("\"{name}\""));
        }
    }

    #[test]
    fn test_new_sets_defaults() {
        let env = Envelope::new(EventType::Thinking, json!({}), "ws.s1");
        assert_eq!(env.v, CURRENT_EVENT_VERSION);
        assert!(!env.trace_id.is_empty());
        let ago = Utc::now() - chrono::Duration::seconds(5);
        assert!(env.ts >= ago, "ts 应接近当前时间");
    }

    #[test]
    fn test_roundtrip_with_nested_payload() {
        let env = Envelope::new(
            EventType::Plan,
            json!({
                "steps": [
                    { "id": "s1", "tool": "read_file" },
                    { "id": "s2", "tool": "edit_file", "deps": ["s1"] }
                ],
                "meta": { "n": 42, "flag": true }
            }),
            "ws.s2",
        );
        let back = Envelope::from_sse(&env.to_sse()).unwrap();
        assert_eq!(env, back);
    }

    #[test]
    fn test_route_key_format() {
        let env = Envelope::new(EventType::ToolCall, json!({}), "ws.s1");
        assert_eq!(env.route_key(), "ws.s1.tool_call");
    }

    #[test]
    fn test_ensemble_progress_builder_matches_webui_contract() {
        // 事件名必须精确等于 ensemble_progress —— 网关按 `session.event.{type}`
        // 组帧，前端 useChatRouterDecisionRuntime 只认这一个事件名。
        let p = EnsembleProgress::proposer_start(0, "Captain");
        let env = Envelope::ensemble_progress("ws.s1", &p);
        assert_eq!(env.r#type, EventType::EnsembleProgress);
        assert_eq!(env.route_key(), "ws.s1.ensemble_progress");
        assert_eq!(env.payload["event_type"], "proposer_start");
        assert_eq!(env.payload["proposer_index"], 0);
        assert_eq!(env.payload["proposer_label"], "Captain");
        // 未设置的可选字段不得出现（对齐前端 optional 契约）
        assert!(env.payload.get("elapsed_ms").is_none());
        assert!(env.payload.get("error").is_none());
    }

    #[test]
    fn test_ensemble_progress_finish_carries_model_and_usage() {
        let p = EnsembleProgress::proposer_finish(1, "Copilot", "deepseek-chat", 42)
            .with_usage(11, 22)
            .with_provider("deepseek");
        let env = Envelope::ensemble_progress("ws.s1", &p);
        assert_eq!(env.payload["event_type"], "proposer_finish");
        assert_eq!(env.payload["proposer_index"], 1);
        assert_eq!(env.payload["proposer_model"], "deepseek-chat");
        assert_eq!(env.payload["proposer_provider"], "deepseek");
        assert_eq!(env.payload["elapsed_ms"], 42);
        assert_eq!(env.payload["input_tokens"], 11);
        assert_eq!(env.payload["output_tokens"], 22);
    }

    #[test]
    fn test_ensemble_progress_error_and_aggregator_stages() {
        let e = EnsembleProgress::proposer_error(1, "Copilot", "connection refused", 7);
        assert_eq!(
            e.event_type, "proposer_finish",
            "失败也算结束帧，前端才会收敛该成员"
        );
        assert_eq!(e.error.as_deref(), Some("connection refused"));

        let start = EnsembleProgress::aggregator("start", None);
        let finish = EnsembleProgress::aggregator("finish", Some(9));
        assert_eq!(start.event_type, "aggregator_start");
        assert_eq!(finish.event_type, "aggregator_finish");
        assert_eq!(finish.elapsed_ms, Some(9));
    }

    #[test]
    fn test_ensemble_progress_envelope_roundtrip() {
        let p = EnsembleProgress::proposer_finish(0, "Captain", "m", 5).with_usage(1, 2);
        let env = Envelope::ensemble_progress("ws.s9", &p);
        let back = Envelope::from_sse(&env.to_sse()).expect("roundtrip");
        assert_eq!(env, back);
        let decoded: EnsembleProgress = serde_json::from_value(back.payload).unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn test_mode_changed_builder() {
        let env = Envelope::mode_changed("core", "agent", "bloom");
        assert_eq!(env.r#type, EventType::Status);
        assert_eq!(env.session, "core");
        assert_eq!(env.payload["kind"], "mode.changed");
        assert_eq!(env.payload["from"], "agent");
        assert_eq!(env.payload["to"], "bloom");
    }
}
