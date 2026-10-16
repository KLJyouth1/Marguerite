//! 升级确认注册表（M2-C4 跨端确认桥）
//!
//! escalation 事件发布后，前端渲染「需要确认」，用户选择「继续 / 中止」。
//! 与 [`super::approval_registry`] 的 `id → Arc<ApprovalGate>` 模式对齐，
//! 本注册表以 `id → EscalationEvent` 索引**挂起中**的升级事件，收口为进程级
//! 单例，使任一端（GUI / Web / TUI）拿到 escalation_id 都能完成决策，
//! 避免各宿主各自复制升级状态。决策（继续/中止）后即从挂起表移除，
//! 并记入历史，供 `escalation.pending` / 测试断言使用。

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

use super::escalation::EscalationEvent;

/// 用户对升级事件的处置决策
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscalationResolution {
    /// 人工确认，继续执行
    Continue,
    /// 中止执行（fail-closed 下与超时同样收敛为不执行）
    Abort,
}

impl EscalationResolution {
    pub fn as_str(&self) -> &'static str {
        match self {
            EscalationResolution::Continue => "continue",
            EscalationResolution::Abort => "abort",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "continue" | "proceed" | "ok" | "confirm" => Some(Self::Continue),
            "abort" | "cancel" | "stop" => Some(Self::Abort),
            _ => None,
        }
    }
}

/// 解析结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscalationAck {
    /// 已按指定决策结束该升级（从挂起表移除）
    Resolved(EscalationEvent, EscalationResolution),
    /// 未找到挂起升级（可能已消费或 ID 不存在）
    NotFound,
}

/// 全局升级注册表
pub struct EscalationRegistry {
    pending: Mutex<HashMap<String, EscalationEvent>>,
    /// 最近处置历史（id → 事件 + 决策），供跨进程断言与审计
    history: Mutex<VecDeque<(EscalationEvent, EscalationResolution)>>,
}

impl EscalationRegistry {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            history: Mutex::new(VecDeque::new()),
        }
    }

    /// 登记一条挂起升级（发布后调用）；返回 escalation ID（幂等：同 id 直接忽略）。
    pub fn register(&self, event: &EscalationEvent) -> String {
        let mut slots = self.pending.lock().unwrap();
        slots.insert(event.id.clone(), event.clone());
        event.id.clone()
    }

    /// 待确认升级清单（按登记序）
    pub fn pending(&self) -> Vec<EscalationEvent> {
        let mut list: Vec<_> = self.pending.lock().unwrap().values().cloned().collect();
        list.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        list
    }

    /// 挂起数量（badge 计数）
    pub fn len(&self) -> usize {
        self.pending.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 按 id 查加固有的 trigger / reason（供前端「需要确认」卡渲染）
    pub fn event_for(&self, id: &str) -> Option<EscalationEvent> {
        self.pending.lock().unwrap().get(id).cloned()
    }

    /// 决策：继续或中止。命中则从挂起表移除并记入历史；未命中返回 `NotFound`。
    pub fn resolve(&self, id: &str, resolution: EscalationResolution) -> EscalationAck {
        let Some(event) = self.pending.lock().unwrap().remove(id) else {
            return EscalationAck::NotFound;
        };
        self.history
            .lock()
            .unwrap()
            .push_back((event.clone(), resolution));
        EscalationAck::Resolved(event, resolution)
    }

    /// 最近 N 条处置历史（新→旧）
    pub fn recent_history(&self, n: usize) -> Vec<(EscalationEvent, EscalationResolution)> {
        let h = self.history.lock().unwrap();
        h.iter().rev().take(n).cloned().collect()
    }
}

impl Default for EscalationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// 进程级单例（供网关 RPC / 测试共用同一挂起表）
static GLOBAL: OnceLock<EscalationRegistry> = OnceLock::new();

pub fn global_registry() -> &'static EscalationRegistry {
    GLOBAL.get_or_init(EscalationRegistry::new)
}

/// 测试专用：重置全局挂起表（与 approval_registry 的测试隔离策略一致）
pub fn reset_global() {
    if let Some(r) = GLOBAL.get() {
        *r.pending.lock().unwrap() = HashMap::new();
        r.history.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::escalation::{EscalationEvent, EscalationTrigger};

    fn sample_event() -> EscalationEvent {
        EscalationEvent::new(
            "sess",
            EscalationTrigger::HighImpactSideEffect,
            5,
            "删除前需确认",
            serde_json::json!({ "operation": "rm -rf tmp" }),
            "confirm",
        )
    }

    #[test]
    fn register_and_pending_listing() {
        let reg = EscalationRegistry::new();
        let ev = sample_event();
        let id = reg.register(&ev);
        assert_eq!(reg.len(), 1);
        let evs = reg.pending();
        assert_eq!(evs[0].id, id);
        assert_eq!(evs[0].trigger, EscalationTrigger::HighImpactSideEffect);
    }

    #[test]
    fn resolve_continue_consumes_and_records_history() {
        let reg = EscalationRegistry::new();
        let ev = sample_event();
        let id = reg.register(&ev);
        assert_eq!(
            reg.resolve(&id, EscalationResolution::Continue),
            EscalationAck::Resolved(ev.clone(), EscalationResolution::Continue)
        );
        assert!(reg.is_empty(), "消费后应从挂起表移除");
        assert_eq!(reg.resolve(&id, EscalationResolution::Continue), EscalationAck::NotFound);
        let h = reg.recent_history(1);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].0.id, id);
        assert_eq!(h[0].1, EscalationResolution::Continue);
    }

    #[test]
    fn resolve_abort() {
        let reg = EscalationRegistry::new();
        let ev = sample_event();
        let id = reg.register(&ev);
        assert_eq!(
            reg.resolve(&id, EscalationResolution::Abort),
            EscalationAck::Resolved(ev, EscalationResolution::Abort)
        );
        assert!(reg.is_empty());
    }

    #[test]
    fn parse_resolution_words() {
        assert_eq!(EscalationResolution::parse("continue"), Some(EscalationResolution::Continue));
        assert_eq!(EscalationResolution::parse("abort"), Some(EscalationResolution::Abort));
        assert_eq!(EscalationResolution::parse("cancel"), Some(EscalationResolution::Abort));
        assert_eq!(EscalationResolution::parse("nonsense"), None);
    }

    #[test]
    fn unknown_id_not_found() {
        let reg = EscalationRegistry::new();
        assert!(reg.event_for("missing").is_none());
        assert_eq!(
            reg.resolve("missing", EscalationResolution::Continue),
            EscalationAck::NotFound
        );
    }

    #[test]
    fn registry_is_process_singleton() {
        reset_global();
        let a = global_registry();
        let b = global_registry();
        assert!(std::ptr::eq(a, b));
    }
}
