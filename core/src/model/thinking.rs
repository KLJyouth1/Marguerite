//! DeepSeek-V4-Pro 极限思考与超长上下文专项（模型层自适应）。
//!
//! 目标：针对 DeepSeek-V4-Pro-0813 的基准短板做专项优化——
//! - `HLE / Agents' Last Exam / AutomationBench`（推理/长任务）：开高 `reasoning_effort` + 大思考预算 → 极限思考。
//! - `DeepSWE / Terminal Bench / NL2Repo`（代码长输出）：代码任务自动放大 `max_tokens`，防推理+输出共享预算时被截断。
//! - `DSBench / Toolathlon`（工具/全栈）：工具任务走高温度 + 富工具回路。
//! - 长任务：1M 上下文 + 精细压缩，不提前压缩掉关键中间结论。
//!
//! 本模块只产出**可观测、可测试**的档位决策，不臆造后端私有字段：
//! `reasoning_effort` 与既有 `mode/spec.rs` 的 `reasoning_effort` 语义一致（low/medium/high），
//! 对不认识的 provider 不注入（降级为仅本地 `max_tokens` 自适应，绝对安全）。

use serde::{Deserialize, Serialize};

/// DeepSeek-V4-Pro 正式版模型名（作为自适应档位的语义锚点；不强制匹配，仅作参考）
pub const V4_PRO_MODEL: &str = "deepseek-v4-pro";

/// 384K 输出档：长任务/代码任务的最大输出预算（DeepSeek-V4-Pro 支持大 max_tokens）
pub const MAX_TOKENS_LARGE: u32 = 384 * 1024;
/// 160K 输出档：推理/工具任务
pub const MAX_TOKENS_BIG: u32 = 160 * 1024;
/// 普通对话档
pub const MAX_TOKENS_NORMAL: u32 = 32 * 1024;
/// 1M 上下文：长任务模式的目标上下文窗口
pub const CONTEXT_WINDOW_1M: u32 = 1_000_000;
/// 4K 上下文：普通对话
pub const CONTEXT_WINDOW_DEFAULT: u32 = 64 * 1024;

/// 思考深度档位
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThinkingEffort {
    /// 关闭思考（纯对话）
    #[default]
    Off,
    /// 常规思考
    Normal,
    /// 深度思考（推理/代码）
    Deep,
    /// 极限思考（HLE/Agents' Last Exam 等难推理）
    Max,
}

impl ThinkingEffort {
    /// 映射为 DeepSeek 兼容的 `reasoning_effort` 字符串（与 `mode/spec.rs` 语义一致）。
    /// Off 返回 None（不注入字段）。
    pub fn reasoning_effort(self) -> Option<&'static str> {
        match self {
            ThinkingEffort::Off => None,
            ThinkingEffort::Normal => Some("low"),
            ThinkingEffort::Deep => Some("medium"),
            ThinkingEffort::Max => Some("high"),
        }
    }
    /// 是否在此档位注入思考系统提示（引导模型先推理再作答）
    pub fn wants_reasoning_prompt(self) -> bool {
        matches!(self, ThinkingEffort::Deep | ThinkingEffort::Max)
    }
}

/// 任务类型（用于自适应思考/输出/上下文预算）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    /// 普通对话
    Chat,
    /// 代码生成/修复/仓库改造（DeepSWE / Terminal Bench / NL2Repo）
    Code,
    /// 数学/逻辑/难推理（HLE / Agents' Last Exam / MATH）
    Reasoning,
    /// 工具调用为主（Toolathlon / AutomationBench / DSBench）
    Tool,
    /// 长文档/长任务（1M 上下文）
    Long,
}

impl TaskKind {
    /// 从提示词启发式分类（保守：无法判断时归为 Chat）。
    /// 纯规则、无依赖、可测试。
    pub fn classify(prompt: &str) -> Self {
        let p = prompt.to_ascii_lowercase();
        let has_code = [
            "```",
            "fn ",
            "impl ",
            "def ",
            "class ",
            "return ",
            "function ",
            "import ",
            "cargo",
            "compile",
            "refactor",
            "重构",
            "代码",
            "bug",
            "修复",
            "测试",
            "test",
        ]
        .iter()
        .any(|k| p.contains(k));
        let has_reasoning = [
            "proof",
            "theorem",
            "verify",
            "prove",
            "证明",
            "推理",
            "solve",
            "数学",
            "abc conjecture",
            "imo",
            "olympiad",
        ]
        .iter()
        .any(|k| p.contains(k));
        let has_long = [
            "long task",
            "长任务",
            "whole repo",
            "整个仓库",
            "very long",
            "续写",
            "总结这",
            "entire document",
            "超长",
        ]
        .iter()
        .any(|k| p.contains(k));
        let has_tool = [
            "tool",
            "工具",
            "call",
            "execute",
            "run command",
            "调用",
            "终端",
            "shell",
        ]
        .iter()
        .any(|k| p.contains(k));

        if has_long {
            TaskKind::Long
        } else if has_code {
            TaskKind::Code
        } else if has_reasoning {
            TaskKind::Reasoning
        } else if has_tool {
            TaskKind::Tool
        } else {
            TaskKind::Chat
        }
    }
}

/// 自适应思考/输出/上下文档位（一次调用的完整决策）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThinkingProfile {
    pub effort: ThinkingEffort,
    pub max_tokens: u32,
    pub context_window: u32,
}

/// 为给定任务类型产出自适应档位（DeepSeek-V4-Pro 专项基准映射）。
pub fn profile_for(kind: TaskKind) -> ThinkingProfile {
    match kind {
        // 代码：解决 DeepSWE 截断的根本——放大输出预算，思考 medium。
        TaskKind::Code => ThinkingProfile {
            effort: ThinkingEffort::Deep,
            max_tokens: MAX_TOKENS_LARGE,
            context_window: 256 * 1024,
        },
        // 难推理：极限思考 + 大思考预算，输出留给 reasoning_content。
        TaskKind::Reasoning => ThinkingProfile {
            effort: ThinkingEffort::Max,
            max_tokens: MAX_TOKENS_BIG,
            context_window: 128 * 1024,
        },
        // 工具：普通思考 + 中输出，富工具回路。
        TaskKind::Tool => ThinkingProfile {
            effort: ThinkingEffort::Normal,
            max_tokens: MAX_TOKENS_BIG,
            context_window: 128 * 1024,
        },
        // 长任务：1M 上下文 + 大输出，配合精细压缩。
        TaskKind::Long => ThinkingProfile {
            effort: ThinkingEffort::Deep,
            max_tokens: MAX_TOKENS_LARGE,
            context_window: CONTEXT_WINDOW_1M,
        },
        // 对话：小预算、低延迟。
        TaskKind::Chat => ThinkingProfile {
            effort: ThinkingEffort::Off,
            max_tokens: MAX_TOKENS_NORMAL,
            context_window: CONTEXT_WINDOW_DEFAULT,
        },
    }
}

/// 极限思考系统提示：在 `effort.wants_reasoning_prompt()` 时注入，引导模型先深思再作答。
pub fn reasoning_system_prompt() -> &'static str {
    "你是深度推理模型。面对困难问题，请先完整推理（reasoning），\
     拆解约束、验证每步依据、主动寻找反例与边界情况，再给出最终答案。\
     必要时给出多个候选方案并说明为何选择其一。不要过早收敛。"
}

/// 依据模型名是否为 DeepSeek 推理系，决定是否注入 `reasoning_effort` 字段。
/// 对未知 provider 返回 false → 调用方仅做本地 `max_tokens` 自适应（安全降级）。
pub fn supports_reasoning_effort(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.contains("deepseek") || m.starts_with("reasoner") || m.contains("v4-pro")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_routes_code() {
        assert_eq!(
            TaskKind::classify("请修复这个 bug：fn broken() { return; }"),
            TaskKind::Code
        );
        assert_eq!(
            TaskKind::classify("cargo build 报错，帮我改"),
            TaskKind::Code
        );
    }

    #[test]
    fn classify_routes_reasoning() {
        assert_eq!(
            TaskKind::classify("证明费马大定理的 n=3 情形"),
            TaskKind::Reasoning
        );
        assert_eq!(
            TaskKind::classify("solve the olympiad inequality"),
            TaskKind::Reasoning
        );
    }

    #[test]
    fn classify_routes_long_and_tool() {
        assert_eq!(TaskKind::classify("总结整个仓库的架构"), TaskKind::Long);
        assert_eq!(
            TaskKind::classify("调用工具执行 shell 命令"),
            TaskKind::Tool
        );
    }

    #[test]
    fn classify_defaults_to_chat() {
        assert_eq!(TaskKind::classify("你好"), TaskKind::Chat);
        assert_eq!(TaskKind::classify(""), TaskKind::Chat);
    }

    #[test]
    fn profile_code_uses_large_output_to_avoid_truncation() {
        // DeepSWE/Terminal Bench 截断元凶是 max_tokens 太小 → 代码必须大输出。
        let p = profile_for(TaskKind::Code);
        assert_eq!(p.effort, ThinkingEffort::Deep);
        assert!(p.max_tokens >= 32 * 1024, "代码任务输出应足够大防截断");
    }

    #[test]
    fn profile_reasoning_uses_max_effort() {
        let p = profile_for(TaskKind::Reasoning);
        assert_eq!(p.effort, ThinkingEffort::Max);
        assert_eq!(p.effort.reasoning_effort(), Some("high"));
        assert!(p.effort.wants_reasoning_prompt());
    }

    #[test]
    fn profile_long_uses_1m_context() {
        let p = profile_for(TaskKind::Long);
        assert_eq!(p.context_window, CONTEXT_WINDOW_1M);
    }

    #[test]
    fn profile_chat_keeps_small_budget() {
        let p = profile_for(TaskKind::Chat);
        assert_eq!(p.effort, ThinkingEffort::Off);
        assert_eq!(p.effort.reasoning_effort(), None);
        assert!(!p.effort.wants_reasoning_prompt());
    }

    #[test]
    fn effort_mapping_is_consistent() {
        assert_eq!(ThinkingEffort::Normal.reasoning_effort(), Some("low"));
        assert_eq!(ThinkingEffort::Deep.reasoning_effort(), Some("medium"));
        assert_eq!(ThinkingEffort::Max.reasoning_effort(), Some("high"));
    }

    #[test]
    fn supports_reasoning_effort_matches_deepseek() {
        assert!(supports_reasoning_effort("deepseek-v4-pro-0813"));
        assert!(supports_reasoning_effort("deepseek-reasoner"));
        assert!(!supports_reasoning_effort("gpt-4o"));
    }
}
