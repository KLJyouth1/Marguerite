//! 记忆模块：感知、短期与长期记忆
//!
//! 感知记忆：当前会话的上下文存储，纯内存 + SQLite 持久化。
//! 短期记忆：跨会话的全文搜索，使用 SQLite FTS5 实现。
//! 长期记忆：语义向量存储与分层检索（嵌入后端可插拔，默认零依赖特征哈希）。

pub mod behavior;
pub mod compact;
pub mod cross_project;
pub mod embedding;
#[cfg(feature = "real-embed")]
pub mod embedding_real;
pub mod experience;
pub mod federated;
pub mod flyhash;
pub mod gateway;
pub mod hooks;
pub mod home_memory;
pub mod lexical;
pub mod long_term;
pub mod pages;
pub mod recall;
pub mod scoring;
pub mod sensory;
pub mod short_term;
pub mod synapse;
pub mod tiered;
pub mod inject;
pub mod layers;

/// 记忆检索质量的常驻离线回归（夹具 + 棘轮基线，只在 `cargo test` 里编译）。
#[cfg(test)]
mod recall_probe;

pub use self::compact::{
    compact_messages, hybrid_retrieve, render_timeline, token_economics, MemoryHit, Observation,
    TokenEconomics,
};
pub use self::embedding::{Embedding, EmbeddingProvider, LocalEmbeddingBackend};
pub use self::experience::{
    Experience, ExperienceKind, ExperienceLedger, ExperienceMeta, SelfEvolution,
};
pub use self::federated::{
    ConsolidationReport, CrossProjectBackend, ExperienceBackend, FederatedHit, FederatedMemory,
    FederatedResult, FederatedStats, LayerBackend, MemoryTier, RecallMode, SensoryBackend,
    ShortTermBackend,
};
pub use self::flyhash::{FlyHashIndex, FlyHit, SparseVector, WtaFingerprint, FLY_DIM, WTA_ROUNDS};
pub use self::hooks::{MemoryHookEvent, MemoryHooks};
pub use self::long_term::{
    HierarchicalMemory, LocalEmbedding, LongTermMemory, MemoryItem, RetrievalResult,
};
pub use self::pages::{KnowledgePage, PageStore};
pub use self::sensory::SensoryMemory;
pub use self::short_term::{SessionSummary, ShortTermMemory};
pub use self::synapse::{SynapseEdge, SynapseGraph, SynapseStats};
pub use self::tiered::{MemoryTierVariant, TieredEntry, TieredHit, TieredMemory};
