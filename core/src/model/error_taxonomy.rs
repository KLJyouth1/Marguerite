//! 上游 provider 错误归类（P0-4：不把上游原始正文原样抛给用户）。
//!
//! 背景：此前 `chat_complete` 等把上游返回的**原始响应正文**直接拼进
//! `anyhow!("API error ({status}): {error_text}")`，一路透传到网关 `Error` 事件与前端。
//! DeepSeek 的鉴权失败正文形如 `Your api key: ****gEni is invalid`，于是用户界面
//! 直接看到了 `****gEni`（**上游回显的我方掩码片段，不是我方新增泄漏**，但不应作为
//! 主文案暴露给用户）。
//!
//! 本模块把 provider 失败**归类**为稳定错误码（[`ProviderErrorClass`]），
//! 用户可见文案用中性、可本地化的安全描述；原始正文只保留在 [`ProviderError::detail`]，
//! 供服务端日志（gateway 端 tracing）诊断，**绝不**成为主文案（[`ProviderError`] 的
//! `Display` 只输出 `code: safe_message`）。
//!
//! 可诊断性边界（如实声明）：前端**不展示**原始正文，只保留稳定 `errorCode`
//! 元数据；原始正文仅存在于服务端日志，供排障时查看。
//!
//! 依赖方向：本模块仅依赖 `std` 与 `anyhow`，被 `model::llm` / `model::router` /
//! `gateway` / `cli` 单向调用，不反向依赖它们（避免环路）。

use std::fmt;

use anyhow::Error as AnyhowError;

/// provider 错误类别 —— 每类映射一个稳定错误码（与前端 `localizedChatErrorMessage` 对齐）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorClass {
    /// 鉴权失败（HTTP 401/403）：密钥缺失、无效或过期。
    AuthFailed,
    /// 限流（HTTP 429）：请求过于频繁，稍后重试。
    RateLimited,
    /// 请求超时（连接/读取超时，无 HTTP 状态；由超时判定点显式归类）。
    Timeout,
    /// 网络层失败：连接失败 / 中途断开（无 HTTP 状态，且非超时）。
    Network,
    /// 服务端错误（HTTP 5xx）。
    Server,
    /// 其它未归类失败（如 400 请求体错误、4xx 非鉴权）。
    Other,
}

impl ProviderErrorClass {
    /// 稳定错误码（大写蛇形；前端按此后缀做本地化，不解析文案）。
    pub const fn code(self) -> &'static str {
        match self {
            Self::AuthFailed => "AUTH_FAILED",
            Self::RateLimited => "RATE_LIMITED",
            Self::Timeout => "TIMEOUT",
            Self::Network => "NETWORK",
            Self::Server => "SERVER",
            Self::Other => "OTHER",
        }
    }

    /// 由 HTTP 状态码归类（网络失败无状态码，由调用方直接给 [`Self::Network`]）。
    pub fn from_status(status: u16) -> Self {
        match status {
            401 | 403 => Self::AuthFailed,
            429 => Self::RateLimited,
            500..=599 => Self::Server,
            _ => Self::Other,
        }
    }
}

/// 归类后的 provider 错误。
///
/// - `message`：中性、可本地化的安全描述（**不含**上游原始正文）；
/// - `detail`：上游原始正文（**仅供服务端日志诊断**；前端不展示原始正文，只保留稳定
///   错误码），绝不作为主文案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    class: ProviderErrorClass,
    message: String,
    detail: String,
}

impl ProviderError {
    /// 由类别 + 原始正文构造（无已知状态码；安全文案不含状态码）。
    pub fn new(class: ProviderErrorClass, detail: impl Into<String>) -> Self {
        Self {
            class,
            message: safe_message(class, None),
            detail: detail.into(),
        }
    }

    /// 由 HTTP 状态码 + 原始正文构造（经 [`ProviderErrorClass::from_status`] 归类）。
    pub fn from_status(status: u16, detail: impl Into<String>) -> Self {
        let class = ProviderErrorClass::from_status(status);
        Self {
            class,
            message: safe_message(class, Some(status)),
            detail: detail.into(),
        }
    }

    /// 错误类别。
    pub fn class(&self) -> ProviderErrorClass {
        self.class
    }

    /// 稳定错误码（见 [`ProviderErrorClass::code`]）。
    pub fn code(&self) -> &'static str {
        self.class.code()
    }

    /// 中性安全文案（用户可见）。
    pub fn safe_message(&self) -> &str {
        &self.message
    }

    /// 上游原始正文（诊断用，不得作为用户主文案）。
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for ProviderError {
    /// **只**输出 `code: safe_message` —— 绝不含 `detail`（防上游正文回显）。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code(), self.message)
    }
}

impl std::error::Error for ProviderError {}

/// 中性安全文案（英文兜底，最终由前端按错误码本地化）。
fn safe_message(class: ProviderErrorClass, status: Option<u16>) -> String {
    let with_status = |base: &str| match status {
        Some(s) => format!("{base} (HTTP {s})"),
        None => base.to_string(),
    };
    match class {
        ProviderErrorClass::AuthFailed => with_status(
            "The model provider rejected the API key. Check the key in Settings and retry.",
        ),
        ProviderErrorClass::RateLimited => {
            with_status("The model provider rate-limited the request. Please retry shortly.")
        }
        ProviderErrorClass::Timeout => {
            "The model provider did not respond in time (request timed out).".to_string()
        }
        ProviderErrorClass::Network => {
            "Could not reach the model provider (network error or timeout).".to_string()
        }
        ProviderErrorClass::Server => {
            with_status("The model provider returned a server error. Please retry later.")
        }
        ProviderErrorClass::Other => with_status("The model provider request failed."),
    }
}

/// 从任意 `anyhow` 错误链中提取最内层 [`ProviderError`]（若存在）。
///
/// `anyhow` 的 `chain()` 会遍历 context 链，逐层 `downcast_ref`，
/// 因此即使错误经编排层 `?` 层层包裹，仍能取回原始归类。
pub fn provider_error_of(err: &AnyhowError) -> Option<&ProviderError> {
    err.chain().find_map(|e| e.downcast_ref::<ProviderError>())
}

/// 从任意 `anyhow` 错误链中提取稳定错误码（无归类时返回 `None`）。
pub fn code_of(err: &AnyhowError) -> Option<&'static str> {
    provider_error_of(err).map(ProviderError::code)
}

// ---------------------------------------------------------------------------
// 计划执行失败码（R65/B1）
//
// 背景（真机现象）：计划子步骤失败时，`plan::runner` 把**内部术语**串
// （`计划子步骤 step-2 执行失败：<内层错误>`）直接作为错误事件正文下发，前端原样
// 展示，用户看到的是编排实现细节而非可行动的说明。
//
// 修复原则（与 provider 错误归类同构）：后端只给出**稳定错误码 + 结构化上下文**
// （`step_id` / `step_title`），中性英文兜底文案；用户可见的本地化文案由前端按码
// 组装（`webui/src/utils/chat/errors.ts`），绝不把内部术语拼进主文案。
// ---------------------------------------------------------------------------

/// 计划子步骤执行失败（内层错误无法进一步归类时使用）。
pub const PLAN_STEP_FAILED: &str = "PLAN_STEP_FAILED";
/// 计划子步骤未产出其标题声明的交付物。
pub const PLAN_STEP_ARTIFACT_MISSING: &str = "PLAN_STEP_ARTIFACT_MISSING";
/// 计划聚合结论未通过完成门禁（四检查 / 产物契约未满足），终态降为 partial。
pub const PLAN_COMPLETION_BLOCKED: &str = "PLAN_COMPLETION_BLOCKED";

/// 计划步骤失败的稳定错误码：优先沿用内层已归类的 provider 码（如 `AUTH_FAILED`），
/// 否则回退到 [`PLAN_STEP_FAILED`]。前端据此选文案模板，而非解析内部文案。
pub fn plan_step_code(err: &AnyhowError) -> &'static str {
    code_of(err).unwrap_or(PLAN_STEP_FAILED)
}

/// 用户可见文案：有归类时用其安全文案；否则退回错误自身 `Display`（调用方决定兜底）。
pub fn user_message_of(err: &AnyhowError) -> Option<String> {
    provider_error_of(err).map(|p| p.safe_message().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_http_status_codes() {
        assert_eq!(
            ProviderErrorClass::from_status(401),
            ProviderErrorClass::AuthFailed
        );
        assert_eq!(
            ProviderErrorClass::from_status(403),
            ProviderErrorClass::AuthFailed
        );
        assert_eq!(
            ProviderErrorClass::from_status(429),
            ProviderErrorClass::RateLimited
        );
        assert_eq!(
            ProviderErrorClass::from_status(503),
            ProviderErrorClass::Server
        );
        assert_eq!(ProviderErrorClass::from_status(400), ProviderErrorClass::Other);
    }

    #[test]
    fn stable_codes_are_distinct_and_stable() {
        assert_eq!(ProviderErrorClass::AuthFailed.code(), "AUTH_FAILED");
        assert_eq!(ProviderErrorClass::RateLimited.code(), "RATE_LIMITED");
        assert_eq!(ProviderErrorClass::Timeout.code(), "TIMEOUT");
        assert_eq!(ProviderErrorClass::Network.code(), "NETWORK");
        assert_eq!(ProviderErrorClass::Server.code(), "SERVER");
        assert_eq!(ProviderErrorClass::Other.code(), "OTHER");
        // 超时不来自 HTTP 状态（由超时判定点显式归类），from_status 不产出 Timeout。
        assert_ne!(ProviderErrorClass::from_status(408), ProviderErrorClass::Timeout);
    }

    /// P2-13：超时错误给出稳定 TIMEOUT 码，安全文案不泄露原始正文。
    #[test]
    fn timeout_class_has_stable_code_and_safe_message() {
        let err = ProviderError::new(ProviderErrorClass::Timeout, "raw upstream timeout body");
        assert_eq!(err.code(), "TIMEOUT");
        assert!(err.safe_message().contains("timed out"));
        assert!(!err.to_string().contains("raw upstream timeout body"));
    }

    /// P0-4 验收核心：喂入含上游回显片段（`****gEni`）的原始正文 →
    /// **用户可见文案（Display / safe_message）不含该片段**，但 detail 保留（可诊断）。
    #[test]
    fn upstream_body_is_not_exposed_as_user_message() {
        let raw = "Your api key: ****gEni is invalid";
        let err = ProviderError::from_status(401, raw);

        let visible = err.to_string();
        assert!(
            !visible.contains("****gEni"),
            "用户可见文案不得回显上游正文片段: {visible}"
        );
        assert!(
            !err.safe_message().contains("****gEni"),
            "safe_message 不得回显上游正文片段: {}",
            err.safe_message()
        );
        // 原始正文仍可在 detail 中取回（仅服务端日志诊断用；前端不展示）
        assert_eq!(err.detail(), raw);
        assert_eq!(err.code(), "AUTH_FAILED");
    }

    #[test]
    fn extracts_classification_through_anyhow_context_chain() {
        let inner = anyhow::Error::new(ProviderError::from_status(403, "forbidden detail"));
        let wrapped = inner.context("orchestrator failed");
        assert_eq!(code_of(&wrapped), Some("AUTH_FAILED"));
        assert_eq!(
            provider_error_of(&wrapped).map(ProviderError::class),
            Some(ProviderErrorClass::AuthFailed)
        );
        assert_eq!(
            user_message_of(&wrapped)
                .as_deref()
                .map(|m| m.contains("forbidden detail")),
            Some(false),
            "user_message_of 不得含原始正文"
        );
    }

    /// R65/B1：计划步骤失败码——内层已归类时沿用 provider 码，否则回退 PLAN_STEP_FAILED。
    #[test]
    fn plan_step_code_prefers_classified_provider_code() {
        let classified = anyhow::Error::new(ProviderError::from_status(401, "raw"));
        assert_eq!(plan_step_code(&classified), "AUTH_FAILED");
        let plain = anyhow::anyhow!("计划子步骤 step-2 执行失败：内部细节");
        assert_eq!(plan_step_code(&plain), PLAN_STEP_FAILED);
    }

    /// 计划失败码是稳定、互异的字符串常量（前端按码本地化，不得随意改写）。
    #[test]
    fn plan_failure_codes_are_stable() {
        assert_eq!(PLAN_STEP_FAILED, "PLAN_STEP_FAILED");
        assert_eq!(PLAN_STEP_ARTIFACT_MISSING, "PLAN_STEP_ARTIFACT_MISSING");
        assert_eq!(PLAN_COMPLETION_BLOCKED, "PLAN_COMPLETION_BLOCKED");
        assert_ne!(PLAN_STEP_FAILED, PLAN_STEP_ARTIFACT_MISSING);
    }

    #[test]
    fn returns_none_for_plain_errors() {
        let plain = anyhow::anyhow!("some internal failure");
        assert!(provider_error_of(&plain).is_none());
        assert!(code_of(&plain).is_none());
    }
}
