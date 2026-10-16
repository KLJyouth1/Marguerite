//! 升级路径（M2-C4）
//!
//! 定义「何时停下问人」：高影响副作用 / 意图歧义 / 超时 / 成本超预算 四场景
//! 由护栏模块输出结构化 [`EscalationEvent`]，事件总线发布
//! `session.event.escalation`（route_key = `{session}.escalation`），
//! 两端（GUI / Web）渲染「需要确认」而非静默继续。
//!
//! 设计原则：
//! - 判定（`evaluate`）与发布（`publish`）解耦：core 模块可就地判级，
//!   发布仅在有总线上下文时进行，CLI / 单测环境 no-op。
//! - escalation 是「先于执行」的护栏：默认 fail-closed——命中即停下等待确认，
//!   绝不静默放行。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

/// 升级触发场景（四类全集）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscalationTrigger {
    /// 高影响副作用（删除/覆盖/批量操作/发信等，需人工背书）
    HighImpactSideEffect,
    /// 意图歧义（无法从上下文可靠推断用户意图）
    IntentAmbiguity,
    /// 超时（操作超时，fail-closed 停下问人）
    Timeout,
    /// 成本超预算（预估/已消耗超过预算阈值）
    BudgetExceeded,
}

impl EscalationTrigger {
    pub fn as_str(&self) -> &'static str {
        match self {
            EscalationTrigger::HighImpactSideEffect => "high_impact_side_effect",
            EscalationTrigger::IntentAmbiguity => "intent_ambiguity",
            EscalationTrigger::Timeout => "timeout",
            EscalationTrigger::BudgetExceeded => "budget_exceeded",
        }
    }
}

/// 结构化升级事件（护栏输出，事件总线透传）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscalationEvent {
    /// 全局唯一升级 ID（跨端确认引用）
    pub id: String,
    /// 所属会话键
    pub session: String,
    /// 触发场景
    pub trigger: EscalationTrigger,
    /// 严重度（1-5，数值越大越需人工介入）
    pub severity: u8,
    /// 人类可读原因（用于两端渲染「需要确认」）
    pub reason: String,
    /// 结构化上下文（操作目标、成本、耗时等，供确认卡展示）
    pub context: serde_json::Value,
    /// 建议处置（continue / pause / abort）
    pub suggested_action: String,
    /// 触发时间（RFC3339）
    pub created_at: String,
}

impl EscalationEvent {
    pub fn new(
        session: impl Into<String>,
        trigger: EscalationTrigger,
        severity: u8,
        reason: impl Into<String>,
        context: serde_json::Value,
        suggested_action: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            session: session.into(),
            trigger,
            severity: severity.clamp(1, 5),
            reason: reason.into(),
            context,
            suggested_action: suggested_action.into(),
            created_at: Utc::now().to_rfc3339(),
        }
    }

    /// 事件种类标识（事件流 `kind` 字段）
    pub const KIND: &'static str = "escalation";

    /// 序列化为事件总线 payload（对齐 websocket 事件信封约定：
    /// `{ session_key, kind, escalation: {...} }`）
    pub fn to_payload(&self) -> serde_json::Value {
        json!({
            "kind": Self::KIND,
            "escalation": self,
        })
    }
}

/// 经事件总线发布升级事件（route_key = `{session}.escalation`）。
///
/// 判定（`evaluate`）与发布解耦：本函数仅在实际持有总线时才被调用，
/// CLI / 单测环境直接判级无需发布。payload 对齐 WebUI 事件信封约定：
/// `session.event.escalation`（订阅侧见 envelope.rs 的 `Escalation` 枚举）。
pub fn publish_to(bus: &crate::events::EventBus, event: &EscalationEvent) {
    use crate::events::{Envelope, EventType};
    let mut payload = event.to_payload();
    payload["session_key"] = json!(event.session);
    payload["key"] = json!(event.session);
    bus.publish(Envelope::new(EventType::Escalation, payload, &event.session));
}

/// 升级判定策略（可配置阈值）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationPolicy {
    /// 成本预算上限（美元/次任务），超出触发 BudgetExceeded
    #[serde(default = "default_cost_budget")]
    pub cost_budget_usd: f64,
    /// 单步超时阈值（秒），超出触发 Timeout
    #[serde(default = "default_step_timeout")]
    pub step_timeout_secs: u64,
    /// 高影响操作兜底开关（默认开启 fail-closed）
    #[serde(default = "default_true")]
    pub guard_high_impact: bool,
}

const fn default_cost_budget() -> f64 {
    5.0
}
const fn default_step_timeout() -> u64 {
    120
}
const fn default_true() -> bool {
    true
}

impl Default for EscalationPolicy {
    fn default() -> Self {
        Self {
            cost_budget_usd: 5.0,
            step_timeout_secs: 120,
            guard_high_impact: true,
        }
    }
}

/// 判定输入（由各调用方组装后交给 [`EscalationPolicy::evaluate`]）
#[derive(Debug, Clone)]
pub struct EscalationSignal {
    /// 会话键
    pub session: String,
    /// 操作描述（用于 reason / context）
    pub operation: String,
    /// 是否高影响副作用（删除/覆盖/批量等）
    pub high_impact: bool,
    /// 是否意图歧义
    pub ambiguous: bool,
    /// 当前已耗时
    pub elapsed: Duration,
    /// 已消耗成本（美元）
    pub cost_usd: f64,
}

/// 判级结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscalationDecision {
    /// 无需升级，继续执行
    Proceed,
    /// 需要升级，停下问人
    NeedsEscalation,
}

impl EscalationPolicy {
    /// 单次信号判级：命中任一场景即返回升级（fail-closed，可同时给出多条 reason）。
    /// 返回 `Vec<EscalationTrigger>` 为空表示放行。
    pub fn evaluate_triggers(&self, signal: &EscalationSignal) -> Vec<EscalationTrigger> {
        let mut triggers = Vec::new();
        if self.guard_high_impact && signal.high_impact {
            triggers.push(EscalationTrigger::HighImpactSideEffect);
        }
        if signal.ambiguous {
            triggers.push(EscalationTrigger::IntentAmbiguity);
        }
        if signal.elapsed.as_secs() >= self.step_timeout_secs {
            triggers.push(EscalationTrigger::Timeout);
        }
        if signal.cost_usd >= self.cost_budget_usd {
            triggers.push(EscalationTrigger::BudgetExceeded);
        }
        triggers
    }

    /// 生成升级事件；无命中时返回 `Proceed`。
    pub fn evaluate(&self, signal: &EscalationSignal) -> (EscalationDecision, Option<EscalationEvent>) {
        let triggers = self.evaluate_triggers(signal);
        if triggers.is_empty() {
            return (EscalationDecision::Proceed, None);
        }
        // 取最高严重度场景作为主触发，其余并入 context。
        let primary = triggers[0];
        let severity = match primary {
            EscalationTrigger::HighImpactSideEffect => 5,
            EscalationTrigger::IntentAmbiguity => 4,
            EscalationTrigger::Timeout => 3,
            EscalationTrigger::BudgetExceeded => 2,
        };
        let reason = build_reason(primary, signal, self.cost_budget_usd);
        let context = json!({
            "operation": signal.operation,
            "triggers": triggers.iter().map(|t| t.as_str()).collect::<Vec<_>>(),
            "high_impact": signal.high_impact,
            "ambiguous": signal.ambiguous,
            "elapsed_secs": signal.elapsed.as_secs(),
            "cost_usd": signal.cost_usd,
            "step_timeout_secs": self.step_timeout_secs,
            "cost_budget_usd": self.cost_budget_usd,
        });
        let suggested_action = if matches!(
            primary,
            EscalationTrigger::HighImpactSideEffect | EscalationTrigger::IntentAmbiguity
        ) {
            "confirm"
        } else {
            "confirm_or_abort"
        };
        let event = EscalationEvent::new(
            &signal.session,
            primary,
            severity,
            reason,
            context,
            suggested_action,
        );
        (EscalationDecision::NeedsEscalation, Some(event))
    }
}

fn build_reason(
    primary: EscalationTrigger,
    signal: &EscalationSignal,
    cost_budget_usd: f64,
) -> String {
    match primary {
        EscalationTrigger::HighImpactSideEffect => format!(
            "操作「{}」具有高影响副作用（删除/覆盖/批量等），需要确认后执行",
            signal.operation
        ),
        EscalationTrigger::IntentAmbiguity => format!(
            "操作「{}」存在意图歧义，无法可靠推断用户意图，需确认后继续",
            signal.operation
        ),
        EscalationTrigger::Timeout => format!(
            "操作「{}」耗时 {:.1}s，超过护栏阈值 {:.0}s，需确认是否继续等待",
            signal.operation,
            signal.elapsed.as_secs_f64(),
            signal.elapsed.as_secs(),
        ),
        EscalationTrigger::BudgetExceeded => format!(
            "操作「{}」预计成本 ${:.2} 超出预算上限 ${:.2}，需确认是否继续",
            signal.operation, signal.cost_usd, cost_budget_usd,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_signal() -> EscalationSignal {
        EscalationSignal {
            session: "sess-test".into(),
            operation: "批量删除 tmp 文件".into(),
            high_impact: false,
            ambiguous: false,
            elapsed: Duration::ZERO,
            cost_usd: 0.0,
        }
    }

    #[test]
    fn c4_high_impact_side_effect_escalates_and_pauses() {
        let policy = EscalationPolicy::default();
        let mut signal = default_signal();
        signal.high_impact = true;
        let (decision, event) = policy.evaluate(&signal);
        assert_eq!(decision, EscalationDecision::NeedsEscalation);
        let event = event.expect("must emit escalation");
        assert_eq!(event.trigger, EscalationTrigger::HighImpactSideEffect);
        assert_eq!(event.severity, 5);
        assert_eq!(event.suggested_action, "confirm");
        // 结构化 payload 可经事件总线透传（含 session 路由字段）
        let payload = event.to_payload();
        assert_eq!(payload["kind"], "escalation");
        assert_eq!(event.session, "sess-test");
    }

    #[test]
    fn c4_intent_ambiguity_escalates() {
        let policy = EscalationPolicy::default();
        let mut signal = default_signal();
        signal.ambiguous = true;
        let (decision, event) = policy.evaluate(&signal);
        assert_eq!(decision, EscalationDecision::NeedsEscalation);
        let event = event.unwrap();
        assert_eq!(event.trigger, EscalationTrigger::IntentAmbiguity);
        assert_eq!(event.severity, 4);
        assert!(event.reason.contains("意图歧义"));
    }

    #[test]
    fn c4_timeout_escalates_after_threshold() {
        let policy = EscalationPolicy::default();
        let mut signal = default_signal();
        signal.elapsed = Duration::from_secs(119);
        assert_eq!(
            policy.evaluate(&signal).0,
            EscalationDecision::Proceed,
            "阈值内放行"
        );
        signal.elapsed = Duration::from_secs(120);
        let (decision, event) = policy.evaluate(&signal);
        assert_eq!(decision, EscalationDecision::NeedsEscalation);
        assert_eq!(event.unwrap().trigger, EscalationTrigger::Timeout);
    }

    #[test]
    fn c4_budget_exceeded_escalates() {
        let policy = EscalationPolicy {
            cost_budget_usd: 1.0,
            ..Default::default()
        };
        let mut signal = default_signal();
        signal.cost_usd = 0.99;
        assert_eq!(
            policy.evaluate(&signal).0,
            EscalationDecision::Proceed,
            "预算内放行"
        );
        signal.cost_usd = 1.0;
        let (decision, event) = policy.evaluate(&signal);
        assert_eq!(decision, EscalationDecision::NeedsEscalation);
        let event = event.unwrap();
        assert_eq!(event.trigger, EscalationTrigger::BudgetExceeded);
        assert_eq!(event.context["cost_usd"], 1.0);
        assert_eq!(event.suggested_action, "confirm_or_abort");
        // reason 中的 cost 应取 signal.cost_usd，预算上限取 policy.cost_budget_usd（不得复用 cost）
        let r = event.reason.clone();
        assert!(
            r.contains("$1.00") && r.contains("$1.00"),
            "budget 阈值显示错误, reason={}",
            r
        );
        // 再验证 cost 与预算不同时能正确区分显示
        signal.cost_usd = 2.5;
        let (_, event) = policy.evaluate(&signal);
        let r = event.unwrap().reason;
        assert!(r.contains("$2.50"), "cost 显示错误, reason={}", r);
        assert!(r.contains("$1.00"), "预算上限显示错误, reason={}", r);
    }

    #[test]
    fn c4_multi_trigger_keeps_primary_and_aggregates_context() {
        let policy = EscalationPolicy::default();
        let mut signal = default_signal();
        signal.high_impact = true;
        signal.ambiguous = true;
        signal.cost_usd = 6.0; // 超默认预算 5$
        let (_, event) = policy.evaluate(&signal);
        let event = event.unwrap();
        assert_eq!(event.trigger, EscalationTrigger::HighImpactSideEffect);
        let triggers: Vec<&str> = event
            .context["triggers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(triggers.len(), 3);
        assert!(triggers.contains(&"high_impact_side_effect"));
        assert!(triggers.contains(&"intent_ambiguity"));
        assert!(triggers.contains(&"budget_exceeded"));
    }

    #[test]
    fn c4_fail_closed_never_silently_proceeds_on_any_scenario() {
        // 任一场景命中都不得静默放行（fail-closed 语义）
        let policy = EscalationPolicy::default();
        let cases: [(bool, bool, u64, f64); 4] = [
            (true, false, 0, 0.0),
            (false, true, 0, 0.0),
            (false, false, 120, 0.0),
            (false, false, 0, 5.0),
        ];
        for (hi, amb, secs, cost) in cases {
            let signal = EscalationSignal {
                session: "sess".into(),
                operation: "op".into(),
                high_impact: hi,
                ambiguous: amb,
                elapsed: Duration::from_secs(secs),
                cost_usd: cost,
            };
            let (decision, _) = policy.evaluate(&signal);
            assert_eq!(decision, EscalationDecision::NeedsEscalation);
        }
    }

    /// C4 真实事件总线链路：四场景各发布一次 `session.event.escalation`，
    /// 订阅端（模拟另一端）必须在 <200ms 内收到结构化 escalation 事件，
    /// 且 payload 携带 session 路由字段与完整确认信息（跨端渲染「需要确认」）。
    #[tokio::test]
    async fn c4_four_scenarios_publish_over_real_bus_under_200ms() {
        use crate::events::{EventBus, EventType};
        let bus = std::sync::Arc::new(EventBus::default());
        // 订阅端模拟「另一端」：只订阅本 session 的 escalation 路由
        // （EventBus route_key = `{session}.{type}` => `sess.escalation`）。
        let mut rx = bus.subscribe("sess.escalation");
        let policy = EscalationPolicy::default();
        // 四场景确定性构造（任一命中即升级）
        let signals = [
            EscalationSignal {
                session: "sess".into(),
                operation: "批量删除 tmp".into(),
                high_impact: true,
                ambiguous: false,
                elapsed: Duration::ZERO,
                cost_usd: 0.0,
            },
            EscalationSignal {
                session: "sess".into(),
                operation: "分析报告的意图".into(),
                high_impact: false,
                ambiguous: true,
                elapsed: Duration::ZERO,
                cost_usd: 0.0,
            },
            EscalationSignal {
                session: "sess".into(),
                operation: "慢查询".into(),
                high_impact: false,
                ambiguous: false,
                elapsed: Duration::from_secs(300),
                cost_usd: 0.0,
            },
            EscalationSignal {
                session: "sess".into(),
                operation: "大规模训练".into(),
                high_impact: false,
                ambiguous: false,
                elapsed: Duration::ZERO,
                cost_usd: 7.5,
            },
        ];
        for signal in signals {
            let (decision, event) = policy.evaluate(&signal);
            assert_eq!(decision, EscalationDecision::NeedsEscalation);
            let event = event.expect("must emit escalation");
            publish_to(&bus, &event);
            let t0 = std::time::Instant::now();
            let env = tokio::time::timeout(Duration::from_millis(500), rx.recv())
                .await
                .expect("应收到 escalation 事件")
                .expect("channel 不应关闭");
            assert!(
                t0.elapsed().as_millis() < 200,
                "发布→订阅延迟应 <200ms，实际 {}ms",
                t0.elapsed().as_millis()
            );
            assert_eq!(env.r#type, EventType::Escalation);
            // payload 信封：route/session 字段 + 结构化 escalation 确认信息
            assert_eq!(env.payload["session_key"], "sess");
            assert_eq!(env.payload["key"], "sess");
            assert_eq!(env.payload["kind"], "escalation");
            let esc = &env.payload["escalation"];
            assert_eq!(esc["session"], "sess");
            assert!(esc["id"].as_str().unwrap().len() >= 16, "应有全局唯一 id");
            assert!(esc["reason"].as_str().unwrap().len() >= 4);
            assert!(esc["suggested_action"].as_str().is_some());
            assert!(esc["created_at"].as_str().is_some());
        }
    }

    /// C4 跨端确认闭环：发布 → 注册进全局确认桥 → 任一端按 escalation_id
    /// 决策（继续/中止）→ 挂起表消费；未决决 ID 返回 NotFound。
    #[tokio::test]
    async fn c4_registry_bridge_resolves_by_id_across_hosts() {
        use crate::events::EventBus;
        use crate::guard::escalation_registry::{
            reset_global, global_registry, EscalationAck, EscalationRegistry, EscalationResolution,
        };
        let bus = std::sync::Arc::new(EventBus::default());
        let mut rx = bus.subscribe("sess.escalation");
        let policy = EscalationPolicy::default();
        // 模拟「发布端」生成事件并广播
        let signal = EscalationSignal {
            session: "sess".into(),
            operation: "处理用户上传".into(),
            high_impact: true,
            ambiguous: false,
            elapsed: Duration::ZERO,
            cost_usd: 0.0,
        };
        let (_decision, event) = policy.evaluate(&signal);
        let event = event.unwrap();
        publish_to(&bus, &event);
        let env = tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(env.r#type, crate::events::EventType::Escalation);
        // 「另一端」决策：注册 → 按 id 继续
        let reg = EscalationRegistry::new();
        let id = reg.register(&event);
        assert_eq!(reg.len(), 1);
        match reg.resolve(&id, EscalationResolution::Continue) {
            EscalationAck::Resolved(ev, r) => {
                assert_eq!(ev.id, id);
                assert_eq!(r, EscalationResolution::Continue);
            }
            _ => panic!("应解析成功"),
        }
        assert!(reg.is_empty());
        assert_eq!(
            reg.resolve(&id, EscalationResolution::Abort),
            EscalationAck::NotFound
        );
        // 全局单例同样可决策（生产宿主路径）
        reset_global();
        let gid = global_registry().register(&event);
        assert_eq!(global_registry().pending().len(), 1);
        assert_eq!(
            global_registry().resolve(&gid, EscalationResolution::Abort),
            EscalationAck::Resolved(event, EscalationResolution::Abort)
        );
        assert!(global_registry().is_empty());
        reset_global();
    }
}
