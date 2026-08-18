//! Track E — 第三阶段研究主干（RAG / 知识图谱 / NLP / 缓存命中率 / 记忆增强）。
//!
//! 本模块是协同链路与智能化的纯逻辑主干切片。只做 **Rust 原生、可单测、零新依赖**
//! 的部分；重 ML 推理（ONNX / lgbm 实际推理）、真实向量库、推背图（零来源）明确不落地。
//!
//! ## 透明标注（skeptical，已核实）
//! - **真实 lgbm 权重**：`pyengine/src/opensquilla/squilla_router/models/v4.2_phase3_inference/lgbm_main.bin`
//!   + `lgbm_aux.bin` 确实存在于本地 opensquilla 检出中；`bge_onnx.py` 亦存在。
//!   但本切片 **不链接** 任何 ONNX/torch 二进制 —— 只移植 opensquilla `self_learning` 的
//!   **闸门(gate)/晋升(promote)判定逻辑**（`learning.rs`）。生产接入点：候选 `score` 来自真实
//!   lgbm 推理、`embedding` 来自 bge；本切片仅固化"何时提升 / 何时拦截"的决策面。
//! - **推背图（Tuibei Tu）**：上游零契约、属推测项，**本切片不落地任何推背图代码**，
//!   不写占位假功能。留待用户明确需求（若指"预测/推演"NLP，可化为 speculative item）后再做。
//! - **缓存命中率**：既有的 `cache::semantic::MultiTierStats` 此前未导出命中率，本切片在
//!   `cache/semantic.rs` 上做最小加性方法（`hit_rate` / `total_hits` / `saved_tokens_ratio`），
//!   不改动既有字段与语义。

pub mod context_budget;
pub mod learning;
pub mod nlp;
pub mod recall;
