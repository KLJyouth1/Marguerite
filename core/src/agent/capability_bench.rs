//! W4.7 能力评测集（Capability Bench 60）。
//!
//! 60 条**离线确定性**用例：给定「用户意图文本 + 当前工具面」，检查注入的路由函数
//! 是否把它送到期望的能力族、是否给出了期望工具、是否返回了禁止的工具（例如
//! 「本应结构化、却写脚本替代」）。
//!
//! 它**不是**真实模型的基准：本模块不联网、不读时钟、不写文件，只评测注入的
//! `route_fn`（离线纯函数）。真实模型端到端评测需要 API key，见
//! `scripts/bench/capability/README.md`；不得把未跑过的在线结果当作基准。
//!
//! 覆盖 11 个能力族：file 10 / shell 5 / browser 10 / desktop.uia 7 /
//! desktop.pixel 3 / memory 5 / kb 5 / skill 5 / mcp 5 / cron 3 / team 2。
//! 其中 55 条是「本应结构化、不得脚本替代」的负例（`forbidden_tools` 含
//! `shell_exec` / `code_exec`）。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::capability_cards::{CapabilityFamily, RouteDecision};
use crate::model::llm::ToolDef;

/// 负例的禁止工具：脚本工具替代结构化能力。
const SCRIPT_TOOLS: &[&str] = &["shell_exec", "code_exec"];
/// 正例（shell 族）不设禁止工具：它本来就是脚本/命令能力。
const NO_FORBIDDEN: &[&str] = &[];

/// 一条评测用例（全部字段为静态数据，保证离线与确定性）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BenchTask {
    /// 稳定 id（`file-01` … `team-02`），用于报告与回归对比。
    pub id: &'static str,
    /// 真实用户说法（中英混合），不出现内部术语。
    pub intent: &'static str,
    /// 期望命中的能力族。
    pub expected_family: CapabilityFamily,
    /// 期望路由结果里出现的工具（任一即可；空 = 该族模型侧无工具，如 team）。
    pub expected_any_tools: &'static [&'static str],
    /// 禁止出现在路由结果里的工具（例如「写脚本替代」）。
    pub forbidden_tools: &'static [&'static str],
}

macro_rules! bench_task {
    ($id:expr, $family:ident, $intent:expr, $expected:expr, $forbidden:expr) => {
        BenchTask {
            id: $id,
            intent: $intent,
            expected_family: CapabilityFamily::$family,
            expected_any_tools: $expected,
            forbidden_tools: $forbidden,
        }
    };
}

/// 60 条用例（顺序固定 = 报告顺序固定）。
const TASKS: &[BenchTask] = &[
    // ── file（10）────────────────────────────────────────────────────────
    bench_task!(
        "file-01",
        File,
        "读一下 README.md 这个文件，告诉我安装步骤",
        &["read_file"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "file-02",
        File,
        "把 src/main.rs 里的 main 函数改一下，加上错误处理",
        &["str_replace_editor", "apply_patch"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "file-03",
        File,
        "列出项目根目录下有哪些目录",
        &["list_dir"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "file-04",
        File,
        "在 config.yaml 里写入默认端口 18790",
        &["write_file"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "file-05",
        File,
        "把 utils.py 里的旧函数名替换成新名字",
        &["str_replace_editor"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "file-06",
        File,
        "给这个补丁文件应用一下 apply patch",
        &["apply_patch"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "file-07",
        File,
        "重构一下 agent 模块里的重复代码",
        &["str_replace_editor", "apply_patch"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "file-08",
        File,
        "在代码里 grep 一下 MAX_INITIAL_TOOLS 定义在哪",
        &["grep"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "file-09",
        File,
        "读取 docs/development.md 的冒烟测试章节",
        &["read_file"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "file-10",
        File,
        "list_dir 看看 webui 目录下有哪些子目录",
        &["list_dir"],
        SCRIPT_TOOLS
    ),
    // ── shell（5，正例：不禁止脚本工具）──────────────────────────────────
    bench_task!(
        "shell-01",
        Shell,
        "运行测试套件看看有没有失败",
        &["shell_exec", "code_exec"],
        NO_FORBIDDEN
    ),
    bench_task!(
        "shell-02",
        Shell,
        "编译这个 Rust 项目",
        &["shell_exec", "code_exec"],
        NO_FORBIDDEN
    ),
    bench_task!(
        "shell-03",
        Shell,
        "用 git 看一下最近的提交记录",
        &["shell_exec", "code_exec"],
        NO_FORBIDDEN
    ),
    bench_task!(
        "shell-04",
        Shell,
        "npm install 安装依赖，然后把构建跑起来",
        &["shell_exec", "code_exec"],
        NO_FORBIDDEN
    ),
    bench_task!(
        "shell-05",
        Shell,
        "执行一条命令看看当前路径",
        &["shell_exec", "code_exec"],
        NO_FORBIDDEN
    ),
    // ── browser（10）─────────────────────────────────────────────────────
    bench_task!(
        "browser-01",
        Browser,
        "打开浏览器登录公司后台",
        &["browser"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "browser-02",
        Browser,
        "在这个网站上填写表单并提交",
        &["browser"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "browser-03",
        Browser,
        "抓取这个网页的标题和正文",
        &["browser"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "browser-04",
        Browser,
        "导航到 GitHub 的仓库页面",
        &["browser"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "browser-05",
        Browser,
        "点击链接进入订单详情页",
        &["browser"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "browser-06",
        Browser,
        "帮我在网页上搜索一下机票",
        &["browser"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "browser-07",
        Browser,
        "登录这个网站，用户名密码我发你",
        &["browser"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "browser-08",
        Browser,
        "把这个表单里的地址改成公司地址",
        &["browser"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "browser-09",
        Browser,
        "浏览器里刷新一下页面看看有没有报错",
        &["browser"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "browser-10",
        Browser,
        "抓取网页里的表格数据导出",
        &["browser"],
        SCRIPT_TOOLS
    ),
    // ── desktop.uia（7）─────────────────────────────────────────────────
    bench_task!(
        "desktop-01",
        DesktopUia,
        "打开记事本，点一下保存按钮",
        &["desktop_launch", "desktop_action"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "desktop-02",
        DesktopUia,
        "在计算器应用里点击乘号",
        &["desktop_launch", "desktop_action"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "desktop-03",
        DesktopUia,
        "把这个窗口最大化的按钮点一下",
        &["desktop_action"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "desktop-04",
        DesktopUia,
        "在弹出的弹窗里点确定",
        &["desktop_action"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "desktop-05",
        DesktopUia,
        "在输入框里输入我的邮箱然后回车",
        &["desktop_action"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "desktop-06",
        DesktopUia,
        "打开系统设置菜单，找到蓝牙开关",
        &["desktop_launch", "desktop_action"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "desktop-07",
        DesktopUia,
        "点开微信窗口里的设置按钮",
        &["desktop_launch", "desktop_action"],
        SCRIPT_TOOLS
    ),
    // ── desktop.pixel（3）───────────────────────────────────────────────
    bench_task!(
        "pixel-01",
        DesktopPixel,
        "在游戏里按住鼠标拖拽角色移动",
        &["computer_use"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "pixel-02",
        DesktopPixel,
        "截个图看看屏幕现在显示什么",
        &["computer_use"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "pixel-03",
        DesktopPixel,
        "在画布上把图形拖拽到目标位置",
        &["computer_use"],
        SCRIPT_TOOLS
    ),
    // ── memory（5）──────────────────────────────────────────────────────
    bench_task!(
        "memory-01",
        Memory,
        "记住我偏好用中文回复",
        &["memory_save", "memory_search"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "memory-02",
        Memory,
        "上次我们说到的部署方案是什么",
        &["memory_search"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "memory-03",
        Memory,
        "你之前说会帮我记住这个习惯",
        &["memory_save", "memory_search"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "memory-04",
        Memory,
        "我有什么习惯是你记住的",
        &["memory_search"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "memory-05",
        Memory,
        "调用记忆查一下我上次的偏好",
        &["memory_search"],
        SCRIPT_TOOLS
    ),
    // ── knowledge（5）───────────────────────────────────────────────────
    bench_task!(
        "kb-01",
        Knowledge,
        "在知识库里检索一下部署规范",
        &["kb_search"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "kb-02",
        Knowledge,
        "把这份资料灌入知识库",
        &["kb_ingest"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "kb-03",
        Knowledge,
        "查一下员工手册里怎么申请权限",
        &["kb_search"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "kb-04",
        Knowledge,
        "文档库里有没有 API 规范文档",
        &["kb_search"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "kb-05",
        Knowledge,
        "检索一下知识库中关于缓存的设计资料",
        &["kb_search"],
        SCRIPT_TOOLS
    ),
    // ── skill（5）───────────────────────────────────────────────────────
    bench_task!(
        "skill-01",
        Skill,
        "用已有技能走一下发布流程",
        &["list_skills", "use_skill"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "skill-02",
        Skill,
        "有没有做安全审计的最佳实践",
        &["list_skills"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "skill-03",
        Skill,
        "列出你当前可用的技能",
        &["list_skills"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "skill-04",
        Skill,
        "这个任务该用什么方法做",
        &["list_skills", "use_skill"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "skill-05",
        Skill,
        "调用 use_skill 执行评审流程",
        &["use_skill"],
        SCRIPT_TOOLS
    ),
    // ── mcp（5）─────────────────────────────────────────────────────────
    // MCP 工具名是 sanitize 后的动态外部名；这里用代表性名字，
    // 真实部署以已连接 server 的实际工具名为准（只影响可见性统计）。
    bench_task!(
        "mcp-01",
        Mcp,
        "用已连接的 mcp 服务器查一下 GitHub 上的 issue",
        &["github_list_issues"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "mcp-02",
        Mcp,
        "从 Jira 这个外部系统拉一下我的工单",
        &["jira_search_issues"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "mcp-03",
        Mcp,
        "查一下集成到工作台的 Notion 里有哪些页面",
        &["notion_search"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "mcp-04",
        Mcp,
        "在数据库 MCP 里执行一条只读查询",
        &["postgres_query"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "mcp-05",
        Mcp,
        "用 mcp 工具在 Slack 里发一条消息",
        &["slack_post_message"],
        SCRIPT_TOOLS
    ),
    // ── cron（3）────────────────────────────────────────────────────────
    bench_task!(
        "cron-01",
        Cron,
        "每天早上九点提醒我喝水",
        &["automation"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "cron-02",
        Cron,
        "每周一自动生成周报",
        &["automation"],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "cron-03",
        Cron,
        "定时任务里加一个每小时同步的周期",
        &["automation"],
        SCRIPT_TOOLS
    ),
    // ── team（2，模型侧无工具，入口在编排层）───────────────────────────
    bench_task!(
        "team-01",
        Team,
        "让多个专家并行评审这个方案",
        &[],
        SCRIPT_TOOLS
    ),
    bench_task!(
        "team-02",
        Team,
        "把任务分工给团队里的成员",
        &[],
        SCRIPT_TOOLS
    ),
];

// 常量断言：用例数必须正好 60（新增/删除用例时这里会先编译失败）。
const _: () = assert!(TASKS.len() == 60);

/// 全部 60 条用例（顺序即报告顺序）。
pub fn tasks() -> &'static [BenchTask] {
    TASKS
}

/// 能力族 → 能力卡 id（与 `capability_cards::cards()` 一一对应）。
pub fn card_id_for_family(family: CapabilityFamily) -> &'static str {
    match family {
        CapabilityFamily::File => "file.edit",
        CapabilityFamily::Shell => "shell.oneshot",
        CapabilityFamily::Browser => "browser.a11y",
        CapabilityFamily::DesktopUia => "desktop.uia",
        CapabilityFamily::DesktopPixel => "desktop.pixel",
        CapabilityFamily::Memory => "memory",
        CapabilityFamily::Knowledge => "knowledge",
        CapabilityFamily::Skill => "skill",
        CapabilityFamily::Mcp => "mcp",
        CapabilityFamily::Cron => "cron",
        CapabilityFamily::Team => "team",
    }
}

/// 能力族的稳定短名（用于报告分组的键）。
pub fn family_id(family: CapabilityFamily) -> &'static str {
    match family {
        CapabilityFamily::File => "file",
        CapabilityFamily::Shell => "shell",
        CapabilityFamily::Browser => "browser",
        CapabilityFamily::DesktopUia => "desktop.uia",
        CapabilityFamily::DesktopPixel => "desktop.pixel",
        CapabilityFamily::Memory => "memory",
        CapabilityFamily::Knowledge => "kb",
        CapabilityFamily::Skill => "skill",
        CapabilityFamily::Mcp => "mcp",
        CapabilityFamily::Cron => "cron",
        CapabilityFamily::Team => "team",
    }
}

/// 单个能力族的通过统计。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FamilyTally {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub forbidden_hits: usize,
}

/// 一条失败用例的完整现场（期望 vs 实际 vs 原因）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchFailure {
    pub id: String,
    pub intent: String,
    pub expected_family: String,
    pub expected_card: String,
    pub expected_any_tools: Vec<String>,
    /// `None` = 路由函数没给出任何能力卡。
    pub actual_card: Option<String>,
    pub actual_tools: Vec<String>,
    pub reason: String,
}

/// 一次离线评测的结果（可序列化为 JSON；字段顺序稳定）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchReport {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    /// 首轮工具面里就能看到期望工具的用例数（看不到 = 路由再好也执行不了）。
    pub tool_visible_at_first_turn: usize,
    /// 返回了禁止工具的**任务数**（每个任务最多计 1 次）。
    pub forbidden_hits: usize,
    /// 按能力族分组（BTreeMap 保证键序稳定）。
    pub by_family: BTreeMap<String, FamilyTally>,
    /// 失败明细，顺序 = 用例顺序。
    pub failures: Vec<BenchFailure>,
}

/// 对 60 条用例跑一遍注入的路由函数。
///
/// 判定：能力卡 id 命中期望族 **且** 路由结果包含至少一个期望工具
/// （期望工具列表为空时只要求能力族命中，如 team）**且** 不含禁止工具。
/// `available` 只用于统计 `tool_visible_at_first_turn`，不参与通过判定
/// （工具不可见属于工具面问题，会单独计数，不被静默算成路由失败）。
pub fn evaluate(
    route_fn: impl Fn(&str, &[ToolDef]) -> Option<RouteDecision>,
    available: &[ToolDef],
) -> BenchReport {
    let available_names: BTreeSet<&str> = available.iter().map(|tool| tool.name.as_str()).collect();

    let mut report = BenchReport {
        total: TASKS.len(),
        passed: 0,
        failed: 0,
        tool_visible_at_first_turn: 0,
        forbidden_hits: 0,
        by_family: BTreeMap::new(),
        failures: Vec::new(),
    };

    for task in TASKS {
        let family = family_id(task.expected_family);
        let expected_card = card_id_for_family(task.expected_family);

        // 可见性：期望工具为空（team 等编排族）视为无需可见；
        // 否则首轮工具面必须至少含一个期望工具。
        let visible = task.expected_any_tools.is_empty()
            || task
                .expected_any_tools
                .iter()
                .any(|name| available_names.contains(name));
        if visible {
            report.tool_visible_at_first_turn += 1;
        }

        let decision = route_fn(task.intent, available);
        let actual_card = decision.as_ref().map(|d| d.card_id.to_string());
        let actual_tools: Vec<String> = decision
            .as_ref()
            .map(|d| d.tools.clone())
            .unwrap_or_default();

        let forbidden: Vec<&str> = actual_tools
            .iter()
            .map(String::as_str)
            .filter(|name| task.forbidden_tools.contains(name))
            .collect();
        let forbidden_hit = !forbidden.is_empty();
        if forbidden_hit {
            report.forbidden_hits += 1;
        }

        let card_ok = actual_card.as_deref() == Some(expected_card);
        let tools_ok = task.expected_any_tools.is_empty()
            || actual_tools
                .iter()
                .any(|name| task.expected_any_tools.contains(&name.as_str()));
        let passed = card_ok && tools_ok && !forbidden_hit;

        let tally = report.by_family.entry(family.to_string()).or_default();
        tally.total += 1;
        if forbidden_hit {
            tally.forbidden_hits += 1;
        }
        if passed {
            report.passed += 1;
            tally.passed += 1;
            continue;
        }
        report.failed += 1;
        tally.failed += 1;
        // 先算原因：`forbidden` 借用了 `actual_tools`，之后才能把后者移进报告。
        let reason = failure_reason(expected_card, task, card_ok, tools_ok, &forbidden);
        report.failures.push(BenchFailure {
            id: task.id.to_string(),
            intent: task.intent.to_string(),
            expected_family: family.to_string(),
            expected_card: expected_card.to_string(),
            expected_any_tools: task
                .expected_any_tools
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
            actual_card,
            actual_tools,
            reason,
        });
    }

    report
}

fn failure_reason(
    expected_card: &str,
    task: &BenchTask,
    card_ok: bool,
    tools_ok: bool,
    forbidden: &[&str],
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !card_ok {
        parts.push(format!("未命中期望能力卡 {expected_card}"));
    }
    if !tools_ok {
        parts.push(format!(
            "未返回期望工具之一（{}）",
            task.expected_any_tools.join(" / ")
        ));
    }
    if !forbidden.is_empty() {
        parts.push(format!("返回了禁止工具：{}", forbidden.join(" / ")));
    }
    if parts.is_empty() {
        "未知原因".to_string()
    } else {
        parts.join("；")
    }
}

fn md_cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

impl BenchReport {
    /// 生成可贴进报告的 Markdown 摘要（确定性输出，无时间戳）。
    pub fn render_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# Capability Bench 60 (offline routing)\n\n");
        out.push_str("| metric | value |\n|---|---|\n");
        out.push_str(&format!("| total | {} |\n", self.total));
        out.push_str(&format!("| passed | {} |\n", self.passed));
        out.push_str(&format!("| failed | {} |\n", self.failed));
        out.push_str(&format!(
            "| tool_visible_at_first_turn | {} |\n",
            self.tool_visible_at_first_turn
        ));
        out.push_str(&format!("| forbidden_hits | {} |\n", self.forbidden_hits));

        out.push_str(
            "\n## By capability family\n\n\
             | family | total | passed | failed | forbidden_hits |\n\
             |---|---|---|---|---|\n",
        );
        for (family, tally) in &self.by_family {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                family, tally.total, tally.passed, tally.failed, tally.forbidden_hits
            ));
        }

        out.push_str("\n## Failures\n\n");
        if self.failures.is_empty() {
            out.push_str("None.\n");
            return out;
        }
        out.push_str("| id | intent | expected | actual | reason |\n");
        out.push_str("|---|---|---|---|---|\n");
        for failure in &self.failures {
            let actual = failure.actual_card.as_deref().unwrap_or("(none)");
            let actual_tools = if failure.actual_tools.is_empty() {
                String::new()
            } else {
                format!(" [{}]", failure.actual_tools.join(", "))
            };
            out.push_str(&format!(
                "| {} | {} | {}{} | {}{} | {} |\n",
                md_cell(&failure.id),
                md_cell(&failure.intent),
                failure.expected_family,
                md_cell(&failure.expected_card),
                md_cell(actual),
                md_cell(&actual_tools),
                md_cell(&failure.reason),
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> ToolDef {
        ToolDef {
            name: name.to_string(),
            description: String::new(),
            parameters: serde_json::json!({ "type": "object" }),
        }
    }

    /// 完整工具面：60 条用例期望到的所有工具（去重）。
    fn full_face() -> Vec<ToolDef> {
        let mut names = BTreeSet::new();
        for task in tasks() {
            for name in task.expected_any_tools {
                names.insert(*name);
            }
        }
        names.into_iter().map(tool).collect()
    }

    /// 只路由到 shell 的假路由（最典型的「用脚本替代一切」退化）。
    fn shell_only_route(_: &str, _: &[ToolDef]) -> Option<RouteDecision> {
        Some(RouteDecision {
            card_id: "shell.oneshot",
            tools: vec!["shell_exec".to_string()],
            reason: "shell-only test double",
            fallback: &[],
        })
    }

    /// oracle 假路由：直接按用例的期望族/期望工具作答（验证评分逻辑本身）。
    fn oracle_route(intent: &str, _: &[ToolDef]) -> Option<RouteDecision> {
        let task = tasks().iter().find(|task| task.intent == intent)?;
        Some(RouteDecision {
            card_id: card_id_for_family(task.expected_family),
            tools: task
                .expected_any_tools
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
            reason: "oracle test double",
            fallback: &[],
        })
    }

    #[test]
    fn sixty_unique_tasks_cover_all_eleven_families_with_enough_negatives() {
        assert_eq!(tasks().len(), 60, "常量断言之外再做一次运行时校验");

        let ids: BTreeSet<&str> = tasks().iter().map(|task| task.id).collect();
        assert_eq!(ids.len(), 60, "用例 id 必须唯一");

        let intents: BTreeSet<&str> = tasks().iter().map(|task| task.intent).collect();
        assert_eq!(intents.len(), 60, "用例 intent 必须唯一（oracle 按 intent 反查）");

        let families: BTreeSet<&str> = tasks()
            .iter()
            .map(|task| family_id(task.expected_family))
            .collect();
        assert_eq!(families.len(), 11, "必须覆盖 11 个能力族: {families:?}");

        let negatives = tasks()
            .iter()
            .filter(|task| task.forbidden_tools.contains(&"shell_exec"))
            .count();
        assert!(
            negatives >= 12,
            "至少 12 条「本应结构化、不得脚本」的负例，实际 {negatives}"
        );
    }

    #[test]
    fn oracle_route_passes_all_sixty() {
        let report = evaluate(oracle_route, &full_face());
        assert_eq!(report.total, 60);
        assert_eq!(report.passed, 60, "oracle 应全过: {:?}", report.failures);
        assert_eq!(report.failed, 0);
        assert_eq!(report.forbidden_hits, 0);
        assert_eq!(report.tool_visible_at_first_turn, 60);
        assert!(report.failures.is_empty());
    }

    #[test]
    fn shell_only_route_trips_the_forbidden_negatives() {
        let report = evaluate(shell_only_route, &full_face());
        assert!(
            report.forbidden_hits > 0,
            "全部路由到 shell 时负例必须命中 forbidden"
        );
        assert!(
            report.forbidden_hits >= 12,
            "负例至少 12 条，实际 {}",
            report.forbidden_hits
        );
        assert!(report.failed > 0);
        assert_eq!(report.passed, 5, "只有 shell 族的 5 条正例应通过");
        let desktop = report
            .failures
            .iter()
            .find(|failure| failure.id == "desktop-01")
            .expect("desktop-01 必须失败");
        assert!(
            desktop.reason.contains("禁止工具"),
            "失败原因必须指出禁止工具: {}",
            desktop.reason
        );
        assert_eq!(desktop.actual_card.as_deref(), Some("shell.oneshot"));
    }

    #[test]
    fn json_and_markdown_are_non_empty_and_byte_deterministic() {
        for route in [oracle_route as fn(&str, &[ToolDef]) -> Option<RouteDecision>, shell_only_route] {
            let first = evaluate(route, &full_face());
            let second = evaluate(route, &full_face());

            let json_a = serde_json::to_string_pretty(&first).expect("serialize report");
            let json_b = serde_json::to_string_pretty(&second).expect("serialize report");
            assert!(!json_a.is_empty());
            assert_eq!(json_a, json_b, "两次评测的 JSON 必须逐字节相同");
            assert!(json_a.contains("\"total\": 60"), "{json_a}");

            let parsed: serde_json::Value = serde_json::from_str(&json_a).expect("valid json");
            assert_eq!(parsed["total"], 60);
            assert_eq!(parsed["passed"], first.passed);
            assert_eq!(parsed["forbidden_hits"], first.forbidden_hits);
            assert!(parsed["by_family"].is_object());
            assert!(parsed["failures"].is_array());

            let md_a = first.render_markdown();
            let md_b = second.render_markdown();
            assert!(!md_a.is_empty());
            assert_eq!(md_a, md_b, "两次评测的 Markdown 必须逐字节相同");
            assert!(md_a.contains("| total | 60 |"), "{md_a}");
            assert!(md_a.contains("## By capability family"), "{md_a}");
        }
    }

    #[test]
    fn report_family_tallies_add_up_to_the_total() {
        let report = evaluate(shell_only_route, &full_face());
        let total: usize = report.by_family.values().map(|tally| tally.total).sum();
        let passed: usize = report.by_family.values().map(|tally| tally.passed).sum();
        let failed: usize = report.by_family.values().map(|tally| tally.failed).sum();
        let forbidden: usize = report
            .by_family
            .values()
            .map(|tally| tally.forbidden_hits)
            .sum();
        assert_eq!(total, report.total);
        assert_eq!(passed, report.passed);
        assert_eq!(failed, report.failed);
        assert_eq!(forbidden, report.forbidden_hits);
        assert_eq!(report.by_family["shell"].passed, 5);
    }
}
