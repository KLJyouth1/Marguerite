//! 任务分解器（Wish Decomposer）——把六要素契约拆成带验收标准的 DAG。
//!
//! # 为什么需要
//!
//! 「一句话许愿」最大的失败模式不是「不会做」，而是**一次做太大、方向错了才发现**。
//! 百墨做航母拆了 20 个任务；剪辑号先出 1 分钟样片。本模块把这两条经验固化成结构：
//!
//! - 每个节点都有 **acceptance**（怎么算这个节点做完）——没有验收标准的节点不许存在；
//! - 每个节点都有 **risk**（auto / human_gate / critical）——来自「4 类能丢 / 3 类别丢」；
//! - 节点数超过 [`MVP_SLICE_THRESHOLD`] 时 **强制先出最小切片**（`needs_slice`）。
//!
//! 与 `wish.rs` 一样：纯逻辑、零 LLM、可单测。模板是「默认拆法」，不是死规定——
//! 模型可以在执行中增删节点，但**必须保留验收标准与风险级这两个字段**。
//!
//! 诚实边界：这里给的是**结构化的拆解骨架**，不是「任意任务都能自动拆对」。
//! 真正贴合具体需求的拆分仍由模型在骨架内细化；本模块保证的是「不会漏掉验收与风险」。

use serde::{Deserialize, Serialize};

use crate::governance::wish::{TaskKind, WishDraft};

/// 节点难度：决定用便宜模型还是贵模型（R67 §4.5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Difficulty {
    /// 样板/搬运/格式转换：Flash 足够
    Trivial,
    /// 常规实现：Flash 主力
    Standard,
    /// 架构决策/难调试/审美：需要更强模型
    Hard,
}

/// 风险级：决定要不要人点头（R65「4 类能丢 / 3 类别丢」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// 一眼可验、可回滚 → 自动执行
    Auto,
    /// 需要人判断（审美/方向/金额口径）→ 人点头
    HumanGate,
    /// 对外发布/付款/不可逆 → 必须显式批准
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskNode {
    pub id: String,
    pub title: String,
    pub difficulty: Difficulty,
    pub depends_on: Vec<String>,
    /// 怎么算这个节点做完（必须可验证：命令/文件/现象）
    pub acceptance: Vec<String>,
    pub risk: RiskLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDag {
    pub kind: TaskKind,
    pub nodes: Vec<TaskNode>,
    /// 是否应先做最小切片（节点多 / 用户说了「整套/完整」）
    pub needs_slice: bool,
}

impl TaskDag {
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// 渲染成提示词片段（节点 + 验收 + 风险）。
    pub fn to_hint(&self) -> String {
        if self.nodes.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        out.push_str("\n【任务分解（默认骨架；可细化，但不得删掉验收标准与风险级）】\n");
        if self.needs_slice {
            out.push_str(
                "⚠ 本任务节点较多：先做**最小可验证切片**（能跑、能看、能判断方向），\
                 拿到用户「对，就是这个方向」的确认后再全量展开。\n",
            );
        }
        for (i, n) in self.nodes.iter().enumerate() {
            out.push_str(&format!(
                "{}. {}｜难度={:?}｜风险={:?}\n",
                i + 1,
                n.title,
                n.difficulty,
                n.risk
            ));
            if !n.depends_on.is_empty() {
                out.push_str(&format!("   依赖：{}\n", n.depends_on.join(", ")));
            }
            out.push_str(&format!("   验收：{}\n", n.acceptance.join("；")));
        }
        out
    }
}

/// 超过这个节点数就必须先出切片（R67 §4.4）。
pub const MVP_SLICE_THRESHOLD: usize = 5;

/// 按任务类型给出默认拆解骨架。
pub fn decompose(draft: &WishDraft) -> TaskDag {
    // 非任务型输入（闲聊/提问）不产出 DAG：否则控制台会为一句「你好」显示一份
    // 三段式任务计划，用户会以为系统要自作主张干活。
    if !draft.is_wish {
        return TaskDag {
            kind: draft.kind,
            nodes: Vec::new(),
            needs_slice: false,
        };
    }
    let nodes = match draft.kind {
        TaskKind::WebApp => web_nodes(),
        TaskKind::ThreeD => three_d_nodes(),
        TaskKind::Game => game_nodes(),
        TaskKind::Document => document_nodes(),
        TaskKind::Media => media_nodes(),
        TaskKind::Automation => automation_nodes(),
        TaskKind::Research => research_nodes(),
        TaskKind::Code => code_nodes(),
        TaskKind::Unknown => generic_nodes(),
    };
    let needs_slice = draft.prefer_slice_first || nodes.len() > MVP_SLICE_THRESHOLD;
    TaskDag {
        kind: draft.kind,
        nodes,
        needs_slice,
    }
}

fn n(
    id: &str,
    title: &str,
    difficulty: Difficulty,
    deps: &[&str],
    acceptance: &[&str],
    risk: RiskLevel,
) -> TaskNode {
    TaskNode {
        id: id.into(),
        title: title.into(),
        difficulty,
        depends_on: deps.iter().map(|s| s.to_string()).collect(),
        acceptance: acceptance.iter().map(|s| s.to_string()).collect(),
        risk,
    }
}

fn web_nodes() -> Vec<TaskNode> {
    vec![
        n(
            "structure",
            "确定页面结构与信息层级",
            Difficulty::Standard,
            &[],
            &["列出页面区块清单", "每个区块写明承载什么内容"],
            RiskLevel::Auto,
        ),
        n(
            "build",
            "实现单文件页面（HTML/CSS/JS）",
            Difficulty::Standard,
            &["structure"],
            &["文件可双击打开无报错", "控制台无 JS 错误", "移动端宽度不横向溢出"],
            RiskLevel::Auto,
        ),
        n(
            "visual",
            "视觉与动效打磨",
            Difficulty::Hard,
            &["build"],
            &["配色/间距/层级符合既定风格", "交互有明确反馈（hover/active）"],
            RiskLevel::Auto,
        ),
        n(
            "verify",
            "端到端自检",
            Difficulty::Standard,
            &["build", "visual"],
            &["浏览器实测关键路径", "硬约束逐条对账", "产出文件真实存在且非空"],
            RiskLevel::Auto,
        ),
    ]
}

fn three_d_nodes() -> Vec<TaskNode> {
    vec![
        n(
            "data",
            "准备真实数据（条目清单，不得占位）",
            Difficulty::Standard,
            &[],
            &["条目数与来源可核对", "每条数据字段完整"],
            RiskLevel::Auto,
        ),
        n(
            "scene",
            "搭建 3D 场景（相机/光照/材质）",
            Difficulty::Standard,
            &["data"],
            &["场景可渲染无黑屏", "相机可控"],
            RiskLevel::Auto,
        ),
        n(
            "interaction",
            "交互（拖拽旋转/切换/缩放）",
            Difficulty::Standard,
            &["scene"],
            &["鼠标拖拽可旋转", "切换条目不卡顿"],
            RiskLevel::Auto,
        ),
        n(
            "ui",
            "信息面板与布局",
            Difficulty::Standard,
            &["scene"],
            &["左右分栏不重叠", "长文本不溢出（按约定策略处理）"],
            RiskLevel::Auto,
        ),
        n(
            "visual",
            "视觉调性与动效",
            Difficulty::Hard,
            &["ui", "interaction"],
            &["模糊词（如「炫酷」）已落成具体决定", "动效不影响可读性"],
            RiskLevel::Auto,
        ),
        n(
            "verify",
            "端到端自检 + 截图确认",
            Difficulty::Standard,
            &["visual"],
            &["浏览器打开无报错", "截图确认画面正常", "硬约束逐条对账"],
            RiskLevel::Auto,
        ),
    ]
}

fn game_nodes() -> Vec<TaskNode> {
    vec![
        n(
            "loop",
            "核心循环（输入→更新→渲染）",
            Difficulty::Hard,
            &[],
            &["可稳定 60fps 运行 30 秒", "无内存增长"],
            RiskLevel::Auto,
        ),
        n(
            "mechanics",
            "核心玩法（移动/碰撞/计分）",
            Difficulty::Standard,
            &["loop"],
            &["每个机制可单独复现", "边界（撞墙/越界）不崩"],
            RiskLevel::Auto,
        ),
        n(
            "content",
            "关卡/内容",
            Difficulty::Standard,
            &["mechanics"],
            &["至少 1 个可完整通关的关卡", "内容非占位"],
            RiskLevel::Auto,
        ),
        n(
            "feel",
            "手感与反馈（音效/震动/粒子）",
            Difficulty::Hard,
            &["mechanics"],
            &["操作有即时反馈", "难度曲线可感知"],
            RiskLevel::HumanGate,
        ),
        n(
            "verify",
            "实机试玩自检",
            Difficulty::Standard,
            &["content", "feel"],
            &["完整通关一次并记录", "无阻塞性 bug"],
            RiskLevel::Auto,
        ),
    ]
}

fn document_nodes() -> Vec<TaskNode> {
    vec![
        n(
            "outline",
            "确定结构与要点",
            Difficulty::Standard,
            &[],
            &["章节清单明确", "每节结论先行"],
            RiskLevel::Auto,
        ),
        n(
            "content",
            "撰写/生成正文",
            Difficulty::Standard,
            &["outline"],
            &["无占位与待补", "数据有来源或标注推断"],
            RiskLevel::Auto,
        ),
        n(
            "format",
            "排版与格式",
            Difficulty::Standard,
            &["content"],
            &["可被对应软件正常打开", "层级样式一致"],
            RiskLevel::Auto,
        ),
        n(
            "verify",
            "打开实测",
            Difficulty::Standard,
            &["format"],
            &["真实打开一次并检查首页/末页", "文件非空"],
            RiskLevel::Auto,
        ),
    ]
}

fn media_nodes() -> Vec<TaskNode> {
    vec![
        n(
            "probe",
            "探测源文件（时长/分辨率/编码）",
            Difficulty::Trivial,
            &[],
            &["拿到真实元数据", "确认输入文件存在且非空"],
            RiskLevel::Auto,
        ),
        n(
            "process",
            "执行处理（转码/裁剪/抽帧）",
            Difficulty::Standard,
            &["probe"],
            &["产物文件真实落盘且体积 > 0", "时长/分辨率符合要求"],
            RiskLevel::Auto,
        ),
        n(
            "verify",
            "回读校验",
            Difficulty::Standard,
            &["process"],
            &["对产物再 probe 一次，参数符合预期", "原文件未被改动"],
            RiskLevel::Auto,
        ),
    ]
}

fn automation_nodes() -> Vec<TaskNode> {
    vec![
        n(
            "observe",
            "先观察现状（截图/读界面）",
            Difficulty::Trivial,
            &[],
            &["拿到当前界面证据（截图或控件树）"],
            RiskLevel::Auto,
        ),
        n(
            "script",
            "实现自动化步骤",
            Difficulty::Standard,
            &["observe"],
            &["每一步可单独验证", "失败有明确错误信息"],
            RiskLevel::Auto,
        ),
        n(
            "run",
            "真实执行一次",
            Difficulty::Standard,
            &["script"],
            &["真实跑通并留下证据（输出/截图/文件）"],
            RiskLevel::HumanGate,
        ),
        n(
            "verify",
            "结果核对",
            Difficulty::Standard,
            &["run"],
            &["目标状态达成且可复查", "无残留进程/窗口"],
            RiskLevel::Auto,
        ),
    ]
}

fn research_nodes() -> Vec<TaskNode> {
    vec![
        n(
            "collect",
            "多源检索（至少两个独立来源）",
            Difficulty::Standard,
            &[],
            &["来源链接可点开", "发布时间可查"],
            RiskLevel::Auto,
        ),
        n(
            "crosscheck",
            "交叉验证与冲突标注",
            Difficulty::Hard,
            &["collect"],
            &["冲突处明确写出两方说法", "不把推测写成事实"],
            RiskLevel::Auto,
        ),
        n(
            "report",
            "产出带引用的结论",
            Difficulty::Standard,
            &["crosscheck"],
            &["每条结论可追溯到来源", "无编造引用"],
            RiskLevel::Auto,
        ),
    ]
}

fn code_nodes() -> Vec<TaskNode> {
    vec![
        n(
            "locate",
            "定位相关代码与根因",
            Difficulty::Standard,
            &[],
            &["给出文件与行号", "根因有证据支撑"],
            RiskLevel::Auto,
        ),
        n(
            "change",
            "实施最小改动",
            Difficulty::Standard,
            &["locate"],
            &["改动面可控", "不引入无关重构"],
            RiskLevel::Auto,
        ),
        n(
            "test",
            "跑测试/构建验证",
            Difficulty::Standard,
            &["change"],
            &["目标测试通过", "无新增失败"],
            RiskLevel::Auto,
        ),
    ]
}

fn generic_nodes() -> Vec<TaskNode> {
    vec![
        n(
            "clarify",
            "把目标落成可验证的完成条件",
            Difficulty::Standard,
            &[],
            &["写出 1-3 条可验证的完成条件"],
            RiskLevel::Auto,
        ),
        n(
            "execute",
            "执行并留下证据",
            Difficulty::Standard,
            &["clarify"],
            &["产物真实存在", "有可复核的证据"],
            RiskLevel::Auto,
        ),
        n(
            "verify",
            "自检与对账",
            Difficulty::Standard,
            &["execute"],
            &["完成条件逐条核对", "硬约束无违反"],
            RiskLevel::Auto,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::governance::wish::WishParser;

    fn dag_of(text: &str) -> TaskDag {
        decompose(&WishParser::parse(text))
    }

    #[test]
    fn every_node_has_acceptance_and_risk() {
        for text in [
            "做个 3D 分子网站",
            "做个 FPS 小游戏",
            "把这个视频剪成 15 秒",
            "写一份竞品调研报告",
            "帮我重构这段代码",
            "自动打开浏览器导出表格",
            "做个仪表盘网页",
        ] {
            let dag = dag_of(text);
            assert!(!dag.is_empty(), "{text} 应产出节点");
            for node in &dag.nodes {
                assert!(
                    !node.acceptance.is_empty(),
                    "{text} 的节点 {} 缺验收标准",
                    node.id
                );
                assert!(!node.title.trim().is_empty());
            }
        }
    }

    #[test]
    fn three_d_dag_covers_data_scene_interaction_verify() {
        let dag = dag_of("做个 3D 分子网站");
        let ids: Vec<&str> = dag.nodes.iter().map(|n| n.id.as_str()).collect();
        for need in ["data", "scene", "interaction", "verify"] {
            assert!(ids.contains(&need), "3D 骨架缺 {need}：{ids:?}");
        }
        // 数据节点必须要求「不得占位」（这是最常见的翻车点）
        let data = dag.nodes.iter().find(|n| n.id == "data").unwrap();
        assert!(data.acceptance.iter().any(|a| a.contains("完整")));
    }

    #[test]
    fn many_nodes_force_slice_first() {
        // 3D 有 6 个节点 > 阈值 5 → 必须切片
        let dag = dag_of("做个 3D 分子网站");
        assert!(dag.len() > MVP_SLICE_THRESHOLD);
        assert!(dag.needs_slice);
        assert!(dag.to_hint().contains("最小可验证切片"));
    }

    #[test]
    fn small_dag_does_not_force_slice() {
        let dag = dag_of("把这个视频剪成 15 秒");
        assert!(dag.len() <= MVP_SLICE_THRESHOLD);
        assert!(!dag.needs_slice);
    }

    #[test]
    fn human_gate_appears_where_judgement_is_needed() {
        let game = dag_of("做个 FPS 小游戏");
        assert!(
            game.nodes.iter().any(|n| n.risk == RiskLevel::HumanGate),
            "手感/审美类节点应标 human_gate"
        );
        let auto = dag_of("自动打开浏览器导出表格");
        assert!(
            auto.nodes.iter().any(|n| n.risk == RiskLevel::HumanGate),
            "真实操控电脑执行前应要人点头"
        );
    }

    #[test]
    fn hint_renders_dependencies_and_acceptance() {
        let hint = dag_of("做个 3D 分子网站").to_hint();
        assert!(hint.contains("任务分解"));
        assert!(hint.contains("验收："));
        assert!(hint.contains("依赖："));
    }
}
