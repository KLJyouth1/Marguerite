//! 元智能体（Meta Agent）：自迭代闭环 —— 监控 → 诊断 → 改进规划 → 沙箱评估 → 版本闭环
//!
//! 与既有 [`crate::harness::distill`] / [`crate::harness::evolve`] 的分工：
//!
//! | 层 | 既有能力 | 本模块补齐 |
//! |----|----------|------------|
//! | 监控 | 无（`diagnose(aspects)` 的输入是**人工给的字符串**） | 从真实行为台账抽取**带类型的失败信号**（[`extract_signals`]） |
//! | 诊断 | `diagnose()` 仅拼接字符串 | 四类**根因裁决** + 证据 + 置信度（[`diagnose`]） |
//! | 规划 | 只有「往 system prompt 追加一句」一种变异 | **白名单化的改动目标** + 风险分级（[`ChangeTarget`]） |
//! | 沙箱 | 评分由调用方外部传入 | **回放用例集**上的前后对照 + 成本/时延闸（[`evaluate_candidate`]） |
//! | 闭环 | `EvolutionRunner` 有历史与回滚 | **版本快照 + 灰度阶段 + 回滚目标**（[`MetaLedger`]） |
//!
//! ## 三条硬边界（用户方案里的「避坑点」，此处用类型与门禁固化）
//!
//! 1. **不许碰核心代码**：[`ChangeTarget`] 只枚举 prompt / 规则 / 工具默认参数 /
//!    沙箱内脚本 / 知识库 / few-shot 六类，**没有**「改 core 源码」这一变体 ——
//!    结构上不可能。护栏测试 [`tests::core_source_is_not_a_change_target`] 断言这一点。
//! 2. **默认不自动上线**：[`MetaPolicy::auto_apply`] 默认 `false`（对应方案里的「路线 A」），
//!    提案进待审队列；人工 [`MetaAgent::approve`] 后才进入灰度。
//! 3. **成本进闸**：采纳不只看「跑得通」，还看 token 与时延相对基线的涨幅
//!    （[`PromotionCriteria::max_token_growth`] / `max_latency_growth`），
//!    杜绝「自动改好、但每轮多烧一倍 token」。
//!
//! ## 不做什么（诚实声明）
//!
//! - **不自动改工具源码**：默认风险上限为 [`RiskLevel::Medium`]，`ToolScript` 属
//!   [`RiskLevel::High`]，需显式放宽 `max_risk` 才会产出该类提案。
//! - **不自动判定「改进有效」**：[`EvalOutcome`] 由真实回放产生，本模块只做**裁决**，
//!   不臆造评分。

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::memory::behavior::BehaviorEvent;
use super::trust::{budget_gate, trust_gate, ChangeProposal, EditBudget};

/// 计划闸：裁决通过、登记快照之前运行；返回 `Err` 表示拒绝合并。
///
/// 抽成别名是为了让 `run_cycle_gated` 的签名可读（否则 clippy 会判定
/// `Option<&dyn Fn(..) -> Result<..>>` 为 `type_complexity`），语义完全不变。
type PlanGate<'a> = &'a dyn Fn(&ImprovementPlan) -> Result<(), String>;

/// 元台账文件名（落在 `data_dir/meta/`）
const LEDGER_FILE: &str = "meta_evolution.jsonl";
/// 诊断证据最多保留条数（防 report 膨胀）
const MAX_EVIDENCE: usize = 8;
/// 信号 detail 截断字符数
const SIGNAL_CHARS: usize = 160;
/// 同一工具在窗口内失败达到该次数 → 判定为工具缺陷而非偶发
const TOOL_DEFECT_THRESHOLD: usize = 3;
/// 同一需求重复出现达到该次数 → 判定为能力缺口
const REPEAT_ASK_THRESHOLD: usize = 2;

// ───────────────────────────────────────────────────────────────────────────
// 第 1 层：监控 —— 从行为台账抽取带类型的失败信号
// ───────────────────────────────────────────────────────────────────────────

/// 失败信号类别（对应方案里的三类问题）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    /// 主 Agent 自身：规划跑偏 / 轮次空转 / 主动中止 / 回复自承失败
    AgentSelf,
    /// 工具故障：报错、超时、空值、鉴权失败
    ToolFault,
    /// 工具能力不足：能跑但产出不足（同工具反复失败或产出被判定不可用）
    ToolInsufficient,
    /// 任务能力不足：同类需求反复提出仍未完成（缺工具 / 缺知识 / 缺步骤）
    CapabilityGap,
}

impl SignalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgentSelf => "agent_self",
            Self::ToolFault => "tool_fault",
            Self::ToolInsufficient => "tool_insufficient",
            Self::CapabilityGap => "capability_gap",
        }
    }
}

/// 一条失败信号
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailureSignal {
    pub kind: SignalKind,
    /// 所属会话
    pub session: String,
    /// 工具名（工具相关信号才有）
    pub tool: Option<String>,
    /// 证据细节（已脱敏、限长）
    pub detail: String,
    /// 事件时间戳（毫秒）
    pub at_ms: u64,
}

/// 从行为事件窗口抽取失败信号（纯函数，确定性、无副作用）。
///
/// 识别规则（全部基于台账里**已经存在**的 kind / detail，不额外埋点）：
/// - `aborted` → 主 Agent 自身（用户或系统中止了本轮）；
/// - `tool_result` 命中错误词表 → 工具故障；
/// - 同一工具在窗口内失败 ≥ [`TOOL_DEFECT_THRESHOLD`] 次 → 追加一条工具能力不足；
/// - `assistant_reply` 命中自承失败词表 → 主 Agent 自身（规划/产出问题）；
/// - 同一 `user_message` 重复 ≥ [`REPEAT_ASK_THRESHOLD`] 次且其后无成功回复 → 能力缺口。
pub fn extract_signals(events: &[BehaviorEvent]) -> Vec<FailureSignal> {
    const TOOL_ERR: &[&str] = &[
        "error", "failed", "失败", "超时", "timeout", "panic", "not found",
        "permission denied", "拒绝访问", "unavailable", "不可用",
    ];
    const SELF_ERR: &[&str] = &[
        "无法", "失败", "未产出", "降级", "不可用", "中止", "缺失",
    ];

    let mut signals: Vec<FailureSignal> = Vec::new();
    // 工具 → 失败次数（用于「能力不足」判定）
    let mut tool_failures: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    // 用户需求 → 出现次数（用于「能力缺口」判定）
    let mut asks: std::collections::BTreeMap<String, (usize, u64, String)> =
        std::collections::BTreeMap::new();
    // 每个会话是否出现过成功回复（用于能力缺口判定）
    let mut had_success: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    let lower = |s: &str| s.to_lowercase();
    let hits = |text: &str, table: &[&str]| -> bool {
        let t = lower(text);
        table.iter().any(|k| t.contains(k))
    };

    for ev in events {
        match ev.kind.as_str() {
            "tool_result" if hits(&ev.detail, TOOL_ERR) => {
                let tool = tool_name(&ev.detail);
                if let Some(t) = &tool {
                    *tool_failures.entry(t.clone()).or_default() += 1;
                }
                signals.push(FailureSignal {
                    kind: SignalKind::ToolFault,
                    session: ev.session.clone(),
                    tool: tool.clone(),
                    detail: clip(&ev.detail),
                    at_ms: ev.at_ms,
                });
            }
            "aborted" => signals.push(FailureSignal {
                kind: SignalKind::AgentSelf,
                session: ev.session.clone(),
                tool: None,
                detail: clip(&ev.detail),
                at_ms: ev.at_ms,
            }),
            "assistant_reply" => {
                if hits(&ev.detail, SELF_ERR) {
                    signals.push(FailureSignal {
                        kind: SignalKind::AgentSelf,
                        session: ev.session.clone(),
                        tool: None,
                        detail: clip(&ev.detail),
                        at_ms: ev.at_ms,
                    });
                } else {
                    had_success.insert(ev.session.clone());
                }
            }
            "user_message" => {
                let key = ev.detail.trim().to_string();
                if !key.is_empty() {
                    let slot = asks
                        .entry(key)
                        .or_insert((0, ev.at_ms, ev.session.clone()));
                    slot.0 += 1;
                    slot.1 = ev.at_ms;
                }
            }
            _ => {}
        }
    }

    // 工具反复失败 → 能力不足（能跑，但解决不了问题）
    for (tool, count) in tool_failures {
        if count >= TOOL_DEFECT_THRESHOLD {
            signals.push(FailureSignal {
                kind: SignalKind::ToolInsufficient,
                session: String::new(),
                tool: Some(tool.clone()),
                detail: format!("工具 {tool} 在观察窗口内失败 {count} 次，产出不可用"),
                at_ms: 0,
            });
        }
    }

    // 同类需求反复提出且从未成功 → 能力缺口
    for (ask, (count, at_ms, session)) in asks {
        if count >= REPEAT_ASK_THRESHOLD && !had_success.contains(&session) {
            signals.push(FailureSignal {
                kind: SignalKind::CapabilityGap,
                session,
                tool: None,
                detail: format!("需求「{ask}」重复 {count} 次且无成功产出"),
                at_ms,
            });
        }
    }

    signals
}

/// 从真实行为台账抽取最近信号（监控层入口）。
pub fn observe_recent(limit: usize) -> Vec<FailureSignal> {
    extract_signals(&crate::memory::behavior::recent_events(limit))
}

/// 从工具结果明细里取工具名（约定形如 `tool_name(args…)`，与 `behavior_summary` 同源）
fn tool_name(detail: &str) -> Option<String> {
    let head = detail.split(['(', '（', ':', '：']).next()?.trim();
    if head.is_empty() || head.chars().any(char::is_whitespace) {
        return None;
    }
    Some(head.to_string())
}

fn clip(s: &str) -> String {
    if s.chars().count() <= SIGNAL_CHARS {
        return s.to_string();
    }
    format!("{}…", s.chars().take(SIGNAL_CHARS).collect::<String>())
}

// ───────────────────────────────────────────────────────────────────────────
// 第 2 层：诊断 —— 根因裁决
// ───────────────────────────────────────────────────────────────────────────

/// 根因（对应方案里「改什么」的四类来源）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootCause {
    /// Agent 规划逻辑弱 / 提示词不足
    PlanLogicWeak,
    /// 工具调用参数错（入参、超时、重试次数）
    ToolParamWrong,
    /// 工具脚本缺陷（沙箱内逻辑缺失）
    ToolScriptDefect,
    /// 能力缺口（缺工具 / 缺知识 / 缺示例）
    CapabilityMissing,
}

impl RootCause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PlanLogicWeak => "plan_logic_weak",
            Self::ToolParamWrong => "tool_param_wrong",
            Self::ToolScriptDefect => "tool_script_defect",
            Self::CapabilityMissing => "capability_missing",
        }
    }
}

/// 结构化诊断报告
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnosis {
    pub cause: RootCause,
    /// 支撑证据（已在抽取阶段脱敏、限长）
    pub evidence: Vec<String>,
    /// 置信度 0.0–1.0（由信号数与一致性决定，可测）
    pub confidence: f64,
    /// 参与裁决的信号总数
    pub signals: usize,
}

/// 根因裁决（确定性仲裁，非 LLM 猜测 —— 保证可复现与可测）。
///
/// 优先级（高影响歧义优先）：
/// 1. 存在**能力缺口**信号 → [`RootCause::CapabilityMissing`]（缺的东西改 prompt 也补不上）；
/// 2. 同一工具反复失败（工具能力不足）→ [`RootCause::ToolScriptDefect`]；
/// 3. 工具故障命中参数词表 → [`RootCause::ToolParamWrong`]；
/// 4. 其余（含仅 AgentSelf 信号）→ [`RootCause::PlanLogicWeak`]。
///
/// 无信号时返回 `None` —— **不编造**一个根因。
pub fn diagnose(signals: &[FailureSignal]) -> Option<Diagnosis> {
    if signals.is_empty() {
        return None;
    }

    let mut evidence: Vec<String> = Vec::new();
    for s in signals {
        if evidence.len() >= MAX_EVIDENCE {
            break;
        }
        if s.detail.is_empty() {
            continue;
        }
        evidence.push(format!("[{}] {}", s.kind.as_str(), s.detail));
    }

    let cause = if signals
        .iter()
        .any(|s| s.kind == SignalKind::CapabilityGap)
    {
        RootCause::CapabilityMissing
    } else if signals
        .iter()
        .any(|s| s.kind == SignalKind::ToolInsufficient)
    {
        RootCause::ToolScriptDefect
    } else if signals.iter().any(|s| {
        s.kind == SignalKind::ToolFault
            && s.tool.is_some()
            && looks_like_param_error(&s.detail)
    }) {
        RootCause::ToolParamWrong
    } else {
        RootCause::PlanLogicWeak
    };

    // 置信度：信号越多越高，但同一类重复不再加权（避免「刷量」抬置信）。
    let distinct_kinds = {
        let mut kinds: Vec<&str> = signals.iter().map(|s| s.kind.as_str()).collect();
        kinds.sort_unstable();
        kinds.dedup();
        kinds.len()
    };
    let volume = (signals.len() as f64 / 5.0).min(1.0);
    let diversity = distinct_kinds as f64 / 4.0;
    let confidence = (0.4 + 0.35 * volume + 0.25 * diversity).min(0.95);

    Some(Diagnosis {
        cause,
        evidence,
        confidence,
        signals: signals.len(),
    })
}

/// 参数类错误词表（决定「工具参数错」与「工具脚本缺陷」的分流）
fn looks_like_param_error(detail: &str) -> bool {
    const PARAM_WORDS: &[&str] = &[
        "invalid", "missing", "required", "argument", "parameter", "参数", "缺少", "格式",
        "timeout", "超时", "retry", "重试",
    ];
    let t = detail.to_lowercase();
    PARAM_WORDS.iter().any(|w| t.contains(w))
}

// ───────────────────────────────────────────────────────────────────────────
// 第 3 层：改进规划 —— 白名单化的改动目标
// ───────────────────────────────────────────────────────────────────────────

/// 允许自动修改的对象（**白名单，且不含核心源码**）
///
/// 变体保持 `Copy`（无载荷）；**具体是哪个专家/团队/技能，写在
/// [`ImprovementPlan::mutation`] 的文本里**。这样做的代价是"改哪个对象"不再是
/// 类型级信息，收益是 `ChangeTarget` 保持 `Copy`、且白名单的穷尽匹配不被破坏。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeTarget {
    /// 系统提示词模板
    SystemPrompt,
    /// 任务拆解 / 校验规则
    TaskRules,
    /// 工具默认入参（超时、重试次数等）
    ToolDefaultParams,
    /// 沙箱内的工具脚本
    ToolScript,
    /// 知识库条目
    KnowledgeBase,
    /// few-shot 示例
    FewShot,
    // ── RSI 扩展：让「专家 / 团队 / 蜂群 / 记忆 / 技能 / 检索 / 评测集」进闭环 ──
    /// 专家（43 角色）的 checklist 维度与阈值
    ExpertChecklist,
    /// 团队（8 个）的 SOP 编排参数（阶段门、并行度）
    TeamSop,
    /// 蜂群规模与分工策略
    SwarmPolicy,
    /// 记忆分层权重（写入价值门、召回权重）
    MemoryWeight,
    /// 技能定义（前置条件 / 完成证据 / 反例）
    SkillDefinition,
    /// 检索策略（混合权重、召回阈值）
    RetrievalPolicy,
    /// 评测集本身（L5 元改进：改「怎么量」）
    EvalTaskSet,
}

impl ChangeTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SystemPrompt => "system_prompt",
            Self::TaskRules => "task_rules",
            Self::ToolDefaultParams => "tool_default_params",
            Self::ToolScript => "tool_script",
            Self::KnowledgeBase => "knowledge_base",
            Self::FewShot => "few_shot",
            Self::ExpertChecklist => "expert_checklist",
            Self::TeamSop => "team_sop",
            Self::SwarmPolicy => "swarm_policy",
            Self::MemoryWeight => "memory_weight",
            Self::SkillDefinition => "skill_definition",
            Self::RetrievalPolicy => "retrieval_policy",
            Self::EvalTaskSet => "eval_task_set",
        }
    }

    /// 该目标自带的风险级别
    ///
    /// 分级依据是**改坏了有多难发现**，不是改了多少行：
    /// - 评测集是「量尺」，改它等于允许自己给自己出题 → High；
    /// - 技能与蜂群直接改变可执行行为与并发 → High；
    /// - 专家 checklist / 团队 SOP / 记忆权重 / 检索策略影响判断但不直接执行 → Medium。
    pub fn risk(self) -> RiskLevel {
        match self {
            Self::SystemPrompt | Self::TaskRules | Self::FewShot => RiskLevel::Low,
            Self::ToolDefaultParams
            | Self::KnowledgeBase
            | Self::ExpertChecklist
            | Self::TeamSop
            | Self::MemoryWeight
            | Self::RetrievalPolicy => RiskLevel::Medium,
            Self::ToolScript | Self::SwarmPolicy | Self::SkillDefinition | Self::EvalTaskSet => {
                RiskLevel::High
            }
        }
    }
}

/// 改动风险级别
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

impl RiskLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// 一条改进提案
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImprovementPlan {
    pub target: ChangeTarget,
    pub cause: RootCause,
    pub risk: RiskLevel,
    /// 建议的改动内容（可读文本，落地时由人工或受控执行器应用）
    pub mutation: String,
    /// 为什么这么改
    pub rationale: String,
}

/// 元智能体策略
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaPolicy {
    /// 是否允许自动应用（默认 `false` = 路线 A：提案进待审队列）
    pub auto_apply: bool,
    /// 允许产出的最高风险提案（默认 Medium：不自动改工具脚本）
    pub max_risk: RiskLevel,
    /// 采纳所需最低成功率
    pub min_success_rate: f64,
    /// 允许的 token 成本涨幅（1.2 = 最多 +20%）
    pub max_token_growth: f64,
    /// 允许的时延涨幅（1.25 = 最多 +25%）
    pub max_latency_growth: f64,
    /// 单个任务允许损失的通过回合数（`0` = 一个都不许丢）
    ///
    /// 这是**回归门控**的核心：只看总分会放过「总分涨、某任务崩」的情形。
    pub max_per_task_loss: usize,
    /// 红线任务 ID：只要退化就拒，不看总分涨多少
    pub protected_tasks: Vec<String>,
}

impl Default for MetaPolicy {
    fn default() -> Self {
        Self {
            auto_apply: false,
            max_risk: RiskLevel::Medium,
            min_success_rate: 0.9,
            max_token_growth: 1.2,
            max_latency_growth: 1.25,
            max_per_task_loss: 0,
            protected_tasks: Vec::new(),
        }
    }
}

/// 由诊断生成改进提案（含白名单与风险闸）。
///
/// 返回 `None` 的两种情况都**如实说明**：
/// - 诊断置信度过低（< 0.5）→ 证据不足，不提案；
/// - 首选目标的风险超过策略上限 → 需人工放宽 `max_risk`。
pub fn plan_improvement(diag: &Diagnosis, policy: &MetaPolicy) -> Option<ImprovementPlan> {
    if diag.confidence < 0.5 {
        return None;
    }
    let target = match diag.cause {
        RootCause::PlanLogicWeak => ChangeTarget::SystemPrompt,
        RootCause::ToolParamWrong => ChangeTarget::ToolDefaultParams,
        RootCause::ToolScriptDefect => ChangeTarget::ToolScript,
        RootCause::CapabilityMissing => ChangeTarget::KnowledgeBase,
    };
    let risk = target.risk();
    if risk > policy.max_risk {
        return None;
    }
    let (mutation, rationale) = match diag.cause {
        RootCause::PlanLogicWeak => (
            "在系统提示词中补入失败驱动的拆解与校验规则（固定小节 + 结果自检）".to_string(),
            "信号集中在规划/产出侧，未指向具体工具参数或脚本".to_string(),
        ),
        RootCause::ToolParamWrong => (
            "调整相关工具的默认入参与超时/重试配置".to_string(),
            "工具故障明细命中参数类词表（缺少/格式/超时）".to_string(),
        ),
        RootCause::ToolScriptDefect => (
            "在沙箱内修补该工具脚本的异常捕获与输出解析".to_string(),
            "同一工具在观察窗口内反复失败，属脚本逻辑而非调用姿势".to_string(),
        ),
        RootCause::CapabilityMissing => (
            "补充知识库条目与 few-shot 示例，覆盖反复未完成的需求类型".to_string(),
            "同类需求重复提出且从未成功，缺的是能力而非参数".to_string(),
        ),
    };
    Some(ImprovementPlan {
        target,
        cause: diag.cause,
        risk,
        mutation,
        rationale,
    })
}

// ───────────────────────────────────────────────────────────────────────────
// 第 4 层：沙箱评估 —— 回放对照 + 成本/时延闸
// ───────────────────────────────────────────────────────────────────────────

/// 一次回放评估的汇总（由真实回放产生，本模块不臆造）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EvalOutcome {
    pub passed: usize,
    pub total: usize,
    /// 累计 token 消耗
    pub tokens: u64,
    /// 累计耗时（毫秒）
    pub ms: u64,
}

impl EvalOutcome {
    pub fn success_rate(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.passed as f64 / self.total as f64
    }
}

/// 采纳判据（阈值由策略给出，可测）
///
/// 注意：本结构**不是 `Copy`** —— `protected_tasks` 是 `Vec<String>`。
/// 这是刻意的：红线集一旦被复制就可能出现两份不同的红线，宁可不给 `Copy`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromotionCriteria {
    pub min_success_rate: f64,
    pub max_token_growth: f64,
    pub max_latency_growth: f64,
    pub max_per_task_loss: usize,
    pub protected_tasks: Vec<String>,
}

impl From<&MetaPolicy> for PromotionCriteria {
    fn from(p: &MetaPolicy) -> Self {
        Self {
            min_success_rate: p.min_success_rate,
            max_token_growth: p.max_token_growth,
            max_latency_growth: p.max_latency_growth,
            max_per_task_loss: p.max_per_task_loss,
            protected_tasks: p.protected_tasks.clone(),
        }
    }
}

/// 单个任务在一次回放中的结果 —— 任务级门控的输入。
///
/// 用 `passed / total` 而不是 `bool`：真实回放里一个任务往往有多个回合
/// （Recursia 是每任务 50 个状态），丢 1 个和丢 20 个不是一回事。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskOutcome {
    pub id: String,
    /// 该任务通过的回合数
    pub passed: usize,
    /// 该任务的总回合数
    pub total: usize,
}

/// 采纳裁决
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "verdict", content = "detail")]
pub enum PromotionVerdict {
    /// 允许进入灰度
    Promote,
    /// 跑得不比基线好（含持平）
    RejectNoGain,
    /// 未达最低成功率
    RejectBelowThreshold { rate: f64, required: f64 },
    /// 出现回退（有用例从通过变失败）
    RejectRegression { lost: usize },
    /// 红线任务退化 —— 无论总分涨多少都拒
    RejectProtectedTask { task: String },
    /// 单任务损失超过上限（回归门控）
    RejectTaskRegression { task: String, lost: usize, limit: usize },
    /// 成本超涨
    RejectCostRegression { growth: f64, limit: f64 },
    /// 时延超涨
    RejectLatencyRegression { growth: f64, limit: f64 },
}

impl PromotionVerdict {
    pub fn is_promote(&self) -> bool {
        matches!(self, Self::Promote)
    }

    /// 面向人的一句话（本地化由前端做；此处是稳定的英文码 + 数值）
    pub fn code(&self) -> &'static str {
        match self {
            Self::Promote => "promote",
            Self::RejectNoGain => "reject_no_gain",
            Self::RejectBelowThreshold { .. } => "reject_below_threshold",
            Self::RejectRegression { .. } => "reject_regression",
            Self::RejectProtectedTask { .. } => "reject_protected_task",
            Self::RejectTaskRegression { .. } => "reject_task_regression",
            Self::RejectCostRegression { .. } => "reject_cost_regression",
            Self::RejectLatencyRegression { .. } => "reject_latency_regression",
        }
    }
}

/// 任务级回归闸：**先红线、再单任务、最后才轮到总分**。
///
/// 返回 `Some(verdict)` 表示已被否决；`None` 表示任务级通过、可继续走
/// [`evaluate_candidate`] 的聚合判定。
///
/// 为什么分成两步而不是一个函数：任务级只需要 [`TaskOutcome`]，聚合级需要
/// [`EvalOutcome`]，两者数据来源不同。合成一个会让调用方误以为
/// 「任务级通过 = 整体通过」—— 那正是这条闸要防的错。
///
/// 依据：Recursia 的公开案例里，总成功数 7/50 → 11/50，但某任务丢掉两个
/// 原本通过的状态，候选**仍被拒绝**。只看总分会把这个案例判成进步。
pub fn task_level_gate(
    baseline: &[TaskOutcome],
    candidate: &[TaskOutcome],
    criteria: &PromotionCriteria,
) -> Option<PromotionVerdict> {
    let cand: std::collections::HashMap<&str, usize> =
        candidate.iter().map(|t| (t.id.as_str(), t.passed)).collect();

    // 1) 红线任务：基线有成功、候选变少 → 直接拒，不看总分
    for id in &criteria.protected_tasks {
        let Some(base_passed) = baseline.iter().find(|t| &t.id == id).map(|t| t.passed) else {
            continue;
        };
        let now = cand.get(id.as_str()).copied().unwrap_or(0);
        if now < base_passed {
            return Some(PromotionVerdict::RejectProtectedTask { task: id.clone() });
        }
    }

    // 2) 单任务损失上限：逐任务比，而不是比总和
    for t in baseline {
        let now = cand.get(t.id.as_str()).copied().unwrap_or(0);
        let lost = t.passed.saturating_sub(now);
        if lost > criteria.max_per_task_loss {
            return Some(PromotionVerdict::RejectTaskRegression {
                task: t.id.clone(),
                lost,
                limit: criteria.max_per_task_loss,
            });
        }
    }
    None
}

/// 沙箱门禁：候选必须**同时**满足成功率、无回退、成本与时延四条。
///
/// 判据顺序有意从「硬失败」到「软代价」：先看有没有跑挂，再看贵不贵 ——
/// 这样用户看到的拒绝原因是**最先触发的那一条**，而不是笼统的「不通过」。
pub fn evaluate_candidate(
    baseline: &EvalOutcome,
    candidate: &EvalOutcome,
    criteria: &PromotionCriteria,
) -> PromotionVerdict {
    if candidate.total == 0 {
        return PromotionVerdict::RejectNoGain;
    }
    // 回退：通过的用例数比基线少（新增用例通过不算回退）
    if candidate.passed < baseline.passed {
        return PromotionVerdict::RejectRegression {
            lost: baseline.passed - candidate.passed,
        };
    }
    if candidate.passed <= baseline.passed {
        return PromotionVerdict::RejectNoGain;
    }
    let rate = candidate.success_rate();
    if rate < criteria.min_success_rate {
        return PromotionVerdict::RejectBelowThreshold {
            rate,
            required: criteria.min_success_rate,
        };
    }
    let token_growth = ratio(candidate.tokens, baseline.tokens);
    if token_growth > criteria.max_token_growth {
        return PromotionVerdict::RejectCostRegression {
            growth: token_growth,
            limit: criteria.max_token_growth,
        };
    }
    let latency_growth = ratio(candidate.ms, baseline.ms);
    if latency_growth > criteria.max_latency_growth {
        return PromotionVerdict::RejectLatencyRegression {
            growth: latency_growth,
            limit: criteria.max_latency_growth,
        };
    }
    PromotionVerdict::Promote
}

/// 涨幅比（基线为 0 时视为无约束，返回 1.0）
fn ratio(candidate: u64, baseline: u64) -> f64 {
    if baseline == 0 {
        return 1.0;
    }
    candidate as f64 / baseline as f64
}

// ───────────────────────────────────────────────────────────────────────────
// 第 5 层：版本闭环 —— 快照 / 灰度 / 回滚
// ───────────────────────────────────────────────────────────────────────────

/// 灰度阶段
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RolloutStage {
    /// 已产出提案，等待人工审核（路线 A 的默认落点）
    Proposed,
    /// 人工已批准，尚未灰度
    Approved,
    /// 灰度中
    Canary,
    /// 已全量
    Live,
    /// 已回滚
    RolledBack,
    /// 沙箱裁决未通过，未进入灰度
    Rejected,
}

impl RolloutStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Approved => "approved",
            Self::Canary => "canary",
            Self::Live => "live",
            Self::RolledBack => "rolled_back",
            Self::Rejected => "rejected",
        }
    }
}

/// 一个版本快照（每次提案/裁决/推进都追加一条，只增不改 —— 支持回滚到任一历史）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionSnapshot {
    /// 单调递增版本号（从 1 开始）
    pub version: u32,
    /// ISO-8601 时间戳
    pub at: String,
    pub stage: RolloutStage,
    pub cause: RootCause,
    pub plan: ImprovementPlan,
    /// 基线回放结果；提案阶段尚未回放时为 `None`（**「未测量」不等于「测得 0」**）
    pub baseline: Option<EvalOutcome>,
    /// 未评估时为 `None`（提案阶段）
    pub candidate: Option<EvalOutcome>,
    pub verdict: Option<PromotionVerdict>,
    /// 备注（人工意见 / 回滚原因）
    pub note: Option<String>,
}

/// 元台账：JSONL 落盘 + 历史读取 + 回滚目标
pub struct MetaLedger {
    path: PathBuf,
}

impl MetaLedger {
    /// 创建（`data_dir` 下建 `meta/` 子目录）
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self {
            path: data_dir.as_ref().join("meta").join(LEDGER_FILE),
        }
    }

    /// 台账文件路径
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 追加一条快照
    pub fn append(&self, snap: &VersionSnapshot) -> Result<()> {
        use std::io::Write;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let line = serde_json::to_string(snap)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{line}")?;
        Ok(())
    }

    /// 全部历史（按追加顺序）
    pub fn history(&self) -> Result<Vec<VersionSnapshot>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let raw = std::fs::read_to_string(&self.path)?;
        Ok(raw
            .lines()
            .filter_map(|l| serde_json::from_str::<VersionSnapshot>(l).ok())
            .collect())
    }

    /// 按版本折叠历史：一个版本取其**最后一条**阶段记录。
    ///
    /// 台账只追加不修改（可审计），因此「阶段推进」是同一版本的又一条记录；
    /// 判断某版本**现在**处于什么阶段，必须折叠后再看。返回顺序按版本首次出现顺序。
    pub fn folded(&self) -> Vec<VersionSnapshot> {
        let Ok(history) = self.history() else {
            return Vec::new();
        };
        let mut out: Vec<VersionSnapshot> = Vec::new();
        for snap in history {
            match out.iter_mut().find(|s| s.version == snap.version) {
                Some(slot) => *slot = snap,
                None => out.push(snap),
            }
        }
        out
    }

    /// 当前最新版本号（无历史为 0）。
    ///
    /// 取**最大**而非末条：回滚会为更早的版本追加记录，末条版本号未必最大。
    pub fn latest_version(&self) -> u32 {
        self.history()
            .map(|h| h.iter().map(|s| s.version).max().unwrap_or(0))
            .unwrap_or(0)
    }

    /// 处于灰度/全量的版本（供状态查询）。
    ///
    /// 折叠后取最后一个 Canary/Live：灰度版被回滚时会自动落回更早的全量版。
    pub fn active(&self) -> Option<VersionSnapshot> {
        self.folded()
            .into_iter()
            .rev()
            .find(|s| matches!(s.stage, RolloutStage::Canary | RolloutStage::Live))
    }

    /// 回滚的落点：一个**更早**且当前仍为 `Live` 的版本。
    ///
    /// 只认仍处于 Live 的版本（被回滚过的版本不再算数）；若从未有过更早的全量版本
    /// 则返回 `None` —— **不假装能回到一个不存在的版本**。
    pub fn rollback_target(&self) -> Option<VersionSnapshot> {
        let active = self.active()?;
        self.folded()
            .into_iter()
            .rev()
            .find(|s| s.stage == RolloutStage::Live && s.version < active.version)
    }
}

/// 一次自迭代循环的结果
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaCycleReport {
    /// 本轮止于哪一步
    pub stage: RolloutStage,
    /// 参与本轮裁决的信号数
    pub signals: usize,
    pub diagnosis: Option<Diagnosis>,
    pub plan: Option<ImprovementPlan>,
    pub verdict: Option<PromotionVerdict>,
    /// 本轮登记的版本号（未登记为 0）
    pub version: u32,
    /// 面向人的一句话
    pub message: String,
}

/// 元智能体：把五层串成一个可重放的循环
pub struct MetaAgent {
    policy: MetaPolicy,
    ledger: MetaLedger,
}

impl MetaAgent {
    pub fn new(data_dir: impl AsRef<Path>, policy: MetaPolicy) -> Self {
        Self {
            policy,
            ledger: MetaLedger::new(data_dir),
        }
    }

    pub fn policy(&self) -> &MetaPolicy {
        &self.policy
    }

    pub fn ledger(&self) -> &MetaLedger {
        &self.ledger
    }

    /// 跑一轮：监控 → 诊断 → 规划 → （有候选评估时）沙箱裁决 → 登记版本。
    ///
    /// - `signals`：监控层产出（[`extract_signals`] / [`observe_recent`]）；
    /// - `baseline`：当前基线的回放结果；`None` = 尚未回放（提案阶段允许）；
    /// - `candidate`：候选改动的回放结果；`None` = 尚未回放（例如仍需人工准备沙箱）。
    ///
    /// 有候选却**无基线** → 直接驳回：没有对照就没有裁决，不拿默认值凑一个「看起来通过」。
    ///
    /// 注意：无论 `auto_apply` 与否，本函数**都不会**去改任何真实文件 ——
    /// 它只登记提案与裁决。真正的应用由调用方在人工批准后执行（路线 A）。
    pub fn run_cycle(
        &mut self,
        signals: &[FailureSignal],
        baseline: Option<EvalOutcome>,
        candidate: Option<EvalOutcome>,
    ) -> MetaCycleReport {
        self.run_cycle_with_tasks(signals, baseline, candidate, &[], &[])
    }

    /// 带任务级回归门的循环 —— **推荐入口**。
    ///
    /// 与 [`Self::run_cycle`] 的唯一区别：在聚合裁决之前先过
    /// [`task_level_gate`]。传空切片时行为与 `run_cycle` **完全一致**。
    ///
    /// 为什么需要它：聚合 [`EvalOutcome`] 只记总通过数，表达不了
    /// 「总分涨、某任务崩」。要拦住这类候选，必须把每任务的
    /// [`TaskOutcome`] 一并交给裁决 —— 否则回归门形同虚设。
    pub fn run_cycle_with_tasks(
        &mut self,
        signals: &[FailureSignal],
        baseline: Option<EvalOutcome>,
        candidate: Option<EvalOutcome>,
        baseline_tasks: &[TaskOutcome],
        candidate_tasks: &[TaskOutcome],
    ) -> MetaCycleReport {
        self.run_cycle_gated(signals, baseline, candidate, baseline_tasks, candidate_tasks, None)
    }

    /// 带 Trust 锁的循环 —— 合并前强制链的推荐入口（路线图 §5.3 机制 1 + 3）。
    ///
    /// 与 [`Self::run_cycle_with_tasks`] 的唯一区别：当裁决**走向合并**
    /// （[`PromotionVerdict::Promote`]）时，还必须过 [`trust_gate`]（提案 →
    /// 变更集 → 回滚点 → 外生验证，缺一不可）与 [`budget_gate`]（改动半径上限）。
    /// 被拒的候选**不登记快照**（`version` 恒 0）—— 台账里绝不出现
    /// 「没有证据却已灰度」的版本。
    ///
    /// 其余阶段（等回放、被沙箱拒绝）不需要证据：不在错误阶段强要证据，
    /// 但凡要合并，一环都不能少。
    ///
    /// 参数确实偏多：这是对外暴露的完整合并入口（信号 + 前后评估 + 前后任务 +
    /// 提案 + 预算）。拆结构体会让所有现有调用方一起改签名，收益不抵风险，
    /// 因此在这里显式放行 `too_many_arguments`。
    #[allow(clippy::too_many_arguments)]
    pub fn run_cycle_with_trust(
        &mut self,
        signals: &[FailureSignal],
        baseline: Option<EvalOutcome>,
        candidate: Option<EvalOutcome>,
        baseline_tasks: &[TaskOutcome],
        candidate_tasks: &[TaskOutcome],
        proposal: Option<&ChangeProposal>,
        budget: &EditBudget,
    ) -> MetaCycleReport {
        let gate = move |plan: &ImprovementPlan| -> Result<(), String> {
            let Some(p) = proposal else {
                return Err(
                    "聚合裁决通过但未提交变更集提案：无 diff 留痕、无回滚点、无外生验证 —— 不允许进入合并（Trust 锁）"
                        .to_string(),
                );
            };
            let bv = budget_gate(std::slice::from_ref(p), budget);
            if !bv.is_within() {
                return Err(format!(
                    "编辑预算超限（{}）：max_files={}，max_proposals={}",
                    bv.code(),
                    budget.max_files,
                    budget.max_proposals
                ));
            }
            let tv = trust_gate(plan, p);
            if !tv.is_merge() {
                return Err(format!(
                    "Trust 锁未通过（{}）：提案 → 变更集 → 回滚点 → 外生验证，缺一不可",
                    tv.code()
                ));
            }
            Ok(())
        };
        self.run_cycle_gated(
            signals,
            baseline,
            candidate,
            baseline_tasks,
            candidate_tasks,
            Some(&gate),
        )
    }

    /// 循环主体；`gate` 在「裁决通过、快照登记之前」运行，返回 `Err` 则
    /// 以 [`RolloutStage::Rejected`] 收束且**不登记**。
    fn run_cycle_gated(
        &mut self,
        signals: &[FailureSignal],
        baseline: Option<EvalOutcome>,
        candidate: Option<EvalOutcome>,
        baseline_tasks: &[TaskOutcome],
        candidate_tasks: &[TaskOutcome],
        gate: Option<PlanGate<'_>>,
    ) -> MetaCycleReport {
        // 第 2 层：诊断
        let Some(diag) = diagnose(signals) else {
            return MetaCycleReport {
                stage: RolloutStage::Rejected,
                signals: 0,
                diagnosis: None,
                plan: None,
                verdict: None,
                version: 0,
                message: "观察窗口内没有失败信号，无需改进".to_string(),
            };
        };
        // 第 3 层：规划（白名单 + 风险闸）
        let Some(plan) = plan_improvement(&diag, &self.policy) else {
            return MetaCycleReport {
                stage: RolloutStage::Rejected,
                signals: diag.signals,
                message: format!(
                    "根因 {} 置信度 {:.2}：未达提案条件或所需风险（{}）超过策略上限（{}）",
                    diag.cause.as_str(),
                    diag.confidence,
                    RootCause::ToolScriptDefect.as_str(),
                    self.policy.max_risk.as_str()
                ),
                diagnosis: Some(diag),
                plan: None,
                verdict: None,
                version: 0,
            };
        };

        // 第 4 层：沙箱裁决（无候选则停在待回放；有候选无基线则拒绝裁决）
        let (stage, verdict, candidate_outcome) = match (candidate, baseline.as_ref()) {
            (None, _) => (RolloutStage::Proposed, None, None),
            (Some(_), None) => {
                return MetaCycleReport {
                    stage: RolloutStage::Rejected,
                    signals: diag.signals,
                    diagnosis: Some(diag),
                    plan: Some(plan),
                    verdict: None,
                    version: 0,
                    message: "缺少基线回放结果，无法裁决（未测量不等于测得 0）".to_string(),
                };
            }
            (Some(cand), Some(base)) => {
                let criteria = PromotionCriteria::from(&self.policy);
                // 任务级回归门先行：总分涨但单任务崩的候选，在这里就被拦下，
                // 不会走到「总分更高所以采纳」那一步。
                let v = match task_level_gate(baseline_tasks, candidate_tasks, &criteria) {
                    Some(reject) => reject,
                    None => evaluate_candidate(base, &cand, &criteria),
                };
                let stage = if v.is_promote() {
                    if self.policy.auto_apply {
                        RolloutStage::Canary
                    } else {
                        RolloutStage::Proposed
                    }
                } else {
                    RolloutStage::Rejected
                };
                (stage, Some(v), Some(cand))
            }
        };

        // 第 5 层：登记版本快照
        // Trust 锁只拦「走向合并」的裁决（Promote 之后、登记之前）：
        // 等回放的提案、被沙箱拒绝的候选照常登记归档 —— 但凡是 Promote，
        // 没有走完「提案 → 变更集 → 回滚点 → 外生验证」就**不留快照**，
        // 否则台账会出现一个没有证据却处于灰度/待审的版本。
        if verdict.as_ref().is_some_and(PromotionVerdict::is_promote) {
            if let Some(gate) = gate {
                if let Err(reason) = gate(&plan) {
                    return MetaCycleReport {
                        stage: RolloutStage::Rejected,
                        signals: diag.signals,
                        diagnosis: Some(diag),
                        plan: Some(plan),
                        verdict,
                        version: 0,
                        message: reason,
                    };
                }
            }
        }
        let version = self.ledger.latest_version() + 1;
        let snap = VersionSnapshot {
            version,
            at: now_iso(),
            stage,
            cause: diag.cause,
            plan: plan.clone(),
            baseline,
            candidate: candidate_outcome,
            verdict: verdict.clone(),
            note: None,
        };
        let registered = match self.ledger.append(&snap) {
            Ok(()) => version,
            Err(e) => {
                tracing::warn!(err = %e, "元台账落盘失败（本轮提案未持久化）");
                0
            }
        };

        let message = match (&verdict, stage) {
            (None, _) => format!(
                "根因 {}（置信度 {:.2}）→ 提案「改 {}」，等待沙箱回放",
                diag.cause.as_str(),
                diag.confidence,
                plan.target.as_str()
            ),
            (Some(v), RolloutStage::Rejected) => {
                format!("沙箱裁决未通过（{}），提案已归档", v.code())
            }
            (Some(_), RolloutStage::Canary) => {
                format!("沙箱裁决通过，已灰度（v{registered}，auto_apply 开启）")
            }
            (Some(_), _) => format!(
                "沙箱裁决通过 → 待人工审核（v{registered}，改 {}）",
                plan.target.as_str()
            ),
        };

        MetaCycleReport {
            stage,
            signals: diag.signals,
            diagnosis: Some(diag),
            plan: Some(plan),
            verdict,
            version: registered,
            message,
        }
    }

    /// 人工批准（路线 A 的审核节点）：把某版本从 `Proposed` 推进到 `Approved`。
    pub fn approve(&mut self, version: u32, note: Option<String>) -> Result<VersionSnapshot> {
        self.transition(version, RolloutStage::Approved, note)
    }

    /// 进入灰度
    pub fn promote_canary(&mut self, version: u32) -> Result<VersionSnapshot> {
        self.transition(version, RolloutStage::Canary, None)
    }

    /// 灰度转全量
    pub fn promote_live(&mut self, version: u32) -> Result<VersionSnapshot> {
        self.transition(version, RolloutStage::Live, None)
    }

    /// 回滚：把**当前正在灰度/全量**的版本标记为 `RolledBack`。
    ///
    /// 同一版本追加一条记录（不改历史，因此回滚本身也可审计）；回滚后
    /// [`MetaLedger::active`] 会自动落回更早的全量版本。
    pub fn rollback(&mut self, reason: &str) -> Result<VersionSnapshot> {
        let active = self
            .ledger
            .active()
            .ok_or_else(|| anyhow::anyhow!("没有正在灰度/全量的版本可回滚"))?;
        let fallback = match self.ledger.rollback_target() {
            Some(t) => format!("回落到 v{}", t.version),
            None => "无更早的全量版本，回落到内置基线".to_string(),
        };
        let snap = VersionSnapshot {
            version: active.version,
            at: now_iso(),
            stage: RolloutStage::RolledBack,
            cause: active.cause,
            plan: active.plan.clone(),
            baseline: active.baseline,
            candidate: active.candidate,
            verdict: active.verdict,
            note: Some(format!("{reason}（{fallback}）")),
        };
        self.ledger.append(&snap)?;
        Ok(snap)
    }

    /// 人工否决一个尚未生效的提案（`Proposed` / `Approved` → `Rejected`）
    pub fn reject(&mut self, version: u32, reason: &str) -> Result<VersionSnapshot> {
        self.transition(version, RolloutStage::Rejected, Some(reason.to_string()))
    }

    /// 状态：当前生效版本 + 待审提案数 + 历史规模
    pub fn status(&self) -> serde_json::Value {
        let records = self.ledger.history().unwrap_or_default();
        let versions = self.ledger.folded();
        let pending = versions
            .iter()
            .filter(|s| s.stage == RolloutStage::Proposed)
            .count();
        let active = self.ledger.active();
        serde_json::json!({
            "auto_apply": self.policy.auto_apply,
            "max_risk": self.policy.max_risk.as_str(),
            "min_success_rate": self.policy.min_success_rate,
            "max_token_growth": self.policy.max_token_growth,
            "max_latency_growth": self.policy.max_latency_growth,
            "pending_approvals": pending,
            "versions": versions.len(),
            "records": records.len(),
            "active_version": active.as_ref().map(|s| s.version),
            "active_stage": active.as_ref().map(|s| s.stage.as_str()),
            "active_target": active.as_ref().map(|s| s.plan.target.as_str()),
            "rollback_target": self.ledger.rollback_target().map(|s| s.version),
            "ledger": self.ledger.path().display().to_string(),
        })
    }

    fn transition(
        &mut self,
        version: u32,
        stage: RolloutStage,
        note: Option<String>,
    ) -> Result<VersionSnapshot> {
        // 折叠后再查：某版本的**当前**阶段，而不是它最初被登记时的阶段
        let source = self
            .ledger
            .folded()
            .into_iter()
            .find(|s| s.version == version)
            .ok_or_else(|| anyhow::anyhow!("版本 v{version} 不存在"))?;
        // 只允许从「待审/已批/灰度」向前推进，防止把已回滚版本重新拉活。
        let allowed = match stage {
            RolloutStage::Approved => source.stage == RolloutStage::Proposed,
            RolloutStage::Rejected => {
                matches!(source.stage, RolloutStage::Proposed | RolloutStage::Approved)
            }
            RolloutStage::Canary => {
                matches!(source.stage, RolloutStage::Proposed | RolloutStage::Approved)
            }
            RolloutStage::Live => source.stage == RolloutStage::Canary,
            _ => false,
        };
        if !allowed {
            anyhow::bail!(
                "不允许的推进：{} → {}（v{version}）",
                source.stage.as_str(),
                stage.as_str()
            );
        }
        // 阶段推进**沿用同一版本号**：版本是改动的身份，推进不是新改动。
        let next = VersionSnapshot {
            version: source.version,
            at: now_iso(),
            stage,
            cause: source.cause,
            plan: source.plan.clone(),
            baseline: source.baseline,
            candidate: source.candidate,
            verdict: source.verdict.clone(),
            note,
        };
        self.ledger.append(&next)?;
        Ok(next)
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mgt-meta-{tag}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ev(seq: u64, session: &str, kind: &str, detail: &str) -> BehaviorEvent {
        BehaviorEvent {
            seq,
            at_ms: 1_700_000_000_000 + seq,
            session: session.to_string(),
            kind: kind.to_string(),
            detail: detail.to_string(),
        }
    }

    // ── 第 1 层：监控 ───────────────────────────────────────────────────

    #[test]
    fn extract_signals_labels_tool_fault_and_self_fault() {
        let events = vec![
            ev(1, "s1", "user_message", "帮我抓一下这个页面"),
            ev(2, "s1", "tool_call", "web_fetch(url)"),
            ev(3, "s1", "tool_result", "web_fetch(url): error timeout after 30s"),
            ev(4, "s1", "aborted", "用户取消"),
        ];
        let sigs = extract_signals(&events);
        assert!(sigs
            .iter()
            .any(|s| s.kind == SignalKind::ToolFault && s.tool.as_deref() == Some("web_fetch")));
        assert!(sigs.iter().any(|s| s.kind == SignalKind::AgentSelf));
    }

    #[test]
    fn extract_signals_escalates_repeated_tool_failure_to_insufficient() {
        let mut events = vec![ev(1, "s1", "user_message", "解析表格")];
        for i in 0..TOOL_DEFECT_THRESHOLD {
            events.push(ev(2 + i as u64, "s1", "tool_result", "parse_xlsx(): error 库缺失"));
        }
        let sigs = extract_signals(&events);
        assert!(
            sigs.iter().any(|s| s.kind == SignalKind::ToolInsufficient),
            "同工具反复失败应升级为能力不足：{sigs:?}"
        );
    }

    #[test]
    fn extract_signals_flags_capability_gap_only_when_never_succeeded() {
        // 重复提出 + 无成功回复 → 能力缺口
        let failing = vec![
            ev(1, "s1", "user_message", "做一份竞品分析报告"),
            ev(2, "s1", "assistant_reply", "无法完成，缺少数据源"),
            ev(3, "s1", "user_message", "做一份竞品分析报告"),
        ];
        assert!(extract_signals(&failing)
            .iter()
            .any(|s| s.kind == SignalKind::CapabilityGap));

        // 重复提出但已有成功回复 → **不**误报能力缺口
        let healthy = vec![
            ev(1, "s2", "user_message", "做一份竞品分析报告"),
            ev(2, "s2", "assistant_reply", "报告已生成，产物写入 report.html"),
            ev(3, "s2", "user_message", "做一份竞品分析报告"),
        ];
        assert!(
            !extract_signals(&healthy)
                .iter()
                .any(|s| s.kind == SignalKind::CapabilityGap),
            "有成功产出时不应报能力缺口"
        );
    }

    #[test]
    fn extract_signals_is_empty_on_clean_window() {
        let events = vec![
            ev(1, "s1", "user_message", "你好"),
            ev(2, "s1", "assistant_reply", "你好，需要我做什么？"),
        ];
        assert!(extract_signals(&events).is_empty());
    }

    // ── 第 2 层：诊断 ───────────────────────────────────────────────────

    #[test]
    fn diagnose_prefers_capability_gap_over_tool_faults() {
        let sigs = vec![
            FailureSignal {
                kind: SignalKind::ToolFault,
                session: "s".into(),
                tool: Some("x".into()),
                detail: "x: error".into(),
                at_ms: 1,
            },
            FailureSignal {
                kind: SignalKind::ToolInsufficient,
                session: "s".into(),
                tool: Some("x".into()),
                detail: "x 反复失败".into(),
                at_ms: 2,
            },
            FailureSignal {
                kind: SignalKind::CapabilityGap,
                session: "s".into(),
                tool: None,
                detail: "需求重复 3 次且无成功产出".into(),
                at_ms: 3,
            },
        ];
        let d = diagnose(&sigs).expect("有信号应有诊断");
        assert_eq!(d.cause, RootCause::CapabilityMissing);
        assert!(!d.evidence.is_empty());
        assert!(d.confidence > 0.0 && d.confidence <= 0.95);
    }

    #[test]
    fn diagnose_splits_param_error_from_script_defect() {
        let param = vec![FailureSignal {
            kind: SignalKind::ToolFault,
            session: "s".into(),
            tool: Some("http".into()),
            detail: "http: invalid argument 'url' (missing required parameter)".into(),
            at_ms: 1,
        }];
        assert_eq!(diagnose(&param).unwrap().cause, RootCause::ToolParamWrong);

        let script = vec![FailureSignal {
            kind: SignalKind::ToolInsufficient,
            session: "s".into(),
            tool: Some("parse".into()),
            detail: "parse 反复失败".into(),
            at_ms: 1,
        }];
        assert_eq!(diagnose(&script).unwrap().cause, RootCause::ToolScriptDefect);
    }

    #[test]
    fn diagnose_returns_none_without_signals() {
        assert!(diagnose(&[]).is_none(), "无信号时不得编造根因");
    }

    // ── 第 3 层：规划（白名单与风险闸）──────────────────────────────────

    #[test]
    fn core_source_is_not_a_change_target() {
        // 结构护栏：白名单里不存在「改 core 源码」这一变体。
        let all = [
            ChangeTarget::SystemPrompt,
            ChangeTarget::TaskRules,
            ChangeTarget::ToolDefaultParams,
            ChangeTarget::ToolScript,
            ChangeTarget::KnowledgeBase,
            ChangeTarget::FewShot,
            ChangeTarget::ExpertChecklist,
            ChangeTarget::TeamSop,
            ChangeTarget::SwarmPolicy,
            ChangeTarget::MemoryWeight,
            ChangeTarget::SkillDefinition,
            ChangeTarget::RetrievalPolicy,
            ChangeTarget::EvalTaskSet,
        ];
        for t in all {
            let s = t.as_str();
            assert!(
                !s.contains("core") && !s.contains("source") && !s.contains("src"),
                "改动目标不得指向核心源码：{s}"
            );
        }
        assert_eq!(
            all.len(),
            13,
            "白名单为 13 类（6 原有 + 7 RSI 扩展），新增即需重新评审"
        );

        // 每条 as_str 必须唯一，否则台账里无法区分改动对象
        let mut seen = std::collections::HashSet::new();
        for t in all {
            assert!(seen.insert(t.as_str()), "改动目标字符串重复：{}", t.as_str());
        }
    }

    /// RSI 扩展的 7 类必须**真的**能被 `risk()` 分级，而不是默认落到某一档。
    #[test]
    fn rsi_targets_are_risk_classified() {
        // 量尺与可执行行为属高危；判断类属中危。任何一类被降级都要显式改测试。
        let high = [
            ChangeTarget::SkillDefinition,
            ChangeTarget::SwarmPolicy,
            ChangeTarget::EvalTaskSet,
        ];
        let medium = [
            ChangeTarget::ExpertChecklist,
            ChangeTarget::TeamSop,
            ChangeTarget::MemoryWeight,
            ChangeTarget::RetrievalPolicy,
        ];
        for t in high {
            assert_eq!(t.risk(), RiskLevel::High, "{} 应为 High", t.as_str());
        }
        for t in medium {
            assert_eq!(t.risk(), RiskLevel::Medium, "{} 应为 Medium", t.as_str());
        }
        // 默认策略上限是 Medium → 高危三类默认进不了提案
        let policy = MetaPolicy::default();
        for t in high {
            assert!(
                t.risk() > policy.max_risk,
                "{} 不应在默认策略下被自动提案",
                t.as_str()
            );
        }
    }

    #[test]
    fn plan_respects_risk_ceiling() {
        let diag = Diagnosis {
            cause: RootCause::ToolScriptDefect,
            evidence: vec!["parse 反复失败".into()],
            confidence: 0.8,
            signals: 3,
        };
        // 默认上限 Medium → 高危的 ToolScript 提案被拦下
        assert!(plan_improvement(&diag, &MetaPolicy::default()).is_none());
        // 显式放宽后才产出
        let loose = MetaPolicy {
            max_risk: RiskLevel::High,
            ..MetaPolicy::default()
        };
        let plan = plan_improvement(&diag, &loose).expect("放宽上限后应可提案");
        assert_eq!(plan.target, ChangeTarget::ToolScript);
        assert_eq!(plan.risk, RiskLevel::High);
    }

    // ── 第 4 层：任务级回归闸（Recursia 回归门控）────────────────────

    #[test]
    fn task_gate_rejects_when_total_up_but_one_task_down() {
        // Recursia 公开案例的等价构造：总通过数 13 → 17（涨），
        // 但 cup_on_plate 从 10 掉到 8（丢 2）。总分看着是进步，必须拒。
        let baseline = vec![
            TaskOutcome {
                id: "cup_on_plate".into(),
                passed: 10,
                total: 10,
            },
            TaskOutcome {
                id: "pudding_combo".into(),
                passed: 3,
                total: 10,
            },
        ];
        let candidate = vec![
            TaskOutcome {
                id: "cup_on_plate".into(),
                passed: 8,
                total: 10,
            },
            TaskOutcome {
                id: "pudding_combo".into(),
                passed: 9,
                total: 10,
            },
        ];
        let total_before: usize = baseline.iter().map(|t| t.passed).sum();
        let total_after: usize = candidate.iter().map(|t| t.passed).sum();
        assert!(total_after > total_before, "构造成立：总分确实上涨");

        let mut c = crit();
        c.max_per_task_loss = 0;
        match task_level_gate(&baseline, &candidate, &c) {
            Some(PromotionVerdict::RejectTaskRegression { task, lost, limit }) => {
                assert_eq!(task, "cup_on_plate");
                assert_eq!(lost, 2);
                assert_eq!(limit, 0);
            }
            other => panic!("应被任务级闸拒绝，实际：{other:?}"),
        }
    }

    #[test]
    fn task_gate_rejects_protected_task_even_within_loss_budget() {
        let baseline = vec![TaskOutcome {
            id: "checkout".into(),
            passed: 5,
            total: 5,
        }];
        let candidate = vec![TaskOutcome {
            id: "checkout".into(),
            passed: 4,
            total: 5,
        }];
        let mut c = crit();
        c.max_per_task_loss = 3; // 预算宽松到足以容忍
        c.protected_tasks = vec!["checkout".into()];
        let v = task_level_gate(&baseline, &candidate, &c);
        assert!(
            matches!(v, Some(PromotionVerdict::RejectProtectedTask { .. })),
            "红线任务退化必须被拒，实际：{v:?}"
        );
    }

    #[test]
    fn task_gate_passes_when_nothing_lost() {
        let baseline = vec![TaskOutcome {
            id: "a".into(),
            passed: 3,
            total: 5,
        }];
        let candidate = vec![TaskOutcome {
            id: "a".into(),
            passed: 4,
            total: 5,
        }];
        let c = crit();
        assert!(task_level_gate(&baseline, &candidate, &c).is_none());
    }

    #[test]
    fn task_gate_tolerates_loss_within_budget() {
        let baseline = vec![TaskOutcome {
            id: "a".into(),
            passed: 5,
            total: 10,
        }];
        let candidate = vec![TaskOutcome {
            id: "a".into(),
            passed: 3,
            total: 10,
        }];
        let mut c = crit();
        c.max_per_task_loss = 2;
        assert!(
            task_level_gate(&baseline, &candidate, &c).is_none(),
            "丢 2 恰好等于上限，应通过"
        );

        c.max_per_task_loss = 1;
        assert!(
            matches!(
                task_level_gate(&baseline, &candidate, &c),
                Some(PromotionVerdict::RejectTaskRegression { .. })
            ),
            "丢 2 超过上限 1，应拒绝"
        );
    }

    #[test]
    fn plan_refuses_low_confidence() {
        let diag = Diagnosis {
            cause: RootCause::PlanLogicWeak,
            evidence: vec![],
            confidence: 0.3,
            signals: 1,
        };
        assert!(plan_improvement(&diag, &MetaPolicy::default()).is_none());
    }

    // ── 第 4 层：沙箱门禁 ───────────────────────────────────────────────

    fn crit() -> PromotionCriteria {
        PromotionCriteria::from(&MetaPolicy::default())
    }

    #[test]
    fn gate_promotes_only_real_improvement_within_cost() {
        let base = EvalOutcome {
            passed: 8,
            total: 10,
            tokens: 1000,
            ms: 10_000,
        };
        let good = EvalOutcome {
            passed: 10,
            total: 10,
            tokens: 1100,
            ms: 11_000,
        };
        assert_eq!(evaluate_candidate(&base, &good, &crit()), PromotionVerdict::Promote);
    }

    #[test]
    fn gate_rejects_regression_below_threshold_cost_and_latency() {
        let base = EvalOutcome {
            passed: 8,
            total: 10,
            tokens: 1000,
            ms: 10_000,
        };
        // 回退
        let worse = EvalOutcome {
            passed: 6,
            total: 10,
            tokens: 900,
            ms: 9_000,
        };
        assert!(matches!(
            evaluate_candidate(&base, &worse, &crit()),
            PromotionVerdict::RejectRegression { lost: 2 }
        ));
        // 持平
        let same = EvalOutcome {
            passed: 8,
            total: 10,
            tokens: 1000,
            ms: 10_000,
        };
        assert_eq!(evaluate_candidate(&base, &same, &crit()), PromotionVerdict::RejectNoGain);
        // 未达成功率（新用例大量失败，但通过数仍 > 基线）
        let weak = EvalOutcome {
            passed: 9,
            total: 20,
            tokens: 1000,
            ms: 10_000,
        };
        assert!(matches!(
            evaluate_candidate(&base, &weak, &crit()),
            PromotionVerdict::RejectBelowThreshold { .. }
        ));
        // 成本超涨
        let pricey = EvalOutcome {
            passed: 10,
            total: 10,
            tokens: 5_000,
            ms: 10_000,
        };
        assert!(matches!(
            evaluate_candidate(&base, &pricey, &crit()),
            PromotionVerdict::RejectCostRegression { .. }
        ));
        // 时延超涨
        let slow = EvalOutcome {
            passed: 10,
            total: 10,
            tokens: 1_000,
            ms: 99_000,
        };
        assert!(matches!(
            evaluate_candidate(&base, &slow, &crit()),
            PromotionVerdict::RejectLatencyRegression { .. }
        ));
    }

    // ── 第 5 层：闭环端到端 ─────────────────────────────────────────────

    fn failing_signals() -> Vec<FailureSignal> {
        vec![
            FailureSignal {
                kind: SignalKind::AgentSelf,
                session: "s1".into(),
                tool: None,
                detail: "无法完成，缺少数据源".into(),
                at_ms: 1,
            },
            FailureSignal {
                kind: SignalKind::ToolFault,
                session: "s1".into(),
                tool: Some("web_fetch".into()),
                detail: "web_fetch: error".into(),
                at_ms: 2,
            },
        ]
    }

    #[test]
    fn cycle_with_tasks_rejects_aggregate_gain_when_a_task_regresses() {
        // 端到端：聚合指标变好（5/10 → 9/10），但 cup_on_plate 从 10 掉到 8。
        // 走 run_cycle_with_tasks 必须被拒；同样聚合数据走 run_cycle（无任务级）
        // 会被采纳 —— 这个对照证明任务级数据是**必需的**，不是装饰。
        let dir = tmp_dir("cycle_tasks");
        let mut agent = MetaAgent::new(&dir, MetaPolicy::default());
        let base = EvalOutcome {
            passed: 5,
            total: 10,
            tokens: 1000,
            ms: 5000,
        };
        let good = EvalOutcome {
            passed: 9,
            total: 10,
            tokens: 1050,
            ms: 5100,
        };

        let base_tasks = vec![
            TaskOutcome {
                id: "cup_on_plate".into(),
                passed: 10,
                total: 10,
            },
            TaskOutcome {
                id: "pudding_combo".into(),
                passed: 0,
                total: 10,
            },
        ];
        let cand_tasks = vec![
            TaskOutcome {
                id: "cup_on_plate".into(),
                passed: 8,
                total: 10,
            },
            TaskOutcome {
                id: "pudding_combo".into(),
                passed: 10,
                total: 10,
            },
        ];

        let r = agent.run_cycle_with_tasks(
            &failing_signals(),
            Some(base),
            Some(good),
            &base_tasks,
            &cand_tasks,
        );
        assert_eq!(r.stage, RolloutStage::Rejected);
        assert!(
            matches!(
                &r.verdict,
                Some(PromotionVerdict::RejectTaskRegression { .. })
            ),
            "应被任务级闸拒绝，实际：{:?}",
            r.verdict
        );

        // 对照：不给任务级数据，同样聚合指标会被采纳
        let mut ctrl = MetaAgent::new(tmp_dir("cycle_tasks_ctrl"), MetaPolicy::default());
        let r2 = ctrl.run_cycle(&failing_signals(), Some(base), Some(good));
        assert!(
            r2.verdict.as_ref().is_some_and(|v| v.is_promote()),
            "无任务级数据时应按聚合指标采纳，实际：{:?}",
            r2.verdict
        );
    }

    // ── Trust 锁接线（run_cycle_with_trust，§5.3 机制 1 + 3）────────────────

    use crate::harness::trust::{ChangeKind, ChangeProposal, ChangeRecord, EditBudget, VerificationRecord};

    /// 与 `plan_improvement` 对 PlanLogicWeak 产出的提案**逐字一致** ——
    /// trust_gate 的身份检查按全字段比较，差一个字就是 RejectPlanMismatch。
    fn matching_proposal() -> ChangeProposal {
        ChangeProposal {
            plan: ImprovementPlan {
                target: ChangeTarget::SystemPrompt,
                cause: RootCause::PlanLogicWeak,
                risk: RiskLevel::Low,
                mutation: "在系统提示词中补入失败驱动的拆解与校验规则（固定小节 + 结果自检）".to_string(),
                rationale: "信号集中在规划/产出侧，未指向具体工具参数或脚本".to_string(),
            },
            changes: vec![ChangeRecord {
                path: "skills/prompt/playbook.md".to_string(),
                kind: ChangeKind::Modified,
                diff: "- 旧规则：直接产出\n+ 新规则：先拆解再自检".to_string(),
            }],
            rollback: Some("ckpt-test".to_string()),
            verification: Some(VerificationRecord {
                command: "cargo test -p marguerite-core --lib harness::".to_string(),
                exit_code: 0,
                evidence: "test result: ok".to_string(),
            }),
        }
    }

    fn eval(passed: usize) -> EvalOutcome {
        EvalOutcome {
            passed,
            total: 10,
            tokens: 1000,
            ms: 5000,
        }
    }

    #[test]
    fn cycle_with_trust_rejects_promotion_without_proposal() {
        // 聚合裁决通过，但没带变更集提案 → 拒，且**不登记快照**。
        let mut agent = MetaAgent::new(tmp_dir("trust_no_proposal"), MetaPolicy::default());
        let r = agent.run_cycle_with_trust(
            &failing_signals(),
            Some(eval(5)),
            Some(eval(9)),
            &[],
            &[],
            None,
            &EditBudget::default(),
        );
        assert_eq!(r.stage, RolloutStage::Rejected);
        assert!(
            r.message.contains("变更集提案"),
            "拒绝原因必须说明缺什么：{}",
            r.message
        );
        assert_eq!(r.version, 0, "被 Trust 锁拒绝的候选不得登记快照");
        assert_eq!(
            agent.ledger().latest_version(),
            0,
            "台账里也不得出现「无证据却已待审/灰度」的版本"
        );
    }

    #[test]
    fn cycle_with_trust_rejects_promotion_without_verification() {
        let mut agent = MetaAgent::new(tmp_dir("trust_no_verify"), MetaPolicy::default());
        let mut p = matching_proposal();
        p.verification = None;
        let r = agent.run_cycle_with_trust(
            &failing_signals(),
            Some(eval(5)),
            Some(eval(9)),
            &[],
            &[],
            Some(&p),
            &EditBudget::default(),
        );
        assert_eq!(r.stage, RolloutStage::Rejected);
        assert!(
            r.message.contains("Trust 锁未通过"),
            "message: {}",
            r.message
        );
        assert_eq!(agent.ledger().latest_version(), 0);
    }

    #[test]
    fn cycle_with_trust_allows_full_chain() {
        let mut agent = MetaAgent::new(tmp_dir("trust_full"), MetaPolicy::default());
        let p = matching_proposal();
        let r = agent.run_cycle_with_trust(
            &failing_signals(),
            Some(eval(5)),
            Some(eval(9)),
            &[],
            &[],
            Some(&p),
            &EditBudget::default(),
        );
        assert_eq!(
            r.stage,
            RolloutStage::Proposed,
            "auto_apply=false：证据齐全也只进待审，不自动灰度"
        );
        assert!(r.verdict.as_ref().is_some_and(|v| v.is_promote()));
        assert_eq!(r.version, 1, "完整链路正常登记");
        assert_eq!(agent.ledger().latest_version(), 1);
    }

    #[test]
    fn cycle_with_trust_does_not_demand_evidence_before_replay() {
        // candidate=None（还没回放）阶段不许要变更集 —— 否则「提案 → 准备 →
        // 回放」的增量流程第一步就走不下去。Trust 锁只在 Promote 时介入。
        let mut agent = MetaAgent::new(tmp_dir("trust_wait"), MetaPolicy::default());
        let r = agent.run_cycle_with_trust(
            &failing_signals(),
            None,
            None,
            &[],
            &[],
            None,
            &EditBudget::default(),
        );
        assert_eq!(r.stage, RolloutStage::Proposed);
        assert!(r.verdict.is_none(), "等回放阶段没有裁决");
    }

    #[test]
    fn cycle_with_trust_budget_rejects_oversized_changeset() {
        let mut agent = MetaAgent::new(tmp_dir("trust_budget"), MetaPolicy::default());
        let mut p = matching_proposal();
        p.changes = (0..11)
            .map(|i| ChangeRecord {
                path: format!("skills/item-{i}.md"),
                kind: ChangeKind::Added,
                diff: "+ x".to_string(),
            })
            .collect();
        let r = agent.run_cycle_with_trust(
            &failing_signals(),
            Some(eval(5)),
            Some(eval(9)),
            &[],
            &[],
            Some(&p),
            &EditBudget::default(),
        );
        assert_eq!(r.stage, RolloutStage::Rejected);
        assert!(
            r.message.contains("编辑预算超限"),
            "改动半径超限必须被拦：{}",
            r.message
        );
        assert_eq!(agent.ledger().latest_version(), 0);
    }

    #[test]
    fn cycle_proposes_then_awaits_approval_by_default() {
        let dir = tmp_dir("cycle");
        let mut agent = MetaAgent::new(&dir, MetaPolicy::default());
        let base = EvalOutcome {
            passed: 5,
            total: 10,
            tokens: 1000,
            ms: 5000,
        };
        // 无候选 → 停在待回放
        let r1 = agent.run_cycle(&failing_signals(), Some(base), None);
        assert_eq!(r1.stage, RolloutStage::Proposed);
        assert!(r1.version > 0);
        assert!(r1.message.contains("等待沙箱回放"));

        // 有候选且通过 → 默认 auto_apply=false，仍是待审（不自动上线）
        let good = EvalOutcome {
            passed: 9,
            total: 10,
            tokens: 1050,
            ms: 5100,
        };
        let r2 = agent.run_cycle(&failing_signals(), Some(base), Some(good));
        assert_eq!(r2.stage, RolloutStage::Proposed);
        assert!(r2.verdict.as_ref().is_some_and(|v| v.is_promote()));
        assert!(r2.message.contains("待人工审核"));

        // 人工批准 → 灰度 → 全量
        let approved = agent.approve(r2.version, Some("同意先灰度".into())).unwrap();
        assert_eq!(approved.stage, RolloutStage::Approved);
        let canary = agent.promote_canary(approved.version).unwrap();
        assert_eq!(canary.stage, RolloutStage::Canary);
        let live = agent.promote_live(canary.version).unwrap();
        assert_eq!(live.stage, RolloutStage::Live);

        let st = agent.status();
        assert_eq!(st["active_stage"], "live");
        assert_eq!(st["active_version"], r2.version, "生效的是被推进的那个版本本身");
        assert_eq!(st["pending_approvals"], 1); // 第一条仍挂待审
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cycle_auto_apply_goes_straight_to_canary() {
        let dir = tmp_dir("auto");
        let policy = MetaPolicy {
            auto_apply: true,
            ..MetaPolicy::default()
        };
        let mut agent = MetaAgent::new(&dir, policy);
        let base = EvalOutcome {
            passed: 5,
            total: 10,
            tokens: 1000,
            ms: 5000,
        };
        let good = EvalOutcome {
            passed: 9,
            total: 10,
            tokens: 1000,
            ms: 5000,
        };
        let r = agent.run_cycle(&failing_signals(), Some(base), Some(good));
        assert_eq!(r.stage, RolloutStage::Canary, "auto_apply 开启才可跳过人工");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cycle_rejects_bad_candidate_and_records_reason() {
        let dir = tmp_dir("reject");
        let mut agent = MetaAgent::new(&dir, MetaPolicy::default());
        let base = EvalOutcome {
            passed: 8,
            total: 10,
            tokens: 1000,
            ms: 5000,
        };
        let bad = EvalOutcome {
            passed: 6,
            total: 10,
            tokens: 4000,
            ms: 5000,
        };
        let r = agent.run_cycle(&failing_signals(), Some(base), Some(bad));
        assert_eq!(r.stage, RolloutStage::Rejected);
        assert_eq!(
            r.verdict.as_ref().map(|v| v.code()),
            Some("reject_regression")
        );
        // 被拒的版本不得成为回滚目标
        assert!(agent.ledger().rollback_target().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rollback_requires_an_active_version() {
        let dir = tmp_dir("rollback");
        let mut agent = MetaAgent::new(&dir, MetaPolicy::default());
        let base = EvalOutcome {
            passed: 5,
            total: 10,
            tokens: 1000,
            ms: 5000,
        };
        // 从未生效过 → 回滚必须报错，不假装能回到不存在的版本
        assert!(agent.rollback("实验回滚").is_err());

        let good = EvalOutcome {
            passed: 9,
            total: 10,
            tokens: 1000,
            ms: 5000,
        };
        let r = agent.run_cycle(&failing_signals(), Some(base), Some(good));
        // 仅「已批准但未灰度」不算生效，仍不可回滚（应走人工否决）
        agent.approve(r.version, None).unwrap();
        assert!(agent.rollback("灰度指标回落").is_err());
        assert_eq!(
            agent.reject(r.version, "复测不达标").unwrap().stage,
            RolloutStage::Rejected
        );

        // 真正灰度后回滚成功，且记录里写明回落落点
        let r2 = agent.run_cycle(&failing_signals(), Some(base), Some(good));
        agent.promote_canary(r2.version).unwrap();
        let rb = agent.rollback("灰度指标回落").unwrap();
        assert_eq!(rb.stage, RolloutStage::RolledBack);
        assert_eq!(rb.version, r2.version, "回滚是同一版本的阶段变更");
        assert!(rb.note.unwrap().contains("灰度指标回落"));
        assert!(agent.status()["active_version"].is_null(), "无更早全量版则无生效版本");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rollback_of_canary_falls_back_to_earlier_live() {
        let dir = tmp_dir("rollback-fallback");
        let mut agent = MetaAgent::new(&dir, MetaPolicy::default());
        let base = EvalOutcome {
            passed: 5,
            total: 10,
            tokens: 1000,
            ms: 5000,
        };
        let good = EvalOutcome {
            passed: 9,
            total: 10,
            tokens: 1000,
            ms: 5000,
        };
        // v1 全量
        let v1 = agent.run_cycle(&failing_signals(), Some(base), Some(good));
        agent.promote_canary(v1.version).unwrap();
        agent.promote_live(v1.version).unwrap();
        // v2 灰度后回滚 → 生效版本自动落回 v1
        let v2 = agent.run_cycle(&failing_signals(), Some(base), Some(good));
        agent.promote_canary(v2.version).unwrap();
        assert_eq!(agent.status()["active_version"], v2.version);
        agent.rollback("新版本错误率上升").unwrap();
        let st = agent.status();
        assert_eq!(st["active_version"], v1.version, "回滚应落回更早的全量版本");
        assert_eq!(st["active_stage"], "live");
        assert_eq!(st["versions"], 2, "折叠后版本数不因阶段推进而膨胀");
        assert_eq!(st["pending_approvals"], 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn illegal_stage_transition_is_refused() {
        let dir = tmp_dir("illegal");
        let mut agent = MetaAgent::new(&dir, MetaPolicy::default());
        let base = EvalOutcome {
            passed: 5,
            total: 10,
            tokens: 1000,
            ms: 5000,
        };
        let good = EvalOutcome {
            passed: 9,
            total: 10,
            tokens: 1000,
            ms: 5000,
        };
        let r = agent.run_cycle(&failing_signals(), Some(base), Some(good));
        // Proposed 不能直接跳 Live（必须先 Canary）
        assert!(agent.promote_live(r.version).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_signals_yields_no_proposal() {
        let dir = tmp_dir("nosignal");
        let mut agent = MetaAgent::new(&dir, MetaPolicy::default());
        let r = agent.run_cycle(
            &[],
            Some(EvalOutcome {
                passed: 5,
                total: 10,
                tokens: 100,
                ms: 100,
            }),
            None,
        );
        assert!(r.plan.is_none());
        assert_eq!(r.signals, 0);
        assert_eq!(agent.ledger().latest_version(), 0, "无信号不应写台账");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 有候选但无基线 → 拒绝裁决，不拿默认值凑一个「看起来通过」。
    #[test]
    fn candidate_without_baseline_is_refused() {
        let dir = tmp_dir("no-baseline");
        let mut agent = MetaAgent::new(&dir, MetaPolicy::default());
        let good = EvalOutcome {
            passed: 9,
            total: 10,
            tokens: 1000,
            ms: 5000,
        };
        let r = agent.run_cycle(&failing_signals(), None, Some(good));
        assert_eq!(r.stage, RolloutStage::Rejected);
        assert!(r.verdict.is_none());
        assert_eq!(r.version, 0, "未裁决不应登记版本");
        assert!(r.message.contains("缺少基线"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 提案阶段的快照：基线如实为 `None`，不得伪造成「测得 0」。
    #[test]
    fn proposal_snapshot_keeps_baseline_unmeasured() {
        let dir = tmp_dir("unmeasured");
        let mut agent = MetaAgent::new(&dir, MetaPolicy::default());
        let r = agent.run_cycle(&failing_signals(), None, None);
        assert_eq!(r.stage, RolloutStage::Proposed);
        let snap = agent.ledger().folded().pop().expect("应有一条版本快照");
        assert!(snap.baseline.is_none());
        assert!(snap.candidate.is_none());
        // 而带真实回放时，基线必须原样落盘
        let base = EvalOutcome {
            passed: 5,
            total: 10,
            tokens: 1000,
            ms: 5000,
        };
        let _ = agent.run_cycle(&failing_signals(), Some(base), None);
        let last = agent.ledger().folded().pop().expect("应有第二条快照");
        assert_eq!(last.baseline.map(|b| b.passed), Some(5));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn success_rate_handles_empty_total() {
        let empty = EvalOutcome {
            passed: 0,
            total: 0,
            tokens: 0,
            ms: 0,
        };
        assert_eq!(empty.success_rate(), 0.0);
        assert_eq!(
            evaluate_candidate(&empty, &empty, &crit()),
            PromotionVerdict::RejectNoGain
        );
    }
}