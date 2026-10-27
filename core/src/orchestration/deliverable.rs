//! 交付物契约检测与 specs 映射（P0-1：复合任务分解）。
//!
//! 归因（架构师实测复现）：用户基准句「搜索今天的AI新闻，并整理成html网页再成PPT给我看」
//! 在 [`crate::orchestration::Orchestrator::plan`] 被判为 `Solo`（`subtasks=[]`），
//! 在 [`crate::plan::runner`] 的 `qualifies` 也判 `false` —— **两条独立机制都不识别
//! 「一句话含多个交付物」**。
//!
//! 本模块提供一条**独立的、纯规则、离线可测、零 LLM** 的「交付物计数」信号
//! [`detect`]，同时护住上述两条机制：
//! - 扫描「产物类型词」，统计**互异产物类型数**；
//! - `≥2 类产物`，或 `≥1 类产物 + 检索动词` → 复合，产出 [`DeliverableContract`]（含 DAG）；
//! - 单一产物类型且无检索动词（「写一个网页」「改个 bug」）→ `None`，仍走 Solo（防过度分解）。
//!
//! 检索词**复用 T02 已落地的** [`crate::tools::builtin::distill_query`]（只读复用，
//! 不另起一份实现，避免两条逻辑漂移）。

use serde::{Deserialize, Serialize};

use crate::model::router::{Complexity, IntentType};
use crate::orchestration::SubtaskSpec;

/// 交付物种类（与前端 `DeliverableKind` 逐字一致：`Research` / `HtmlPage` / …）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliverableKind {
    /// 检索/研究（无实体产物，产出为资料汇总）
    Research,
    /// HTML 网页
    HtmlPage,
    /// 演示文稿（PPT）
    Presentation,
    /// 文档（Word 等）
    Document,
    /// 电子表格
    Spreadsheet,
    /// 其它交付物
    Other,
}

impl DeliverableKind {
    /// 稳定显示名（供事件负载 / 前端 kind 判定；与 TS 字面量一致）。
    pub fn as_str(&self) -> &'static str {
        match self {
            DeliverableKind::Research => "Research",
            DeliverableKind::HtmlPage => "HtmlPage",
            DeliverableKind::Presentation => "Presentation",
            DeliverableKind::Document => "Document",
            DeliverableKind::Spreadsheet => "Spreadsheet",
            DeliverableKind::Other => "Other",
        }
    }
}

/// 一个交付物（契约节点）。
///
/// 注：`role` 与 [`SubtaskSpec::role`] 同类型（`String`）——角色名不再要求
/// `&'static str`，以便蜂群/名册按规模动态生成唯一角色名。本结构仍**仅派 `Serialize`**
/// （只用于向前端投递，从不需要反序列化）。
#[derive(Debug, Clone, Serialize)]
pub struct DeliverableSpec {
    /// 稳定 ID（`"d1"` / `"d2"` …），供依赖边与前端步骤键引用。
    pub id: String,
    /// 交付物种类。
    pub kind: DeliverableKind,
    /// 角色名（与 [`SubtaskSpec::role`] 同源）。
    pub role: String,
    /// 人类可读标题（"检索 AI 新闻" / "生成 PPT" …）。
    pub title: String,
    /// 目标落盘路径（相对工作区）。`Research` 为 `None`；其余为具体相对路径。
    pub target_path: Option<String>,
    /// 蒸馏后的检索词（仅 `Research` 交付物有值）。
    pub search_query: Option<String>,
    /// 依赖的其它交付物 ID（如 `["d1"]`）。
    pub depends_on: Vec<String>,
}

/// 交付物契约：一次复合任务的交付物清单 + DAG + 决策理由。
///
/// 同 [`DeliverableSpec`]：仅派 `Serialize`（内含 `&'static str`，无可行的 `Deserialize`）。
#[derive(Debug, Clone, Serialize)]
pub struct DeliverableContract {
    /// 交付物清单（按依赖链顺序）。
    pub deliverables: Vec<DeliverableSpec>,
    /// 决策理由（用于可观测性与前端只读展示）。
    pub rationale: String,
}

impl DeliverableContract {
    /// 是否复合：**`≥2` 个交付物且存在依赖边**。
    ///
    /// 防过度分解：单一产物（如「写一个网页」）恒为 `false`，不会误升级为流水线。
    pub fn is_compound(&self) -> bool {
        self.deliverables.len() >= 2 && self.deliverables.iter().any(|d| !d.depends_on.is_empty())
    }
}

/// 产物类型关键词表（按**规范顺序**：页面 → 演示 → 表格 → 文档）。
///
/// 顺序即依赖链顺序（检索 → 页面 → 演示 → …），保证 DAG 确定、可离线断言。
/// 关键词刻意收窄，避免误命中普通叙述（如「报告」仅指产物，单产物时不会触发复合）。
const ARTIFACT_KEYWORDS: &[(DeliverableKind, &[&str])] = &[
    (
        DeliverableKind::HtmlPage,
        &["html网页", "html", "网页", "页面", "网站", "h5", "前端页面"],
    ),
    (
        DeliverableKind::Presentation,
        &["pptx", "ppt", "幻灯片", "演示文稿", "演示稿"],
    ),
    (
        DeliverableKind::Spreadsheet,
        &["excel", "xlsx", "电子表格", "表格", "csv", "工作表"],
    ),
    (
        DeliverableKind::Document,
        &["docx", "word", "文档", "白皮书", "报告"],
    ),
];

/// 检索类动词（判定是否存在 `Research` 交付物）。
///
/// 刻意**不收裸「查」/「搜」**：会误命中「检查」「审查」等非检索语境；只收明确检索构词。
const SEARCH_VERBS: &[&str] = &[
    "搜索",
    "检索",
    "查询",
    "查找",
    "搜集",
    "查一下",
    "搜一下",
    "search",
];

/// 交付物种类 → 目标文件默认相对路径。
fn artifact_target_path(kind: DeliverableKind) -> &'static str {
    match kind {
        DeliverableKind::HtmlPage => "output.html",
        DeliverableKind::Presentation => "presentation.pptx",
        DeliverableKind::Spreadsheet => "spreadsheet.xlsx",
        DeliverableKind::Document => "document.docx",
        DeliverableKind::Research => "",
        DeliverableKind::Other => "deliverable.bin",
    }
}

/// 交付物种类 → 静态角色名（`SubtaskSpec.role`）。
fn artifact_role(kind: DeliverableKind) -> &'static str {
    match kind {
        DeliverableKind::Research => "检索",
        DeliverableKind::HtmlPage => "HTML",
        DeliverableKind::Presentation => "PPT",
        DeliverableKind::Spreadsheet => "表格",
        DeliverableKind::Document => "文档",
        DeliverableKind::Other => "交付",
    }
}

/// 交付物种类 → 人类可读种类名（用于 title / rationale）。
fn kind_title(kind: DeliverableKind) -> &'static str {
    match kind {
        DeliverableKind::Research => "检索",
        DeliverableKind::HtmlPage => "HTML 网页",
        DeliverableKind::Presentation => "PPT",
        DeliverableKind::Spreadsheet => "电子表格",
        DeliverableKind::Document => "文档",
        DeliverableKind::Other => "交付物",
    }
}

/// 交付物种类 → 子 Agent 系统人设（强调「必须真实落盘后再报告完成」，对齐单一真源约定）。
fn kind_system_prompt(kind: DeliverableKind) -> &'static str {
    match kind {
        DeliverableKind::Research => {
            "你是检索专家。围绕给定主题进行联网检索，汇总可信来源与要点，输出结构化摘要。\
             不要展开其它交付步骤。"
        }
        DeliverableKind::HtmlPage => {
            "你是前端工程师。把前序检索结论整理为一份自包含的 HTML 网页，并写入指定的 \
             target_path 文件（用 write_file 真实落盘）。必须真实写盘后再报告完成。"
        }
        DeliverableKind::Presentation => {
            "你是演示文稿工程师。把前序结论整理为 PPT 内容，并使用 office_automation 生成真实的 \
             .pptx 文件（写入 target_path）。必须真实落盘后再报告完成。"
        }
        DeliverableKind::Spreadsheet => {
            "你是数据工程师。把前序结论整理为电子表格并写入 target_path（.xlsx/.csv）。\
             必须真实落盘后再报告完成。"
        }
        DeliverableKind::Document => {
            "你是文档工程师。把前序结论整理为 Word/文档并写入 target_path。\
             必须真实落盘后再报告完成。"
        }
        DeliverableKind::Other => {
            "你是交付物生成器。产出该交付物并写入 target_path。必须真实落盘后再报告完成。"
        }
    }
}

/// 交付物种类 → 模型路由意图（检索走 Chat，产物生成走 Code 以带写文件工具）。
fn kind_intent(kind: DeliverableKind) -> IntentType {
    match kind {
        DeliverableKind::Research => IntentType::Chat,
        _ => IntentType::Code,
    }
}

/// 交付物检测（纯规则、离线可测、零 LLM）。
///
/// 命中复合条件时返回带 DAG 的 [`DeliverableContract`]；否则 `None`（调用方保持既有行为）。
pub fn detect(text: &str) -> Option<DeliverableContract> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();

    // 1) 收集互异产物类型（保持 ARTIFACT_KEYWORDS 的规范顺序）。
    let mut kinds: Vec<DeliverableKind> = Vec::new();
    for (kind, kws) in ARTIFACT_KEYWORDS {
        if kws.iter().any(|k| lower.contains(*k)) {
            kinds.push(*kind);
        }
    }
    let searched = SEARCH_VERBS.iter().any(|v| lower.contains(*v));

    // 2) 复合判定：≥2 类产物，或 ≥1 类产物 + 检索动词（都不满足 → None，防过度分解）。
    let compound = kinds.len() >= 2 || (!kinds.is_empty() && searched);
    if !compound {
        return None;
    }

    // 3) 检索词蒸馏（复用 T02 的 distill_query，仅读不改；裁空则回退原始输入）。
    let topic = crate::tools::builtin::distill_query(trimmed);
    let topic = if topic.trim().is_empty() {
        trimmed.to_string()
    } else {
        topic
    };

    // 4) 构造依赖链：检索（若命中）→ 各产物（按规范顺序，逐级 depends_on 前驱）。
    let mut deliverables: Vec<DeliverableSpec> = Vec::new();
    let mut prev: Option<String> = None;
    if searched {
        let id = "d1".to_string();
        deliverables.push(DeliverableSpec {
            id: id.clone(),
            kind: DeliverableKind::Research,
            role: artifact_role(DeliverableKind::Research).to_string(),
            title: format!("检索 {topic}"),
            target_path: None,
            search_query: Some(topic.clone()),
            depends_on: Vec::new(),
        });
        prev = Some(id);
    }
    for kind in &kinds {
        let index = deliverables.len() + 1;
        let id = format!("d{index}");
        let path = artifact_target_path(*kind);
        let depends_on: Vec<String> = prev.iter().cloned().collect();
        deliverables.push(DeliverableSpec {
            id: id.clone(),
            kind: *kind,
            role: artifact_role(*kind).to_string(),
            title: format!("生成 {}（{path}）", kind_title(*kind)),
            target_path: Some(path.to_string()),
            search_query: None,
            depends_on,
        });
        prev = Some(id);
    }

    let kinds_label = kinds
        .iter()
        .map(|k| kind_title(*k))
        .collect::<Vec<_>>()
        .join("、");
    let rationale = if searched {
        format!(
            "检测到检索意图 + {} 类互异产物（{}），按「检索 → 产物」依赖链执行",
            kinds.len(),
            kinds_label
        )
    } else {
        format!(
            "检测到 {} 类互异产物（{}），按交付依赖链执行",
            kinds.len(),
            kinds_label
        )
    };

    Some(DeliverableContract {
        deliverables,
        rationale,
    })
}

/// 契约 → 既有 [`SubtaskSpec`] 列表（含 deps）。
///
/// 注意 `SubtaskSpec.deps` 的既有语义是**依赖的其它 subtask 的 role 名**，
/// 而契约 `depends_on` 存的是交付物 ID；此处做 `id → role` 映射。
pub fn contract_to_specs(c: &DeliverableContract) -> Vec<SubtaskSpec> {
    c.deliverables
        .iter()
        .map(|d| {
            let task = match (&d.search_query, &d.target_path) {
                (Some(q), _) if d.kind == DeliverableKind::Research => {
                    format!("围绕「{q}」进行联网检索，汇总可信来源与要点。")
                }
                // A1：单一事实源是 `title` —— 它已携带落盘路径（如
                // 「生成 HTML 网页（output.html）」）。此处**不再复述** `target_path`，
                // 否则同一路径会在一步里出现两次（「…（output.html），产物写入 output.html。」）。
                (_, Some(_)) => format!("{}。", d.title),
                _ => d.title.clone(),
            };
            let deps: Vec<String> = d
                .depends_on
                .iter()
                .filter_map(|dep_id| {
                    c.deliverables
                        .iter()
                        .find(|x| &x.id == dep_id)
                        .map(|x| x.role.to_string())
                })
                .collect();
            SubtaskSpec {
                role: d.role.clone(),
                system_prompt: kind_system_prompt(d.kind).to_string(),
                task,
                intent: kind_intent(d.kind),
                complexity: Complexity::Medium,
                deps,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用户截图故障原文（基准句）。
    const BASELINE: &str = "搜索今天的AI新闻，并整理成html网页再成PPT给我看";

    #[test]
    fn baseline_sentence_becomes_compound_contract() {
        let c = detect(BASELINE).expect("基准句应识别为复合任务");
        assert!(c.is_compound(), "基准句应判为复合");
        assert_eq!(c.deliverables.len(), 3, "应有 检索/HTML/PPT 三个交付物");
        assert_eq!(c.deliverables[0].kind, DeliverableKind::Research);
        assert_eq!(c.deliverables[1].kind, DeliverableKind::HtmlPage);
        assert_eq!(c.deliverables[2].kind, DeliverableKind::Presentation);
        // 依赖链：d1 → d2 → d3
        assert_eq!(c.deliverables[1].depends_on, vec!["d1".to_string()]);
        assert_eq!(c.deliverables[2].depends_on, vec!["d2".to_string()]);
        // 检索词已蒸馏（复用 T02 distill_query），不含交付指令
        assert_eq!(
            c.deliverables[0].search_query.as_deref(),
            Some("今天的AI新闻")
        );
        assert!(c.deliverables[1].target_path.is_some());
        assert!(c.deliverables[2].target_path.is_some());
    }

    #[test]
    fn contract_maps_to_three_specs_with_role_deps() {
        let c = detect(BASELINE).unwrap();
        let specs = contract_to_specs(&c);
        assert_eq!(specs.len(), 3);
        assert!(specs[0].deps.is_empty(), "首个交付物无依赖");
        assert_eq!(
            specs[1].deps,
            vec!["检索".to_string()],
            "deps 用前置 role 名"
        );
        assert_eq!(specs[2].deps, vec![specs[1].role.to_string()]);
        // A1：落盘路径在任务文案中只出现一次（`title` 是唯一事实源，不再复述路径）。
        for (spec, path) in [(&specs[1], "output.html"), (&specs[2], "presentation.pptx")] {
            assert_eq!(
                spec.task.matches(path).count(),
                1,
                "产物路径在任务文案中应恰好出现一次，实际：{}",
                spec.task
            );
        }
    }

    #[test]
    fn single_artifact_without_search_is_not_compound() {
        assert!(detect("写一个网页").is_none(), "单产物不应复合");
        assert!(detect("改个 bug").is_none());
        assert!(detect("解释一下闭包是什么").is_none());
        assert!(detect("写一个网页再改个 bug").is_none());
    }

    #[test]
    fn search_without_artifact_is_not_compound() {
        assert!(detect("搜索这个文件").is_none(), "仅检索无产物不应复合");
        assert!(detect("查一下今天的天气").is_none());
    }

    #[test]
    fn search_plus_single_artifact_is_compound() {
        let c = detect("搜索最新的AI新闻并整理成网页").expect("检索 + 单产物应复合");
        assert_eq!(c.deliverables.len(), 2);
        assert_eq!(c.deliverables[0].kind, DeliverableKind::Research);
        assert_eq!(c.deliverables[1].kind, DeliverableKind::HtmlPage);
        assert_eq!(c.deliverables[1].depends_on, vec!["d1".to_string()]);
    }

    #[test]
    fn two_artifacts_without_search_are_compound() {
        let c = detect("先做一个网页再做一份PPT").expect("≥2 类产物应复合");
        assert_eq!(c.deliverables.len(), 2);
        assert_eq!(c.deliverables[0].kind, DeliverableKind::HtmlPage);
        assert_eq!(c.deliverables[1].kind, DeliverableKind::Presentation);
        assert_eq!(c.deliverables[1].depends_on, vec!["d1".to_string()]);
    }

    #[test]
    fn empty_or_plain_text_is_none() {
        assert!(detect("").is_none());
        assert!(detect("   ").is_none());
        assert!(detect("你好").is_none());
    }

    #[test]
    fn is_compound_requires_dependency_edge() {
        let single = DeliverableContract {
            deliverables: vec![DeliverableSpec {
                id: "d1".into(),
                kind: DeliverableKind::HtmlPage,
                role: "HTML".to_string(),
                title: "x".into(),
                target_path: Some("output.html".into()),
                search_query: None,
                depends_on: vec![],
            }],
            rationale: "单产物".into(),
        };
        assert!(!single.is_compound(), "单产物无依赖边，不得判为复合");
    }
}
