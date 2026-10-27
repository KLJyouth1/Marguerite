//! 愿望解析器（Wish Parser）——把一句口语翻译成可执行的「六要素 + 硬约束」契约。
//!
//! # 为什么需要这一层
//!
//! 现状：用户说「做个炫酷的分子 3D 网站」，引擎把整句原样丢给模型，模型各自理解
//! —— 于是出现「漏条目、用占位符、缩写号、方向跑偏」这些「开盲盒」结果。
//! [`crate::governance::brief`] 早就定义了六要素与硬约束的**数据结构**，但全仓
//! **零生产调用点**（本次改造前实测：`grep RequirementBrief` 只命中 mod.rs 的 re-export）。
//!
//! 本模块补的是「人说愿望 → 机器拿到契约」这一步，并刻意遵守两条产品原则：
//!
//! 1. **能推的就推，推不出就明说假设**——绝不因为「用户没说技术栈」就停下来反问。
//!    用户的诉求是「一句话就干完」，不是「被问卷拦住」。
//! 2. **模糊词要翻译，不要复述**——「炫酷」「手感不对」这类词不能原样塞回给模型
//!    （模型会各自发挥），而是登记成「必须落成具体决定」的待办。
//!
//! 纯逻辑、零网络、零 LLM：解析结果可复现、可单测，也是「同一句话每次得到同一契约」
//! 的前提（提示词前缀稳定 → 缓存可命中）。

use serde::{Deserialize, Serialize};

use crate::governance::brief::{
    HardConstraint, HardConstraintKind, RequirementBrief,
};

/// 任务大类（决定默认交付物、默认假设、任务分解模板）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// 网页 / 前端 / 看板
    WebApp,
    /// 3D / 可视化 / 分子 / 场景
    ThreeD,
    /// 游戏（含 FPS / 闯关 / 小游戏）
    Game,
    /// 文档 / 表格 / PPT / 报告
    Document,
    /// 音视频 / 图片处理
    Media,
    /// 自动化 / 批量 / 定时 / 操控电脑
    Automation,
    /// 调研 / 检索 / 对比分析
    Research,
    /// 代码 / 修 bug / 重构 / 测试
    Code,
    /// 未识别
    Unknown,
}

impl TaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskKind::WebApp => "web_app",
            TaskKind::ThreeD => "three_d",
            TaskKind::Game => "game",
            TaskKind::Document => "document",
            TaskKind::Media => "media",
            TaskKind::Automation => "automation",
            TaskKind::Research => "research",
            TaskKind::Code => "code",
            TaskKind::Unknown => "unknown",
        }
    }

    /// 人类可读名（写进提示词）
    pub fn label_zh(self) -> &'static str {
        match self {
            TaskKind::WebApp => "网页应用",
            TaskKind::ThreeD => "3D / 可视化",
            TaskKind::Game => "游戏",
            TaskKind::Document => "文档 / 表格",
            TaskKind::Media => "音视频 / 图片处理",
            TaskKind::Automation => "自动化 / 电脑操控",
            TaskKind::Research => "联网调研",
            TaskKind::Code => "代码工程",
            TaskKind::Unknown => "通用任务",
        }
    }

    /// 该类任务是否天然依赖「数据/内容清单」（决定 data_source 是否为必选要素）。
    pub fn is_data_sensitive(self) -> bool {
        matches!(
            self,
            TaskKind::ThreeD | TaskKind::Research | TaskKind::Document | TaskKind::Game
        )
    }

    /// 该类任务是否「有画面」（决定视觉风格/交互是否该给默认值）。
    pub fn is_visual(self) -> bool {
        matches!(
            self,
            TaskKind::WebApp | TaskKind::ThreeD | TaskKind::Game
        )
    }

    /// 该类任务建议优先使用的工具（写进提示词，避免模型「手里有锤子却用牙咬」）。
    pub fn suggested_tools(self) -> &'static [&'static str] {
        match self {
            TaskKind::WebApp => &["write_file", "media", "browser", "shell_exec"],
            TaskKind::ThreeD | TaskKind::Game => {
                &["blender", "write_file", "media", "browser", "shell_exec"]
            }
            TaskKind::Media => &["media", "write_file"],
            TaskKind::Document => &["office_automation", "write_file", "read_file"],
            TaskKind::Automation => &["system_control", "computer_use", "shell_exec", "process_manage"],
            TaskKind::Research => &["web_search", "web_fetch", "browser", "write_file"],
            TaskKind::Code => &["read_file", "edit_file", "shell_exec", "grep", "git"],
            TaskKind::Unknown => &["web_search", "read_file", "write_file"],
        }
    }
}

/// 解析产物：一份可以直接写进系统提示词的「执行契约」。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WishDraft {
    /// 原始口语（逐字保留，审计用）
    pub raw: String,
    /// 是否判定为「任务型」输入（false = 闲聊/提问，不注入契约）
    pub is_wish: bool,
    pub kind: TaskKind,
    /// 六要素（已按假设补全）
    pub brief: RequirementBrief,
    /// 补全时用到的假设——必须向用户明示，不能偷偷决定
    pub assumptions: Vec<String>,
    /// 未能推出、且没有安全默认值的槽位（目前只在 goal 缺失时出现）
    pub unknown_slots: Vec<String>,
    /// 硬约束（否定式指令）
    pub constraints: Vec<HardConstraint>,
    /// 模糊词（「炫酷」「差不多」）——必须落成具体决定，不得原样反问
    pub vague_terms: Vec<String>,
    /// 是否需要先做最小切片（任务复杂时由 decomposer 决定，解析阶段只给建议）
    pub prefer_slice_first: bool,
}

impl WishDraft {
    /// 渲染成系统提示词片段。
    ///
    /// 只输出**模型需要遵守的事实**，不输出「你是一个解析器」这类元话语。
    /// 空字段整行省略（避免提示词里出现「视觉风格：」后面跟空白）。
    pub fn to_hint(&self) -> String {
        if !self.is_wish {
            return String::new();
        }
        let mut out = String::new();
        out.push_str("\n\n【执行契约（由用户一句话自动解析；按此执行，不要重新猜需求）】\n");
        out.push_str(&format!("任务类型：{}\n", self.kind.label_zh()));

        let b = &self.brief;
        out.push_str("六要素：\n");
        let mut push = |k: &str, v: &str| {
            if !v.trim().is_empty() {
                out.push_str(&format!("- {k}：{}\n", v.trim()));
            }
        };
        push("要做什么", &b.goal);
        push("怎么交互", &b.interaction);
        push("技术底线", &b.tech_floor);
        push("视觉风格", &b.visual_style);
        push("数据来源", &b.data_source);
        push("输出什么", &b.deliverable);

        if !self.assumptions.is_empty() {
            out.push_str("已替你做的假设（在回复开头用一句话说明，不要反问用户）：\n");
            for a in &self.assumptions {
                out.push_str(&format!("- {a}\n"));
            }
        }
        if !self.constraints.is_empty() {
            out.push_str("硬约束（交付前必须逐条自检；违反即视为未完成）：\n");
            for c in &self.constraints {
                out.push_str(&format!("- {}\n", c.raw));
            }
        }
        if !self.vague_terms.is_empty() {
            out.push_str(&format!(
                "模糊词：{} —— 必须把它们翻译成具体决定（配色/布局/动效/参数各选一个并说明），不要原样反问。\n",
                self.vague_terms.join("、")
            ));
        }
        let tools = self.kind.suggested_tools().join(" / ");
        out.push_str(&format!(
            "建议工具：{tools}（需要联网就联网、需要操控本机就调用对应工具，不要假设自己没有能力）。\n"
        ));
        out.push_str(
            "执行纪律：先做最小可验证切片并自检 → 方向对再全量 → 每步给出可验证证据（命令输出/文件路径/截图）；\
             中途不要为「你希望用什么框架」这类非阻塞问题打断用户。\n",
        );
        out
    }
}

/// 否定式指令 → 硬约束的映射表（顺序即优先级）。
///
/// 只登记**机器可判**的几类；其余否定式（如「不要用红色」）留在原文里由模型遵守，
/// 不硬塞进 `HardConstraintKind`（那会让检查器给出假的「已核对」）。
const CONSTRAINT_RULES: &[(&[&str], HardConstraintKind, &str)] = &[
    (
        &["占位符", "placeholder", "todo", "待补充", "待完善"],
        HardConstraintKind::NoPlaceholder,
        "不要使用占位符",
    ),
    (
        &["省略", "一个都不能少", "全部都要", "别漏"],
        HardConstraintKind::NoOmittedEntries,
        "不要省略任何条目",
    ),
    (
        &["缩写号", "省略号"],
        HardConstraintKind::NoEllipsis,
        "不要使用缩写号/省略号截断内容",
    ),
];

/// 模糊词表：这些词本身不是需求，必须被翻译成具体决定。
const VAGUE_WORDS: &[&str] = &[
    "炫酷", "酷炫", "好看", "漂亮", "高级感", "大气", "差不多", "随便", "大概", "你看着办",
    "手感不对", "不太对", "感觉不对", "再快点", "再慢点", "优化一下", "改好看点",
];

/// 任务型输入的动作词（用于区分「闲聊/提问」与「让我干活」）。
const WISH_MARKERS: &[&str] = &[
    "做", "写", "生成", "帮我", "给我", "搞", "弄", "改", "修", "部署", "爬", "整理",
    "转", "剪", "训练", "分析", "调研", "查", "搭", "建", "画", "设计", "实现", "重构",
    "导出", "合并", "拆分", "压缩", "翻译", "总结", "对比", "监控", "自动化", "批量",
];

/// 纯提问的标记（即使含动作词，也优先按「提问」处理）。
const QUESTION_MARKERS: &[&str] = &[
    "是什么", "为什么", "怎么理解", "有什么区别", "能不能解释", "是什么意思",
];

pub struct WishParser;

impl WishParser {
    /// 解析一句口语。纯函数：同输入必得同输出（提示词前缀稳定性的前提）。
    pub fn parse(raw: &str) -> WishDraft {
        let text = raw.trim();
        let lower = text.to_lowercase();

        let is_question = QUESTION_MARKERS.iter().any(|m| text.contains(m))
            || (text.ends_with('？') || text.ends_with('?'))
                && !text.contains("帮我")
                && !text.contains("给我");
        let has_marker = WISH_MARKERS.iter().any(|m| text.contains(m));
        // 短句（<6 字）且无动作词 → 视为闲聊，不注入契约（避免「你好」也生成六要素）
        let is_wish = has_marker && !is_question && text.chars().count() >= 6;

        if !is_wish {
            return WishDraft {
                raw: text.to_string(),
                is_wish: false,
                kind: TaskKind::Unknown,
                brief: RequirementBrief::default(),
                assumptions: Vec::new(),
                unknown_slots: Vec::new(),
                constraints: Vec::new(),
                vague_terms: Vec::new(),
                prefer_slice_first: false,
            };
        }

        let kind = detect_kind(&lower);
        let mut assumptions = Vec::new();
        let mut unknown_slots = Vec::new();

        // ── 六要素：先抽原文，再按 kind 补默认假设 ──
        let goal = text.to_string();
        let deliverable = extract_deliverable(&lower, kind)
            .unwrap_or_else(|| {
                let d = default_deliverable(kind);
                assumptions.push(format!("未说明交付格式 → 按「{d}」产出"));
                d
            });
        let tech_floor = extract_tech_floor(&lower).unwrap_or_else(|| {
            let t = default_tech_floor(kind);
            assumptions.push(format!("未指定技术栈 → {t}"));
            t
        });
        let visual_style = extract_visual_style(&lower, text).unwrap_or_else(|| {
            if kind.is_visual() {
                let v = "简洁现代、深色优先、响应式（桌面/移动都能用）";
                assumptions.push(format!("未指定视觉风格 → 默认{v}"));
                v.to_string()
            } else {
                String::new()
            }
        });
        let interaction = extract_interaction(&lower).unwrap_or_else(|| {
            if kind.is_visual() {
                let i = "鼠标拖拽旋转 + 点击切换 + 键盘快捷键";
                assumptions.push(format!("未说明交互 → 默认{i}"));
                i.to_string()
            } else {
                String::new()
            }
        });
        let data_source = extract_data_source(text, &lower).unwrap_or_else(|| {
            if kind.is_data_sensitive() {
                let d = "优先使用可公开获取的真实数据/权威来源；拿不到就明说缺什么，不编造";
                assumptions.push(format!("未指定数据来源 → {d}"));
                d.to_string()
            } else {
                String::new()
            }
        });

        // goal 永远可从原文得到；仍保留 unknown_slots 机制供未来扩展
        if goal.trim().is_empty() {
            unknown_slots.push("goal".to_string());
        }

        let constraints = extract_constraints(text, &lower);
        let vague_terms: Vec<String> = VAGUE_WORDS
            .iter()
            .filter(|w| text.contains(**w))
            .map(|w| w.to_string())
            .collect();

        // 复杂任务（多动作词 / 长句 / 含「整套/完整/全部」）建议先出切片
        let prefer_slice_first = text.chars().count() > 40
            || ["整套", "完整", "全部", "全量", "一键"]
                .iter()
                .any(|w| text.contains(w));

        WishDraft {
            raw: text.to_string(),
            is_wish: true,
            kind,
            brief: RequirementBrief {
                goal,
                interaction,
                tech_floor,
                visual_style,
                data_source,
                deliverable,
            },
            assumptions,
            unknown_slots,
            constraints,
            vague_terms,
            prefer_slice_first,
        }
    }
}

fn detect_kind(lower: &str) -> TaskKind {
    // 打分而非「首个命中」。
    //
    // 为什么：顺序匹配会误判真实需求 —— 实测「帮我自动打开浏览器登录后台并把表格导出来」
    // 里同时有「表格」（文档）与「自动」（自动化），按列表顺序会判成文档任务，
    // 于是给出一套「写文档」的拆解，与实际要干的「操控电脑」完全错位。
    //
    // 权重：强信号词（动作性、几乎无歧义）记 2 分，一般名词记 1 分。
    // 平分时按 RULES 的先后顺序取胜（越靠前越具体）。
    const RULES: &[(TaskKind, &[(&str, u32)])] = &[
        (
            TaskKind::Game,
            &[("游戏", 2), ("fps", 2), ("射击", 2), ("闯关", 1), ("小游戏", 2), ("game", 1)],
        ),
        (
            TaskKind::ThreeD,
            &[("3d", 2), ("three.js", 2), ("threejs", 2), ("分子", 2), ("渲染", 2), ("建模", 2), ("模型", 1), ("场景", 1)],
        ),
        (
            TaskKind::Media,
            &[("视频", 2), ("音频", 2), ("剪辑", 2), ("转码", 2), ("抽帧", 2), ("封面", 1), ("图片处理", 2), ("压缩视频", 2)],
        ),
        (
            TaskKind::Automation,
            &[
                ("自动化", 2), ("自动", 2), ("批量", 2), ("定时", 2), ("监控", 1),
                ("爬虫", 2), ("爬取", 2), ("脚本", 2), ("操控", 2), ("点击", 1),
                ("窗口", 1), ("导出", 1), ("登录", 1), ("打开", 1),
            ],
        ),
        (
            TaskKind::Document,
            &[("文档", 2), ("报告", 1), ("ppt", 2), ("word", 2), ("excel", 2), ("表格", 1), ("pdf", 2), ("幻灯片", 2)],
        ),
        (
            TaskKind::Research,
            &[("调研", 2), ("查一下", 2), ("搜索", 1), ("对比", 1), ("竞品", 2), ("资料", 1)],
        ),
        (
            TaskKind::WebApp,
            &[("网站", 2), ("网页", 2), ("页面", 1), ("html", 2), ("前端", 1), ("看板", 2), ("仪表盘", 2), ("web", 1)],
        ),
        (
            TaskKind::Code,
            &[("代码", 1), ("函数", 1), ("重构", 2), ("bug", 2), ("测试", 1), ("接口", 1), ("api", 1)],
        ),
    ];

    let mut best: Option<(TaskKind, u32)> = None;
    for (kind, keys) in RULES {
        let score: u32 = keys
            .iter()
            .filter(|(k, _)| lower.contains(k))
            .map(|(_, w)| *w)
            .sum();
        if score == 0 {
            continue;
        }
        // 严格大于才替换 → 平分时保留 RULES 中更靠前（更具体）的那一类
        if best.is_none_or(|(_, s)| score > s) {
            best = Some((*kind, score));
        }
    }
    best.map(|(k, _)| k).unwrap_or(TaskKind::Unknown)
}

fn default_deliverable(kind: TaskKind) -> String {
    match kind {
        TaskKind::WebApp | TaskKind::ThreeD | TaskKind::Game => {
            "可直接打开的网页（index.html，单文件或含本地静态资源），双击即用".into()
        }
        TaskKind::Media => "处理后的媒体文件（保留原文件，产物另存）".into(),
        TaskKind::Document => "可直接打开的文档文件（.docx/.xlsx/.pptx 或 .md）".into(),
        TaskKind::Automation => "可重复执行的脚本 + 一次真实运行的证据".into(),
        TaskKind::Research => "带来源链接的调研结论（每条结论可追溯）".into(),
        TaskKind::Code => "可运行的代码改动 + 验证输出".into(),
        TaskKind::Unknown => "完成该任务所需的产物文件 + 验证证据".into(),
    }
}

fn default_tech_floor(kind: TaskKind) -> String {
    match kind {
        TaskKind::WebApp | TaskKind::ThreeD | TaskKind::Game => {
            "原生 HTML/CSS/JS，依赖走 CDN；不引入需要构建的框架（保证双击可开）".into()
        }
        TaskKind::Media => "本机 ffmpeg（已装）；不改动原文件".into(),
        TaskKind::Document => "本机 Office/WPS 或纯文本生成；产物可直接打开".into(),
        TaskKind::Automation => "优先用系统自带能力（PowerShell/Python），不额外装重型依赖".into(),
        TaskKind::Research => "公开可访问来源；每条结论标注出处".into(),
        TaskKind::Code => "沿用仓库既有技术栈与风格；不引入新依赖除非必要".into(),
        TaskKind::Unknown => "最小依赖、可离线复现".into(),
    }
}

fn extract_deliverable(lower: &str, kind: TaskKind) -> Option<String> {
    const FORMATS: &[(&str, &str)] = &[
        ("index.html", "单文件 index.html"),
        ("html", "HTML 网页"),
        ("网页", "网页"),
        ("网站", "网站"),
        ("exe", "可执行程序 exe"),
        ("ppt", "PPT 幻灯片"),
        ("word", "Word 文档"),
        ("docx", "Word 文档"),
        ("excel", "Excel 表格"),
        ("xlsx", "Excel 表格"),
        ("pdf", "PDF"),
        ("md", "Markdown 文档"),
        ("mp4", "MP4 视频"),
        ("gif", "GIF 动图"),
        ("png", "PNG 图片"),
        ("脚本", "可执行脚本"),
        ("报告", "报告文档"),
    ];
    FORMATS
        .iter()
        .find(|(k, _)| lower.contains(k))
        .map(|(_, v)| v.to_string())
        .or_else(|| kind.is_visual().then(|| "网页".to_string()))
}

fn extract_tech_floor(lower: &str) -> Option<String> {
    const TECH: &[&str] = &[
        "three.js", "threejs", "react", "vue", "svelte", "next.js", "vite", "rust", "python",
        "golang", "node.js", "nodejs", "tauri", "electron", "flutter", "unity", "unreal",
        "blender", "ffmpeg", "opencv", "pytorch", "transformers",
    ];
    let hits: Vec<&str> = TECH.iter().copied().filter(|t| lower.contains(t)).collect();
    (!hits.is_empty()).then(|| format!("必须使用 {}", hits.join(" / ")))
}

fn extract_visual_style(lower: &str, raw: &str) -> Option<String> {
    const STYLES: &[&str] = &[
        "极简", "简约", "暗色", "深色", "亮色", "浅色", "赛博", "复古", "像素", "科技感",
        "左右分栏", "上下布局", "卡片", "玻璃拟态", "手绘", "国风",
    ];
    let hits: Vec<&str> = STYLES.iter().copied().filter(|s| raw.contains(*s)).collect();
    if !hits.is_empty() {
        return Some(hits.join(" / "));
    }
    // 「炫酷」这类模糊词不进 style（它需要被翻译），交给 vague_terms
    let _ = lower;
    None
}

fn extract_interaction(lower: &str) -> Option<String> {
    const ACTIONS: &[&str] = &[
        "拖拽", "拖动", "点击", "切换", "滚动", "缩放", "键盘", "搜索框", "输入框", "下拉",
        "上传", "下载", "全屏", "翻页",
    ];
    let hits: Vec<&str> = ACTIONS.iter().copied().filter(|a| lower.contains(a)).collect();
    (!hits.is_empty()).then(|| hits.join(" / "))
}

fn extract_data_source(raw: &str, lower: &str) -> Option<String> {
    const SOURCES: &[&str] = &[
        "教材", "考纲", "官网", "官方文档", "数据库", "api", "维基", "wikipedia", "github",
        "本地文件", "我的文件", "现有代码", "仓库",
    ];
    let hits: Vec<&str> = SOURCES.iter().copied().filter(|s| lower.contains(s)).collect();
    if hits.is_empty() {
        return None;
    }
    // 原文里通常还有「用 XX 为准」的限定语，直接带回原文片段更有信息量
    Some(format!("{}（原文：{}）", hits.join(" / "), raw.trim()))
}

fn extract_constraints(raw: &str, lower: &str) -> Vec<HardConstraint> {
    // 只在出现否定语气时才登记，避免「占位符」出现在正常描述里被误判
    let negated = ["不要", "别", "不能", "禁止", "不得", "无需", "不用"]
        .iter()
        .any(|n| raw.contains(n));
    if !negated {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (i, (keys, kind, canon)) in CONSTRAINT_RULES.iter().enumerate() {
        if keys.iter().any(|k| lower.contains(k)) {
            out.push(HardConstraint {
                id: format!("hc-{}", i + 1),
                raw: canon.to_string(),
                kind: *kind,
            });
        }
    }
    // 「溢出」类处理方式：登记为待人工确认的策略（机器判不了，但必须被看见）
    if raw.contains("溢出") {
        out.push(HardConstraint {
            id: format!("hc-{}", out.len() + 1),
            raw: "溢出时按指定方式处理（不得直接截断内容）".to_string(),
            kind: HardConstraintKind::OverflowPolicy,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chitchat_is_not_a_wish() {
        let d = WishParser::parse("你好呀");
        assert!(!d.is_wish, "闲聊不应生成契约");
        assert!(d.to_hint().is_empty());
    }

    #[test]
    fn pure_question_is_not_a_wish() {
        let d = WishParser::parse("Rust 和 Go 有什么区别？");
        assert!(!d.is_wish, "提问不应被当成任务");
    }

    #[test]
    fn three_d_wish_fills_six_elements_with_declared_assumptions() {
        let d = WishParser::parse("帮我做个高中化学分子的 3D 网站，要炫酷");
        assert!(d.is_wish);
        assert_eq!(d.kind, TaskKind::ThreeD);
        // 抽得到的：目标、交付物（网站/网页）
        assert!(d.brief.goal.contains("分子"));
        assert!(
            d.brief.deliverable.contains("网页") || d.brief.deliverable.contains("网站"),
            "交付物应识别为网页类：{}",
            d.brief.deliverable
        );
        // 抽不到的：必须给假设并明示，而不是空着
        assert!(!d.assumptions.is_empty(), "未指定的要素必须给假设");
        assert!(d.brief.tech_floor.contains("CDN"), "3D 默认技术底线应可双击打开");
        // 模糊词必须被登记（要翻译，不能复述）
        assert!(d.vague_terms.iter().any(|w| w == "炫酷"));
        let hint = d.to_hint();
        assert!(hint.contains("执行契约"));
        assert!(hint.contains("已替你做的假设"));
        assert!(hint.contains("不要原样反问"));
    }

    #[test]
    fn explicit_tech_stack_is_respected_not_overridden() {
        let d = WishParser::parse("用 three.js 做一个分子结构展示页面，鼠标拖拽旋转");
        assert_eq!(d.kind, TaskKind::ThreeD);
        assert!(d.brief.tech_floor.contains("three.js"));
        assert!(
            !d.assumptions.iter().any(|a| a.contains("未指定技术栈")),
            "用户说了技术栈就不该再假设：{:?}",
            d.assumptions
        );
        assert!(d.brief.interaction.contains("拖拽"));
    }

    #[test]
    fn negation_becomes_hard_constraint() {
        let d = WishParser::parse("做一个分子网站，不要用占位符，不要省略任何分子");
        assert!(d.is_wish);
        let kinds: Vec<HardConstraintKind> = d.constraints.iter().map(|c| c.kind).collect();
        assert!(kinds.contains(&HardConstraintKind::NoPlaceholder));
        assert!(kinds.contains(&HardConstraintKind::NoOmittedEntries));
        let hint = d.to_hint();
        assert!(hint.contains("硬约束"));
        assert!(hint.contains("占位符"));
    }

    #[test]
    fn plain_description_does_not_fabricate_constraints() {
        // 提到「占位符」但没有否定语气 → 不登记硬约束（避免假约束）
        let d = WishParser::parse("帮我写个组件，里面有个 placeholder 属性");
        assert!(d.constraints.is_empty(), "无否定语气不得伪造硬约束");
    }

    #[test]
    fn media_task_maps_to_media_tool_and_ffmpeg_floor() {
        let d = WishParser::parse("把这个视频剪成 15 秒并导出 mp4");
        assert_eq!(d.kind, TaskKind::Media);
        assert!(d.brief.tech_floor.contains("ffmpeg"));
        assert!(d.kind.suggested_tools().contains(&"media"));
        assert!(d.brief.deliverable.contains("MP4") || d.brief.deliverable.contains("媒体"));
    }

    #[test]
    fn automation_task_points_at_computer_control_tools() {
        let d = WishParser::parse("帮我自动打开浏览器登录后台并把表格导出来");
        assert_eq!(d.kind, TaskKind::Automation);
        let tools = d.kind.suggested_tools();
        assert!(
            tools.contains(&"computer_use") || tools.contains(&"system_control"),
            "电脑操控类任务应提示可用操控工具：{tools:?}"
        );
    }

    #[test]
    fn research_task_defaults_to_real_sources_not_fabrication() {
        let d = WishParser::parse("调研一下 2026 年的桌面 GUI agent 有哪些");
        assert_eq!(d.kind, TaskKind::Research);
        assert!(
            d.brief.data_source.contains("不编造") || d.brief.data_source.contains("真实"),
            "调研类必须默认「不编造」：{}",
            d.brief.data_source
        );
    }

    #[test]
    fn long_wish_prefers_slice_first() {
        let d = WishParser::parse(
            "帮我做一整套完整的 3D 分子学习网站，包含全部常见物质、测验、动画、部署上线",
        );
        assert!(d.prefer_slice_first, "复杂任务应先出最小切片");
    }

    #[test]
    fn parse_is_deterministic() {
        let a = WishParser::parse("做个 3D 分子网站，要炫酷");
        let b = WishParser::parse("做个 3D 分子网站，要炫酷");
        assert_eq!(a, b, "同输入必须同输出（提示词前缀稳定）");
    }
}
