//! Run 级 token 归因结构与累加规则。
//!
//! 该模块只负责一次运行范围内的结构化 token 计量，不负责数据库持久化或 provider 接入。

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 一个 run 在指定 session/model 下的 token 归因。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageAttribution {
    pub run_id: String,
    pub session_id: String,
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct RawUsageAttribution {
    run_id: String,
    session_id: String,
    model: String,
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    #[serde(default)]
    cache_read_tokens: Option<u64>,
}

impl<'de> Deserialize<'de> for UsageAttribution {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawUsageAttribution::deserialize(deserializer)?;
        Self::new(
            raw.run_id,
            raw.session_id,
            raw.model,
            raw.prompt_tokens,
            raw.completion_tokens,
            raw.total_tokens,
            raw.cache_read_tokens,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// 归因输入或累加失败原因。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AttributionError {
    #[error("total_tokens must equal prompt_tokens + completion_tokens")]
    TotalMismatch {
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
    },
    #[error("usage attribution scope does not match run/session/model")]
    ScopeMismatch {
        run_id: String,
        session_id: String,
        model: String,
    },
    #[error("token counter overflow: {field}")]
    Overflow { field: &'static str },
}

impl UsageAttribution {
    /// 创建并校验归因。token 使用无符号整数，天然拒绝负数。
    pub fn new(
        run_id: impl Into<String>,
        session_id: impl Into<String>,
        model: impl Into<String>,
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
        cache_read_tokens: Option<u64>,
    ) -> Result<Self, AttributionError> {
        let expected = prompt_tokens.checked_add(completion_tokens).ok_or(
            AttributionError::Overflow {
                field: "total_tokens",
            },
        )?;
        if total_tokens != expected {
            return Err(AttributionError::TotalMismatch {
                prompt_tokens,
                completion_tokens,
                total_tokens,
            });
        }
        Ok(Self {
            run_id: run_id.into(),
            session_id: session_id.into(),
            model: model.into(),
            prompt_tokens,
            completion_tokens,
            total_tokens,
            cache_read_tokens,
        })
    }

    /// 从既有模型层 TokenUsage 转换为 run 归因记录。
    pub fn from_token_usage(
        run_id: impl Into<String>,
        session_id: impl Into<String>,
        model: impl Into<String>,
        usage: &crate::model::llm::TokenUsage,
    ) -> Result<Self, AttributionError> {
        Self::new(
            run_id,
            session_id,
            model,
            usage.prompt_tokens as u64,
            usage.completion_tokens as u64,
            usage.total_tokens as u64,
            Some(usage.cache_read_input_tokens as u64),
        )
    }

    /// 累加同一 run/session/model 的计量；失败时保证 self 不变。
    pub fn accumulate(&mut self, increment: &Self) -> Result<(), AttributionError> {
        if self.run_id != increment.run_id
            || self.session_id != increment.session_id
            || self.model != increment.model
        {
            return Err(AttributionError::ScopeMismatch {
                run_id: increment.run_id.clone(),
                session_id: increment.session_id.clone(),
                model: increment.model.clone(),
            });
        }

        let prompt = self
            .prompt_tokens
            .checked_add(increment.prompt_tokens)
            .ok_or(AttributionError::Overflow {
                field: "prompt_tokens",
            })?;
        let completion = self
            .completion_tokens
            .checked_add(increment.completion_tokens)
            .ok_or(AttributionError::Overflow {
                field: "completion_tokens",
            })?;
        let total = self
            .total_tokens
            .checked_add(increment.total_tokens)
            .ok_or(AttributionError::Overflow {
                field: "total_tokens",
            })?;
        let cache_read = match (self.cache_read_tokens, increment.cache_read_tokens) {
            (Some(left), Some(right)) => Some(left.checked_add(right).ok_or(
                AttributionError::Overflow {
                    field: "cache_read_tokens",
                },
            )?),
            (Some(value), None) | (None, Some(value)) => Some(value),
            (None, None) => None,
        };

        self.prompt_tokens = prompt;
        self.completion_tokens = completion;
        self.total_tokens = total;
        self.cache_read_tokens = cache_read;
        Ok(())
    }
}
