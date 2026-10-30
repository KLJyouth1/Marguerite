//! 专家团队（Agent Team）编排原语
//!
//! 把「一组角色专精的子 Agent 协同完成一个任务」抽象为可复用的 [`Team`]，
//! 底层复用 [`crate::agent::subagent::SubAgentManager`] 的真实 spawn 模型
//! （每个成员经 `spawn_with_spec` 真实执行，含角色人设 + 模型路由 + 工具链）。
//!
//! 本模块与 [`crate::orchestration`] 的自动决策正交：只负责「给我一组成员，
//! 我帮你并行 spawn、收集、聚合」，不决定用哪种编排形态。既可作为编排器
//! `ExpertTeam` / `Cluster` 分支的底层执行体，也可被 CLI / ACP / 上层团队
//! 调度直接复用——这是「自定义专家团队 / agent team」从叙事走向真实的关键一环。

use crate::agent::prefix_scope::SharedPrefixCache;
use crate::agent::subagent::{SubAgentManager, SubAgentStatus};
use crate::cache::PrefixCache;
use crate::e::nlp::{cosine, jaccard, tokenize_zh};
use crate::model::prefetch::PrefetchDedup;
use crate::model::router::{Complexity, IntentType};
use crate::types::Message;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;

/// 团队成员：一个角色专精的子 Agent 规格。
#[derive(Debug, Clone)]
pub struct TeamMember {
    /// 角色名（用于聚合标签与事件）
    pub role: String,
    /// 该成员的系统人设提示
    pub system_prompt: String,
    /// 具体子任务描述
    pub task: String,
    /// 模型路由意图
    pub intent: IntentType,
    /// 模型路由复杂度
    pub complexity: Complexity,
    /// 依赖的其他成员角色（P2P 唤醒调度：须在其全部完成后本成员才被唤醒）
    pub deps: Vec<String>,
}

/// P2P 成员间直连消息。
#[derive(Debug, Clone)]
pub struct TeamMessage {
    /// 发送者角色
    pub from: String,
    /// 接收者角色（"*" 表示广播给所有依赖者）
    pub to: String,
    /// 消息体（通常是该成员的产出摘要）
    pub body: String,
    /// 消息时间戳（RFC3339）
    pub timestamp: String,
}

impl TeamMessage {
    pub fn new(from: impl Into<String>, to: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            body: body.into(),
            timestamp: Utc::now().to_rfc3339(),
        }
    }
}

/// 团队共享信箱：成员完成后投递消息，被唤醒的依赖者从中抽取发给自己的消息。
///
/// 增强自 dsh-agent-teams 的邮箱直传模式：
/// - 支持定向消息（`send_to`）与广播（`broadcast`）
/// - 支持非破坏性读取（`peek_for`）与抽取清空（`drain_for`）
/// - 线程安全：可被多成员并发访问（`Arc<Mutex<Mailbox>>`）
#[derive(Debug, Default)]
pub struct Mailbox {
    inbox: Vec<TeamMessage>,
}

impl Mailbox {
    /// 空信箱。
    pub fn new() -> Self {
        Self { inbox: Vec::new() }
    }

    /// 投递一条消息。
    pub fn send(&mut self, msg: TeamMessage) {
        self.inbox.push(msg);
    }

    /// 定向发送消息给指定角色。
    pub fn send_to(&mut self, from: &str, to: &str, body: &str) {
        self.send(TeamMessage::new(from, to, body));
    }

    /// 抽取并清空「发给 `role`」或广播（to == "*"）的消息。
    pub fn drain_for(&mut self, role: &str) -> Vec<TeamMessage> {
        let mut out = Vec::new();
        let mut keep = Vec::new();
        for msg in self.inbox.drain(..) {
            if msg.to == role || msg.to == "*" {
                out.push(msg);
            } else {
                keep.push(msg);
            }
        }
        self.inbox = keep;
        out
    }

    /// 非破坏性读取「发给 `role`」或广播的消息（不移除）。
    pub fn peek_for(&self, role: &str) -> Vec<&TeamMessage> {
        self.inbox
            .iter()
            .filter(|msg| msg.to == role || msg.to == "*")
            .collect()
    }

    /// 广播一条消息给所有成员。
    pub fn broadcast(&mut self, from: &str, body: &str) {
        self.send(TeamMessage::new(from, "*", body));
    }

    /// 信箱内消息数（可观测）。
    pub fn len(&self) -> usize {
        self.inbox.len()
    }

    /// 信箱是否为空。
    pub fn is_empty(&self) -> bool {
        self.inbox.is_empty()
    }
}

impl TeamMember {
    pub fn new(
        role: &str,
        system_prompt: &str,
        task: &str,
        intent: IntentType,
        complexity: Complexity,
    ) -> Self {
        Self {
            role: role.to_string(),
            system_prompt: system_prompt.to_string(),
            task: task.to_string(),
            intent,
            complexity,
            deps: Vec::new(),
        }
    }

    /// 声明本成员依赖的其他角色（须在其完成后才被唤醒）。
    pub fn depends_on(mut self, role: &str) -> Self {
        self.deps.push(role.to_string());
        self
    }
}

/// 团队编排结果。
#[derive(Debug, Clone, Default)]
pub struct TeamOutcome {
    /// 各成员产出（`角色, 结论`），按 spawn 顺序。失败成员的「结论」也是失败原因（不丢弃）。
    pub member_results: Vec<(String, String)>,
    /// 聚合后的综合上下文（各成员产出按 `[角色] 结论` 拼接），供 lead 综合。
    pub aggregated: String,
}

impl TeamOutcome {
    /// 是否没有任何成员（空团队）。
    pub fn is_empty(&self) -> bool {
        self.member_results.is_empty()
    }
}

/// 团队默认共享前缀缓存容量（FIFO 上限：超出仅淘汰最旧前缀、不影响复用正确性）。
const DEFAULT_TEAM_PREFIX_CAP: usize = 16;

/// 一个专家团队：角色专精成员集合，可并行 spawn 并聚合。
#[derive(Debug, Clone)]
pub struct Team {
    pub members: Vec<TeamMember>,
    /// 共享稳定前缀缓存作用域：默认构造即注入（W-2），团队内所有成员复用同一
    /// `PrefixCache`；每次成员 spawn 前观测其 system 前缀，同系统前缀的
    /// 多次/多成员请求即被服务端 KV 块复用（`prefix_hits` 上升，KV-Cache 原则 P1）。
    /// 跨团队共享需显式 `with_prefix_cache` 注入同一 `Arc<PrefixCache>`。
    pub prefix_cache: Option<SharedPrefixCache>,
    /// 团队级去重预填充（W-9）：可选注入。注入后，成员 spawn 前对相同稳定前缀
    /// 只 `warmup_once` 一次（窗口内去重），让人工/批处理下的同前缀多成员请求
    /// 复用服务端已填充的 KV 块，省去重复预填充的 prompt 与无效输出 token。
    /// 默认未注入（保持向后兼容，不改变 `run` 的网络行为）。
    pub prefetch: Option<Arc<PrefetchDedup>>,
    /// 会话标识（可选，v10）：注入后预填充去重按会话生命周期（无 TTL）——
    /// 同会话同前缀恒去重、跨会话互不影响（对齐验收报告 §4 剩余边界）。
    pub session: Option<String>,
}

impl Default for Team {
    fn default() -> Self {
        Self {
            members: Vec::new(),
            // W-2：默认即注入团队内共享前缀缓存，使同 system 前缀的成员自动复用 KV 块。
            prefix_cache: Some(SharedPrefixCache::new(Arc::new(PrefixCache::new(
                DEFAULT_TEAM_PREFIX_CAP,
            )))),
            prefetch: None,
            session: None,
        }
    }
}

impl Team {
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一个成员（builder 风格）。
    pub fn member(
        mut self,
        role: &str,
        system_prompt: &str,
        task: &str,
        intent: IntentType,
        complexity: Complexity,
    ) -> Self {
        self.members.push(TeamMember::new(
            role,
            system_prompt,
            task,
            intent,
            complexity,
        ));
        self
    }

    /// 追加一个带依赖声明的成员（P2P 唤醒调度）：`deps` 为须先于本成员完成的角色名。
    pub fn member_dep(
        mut self,
        role: &str,
        system_prompt: &str,
        task: &str,
        intent: IntentType,
        complexity: Complexity,
        deps: &[&str],
    ) -> Self {
        let mut m = TeamMember::new(role, system_prompt, task, intent, complexity);
        for d in deps {
            m = m.depends_on(d);
        }
        self.members.push(m);
        self
    }

    /// 是否为空团队（无成员）。
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// 注入一个共享稳定前缀缓存作用域（团队内所有成员复用同一 `PrefixCache`）。
    pub fn with_prefix_cache(mut self, cache: Arc<PrefixCache>) -> Self {
        self.prefix_cache = Some(SharedPrefixCache::new(cache));
        self
    }

    /// 注入团队级去重预填充（W-9）：成员 spawn 前对相同稳定前缀只预热一次。
    pub fn with_prefetch(mut self, prefetch: Arc<PrefetchDedup>) -> Self {
        self.prefetch = Some(prefetch);
        self
    }

    /// 注入会话标识（v10）：预填充去重按会话生命周期（无 TTL，跨会话互不影响）。
    pub fn with_session(mut self, session: impl Into<String>) -> Self {
        self.session = Some(session.into());
        self
    }

    /// 并行 spawn 所有成员并经子 Agent 管理器真实执行。
    ///
    /// - spawn：每个成员经 [`SubAgentManager::spawn_with_spec`] 真实执行
    ///   （角色人设 + 模型路由 + 工具链），与编排器 `ExpertTeam`/`Cluster` 分支同源。
    /// - 收集：等待全部完成后逐成员取结论；spawn 失败或子 Agent 无结果
    ///   均**如实**进入 `member_results`，不静默跳过、不冒充成功。
    pub async fn run(&self, subagents: &SubAgentManager, context: &str) -> TeamOutcome {
        // 并发闸门按本团队成员数抬升：默认闸门（5）会让 8 人团队只同时跑 5 人，
        // 看板上「选了 8 路只看到 5 个成员」。用户口径是不对成员数设上限。
        subagents.ensure_concurrency(self.members.len());
        // 单波并行扇出：一次性 spawn 全部成员（无依赖图调度）。
        // 迭代身份统一用「成员索引」（与 run_p2p 一致风格）：单波扇出下所有成员同批唤醒，
        // 索引键无副作用，仅作一致性表达，并为未来可能的依赖调度预留同角色隔离基础。
        let mut spawned: Vec<(usize, Result<String, String>)> =
            Vec::with_capacity(self.members.len());
        for (i, m) in self.members.iter().enumerate() {
            // 共享前缀观测：成员 system 前缀相同 → 后续请求自动复用 KV 块（前缀命中率可观测）。
            if let Some(scope) = &self.prefix_cache {
                let sys_msg = Message::System {
                    content: m.system_prompt.clone(),
                    timestamp: Utc::now(),
                };
                scope.observe(&[sys_msg], 1.0);
            }
            // 团队级去重预填充（W-9 / v10）：相同稳定前缀只 warmup_once 一次；
            // 注入会话标识时按会话生命周期去重（无 TTL，跨会话互不影响）。
            if let Some(prefetch) = &self.prefetch {
                let sys_msg = Message::System {
                    content: m.system_prompt.clone(),
                    timestamp: Utc::now(),
                };
                match &self.session {
                    Some(sid) => {
                        let _ = prefetch.warmup_once_for_session(sid, &[sys_msg]).await;
                    }
                    None => {
                        let _ = prefetch.warmup_once(&[sys_msg]).await;
                    }
                }
            }
            let r = subagents
                .spawn_with_spec(
                    &m.task,
                    context,
                    &m.system_prompt,
                    m.intent.clone(),
                    m.complexity.clone(),
                )
                .await;
            spawned.push((i, r.map_err(|e| e.to_string())));
        }
        // 2) 等待全部完成
        let _ = subagents.wait_all().await;
        // 3) 收集每个成员结论（成功结论或失败原因）
        let mut member_results: Vec<(String, String)> = Vec::with_capacity(spawned.len());
        for (i, r) in &spawned {
            let role = self.members[*i].role.clone();
            let result = match r {
                Ok(id) => match subagents.get_result(id).await {
                    Some(res) => res,
                    // 结果缺失：用真实状态回填，避免误导性的「无结果返回」。
                    None => match subagents.get_status(id).await {
                        Some(SubAgentStatus::Failed { error }) => {
                            format!("[子专家执行失败] {error}")
                        }
                        Some(SubAgentStatus::TimedOut) => "[子专家执行超时]".to_string(),
                        Some(SubAgentStatus::Pending) | Some(SubAgentStatus::Running) => {
                            "[子专家未完成]".to_string()
                        }
                        _ => "[子专家无结果返回]".to_string(),
                    },
                },
                Err(e) => format!("[成员 spawn 失败: {e}]"),
            };
            member_results.push((role, result));
        }
        // 4) 聚合（按角色拼接，供 lead 综合）
        let aggregated = member_results
            .iter()
            .map(|(role, res)| format!("[{role}] {res}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        TeamOutcome {
            member_results,
            aggregated,
        }
    }

    /// P2P 团队执行：成员直连信箱 + 依赖图唤醒调度。
    ///
    /// 与 [`Team::run`]（主管一次性并行扇出）不同，`run_p2p`：
    /// - 按 `TeamMember.deps` 构建依赖图，分波（wave）调度——仅当某成员的全部依赖
    ///   已完成才被唤醒 spawn；
    /// - 成员完成后向其依赖者投递 `TeamMessage` 进入共享 `Mailbox`，被唤醒的成员
    ///   在 spawn 前把信箱中「发给自己的消息」注入上下文（P2P 直连，而非仅经主管聚合）；
    /// - 无依赖声明的成员（含既有 `run` 测试）在第一波全部并行，行为与 `run` 一致。
    pub async fn run_p2p(&self, subagents: &SubAgentManager, context: &str) -> TeamOutcome {
        // 同 `run`：并发闸门按成员数抬升，避免「大团队被默认 5 并发卡住」。
        subagents.ensure_concurrency(self.members.len());
        let mut mailbox = Mailbox::new();
        // 调度身份用「成员索引」而非「角色名」：避免两个同角色但依赖集不同的成员，
        // 因 done 以角色为键而在 retain 阶段被整批误删（静默丢弃未 spawn 的成员）。
        let mut done: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut remaining: Vec<usize> = (0..self.members.len()).collect();
        let mut member_results: Vec<(String, String)> = Vec::with_capacity(self.members.len());

        loop {
            // 本波：依赖（按角色名声明）已全部完成的成员。
            // 依赖「角色 d 已达成」= 存在某个已完成的成员其角色为 d。
            let ready: Vec<usize> = remaining
                .iter()
                .copied()
                .filter(|&i| {
                    self.members[i].deps.iter().all(|d| {
                        self.members
                            .iter()
                            .enumerate()
                            .any(|(j, m)| done.contains(&j) && &m.role == d)
                    })
                })
                .collect();
            if ready.is_empty() {
                // 死锁 / 环依赖：剩余成员的前置永远无法达成。如实收集（不丢弃），
                // 标记环依赖，保持与 `run` 一致的「失败也进 member_results」不变量，
                // 绝不静默丢失成员（此前此处直接 break 会吞掉环中成员）。
                for &i in &remaining {
                    member_results.push((
                        self.members[i].role.clone(),
                        format!(
                            "[依赖环：成员 {} 的前置未达成，无法调度]",
                            self.members[i].role
                        ),
                    ));
                }
                break;
            }

            let mut wave = Vec::with_capacity(ready.len());
            for i in ready {
                let m = &self.members[i];
                // P2P：从信箱抽取发给本成员的消息（非破坏性 peek，保留给后续同角色成员）
                let msgs: Vec<String> = mailbox
                    .peek_for(&m.role)
                    .iter()
                    .map(|msg| msg.body.clone())
                    .collect();
                let member_context = if msgs.is_empty() {
                    context.to_string()
                } else {
                    format!("{context}\n\n[来自队友的消息]\n{}", msgs.join("\n"))
                };
                let r = subagents
                    .spawn_with_spec(
                        &m.task,
                        &member_context,
                        &m.system_prompt,
                        m.intent.clone(),
                        m.complexity.clone(),
                    )
                    .await;
                wave.push((i, r));
            }
            // 与本模块 `run` 一致：等待本波全部完成再取结论（真实等待语义）
            let _ = subagents.wait_all().await;
            for (i, r) in wave {
                let m = &self.members[i];
                let result = match r {
                    Ok(id) => match subagents.get_result(&id).await {
                        Some(res) => res,
                        None => match subagents.get_status(&id).await {
                            Some(SubAgentStatus::Failed { error }) => {
                                format!("[子专家执行失败] {error}")
                            }
                            Some(SubAgentStatus::TimedOut) => "[子专家执行超时]".to_string(),
                            Some(SubAgentStatus::Pending) | Some(SubAgentStatus::Running) => {
                                "[子专家未完成]".to_string()
                            }
                            _ => "[子专家无结果返回]".to_string(),
                        },
                    },
                    Err(e) => format!("[成员 spawn 失败: {e}]"),
                };
                // 广播本成员产出到信箱，供依赖者消费（P2P 直连）
                mailbox.broadcast(&m.role, &format!("[{}] {}", m.role, result));
                // 标记「本成员索引」完成，而非角色——同角色的其他成员不受影响。
                done.insert(i);
                member_results.push((m.role.clone(), result));
            }
            // 仅移除本波已真实 spawn 并完成的成员（按索引），绝不波及未 spawn 的
            // 同角色成员——这是修复「同角色异依赖被静默丢弃」的关键。
            remaining.retain(|&i| !done.contains(&i));
        }

        let aggregated = member_results
            .iter()
            .map(|(role, res)| format!("[{role}] {res}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        TeamOutcome {
            member_results,
            aggregated,
        }
    }
}

// ---------------------------------------------------------------------------
// P3-2 辩论防极化：论点独立性校验 + 按证据链强度收敛 + 保留少数派
// ---------------------------------------------------------------------------
//
// 要防的反模式：团队里多人复述同一条结论，人头越多声音越响，而真正带新证据的
// 少数派被淹掉。判定全部用**确定性文本指纹**（粗分词 + 词频余弦 / 引用 Jaccard /
// 推理链 shingle Jaccard），不引入任何模型或网络依赖：同样输入恒得同样权重与结论。
//
// 规则：
// - 与历史论点的余弦 > 0.85、引用 Jaccard > 0.6、推理链近似 > 0.8 → 判「附和」，
//   权重 ×0.2（同一角色的多条论点按 `Σ 权重 × 证据分` 汇总）；
// - 收敛按证据链强度（`Σ 权重 × 证据分`）而非人头：证据变强的少数派可以赢；
// - 输出保留全部少数派意见与置信度（置信度 = 该角色强度 / 总强度）。

/// 论点与历史论点的余弦阈值（严格大于即判附和）。
pub const SYCOPHANCY_COSINE_THRESHOLD: f64 = 0.85;
/// 引用集合 Jaccard 阈值（严格大于即判附和）。
pub const SYCOPHANCY_CITATION_JACCARD: f64 = 0.6;
/// 推理链近似阈值（3-gram shingle Jaccard，严格大于即判附和）。
pub const SYCOPHANCY_CHAIN_THRESHOLD: f64 = 0.8;
/// 附和的权重惩罚（×0.2）。
pub const SYCOPHANCY_PENALTY: f64 = 0.2;
/// 独立论点的权重。
pub const ARGUMENT_WEIGHT: f64 = 1.0;
/// 收敛所需的最低支持占比（证据链强度占比）。
pub const CONVERGENCE_MIN_SHARE: f64 = 0.5;

/// 附和信号（三种确定性相似度之一命中即判附和）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SycophancySignal {
    /// 与历史论点余弦过高（换皮复述）
    ClaimCosine,
    /// 引用集合 Jaccard 过高（引用同一批来源，无新证据）
    CitationJaccard,
    /// 推理链近似（3-gram shingle 重合）
    ReasoningChain,
}

impl std::fmt::Display for SycophancySignal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            SycophancySignal::ClaimCosine => "论点余弦近似",
            SycophancySignal::CitationJaccard => "引用高度重合",
            SycophancySignal::ReasoningChain => "推理链近似",
        };
        write!(f, "{label}")
    }
}

/// 一条辩论论点：结论 + 证据引用 + 推理链 + 证据分。
#[derive(Debug, Clone, Default)]
pub struct DebateArgument {
    /// 发言人角色
    pub role: String,
    /// 结论（本轮主张）
    pub claim: String,
    /// 证据引用（文档 / 用例 / 数据来源标识），用于算引用 Jaccard
    pub citations: Vec<String>,
    /// 推理链文本，用于算「推理链近似」
    pub reasoning: String,
    /// 证据分（0..=1，由调用方按可验证证据强弱给分；会被夹紧到该区间）
    pub evidence: f64,
}

impl DebateArgument {
    pub fn new(role: impl Into<String>, claim: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            claim: claim.into(),
            citations: Vec::new(),
            reasoning: String::new(),
            evidence: 0.0,
        }
    }

    /// 设定证据分（0..=1，越界自动夹紧）。
    pub fn with_evidence(mut self, evidence: f64) -> Self {
        self.evidence = evidence;
        self
    }

    /// 设定证据引用。
    pub fn with_citations(mut self, citations: &[&str]) -> Self {
        self.citations = citations.iter().map(|c| (*c).to_string()).collect();
        self
    }

    /// 设定推理链文本。
    pub fn with_reasoning(mut self, reasoning: impl Into<String>) -> Self {
        self.reasoning = reasoning.into();
        self
    }
}

/// 单条论点的评分结果（权重已被附和判定修正）。
#[derive(Debug, Clone, PartialEq)]
pub struct ArgumentEvaluation {
    /// 发言人角色
    pub role: String,
    /// 结论
    pub claim: String,
    /// 权重（独立论点 1.0；附和 0.2）
    pub weight: f64,
    /// 证据分
    pub evidence: f64,
    /// 证据链强度 = 权重 × 证据分
    pub score: f64,
    /// 附和判定（`None` = 独立论点）
    pub sycophancy: Option<SycophancySignal>,
    /// 被附和的原角色（命中附和信号时记录）
    pub echo_of: Option<String>,
}

/// 少数派意见（结论与置信度一并保留，不因落败被丢弃）。
#[derive(Debug, Clone, PartialEq)]
pub struct MinorityOpinion {
    /// 少数派角色
    pub role: String,
    /// 该角色的结论（原文保留）
    pub claim: String,
    /// 该角色的证据分
    pub evidence: f64,
    /// 置信度 = 该角色强度 / 总强度（0..=1）
    pub confidence: f64,
    /// 附和判定（`None` = 独立论点）
    pub sycophancy: Option<SycophancySignal>,
}

/// 一轮辩论的收敛结果。
#[derive(Debug, Clone, Default)]
pub struct DebateOutcome {
    /// 每条论点的评分
    pub evaluations: Vec<ArgumentEvaluation>,
    /// 按角色的证据链强度汇总（`角色, Σ 权重×证据分`），按首次出现顺序
    pub support: Vec<(String, f64)>,
    /// 收敛胜出的角色（无有效论点时为 `None`）
    pub winner: Option<String>,
    /// 是否收敛（胜者证据链强度占比 ≥ [`CONVERGENCE_MIN_SHARE`]）
    pub converged: bool,
    /// 少数派意见（含置信度，按支持顺序）
    pub minority: Vec<MinorityOpinion>,
    /// 供 lead 综合的确定性摘要（含少数派）
    pub summary: String,
}

fn term_counts(tokens: &[String]) -> HashMap<String, f64> {
    let mut counts: HashMap<String, f64> = HashMap::new();
    for token in tokens {
        *counts.entry(token.clone()).or_insert(0.0) += 1.0;
    }
    counts
}

/// 论点余弦相似度：粗分词后的词频向量余弦（确定性，无模型/网络）。
fn claim_cosine(a: &str, b: &str) -> f64 {
    let ca = term_counts(&tokenize_zh(a));
    let cb = term_counts(&tokenize_zh(b));
    if ca.is_empty() || cb.is_empty() {
        return 0.0;
    }
    // 词表排序后再取向量：不依赖 HashMap 迭代顺序，保证跨进程可复现。
    let mut vocab: Vec<&String> = ca.keys().chain(cb.keys()).collect();
    vocab.sort_unstable();
    vocab.dedup();
    let va: Vec<f64> = vocab.iter().map(|k| *ca.get(*k).unwrap_or(&0.0)).collect();
    let vb: Vec<f64> = vocab.iter().map(|k| *cb.get(*k).unwrap_or(&0.0)).collect();
    cosine(&va, &vb)
}

/// 引用集合（去空白、统一小写、去重）。
fn citation_set(citations: &[String]) -> Vec<String> {
    let mut out: Vec<String> = citations
        .iter()
        .map(|c| c.trim().to_lowercase())
        .filter(|c| !c.is_empty())
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// 引用集合 Jaccard；任一方无引用即 0.0（无引用不构成「引用重合」证据）。
fn citation_jaccard(a: &[String], b: &[String]) -> f64 {
    let sa = citation_set(a);
    let sb = citation_set(b);
    if sa.is_empty() || sb.is_empty() {
        return 0.0;
    }
    jaccard(&sa, &sb)
}

/// 推理链 3-gram shingle 集合（token 不足 3 时整链作一个 shingle）。
fn chain_shingles(reasoning: &str) -> Vec<String> {
    let tokens = tokenize_zh(reasoning);
    let n = 3.min(tokens.len());
    if n == 0 {
        return Vec::new();
    }
    tokens.windows(n).map(|w| w.join("\u{1f}")).collect()
}

/// 推理链近似度（shingle Jaccard，确定性近似 ROUGE）。
fn chain_similarity(a: &str, b: &str) -> f64 {
    jaccard(&chain_shingles(a), &chain_shingles(b))
}

/// 判本条论点相对某条历史论点是否「附和」（三种信号按 余弦 → 引用 → 推理链 顺序）。
fn sycophancy_signal(arg: &DebateArgument, prev: &DebateArgument) -> Option<SycophancySignal> {
    if claim_cosine(&arg.claim, &prev.claim) > SYCOPHANCY_COSINE_THRESHOLD {
        return Some(SycophancySignal::ClaimCosine);
    }
    if citation_jaccard(&arg.citations, &prev.citations) > SYCOPHANCY_CITATION_JACCARD {
        return Some(SycophancySignal::CitationJaccard);
    }
    if chain_similarity(&arg.reasoning, &prev.reasoning) > SYCOPHANCY_CHAIN_THRESHOLD {
        return Some(SycophancySignal::ReasoningChain);
    }
    None
}

/// 一轮辩论的收敛：先做论点独立性校验（附和降权），再按证据链强度定胜负。
///
/// 「历史」= 本条之前提交的论点（按提交顺序），故第一条论点天然独立。
pub fn converge_debate(arguments: &[DebateArgument]) -> DebateOutcome {
    let mut evaluations: Vec<ArgumentEvaluation> = Vec::with_capacity(arguments.len());
    for (i, arg) in arguments.iter().enumerate() {
        let mut signal = None;
        let mut echo_of = None;
        for prev in arguments.iter().take(i) {
            if let Some(hit) = sycophancy_signal(arg, prev) {
                signal = Some(hit);
                echo_of = Some(prev.role.clone());
                break;
            }
        }
        let weight = if signal.is_some() {
            SYCOPHANCY_PENALTY
        } else {
            ARGUMENT_WEIGHT
        };
        let evidence = arg.evidence.clamp(0.0, 1.0);
        evaluations.push(ArgumentEvaluation {
            role: arg.role.clone(),
            claim: arg.claim.clone(),
            weight,
            evidence,
            score: weight * evidence,
            sycophancy: signal,
            echo_of,
        });
    }

    // 按角色汇总证据链强度（同一角色多条论点累加：Σ 权重 × 证据分）。
    let mut support: Vec<(String, f64)> = Vec::new();
    for ev in &evaluations {
        match support.iter_mut().find(|(role, _)| role == &ev.role) {
            Some((_, sum)) => *sum += ev.score,
            None => support.push((ev.role.clone(), ev.score)),
        }
    }
    let total: f64 = support.iter().map(|(_, score)| *score).sum();

    // 胜者按强度（首个严格最大者），不按人头。
    let mut best: Option<(usize, f64)> = None;
    for (idx, (_, score)) in support.iter().enumerate() {
        let stronger = match best {
            Some((_, top)) => *score > top,
            None => true,
        };
        if stronger {
            best = Some((idx, *score));
        }
    }
    let converged = match best {
        Some((_, score)) => total > 0.0 && score / total >= CONVERGENCE_MIN_SHARE,
        None => false,
    };
    let winner = best.map(|(idx, _)| support[idx].0.clone());

    // 少数派：每个落败角色的最强论点原样保留 + 置信度。
    let mut minority: Vec<MinorityOpinion> = Vec::new();
    if let Some((winner_idx, _)) = best {
        for (idx, (role, score)) in support.iter().enumerate() {
            if idx == winner_idx {
                continue;
            }
            let mut strongest: Option<&ArgumentEvaluation> = None;
            for ev in &evaluations {
                if &ev.role != role {
                    continue;
                }
                let better = match strongest {
                    Some(top) => ev.score > top.score,
                    None => true,
                };
                if better {
                    strongest = Some(ev);
                }
            }
            if let Some(ev) = strongest {
                minority.push(MinorityOpinion {
                    role: role.clone(),
                    claim: ev.claim.clone(),
                    evidence: ev.evidence,
                    confidence: if total > 0.0 { score / total } else { 0.0 },
                    sycophancy: ev.sycophancy,
                });
            }
        }
    }

    let mut summary = String::new();
    match best {
        Some((idx, score)) => {
            let share = if total > 0.0 { score / total } else { 0.0 };
            let tag = if converged { "[收敛]" } else { "[未收敛]" };
            summary.push_str(&format!(
                "{tag} 胜出：{}（证据链强度 {score:.2}，占比 {:.0}%，阈值 {:.0}%）",
                support[idx].0,
                share * 100.0,
                CONVERGENCE_MIN_SHARE * 100.0
            ));
        }
        None => summary.push_str("[未收敛] 本轮无有效论点"),
    }
    for opinion in &minority {
        let mark = match opinion.sycophancy {
            Some(signal) => format!("，附和降权（{signal}）"),
            None => String::new(),
        };
        summary.push_str(&format!(
            "\n[少数派] {}：{}（置信度 {:.0}%{mark}）",
            opinion.role,
            opinion.claim,
            opinion.confidence * 100.0
        ));
    }

    DebateOutcome {
        evaluations,
        support,
        winner,
        converged,
        minority,
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::mcp::registry::ToolRegistry;
    use crate::model::llm::{LLMClient, LLMConfig};
    use crate::model::router::ModelRouter;
    use crate::persistence::config::Config;

    /// 构建一个「离线」子 Agent 管理器：模型不可达，子 Agent 执行会失败，
    /// 但 spawn / wait / 收集链路仍然真实跑通——证明 [`Team::run`] 不依赖 LLM。
    fn offline_subagents() -> SubAgentManager {
        let cfg = Config {
            model: crate::persistence::config::ModelConfig {
                provider: "custom".to_string(),
                model_name: "offline-model".to_string(),
                api_key: "offline".to_string(),
                base_url: "http://127.0.0.1:1".to_string(),
                copilot_model: None,
                timeout_secs: 0,
                    wire_api: crate::model::wire::WireApi::Chat,
            },
            ..Default::default()
        };
        let router = Arc::new(ModelRouter::new(&cfg));
        let reg = Arc::new(ToolRegistry::new());
        SubAgentManager::new(
            crate::agent::subagent::SubAgentConfig::default(),
            reg,
            router,
        )
    }

    use std::sync::Arc;

    #[tokio::test]
    async fn test_team_run_spawns_and_collects_all_members_offline() {
        // 离线下子 Agent 执行会失败，但 Team::run 必须：spawn N 个、收集 N 个、
        // 角色不丢、聚合非空——证明团队编排是真实执行体，而非占位。
        let subagents = offline_subagents();
        let team = Team::new()
            .member(
                "编码实现",
                "你是资深工程师，给出可运行实现。",
                "实现登录功能",
                IntentType::Code,
                Complexity::Medium,
            )
            .member(
                "测试",
                "你是测试专家，设计可执行用例。",
                "为登录功能编写单元测试",
                IntentType::Code,
                Complexity::Medium,
            );

        let outcome = team.run(&subagents, "用户要求登录能力").await;

        // 每个成员都被如实收集（离线失败也进结果，不丢弃）
        assert_eq!(outcome.member_results.len(), 2, "应收集到 2 个成员结论");
        let roles: Vec<&str> = outcome
            .member_results
            .iter()
            .map(|(r, _)| r.as_str())
            .collect();
        assert!(roles.contains(&"编码实现"), "应保留编码实现角色");
        assert!(roles.contains(&"测试"), "应保留测试角色");
        // 聚合上下文按角色拼接、非空
        assert!(!outcome.aggregated.is_empty(), "聚合结论不应为空");
        assert!(
            outcome.aggregated.contains("[编码实现]"),
            "聚合应含编码角色标签"
        );
        assert!(
            outcome.aggregated.contains("[测试]"),
            "聚合应含测试角色标签"
        );
    }

    #[tokio::test]
    async fn test_team_shared_prefix_counts_reuse() {
        // 注入共享前缀缓存后，同 system 前缀的多个成员应被观测为前缀复用（prefix_hits 上升）。
        let subagents = offline_subagents();
        let shared = Arc::new(PrefixCache::new(16));
        let team = Team::new()
            .with_prefix_cache(shared.clone())
            .member(
                "检索",
                "你是团队共享的检索专家，依据同一知识库作答。",
                "查找相关信息",
                IntentType::General,
                Complexity::Medium,
            )
            .member(
                "写作",
                "你是团队共享的检索专家，依据同一知识库作答。",
                "据此撰写摘要",
                IntentType::General,
                Complexity::Medium,
            );

        let _outcome = team.run(&subagents, "团队任务").await;

        // 两个成员 system 前缀相同 → 第二次起记为复用，prefix_hits 至少 1。
        assert!(
            shared.stats().prefix_hits >= 1,
            "同前缀成员应被观测为前缀复用，prefix_hits={}",
            shared.stats().prefix_hits
        );
    }

    #[tokio::test]
    async fn test_team_default_constructs_shared_prefix_cache() {
        // W-2：默认构造的 Team（不显式 with_prefix_cache）应自动具备共享前缀缓存，
        // 同 system 前缀的多成员运行后 prefix_hits 上升——验证「默认注入」。
        let subagents = offline_subagents();
        let team = Team::new()
            .member(
                "检索",
                "你是团队共享的检索专家，依据同一知识库作答。",
                "查找相关信息",
                IntentType::General,
                Complexity::Medium,
            )
            .member(
                "写作",
                "你是团队共享的检索专家，依据同一知识库作答。",
                "据此撰写摘要",
                IntentType::General,
                Complexity::Medium,
            );

        // 默认构造即应注入 SharedPrefixCache（非 None）——此为 W-2 核心行为。
        assert!(
            team.prefix_cache.is_some(),
            "Team::new 应默认注入 SharedPrefixCache（W-2 尚未接线时缺失）"
        );

        let _outcome = team.run(&subagents, "团队任务").await;

        // 自动注入的缓存应真实观测到同前缀复用（prefix_hits 上升）。
        let stats = team.prefix_cache.as_ref().unwrap().cache().stats();
        assert!(
            stats.prefix_hits >= 1,
            "默认注入的共享前缀缓存应观测到同前缀成员复用，prefix_hits={}",
            stats.prefix_hits
        );
    }

    #[tokio::test]
    async fn test_team_with_prefetch_dedups_identical_prefix() {
        // W-9：注入团队级去重预填充后，相同稳定前缀的多成员只预热一次（窗口内去重）。
        let subagents = offline_subagents();
        let dedup = Arc::new(PrefetchDedup::new(
            LLMClient::new(LLMConfig {
                base_url: "http://127.0.0.1:1".to_string(), // 不可达——预热失败但尽力而为不去重例外
                api_key: "k".to_string(),
                timeout_secs: 1,
                ..Default::default()
            }),
            std::time::Duration::from_secs(60),
        ));
        let team = Team::new()
            .with_prefetch(dedup.clone())
            .member(
                "甲",
                "同一人设前缀",
                "任务A",
                IntentType::General,
                Complexity::Medium,
            )
            .member(
                "乙",
                "同一人设前缀",
                "任务B",
                IntentType::General,
                Complexity::Medium,
            );

        let _outcome = team.run(&subagents, "团队任务").await;

        let s = dedup.stats();
        assert_eq!(s.prefetches, 1, "同前缀多成员只预热一次, got {s:?}");
        assert!(s.deduped >= 1, "第二个同前缀成员应去重跳过, got {s:?}");
    }

    // ---- P2P 直连信箱（纯逻辑，不需 LLM）----

    #[test]
    fn mailbox_delivers_to_role_and_broadcast() {
        let mut mb = Mailbox::new();
        mb.send(TeamMessage::new("A", "B", "hi B"));
        mb.broadcast("C", "to all");
        // B 抽到 A 的私信 + C 的广播（共 2 条）
        let b = mb.drain_for("B");
        assert_eq!(b.len(), 2);
        // 抽完后信箱清空，再抽为空（幂等）
        assert!(mb.is_empty());
        assert!(mb.drain_for("B").is_empty());
    }

    // ---- run_p2p：依赖图唤醒调度（离线冒烟，不依赖 LLM）----

    #[tokio::test]
    async fn test_run_p2p_collects_all_members_offline() {
        // 离线下子 Agent 执行失败，但 run_p2p 必须：spawn 并收集全部成员、角色不丢、聚合非空。
        let subagents = offline_subagents();
        let team = Team::new()
            .member(
                "编码实现",
                "你是资深工程师。",
                "实现登录功能",
                IntentType::Code,
                Complexity::Medium,
            )
            .member_dep(
                "测试",
                "你是测试专家。",
                "为登录功能编写测试",
                IntentType::Code,
                Complexity::Medium,
                &["编码实现"], // 测试 依赖 编码实现
            );

        let outcome = team.run_p2p(&subagents, "用户要求登录能力").await;

        assert_eq!(outcome.member_results.len(), 2, "应收集到 2 个成员结论");
        let roles: Vec<&str> = outcome
            .member_results
            .iter()
            .map(|(r, _)| r.as_str())
            .collect();
        assert!(roles.contains(&"编码实现"));
        assert!(roles.contains(&"测试"));
        assert!(!outcome.aggregated.is_empty(), "聚合结论不应为空");
    }

    #[tokio::test]
    async fn test_run_p2p_acyclic_collects_even_with_deps() {
        // 依赖链 A->B->C：离线全失败，但三成员都应被如实收集（不受依赖图影响）。
        let subagents = offline_subagents();
        let team = Team::new()
            .member("A", "a", "任务A", IntentType::General, Complexity::Medium)
            .member_dep(
                "B",
                "b",
                "任务B",
                IntentType::General,
                Complexity::Medium,
                &["A"],
            )
            .member_dep(
                "C",
                "c",
                "任务C",
                IntentType::General,
                Complexity::Medium,
                &["B"],
            );

        let outcome = team.run_p2p(&subagents, "链任务").await;
        assert_eq!(outcome.member_results.len(), 3, "依赖链三成员应全部收集");
    }

    #[tokio::test]
    async fn test_run_p2p_matches_run_when_no_deps() {
        // 无依赖时 run_p2p 必须与 run 逐成员一致（角色序 / 数量），保证把编排层
        // 的 `team.run()` 换成 `run_p2p()` 零行为回归。
        let subagents = offline_subagents();
        let mk = || {
            Team::new()
                .member(
                    "编码实现",
                    "你是工程师。",
                    "实现登录功能",
                    IntentType::Code,
                    Complexity::Medium,
                )
                .member(
                    "测试",
                    "你是测试专家。",
                    "为登录编写测试",
                    IntentType::Code,
                    Complexity::Medium,
                )
        };
        let a = mk().run(&subagents, "用户要求登录能力").await;
        let b = mk().run_p2p(&subagents, "用户要求登录能力").await;
        let ra: Vec<&str> = a.member_results.iter().map(|(r, _)| r.as_str()).collect();
        let rb: Vec<&str> = b.member_results.iter().map(|(r, _)| r.as_str()).collect();
        assert_eq!(ra, rb, "无依赖时 run_p2p 应与 run 角色序一致");
        assert_eq!(a.member_results.len(), b.member_results.len());
    }

    #[tokio::test]
    async fn test_run_p2p_cycle_marks_unreachable() {
        // A<->B 环依赖：两成员都应被如实收集（带环标记），绝不静默丢弃。
        let subagents = offline_subagents();
        let team = Team::new()
            .member_dep(
                "A",
                "角色A。",
                "任务A",
                IntentType::General,
                Complexity::Medium,
                &["B"],
            )
            .member_dep(
                "B",
                "角色B。",
                "任务B",
                IntentType::General,
                Complexity::Medium,
                &["A"],
            );
        let outcome = team.run_p2p(&subagents, "环任务").await;
        assert_eq!(
            outcome.member_results.len(),
            2,
            "环依赖两成员都应被收集，不丢"
        );
        let roles: Vec<String> = outcome
            .member_results
            .iter()
            .map(|(r, _)| r.clone())
            .collect();
        assert!(roles.contains(&"A".to_string()) && roles.contains(&"B".to_string()));
        for (_, res) in &outcome.member_results {
            assert!(res.contains("依赖环"), "环成员应被标记: {res}");
        }
    }

    #[tokio::test]
    async fn test_run_p2p_same_role_distinct_deps_not_dropped() {
        // 守卫（P2）：两个同角色但依赖集不同的成员，二者都必须被调度并收集，
        // 绝不能因 run_p2p 以「角色名」为调度键而把后一个未 spawn 的成员静默丢弃。
        // 构造跨波场景：ImplA（依赖 Dep1）在第 2 波完成，ImplB（依赖 Impl）在第 3 波；
        // 旧实现会在 ImplA 完成后以角色 "Impl" 为键 retain，误删未 spawn 的 ImplB。
        let subagents = offline_subagents();
        let team = Team::new()
            .member_dep(
                "Dep1",
                "你是调研角色。",
                "产出调研结论",
                IntentType::General,
                Complexity::Medium,
                &[],
            )
            .member_dep(
                "Impl",
                "你是实现角色A。",
                "基于调研实现功能",
                IntentType::General,
                Complexity::Medium,
                &["Dep1"],
            )
            .member_dep(
                "Impl",
                "你是实现角色B。",
                "基于上游实现再加工",
                IntentType::General,
                Complexity::Medium,
                &["Impl"],
            );

        let outcome = team.run_p2p(&subagents, "同角色异依赖守卫").await;
        // 三个成员都应被如实收集（无「依赖环」误判、无静默丢弃）
        assert_eq!(
            outcome.member_results.len(),
            3,
            "同角色异依赖的两个 Impl 都应被调度并收集，不得静默丢弃"
        );
        for (role, res) in &outcome.member_results {
            assert!(
                !res.contains("依赖环"),
                "成员 {} 不应被误判为依赖环而丢弃",
                role
            );
        }
    }
}

/// P3-2 辩论防极化的回归守卫（纯逻辑，无 LLM / 无网络）。
#[cfg(test)]
mod debate_tests {
    use super::*;

    /// 附和（换皮复述）必须被降权到 ×0.2，并记录被附和的来源角色。
    #[test]
    fn debate_penalizes_sycophantic_restatement_weight() {
        let original = DebateArgument::new("架构", "应引入本地缓存层以降低读延迟")
            .with_evidence(0.5)
            .with_citations(&["bench-2026-10"]);
        let echo = DebateArgument::new("后端", "应引入本地缓存层，可降低读延迟")
            .with_evidence(0.5)
            .with_citations(&["bench-2026-10"]);

        let outcome = converge_debate(&[original, echo]);
        let first = &outcome.evaluations[0];
        let second = &outcome.evaluations[1];

        assert_eq!(first.weight, ARGUMENT_WEIGHT, "第一条论点无历史可比，应视为独立");
        assert_eq!(first.sycophancy, None);
        assert_eq!(
            second.weight, SYCOPHANCY_PENALTY,
            "换皮复述必须被降权到 ×{}",
            SYCOPHANCY_PENALTY
        );
        assert_eq!(second.sycophancy, Some(SycophancySignal::ClaimCosine));
        assert_eq!(second.echo_of.as_deref(), Some("架构"), "应记录被附和的角色");
        assert!(
            (second.score - SYCOPHANCY_PENALTY * 0.5).abs() < 1e-9,
            "降权后强度 = 权重 × 证据分，实际 {}",
            second.score
        );
    }

    /// 收敛按证据链强度而非人头：两人复述旧结论，仍输给一个带新证据的少数派。
    #[test]
    fn debate_converges_by_evidence_strength_not_headcount() {
        let majority_a = DebateArgument::new("架构", "应引入本地缓存层以降低读延迟")
            .with_evidence(0.5)
            .with_citations(&["bench-2026-10"]);
        let majority_b = DebateArgument::new("性能", "应引入本地缓存层，可降低读延迟")
            .with_evidence(0.5)
            .with_citations(&["bench-2026-10"]);
        let dissenter = DebateArgument::new("安全", "本地缓存会暴露陈旧凭据，应改用短期令牌")
            .with_evidence(1.0)
            .with_citations(&["cve-2024-1234"]);

        let outcome = converge_debate(&[majority_a, majority_b, dissenter]);

        // 人头 2:1 站在「本地缓存」一边，但证据链强度：0.5 + 0.1 < 1.0。
        let support: Vec<(&str, f64)> = outcome
            .support
            .iter()
            .map(|(role, score)| (role.as_str(), *score))
            .collect();
        let score_of = |role: &str| {
            support
                .iter()
                .find(|(r, _)| *r == role)
                .map(|(_, s)| *s)
                .unwrap_or(f64::NAN)
        };
        assert_eq!(score_of("架构"), 0.5);
        assert_eq!(score_of("性能"), SYCOPHANCY_PENALTY * 0.5);
        assert_eq!(score_of("安全"), 1.0);

        assert_eq!(
            outcome.winner.as_deref(),
            Some("安全"),
            "胜者应由证据链强度决定，而非人头"
        );
        assert!(outcome.converged, "1.0/(1.6) 超过收敛阈值，应判收敛");
        assert!(
            outcome.summary.contains("[收敛]") && outcome.summary.contains("安全"),
            "摘要应给出收敛胜者，实际 {}",
            outcome.summary
        );
    }

    /// 落败的少数派意见与置信度必须被保留（不被胜者吞掉）。
    #[test]
    fn debate_keeps_minority_opinion_with_confidence() {
        let gate = DebateArgument::new("发布", "应把发布门禁前置到提交前")
            .with_evidence(1.0)
            .with_citations(&["ci-log-1"]);
        let canary = DebateArgument::new("运维", "应增加灰度发布窗口")
            .with_evidence(0.6)
            .with_citations(&["ops-runbook-2"]);
        let docs = DebateArgument::new("文档", "应先在文档补齐再发版")
            .with_evidence(0.3)
            .with_citations(&["doc-audit-3"]);

        let outcome = converge_debate(&[gate, canary, docs]);
        assert_eq!(outcome.winner.as_deref(), Some("发布"));
        assert_eq!(outcome.minority.len(), 2, "两名少数派都应被保留");
        assert_eq!(outcome.minority[0].role, "运维");
        assert_eq!(
            outcome.minority[0].claim, "应增加灰度发布窗口",
            "少数派结论必须原文保留"
        );
        assert_eq!(outcome.minority[1].role, "文档");
        // 置信度 = 该角色强度 / 总强度（1.0 + 0.6 + 0.3 = 1.9）
        let total = 1.0 + 0.6 + 0.3;
        assert!((outcome.minority[0].confidence - 0.6 / total).abs() < 1e-9);
        assert!((outcome.minority[1].confidence - 0.3 / total).abs() < 1e-9);
        assert!(
            outcome.summary.contains("[少数派] 运维") && outcome.summary.contains("[少数派] 文档"),
            "摘要必须带上少数派意见，实际 {}",
            outcome.summary
        );
    }
}
