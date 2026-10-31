//! L1（全局）→ L2（项目）的**投影规则**：同一批事实，两层只取各自该看的那些。
//!
//! # 为什么要投影
//!
//! 此前 L1 与 L2 读的是同一份 brief，只是字符预算不同 —— 「全局助理」照样看得见
//! 本项目的文件细节，「项目助理」也照样看得见别的项目的内部约定。前者浪费上下文，
//! 后者是**跨项目污染**：换一个 workspace，上一家的部署约定还在里面。
//!
//! 本模块只做一件事：给定一层与一批事实，返回**这一层该留下的事实**，以及
//! **每条被丢事实的 `(fact_key, reason)`**。没有静默丢弃 —— 被挡下的事实都能解释。
//!
//! # 规则（逐条对应下面的测试）
//!
//! 1. **L1 只给不可推导的**：`source` 指向**当前 workspace 内**的文件（或
//!    `project:<当前项目名>#...`）的事实被丢弃（reason [`REASON_WORKSPACE_DERIVABLE`]）——
//!    这类内容项目里的文件自己就有，全局层再复述一遍只是噪声。
//!    保留 `user` / `profile` / `cross_project`（这些是项目文件推导不出来的）。
//! 2. **L2 只给本项目可用的**：保留 workspace 内来源 + `user` / `profile`；
//!    丢弃与本 workspace 无关的跨项目事实（reason [`REASON_FOREIGN_PROJECT`]）。
//!    判「相关」的口径：`source` 里出现 workspace 全路径或末段项目名即算相关
//!    （跨项目事实的出处形如 `cross_project:<项目名>#<文件>`）。
//! 3. **`pinned` 在任何层都不丢**：文本（去空白后）与任一钉住条目相同的**事实**
//!    享有最高优先权，连第 4 条的低分规则也让位 —— 用户亲手钉住的不能被机器挤掉
//!    （与 [`super::brief::compose_brief`] 里 pinned 不参与截断同一口径）。
//! 4. **`score < 0.35` 的事实任何层都丢**（reason [`REASON_LOW_SCORE`]），
//!    这是两层共同的地板：低分噪声不该因为换了层就复活。
//! 5. **禁止静默丢弃**：每条被丢事实都留 `(fact_key, reason)`；`fact_key` 形如
//!    `<source>::<归一化文本前 120 字>`。
//!
//! 判定顺序：**pinned 豁免 → 低分 → 层规则**（第 5 条之外没有别的前后歧义）。
//! 无法判定归属的来源（例如不含任何路径信息、也不带 `project:` 前缀的裸文件名、
//! 或 workspace 缺失）一律**保留**：规则只管「能证明该丢」的，不靠猜删数据。
//!
//! 纯函数：无 IO、无全局状态、不读配置文件，`Input` 进 `Outcome` 出，可直接单测。

use super::brief::{Fact, PinnedItem, Preference};
use super::scope::AssistantScope;

/// 低分地板：低于它的事实两层都丢（与 [`super`] 其它阈值一样集中在这里定义一次）。
pub const MIN_FACT_SCORE: f64 = 0.35;

/// 丢弃原因：分数低于 [`MIN_FACT_SCORE`]
pub const REASON_LOW_SCORE: &str = "low_score";
/// 丢弃原因：L1 判定该事实可由当前 workspace 内文件推导
pub const REASON_WORKSPACE_DERIVABLE: &str = "workspace_derivable";
/// 丢弃原因：L2 判定该事实与当前 workspace 无关
pub const REASON_FOREIGN_PROJECT: &str = "foreign_project";

/// 投影策略标识（进 `budget.notes`，便于事后解释「这条为什么被丢」）
pub const POLICY_L1: &str = "l1_unpredictable_only";
/// L2 策略标识
pub const POLICY_L2: &str = "l2_workspace_scoped";

/// 助理层级
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// L1：全局，跨项目
    L1Global,
    /// L2：项目，绑定某个 workspace
    L2Project,
}

impl Layer {
    /// 作用域 → 层级（`Global` → L1，`Project{..}` → L2）
    pub fn from_scope(scope: &AssistantScope) -> Layer {
        match scope {
            AssistantScope::Global => Layer::L1Global,
            AssistantScope::Project { .. } => Layer::L2Project,
        }
    }

    /// 策略标识（[`POLICY_L1`] / [`POLICY_L2`]）
    pub fn policy(self) -> &'static str {
        match self {
            Layer::L1Global => POLICY_L1,
            Layer::L2Project => POLICY_L2,
        }
    }
}

/// 投影的全部输入（纯数据；`preferences` / `open_loops` 原样带出，
/// 本层不做偏好或未完成事项的取舍 —— 只投影事实）。
#[derive(Debug, Clone)]
pub struct ProjectionInput {
    pub layer: Layer,
    /// 当前 workspace（L1 通常为 `None`；缺失时无法判定「项目内」）
    pub workspace: Option<String>,
    pub facts: Vec<Fact>,
    /// 用户钉住条目：文本相同的事实享有 pinned 豁免
    pub pinned: Vec<PinnedItem>,
    pub preferences: Vec<Preference>,
    pub open_loops: Vec<String>,
}

/// 投影结论：留下的事实 + 每条被丢事实的 `(fact_key, reason)` + 策略标识
#[derive(Debug, Clone)]
pub struct ProjectionOutcome {
    pub facts: Vec<Fact>,
    pub dropped: Vec<(String, &'static str)>,
    pub policy: &'static str,
}

/// 事实的来源类型（只用于决定哪层该丢，不对外暴露）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    /// `user`：用户直接说的，项目文件推导不出来
    User,
    /// `profile`：稳定画像
    Profile,
    /// `cross_project[:...]`：跨项目事实
    CrossProject,
    /// `project:<name>#...`：某个项目的记忆
    ProjectTagged,
    /// 文件路径 / 文件名（可能是某个 workspace 内的文件）
    WorkspaceFile,
    /// 其它未知来源（保留，不猜）
    Other,
}

/// 投影（纯函数）。
pub fn project(input: ProjectionInput) -> ProjectionOutcome {
    let ProjectionInput {
        layer,
        workspace,
        facts,
        pinned,
        ..
    } = input;
    let policy = layer.policy();
    let pinned_texts: Vec<String> = pinned.iter().map(|p| normalize_text(&p.text)).collect();
    let workspace = workspace.as_deref().filter(|w| !w.trim().is_empty());

    let mut kept = Vec::with_capacity(facts.len());
    let mut dropped: Vec<(String, &'static str)> = Vec::new();

    for fact in facts {
        let key = fact_key(&fact);
        let text = normalize_text(&fact.text);
        if pinned_texts.iter().any(|pinned| pinned == &text) {
            // 规则 3：pinned 豁免优先于一切（含低分地板）。
            kept.push(fact);
            continue;
        }
        if fact.score < MIN_FACT_SCORE {
            // 规则 4：低分两层都丢。
            dropped.push((key, REASON_LOW_SCORE));
            continue;
        }
        let verdict = match layer {
            // 规则 1：只丢「当前 workspace 内文件可推导」的那一类。
            Layer::L1Global => {
                matches!(origin_of(&fact.source), Origin::WorkspaceFile | Origin::ProjectTagged)
                    && workspace_says(&fact.source, workspace) == Some(true)
            }
            // 规则 2：只丢「能证明与本 workspace 无关」的那一类。
            Layer::L2Project => workspace_says(&fact.source, workspace) == Some(false),
        };
        if verdict {
            dropped.push((
                key,
                match layer {
                    Layer::L1Global => REASON_WORKSPACE_DERIVABLE,
                    Layer::L2Project => REASON_FOREIGN_PROJECT,
                },
            ));
        } else {
            kept.push(fact);
        }
    }

    ProjectionOutcome {
        facts: kept,
        dropped,
        policy,
    }
}

/// 事实的键：`<source>::<归一化文本前 120 字>`，与 `memory::writeback` 的 key 同构
/// （但不跨模块依赖，投影规则保持自洽）。
fn fact_key(fact: &Fact) -> String {
    let text = normalize_text(&fact.text);
    let text: String = text.chars().take(120).collect();
    format!("{}::{text}", fact.source.trim())
}

/// 文本归一化：去首尾与重复空白、转小写（与 `memory::writeback::normalize_text` 同口径，
/// 这里自带一份，避免投影规则依赖被别的写者维护的模块）。
fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// 来源分类（判定顺序：user → profile → cross_project → project: → 路径 → 其它）
fn origin_of(source: &str) -> Origin {
    let trimmed = source.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower == "user" || lower.starts_with("user:") {
        return Origin::User;
    }
    if lower == "profile" || lower.starts_with("profile:") {
        return Origin::Profile;
    }
    if lower.contains("cross_project") || lower.contains("cross-project") {
        return Origin::CrossProject;
    }
    if let Some(rest) = strip_prefix_ci(trimmed, "project:") {
        if !rest.trim().is_empty() {
            return Origin::ProjectTagged;
        }
    }
    if looks_like_path(trimmed) {
        return Origin::WorkspaceFile;
    }
    Origin::Other
}

/// 大小写不敏感地去前缀（只用于 ASCII 标记，不碰非 ASCII 字符边界）。
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let bytes = s.as_bytes();
    if bytes.len() >= prefix.len()
        && bytes[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
    {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// 像不像一个文件路径（含分隔符，或以常见源码/数据文件后缀结尾）
fn looks_like_path(source: &str) -> bool {
    if source.contains('/') || source.contains('\\') {
        return true;
    }
    let lower = source.to_ascii_lowercase();
    [
        ".md", ".jsonl", ".json", ".txt", ".rs", ".ts", ".tsx", ".vue", ".py", ".toml",
        ".yaml", ".yml",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

/// `project:<name>#...` 里的项目名（小写；取不到就是空串）
fn project_tag_name(source: &str) -> String {
    let rest = strip_prefix_ci(source.trim(), "project:")
        .unwrap_or(source)
        .trim();
    rest.split(['#', ':', '/', '\\'])
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// 路径归一化：去首尾、`\` → `/`、小写（Windows 路径大小写不敏感）。
fn path_norm(path: &str) -> String {
    path.trim().replace('\\', "/").to_ascii_lowercase()
}

/// 绝对路径判定（归一化后）：`/x` 或 `c:/x`
fn is_absolute(normalized: &str) -> bool {
    let bytes = normalized.as_bytes();
    normalized.starts_with('/')
        || (bytes.len() >= 3 && bytes[1] == b':' && bytes[2] == b'/')
}

/// **与当前 workspace 的关系**：
/// `Some(true)` = 出处指向本 workspace；`Some(false)` = 指向别处；
/// `None` = 无法判定（不猜，调用方按「保留」处理）。
fn workspace_says(source: &str, workspace: Option<&str>) -> Option<bool> {
    let ws = workspace?.trim();
    if ws.is_empty() {
        return None;
    }
    let ws_norm = path_norm(ws);
    let ws_name = ws_norm
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string();
    let src_norm = path_norm(source);
    match origin_of(source) {
        Origin::ProjectTagged => {
            let name = project_tag_name(source);
            if name.is_empty() {
                None
            } else {
                Some(name == ws_name)
            }
        }
        // 跨项目事实的出处里带项目名：出现本 workspace 全路径或项目名即算相关。
        Origin::CrossProject => {
            Some(src_norm.contains(&ws_norm) || (!ws_name.is_empty() && src_norm.contains(&ws_name)))
        }
        Origin::WorkspaceFile => {
            if src_norm.contains(&ws_norm) {
                Some(true)
            } else if is_absolute(&src_norm) {
                Some(false)
            } else {
                // 相对路径：不知道属于哪个项目，按「保留」处理。
                None
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WS: &str = "D:\\work\\app";

    fn fact(text: &str, source: &str, score: f64) -> Fact {
        Fact {
            text: text.to_string(),
            source: source.to_string(),
            score,
            id: None,
            deletable: false,
        }
    }

    fn pinned(text: &str) -> PinnedItem {
        PinnedItem {
            text: text.to_string(),
            source: "user".to_string(),
            at: "2026-10-31T00:00:00Z".to_string(),
        }
    }

    fn input(layer: Layer, workspace: Option<&str>, facts: Vec<Fact>) -> ProjectionInput {
        ProjectionInput {
            layer,
            workspace: workspace.map(str::to_string),
            facts,
            pinned: Vec::new(),
            preferences: Vec::new(),
            open_loops: Vec::new(),
        }
    }

    fn reasons(outcome: &ProjectionOutcome) -> Vec<&'static str> {
        outcome.dropped.iter().map(|(_, reason)| *reason).collect()
    }

    /// 规则 1：L1 丢弃「当前 workspace 内文件可推导」的事实（路径 + project 标签两种写法）。
    #[test]
    fn l1_drops_facts_derivable_from_current_workspace_files() {
        let outcome = project(input(
            Layer::L1Global,
            Some(WS),
            vec![
                fact("src/lib.rs 的主入口是 main()", "D:\\work\\app\\src\\lib.rs", 0.8),
                fact("本项目构建用 cargo", "project:app#project_memory.md", 0.7),
                fact("别的项目的构建约定", "project:other#project_memory.md", 0.6),
                fact("跨项目通用约定", "cross_project:shared#notes.md", 0.5),
            ],
        ));
        assert_eq!(outcome.facts.len(), 2, "只留下与别的项目/跨项目有关的两条");
        assert_eq!(reasons(&outcome), vec![REASON_WORKSPACE_DERIVABLE; 2]);
        assert_eq!(outcome.policy, POLICY_L1);
        assert!(outcome
            .facts
            .iter()
            .all(|f| f.source.starts_with("project:other") || f.source.starts_with("cross_project")));
    }

    /// 规则 1 的另一半：L1 保留 `user` / `cross_project` / `profile`（项目文件推导不出来）。
    #[test]
    fn l1_keeps_user_cross_project_and_profile_facts() {
        let outcome = project(input(
            Layer::L1Global,
            Some(WS),
            vec![
                fact("用户要求所有文档用中文", "user", 0.6),
                fact("用户禁用 force push", "profile:taboo", 0.5),
                fact("跨项目：部署只用 canary", "cross_project:notes.md", 0.55),
            ],
        ));
        assert_eq!(outcome.facts.len(), 3, "三条都该留下");
        assert!(outcome.dropped.is_empty());
        assert_eq!(outcome.policy, POLICY_L1);
    }

    /// 规则 2：L2 丢弃与本 workspace 无关的跨项目事实，留下本项目来源与用户/画像。
    #[test]
    fn l2_drops_cross_project_facts_unrelated_to_workspace() {
        let outcome = project(input(
            Layer::L2Project,
            Some(WS),
            vec![
                fact("别的项目的记忆", "project:other#project_memory.md", 0.8),
                fact("无关的跨项目事实", "cross_project:notes-9.md", 0.7),
                fact("本项目的记忆", "project:app#project_memory.md", 0.6),
                fact("本项目源码里的约定", "D:\\work\\app\\src\\lib.rs", 0.7),
                fact("用户要求中文", "user", 0.5),
                fact("用户禁用 force push", "profile:taboo", 0.5),
            ],
        ));
        assert_eq!(outcome.facts.len(), 4, "本项目来源 + 用户/画像留下");
        assert_eq!(reasons(&outcome), vec![REASON_FOREIGN_PROJECT; 2]);
        assert_eq!(outcome.policy, POLICY_L2);
        assert!(outcome.dropped.iter().all(|(key, _)| {
            key.starts_with("project:other#project_memory.md::")
                || key.starts_with("cross_project:notes-9.md::")
        }));
    }

    /// 规则 3：pinned 在两层都不丢 —— 包括本会被层规则/低分挡下的两个最坏情形。
    #[test]
    fn pinned_facts_survive_both_layers() {
        let l1_facts = vec![fact("本项目入口是 main()", "D:\\work\\app\\src\\lib.rs", 0.9)];
        let mut l1 = input(Layer::L1Global, Some(WS), l1_facts);
        l1.pinned = vec![pinned("本项目入口是 main()")];
        let l1_outcome = project(l1);
        assert_eq!(l1_outcome.facts.len(), 1, "pinned 事实不得被 L1 的推导规则丢掉");
        assert!(l1_outcome.dropped.is_empty());

        let l2_facts = vec![
            fact("无关跨项目事实", "cross_project:notes-9.md", 0.8),
            fact("低分但用户亲手钉住", "cross_project:notes-9.md", 0.1),
        ];
        let mut l2 = input(Layer::L2Project, Some(WS), l2_facts);
        l2.pinned = vec![pinned("无关跨项目事实"), pinned("低分但用户亲手钉住")];
        let l2_outcome = project(l2);
        assert_eq!(l2_outcome.facts.len(), 2, "pinned 连低分地板也让位");
        assert!(l2_outcome.dropped.is_empty());
    }

    /// 规则 4：`score < 0.35` 的事实两层都丢，reason 必须是 `low_score`。
    #[test]
    fn low_score_facts_are_dropped_in_both_layers() {
        let facts = vec![
            fact("刚好低一分", "user", 0.34),
            fact("明显噪声", "cross_project:notes.md", 0.2),
            fact("零分", "profile", 0.0),
            fact("刚好过线", "user", 0.35),
        ];
        for layer in [Layer::L1Global, Layer::L2Project] {
            let outcome = project(input(layer, Some(WS), facts.clone()));
            assert_eq!(outcome.facts.len(), 1, "{layer:?} 只留 0.35 那条");
            assert_eq!(outcome.facts[0].score, 0.35);
            assert_eq!(reasons(&outcome), vec![REASON_LOW_SCORE; 3]);
        }
    }

    /// 规则 5：每条被丢事实都留 `(fact_key, reason)`，键里带出处，便于事后解释。
    #[test]
    fn every_dropped_fact_leaves_a_key_and_reason() {
        let outcome = project(input(
            Layer::L2Project,
            Some(WS),
            vec![
                fact("别的项目的事", "project:other#project_memory.md", 0.8),
                fact("低分噪声", "cross_project:notes.md", 0.1),
            ],
        ));
        assert_eq!(outcome.dropped.len(), 2, "两条被丢的必须都留痕");
        let (key0, reason0) = &outcome.dropped[0];
        assert_eq!(*reason0, REASON_FOREIGN_PROJECT);
        assert!(
            key0.starts_with("project:other#project_memory.md::"),
            "fact_key 必须带出处：{key0}"
        );
        assert!(key0.contains("别的项目的事"), "fact_key 必须能定位内容：{key0}");
        let (key1, reason1) = &outcome.dropped[1];
        assert_eq!(*reason1, REASON_LOW_SCORE);
        assert!(
            key1.starts_with("cross_project:notes.md::"),
            "fact_key 必须带出处：{key1}"
        );
    }
}
