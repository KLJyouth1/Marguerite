//! 技能注册表
//!
//! 管理所有技能的注册中心，支持按名称、意图查找和评分管理。

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// 技能来源
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SkillSource {
    /// 内置技能
    Builtin,
    /// 本地文件
    Local { path: std::path::PathBuf },
    /// 远程安装
    Remote { url: String },
}

impl Default for SkillSource {
    fn default() -> Self {
        Self::Builtin
    }
}

/// 技能定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// 技能名称
    pub name: String,
    /// 技能描述
    pub description: String,
    /// 版本号
    pub version: String,
    /// 作者
    pub author: Option<String>,
    /// 触发的意图关键词
    #[serde(default)]
    pub intent: Vec<String>,
    /// 需要的工具列表
    #[serde(default)]
    pub tools: Vec<String>,
    /// Prompt 模板
    pub prompt_template: String,
    /// 依赖的其他技能
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// 技能来源
    #[serde(skip)]
    pub source: SkillSource,
    /// 效果评分 (0.0 - 5.0)
    #[serde(default)]
    pub rating: f64,
    /// 使用次数
    #[serde(default)]
    pub uses: u32,
}

/// 技能注册表
///
/// 管理所有技能的注册、查找和索引。
pub struct SkillRegistry {
    /// 技能名称 → 技能映射
    skills: HashMap<String, Arc<Skill>>,
    /// 意图关键词 → 技能名称列表
    intent_index: HashMap<String, Vec<String>>,
}

impl SkillRegistry {
    /// 创建新的空技能注册表
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
            intent_index: HashMap::new(),
        }
    }

    /// 注册技能
    ///
    /// 将技能加入注册表，并更新意图索引。
    pub fn register(&mut self, skill: Skill) -> Result<()> {
        let name = skill.name.clone();
        if self.skills.contains_key(&name) {
            bail!("技能 '{}' 已注册", name);
        }

        // 更新意图索引
        for keyword in &skill.intent {
            self.intent_index
                .entry(keyword.to_lowercase())
                .or_default()
                .push(name.clone());
        }

        self.skills.insert(name, Arc::new(skill));
        Ok(())
    }

    /// 注销技能
    ///
    /// 从注册表中移除技能，并清理意图索引。
    pub fn unregister(&mut self, name: &str) -> Result<()> {
        let skill = self
            .skills
            .remove(name)
            .ok_or_else(|| anyhow::anyhow!("技能 '{}' 未注册", name))?;

        // 清理意图索引
        for keyword in &skill.intent {
            if let Some(names) = self.intent_index.get_mut(&keyword.to_lowercase()) {
                names.retain(|n| n != name);
                if names.is_empty() {
                    self.intent_index.remove(&keyword.to_lowercase());
                }
            }
        }

        Ok(())
    }

    /// 获取技能
    pub fn get(&self, name: &str) -> Option<Arc<Skill>> {
        self.skills.get(name).cloned()
    }

    /// 列出所有技能
    pub fn list(&self) -> Vec<Arc<Skill>> {
        self.skills.values().cloned().collect()
    }

    /// 按意图关键词查找技能
    ///
    /// 遍历所有技能，匹配意图关键词（不区分大小写）。
    pub fn find_by_intent(&self, intent: &str) -> Vec<Arc<Skill>> {
        let intent_lower = intent.to_lowercase();
        self.skills
            .values()
            .filter(|skill| {
                skill
                    .intent
                    .iter()
                    .any(|kw| kw.to_lowercase() == intent_lower)
            })
            .cloned()
            .collect()
    }

    /// 技能数量
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// 注册表是否为空
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// 更新技能评分
    ///
    /// 更新指定技能的评分（0.0 - 5.0），评分会被限制在有效范围内。
    pub fn update_rating(&mut self, name: &str, rating: f64) {
        if let Some(skill) = self.skills.get_mut(name) {
            let skill = Arc::make_mut(skill);
            skill.rating = rating.clamp(0.0, 5.0);
        }
    }

    /// 增加技能使用次数
    pub fn increment_uses(&mut self, name: &str) {
        if let Some(skill) = self.skills.get_mut(name) {
            let skill = Arc::make_mut(skill);
            skill.uses = skill.uses.saturating_add(1);
        }
    }

    /// 检查技能是否已注册
    pub fn has(&self, name: &str) -> bool {
        self.skills.contains_key(name)
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_skill(name: &str, intents: Vec<&str>) -> Skill {
        Skill {
            name: name.to_string(),
            description: format!("测试技能: {}", name),
            version: "1.0.0".to_string(),
            author: None,
            intent: intents.into_iter().map(String::from).collect(),
            tools: vec![],
            prompt_template: "模板: {{text}}".to_string(),
            dependencies: vec![],
            source: SkillSource::Builtin,
            rating: 0.0,
            uses: 0,
        }
    }

    #[test]
    fn test_register_and_get() {
        let mut registry = SkillRegistry::new();
        let skill = create_test_skill("explain", vec!["explain", "解释"]);
        registry.register(skill).unwrap();

        let found = registry.get("explain");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "explain");
    }

    #[test]
    fn test_register_duplicate() {
        let mut registry = SkillRegistry::new();
        let skill = create_test_skill("test", vec!["test"]);
        registry.register(skill).unwrap();

        let duplicate = create_test_skill("test", vec!["test"]);
        assert!(registry.register(duplicate).is_err());
    }

    #[test]
    fn test_unregister() {
        let mut registry = SkillRegistry::new();
        let skill = create_test_skill("refactor", vec!["refactor", "重构"]);
        registry.register(skill).unwrap();
        assert!(registry.has("refactor"));

        registry.unregister("refactor").unwrap();
        assert!(!registry.has("refactor"));
    }

    #[test]
    fn test_unregister_nonexistent() {
        let mut registry = SkillRegistry::new();
        assert!(registry.unregister("nonexistent").is_err());
    }

    #[test]
    fn test_list() {
        let mut registry = SkillRegistry::new();
        registry
            .register(create_test_skill("skill_a", vec!["a"]))
            .unwrap();
        registry
            .register(create_test_skill("skill_b", vec!["b"]))
            .unwrap();

        let list = registry.list();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_find_by_intent() {
        let mut registry = SkillRegistry::new();
        registry
            .register(create_test_skill("explain", vec!["explain", "解释"]))
            .unwrap();
        registry
            .register(create_test_skill("refactor", vec!["refactor", "重构"]))
            .unwrap();

        let skills = registry.find_by_intent("explain");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "explain");

        let skills = registry.find_by_intent("解释");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "explain");

        // 不存在的意图
        let skills = registry.find_by_intent("nonexistent");
        assert!(skills.is_empty());
    }

    #[test]
    fn test_len_and_is_empty() {
        let mut registry = SkillRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);

        registry
            .register(create_test_skill("a", vec!["a"]))
            .unwrap();
        assert!(!registry.is_empty());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_update_rating() {
        let mut registry = SkillRegistry::new();
        registry
            .register(create_test_skill("test", vec!["test"]))
            .unwrap();

        registry.update_rating("test", 4.5);
        let skill = registry.get("test").unwrap();
        assert!((skill.rating - 4.5).abs() < 1e-10);

        // 超出范围应被 clamp
        registry.update_rating("test", 6.0);
        let skill = registry.get("test").unwrap();
        assert!((skill.rating - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_increment_uses() {
        let mut registry = SkillRegistry::new();
        registry
            .register(create_test_skill("test", vec!["test"]))
            .unwrap();

        registry.increment_uses("test");
        let skill = registry.get("test").unwrap();
        assert_eq!(skill.uses, 1);

        registry.increment_uses("test");
        let skill = registry.get("test").unwrap();
        assert_eq!(skill.uses, 2);
    }

    #[test]
    fn test_has() {
        let mut registry = SkillRegistry::new();
        registry
            .register(create_test_skill("exists", vec!["yes"]))
            .unwrap();

        assert!(registry.has("exists"));
        assert!(!registry.has("nonexistent"));
    }
}
