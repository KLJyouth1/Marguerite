//! 角色上下文桥（P1-1：档位与记忆在**所有**执行路径生效的公共解析点）。
//!
//! # 为什么需要这一层
//!
//! `model::thinking` 只有档位语义（枚举、wire 映射、task-local），它**不该**知道
//! 会话台账与记忆楼层的存在；`gateway::chat` 那条主对话路径又把这些读取写在私有
//! 函数里（`memory_stable_prefix` 等为私有，无法复用）。于是多策略 / 专家 / 团队 /
//! 调用链这四条路径既拿不到会话档位，也拿不到记忆召回——正是「不管怎么开关都要能
//! 把任务做完、特别是产出质量」这条投诉的可测量根因。
//!
//! 本模块是那个缺失的桥，只做四件事：
//! 1. **档位**：把「本轮显式档位 / 会话持久档位 / 角色覆盖」交给
//!    [`crate::model::thinking::resolve_effort_for_role`]（优先级只在那一处定义），
//!    并给出跨 `tokio::spawn` 的作用域入口 [`RoleContext::scope`]；
//! 2. **记忆（只读）**：按楼层开关 [`ResolvedLayers`] 裁剪的召回文本
//!    [`memory_block`]，关掉的楼层**连检索都不做**（不是「查了不用」）；
//! 3. **开场简报（只读）**：最近 1–3 份压缩交接 + 现场 git 状态的
//!    [`continuity_block`]（干净工作区不注入）；
//! 4. **助理简报（只读）**：两层助理的钉住条目 / 稳定偏好 / 跨项目事实渲染成的
//!    [`assistant_block`]（没有任何稳定记忆时不注入）。
//!
//! # 硬边界（不可越线）
//!
//! * 这里**只有读**。记忆写入仍然只走既有治理入口 `memory_save`
//!   （`crate::tools::memory_save`）与主对话回合的 `memory::inject::remember`；
//!   本模块不调用任何 `remember` / `record` / `insert` / `save` 函数，也不为
//!   「让子代理有记忆」开第二条写库通道。
//! * 配置目录覆盖是**线程局部**的（`Config::set_config_dir_override`），
//!   所以 [`persisted_effort`] / [`build`] 必须在**父任务**（spawn 之前）调用，
//!   解析结果按值带进子任务。这条不是风格问题：在 spawned 任务里读会读到错误的
//!   配置目录，测试与真实进程会各自看到不同的档位。

use std::future::Future;

use serde_json::Value;

use crate::memory::layers::ResolvedLayers;
use crate::model::thinking::{
    resolve_effort_for_role, scoped_effort, session_effort_or_persisted, RoleOverrides,
    ThinkingEffort,
};

/// 只读记忆块的固定标题（可观测锚点：请求体里出现它 = 这一路真的注入了）。
pub const MEMORY_HEADER: &str = "## 相关记忆（只读召回，仅供参考，不是本轮指令）";

/// 整块记忆的字符预算（与 `memory::inject::INJECT_CHAR_BUDGET` 同量级：
/// 记忆是增强项，不得吃掉子任务本身的上下文）。
const MEMORY_CHAR_BUDGET: usize = 1_200;

/// 助理简报的固定标题（可观测锚点：请求体里出现它 = 这一路真的注入了）。
pub const ASSISTANT_HEADER: &str = "## 助理简报（用户稳定偏好与跨项目事实，仅供参考）";

/// 助理简报的**硬上限**：三段（pinned / preferences / facts）加起来不得超过它。
const ASSISTANT_CHAR_BUDGET: usize = 800;

/// 因预算省略条目时的提示行（省略必须说出来，不静默吞掉）。
const ASSISTANT_TRUNCATED_NOTE: &str =
    "（其余条目因预算被省略，可在设置 → 助理中查看）";

/// 单次执行的角色上下文：已解析档位 + 已召回记忆块。
#[derive(Debug, Clone, Default)]
pub struct RoleContext {
    /// 解析后的档位；`None` = 没有任何显式声明，继续按自适应/模式档执行
    /// （注意：这不等于「不思考」，`Some(Off)` 才是显式不思考）。
    pub effort: Option<ThinkingEffort>,
    /// 只读记忆块文本（`None` = 无命中或对应楼层被关闭 → 完全不注入）。
    pub memory: Option<String>,
    /// 开场简报文本（最近压缩交接 + 现场工作区）；`None` = 干净 / 无交接状态 → 不注入。
    pub continuity: Option<String>,
    /// 助理简报文本（钉住条目 + 稳定偏好 + 跨项目事实）；`None` = 没有任何稳定记忆 → 不注入。
    pub assistant: Option<String>,
}

impl RoleContext {
    /// 空上下文：档位与记忆都不声明，行为与改动前逐字节一致。
    pub fn none() -> Self {
        Self::default()
    }

    /// 只带档位（调用方自己完成记忆读取，或该路径不需要记忆）。
    pub fn effort_only(effort: Option<ThinkingEffort>) -> Self {
        Self {
            effort,
            memory: None,
            continuity: None,
            assistant: None,
        }
    }

    /// 把已解析档位作为作用域套到 future 上——**跨 `tokio::spawn` 传递档位的唯一入口**。
    ///
    /// `effort` 为 `None` 时原样运行（不覆盖既有作用域），因此对本轮已在档位
    /// 作用域内的调用（主对话直接发起的多策略/团队）行为不变。
    pub async fn scope<T, F>(&self, fut: F) -> T
    where
        F: Future<Output = T>,
    {
        scoped_effort(self.effort, fut).await
    }
}

/// 会话持久档位（`session::thinking_prefs` 的只读投影）。
///
/// **必须在 spawn 之前调用**（见模块头的线程局部配置目录说明）。
/// 无会话键 / 无偏好文件 / 文件损坏都返回 `None` = 「没声明」，交给自适应档。
pub fn persisted_effort(session: Option<&str>) -> Option<ThinkingEffort> {
    let key = session.map(str::trim).filter(|s| !s.is_empty())?;
    crate::session::thinking_prefs::load(key).map(|p| p.effort)
}

/// 会话/本轮档位 + 角色覆盖 → 最终档位（优先级本身在 `thinking.rs`）。
pub fn resolve_effort(session: Option<&str>, overrides: &RoleOverrides) -> Option<ThinkingEffort> {
    resolve_effort_for_role(
        session_effort_or_persisted(persisted_effort(session)),
        overrides.effort,
    )
}

/// 只读记忆召回：按楼层开关裁剪，逐路带来源标记，无命中返回 `None`。
///
/// 复用的是**公开只读**入口，不复制主对话那套私有实现：
/// * `tiered` → [`crate::memory::inject::recall`] + `render`（第六路，默认关）
/// * `cross_project` → [`crate::memory::cross_project::recall_stable_prefix`]
/// * `experience` → [`crate::memory::experience::ExperienceLedger::search`]
/// * `behavior` → [`crate::memory::behavior::recent_events`]
///
/// `sensory`（当前会话历史）与 `short_term` / `long_term` / `recall` 三路仍然只在
/// 主对话回合注入：它们的读取点在 `gateway::chat` 的私有函数里，跨模块复用需要
/// 先给那层开一个新契约（不在本次改动权限内），因此这里**不**假装已经覆盖。
pub fn memory_block(query: &str, session: Option<&str>, layers: &ResolvedLayers) -> Option<String> {
    let query = query.trim();
    if query.is_empty() {
        return None;
    }
    let session_key = session
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("agent");
    let mut lines: Vec<String> = Vec::new();

    // 第六路：分层稀疏（flyhash 近邻）。默认关闭，由 memory.tiered_enabled 决定。
    if layers.tiered {
        if let Some(recalled) = crate::memory::inject::recall(query) {
            for l in recalled {
                lines.push(l);
            }
        }
    }

    // 第五路：跨项目既有经验（与主对话同源的稳定前缀渲染）。
    if layers.cross_project {
        if let Some(msg) = crate::memory::cross_project::recall_stable_prefix(query, 3, session_key)
        {
            let content = message_text(&msg);
            for line in content.lines().filter(|l| !l.trim().is_empty()).take(3) {
                lines.push(format!("[跨项目记忆] {}", line.trim()));
            }
        }
    }

    // 第八路：自进化经验台账（踩坑 / 决策 / 模式），只查不写。
    if layers.experience {
        let db = crate::persistence::config::Config::data_dir().join("marguerite_experience.db");
        let ledger = crate::memory::experience::ExperienceLedger::new(&db);
        if let Ok(items) = ledger.search(query, 3) {
            for e in items {
                lines.push(format!("[经验] {}", e.title.trim()));
            }
        }
    }

    // 第六路之外最轻的一路：最近真实执行过的动作（进程内台账，无磁盘检索）。
    if layers.behavior {
        for ev in crate::memory::behavior::recent_events(3) {
            lines.push(format!("[行为] {}：{}", ev.kind, ev.detail.trim()));
        }
    }

    if lines.is_empty() {
        return None;
    }
    let mut out = String::from(MEMORY_HEADER);
    let mut used = out.chars().count();
    let mut pushed = 0usize;
    for line in lines {
        let cost = line.chars().count() + 2;
        if used + cost > MEMORY_CHAR_BUDGET {
            break;
        }
        used += cost;
        pushed += 1;
        out.push('\n');
        out.push_str(&line);
    }
    if pushed == 0 {
        // 一条正文都没放进去 → 不注入只剩标题的空块。
        return None;
    }
    Some(out)
}

/// 生产入口：在**调用方所在任务/线程**上解析档位 + 记忆（自己读楼层开关）。
///
/// 返回的 `RoleContext` 是按值拷贝，可以安全地搬进 `tokio::spawn`。
pub fn build(session: Option<&str>, overrides: &RoleOverrides, query: &str) -> RoleContext {
    build_with_layers(session, overrides, query, &ResolvedLayers::load())
}

/// 已持有配置快照时的入口（测试/调用方在别的线程读盘时用，避免线程局部覆盖漂移）。
pub fn build_with_layers(
    session: Option<&str>,
    overrides: &RoleOverrides,
    query: &str,
    layers: &ResolvedLayers,
) -> RoleContext {
    // 段落顺序即提示词里的顺序：既有段（记忆）→ continuity（开场简报）→ assistant（助理简报）。
    let continuity = continuity_block(session);
    let assistant = assistant_block(session);
    RoleContext {
        effort: resolve_effort(session, overrides),
        memory: memory_block(query, session, layers),
        continuity,
        assistant,
    }
}

/// 开场简报（只读）：最近 1–3 份压缩交接 + 现场 git 状态。
///
/// 项目根取自会话台账（[`crate::context::session::session_project`]）；工作区干净
/// 或没有任何交接状态时返回 `None`（不注入空块）。不写任何账本。
pub fn continuity_block(session: Option<&str>) -> Option<String> {
    let project = session.and_then(crate::context::session::session_project);
    crate::context::continuity::inject_block(project.as_deref())
}

/// 助理简报（只读）：把两层助理的稳定记忆压成一段紧凑文本，接在开场简报之后。
///
/// 作用域取自 `assistant_scope.json`（[`crate::assistant::scope::load_state`]），
/// 简报由 [`crate::assistant::brief::build_brief`] 合成（`max_chars` 即本块的硬上限）。
///
/// `session` 只表示「这条路径属于哪个会话」，**不**当作检索查询：会话键是 id 而不是
/// 语义查询，拿它去检索只会白扫一遍全部历史记忆（本机实测 79 个文件 / 230+ KB），
/// 而效果与空串无异。传空串让 `build_brief` 在 `project` 作用域自动改用 workspace 名
/// 检索，`global` 作用域则不做语义检索——不拖慢每一轮对话。
///
/// 返回 `None` = 没有任何稳定记忆（不注入空段落，避免污染提示词）。
pub fn assistant_block(_session: Option<&str>) -> Option<String> {
    let scope = crate::assistant::scope::load_state().scope();
    let brief =
        crate::assistant::brief::build_brief(&scope, "", Some(ASSISTANT_CHAR_BUDGET));
    render_assistant_brief(&brief)
}

/// 把 [`crate::assistant::brief::build_brief`] 的结构化简报渲染成紧凑文本段。
///
/// * `pinned` 逐条 `- `，排最前（用户亲手钉住的偏好）；
/// * `preferences` 一行一条（`key：value`）；
/// * `facts` 带出处：`- 正文（来源：xxx）`（简报内已按分值降序，所以砍尾部 = 先砍低分）；
/// * 三段全空（`budget.used == 0`）⇒ `None`，**不注入空段落**；
/// * 总长硬上限 [`ASSISTANT_CHAR_BUDGET`]：放不下时从尾部让位，并补一行省略提示。
fn render_assistant_brief(brief: &Value) -> Option<String> {
    let used = brief
        .get("budget")
        .and_then(|b| b.get("used"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let items = |key: &str| {
        brief
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let (pinned, preferences, facts) = (items("pinned"), items("preferences"), items("facts"));
    if used == 0 && pinned.is_empty() && preferences.is_empty() && facts.is_empty() {
        return None;
    }

    let text_of = |v: &Value| {
        v.get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let mut lines: Vec<String> = Vec::new();
    for p in &pinned {
        let text = text_of(p);
        if !text.is_empty() {
            lines.push(format!("- {text}"));
        }
    }
    for p in &preferences {
        let key = p.get("key").and_then(Value::as_str).unwrap_or("").trim();
        let value = p.get("value").and_then(Value::as_str).unwrap_or("").trim();
        if !key.is_empty() || !value.is_empty() {
            lines.push(format!("- {key}：{value}"));
        }
    }
    for f in &facts {
        let text = text_of(f);
        if text.is_empty() {
            continue;
        }
        let source = f
            .get("source")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("cross_project");
        lines.push(format!("- {text}（来源：{source}）"));
    }
    if lines.is_empty() {
        return None;
    }

    // 按顺序装满：pinned → preferences → facts，装不下的尾部一律让位。
    let mut kept = 0usize;
    let mut used_chars = ASSISTANT_HEADER.chars().count();
    for line in &lines {
        let cost = line.chars().count() + 1; // '\n'
        if used_chars + cost > ASSISTANT_CHAR_BUDGET {
            break;
        }
        used_chars += cost;
        kept += 1;
    }
    let mut dropped = kept < lines.len();
    let note_cost = ASSISTANT_TRUNCATED_NOTE.chars().count() + 1;
    let brief_truncated = brief
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if brief_truncated || dropped {
        // 提示行必须放得下：不够就继续从尾部（低分事实）让位。
        while kept > 0 && used_chars + note_cost > ASSISTANT_CHAR_BUDGET {
            kept -= 1;
            used_chars -= lines[kept].chars().count() + 1;
            dropped = true;
        }
    }
    if kept == 0 {
        // 一条正文都放不下 → 不注入只剩标题的空块。
        return None;
    }

    let mut out = String::from(ASSISTANT_HEADER);
    for line in lines.iter().take(kept) {
        out.push('\n');
        out.push_str(line);
    }
    if brief_truncated || dropped {
        out.push('\n');
        out.push_str(ASSISTANT_TRUNCATED_NOTE);
    }
    Some(out)
}

/// 从 `Message` 里取正文文本（跨项目召回返回的是 System 消息）。
fn message_text(msg: &crate::types::Message) -> String {
    match msg {
        crate::types::Message::System { content, .. }
        | crate::types::Message::User { content, .. }
        | crate::types::Message::Assistant { content, .. } => content.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::layers::ResolvedLayers;

    fn all_off() -> ResolvedLayers {
        ResolvedLayers {
            sensory: false,
            short_term: false,
            long_term: false,
            recall: false,
            cross_project: false,
            behavior: false,
            tiered: false,
            experience: false,
        }
    }

    /// 优先级只有一处定义：角色覆盖 > 本轮/会话 > 无声明（继续自适应）。
    #[test]
    fn role_context_effort_follows_single_precedence_rule() {
        // 无会话键（`None` 持久档）时，角色覆盖位就是最终档位。
        let ov = RoleOverrides::with_effort(ThinkingEffort::Max);
        assert_eq!(resolve_effort(None, &ov).unwrap(), ThinkingEffort::Max);
        // 无覆盖 + 无会话键 = 不声明（绝不臆造档位）。
        assert_eq!(resolve_effort(None, &RoleOverrides::NONE), None);
        // 显式 Off 是声明，压过「没有档位」。
        assert_eq!(
            resolve_effort(None, &RoleOverrides::with_effort(ThinkingEffort::Off)),
            Some(ThinkingEffort::Off)
        );
    }

    /// 会话未知键 → 无持久档位（读盘失败只降级，不阻断执行）。
    #[test]
    fn unknown_session_has_no_persisted_effort() {
        assert_eq!(persisted_effort(None), None);
        assert_eq!(persisted_effort(Some("")), None);
        assert_eq!(persisted_effort(Some("   ")), None);
        assert_eq!(
            persisted_effort(Some("no-such-session-key-xyz")),
            None,
            "不存在的会话档位必须是「没声明」而不是 Off"
        );
    }

    /// 楼层全关 → 一块记忆都不注入（这是「开关在非主对话路径同样有效」的锚点）。
    #[test]
    fn all_layers_off_yields_no_memory_block() {
        assert!(
            memory_block("如何部署本地模型", Some("s-off"), &all_off()).is_none(),
            "楼层关闭时不得注入任何记忆文本"
        );
        assert!(memory_block("  ", Some("s-off"), &ResolvedLayers::load()).is_none());
    }

    /// tiered 开启时，只读召回能落地成带来源标记的块（进程内单例，离线确定性）。
    #[test]
    fn tiered_on_recalls_read_only_block() {
        let _g = crate::memory::inject::test_guard();
        crate::memory::inject::reset_for_test();
        let text = "角色上下文只读召回必须把这条带进团队执行";
        assert!(crate::memory::inject::remember(text, "role-context-test"));
        let mut layers = all_off();
        layers.tiered = true;
        let block = memory_block(text, Some("s-tiered"), &layers).expect("tiered 开启应命中");
        assert!(block.starts_with(MEMORY_HEADER), "{block}");
        assert!(block.contains("[分层记忆]"), "必须带来源标记: {block}");
        assert!(block.contains(text), "召回正文必须在块里: {block}");
        // 只读：本模块不得写入任何库。查完的这条也只是进程内 tiered 缓存（测试自己播的种）。
        crate::memory::inject::reset_for_test();
    }

    /// `RoleContext::scope` 是跨 spawn 传递档位的作用域入口。
    #[tokio::test]
    async fn role_context_scopes_effort_for_spawned_tasks() {
        let ctx = RoleContext::effort_only(Some(ThinkingEffort::Max));
        let seen = ctx
            .scope(async { crate::model::thinking::current_turn_effort() })
            .await;
        assert_eq!(seen, Some(ThinkingEffort::Max));

        let empty = RoleContext::none();
        // 无档位时不得覆盖外层作用域（否则「继承」会变成「清空」）。
        let nested = crate::model::thinking::TURN_EFFORT
            .scope(
                Some(ThinkingEffort::Deep),
                empty.scope(async { crate::model::thinking::current_turn_effort() }),
            )
            .await;
        assert_eq!(nested, Some(ThinkingEffort::Deep));
    }

    /// 记忆块受字符预算约束（不得让记忆吃掉子任务上下文）。
    #[test]
    fn memory_block_respects_char_budget() {
        let _g = crate::memory::inject::test_guard();
        crate::memory::inject::reset_for_test();
        for i in 0..12 {
            crate::memory::inject::remember(
                &format!("第{i}条用于验证角色上下文注入预算的记忆片段：这一段刻意写得很长很长以求撑爆上限阈值"),
                "role-context-budget",
            );
        }
        let mut layers = all_off();
        layers.tiered = true;
        let block = memory_block("验证角色上下文注入预算", Some("s-budget"), &layers).expect("应命中");
        assert!(
            block.chars().count() <= MEMORY_CHAR_BUDGET,
            "记忆块 {} 字符超过预算 {MEMORY_CHAR_BUDGET}",
            block.chars().count()
        );
        crate::memory::inject::reset_for_test();
    }

    /// 隔离配置目录：助理的作用域 / 钉住 / 画像都落在临时目录里（不读真实用户数据）。
    fn iso(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "marguerite_role_ctx_{}_{}",
            tag,
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        crate::persistence::config::Config::set_config_dir_override(dir.clone());
        dir
    }

    /// 没有任何稳定记忆 → 不注入空段落（否则每轮对话都被一段空标题污染）。
    #[test]
    fn assistant_block_is_absent_when_no_stable_memory() {
        let _dir = iso("assist-empty");
        assert!(
            assistant_block(None).is_none(),
            "无钉住 / 无偏好 / 无事实时不得注入助理简报"
        );
        assert!(assistant_block(Some("s-empty")).is_none());
        // 纯渲染路径：`budget.used == 0` 且三段全空 ⇒ None。
        let empty = crate::assistant::compose_brief(crate::assistant::BriefInput {
            scope: "global".to_string(),
            ..Default::default()
        });
        assert!(
            render_assistant_brief(&empty).is_none(),
            "空简报不得渲染出只剩标题的段落"
        );
    }

    /// pinned / preferences / facts 都要落地，且事实必须带出处。
    #[test]
    fn assistant_block_renders_pinned_preferences_and_facts_with_source() {
        use crate::assistant::brief::{Fact, PinnedItem, Preference};
        let _dir = iso("assist-render");
        let brief = crate::assistant::compose_brief(crate::assistant::BriefInput {
            scope: "project".to_string(),
            pinned: vec![PinnedItem {
                text: "所有文档用中文".to_string(),
                source: "user".to_string(),
                at: "2026-10-30T00:00:00Z".to_string(),
            }],
            preferences: vec![Preference {
                key: "stack".to_string(),
                value: "Rust".to_string(),
                confidence: 0.9,
                evidence: 1,
            }],
            facts: vec![Fact {
                text: "0.2.19 修了启动 panic".to_string(),
                source: "project:marguerite".to_string(),
                score: 0.8,
            }],
            ..Default::default()
        });
        let block = render_assistant_brief(&brief).expect("有内容必须注入");
        assert!(block.starts_with(ASSISTANT_HEADER), "{block}");
        assert!(
            block.contains("\n- 所有文档用中文"),
            "钉住条目应逐条 `- ` 列出：{block}"
        );
        assert!(block.contains("\n- stack：Rust"), "偏好一行一条：{block}");
        assert!(
            block.contains("0.2.19 修了启动 panic（来源：project:marguerite）"),
            "事实必须带出处：{block}"
        );
        assert!(
            !block.contains(ASSISTANT_TRUNCATED_NOTE),
            "没截断时不得出现省略提示：{block}"
        );

        // 生产路径（不是只测纯函数）：画像落盘后 assistant_block 必须真的读到它。
        crate::assistant::UserProfile {
            language: Some("中文".to_string()),
            stacks: vec!["Rust".to_string()],
            taboos: Vec::new(),
        }
        .save()
        .expect("写入 user_profile.md 应成功");
        let live = assistant_block(None).expect("有稳定偏好就该注入");
        assert!(live.contains("\n- language：中文"), "{live}");
    }

    /// 超 800 字符：从 facts 尾部（低分项）砍起，并补一行省略提示。
    #[test]
    fn assistant_block_clips_facts_tail_to_hard_limit() {
        use crate::assistant::brief::Fact;
        let _dir = iso("assist-clip");
        let facts: Vec<Fact> = (0..40)
            .map(|i| Fact {
                text: format!("第{i}条跨项目事实：{}", "很长的说明文字".repeat(4)),
                source: format!("cross_project:notes-{i}.md"),
                score: 1.0 - (i as f64) * 0.01,
            })
            .collect();
        let brief = crate::assistant::compose_brief(crate::assistant::BriefInput {
            scope: "global".to_string(),
            facts,
            max_chars: Some(4_000),
            ..Default::default()
        });
        assert!(
            !brief["truncated"].as_bool().unwrap(),
            "compose 阶段预算给足，不该截断（要测的是渲染上限）"
        );
        let block = render_assistant_brief(&brief).expect("有事实必须注入");
        assert!(
            block.chars().count() <= ASSISTANT_CHAR_BUDGET,
            "助理简报 {} 字符超过硬上限 {ASSISTANT_CHAR_BUDGET}",
            block.chars().count()
        );
        assert!(
            block.contains(ASSISTANT_TRUNCATED_NOTE),
            "省略条目必须明说：{block}"
        );
        assert!(block.contains("第0条"), "高分事实优先保留：{block}");
        assert!(
            !block.contains("第39条"),
            "尾部低分事实必须先被砍：{block}"
        );
    }
}
