//! 需求门（Brief Gate）—— 主 Agent 在动 PlanGate 之前先过的一道前置门。
//!
//! 灵感来源：抖音 @英雄哪里出来《三分钟开发个 3D 可视化交互学习网站并部署上线》
//! 提出的「六要素万能提示词公式」：
//!   要做什么 / 怎么交互 / 技术底线 / 视觉风格 / 数据来源 / 输出什么。
//!
//! 现状 PlanGate（governance::mod::PlanGate）只裁决「动作是否安全」（C1–C4），
//! 完全不裁决「用户的需求有没有说清楚」。这导致主 Agent 经常在需求模糊时
//! 直接开干，产出「开盲盒」式结果——这正是视频里批评的核心痛点：
//! 「很多人用 AI 像开盲盒，本质原因就一个，你的想法 AI 没听懂」。
//!
//! Brief Gate 的职责：
//! 1. 把用户的原始需求解析成结构化的 [`RequirementBrief`] 六要素；
//! 2. 校验必选要素是否齐备，缺失项以 [`BriefGap`] 形式返回（Advisory，不阻断）；
//! 3. 把用户在开发阶段追加的「否定式硬约束」（不要省略 / 不要占位符 / 溢出怎么处理 /
//!    不要缩写号）登记进 [`HardConstraint`] 清单，交给 CompletionGate 兑现核对。
//!
//! 设计原则：纯逻辑、零外部依赖、可离线单测；不修改 C1–C4 既有行为。

use serde::{Deserialize, Serialize};

/// 需求六要素之一。缺失时按 [`RequirementBrief::required_slots`] 的判定决定是否报 gap。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementBrief {
    /// 【要做什么】一句话说清产品/任务目标。
    pub goal: String,
    /// 【怎么交互】用户做什么 → 系统响应什么（交互类产品必填，纯批处理可空）。
    pub interaction: String,
    /// 【技术底线】必须用的技术栈 / 运行时 / 硬约束（如 three.js、Rust、离线可跑）。
    pub tech_floor: String,
    /// 【视觉风格】布局 / 配色 / 调性（纯后端任务可空）。
    pub visual_style: String,
    /// 【数据来源】数据从哪来、以谁为准（涉及数据/内容生成时必填）。
    pub data_source: String,
    /// 【输出什么】最终交付物格式与完整度（飞书文档 / index.html / EXE / 可运行代码）。
    pub deliverable: String,
}

/// 一条「否定式硬约束」——视频里作者在应用生成阶段追加的那类指令：
/// 「不要省略任何分子」「不要用占位符」「文字溢出改成 2 列 + 内部滚动」「不要缩写号」。
///
/// CompletionGate 兑现时必须逐条核对产物是否违反。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HardConstraint {
    pub id: String,
    /// 人类可读的约束原文（逐字保留用户原话，便于审计）。
    pub raw: String,
    /// 机器可判的检查函数名（由 [`HardConstraintKind`] 决定）。
    pub kind: HardConstraintKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HardConstraintKind {
    /// 产物中不得出现 TODO / FIXME / lorem / placeholder / 占位 等占位符。
    NoPlaceholder,
    /// 列表/清单类产物不得省略任何条目（按用户给的条目数对账）。
    NoOmittedEntries,
    /// 不得使用缩写号 / 省略号（… / ... / 等）作为内容截断。
    NoEllipsis,
    /// 文字溢出/截断时的指定处理方式（自由文本，由人审）。
    OverflowPolicy,
}

/// Brief Gate 发现的需求缺口。Advisory 级别——不阻断计划，但主 Agent 应在动手前补问。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefGap {
    pub slot: String,
    pub hint: String,
}

/// Brief Gate 裁决结果。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefReport {
    pub gaps: Vec<BriefGap>,
    pub hard_constraints: Vec<HardConstraint>,
}

impl BriefReport {
    /// 是否所有必选要素齐备（gap 数量为 0）。
    pub fn is_complete(&self) -> bool {
        self.gaps.is_empty()
    }
}

impl RequirementBrief {
    /// 构造空 brief（用于流式填充）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 判定某 slot 是否为必选。
    ///
    /// 规则（来自视频六要素公式 + 工程实践）：
    /// - `goal` / `tech_floor` / `deliverable` 永远必选；
    /// - `data_source` 在任务涉及"数据/内容/清单"时必选（由调用方设置
    ///   [`Self::with_data_sensitive`]）；
    /// - `interaction` / `visual_style` 对纯后端 / 批处理任务可空。
    pub fn required_slots(&self, data_sensitive: bool) -> Vec<&'static str> {
        let mut slots = vec!["goal", "tech_floor", "deliverable"];
        if data_sensitive {
            slots.push("data_source");
        }
        slots
    }

    fn is_blank(s: &str) -> bool {
        s.trim().is_empty()
    }

    /// 跑 Brief Gate：返回缺失的必选 slot 清单。
    pub fn audit(&self, data_sensitive: bool) -> Vec<BriefGap> {
        let mut gaps = Vec::new();
        let check = |slot: &'static str, val: &str, hint: &str| {
            if Self::is_blank(val) {
                Some(BriefGap {
                    slot: slot.to_string(),
                    hint: hint.to_string(),
                })
            } else {
                None
            }
        };
        if let Some(g) = check("goal", &self.goal, "一句话说清要做什么 / 交付什么产品") {
            gaps.push(g);
        }
        if let Some(g) = check(
            "tech_floor",
            &self.tech_floor,
            "技术底线：必须用的栈 / 运行时 / 不能动的边界",
        ) {
            gaps.push(g);
        }
        if let Some(g) = check(
            "deliverable",
            &self.deliverable,
            "最终交付物格式与完整度（飞书文档 / index.html / EXE / 可运行代码）",
        ) {
            gaps.push(g);
        }
        if data_sensitive {
            if let Some(g) = check(
                "data_source",
                &self.data_source,
                "数据来源：以谁为准 / 范围边界（如「高考考纲全部常见物质」）",
            ) {
                gaps.push(g);
            }
        }
        gaps
    }
}

/// HardConstraint 检查结果：产物文本是否违反了某条硬约束。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HardConstraintViolation {
    pub constraint_id: String,
    pub raw: String,
    pub detail: String,
}

/// 离线、纯文本的硬约束核对器。CompletionGate 在产物落盘后调用。
pub struct HardConstraintChecker;

impl HardConstraintChecker {
    /// 对一段产物文本（源码 / 文档 / HTML）跑全部硬约束。
    pub fn check_all(
        constraints: &[HardConstraint],
        artifact_text: &str,
        expected_entry_count: Option<usize>,
    ) -> Vec<HardConstraintViolation> {
        let mut out = Vec::new();
        for c in constraints {
            let v = match c.kind {
                HardConstraintKind::NoPlaceholder => Self::check_no_placeholder(artifact_text),
                HardConstraintKind::NoOmittedEntries => {
                    Self::check_no_omitted(artifact_text, expected_entry_count)
                }
                HardConstraintKind::NoEllipsis => Self::check_no_ellipsis(artifact_text),
                HardConstraintKind::OverflowPolicy => None, // 视觉类，由人审
            };
            if let Some(detail) = v {
                out.push(HardConstraintViolation {
                    constraint_id: c.id.clone(),
                    raw: c.raw.clone(),
                    detail,
                });
            }
        }
        out
    }

    fn check_no_placeholder(text: &str) -> Option<String> {
        const BAD: &[&str] = &[
            "TODO",
            "FIXME",
            "lorem ipsum",
            "placeholder",
            "占位符",
            "占位",
            "xxx",
            "待补充",
            "待完善",
        ];
        let lower = text.to_lowercase();
        let hits: Vec<&str> = BAD
            .iter()
            .copied()
            .filter(|b| lower.contains(&b.to_lowercase()))
            .collect();
        if hits.is_empty() {
            None
        } else {
            Some(format!("artifact contains placeholder markers: {:?}", hits))
        }
    }

    fn check_no_omitted(text: &str, expected: Option<usize>) -> Option<String> {
        // 用户原话是「不要省略任何分子」。我们没有分子清单本体时，
        // 只能做静态兜底：产物里若出现「其余省略」「以此类推」「...等」这类自认省略措辞，判违规。
        const BAD: &[&str] = &["其余省略", "以此类推", "等等等", "（省略", "(省略"];
        let hits: Vec<&str> = BAD.iter().copied().filter(|b| text.contains(b)).collect();
        if !hits.is_empty() {
            return Some(format!("artifact admits omission: {:?}", hits));
        }
        if let Some(n) = expected {
            // 调用方给了期望条目数时，要求产物里至少出现 n 次明显的条目锚点（这里粗略用行内
            // 反引号 code 块数；更精确的对账由上层 CompletionGate 做）。
            let code_spans = text.matches('`').count() / 2;
            if code_spans + 1 < n {
                return Some(format!(
                    "expected >= {} entries, only ~{} code spans found",
                    n, code_spans
                ));
            }
        }
        None
    }

    fn check_no_ellipsis(text: &str) -> Option<String> {
        // 用户原话是「不要缩写号」——指中文里用 … 或 ... 截断内容。
        // 注意：代码里的 `...`（spread）和省略号菜单是合法的，这里只对**自然语言叙述**粗判。
        // 策略：行尾出现 … / ... 且该行不在 ``` 代码块内 → 命中。
        let mut in_code = false;
        let mut hits = 0;
        for line in text.lines() {
            if line.trim_start().starts_with("```") {
                in_code = !in_code;
                continue;
            }
            if in_code {
                continue;
            }
            let trimmed = line.trim_end();
            if trimmed.ends_with('…') || trimmed.ends_with("...") {
                hits += 1;
            }
        }
        if hits > 0 {
            Some(format!("{} prose line(s) end with ellipsis", hits))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_brief_fires_all_required_gaps() {
        let b = RequirementBrief::new();
        let gaps = b.audit(true);
        let slots: Vec<&str> = gaps.iter().map(|g| g.slot.as_str()).collect();
        assert_eq!(slots, vec!["goal", "tech_floor", "deliverable", "data_source"]);
    }

    #[test]
    fn filled_brief_is_complete() {
        let b = RequirementBrief {
            goal: "用 three.js 做高中分子结构 3D 演示".into(),
            interaction: "鼠标拖拽旋转 / 左侧切换 / 翻页".into(),
            tech_floor: "three.js".into(),
            visual_style: "左右分栏".into(),
            data_source: "高考考纲全部常见物质".into(),
            deliverable: "完整 index.html".into(),
        };
        assert!(b.audit(true).is_empty());
    }

    #[test]
    fn non_data_sensitive_skips_data_source() {
        let b = RequirementBrief {
            goal: "写个 CLI".into(),
            tech_floor: "Rust".into(),
            deliverable: "exe".into(),
            ..Default::default()
        };
        assert!(b.audit(false).is_empty());
        // 同一份 brief，data_sensitive=true 时应报 data_source gap
        assert_eq!(b.audit(true).len(), 1);
    }

    #[test]
    fn no_placeholder_catches_todo_and_lorem() {
        let violations = HardConstraintChecker::check_all(
            &[HardConstraint {
                id: "hc-1".into(),
                raw: "不要用占位符".into(),
                kind: HardConstraintKind::NoPlaceholder,
            }],
            "这里 TODO: 稍后补全\nlorem ipsum dolor",
            None,
        );
        assert_eq!(violations.len(), 1);
        assert!(violations[0].detail.contains("TODO"));
    }

    #[test]
    fn no_placeholder_passes_clean_text() {
        let violations = HardConstraintChecker::check_all(
            &[HardConstraint {
                id: "hc-1".into(),
                raw: "不要用占位符".into(),
                kind: HardConstraintKind::NoPlaceholder,
            }],
            "这是一段干净的正文，所有字段都已填实，没有任何待补标记。",
            None,
        );
        assert!(violations.is_empty());
    }

    #[test]
    fn no_ellipsis_ignores_code_blocks() {
        let text = "正文段落一。\n```rust\nlet x = (0..10).collect::<Vec<_>>();\n```\n正文段落二...\n";
        let violations = HardConstraintChecker::check_all(
            &[HardConstraint {
                id: "hc-2".into(),
                raw: "不要缩写号".into(),
                kind: HardConstraintKind::NoEllipsis,
            }],
            text,
            None,
        );
        assert_eq!(violations.len(), 1, "only the prose line should hit");
    }

    #[test]
    fn no_omitted_catches_self_admission() {
        let violations = HardConstraintChecker::check_all(
            &[HardConstraint {
                id: "hc-3".into(),
                raw: "不要省略任何分子".into(),
                kind: HardConstraintKind::NoOmittedEntries,
            }],
            "其余分子省略，以此类推",
            None,
        );
        assert_eq!(violations.len(), 1);
    }
}
