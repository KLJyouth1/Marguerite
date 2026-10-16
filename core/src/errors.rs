//! 结构化步骤错误（P1-5 / T05）：把「plan runner 该不该重试」从**中文子串嗅探**
//! 换成一个确定性的类别裁决。
//!
//! 背景：`plan::runner::is_retryable_step_error` 此前只看错误文本里是否含「治理门」
//! /「宪法」/反思门 marker。于是任何**恰好包含这些词**的瞬时错误（例如 LLM 输出的
//! 讨论里出现「治理门设计」随后网络超时）会被误判为「确定性裁决、不可重试」，
//! 白白放弃一次本可成功的退避重试。
//!
//! 本模块把「确定性裁决」（治理门 / 宪法 / 反思门禁）做成可 downcast 的类型
//! [`StepError`]，并给出 [`classify_step_error`]：优先按**类型**（StepError →
//! provider 归类）判定，只有都无法判定时才回退旧的子串嗅探（过渡期兜底 + warn）。
//!
//! 依赖方向：仅依赖 `std` / `anyhow` / `model::error_taxonomy`，被 `plan::runner`
//! 与 `orchestration` 单向调用。

use std::fmt;

use anyhow::Error as AnyhowError;

use crate::model::error_taxonomy::{provider_error_of, ProviderErrorClass};

/// 步骤错误的确定性类别（plan runner 重试裁决用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepErrorKind {
    /// 治理门（宪法裁决）：确定性裁决，不可重试。
    GovernanceGate,
    /// 宪法拦截：确定性裁决，不可重试。
    Constitution,
    /// 反思门禁失败：确定性裁决，不可重试。
    ReflectGate,
    /// provider 语义错误：是否可重试随内层类别（限流/5xx/网络/超时可重试）。
    Provider(ProviderErrorClass),
    /// 无法判定：调用方按兜底策略处理。
    Unknown,
}

/// 带确定性类别的步骤错误（`anyhow` 链可 downcast）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepError {
    kind: StepErrorKind,
    message: String,
}

impl StepError {
    pub fn new(kind: StepErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// 治理门（宪法裁决）确定的不可重试错误。
    pub fn governance_gate(message: impl Into<String>) -> Self {
        Self::new(StepErrorKind::GovernanceGate, message)
    }

    /// 反思门禁失败（不可重试）。
    pub fn reflect_gate(message: impl Into<String>) -> Self {
        Self::new(StepErrorKind::ReflectGate, message)
    }

    /// 宪法拦截（不可重试）。
    pub fn constitution(message: impl Into<String>) -> Self {
        Self::new(StepErrorKind::Constitution, message)
    }

    pub fn kind(&self) -> StepErrorKind {
        self.kind
    }
}

impl fmt::Display for StepError {
    /// 输出与旧实现逐字一致的文案（`src` 断言不受影响）。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for StepError {}

/// 治理门 / 宪法 / 反思门禁的**子串兜底**（过渡期）。
///
/// 仅当类型与 provider 归类都无法判定时使用；命中即不可重试。保留一个版本，
/// 待所有确定性错误产生点迁移到 [`StepError`] 后移除。
fn deterministic_kind_from_message(message: &str) -> Option<StepErrorKind> {
    if message.contains(crate::orchestration::REFLECT_GATE_MARKER) {
        return Some(StepErrorKind::ReflectGate);
    }
    if message.contains("治理门") {
        return Some(StepErrorKind::GovernanceGate);
    }
    if message.contains("宪法") {
        return Some(StepErrorKind::Constitution);
    }
    None
}

/// 归类一个步骤错误（`anyhow` 链）。
///
/// 判定优先级（关键：**类型优先于子串**，修掉「含『治理门』字样的瞬时错误被误判
/// 不可重试」的原缺陷）：
/// 1. 链中存在 [`StepError`] → 直接取其类别；
/// 2. 链中存在 provider 归类（[`ProviderErrorClass`]）→ `Provider(class)`；
/// 3. 均无 → 子串兜底；仍无 → `Unknown`。
pub fn classify_step_error(err: &AnyhowError) -> StepErrorKind {
    if let Some(step) = err.chain().find_map(|e| e.downcast_ref::<StepError>()) {
        return step.kind();
    }
    if let Some(provider) = provider_error_of(err) {
        // provider 语义优先于文本子串：链上任何一层（含 context）出现「治理门」
        // 等词都不影响瞬时/确定性判定。
        return StepErrorKind::Provider(provider.class());
    }
    if let Some(kind) = deterministic_kind_from_message(&err.to_string()) {
        tracing::warn!(
            kind = ?kind,
            "步骤错误未结构化，按子串兜底判定（待迁移到 StepError）"
        );
        return kind;
    }
    StepErrorKind::Unknown
}

/// 该步骤错误是否可退避重试。
///
/// - 治理门 / 宪法 / 反思门禁：确定性裁决 → **不可重试**；
/// - provider 归类：限流 / 5xx / 网络 / 超时 → 可重试；鉴权 / 其它 → 不可重试；
/// - 未知：可重试（与旧实现方向一致：旧实现只排除确定性裁决，其余放行重试）。
pub fn is_retryable_step_error(err: &AnyhowError) -> bool {
    match classify_step_error(err) {
        StepErrorKind::GovernanceGate
        | StepErrorKind::Constitution
        | StepErrorKind::ReflectGate => false,
        StepErrorKind::Provider(class) => matches!(
            class,
            ProviderErrorClass::RateLimited
                | ProviderErrorClass::Server
                | ProviderErrorClass::Network
                | ProviderErrorClass::Timeout
        ),
        StepErrorKind::Unknown => true,
    }
}

/// 仅凭文本判定可重试性（无 `anyhow` 链的调用方，如 `cache::selfcheck`）。
///
/// 与 [`is_retryable_step_error`] 的兜底口径一致：确定性裁决（治理门/宪法/反思门禁）
/// 不可重试，其余可重试。等价于旧实现 `!contains(...)`，行为不变。
pub fn is_retryable_step_error_message(message: &str) -> bool {
    deterministic_kind_from_message(message).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_deterministic_errors_are_not_retryable() {
        let gov = anyhow::Error::new(StepError::governance_gate("治理门裁决未通过（违反：C1）"));
        assert_eq!(classify_step_error(&gov), StepErrorKind::GovernanceGate);
        assert!(!is_retryable_step_error(&gov));

        let reflect = anyhow::Error::new(StepError::reflect_gate(format!(
            "{} 反思门禁未通过",
            crate::orchestration::REFLECT_GATE_MARKER
        )));
        assert_eq!(classify_step_error(&reflect), StepErrorKind::ReflectGate);
        assert!(!is_retryable_step_error(&reflect));
    }

    /// P1-5 验收核心：**外层 context 含「治理门」字样的瞬时网络错误**必须可重试
    /// （旧实现只 sniff `e.to_string()`＝最外层 context，会误判为不可重试）。
    #[test]
    fn transient_error_mentioning_governance_gate_is_retryable() {
        let transient = anyhow::Error::new(
            crate::model::error_taxonomy::ProviderError::new(
                ProviderErrorClass::Network,
                "raw transport failure",
            ),
        )
        .context("讨论「治理门」设计时连接超时：network error");
        // 关键：Display 里确实含「治理门」，但链上有 provider 归类 → 仍判瞬时。
        assert!(transient.to_string().contains("治理门"));
        assert_eq!(
            classify_step_error(&transient),
            StepErrorKind::Provider(ProviderErrorClass::Network),
            "provider 归类必须优先于子串嗅探"
        );
        assert!(
            is_retryable_step_error(&transient),
            "含「治理门」字样的瞬时网络错误必须可重试（原缺陷：被误判不可重试）"
        );
    }

    #[test]
    fn server_and_rate_limited_provider_errors_are_retryable() {
        for class in [
            ProviderErrorClass::RateLimited,
            ProviderErrorClass::Server,
            ProviderErrorClass::Timeout,
        ] {
            let err = anyhow::Error::new(
                crate::model::error_taxonomy::ProviderError::new(class, "transient"),
            );
            assert!(is_retryable_step_error(&err), "{class:?} 应可重试");
        }
        let auth = anyhow::Error::new(
            crate::model::error_taxonomy::ProviderError::new(
                ProviderErrorClass::AuthFailed,
                "bad key",
            ),
        );
        assert!(!is_retryable_step_error(&auth), "鉴权失败不应重试");
    }

    #[test]
    fn legacy_substring_fallback_still_blocks_unstructured_deterministic_errors() {
        // 尚未迁移的确定性错误产生点（纯字符串）仍按子串兜底判不可重试。
        let legacy = anyhow::anyhow!("治理门裁决未通过（违反：C1），需人工放行");
        assert_eq!(classify_step_error(&legacy), StepErrorKind::GovernanceGate);
        assert!(!is_retryable_step_error(&legacy));
    }

    #[test]
    fn unknown_errors_are_retryable_by_default() {
        let plain = anyhow::anyhow!("some internal hiccup");
        assert_eq!(classify_step_error(&plain), StepErrorKind::Unknown);
        assert!(is_retryable_step_error(&plain));
    }
}