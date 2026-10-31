//! 对话产线（Conversation Line）—— 把协作形状做成**可编译的规格**。
//!
//! ## 为什么存在
//!
//! 本仓的三处真机缺陷（DEF-10「并行规模选 3、实际只跑 1」、DEF-08 档位两处漂移连拒两次、
//! `orchestrate.rs` 里 `let _ = capabilities;`）经 5-Why 收敛到同一条根因：
//! **一轮协作的"形状"是两份手写的描述**（前端 `OrchestrationState` 一份、后端
//! `route_meta` 一份），描述会漂移，规格会被编译。本模块就是那份规格。
//!
//! 完整推导与契约见 `docs/plans/2026-10-30-conversation-line-workflow.md`。
//!
//! ## 三条设计定律（写代码时反复用到）
//!
//! 1. 形状必须是可编译的规格，不是可勾选的开关 ⇒ 本模块唯一的入口是 [`LineSpec`]。
//! 2. 对话的产物是可验收的断言链，不是文字流 ⇒ [`LineSpec::acceptance`] 逐条可判真。
//! 3. 先验必须出现在派单之前、复盘之后 ⇒ [`preflight`] 复用 `tactics::plan`，
//!    回写走 `gateway::line`（不在本模块，本模块保持**纯函数、零 LLM、离线**）。
//!
//! ## 分层
//!
//! 本模块不 import `gateway`（形状数学不该知道 RPC 长什么样）；`gateway::orchestrate` 反过来
//! 引用 [`LegacyForm`] 做兼容映射，`gateway::line` 负责把 [`LineSpec`] 变成真实调用。

use serde::{Deserialize, Serialize};

use crate::tactics::{self, Topology};

/// 一条产线最多几个阶段。
///
/// 3 不是审美数字：子智能体**嵌套深度恒等于 1**（`execute_subagent_task_spec` 不接
/// `SubAgentManager`），阶段串行是当前唯一可靠的"深度"来源；再多阶段的收益低于
/// 综合误差的复利。提高它之前先证明深度 2 能表示。
pub const MAX_STAGES: usize = 3;
/// 单阶段宽度上限：`swarm_role` 的静态命名到 16，超出后靠编号切面兜底，
/// 但再宽就已经越过"一次 human 审阅能看完"的量级。
pub const MAX_WIDTH: usize = 16;
/// 终态运行记录的保留条数（有界化纪律，同 `plan::RUN_HISTORY_KEEP`）。
pub const LINE_RUN_KEEP: usize = 100;
/// `line/` 目录的字节预算（W4 同一口径：新存储必须带生即有界）。
pub const LINE_DIR_BUDGET_BYTES: u64 = 32 * 1024 * 1024;
/// 阶段内每份工作的千字 token 数，取 `tactics::Scenario::from_params` 的同一个缺省值，
/// 于是「产线预演」与「战术页推演」用的是同一套工作量口径。
const DEFAULT_WORK_K: f64 = 40.0;

/// 产品名的形状。与 [`Topology`] 一比一：`Topology` 是数学名，这里是产品名。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinePattern {
    Solo,
    Fanout,
    Panel,
    Swarm,
    Relay,
}

impl LinePattern {
    pub const ALL: [LinePattern; 5] = [
        LinePattern::Solo,
        LinePattern::Fanout,
        LinePattern::Panel,
        LinePattern::Swarm,
        LinePattern::Relay,
    ];

    pub fn as_key(&self) -> &'static str {
        match self {
            LinePattern::Solo => "solo",
            LinePattern::Fanout => "fanout",
            LinePattern::Panel => "panel",
            LinePattern::Swarm => "swarm",
            LinePattern::Relay => "relay",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            LinePattern::Solo => "主智能体",
            LinePattern::Fanout => "子智能体并联",
            LinePattern::Panel => "专家团",
            LinePattern::Swarm => "蜂群",
            LinePattern::Relay => "接力链",
        }
    }

    /// 折算成推演用的拓扑名（`tactics` 那一侧的语言）。
    pub fn topology(&self) -> Topology {
        match self {
            LinePattern::Solo => Topology::Solo,
            LinePattern::Fanout => Topology::Star,
            LinePattern::Panel => Topology::Panel,
            LinePattern::Swarm => Topology::Swarm,
            LinePattern::Relay => Topology::Relay,
        }
    }

    /// 反方向映射。`Topology` 多出来的形状不存在，所以这里返回 `Option` 而不是 panic：
    /// 目录扩展时最先发现的是这里。
    pub fn from_topology(t: Topology) -> Option<Self> {
        match t {
            Topology::Solo => Some(LinePattern::Solo),
            Topology::Star => Some(LinePattern::Fanout),
            Topology::Panel => Some(LinePattern::Panel),
            Topology::Swarm => Some(LinePattern::Swarm),
            Topology::Relay => Some(LinePattern::Relay),
        }
    }

    /// 这一阶段是否天生需要"谁来收口"。并联与蜂群没有收口就是散会；面板与接力自带收口。
    pub fn needs_synthesis(&self) -> bool {
        matches!(self, LinePattern::Panel | LinePattern::Relay)
    }

    /// 名册里该填什么。这不是排版细节，而是**能不能解析**：
    /// `panel` 走 `team.run` 的 `teams` 参数（按团队 id 展开成蒸馏专家人设），
    /// `fanout`/`relay` 走子智能体的角色标签。填错就解析不到，静默降级成通用执行者。
    pub fn roster_kind(&self) -> &'static str {
        match self {
            LinePattern::Panel => "teams",
            LinePattern::Fanout | LinePattern::Relay => "roles",
            LinePattern::Solo | LinePattern::Swarm => "none",
        }
    }

    /// 一句话说明这个形状在赌什么（进 `line.catalog`，前端不复制这份文案）。
    pub fn blurb(&self) -> &'static str {
        match self {
            LinePattern::Solo => "一个人做完：没有分歧就没有协调成本。",
            LinePattern::Fanout => "并联取证：同一步要多种互相独立的证据，末端不综合。",
            LinePattern::Panel => "专家会诊：多个视角对同一件事下判断，由 lead 收口。",
            LinePattern::Swarm => "蜂群发散：把事实面铺满，切面互斥，先发散再收敛。",
            LinePattern::Relay => "接力：后一棒依赖前一棒的产物，顺序不可打乱。",
        }
    }
}

/// 旧五挡（`orchestrate.run` 的 `form`）。在本设计里降级为"一个阶段的简写"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyForm {
    Single,
    SubAgent,
    Team,
    Expert,
    Swarm,
}

impl LegacyForm {
    pub const ALL: [LegacyForm; 5] = [
        LegacyForm::Single,
        LegacyForm::SubAgent,
        LegacyForm::Team,
        LegacyForm::Expert,
        LegacyForm::Swarm,
    ];

    pub fn as_key(&self) -> &'static str {
        match self {
            LegacyForm::Single => "single",
            LegacyForm::SubAgent => "subagent",
            LegacyForm::Team => "team",
            LegacyForm::Expert => "expert",
            LegacyForm::Swarm => "swarm",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "single" => Some(LegacyForm::Single),
            "subagent" | "sub_agent" | "sub-agent" => Some(LegacyForm::SubAgent),
            "team" => Some(LegacyForm::Team),
            "expert" => Some(LegacyForm::Expert),
            "swarm" => Some(LegacyForm::Swarm),
            _ => None,
        }
    }

    /// 兼容表（设计文档 §2）——这张表是**可执行的**，见测试
    /// `legacy_forms_compile_to_the_documented_stage_sequences`。
    pub fn to_stages(&self) -> Vec<LinePattern> {
        match self {
            LegacyForm::Single => vec![LinePattern::Solo],
            // 子智能体挡位今天**不做** lead 综合（直接返回 summaries[]），
            // 所以这里如实给单个 fanout，不把它没做的事写成它做了。
            LegacyForm::SubAgent => vec![LinePattern::Fanout],
            LegacyForm::Team | LegacyForm::Expert => vec![LinePattern::Panel],
            LegacyForm::Swarm => vec![LinePattern::Swarm],
        }
    }
}

/// 判据。字段与 `tactics::Criteria` 对齐，但这里要能**反序列化**（前端会改它），
/// 所以不复用它，靠 [`LineCriteria::to_tactics`] 保证口径只有一处解释。
///
/// 预算阈值只有这一处（`budgetTokens` / `deadlineMs`）—— 曾经的 `LineSpec.budget`
/// 是第二个入口，它不喂给任何判据，于是"设了预算但没人执行"。一个阈值两个入口
/// 必然漂移，删掉不消费的那一个才是修。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineCriteria {
    pub max_fail_probability: f64,
    pub budget_tokens: Option<f64>,
    pub deadline_ms: Option<f64>,
}

impl Default for LineCriteria {
    fn default() -> Self {
        Self {
            // 与 `tactics::Criteria::default()` 同一个数字：两处不一致就会有两套裁决。
            max_fail_probability: 0.15,
            budget_tokens: None,
            deadline_ms: None,
        }
    }
}

impl LineCriteria {
    pub fn to_tactics(&self) -> tactics::Criteria {
        tactics::Criteria {
            max_fail_probability: self.max_fail_probability.clamp(0.0, 1.0),
            budget_tokens: self.budget_tokens.filter(|x| *x > 0.0),
            deadline_ms: self.deadline_ms.filter(|x| *x > 0.0),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineStage {
    pub id: String,
    pub pattern: LinePattern,
    /// solo 恒 1；fanout=角色数；panel=名册数；swarm=并行规模；relay=链长。
    pub width: usize,
    /// 名册内容随形状而变（见 [`LinePattern::roster_kind`]）：
    /// `panel` 填**团队 id**（`line.catalog.teams[].id`，走 `team.run` 的蒸馏人设），
    /// `fanout`/`relay` 填**角色标签**（走子智能体人设），`solo`/`swarm` 留空。
    /// 填错不会报错但会解析不到 ⇒ 静默降级成通用执行者，所以 `line.catalog` 必须给候选。
    pub roster: Vec<String>,
    /// 仅 swarm；空 ⇒ 由引擎按 `SWARM_FACETS` 轮转（切面表在引擎里，不在这里复制）。
    pub facets: Vec<String>,
    pub synthesize: bool,
    /// 人写的"为什么要这一段"，进运行记录供复盘（没有理由的阶段就是漂移的前兆）。
    pub note: String,
}

impl LineStage {
    pub fn new(id: &str, pattern: LinePattern) -> Self {
        let width = match pattern {
            LinePattern::Solo => 1,
            LinePattern::Swarm | LinePattern::Panel => 3,
            LinePattern::Fanout => 2,
            LinePattern::Relay => 3,
        };
        Self {
            id: id.to_string(),
            pattern,
            width,
            roster: Vec::new(),
            facets: Vec::new(),
            synthesize: pattern.needs_synthesis(),
            note: String::new(),
        }
    }
}

/// 一轮协作的完整规格：可编辑、可编译、可回溯，三者同一个对象。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LineSpec {
    pub spec_id: String,
    pub goal: String,
    /// 逐条可判真的验收条款（设计定律二的落点）。
    pub acceptance: Vec<String>,
    pub stages: Vec<LineStage>,
    pub criteria: LineCriteria,
}

impl Default for LineSpec {
    fn default() -> Self {
        Self {
            spec_id: String::new(),
            goal: String::new(),
            acceptance: Vec::new(),
            stages: vec![LineStage::new("s1", LinePattern::Solo)],
            criteria: LineCriteria::default(),
        }
    }
}

impl LineSpec {
    pub fn with_stages(spec_id: impl Into<String>, goal: impl Into<String>, stages: Vec<LineStage>) -> Self {
        Self {
            spec_id: spec_id.into(),
            goal: goal.into(),
            acceptance: Vec::new(),
            stages,
            criteria: LineCriteria::default(),
        }
    }

    /// 阶段串行，所以总工作量 = 各阶段之和；`work_k` 摊到每份工作上，
    /// 于是「换个形状」比较的是同一件事，而不是不同的事。
    fn scenario(&self) -> tactics::Scenario {
        tactics::Scenario {
            tasks: self.stage_tasks(),
            trials: tactics::model::DEFAULT_TRIALS,
            seed: 42,
            budget_tokens: self.criteria.budget_tokens,
            deadline_ms: self.criteria.deadline_ms,
            model: tactics::Model::default(),
        }
    }

    fn stage_tasks(&self) -> Vec<tactics::TacticalTask> {
        let mut tasks = Vec::new();
        let mut prev: Option<String> = None;
        for stage in &self.stages {
            let unit = DEFAULT_WORK_K;
            let ids: Vec<String> = match stage.pattern {
                LinePattern::Solo => vec![format!("{}.lead", stage.id)],
                other => (0..stage.width.max(1)).map(|i| format!("{}.{}", stage.id, i)).chain(
                    if other.needs_synthesis() || stage.synthesize {
                        vec![format!("{}.lead", stage.id)]
                    } else {
                        Vec::new()
                    },
                ).collect(),
            };
            let per = unit / ids.len().max(1).saturating_sub(if stage.synthesize || stage.pattern.needs_synthesis() { 1 } else { 0 }).max(1) as f64;
            match stage.pattern {
                LinePattern::Solo | LinePattern::Relay => {
                    for (i, id) in ids.iter().enumerate() {
                        let dep = if i == 0 { prev.clone() } else { Some(ids[i - 1].clone()) };
                        tasks.push(worked(id.clone(), per, dep));
                    }
                }
                LinePattern::Panel => {
                    let lead = ids.last().cloned().unwrap_or_default();
                    for id in &ids[..ids.len().saturating_sub(1)] {
                        tasks.push(worked(id.clone(), per, prev.clone()));
                    }
                    let members: Vec<String> = ids[..ids.len().saturating_sub(1)].to_vec();
                    tasks.push(led(lead, per, members, prev.clone()));
                }
                LinePattern::Fanout | LinePattern::Swarm => {
                    let has_lead = stage.synthesize;
                    let body_len = if has_lead { ids.len() - 1 } else { ids.len() };
                    for id in &ids[..body_len] {
                        tasks.push(worked(id.clone(), per, prev.clone()));
                    }
                    if has_lead {
                        let members: Vec<String> = ids[..body_len].to_vec();
                        tasks.push(led(ids.last().cloned().unwrap_or_default(), per, members, prev.clone()));
                    }
                }
            }
            prev = ids.last().cloned().or(prev);
        }
        tasks
    }
}

fn worked(id: String, work_k: f64, depends_on: Option<String>) -> tactics::TacticalTask {
    tactics::TacticalTask {
        id,
        work_k,
        depends_on: depends_on.into_iter().collect(),
    }
}

fn led(id: String, work_k: f64, members: Vec<String>, stage_dep: Option<String>) -> tactics::TacticalTask {
    let mut deps = members;
    if let Some(d) = stage_dep {
        deps.push(d);
    }
    tactics::TacticalTask {
        id,
        work_k,
        depends_on: deps,
    }
}

/// 校验结果。`errors` 拦下发送，`warnings` 只说人话——**排队不是失败**，
/// 但把「选 8 实际并发 5」藏起来就是 DEF-10 那一类缺陷。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationResult {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl ValidationResult {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

/// 按 §3 的约束表校验。`max_concurrent` 由调用方从 `budget_policy` 现场读，
/// 本模块不 import 它，免得把"策略常量"再抄一份。
pub fn validate(spec: &LineSpec, max_concurrent: usize) -> ValidationResult {
    let mut r = ValidationResult::default();
    if spec.goal.trim().is_empty() {
        r.errors.push("goal 不可为空：产线没有目标就退化成文字流".to_string());
    }
    if spec.stages.is_empty() {
        r.errors.push("至少需要一个阶段".to_string());
    }
    if spec.stages.len() > MAX_STAGES {
        r.errors.push(format!(
            "阶段数 {} 超过上限 {MAX_STAGES}（子智能体嵌套深度恒为 1，阶段串行是当前唯一的深度来源）",
            spec.stages.len()
        ));
    }
    let mut seen: Vec<&str> = Vec::new();
    for stage in &spec.stages {
        if seen.contains(&stage.id.as_str()) {
            r.errors.push(format!("阶段 id 重复：{}", stage.id));
        }
        seen.push(&stage.id);

        if stage.pattern == LinePattern::Solo {
            if stage.width != 1 {
                r.warnings.push(format!("{}：solo 阶段宽度恒为 1，已忽略 {}", stage.id, stage.width));
            }
            continue;
        }
        if stage.width == 0 {
            r.errors.push(format!("{}：宽度必须 ≥ 1", stage.id));
        }
        if stage.width > MAX_WIDTH {
            r.errors.push(format!("{}：宽度 {} 超过上限 {MAX_WIDTH}", stage.id, stage.width));
        }
        if matches!(stage.pattern, LinePattern::Panel | LinePattern::Relay) && stage.roster.is_empty() {
            r.errors.push(format!(
                "{}：{} 需要非空名册，否则会静默降级成单个通用执行者",
                stage.id,
                stage.pattern.label()
            ));
        }
        if stage.width > max_concurrent {
            r.warnings.push(format!(
                "{}：{} 个成员 > 并发上限 {max_concurrent}，多出的会排队（排队不是失败，但吞吐不会翻倍）",
                stage.id, stage.width
            ));
        }
    }
    r
}

/// 兼容表的可执行入口：旧挡位 + 旧旋钮 ⇒ 规格。
pub fn compile_legacy(
    spec_id: &str,
    goal: &str,
    form: LegacyForm,
    agents: &[String],
    teams: &[String],
    swarm_size: usize,
) -> LineSpec {
    let pattern = form
        .to_stages()
        .into_iter()
        .next()
        .unwrap_or(LinePattern::Solo);
    let mut stage = LineStage::new("s1", pattern);
    match pattern {
        LinePattern::Fanout => {
            stage.roster = if agents.is_empty() {
                vec!["orchestrator".to_string()]
            } else {
                agents.to_vec()
            };
            stage.width = stage.roster.len();
            // 如实：子智能体路径今天不做 lead 综合。
            stage.synthesize = false;
        }
        LinePattern::Panel => {
            stage.roster = if teams.is_empty() { agents.to_vec() } else { teams.to_vec() };
            stage.width = stage.roster.len().max(1);
        }
        LinePattern::Swarm => {
            stage.width = if swarm_size == 0 { 3 } else { swarm_size };
            stage.synthesize = false;
        }
        LinePattern::Solo | LinePattern::Relay => {}
    }
    LineSpec::with_stages(spec_id, goal, vec![stage])
}

/// 一条命名配方。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    pub id: &'static str,
    pub label: &'static str,
    pub stages: Vec<LineStage>,
}

fn stage(id: &str, pattern: LinePattern, width: usize, note: &str) -> LineStage {
    let mut s = LineStage::new(id, pattern);
    s.width = width;
    s.note = note.to_string();
    s
}

/// 带名册的阶段：`Panel` / `Relay` 没有名册就是非法形状（会静默降级成单个通用
/// 执行者），所以配方里这两类必须自带角色标签——名字全部取自本仓已有的角色词汇
/// （`gateway::team::EXPERT_ROSTERS` 与 `orchestration` 的蜂群切面），不另造一套。
fn cast(id: &str, pattern: LinePattern, roster: &[&str], note: &str) -> LineStage {
    let mut s = stage(id, pattern, roster.len(), note);
    s.roster = roster.iter().map(|r| r.to_string()).collect();
    s
}

/// 八条命名配方。前两条 = 兼容表里的挡位，后六条 = 本设计新增的**组合**能力
/// （旧契约里"蜂群 + 专家团"这种形状根本无法表示）。
pub fn recipes() -> Vec<Recipe> {
    vec![
        Recipe {
            id: "solo",
            label: "主智能体单跑",
            stages: vec![stage("s1", LinePattern::Solo, 1, "目标本身不需要多视角")],
        },
        Recipe {
            id: "fanout",
            label: "子智能体并联",
            stages: vec![stage("s2", LinePattern::Fanout, 3, "同一步要多种独立取证")],
        },
        Recipe {
            id: "hive_review",
            label: "蜂群发散 → 专家评审 → 主智能体定稿",
            stages: vec![
                stage("s1", LinePattern::Swarm, 6, "先把事实面铺满，别过早收敛"),
                cast(
                    "s2",
                    LinePattern::Panel,
                    &["legal_guard", "strategy_council", "fullstack_craft"],
                    "对发散结果做对抗性评审，不是鼓掌",
                ),
                stage("s3", LinePattern::Solo, 1, "只有一个签字人，责任不落空"),
            ],
        },
        Recipe {
            id: "panel_probe",
            label: "蜂群取证 → 专家团定案",
            stages: vec![
                stage("s1", LinePattern::Swarm, 4, "取证并行，切面互斥"),
                cast(
                    "s2",
                    LinePattern::Panel,
                    &["data_insight", "fullstack_craft"],
                    "证据到手后再让专家做判断",
                ),
            ],
        },
        Recipe {
            id: "relay_check",
            label: "接力实现 → 蜂群验证",
            stages: vec![
                cast(
                    "s1",
                    LinePattern::Relay,
                    &["架构设计", "编码实现", "测试验证"],
                    "实现有依赖顺序，只能接力",
                ),
                stage("s2", LinePattern::Swarm, 4, "验证之间无依赖，就该并行"),
            ],
        },
        Recipe {
            id: "two_view",
            label: "双专家并联 → 主智能体裁决",
            stages: vec![
                stage("s1", LinePattern::Fanout, 2, "两条独立路线，避免锚定"),
                stage("s2", LinePattern::Solo, 1, "把分歧摊在一个决策点上"),
            ],
        },
        Recipe {
            id: "council",
            label: "专家团 → 专家团复核",
            stages: vec![
                cast(
                    "s1",
                    LinePattern::Panel,
                    &["strategy_council"],
                    "第一团队给结论",
                ),
                cast(
                    "s2",
                    LinePattern::Panel,
                    &["legal_guard"],
                    "第二团队只负责推翻它",
                ),
            ],
        },
        Recipe {
            id: "swarm_of_panels",
            label: "蜂群 × 两路评审 × 综合",
            stages: vec![
                stage("s1", LinePattern::Swarm, 6, "发散"),
                cast(
                    "s2",
                    LinePattern::Panel,
                    &["legal_guard", "trae_fullstack_delivery"],
                    "风险视角评审",
                ),
                cast(
                    "s3",
                    LinePattern::Panel,
                    &["product_craft", "trae_design_to_code"],
                    "落地视角评审",
                ),
            ],
        },
    ]
}

/// 预演：逐阶段算「你选的形状值多少」，再整体算「推荐形状是谁、差多少」。
///
/// 两个问题不能合成一个：合并了就只剩"系统推荐"，人无从知道自己选的代价。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preflight {
    pub ok: bool,
    pub stages: Vec<tactics::TacticResult>,
    pub total: TotalEstimate,
    /// 整条产线的场景跑完整目录后的裁决（含 `whyNot`：为什么其它形状输了）。
    pub rationale: tactics::Rationale,
    pub premortem: Vec<tactics::PremortemItem>,
    /// 非空 ⇒ 这些数字没有实测支撑，前端必须显式说出来。
    pub unknowns: Vec<tactics::Unknown>,
    /// 人自选形状 vs 推荐形状的差距（产品此前没有这个对照）。
    pub chosen: Option<String>,
    pub chosen_vs_recommended: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TotalEstimate {
    pub p50_ms: f64,
    pub p90_ms: f64,
    pub p50_tokens: f64,
    pub p90_tokens: f64,
    pub p90_cost_usd: f64,
    /// 阶段串行 ⇒ 成功率**相乘**。取平均会把"三段各 90%"谎报成"整体 90%"（实为 72.9%）。
    pub success_rate: f64,
    pub fail_probability: f64,
}

impl Default for TotalEstimate {
    fn default() -> Self {
        Self {
            p50_ms: 0.0,
            p90_ms: 0.0,
            p50_tokens: 0.0,
            p90_tokens: 0.0,
            p90_cost_usd: 0.0,
            success_rate: 0.0,
            fail_probability: 1.0,
        }
    }
}

/// 在该拓扑的目录条目里挑一条最合适的实例：先要收口（带闸门）的那条，
/// 再按成功率；「有/无闸门各留一条」正是 `catalog()` 刻意的对照设计。
fn best_for_topology(catalog: &[tactics::Tactic], topology: Topology) -> Option<tactics::Tactic> {
    catalog.iter().filter(|t| t.topology == topology).max_by(|a, b| {
        let gate_a = a.gate.is_some();
        let gate_b = b.gate.is_some();
        gate_a.cmp(&gate_b).then_with(|| {
            a.width.max(1).cmp(&b.width.max(1))
        })
    })
    .copied()
}

pub fn preflight(spec: &LineSpec, max_concurrent: usize) -> Preflight {
    let v = validate(spec, max_concurrent);
    if !v.is_valid() {
        return Preflight {
            ok: false,
            stages: Vec::new(),
            total: TotalEstimate::default(),
            rationale: empty_rationale("规格校验未通过：先修形状，再谈成本".to_string()),
            premortem: Vec::new(),
            unknowns: Vec::new(),
            chosen: None,
            chosen_vs_recommended: None,
            error: Some(v.errors.join("；")),
        };
    }

    let catalog = tactics::catalog();
    let mut per_stage = Vec::with_capacity(spec.stages.len());
    for stage in &spec.stages {
        let single = tactics::Scenario {
            tasks: LineSpec::with_stages(&stage.id, &spec.goal, vec![stage.clone()]).stage_tasks(),
            trials: tactics::model::DEFAULT_TRIALS,
            seed: 42,
            budget_tokens: None,
            deadline_ms: None,
            model: tactics::Model::default(),
        };
        let Some(tactic) = best_for_topology(&catalog, stage.pattern.topology()) else {
            continue;
        };
        per_stage.push(tactics::simulate(&single, &tactic));
    }

    let mut total = TotalEstimate::default();
    let mut success_product = 1.0_f64;
    for r in &per_stage {
        total.p50_ms += r.p50_ms;
        total.p90_ms += r.p90_ms;
        total.p50_tokens += r.p50_tokens;
        total.p90_tokens += r.p90_tokens;
        total.p90_cost_usd += r.p90_cost_usd;
        success_product *= r.success_rate;
    }
    total.success_rate = success_product;
    total.fail_probability = (1.0 - success_product).clamp(0.0, 1.0);

    let scenario = spec.scenario();
    let plan = tactics::plan(&scenario, &spec.criteria.to_tactics());
    let chosen = per_stage
        .iter()
        .map(|r| r.tactic)
        .collect::<Vec<_>>()
        .join("+");
    let chosen_vs_recommended = match &plan.rationale.recommended {
        Some(rec) => Some(format!(
            "推荐 {}；你选的是 {chosen}。{}",
            rec, plan.rationale.reason
        )),
        None => None,
    };
    Preflight {
        ok: true,
        stages: per_stage,
        total,
        rationale: plan.rationale,
        premortem: plan.premortem,
        unknowns: plan.unknowns,
        chosen: Some(chosen),
        chosen_vs_recommended,
        error: None,
    }
}

fn empty_rationale(reason: String) -> tactics::Rationale {
    tactics::Rationale {
        recommended: None,
        no_viable_tactic: true,
        reason,
        verdicts: Vec::new(),
        why_not: Vec::new(),
    }
}

/// 把形状说成一句人话（轨道摘要用）。
pub fn describe(spec: &LineSpec) -> String {
    spec.stages
        .iter()
        .map(|s| {
            if s.pattern == LinePattern::Solo {
                s.pattern.label().to_string()
            } else {
                format!("{}×{}", s.pattern.label(), s.width)
            }
        })
        .collect::<Vec<_>>()
        .join(" → ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_of(patterns: &[LinePattern]) -> LineSpec {
        let stages = patterns
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let mut s = stage(
                    &format!("s{}", i + 1),
                    *p,
                    if *p == LinePattern::Solo { 1 } else { 2 },
                    "组合矩阵用例",
                );
                if matches!(p, LinePattern::Panel | LinePattern::Relay) {
                    s.roster = vec!["架构设计".into(), "编码实现".into()];
                }
                s
            })
            .collect();
        LineSpec::with_stages("ls-test", "测试目标", stages)
    }

    /// §2 的组合空间：5 + 5² + 5³ = 155 种形状全部可编译且预演非空。
    /// 这是「各种组合」的可执行证据，不是文档里的一句承诺。
    #[test]
    fn all_one_to_three_stage_shapes_compile() {
        let p = LinePattern::ALL;
        let mut checked = 0usize;
        for a in p {
            checked += 1;
            assert_shape(&spec_of(&[a]));
        }
        for a in p {
            for b in p {
                checked += 1;
                assert_shape(&spec_of(&[a, b]));
            }
        }
        for a in p {
            for b in p {
                for c in p {
                    checked += 1;
                    assert_shape(&spec_of(&[a, b, c]));
                }
            }
        }
        assert_eq!(checked, 155, "组合空间必须正好是 5 + 25 + 125");
    }

    fn assert_shape(spec: &LineSpec) {
        let pf = preflight(spec, 5);
        assert!(pf.ok, "{} 应可预演：{:?}", describe(spec), pf.error);
        assert!(
            !pf.stages.is_empty(),
            "{} 预演不能是空集",
            describe(spec)
        );
        assert!(pf.total.p90_ms > 0.0, "必须有耗时估计");
        assert!(
            (0.0..=1.0).contains(&pf.total.success_rate),
            "成功率必须在 0..1，实为 {}",
            pf.total.success_rate
        );
    }

    /// 兼容表（§2）可执行：五个旧挡位各自编译成文档写的那个阶段序列。
    #[test]
    fn legacy_forms_compile_to_the_documented_stage_sequences() {
        let cases = [
            (LegacyForm::Single, vec![LinePattern::Solo]),
            (LegacyForm::SubAgent, vec![LinePattern::Fanout]),
            (LegacyForm::Team, vec![LinePattern::Panel]),
            (LegacyForm::Expert, vec![LinePattern::Panel]),
            (LegacyForm::Swarm, vec![LinePattern::Swarm]),
        ];
        for (form, want) in cases {
            let spec = compile_legacy("ls-1", "目标", form, &["coder".into()], &["deck_craft".into()], 4);
            let got: Vec<LinePattern> = spec.stages.iter().map(|s| s.pattern).collect();
            assert_eq!(got, want, "{:?} 编译结果与兼容表不符", form);
        }
        // 子智能体不做综合，是引擎今天的事实，不是漏写。
        let sub = compile_legacy("ls-1", "目标", LegacyForm::SubAgent, &["coder".into()], &[], 0);
        assert!(!sub.stages[0].synthesize, "subagent 挡位必须如实 synthesize=false");
        // 蜂群规模一路传到底，否则又是一次 DEF-10。
        let swarm = compile_legacy("ls-1", "目标", LegacyForm::Swarm, &[], &[], 8);
        assert_eq!(swarm.stages[0].width, 8, "并行规模必须是 8");
    }

    #[test]
    fn pattern_and_topology_are_a_total_bijection() {
        for p in LinePattern::ALL {
            let back = LinePattern::from_topology(p.topology())
                .unwrap_or_else(|| panic!("{} 的拓扑反查失败", p.as_key()));
            assert_eq!(back, p, "拓扑映射必须可逆");
        }
        assert_eq!(LinePattern::ALL.len(), 5);
    }

    /// 每条错误都必须点名，否则用户无从修起（沿用 `tactics_rpc` 的报错纪律）。
    #[test]
    fn every_constraint_table_row_names_its_reason() {
        let empty_goal = LineSpec::with_stages("ls", "", vec![stage("s1", LinePattern::Solo, 1, "")]);
        assert!(validate(&empty_goal, 5).errors[0].contains("goal"));

        let too_many = spec_of(&[
            LinePattern::Solo,
            LinePattern::Solo,
            LinePattern::Solo,
            LinePattern::Solo,
        ]);
        let v = validate(&too_many, 5);
        assert!(v.errors.iter().any(|e| e.contains("阶段数") && e.contains("4")), "{:?}", v.errors);

        let zero_width = spec_of(&[LinePattern::Swarm]);
        let mut z = zero_width;
        z.stages[0].width = 0;
        assert!(validate(&z, 5).errors.iter().any(|e| e.contains("宽度")));

        let over_width = {
            let mut s = spec_of(&[LinePattern::Swarm]);
            s.stages[0].width = MAX_WIDTH + 1;
            s
        };
        assert!(validate(&over_width, 5).errors.iter().any(|e| e.contains("上限")));

        let empty_roster = {
            let mut s = spec_of(&[LinePattern::Panel]);
            s.stages[0].roster.clear();
            s
        };
        assert!(validate(&empty_roster, 5)
            .errors
            .iter()
            .any(|e| e.contains("名册") && e.contains("静默降级")));

        let duplicated_id = {
            let mut s = spec_of(&[LinePattern::Solo]);
            s.stages.push(stage("s1", LinePattern::Solo, 1, ""));
            s
        };
        assert!(validate(&duplicated_id, 5).errors.iter().any(|e| e.contains("重复")));
    }

    /// 排队不是错误，但必须说出来：这正是 DEF-10「选 3 跑 1」的解药。
    #[test]
    fn width_above_concurrency_is_a_warning_not_an_error() {
        let s = {
            let mut s = spec_of(&[LinePattern::Swarm]);
            s.stages[0].width = 8;
            s
        };
        let v = validate(&s, 5);
        assert!(v.is_valid(), "排队不该拦下发送：{:?}", v.errors);
        assert_eq!(v.warnings.len(), 1, "必须恰好一条警告");
        assert!(v.warnings[0].contains("排队") && v.warnings[0].contains('5'), "{}", v.warnings[0]);
    }

    #[test]
    fn preflight_refuses_an_invalid_shape_instead_of_guessing() {
        let mut s = spec_of(&[LinePattern::Panel]);
        s.stages[0].roster.clear();
        let pf = preflight(&s, 5);
        assert!(!pf.ok);
        assert!(pf.error.unwrap_or_default().contains("名册"));
        assert!(pf.rationale.no_viable_tactic, "不可行必须明说，不能矮子里拔将军");
    }

    #[test]
    fn preflight_is_reproducible_for_the_same_spec() {
        let s = spec_of(&[LinePattern::Swarm, LinePattern::Panel]);
        let a = serde_json::to_string(&preflight(&s, 5)).unwrap();
        let b = serde_json::to_string(&preflight(&s, 5)).unwrap();
        assert_eq!(a, b, "同规格必须同输出（推演是种子化 LCG）");
    }

    /// 汇总口径：串行 ⇒ 成功率相乘，绝不取平均。
    #[test]
    fn total_success_rate_multiplies_stages_instead_of_averaging() {
        let one = preflight(&spec_of(&[LinePattern::Solo]), 5);
        let two = preflight(&spec_of(&[LinePattern::Solo, LinePattern::Solo]), 5);
        let avg = (one.stages[0].success_rate + two.stages[1].success_rate) / 2.0;
        assert!(
            two.total.success_rate <= one.total.success_rate + f64::EPSILON,
            "加一个阶段不可能让整体更可靠"
        );
        assert!(
            (two.total.success_rate - avg).abs() > 1e-9 || avg == two.total.success_rate,
            "只有两段成功率完全相等时相乘才等于平均"
        );
        assert!(two.total.p90_ms > one.total.p90_ms, "耗时必须相加");
        assert!(two.total.p90_tokens > one.total.p90_tokens, "token 必须相加");
    }

    #[test]
    fn recipes_cover_genuine_compositions_and_all_validate() {
        let list = recipes();
        assert!(list.len() >= 8);
        let named = list.iter().find(|r| r.id == "hive_review").expect("旗舰配方必须在目录里");
        assert_eq!(
            named.stages.iter().map(|s| s.pattern.as_key()).collect::<Vec<_>>(),
            vec!["swarm", "panel", "solo"],
        );
        for r in &list {
            let spec = LineSpec::with_stages("ls-r", "目标", r.stages.clone());
            let v = validate(&spec, 5);
            // 配方必须自带名册（panel/relay 不给名册就是给自己埋雷）
            assert!(
                v.errors.iter().all(|e| !e.contains("名册")),
                "配方 {} 名册不全：{:?}",
                r.id,
                v.errors
            );
        }
    }

    #[test]
    fn criteria_defaults_match_tactics_so_the_two_verdicts_agree() {
        assert_eq!(
            LineCriteria::default().to_tactics().max_fail_probability,
            tactics::Criteria::default().max_fail_probability,
            "两处缺省判据不一致就会出现两次裁决不一致"
        );
    }

    #[test]
    fn spec_round_trips_through_the_wire_shape() {
        let s = spec_of(&[LinePattern::Swarm, LinePattern::Solo]);
        let raw = serde_json::to_value(&s).unwrap();
        assert!(raw["stages"][0]["pattern"].is_string());
        assert!(raw["criteria"]["maxFailProbability"].is_number());
        let back: LineSpec = serde_json::from_value(raw).unwrap();
        assert_eq!(back.stages.len(), 2);
        assert_eq!(describe(&back), describe(&s));
    }
}
