//! 能力卡（W4.1/W4.5）：把「真实能力面 + 何时用哪个 + 禁止用什么替代」做成
//! **确定性文本**，注入系统提示的稳定半区。
//!
//! 背景（用户投诉 + 代码事实）：默认工具面只有 7 个、其中 2 个是脚本工具；
//! 能力自述 `brief()` 零调用；提示词只说「优先用工具」，没有「意图 → 能力族 →
//! 工具」的映射，也没有「禁止用脚本替代」的负规则。于是模型的第一反应是写
//! `pyautogui`/`selenium` 脚本去"盲控"本机，而不是调用 `desktop_action`/`browser`。
//!
//! 本模块只负责**生成文本与路由**，不执行任何动作；可用性永远来自调用方传入的
//! 真实工具名（本轮 function schema 里的那份），绝不猜测。

use serde_json::Value;

use crate::model::llm::ToolDef;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityFamily {
    File,
    Shell,
    Browser,
    DesktopUia,
    DesktopPixel,
    Memory,
    Knowledge,
    Skill,
    Mcp,
    Cron,
    Team,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostClass {
    Cheap,
    Medium,
    Expensive,
    Interactive,
}

/// 一张能力卡：工具名是**模型可直接调用**的真实名字；`entries` 只是说明性入口。
pub struct CapabilityCard {
    pub id: &'static str,
    pub family: CapabilityFamily,
    /// 模型可调用的真实工具名（空 = 本轮模型无工具，只能由 UI/编排层触发）
    pub tools: &'static [&'static str],
    /// UI / 编排 RPC 入口（仅供说明，不参与路由匹配）
    pub entries: &'static [&'static str],
    pub triggers: &'static [&'static str],
    pub prerequisites: &'static [&'static str],
    pub fallback: &'static [&'static str],
    pub forbidden_shortcuts: &'static [&'static str],
    pub cost: CostClass,
}

/// 首批能力卡。顺序固定（稳定半区必须逐字节可复现）。
pub fn cards() -> &'static [CapabilityCard] {
    &[
        CapabilityCard {
            id: "file.edit",
            family: CapabilityFamily::File,
            tools: &[
                "read_file",
                "write_file",
                "str_replace_editor",
                "apply_patch",
                "list_dir",
                "grep",
            ],
            entries: &[],
            triggers: &["文件", "代码", "目录", "读取", "写入", "替换", "补丁", "重构", "改一下"],
            prerequisites: &["已绑定工作区根目录"],
            fallback: &["shell_exec（仅当文件工具不可用）"],
            forbidden_shortcuts: &["用 python / node 脚本读写文件（open()/Path()/cat >）"],
            cost: CostClass::Cheap,
        },
        CapabilityCard {
            id: "shell.oneshot",
            family: CapabilityFamily::Shell,
            tools: &["shell_exec", "code_exec"],
            entries: &[],
            triggers: &["命令", "构建", "编译", "测试", "运行", "安装依赖", "git"],
            prerequisites: &["工作目录 = 工作区根"],
            fallback: &["code_exec（跑一段临时代码）"],
            forbidden_shortcuts: &["用 shell 启动 GUI/模拟鼠标键盘（应走 desktop.*）"],
            cost: CostClass::Medium,
        },
        CapabilityCard {
            id: "browser.a11y",
            family: CapabilityFamily::Browser,
            tools: &["browser"],
            entries: &[],
            triggers: &["网页", "网站", "浏览器", "登录", "表单", "抓取", "导航", "点击链接"],
            prerequisites: &["浏览器后端在线"],
            fallback: &["web_fetch（只读抓取）"],
            forbidden_shortcuts: &["用 curl/requests/selenium/playwright 脚本替代浏览器交互"],
            cost: CostClass::Interactive,
        },
        CapabilityCard {
            id: "desktop.uia",
            family: CapabilityFamily::DesktopUia,
            tools: &["desktop_launch", "desktop_attach", "desktop_action"],
            entries: &[],
            triggers: &["记事本", "计算器", "窗口", "按钮", "点击", "输入框", "菜单", "弹窗", "应用"],
            prerequisites: &["Windows UI Automation", "用 desktop_launch 启动新应用，或 desktop_attach 附着已打开窗口"],
            fallback: &["computer_use（像素级兜底）"],
            forbidden_shortcuts: &["用 pyautogui/uiautomation/pywinauto/autoit/ahk 脚本"],
            cost: CostClass::Interactive,
        },
        CapabilityCard {
            id: "desktop.pixel",
            family: CapabilityFamily::DesktopPixel,
            tools: &["computer_use"],
            entries: &[],
            triggers: &["游戏", "画布", "截图", "像素", "远程桌面", "拖拽"],
            prerequisites: &["屏幕权限"],
            fallback: &[],
            forbidden_shortcuts: &[],
            cost: CostClass::Expensive,
        },
        CapabilityCard {
            id: "memory",
            family: CapabilityFamily::Memory,
            tools: &["memory_search", "memory_save"],
            entries: &["memory.tiered.status"],
            triggers: &["记住", "记忆", "上次", "之前说", "偏好", "习惯", "跨会话"],
            prerequisites: &["记忆已初始化"],
            fallback: &["直接问用户"],
            forbidden_shortcuts: &["把「记忆」写进代码/脚本或临时文件"],
            cost: CostClass::Cheap,
        },
        CapabilityCard {
            id: "knowledge",
            family: CapabilityFamily::Knowledge,
            tools: &["kb_search", "kb_ingest"],
            entries: &[],
            triggers: &["知识库", "文档库", "资料", "检索", "手册", "规范"],
            prerequisites: &["知识库索引可用"],
            fallback: &["grep 文档目录"],
            forbidden_shortcuts: &["用脚本遍历全盘找资料"],
            cost: CostClass::Medium,
        },
        CapabilityCard {
            id: "skill",
            family: CapabilityFamily::Skill,
            tools: &["search_skills", "list_skills", "use_skill"],
            entries: &[],
            triggers: &["技能", "流程", "方法", "怎么做", "最佳实践"],
            prerequisites: &["技能目录"],
            fallback: &["按通用流程直接执行"],
            forbidden_shortcuts: &["无视已有技能自己造流程"],
            cost: CostClass::Cheap,
        },
        CapabilityCard {
            id: "mcp",
            family: CapabilityFamily::Mcp,
            // MCP 工具名是动态的（sanitize 后的外部工具名），无法静态写死；
            // 渲染时给出权威枚举（配置的服务器名 + 运行中插件的工具名），
            // 数据源见 collect_mcp_facts / render_mcp_line。
            tools: &[],
            entries: &["plugins.mcp.list（管理面，不是模型工具）"],
            triggers: &["mcp", "外部系统", "集成", "数据库", "工单"],
            prerequisites: &["MCP server 已连接"],
            fallback: &[],
            forbidden_shortcuts: &["用 HTTP 脚本绕过已连接的 MCP"],
            cost: CostClass::Medium,
        },
        CapabilityCard {
            id: "cron",
            family: CapabilityFamily::Cron,
            tools: &["automation"],
            entries: &["cron.list", "cron.create"],
            triggers: &["定时", "每天", "每周", "提醒", "周期", "重复"],
            prerequisites: &["调度器在线"],
            fallback: &["提醒用户手动执行"],
            forbidden_shortcuts: &["用 sleep/while 脚本轮询等待"],
            cost: CostClass::Cheap,
        },
        CapabilityCard {
            id: "team",
            family: CapabilityFamily::Team,
            // 模型侧没有 team 工具：入口是 UI/编排层 RPC。禁止假装它存在。
            tools: &[],
            entries: &["team.run", "team.run.background", "orchestrate.run", "subagent"],
            triggers: &["并行", "多个专家", "团队", "分工"],
            prerequisites: &["团队/专家已启用"],
            fallback: &["单代理顺序执行"],
            forbidden_shortcuts: &["串行脚本模拟并行", "假装存在 team 工具"],
            cost: CostClass::Expensive,
        },
    ]
}

fn tool_names(available: &[ToolDef]) -> Vec<String> {
    let mut names: Vec<String> = available.iter().map(|tool| tool.name.clone()).collect();
    names.sort();
    names.dedup();
    names
}

fn card_present_tools(card: &CapabilityCard, names: &[String]) -> Vec<&'static str> {
    card.tools
        .iter()
        .copied()
        .filter(|tool| names.iter().any(|name| name == tool))
        .collect()
}

/// MCP 权威快照。两个字段都来自**既有只读源**，不新增任何 RPC / 配置键：
/// - `configured`：config.yaml `mcp.servers` 中启用的服务器名（经 `capability::selfcheck`，
///   与调用方传入的 `mcp_count` 同源，30s 缓存）；
/// - `running_plugins`：`plugins::mcp_launch` 的内存快照（名字 + 工具名，不阻塞、不读盘）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct McpFacts {
    pub(crate) configured: Vec<String>,
    /// `(插件名, 该进程已上报的工具名)`，仅含 running=true 的条目。
    pub(crate) running_plugins: Vec<(String, Vec<String>)>,
}

/// 枚举上限：超限必须**如实标注被截断**（口径同 `tool_catalog` 的技能索引）。
const MAX_MCP_SERVERS_LISTED: usize = 12;
const MAX_MCP_TOOLS_LISTED: usize = 8;

/// 从既有源收集 MCP 事实；任何一环读不到都退化为空清单（诚实，不编造名字）。
fn collect_mcp_facts() -> McpFacts {
    let report = crate::agent::capability::selfcheck(&serde_json::json!({}));
    let mut configured: Vec<String> = report["mcp"]["configuredServers"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    configured.sort();
    configured.dedup();

    let running = crate::plugins::mcp_launch::list();
    let mut running_plugins: Vec<(String, Vec<String>)> = running["plugins"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let name = item.get("name").and_then(Value::as_str)?;
                    let mut tools: Vec<String> = item
                        .get("tools")
                        .and_then(Value::as_array)
                        .map(|tools| {
                            tools
                                .iter()
                                .filter_map(|t| t.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    // 排序保证同一状态下逐字节可复现（KV 前缀稳定）。
                    tools.sort();
                    Some((name.to_string(), tools))
                })
                .collect()
        })
        .unwrap_or_default();
    running_plugins.sort_by(|a, b| a.0.cmp(&b.0));

    McpFacts {
        configured,
        running_plugins,
    }
}

/// mcp 行的纯渲染：`(mcp_count, facts)` 决定文本，供单测注入合成事实。
///
/// `mcp_count` 以调用方为准（生产路径与 `configured` 同源于 selfcheck）；
/// 两者不一致时计数优先，名称只作补充说明。
fn render_mcp_line(mcp_count: usize, facts: &McpFacts) -> String {
    if mcp_count == 0 && facts.running_plugins.is_empty() {
        return "- mcp：不可用（未配置 MCP 服务器，也没有运行中的插件 MCP 进程）".to_string();
    }
    let mut clauses: Vec<String> = Vec::new();
    if mcp_count > 0 {
        let mut clause = format!("已配置 {mcp_count} 个 MCP 服务器");
        if facts.configured.is_empty() {
            clause.push_str("（未能读取到服务器名称清单）");
        } else {
            let shown: Vec<&str> = facts
                .configured
                .iter()
                .map(String::as_str)
                .take(MAX_MCP_SERVERS_LISTED)
                .collect();
            clause.push_str(&format!("：{}", shown.join("、")));
            let hidden = facts.configured.len().saturating_sub(shown.len());
            if hidden > 0 {
                clause.push_str(&format!("（另有 {hidden} 个未列出，见 plugins.mcp.list）"));
            }
        }
        clauses.push(clause);
    } else {
        clauses.push("未配置 MCP 服务器".to_string());
    }
    if !facts.running_plugins.is_empty() {
        let mut parts: Vec<String> = Vec::new();
        for (name, tools) in facts.running_plugins.iter().take(MAX_MCP_SERVERS_LISTED) {
            if tools.is_empty() {
                parts.push(format!("{name}（未暴露工具）"));
                continue;
            }
            let shown: Vec<&str> = tools
                .iter()
                .map(String::as_str)
                .take(MAX_MCP_TOOLS_LISTED)
                .collect();
            let hidden = tools.len().saturating_sub(shown.len());
            if hidden > 0 {
                parts.push(format!("{}（工具：{}，另有 {hidden} 个未列出）", name, shown.join("、")));
            } else {
                parts.push(format!("{}（工具：{}）", name, shown.join("、")));
            }
        }
        let mut clause = format!("运行中的插件 MCP：{}", parts.join("；"));
        let hidden_servers = facts.running_plugins.len().saturating_sub(MAX_MCP_SERVERS_LISTED);
        if hidden_servers > 0 {
            clause.push_str(&format!("（另有 {hidden_servers} 个插件进程未列出）"));
        }
        clauses.push(clause);
    }
    let mut line = format!("- mcp：{}", clauses.join("；"));
    if mcp_count > 0 {
        line.push_str("；其工具名会直接出现在本轮 function schema 里（未出现 = 未连接）");
    }
    if !facts.running_plugins.is_empty() {
        line.push_str("；插件进程的工具经 plugins.mcp.call 调用");
    }
    line.push_str("。禁止：用 HTTP 脚本绕过 MCP");
    line
}

/// 渲染能力卡文本。`mcp_count` 来自真实配置（`capability::selfcheck`）。
///
/// 规则：只列**当前真实可用**的工具；不可用的卡片以一行「不可用」保留（让模型
/// 知道「存在但离线」，而不是彻底隐藏）；纯编排型能力（team）明确写「本轮模型无工具」。
/// mcp 行除计数外还枚举服务器与工具名（`collect_mcp_facts`，只读既有源）。
pub fn render_stable_cards_with(available: &[ToolDef], mcp_count: usize) -> String {
    render_stable_cards_from_names(&tool_names(available), mcp_count)
}

/// 同 [`render_stable_cards_with`]，但直接接收工具名（调用方已排序/去重亦可）。
pub fn render_stable_cards_from_names(names: &[String], mcp_count: usize) -> String {
    render_stable_cards_from_names_with_mcp(names, mcp_count, &collect_mcp_facts())
}

/// 数据源显式注入的渲染入口（生产路径经 [`render_stable_cards_from_names`] 注入真实事实，
/// 单测注入合成事实），mcp 行之外的卡片行为不受影响。
fn render_stable_cards_from_names_with_mcp(
    names: &[String],
    mcp_count: usize,
    mcp: &McpFacts,
) -> String {
    let mut names = names.to_vec();
    names.sort();
    names.dedup();
    let names = names.as_slice();
    let mut lines = vec![
        "【此刻真实可用的能力】按「意图 → 工具」选择；有结构化工具时，禁止再用脚本替代。".to_string(),
    ];
    for card in cards() {
        let present = card_present_tools(card, names);
        let label = card.id;
        if card.id == "mcp" {
            lines.push(render_mcp_line(mcp_count, mcp));
            continue;
        }
        if card.id == "team" || card.tools.is_empty() {
            lines.push(format!(
                "- {label}：本轮模型无工具（入口：{}）；禁止：{}",
                card.entries.join(" / "),
                card.forbidden_shortcuts.join("；")
            ));
            continue;
        }
        if present.is_empty() {
            lines.push(format!(
                "- {label}：不可用（本轮工具面未提供 {}）",
                card.tools.join(" / ")
            ));
            continue;
        }
        let mut line = format!("- {label}：{}", present.join(" / "));
        if !card.triggers.is_empty() {
            line.push_str(&format!("；何时用：{}", card.triggers.join("、")));
        }
        if !card.prerequisites.is_empty() {
            line.push_str(&format!("；前置：{}", card.prerequisites.join("、")));
        }
        if !card.fallback.is_empty() {
            line.push_str(&format!("；回退：{}", card.fallback.join("、")));
        }
        if !card.forbidden_shortcuts.is_empty() {
            line.push_str(&format!("；禁止：{}", card.forbidden_shortcuts.join("；")));
        }
        lines.push(line);
    }
    lines.join("\n")
}

/// 便捷包装：没有 MCP 计数时按 0 处理（离线 / 纯 Rust 模式）。
pub fn render_stable_cards(available: &[ToolDef]) -> String {
    render_stable_cards_with(available, 0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDecision {
    pub card_id: &'static str,
    /// 当前可用的对应工具（第一个是首选）
    pub tools: Vec<String>,
    pub reason: &'static str,
    pub fallback: &'static [&'static str],
}

/// 按意图文本路由到能力卡：只返回**当前工具面真的能执行**的卡片。
///
/// 这是给编排层/守卫用的确定性映射（不是给模型的自由发挥留空间）：
/// - 命中触发器且该卡至少有一个可用工具 → `Some`；
/// - 纯编排卡（team/mcp）不参与路由（模型无工具）；
/// - 都不命中 → `None`（按普通对话处理）。
pub fn route(intent: &str, available: &[ToolDef]) -> Option<RouteDecision> {
    let text = intent.to_lowercase();
    if text.trim().is_empty() {
        return None;
    }
    let names = tool_names(available);
    for card in cards() {
        if card.tools.is_empty() {
            continue;
        }
        let present = card_present_tools(card, &names);
        if present.is_empty() {
            continue;
        }
        if card
            .triggers
            .iter()
            .any(|trigger| text.contains(&trigger.to_lowercase()))
        {
            return Some(RouteDecision {
                card_id: card.id,
                tools: present.iter().map(|tool| (*tool).to_string()).collect(),
                reason: "意图命中能力卡的触发器，且对应工具在当前工具面内",
                fallback: card.fallback,
            });
        }
    }
    None
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

    fn available(names: &[&str]) -> Vec<ToolDef> {
        names.iter().map(|name| tool(name)).collect()
    }

    #[test]
    fn render_is_byte_stable_and_lists_only_real_tools() {
        let tools = available(&[
            "read_file",
            "write_file",
            "shell_exec",
            "browser",
            "memory_search",
        ]);
        let a = render_stable_cards_with(&tools, 0);
        let b = render_stable_cards_with(&tools, 0);
        assert_eq!(a, b, "同一工具面必须逐字节可复现（KV 前缀稳定）");
        assert!(a.contains("read_file / write_file"), "{a}");
        assert!(a.contains("browser"), "{a}");
        assert!(a.contains("memory_search"), "{a}");
        // 不可用的能力保留一行（存在但离线），且不列假工具。
        assert!(a.contains("desktop.uia：不可用"), "{a}");
        assert!(!a.contains("desktop_action /"), "{a}");
        assert!(a.contains("本轮模型无工具"), "{a}");
        assert!(a.contains("不要") || a.contains("禁止"), "{a}");
    }

    #[test]
    fn mcp_line_reflects_the_real_server_count() {
        let tools = available(&["read_file"]);
        assert!(render_stable_cards_with(&tools, 0).contains("未配置 MCP 服务器"));
        assert!(render_stable_cards_with(&tools, 2).contains("已配置 2 个 MCP 服务器"));
    }

    fn facts(configured: &[&str], running: &[(&str, &[&str])]) -> McpFacts {
        McpFacts {
            configured: configured.iter().map(|s| s.to_string()).collect(),
            running_plugins: running
                .iter()
                .map(|(n, tools)| (n.to_string(), tools.iter().map(|t| t.to_string()).collect()))
                .collect(),
        }
    }

    #[test]
    fn mcp_card_line_enumerates_servers_and_plugin_tools() {
        let f = facts(&["fs", "gh"], &[("btpanel", &["run_cmd", "query_devices"])]);
        let text = render_stable_cards_from_names_with_mcp(&["read_file".to_string()], 2, &f);
        let line = text
            .lines()
            .find(|l| l.starts_with("- mcp："))
            .expect("mcp card line");
        assert!(line.contains("已配置 2 个 MCP 服务器"), "{line}");
        // 还原成「只有计数」的旧渲染会在这里判红：名字与工具必须被点名。
        assert!(line.contains("fs") && line.contains("gh"), "必须枚举配置的服务器名：{line}");
        assert!(line.contains("btpanel"), "必须枚举运行中的插件：{line}");
        assert!(
            line.contains("query_devices") && line.contains("run_cmd"),
            "必须带插件的工具名：{line}"
        );
        assert!(line.contains("禁止：用 HTTP 脚本绕过 MCP"), "{line}");
    }

    #[test]
    fn mcp_line_truncation_is_labeled_not_silent() {
        let f = McpFacts {
            configured: (0..15).map(|i| format!("srv{i:02}")).collect(),
            running_plugins: vec![("big".to_string(), (0..10).map(|i| format!("tool{i}")).collect())],
        };
        let line = render_mcp_line(15, &f);
        assert!(line.contains("另有 3 个未列出"), "服务器截断必须如实标注：{line}");
        assert!(!line.contains("srv14"), "被截断的条目不得出现：{line}");
        assert!(line.contains("另有 2 个未列出"), "工具截断必须如实标注：{line}");
        assert!(!line.contains("tool9"), "{line}");
    }

    #[test]
    fn mcp_line_without_any_source_admits_nothing_configured() {
        let line = render_mcp_line(0, &McpFacts::default());
        assert!(line.contains("不可用"), "{line}");
        assert!(line.contains("未配置 MCP 服务器"), "{line}");
        // 不得为好看伪造条目
        assert!(!line.contains("工具："), "{line}");
    }

    #[test]
    fn mcp_line_surfaces_running_plugins_even_with_zero_config() {
        let f = facts(&[], &[("gh", &["create_issue"])]);
        let line = render_mcp_line(0, &f);
        assert!(line.contains("未配置 MCP 服务器"), "配置面要如实陈述：{line}");
        assert!(!line.contains("不可用"), "有运行中插件时不得整行判不可用：{line}");
        assert!(line.contains("gh（工具：create_issue）"), "{line}");
    }

    #[test]
    fn live_mcp_facts_are_sorted_and_deduplicated_for_kv_stability() {
        let f = collect_mcp_facts();
        let mut sorted = f.configured.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(f.configured, sorted, "配置的服务器名必须排序去重（稳定半区）");
        let mut running_names: Vec<&str> = f.running_plugins.iter().map(|(n, _)| n.as_str()).collect();
        running_names.sort();
        let current: Vec<&str> = f.running_plugins.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(current, running_names, "插件条目必须按名字排序");
        for (_, tools) in &f.running_plugins {
            let mut st = tools.clone();
            st.sort();
            assert_eq!(tools, &st, "工具名必须排序");
        }
    }

    #[test]
    fn route_prefers_the_matching_capability_and_never_invents_tools() {
        let tools = available(&[
            "desktop_launch",
            "desktop_action",
            "browser",
            "memory_search",
            "shell_exec",
            "read_file",
            "automation",
        ]);
        let desktop = route("帮我点一下记事本的保存按钮", &tools).expect("desktop route");
        assert_eq!(desktop.card_id, "desktop.uia");
        assert_eq!(desktop.tools[0], "desktop_launch");

        let browser = route("登录这个网站并提交表单", &tools).expect("browser route");
        assert_eq!(browser.card_id, "browser.a11y");

        let memory = route("记住我的偏好：回复要短", &tools).expect("memory route");
        assert_eq!(memory.card_id, "memory");

        let cron = route("每天早上九点提醒我", &tools).expect("cron route");
        assert_eq!(cron.card_id, "cron");
    }

    #[test]
    fn route_returns_none_when_the_matching_tools_are_not_available() {
        // 只有文件工具时，「点记事本」不能路由到 desktop（工具不可见 → 不装懂）。
        let tools = available(&["read_file"]);
        assert!(route("点一下记事本", &tools).is_none());
        assert!(route("随便聊聊", &tools).is_none());
    }

    #[test]
    fn team_card_never_advertises_a_model_side_tool() {
        let text = render_stable_cards(&available(&["read_file"]));
        let team_line = text
            .lines()
            .find(|line| line.contains("team："))
            .expect("team card line");
        assert!(team_line.contains("本轮模型无工具"), "{team_line}");
        assert!(team_line.contains("team.run"), "{team_line}");
    }
}
