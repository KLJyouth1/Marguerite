//! 按需暴露工具 schema：首轮只发送常用工具，按需发现并加载其余完整 schema。
//!
//! 所有工具仍保留在本轮的本地目录里。`marguerite_discover_tools` 只负责发现能力，
//! 找到后会把工具的原始参数 schema 加入后续模型请求；它不执行工具，也不改变权限。
//!
//! ## W4.2：会话级稳定工具面
//!
//! 同一会话内模型可见的工具数组**只增不减**，且按工具名排序返回确定性数组。
//! 工具面由 [`SessionToolFace`] 持有（`AgentRuntime` 按会话键保存），
//! 不再随每轮用户文本重选——工具数组逐轮变化会让服务端 KV 前缀从 `tools` 处失配。
//! [`ToolDisclosure`] 只是这一轮的操作句柄：`discover()` / `permits()` 读写同一个面。
//!
//! ## W5.3：工具面变化的归因
//!
//! 面一旦真的变化（`discover()` 追加、`reconcile()` 见到 `tools.patch` / MCP 热插拔 /
//! 技能目录带来的目录增减、发现器开关翻转），就写一条 `face_changed` 日志
//! （field `scope = "agent.tool_face"` + `source` + `added`/`removed`）。
//! 它与 `cache.status` 的 `stable_prefix_drift_by_scope["agent.stable_half"]` 对齐：
//! 漂移是结果，`face_changed` 是原因，排障时不必再靠猜。

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

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
///
/// W4.3：从 12 提到 18，并**只**放固定高频面（不再按本轮请求词预热）——
/// 工具面逐轮变化会破坏 KV 前缀，也让模型无法形成稳定的「我有哪些工具」心智模型。
const MAX_INITIAL_TOOLS: usize = 19;

/// 固定高频面（W4.3）：按「用户最常要求、最能减少脚本」排序。
///
/// 全部是**已注册且模式允许**的真实工具名；不在当前模式内的会在 `new` 里被过滤。
/// `desktop_launch`/`desktop_attach` 与 `desktop_action` 成对出现：动作需要
/// launch/attach 返回的 handle（W6.2 起也可以省略 handle、按 window 附着）。
pub const FIXED_FACE: &[&str] = &[
    "read_file",
    "list_dir",
    "str_replace_editor",
    "write_file",
    "apply_patch",
    "grep",
    "web_search",
    "web_fetch",
    "browser",
    "memory_search",
    "kb_search",
    "list_skills",
    "use_skill",
    "desktop_launch",
    "desktop_attach",
    "desktop_action",
    "computer_use",
    "shell_exec",
    "code_exec",
];

// 常量级防漂移：固定面长度必须与首轮上限一致（否则多出的工具永远不可见）。
const _: () = assert!(FIXED_FACE.len() <= MAX_INITIAL_TOOLS);
/// 单次发现最多加入的 schema 数，防止一次模糊查询重新塞回整个目录。
const MAX_DISCOVERED_TOOLS: usize = 8;

/// 会话级稳定工具面（W4.2）。
///
/// 契约（违反即缺陷）：
/// 1. **只增不减**：同一会话内 [`SessionToolFace::active_tools`] 返回的工具名集合
///    只会在发现/目录扩展时增长；既有工具的相对顺序由「按名字排序」天然保持。
/// 2. **确定性**：数组按工具名排序，同一面无论查询多少次都逐字节相同。
/// 3. **会话边界**：面只在首次 turn、会话切换、`AgentRuntime::reset()` /
///    模式切换时重建；重建后从头开始（不携带上一会话的发现结果）。
///
/// 首轮面 = 固定高频面（[`FIXED_FACE`]，已按模式过滤）∩ 当前可用工具；
/// 宽目录（> [`MAX_INITIAL_TOOLS`]）才启用按需发现，与既有语义一致。
#[derive(Debug, Clone)]
pub struct SessionToolFace {
    /// 全会话可用工具（已按模式过滤、按名字排序）。
    available: Vec<ToolDef>,
    /// 已暴露的工具名。
    visible: HashSet<String>,
    /// 是否启用按需发现（宽目录 + 无保留名冲突）。
    discovery_enabled: bool,
}

impl SessionToolFace {
    /// 为新会话创建工具面：固定高频面 + 模式允许的工具（不再按请求文本裁剪）。
    pub fn bootstrap(available: Vec<ToolDef>, mode: ModeKind) -> Self {
        let mut face = Self {
            available: normalize_available(available, mode),
            visible: HashSet::new(),
            discovery_enabled: false,
        };
        face.discovery_enabled = face.catalog_needs_discovery();
        if face.discovery_enabled {
            for name in FIXED_FACE.iter().copied() {
                if face.available.iter().any(|tool| tool.name == name) {
                    face.visible.insert(name.to_string());
                }
            }
            // 硬性预算守卫：单个超大 schema（如 140KB 的工具）不得把首轮撑爆；
            // 超预算时按「schema 最大优先」确定性地剔除，直到落回预算内。
            face.fit_budget();
        } else {
            // 窄目录直接全量暴露（与旧行为一致：小工具面不值得引入发现器）。
            face.visible = face
                .available
                .iter()
                .map(|tool| tool.name.clone())
                .collect();
        }
        face
    }

    /// 当前模型可见的工具定义（按工具名排序，确定性；含轻量发现工具）。
    pub fn active_tools(&self) -> Vec<ToolDef> {
        let mut tools: Vec<ToolDef> = self
            .available
            .iter()
            .filter(|tool| self.visible.contains(&tool.name))
            .cloned()
            .collect();
        if self.discovery_enabled {
            tools.push(discover_tool_def());
        }
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        tools
    }

    /// 追加暴露工具：只增不减；不存在于当前目录的名字被忽略（不报错）。
    pub fn extend(&mut self, names: &[String]) {
        let mut added: Vec<String> = Vec::new();
        for name in names {
            // `insert` 返回 true 才是「本次真的新增」——重复发现不重复记账。
            if self.available.iter().any(|tool| &tool.name == name)
                && self.visible.insert(name.clone())
            {
                added.push(name.clone());
            }
        }
        if !added.is_empty() {
            added.sort();
            // W5.3：工具面变化是稳定前缀漂移的**原因**，必须留下可归因的现场。
            tracing::info!(
                event = "face_changed",
                scope = "agent.tool_face",
                source = "discovery",
                added = ?added,
                visible = self.visible.len(),
                "会话工具面经发现扩展（只增不减）"
            );
        }
    }

    /// 已暴露的工具数（不含发现工具）。
    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    /// 当前工具面是否包含某个真实工具（不含发现工具）。
    pub fn exposes(&self, tool_name: &str) -> bool {
        self.visible.contains(tool_name)
            && self.available.iter().any(|tool| tool.name == tool_name)
    }

    /// 调用是否命中内部发现器（不是恰巧同名的 MCP/本地工具）。
    pub fn is_discovery_tool(&self, tool_name: &str) -> bool {
        self.discovery_enabled
            && tool_name == DISCOVER_TOOLS_NAME
            && !self
                .available
                .iter()
                .any(|tool| tool.name == DISCOVER_TOOLS_NAME)
    }

    /// 是否启用了延迟目录与 discovery 工具。
    pub fn discovery_enabled(&self) -> bool {
        self.discovery_enabled
    }

    /// 当前可用目录（只读）。
    pub fn available(&self) -> &[ToolDef] {
        &self.available
    }

    /// 用最新工具目录/模式对齐可用面（同一会话内调用）。
    ///
    /// 只做两件事：① 目录里消失的工具从可见面移除（否则 `permits` 会放行不存在的工具）；
    /// ② 目录里新出现的**固定面**工具追加进可见面。**绝不因目录变化撤回已暴露的工具**
    /// ——撤回会让工具数组缩短，破坏会话内单调契约。预算守卫只在 `bootstrap` 做一次。
    pub(crate) fn reconcile(&mut self, available: Vec<ToolDef>, mode: ModeKind) {
        let previously_discovery = self.discovery_enabled;
        let before = self.visible.clone();
        self.available = normalize_available(available, mode);
        self.discovery_enabled = self.catalog_needs_discovery();
        self.visible.retain(|name| {
            self.available.iter().any(|tool| &tool.name == name)
        });
        if !self.discovery_enabled {
            // 目录缩到阈值以下：全量暴露（仍是「只增不减」方向）。
            self.visible = self
                .available
                .iter()
                .map(|tool| tool.name.clone())
                .collect();
        } else if previously_discovery {
            for name in FIXED_FACE.iter().copied() {
                if self.available.iter().any(|tool| tool.name == name) {
                    self.visible.insert(name.to_string());
                }
            }
        }
        // previously_discovery == false 且现在需要发现器：保持现状（全部可见），不回撤。

        // W5.3：`tools.patch` / MCP 热插拔 / 技能目录变化最终都表现为「目录变了」。
        // 只在可见面真的变化时留一条 `face_changed`（稳态轮次零噪声），
        // 这样 `cache.status` 里 `agent.stable_half` 的漂移就有明确归因。
        let mut added: Vec<String> = self.visible.difference(&before).cloned().collect();
        let mut removed: Vec<String> = before.difference(&self.visible).cloned().collect();
        // 发现器开关翻转同样改变工具数组（轻量发现工具出现/消失），必须计入。
        let discovery_flipped = previously_discovery != self.discovery_enabled;
        if !added.is_empty() || !removed.is_empty() || discovery_flipped {
            added.sort();
            removed.sort();
            tracing::info!(
                event = "face_changed",
                scope = "agent.tool_face",
                source = "registry_reconcile",
                added = ?added,
                removed = ?removed,
                discovery_enabled = self.discovery_enabled,
                visible = self.visible.len(),
                "会话工具面随目录变化（tools.patch / MCP 热插拔 / 技能目录）"
            );
        }
    }

    fn catalog_needs_discovery(&self) -> bool {
        self.available.len() > MAX_INITIAL_TOOLS
            && !self
                .available
                .iter()
                .any(|tool| tool.name == DISCOVER_TOOLS_NAME)
    }

    /// 首轮 schema 不得超出工具回路上下文预算（含发现工具）。
    fn fit_budget(&mut self) {
        let budget = TOOL_SCHEMA_CONTEXT_WINDOW / TOOL_SCHEMA_BUDGET_FRACTION;
        loop {
            if estimate_tool_schema_tokens(&self.active_tools()) <= budget {
                return;
            }
            // 确定性剔除：schema 最大的工具优先（同尺寸按名字倒序），保证同目录同结果。
            let mut ranked: Vec<(usize, String)> = self
                .visible
                .iter()
                .filter_map(|name| {
                    self.available
                        .iter()
                        .find(|tool| &tool.name == name)
                        .map(|tool| {
                            (
                                estimate_tool_schema_tokens(std::slice::from_ref(tool)),
                                name.clone(),
                            )
                        })
                })
                .collect();
            ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
            match ranked.first() {
                Some((_, name)) => {
                    self.visible.remove(name);
                }
                None => return,
            }
        }
    }
}

/// 过滤掉当前模式禁止的工具，并按工具名排序（确定性数组的前提）。
fn normalize_available(available: Vec<ToolDef>, mode: ModeKind) -> Vec<ToolDef> {
    let mut tools: Vec<ToolDef> = available
        .into_iter()
        .filter(|tool| !crate::mode::mode_denies_tool(mode, &tool.name))
        .collect();
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    tools
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSchemaMetrics {
    pub available_tools: usize,
    pub initially_exposed_tools: usize,
    pub full_estimated_tokens: usize,
    pub initial_estimated_tokens: usize,
    pub budget_tokens: usize,
    /// 本轮请求被判定为「直答」（W4.2 起**仅观测**：不再改变工具面）。
    pub direct_answer: bool,
}

/// 直答判定的长度上限（字符数）。超过即视为可能含真实任务，维持完整工具面。
const DIRECT_ANSWER_MAX_CHARS: usize = 40;

/// 寒暄 / 主观评价 / 身份类措辞：这类请求没有可执行的外部动作。
const CHAT_PATTERNS: &[&str] = &[
    "你好", "您好", "hi", "hello", "在吗", "谢谢", "感谢", "辛苦了",
    "你是谁", "你叫什么", "你多大", "你能做什么", "你会什么",
    "你觉得", "你认为", "你看", "好看", "漂亮", "可爱", "喜不喜欢",
    "开心", "难过", "累了吗", "晚安", "早安", "早上好", "下午好", "晚上好",
    "怎么样", "好不好", "行不行", "可以吗", "是不是",
];

/// 动作 / 产物 / 实时信息词：命中任意一个都**不允许**走直答（宁多给工具）。
const ACTION_HINTS: &[&str] = &[
    "查", "搜", "检索", "查找", "找一下", "导航", "打开", "运行", "执行", "创建", "新建",
    "生成", "写", "改", "修", "重构", "构建", "编译", "部署", "安装", "下载", "上传",
    "读取", "文件", "目录", "代码", "仓库", "项目", "报错", "错误", "异常", "测试", "日志",
    "网页", "网站", "浏览器", "截图", "表格", "文档", "ppt", "幻灯片", "excel", "word",
    "pdf", "天气", "气温", "下雨", "降雨", "预报", "新闻", "最新", "实时", "汇率", "股价",
    "价格", "邮件", "日程", "提醒", "翻译", "画", "设计", "做一个", "帮我做", "建模",
    "视频", "音频", "mcp", "技能", "工具",
];

/// 判定这轮请求是否可以**按直答处理**。
///
/// 采用白名单式保守判定，必须同时满足三条：
/// 1. 去空白后字符数 ≤ [`DIRECT_ANSWER_MAX_CHARS`]；
/// 2. 命中至少一个 [`CHAT_PATTERNS`]（寒暄 / 主观评价 / 身份询问）；
/// 3. 不命中任何 [`ACTION_HINTS`]（动作、产物、实时信息）。
///
/// 任一条件不满足就返回 false —— 也就是说「拿不准就给工具」。
/// ⚠️ W4.2 起该判定**不再改变工具数组**（同一会话的工具面必须逐字节稳定）：
/// 它只作为观测指标与动态尾部提示的输入；模型仍能看到完整会话工具面。
pub fn is_direct_answer_request(request: &str) -> bool {
    let text = request.trim();
    if text.is_empty() || text.chars().count() > DIRECT_ANSWER_MAX_CHARS {
        return false;
    }
    let lowered = text.to_lowercase();
    let has_chat_pattern = CHAT_PATTERNS.iter().any(|p| lowered.contains(p));
    if !has_chat_pattern {
        return false;
    }
    !ACTION_HINTS.iter().any(|hint| lowered.contains(hint))
}

/// 一轮工具回路内的工具 schema 暴露句柄。
///
/// W4.2：真正的可见面由 [`SessionToolFace`] 持有——**会话级、跨轮复用、只增不减**。
/// 本结构只是这一轮的操作句柄：`discover()` 扩展的是会话面，因此下一次 turn
/// 仍能看到已发现的工具，工具数组不会缩回去；`active_tools()` 直接委托给会话面。
#[derive(Debug, Clone)]
pub struct ToolDisclosure {
    mode: ModeKind,
    enabled: bool,
    metrics: ToolSchemaMetrics,
    face: Arc<Mutex<SessionToolFace>>,
}

impl ToolDisclosure {
    /// 为当前请求创建按需暴露策略（独立会话面：仅用于一次性调用方/单测）。
    ///
    /// 即使一轮开始时只发少量 schema，未暴露工具也不会被永久隐藏：模型可调用
    /// `marguerite_discover_tools`，匹配到的完整原始 schema 会加入下一轮请求。
    pub fn new(all_tools: &[ToolDef], request: &str, mode: ModeKind) -> Self {
        let face = Arc::new(Mutex::new(SessionToolFace::bootstrap(
            all_tools.to_vec(),
            mode,
        )));
        Self::assemble(face, request, mode)
    }

    /// 复用会话级工具面（W4.2）：`AgentRuntime` 按会话键持有，首次 turn 创建。
    ///
    /// 每轮只做「目录对齐」（新增的固定面工具追加、消失的工具移除），
    /// 不会因为本轮用户文本改变已暴露集合。
    pub fn with_face(
        all_tools: &[ToolDef],
        request: &str,
        mode: ModeKind,
        face: Arc<Mutex<SessionToolFace>>,
    ) -> Self {
        {
            let mut guard = face.lock().unwrap_or_else(|e| e.into_inner());
            guard.reconcile(all_tools.to_vec(), mode);
        }
        Self::assemble(face, request, mode)
    }

    /// 本轮的会话面（供运行时/测试观察；扩展结果跨轮保留）。
    pub fn face(&self) -> Arc<Mutex<SessionToolFace>> {
        Arc::clone(&self.face)
    }

    fn assemble(face: Arc<Mutex<SessionToolFace>>, request: &str, mode: ModeKind) -> Self {
        let (active, available, exposed, enabled) = {
            let guard = face.lock().unwrap_or_else(|e| e.into_inner());
            (
                guard.active_tools(),
                guard.available().to_vec(),
                guard.visible_len(),
                guard.discovery_enabled(),
            )
        };
        // W4.2：直答判定只保留为观测（日志/指标）。工具数组必须整个会话稳定，
        // 不能因为一句寒暄就清空——那会让服务端 KV 前缀在 `tools` 处整段失配。
        let direct_answer = is_direct_answer_request(request) && !active.is_empty();
        let metrics = ToolSchemaMetrics {
            available_tools: available.len(),
            initially_exposed_tools: exposed,
            full_estimated_tokens: estimate_tool_schema_tokens(&available),
            initial_estimated_tokens: estimate_tool_schema_tokens(&active),
            budget_tokens: TOOL_SCHEMA_CONTEXT_WINDOW / TOOL_SCHEMA_BUDGET_FRACTION,
            direct_answer,
        };

        tracing::info!(
            available_tools = metrics.available_tools,
            initially_exposed_tools = metrics.initially_exposed_tools,
            full_schema_estimated_tokens = metrics.full_estimated_tokens,
            initially_schema_estimated_tokens = metrics.initial_estimated_tokens,
            schema_budget_tokens = metrics.budget_tokens,
            direct_answer,
            "按需暴露工具 schema（会话级稳定面）"
        );

        Self {
            mode,
            enabled,
            metrics,
            face,
        }
    }

    /// 当前模型请求可见的完整工具定义（含轻量发现工具；按名字排序）。
    pub fn active_tools(&self) -> Vec<ToolDef> {
        self.face
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active_tools()
    }

    pub fn metrics(&self) -> &ToolSchemaMetrics {
        &self.metrics
    }

    /// 根据自然语言或工具名加载最多 8 个完整 schema；返回模型可读的发现结果。
    ///
    /// 扩展落在**会话面**上：本轮结束后该面仍包含新工具，下一轮继续可见。
    pub fn discover(&mut self, query: &str) -> String {
        let mut face = self.face.lock().unwrap_or_else(|e| e.into_inner());
        if !face.discovery_enabled() {
            return "当前工具目录较小，已直接暴露全部可用工具；无需再发现。".to_string();
        }
        let query = query.trim();
        if query.is_empty() {
            return "请在 query 中描述需要的能力或工具名；例如：浏览网页、操作 Excel、查看 Git 差异。"
                .to_string();
        }

        let mut ranked: Vec<(u8, &ToolDef)> = face
            .available()
            .iter()
            .filter(|tool| !face.exposes(&tool.name))
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
            let mut suggestions: Vec<&str> = face
                .available()
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
        let names: Vec<String> = matches.iter().map(|tool| tool.name.clone()).collect();
        // W4.2：只追加、绝不删除；本会话内已暴露的工具永不撤回。
        face.extend(&names);
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
        let face = self.face.lock().unwrap_or_else(|e| e.into_inner());
        if face.is_discovery_tool(tool_name) {
            return true;
        }
        face.exposes(tool_name) && !crate::mode::mode_denies_tool(self.mode, tool_name)
    }

    /// 判断调用是否命中本轮内部发现器（不是恰巧同名的 MCP/本地工具）。
    pub fn is_discovery_tool(&self, tool_name: &str) -> bool {
        self.face
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_discovery_tool(tool_name)
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
        // 多模态与 RPA（2026-10-23）：用户的自然语言几乎不会直接说工具名。
        (
            &["生图", "画图", "生成图片", "配图", "出图", "设计一张图"][..],
            &["image_generate", "图片", "图像生成"][..],
        ),
        (
            &["生视频", "生成视频", "做视频", "文生视频", "图生视频"][..],
            &["video_generate", "视频生成"][..],
        ),
        // 本地生视频（2026-10-24）：三种渲染路径各自有明确的口语说法，
        // 避免「动画」这种泛词把用户导到需要云端模型的 video_generate。
        (
            &["代码动画", "代码视频", "打字动画", "敲代码", "程序员浪漫", "代码生视频"][..],
            &["code_video", "代码打字"][..],
        ),
        (
            &["网页视频", "html 视频", "css 动画", "动效视频", "数据动效", "网页动效"][..],
            &["web_video", "网页逐帧"][..],
        ),
        (
            &["manim", "数学动画", "算法动画", "公式动画", "推导动画"][..],
            &["manim_video", "manim"][..],
        ),
        (
            &["转写", "录音", "语音转文字", "听写", "音频"][..],
            &["audio_transcribe", "转写"][..],
        ),
        (
            &["朗读", "配音", "语音合成", "读出来", "念一遍"][..],
            &["audio_speak", "语音合成"][..],
        ),
        (
            &["录制", "录一遍", "回放", "照做", "重放", "宏"][..],
            &["auto_record", "auto_replay", "录制", "回放"][..],
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
        tools.extend((0..24).map(|i| tool(&format!("special_{i}"), "专用能力")));
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
    fn fixed_face_is_exposed_verbatim_and_covers_six_capability_families() {
        let tools: Vec<ToolDef> = FIXED_FACE
            .iter()
            .map(|name| tool(name, "x"))
            .chain((0..10).map(|i| tool(&format!("special_{i}"), "专用能力")))
            .collect();
        let disclosure = ToolDisclosure::new(&tools, "随便什么请求", ModeKind::Agent);
        let names: Vec<String> = disclosure
            .active_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        for required in FIXED_FACE {
            assert!(names.iter().any(|name| name == required), "固定面缺少 {required}");
        }
        // 六个能力族（文件/浏览器/记忆/知识/技能/桌面/脚本）必须首轮可见。
        for family in [
            "read_file",
            "browser",
            "memory_search",
            "kb_search",
            "use_skill",
            "desktop_action",
            "computer_use",
            "shell_exec",
        ] {
            assert!(names.iter().any(|name| name == family), "能力族缺失：{family}");
        }
        // 故意不按请求词裁剪：同一工具面在不同请求下逐字节一致。
        let other = ToolDisclosure::new(&tools, "帮我点一下记事本", ModeKind::Agent);
        let other_names: Vec<String> = other
            .active_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert_eq!(names, other_names, "首轮工具面不得随用户文本变化");
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
        let mut tools: Vec<ToolDef> = (0..20)
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
        tools.extend((0..20).map(|i| tool(&format!("special_{i}"), "专用能力")));
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

    fn wide_tools() -> Vec<ToolDef> {
        let mut tools = vec![
            tool("read_file", "读取工作区文件"),
            tool("web_search", "联网搜索实时信息"),
            tool("shell_exec", "执行命令"),
        ];
        tools.extend((0..12).map(|i| tool(&format!("special_{i}"), "专用能力")));
        tools
    }

    /// 宽目录：固定高频面全部存在 + 专用工具，保证发现器启用。
    fn wide_catalog() -> Vec<ToolDef> {
        let mut tools: Vec<ToolDef> = FIXED_FACE
            .iter()
            .map(|name| tool(name, "固定高频面工具"))
            .collect();
        tools.push(tool("web_research", "联网研究网页资料并汇总来源"));
        tools.push(tool("office_automation", "生成 Excel 工作簿和 Word 文档"));
        tools.extend((0..8).map(|i| tool(&format!("special_{i}"), "专用能力")));
        tools
    }

    fn names(tools: &[ToolDef]) -> Vec<String> {
        tools.iter().map(|tool| tool.name.clone()).collect()
    }

    #[test]
    fn session_face_is_byte_stable_across_turns() {
        // W4.2 契约：同一会话内工具数组逐字节相同，不随用户文本变化。
        // 变异说明：把 `with_face` 换回「每轮 ToolDisclosure::new 后取 active」
        // 的旧按需预热实现，本用例立刻变红。
        let tools = wide_catalog();
        let face = Arc::new(Mutex::new(SessionToolFace::bootstrap(
            tools.clone(),
            ModeKind::Agent,
        )));
        let requests = ["你好", "你觉得这个方案怎么样", "谢谢！", "在吗", "今天天气不错"];
        let first = names(
            &ToolDisclosure::with_face(&tools, requests[0], ModeKind::Agent, Arc::clone(&face))
                .active_tools(),
        );
        assert!(!first.is_empty());
        for request in &requests[1..] {
            let turn = ToolDisclosure::with_face(&tools, request, ModeKind::Agent, Arc::clone(&face));
            assert_eq!(
                names(&turn.active_tools()),
                first,
                "会话工具面不得随用户文本变化：{request}"
            );
        }
    }

    #[test]
    fn discover_only_appends_and_keeps_sorted_order() {
        let tools = wide_catalog();
        let face = Arc::new(Mutex::new(SessionToolFace::bootstrap(
            tools.clone(),
            ModeKind::Agent,
        )));
        let mut turn =
            ToolDisclosure::with_face(&tools, "解释一下", ModeKind::Agent, Arc::clone(&face));
        let before = names(&turn.active_tools());
        assert!(turn.is_enabled(), "宽目录应启用发现器");
        assert!(!before.contains(&"web_research".to_string()));

        let output = turn.discover("web");
        assert!(output.contains("web_research"), "{output}");
        let after = names(&turn.active_tools());
        for name in &before {
            assert!(after.contains(name), "发现不得删除既有工具：{name}");
        }
        assert!(after.contains(&"web_research".to_string()));
        assert!(
            after.windows(2).all(|pair| pair[0] < pair[1]),
            "工具面必须按名字排序：{after:?}"
        );

        // 下一轮复用同一会话面：数组不变（只追加、不回退）。
        let next = ToolDisclosure::with_face(&tools, "换个话题", ModeKind::Agent, Arc::clone(&face));
        assert_eq!(names(&next.active_tools()), after);
        assert_eq!(next.metrics().initially_exposed_tools, after.len() - 1);

        // 再次发现同一查询：没有新工具可加，数组逐字节不变。
        let mut again = ToolDisclosure::with_face(&tools, "继续", ModeKind::Agent, Arc::clone(&face));
        let text = again.discover("web");
        assert!(text.contains("未找到"), "{text}");
        assert_eq!(names(&again.active_tools()), after);
    }

    #[test]
    fn session_face_bootstrap_is_order_independent_and_mode_filtered() {
        let mut tools = wide_catalog();
        let first = names(&SessionToolFace::bootstrap(tools.clone(), ModeKind::Agent).active_tools());
        tools.reverse();
        let second = names(&SessionToolFace::bootstrap(tools, ModeKind::Agent).active_tools());
        assert_eq!(first, second, "工具面不得依赖目录输入顺序");

        let plan = SessionToolFace::bootstrap(wide_catalog(), ModeKind::Plan);
        let plan_names = names(&plan.active_tools());
        assert!(
            !plan_names.contains(&"write_file".to_string()),
            "Plan 模式不得暴露写工具：{plan_names:?}"
        );
        assert!(plan_names.contains(&"read_file".to_string()));
    }

    #[test]
    fn separate_sessions_do_not_share_discovery_results() {
        // 会话切换 = 新面：上一会话的发现结果不会被带过去。
        let tools = wide_catalog();
        let mut first = ToolDisclosure::new(&tools, "解释一下", ModeKind::Agent);
        let second = ToolDisclosure::new(&tools, "解释一下", ModeKind::Agent);
        assert!(first.discover("web").contains("web_research"));
        assert!(!names(&second.active_tools()).contains(&"web_research".to_string()));
    }

    #[test]
    fn direct_answer_request_does_not_resize_the_session_face() {
        // W4.2：直答判定不再裁剪工具数组（只作为观测/尾部提示输入）。
        let tools = wide_tools();
        let direct = ToolDisclosure::new(&tools, "你觉得今天桂林好看吗", ModeKind::Agent);
        assert!(
            direct.metrics().direct_answer,
            "主观评价短句应判定为直答"
        );
        let task = ToolDisclosure::new(&tools, "帮我重构这个模块", ModeKind::Agent);
        assert!(!task.metrics().direct_answer);
        assert_eq!(
            names(&direct.active_tools()),
            names(&task.active_tools()),
            "直答轮与任务轮的工具数组必须一致"
        );
        assert!(!direct.active_tools().is_empty());
    }

    #[test]
    fn realtime_weather_question_keeps_the_full_surface() {
        let tools = wide_tools();
        let disclosure =
            ToolDisclosure::new(&tools, "你觉得桂林明天会不会下雨", ModeKind::Agent);
        assert!(!disclosure.metrics().direct_answer, "实时天气不得走直答");
        assert!(!disclosure.active_tools().is_empty());
    }

    #[test]
    fn action_requests_are_never_direct_answer() {
        for request in [
            "你好，帮我查一下今天的汇率",
            "你觉得这个代码怎么样",
            "谢谢，顺便生成一个 Excel",
            "画一棵树",
            "在吗？打开浏览器搜资料",
        ] {
            assert!(
                !is_direct_answer_request(request),
                "动作意图不得判为直答：{request}"
            );
        }
    }

    #[test]
    fn long_conversational_text_is_not_direct_answer() {
        let long = "你觉得这个方案里关于多智能体协同和上下文压缩的设计在长期运行下是否还站得住脚，我担心扩展性";
        assert!(!is_direct_answer_request(long), "超长请求必须保留工具面");
        assert!(is_direct_answer_request("你好"));
        assert!(is_direct_answer_request("谢谢！"));
        assert!(!is_direct_answer_request(""));
    }
}
