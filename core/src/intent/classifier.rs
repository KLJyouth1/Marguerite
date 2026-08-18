//! 本地意图分类器
//!
//! 基于规则 + 关键词匹配的轻量分类器，实现高频意图的即时识别。
//! 分类器完全基于规则和关键词匹配，不引入 LLM 依赖。

use std::collections::HashMap;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// 意图类型
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IntentType {
    /// 代码解释
    Explain,
    /// 重构
    Refactor,
    /// 修复
    Fix,
    /// 测试生成
    Test,
    /// 文档生成
    Doc,
    /// 提交信息
    Commit,
    /// 搜索
    Search,
    /// 一般问题
    Question,
    /// 未知（需要 LLM 消歧）
    Unknown,
}

/// 分类规则
#[derive(Debug, Clone)]
pub struct ClassificationRule {
    /// 对应的意图类型
    pub intent: IntentType,
    /// 触发关键词列表
    pub keywords: Vec<String>,
    /// 正则模式列表
    pub patterns: Vec<Regex>,
    /// 权重 (0.0 - 1.0)
    pub weight: f64,
}

/// 分类结果
#[derive(Debug, Clone)]
pub struct ClassificationResult {
    /// 识别出的意图
    pub intent: IntentType,
    /// 置信度 (0.0 - 1.0)
    pub confidence: f64,
    /// 是否需要 LLM 消歧
    pub needs_llm: bool,
}

/// 本地意图分类器
#[derive(Debug, Clone)]
pub struct Classifier {
    /// 分类规则列表
    rules: Vec<ClassificationRule>,
    /// 置信度阈值，低于此值需要 LLM 消歧
    confidence_threshold: f64,
}

impl Classifier {
    /// 创建默认分类器，内置高频意图规则
    pub fn new() -> Self {
        Self {
            rules: Self::default_rules(),
            confidence_threshold: 0.6,
        }
    }

    /// 创建带自定义规则和阈值的分类器
    pub fn with_rules(rules: Vec<ClassificationRule>, confidence_threshold: f64) -> Self {
        Self {
            rules,
            confidence_threshold,
        }
    }

    /// 分类主方法
    ///
    /// 对输入文本进行意图分类，返回分类结果。
    /// 分类过程：
    /// 1. 遍历所有规则，计算每条规则的匹配得分
    /// 2. 关键词匹配：匹配数量 / 总数 × 权重
    /// 3. 正则匹配：命中则附加权重加成
    /// 4. 取最高分作为最终意图
    pub fn classify(&self, text: &str) -> ClassificationResult {
        let text_lower = text.to_lowercase();
        let mut intent_scores: HashMap<IntentType, f64> = HashMap::new();

        for rule in &self.rules {
            // 关键词匹配：匹配关键词数 / 总关键词数 × 权重
            let matched_count = rule
                .keywords
                .iter()
                .filter(|kw| text_lower.contains(&kw.to_lowercase()))
                .count();
            let keyword_ratio = if rule.keywords.is_empty() {
                0.0
            } else {
                matched_count as f64 / rule.keywords.len() as f64
            };
            let mut score = keyword_ratio * rule.weight;

            // 正则匹配命中则直接加权重加成
            if !rule.patterns.is_empty() && rule.patterns.iter().any(|p| p.is_match(&text_lower)) {
                score = (score + rule.weight * 0.3).min(rule.weight);
            }

            if score > 0.0 {
                *intent_scores.entry(rule.intent.clone()).or_insert(0.0) += score;
            }
        }

        // 取最高分意图
        let (best_intent, confidence) = intent_scores
            .into_iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .unwrap_or((IntentType::Unknown, 0.0));

        let confidence = confidence.min(1.0);

        // 置信度低于阈值或完全未知时需要 LLM 消歧
        let needs_llm =
            confidence < self.confidence_threshold || best_intent == IntentType::Unknown;

        ClassificationResult {
            intent: if confidence == 0.0 {
                IntentType::Unknown
            } else {
                best_intent
            },
            confidence,
            needs_llm,
        }
    }

    /// 获取当前置信度阈值
    pub fn confidence_threshold(&self) -> f64 {
        self.confidence_threshold
    }

    /// 获取规则列表引用
    pub fn rules(&self) -> &[ClassificationRule] {
        &self.rules
    }

    /// 内置高频意图规则
    fn default_rules() -> Vec<ClassificationRule> {
        vec![
            ClassificationRule {
                intent: IntentType::Explain,
                keywords: vec![
                    "explain".to_string(),
                    "解释".to_string(),
                    "这是什么".to_string(),
                    "how does".to_string(),
                    "what is".to_string(),
                ],
                patterns: vec![Regex::new(r"(?i)^(explain|describe|解释)\b").unwrap()],
                weight: 0.9,
            },
            ClassificationRule {
                intent: IntentType::Refactor,
                keywords: vec![
                    "refactor".to_string(),
                    "重构".to_string(),
                    "优化".to_string(),
                    "improve".to_string(),
                    "clean up".to_string(),
                ],
                patterns: vec![Regex::new(r"(?i)\b(refactor|重构)\b").unwrap()],
                weight: 0.9,
            },
            ClassificationRule {
                intent: IntentType::Fix,
                keywords: vec![
                    "fix".to_string(),
                    "修复".to_string(),
                    "bug".to_string(),
                    "错误".to_string(),
                    "broken".to_string(),
                    "not working".to_string(),
                ],
                patterns: vec![Regex::new(r"(?i)\b(bug|fix|修复|broken)\b").unwrap()],
                weight: 0.9,
            },
            ClassificationRule {
                intent: IntentType::Test,
                keywords: vec![
                    "test".to_string(),
                    "测试".to_string(),
                    "单元测试".to_string(),
                    "coverage".to_string(),
                ],
                patterns: vec![Regex::new(r"(?i)\b(unittest|test_|测试)\b").unwrap()],
                weight: 0.85,
            },
            ClassificationRule {
                intent: IntentType::Doc,
                keywords: vec![
                    "doc".to_string(),
                    "文档".to_string(),
                    "document".to_string(),
                    "注释".to_string(),
                    "comment".to_string(),
                ],
                patterns: vec![],
                weight: 0.8,
            },
            ClassificationRule {
                intent: IntentType::Commit,
                keywords: vec![
                    "commit".to_string(),
                    "提交".to_string(),
                    "changelog".to_string(),
                    "release notes".to_string(),
                ],
                patterns: vec![],
                weight: 0.8,
            },
            ClassificationRule {
                intent: IntentType::Search,
                keywords: vec![
                    "search".to_string(),
                    "搜索".to_string(),
                    "find".to_string(),
                    "查找".to_string(),
                    "where is".to_string(),
                ],
                patterns: vec![],
                weight: 0.8,
            },
        ]
    }
}

impl Default for Classifier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_classify(text: &str, expected: IntentType, min_confidence: f64) {
        let classifier = Classifier::new();
        let result = classifier.classify(text);
        assert_eq!(
            result.intent, expected,
            "文本 \"{}\" 应分类为 {:?}，实际为 {:?}",
            text, expected, result.intent
        );
        assert!(
            result.confidence >= min_confidence,
            "文本 \"{}\" 的置信度 {:.2} 应 >= {:.2}",
            text,
            result.confidence,
            min_confidence
        );
    }

    #[test]
    fn test_classify_explain() {
        assert_classify("explain this code", IntentType::Explain, 0.1);
        assert_classify("解释一下这段代码", IntentType::Explain, 0.1);
        assert_classify("how does this work", IntentType::Explain, 0.1);
        assert_classify("what is a closure", IntentType::Explain, 0.1);
    }

    #[test]
    fn test_classify_refactor() {
        assert_classify("refactor this function", IntentType::Refactor, 0.1);
        assert_classify("重构这个模块", IntentType::Refactor, 0.1);
        assert_classify("improve the performance", IntentType::Refactor, 0.1);
        assert_classify("clean up this code", IntentType::Refactor, 0.1);
    }

    #[test]
    fn test_classify_fix() {
        assert_classify("fix this bug", IntentType::Fix, 0.1);
        assert_classify("修复这个错误", IntentType::Fix, 0.1);
        assert_classify("this code is broken", IntentType::Fix, 0.1);
        assert_classify("not working as expected", IntentType::Fix, 0.1);
    }

    #[test]
    fn test_classify_test() {
        assert_classify("write a test for this", IntentType::Test, 0.1);
        assert_classify("添加单元测试", IntentType::Test, 0.1);
        assert_classify("improve test coverage", IntentType::Test, 0.1);
    }

    #[test]
    fn test_classify_doc() {
        assert_classify("generate doc for this", IntentType::Doc, 0.1);
        assert_classify("写文档", IntentType::Doc, 0.1);
        assert_classify("add comments to the code", IntentType::Doc, 0.1);
        assert_classify("generate documentation", IntentType::Doc, 0.1);
    }

    #[test]
    fn test_classify_commit() {
        assert_classify("generate commit message", IntentType::Commit, 0.1);
        assert_classify("生成提交信息", IntentType::Commit, 0.1);
        assert_classify("write changelog", IntentType::Commit, 0.1);
        assert_classify("create release notes", IntentType::Commit, 0.1);
    }

    #[test]
    fn test_classify_search() {
        assert_classify("search for this function", IntentType::Search, 0.1);
        assert_classify("搜索这个文件", IntentType::Search, 0.1);
        assert_classify("find the implementation", IntentType::Search, 0.1);
        assert_classify("where is the config file", IntentType::Search, 0.1);
    }

    #[test]
    fn test_classify_unknown_low_confidence() {
        let classifier = Classifier::new();
        let result = classifier.classify("hello world");
        assert_eq!(result.intent, IntentType::Unknown);
        assert!(result.needs_llm);
        assert!(result.confidence < 0.6);
    }

    #[test]
    fn test_needs_llm_threshold() {
        let classifier = Classifier::new();

        // 高置信度（匹配多个关键词且正则命中）不需要 LLM
        let result = classifier.classify("fix this bug please broken");
        if result.confidence >= 0.6 {
            assert!(!result.needs_llm, "高置信度 needs_llm 应为 false");
        }

        // 低置信度需要 LLM
        let result = classifier.classify("something random here");
        assert!(result.needs_llm, "低置信度 needs_llm 应为 true");
    }

    #[test]
    fn test_confidence_calculation() {
        let classifier = Classifier::new();

        // 匹配多个关键词应提高置信度
        let single = classifier.classify("fix");
        let multi = classifier.classify("fix this bug");
        assert!(
            multi.confidence >= single.confidence,
            "匹配更多关键词应提高置信度"
        );
    }

    #[test]
    fn test_classifier_default() {
        let classifier = Classifier::default();
        assert!((classifier.confidence_threshold() - 0.6).abs() < 1e-10);
    }
}
