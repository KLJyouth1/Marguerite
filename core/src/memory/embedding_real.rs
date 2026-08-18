//! 真实语义嵌入后端（feature = `real-embed`）
//!
//! 用 `fastembed` 在本地加载 sentence-transformers 模型（默认 all-MiniLM-L6-v2，384 维），
//! 提供真语义向量。模型权重首次使用时从 HuggingFace 下载（约 80MB），故离线不可用；
//! 加载/下载失败由 `EmbeddingProvider::configure` 统一回退到默认特征哈希后端，不阻断启动。
//!
//! 诚实边界：当前仅接线 `AllMiniLML6V2`；`config.embeddings.model` 的自定义模型标识暂未映射到
//! 其他 `EmbeddingModel` 变体（一律回退默认），后续可按需扩展 `resolve_model`。

use anyhow::Context;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use crate::memory::embedding::Embedding;

/// 真实模型嵌入后端（fastembed / ONNX Runtime，本地推理）
pub struct OrtEmbedding {
    model: TextEmbedding,
    dim: usize,
}

impl OrtEmbedding {
    /// 加载模型（留空或未知标识一律用默认 all-MiniLM-L6-v2）
    pub fn try_new(_model: Option<String>) -> anyhow::Result<Self> {
        let emb_model = EmbeddingModel::AllMiniLML6V2;
        let model = TextEmbedding::try_new(
            InitOptions::new(emb_model).with_show_download_progress(false),
        )
        .context(
            "加载 fastembed 模型失败（首次使用需联网下载模型权重，离线环境请保持 backend=local）",
        )?;
        Ok(Self { model, dim: 384 })
    }
}

impl Embedding for OrtEmbedding {
    fn embed(&self, text: &str) -> Vec<f64> {
        match self.model.embed(vec![text.to_string()], None) {
            Ok(mut v) if !v.is_empty() => v.remove(0).into_iter().map(|x| x as f64).collect(),
            // 推理异常：退回当前维度的零向量（检索层按相似度≈0 自然不命中，不 panic）
            _ => vec![0.0; self.dim],
        }
    }

    fn dim(&self) -> usize {
        self.dim
    }
}

/// 运行时验证（仅 `--features real-embed` 下编译/运行）：
/// 确认 `config.embeddings.backend=model` 时 `EmbeddingProvider` 真的加载了 384 维真实后端，
/// 而非静默回退到 256 维特征哈希默认后端。
#[cfg(all(test, feature = "real-embed"))]
mod real_embed_verify {
    use crate::memory::embedding::{Embedding, EmbeddingProvider};
    use crate::persistence::config::Config;

    #[test]
    fn real_backend_loads_when_configured() {
        let mut cfg = Config::default();
        cfg.embeddings.backend = "model".to_string();
        EmbeddingProvider::configure(&cfg);

        let backend = EmbeddingProvider::global();
        assert_eq!(
            backend.dim(),
            384,
            "真实语义嵌入后端未加载：dim != 384，可能回退到默认特征哈希（256）"
        );

        let vec = backend.embed("Marguerite 是一个本地优先、离线可用的 AI 工作台");
        assert_eq!(vec.len(), 384, "嵌入向量长度应为 384");
        assert!(
            vec.iter().any(|x| *x != 0.0),
            "嵌入向量全零，模型推理可能异常"
        );
    }
}
