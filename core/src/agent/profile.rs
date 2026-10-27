//! 画像自动生成（P-11）
//!
//! 蒸馏 GUI_Agent `agent/memory/core_personality.py` 的人格维度
//! （identity/role、communication/tone、capabilities/expertise），
//! 以**确定性、抽取式**方式从对话/代码风格历史生成 `Persona`，
//! 写回 `agent::store`，使新 agent 默认从同类任务历史自动生成人设
//! （闭环「自定义性格」）。
//!
//! 诚实边界：本模块**不调用 LLM**，纯靠词法与标点统计抽取；
//! 历史为空或信号不足时返回中性默认人设，绝不编造。

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::agent::store as agent_store;
use crate::domain::{Message, Role};

// ── 角色信号词典（蒸馏 core_personality 的 identity/role 维度）─────────────
const ROLE_SIGNALS: &[(&str, &[&str])] = &[
    (
        "工程师",
        &[
            "代码",
            "函数",
            "编译",
            "terminal",
            "命令行",
            "file",
            "重构",
            "bug",
            "部署",
            "rust",
            "python",
            "java",
            "typescript",
            "后端",
            "前端",
        ],
    ),
    (
        "数据分析师",
        &[
            "数据",
            "sql",
            "图表",
            "统计",
            "分析",
            "可视化",
            "pandas",
            "excel",
            "指标",
        ],
    ),
    (
        "研究员",
        &["搜索", "调研", "资料", "web", "文献", "对比", "综述"],
    ),
    (
        "写作助手",
        &["文章", "文案", "文档", "报告", "写作", "润色", "总结"],
    ),
];

// ── 专长领域词典（蒸馏 core_personality 的 capabilities 维度）─────────────
const EXPERTISE_SIGNALS: &[&str] = &[
    "Rust",
    "Python",
    "Java",
    "TypeScript",
    "JavaScript",
    "C++",
    "前端",
    "后端",
    "数据库",
    "SQL",
    "DevOps",
    "Kubernetes",
    "Docker",
    "机器学习",
    "数据分析",
    "NLP",
    "安全",
    "密码学",
    "区块链",
];

const DEFAULT_ROLE: &str = "通用助手";
const DEFAULT_TONE: &str = "简洁专业";
const MAX_EXPERTISE: usize = 5;

/// 从对话/代码风格抽取出的人设画像。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Persona {
    /// 角色（蒸馏 identity.role）
    pub role: String,
    /// 语气（蒸馏 communication.style/verbosity）
    pub tone: String,
    /// 专长领域（蒸馏 capabilities）
    pub expertise: Vec<String>,
}

impl Persona {
    /// 中性默认人设：信号不足或历史为空时返回，绝不编造。
    pub fn neutral() -> Self {
        Persona {
            role: DEFAULT_ROLE.to_string(),
            tone: DEFAULT_TONE.to_string(),
            expertise: Vec::new(),
        }
    }

    /// 渲染为可注入 `store::AgentRecord.system_prompt` 的人设文本。
    pub fn system_prompt(&self) -> String {
        let mut parts = vec![format!("你是一个{}。", self.role)];
        parts.push(format!("语气风格：{}。", self.tone));
        if !self.expertise.is_empty() {
            parts.push(format!("专长领域：{}。", self.expertise.join("、")));
        }
        parts.join("\n")
    }
}

// ── 抽取工具 ──────────────────────────────────────────────

fn count_substrings(text: &str, needles: &[&str]) -> usize {
    let lower = text.to_lowercase();
    needles
        .iter()
        .map(|n| lower.matches(&n.to_lowercase()).count())
        .sum()
}

fn history_blob(history: &[Message]) -> String {
    history
        .iter()
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn detect_role(text: &str) -> String {
    let mut best: Option<(&str, usize)> = None;
    for (role, sigs) in ROLE_SIGNALS {
        let c = count_substrings(text, sigs);
        if c > 0 {
            best = match best {
                None => Some((role, c)),
                Some((_, bc)) if c > bc => Some((role, c)),
                other => other,
            };
        }
    }
    best.map(|(r, _)| r.to_string())
        .unwrap_or_else(|| DEFAULT_ROLE.to_string())
}

fn detect_tone(history: &[Message]) -> String {
    let assistant_text: String = history
        .iter()
        .filter(|m| matches!(m.role, Role::Assistant))
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if assistant_text.trim().is_empty() {
        return DEFAULT_TONE.to_string();
    }
    let polite = count_substrings(&assistant_text, &["请", "您", "谢谢", "感谢"]);
    let excited = assistant_text
        .chars()
        .filter(|c| *c == '！' || *c == '!')
        .count();
    let sentences: Vec<&str> = assistant_text
        .split(['。', '.', '\n'])
        .filter(|s| !s.trim().is_empty())
        .collect();
    let avg_len = if sentences.is_empty() {
        0.0
    } else {
        sentences.iter().map(|s| s.chars().count()).sum::<usize>() as f64 / sentences.len() as f64
    };
    let style = if excited >= 2 {
        "热情"
    } else if polite >= 2 {
        "礼貌"
    } else {
        "直接"
    };
    let verbosity = if avg_len < 25.0 {
        "简洁"
    } else if avg_len > 80.0 {
        "详尽"
    } else {
        "稳健"
    };
    format!("{}{}", style, verbosity)
}

fn detect_expertise(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut scored: Vec<(usize, &str)> = EXPERTISE_SIGNALS
        .iter()
        .map(|kw| (lower.matches(&kw.to_lowercase()).count(), *kw))
        .filter(|(c, _)| *c > 0)
        .collect();
    // 降序：计数多者在前；同计数按关键词词典序（确定性，避免抖动）
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(MAX_EXPERTISE)
        .map(|(_, kw)| kw.to_string())
        .collect()
}

// ── 公共 API ──────────────────────────────────────────────

/// 从会话历史确定性抽取人设（抽取式，不烧 token、不编造）。
///
/// 蒸馏 GUI_Agent `core_personality.py` 的人格三维度：
/// - `role` ← identity/role（按角色信号词典加权取最高）
/// - `tone` ← communication/style+verbosity（标点与句长统计）
/// - `expertise` ← capabilities（领域关键词频次，取 top-N）
pub fn auto_profile(history: &[Message]) -> Persona {
    if history.is_empty() {
        return Persona::neutral();
    }
    let text = history_blob(history);
    Persona {
        role: detect_role(&text),
        tone: detect_tone(history),
        expertise: detect_expertise(&text),
    }
}

/// 从同类任务历史生成人设并写回 `agent::store`（闭环「自定义性格」）。
///
/// 复用 `store::update`：内置智能体返回 `agent.builtin_immutable`、
/// 不存在返回 `agent.not_found`，与既有台账语义一致。
pub fn apply_auto_profile(agent_id: &str, history: &[Message]) -> Result<Persona, String> {
    let persona = auto_profile(history);
    let sp = persona.system_prompt();
    agent_store::update(&json!({ "id": agent_id, "system_prompt": sp })).map(|_| persona)
}

// ── 用户画像注入（此前只有 RPC 展示，从未真正注入对话）──────────────
//
// 诊断（2026-10-18）：`agent.profile.auto` 只把画像算出来返回给前端，聊天链路
// 里**没有任何地方消费它**；而 ExpertCenterView 却写着「经 persona 注入主智能体」。
// 这是「能跑但假的」典型：UI 承诺了一个不存在的链路。
//
// 修法不是删文案，而是把链路补真——但要守住两条边界：
// 1. **只用 user 消息**：把 assistant 的措辞算进「用户是谁」会系统性偏移；
// 2. **低置信度不注入**：样本不足或角色落回中性默认时不注入，宁可不猜也不误导。

/// 用户画像注入的最小样本量（低于此数一律不注入）。
pub const USER_PROFILE_MIN_SOURCES: usize = 20;

/// 画像缓存有效期：画像由词法统计得出，无需每轮重算；5 分钟粒度同时保证
/// 系统提示在窗口内**逐字节稳定**（避免每轮 prompt 前缀缓存全灭）。
const PROFILE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

type ProfileCache = std::sync::Mutex<Option<(std::time::Instant, Option<String>)>>;

fn profile_cache() -> &'static ProfileCache {
    static C: std::sync::OnceLock<ProfileCache> = std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::Mutex::new(None))
}

/// 取（带缓存的）用户画像提示片段；不需要注入时返回 `None`。
///
/// 供 `gateway::chat::agent_personality_hint` 在**每一条**对话路径上调用
/// （计划路径 / 编排路径 / 直连引擎路径共用同一入口，避免只修一条路径）。
pub fn cached_user_profile_hint() -> Option<String> {
    {
        let guard = profile_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, cached)) = guard.as_ref() {
            if at.elapsed() < PROFILE_CACHE_TTL {
                return cached.clone();
            }
        }
    }
    let fresh = compute_user_profile_hint();
    let mut guard = profile_cache().lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some((std::time::Instant::now(), fresh.clone()));
    fresh
}

/// 清空画像缓存（测试用：避免用例之间互相看到对方的上下文台账）。
#[cfg(test)]
pub fn reset_profile_cache_for_test() {
    *profile_cache().lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn compute_user_profile_hint() -> Option<String> {
    let mut user_messages: Vec<Message> = Vec::new();
    for cs in crate::context::session::context_sessions()
        .into_iter()
        .take(20)
    {
        for entry in crate::context::session::context_entries(&cs.session)
            .into_iter()
            .take(20)
        {
            if entry.role != "user" {
                continue;
            }
            if entry.text.trim().is_empty() {
                continue;
            }
            user_messages.push(Message {
                role: Role::User,
                content: entry.text,
                meta: None,
            });
        }
    }
    if user_messages.len() < USER_PROFILE_MIN_SOURCES {
        return None;
    }
    render_profile_hint(&user_messages)
}

/// 纯函数：由 user 消息渲染画像提示（样本不足或角色中性时返回 `None`）。
///
/// 抽成纯函数是为了让「不注入」的边界可被单测直接钉住——此前这段逻辑只存在于
/// RPC 展示层，没有任何断言守着「样本不足就不该注入」。
fn render_profile_hint(user_messages: &[Message]) -> Option<String> {
    if user_messages.len() < USER_PROFILE_MIN_SOURCES {
        return None;
    }
    let persona = auto_profile(user_messages);
    if persona.role == DEFAULT_ROLE || persona.expertise.is_empty() {
        return None;
    }
    Some(format!(
        "用户画像（由历史对话确定性抽取，仅用于适配表达，不要主动复述或据此下判断）：\
         角色倾向 {}；表达风格 {}；常用专长 {}。据此调整术语深度与举例，不必点明你在用画像。",
        persona.role,
        persona.tone,
        persona.expertise.join("、")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Message;

    fn msg(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
            meta: None,
        }
    }

    // ── 纯抽取：角色 ──
    #[test]
    fn auto_profile_empty_history_is_neutral() {
        let p = auto_profile(&[]);
        assert_eq!(p, Persona::neutral());
    }

    #[test]
    fn auto_profile_detects_engineer_role() {
        let h = vec![
            msg(Role::User, "帮我写个函数"),
            msg(Role::Assistant, "好的，我写一段 rust 代码并部署到后端。"),
        ];
        assert_eq!(auto_profile(&h).role, "工程师");
    }

    #[test]
    fn auto_profile_detects_data_analyst_role() {
        let h = vec![
            msg(Role::User, "分析下这份数据"),
            msg(Role::Assistant, "我用 sql 出图表，做统计可视化。"),
        ];
        assert_eq!(auto_profile(&h).role, "数据分析师");
    }

    #[test]
    fn auto_profile_detects_researcher_role() {
        let h = vec![
            msg(Role::User, "查点资料"),
            msg(Role::Assistant, "我搜索并调研文献，做了对比综述。"),
        ];
        assert_eq!(auto_profile(&h).role, "研究员");
    }

    #[test]
    fn auto_profile_detects_writer_role() {
        let h = vec![
            msg(Role::User, "写篇文章"),
            msg(Role::Assistant, "我帮你润色文案，整理成报告文档。"),
        ];
        assert_eq!(auto_profile(&h).role, "写作助手");
    }

    #[test]
    fn auto_profile_role_fallback_when_no_signals() {
        let h = vec![
            msg(Role::User, "你好"),
            msg(Role::Assistant, "你好，有什么可以帮你？"),
        ];
        assert_eq!(auto_profile(&h).role, DEFAULT_ROLE);
    }

    // ── 纯抽取：专长 ──
    #[test]
    fn auto_profile_detects_expertise_top_n_and_order() {
        let h = vec![msg(
            Role::Assistant,
            "我用 Python 写 Python 脚本，连 SQL 和 Rust 服务。",
        )];
        let ex = auto_profile(&h).expertise;
        assert_eq!(ex, vec!["Python", "Rust", "SQL"]);
    }

    #[test]
    fn auto_profile_expertise_deterministic_tie_break() {
        // Java 与 SQL 各出现一次 → 同计数，按关键词词典序确定（Java < SQL）
        let h = vec![msg(Role::Assistant, "涉及 Java 与 SQL 的系统。")];
        let ex = auto_profile(&h).expertise;
        assert_eq!(ex, vec!["Java", "SQL"]);
    }

    #[test]
    fn auto_profile_expertise_capped_at_max() {
        let h = vec![msg(
            Role::Assistant,
            "Rust Python Java TypeScript JavaScript C++ 前端 后端 数据库 SQL DevOps Kubernetes",
        )];
        let ex = auto_profile(&h).expertise;
        assert!(ex.len() <= MAX_EXPERTISE, "专长应被截断到 {MAX_EXPERTISE}");
        assert_eq!(ex.len(), MAX_EXPERTISE);
    }

    // ── 纯抽取：语气 ──
    #[test]
    fn auto_profile_detects_enthusiastic_tone() {
        let h = vec![msg(Role::Assistant, "搞定！很棒！我们继续！")];
        let t = auto_profile(&h).tone;
        assert!(t.contains("热情"), "实际语气：{t}");
        assert!(t.contains("简洁"), "实际语气：{t}");
    }

    #[test]
    fn auto_profile_detects_polite_tone() {
        let h = vec![msg(Role::Assistant, "请您确认。感谢您的耐心。")];
        let t = auto_profile(&h).tone;
        assert!(t.contains("礼貌"), "实际语气：{t}");
    }

    #[test]
    fn auto_profile_detects_direct_tone() {
        let h = vec![msg(Role::Assistant, "好的。")];
        let t = auto_profile(&h).tone;
        assert!(t.contains("直接"), "实际语气：{t}");
    }

    #[test]
    fn auto_profile_tone_default_when_no_assistant() {
        let h = vec![msg(Role::User, "帮我搜索一下资料")];
        assert_eq!(auto_profile(&h).tone, DEFAULT_TONE);
    }

    // ── 渲染与确定性 ──
    #[test]
    fn persona_system_prompt_format() {
        let p = Persona {
            role: "工程师".into(),
            tone: "直接简洁".into(),
            expertise: vec!["Rust".into(), "Python".into()],
        };
        let sp = p.system_prompt();
        assert!(sp.contains("你是一个工程师。"));
        assert!(sp.contains("语气风格：直接简洁。"));
        assert!(sp.contains("专长领域：Rust、Python。"));
    }

    #[test]
    fn auto_profile_is_deterministic() {
        let h = vec![
            msg(Role::User, "写个 python 后端函数部署"),
            msg(Role::Assistant, "好的，我写代码并测试！"),
        ];
        assert_eq!(auto_profile(&h), auto_profile(&h));
    }

    // ── 写回 store 接线 ──
    #[test]
    fn apply_auto_profile_writes_and_reflects() {
        let id = "auto-profile-it-1";
        let _ = agent_store::delete(id); // 清理跨运行残留
        let _ = agent_store::create(&json!({ "id": id, "name": "画像测试" }));
        let h = vec![
            msg(Role::User, "写个 rust 函数"),
            msg(Role::Assistant, "好的，我用 Python 和 Rust 写后端代码。"),
        ];
        let persona = apply_auto_profile(id, &h).expect("apply 应成功");
        assert_eq!(persona.role, "工程师");
        let injected = agent_store::personality_for(id).expect("应有人设");
        assert!(injected.contains("你是一个工程师。"), "实际：{injected}");
        assert!(injected.contains("Rust"), "实际：{injected}");
        let _ = agent_store::delete(id);
    }

    #[test]
    fn apply_auto_profile_rejects_builtin() {
        let h = vec![msg(Role::Assistant, "写代码 rust")];
        let err = apply_auto_profile("main", &h).unwrap_err();
        assert_eq!(err, "agent.builtin_immutable");
    }

    #[test]
    fn apply_auto_profile_rejects_missing_agent() {
        let h = vec![msg(Role::Assistant, "写代码 rust")];
        let err = apply_auto_profile("definitely-not-real-xyz", &h).unwrap_err();
        assert_eq!(err, "agent.not_found");
    }

    // ── 用户画像注入：低置信度必须「不注入」 ──────────────────────────

    /// 样本不足（< USER_PROFILE_MIN_SOURCES）时绝不注入——宁可不懂你，也不误导。
    #[test]
    fn profile_hint_skipped_below_min_sources() {
        let h: Vec<Message> = (0..USER_PROFILE_MIN_SOURCES - 1)
            .map(|_| msg(Role::User, "写个 rust 函数并部署到后端"))
            .collect();
        assert!(
            render_profile_hint(&h).is_none(),
            "样本 {} < {} 时不得注入",
            h.len(),
            USER_PROFILE_MIN_SOURCES
        );
    }

    /// 样本够但角色落回中性默认（无任何角色信号）时同样不注入。
    #[test]
    fn profile_hint_skipped_when_role_is_neutral() {
        let h: Vec<Message> = (0..USER_PROFILE_MIN_SOURCES)
            .map(|_| msg(Role::User, "嗯"))
            .collect();
        assert!(render_profile_hint(&h).is_none(), "中性角色不得注入");
    }

    /// 高置信度时注入，且内容包含角色与专长、并明确「不要复述画像」。
    #[test]
    fn profile_hint_injected_when_confident() {
        let h: Vec<Message> = (0..USER_PROFILE_MIN_SOURCES)
            .map(|_| msg(Role::User, "这个 rust 后端函数编译不过，帮我重构并加单测"))
            .collect();
        let hint = render_profile_hint(&h).expect("高置信度应注入");
        assert!(hint.contains("工程师"), "实际：{hint}");
        assert!(hint.contains("Rust"), "实际：{hint}");
        assert!(hint.contains("不要主动复述"), "必须禁止复述画像：{hint}");
    }

    /// 缓存命中：同一次调用两次得到逐字节相同的文本（保证 prompt 前缀缓存不被抖掉）。
    #[test]
    fn profile_hint_cache_is_byte_stable() {
        reset_profile_cache_for_test();
        let a = cached_user_profile_hint();
        let b = cached_user_profile_hint();
        assert_eq!(a, b, "缓存窗口内两次调用必须逐字节一致");
    }
}
