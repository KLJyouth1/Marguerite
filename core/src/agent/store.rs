//! 智能体（Agent）台账：真实 CRUD + 持久化 `data_dir/agents.json`
//!
//! 解决「自定义性格与技能组合智能体是假的」问题：此前 `agents.list` 硬编码空数组，
//! 且 create/update/delete 无 Rust 实现。本模块提供进程单例台账，内置预置智能体
//! （builtin）与用户自定义智能体（custom）统一落盘，供 `agents.list/create/update/delete`
//! 网关方法与对话人设注入（`personality_hint`）消费。

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent::profile::auto_profile;
use crate::domain::Message;
use crate::persistence::config::Config;

/// 智能体记录（字段对齐 WebUI `types/agents.ts` 契约，camelCase + snake_case 双写）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentRecord {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tools: Vec<String>,
    /// 技能组合（自定义智能体可为空，内置智能体通常带默认技能）
    #[serde(default)]
    pub skills: Vec<String>,
    /// 性格/人设（system prompt）：对话时注入到模型系统消息
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub workspace: String,
    #[serde(default)]
    pub agent_dir: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub is_builtin: bool,
    #[serde(default)]
    pub model: String,
}

fn default_true() -> bool {
    true
}

impl AgentRecord {
    /// 序列化为网关契约（camelCase + snake_case 双写，兼容两端读取）
    pub fn to_value(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "description": self.description,
            "tools": self.tools,
            "skills": self.skills,
            "system_prompt": self.system_prompt,
            "systemPrompt": self.system_prompt,
            "workspace": self.workspace,
            "agent_dir": self.agent_dir,
            "agentDir": self.agent_dir,
            "enabled": self.enabled,
            "is_builtin": self.is_builtin,
            "isBuiltin": self.is_builtin,
            "type": if self.is_builtin { "builtin" } else { "custom" },
            "model": self.model,
        })
    }
}

/// 智能体台账（单例 + 落盘 `data_dir/agents.json`）
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AgentStore {
    pub agents: Vec<AgentRecord>,
}

impl AgentStore {
    fn path() -> PathBuf {
        Config::data_dir().join("agents.json")
    }

    fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    /// 落盘（原子写，走 `persistence::write_json_atomic`）。
    ///
    /// R13（2026-10-18）：此前用 `std::fs::write` 直写目标文件——该 API **先截断
    /// 再写**，进程在写入中途被杀/断电就留下半截 JSON；`load()` 解析失败走
    /// `unwrap_or_default()`，于是**用户自建智能体被静默清空**（内置会被 seed
    /// 补回，表面看不出丢过东西，丢的是用户数据）。
    fn save(&self) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(raw) = serde_json::to_string_pretty(self) {
            let _ = crate::persistence::write_json_atomic(&path, &raw);
        }
    }

    fn seed_builtins(&mut self) {
        let builtins = builtin_agents();
        let mut changed = false;
        for b in builtins {
            if let Some(existing) = self.agents.iter_mut().find(|a| a.id == b.id) {
                // 更新已有内置智能体的定义（工具名、system_prompt 等可能已修正）
                if existing.is_builtin {
                    existing.name = b.name;
                    existing.description = b.description;
                    existing.model = b.model;
                    existing.tools = b.tools;
                    existing.system_prompt = b.system_prompt;
                    existing.skills = b.skills;
                    // P0-5：同步 `enabled`。内置智能体不可经 RPC 编辑（builtin_immutable），
                    // 故其启用态只来自模板；不在这同步，旧装机（enabled:false）即使升级代码
                    // 也不会被修正——缺陷会「修了等于没修」。
                    existing.enabled = b.enabled;
                    changed = true;
                }
            } else {
                self.agents.push(b);
                changed = true;
            }
        }
        if changed {
            self.save();
        }
    }

    fn find(&self, id: &str) -> Option<&AgentRecord> {
        self.agents.iter().find(|a| a.id == id)
    }

    fn find_mut(&mut self, id: &str) -> Option<&mut AgentRecord> {
        self.agents.iter_mut().find(|a| a.id == id)
    }
}

fn store() -> &'static Mutex<AgentStore> {
    static S: OnceLock<Mutex<AgentStore>> = OnceLock::new();
    S.get_or_init(|| {
        let mut s = AgentStore::load();
        s.seed_builtins();
        Mutex::new(s)
    })
}

/// 主智能体（`main`）系统人设：高级开发工程师 · 严工。
///
/// 来源：专家包 `my-experts/plugins/senior-dev-engineer/agents/senior-dev-engineer.md`
/// （正文整篇纳入；仅把原文示例里的异域业务词汇换成 Marguerite Domain 词汇，
/// 末尾追加本机工程化操作守则）。
///
/// 人设是**内容资产**而非代码：正文放在 `agent/personas/senior-dev-engineer.md`，
/// 编译期 `include_str!` 内联。此前它以 `r#"…"#` 内联在本文件里（160+ 行），
/// 导致台账逻辑与提示词互相淹没、diff 不可读、改文案要改 Rust。
///
/// ⚠️ 两处硬约束（改前必读）：
/// 1. **不得包含** `<!--persona-complete-->` 独立行——那会让 `strip_complete_marker`
///    置 complete=true，从而阻断 skills/工具段的追加（`personality_for` 的段顺序）。
/// 2. 避免 `{{` / `}}`——`resolve_template` 会把未知变量替换为空串。
const MAIN_AGENT_SYSTEM_PROMPT: &str = include_str!("personas/senior-dev-engineer.md");

/// 主智能体人设正文（编译期内联）。
///
/// 存在的唯一理由：让 `agent.selfcheck` 能**证明**"定义真的被读进来了"——
/// 用户问「你到底有没有加载我的智能体定义」时，答案必须是字节数 + sha256，
/// 而不是"应该加载了"。
pub(crate) fn main_agent_prompt() -> &'static str {
    MAIN_AGENT_SYSTEM_PROMPT
}

/// UI/UX 负责人的完整人设（与主智能体同一套"内容资产"约定）。
///
/// ⚠️ 同样约束：不得出现 `<!--persona-complete-->` 独立行，避免 `{{ }}`。
const UI_DESIGN_SYSTEM_PROMPT: &str = include_str!("personas/ui-design-engineer.md");

/// 内置预置智能体（可被「自定义」复制为 custom，本身不可编辑）
fn builtin_agents() -> Vec<AgentRecord> {
    let s = |items: &[&str]| items.iter().map(|x| x.to_string()).collect::<Vec<String>>();
    let all_tools = s(&[
        "web_search",
        "web_fetch",
        "web_research",
        "browser",
        "system_control",
        "computer_use",
        "window_manage",
        "clipboard",
        "process_manage",
        "office_automation",
        "read_file",
        "write_file",
        "edit_file",
        "apply_patch",
        "make_dir",
        "copy_file",
        "move_file",
        "delete_file",
        "list_dir",
        "find_files",
        "grep",
        "codebase_search",
        "shell_exec",
        "code_exec",
        "git",
        "memory_search",
        "memory_save",
        "vision",
        "research",
        "list_skills",
        "use_skill",
        "create_skill",
        "automation",
        "json_query",
        "kb_search",
        "kb_ingest",
        // 媒体域（R14）：ffmpeg 真实桥 —— probe / convert / frames / thumbnail
        "media",
        // 3D 域（R72）：Blender 无头桥 —— run_script / render / probe
        "blender",
        // 多模态（2026-10-23）：生图 / 生视频 / 语音转写 / 语音合成
        "image_generate",
        "video_generate",
        // 本地生视频（2026-10-24）：代码打字动画 / 网页动效 / Manim
        "code_video",
        "web_video",
        "manim_video",
        "audio_transcribe",
        "audio_speak",
        // RPA 录放（2026-10-23）：录制用户真实键鼠操作并回放
        "auto_record",
        "auto_replay",
    ]);
    let mut agents = vec![
        AgentRecord {
            id: "main".to_string(),
            name: "严工 · 高级开发工程师".to_string(),
            description: "学霸型高级工程师：先盘点工具再动手、跨学科融合、诊断根因扫全类，联网找现成方案，工匠精神死磕到底，端到端验证后才交付。".to_string(),
            tools: all_tools.clone(),
            skills: s(&[
                "根因诊断",
                "举一反三",
                "边界穷举",
                "端到端验证",
                "工具编排",
                "跨学科分析",
                "联网研究",
                "工匠交付",
                "音视频处理",
            ]),
            // 2026-10-12：主智能体人设换为「高级开发工程师 · 严工」（senior-dev-engineer
            // 专家包整包接入桌面端）。**id 保持 `main` 不变**——新会话默认键
            // `agent:main:webchat:default`、侧边栏、定时任务 agentId 默认值、`@` 引用
            // 全部按 `main` 寻址，换 id 会让存量会话与定时任务集体失联。
            // 人设来源：my-experts/plugins/senior-dev-engineer/agents/senior-dev-engineer.md
            // （正文整篇纳入，仅把示例中的异域业务换成 Marguerite Domain 词汇）；
            // 末尾追加本机操作守则（源自原主智能体提示词的工程化条款）。
            // ⚠️ 内置记录由 `seed_builtins()` 就地热更新，存量 agents.json 下次启动即生效。
            system_prompt: MAIN_AGENT_SYSTEM_PROMPT.to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "code-architect".to_string(),
            name: "架构师".to_string(),
            description: "系统设计与架构决策，技术方案评审。".to_string(),
            tools: s(&["read_file", "write_file", "edit_file", "list_dir", "find_files", "grep", "codebase_search", "web_search", "web_fetch", "memory_search", "list_skills", "use_skill"]),
            skills: s(&["系统设计", "架构评审", "技术选型", "代码规划"]),
            system_prompt: "你是资深软件架构师 Bob。擅长系统设计、模块划分、技术选型与架构评审。输出结构化设计文档，包含类图/时序图/文件清单/任务列表。用与用户需求相同的语言回复。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "code-engineer".to_string(),
            name: "工程师".to_string(),
            description: "编写优雅、可读、可扩展、高效的代码。".to_string(),
            tools: s(&["read_file", "write_file", "edit_file", "apply_patch", "list_dir", "find_files", "grep", "codebase_search", "shell_exec", "code_exec", "git", "memory_search", "list_skills", "use_skill", "create_skill"]),
            skills: s(&["代码实现", "测试驱动", "调试修复", "Git 工作流"]),
            system_prompt: "你是软件工程师 Alex。负责编写优雅、可读、可扩展、高效的代码，遵循 Google 风格规范。先理解上下文再动手，小步提交，每步可验证。用与用户需求相同的语言回复。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "code-reviewer".to_string(),
            name: "代码审查专家".to_string(),
            description: "代码审查、安全审计、质量把关。".to_string(),
            tools: s(&["read_file", "grep", "codebase_search", "find_files", "list_dir", "memory_search", "list_skills", "use_skill"]),
            skills: s(&["代码审查", "安全审计", "性能优化", "规范检查"]),
            system_prompt: "你是资深代码审查专家。注重代码规范、安全性、可维护性与边界条件，给出可执行的改进建议。审查时逐项列出问题、严重级别和修复方案。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "ui-designer".to_string(),
            name: "UI 设计团队 · 液态玻璃".to_string(),
            description: "界面与体验设计负责人（设计系统 / 视觉 / 动效 / 空间 3D / 验收 五个角色）：令牌驱动、先量后调、克制四原则，产出可测量可回归的界面系统。".to_string(),
            tools: s(&[
                "read_file",
                "write_file",
                "edit_file",
                "apply_patch",
                "list_dir",
                "find_files",
                "grep",
                "codebase_search",
                "shell_exec",
                "code_exec",
                "vision",
                "image_generate",
                "browser",
                "computer_use",
                "web_search",
                "web_fetch",
                "memory_search",
                "memory_save",
                "list_skills",
                "use_skill",
            ]),
            skills: s(&["设计系统", "视觉层级", "动效设计", "三维背景", "可访问性", "参数化调参", "截图取证"]),
            // 2026-10-24：把两轮沉淀（液态玻璃的参数面板工作流 + 玛格丽特三维山脊的场景纪律）
            // 整篇作为人设，与主智能体同一套 include_str! 内容资产约定，改文案不用改 Rust。
            system_prompt: UI_DESIGN_SYSTEM_PROMPT.to_string(),
            is_builtin: true,
            // 默认启用与否由函数末尾的 DEFAULT_ENABLED_BUILTINS 统一决定（那里是唯一生效处）
            ..Default::default()
        },
        AgentRecord {
            id: "qa-engineer".to_string(),
            name: "测试工程师".to_string(),
            description: "测试用例设计、质量保障、缺陷追踪。".to_string(),
            tools: s(&["read_file", "write_file", "edit_file", "shell_exec", "code_exec", "grep", "find_files", "list_skills", "use_skill"]),
            skills: s(&["测试设计", "自动化测试", "边界分析", "回归验证"]),
            system_prompt: "你是 QA 测试工程师。擅长设计测试用例、边界分析、自动化测试和缺陷报告。确保每个功能都有正面和负面测试覆盖。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "product-manager".to_string(),
            name: "产品经理".to_string(),
            description: "需求分析、PRD 撰写、优先级排序。".to_string(),
            tools: s(&["web_search", "web_fetch", "web_research", "memory_search", "memory_save", "list_skills", "use_skill"]),
            skills: s(&["需求分析", "PRD 撰写", "用户故事", "优先级排序"]),
            system_prompt: "你是产品经理 Alice。负责需求分析、PRD 撰写、用户故事拆解和优先级排序。输出结构化的产品文档，包含用户场景、功能列表和验收标准。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "researcher".to_string(),
            name: "研究员".to_string(),
            description: "深度联网研究、信息检索、报告撰写。".to_string(),
            tools: s(&["web_search", "web_fetch", "web_research", "browser", "research", "memory_search", "memory_save", "read_file", "write_file", "list_skills", "use_skill", "kb_search", "kb_ingest"]),
            skills: s(&["联网研究", "文献综述", "信息验证", "报告撰写"]),
            system_prompt: "你是深度研究员。擅长多源信息检索、交叉验证、综合分析。使用 web_research 进行检索+深读，用 browser 处理需要登录的页面，输出带来源引用的结构化研究报告。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "browser-operator".to_string(),
            name: "浏览器操控员".to_string(),
            description: "浏览器自动化、网页操作、数据采集。".to_string(),
            tools: s(&["browser", "web_search", "web_fetch", "system_control", "computer_use", "vision", "screenshot", "list_skills", "use_skill"]),
            skills: s(&["浏览器自动化", "网页数据采集", "表单填写", "视觉定位"]),
            system_prompt: "你是浏览器自动化专家。使用 browser 工具操控真实 Chromium（open/click/type/screenshot/a11y/vision_click），完成网页导航、数据采集、表单填写等任务。优先用 a11y 语义定位，视觉定位作为补充。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "computer-operator".to_string(),
            name: "电脑操控员".to_string(),
            description: "桌面操控、鼠标键盘控制、GUI 自动化。".to_string(),
            tools: s(&["system_control", "computer_use", "browser", "vision", "screenshot", "shell_exec", "window_manage", "clipboard", "process_manage", "office_automation", "auto_record", "auto_replay", "image_generate", "video_generate", "audio_transcribe", "audio_speak", "list_skills", "use_skill"]),
            skills: s(&["桌面自动化", "GUI 定位", "键鼠控制", "OCR 识别", "办公软件操控", "窗口管理"]),
            system_prompt: "你是桌面自动化专家。**优先 computer_use**（屏幕感知 + 元素定位，抗分辨率/DPI 变化）；system_control 作为兜底（全屏游戏、无 UIA 树的自绘界面、需要精确物理坐标的拖拽）。用 window_manage 管理窗口；用 process_manage 启动/关闭程序；用 office_automation 操控 WPS/Office；用 clipboard 读写剪贴板。**「我做一遍你照做」用 auto_record start → stop → auto_replay run**（回放前确认窗口位置未变）。生图用 image_generate，生视频用 video_generate，音频转写用 audio_transcribe，朗读用 audio_speak（未配置就如实说缺口，不要假装完成）。操作前先截图观察，确认坐标后再执行，避免误操作。绝不轻易说无法完成，先用工具尝试。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "data-analyst".to_string(),
            name: "数据分析师".to_string(),
            description: "数据统计、分析、可视化。".to_string(),
            tools: s(&["read_file", "write_file", "shell_exec", "code_exec", "web_search", "web_fetch", "list_dir", "find_files", "json_query", "list_skills", "use_skill", "kb_search"]),
            skills: s(&["数据分析", "统计建模", "图表生成", "SQL 查询"]),
            system_prompt: "你是资深数据分析师。偏好用数据和事实说话，使用 code_exec 运行 Python 进行数据分析和可视化，输出结构化结论与图表建议。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "devops".to_string(),
            name: "运维工程师".to_string(),
            description: "部署、CI/CD、服务器管理、监控。".to_string(),
            tools: s(&["shell_exec", "read_file", "write_file", "edit_file", "git", "web_search", "web_fetch", "list_dir", "find_files", "list_skills", "use_skill"]),
            skills: s(&["部署运维", "CI/CD", "监控告警", "故障排查"]),
            system_prompt: "你是运维工程师。擅长服务器管理、部署流水线、容器编排、监控告警和故障排查。操作前确认影响范围，危险操作需说明后果。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "writer".to_string(),
            name: "文案写作".to_string(),
            description: "内容创作、文案撰写、翻译润色。".to_string(),
            tools: s(&["web_search", "web_fetch", "memory_search", "read_file", "write_file", "list_skills", "use_skill", "create_skill"]),
            skills: s(&["文案创作", "翻译润色", "内容策划", "SEO 优化"]),
            system_prompt: "你是专业文案写手。擅长各类文体创作、翻译润色、内容策划。根据场景调整语气和风格，输出有感染力的文字。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
    ];
    // P0-5：内置智能体此前全用 `..Default::default()`，而 `AgentRecord` 的 `Default`
    // 让 `enabled` 落为 `false` → `personality_for()` 对 `!enabled` 直接返回 `None`，
    // 12 个内置专家的人格永不注入。这里按 id **显式**启用 main + 4 个常用角色 +
    // UI 设计专家，其余保持禁用（用户可在界面按需打开）。不依赖 `Default`（其语义会被误读）。
    // ⚠️ 新增内置角色若要默认可用，必须同时加进这个常量——它是唯一生效处，
    // 记录里的 `enabled` 字段会被下一行无条件覆盖。
    const DEFAULT_ENABLED_BUILTINS: &[&str] = &[
        "main",
        "code-engineer",
        "researcher",
        "data-analyst",
        "code-reviewer",
        "ui-designer",
    ];
    for a in &mut agents {
        a.enabled = DEFAULT_ENABLED_BUILTINS.contains(&a.id.as_str());
    }
    agents
}

fn apply_patch(record: &mut AgentRecord, params: &Value) {
    if let Some(v) = params.get("name").and_then(Value::as_str) {
        record.name = v.to_string();
    }
    if let Some(v) = params.get("description").and_then(Value::as_str) {
        record.description = v.to_string();
    }
    if let Some(v) = params.get("system_prompt").and_then(Value::as_str) {
        record.system_prompt = v.to_string();
    }
    if let Some(v) = params.get("systemPrompt").and_then(Value::as_str) {
        record.system_prompt = v.to_string();
    }
    if let Some(v) = params.get("workspace").and_then(Value::as_str) {
        record.workspace = v.to_string();
    }
    if let Some(v) = params.get("agent_dir").and_then(Value::as_str) {
        record.agent_dir = v.to_string();
    }
    if let Some(v) = params.get("agentDir").and_then(Value::as_str) {
        record.agent_dir = v.to_string();
    }
    if let Some(v) = params.get("model").and_then(Value::as_str) {
        record.model = v.to_string();
    }
    if let Some(v) = params.get("enabled").and_then(Value::as_bool) {
        record.enabled = v;
    }
    if let Some(v) = params.get("tools").and_then(Value::as_array) {
        record.tools = v
            .iter()
            .filter_map(|x| {
                x.as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
            .collect();
    }
    if let Some(v) = params.get("skills").and_then(Value::as_array) {
        record.skills = v
            .iter()
            .filter_map(|x| {
                x.as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
            .collect();
    }
}

/// `agents.list`：返回全部智能体（内置 + 自定义），按是否内置排序、内置在前。
pub fn list() -> Value {
    let guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let mut items: Vec<Value> = guard.agents.iter().map(AgentRecord::to_value).collect();
    items.sort_by_key(|v| v["is_builtin"].as_bool() != Some(true)); // 内置在前
    json!({ "agents": items, "items": items })
}

/// `agents.get`：按 id 查单个智能体；不存在返回 None。
pub fn get(id: &str) -> Option<Value> {
    let guard = store().lock().unwrap_or_else(|e| e.into_inner());
    guard.find(id).map(AgentRecord::to_value)
}

/// 按 id 求系统人设片段（对话注入用；业务无关，供 chat 层调用）。
///
/// DSH persona 组合语义（v6）：core 段文本含独立 `<!--persona-complete-->`
/// 标记行时，标记剥除且该段 `complete=true`——其后技能/工具段被阻断不追加
/// （对齐上游 persona「complete 阻断后续 prompt 追加」）。
pub fn personality_for(id: &str) -> Option<String> {
    let guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let record = guard.find(id)?;
    if !record.enabled {
        return None;
    }
    let mut sections: Vec<crate::persona::PersonaSection> = Vec::new();
    if !record.system_prompt.trim().is_empty() {
        let (content, complete) =
            crate::persona::strip_complete_marker(record.system_prompt.trim());
        sections.push(crate::persona::PersonaSection {
            id: "core".into(),
            content,
            complete,
        });
    }
    if !record.skills.is_empty() {
        sections.push(crate::persona::PersonaSection::new(
            "skills",
            &format!("技能组合：{}。", record.skills.join("、")),
        ));
    }
    // 工具段只声明「偏好」，不声明「可用」：本轮真实工具面由运行时动态裁剪
    // （`agent::runtime::tool_guide_text` 的 discover 规则），台账里的 tools 只是
    // 声明式清单。此前文案写「可用工具：…」，会让 36 个工具名与「当前只暴露了常用
    // 工具」的基础指引互相矛盾，诱导模型调用本轮并不存在的工具。
    if !record.tools.is_empty() {
        sections.push(crate::persona::PersonaSection::new(
            "tools",
            &format!(
                "偏好工具（实际可用以本轮工具清单为准，缺能力时先调用工具发现）：{}。",
                record.tools.join("、")
            ),
        ));
    }
    let rendered = crate::persona::render_sections(&sections, &std::collections::BTreeMap::new());
    (!rendered.trim().is_empty()).then_some(rendered)
}

/// 解析 create 参数中的可选 `history`，转换为抽取式画像所需的 `Vec<Message>`。
/// 解析失败（字段缺失/类型异常）或缺失时返回空，交由上游决定（诚实边界：不编造）。
fn parse_history(params: &Value) -> Vec<Message> {
    params
        .get("history")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| serde_json::from_value::<Message>(v.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// `agents.create`：新建自定义智能体。id 已存在（含内置）返回 `agent.exists`。
pub fn create(params: &Value) -> Result<Value, String> {
    let id = params
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "id 不可为空".to_string())?;
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    if guard.find(id).is_some() {
        return Err("agent.exists".to_string());
    }
    let mut record = AgentRecord {
        id: id.to_string(),
        enabled: true,
        is_builtin: false,
        ..Default::default()
    };
    apply_patch(&mut record, params);
    // W-5：未显式给定 system_prompt 时，据可选 history 以确定性抽取式自动生成人设
    // （接 P-11 auto_profile；不烧 token、不编造）。
    // 注意：就地写入 record.system_prompt，**绝不**二次加锁调 update —— 否则与上方
    // guard 重入死锁（std::sync::Mutex 非重入）。
    if record.system_prompt.trim().is_empty() {
        let history = parse_history(params);
        if !history.is_empty() {
            record.system_prompt = auto_profile(&history).system_prompt();
        }
    }
    guard.agents.push(record.clone());
    guard.save();
    Ok(record.to_value())
}

/// `agents.update`：更新自定义智能体。内置智能体不可改（`agent.builtin_immutable`）；
/// 不存在返回 `agent.not_found`。
pub fn update(params: &Value) -> Result<Value, String> {
    let id = params
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "id 不可为空".to_string())?;
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let record = guard
        .find_mut(id)
        .ok_or_else(|| "agent.not_found".to_string())?;
    if record.is_builtin {
        return Err("agent.builtin_immutable".to_string());
    }
    apply_patch(record, params);
    let cloned = record.clone();
    guard.save();
    Ok(cloned.to_value())
}

/// `agents.delete`：删除自定义智能体。内置不可删（`agent.builtin_immutable`）。
pub fn delete(id: &str) -> Result<Value, String> {
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let idx = guard
        .agents
        .iter()
        .position(|a| a.id == id)
        .ok_or_else(|| "agent.not_found".to_string())?;
    if guard.agents[idx].is_builtin {
        return Err("agent.builtin_immutable".to_string());
    }
    guard.agents.remove(idx);
    guard.save();
    Ok(json!({ "deleted": true, "id": id }))
}

/// 从会话 key 解析智能体 id（`agent:{agentId}:...` 前缀），非智能体会话返回 None。
pub fn agent_id_from_session(session: &str) -> Option<&str> {
    session
        .strip_prefix("agent:")?
        .split(':')
        .next()
        .filter(|s| !s.is_empty())
}

/// 测试专用：清空自定义智能体并重建内置预置，保证单例台账在测试间隔离。
#[cfg(test)]
pub fn reset_for_test() {
    let s = store();
    let mut g = s.lock().unwrap_or_else(|e| e.into_inner());
    g.agents.retain(|a| a.is_builtin);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_builtins_once_and_list_contract() {
        // 直接构造 store 验证 seed 幂等 + 契约键
        let mut s = AgentStore::default();
        s.seed_builtins();
        assert!(s.agents.iter().any(|a| a.id == "main" && a.is_builtin));
        let n = s.agents.len();
        s.seed_builtins(); // 幂等：不重复 seed
        assert_eq!(s.agents.len(), n);
        let v = s.agents[0].to_value();
        assert_eq!(v["isBuiltin"], v["is_builtin"]);
        assert_eq!(v["systemPrompt"], v["system_prompt"]);
        assert_eq!(v["type"], "builtin");
        assert!(v["skills"].is_array());
    }

    /// 2026-10-12：主智能体（`main`）人设 = 高级开发工程师 · 严工（专家包接入桌面端）。
    /// 逐条钉死关键身份与两条硬约束（complete 标记 / 模板花括号）。
    #[test]
    fn main_agent_persona_is_senior_dev_engineer() {
        let agents = builtin_agents();
        let main = agents
            .iter()
            .find(|a| a.id == "main")
            .expect("main 内置智能体应存在");
        assert!(
            main.name.contains("严工"),
            "主智能体名称应体现严工身份；实际: {}",
            main.name
        );
        assert!(
            main.description.contains("根因"),
            "描述应说明专家定位；实际: {}",
            main.description
        );
        assert!(!main.skills.is_empty(), "技能组合不应为空");
        // 人格注入的硬前提：`personality_for` 对 `!enabled` 直接返回 None，
        // 主智能体一旦被停用，再完整的人设也只会是一段死文本。
        assert!(main.enabled, "主智能体必须默认启用，否则人格永不注入");
        // 人设核心条款必须在场（防回归成通用助手）
        for needle in [
            "能跑通不算做完，做到极致才算开始",
            "七条铁律",
            "九步工作流程",
            "Marguerite 工作台操作守则",
        ] {
            assert!(
                main.system_prompt.contains(needle),
                "主智能体人设缺少关键段: {needle}"
            );
        }
        // 硬约束 1：不得含 complete 标记（否则 skills/工具段被阻断不追加）
        assert!(
            !main.system_prompt.contains("<!--persona-complete-->"),
            "人设不得含 complete 标记"
        );
        // 硬约束 2：不得含模板花括号（resolve_template 会把未知变量替换为空串）
        assert!(
            !main.system_prompt.contains("{{") && !main.system_prompt.contains("}}"),
            "人设不得含 {{{{ }}}} 模板占位符"
        );
        // 工具集保持全量（主智能体要能调度全部工具）
        assert!(
            main.tools.len() >= 30,
            "主智能体应持全量工具；实际 {}",
            main.tools.len()
        );
    }

    /// P0-5：内置智能体默认启用 main + 4 个常用角色 + UI 设计专家；其余保持禁用。
    /// 直接对 `builtin_agents()` 断言（纯内存，不落盘、不触碰用户运行期 agents.json）。
    #[test]
    fn builtins_enable_only_the_common_set() {
        let agents = builtin_agents();
        let enabled: Vec<&str> = agents
            .iter()
            .filter(|a| a.enabled)
            .map(|a| a.id.as_str())
            .collect();
        for id in [
            "main",
            "code-engineer",
            "researcher",
            "data-analyst",
            "code-reviewer",
            "ui-designer",
        ] {
            assert!(
                enabled.contains(&id),
                "{id} 应默认启用；实际启用: {enabled:?}"
            );
        }
        for id in [
            "writer",
            "devops",
            "browser-operator",
            "computer-operator",
            "code-architect",
            "qa-engineer",
            "product-manager",
        ] {
            let a = agents
                .iter()
                .find(|a| a.id == id)
                .unwrap_or_else(|| panic!("内置智能体 {id} 应存在"));
            assert!(!a.enabled, "{id} 应保持禁用；实际启用: {enabled:?}");
        }
        assert_eq!(
            enabled.len(),
            6,
            "应恰好启用 6 个内置智能体；实际: {enabled:?}"
        );
    }

    /// P0-5 直接断言（QA 要求）：白名单 builtin 经**真实 API** `personality_for`
    /// 必须能取到人格。此前只靠「`builtin_agents()` 白名单 + `personality_for` 对
    /// `!enabled` 返回 `None`」的逻辑链间接保证；本测试对行为终点直接断言——
    /// 首次访问触发全局台账 `seed_builtins`（含 P0-5 的 `enabled` 同步，升级路径）。
    #[test]
    fn personality_for_returns_some_for_whitelisted_builtin() {
        for id in [
            "main",
            "code-engineer",
            "researcher",
            "data-analyst",
            "code-reviewer",
            "ui-designer",
        ] {
            let persona = personality_for(id)
                .unwrap_or_else(|| panic!("白名单 builtin {id} 必须能取到人格（enabled 且非空）"));
            assert!(!persona.trim().is_empty(), "{id} 的人格不得为空");
        }
        // 反向：非白名单 builtin 保持禁用 → personality_for 返回 None（P0-5 行为终点）。
        assert!(
            personality_for("writer").is_none(),
            "writer 未在白名单，必须保持禁用（personality_for → None）"
        );
    }

    /// `include_str!` 必须真的指向仓库内资产（防止资产被清空/挪走后静默变空串）。
    #[test]
    fn main_persona_asset_is_embedded() {
        assert!(
            MAIN_AGENT_SYSTEM_PROMPT.starts_with("你是严工"),
            "人设首行应为身份声明，实际：{}",
            MAIN_AGENT_SYSTEM_PROMPT.chars().take(20).collect::<String>()
        );
        assert!(
            MAIN_AGENT_SYSTEM_PROMPT.len() > 3000,
            "人设资产过短（{} 字节），疑似未内联",
            MAIN_AGENT_SYSTEM_PROMPT.len()
        );
    }

    /// 物理边界必须留在**生效**的人设里（R13，2026-10-18）。
    ///
    /// 背景：人设曾同时存在三份——专家包源、仓库根 `agents/` 草稿、以及真正被
    /// `include_str!` 内联的 `core/src/agent/personas/`。**只有最后一份生效**，
    /// 而「本机硬件边界（RTX 3050 Ti 4GB → 7B 不可本地训练）」那段恰好在**没人
    /// 引用的草稿**里 → 模型读不到这条约束，就可能承诺本地训练 7B 这类物理
    /// 不可能的事。现在收敛为单一真相源，并用本测试把它钉住。
    ///
    /// 硬件数字取自 2026-10-18 实测（CIM：i5-12500H 12C/16T、31.7 GB、
    /// RTX 3050 Ti Laptop 4 GB）。若换机器，改人设资产 + 本断言一起改。
    #[test]
    fn main_persona_keeps_physical_boundary_of_this_machine() {
        for needle in [
            "RTX 3050 Ti",
            "4 GB 显存",
            "7B 不可行",
            "不得把\"集成\"包装成\"训练\"",
        ] {
            assert!(
                MAIN_AGENT_SYSTEM_PROMPT.contains(needle),
                "生效人设必须保留本机物理边界条款：缺「{needle}」"
            );
        }
    }

    /// 生效人设必须保留「读题台账」与「PRD 门禁」两条前置纪律（2026-10-21）。
    ///
    /// 背景：用户连续两轮批评本智能体「老是没有读完我发的提示词」「没写 PRD」。
    /// 根因不是能力，而是人设里**没有任何可判定的产物要求** —— 于是漏读与范围膨胀
    /// 无人可查。修复方式是把它们写成硬条款（台账 R1–RN / PRD 未落盘不得写业务代码），
    /// 并用本测试钉住：任何人删掉这几句，门禁立即变红。
    ///
    /// 与 `main_persona_keeps_physical_boundary_of_this_machine` 同一思路：
    /// **关键约束必须留在真正被 `include_str!` 内联的那一份里**，不能只写在
    /// 专家包源或草稿里（历史上正是「写在了没人引用的草稿」导致约束失效）。
    #[test]
    fn main_persona_keeps_intake_and_prd_gates() {
        for needle in [
            "需求台账",
            "验收信号",
            "漏读",
            "等等",
            "PRD 门禁",
            "非目标",
            "改动契约",
        ] {
            assert!(
                MAIN_AGENT_SYSTEM_PROMPT.contains(needle),
                "生效人设必须保留读题/PRD 门禁条款：缺「{needle}」"
            );
        }
        // 门禁必须在**正文前部**（用户批评的是"动手太快"，条款埋在尾部等于没有）
        let intake_pos = MAIN_AGENT_SYSTEM_PROMPT.find("第 0 条：先读完").unwrap_or(usize::MAX);
        let first_capability = MAIN_AGENT_SYSTEM_PROMPT.find("核心能力").unwrap_or(usize::MAX);
        assert!(
            intake_pos < first_capability,
            "读题条款必须排在「核心能力」之前，否则模型会先动手后读题"
        );
    }

    #[test]
    fn create_update_delete_roundtrip() {
        // 不再调用 reset_for_test()：它会清空全局台账里所有自定义智能体，
        // 与并发运行的 W-5 测试（创建 auto-profile-agent-*）互相擦除导致隔离性假失败。
        // 改用唯一 id + 首尾 delete，与其他测试严格隔离。
        let _ = delete("roundtrip-polite-assistant");
        let _prev = AgentStore::path();
        // create
        let created = create(&json!({
            "id": "roundtrip-polite-assistant",
            "name": "礼貌助手",
            "system_prompt": "你说话礼貌客气。",
            "skills": ["礼貌", "幽默"],
            "tools": ["web_search"],
        }))
        .expect("create 应成功");
        assert_eq!(created["id"], "roundtrip-polite-assistant");
        assert_eq!(created["type"], "custom");
        assert_eq!(created["skills"][0], "礼貌");

        // 重复创建 → agent.exists
        let dup = create(&json!({ "id": "roundtrip-polite-assistant" })).unwrap_err();
        assert_eq!(dup, "agent.exists");

        // update
        let updated = update(&json!({ "id": "roundtrip-polite-assistant", "name": "超礼貌助手" }))
            .expect("update 应成功");
        assert_eq!(updated["name"], "超礼貌助手");

        // personality 注入片段
        let p = personality_for("roundtrip-polite-assistant").expect("应有人设");
        assert!(p.contains("礼貌客气"));
        assert!(p.contains("技能组合"));

        // delete
        let del = delete("roundtrip-polite-assistant").expect("delete 应成功");
        assert_eq!(del["deleted"], true);
        assert!(personality_for("roundtrip-polite-assistant").is_none());
    }

    #[test]
    fn persona_complete_marker_blocks_skill_and_tool_sections() {
        // DSH persona complete 语义：core 段含标记 → 技能/工具段被阻断
        let _ = delete("persona-complete-agent");
        let _prev = AgentStore::path();
        create(&json!({
            "id": "persona-complete-agent",
            "name": "阻断测试",
            "system_prompt": "只读我这一句。\n<!--persona-complete-->",
            "skills": ["礼貌"],
            "tools": ["web_search"],
        }))
        .expect("create 应成功");
        let p = personality_for("persona-complete-agent").expect("应有人设");
        assert!(p.contains("只读我这一句"), "core 段应保留: {p}");
        assert!(!p.contains("<!--persona-complete-->"), "标记应剥除: {p}");
        assert!(!p.contains("技能组合"), "complete 后技能段应被阻断: {p}");
        assert!(!p.contains("偏好工具"), "complete 后工具段应被阻断: {p}");
        let _ = delete("persona-complete-agent");
    }

    #[test]
    fn builtin_immutable_update_and_delete_rejected() {
        // 仅操作内置智能体，无需 reset_for_test()（避免清空并发测试创建的自定义智能体）
        let err_u = update(&json!({ "id": "main", "name": "x" })).unwrap_err();
        assert_eq!(err_u, "agent.builtin_immutable");
        let err_d = delete("main").unwrap_err();
        assert_eq!(err_d, "agent.builtin_immutable");
    }

    #[test]
    fn session_agent_id_parsing() {
        assert_eq!(
            agent_id_from_session("agent:data-analyst:webchat:abc123"),
            Some("data-analyst")
        );
        assert_eq!(
            agent_id_from_session("agent:main:webchat:xyz"),
            Some("main")
        );
        assert_eq!(agent_id_from_session("ws.plain"), None);
        assert_eq!(agent_id_from_session("agent::webchat:x"), None);
    }

    // ── W-5：新 agent 创建主路径默认自动生成人设（接 P-11 auto_profile）──
    #[test]
    fn create_auto_profiles_from_history_when_no_explicit_prompt() {
        let _ = delete("auto-profile-agent-1");
        let created = create(&json!({
            "id": "auto-profile-agent-1",
            "name": "画像自动生成",
            "history": [
                {"role": "user", "content": "写个 rust 函数部署到后端"},
                {"role": "assistant", "content": "好的，我用 Python 和 Rust 写后端代码。"}
            ]
        }))
        .expect("create 应成功");
        // 未显式给 system_prompt → 默认据 history 自动注入人设
        let sp = personality_for("auto-profile-agent-1").expect("应有人设");
        assert!(sp.contains("你是一个工程师。"), "实际：{sp}");
        assert!(sp.contains("Rust"), "实际：{sp}");
        // create 返回值也应含该人设（双写契约一致）
        assert!(created["systemPrompt"]
            .as_str()
            .unwrap_or("")
            .contains("你是一个工程师。"));
        let _ = delete("auto-profile-agent-1");
    }

    #[test]
    fn create_explicit_system_prompt_takes_precedence_over_history() {
        let _ = delete("auto-profile-agent-2");
        let _ = create(&json!({
            "id": "auto-profile-agent-2",
            "name": "显式人设优先",
            "system_prompt": "你是一个自定义翻译官。",
            "history": [
                {"role": "user", "content": "写个 rust 函数部署到后端"},
                {"role": "assistant", "content": "我用 Python 和 Rust 写后端代码。"}
            ]
        }))
        .expect("create 应成功");
        // 显式 system_prompt 不被 auto_profile 覆盖
        let sp = personality_for("auto-profile-agent-2").expect("应有人设");
        assert_eq!(sp, "你是一个自定义翻译官。");
        let _ = delete("auto-profile-agent-2");
    }

    #[test]
    fn create_without_history_leaves_prompt_empty() {
        let _ = delete("auto-profile-agent-3");
        let _ = create(&json!({
            "id": "auto-profile-agent-3",
            "name": "无历史无画像"
        }))
        .expect("create 应成功");
        // 既无 history 也无 system_prompt → 不伪造人设（保持默认空，personality_for 返回 None）
        assert!(
            personality_for("auto-profile-agent-3").is_none(),
            "无种子历史不应编造人设"
        );
        let _ = delete("auto-profile-agent-3");
    }
}
