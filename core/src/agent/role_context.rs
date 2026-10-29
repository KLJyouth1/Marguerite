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
//! 本模块是那个缺失的桥，只做两件事：
//! 1. **档位**：把「本轮显式档位 / 会话持久档位 / 角色覆盖」交给
//!    [`crate::model::thinking::resolve_effort_for_role`]（优先级只在那一处定义），
//!    并给出跨 `tokio::spawn` 的作用域入口 [`RoleContext::scope`]；
//! 2. **记忆（只读）**：按楼层开关 [`ResolvedLayers`] 裁剪的召回文本
//!    [`memory_block`]，关掉的楼层**连检索都不做**（不是「查了不用」）。
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

/// 单次执行的角色上下文：已解析档位 + 已召回记忆块。
#[derive(Debug, Clone, Default)]
pub struct RoleContext {
    /// 解析后的档位；`None` = 没有任何显式声明，继续按自适应/模式档执行
    /// （注意：这不等于「不思考」，`Some(Off)` 才是显式不思考）。
    pub effort: Option<ThinkingEffort>,
    /// 只读记忆块文本（`None` = 无命中或对应楼层被关闭 → 完全不注入）。
    pub memory: Option<String>,
}

impl RoleContext {
    /// 空上下文：档位与记忆都不声明，行为与改动前逐字节一致。
    pub fn none() -> Self {
        Self::default()
    }

    /// 只带档位（调用方自己完成记忆读取，或该路径不需要记忆）。
    pub fn effort_only(effort: Option<ThinkingEffort>) -> Self {
        Self { effort, memory: None }
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
    RoleContext {
        effort: resolve_effort(session, overrides),
        memory: memory_block(query, session, layers),
    }
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
}
