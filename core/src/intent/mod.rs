//! 意图系统模块
//!
//! 提供本地意图分类、意图路由和规则引擎三大核心功能：
//!
//! - `classifier`：基于规则 + 关键词匹配的轻量意图分类器，实现高频意图的即时识别
//! - `router`：维护意图 → 处理器的映射表，支持路由分发和热加载
//! - `rules`：用户可自定义的规则引擎，支持 Always / Requested / Manual 三种模式

pub mod classifier;
pub mod router;
pub mod rules;

pub use classifier::{ClassificationResult, ClassificationRule, Classifier, IntentType};
pub use router::{IntentContext, IntentHandler, IntentOutput, IntentRouter};
pub use rules::{Rule, RuleAction, RuleCondition, RuleContext, RuleMode, RulesEngine};
