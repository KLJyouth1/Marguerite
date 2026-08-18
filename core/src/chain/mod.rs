//! 调用链执行器（v2 工作台）
//!
//! 意图 → 有序/并行步骤链（tool/skill/expert/guard），
//! 每步支持超时、重试、fallback、审批；链后护栏汇总修复。
//! 复用 v1 工具注册表、技能引擎、子代理池、护栏网关与审批门控。

pub mod builtin;
pub mod cli;
pub mod executor;
pub mod schema;
pub mod templates;

pub use executor::{ChainExecutor, ChainOutcome, StepOutcome, StepStatus};
pub use schema::{ChainContext, ChainPlan, ChainStep, ChainStepType, ExecutionMode, StepOutput};
