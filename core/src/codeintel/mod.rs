//! 代码智能（CodeIntel）：项目索引 + 混合检索 + 上下文组装
//!
//! 进程级按 workspace 键控的索引缓存，SQLite 持久化到 `config_dir/codeintel`
//! （受 `OVERRIDE_CONFIG_DIR` 影响，单测可隔离，绝不污染真实数据目录）。

pub mod gateway;
pub mod index;
pub mod search;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use crate::codeintel::index::{IndexReport, ProjectIndex, Symbol};
use crate::codeintel::search::{search as hybrid_search, SearchHit};
use crate::persistence::config::Config;

static INDEXES: OnceLock<Mutex<HashMap<String, Arc<ProjectIndex>>>> = OnceLock::new();

fn indexes() -> &'static Mutex<HashMap<String, Arc<ProjectIndex>>> {
    INDEXES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 取（或懒建）workspace 对应的索引。索引库落 `config_dir/codeintel`。
pub fn acquire(workspace: &Path) -> anyhow::Result<Arc<ProjectIndex>> {
    let canonical = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let key = canonical.to_string_lossy().to_string();
    let mut guard = indexes().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(idx) = guard.get(&key) {
        return Ok(idx.clone());
    }
    let db_dir = Config::config_dir().join("codeintel");
    let idx = Arc::new(ProjectIndex::open(&canonical, &db_dir)?);
    guard.insert(key, idx.clone());
    Ok(idx)
}

/// 代码智能门面（供网关 RPC 与工具层复用）
pub struct CodeIntel;

impl CodeIntel {
    /// 刷新（增量）索引
    pub fn index(workspace: &Path) -> anyhow::Result<IndexReport> {
        acquire(workspace)?.refresh()
    }

    /// 混合检索（符号优先 + BM25 文本）
    pub fn search(workspace: &Path, query: &str, limit: usize) -> anyhow::Result<Vec<SearchHit>> {
        let idx = acquire(workspace)?;
        hybrid_search(&idx, query, limit)
    }

    /// 按名称查询符号
    pub fn symbols(workspace: &Path, query: &str, limit: usize) -> anyhow::Result<Vec<Symbol>> {
        acquire(workspace)?.symbols_named(query, limit)
    }

    /// 索引统计（文件 / 符号 / 分块）
    pub fn stats(workspace: &Path) -> anyhow::Result<(usize, usize, usize)> {
        acquire(workspace)?.stats()
    }
}
