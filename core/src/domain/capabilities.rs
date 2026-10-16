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
        cap!("core.diff", "结构化差异预览", CapabilityGroup::CoreLoop, CapabilityTier::Enhanced, "行级 diff 预览（diff_lines LCS）+ apply_patch 统一 diff 接受（hunk 应用）", Real),
        cap!("core.apply", "安全应用改动", CapabilityGroup::CoreLoop, CapabilityTier::Enhanced, "改动经护栏校验后才落盘，等价但不止于 Cursor 的 apply", Real),

        // ── 编排与协作（独家，超越项）──
        cap!("orch.dual_leader", "MAX 双带队", CapabilityGroup::Orchestration, CapabilityTier::Exclusive, "规划(Flash)与执行(Pro)双轨分离，顺序两阶段执行（dual_leader_specs/AgentMode::DualLeader，planner 先 executor 后）", Real, "nexustar max_orchestrator"),
        cap!("orch.expert_team", "专家团队", CapabilityGroup::Orchestration, CapabilityTier::Exclusive, "角色专精子 Agent 按 spawn 模型协作，真实并行执行 + lead 综合（agent::team::Team 原语）", Real, "nexustar expert_team_orchestrator"),
        cap!("orch.cluster", "集群调度", CapabilityGroup::Orchestration, CapabilityTier::Exclusive, "fan-out 多 Worker 并行 spawn + 聚合（cluster_specs/dispatch Cluster 分支；非完整 allocator）", Real, "nexustar cluster/allocator"),
        cap!("orch.swarm", "swarm 有向无环图", CapabilityGroup::Orchestration, CapabilityTier::Exclusive, "DAG 依赖驱动并行执行（dispatch_bloom_swarm + orchestration/dag.rs，含拓扑/环检测单测）+ Bloom 蜂群计划生成器", Real),

        // ── 工具集（46 工具原生实现，统一 ToolExecutor）──
        cap!("tool.exec", "命令执行", CapabilityGroup::Tooling, CapabilityTier::Baseline, "在 OS 沙箱内执行 shell 命令", Real, "nexustar tools/exec"),
        cap!("tool.file", "文件读写", CapabilityGroup::Tooling, CapabilityTier::Baseline, "读/写/搜索/批量重命名", Real, "nexustar tools/file"),
        cap!("tool.terminal", "终端操控", CapabilityGroup::Tooling, CapabilityTier::Enhanced, "原生终端会话管理：行模式命令执行器（ANSI 安全剥离）+ Windows ConPTY 交互式（彩色/TUI/光标，vt100 渲染）；行模式跨平台，PTY 限 Windows 10 1809+", Real, "nexustar tools/terminal"),
        cap!("tool.browser", "浏览器自动化", CapabilityGroup::Tooling, CapabilityTier::Enhanced, "页面导航/截图/抽取（CdpSession：CDP 客户端 connect/navigate/evaluate/screenshot_png/a11y_nodes，需本机 Chromium 远程调试端）", Real, "nexustar tools/browser"),
        cap!("tool.git", "版本控制", CapabilityGroup::Tooling, CapabilityTier::Baseline, "commit/PR/blame/二分调试（真实 git CLI 桥，白名单+工作区守卫）", Real),
        cap!("tool.fs_watch", "文件监听", CapabilityGroup::Tooling, CapabilityTier::Enhanced, "工作区 notify 递归监听：新增/修改/删除事件经事件总线发布（workspace_watcher，忽略 git 目录）", Real, "nexustar notify"),
        cap!("tool.rollback", "快照回滚", CapabilityGroup::Tooling, CapabilityTier::Exclusive, "增量 tar 快照 + sha256，随时回退", Real, "nexustar recovery"),
        cap!("tool.mcp_bridge", "MCP 工具桥", CapabilityGroup::Tooling, CapabilityTier::Enhanced, "将任意 MCP server 工具暴露为原生工具（McpClient HTTP JSON-RPC + ToolRegistry::add_mcp_client + describe_mcp_tools + 缓存执行 + 编排接线）", Real, "nexustar mcp"),
        cap!("tool.baota", "宝塔面板运维工具箱", CapabilityGroup::Tooling, CapabilityTier::Enhanced, "宝塔 Linux 面板 7.x/8.x 的站点/数据库/SSL/文件/计划任务管理：API key 优先、面板 CA 证书兜底；批量建站、一键下发 SSL、端口放行、容器/运行时管理、日志抽取、备份迁移", Real, "marguerite tools/baota"),
        cap!("tool.desktop_control", "桌面级 Computer Use（键鼠屏幕）", CapabilityGroup::Tooling, CapabilityTier::Exclusive, "Windows user32/SendInput 原生系统调用：mouse_move(贝塞尔拟人化轨迹)/mouse_click/mouse_scroll/mouse_pos/screen_size/key_press/type_text；支持多显示器，可被 FPS 助手/跨应用 RPA/录放回放直接消费", Real, "marguerite tools/system_control"),
        cap!("tool.fps_assistant", "FPS 第一人称射击操作助手", CapabilityGroup::Automation, CapabilityTier::Exclusive, "基于 Computer Use 封装的射击操作代理（computer_use_loop 重试/退避 + 贝塞尔 humanize 轨迹 + system_control 键鼠 + skills/fps 技能；Labs 显式开启）", Real, "marguerite skills/fps"),
        cap!("auto.uia", "UIA 跨应用桥", CapabilityGroup::Automation, CapabilityTier::Exclusive, "已实装两条真实路径：① WPS Presentation（desktop_automation/com.rs：cfg(windows) COM/OLE，CoCreateInstance(Kwpp.Application)/IDispatch/VARIANT 生成 .pptx）；② 通用跨应用 UIA（desktop_automation/windows_uia.rs 真实 COM 后端 + uia_driver.rs 占 AppKind::Generic：snapshot 读前台窗口控件树、click 按 ref_id/name 走 Invoke/Toggle/Select/SetFocus）。仍未接通：非前台窗口、Browser/Jianying/ThreeD 专属驱动未注册；键鼠级兜底另走 tool.desktop_control，且 UIA 效果取决于目标程序是否暴露控件树", Partial),

        // ── 知识与检索（独家）──
        cap!("kb.rag", "RAG 检索", CapabilityGroup::Knowledge, CapabilityTier::Exclusive, "本地向量检索 + 重排，零外部依赖", Real, "nexustar kb_rag"),
        cap!("kb.graph", "知识图谱", CapabilityGroup::Knowledge, CapabilityTier::Exclusive, "实体关系图谱，支撑跨文件推理", Real, "nexustar knowledge_graph"),
        cap!("kb.denoiser", "去噪层", CapabilityGroup::Knowledge, CapabilityTier::Exclusive, "实体关系去噪 + Jaccard 去重（本会话实装，已补登记）", Real),
        cap!("kb.integrity", "完整性审计", CapabilityGroup::Knowledge, CapabilityTier::Exclusive, "知识入库完整性审计（本会话实装，已补登记）", Real),
        cap!("kb.ima", "私有知识库", CapabilityGroup::Knowledge, CapabilityTier::Exclusive, "本地 RAG（kb ingest/search BM25+向量）+ cross_project 跨项目记忆 + 画像导入（memory.import.*）可接用户私有文档（CSV/MD/文本）；批量导入 Wizard 后续补", Real),

        // ── 安全与沙箱（独家，OS 级）──
        cap!("sec.os_sandbox", "OS 级沙箱", CapabilityGroup::Security, CapabilityTier::Exclusive, "Landlock(Linux)/Seatbelt(macOS) 内核级隔离；Windows 显式 Unsupported，命令级 sandbox/ 兜底", Partial, "TUI 工作台 sandbox"),
        cap!("sec.malware", "恶意代码扫描", CapabilityGroup::Security, CapabilityTier::Exclusive, "YARA/正则扫描生成与执行的代码", Real, "nexustar malicious_code_scanner"),
        cap!("sec.ssrf", "SSRF 守卫", CapabilityGroup::Security, CapabilityTier::Exclusive, "拦截内网/元数据端点请求", Real, "nexustar ssrf_guard"),
        cap!("sec.guardrail", "操作护栏", CapabilityGroup::Security, CapabilityTier::Enhanced, "危险操作需审批门", Real, "外部参考 exec 审批门"),

        // ── 自动化与 RPA（独家）──
        cap!("auto.record", "操作录制", CapabilityGroup::Automation, CapabilityTier::Exclusive, "未实装：录制用户键鼠/窗口操作序列暂无任何实现（desktop_automation::Recorder 只落桌面动作的审计快照供 desktop.snapshots 回放审计，不是用户操作录制）", Narrative, "nexustar automations"),
        cap!("auto.replay", "回放与编排", CapabilityGroup::Automation, CapabilityTier::Exclusive, "未实装：按条件/循环回放录制序列暂无实现（projection::replay 是事件溯源 fold，与本能力无关）", Narrative, "nexustar control_tool"),
        cap!("auto.fix", "自动修复", CapabilityGroup::Automation, CapabilityTier::Exclusive, "基于 LLM 后端对错误/测试失败生成修复并安全落盘（依赖 model.cloud 运行期出真回复）", Partial, "hermes self-healing"),
        // 注：auto.uia 使用上方「UIA 跨应用桥」条目（Partial：仅 WPS COM 为真），不得重复登记同一 ID。
        // auto.record / auto.replay 为纯叙事（零实现），如实降级，不得回填 Real。

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
        cap!("res.self_heal", "自进化闭环", CapabilityGroup::Resilience, CapabilityTier::Exclusive, "踩坑→记录→验证→升格为决策（harness/distill.rs run_self_evolve + EvolutionGate + memory SelfEvolution/ExperienceLedger）", Real, "hermes self-improving"),
        cap!("res.meta_agent", "元智能体自迭代闭环", CapabilityGroup::Resilience, CapabilityTier::Exclusive, "监控(行为台账抽取失败信号)→根因裁决(规划/工具参数/工具脚本/能力缺口)→白名单改进规划(6 类目标，结构上不含核心源码)→沙箱回放四闸裁决(无回退/成功率/token 涨幅/时延涨幅)→版本快照+灰度+回滚；默认 auto_apply=false，提案须人工批准（harness/meta.rs + selfevolve.* RPC）", Real, "AI Agent 自迭代方案五层架构"),

        // ── 模型与后端（双栈，独家）──
        cap!("model.cloud", "云端模型", CapabilityGroup::Model, CapabilityTier::Baseline, "接入主流云端 LLM API（已真接 DeepSeek，需 API Key 出真回复）", Real),
        cap!("model.local", "本地量化模型", CapabilityGroup::Model, CapabilityTier::Exclusive, "本地运行量化模型，离线可用", Narrative, "nexustar local_model_loader"),
        cap!("model.dual", "双模型配对", CapabilityGroup::Model, CapabilityTier::Exclusive, "主模型 + Bloom copilot 副模型配对（use_copilot/copilot_config），规划/执行可不同模型", Real, "nexustar dual_model"),
        cap!("model.router", "智能路由", CapabilityGroup::Model, CapabilityTier::Enhanced, "按任务复杂度选模型", Real, "nexustar model_router"),

        // ── 记忆系统（5 层，独家）──
        cap!("mem.working", "工作记忆", CapabilityGroup::Memory, CapabilityTier::Baseline, "当前上下文窗口管理", Real),
        cap!("mem.episodic", "情景记忆", CapabilityGroup::Memory, CapabilityTier::Enhanced, "历史对话检索", Real, "hermes episodic"),
        cap!("mem.semantic", "语义记忆", CapabilityGroup::Memory, CapabilityTier::Enhanced, "提炼的知识片段（embedding 向量 + hybrid_retrieve 词法×向量加权检索 + retrieve，含余弦/重排单测）", Real, "hermes semantic"),
        cap!("mem.project", "项目记忆", CapabilityGroup::Memory, CapabilityTier::Exclusive, "项目级长期约定", Real, "nexustar project_memory"),
        cap!("mem.evolutionary", "自进化记忆", CapabilityGroup::Memory, CapabilityTier::Exclusive, "从经验中沉淀可复用策略", Partial, "hermes evolutionary"),
        cap!("mem.persistent", "跨会话记忆持久化", CapabilityGroup::Memory, CapabilityTier::Enhanced, "User/Feedback/Auto 三类记忆 JSON 落盘，跨会话沉淀", Real, "claude-mem cross-session"),

        // ── DSH 二创融合能力（插件清单 SSOT 对账：8 份 manifests × capabilities 合计 35 条，NFR1 强制登记）──
        // | 来源插件               | 条数 |
        // |---|---|
        // | dsh-harness-core       |  4   |
        // | dsh-modlens            |  4   |
        // | dsh-market             |  4   |
        // | dsh-better-sidebar     |  4   |
        // | dsh-router-standard    |  3   |
        // | dsh-usage              |  6   |
        // | dsh-pet-whale          |  5   |
        // | dsh-minigames          |  5   |
        // | 合计                   | 35   |

        // ··· harness-core 锚点（4）
        cap!("dsh.runtime.version",  "DSH 版本查询",        CapabilityGroup::Governance, CapabilityTier::Enhanced, "查询 Marguerite 集成的 DSH 二创版本号（对应 upstream 0.1.0-rc.7）", Real, "DeepSeek Harness core"),
        cap!("dsh.runtime.ping",     "DSH 运行时探测",      CapabilityGroup::Governance, CapabilityTier::Enhanced, "探测 DSH Bridge 是否就绪及 bundle 加载状态", Real, "DeepSeek Harness core"),
        cap!("dsh.runtime.list_plugins","DSH 插件清单枚举", CapabilityGroup::Governance, CapabilityTier::Enhanced, "枚举已加载的 manifest 化 DSH 插件（topo_order 排序）", Real, "DeepSeek Harness core"),
        cap!("dsh.bundle.topo_order","DSH Bundle 拓扑加载", CapabilityGroup::Orchestration, CapabilityTier::Enhanced, "依据 dependencies 做 Kahn 拓扑排序（registry_topological_order：入度队列+环检测），弱依赖先注册", Real, "DeepSeek Harness core"),

        // ··· Modlens 视觉（4）
        cap!("dsh.vision.img2txt",     "Modlens 图文问答",    CapabilityGroup::Multimodal, CapabilityTier::Enhanced, "图片→自然语言描述；复用 Marguerite vision/modlens，PreToolUse 钩子自动注入", Real, "DSH plugin: modlens (image-to-text)"),
        cap!("dsh.vision.ocr",         "Modlens 文字 OCR",    CapabilityGroup::Multimodal, CapabilityTier::Enhanced, "图片文字识别（中英混排、版面顺序保留），输出纯文本", Real, "DSH plugin: modlens (OCR)"),
        cap!("dsh.vision.layout",      "Modlens 版面分析",    CapabilityGroup::Multimodal, CapabilityTier::Enhanced, "图片版面分割：VisionDocument.layout(columns/regions)，analyze 产出、extract_json 解析供消费", Real, "DSH plugin: modlens (layout)"),
        cap!("dsh.vision.extract_json","Modlens JSON 抽取",   CapabilityGroup::Multimodal, CapabilityTier::Exclusive,"基于版面 + OCR 产出结构化 JSON：键值表、列表、表格行；偏好模型 opus", Real, "DSH plugin: modlens (extract JSON)"),

        // ··· Market 插件市场（4）
        cap!("dsh.market.list",    "Market 插件浏览",    CapabilityGroup::Tooling, CapabilityTier::Enhanced, "本地 manifest 目录浏览已安装 DSH 插件及其版本、capabilities", Real, "DSH plugin: market (list)"),
        cap!("dsh.market.search",  "Market 关键词搜索",  CapabilityGroup::Tooling, CapabilityTier::Enhanced, "在本地 manifests 与内置 catalog.json 中按关键词模糊搜索", Real, "DSH plugin: market (search)"),
        cap!("dsh.market.install", "Market 插件安装",    CapabilityGroup::Tooling, CapabilityTier::Enhanced, "按名称安装/校验 manifest（纯离线，运行期注册随网关重启生效）；含 SKILL_SCAN 二次签名校验", Real, "DSH plugin: market (install)"),
        cap!("dsh.market.upgrade", "Market 插件升级",    CapabilityGroup::Tooling, CapabilityTier::Enhanced, "同插件新版本 manifest 原子替换（semver 升序校验 + tmp/rename，版本未升据实失败）", Real, "DSH plugin: market (upgrade)"),

        // ··· Better Sidebar 工作台（4）
        cap!("dsh.sidebar.list",      "Better Sidebar 列举",   CapabilityGroup::CoreLoop, CapabilityTier::Enhanced, "侧边栏三栏（文件树/浏览器/编辑器）内容列举与切换", Real, "DSH plugin: better-sidebar"),
        cap!("dsh.sidebar.pin_slot",  "Better Sidebar 固定槽", CapabilityGroup::CoreLoop, CapabilityTier::Enhanced, "按 SlotId 把某面板 pin 到侧边栏；支持 runtime 切换", Real, "DSH plugin: better-sidebar"),
        cap!("dsh.sidebar.refresh",   "Better Sidebar 刷新",   CapabilityGroup::CoreLoop, CapabilityTier::Enhanced, "刷新工作区视图：文件树重新枚举 + FS watcher merge", Real, "DSH plugin: better-sidebar"),
        cap!("dsh.sidebar.open_path", "Better Sidebar 打开路径",CapabilityGroup::CoreLoop, CapabilityTier::Enhanced, "在侧边栏聚焦并展开指定 path 的节点（/open <path>：切文件树 + sidebar_open_path 状态供渲染消费）", Real, "DSH plugin: better-sidebar"),

        // ··· Router 路由套件（3）
        cap!("dsh.router.set_mode",   "Router 切换模式",   CapabilityGroup::Orchestration, CapabilityTier::Enhanced, "在 plan/execute/weak 三个 profile 间切换 FleetRouter 的默认路由", Real, "DSH plugin: dsh-routing-suite"),
        cap!("dsh.router.query",      "Router 查询模式",   CapabilityGroup::Orchestration, CapabilityTier::Enhanced, "查询当前活动的思维模式 profile 及子代理路由详情", Real, "DSH plugin: dsh-routing-suite"),
        cap!("dsh.router.auto_route", "Router 自动路由",   CapabilityGroup::Orchestration, CapabilityTier::Exclusive,"基于意图×复杂度启发式选 plan/execute/weak profile（resolve_model：IntentType×Complexity 规则+平局确定性）", Real, "DSH plugin: dsh-routing-suite"),

        // ··· Daily Usage 消费统计（6）
        cap!("dsh.usage.query_day",   "Usage 日维度统计",  CapabilityGroup::Governance, CapabilityTier::Enhanced, "按日聚合 TOKEN 消耗（prompt/cache_write/cache_read/completion 分项）+ SQLite 持久化", Real, "DSH plugin: dsh-daily-usage"),
        cap!("dsh.usage.query_week",  "Usage 周维度统计",  CapabilityGroup::Governance, CapabilityTier::Enhanced, "最近 7 天聚合；含模型分布 breakdown", Real, "DSH plugin: dsh-daily-usage"),
        cap!("dsh.usage.query_month", "Usage 月维度统计",  CapabilityGroup::Governance, CapabilityTier::Enhanced, "月度开销累计 + 日均 + 预估月末；支持 budget ratio 计算", Real, "DSH plugin: dsh-daily-usage"),
        cap!("dsh.usage.balance",     "Usage 余额查询",    CapabilityGroup::Governance, CapabilityTier::Enhanced, "账户余额占位（需接入上游账户系统时替换实现）", Partial, "DSH plugin: dsh-daily-usage"),
        cap!("dsh.usage.heatmap",     "Usage 热力图渲染",  CapabilityGroup::Governance, CapabilityTier::Enhanced, "日 TOKEN 密度字符热力图（周×日矩阵，4 级灰度）；供 TUI Slot 直接渲染", Real, "DSH plugin: dsh-daily-usage"),
        cap!("dsh.usage.accumulate",  "Usage 累计钩子",    CapabilityGroup::Governance, CapabilityTier::Enhanced, "PostToolUse 自动累计 token 用量 + budget 阈值告警（WARN 事件）", Real, "DSH plugin: dsh-daily-usage"),

        // ··· 鲸鱼娘桌宠（5）
        cap!("dsh.pet.status", "Pet 当前状态查询",       CapabilityGroup::CoreLoop, CapabilityTier::Enhanced, "查询桌宠当前 6 态（idle/thinking/running/sleeping/eating/petted）+ ASCII 精灵", Real, "DSH plugin: dsh-harness-pet"),
        cap!("dsh.pet.pet",    "Pet 摸头互动",          CapabilityGroup::CoreLoop, CapabilityTier::Enhanced, "触发摸头动作：状态 petted +15s，affinity +2（溢出 clamp 100）", Real, "DSH plugin: dsh-harness-pet"),
        cap!("dsh.pet.feed",   "Pet 投喂小鱼干",        CapabilityGroup::CoreLoop, CapabilityTier::Enhanced, "投喂小鱼干：eating 态 +10s，affinity +5，饱食度上限避免爆饲", Real, "DSH plugin: dsh-harness-pet"),
        cap!("dsh.pet.save",   "Pet 状态持久化保存",    CapabilityGroup::Memory,   CapabilityTier::Enhanced, "保存 PetV2 (affinity, mood, last_state, last_fed_at) 到会话配置目录 JSON", Real, "DSH plugin: dsh-harness-pet"),
        cap!("dsh.pet.load",   "Pet 状态持久化加载",    CapabilityGroup::Memory,   CapabilityTier::Enhanced, "从 JSON 还原 PetV2 状态（首次启动自动创建默认 id=0,affinity=0, mood=Happy）", Real, "DSH plugin: dsh-harness-pet"),

        // ··· Mini Games 小游戏（5）
        cap!("dsh.games.list",      "Games 列表",           CapabilityGroup::CoreLoop, CapabilityTier::Baseline, "列出可用游戏（锚点 3 款：2048/TicTacToe/Snake；其余 15 款 P3 待补）", Real, "DSH plugin: mini-games"),
        cap!("dsh.games.play",      "Games 启动入口",       CapabilityGroup::CoreLoop, CapabilityTier::Baseline, "按 game_id 经 /games play <id> <动作> 运行对应游戏引擎并渲染结果（REPL/控制台）", Real, "DSH plugin: mini-games"),
        cap!("dsh.games.2048",      "2048 数字合成",        CapabilityGroup::CoreLoop, CapabilityTier::Baseline, "2048（Rust 引擎）：4×4 棋盘、方向键滑动合并、计分、结束判定；规则全量单测覆盖", Real, "DSH plugin: mini-games (2048)"),
        cap!("dsh.games.tictactoe", "井字棋 TicTacToe",     CapabilityGroup::CoreLoop, CapabilityTier::Baseline, "双人井字棋（Rust 引擎）：落子、胜负判定、悔棋一步；引擎规则全量单测覆盖", Real, "DSH plugin: mini-games (tictactoe)"),
        cap!("dsh.games.snake",     "贪吃蛇 Snake",         CapabilityGroup::CoreLoop, CapabilityTier::Baseline, "贪吃蛇（Rust 引擎）：蛇身移动、转向、食物增长、碰撞/计分；规则全量单测覆盖", Real, "DSH plugin: mini-games (snake)"),
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
    /// 真货：完整实现（Real）
    pub real: usize,
    /// 部分实装（Partial，框架/核心在但依赖运行环境）
    pub partial: usize,
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
        real: 0,
        partial: 0,
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
        match c.maturity {
            CapabilityMaturity::Real => {
                s.real += 1;
                s.implemented += 1;
            }
            CapabilityMaturity::Partial => {
                s.partial += 1;
                s.implemented += 1;
            }
            CapabilityMaturity::Narrative => s.narrative += 1,
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// 能力 ID 是机器可校验契约：必须全局唯一、稳定。
    /// （修复：auto.uia 曾重复登记「UIA 桥」纯叙事条目 → 总数虚高、自检自相矛盾。）
    #[test]
    fn test_capability_ids_unique() {
        let ids: Vec<&str> = registry().iter().map(|c| c.id.0.as_str()).collect();
        let unique: HashSet<&str> = ids.iter().copied().collect();
        assert_eq!(
            ids.len(),
            unique.len(),
            "能力清单存在重复 ID：{:?}",
            ids.iter()
                .enumerate()
                .filter(|(i, id)| ids[..*i].contains(id))
                .map(|(_, id)| *id)
                .collect::<Vec<_>>()
        );
        // auto.uia 只允许一条（Partial 详案），不得再以纯叙事重复登记
        assert_eq!(
            registry().iter().filter(|c| c.id.0 == "auto.uia").count(),
            1,
            "auto.uia 应只登记一次"
        );
    }

    /// 统计自洽：全量 = 已实装 + 纯叙事；已实装 = 真货 + 部分实装。
    #[test]
    fn test_stats_consistent() {
        let s = stats();
        assert_eq!(s.total, s.implemented + s.narrative);
        assert_eq!(s.implemented, s.real + s.partial);
        assert!(s.total > 0);
        assert!(
            s.real >= s.partial,
            "真货数不应少于部分实装数（当前清单应如实）"
        );
    }

    /// 数字漂移防线：README 中 capabilities 输出示例的
    /// 「total | implemented (real + partial) | narrative」必须与运行时统计一致。
    /// 新增/改动能力清单时**同步更新 README.md**，否则此测失败（防止宣传与实现脱节回潮）。
    #[test]
    fn test_stats_match_readme_claim() {
        let s = stats();
        assert_eq!(
            (s.total, s.implemented, s.real, s.partial, s.narrative),
            (89, 85, 78, 7, 4),
            "能力统计与 README 宣传不一致：请同步 README.md 的 capabilities 输出示例"
        );
    }

    /// 纯叙事清单当前仅剩依赖重型运行时/模型的后端大件与已核实的零实现项：
    /// 音频理解、本地量化模型、操作录制（auto.record）、回放与编排（auto.replay）。
    /// 若未来新增纯叙事项请同步此处；这也防止「能力与实现脱节」悄然回潮。
    #[test]
    fn test_narrative_ids_are_honest_minimal() {
        let mut narrative: Vec<String> = registry()
            .iter()
            .filter(|c| c.maturity == CapabilityMaturity::Narrative)
            .map(|c| c.id.0.clone())
            .collect();
        narrative.sort();
        assert_eq!(
            narrative,
            vec![
                "auto.record".to_string(),
                "auto.replay".to_string(),
                "model.local".to_string(),
                "vis.audio".to_string(),
            ],
            "纯叙事能力应为真实立项清单（音频理解/本地量化模型/操作录制/回放编排）"
        );
    }

    /// `auto.uia` 只许宣称真实存在的东西。
    /// （修复：曾登记为 Real，暗示任意应用元素级操作可用——实际只有 WPS。）
    /// （2026-10-15：通用跨应用 UIA 已真实接通，故断言从「必须说未接通」反转为
    /// 「必须点名两条真实路径 + 必须继续如实列出残余缺口」。）
    #[test]
    fn auto_uia_claims_only_the_real_paths() {
        let cap = registry()
            .iter()
            .find(|c| c.id.0 == "auto.uia")
            .expect("auto.uia 应登记一次");
        assert_eq!(
            cap.maturity,
            CapabilityMaturity::Partial,
            "auto.uia 依赖运行环境（桌面会话/目标程序是否暴露 UIA），必须是 Partial"
        );
        assert!(
            cap.description.contains("com.rs") && cap.description.contains("WPS"),
            "auto.uia 描述必须点名 WPS COM 真实路径（com.rs）: {}",
            cap.description
        );
        assert!(
            cap.description.contains("windows_uia.rs") && cap.description.contains("uia_driver.rs"),
            "auto.uia 描述必须点名通用 UIA 真实路径（windows_uia.rs / uia_driver.rs）: {}",
            cap.description
        );
        assert!(
            cap.description.contains("仍未接通"),
            "auto.uia 描述必须继续如实列出残余未接通项，不得只报喜: {}",
            cap.description
        );
        assert!(
            !cap.description.contains("Win32_UI_Accessibility"),
            "auto.uia 描述不得再出现「windows crate 未启用 Win32_UI_Accessibility」这类过时结论: {}",
            cap.description
        );
    }
}
