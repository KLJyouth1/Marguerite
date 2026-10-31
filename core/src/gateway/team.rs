//! 专家团队方法族（team.* / experts.* / agent.profile.*）
//!
//! 管理专家团队组合的持久化，执行专家团队一键启动（Team::run），
//! 以及用户画像分析与自动抽取。

use std::sync::Arc;

use serde_json::{json, Value};

use crate::events::{Envelope, EventBus, EventType};
use crate::gateway::_helpers::param_str;
use crate::persistence::config::Config;

/// 长任务台账的心跳超时阈值（秒）。
///
/// 与下面的心跳循环周期（[`SWARM_HEARTBEAT_TICK_S`]）配对：60s = 允许连续丢 4 次
/// 心跳才判 stale。它只影响**台账/看板**的心跳健康度，不参与整轮超时预算
/// （后者由 `budget_policy::team_run_timeout` 按成员数推导）。
const SWARM_HEARTBEAT_INTERVAL_S: i64 = 60;
/// 心跳循环周期（秒）。
const SWARM_HEARTBEAT_TICK_S: u64 = 15;

// ── 专家团队持久化组合（data_dir/team_compositions.json）──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, PartialEq)]
struct TeamCompositionMember {
    role: String,
    brief: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, PartialEq)]
struct TeamComposition {
    id: String,
    name: String,
    goal: String,
    members: Vec<TeamCompositionMember>,
}

fn team_compositions_path() -> std::path::PathBuf {
    Config::data_dir().join("team_compositions.json")
}

fn team_compositions_load() -> Vec<TeamComposition> {
    match std::fs::read_to_string(team_compositions_path()) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn team_compositions_save_all(items: &[TeamComposition]) {
    let p = team_compositions_path();
    let _ = std::fs::write(&p, serde_json::to_string_pretty(items).unwrap_or_default());
}

/// 专家团名册：id 与 `workbench/backend/marguerite_workbench/expert_teams.py` 的
/// `TEAM_DEFS` 对齐；角色标签收敛到 Rust 内建八人设（`role_table_for_label`），
/// 这样团队模式能复用蒸馏后的完整专家提示，而不是一句泛化的「你是专家」。
///
/// 为什么名册要落到 Rust：输入框选中的团队此前**根本没有传进执行层**
/// （`orchestrate.run` 只转发 goal），团队模式仍按关键词自动检测角色 ——
/// 目标里没有「代码/测试」时只跑 1 个通用执行者，两个不同专家团产出完全雷同。
const EXPERT_ROSTERS: &[(&str, &[(&str, &str)])] = &[
    (
        "deck_craft",
        &[
            ("文档", "内容策略与叙事结构：明确受众、主线与每页主张，给出页面大纲与讲稿要点。"),
            ("架构设计", "演示结构与信息层级：把大纲落成页序、每页版式与图表类型。"),
            ("重构优化", "视觉与版式统一：检查配色、字号、留白与图表一致性，列出修改清单。"),
        ],
    ),
    (
        "fullstack_craft",
        &[
            ("架构设计", "方案与接口设计：模块边界、数据模型、接口契约与失败路径。"),
            ("编码实现", "前后端实现：给出可直接合并的代码改动与文件清单。"),
            ("测试验证", "测试与验收：覆盖主路径、边界与回归，给出可复现命令。"),
            ("安全", "安全审计：鉴权、注入、越权、密钥与依赖风险评估。"),
        ],
    ),
    (
        "strategy_council",
        &[
            ("调研", "市场与竞争事实：规模、玩家、政策与可核证数据。"),
            ("架构设计", "战略路径与取舍：给出 2-3 条路线及各自代价与前提。"),
            ("文档", "决策备忘录：结论、证据、风险与下一步负责人。"),
        ],
    ),
    (
        "data_insight",
        &[
            ("调研", "数据口径与来源核验：指标定义、样本范围与偏差。"),
            ("编码实现", "分析与可视化脚本：可复现的查询/计算代码。"),
            ("测试验证", "口径校验：交叉验证关键数字，列出不一致项。"),
            ("文档", "结论报告：图表 + 解释 + 局限。"),
        ],
    ),
    (
        "legal_guard",
        &[
            ("调研", "法规与判例检索：适用条款、管辖与时效。"),
            ("安全", "合规风险评估：红线、灰区与缓释措施。"),
            ("文档", "合规意见书：结论分级（可做/需整改/不可做）与依据。"),
        ],
    ),
    (
        "invest_lab",
        &[
            ("调研", "基本面与行业事实：财务、竞争与周期位置。"),
            ("编码实现", "量化回测：数据、信号、成本与滑点假设。"),
            ("测试验证", "回测有效性：过拟合、幸存者偏差与样本外检验。"),
            ("文档", "投资备忘录：论点、估值区间、风险与跟踪指标。"),
        ],
    ),
    (
        "game_forge",
        &[
            ("架构设计", "玩法与系统设计：核心循环、数值与经济、关卡节奏。"),
            ("编码实现", "原型实现：可运行的最小玩法闭环与资产清单。"),
            ("测试验证", "可玩性验证：操作、反馈、难度曲线与崩溃路径。"),
            ("重构优化", "性能与手感：帧率、输入延迟与资源体积优化项。"),
        ],
    ),
    (
        "product_craft",
        &[
            ("调研", "用户与场景：目标用户、关键任务、现有替代方案。"),
            ("架构设计", "信息架构与流程：主路径、状态与异常分支。"),
            ("文档", "PRD：范围、验收标准、埋点与非目标。"),
            ("测试验证", "验收核对：逐条验收标准与可复现步骤。"),
        ],
    ),
    // TRAE 官方最佳实践蒸馏的三支团队：角色名对齐 `agent::expertise::TRAE_SPECIALISTS`，
    // 由 `roster_specs` 按 label 命中完整专家人设（含 mindset / playbook）。
    (
        "trae_fullstack_delivery",
        &[
            ("前端架构", "组件边界与数据流：目录结构、状态归属、懒加载与性能预算。"),
            ("后端架构", "接口契约与数据层：请求/响应/错误码、表结构与失败路径。"),
            ("API 测试", "契约与边界：有效/无效/边界三类输入、错误码一致性、可复现命令。"),
            ("DevOps", "发布与回滚：流水线阶段、灰度策略、监控阈值与运行手册。"),
        ],
    ),
    (
        "trae_design_to_code",
        &[
            ("UI 设计", "组件与状态：变体/状态矩阵、设计 token、窄屏折叠与可访问性。"),
            ("前端架构", "实现结构：token 落地、组件分层、响应式与交互状态。"),
            ("性能优化", "前后对比：首屏与交互延迟基准、瓶颈定位、优化验证数据。"),
        ],
    ),
    (
        "trae_ai_feature",
        &[
            ("AI 集成", "模型接入与编排：提示/工具最小集、评测集与通过门槛、成本与延迟。"),
            ("后端架构", "服务侧落地：调用链、限流重试、缓存与失败降级路径。"),
            ("合规审查", "数据与条款：收集类别、留存期限、第三方共享与风险分级。"),
        ],
    ),
];

fn expert_roster(id: &str) -> Option<&'static [(&'static str, &'static str)]> {
    EXPERT_ROSTERS
        .iter()
        .find(|(key, _)| *key == id)
        .map(|(_, roster)| *roster)
}

/// 内建专家团目录（`id` + 角色标签），供对话产线的名册候选使用。
///
/// 为什么导出而不是让产线自己列一份：名册一旦有两份，就会出现「UI 能选、执行不认识」
/// 的漂移（DEF-10 同族）。这里返回 `&'static str`，调用方不可能改写它。
pub fn builtin_rosters() -> Vec<(&'static str, Vec<&'static str>)> {
    EXPERT_ROSTERS
        .iter()
        .map(|(id, roster)| (*id, roster.iter().map(|(role, _)| *role).collect()))
        .collect()
}

/// 内建角色标签全集（去重、保序）——产线 `fanout`/`relay` 阶段的候选名册。
pub fn builtin_role_labels() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for entry in EXPERT_ROSTERS {
        for (role, _) in entry.1 {
            if !out.contains(role) {
                out.push(*role);
            }
        }
    }
    out
}

// ── 开工前澄清问卷（团队启动前；由 gateway/clarify.rs 装载进 QuestionQueue）──

/// deck_craft 开工问卷导语。必须如实写明「会员状态未自动探测」，
/// 否则用户会以为选项里的「可用/不可用」是引擎探测出来的事实。
const DECK_CRAFT_KICKOFF_INTRO: &str =
    "PPT 专家团队（deck_craft）开工前先对齐：生成方式（操控软件 / 脚本生成 / 其他自填）、\
     大纲确认与图片来源。选择「操控电脑」后会追问所用软件及其会员素材库可用性 ——\
     会员状态当前由引擎标记为未探测，请以你本机的真实情况回答，引擎不会替你假定。";

/// 会员/素材库探测的三态（诚实口径：探测到可用 / 探测到不可用 / 未探测）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembershipProbe {
    /// 探测到可用：素材库/模板/字体经感知层确认真实可达
    Available,
    /// 探测到不可用：确认无会员或素材库不可用
    Unavailable,
    /// 未探测：没有（或无法）完成自动探测
    NotProbed,
}

impl MembershipProbe {
    /// 机器可读值（写进问卷字段/回执，供后续接线时不改形状）
    pub fn as_str(self) -> &'static str {
        match self {
            MembershipProbe::Available => "available",
            MembershipProbe::Unavailable => "unavailable",
            MembershipProbe::NotProbed => "not_probed",
        }
    }

    /// 面向用户的文案 —— 直接进问题标题，避免「看起来像探测过」的错觉。
    pub fn label(self) -> &'static str {
        match self {
            MembershipProbe::Available => "探测到可用",
            MembershipProbe::Unavailable => "探测到不可用",
            MembershipProbe::NotProbed => "未探测",
        }
    }
}

/// 会员/素材库的当前探测态。
///
/// 诚实默认：自动探测需要 computer-use 感知层的多步 GUI 作业
/// （`tools::computer_use::dispatch` 先要求配置 VLMM，否则 VISION_UNAVAILABLE），
/// 而问卷开启这条 RPC 链路没有可同步完成的探测目标 —— 因此这里**不新写屏幕抓取、
/// 不假定任何软件有会员**，如实返回 `NotProbed`。接线探测属于执行期能力，见模块汇报。
pub fn current_membership_probe() -> MembershipProbe {
    MembershipProbe::NotProbed
}

fn choice(label: &str, description: &str) -> crate::mode::ask::QuestionOption {
    crate::mode::ask::QuestionOption {
        label: label.to_string(),
        description: Some(description.to_string()),
    }
}

/// 可复用问卷片段：图片来源（A 网络 / B 自动生成 / C 指定目录 + 「其他」自填走 allow_other）。
///
/// 复用落点：任何需要图源决策的团队都直接调用本函数，再按需覆盖 `depends_on`。
pub fn image_source_question() -> crate::mode::ask::Question {
    crate::mode::ask::Question {
        id: "deck_image_source".into(),
        header: Some("图片来源".into()),
        question: "PPT 使用的图片来源？".into(),
        options: vec![
            choice("网络获取图片", "联网检索/获取图片，需要出网且注意素材授权"),
            choice("自动生成图片", "由程序生成图表/示意，不外网取图"),
            choice("指定目录图片", "使用本机目录中的现有图片；请在「其他」补充目录路径"),
        ],
        multi_select: false,
        allow_other: true,
        depends_on: None,
    }
}

/// 可复用问卷片段：大纲确认（交付前必须对齐页面大纲）。
pub fn outline_confirmation_question() -> crate::mode::ask::Question {
    crate::mode::ask::Question {
        id: "deck_outline".into(),
        header: Some("大纲确认".into()),
        question: "页面大纲如何确认？团队成员会按这条回答直接落结构。".into(),
        options: vec![
            choice("按需求中的大纲执行", "需求描述里已给出大纲，原样使用"),
            choice("先生成大纲再逐项确认", "团队先出大纲草案，再回到这里复核"),
        ],
        multi_select: false,
        allow_other: true,
        depends_on: None,
    }
}

/// 团队专属开工问卷登记表：目前只有 deck_craft（首例）。
///
/// 返回 `(导语, 问题集)`；问题集带分支依赖，由 clarify.rs 装载进
/// [`crate::mode::ask::QuestionQueue`] 并按批派发卡。
pub fn kickoff_questionnaire_for(team_id: &str) -> Option<(&'static str, Vec<crate::mode::ask::Question>)> {
    match team_id {
        "deck_craft" => Some((DECK_CRAFT_KICKOFF_INTRO, deck_craft_kickoff_questions())),
        _ => None,
    }
}

fn deck_craft_kickoff_questions() -> Vec<crate::mode::ask::Question> {
    use crate::mode::ask::{Question, QuestionDependency};
    let probe = current_membership_probe();
    let gate_operate = QuestionDependency {
        question: "deck_form".into(),
        answer: Some("操控电脑".into()),
    };
    let gate_any_form = QuestionDependency {
        question: "deck_form".into(),
        answer: None,
    };
    vec![
        Question {
            id: "deck_form".into(),
            header: Some("生成方式".into()),
            question: "这份 PPT 希望怎样生成？".into(),
            options: vec![
                choice("操控电脑", "真实驱动本机安装的演示软件（如 WPS）生成页面"),
                choice("脚本生成", "由脚本/程序直接产出大纲与 PPT 结构，不驱动本机软件"),
            ],
            multi_select: false,
            // C 分支：自由描述生成方式（自定义答案不会解锁 A 分支专属追问，见 ready_batch 口径）
            allow_other: true,
            depends_on: None,
        },
        Question {
            id: "deck_software".into(),
            header: Some("演示软件".into()),
            question: "准备操控哪款演示软件？引擎不代为检测本机软件，请按实际填写。".into(),
            options: vec![
                choice("WPS", "WPS 演示（随 WPS Office 安装）"),
                choice("Microsoft Office", "PowerPoint"),
            ],
            multi_select: false,
            allow_other: true,
            depends_on: Some(gate_operate.clone()),
        },
        Question {
            id: "deck_membership".into(),
            header: Some(format!("会员与素材库 · {}", probe.label())),
            question: format!(
                "该软件的账号是否能使用会员素材库/模板/字体？当前引擎探测状态：{}。\
                     请勿默认「可用」，不确定就选「无法确定」，开工后以本机实际为准。",
                probe.label()
            ),
            options: vec![
                choice("可用", "确认账号为会员且素材库/模板/字体可用"),
                choice("不可用", "无会员或素材库不可用"),
                choice("无法确定", "开工后由成员在执行现场确认，勿凭空假定"),
            ],
            multi_select: false,
            allow_other: true,
            depends_on: Some(gate_operate),
        },
        Question {
            depends_on: Some(gate_any_form.clone()),
            ..outline_confirmation_question()
        },
        Question {
            depends_on: Some(gate_any_form),
            ..image_source_question()
        },
    ]
}

/// 读取调用方请求的并行规模（0 = 未指定，按自动检测）。
///
/// 这里是数量上限的**唯一来源**：不再 `.min(16)`。用户选多少就执行多少，
/// 超出物理并发时由 `SubAgentConfig::max_concurrent` 闸门排队（并发控制 ≠ 数量截断）。
fn requested_members(params: &Value) -> usize {
    params
        .get("members")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(0)
}

/// 解析调用方选择的团队 id → 合并后的名册。
///
/// 支持 `teams` 数组（多选）与旧的 `team` 单值；自定义组合按 id 或 name 匹配。
/// 多选时角色会重叠，按角色去重（同一角色只保留首次出现）；**不限制人数**
/// （旧的 `.truncate(8)` 会静默丢掉用户勾选的专家）。
fn selected_rosters(params: &Value) -> Vec<(String, String)> {
    let mut ids: Vec<String> = params
        .get("teams")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        if let Some(single) = params
            .get("team")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            ids.push(single.to_string());
        }
    }
    if ids.is_empty() {
        return Vec::new();
    }
    let custom = team_compositions_load();
    let mut out: Vec<(String, String)> = Vec::new();
    for id in ids {
        if let Some(c) = custom.iter().find(|c| c.id == id || c.name == id) {
            for m in &c.members {
                out.push((m.role.clone(), m.brief.clone()));
            }
            continue;
        }
        if let Some(roster) = expert_roster(&id) {
            for (role, brief) in roster {
                out.push(((*role).to_string(), (*brief).to_string()));
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|(role, _)| seen.insert(role.clone()));
    // 不再 `truncate(8)`：多选几个专家团是用户的显式选择，执行侧不静默砍成员。
    // 合并语义（同角色只保留首次出现）保留，避免同名专家在多团里重复占席位。
    out
}

/// `team.compositions.list`：已保存的专家团队组合。
pub fn team_compositions_list() -> Value {
    let items: Vec<Value> = team_compositions_load()
        .into_iter()
        .map(|c| {
            json!({
                "id": c.id,
                "name": c.name,
                "goal": c.goal,
                "members": c.members.iter().map(|m| json!({ "role": m.role, "brief": m.brief })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({ "ok": true, "items": items, "total": items.len() })
}

/// `team.compositions.save`：保存/覆盖团队组合（name 唯一，成员数组 [{role,brief}]）。
pub fn team_compositions_save(params: &Value) -> Value {
    let name = param_str(params, "name", "name", "").trim().to_string();
    if name.is_empty() {
        return json!({ "ok": false, "error": "team.compositions.save 需要 name" });
    }
    let goal = param_str(params, "goal", "goal", "").trim().to_string();
    let mut members = Vec::new();
    if let Some(arr) = params.get("members").and_then(Value::as_array) {
        for m in arr {
            let role = m
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let brief = m
                .get("brief")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if role.is_empty() {
                continue;
            }
            members.push(TeamCompositionMember { role, brief });
        }
    }
    let mut items = team_compositions_load();
    items.retain(|c| c.name != name);
    let id = format!("team-{}", name.replace(" ", "-"));
    items.push(TeamComposition {
        id,
        name,
        goal,
        members,
    });
    team_compositions_save_all(&items);
    team_compositions_list()
}

/// `team.compositions.remove`：删除团队组合。
pub fn team_compositions_remove(params: &Value) -> Value {
    let name = param_str(params, "name", "name", "").trim();
    if name.is_empty() {
        return json!({ "ok": false, "error": "team.compositions.remove 需要 name" });
    }
    let mut items = team_compositions_load();
    items.retain(|c| c.name != name);
    team_compositions_save_all(&items);
    team_compositions_list()
}

/// 团队 / 蜂群收尾写回项目进度文档（成员数 + 是否成功）。
///
/// 未绑定项目时回落默认工作区（见 [`crate::gateway::chat::project_doc_roots`]）；
/// 失败仅记日志，绝不影响编排结果。
fn write_team_progress(session: &str, members: usize, ok: bool) {
    // 单测不得写真实用户目录（与 chat 路径同一约定）。
    if cfg!(test) {
        return;
    }
    // 用 `project_doc_roots`（未绑定项目时回落默认工作区）：此前用仅返回绑定项目的
    // `session_project_roots`，桌面端默认（未绑定）时恒为空 → 规则静默失效。
    let roots = crate::gateway::chat::project_doc_roots(session);
    if roots.is_empty() {
        return;
    }
    let summary = format!(
        "团队/蜂群收尾：{members} 名成员，{}",
        if ok { "成功" } else { "失败" }
    );
    if let Err(e) =
        crate::context::project_docs::append_progress(&roots, session, "main", &summary)
    {
        tracing::warn!(err = %e, session = %session, "团队进度写回失败（不影响回合结果）");
    }
}

/// `team.run`：专家团队一键启动（Team::run 真实并行 spawn：自动角色检测 → 成员并行 → lead 综合）。
/// 未配置 API Key 时成员真实启动但 LLM 调用失败——如实回报错误，不做假成功。
pub async fn team_run(params: &Value) -> Value {
    team_run_with(params, None).await
}

/// 带事件总线的同步团队执行：成员 / 工具 / 综合阶段都会回流到 `sessionKey` 对话。
///
/// 为什么同步路径也要总线：`orchestrate.run` 的 team / expert 形态走的是这条路径，
/// 若只透传 goal，前端泳道永远空白 —— 与蜂群同样的黑盒问题。
pub async fn team_run_with(params: &Value, bus: Option<Arc<EventBus>>) -> Value {
    let goal = param_str(params, "goal", "goal", "");
    if goal.trim().is_empty() {
        return json!({ "ok": false, "error": "team.run 需要 goal（目标描述）" });
    }
    let session_key = param_str(params, "sessionKey", "sessionKey", "");
    let members = requested_members(params);
    // 用户选定的团队名册（可多选合并）；为空时退回自动角色检测。
    let roster = selected_rosters(params);
    // 卡片形态标签：expert 必须显示「专家团队」，不能因为走了同一条执行路径而降级成「团队」。
    let form_hint = param_str(params, "form", "form", "team");
    let cfg = effective_model_config();
    let mut orch = crate::orchestration::Orchestrator::new(cfg);
    let session = if session_key.trim().is_empty() {
        format!("ws.{}", uuid::Uuid::new_v4())
    } else {
        session_key.trim().to_string()
    };
    let bus_for_watch = bus.clone();
    if let Some(bus) = bus {
        orch.set_event_bus(bus, session.clone());
    }
    // 看门狗：成员任务 panic 时 `Team::run_p2p → wait_all` 会永远等下去
    // （完成信号发不出来），同步路径此前没有任何超时 —— 卡片与 HTTP 调用一起挂死。
    // 超时按目标体量 × 成员规模自适应（成员越多、分波越多，整轮预算越大；
    // 硬护栏仍是 2 小时，防止无界挂死），`MARGUERITE_TEAM_RUN_TIMEOUT_SECS` 可覆盖。
    let planned_members = roster.len().max(members);
    let run_timeout = crate::agent::budget_policy::team_run_timeout(goal, planned_members);
    let outcome = tokio::time::timeout(
        run_timeout,
        async {
            if roster.is_empty() {
                orch.dispatch_team_sized(goal, members).await
            } else {
                orch.dispatch_team_with_roster(goal, &roster, members, form_hint)
                    .await
            }
        },
    )
    .await;
    // 收尾写回项目进度（同步团队路径）：成员数 + 是否成功。
    let (team_ok, team_members) = match &outcome {
        Ok(Ok(out)) => (true, out.subagent_summaries.len()),
        _ => (false, 0),
    };
    write_team_progress(&session, team_members, team_ok);
    match outcome {
        Ok(Ok(out)) => json!({
            "ok": true,
            "strategy": out.strategy.label(),
            // 真实执行成员数（前端用它核对「所选规模」是否落地）。
            "members": out.subagent_summaries.len(),
            "memberCount": out.subagent_summaries.len(),
            "requestedMembers": members,
            "rosterSize": roster.len(),
            "summaries": out.subagent_summaries,
            "lead_reply": out.lead_reply,
        }),
        Ok(Err(e)) => json!({ "ok": false, "error": format!("团队编排失败: {e}") }),
        Err(_) => {
            let msg = format!("团队编排超时（{}s）", run_timeout.as_secs());
            if let (Some(bus), Some(run_id)) = (bus_for_watch, orch.last_run_id()) {
                bus.publish(Envelope::new(
                    EventType::Progress,
                    json!({
                        "kind": "orchestration",
                        "phase": "run_failed",
                        "runId": run_id,
                        "error": msg.clone(),
                    }),
                    session,
                ));
            }
            json!({ "ok": false, "error": msg, "timedOut": true })
        }
    }
}

/// 编排入口的配置必须与对话页**同源**：`apply_effective_model` 把 `models.json`
/// 的 active 档案叠加进 config。
///
/// 为什么不能直接用 `Config::load()`：两者会不一致（真机 2026-10-23 实测）。
/// `config.yaml` 里写 `provider=deepseek`，而 active 档案是本地 `ollama-omnipilot17`；
/// 密钥分槽只给 `MARGUERITE_API_KEY_DEEPSEEK_PRIMARY`，且归属标记不允许把它发给
/// 别的 provider —— 于是 `config.model.api_key` 为空，团队/蜂群的每一次模型调用
/// 都以 401 失败（`AUTH_FAILED`），而对话页因为先做了同样的叠加所以正常。
fn effective_model_config() -> Config {
    crate::model::selector::apply_effective_model(&Config::load().unwrap_or_default())
}

/// `team.run.background`：蜂群后台化——注册 long_running 台账（心跳/检查点/完成态），
/// 立即返回 taskId；执行体在后台 tokio 任务中运行 dispatch_team，结果写入检查点。
///
/// 无事件总线时保持旧行为（只写台账），供内部调用方使用；
/// 对话入口请用 [`team_run_background_with`] 带上总线与会话键。
pub async fn team_run_background(params: &Value) -> Value {
    team_run_background_with(params, None).await
}

/// 带事件总线的后台蜂群：执行过程与最终结论都会回流到 `sessionKey` 对应的对话。
///
/// 关键点（黑盒根因）：
/// 1. `Orchestrator` 必须 `set_event_bus`，否则子代理进度无处发布；
/// 2. 终态必须 `record_turn_persisted`，否则回复只存在于台账，用户刷新对话就看不到；
/// 3. `sessionKey` 必须由调用方传入，缺省才回退到临时 `ws.*`。
pub async fn team_run_background_with(
    params: &Value,
    bus: Option<Arc<EventBus>>,
) -> Value {
    let goal = param_str(params, "goal", "goal", "");
    if goal.trim().is_empty() {
        return json!({ "ok": false, "error": "team.run.background 需要 goal" });
    }
    let goal = goal.trim().to_string();
    // 用户选定的并行规模（0/1 = 按自动角色检测决定）。输入框的「并行规模」必须是
    // 真旋钮：此前它只改一句提示文案，后端仍按检测结果决定成员数。
    let members = requested_members(params);
    // 团队名册必须随后台任务一起移动：蜂群/团队多选同样要按名册执行。
    let roster_for_task = selected_rosters(params);
    let form_hint_for_task = param_str(params, "form", "form", "swarm").to_string();
    let session_key = param_str(params, "sessionKey", "sessionKey", "");
    let session = if session_key.trim().is_empty() {
        format!("ws.{}", uuid::Uuid::new_v4())
    } else {
        session_key.trim().to_string()
    };
    // 整轮看门狗用 `budget_policy::team_run_timeout`：按成员数分波放大
    // （env 可覆盖、有硬护栏），不再统一写死 1 小时。
    let run_timeout = crate::agent::budget_policy::team_run_timeout(
        &goal,
        roster_for_task.len().max(members),
    );
    // 台账登记是同步 SQLite 写（行数多时是毫秒级磁盘 IO），放到 blocking 线程池：
    // RPC dispatch 只有 2 个 worker（网关 slow runtime），请求路径上任何同步磁盘 IO
    // 都会把在线请求一起拖住，用户侧表现为「发送后没有任何反应」。
    let goal_for_reg = goal.clone();
    let task_id = match tokio::task::spawn_blocking(move || {
        let mut reg = crate::automation::long_running::global()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reg.register(
            &format!("[蜂群] {goal_for_reg}"),
            SWARM_HEARTBEAT_INTERVAL_S,
        )
    })
    .await
    {
        Ok(id) => id,
        Err(e) => {
            return json!({ "ok": false, "error": format!("长任务台账登记失败: {e}") });
        }
    };
    let goal2 = goal.clone();
    let tid = task_id.clone();
    // 心跳循环：任务存续期每 15s 打点（complete/fail 后 heartbeat 返回 Err → 自动退出）
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(
            SWARM_HEARTBEAT_TICK_S,
        ));
        loop {
            tick.tick().await;
            let mut reg = crate::automation::long_running::global()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if reg.heartbeat(&tid).is_err() {
                break;
            }
        }
    });
    let tid2 = task_id.clone();
    let session_for_task = session.clone();
    let bus_for_task = bus.clone();
    // 名册随后台任务 move 走；响应体还需要它的规模，先取一份长度。
    let roster_size_for_reply = roster_for_task.len();
    let handle = tokio::spawn(async move {
        // 与对话页同源解析生效模型（见 `effective_model_config` 的说明）：
        // 直接用 config.yaml 会拿不到 active 档案，团队成员的请求全部 401。
        let cfg = effective_model_config();
        let mut orch = crate::orchestration::Orchestrator::new(cfg);
        // 关键：把总线与会话键交给编排器，子代理的 member/tool 进度才有地方发布。
        if let Some(bus) = bus_for_task.clone() {
            orch.set_event_bus(bus, session_for_task.clone());
        }
        // 整轮预算已在登记台账时算好（按成员数分波放大），这里直接复用。
        let result = tokio::time::timeout(
            run_timeout,
            async {
                if roster_for_task.is_empty() {
                    orch.dispatch_team_sized(&goal2, members).await
                } else {
                    orch.dispatch_team_with_roster(
                        &goal2,
                        &roster_for_task,
                        members,
                        &form_hint_for_task,
                    )
                    .await
                }
            },
        )
        .await;

        // 收尾写回项目进度（后台蜂群路径）：成员数 + 是否成功。
        let (team_ok, team_members) = match &result {
            Ok(Ok(out)) => (true, out.subagent_summaries.len()),
            _ => (false, 0),
        };
        write_team_progress(&session_for_task, team_members, team_ok);

        // 先把最终答复写回对话（`record_turn_persisted` 同时落 context 台账与 sessions.db），
        // 再更新长任务台账：这样用户刷新页面也能在历史里看到蜂群结论，而不是只有一个 taskId。
        let checkpoint = match &result {
            Ok(Ok(out)) => {
                crate::gateway::chat::record_turn_persisted(
                    &session_for_task,
                    "assistant",
                    &out.lead_reply,
                );
                crate::gateway::chat::persist_session(&session_for_task);
                json!({
                    "lead_reply": out.lead_reply,
                    "summaries": out.subagent_summaries,
                    "strategy": out.strategy.label(),
                    // 真实执行规模（台账/对账可核对「8 路蜂群」是否真的跑了 8 个成员）。
                    "members": out.subagent_summaries.len(),
                    "memberCount": out.subagent_summaries.len(),
                    "requestedMembers": members,
                    "rosterSize": roster_for_task.len(),
                })
            }
            Ok(Err(e)) => json!({ "error": e.to_string() }),
            Err(_) => json!({
                "error": format!("后台蜂群执行超时（{}s）", run_timeout.as_secs()),
            }),
        };
        // 兜底终态帧复用编排 reporter 的真实 runId；未真正进入编排时退回 taskId（保持旧行为）。
        let run_id_for_fallback = orch.last_run_id().unwrap_or_else(|| tid2.clone());
        // 超时/调度失败时补一条终态事件：dispatch_team 内部只会覆盖「综合阶段」失败，
        // 外层超时没有机会发布，用户会看到泳道永远停在运行中。
        //
        // runId 必须复用编排 reporter 的真实 id（`orch-*`）。此前这里误用 long_running
        // 的 taskId —— 前端 `findRun(runId)` 永远匹配不上，**失败场景的卡片必然永久
        // 停在「运行中」**（真机 2026-10-24 复现）。taskId 作为附加字段保留，便于对账。
        // 用户主动取消时，`dispatch_team_sized` 已经发过 `run_failed("已取消")`；
        // 再补一条「蜂群执行失败」只会把用户看得懂的原因覆盖掉。
        if !matches!(&result, Ok(Ok(_))) && !orch.is_cancelled() {
            if let Some(bus) = bus_for_task {
                bus.publish(crate::events::Envelope::new(
                    crate::events::EventType::Progress,
                    json!({
                        "kind": "orchestration",
                        "phase": "run_failed",
                        "runId": run_id_for_fallback,
                        "taskId": tid2,
                        "error": checkpoint.get("error").cloned().unwrap_or(json!("蜂群执行失败")),
                    }),
                    session_for_task.clone(),
                ));
            }
        }
        let mut reg = crate::automation::long_running::global()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = reg.save_checkpoint(
            &tid2,
            crate::automation::long_running::Stage::Execute,
            checkpoint,
        );
        match result {
            Ok(Ok(_)) => {
                let _ = reg.complete(&tid2);
            }
            _ => {
                let _ = reg.fail(&tid2);
            }
        }
    });
    // 监工：后台任务 panic / 被 abort 时，闭包里的兜底终态帧与台账 fail **都不会执行**
    // （语句没机会跑到），前端泳道就永久停在「运行中」，连心跳都会打到超时才停。
    // 这里单独盯 JoinHandle，异常退出时补一条 run_failed 并收口台账。
    let bus_for_watch = bus.clone();
    let session_for_watch = session.clone();
    let tid_for_watch = task_id.clone();
    tokio::spawn(async move {
        let Err(join_err) = handle.await else {
            return;
        };
        // 任务已 panic，`Orchestrator` 随闭包一起销毁，只能从总线历史里找回本轮 runId。
        let run_id = bus_for_watch
            .as_ref()
            .and_then(|b| latest_orchestration_run_id(b, &session_for_watch))
            .unwrap_or_else(|| tid_for_watch.clone());
        if let Some(bus) = bus_for_watch {
            bus.publish(Envelope::new(
                EventType::Progress,
                json!({
                    "kind": "orchestration",
                    "phase": "run_failed",
                    "runId": run_id,
                    "taskId": tid_for_watch,
                    "error": format!("后台任务异常终止: {join_err}"),
                }),
                session_for_watch,
            ));
        }
        let mut reg = crate::automation::long_running::global()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = reg.fail(&tid_for_watch);
    });
    json!({
        "ok": true,
        "taskId": task_id,
        "sessionKey": session,
        "requestedMembers": members,
        "rosterSize": roster_size_for_reply,
        // 后台任务还在跑，真实成员数由编排终态帧 / 检查点回报；这里只回显请求规模。
        "memberCount": Value::Null,
        "message": "蜂群已后台启动（long_running 台账可见心跳/检查点/完成态）"
    })
}

/// 从总线历史里取该会话**最后一条**编排帧的 runId。
///
/// 只给后台任务的监工用：任务 panic 后 `Orchestrator` 已随闭包销毁，
/// 拿不到 `last_run_id()`；而 `run_started` 早已发布在总线里，可据此找回
/// 前端认领用的 id（否则兜底 `run_failed` 只能用 taskId，卡片永远对不上）。
fn latest_orchestration_run_id(bus: &EventBus, session: &str) -> Option<String> {
    // 与 `orchestration.status` 同口径：整段缓冲 + 墙钟过滤，不用 `Instant` 减法
    // （Windows 上开机不足窗口时长时会下溢成空集）。
    let cutoff = chrono::Utc::now() - chrono::Duration::hours(6);
    bus.history_all()
        .into_iter()
        .filter(|env| env.ts >= cutoff && env.r#type == EventType::Progress && env.session == session)
        .filter_map(|env| {
            let obj = env.payload.as_object()?;
            if obj.get("kind")?.as_str()? != "orchestration" {
                return None;
            }
            obj.get("runId")?.as_str().map(str::to_string)
        })
        .next_back()
}

/// `experts.list`：列出行业领军人经验蒸馏的专家档案库。
pub fn experts_list(_params: &Value) -> Value {
    let experts: Vec<Value> = crate::agent::expertise::experts_list()
        .into_iter()
        .map(|e| {
            json!({
                "id": e.id,
                "label": e.label,
                "lineage": e.lineage,
                "mindset": e.mindset,
                "expertise": e.expertise,
                "skills": e.skills,
                "playbook": e.playbook,
            })
        })
        .collect();
    json!({ "experts": experts })
}

/// `agent.profile.analyze`：从对话历史确定性抽取用户画像。
pub fn agent_profile_analyze(params: &Value) -> Value {
    let Some(messages) = params.get("messages").and_then(Value::as_array) else {
        return json!({ "error": "missing 'messages'", "code": "invalid_params" });
    };
    let history: Vec<crate::domain::Message> = messages
        .iter()
        .filter_map(|m| {
            let role = match m.get("role").and_then(Value::as_str).unwrap_or("") {
                "user" => crate::domain::Role::User,
                "assistant" => crate::domain::Role::Assistant,
                "system" => crate::domain::Role::System,
                _ => return None,
            };
            let content = m.get("content").and_then(Value::as_str).unwrap_or("");
            Some(crate::domain::Message {
                role,
                content: content.to_string(),
                meta: None,
            })
        })
        .collect();
    let persona = crate::agent::profile::auto_profile(&history);
    json!({
        "role": persona.role,
        "tone": persona.tone,
        "expertise": persona.expertise,
    })
}

/// `agent.profile.auto`：自动从记忆与会话历史中抽取用户画像，无需手动粘贴。
pub fn agent_profile_auto() -> Value {
    let mut all_entries = Vec::new();
    // 从上下文台账取最近会话的条目
    for cs in crate::context::session::context_sessions()
        .into_iter()
        .take(20)
    {
        let entries = crate::context::session::context_entries(&cs.session);
        all_entries.extend(entries.into_iter().take(20));
    }
    if all_entries.is_empty() {
        return json!({
            "role": "通用用户",
            "tone": "自然",
            "expertise": "暂无足够数据生成画像，多对话后自动更新",
            "auto": true,
            "source_count": 0,
        });
    }
    let history: Vec<crate::domain::Message> = all_entries
        .iter()
        .map(|e| {
            let role = match e.role.as_str() {
                "user" => crate::domain::Role::User,
                "assistant" => crate::domain::Role::Assistant,
                _ => crate::domain::Role::User,
            };
            crate::domain::Message {
                role,
                content: e.text.clone(),
                meta: None,
            }
        })
        .collect();
    let persona = crate::agent::profile::auto_profile(&history);
    json!({
        "role": persona.role,
        "tone": persona.tone,
        "expertise": persona.expertise,
        "auto": true,
        "source_count": all_entries.len(),
    })
}

#[cfg(test)]
mod roster_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merges_multiple_expert_rosters_and_dedupes_roles() {
        let roster = selected_rosters(&json!({
            "teams": ["fullstack_craft", "legal_guard"],
        }));
        let roles: Vec<&str> = roster.iter().map(|(role, _)| role.as_str()).collect();
        // fullstack 4 人 + legal 3 人，其中「安全」重复 → 6 人。
        assert_eq!(roles.len(), 6, "实际名册：{roles:?}");
        assert_eq!(roles[0], "架构设计");
        assert!(roles.contains(&"调研"));
        assert!(!roles.contains(&"生产环境"));
        assert_eq!(
            roles.iter().filter(|role| **role == "安全").count(),
            1,
            "同一角色只能出现一次：{roles:?}"
        );
    }

    #[test]
    fn accepts_legacy_single_team_and_ignores_unknown_ids() {
        let roster = selected_rosters(&json!({ "team": "deck_craft" }));
        assert_eq!(roster.len(), 3);
        assert_eq!(roster[0].0, "文档");

        let empty = selected_rosters(&json!({ "teams": ["no-such-team"] }));
        assert!(empty.is_empty());
    }
}

/// deck_craft 开工问卷的形状与分支流（纯 Rust，离线可测；不触网不抓屏）。
#[cfg(test)]
mod kickoff_questionnaire_tests {
    use super::*;
    use crate::mode::ask::{Answer, QuestionQueue};

    fn ids(qs: &[crate::mode::ask::Question]) -> Vec<String> {
        qs.iter().map(|q| q.id.clone()).collect()
    }

    #[test]
    fn deck_craft_questionnaire_has_the_product_shape() {
        let (intro, qs) = kickoff_questionnaire_for("deck_craft").expect("deck_craft 应有开工问卷");
        assert!(intro.contains("未探测"), "导语必须如实声明会员探测未接线: {intro}");
        assert_eq!(
            ids(&qs),
            vec!["deck_form", "deck_software", "deck_membership", "deck_outline", "deck_image_source"]
        );
        let form = &qs[0];
        assert!(form.allow_other, "C 分支（其他自填）靠 allow_other 表达");
        assert_eq!(
            form.depends_on, None,
            "gate 本身必须无条件就绪"
        );
        // 分支表达：软件与会员追问仅在「操控电脑」后出现
        use crate::mode::ask::QuestionDependency;
        assert_eq!(
            qs[1].depends_on,
            Some(QuestionDependency {
                question: "deck_form".into(),
                answer: Some("操控电脑".into())
            })
        );
        assert_eq!(qs[2].depends_on.as_ref().map(|d| d.question.as_str()), Some("deck_form"));
        // 大纲与图片来源：两条分支都要确认（gate 已答即就绪，不限选项）
        assert_eq!(
            qs[3].depends_on,
            Some(QuestionDependency { question: "deck_form".into(), answer: None })
        );
        assert_eq!(
            qs[4].depends_on,
            Some(QuestionDependency { question: "deck_form".into(), answer: None })
        );
        // 图片来源 = 三选项 + 其他自填（第四路），与产品口径一致
        let labels: Vec<&str> = qs[4].options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels, vec!["网络获取图片", "自动生成图片", "指定目录图片"]);
        assert!(qs[4].allow_other, "图片来源必须保留「其他-自填」入口");
        // 会员题面：探测态如实写进标题/题面，绝无「WPS 有会员」式假定
        assert!(qs[2].header.as_deref().unwrap_or("").contains("未探测"));
        assert!(qs[2].question.contains("未探测"));
        assert!(
            kickoff_questionnaire_for("fullstack_craft").is_none(),
            "目前只有 deck_craft 首例；其它团队不得被凭空弹出问卷"
        );
    }

    #[test]
    fn computer_branch_asks_software_and_membership() {
        let (_, qs) = kickoff_questionnaire_for("deck_craft").unwrap();
        let mut queue = QuestionQueue::new();
        queue.ask_many(qs);
        assert_eq!(ids(&queue.ready_batch()), vec!["deck_form"]);
        assert!(queue.answer(Answer {
            id: "deck_form".into(),
            selected: vec!["操控电脑".into()],
            custom: None,
        }));
        let stage2 = ids(&queue.ready_batch());
        assert_eq!(
            stage2,
            vec!["deck_software", "deck_membership", "deck_outline", "deck_image_source"],
            "A 分支追问软件与会员，且仍要确认大纲/图源"
        );
    }

    #[test]
    fn script_branch_completes_without_computer_questions() {
        let (_, qs) = kickoff_questionnaire_for("deck_craft").unwrap();
        let mut queue = QuestionQueue::new();
        queue.ask_many(qs);
        queue.answer(Answer {
            id: "deck_form".into(),
            selected: vec!["脚本生成".into()],
            custom: None,
        });
        assert_eq!(
            ids(&queue.ready_batch()),
            vec!["deck_outline", "deck_image_source"],
            "B 分支不得追问软件/会员（不可达题被丢弃而非伪造回答）"
        );
        queue.answer(Answer {
            id: "deck_outline".into(),
            selected: vec!["按需求中的大纲执行".into()],
            custom: None,
        });
        queue.answer(Answer {
            id: "deck_image_source".into(),
            selected: vec![],
            custom: Some("用我自己的图库".into()),
        });
        assert!(queue.is_empty());
    }
}
