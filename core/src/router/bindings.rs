//! 与现有推理路径的适配：思维模式 / prompt 策略 ↔ ThinkingEffort / system 片段桥接。

use super::*;
use crate::model::thinking::ThinkingEffort;

impl ThinkingMode {
    /// T0 -> Off（低延迟、无思考预算）；T1 -> Deep（开思考）。
    pub fn to_thinking_effort(self) -> ThinkingEffort {
        match self {
            ThinkingMode::T0 => ThinkingEffort::Off,
            ThinkingMode::T1 => ThinkingEffort::Deep,
        }
    }
}

impl PromptPolicy {
    /// P0 -> 空（不额外注入）；P1 -> 稳定策略片段（思考/格式引导）。
    pub fn system_fragment(self) -> String {
        match self {
            PromptPolicy::P0 => String::new(),
            PromptPolicy::P1 => {
                "你在作答前先完整推理：拆解约束、列出假设、主动寻找反例与边界情况，再给出最终答案；\
                 复杂任务先列执行计划，逐步推进，每步报告进度。"
                    .to_string()
            }
        }
    }
}

impl RouteDecision {
    /// 本决策的思考预算档位：会话级/请求级覆盖（T3）优先，否则按 thinking_mode。
    pub fn thinking_effort(&self) -> ThinkingEffort {
        self.effort_override
            .unwrap_or_else(|| self.thinking_mode.to_thinking_effort())
    }
}
