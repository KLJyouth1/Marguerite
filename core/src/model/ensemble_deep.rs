//! Bloom 深度推理管线 — EvoMaster 式「Solve → Critique → Rewrite → Select」（Phase G4）。
//!
//! 借鉴 EvoMaster 的「生成-评估-择优」循环与 Self-Refine 的「初稿→批判→重写」范式，
//! 用多轮 LLM 调用把一次回答打磨成更高质解，专项提升代码（DeepSWE/NL2Repo/Terminal Bench）
//! 与推理（HLE/Agents' Last Exam）类基准。
//!
//! 四阶段：
//! 1. **Solve**：Captain 生成第一版候选。
//! 2. **Critique**：Critic 指出疏漏、反例、边界缺陷（不重写）。
//! 3. **Rewrite**：Captain 基于批判重写（吸收有效意见）。
//! 4. **Select**：Critic 相对取舍，产出最终答案（原稿 vs 重写稿，择优或合并）。
//!
//! 避免重写退化：Critic 被明确要求"无实质改进时返回原稿"，Select 阶段以原稿为基准，
//! 只接受能消除已指出缺陷的改进，防止"为了改而改"。

use anyhow::Result;
use chrono::Utc;

use crate::model::ensemble::ChatBackend;
use crate::types::Message;

/// 四阶段深度推理编排器（泛型后端，零成本抽象；用 `ChatBackend` 便于注入 FakeBackend 单测）
pub struct DeepReasoning<B: ChatBackend> {
    /// 解决者/重写者（可用主模型 Captain）
    solver: B,
    /// 批判者/选择者（可用副模型 Critic）
    critic: B,
}

/// 深度推理结果
#[derive(Debug)]
pub struct DeepResult {
    /// 最终答案
    pub final_answer: String,
    /// 初稿（Solve 阶段的原始输出）
    pub draft: String,
    /// 批判意见（Critique 阶段）
    pub critique: String,
    /// 重写稿（Rewrite 阶段，可能被 Select 舍弃）
    pub rewritten: String,
}

impl<B: ChatBackend> DeepReasoning<B> {
    /// 用同一后端（如单模型）构造：solver=critic=同一模型。
    pub fn single(backend: B) -> Self
    where
        B: Clone,
    {
        Self {
            solver: backend.clone(),
            critic: backend,
        }
    }

    /// 用双后端构造（Captain + Copilot 分离，双模并行增强）。
    pub fn dual(solver: B, critic: B) -> Self {
        Self { solver, critic }
    }

    fn sys(role: &str) -> Message {
        Message::System {
            content: role.to_string(),
            timestamp: Utc::now(),
        }
    }

    fn user(content: &str) -> Message {
        Message::User {
            content: content.to_string(),
            timestamp: Utc::now(),
        }
    }

    /// 运行四阶段管线。
    pub async fn run(&self, prompt: &str) -> Result<DeepResult> {
        // Phase 1 · Solve
        let draft = self
            .solver
            .chat(&[
                Self::sys("你是解决者。给出高质量、可直接使用的完整答案。"),
                Self::user(prompt),
            ])
            .await?
            .content;

        // Phase 2 · Critique
        let critique = self
            .critic
            .chat(&[
                Self::sys("你是严格批判者。找出答案的疏漏、反例、边界缺陷与可改进点。若答案已足够好，明确说『无改进点』。不要重写答案，只列问题。"),
                Self::user(&format!("问题：{prompt}\n答案：{draft}")),
            ])
            .await?
            .content;

        // Phase 3 · Rewrite（批判无实质改进则跳过重写，防退化）
        let rewritten = if Self::is_noop_critique(&critique) {
            draft.clone()
        } else {
            self.solver
                .chat(&[
                    Self::sys("你是解决者。根据批判意见重写答案，吸收其中有效改进，保持其余部分一致。只输出重写后的完整答案。没有要改的就原样返回。"),
                    Self::user(&format!("问题：{prompt}\n原答案：{draft}\n批判意见：{critique}")),
                ])
                .await?
                .content
        };

        // Phase 4 · Select（以原稿为基准，只接受消除缺陷的改进）
        let final_answer = self
            .critic
            .chat(&[
                Self::sys("你是在『原答案』与『重写稿』之间做选择。若重写稿相对于原答案有实质改进（修复了缺陷/更完整/更准确），返回重写稿；否则返回原答案。只输出最终选择的内容，不要解释。"),
                Self::user(&format!("原答案：\n{draft}\n\n重写稿：\n{rewritten}")),
            ])
            .await?
            .content;

        Ok(DeepResult {
            final_answer,
            draft,
            critique,
            rewritten,
        })
    }

    /// 判定批判意见是否为空/无实质改进（启发式，保守）。
    fn is_noop_critique(critique: &str) -> bool {
        let c = critique.trim();
        c.is_empty()
            || c.contains("无改进点")
            || c.contains("没有需要改进")
            || c.contains("已足够好")
            || c.contains("无问题")
    }
}

/// 便捷函数：用主/副后端跑深度推理并返回最终答案。
pub async fn run_deep<B: ChatBackend>(solver: B, critic: B, prompt: &str) -> Result<DeepResult> {
    DeepReasoning::dual(solver, critic).run(prompt).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::llm::LLMResponse;
    use anyhow::anyhow;

    /// 可编程假后端：按 sender 返回固定回复；fail 时返回错误。
    #[derive(Clone)]
    struct Fake {
        reply: String,
        fail: Option<String>,
    }

    impl Fake {
        fn ok(reply: &str) -> Self {
            Self {
                reply: reply.to_string(),
                fail: None,
            }
        }
    }

    impl ChatBackend for Fake {
        async fn chat(&self, _m: &[Message]) -> Result<LLMResponse> {
            if let Some(e) = &self.fail {
                return Err(anyhow!("{e}"));
            }
            Ok(LLMResponse {
                content: self.reply.clone(),
                thinking: None,
                model: "fake".into(),
                usage: crate::model::llm::TokenUsage::default(),
                tool_calls: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn four_stage_pipeline_merges_into_final() {
        let dr = DeepReasoning::dual(Fake::ok("初稿答案"), Fake::ok("发现边界缺陷"));
        let r = dr.run("写一个函数").await.unwrap();
        assert_eq!(r.draft, "初稿答案");
        assert_eq!(r.critique, "发现边界缺陷");
        assert_eq!(
            r.rewritten, "初稿答案",
            "批判非 noop 时触发重写（Fake solver 恒返初稿）"
        );
        assert_eq!(
            r.final_answer, "发现边界缺陷",
            "Select 由 critic 产出最终答案"
        );
    }

    #[tokio::test]
    async fn noop_critique_skips_rewrite() {
        // 批判『无改进点』→ Rewrite 阶段跳过，rewritten 直接等于 draft，
        // 避免无谓重写退化。
        let dr = DeepReasoning::dual(Fake::ok("好答案"), Fake::ok("无改进点"));
        let r = dr.run("问题").await.unwrap();
        assert_eq!(r.draft, r.rewritten, "无改进点时应跳过重写");
    }

    #[tokio::test]
    async fn single_backend_walks_full_pipeline() {
        let dr = DeepReasoning::single(Fake::ok("同一模型输出"));
        let r = dr.run("问题").await.unwrap();
        assert!(!r.final_answer.is_empty());
        assert_eq!(r.draft, "同一模型输出");
    }

    #[tokio::test]
    async fn solver_failure_propagates() {
        let dr = DeepReasoning::dual(
            Fake {
                reply: String::new(),
                fail: Some("solver down".into()),
            },
            Fake::ok("critic"),
        );
        let err = dr.run("问题").await;
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("solver down"));
    }

    #[test]
    fn noop_critique_detection() {
        assert!(DeepReasoning::<Fake>::is_noop_critique(
            "无改进点，答案已足够好"
        ));
        assert!(DeepReasoning::<Fake>::is_noop_critique("  "));
        assert!(DeepReasoning::<Fake>::is_noop_critique(
            "没有需要改进的地方"
        ));
        assert!(!DeepReasoning::<Fake>::is_noop_critique(
            "第 3 步逻辑有漏洞，建议补充边界"
        ));
    }
}
