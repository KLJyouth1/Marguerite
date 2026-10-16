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
    /// 构建 / 创建新产物（应用、游戏、脚手架、脚本）
    ///
    /// 为什么单独一个变体：此前「帮我做一个 FPS 游戏」这类**创建型**请求在 7 条规则上
    /// 逐条 score=0 → 落进 `Unknown` → 展示为「对话」徽章。而创建型请求是本产品最高频的
    /// 用法之一，把它算作「未识别」既是错误的用户可见标签，也会无谓触发 LLM 消歧。
    Build,
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

impl IntentType {
    /// v3.3：任务类型中文标签（WebUI 步骤进度徽章用）。
    /// 面向用户的展示分类：编码/文档/检索/对话。
    pub fn task_label(&self) -> &'static str {
        match self {
            IntentType::Explain => "代码讲解",
            IntentType::Refactor | IntentType::Build => "编码",
            IntentType::Fix => "修复",
            IntentType::Test => "测试",
            IntentType::Doc => "文档",
            IntentType::Commit => "编码",
            IntentType::Search => "检索",
            IntentType::Question | IntentType::Unknown => "对话",
        }
    }
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

/// 置信度阈值：默认 0.6；可用环境变量 MARGUERITE_INTENT_CONFIDENCE_THRESHOLD 调低/调高
/// （0..=1，非法值回退 0.6）。调低可减少「低置信度强制 LLM 消歧」，任务计划更顺畅执行。
fn default_confidence_threshold() -> f64 {
    std::env::var("MARGUERITE_INTENT_CONFIDENCE_THRESHOLD")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| (0.0..=1.0).contains(v))
        .unwrap_or(0.6)
}

impl Classifier {
    /// 创建默认分类器，内置高频意图规则
    pub fn new() -> Self {
        Self {
            rules: Self::default_rules(),
            confidence_threshold: default_confidence_threshold(),
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
                intent: IntentType::Build,
                // 关键词的选择有两条约束，改词表前请先读：
                //
                // 1) 打分是「命中数 / 词表长度 × 权重」，所以词表越长、单次命中越不值钱。
                //    词表刻意保持精简：让「帮我做一个 FPS 游戏」这种真实输入一次命中
                //    5 个词（帮我做 / 做一个 / 游戏 / fps / 射击）就能越过 0.6 阈值。
                // 2) 匹配是**子串**匹配（`contains`），不是词边界匹配。因此绝不能放
                //    裸的 "write a" / "make" / "implement"，否则会吃掉现有用例：
                //    "write a test for this"（Test）、"create release notes"（Commit）、
                //    "find the implementation"（Search）都会被误判成 Build。
                //    用 "implement a"（带空格与冠词）就不匹配 "implementation"。
                //
                // 为什么把「游戏 / fps / 射击」这类**领域名词**也算进创建意图：在智能体
                // 工作台里，用户说「游戏」几乎总是「给我做一个游戏」。单独一个「游戏」
                // 只会得到很低的分（1/N）并保持 needs_llm=true，仍会被 LLM 消歧，
                // 比直接落进 Unknown 更诚实。
                keywords: vec![
                    "帮我做".to_string(),
                    "做一个".to_string(),
                    "写一个".to_string(),
                    "写个".to_string(),
                    "开发一个".to_string(),
                    "实现一个".to_string(),
                    "build a".to_string(),
                    "create a".to_string(),
                    "implement a".to_string(),
                    "游戏".to_string(),
                    "fps".to_string(),
                    "射击".to_string(),
                ],
                // 3) **英文自然语序必须能命中**（R23 补）。词表里的 "build a" 只覆盖
                //    "build a X"，而真实英文输入几乎总是 "Build me a X" / "Can you make me a X"。
                //    实测：`Build me a polished first-person shooter game` 在词表上命中 **0**，
                //    旧正则也不命中 → 整条规则 score=0 → 落进 `Unknown`；而语义相同的中文句
                //    命中 5 个词 → 0.645 Build。也就是说创建意图实际上**只对中文有效**。
                //    修法刻意**只扩正则、不动词表**：打分是「命中数 / 词表长度」，词表 12 → 14
                //    会把中文那 5 个命中稀释成 5/14*0.9+0.27 = 0.591 < 0.6，直接打红既有的
                //    `test_classify_build_game_request_regression`（实测边界：词表**最多只能加到 13**）。
                //    正则加成是**定额** `weight * 0.3`、不参与稀释，所以扩正则既补上英文、又不回退中文。
                //    英文句因此停在 0.27、`needs_llm=true` —— 这与上面「领域名词只给低分、
                //    交给 LLM 消歧、比落进 Unknown 更诚实」是同一套设计，不是妥协。
                patterns: vec![
                    Regex::new(concat!(
                        r"(游戏|做一个|写一个|写个|开发一个|实现一个",
                        r"|build a|create a|implement a",
                        r"|build me|create me|make me|write me)",
                    ))
                    .unwrap(),
                ],
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
                    // v3.3：规划/方案/设计类输入归「文档」——复杂任务步骤标题
                    // （架构设计/技术方案等）此前全部落入 Unknown→对话徽章
                    "方案".to_string(),
                    "设计".to_string(),
                    "规划".to_string(),
                    "架构".to_string(),
                    "实施步骤".to_string(),
                ],
                patterns: vec![Regex::new(r"(方案|设计|规划|架构|实施)").unwrap()],
                weight: 0.9,
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
        // v3.3 扩充「文档」规则关键词后（方案/设计/规划/架构），单关键词短句的
        // keyword_ratio 稀释，置信度从 0.10 降到 ~0.09——分类仍正确，仅阈值放宽。
        assert_classify("generate doc for this", IntentType::Doc, 0.05);
        assert_classify("写文档", IntentType::Doc, 0.05);
        assert_classify("add comments to the code", IntentType::Doc, 0.05);
        assert_classify("generate documentation", IntentType::Doc, 0.05);
        // 新增：规划类中文输入应命中 Doc 意图（步骤徽章「文档」）
        assert_classify("设计一个缓存模块", IntentType::Doc, 0.05);
        assert_classify("输出技术方案", IntentType::Doc, 0.05);
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
    fn test_classify_build() {
        assert_classify("build a CLI tool", IntentType::Build, 0.1);
        assert_classify("create a REST service", IntentType::Build, 0.1);
        assert_classify("implement a parser", IntentType::Build, 0.1);
        assert_classify("写一个脚本", IntentType::Build, 0.05);
        assert_classify("开发一个后台服务", IntentType::Build, 0.05);
    }

    /// 回归：**英文自然语序**的创建请求此前落进 `Unknown`（R23 实测）。
    ///
    /// 词表里的 `"build a"` 是子串匹配，覆盖不到 `"Build me a …"`；实测该句在 Build 规则上
    /// 命中 0 个关键词、旧正则也不命中 → score=0 → `Unknown`；而中文同义句是 0.645 的 Build。
    /// 现在由正则兜住：至少是 `Build`（低置信度 + `needs_llm`，交 LLM 消歧），不再当作「没看懂」。
    #[test]
    fn test_classify_build_english_natural_phrasing() {
        let classifier = Classifier::new();
        for text in [
            "Build me a polished first-person shooter game",
            "Can you make me a small CLI tool?",
            "write me a Python script",
        ] {
            let r = classifier.classify(text);
            assert_eq!(
                r.intent,
                IntentType::Build,
                "{text:?} 应识别为 Build，实际 {:?}（confidence {:.3}）",
                r.intent,
                r.confidence
            );
        }

        // 词表未动 ⇒ 中文高置信度路径不得回退（这正是「只扩正则」的原因）
        let zh = classifier.classify("帮我做一个完善的 FPS 第一人称射击游戏");
        assert_eq!(zh.intent, IntentType::Build);
        assert!(
            zh.confidence >= 0.6,
            "中文置信度不得被稀释，实际 {:.3}",
            zh.confidence
        );
        assert!(!zh.needs_llm);

        // 英文句只吃到**定额**正则加成：0.9 * 0.3 = 0.27，达不到 0.6 阈值，
        // 因此仍是 `needs_llm=true` —— 这是**有意的**：低置信度交 LLM 消歧，
        // 比整句落进 Unknown 更诚实。这里把该折衷钉死，避免后人误以为已经等价。
        let en = classifier.classify("Build me a polished first-person shooter game");
        assert!(
            (en.confidence - 0.27).abs() < 1e-9,
            "英文自然语序目前只应拿到正则加成 0.27，实际 {:.3}",
            en.confidence
        );
        assert!(
            en.needs_llm,
            "0.27 < 0.6，英文句应仍标记为需要 LLM 消歧（不要把它当成已等价）"
        );
    }

    /// 回归：真实用户输入此前在 7 条规则上逐条 score=0 → `Unknown` → 展示为「对话」徽章。
    /// 创建型请求是本产品最高频的用法之一，必须被识别为 Build 而不是「未识别」。
    #[test]
    fn test_classify_build_game_request_regression() {
        let classifier = Classifier::new();
        let result = classifier.classify("帮我做一个完善的 FPS 第一人称射击游戏");

        assert_eq!(
            result.intent,
            IntentType::Build,
            "创建型请求不应落进 Unknown，实际为 {:?}",
            result.intent
        );
        // 命中 5 个关键词（帮我做/做一个/游戏/fps/射击）+ 正则加成 → 越过 0.6 阈值。
        assert!(
            result.confidence >= 0.6,
            "真实创建型请求的置信度应 >= 0.6，实际 {:.3}",
            result.confidence
        );
        assert!(
            !result.needs_llm,
            "高置信度的创建型请求不应被强制送去 LLM 消歧"
        );
        assert_eq!(IntentType::Build.task_label(), "编码");
    }

    /// 反例守卫：新增 Build 规则**不得**吃掉既有意图。
    ///
    /// 这里钉住的全部是**子串匹配**带来的真实风险点（匹配用 `contains`，不是词边界）：
    /// 一旦有人把词表放宽成裸的 "write a" / "make" / "implement"，下面四条会立刻变红。
    #[test]
    fn test_build_rule_does_not_swallow_sibling_intents() {
        // "create release notes" 含 "create"，但不含带冠词的 "create a" → 仍是 Commit
        assert_classify("create release notes", IntentType::Commit, 0.1);
        // "implementation" 不匹配 "implement a" → 仍是 Search
        assert_classify("find the implementation", IntentType::Search, 0.1);
        // "write a test for this" 不含 "写一个"/"build a" → 仍是 Test
        assert_classify("write a test for this", IntentType::Test, 0.1);
        // "设计一个缓存模块" 归 Doc 是 v3.3 的既定行为，Build 不得抢走
        assert_classify("设计一个缓存模块", IntentType::Doc, 0.05);
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
