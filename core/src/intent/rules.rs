//! 规则引擎
//!
//! 用户可自定义的规则系统，支持 Always / Requested / Manual 三种模式。
//! 用于在特定条件下自动执行配置变更、技能调用、提示词注入等操作。

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::classifier::IntentType;

/// 规则执行模式
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RuleMode {
    /// 总是执行
    Always,
    /// 仅在用户请求时执行
    Requested,
    /// 手动触发
    Manual,
}

/// 规则条件
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RuleCondition {
    /// 当意图匹配时
    IntentMatch(IntentType),
    /// 当文件匹配 glob 模式时
    FilePattern(String),
    /// 当包含关键词时
    KeywordMatch(Vec<String>),
    /// 与条件：两个条件同时满足
    And(Box<RuleCondition>, Box<RuleCondition>),
    /// 或条件：两个条件任一满足
    Or(Box<RuleCondition>, Box<RuleCondition>),
}

/// 规则动作
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RuleAction {
    /// 设置配置项（键，值）
    SetConfig(String, String),
    /// 运行技能
    RunSkill(String),
    /// 注入提示词
    InjectPrompt(String),
    /// 记录警告
    LogWarning(String),
}

/// 规则定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// 规则名称
    pub name: String,
    /// 规则描述
    pub description: String,
    /// 执行模式
    pub mode: RuleMode,
    /// 触发条件
    pub condition: RuleCondition,
    /// 执行动作
    pub action: RuleAction,
}

/// 规则评估上下文
#[derive(Debug, Clone)]
pub struct RuleContext {
    /// 识别出的意图
    pub intent: IntentType,
    /// 当前文件路径
    pub file: Option<String>,
    /// 用户输入文本
    pub text: String,
}

impl RuleContext {
    /// 创建新的规则评估上下文
    pub fn new(intent: IntentType, text: String) -> Self {
        Self {
            intent,
            file: None,
            text,
        }
    }
}

/// 规则引擎
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RulesEngine {
    /// 规则列表
    rules: Vec<Rule>,
}

impl RulesEngine {
    /// 创建空规则引擎
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// 添加规则
    pub fn add_rule(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    /// 评估所有规则，返回匹配的动作
    ///
    /// 遍历所有规则，检查每个规则的条件是否满足。
    /// 只评估模式为 Always 或 Requested 的规则。
    pub fn evaluate(&self, ctx: &RuleContext) -> Vec<RuleAction> {
        self.rules
            .iter()
            .filter(|rule| rule.mode == RuleMode::Always || rule.mode == RuleMode::Requested)
            .filter(|rule| Self::evaluate_condition(&rule.condition, ctx))
            .map(|rule| rule.action.clone())
            .collect()
    }

    /// 手动触发指定规则，返回匹配的动作
    ///
    /// 只执行模式为 Manual 的规则
    pub fn trigger(&self, ctx: &RuleContext) -> Vec<RuleAction> {
        self.rules
            .iter()
            .filter(|rule| rule.mode == RuleMode::Manual)
            .filter(|rule| Self::evaluate_condition(&rule.condition, ctx))
            .map(|rule| rule.action.clone())
            .collect()
    }

    /// 递归评估条件
    fn evaluate_condition(condition: &RuleCondition, ctx: &RuleContext) -> bool {
        match condition {
            RuleCondition::IntentMatch(intent) => ctx.intent == *intent,
            RuleCondition::FilePattern(pattern) => ctx.file.as_ref().is_some_and(|f| {
                let lower_file = f.to_lowercase();
                let lower_pattern = pattern.to_lowercase();
                if lower_pattern.contains('*') {
                    let regex_pattern = pattern_for_glob(&lower_pattern);
                    regex::Regex::new(&regex_pattern)
                        .map(|re| re.is_match(&lower_file))
                        .unwrap_or(false)
                } else {
                    lower_file.contains(&lower_pattern)
                }
            }),
            RuleCondition::KeywordMatch(keywords) => {
                let text_lower = ctx.text.to_lowercase();
                keywords
                    .iter()
                    .any(|kw| text_lower.contains(&kw.to_lowercase()))
            }
            RuleCondition::And(left, right) => {
                Self::evaluate_condition(left, ctx) && Self::evaluate_condition(right, ctx)
            }
            RuleCondition::Or(left, right) => {
                Self::evaluate_condition(left, ctx) || Self::evaluate_condition(right, ctx)
            }
        }
    }

    /// 获取所有规则
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// 从文件加载规则
    pub fn load(&mut self, path: &Path) -> Result<()> {
        let content = std::fs::read_to_string(path)?;
        let rules: Vec<Rule> = serde_yaml::from_str(&content)?;
        self.rules = rules;
        Ok(())
    }

    /// 保存规则到文件
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = serde_yaml::to_string(&self.rules)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }
}

impl Default for RulesEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// 将 glob 模式转换为正则表达式
fn pattern_for_glob(pattern: &str) -> String {
    let mut regex = String::with_capacity(pattern.len() + 2);
    regex.push('^');
    for ch in pattern.chars() {
        match ch {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            '.' => regex.push_str("\\."),
            '\\' => regex.push_str("\\\\"),
            c => regex.push(c),
        }
    }
    regex.push('$');
    regex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intent_match_condition() {
        let rule = Rule {
            name: "test-rule".to_string(),
            description: "测试规则".to_string(),
            mode: RuleMode::Always,
            condition: RuleCondition::IntentMatch(IntentType::Fix),
            action: RuleAction::LogWarning("检测到修复意图".to_string()),
        };

        let engine = RulesEngine::new();
        // 引擎为空时无规则
        let actions = engine.evaluate(&RuleContext::new(IntentType::Fix, "fix bug".to_string()));
        assert!(actions.is_empty());

        // 添加规则后评估
        let mut engine = RulesEngine::new();
        engine.add_rule(rule);
        let actions = engine.evaluate(&RuleContext::new(IntentType::Fix, "fix bug".to_string()));
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0],
            RuleAction::LogWarning("检测到修复意图".to_string())
        );

        // 不匹配的意图
        let actions = engine.evaluate(&RuleContext::new(
            IntentType::Explain,
            "explain".to_string(),
        ));
        assert!(actions.is_empty());
    }

    #[test]
    fn test_file_pattern_condition() {
        let rule = Rule {
            name: "rust-rule".to_string(),
            description: "Rust 文件规则".to_string(),
            mode: RuleMode::Always,
            condition: RuleCondition::FilePattern("*.rs".to_string()),
            action: RuleAction::InjectPrompt("这是 Rust 文件".to_string()),
        };

        let mut engine = RulesEngine::new();
        engine.add_rule(rule);

        let mut ctx = RuleContext::new(IntentType::Explain, "explain".to_string());
        ctx.file = Some("src/main.rs".to_string());
        let actions = engine.evaluate(&ctx);
        assert_eq!(actions.len(), 1);

        // 不匹配的文件
        let mut ctx = RuleContext::new(IntentType::Explain, "explain".to_string());
        ctx.file = Some("src/main.py".to_string());
        let actions = engine.evaluate(&ctx);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_keyword_match_condition() {
        let rule = Rule {
            name: "keyword-rule".to_string(),
            description: "关键词规则".to_string(),
            mode: RuleMode::Always,
            condition: RuleCondition::KeywordMatch(vec!["urgent".to_string(), "紧急".to_string()]),
            action: RuleAction::SetConfig("priority".to_string(), "high".to_string()),
        };

        let mut engine = RulesEngine::new();
        engine.add_rule(rule);

        let actions = engine.evaluate(&RuleContext::new(
            IntentType::Fix,
            "urgent fix needed".to_string(),
        ));
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0],
            RuleAction::SetConfig("priority".to_string(), "high".to_string())
        );
    }

    #[test]
    fn test_and_condition() {
        let rule = Rule {
            name: "and-rule".to_string(),
            description: "与条件规则".to_string(),
            mode: RuleMode::Always,
            condition: RuleCondition::And(
                Box::new(RuleCondition::IntentMatch(IntentType::Fix)),
                Box::new(RuleCondition::KeywordMatch(vec!["urgent".to_string()])),
            ),
            action: RuleAction::LogWarning("紧急修复".to_string()),
        };

        let mut engine = RulesEngine::new();
        engine.add_rule(rule);

        // 两个条件都满足
        let actions = engine.evaluate(&RuleContext::new(
            IntentType::Fix,
            "urgent fix needed".to_string(),
        ));
        assert_eq!(actions.len(), 1);

        // 只满足一个条件
        let actions = engine.evaluate(&RuleContext::new(
            IntentType::Explain,
            "urgent fix needed".to_string(),
        ));
        assert!(actions.is_empty());

        // 都不满足
        let actions = engine.evaluate(&RuleContext::new(IntentType::Explain, "hello".to_string()));
        assert!(actions.is_empty());
    }

    #[test]
    fn test_or_condition() {
        let rule = Rule {
            name: "or-rule".to_string(),
            description: "或条件规则".to_string(),
            mode: RuleMode::Always,
            condition: RuleCondition::Or(
                Box::new(RuleCondition::IntentMatch(IntentType::Doc)),
                Box::new(RuleCondition::IntentMatch(IntentType::Commit)),
            ),
            action: RuleAction::RunSkill("documentation".to_string()),
        };

        let mut engine = RulesEngine::new();
        engine.add_rule(rule);

        // Doc 意图匹配
        let actions = engine.evaluate(&RuleContext::new(IntentType::Doc, "write docs".to_string()));
        assert_eq!(actions.len(), 1);

        // Commit 意图匹配
        let actions = engine.evaluate(&RuleContext::new(IntentType::Commit, "commit".to_string()));
        assert_eq!(actions.len(), 1);

        // 都不匹配
        let actions = engine.evaluate(&RuleContext::new(IntentType::Fix, "fix".to_string()));
        assert!(actions.is_empty());
    }

    #[test]
    fn test_rule_mode_always() {
        let rule = Rule {
            name: "always-rule".to_string(),
            description: "总是执行".to_string(),
            mode: RuleMode::Always,
            condition: RuleCondition::IntentMatch(IntentType::Fix),
            action: RuleAction::LogWarning("fix".to_string()),
        };

        let mut engine = RulesEngine::new();
        engine.add_rule(rule);

        // Always 模式在 evaluate 中执行
        let actions = engine.evaluate(&RuleContext::new(IntentType::Fix, "fix".to_string()));
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn test_rule_mode_requested() {
        let rule = Rule {
            name: "requested-rule".to_string(),
            description: "请求时执行".to_string(),
            mode: RuleMode::Requested,
            condition: RuleCondition::IntentMatch(IntentType::Fix),
            action: RuleAction::LogWarning("requested fix".to_string()),
        };

        let mut engine = RulesEngine::new();
        engine.add_rule(rule);

        // Requested 模式在 evaluate 中也执行
        let actions = engine.evaluate(&RuleContext::new(IntentType::Fix, "fix".to_string()));
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn test_rule_mode_manual() {
        let rule = Rule {
            name: "manual-rule".to_string(),
            description: "手动触发".to_string(),
            mode: RuleMode::Manual,
            condition: RuleCondition::IntentMatch(IntentType::Fix),
            action: RuleAction::LogWarning("manual fix".to_string()),
        };

        let mut engine = RulesEngine::new();
        engine.add_rule(rule);

        // Manual 模式在 evaluate 中不执行
        let actions = engine.evaluate(&RuleContext::new(IntentType::Fix, "fix".to_string()));
        assert!(actions.is_empty());

        // Manual 模式在 trigger 中执行
        let actions = engine.trigger(&RuleContext::new(IntentType::Fix, "fix".to_string()));
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn test_save_and_load() {
        let mut engine = RulesEngine::new();
        engine.add_rule(Rule {
            name: "test-rule".to_string(),
            description: "测试规则".to_string(),
            mode: RuleMode::Always,
            condition: RuleCondition::IntentMatch(IntentType::Fix),
            action: RuleAction::LogWarning("test".to_string()),
        });

        let tmp_path = std::env::temp_dir().join("marguerite_test_rules.yaml");
        engine.save(&tmp_path).unwrap();
        assert!(tmp_path.exists());

        let mut loaded = RulesEngine::new();
        loaded.load(&tmp_path).unwrap();
        assert_eq!(loaded.rules().len(), 1);
        assert_eq!(loaded.rules()[0].name, "test-rule");

        // 清理
        let _ = std::fs::remove_file(&tmp_path);
    }

    #[test]
    fn test_multiple_rules_evaluation() {
        let mut engine = RulesEngine::new();

        engine.add_rule(Rule {
            name: "rule1".to_string(),
            description: "规则1".to_string(),
            mode: RuleMode::Always,
            condition: RuleCondition::IntentMatch(IntentType::Fix),
            action: RuleAction::LogWarning("fix warning".to_string()),
        });

        engine.add_rule(Rule {
            name: "rule2".to_string(),
            description: "规则2".to_string(),
            mode: RuleMode::Always,
            condition: RuleCondition::IntentMatch(IntentType::Fix),
            action: RuleAction::SetConfig("mode".to_string(), "fix".to_string()),
        });

        let actions = engine.evaluate(&RuleContext::new(IntentType::Fix, "fix".to_string()));
        assert_eq!(actions.len(), 2);
    }

    #[test]
    fn test_glob_to_regex() {
        assert_eq!(pattern_for_glob("*.rs"), "^.*\\.rs$");
        assert_eq!(pattern_for_glob("src/*.rs"), "^src/.*\\.rs$");
        assert_eq!(pattern_for_glob("test?"), "^test.$");
    }
}
