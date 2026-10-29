//! DeepSeek-V4-Pro 极限思考与超长上下文专项（模型层自适应）。
//!
//! 目标：针对 DeepSeek-V4-Pro-0813 的基准短板做专项优化——
//! - `HLE / Agents' Last Exam / AutomationBench`（推理/长任务）：开高 `reasoning_effort` + 大思考预算 → 极限思考。
//! - `DeepSWE / Terminal Bench / NL2Repo`（代码长输出）：代码任务自动放大 `max_tokens`，防推理+输出共享预算时被截断。
//! - `DSBench / Toolathlon`（工具/全栈）：工具任务走高温度 + 富工具回路。
//! - 长任务：1M 上下文 + 精细压缩，不提前压缩掉关键中间结论。
//!
//! 本模块只产出**可观测、可测试**的档位决策，不臆造后端私有字段：
//! `reasoning_effort` 与既有 `mode/spec.rs` 的 `reasoning_effort` 语义一致（low/medium/high），
//! 对不认识的 provider 不注入（降级为仅本地 `max_tokens` 自适应，绝对安全）。

use serde::{Deserialize, Serialize};

/// DeepSeek-V4-Pro 正式版模型名（作为自适应档位的语义锚点；不强制匹配，仅作参考）
pub const V4_PRO_MODEL: &str = "deepseek-v4-pro";

/// 384K 输出档：长任务/代码任务的最大输出预算（DeepSeek-V4-Pro 支持大 max_tokens）
pub const MAX_TOKENS_LARGE: u32 = 384 * 1024;
/// 160K 输出档：推理/工具任务
pub const MAX_TOKENS_BIG: u32 = 160 * 1024;
/// 普通对话档
pub const MAX_TOKENS_NORMAL: u32 = 32 * 1024;
/// 1M 上下文：长任务模式的目标上下文窗口
pub const CONTEXT_WINDOW_1M: u32 = 1_000_000;
/// 4K 上下文：普通对话
pub const CONTEXT_WINDOW_DEFAULT: u32 = 64 * 1024;

/// 思考深度档位
///
/// `PartialOrd`/`Ord` 由**声明顺序**给出，且声明顺序即语义强度顺序：
/// `Off < Normal < Deep < Max`。档位准入校验（`tier.rs::TierCaps::accepts`）依赖
/// 该顺序比较「档案支持的推理强度是否达到档位准入线」，因此**禁止重排变体**——
/// `thinking_effort_ordering_is_semantic` 测试会在此被破坏时立即失败。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub enum ThinkingEffort {
    /// 关闭思考（纯对话）
    #[default]
    Off,
    /// 常规思考
    Normal,
    /// 深度思考（推理/代码）
    Deep,
    /// 极限思考（HLE/Agents' Last Exam 等难推理）
    Max,
}

impl ThinkingEffort {
    /// 映射为 DeepSeek 兼容的 `reasoning_effort` 字符串（与 `mode/spec.rs` 语义一致）。
    /// Off 返回 None（不注入字段）。
    pub fn reasoning_effort(self) -> Option<&'static str> {
        match self {
            ThinkingEffort::Off => None,
            ThinkingEffort::Normal => Some("low"),
            ThinkingEffort::Deep => Some("medium"),
            ThinkingEffort::Max => Some("high"),
        }
    }
    /// 是否在此档位注入思考系统提示（引导模型先推理再作答）
    pub fn wants_reasoning_prompt(self) -> bool {
        matches!(self, ThinkingEffort::Deep | ThinkingEffort::Max)
    }

    /// 档位的**规范键**（`off` / `normal` / `deep` / `max`）——UI 与 RPC 的对外口径。
    ///
    /// 与 [`Self::reasoning_effort`] 的区别：那个是**请求体 wire 值**（`low/medium/high`，
    /// Off 为 None），这个是**档位标识**（恒有值）。两者不可互相替代：
    /// 前端滑块按 `as_key` 定位档位，请求体只认 `reasoning_effort`。
    pub fn as_key(self) -> &'static str {
        match self {
            ThinkingEffort::Off => "off",
            ThinkingEffort::Normal => "normal",
            ThinkingEffort::Deep => "deep",
            ThinkingEffort::Max => "max",
        }
    }

    /// 严格解析档位键（`off|normal|deep|max`，别名 `none|low|medium|high`）。
    ///
    /// 与 [`Self::from_spec_str`] 的差别是**对未知值返回 `None`**：
    /// 这是 RPC/请求参数入口，成本治理不允许把 typo（如 `"higg"`）静默升级成 Deep。
    /// 调用方必须响亮拒绝（见 `gateway::chat` 与 `session.thinking.set`）。
    pub fn from_key(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" => Some(ThinkingEffort::Off),
            "low" | "normal" => Some(ThinkingEffort::Normal),
            "medium" | "deep" => Some(ThinkingEffort::Deep),
            "high" | "max" => Some(ThinkingEffort::Max),
            _ => None,
        }
    }

    /// 从模式规格字符串解析档位（`mode/spec.rs` 的 `BloomSpec::reasoning_effort`）。
    /// 历史缺陷：`BloomSpec.reasoning_effort`（bloom.yaml = `max`，omni_transcend.yaml = `max`）
    /// 是纯展示字段，从无解析入口，导致模式声明的思考强度到不了请求体。此处补上解析，
    /// 让 `mode/spec.rs` 的字符串与 `ThinkingEffort` 枚举真正贯通。
    ///
    /// 映射（大小写与首尾空白无关）：
    /// - `max` / `high` → [`ThinkingEffort::Max`]（请求体 `reasoning_effort: high`）
    /// - `medium` / `deep` → [`ThinkingEffort::Deep`]
    /// - `low` / `normal` → [`ThinkingEffort::Normal`]
    /// - `off` / `none` / 空串 → [`ThinkingEffort::Off`]
    /// - 未知值 → [`ThinkingEffort::Deep`]（保守：宁可多想，不静默降级为不思考）
    pub fn from_spec_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "max" | "high" => ThinkingEffort::Max,
            "medium" | "deep" => ThinkingEffort::Deep,
            "low" | "normal" => ThinkingEffort::Normal,
            "off" | "none" | "" => ThinkingEffort::Off,
            _ => ThinkingEffort::Deep,
        }
    }
}

/// 任务类型（用于自适应思考/输出/上下文预算）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    /// 普通对话
    Chat,
    /// 代码生成/修复/仓库改造（DeepSWE / Terminal Bench / NL2Repo）
    Code,
    /// 数学/逻辑/难推理（HLE / Agents' Last Exam / MATH）
    Reasoning,
    /// 工具调用为主（Toolathlon / AutomationBench / DSBench）
    Tool,
    /// 长文档/长任务（1M 上下文）
    Long,
}

impl TaskKind {
    /// 从提示词启发式分类（保守：无法判断时归为 Chat）。
    /// 纯规则、无依赖、可测试。
    pub fn classify(prompt: &str) -> Self {
        let p = prompt.to_ascii_lowercase();
        let has_code = [
            "```",
            "fn ",
            "impl ",
            "def ",
            "class ",
            "return ",
            "function ",
            "import ",
            "cargo",
            "compile",
            "refactor",
            "重构",
            "代码",
            "bug",
            "修复",
            "测试",
            "test",
        ]
        .iter()
        .any(|k| p.contains(k));
        let has_reasoning = [
            "proof",
            "theorem",
            "verify",
            "prove",
            "证明",
            "推理",
            "solve",
            "数学",
            "abc conjecture",
            "imo",
            "olympiad",
        ]
        .iter()
        .any(|k| p.contains(k));
        let has_long = [
            "long task",
            "长任务",
            "whole repo",
            "整个仓库",
            "very long",
            "续写",
            "总结这",
            "entire document",
            "超长",
        ]
        .iter()
        .any(|k| p.contains(k));
        let has_tool = [
            "tool",
            "工具",
            "call",
            "execute",
            "run command",
            "调用",
            "终端",
            "shell",
        ]
        .iter()
        .any(|k| p.contains(k));

        if has_long {
            TaskKind::Long
        } else if has_code {
            TaskKind::Code
        } else if has_reasoning {
            TaskKind::Reasoning
        } else if has_tool {
            TaskKind::Tool
        } else {
            TaskKind::Chat
        }
    }
}

/// 自适应思考/输出/上下文档位（一次调用的完整决策）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThinkingProfile {
    pub effort: ThinkingEffort,
    pub max_tokens: u32,
    pub context_window: u32,
}

/// 为给定任务类型产出自适应档位（DeepSeek-V4-Pro 专项基准映射）。
pub fn profile_for(kind: TaskKind) -> ThinkingProfile {
    match kind {
        // 代码：解决 DeepSWE 截断的根本——放大输出预算，思考 medium。
        TaskKind::Code => ThinkingProfile {
            effort: ThinkingEffort::Deep,
            max_tokens: MAX_TOKENS_LARGE,
            context_window: 256 * 1024,
        },
        // 难推理：极限思考 + 大思考预算，输出留给 reasoning_content。
        TaskKind::Reasoning => ThinkingProfile {
            effort: ThinkingEffort::Max,
            max_tokens: MAX_TOKENS_BIG,
            context_window: 128 * 1024,
        },
        // 工具：普通思考 + 中输出，富工具回路。
        TaskKind::Tool => ThinkingProfile {
            effort: ThinkingEffort::Normal,
            max_tokens: MAX_TOKENS_BIG,
            context_window: 128 * 1024,
        },
        // 长任务：1M 上下文 + 大输出，配合精细压缩。
        TaskKind::Long => ThinkingProfile {
            effort: ThinkingEffort::Deep,
            max_tokens: MAX_TOKENS_LARGE,
            context_window: CONTEXT_WINDOW_1M,
        },
        // 对话：小预算、低延迟。
        TaskKind::Chat => ThinkingProfile {
            effort: ThinkingEffort::Off,
            max_tokens: MAX_TOKENS_NORMAL,
            context_window: CONTEXT_WINDOW_DEFAULT,
        },
    }
}

/// 按档位给出**差异化**的思考指令；`Off` 返回 None（完全不注入）。
///
/// 缺陷背景：此前只有一个通用常量 [`reasoning_system_prompt`]，而且**全仓没有任何
/// 调用点**——档位只改了请求体里的 `reasoning_effort` 字段，模型侧完全不知道
/// 「这一轮该想多深」。用户实测感受就是「档位是玩具」。
///
/// 四档语义（对应 UI 的 不思考 / 思考 / 深度思考 / 极限推理）：
/// - Off：不注入（省 token、低延迟）；
/// - Normal：单句要求“心里过一遍约束”，不要求输出推理；
/// - Deep：完整推理（拆解约束 + 验证依据 + 找反例）；
/// - Max：穷尽推理（列全路径 + 自我反驳 + 显式写出排除理由 + 边界检查）。
pub fn reasoning_system_prompt_for(effort: ThinkingEffort) -> Option<&'static str> {
    match effort {
        ThinkingEffort::Off => None,
        ThinkingEffort::Normal => Some(
            "回答前先在心里过一遍关键约束与已知事实；不要输出冗长的推理过程，直接给结论。",
        ),
        ThinkingEffort::Deep => Some(
            "你是深度推理模型。面对困难问题，请先完整推理（reasoning）：\
             拆解约束、验证每步依据、主动寻找反例与边界情况，再给出最终答案；\
             必要时给出多个候选方案并说明为何选择其一。不要过早收敛。",
        ),
        ThinkingEffort::Max => Some(
            "你是极限推理模型。必须穷尽推理：把所有可能路径列全并逐一评估，\
             主动构造反例自我攻击，明确写出被排除的方案与排除理由，\
             检查边界条件与单位/量纲一致性，最后再给结论。\
             宁可多花推理预算，也不要漏掉一个约束。",
        ),
    }
}

/// 本轮档位对应的思考指令（无显式档位 = 不注入，交给自适应策略）。
pub fn reasoning_system_prompt_for_turn() -> Option<&'static str> {
    reasoning_system_prompt_for(current_turn_effort()?)
}

/// 兼容旧入口（等价 Deep 档文本；新代码请用 [`reasoning_system_prompt_for`]）。
///
/// 保留它只为不破坏既有调用/文档；实现直接委托，避免"两处文案各自漂移"。
pub fn reasoning_system_prompt() -> &'static str {
    reasoning_system_prompt_for(ThinkingEffort::Deep).unwrap_or("")
}

/// 依据模型名是否为 DeepSeek 推理系，决定是否注入 `reasoning_effort` 字段。
/// 对未知 provider 返回 false → 调用方仅做本地 `max_tokens` 自适应（安全降级）。
pub fn supports_reasoning_effort(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.contains("deepseek") || m.starts_with("reasoner") || m.contains("v4-pro")
}

tokio::task_local! {
    /// 本轮用户显式选择的思考档位（`chat.send` 的 `effort` 参数）。
    ///
    /// 为什么需要它：模型请求的构造点在 `ModelRouter::chat_with_tools` /
    /// `chat_stream_with_tools`（工具回路），那里**只有** `default_client` 与任务分类，
    /// 拿不到 `RouteDecision`。于是 UI 选 `deep` 会被 `with_adaptive_profile(chat)`
    /// 覆盖成 `Off` —— 用户拖动档位、请求体里却从来没有这个参数（真机实测确认）。
    /// 用 task-local 把「本轮显式档位」传到请求构造点，且不跨会话串扰。
    pub static TURN_EFFORT: Option<ThinkingEffort>;
}

/// 读取本轮显式档位（无 scope / 未选择时返回 `None`，调用方回落到自适应）。
pub fn current_turn_effort() -> Option<ThinkingEffort> {
    TURN_EFFORT.try_with(|e| *e).ok().flatten()
}

// ── 角色级档位/模型覆盖（P1-1：把解析点从「模式级」扩到「角色级」）────────────
//
// 历史缺陷（2026-10-27 路线 §5.2 的四个 ❌）：思考档位与记忆召回只在**主对话回合**
// 生效；多策略 / 专家 / 团队 / 调用链这四条执行路径静默忽略它们。根因有两层：
// 1. `TURN_EFFORT` 是 task-local，**不跨 `tokio::spawn` 继承** —— 后台/蜂群路径
//    一旦 spawn，档位就退回「自适应」，用户拖到 max 也没有任何请求体变化；
// 2. 角色（专家档案 / 人设 / 策略）**没有承载位**，即使解析出档位也无处传下去。
//
// 这里只补两件最小的东西：一个**可选**承载位 [`RoleOverrides`]，和**唯一一处**
// 优先级定义 [`resolve_effort_for_role`]。`None` = 完全继承会话/模式档位，
// 因此对所有既有调用点与既有测试，行为逐字节不变。

/// 角色级可选覆盖位（专家档案 / 人设 / 策略 / 链步骤共用同一个承载位）。
///
/// 语义铁律：
/// - `None` 不是「关闭」，而是「**没有声明**」→ 继承会话/模式档位；
/// - `Some(Off)` 才是「显式要求不思考」→ 请求体不注入 `reasoning_effort`；
///   两者混淆过一次就是真实事故（清除会话档位 = 删文件返回 `None`，
///   见 `session::thinking_prefs::clear` 的同源注释）。
///
/// 可 `const` 构造（[`RoleOverrides::NONE`]）是因为 `ExpertProfile` 的两张内置表
/// 是 `static`：静态表里的每条档案都要能在 const 上下文里声明「无覆盖」。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleOverrides {
    /// 角色声明的思考档位（`None` = 继承会话/模式档位）
    #[serde(default)]
    pub effort: Option<ThinkingEffort>,
    /// 角色声明的模型（`None` = 继承会话/路由决策的模型）
    #[serde(default)]
    pub model: Option<String>,
}

impl RoleOverrides {
    /// 「无覆盖」常量：可在 `const`/`static` 上下文里使用（内置专家库表用）。
    pub const NONE: Self = Self {
        effort: None,
        model: None,
    };

    /// 同 [`Self::NONE`]（运行时构造点用它表达「本角色不覆盖任何东西」）。
    pub fn none() -> Self {
        Self::NONE
    }

    /// 只覆盖思考档位（最常用的一种声明）。
    pub fn with_effort(effort: ThinkingEffort) -> Self {
        Self {
            effort: Some(effort),
            model: None,
        }
    }

    /// 是否两个覆盖位都没声明（= 完全继承，行为与今日一致）。
    pub fn is_empty(&self) -> bool {
        self.effort.is_none() && self.model.is_none()
    }
}

/// **唯一**一处档位优先级定义：角色覆盖 > 会话/本轮显式档位 > 上游回落（自适应）。
///
/// 四条执行路径（多策略 / 专家 / 团队 / 调用链）都必须经它，不得各自再写一遍
/// `or` / `unwrap_or` —— 一旦有两处定义，「谁赢」就变成读代码才能猜的问题，
/// 而这正是本缺陷最难排查的部分（同一档位在主对话生效、在团队不生效）。
///
/// 返回 `Option` 是刻意的：`None` 表示「这两层都没有声明」，调用方**继续**用它
/// 自己的默认（模型配置档 / 任务自适应档），而不是在这里臆造一个档位。
pub fn resolve_effort_for_role(
    session_effort: Option<ThinkingEffort>,
    role_override: Option<ThinkingEffort>,
) -> Option<ThinkingEffort> {
    role_override.or(session_effort)
}

/// 本轮/会话档位：task-local 显式档位优先，其次会话持久档位。
///
/// `persisted` 必须由调用方**在 spawn 之前**读好传进来
/// （`session::thinking_prefs::load` 落到 `Config::data_dir()`，而配置目录覆盖是
/// 线程局部的 —— 在 spawned 任务里读会读到错误的目录）。
pub fn session_effort_or_persisted(persisted: Option<ThinkingEffort>) -> Option<ThinkingEffort> {
    // 参数顺序不能反：`resolve_effort_for_role(session_effort, role_override)` 是**后者赢**。
    // 这里「更具体的一层」是本轮显式档位（TURN_EFFORT task-local），持久档位才是兜底。
    // 此前写成 `(current_turn_effort(), persisted)`，等价于让持久档位压过本轮显式档位，
    // 与函数文档「本轮显式档位优先」相反。
    resolve_effort_for_role(persisted, current_turn_effort())
}

/// 模型名的角色级优先级（与 [`resolve_effort_for_role`] 同构，口径也只定义一次）。
///
/// 注意：这里只做**解析**，不改路由。真正把角色模型写进请求体需要
/// `model::router` 的入口改造（本模块外），见交付报告的「未做事项」。
pub fn resolve_model_for_role<'a>(
    session_model: &'a str,
    role_override: Option<&'a str>,
) -> &'a str {
    match role_override {
        Some(m) if !m.trim().is_empty() => m,
        _ => session_model,
    }
}

/// 在给定档位作用域内运行 future；`None` = 原样运行（不覆盖已有作用域）。
///
/// 存在意义：跨 `tokio::spawn` 的显式传递在每条路径上都是同一个 `match`，
/// 各处手写就会有人漏掉 `Some(Off)` 之外的分支。集中到这里后，
/// 「传档位」这件事只有一个实现可漏。
pub async fn scoped_effort<T, F>(effort: Option<ThinkingEffort>, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    match effort {
        Some(e) => TURN_EFFORT.scope(Some(e), fut).await,
        None => fut.await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 优先级真值表（角色覆盖 > 会话/本轮 > 上游回落）──────────────────────
    //
    // 每条都对应一次真实用户操作：滑块选档（会话/本轮）、角色声明覆盖（role）、
    // 什么都没选（两者皆 None → 不得臆造档位）。`Off` 单列一行，因为它必须
    // 被当作「显式不思考」而不是「没声明」。

    #[test]
    fn resolve_effort_for_role_truth_table() {
        use ThinkingEffort as E;
        // 1) 双方都有声明：角色覆盖赢（这是「专家比通用成员多想一层」的入口）
        assert_eq!(resolve_effort_for_role(Some(E::Normal), Some(E::Max)), Some(E::Max));
        assert_eq!(resolve_effort_for_role(Some(E::Max), Some(E::Normal)), Some(E::Normal));
        // 2) 只有会话/本轮档位：原样继承（现状行为，必须逐字节不变）
        assert_eq!(resolve_effort_for_role(Some(E::Deep), None), Some(E::Deep));
        // 3) 只有角色声明：用它
        assert_eq!(resolve_effort_for_role(None, Some(E::Deep)), Some(E::Deep));
        // 4) 双方都没声明：None —— 绝不臆造档位（调用方继续走自适应）
        assert_eq!(resolve_effort_for_role(None, None), None);
        // 5) 显式 Off 是「声明」，不是「没声明」：它必须赢过会话档位
        assert_eq!(resolve_effort_for_role(Some(E::Max), Some(E::Off)), Some(E::Off));
        assert_eq!(resolve_effort_for_role(Some(E::Off), None), Some(E::Off));
    }

    #[test]
    fn role_overrides_none_is_empty_and_inherits() {
        // 内置角色表的每一行都是 NONE：因此「加了承载位」不等于「改了行为」。
        assert!(RoleOverrides::NONE.is_empty());
        assert_eq!(RoleOverrides::NONE, RoleOverrides::default());
        assert_eq!(
            resolve_effort_for_role(Some(ThinkingEffort::Normal), RoleOverrides::NONE.effort),
            Some(ThinkingEffort::Normal),
            "None 覆盖位必须原样继承会话档位"
        );
        // 显式档位不是「空」：Some(Off) 也算声明。
        assert!(!RoleOverrides::with_effort(ThinkingEffort::Off).is_empty());
    }

    #[test]
    fn resolve_model_for_role_ignores_blank_override() {
        // 空白模型名（用户填了个空格）不得把请求打到空模型上 —— 回落会话模型。
        assert_eq!(resolve_model_for_role("deepseek-v4-pro", Some("  ")), "deepseek-v4-pro");
        assert_eq!(resolve_model_for_role("deepseek-v4-pro", None), "deepseek-v4-pro");
        assert_eq!(
            resolve_model_for_role("deepseek-v4-pro", Some("deepseek-reasoner")),
            "deepseek-reasoner"
        );
    }

    #[tokio::test]
    async fn session_effort_or_persisted_prefers_scoped_turn() {
        use ThinkingEffort as E;
        // 无 scope：回落持久档位（这就是团队/蜂群路径缺的那一层）。
        assert_eq!(session_effort_or_persisted(Some(E::Max)), Some(E::Max));
        assert_eq!(session_effort_or_persisted(None), None);
        // 有本轮显式档位：本轮赢（请求级 > 会话级，与 thinking_prefs 同口径）。
        let inside = TURN_EFFORT
            .scope(Some(E::Normal), async move { session_effort_or_persisted(Some(E::Max)) })
            .await;
        assert_eq!(inside, Some(E::Normal));
    }

    #[tokio::test]
    async fn scoped_effort_does_not_leak_outside_the_future() {
        use ThinkingEffort as E;
        let inner = scoped_effort(Some(E::Max), async { current_turn_effort() }).await;
        assert_eq!(inner, Some(E::Max));
        // 作用域只覆盖被包裹的 future：出来必须回到「无档位」。
        assert_eq!(current_turn_effort(), None);
        // `None` = 不覆盖，也不会把已有作用域清掉。
        let nested = TURN_EFFORT
            .scope(
                Some(E::Deep),
                scoped_effort(None, async { current_turn_effort() }),
            )
            .await;
        assert_eq!(nested, Some(E::Deep));
    }

    /// 四档必须给出**不同**的思考指令，而不是一个通用常量。
    ///
    /// 缺陷背景：修复前 `reasoning_system_prompt()` 是死代码，四档在提示层完全没区别，
    /// 用户拖滑块感受不到任何变化。这个测试锁住"分档"这件事本身。
    #[test]
    fn four_tiers_produce_distinct_reasoning_prompts() {
        assert!(
            reasoning_system_prompt_for(ThinkingEffort::Off).is_none(),
            "不思考档不得注入任何思考指令"
        );
        let normal = reasoning_system_prompt_for(ThinkingEffort::Normal).unwrap();
        let deep = reasoning_system_prompt_for(ThinkingEffort::Deep).unwrap();
        let max = reasoning_system_prompt_for(ThinkingEffort::Max).unwrap();
        assert_ne!(normal, deep, "思考 / 深度思考必须不同");
        assert_ne!(deep, max, "深度思考 / 极限推理必须不同");
        assert_ne!(normal, max);
        // 语义要点：Normal 不要长篇推理；Max 要穷尽 + 自我反驳 + 排除理由。
        assert!(normal.contains("不要输出冗长"));
        assert!(deep.contains("完整推理"));
        assert!(max.contains("穷尽"));
        assert!(max.contains("反例"));
        assert!(max.contains("排除理由"));
        // 层级强度：Max 的指令应当比 Normal 更长更严（不是同义改写）
        assert!(max.chars().count() > normal.chars().count());
    }

    #[test]
    fn legacy_prompt_entry_still_returns_deep_text() {
        // 旧入口保留兼容，但新代码必须走 reasoning_system_prompt_for
        assert_eq!(
            reasoning_system_prompt(),
            reasoning_system_prompt_for(ThinkingEffort::Deep).unwrap()
        );
    }

    #[test]
    fn classify_routes_code() {
        assert_eq!(
            TaskKind::classify("请修复这个 bug：fn broken() { return; }"),
            TaskKind::Code
        );
        assert_eq!(
            TaskKind::classify("cargo build 报错，帮我改"),
            TaskKind::Code
        );
    }

    #[test]
    fn classify_routes_reasoning() {
        assert_eq!(
            TaskKind::classify("证明费马大定理的 n=3 情形"),
            TaskKind::Reasoning
        );
        assert_eq!(
            TaskKind::classify("solve the olympiad inequality"),
            TaskKind::Reasoning
        );
    }

    #[test]
    fn classify_routes_long_and_tool() {
        assert_eq!(TaskKind::classify("总结整个仓库的架构"), TaskKind::Long);
        assert_eq!(
            TaskKind::classify("调用工具执行 shell 命令"),
            TaskKind::Tool
        );
    }

    #[test]
    fn classify_defaults_to_chat() {
        assert_eq!(TaskKind::classify("你好"), TaskKind::Chat);
        assert_eq!(TaskKind::classify(""), TaskKind::Chat);
    }

    #[test]
    fn profile_code_uses_large_output_to_avoid_truncation() {
        // DeepSWE/Terminal Bench 截断元凶是 max_tokens 太小 → 代码必须大输出。
        let p = profile_for(TaskKind::Code);
        assert_eq!(p.effort, ThinkingEffort::Deep);
        assert!(p.max_tokens >= 32 * 1024, "代码任务输出应足够大防截断");
    }

    #[test]
    fn profile_reasoning_uses_max_effort() {
        let p = profile_for(TaskKind::Reasoning);
        assert_eq!(p.effort, ThinkingEffort::Max);
        assert_eq!(p.effort.reasoning_effort(), Some("high"));
        assert!(p.effort.wants_reasoning_prompt());
    }

    #[test]
    fn profile_long_uses_1m_context() {
        let p = profile_for(TaskKind::Long);
        assert_eq!(p.context_window, CONTEXT_WINDOW_1M);
    }

    #[test]
    fn profile_chat_keeps_small_budget() {
        let p = profile_for(TaskKind::Chat);
        assert_eq!(p.effort, ThinkingEffort::Off);
        assert_eq!(p.effort.reasoning_effort(), None);
        assert!(!p.effort.wants_reasoning_prompt());
    }

    #[test]
    fn effort_mapping_is_consistent() {
        assert_eq!(ThinkingEffort::Normal.reasoning_effort(), Some("low"));
        assert_eq!(ThinkingEffort::Deep.reasoning_effort(), Some("medium"));
        assert_eq!(ThinkingEffort::Max.reasoning_effort(), Some("high"));
    }

    #[test]
    fn supports_reasoning_effort_matches_deepseek() {
        assert!(supports_reasoning_effort("deepseek-v4-pro-0813"));
        assert!(supports_reasoning_effort("deepseek-reasoner"));
        assert!(!supports_reasoning_effort("gpt-4o"));
    }

    #[test]
    fn as_key_roundtrips_with_from_key() {
        // UI 档位键必须能「写出去再读回来」，否则滑块切一次就漂移。
        for e in [
            ThinkingEffort::Off,
            ThinkingEffort::Normal,
            ThinkingEffort::Deep,
            ThinkingEffort::Max,
        ] {
            assert_eq!(ThinkingEffort::from_key(e.as_key()), Some(e), "{}", e.as_key());
        }
    }

    #[test]
    fn from_key_accepts_aliases_and_rejects_typos() {
        // 别名表与 chat.send 的 wire 值同源（low/medium/high）
        assert_eq!(ThinkingEffort::from_key("low"), Some(ThinkingEffort::Normal));
        assert_eq!(ThinkingEffort::from_key("medium"), Some(ThinkingEffort::Deep));
        assert_eq!(ThinkingEffort::from_key("high"), Some(ThinkingEffort::Max));
        assert_eq!(ThinkingEffort::from_key("none"), Some(ThinkingEffort::Off));
        // 大小写/空白宽容
        assert_eq!(ThinkingEffort::from_key("  MAX "), Some(ThinkingEffort::Max));
        // typo 必须 None（调用方响亮拒绝），绝不静默降级成 Deep
        assert_eq!(ThinkingEffort::from_key("higg"), None);
        assert_eq!(ThinkingEffort::from_key(""), None);
    }

    #[test]
    fn as_key_is_distinct_from_wire_value() {
        // 两个口径不可混用：档位标识恒有值，wire 值 off 为 None。
        assert_eq!(ThinkingEffort::Off.as_key(), "off");
        assert_eq!(ThinkingEffort::Off.reasoning_effort(), None);
        assert_eq!(ThinkingEffort::Deep.as_key(), "deep");
        assert_eq!(ThinkingEffort::Deep.reasoning_effort(), Some("medium"));
    }

    #[test]
    fn from_spec_str_parses_mode_yaml_values() {
        // 内置模式 YAML 实际写的值必须能解析成最高档，否则「reasoning_effort: max」等于白写
        assert_eq!(ThinkingEffort::from_spec_str("max"), ThinkingEffort::Max);
        assert_eq!(ThinkingEffort::from_spec_str("high"), ThinkingEffort::Max);
        assert_eq!(
            ThinkingEffort::from_spec_str("medium"),
            ThinkingEffort::Deep
        );
        assert_eq!(ThinkingEffort::from_spec_str("deep"), ThinkingEffort::Deep);
        assert_eq!(ThinkingEffort::from_spec_str("low"), ThinkingEffort::Normal);
        assert_eq!(ThinkingEffort::from_spec_str("off"), ThinkingEffort::Off);
        assert_eq!(ThinkingEffort::from_spec_str(""), ThinkingEffort::Off);
        // 大小写与空白不敏感
        assert_eq!(ThinkingEffort::from_spec_str("  MAX "), ThinkingEffort::Max);
        // 未知值保守取 Deep（不静默降级成不思考）
        assert_eq!(
            ThinkingEffort::from_spec_str("ultra-turbo"),
            ThinkingEffort::Deep
        );
    }

    #[test]
    fn builtin_mode_specs_declare_parsable_effort() {
        // 端到端守卫：内置 Bloom / OmniTranscend 规格声明的 reasoning_effort
        // 必须真能解析出「会注入请求体」的档位（非 Off），否则深度声明形同虚设。
        for kind in crate::mode::ModeKind::ALL {
            let spec = crate::mode::spec::builtin::for_kind(*kind);
            if let Some(b) = spec.bloom.as_ref() {
                let e = ThinkingEffort::from_spec_str(&b.reasoning_effort);
                assert_ne!(
                    e,
                    ThinkingEffort::Off,
                    "模式 {} 的 reasoning_effort='{}' 解析为 Off，等于不思考",
                    kind.as_str(),
                    b.reasoning_effort
                );
                assert!(
                    e.reasoning_effort().is_some(),
                    "模式 {} 的思考档位必须能落到请求体",
                    kind.as_str()
                );
            }
        }
    }
}
