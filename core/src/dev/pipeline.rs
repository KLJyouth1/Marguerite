//! Budgeted orchestration for the offline development context pipeline.

use serde::{Deserialize, Serialize};

use super::{classify_intent, render_template, DevIntent, TemplateError, TemplateInput, TemplateOutput};

/// One context layer's requested and granted token budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerBudget {
    pub layer: u8,
    pub requested: u64,
    pub allocated: u64,
}

/// Auditable L0-L3 budget allocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetAllocation {
    pub total: u64,
    pub layers: [LayerBudget; 4],
}

impl BudgetAllocation {
    pub fn allocate(total: u64, requested: [u64; 4]) -> Result<Self, DevPipelineError> {
        if total == 0 {
            return Err(DevPipelineError::ZeroBudget);
        }
        if requested.contains(&0) {
            return Err(DevPipelineError::ZeroLayerBudget);
        }
        let requested_sum = requested
            .iter()
            .try_fold(0u64, |sum, value| sum.checked_add(*value))
            .ok_or(DevPipelineError::Overflow)?;
        if requested_sum > total {
            return Err(DevPipelineError::InsufficientBudget { requested: requested_sum, total });
        }
        let layers = std::array::from_fn(|index| LayerBudget {
            layer: index as u8,
            requested: requested[index],
            allocated: requested[index],
        });
        Ok(Self { total, layers })
    }

    pub fn audit(&self) -> String {
        self.layers
            .iter()
            .map(|layer| format!("L{} requested={} allocated={}", layer.layer, layer.requested, layer.allocated))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DevPipelineError {
    #[error("总预算必须大于零")]
    ZeroBudget,
    #[error("每层预算必须大于零")]
    ZeroLayerBudget,
    #[error("预算总和超出上限: requested={requested}, total={total}")]
    InsufficientBudget { requested: u64, total: u64 },
    #[error("预算计算溢出")]
    Overflow,
    #[error("模板错误: {0}")]
    Template(#[from] TemplateError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineOutput {
    pub intent: DevIntent,
    pub allocation: BudgetAllocation,
    pub template: TemplateOutput,
}

#[derive(Debug, Clone, Copy)]
pub struct DevPipeline {
    max_budget: u64,
}

impl DevPipeline {
    pub fn new(max_budget: u64) -> Self {
        Self { max_budget }
    }

    pub fn run(&self, goal: &str, requested: [u64; 4]) -> Result<PipelineOutput, DevPipelineError> {
        let allocation = BudgetAllocation::allocate(self.max_budget, requested)?;
        let intent = classify_intent(goal);
        let template = render_template(TemplateInput::new(goal).with_intent(intent))?;
        Ok(PipelineOutput { intent, allocation, template })
    }
}
