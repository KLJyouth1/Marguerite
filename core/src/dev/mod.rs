//! Offline development context pipeline.
//!
//! This module is deterministic and provider-agnostic: it classifies intent,
//! allocates a bounded L0-L3 context budget, and renders a structured template.

mod intent;
mod pipeline;
mod template;

pub use intent::{classify_intent, DevIntent};
pub use pipeline::{BudgetAllocation, DevPipeline, DevPipelineError, LayerBudget, PipelineOutput};
pub use template::{render_template, TemplateError, TemplateInput, TemplateOutput};

#[cfg(test)]
mod tests;
