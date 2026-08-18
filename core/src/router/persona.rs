//! persona 名册模型 + 默认 catalog 草例（MVP 内置，5 档各一）。

use super::*;

/// 单条匹配规则（规则版：小写子串匹配 + 加权）。
#[derive(Debug, Clone)]
pub struct MatchRule {
    /// 命中其一即 +weight（小写子串匹配）
    pub keywords: Vec<String>,
    /// 累加权重，最终归一化到 [0,1]
    pub weight: f64,
}

impl MatchRule {
    /// 消息（小写）命中任一关键词（小写子串）即视为命中。
    pub fn matches(&self, message: &str) -> bool {
        let m = message.to_ascii_lowercase();
        self.keywords
            .iter()
            .any(|kw| m.contains(&kw.to_ascii_lowercase()))
    }
}

/// 内置 persona（新建，不搬运任何 TS 代码）。
#[derive(Debug, Clone)]
pub struct Persona {
    pub id: String,
    pub tier: PersonaTier,
    pub label: String,
    /// 可选模型偏好（MVP 仅记录，不强制切换模型）
    pub model_preference: Option<String>,
    pub default_thinking_mode: ThinkingMode,
    pub default_prompt_policy: PromptPolicy,
    /// persona 角色提示（始终注入的 system prompt 片段，与 P0/P1 策略片段分离）
    pub system_prompt: String,
    pub match_rules: Vec<MatchRule>,
    pub enabled: bool,
}

/// persona 名册（进程单例 / 配置驱动，MVP 内置默认 + 允许配置覆盖）。
#[derive(Debug, Clone, Default)]
pub struct PersonaCatalog {
    pub personas: Vec<Persona>,
}

impl PersonaCatalog {
    /// 按 tier 查找首个匹配 persona。
    pub fn lookup(&self, tier: PersonaTier) -> Option<&Persona> {
        self.personas.iter().find(|p| p.tier == tier)
    }

    /// 默认 catalog：5 档各一，tier 唯一且非空（MVP 内置）。
    pub fn default_catalog() -> Self {
        PersonaCatalog {
            personas: vec![
                Persona {
                    id: "general".into(),
                    tier: PersonaTier::General,
                    label: "通用助手".into(),
                    model_preference: None,
                    default_thinking_mode: ThinkingMode::T0,
                    default_prompt_policy: PromptPolicy::P0,
                    system_prompt: "你是通用 AI 助手，简洁直接地回答用户问题。".into(),
                    match_rules: vec![MatchRule {
                        keywords: vec![
                            "你好".into(),
                            "闲聊".into(),
                            "翻译".into(),
                            "总结".into(),
                            "hello".into(),
                            "hi".into(),
                            "help".into(),
                        ],
                        weight: 0.6,
                    }],
                    enabled: true,
                },
                Persona {
                    id: "coder".into(),
                    tier: PersonaTier::Coder,
                    label: "代码专家".into(),
                    model_preference: Some("deepseek-coder".into()),
                    default_thinking_mode: ThinkingMode::T1,
                    default_prompt_policy: PromptPolicy::P1,
                    system_prompt: "你是代码专家，先想后写，给出可运行的改动。".into(),
                    match_rules: vec![MatchRule {
                        keywords: vec![
                            "fn ".into(),
                            "def ".into(),
                            "impl ".into(),
                            "cargo".into(),
                            "代码".into(),
                            "bug".into(),
                            "修复".into(),
                            "重构".into(),
                            "test".into(),
                        ],
                        weight: 0.7,
                    }],
                    enabled: true,
                },
                Persona {
                    id: "reasoner".into(),
                    tier: PersonaTier::Reasoner,
                    label: "推理专家".into(),
                    model_preference: None,
                    default_thinking_mode: ThinkingMode::T1,
                    default_prompt_policy: PromptPolicy::P1,
                    system_prompt: "你是深度推理模型，拆解约束、举反例、验证每步后作答。".into(),
                    match_rules: vec![MatchRule {
                        keywords: vec![
                            "proof".into(),
                            "theorem".into(),
                            "证明".into(),
                            "推理".into(),
                            "solve".into(),
                            "数学".into(),
                            "imo".into(),
                        ],
                        weight: 0.7,
                    }],
                    enabled: true,
                },
                Persona {
                    id: "tooler".into(),
                    tier: PersonaTier::Tooler,
                    label: "工具操控".into(),
                    model_preference: None,
                    default_thinking_mode: ThinkingMode::T1,
                    default_prompt_policy: PromptPolicy::P0,
                    system_prompt: "你主动调用工具完成任务，先思考再行动。".into(),
                    match_rules: vec![MatchRule {
                        keywords: vec![
                            "tool".into(),
                            "工具".into(),
                            "终端".into(),
                            "shell".into(),
                            "浏览器".into(),
                            "桌面".into(),
                            "execute".into(),
                        ],
                        weight: 0.7,
                    }],
                    enabled: true,
                },
                Persona {
                    id: "explorer".into(),
                    tier: PersonaTier::Explorer,
                    label: "长上下文".into(),
                    model_preference: None,
                    default_thinking_mode: ThinkingMode::T1,
                    default_prompt_policy: PromptPolicy::P1,
                    system_prompt: "你处理长上下文，渐进压缩，先给结论再补细节。".into(),
                    match_rules: vec![MatchRule {
                        keywords: vec![
                            "长任务".into(),
                            "整个仓库".into(),
                            "总结这".into(),
                            "超长".into(),
                            "entire document".into(),
                            "whole repo".into(),
                        ],
                        weight: 0.7,
                    }],
                    enabled: true,
                },
            ],
        }
    }
}
