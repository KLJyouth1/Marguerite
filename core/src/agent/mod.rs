//! Agent 模块：Agent 生命周期管理和会话上下文
//!
//! 提供 Agent 运行时，管理从初始化到完成的完整生命周期，
//! 维护会话上下文和消息历史。

pub mod benchmarks;
pub mod bloom_cache;
pub mod budget_policy;
pub mod capability;
pub mod capability_bench;
pub mod capability_cards;
pub mod capability_guard;
pub mod expertise;
pub mod multi_strategy;
pub mod prefix_scope;
pub mod profile;
pub mod role_context;
pub mod runtime;
pub mod store;
pub mod stream_loop;
pub mod subagent;
pub mod team;
pub mod tool_cache;
pub mod tool_catalog;
pub mod task_checkpoint;
pub mod tool_compress;
pub mod tool_cycle;
pub mod tool_schema;
pub mod trellis;
pub mod turn_budget;
pub mod worktree;

pub use bloom_cache::{
    embedding_similarity, BloomDecisionCache, BloomParallelPrefix, CacheQuery, SemanticHit,
    SemanticIntentCache, SimilarityFn, SnapshotCache,
};
pub use multi_strategy::{
    run_multi_strategy, strategy, tool_strategy, MultiResult, MultiStrategyStats, Strategy,
    ToolLoopBackend,
};
pub use prefix_scope::{stable_prefix_key, SharedPrefixCache};
pub use profile::{apply_auto_profile, auto_profile, Persona};
pub use runtime::{AgentRuntime, AgentState, Message, SessionContext};
pub use stream_loop::{
    detect_doom_loop, run_tool_loop_streaming, AgentEvent, StreamAggregator, DOOM_LOOP_ABORT,
    DOOM_LOOP_THRESHOLD,
};
pub use subagent::{SubAgent, SubAgentConfig, SubAgentManager, SubAgentStatus};
pub use team::{
    converge_debate, ArgumentEvaluation, DebateArgument, DebateOutcome, MinorityOpinion,
    SycophancySignal, Team, TeamMember, TeamOutcome,
};
pub use tool_cache::{default_is_idempotent, ToolCache, ToolCacheStats};
pub use tool_cycle::{
    tool_fingerprint, CircuitBreaker, CircuitReason, CircuitVerdict, ToolCall, ToolCycle,
    ToolResult,
};
pub use trellis::{trellis_wait_all, trellis_wait_any, TrellisDone};
pub use worktree::{WorktreeHandle, WorktreeInfo, WorktreeManager};
