//! Structured, non-placeholder template rendering.

use serde::{Deserialize, Serialize};

use super::DevIntent;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateInput {
    pub goal: String,
    pub intent: Option<DevIntent>,
    #[serde(skip)]
    unknown_field: Option<String>,
}

impl TemplateInput {
    pub fn new(goal: impl Into<String>) -> Self {
        Self { goal: goal.into(), intent: None, unknown_field: None }
    }

    pub fn with_intent(mut self, intent: DevIntent) -> Self {
        self.intent = Some(intent);
        self
    }

    /// Test/helper representing input from an untrusted structured payload.
    pub fn with_unknown_field(mut self, field: impl Into<String>) -> Result<Self, TemplateError> {
        self.unknown_field = Some(field.into());
        Err(TemplateError::InvalidField { field: self.unknown_field.clone().unwrap_or_default() })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateOutput {
    pub schema_version: String,
    pub summary: String,
    pub steps: Vec<String>,
    pub risks: Vec<String>,
    pub verification: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TemplateError {
    #[error("目标不能为空")]
    EmptyGoal,
    #[error("不支持的字段: {field}")]
    InvalidField { field: String },
}

pub fn render_template(input: TemplateInput) -> Result<TemplateOutput, TemplateError> {
    let goal = input.goal.trim();
    if goal.is_empty() {
        return Err(TemplateError::EmptyGoal);
    }
    if input.unknown_field.is_some() {
        return Err(TemplateError::InvalidField { field: input.unknown_field.unwrap_or_default() });
    }
    let intent = input.intent.unwrap_or_else(|| super::classify_intent(goal));
    let kind = match intent {
        DevIntent::Coding => "编码任务",
        DevIntent::Reasoning => "分析任务",
        DevIntent::Chat => "对话任务",
        DevIntent::General => "通用任务",
    };
    Ok(TemplateOutput {
        schema_version: "dev.v1".to_string(),
        summary: format!("{}：{}", kind, goal),
        steps: vec!["确认目标与约束".to_string(), "执行离线处理并记录结果".to_string(), "按验收项复核输出".to_string()],
        risks: vec!["输入信息不足可能导致结果不完整".to_string(), "预算耗尽时需缩减上下文".to_string()],
        verification: vec!["检查输出字段完整性".to_string(), "确认分层预算未超出总预算".to_string()],
    })
}
