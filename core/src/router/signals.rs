//! ClassifyInput 构造与信号抽取（复用 `crate::intent::Classifier` 输出）。

use super::*;
use crate::intent::{Classifier, IntentType};
use crate::persistence::config::Config;

/// 从一轮对话构造 classify 输入（纯规则抽取，零 LLM 开销）。
///
/// 复用 [`crate::intent::Classifier`] 的本地意图分类结果，把推断档植入 `routing_history`
/// （Phase 2 将进一步把 `ClassificationResult` 作为 `score_persona` 的一路信号）。
/// `runtime_available` 在 Marguerite 场景中恒为 true（无外部 v4 运行时），即走正常 Rule 路径。
pub fn classify_input_from_turn(text: &str, _session: &str, _config: &Config) -> ClassifyInput {
    let classification = Classifier::new().classify(text);
    let seed_tier = intent_to_tier(&classification.intent);
    let routing_history = vec![seed_tier];
    // 粗略 token 估算（字符数 / 4）；用于 Explorer 大上下文加权信号。
    let context_tokens_est = text.chars().count() / 4;
    ClassifyInput {
        message: text.to_string(),
        routing_history,
        prev_route_decisions: Vec::new(),
        context_tokens_est,
        runtime_available: true,
        has_image: false,
    }
}

/// 本地意图 → persona 档（仅 Coder 相关意图映射到 Coder，其余归 General）。
fn intent_to_tier(intent: &IntentType) -> PersonaTier {
    match intent {
        IntentType::Refactor | IntentType::Fix | IntentType::Test => PersonaTier::Coder,
        _ => PersonaTier::General,
    }
}
