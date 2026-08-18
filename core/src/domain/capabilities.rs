//! Marguerite 统一能力注册表
//!
//! 这是产品的「能力清单」唯一来源。TUI 与原生桌面 GUI 都从这里渲染能力树，
//! 因此 Marguerite 的能力叙事在 Rust 中只定义一次，且严格是 标准 AI 助手
//! 扁平工具列表的超集。
//!
//! 分级：
//! - `Baseline`：与 标准 AI 助手 等价的基线能力
//! - `Enhanced`：在其之上做了实质改进
//! - `Exclusive`：标准 AI 助手 中不存在的能力

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

use super::CapabilityId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityTier {
    Baseline,
    Enhanced,
    Exclusive,
}

impl CapabilityTier {
    pub fn label(&self) -> &'static str {
        match self {
            CapabilityTier::Baseline => "基线",
            CapabilityTier::Enhanced => "增强",
            CapabilityTier::Exclusive => "独家",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityGroup {
    CoreLoop,
    Orchestration,
    Tooling,
    Knowledge,
    Security,
    Automation,
    Multimodal,
    Research,
    Governance,
    Resilience,
    Model,
    Memory,
}

impl CapabilityGroup {
    pub fn label(&self) -> &'static str {
        match self {
            CapabilityGroup::CoreLoop => "核心编码循环",
            CapabilityGroup::Orchestration => "编排与协作",
            CapabilityGroup::Tooling => "工具集",
            CapabilityGroup::Knowledge => "知识与检索",
            CapabilityGroup::Security => "安全与沙箱",
            CapabilityGroup::Automation => "自动化与 RPA",
            CapabilityGroup::Multimodal => "多模态",
            CapabilityGroup::Research => "研究管线",
            CapabilityGroup::Governance => "治理与门禁",
            CapabilityGroup::Resilience => "韧性与恢复",
            CapabilityGroup::Model => "模型与后端",
            CapabilityGroup::Memory => "记忆系统",
        }
    }
}

/// 能力成熟度（机器可校验「叙事 vs 真货」）。
///
/// - `Real`：已实装且测试守护（✅）
/// - `Partial`：框架/核心实装但含桩或依赖运行环境（🟡）
/// - `Narrative`：纯叙事，依赖重型原生或 LLM 后端，当前零实现（⚪）
///
/// 派生 `is_implemented()` = `maturity != Narrative`，根除「宣称未落地」。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityMaturity {
    Real,
    Partial,
    Narrative,
}

impl CapabilityMaturity {
    pub fn label(&self) -> &'static str {
        match self {
            CapabilityMaturity::Real => "已实装",
            CapabilityMaturity::Partial => "部分实装",
            CapabilityMaturity::Narrative => "纯叙事",
        }
    }
    /// 是否为真实落地（非纯叙事）
    pub fn is_implemented(&self) -> bool {
        !matches!(self, CapabilityMaturity::Narrative)
    }
}

/// 一项能力。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Capability {
    pub id: CapabilityId,
    pub name: String,
    pub group: CapabilityGroup,
    pub tier: CapabilityTier,
    /// 成熟度：机器校验「叙事 vs 真货」
    #[serde(default = "default_maturity")]
    pub maturity: CapabilityMaturity,
    pub description: String,
    /// 取代的外部项目能力（用于追溯整合来源）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
}

fn default_maturity() -> CapabilityMaturity {
    CapabilityMaturity::Narrative
}

/// 全量能力清单（超集）。标准 AI 助手 仅覆盖 `CoreLoop` + 部分 `Tooling(Baseline)`。
fn build_capabilities() -> Vec<Capability> {
    use CapabilityMaturity::*;
    macro_rules! cap {
        ($id:expr, $name:expr, $group:expr, $tier:expr, $desc:expr, $mat:expr) => {
            Capability {
                id: CapabilityId($id.to_string()),
                name: $name.to_string(),
                group: $group,
                tier: $tier,
                maturity: $mat,
                description: $desc.to_string(),
                supersedes: None,
            }
        };
        ($id:expr, $name:expr, $group:expr, $tier:expr, $desc:expr, $mat:expr, $from:expr) => {
            Capability {
                id: CapabilityId($id.to_string()),
                name: $name.to_string(),
                group: $group,
                tier: $tier,
                maturity: $mat,
                description: $desc.to_string(),
                supersedes: Some($from.to_string()),
            }
        };
    }

    vec![
        // ── 核心编码循环（基线 + 增强）──
        cap!("core.chat", "自然语言编码对话", CapabilityGroup::CoreLoop, CapabilityTier::Baseline, "用对话完成读/写/重构/调试，等价 标准 AI 助手 的编辑循环", Real),
        cap!("core.tool_cycle", "工具驱动循环", CapabilityGroup::CoreLoop, CapabilityTier::Enhanced, "Agent 自主调用工具并基于结果迭代，带自检与回退", Real, "nexustar agent_runtime"),
        cap!("core.diff", "结构化差异预览", CapabilityGroup::CoreLoop, CapabilityTier::Enhanced, "每次改动产出可读 diff，支持一键接受/拒绝", Partial),
        cap!("core.apply", "安全应用改动", CapabilityGroup::CoreLoop, CapabilityTier::Enhanced, "改动经护栏校验后才落盘，等价但不止于 Cursor 的 apply", Real),

        // ── 编排与协作（独家，超越项）──
        cap!("orch.dual_leader", "MAX 双带队", CapabilityGroup::Orchestration, CapabilityTier::Exclusive, "规划(Flash)与执行(Pro)两条轨道分离，顺序两阶段执行（planner 先、executor 后，非并行）", Partial, "nexustar max_orchestrator"),
        cap!("orch.expert_team", "专家团队", CapabilityGroup::Orchestration, CapabilityTier::Exclusive, "角色专精子 Agent 按 spawn 模型协作，真实并行执行 + lead 综合（agent::team::Team 原语）", Real, "nexustar expert_team_orchestrator"),
        cap!("orch.cluster", "集群调度", CapabilityGroup::Orchestration, CapabilityTier::Exclusive, "fan-out 多 Worker 真实并行 spawn + 聚合（dispatch Cluster 分支；非完整 allocator）", Partial, "nexustar cluster/allocator"),
        cap!("orch.swarm", "swarm 有向无环图", CapabilityGroup::Orchestration, CapabilityTier::Exclusive, "DAG 依赖驱动并行执行（dispatch_team_dag + orchestration/dag.rs）+ Bloom 蜂群计划生成器", Partial),

        // ── 工具集（46 工具原生实现，统一 ToolExecutor）──
        cap!("tool.exec", "命令执行", CapabilityGroup::Tooling, CapabilityTier::Baseline, "在 OS 沙箱内执行 shell 命令", Real, "nexustar tools/exec"),
        cap!("tool.file", "文件读写", CapabilityGroup::Tooling, CapabilityTier::Baseline, "读/写/搜索/批量重命名", Real, "nexustar tools/file"),
        cap!("tool.terminal", "终端操控", CapabilityGroup::Tooling, CapabilityTier::Enhanced, "原生终端会话管理：行模式命令执行器（ANSI 安全剥离）+ Windows ConPTY 交互式（彩色/TUI/光标，vt100 渲染）；行模式跨平台，PTY 限 Windows 10 1809+", Real, "nexustar tools/terminal"),
        cap!("tool.browser", "浏览器自动化", CapabilityGroup::Tooling, CapabilityTier::Enhanced, "页面导航/截图/抽取（browser_bridge 重型桥，需本机 Chromium）", Partial, "nexustar tools/browser"),
        cap!("tool.git", "版本控制", CapabilityGroup::Tooling, CapabilityTier::Baseline, "commit/PR/blame/二分调试（真实 git CLI 桥，白名单+工作区守卫）", Real),
        cap!("tool.fs_watch", "文件监听", CapabilityGroup::Tooling, CapabilityTier::Enhanced, "工作区 notify 递归监听：新增/修改事件通过总线发布；GUI 侧接入为「触发 Agent 反应」待做", Partial, "nexustar notify"),
        cap!("tool.rollback", "快照回滚", CapabilityGroup::Tooling, CapabilityTier::Exclusive, "增量 tar 快照 + sha256，随时回退", Real, "nexustar recovery"),
        cap!("tool.mcp_bridge", "MCP 工具桥", CapabilityGroup::Tooling, CapabilityTier::Enhanced, "将任意 MCP server 的工具暴露为原生工具", Partial, "nexustar mcp"),
        cap!("tool.baota", "宝塔面板运维工具箱", CapabilityGroup::Tooling, CapabilityTier::Enhanced, "宝塔 Linux 面板 7.x/8.x 的站点/数据库/SSL/文件/计划任务管理：API key 优先、面板 CA 证书兜底；批量建站、一键下发 SSL、端口放行、容器/运行时管理、日志抽取、备份迁移", Real, "marguerite tools/baota"),
        cap!("tool.desktop_control", "桌面级 Computer Use（键鼠屏幕）", CapabilityGroup::Tooling, CapabilityTier::Exclusive, "Windows user32/SendInput 原生系统调用：mouse_move(贝塞尔拟人化轨迹)/mouse_click/mouse_scroll/mouse_pos/screen_size/key_press/type_text；支持多显示器，可被 FPS 助手/跨应用 RPA/录放回放直接消费", Real, "marguerite tools/system_control"),
        cap!("tool.fps_assistant", "FPS 第一人称射击操作助手", CapabilityGroup::Automation, CapabilityTier::Exclusive, "基于 Computer Use 封装的射击游戏操作代理：屏幕中心/瞄头线/压枪/跟枪/急停/搜点/扫车/一键拉枪；默认 8ms 步长，配合 humanize 轨迹规避反作弊；需在设置 › Labs 中显式开启并以 FPS-agent 技能启动", Partial, "marguerite skills/fps"),
        cap!("auto.uia", "UIA 跨应用桥", CapabilityGroup::Automation, CapabilityTier::Exclusive, "Windows UI Automation 元素级跨应用操作：定位窗口句柄、读控件树、触发按钮/菜单；可与 desktop_control 组合（前者读树后者精确定位），限桌面安装包", Partial),

        // ── 知识与检索（独家）──
        cap!("kb.rag", "RAG 检索", CapabilityGroup::Knowledge, CapabilityTier::Exclusive, "本地向量检索 + 重排，零外部依赖", Real, "nexustar kb_rag"),
        cap!("kb.graph", "知识图谱", CapabilityGroup::Knowledge, CapabilityTier::Exclusive, "实体关系图谱，支撑跨文件推理", Real, "nexustar knowledge_graph"),
        cap!("kb.denoiser", "去噪层", CapabilityGroup::Knowledge, CapabilityTier::Exclusive, "实体关系去噪 + Jaccard 去重（本会话实装，已补登记）", Real),
        cap!("kb.integrity", "完整性审计", CapabilityGroup::Knowledge, CapabilityTier::Exclusive, "知识入库完整性审计（本会话实装，已补登记）", Real),
        cap!("kb.ima", "私有知识库", CapabilityGroup::Knowledge, CapabilityTier::Exclusive, "本地 RAG（BM25+向量）+ cross_project 跨项目记忆 + 画像导入（memory.import.*），可接入用户私有文档（CSV/MD/文本）；原生批量导入 Wizard 待做", Partial),

        // ── 安全与沙箱（独家，OS 级）──
        cap!("sec.os_sandbox", "OS 级沙箱", CapabilityGroup::Security, CapabilityTier::Exclusive, "Landlock(Linux)/Seatbelt(macOS) 内核级隔离；Windows 显式 Unsupported，命令级 sandbox/ 兜底", Partial, "TUI 工作台 sandbox"),
        cap!("sec.malware", "恶意代码扫描", CapabilityGroup::Security, CapabilityTier::Exclusive, "YARA/正则扫描生成与执行的代码", Real, "nexustar malicious_code_scanner"),
        cap!("sec.ssrf", "SSRF 守卫", CapabilityGroup::Security, CapabilityTier::Exclusive, "拦截内网/元数据端点请求", Real, "nexustar ssrf_guard"),
        cap!("sec.guardrail", "操作护栏", CapabilityGroup::Security, CapabilityTier::Enhanced, "危险操作需审批门", Real, "外部参考 exec 审批门"),

        // ── 自动化与 RPA（独家）──
        cap!("auto.record", "操作录制", CapabilityGroup::Automation, CapabilityTier::Exclusive, "录制用户操作序列", Real, "nexustar automations"),
        cap!("auto.replay", "回放与编排", CapabilityGroup::Automation, CapabilityTier::Exclusive, "回放并支持条件/循环", Real, "nexustar control_tool"),
        cap!("auto.fix", "自动修复", CapabilityGroup::Automation, CapabilityTier::Exclusive, "基于 LLM 后端对错误/测试失败生成修复并安全落盘（依赖 model.cloud 运行期出真回复）", Partial, "hermes self-healing"),
        cap!("auto.uia", "UIA 桥", CapabilityGroup::Automation, CapabilityTier::Exclusive, "跨应用 UI 自动化（RPA）", Narrative),

        // ── 多模态（独家）──
        cap!("vis.image", "图像理解", CapabilityGroup::Multimodal, CapabilityTier::Exclusive, "看图/截图/设计稿理解", Real, "nexustar vision"),
        cap!("vis.audio", "音频理解", CapabilityGroup::Multimodal, CapabilityTier::Exclusive, "语音/音频转写与理解", Narrative),

        // ── 研究管线（独家）──
        cap!("rs.pipeline", "研究管线", CapabilityGroup::Research, CapabilityTier::Exclusive, "多步检索-综合-引用", Real, "nexustar research"),
        cap!("rs.codebase", "代码库检索", CapabilityGroup::Research, CapabilityTier::Exclusive, "语义级跨仓代码检索", Real, "nexustar codebase_search"),

        // ── 治理与门禁（独家）──
        cap!("gov.plan_gate", "计划门禁", CapabilityGroup::Governance, CapabilityTier::Exclusive, "执行前计划需通过门禁", Real, "nexustar plan_gate"),
        cap!("gov.constitution", "宪法约束", CapabilityGroup::Governance, CapabilityTier::Exclusive, "不可违背的顶层规则", Real, "hermes constitution"),
        cap!("gov.decision", "决策协议", CapabilityGroup::Governance, CapabilityTier::Exclusive, "结构化裁决与日志", Real, "nexustar decision_protocol"),
        cap!("gov.review", "自动评审", CapabilityGroup::Governance, CapabilityTier::Exclusive, "对 diff 产出结构化评审裁决（blocking/advisory + 证据，依赖 model.cloud）", Partial, "hermes code-review"),
        cap!("gov.verifier", "验证器", CapabilityGroup::Governance, CapabilityTier::Exclusive, "按验收标准 + 可选校验命令判定变更是否达标（依赖 model.cloud）", Partial, "hermes verifier"),

        // ── 韧性与恢复（独家）──
        cap!("res.snapshot", "会话快照", CapabilityGroup::Resilience, CapabilityTier::Exclusive, "随时保存/恢复完整会话", Real, "nexustar session_restorer"),
        cap!("res.self_heal", "自进化闭环", CapabilityGroup::Resilience, CapabilityTier::Exclusive, "踩坑→记录→验证→升格为决策", Partial, "hermes self-improving"),

        // ── 模型与后端（双栈，独家）──
        cap!("model.cloud", "云端模型", CapabilityGroup::Model, CapabilityTier::Baseline, "接入主流云端 LLM API（已真接 DeepSeek，需 API Key 出真回复）", Real),
        cap!("model.local", "本地量化模型", CapabilityGroup::Model, CapabilityTier::Exclusive, "本地运行量化模型，离线可用", Narrative, "nexustar local_model_loader"),
        cap!("model.dual", "双模型配对", CapabilityGroup::Model, CapabilityTier::Exclusive, "规划/执行用不同模型，成本与质量兼得", Partial, "nexustar dual_model"),
        cap!("model.router", "智能路由", CapabilityGroup::Model, CapabilityTier::Enhanced, "按任务复杂度选模型", Real, "nexustar model_router"),

        // ── 记忆系统（5 层，独家）──
        cap!("mem.working", "工作记忆", CapabilityGroup::Memory, CapabilityTier::Baseline, "当前上下文窗口管理", Real),
        cap!("mem.episodic", "情景记忆", CapabilityGroup::Memory, CapabilityTier::Enhanced, "历史对话检索", Real, "hermes episodic"),
        cap!("mem.semantic", "语义记忆", CapabilityGroup::Memory, CapabilityTier::Enhanced, "提炼的知识片段", Partial, "hermes semantic"),
        cap!("mem.project", "项目记忆", CapabilityGroup::Memory, CapabilityTier::Exclusive, "项目级长期约定", Real, "nexustar project_memory"),
        cap!("mem.evolutionary", "自进化记忆", CapabilityGroup::Memory, CapabilityTier::Exclusive, "从经验中沉淀可复用策略", Partial, "hermes evolutionary"),
        cap!("mem.persistent", "跨会话记忆持久化", CapabilityGroup::Memory, CapabilityTier::Enhanced, "User/Feedback/Auto 三类记忆 JSON 落盘，跨会话沉淀", Real, "claude-mem cross-session"),
    ]
}

static REGISTRY: OnceLock<Vec<Capability>> = OnceLock::new();

/// 获取全量能力清单（进程级单例）。
pub fn registry() -> &'static [Capability] {
    REGISTRY.get_or_init(build_capabilities)
}

/// 按组聚合能力，供前端渲染能力树。
pub fn by_group() -> Vec<(CapabilityGroup, Vec<&'static Capability>)> {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<CapabilityGroup, Vec<&'static Capability>> = BTreeMap::new();
    for c in registry() {
        map.entry(c.group).or_default().push(c);
    }
    map.into_iter().collect()
}

/// 能力统计：用于直观展示「超集」规模与「真货占比」。
#[derive(Debug, Clone, Copy)]
pub struct CapabilityStats {
    pub total: usize,
    pub exclusive: usize,
    pub enhanced: usize,
    pub baseline: usize,
    /// 已实装（Real + Partial）
    pub implemented: usize,
    /// 纯叙事（Narrative）
    pub narrative: usize,
}

pub fn stats() -> CapabilityStats {
    let mut s = CapabilityStats {
        total: 0,
        exclusive: 0,
        enhanced: 0,
        baseline: 0,
        implemented: 0,
        narrative: 0,
    };
    for c in registry() {
        s.total += 1;
        match c.tier {
            CapabilityTier::Exclusive => s.exclusive += 1,
            CapabilityTier::Enhanced => s.enhanced += 1,
            CapabilityTier::Baseline => s.baseline += 1,
        }
        if c.maturity.is_implemented() {
            s.implemented += 1;
        } else {
            s.narrative += 1;
        }
    }
    s
}
