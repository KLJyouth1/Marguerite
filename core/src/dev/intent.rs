//! Deterministic offline intent routing for development requests.

use serde::{Deserialize, Serialize};

/// Coarse intent used to choose a local development context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DevIntent {
    Coding,
    Reasoning,
    Chat,
    General,
}

/// Classify without network calls or model dependencies.
///
/// Rules are intentionally small and ordered: coding, reasoning, chat, then
/// general. Unknown text always falls back to `General`.
pub fn classify_intent(input: &str) -> DevIntent {
    let text = input.trim().to_lowercase();
    if text.is_empty() {
        return DevIntent::General;
    }
    if contains_any(&text, &["code", "coding", "rust", "python", "bug", "fix", "修复", "代码", "编程", "测试", "api", "函数"]) {
        return DevIntent::Coding;
    }
    if contains_any(&text, &["why", "how", "analyze", "analyse", "reason", "tradeoff", "compare", "分析", "原因", "解释", "推理", "利弊"]) {
        return DevIntent::Reasoning;
    }
    if contains_any(&text, &["hello", "hi", "hey", "你好", "您好", "谢谢", "再见", "天气怎么样"]) {
        return DevIntent::Chat;
    }
    DevIntent::General
}

fn contains_any(text: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| text.contains(term))
}
