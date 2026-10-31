//! 两层助理的「简报」（`assistant.brief.get` 的载荷）—— 计划 §4.2。
//!
//! 简报是**结构化**的，不是自由文本：每条都带出处与分值，前端可以展开「凭什么说这句」。
//!
//! 结构：
//! ```json
//! {"scope":"project","pinned":[],"preferences":[],"facts":[],"open_loops":[],
//!  "budget":{"max_chars":1200,"used":0},"truncated":false}
//! ```
//!
//! 裁剪顺序（**硬约束**）：`pinned → preferences → facts（按 score 降序）→ open_loops`；
//! `pinned` 永远排最前且不参与截断 —— 那是用户亲手钉住的偏好，不能被机器挤掉。
//! 裁剪事实要记日志（`truncated` + `notes`），不静默吞掉。
//!
//! # 投影先于裁剪
//!
//! 事实进预算之前先过 [`super::projection::project`]：L1 只留不可推导的、L2 只留本项目可用的。
//! 被投影丢掉的事实以 `(fact_key, reason)` 记在 [`BriefInput::dropped_facts`]，
//! 条数与原因写进 `budget.notes`（**没有就不写该字段**，不塞空数组）。
//!
//! 做过投影的简报另有结构化的 `projection` 字段（`{policy, droppedCount, dropped}`，
//! 每条 `dropped` 是 `{key, reason}`），供前端溯源抽屉逐条展示；`projection_policy`
//! 为 `None`（没投影过）时该字段整体缺席。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::projection::{self, Layer, ProjectionInput};
use super::AssistantScope;

/// 默认字符预算（计划 §4.2：`budget.max_chars`）
pub const DEFAULT_MAX_CHARS: usize = 1200;

/// 一条用户稳定偏好（由 [`super::profile::UserProfile`] 产出）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preference {
    /// `language` / `stack` / `taboo`
    pub key: String,
    pub value: String,
    /// 0.0–1.0：语言与禁忌为硬偏好（1.0），技术栈略低（0.9）
    pub confidence: f64,
    /// 支撑条数（当前画像按条目计，手改文件后仍是 1）
    pub evidence: u32,
}

/// 一条跨项目事实（带出处与分值，可追溯）
///
/// `id` / `deletable` 是**可撤销性**（R-02）的两个字段：
/// - `id` 只对本引擎写回层的事实存在（形如 `fact:<记录 id>`），外部工具写的
///   Markdown 记忆没有可撤销标识 —— 那里不伪造 id，前端也就不显示删除按钮；
/// - `deletable` 显式声明「这条能不能删」。**不靠 id 是否存在去推断**：
///   将来若给外部来源也发了 id，删除语义仍然由这里说了算。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub text: String,
    /// `project:<name>#<会话/版本>` / `cross_project:<file>` 之类
    pub source: String,
    pub score: f64,
    /// 可撤销标识（仅本引擎写回层的事实）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// 是否允许用户删除
    #[serde(default)]
    pub deletable: bool,
}

/// 用户手工钉住的条目（最高优先级，不参与截断）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PinnedItem {
    pub text: String,
    /// 目前只有 `user`
    pub source: String,
    /// RFC3339
    pub at: String,
}

/// 合成简报的全部输入（纯数据，便于单测与替换来源）
#[derive(Debug, Clone, Default)]
pub struct BriefInput {
    /// `"global"` | `"project"`
    pub scope: String,
    pub workspace: Option<String>,
    pub pinned: Vec<PinnedItem>,
    pub preferences: Vec<Preference>,
    pub facts: Vec<Fact>,
    pub open_loops: Vec<String>,
    pub max_chars: Option<usize>,
    /// 投影阶段被丢弃的事实：`(fact_key, reason)`（见 [`super::projection`]）。
    /// 非空时逐条写进 `budget.notes`，**绝不静默丢弃**。
    pub dropped_facts: Vec<(String, &'static str)>,
    /// 投影策略标识（`l1_unpredictable_only` / `l2_workspace_scoped`）。
    /// 为 `Some` 时出参带结构化 `projection`（并随 `notes` 出现）；`None` 时两处都不出现。
    pub projection_policy: Option<&'static str>,
}

/// `assistant_pins.json` 的落盘形态
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PinsFile {
    #[serde(default)]
    items: Vec<PinnedItem>,
}

fn pins_path() -> std::path::PathBuf {
    crate::persistence::config::Config::config_dir().join("assistant_pins.json")
}

/// 读取用户钉住的条目（缺失 / 损坏 → 空表，不挡道）
pub fn load_pins() -> Vec<PinnedItem> {
    std::fs::read_to_string(pins_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<PinsFile>(&raw).ok())
        .map(|f| f.items)
        .unwrap_or_default()
}

/// 写回钉住条目（原子写；`assistant.prefs.pin` / `unpin` 的落点）。
///
/// 去重按 `text` 精确匹配（同一句钉两次没有意义），保持插入顺序。
pub fn save_pins(items: &[PinnedItem]) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    let deduped: Vec<PinnedItem> = items
        .iter()
        .filter(|p| seen.insert(p.text.clone()))
        .cloned()
        .collect();
    let body = serde_json::to_string_pretty(&PinsFile { items: deduped })
        .map_err(|e| format!("序列化失败: {e}"))?;
    super::write_atomic_text(&pins_path(), &body)
}

/// 合成简报。`max_chars = None` 时用 [`DEFAULT_MAX_CHARS`]。
pub fn compose_brief(input: BriefInput) -> Value {
    let max_chars = input.max_chars.unwrap_or(DEFAULT_MAX_CHARS);

    let mut used = 0usize;
    let mut truncated = false;

    // 1) pinned：永远全留（预算不够也留，这是用户意图）
    let pinned: Vec<Value> = input
        .pinned
        .iter()
        .map(|p| json!({"text": p.text, "source": p.source, "at": p.at}))
        .collect();
    for p in &input.pinned {
        used += p.text.chars().count();
    }
    let pinned_overflow = used > max_chars;

    // 2) preferences
    let mut prefs = Vec::new();
    if !pinned_overflow {
        for p in &input.preferences {
            let cost = p.key.chars().count() + p.value.chars().count();
            if used + cost > max_chars {
                truncated = true;
                break;
            }
            used += cost;
            prefs.push(json!({
                "key": p.key, "value": p.value,
                "confidence": p.confidence, "evidence": p.evidence,
            }));
        }
    } else {
        truncated = true;
    }

    // 3) facts：按 score 降序，装不下就停
    let mut facts_sorted = input.facts.clone();
    facts_sorted.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut facts = Vec::new();
    if !pinned_overflow {
        for f in &facts_sorted {
            let cost = f.text.chars().count();
            if used + cost > max_chars {
                truncated = true;
                break;
            }
            used += cost;
            let mut item = json!({"text": f.text, "source": f.source, "score": f.score});
            // 只有真能撤销的事实才带 id/deletable —— 不塞 `null` 占位，
            // 前端 `if (fact.deletable)` 就能直接决定是否渲染删除入口。
            if let Some(id) = &f.id {
                item["id"] = json!(id);
            }
            if f.deletable {
                item["deletable"] = json!(true);
            }
            facts.push(item);
        }
    }

    // 4) open_loops 最后
    let mut open_loops = Vec::new();
    if !pinned_overflow {
        for item in &input.open_loops {
            let cost = item.chars().count();
            if used + cost > max_chars {
                truncated = true;
                break;
            }
            used += cost;
            open_loops.push(json!(item));
        }
    }

    let mut budget = json!({"max_chars": max_chars, "used": used});
    if !input.dropped_facts.is_empty() {
        budget["notes"] = json!(projection_notes(
            input.projection_policy,
            &input.dropped_facts
        ));
    }

    let mut out = json!({
        "scope": input.scope,
        "workspace": input.workspace,
        "pinned": pinned,
        "preferences": prefs,
        "facts": facts,
        "open_loops": open_loops,
        "budget": budget,
        "truncated": truncated,
    });

    // 只在真的做过投影时写 `projection`；`None` 时整字段缺席，
    // 免得调用方把「没投影」误读成「投影过且一条没丢」。
    if let Some(policy) = input.projection_policy {
        let dropped: Vec<Value> = input
            .dropped_facts
            .iter()
            .map(|(key, reason)| json!({"key": key, "reason": *reason}))
            .collect();
        out["projection"] = json!({
            "policy": policy,
            "droppedCount": dropped.len(),
            "dropped": dropped,
        });
    }

    out
}

/// 投影丢弃记录 → `budget.notes` 文案：一行汇总（策略 + 条数 + 原因分布）
/// 加逐条明细（`(fact_key, reason)`），保证每条被丢事实都能被解释。
///
/// `notes` 只是**诊断信息**，不占字符预算（不参与模型上下文）。
fn projection_notes(
    policy: Option<&'static str>,
    dropped: &[(String, &'static str)],
) -> Vec<String> {
    let policy = policy.unwrap_or("projection");
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    for (_, reason) in dropped {
        match counts.iter_mut().find(|(known, _)| *known == *reason) {
            Some((_, count)) => *count += 1,
            None => counts.push((*reason, 1)),
        }
    }
    let breakdown = counts
        .iter()
        .map(|(reason, count)| format!("{reason}×{count}"))
        .collect::<Vec<_>>()
        .join("、");
    let mut notes = vec![format!(
        "projection({policy}): 丢弃 {} 条事实（{breakdown}）",
        dropped.len()
    )];
    for (key, reason) in dropped {
        notes.push(format!("dropped({reason}): {key}"));
    }
    notes
}

/// 从既有跨项目记忆里尽力取事实；取不到就是空表，**绝不**编造。
///
/// `cross_project_search` 返回的是网关形状的 JSON，这里做**宽容解析**：
/// 认 `items` / `hits` / `results` / `data` 四种数组名，条目认 `excerpt` / `text` / `snippet` 字段。
///
/// > 2026-10-31 修：此前只认 `hits/results/data`，而 `cross_project_search` 实际返回
/// > `{"items": […], "root": …}` —— 于是生产路径上 `facts` **恒为空**，投影与裁剪两条
/// > 分支都成了死代码（单测用 `BriefInput` 直接喂数据，所以没抓到）。`items` 必须排在首位。
///
/// 出处的拼法（供 [`super::projection`] 判定归属）：命中带 `project` + `file` 时是
/// `project:<项目名>#<文件>`（与模块文档里的 `source` 约定一致）；带显式 `source` 时原样用；
/// 都没有才回落 `cross_project`。没有项目名的出处无法投影 —— L2 会把它们当「无法判定」保留。
///
/// **可撤销性**：只有写回层的事实（`kind == "fact"`，`line` 是记录 id）能给出 `id` 并
/// 标记 `deletable`。外部工具写的 Markdown 记忆不归本应用写，删了是越权 —— 那里
/// 不给 id，也不谎称可删。
fn collect_facts(query: &str, limit: usize) -> Vec<Fact> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let params = json!({"query": query, "limit": limit});
    let resp = crate::memory::cross_project::cross_project_search(&params);
    parse_facts(&resp)
}

/// [`collect_facts`] 的纯解析半段（可单测：喂一份真实的 `cross_project_search` 响应即可）。
///
/// 拆出来的理由：`collect_facts` 会读真实记忆根目录，而单测不允许碰用户目录；
/// 键名不匹配这个 bug 恰恰只能在这一层抓到 —— 它不会 panic，只是静默返回空表。
fn parse_facts(resp: &Value) -> Vec<Fact> {
    let arr = ["items", "hits", "results", "data"]
        .iter()
        .find_map(|k| resp.get(*k).and_then(Value::as_array))
        .cloned()
        .unwrap_or_default();

    let mut out: Vec<Fact> = Vec::new();
    for item in arr {
        let text = ["excerpt", "text", "snippet"]
            .iter()
            .find_map(|k| item.get(*k).and_then(Value::as_str))
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            continue;
        }
        let explicit = ["source", "path"]
            .iter()
            .find_map(|k| item.get(*k).and_then(Value::as_str))
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let project = item
            .get("project")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let file = item
            .get("file")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let source = match (explicit, project, file) {
            (Some(s), _, _) => s.to_string(),
            (None, Some(p), Some(f)) => format!("project:{p}#{f}"),
            (None, Some(p), None) => format!("project:{p}"),
            (None, None, Some(f)) => f.to_string(),
            (None, None, None) => "cross_project".to_string(),
        };
        let score = item.get("score").and_then(Value::as_f64).unwrap_or(0.0);
        // 写回层事实：`kind=fact` + 固定文件名 + `line` 即记录 id。
        let is_own_fact = item.get("kind").and_then(Value::as_str) == Some("fact")
            && file == Some(crate::memory::writeback::FACT_FILE);
        let id = if is_own_fact {
            item.get("line")
                .and_then(Value::as_u64)
                .map(|line| format!("fact:{line}"))
        } else {
            None
        };
        // 同一段话可能被多个来源命中（同项目不同文件）：按正文去重，先到先得，
        // 但**优先保留可撤销的那一条**，否则用户会看到两条一样的事实、只有一条能删。
        if let Some(existing) = out.iter_mut().find(|f| f.text == text) {
            if existing.id.is_none() && id.is_some() {
                existing.id = id;
                existing.deletable = true;
                existing.source = source;
            }
            continue;
        }
        out.push(Fact {
            text,
            source,
            score,
            deletable: id.is_some(),
            id,
        });
    }
    out
}

/// `assistant.brief.get` 的实现：按当前作用域合成简报。
///
/// - `Global`：只带 pinned + preferences + 跨项目 facts（不碰任何项目内文件）
/// - `Project{workspace}`：同上，facts 的查询会带上 workspace 名做限定
///
/// 事实先过 [`projection::project`]（`AssistantScope` → [`Layer`]），再进
/// [`compose_brief`] 做字符预算裁剪；投影丢弃的条数与原因随 `budget.notes` 透出。
pub fn build_brief(scope: &AssistantScope, query: &str, max_chars: Option<usize>) -> Value {
    let profile = super::profile::UserProfile::load();
    let workspace = scope.workspace().map(|p| p.to_string_lossy().to_string());
    let scoped_query = match (&workspace, query.trim()) {
        (Some(ws), "") => ws.rsplit(['/', '\\']).next().unwrap_or(ws).to_string(),
        (_, q) => q.to_string(),
    };

    let pinned = load_pins();
    let preferences = profile.to_preferences();
    let outcome = projection::project(ProjectionInput {
        layer: Layer::from_scope(scope),
        workspace: workspace.clone(),
        facts: collect_facts(&scoped_query, 6),
        pinned: pinned.clone(),
        preferences: preferences.clone(),
        open_loops: Vec::new(),
    });

    compose_brief(BriefInput {
        scope: scope.kind().to_string(),
        workspace,
        pinned,
        preferences,
        facts: outcome.facts,
        open_loops: Vec::new(),
        max_chars,
        dropped_facts: outcome.dropped,
        projection_policy: Some(outcome.policy),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iso(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "marguerite_assist_brief_{}_{}",
            tag,
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        crate::persistence::config::Config::set_config_dir_override(dir.clone());
        dir
    }

    fn pref(key: &str, value: &str) -> Preference {
        Preference {
            key: key.to_string(),
            value: value.to_string(),
            confidence: 0.9,
            evidence: 1,
        }
    }

    #[test]
    fn empty_input_yields_legal_brief_with_zero_usage() {
        let v = compose_brief(BriefInput {
            scope: "global".to_string(),
            ..Default::default()
        });
        assert_eq!(v["scope"], "global");
        assert_eq!(v["budget"]["used"], 0);
        assert_eq!(v["truncated"], false);
        assert!(v["pinned"].as_array().unwrap().is_empty());
        assert!(v["facts"].as_array().unwrap().is_empty());
    }

    #[test]
    fn pinned_is_never_truncated_even_over_budget() {
        let v = compose_brief(BriefInput {
            scope: "project".to_string(),
            pinned: vec![PinnedItem {
                text: "所有文档用中文，且不要 git commit".to_string(),
                source: "user".to_string(),
                at: "2026-10-30T00:00:00Z".to_string(),
            }],
            preferences: vec![pref("language", "中文")],
            max_chars: Some(10),
            ..Default::default()
        });
        assert_eq!(v["pinned"].as_array().unwrap().len(), 1);
        assert_eq!(
            v["preferences"].as_array().unwrap().len(),
            0,
            "预算被 pinned 吃光后，偏好必须让位"
        );
        assert_eq!(v["truncated"], true);
    }

    #[test]
    fn budget_trims_in_documented_order() {
        // 预算刚好装下 pinned + 一条偏好，事实与 open_loops 必须被裁掉
        let v = compose_brief(BriefInput {
            scope: "project".to_string(),
            pinned: vec![PinnedItem {
                text: "中文".to_string(),
                source: "user".to_string(),
                at: "2026-10-30T00:00:00Z".to_string(),
            }],
            preferences: vec![pref("stack", "Rust")],
            facts: vec![Fact {
                text: "0.2.19 修了启动 panic".to_string(),
                source: "project:marguerite".to_string(),
                score: 0.8,
                id: None,
                deletable: false,
            }],
            open_loops: vec!["sessions.fork 未实现".to_string()],
            // 预算刚好 = pinned(2) + 一条偏好(key 5 + value 4 = 9) = 11，留 1 字符容差到 12；
            // 事实（>12 字符）与 open_loops 必须被裁掉。
            max_chars: Some(12),
            ..Default::default()
        });
        assert_eq!(v["pinned"].as_array().unwrap().len(), 1);
        assert_eq!(
            v["preferences"].as_array().unwrap().len(),
            1,
            "偏好先于事实"
        );
        assert!(v["facts"].as_array().unwrap().is_empty(), "事实被裁掉");
        assert!(v["open_loops"].as_array().unwrap().is_empty());
        assert_eq!(v["truncated"], true);
    }

    #[test]
    fn facts_are_sorted_by_score_desc() {
        let v = compose_brief(BriefInput {
            scope: "global".to_string(),
            facts: vec![
                Fact {
                    text: "低分".to_string(),
                    source: "a".to_string(),
                    score: 0.3,
                    id: None,
                    deletable: false,
                },
                Fact {
                    text: "高分".to_string(),
                    source: "b".to_string(),
                    score: 0.9,
                    id: None,
                    deletable: false,
                },
            ],
            ..Default::default()
        });
        let facts = v["facts"].as_array().unwrap();
        assert_eq!(facts[0]["text"], "高分");
        assert_eq!(facts[1]["text"], "低分");
    }

    #[test]
    fn build_brief_without_any_state_is_legal_and_reports_empty_budget() {
        let _dir = iso("empty");
        let scope = AssistantScope::Global;
        let v = build_brief(&scope, "", None);
        assert_eq!(v["scope"], "global");
        assert_eq!(v["budget"]["used"], 0);
        assert_eq!(v["budget"]["max_chars"], DEFAULT_MAX_CHARS);
        assert!(!v["truncated"].as_bool().unwrap());
    }

    /// 投影丢弃的条数与原因必须写进 `budget.notes`，且与 `dropped_facts` 一一对得上。
    #[test]
    fn projection_drops_are_reported_in_budget_notes() {
        let dropped: Vec<(String, &'static str)> = vec![
            ("project:other::别的项目".to_string(), "foreign_project"),
            ("project:other::另一条".to_string(), "foreign_project"),
            ("cross_project:x::低分噪声".to_string(), "low_score"),
        ];
        let v = compose_brief(BriefInput {
            scope: "project".to_string(),
            facts: vec![Fact {
                text: "本项目的约定".to_string(),
                source: "project:app#project_memory.md".to_string(),
                score: 0.6,
                id: None,
                deletable: false,
            }],
            dropped_facts: dropped.clone(),
            projection_policy: Some(super::projection::POLICY_L2),
            ..Default::default()
        });

        assert_eq!(v["facts"].as_array().unwrap().len(), 1, "留下的只有一条");
        let notes = v["budget"]["notes"]
            .as_array()
            .expect("有丢弃时 budget.notes 必须在");
        assert_eq!(
            notes.len(),
            dropped.len() + 1,
            "一行汇总 + 每条被丢事实一行"
        );
        let summary = notes[0].as_str().unwrap_or("");
        assert!(
            summary.contains(&format!("丢弃 {} 条事实", dropped.len())),
            "汇总行必须给出条数：{summary}"
        );
        assert!(
            summary.contains("foreign_project×2") && summary.contains("low_score×1"),
            "汇总行必须给出原因分布：{summary}"
        );
        assert!(
            summary.contains(super::projection::POLICY_L2),
            "汇总行必须带策略标识：{summary}"
        );
        for (key, reason) in &dropped {
            let found = notes.iter().any(|note| {
                let text = note.as_str().unwrap_or("");
                text.contains(key) && text.contains(reason)
            });
            assert!(found, "被丢事实 {key}（{reason}）必须在 notes 里留痕");
        }
    }

    /// 没有丢弃就不写 `budget.notes`（不塞空数组）。
    #[test]
    fn brief_omits_budget_notes_when_nothing_was_dropped() {
        let v = compose_brief(BriefInput {
            scope: "global".to_string(),
            facts: vec![Fact {
                text: "跨项目事实".to_string(),
                source: "cross_project:notes.md".to_string(),
                score: 0.9,
                id: None,
                deletable: false,
            }],
            projection_policy: Some(super::projection::POLICY_L1),
            ..Default::default()
        });
        assert!(
            v["budget"].get("notes").is_none(),
            "没有丢弃时 budget.notes 必须省略：{}",
            v["budget"]
        );
    }

    /// 做过投影（policy 有值）但一条没丢 → `projection` 仍在，`dropped` 是空数组。
    #[test]
    fn projection_policy_present_yields_structured_empty_dropped() {
        let v = compose_brief(BriefInput {
            scope: "global".to_string(),
            projection_policy: Some(super::projection::POLICY_L1),
            ..Default::default()
        });
        let projection = v
            .get("projection")
            .expect("policy 有值时 projection 字段必须在");
        assert_eq!(projection["policy"], super::projection::POLICY_L1);
        assert_eq!(projection["droppedCount"], 0);
        let dropped = projection["dropped"]
            .as_array()
            .expect("dropped 必须是数组（空数组而不是缺席）");
        assert!(dropped.is_empty());
    }

    /// 每条被丢事实都保留 key 与 reason，顺序与入参一致（确定性）。
    #[test]
    fn projection_dropped_entries_keep_key_and_reason() {
        let dropped: Vec<(String, &'static str)> = vec![
            (
                "project:marguerite#src/lib.rs::某条事实".to_string(),
                super::projection::REASON_FOREIGN_PROJECT,
            ),
            (
                "cross_project:shared#notes.md::低分噪声".to_string(),
                super::projection::REASON_LOW_SCORE,
            ),
        ];
        let v = compose_brief(BriefInput {
            scope: "project".to_string(),
            dropped_facts: dropped.clone(),
            projection_policy: Some(super::projection::POLICY_L2),
            ..Default::default()
        });
        let projection = v.get("projection").expect("projection 字段必须在");
        assert_eq!(projection["policy"], super::projection::POLICY_L2);
        assert_eq!(projection["droppedCount"], dropped.len());
        let entries = projection["dropped"].as_array().unwrap();
        assert_eq!(entries.len(), dropped.len());
        for (entry, (key, reason)) in entries.iter().zip(dropped.iter()) {
            assert_eq!(entry["key"], key.as_str(), "key 必须逐字保留");
            assert_eq!(entry["reason"], *reason, "reason 必须逐字保留");
        }
    }

    /// 没做过投影（policy 为 `None`）→ 出参里**没有** `projection` 键。
    #[test]
    fn compose_brief_without_policy_omits_projection_field() {
        let v = compose_brief(BriefInput {
            scope: "global".to_string(),
            ..Default::default()
        });
        assert!(
            v.get("projection").is_none(),
            "没有 policy 时不得凭空造 projection 字段：{v}"
        );
    }

    /// `build_brief` 两条路径的策略必须分层：全局 → L1，项目 → L2。
    #[test]
    fn build_brief_reports_layer_specific_policy() {
        let dir = iso("policy");
        let global = build_brief(&AssistantScope::Global, "", None);
        assert_eq!(
            global["projection"]["policy"],
            super::projection::POLICY_L1,
            "全局简报必须是 L1 策略"
        );

        let ws = dir.join("proj");
        std::fs::create_dir_all(&ws).unwrap();
        let project = build_brief(&AssistantScope::Project { workspace: ws }, "", None);
        assert_eq!(
            project["projection"]["policy"],
            super::projection::POLICY_L2,
            "项目简报必须是 L2 策略"
        );
    }

    /// 回归：`cross_project_search` 的真实返回键是 `items`，不是 `hits`。
    ///
    /// 这个 bug 不会 panic —— 它让线上 `facts` 恒为空、投影与裁剪静默失效，
    /// 只有把「真实响应形状」喂进解析器才抓得到。
    #[test]
    fn parse_facts_reads_items_key_from_real_search_shape() {
        let resp = json!({
            "items": [{
                "project": "marguerite",
                "file": "project_memory.md",
                "kind": "note",
                "title": "构建约定",
                "line": 12,
                "excerpt": "Windows 上必须先 export cargo 的 PATH",
                "score": 0.71,
                "updated_at": "1761000000"
            }],
            "root": "C:\\Users\\x\\.trae-cn\\memory\\projects"
        });
        let facts = parse_facts(&resp);
        assert_eq!(facts.len(), 1, "items 必须被解析（旧实现认 hits/results/data，静默返回空）");
        assert_eq!(facts[0].source, "project:marguerite#project_memory.md");
        assert!((facts[0].score - 0.71).abs() < 1e-9);
        assert!(
            !facts[0].deletable && facts[0].id.is_none(),
            "外部工具的 Markdown 记忆不归本应用写，不得发可撤销 id"
        );
    }

    /// 写回层事实必须带 `fact:<id>`，并标记可删；兼容旧键名 `hits`。
    #[test]
    fn parse_facts_marks_writeback_facts_deletable() {
        let resp = json!({
            "items": [{
                "project": "app",
                "file": crate::memory::writeback::FACT_FILE,
                "kind": "fact",
                "title": "app",
                "line": 42,
                "excerpt": "用户偏好：先给结论再给理由",
                "score": 0.9
            }]
        });
        let facts = parse_facts(&resp);
        assert_eq!(facts[0].id.as_deref(), Some("fact:42"));
        assert!(facts[0].deletable);
        // 旧键名仍兼容（历史网关 / 假数据不会因此变空）
        let legacy = json!({"hits": [{"excerpt": "旧形状", "score": 0.5}]});
        assert_eq!(parse_facts(&legacy).len(), 1);
        // 无数组键 → 空表，不编造
        assert!(parse_facts(&json!({"root": "x"})).is_empty());
    }

    /// 同一段正文两条命中：保留可撤销的那条，不允许「一样的两行、只有一行能删」。
    #[test]
    fn parse_facts_prefers_deletable_duplicate() {
        let resp = json!({
            "items": [
                {"project": "a", "file": "project_memory.md", "excerpt": "同一句", "score": 0.9},
                {"project": "a", "file": crate::memory::writeback::FACT_FILE,
                 "kind": "fact", "line": 7, "excerpt": "同一句", "score": 0.4}
            ]
        });
        let facts = parse_facts(&resp);
        assert_eq!(facts.len(), 1, "同正文必须去重");
        assert_eq!(facts[0].id.as_deref(), Some("fact:7"));
        assert!(facts[0].deletable);
    }

    /// 简报只给可删的事实发 `deletable`，不给外部来源塞 `null` 占位。
    #[test]
    fn compose_brief_emits_deletable_only_when_true() {
        let v = compose_brief(BriefInput {
            scope: "project".to_string(),
            facts: vec![
                Fact {
                    text: "可删的".to_string(),
                    source: "project:a#cross_project_facts.jsonl".to_string(),
                    score: 0.9,
                    id: Some("fact:1".to_string()),
                    deletable: true,
                },
                Fact {
                    text: "只读的".to_string(),
                    source: "project:a#project_memory.md".to_string(),
                    score: 0.8,
                    id: None,
                    deletable: false,
                },
            ],
            ..Default::default()
        });
        let facts = v["facts"].as_array().unwrap();
        assert_eq!(facts[0]["id"], "fact:1");
        assert_eq!(facts[0]["deletable"], true);
        assert!(facts[1].get("id").is_none(), "无 id 的条目不得出现 id 字段");
        assert!(facts[1].get("deletable").is_none(), "false 不该占位");
    }
}
