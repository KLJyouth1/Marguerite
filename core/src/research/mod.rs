//! 深度研究 pipeline（E 期 research/）。
//!
//! 能力对齐 nexustar-api `research_pipeline.py`：查询分解 → 检索 → 提取 → 综合 → 报告。
//! 本地检索复用 `kb::KnowledgeBase`（BM25），LLM 调用走可注入的 `ResearchLlm` trait，
//! 离线可测（mock 实现），且上游失败时优雅降级，绝不阻断主流程。

pub mod pipeline;

use std::sync::{Arc, Mutex};

use crate::kb::KnowledgeBase;

/// 深度研究能力门面：把 BM25 知识库与可注入的 LLM 研究能力封装为
/// 编排器可直接持有 / 注入 / 调用的「一等能力」。
///
/// 设计要点（对齐治理「离线安全、可注入、优雅降级」约束）：
/// - 知识库与 LLM 均以内部可变性（`Mutex`）包裹，编排器与工具共享同一 `Arc`，
///   运行时可增量入库文档（`add_document`）与替换 LLM（`set_llm`），仅需 `&self`；
/// - 默认 LLM 为 [`MockResearchLlm`]（离线安全，绝不联网）；生产侧在具备真实
///   LLM 配置时注入 [`LlmResearch`]；
/// - `research` 调用仅短暂持有 llm 锁取出 `Arc`（立即释放），再持 kb 锁跨 `.await`，
///   避免同时持两把锁跨 await，把死锁面压到最低。
pub struct ResearchCapability {
    kb: Mutex<KnowledgeBase>,
    llm: Mutex<Arc<dyn ResearchLlm>>,
}

impl ResearchCapability {
    /// 构造研究能力（KB + LLM 均显式注入）。
    pub fn new(kb: KnowledgeBase, llm: Arc<dyn ResearchLlm>) -> Self {
        Self {
            kb: Mutex::new(kb),
            llm: Mutex::new(llm),
        }
    }

    /// 离线默认构造：空 KB + Mock LLM（不触发任何网络 / LLM 调用）。
    pub fn offline_default() -> Self {
        Self::new(KnowledgeBase::default(), Arc::new(MockResearchLlm))
    }

    /// 注入（替换）研究用 LLM（生产侧注入真实 `LlmResearch`）。
    pub fn set_llm(&self, llm: Arc<dyn ResearchLlm>) {
        *self.llm.lock().unwrap() = llm;
    }

    /// 增量入库一篇文档（供检索与综合使用）。
    pub fn add_document(&self, title: &str, content: &str) {
        self.kb.lock().unwrap().add_document(title, content);
    }

    /// 已索引文档数（可观测性）。
    pub fn document_count(&self) -> usize {
        self.kb.lock().unwrap().document_count()
    }

    /// 执行深度研究 pipeline，返回结构化的研究产物。
    pub async fn research(&self, query: &str, pipeline_id: Option<&str>) -> ResearchPipeline {
        // 仅短暂持有两把锁取出所需数据（KB 克隆出 owned 副本、Arc 克隆），
        // 立即释放锁后再跨 .await 调用 run_pipeline——避免持有 MutexGuard 跨 await
        // （std::sync::MutexGuard 非 Send，会导致工具 future 非 Send）。
        let llm = self.llm.lock().unwrap().clone();
        let kb = self.kb.lock().unwrap().clone();
        run_pipeline(query, &kb, &*llm, pipeline_id).await
    }
}

pub use pipeline::{
    decompose_query, run_pipeline, search_and_extract, synthesize_report, LlmResearch,
    MockResearchLlm, ResearchLlm, ResearchPipeline, ResearchStep, Source, StepStatus, StepType,
};
