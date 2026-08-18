//! 嵌入抽象与后端注册（DI 接缝）
//!
//! 设计目标：把「嵌入后端」从写死的 `LocalEmbedding` 特征哈希解耦为可注入的抽象，
//! 使真实语义模型（如本地 MiniLM）可在不改动调用点的前提下替换默认实现。
//!
//! - `Embedding`：统一嵌入后端 trait（同步、返回 f64 向量 + 余弦）。
//! - `LocalEmbeddingBackend`：零依赖特征哈希默认实现（包装 `long_term::LocalEmbedding`）。
//! - `EmbeddingProvider`：进程级全局注册，默认 local；可 `set` 注入真实模型后端。
//!
//! 维度：各后端自行声明 `dim()`。`LongTermMemory` 按当前 `Provider.dim()` 比对持久化向量，
//! 维度不符的旧向量在检索时跳过（可 `reindex_all` 重建），避免换模型后旧数据全失效。

use std::sync::{Arc, Mutex, OnceLock};

use tracing::warn;

use crate::memory::long_term::LocalEmbedding;
use crate::persistence::config::Config;

/// 嵌入后端抽象（同步、确定性、零异步约束，便于在检索热路径直接调用）
pub trait Embedding: Send + Sync {
    /// 文本 → 向量（实现应返回 L2 归一化向量，使余弦即点积）
    fn embed(&self, text: &str) -> Vec<f64>;

    /// 向量维度（须恒定；切换后端若维度变化，旧持久化向量按维度不符处理）
    fn dim(&self) -> usize;

    /// 余弦相似度（默认点积实现，要求输入已 L2 归一化）
    fn cosine(&self, a: &[f64], b: &[f64]) -> f64 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }
}

/// 默认零依赖后端：特征哈希词袋（包装既有 `LocalEmbedding`，维度 256）
///
/// 保留为默认实现以保证离线、无模型权重下载、Windows MSVC 零系统依赖。
pub struct LocalEmbeddingBackend;

impl Embedding for LocalEmbeddingBackend {
    fn embed(&self, text: &str) -> Vec<f64> {
        LocalEmbedding::embed(text)
    }

    fn dim(&self) -> usize {
        LocalEmbedding::dim()
    }
}

/// 进程级嵌入后端注册表（可重设、poison-tolerant）
static PROVIDER: OnceLock<Mutex<Arc<dyn Embedding>>> = OnceLock::new();

fn provider_cell() -> &'static Mutex<Arc<dyn Embedding>> {
    PROVIDER.get_or_init(|| Mutex::new(Arc::new(LocalEmbeddingBackend)))
}

/// 嵌入后端提供者：全局取用当前生效后端，或注入真实模型后端
pub struct EmbeddingProvider;

impl EmbeddingProvider {
    /// 当前生效后端（默认 `LocalEmbeddingBackend`）
    pub fn global() -> Arc<dyn Embedding> {
        provider_cell()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// 注入后端（如 `OrtEmbedding`）。失败由调用方决定回退策略。
    pub fn set(backend: Arc<dyn Embedding>) {
        *provider_cell().lock().unwrap_or_else(|e| e.into_inner()) = backend;
    }

    /// 复位为默认零依赖后端（主要用于测试隔离）
    pub fn reset() {
        Self::set(Arc::new(LocalEmbeddingBackend));
    }

    /// 依据配置决定生效后端：
    /// - `backend = local`（默认）或未知值 → 保持零依赖特征哈希。
    /// - `backend = model` → 在 `real-embed` feature 下加载真实模型后端；加载失败或 feature 未启用
    ///   均回退到默认后端并告警（绝不因嵌入后端故障阻断启动）。
    pub fn configure(config: &Config) {
        if !config.embeddings.backend.eq_ignore_ascii_case("model") {
            Self::reset();
            return;
        }

        #[cfg(feature = "real-embed")]
        {
            use tracing::info;
            match crate::memory::embedding_real::OrtEmbedding::try_new(
                config.embeddings.model.clone(),
            ) {
                Ok(backend) => {
                    info!(dim = backend.dim(), "嵌入后端：启用真实模型后端");
                    Self::set(Arc::new(backend));
                }
                Err(e) => {
                    warn!(error = %e, "嵌入后端：真实模型加载失败，回退特征哈希默认后端");
                    Self::reset();
                }
            }
        }

        #[cfg(not(feature = "real-embed"))]
        {
            warn!(
                "嵌入后端：config.backend=model 但引擎未启用 real-embed feature，回退特征哈希默认后端"
            );
            Self::reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_backend_matches_legacy_local_embedding() {
        let b = LocalEmbeddingBackend;
        let v = b.embed("rust async tokio");
        assert_eq!(v.len(), LocalEmbedding::dim());
        // 确定性：默认后端应与既有 LocalEmbedding 完全一致
        assert_eq!(v, LocalEmbedding::embed("rust async tokio"));
        let same = b.embed("tokio async rust");
        assert!(
            (b.cosine(&v, &same) - LocalEmbedding::cosine(&v, &same)).abs() < 1e-12,
            "默认后端余弦须与既有实现一致"
        );
    }

    #[test]
    fn provider_default_is_local_and_reset_is_idempotent() {
        // 仅复位（写回默认零依赖后端），不切换到改变行为的后端，避免污染并行测试
        EmbeddingProvider::reset();
        assert_eq!(EmbeddingProvider::global().dim(), LocalEmbedding::dim());
        EmbeddingProvider::reset();
        assert_eq!(EmbeddingProvider::global().dim(), LocalEmbedding::dim());

        // set 回同一默认实现应生效且维度不变（验证注入机制本身）
        EmbeddingProvider::set(Arc::new(LocalEmbeddingBackend));
        assert_eq!(EmbeddingProvider::global().dim(), LocalEmbedding::dim());
    }
}
