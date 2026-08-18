//! Track A 路由器与思维模式层（MVP 切片）
//!
//! 纯规则、零 LLM 开销的任务路由器：把一条用户输入映射为
//! [`RouteDecision`]（persona 档 + 思考预算档位 T0/T1 + prompt 策略 P0/P1），
//! 并落地可观测的决策日志（敏感字段仅存 hash）。
//!
//! 依赖方向（单向、无环）：`router` 单向调用 `crate::model::thinking`、
//! `crate::cache`、`crate::events`、`crate::intent`、`crate::persistence::config`；
//! `agent` / `model::router` / `orchestration` / `gateway` / `tui::workbench`
//! 单向调用本模块，本模块不反向依赖它们。

use serde::{Deserialize, Serialize};

pub mod bindings;
pub mod classify;
pub mod decision_log;
pub mod persona;
pub mod signals;
pub mod vision_gate;

pub use classify::{classify, classify_with};
pub use decision_log::{DecisionLog, LogEntry, SCHEMA_VERSION};
pub use persona::{MatchRule, Persona, PersonaCatalog};
pub use signals::classify_input_from_turn;

/// 思考预算档位（thinking_mode 的 Rust 对译）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThinkingMode {
    /// 无思考（低延迟、关思考预算）
    T0,
    /// 开思考（深度推理）
    T1,
}

/// system prompt 注入策略（prompt_policy 的 Rust 对译）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptPolicy {
    /// 不额外注入策略片段
    P0,
    /// 注入稳定策略片段（思考/格式引导）
    P1,
}

/// 路由档（route_class）：表达应选用哪类 persona + 思考/策略组合，与 IntentType 正交。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PersonaTier {
    /// 通用闲聊 / 轻问答
    General,
    /// 代码生成 / 修复 / 重构
    Coder,
    /// 数学 / 逻辑 / 证明
    Reasoner,
    /// 工具 / 终端 / 浏览器 / 桌面操控
    Tooler,
    /// 长文档 / 长任务（大上下文）
    Explorer,
}

/// 决策来源：可观测性 + 弱路由 / 降级标记。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DecisionSource {
    /// 规则版 classify 正常命中
    Rule,
    /// 弱路由：confidence < 阈值，按历史粘性落到最近 persona
    History,
    /// 降级回退：分类器异常
    Default,
    /// runtime（v4 运行时）不可用降级
    #[serde(rename = "v4_unavailable")]
    RuntimeUnavailable,
}

impl DecisionSource {
    /// 小写字符串表示（与 JSONL / 事件负载对齐）。
    pub fn as_str(&self) -> &'static str {
        match self {
            DecisionSource::Rule => "rule",
            DecisionSource::History => "history",
            DecisionSource::Default => "default",
            DecisionSource::RuntimeUnavailable => "v4_unavailable",
        }
    }
}

/// 单次 classify 的完整决策。网关 `router_decision_payload` 直接消费本结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    pub tier: PersonaTier,
    /// [0,1] 规则打分
    pub confidence: f64,
    /// T0 | T1
    pub thinking_mode: ThinkingMode,
    /// P0 | P1
    pub prompt_policy: PromptPolicy,
    pub source: DecisionSource,
    pub fallback_reason: Option<String>,
    /// 结构化可观测：本决策经历的管线步骤
    pub steps: Vec<PipelineStepRecord>,
}

/// 管线步骤记录（PipelineStepRecord 的对译）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStepRecord {
    pub step_name: String,
    pub applied: bool,
    pub routing_source: String,
    pub fallback_reason: Option<String>,
}

/// classify 的输入：消息 + 路由历史 + 上一轮决策 + 上下文 token 估算 + runtime 可用性。
#[derive(Debug, Clone)]
pub struct ClassifyInput {
    pub message: String,
    /// 最近若干轮的 tier 序列（历史粘性用；空 = 无历史）
    pub routing_history: Vec<PersonaTier>,
    /// 最近若干轮的完整决策（弱路由回退到最近一次生效决策）
    pub prev_route_decisions: Vec<RouteDecision>,
    /// 当前上下文 token 估算
    pub context_tokens_est: usize,
    /// runtime（v4 运行时）是否可用；false → 降级默认档 + v4_unavailable 日志
    pub runtime_available: bool,
    /// 本轮是否携带图片（视觉取证门 Track B 的输入信号；默认 false）。
    /// true 时在 classify 中追加 vision_gate 可观测步骤（不改非图片轮路由）。
    pub has_image: bool,
}

/// 路由器错误类型。
#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    #[error("决策日志写入失败: {0}")]
    LogWrite(String),
    #[error("persona 名册未命中 tier={0:?}")]
    UnknownTier(PersonaTier),
}

#[cfg(test)]
mod tests;
