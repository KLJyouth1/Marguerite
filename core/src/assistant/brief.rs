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

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub text: String,
    /// `project:<name>#<会话/版本>` / `cross_project:<file>` 之类
    pub source: String,
    pub score: f64,
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
            facts.push(json!({"text": f.text, "source": f.source, "score": f.score}));
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

    json!({
        "scope": input.scope,
        "workspace": input.workspace,
        "pinned": pinned,
        "preferences": prefs,
        "facts": facts,
        "open_loops": open_loops,
        "budget": {"max_chars": max_chars, "used": used},
        "truncated": truncated,
    })
}

/// 从既有跨项目记忆里尽力取事实；取不到就是空表，**绝不**编造。
///
/// `cross_project_search` 返回的是网关形状的 JSON，这里做**宽容解析**：
/// 认 `hits` / `results` / `data` 三种数组名，条目认 `excerpt` / `text` / `snippet` 字段。
fn collect_facts(query: &str, limit: usize) -> Vec<Fact> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let params = json!({"query": query, "limit": limit});
    let resp = crate::memory::cross_project::cross_project_search(&params);
    let arr = ["hits", "results", "data"]
        .iter()
        .find_map(|k| resp.get(*k).and_then(Value::as_array))
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::new();
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
        let source = ["source", "file", "path"]
            .iter()
            .find_map(|k| item.get(*k).and_then(Value::as_str))
            .unwrap_or("cross_project")
            .to_string();
        let score = item.get("score").and_then(Value::as_f64).unwrap_or(0.0);
        out.push(Fact {
            text,
            source,
            score,
        });
    }
    out
}

/// `assistant.brief.get` 的实现：按当前作用域合成简报。
///
/// - `Global`：只带 pinned + preferences + 跨项目 facts（不碰任何项目内文件）
/// - `Project{workspace}`：同上，facts 的查询会带上 workspace 名做限定
pub fn build_brief(scope: &AssistantScope, query: &str, max_chars: Option<usize>) -> Value {
    let profile = super::profile::UserProfile::load();
    let workspace = scope.workspace().map(|p| p.to_string_lossy().to_string());
    let scoped_query = match (&workspace, query.trim()) {
        (Some(ws), "") => ws.rsplit(['/', '\\']).next().unwrap_or(ws).to_string(),
        (_, q) => q.to_string(),
    };

    compose_brief(BriefInput {
        scope: scope.kind().to_string(),
        workspace,
        pinned: load_pins(),
        preferences: profile.to_preferences(),
        facts: collect_facts(&scoped_query, 6),
        open_loops: Vec::new(),
        max_chars,
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
                },
                Fact {
                    text: "高分".to_string(),
                    source: "b".to_string(),
                    score: 0.9,
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
}
