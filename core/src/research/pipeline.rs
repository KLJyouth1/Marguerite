//! 深度研究 Pipeline —— 多步研究工作流。
//!
//! 职责：
//! - 研究任务分解：query → sub-questions → search → extract → synthesize → report
//! - LLM 驱动的分析与综合（经可注入的 `ResearchLlm`）
//! - 研究报告生成（Markdown 格式）
//! - 检索复用 `kb::KnowledgeBase`，零外部向量依赖
//!
//! 设计：纯编排层，依赖 KB + LLM trait，离线可测（mock 实现）。

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::kb::KnowledgeBase;

/// 研究步骤类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepType {
    /// 查询分解
    Decompose,
    /// 检索
    Search,
    /// 提取
    Extract,
    /// 综合
    Synthesize,
    /// 报告
    Report,
}

/// 研究步骤状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    /// 待执行
    Pending,
    /// 执行中
    Running,
    /// 已完成
    Done,
    /// 失败
    Failed,
}

/// 单条研究来源
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    /// 标题
    pub title: String,
    /// 摘要
    pub summary: String,
    /// 分类
    pub category: String,
    /// 来源 URL
    pub source_url: String,
    /// 重要度
    pub importance: i64,
    /// 命中该来源的查询
    pub query: String,
}

/// 单个研究步骤（运行态）
#[derive(Debug, Clone)]
pub struct ResearchStep {
    /// 步骤 id
    pub id: String,
    /// 步骤类型
    pub step_type: StepType,
    /// 步骤查询
    pub query: String,
    /// 状态
    pub status: StepStatus,
    /// 结果文本
    pub result: String,
    /// 关联来源
    pub sources: Vec<Source>,
    /// 耗时（毫秒）
    pub duration_ms: u64,
}

/// 研究 pipeline 状态
#[derive(Debug, Clone)]
pub struct ResearchPipeline {
    /// pipeline id
    pub id: String,
    /// 原始查询
    pub original_query: String,
    /// 步骤列表
    pub steps: Vec<ResearchStep>,
    /// 最终报告
    pub final_report: String,
    /// 状态
    pub status: String,
    /// 创建时间（毫秒）
    pub created_at_ms: i64,
    /// 完成时间（毫秒）
    pub completed_at_ms: i64,
    /// 总耗时（毫秒）
    pub total_duration_ms: u64,
}

/// LLM 研究能力（可注入；测试用 mock 取代真实调用）
#[async_trait]
pub trait ResearchLlm: Send + Sync {
    /// 将研究查询分解为子问题。
    async fn decompose(&self, query: &str) -> Vec<String>;
    /// 基于来源综合研究报告。
    async fn synthesize(&self, query: &str, sources: &[Source], sub_questions: &[String])
        -> String;
}

/// 把 KB 检索结果映射为研究来源
fn to_source(r: &crate::kb::RetrievalResult, query: &str) -> Source {
    Source {
        title: r.title.clone(),
        summary: r.chunk.clone(),
        category: "kb".to_string(),
        source_url: String::new(),
        importance: (r.score * 10.0).round() as i64,
        query: query.to_string(),
    }
}

/// 将研究查询分解为子问题（带优雅降级）。
pub async fn decompose_query(query: &str, llm: &dyn ResearchLlm) -> Vec<String> {
    let subs = llm.decompose(query).await;
    if subs.is_empty() {
        vec![query.to_string()]
    } else {
        subs
    }
}

/// 检索子问题并提取相关信息（复用 KB BM25）。
pub fn search_and_extract(sub_questions: &[String], kb: &KnowledgeBase) -> Vec<Source> {
    let mut all = Vec::new();
    for q in sub_questions {
        for item in kb.retrieve(q, 5) {
            all.push(to_source(&item, q));
        }
    }
    all
}

/// 综合所有来源生成研究报告（带优雅降级）。
pub async fn synthesize_report(
    query: &str,
    sources: &[Source],
    sub_questions: &[String],
    llm: &dyn ResearchLlm,
) -> String {
    llm.synthesize(query, sources, sub_questions).await
}

/// 执行完整研究 pipeline（异步）。
pub async fn run_pipeline(
    query: &str,
    kb: &KnowledgeBase,
    llm: &dyn ResearchLlm,
    pipeline_id: Option<&str>,
) -> ResearchPipeline {
    let pid = pipeline_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("rp-{}", Utc::now().timestamp_millis()));
    let created = Utc::now().timestamp_millis();
    let mut pipeline = ResearchPipeline {
        id: pid.clone(),
        original_query: query.to_string(),
        steps: Vec::new(),
        final_report: String::new(),
        status: "running".to_string(),
        created_at_ms: created,
        completed_at_ms: 0,
        total_duration_ms: 0,
    };

    // Step 1: 分解
    let t0 = Utc::now().timestamp_millis();
    let subs = decompose_query(query, llm).await;
    pipeline.steps.push(ResearchStep {
        id: format!("{pid}-decompose"),
        step_type: StepType::Decompose,
        query: query.to_string(),
        status: StepStatus::Done,
        result: format!("{:?}", subs),
        sources: Vec::new(),
        duration_ms: (Utc::now().timestamp_millis() - t0) as u64,
    });

    // Step 2: 检索与提取
    let t0 = Utc::now().timestamp_millis();
    let sources = search_and_extract(&subs, kb);
    pipeline.steps.push(ResearchStep {
        id: format!("{pid}-search"),
        step_type: StepType::Search,
        query: query.to_string(),
        status: StepStatus::Done,
        result: format!("找到 {} 个来源", sources.len()),
        sources: sources.clone(),
        duration_ms: (Utc::now().timestamp_millis() - t0) as u64,
    });

    // Step 3: 综合报告
    let t0 = Utc::now().timestamp_millis();
    let report = synthesize_report(query, &sources, &subs, llm).await;
    pipeline.steps.push(ResearchStep {
        id: format!("{pid}-synthesize"),
        step_type: StepType::Synthesize,
        query: query.to_string(),
        status: StepStatus::Done,
        result: report.chars().take(200).collect(),
        sources: Vec::new(),
        duration_ms: (Utc::now().timestamp_millis() - t0) as u64,
    });

    pipeline.final_report = report;
    pipeline.status = "done".to_string();
    pipeline.completed_at_ms = Utc::now().timestamp_millis();
    pipeline.total_duration_ms = (pipeline.completed_at_ms - pipeline.created_at_ms) as u64;
    pipeline
}

// ---------------------------------------------------------------------------
// Mock / 真实 LLM 实现
// ---------------------------------------------------------------------------

/// 测试用 mock：decompose 返回单元素（原查询），synthesize 返回结构化占位。
pub struct MockResearchLlm;

#[async_trait]
impl ResearchLlm for MockResearchLlm {
    async fn decompose(&self, query: &str) -> Vec<String> {
        // 简单按中文/英文分隔符切分，便于测试多子问题；无分隔则返回原查询
        let normalized = query.replace(" and ", ",").replace(" 与 ", ",");
        let parts: Vec<String> = normalized
            .split(['，', ',', '；', ';', '、'])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if parts.len() <= 1 {
            vec![query.to_string()]
        } else {
            parts
        }
    }

    async fn synthesize(&self, query: &str, sources: &[Source], sub: &[String]) -> String {
        let mut out = format!("## 研究报告：{query}\n\n### 关键发现\n");
        for s in sub {
            out.push_str(&format!("- 子问题：{s}\n"));
        }
        out.push_str("\n### 参考来源\n");
        if sources.is_empty() {
            out.push_str("- （无本地来源，建议联网补充）\n");
        } else {
            for (i, src) in sources.iter().enumerate().take(10) {
                out.push_str(&format!("{}. [{}] {}\n", i + 1, src.category, src.title));
            }
        }
        out
    }
}

/// 真实 LLM 实现：复用 `model::llm::LLMClient`（OpenAI 兼容）。
/// 上游失败时不抛错，降级为结构化占位，绝不阻断主流程。
pub struct LlmResearch {
    client: crate::model::llm::LLMClient,
}

impl LlmResearch {
    /// 基于给定配置构造
    pub fn new(client: crate::model::llm::LLMClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ResearchLlm for LlmResearch {
    async fn decompose(&self, query: &str) -> Vec<String> {
        let prompt = format!(
            "将以下研究查询分解为3-5个具体子问题，每个聚焦一个方面。\
             以JSON数组格式返回，不要其他内容。\n查询：{query}\n\
             示例输出：[\"子问题1\", \"子问题2\"]"
        );
        match self
            .client
            .chat_complete(&[crate::types::Message::User {
                content: prompt,
                timestamp: Utc::now(),
            }])
            .await
        {
            Ok(resp) => {
                let text = resp.content;
                if let (Some(s), Some(e)) = (text.find('['), text.rfind(']')) {
                    if let Ok(arr) = serde_json::from_str::<Vec<String>>(&text[s..=e]) {
                        if !arr.is_empty() {
                            return arr;
                        }
                    }
                }
                vec![query.to_string()]
            }
            Err(_) => vec![query.to_string()],
        }
    }

    async fn synthesize(&self, query: &str, sources: &[Source], sub: &[String]) -> String {
        let mut source_text = String::new();
        for (i, src) in sources.iter().enumerate().take(10) {
            source_text.push_str(&format!(
                "\n{}. [{}] {}\n   {}\n",
                i + 1,
                src.category,
                src.title,
                src.summary
            ));
        }
        let prompt = format!(
            "基于以下研究来源，为查询生成结构化研究报告（Markdown）。\n查询：{query}\n\
             子问题：\n{}\n研究来源：\n{}\n\
             请包含：概述 / 关键发现 / 分析与洞察 / 结论与建议 / 参考来源。",
            sub.iter()
                .map(|s| format!("- {s}"))
                .collect::<Vec<_>>()
                .join("\n"),
            source_text
        );
        match self
            .client
            .chat_complete(&[crate::types::Message::User {
                content: prompt,
                timestamp: Utc::now(),
            }])
            .await
        {
            Ok(resp) if !resp.content.trim().is_empty() => resp.content,
            _ => format!("## 研究报告：{query}\n\n（视觉模型/LLM 不可用，返回结构化占位）"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kb_with_docs() -> KnowledgeBase {
        let mut kb = KnowledgeBase::default();
        kb.add_document(
            "Rust Async",
            "Rust 异步编程使用 tokio 与 async/await 模型。",
        );
        kb.add_document("BM25", "BM25 是一种概率信息检索排序函数，用于知识库检索。");
        kb
    }

    #[tokio::test]
    async fn decompose_fallback_single() {
        let llm = MockResearchLlm;
        // 无分隔符 → 返回原查询
        let subs = decompose_query("单体查询", &llm).await;
        assert_eq!(subs, vec!["单体查询".to_string()]);
    }

    #[tokio::test]
    async fn decompose_splits_on_delimiter() {
        let llm = MockResearchLlm;
        let subs = decompose_query("Rust 异步, BM25 检索", &llm).await;
        assert_eq!(subs.len(), 2);
    }

    #[tokio::test]
    async fn search_uses_kb() {
        let kb = kb_with_docs();
        let sources = search_and_extract(&["Rust 异步".to_string()], &kb);
        assert!(!sources.is_empty());
        assert!(sources[0].title.contains("Rust"));
    }

    #[tokio::test]
    async fn run_pipeline_end_to_end() {
        let kb = kb_with_docs();
        let llm = MockResearchLlm;
        let p = run_pipeline("Rust 异步, BM25 检索", &kb, &llm, Some("test-1")).await;
        assert_eq!(p.status, "done");
        assert_eq!(p.steps.len(), 3);
        assert!(p.steps.iter().all(|s| s.status == StepStatus::Done));
        assert!(!p.final_report.is_empty());
        assert!(p.final_report.contains("研究报告"));
    }

    #[tokio::test]
    async fn run_pipeline_empty_kb_graceful() {
        let kb = KnowledgeBase::default();
        let llm = MockResearchLlm;
        let p = run_pipeline("孤立查询", &kb, &llm, Some("test-2")).await;
        assert_eq!(p.status, "done");
        assert!(p.final_report.contains("无本地来源"));
    }
}
