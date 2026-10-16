//! 按需暴露工具 schema：首轮只发送常用工具，按需发现并加载其余完整 schema。
//!
//! 所有工具仍保留在本轮的本地目录里。`marguerite_discover_tools` 只负责发现能力，
//! 找到后会把工具的原始参数 schema 加入后续模型请求；它不执行工具，也不改变权限。

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::mode::ModeKind;
use crate::model::llm::ToolDef;

/// 模型可调用的内部发现工具（不注册进可执行工具表）。
pub const DISCOVER_TOOLS_NAME: &str = "_marguerite_discover_tools";
/// 工具回路的默认上下文窗口估计。与 `TaskKind::Tool` 的 profile 对齐。
pub const TOOL_SCHEMA_CONTEXT_WINDOW: usize = 128 * 1024;
/// 目标：首轮工具 schema 占用不超过工具回路上下文的四分之一。
pub const TOOL_SCHEMA_BUDGET_FRACTION: usize = 4;
/// 首轮至多暴露的真实工具数（另加一个轻量发现工具）。
const MAX_INITIAL_TOOLS: usize = 12;
/// 单次发现最多加入的 schema 数，防止一次模糊查询重新塞回整个目录。
const MAX_DISCOVERED_TOOLS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSchemaMetrics {
    pub available_tools: usize,
    pub initially_exposed_tools: usize,
    pub full_estimated_tokens: usize,
    pub initial_estimated_tokens: usize,
    pub budget_tokens: usize,
}

/// 一轮工具回路内的工具 schema 暴露状态。
#[derive(Debug, Clone)]
pub struct ToolDisclosure {
    available: Vec<ToolDef>,
    active_names: HashSet<String>,
    mode: ModeKind,
    enabled: bool,
    metrics: ToolSchemaMetrics,
}

impl ToolDisclosure {
    /// 为当前请求创建按需暴露策略。
    ///
    /// 即使一轮开始时只发少量 schema，未暴露工具也不会被永久隐藏：模型可调用
    /// `marguerite_discover_tools`，匹配到的完整原始 schema 会加入下一轮请求。
    pub fn new(all_tools: &[ToolDef], request: &str, mode: ModeKind) -> Self {
        let available: Vec<ToolDef> = all_tools
            .iter()
            .filter(|tool| !crate::mode::mode_denies_tool(mode, &tool.name))
            .cloned()
            .collect();
        // 小工具面（尤其 Bloom 锚定冷启动传入的 2–3 个工具）不应额外插入
        // discovery 工具或改变既有锚定语义；只有宽工具目录才启用按需 schema。
        let enabled = available.len() > MAX_INITIAL_TOOLS
            && !available
                .iter()
                .any(|tool| tool.name == DISCOVER_TOOLS_NAME);
        let mut selected: Vec<&ToolDef> = Vec::new();
        let mut selected_names = HashSet::new();

        if enabled {
            // 高频通用面：只放模型经常直接调用的少数工具。被当前模式拒绝的写类工具
            // 已在上面过滤；权限门仍由 ToolCycle 执行期复核，不能被 schema 过滤取代。
            for name in [
                "read_file",
                "list_dir",
                "str_replace_editor",
                "write_file",
                "web_search",
                "shell_exec",
                "code_exec",
            ] {
                if let Some(tool) = available.iter().find(|tool| tool.name == name) {
                    if selected_names.insert(tool.name.clone()) {
                        selected.push(tool);
                    }
                }
            }

            // 依据本轮请求的显式词项预热最相关的专用工具；未命中项仍可通过发现工具加载。
            let mut scored: Vec<(u8, &ToolDef)> = available
                .iter()
                .filter(|tool| !selected_names.contains(&tool.name))
                .filter_map(|tool| {
                    let score = relevance_score(request, &tool.name, &tool.description);
                    (score > 0).then_some((score, tool))
                })
                .collect();
            scored.sort_by(|(score_a, tool_a), (score_b, tool_b)| {
                score_b
                    .cmp(score_a)
                    .then_with(|| tool_a.name.cmp(&tool_b.name))
            });
            for (_, tool) in scored {
                if selected.len() >= MAX_INITIAL_TOOLS {
                    break;
                }
                if selected_names.insert(tool.name.clone()) {
                    selected.push(tool);
                }
            }
            // 硬性预算守卫：极大 schema 不得把首轮重新撑回全目录体积。
            let budget = TOOL_SCHEMA_CONTEXT_WINDOW / TOOL_SCHEMA_BUDGET_FRACTION;
            let discovery_reserve = estimate_tool_schema_tokens(&[discover_tool_def()]);
            let selection_budget = budget.saturating_sub(discovery_reserve);
            while estimate_tool_schema_tokens(
                &selected
                    .iter()
                    .map(|tool| (*tool).clone())
                    .collect::<Vec<_>>(),
            ) > selection_budget
            {
                if let Some(removed) = selected.pop() {
                    selected_names.remove(&removed.name);
                } else {
                    break;
                }
            }
        } else {
            for tool in &available {
                selected_names.insert(tool.name.clone());
                selected.push(tool);
            }
        }

        let mut active_names: HashSet<String> =
            selected.iter().map(|tool| tool.name.clone()).collect();
        // 发现工具名是内部保留名，避免覆盖用户注册的同名 MCP 工具。
        if enabled
            && !available
                .iter()
                .any(|tool| tool.name == DISCOVER_TOOLS_NAME)
        {
            active_names.insert(DISCOVER_TOOLS_NAME.to_string());
        }

        let initial = make_active_tools(&available, &active_names);
        let full_estimated_tokens = estimate_tool_schema_tokens(&available);
        let initial_estimated_tokens = estimate_tool_schema_tokens(&initial);
        let metrics = ToolSchemaMetrics {
            available_tools: available.len(),
            initially_exposed_tools: selected.len(),
            full_estimated_tokens,
            initial_estimated_tokens,
            budget_tokens: TOOL_SCHEMA_CONTEXT_WINDOW / TOOL_SCHEMA_BUDGET_FRACTION,
        };

        tracing::info!(
            available_tools = metrics.available_tools,
            initially_exposed_tools = metrics.initially_exposed_tools,
            full_schema_estimated_tokens = metrics.full_estimated_tokens,
            initial_schema_estimated_tokens = metrics.initial_estimated_tokens,
            schema_budget_tokens = metrics.budget_tokens,
            "按需暴露工具 schema"
        );

        Self {
            available,
            active_names,
            mode,
            enabled,
            metrics,
        }
    }

    /// 当前模型请求可见的完整工具定义（含轻量发现工具）。
    pub fn active_tools(&self) -> Vec<ToolDef> {
        make_active_tools(&self.available, &self.active_names)
    }

    pub fn metrics(&self) -> &ToolSchemaMetrics {
        &self.metrics
    }

    /// 根据自然语言或工具名加载最多 8 个完整 schema；返回模型可读的发现结果。
    pub fn discover(&mut self, query: &str) -> String {
        if !self.enabled {
            return "当前工具目录较小，已直接暴露全部可用工具；无需再发现。".to_string();
        }
        let query = query.trim();
        if query.is_empty() {
            return "请在 query 中描述需要的能力或工具名；例如：浏览网页、操作 Excel、查看 Git 差异。"
                .to_string();
        }

        let mut ranked: Vec<(u8, &ToolDef)> = self
            .available
            .iter()
            .filter(|tool| !self.active_names.contains(&tool.name))
            .filter_map(|tool| {
                let score = relevance_score(query, &tool.name, &tool.description);
                (score > 0).then_some((score, tool))
            })
            .collect();
        ranked.sort_by(|(score_a, tool_a), (score_b, tool_b)| {
            score_b
                .cmp(score_a)
                .then_with(|| tool_a.name.cmp(&tool_b.name))
        });

        if ranked.is_empty() {
            let mut suggestions: Vec<&str> = self
                .available
                .iter()
                .map(|tool| tool.name.as_str())
                .collect();
            suggestions.sort_unstable();
            suggestions.dedup();
            let names = suggestions
                .into_iter()
                .take(24)
                .collect::<Vec<_>>()
                .join(", ");
            return format!(
                "未找到与「{query}」匹配的未加载工具。可尝试更具体的能力描述或工具名。可用目录（节选）：{names}。"
            );
        }

        let matches: Vec<ToolDef> = ranked
            .into_iter()
            .take(MAX_DISCOVERED_TOOLS)
            .map(|(_, tool)| tool.clone())
            .collect();
        for tool in &matches {
            if !crate::mode::mode_denies_tool(self.mode, &tool.name) {
                self.active_names.insert(tool.name.clone());
            }
        }
        let listing = matches
            .iter()
            .map(|tool| format!("- {}：{}", tool.name, short_description(&tool.description)))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "已按需加载以下工具的完整参数 schema，可在下一轮直接调用：\n{listing}\n若仍缺少能力，可再次调用发现工具并描述缺少的能力。"
        )
    }

    /// 本模式下若调用方绕过 schema 过滤，仍把内部发现工具限制在可用目录内。
    pub fn permits(&self, tool_name: &str) -> bool {
        if self.is_discovery_tool(tool_name) {
            return true;
        }
        self.active_names.contains(tool_name)
            && self.available.iter().any(|tool| {
                tool.name == tool_name && !crate::mode::mode_denies_tool(self.mode, tool_name)
            })
    }

    /// 判断调用是否命中本轮内部发现器（不是恰巧同名的 MCP/本地工具）。
    pub fn is_discovery_tool(&self, tool_name: &str) -> bool {
        self.enabled
            && tool_name == DISCOVER_TOOLS_NAME
            && !self
                .available
                .iter()
                .any(|tool| tool.name == DISCOVER_TOOLS_NAME)
    }

    /// 是否启用了延迟目录与 discovery 工具。
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// 构造发现工具的 schema；仅作 function calling 描述，不执行外部动作。
pub fn discover_tool_def() -> ToolDef {
    ToolDef {
        name: DISCOVER_TOOLS_NAME.to_string(),
        description: "按需查找当前未加载的工具。输入自然语言能力或工具名；系统会将匹配工具的完整参数 schema 加入下一轮，随后即可直接调用。它只发现工具，不执行操作。".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "需要的能力或工具名，例如：操作 Excel、浏览器截图、Git 差异、知识库检索"
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
    }
}

/// 估算 JSON function schemas 的 token 占用：保守按 ASCII 每 3 字符/token、非 ASCII 每字符/token。
///
/// 这是跨模型的工程估算，不代替 provider 返回的真实 prompt_tokens；结果用于限制首轮面，
/// 实际账单/上下文用量仍以 provider 回传 usage 为准。
pub fn estimate_tool_schema_tokens(tools: &[ToolDef]) -> usize {
    let serialized = serde_json::to_string(tools).unwrap_or_default();
    let mut tokens = 0usize;
    let mut ascii_run = 0usize;
    for ch in serialized.chars() {
        if ch.is_ascii() {
            ascii_run += 1;
        } else {
            tokens += ascii_run.div_ceil(3);
            ascii_run = 0;
            if !ch.is_whitespace() {
                tokens += 1;
            }
        }
    }
    tokens + ascii_run.div_ceil(3) + tools.len() * 6
}

fn make_active_tools(available: &[ToolDef], active_names: &HashSet<String>) -> Vec<ToolDef> {
    let mut tools: Vec<ToolDef> = available
        .iter()
        .filter(|tool| active_names.contains(&tool.name))
        .cloned()
        .collect();
    if active_names.contains(DISCOVER_TOOLS_NAME) {
        tools.push(discover_tool_def());
    }
    tools
}

fn short_description(description: &str) -> String {
    let mut chars = description.chars();
    let short: String = chars.by_ref().take(220).collect();
    if chars.next().is_some() {
        format!("{short}…")
    } else {
        short
    }
}

fn relevance_score(query: &str, name: &str, description: &str) -> u8 {
    let query_lower = query.to_lowercase();
    let name_lower = name.to_lowercase();
    let desc_lower = description.to_lowercase();
    let mut score = 0u8;

    // 英文词项和工具名权重高，避免只凭泛化语义把很多工具塞进首轮。
    for term in query_lower.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        if term.len() < 3 {
            continue;
        }
        if name_lower.split('_').any(|part| part == term) {
            score = score.saturating_add(6);
        } else if desc_lower.contains(term) {
            score = score.saturating_add(3);
        }
    }

    // 中文能力词表兼顾首轮预热与用户自然语言发现。泛词（如「工具」「文件」）不纳入，
    // 以免普通输入把全工具面重新带回首轮。
    const CUES: &[&str] = &[
        "网页",
        "浏览器",
        "截图",
        "屏幕",
        "窗口",
        "剪贴板",
        "进程",
        "桌面",
        "表格",
        "工作簿",
        "电子表格",
        "excel",
        "xlsx",
        "word",
        "docx",
        "幻灯片",
        "演示文稿",
        "ppt",
        "pdf",
        "git",
        "提交",
        "版本差异",
        "版本控制",
        "知识库",
        "记忆",
        "研究",
        "调研",
        "视觉",
        "图像",
        "自动化",
        "json",
        "csv",
        "差异",
        "补丁",
        "删除文件",
        "复制文件",
        "移动文件",
        "终端",
        "命令",
        "shell",
        "http",
        "url",
        "搜索",
        "测试",
        "编译",
        "构建",
        "重构",
        "代码",
        "文件编辑",
    ];
    for cue in CUES {
        if query_lower.contains(cue) {
            if name_lower.contains(cue) {
                score = score.saturating_add(8);
            } else if desc_lower.contains(cue) {
                score = score.saturating_add(4);
            }
        }
    }

    // 明确的能力同义词映射，解决用户自然语言与工具命名不一致的问题。
    for (terms, needles) in [
        (
            &["写文件", "保存文件", "生成文件", "创建文件"][..],
            &["write_file", "str_replace_editor"][..],
        ),
        (
            &["读文件", "读取文件", "查看文件"][..],
            &["read_file", "file_info"][..],
        ),
        (
            &["打开网页", "网页操作", "网页点击"][..],
            &["browser", "web_fetch"][..],
        ),
        (
            &["电脑操作", "桌面操作", "鼠标", "键盘"][..],
            &["computer", "system_control", "window", "clipboard"][..],
        ),
        (
            &["Git 差异", "git diff", "代码仓库"][..],
            &["git", "code_intel"][..],
        ),
        (
            &["知识检索", "经验记忆", "长期记忆"][..],
            &["memory", "research"][..],
        ),
    ] {
        if terms.iter().any(|term| query_lower.contains(term))
            && needles
                .iter()
                .any(|needle| name_lower.contains(needle) || desc_lower.contains(needle))
        {
            score = score.saturating_add(5);
        }
    }
    score
}

/// 查找发现工具调用的 query 参数；兼容模型把 query 编码为 JSON 字符串的形式。
pub fn discovery_query(arguments: &Value) -> Option<&str> {
    arguments.get("query").and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, description: &str) -> ToolDef {
        ToolDef {
            name: name.to_string(),
            description: description.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            }),
        }
    }

    #[test]
    fn initial_surface_is_small_but_discovery_preserves_specialized_access() {
        let mut tools = vec![
            tool("read_file", "读取工作区文件"),
            tool("list_dir", "列出目录内容"),
            tool("write_file", "写入文件"),
            tool("office_automation", "生成 Excel 工作簿和 Word 文档"),
            tool("browser", "浏览器网页交互与截图"),
            tool("git_diff", "读取 Git 工作区差异"),
        ];
        tools.extend((0..10).map(|i| tool(&format!("special_{i}"), "专用能力")));
        let mut disclosure = ToolDisclosure::new(&tools, "解释一下这个功能", ModeKind::Agent);
        let first = disclosure.active_tools();
        assert!(first
            .iter()
            .any(|tool| tool.name == "_marguerite_discover_tools"));
        assert!(first.iter().any(|tool| tool.name == "read_file"));
        assert!(!first.iter().any(|tool| tool.name == "office_automation"));
        assert!(
            disclosure.metrics().initially_exposed_tools < disclosure.metrics().available_tools
        );

        let output = disclosure.discover("操作 Excel 表格");
        assert!(output.contains("office_automation"), "{output}");
        assert!(disclosure
            .active_tools()
            .iter()
            .any(|tool| tool.name == "office_automation"));
    }

    #[test]
    fn ask_mode_does_not_advertise_or_discover_denied_write_tools() {
        let tools = vec![
            tool("read_file", "读取文件"),
            tool("write_file", "写文件"),
            tool("office_automation", "生成并编辑 Excel 工作簿"),
        ];
        let mut disclosure = ToolDisclosure::new(&tools, "生成 Excel 表格", ModeKind::Plan);
        let first = disclosure.active_tools();
        assert!(!first.iter().any(|tool| tool.name == "write_file"));
        assert!(!first.iter().any(|tool| tool.name == "office_automation"));
        let result = disclosure.discover("Excel 工作簿");
        assert!(!result.contains("已按需加载以下工具"));
        assert!(!disclosure
            .active_tools()
            .iter()
            .any(|tool| tool.name == "office_automation"));
    }

    #[test]
    fn initial_schema_budget_is_one_quarter_of_tool_context() {
        let tools: Vec<ToolDef> = (0..40)
            .map(|i| {
                tool(
                    &format!("special_tool_{i}"),
                    "专用能力的详细说明。用于处理用户明确提出的领域任务，并保留完整操作边界。",
                )
            })
            .collect();
        let disclosure = ToolDisclosure::new(&tools, "解释概念", ModeKind::Agent);
        assert!(
            disclosure.metrics().initial_estimated_tokens <= disclosure.metrics().budget_tokens,
            "首轮估算 {} 超过预算 {}",
            disclosure.metrics().initial_estimated_tokens,
            disclosure.metrics().budget_tokens
        );
        assert_eq!(disclosure.metrics().available_tools, 40);
        assert!(disclosure
            .active_tools()
            .iter()
            .any(|tool| tool.name == DISCOVER_TOOLS_NAME));
    }

    #[test]
    fn oversized_schema_is_removed_to_respect_the_hard_initial_budget() {
        let mut tools: Vec<ToolDef> = (0..14)
            .map(|i| tool(&format!("special_tool_{i}"), "专用能力"))
            .collect();
        tools.push(tool("read_file", &"x".repeat(140_000)));
        let disclosure = ToolDisclosure::new(&tools, "读取文件", ModeKind::Agent);
        assert!(
            disclosure.metrics().initial_estimated_tokens <= disclosure.metrics().budget_tokens,
            "大 schema 必须被硬预算收敛"
        );
        assert!(disclosure
            .active_tools()
            .iter()
            .any(|tool| tool.name == DISCOVER_TOOLS_NAME));
    }

    #[test]
    fn reserved_name_collision_keeps_the_registered_tool_callable() {
        let mut tools = vec![tool(DISCOVER_TOOLS_NAME, "MCP 用户工具，负责调用另一系统")];
        tools.extend((0..12).map(|i| tool(&format!("special_{i}"), "专用能力")));
        let disclosure = ToolDisclosure::new(&tools, "查看信息", ModeKind::Agent);
        assert!(!disclosure.is_enabled(), "同名冲突时关闭内部发现器");
        assert!(!disclosure.is_discovery_tool(DISCOVER_TOOLS_NAME));
        assert!(disclosure.permits(DISCOVER_TOOLS_NAME));
        assert!(disclosure
            .active_tools()
            .iter()
            .any(|tool| tool.name == DISCOVER_TOOLS_NAME
                && tool.description.contains("MCP 用户工具")));
    }

    #[test]
    fn discovery_query_requires_a_string_field() {
        assert_eq!(
            discovery_query(&json!({"query": "browser"})),
            Some("browser")
        );
        assert_eq!(discovery_query(&json!({"query": 1})), None);
    }
}
