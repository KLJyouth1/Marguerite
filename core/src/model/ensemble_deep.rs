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
//!
//! # 深度可配置（模式规格接线）
//!
//! 历史缺陷：管线曾把「1 轮批判 + 1 轮重写 + 1 遍择优」写死，`ModeSpec`/`BloomSpec` 里声明的
//! `thinking_depth`（bloom=3 / omni_transcend=4）与 `verification_passes`（3 / 4）从无执行层
//! 消费点，只在 `mode.spec` RPC 展示——即「配置了深度但不生效」。
//!
//! 现由 [`DeepPlan`] 把模式声明翻译成实际轮数（见 [`DeepPlan::from_mode_spec`]），
//! 使 OmniTranscend 的 S3 深想段真正比 Bloom 多跑打磨与验证。

use anyhow::Result;
use chrono::Utc;

use crate::model::ensemble::ChatBackend;
use crate::types::Message;

/// 打磨轮数硬上限（防止模式 YAML 配置失手导致 token / 时延暴涨）
pub const MAX_REFINE_ROUNDS: usize = 4;
/// 验证遍数硬上限
pub const MAX_VERIFY_ROUNDS: usize = 3;

/// 深度推理轮数计划：把模式规格的 `thinking_depth` / `verification_passes`
/// 翻译成管线实际执行的「Critique→Rewrite 打磨轮数」与「验证遍数」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeepPlan {
    /// Critique→Rewrite 打磨轮数（≥1）
    pub refine_rounds: usize,
    /// 验证遍数（≥1；第 1 遍即 Select 择优，其后为独立复核）
    pub verify_rounds: usize,
}

impl Default for DeepPlan {
    /// 默认计划 = 历史行为（1 轮打磨 + 1 遍择优），保证既有调用方零行为变化。
    fn default() -> Self {
        Self {
            refine_rounds: 1,
            verify_rounds: 1,
        }
    }
}

impl DeepPlan {
    /// 从模式规格派生轮数计划。
    ///
    /// 映射口径（`-2` 的由来，写明以免看起来像魔法数）：
    /// `thinking_depth` 计的是**推理阶段总数**，其中首阶段 Solve 与末阶段 Select 固定各占 1，
    /// 余下的才是可变打磨轮；`verification_passes` 同理——第 1 遍由 Critique 内建承担、
    /// 第 2 遍由 Select 择优承担，余下的才是独立复核遍数。故：
    ///
    /// | 模式 | thinking_depth | refine_rounds | verification_passes | verify_rounds | LLM 调用数 |
    /// |------|---------------|---------------|---------------------|---------------|-----------|
    /// | Bloom / Ensemble | 3 | 1 | 3 | 1 | 4（与历史一致） |
    /// | OmniTranscend    | 4 | 2 | 4 | 2 | 7 |
    ///
    /// 这一口径刻意让 Bloom 的线上成本与行为**逐字节不变**，只让 OmniTranscend 真正加深，
    /// 避免一次改动同时改变既有模式的成本画像。两端都被 [`MAX_REFINE_ROUNDS`] /
    /// [`MAX_VERIFY_ROUNDS`] 钳制，且下限恒为 1（永不退化成「不打磨/不验证」）。
    pub fn from_mode_spec(thinking_depth: usize, verification_passes: usize) -> Self {
        Self {
            refine_rounds: thinking_depth.saturating_sub(2).clamp(1, MAX_REFINE_ROUNDS),
            verify_rounds: verification_passes
                .saturating_sub(2)
                .clamp(1, MAX_VERIFY_ROUNDS),
        }
    }

    /// 该计划在「批判均有实质改进」的最坏情况下的 LLM 调用次数上界。
    /// 用于成本预估与单测断言（Solve 1 次 + 每轮打磨 2 次 + 每遍验证 1 次）。
    pub fn max_llm_calls(&self) -> usize {
        1 + self.refine_rounds * 2 + self.verify_rounds
    }
}

/// 四阶段深度推理编排器（泛型后端，零成本抽象；用 `ChatBackend` 便于注入 FakeBackend 单测）
pub struct DeepReasoning<B: ChatBackend> {
    /// 解决者/重写者（可用主模型 Captain）
    solver: B,
    /// 批判者/选择者（可用副模型 Critic）
    critic: B,
    /// 轮数计划（默认 = 历史行为）
    plan: DeepPlan,
}

/// 深度推理结果
#[derive(Debug)]
pub struct DeepResult {
    /// 最终答案
    pub final_answer: String,
    /// 初稿（Solve 阶段的原始输出）
    pub draft: String,
    /// 批判意见（多轮时为最后一轮批判，保持与单轮时期的语义一致）
    pub critique: String,
    /// 逐轮批判意见全量留痕（长度 = 实际执行的批判次数）
    pub critiques: Vec<String>,
    /// 重写稿（末轮 Rewrite 输出，可能被 Select 舍弃）
    pub rewritten: String,
    /// 实际执行的打磨轮数（批判判定为「无改进点」时提前收敛，可小于计划值）
    pub refine_rounds_run: usize,
    /// 实际执行的验证遍数
    pub verify_rounds_run: usize,
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
            plan: DeepPlan::default(),
        }
    }

    /// 用双后端构造（Captain + Copilot 分离，双模并行增强）。
    pub fn dual(solver: B, critic: B) -> Self {
        Self {
            solver,
            critic,
            plan: DeepPlan::default(),
        }
    }

    /// 覆盖轮数计划（模式规格接线入口）。
    pub fn with_plan(mut self, plan: DeepPlan) -> Self {
        self.plan = plan;
        self
    }

    /// 当前生效的轮数计划。
    pub fn plan(&self) -> DeepPlan {
        self.plan
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

    /// 运行深度推理管线：Solve → N×(Critique→Rewrite) → M×Verify。
    ///
    /// N/M 由 [`DeepPlan`] 决定；默认计划 (1,1) 即历史的四阶段行为。
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

        // Phase 2-3 · Critique→Rewrite 打磨循环（轮数 = plan.refine_rounds）
        //
        // 每轮都基于「上一轮的最新稿」批判，逐轮收敛；任一轮批判判定为「无改进点」
        // 立即收敛退出（沿用防退化语义，不做无谓重写）。
        let mut current = draft.clone();
        let mut critiques: Vec<String> = Vec::new();
        let mut refine_rounds_run = 0usize;
        for round in 0..self.plan.refine_rounds.max(1) {
            let critique = self
                .critic
                .chat(&[
                    Self::sys("你是严格批判者。找出答案的疏漏、反例、边界缺陷与可改进点。若答案已足够好，明确说『无改进点』。不要重写答案，只列问题。"),
                    Self::user(&format!(
                        "问题：{prompt}\n答案：{current}\n（这是第 {} 轮审查，共 {} 轮；请只提出前几轮尚未覆盖的新问题）",
                        round + 1,
                        self.plan.refine_rounds.max(1)
                    )),
                ])
                .await?
                .content;
            critiques.push(critique.clone());
            if Self::is_noop_critique(&critique) {
                break;
            }
            let rewritten = self
                .solver
                .chat(&[
                    Self::sys("你是解决者。根据批判意见重写答案，吸收其中有效改进，保持其余部分一致。只输出重写后的完整答案。没有要改的就原样返回。"),
                    Self::user(&format!(
                        "问题：{prompt}\n原答案：{current}\n批判意见：{critique}"
                    )),
                ])
                .await?
                .content;
            // 空输出不覆盖（防退化：宁可保留上一稿，也不让空回答冒充改进）
            if !rewritten.trim().is_empty() {
                current = rewritten;
            }
            refine_rounds_run += 1;
        }
        let rewritten = current.clone();
        let critique = critiques.last().cloned().unwrap_or_default();

        // Phase 4 · Verify 循环（第 1 遍 = Select 择优；其后为独立复核遍）
        let mut final_answer = self
            .critic
            .chat(&[
                Self::sys("你是在『原答案』与『重写稿』之间做选择。若重写稿相对于原答案有实质改进（修复了缺陷/更完整/更准确），返回重写稿；否则返回原答案。只输出最终选择的内容，不要解释。"),
                Self::user(&format!("原答案：\n{draft}\n\n重写稿：\n{rewritten}")),
            ])
            .await?
            .content;
        let mut verify_rounds_run = 1usize;
        for pass in 1..self.plan.verify_rounds.max(1) {
            let verified = self
                .critic
                .chat(&[
                    Self::sys("你是最终验证者。逐条核对答案是否完整、正确、可直接使用；发现问题就直接修正后输出完整答案；没有问题就原样输出答案。只输出答案本身，不要解释、不要评语。"),
                    Self::user(&format!(
                        "问题：{prompt}\n\n第 {} 遍验证前的答案：\n{final_answer}",
                        pass + 1
                    )),
                ])
                .await?
                .content;
            // 空输出不覆盖（防退化：验证遍不得把已有答案清空）
            if verified.trim().is_empty() {
                break;
            }
            final_answer = verified;
            verify_rounds_run += 1;
        }

        Ok(DeepResult {
            final_answer,
            draft,
            critique,
            critiques,
            rewritten,
            refine_rounds_run,
            verify_rounds_run,
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

/// 便捷函数：用主/副后端跑深度推理并返回最终答案（默认计划 = 历史四阶段行为）。
pub async fn run_deep<B: ChatBackend>(solver: B, critic: B, prompt: &str) -> Result<DeepResult> {
    DeepReasoning::dual(solver, critic).run(prompt).await
}

/// 便捷函数：按指定轮数计划跑深度推理（模式规格接线用）。
pub async fn run_deep_with_plan<B: ChatBackend>(
    solver: B,
    critic: B,
    plan: DeepPlan,
    prompt: &str,
) -> Result<DeepResult> {
    DeepReasoning::dual(solver, critic)
        .with_plan(plan)
        .run(prompt)
        .await
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

    // ===== 模式规格接线守卫：证明「配置的深度真正改变执行行为」 =====

    /// 计数后端：记录被调用次数，用于证明轮数计划确实改变了 LLM 调用次数。
    #[derive(Clone)]
    struct Counting {
        reply: String,
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Counting {
        fn new(reply: &str) -> Self {
            Self {
                reply: reply.to_string(),
                calls: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }
        }
        fn count(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    impl ChatBackend for Counting {
        async fn chat(&self, _m: &[Message]) -> Result<LLMResponse> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(LLMResponse {
                content: self.reply.clone(),
                thinking: None,
                model: "counting".into(),
                usage: crate::model::llm::TokenUsage::default(),
                tool_calls: Vec::new(),
            })
        }
    }

    #[test]
    fn deep_plan_maps_mode_spec_to_rounds() {
        // Bloom / Ensemble（3,3）→ 与历史行为逐字节一致（1 轮打磨 + 1 遍验证 = 4 次调用）
        let bloom = DeepPlan::from_mode_spec(3, 3);
        assert_eq!(bloom, DeepPlan::default(), "Bloom 深度必须等于历史默认计划");
        assert_eq!(bloom.max_llm_calls(), 4);

        // OmniTranscend（4,4）→ 真正加深：2 轮打磨 + 2 遍验证 = 7 次调用
        let omni = DeepPlan::from_mode_spec(4, 4);
        assert_eq!(omni.refine_rounds, 2, "S3 深想应多一轮打磨");
        assert_eq!(omni.verify_rounds, 2, "S3 深想应多一遍验证");
        assert_eq!(omni.max_llm_calls(), 7);
        assert!(
            omni.max_llm_calls() > bloom.max_llm_calls(),
            "深度更高的模式必须真的做更多推理，而不是只改展示字段"
        );

        // 下限保护：配置写 0/1/2 也不退化成「不打磨/不验证」
        for d in [0usize, 1, 2] {
            let p = DeepPlan::from_mode_spec(d, d);
            assert_eq!(
                (p.refine_rounds, p.verify_rounds),
                (1, 1),
                "深度 {d} 应钳到下限 1"
            );
        }

        // 上限保护：配置失手写超大值时被钳制，防 token 暴涨
        let huge = DeepPlan::from_mode_spec(999, 999);
        assert_eq!(huge.refine_rounds, MAX_REFINE_ROUNDS);
        assert_eq!(huge.verify_rounds, MAX_VERIFY_ROUNDS);
    }

    #[tokio::test]
    async fn deeper_plan_runs_strictly_more_llm_calls() {
        // 批判恒为「有实质改进」，确保不提前收敛，从而能对比真实调用次数。
        let solver_shallow = Counting::new("稿件");
        let critic_shallow = Counting::new("第 3 步有边界漏洞");
        let shallow = DeepReasoning::dual(solver_shallow.clone(), critic_shallow.clone())
            .with_plan(DeepPlan::from_mode_spec(3, 3));
        let r_shallow = shallow.run("题目").await.unwrap();
        let calls_shallow = solver_shallow.count() + critic_shallow.count();

        let solver_deep = Counting::new("稿件");
        let critic_deep = Counting::new("第 3 步有边界漏洞");
        let deep = DeepReasoning::dual(solver_deep.clone(), critic_deep.clone())
            .with_plan(DeepPlan::from_mode_spec(4, 4));
        let r_deep = deep.run("题目").await.unwrap();
        let calls_deep = solver_deep.count() + critic_deep.count();

        assert_eq!(calls_shallow, 4, "Bloom 档应保持历史的 4 次调用");
        assert_eq!(calls_deep, 7, "OmniTranscend 档应跑满 1+2*2+2=7 次调用");
        assert!(
            calls_deep > calls_shallow,
            "thinking_depth/verification_passes 必须真正驱动执行，实测 deep={calls_deep} shallow={calls_shallow}"
        );

        assert_eq!(r_shallow.refine_rounds_run, 1);
        assert_eq!(r_shallow.verify_rounds_run, 1);
        assert_eq!(r_deep.refine_rounds_run, 2);
        assert_eq!(r_deep.verify_rounds_run, 2);
        assert_eq!(r_deep.critiques.len(), 2, "每轮批判都应留痕");
    }

    #[tokio::test]
    async fn noop_critique_converges_early_even_at_max_depth() {
        // 深度拉满但首轮即「无改进点」→ 立即收敛，不浪费 token。
        let solver = Counting::new("好答案");
        let critic = Counting::new("无改进点");
        let dr = DeepReasoning::dual(solver.clone(), critic.clone())
            .with_plan(DeepPlan::from_mode_spec(999, 3));
        let r = dr.run("题目").await.unwrap();
        assert_eq!(r.refine_rounds_run, 0, "首轮判无改进点应零打磨");
        assert_eq!(r.draft, r.rewritten);
        // Solve 1 + Critique 1 + Select 1 = 3；打磨与多余批判都被跳过
        assert_eq!(solver.count() + critic.count(), 3);
    }

    #[tokio::test]
    async fn default_plan_preserves_legacy_four_stage_call_count() {
        // 不显式指定计划时，行为与历史四阶段完全一致（回归护栏）。
        let solver = Counting::new("初稿");
        let critic = Counting::new("有缺陷");
        let dr = DeepReasoning::dual(solver.clone(), critic.clone());
        assert_eq!(dr.plan(), DeepPlan::default());
        dr.run("题目").await.unwrap();
        assert_eq!(solver.count() + critic.count(), 4);
    }
}
