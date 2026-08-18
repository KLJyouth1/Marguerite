//! 模型模块：LLM 调用封装、模型路由与双模型协作
//!
//! 提供统一的 LLM API 调用接口、基于任务类型的模型路由能力，
//! 以及规划/执行双模型协作。

pub mod bloom;
pub mod bloom_deep;
pub mod byok;
pub mod cap;
pub mod dual;
pub mod fleet;
pub mod llm;
pub mod prefetch;
pub mod registry;
pub mod router;
pub mod selector;
pub mod thinking;

pub use bloom::{BloomOrchestrator, BloomResult, ChatBackend};
pub use bloom_deep::{run_deep, DeepReasoning, DeepResult};
pub use byok::{chmod_0600, decrypt_api_key, encrypt_api_key, is_encrypted, resolve_api_key};
pub use dual::{DualModelCoordinator, ExecutionReport, Plan, PlanStep, StepStatus};
pub use fleet::{
    FleetConfig, FleetDecision, FleetDecisionSource, FleetRouter, ReasoningLevel, RoleBinding,
    RoleId, RoleKind,
};
pub use llm::{
    LLMChunk, LLMClient, LLMConfig, LLMProvider, LLMResponse, StreamReceiver, TokenUsage,
};
pub use prefetch::{PrefetchDedup, PrefetchStats, DEFAULT_PREFETCH_TTL};
pub use registry::{ModelProfile, ModelRegistry, ProfileKind};
pub use router::{Complexity, IntentType, ModelRouter, ModelRoutingRule};
pub use cap::{
    role_requirements_for, score_role_match, CapabilityState, ModelOffering, OfferingCapabilities,
    ResolvedRoleRoute, RoleCapRouter, RoleRequirements,
};
pub use selector::{models_path, profile_from_flags, ModelSelector};
