//! 模型模块：LLM 调用封装、模型路由与双模型协作
//!
//! 提供统一的 LLM API 调用接口、基于任务类型的模型路由能力，
//! 以及规划/执行双模型协作。

pub mod bloom_failover;
pub mod byok;
pub mod cap;
pub mod deepseek_recipe;
pub mod dual;
pub mod ensemble;
pub mod error_taxonomy;
pub mod ensemble_deep;
pub mod fleet;
pub mod llm;
pub mod media_capability;
pub mod omni;
pub mod tokenizer;
pub mod omni_execution;
pub mod persona;
pub mod prefetch;
pub mod prompt_layout;
pub mod routing_contract;
pub mod registry;
pub mod router;
pub mod selector;
pub mod thinking;
pub mod tier;
pub mod wire;

pub use bloom_failover::{
    CandidateCredential, ExecutionResult, FailoverDecision, FailoverError, FailoverPolicy, SkipReason,
};
pub use byok::{chmod_0600, decrypt_api_key, encrypt_api_key, is_encrypted, resolve_api_key};
pub use cap::{
    role_requirements_for, score_role_match, CapabilityState, ModelOffering, OfferingCapabilities,
    ResolvedRoleRoute, RoleCapRouter, RoleRequirements,
};
pub use dual::{DualModelCoordinator, ExecutionReport, Plan, PlanStep, StepStatus};
pub use error_taxonomy::{code_of, provider_error_of, user_message_of, ProviderError, ProviderErrorClass};
pub use ensemble::{BloomOrchestrator, BloomResult, ChatBackend};
pub use ensemble_deep::{run_deep, DeepReasoning, DeepResult};
pub use fleet::{
    FleetConfig, FleetDecision, FleetDecisionSource, FleetRouter, ReasoningLevel, RoleBinding,
    RoleId, RoleKind,
};
pub use llm::{
    LLMChunk, LLMClient, LLMConfig, LLMProvider, LLMResponse, StreamReceiver, TokenUsage,
};
pub use omni::{OmniCandidateScore, OmniDecision, OmniScorer, OmniWeights, OmniWeightsError, OmniWeightsSource};
pub use persona::{Moli, PersonaAspect, SourceArchetype};
pub use prefetch::{prefetch_counters, PrefetchDedup, PrefetchStats, DEFAULT_PREFETCH_TTL};
pub use registry::{ModelProfile, ModelRegistry, ProfileKind};
pub use router::{Complexity, IntentType, ModelRouter, ModelRoutingRule};
pub use selector::{models_path, profile_from_flags, ModelSelector};
