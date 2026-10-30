//! 上下文模块：分页、摘要缓存与 Token 预算管理
//!
//! 极致利用 token：按语义边界分页、LRU 摘要缓存、预算压缩策略。

pub mod budget;
pub mod cache;
pub mod compact_state;
pub mod continuity;
pub mod middle_truncate;
pub mod optimize;
pub mod paging;
pub mod pressure;
pub mod project_docs;
pub mod session;
pub mod spool;

pub use budget::{BudgetAlert, TokenBudget, TokenBudgetError};
pub use cache::{CacheStats, SummaryCache};
pub use paging::{ContextPage, PagingEngine, PagingOptions};
