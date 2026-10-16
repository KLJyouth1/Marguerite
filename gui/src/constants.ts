import { ModeOption, SubAgent, Workspace } from "./types"

// 唯一合法模式：3 档位（极简 / 标准 / 极致）x 3 工作模式（Ask / Plan / DoIt）= 9。
// 已移除 bloom 等旧模式，其能力按语义重映射进这三档。
export const MODES: ModeOption[] = [
  // 极简 minimal
  { id: "minimal_ask", intensity: "minimal", working: "ask", label: "极简·咨询", short: "咨询", icon: "💬", tagline: "一句话结论 + 关键建议", accent: "var(--status-success)" },
  { id: "minimal_plan", intensity: "minimal", working: "plan", label: "极简·方案", short: "方案", icon: "🗂", tagline: "简短要点清单（3-5 条）", accent: "var(--status-success)" },
  { id: "minimal_doit", intensity: "minimal", working: "doit", label: "极简·执行", short: "执行", icon: "🚀", tagline: "最小可行自动化，仅必要工具", accent: "var(--status-success)" },
  // 标准 standard
  { id: "standard_ask", intensity: "standard", working: "ask", label: "标准·咨询", short: "咨询", icon: "💬", tagline: "标准结构化分析，含依据与边界", accent: "var(--accent-primary)" },
  { id: "standard_plan", intensity: "standard", working: "plan", label: "标准·方案", short: "方案", icon: "🗂", tagline: "完整分步方案，含里程碑", accent: "var(--accent-primary)" },
  { id: "standard_doit", intensity: "standard", working: "doit", label: "标准·执行", short: "执行", icon: "🚀", tagline: "带校验的执行，结果可追溯", accent: "var(--accent-primary)" },
  // 极致 extreme
  { id: "extreme_ask", intensity: "extreme", working: "ask", label: "极致·咨询", short: "咨询", icon: "💬", tagline: "深度根因分析，多视角交叉", accent: "#A855F7" },
  { id: "extreme_plan", intensity: "extreme", working: "plan", label: "极致·方案", short: "方案", icon: "🗂", tagline: "全面实施蓝图，含风险预案", accent: "#A855F7" },
  { id: "extreme_doit", intensity: "extreme", working: "doit", label: "极致·执行", short: "执行", icon: "🚀", tagline: "全流程自主执行，全工具面", accent: "#A855F7" },
];

export const INITIAL_AGENTS: SubAgent[] = [
  {
    id: "orchestrator",
    name: "编排器",
    role: "Orchestrator · 主代理",
    color: "var(--accent-primary)",
    status: "idle",
    progressPct: 0,
    tools: ["plan_task", "run_turn", "recall"],
  },
  {
    id: "coder",
    name: "代码代理",
    role: "Coder Agent · 工程实现",
    color: "#6366F1",
    status: "idle",
    progressPct: 0,
    tools: ["read_file", "edit", "grep"],
  },
  {
    id: "sandbox",
    name: "终端沙箱",
    role: "Sandbox · Shell / 进程",
    color: "var(--status-success)",
    status: "idle",
    progressPct: 0,
    tools: ["pty_exec", "fs_ops", "run_cmd"],
  },
  {
    id: "browser",
    name: "浏览器代理",
    role: "Browser Agent · 网页交互",
    color: "var(--accent-secondary)",
    status: "idle",
    progressPct: 0,
    tools: ["navigate", "click", "snapshot"],
  },
  {
    id: "researcher",
    name: "研究代理",
    role: "Researcher · 知识检索",
    color: "#A855F7",
    status: "idle",
    progressPct: 0,
    tools: ["web_search", "fetch", "kb_query"],
  },
  {
    id: "guard",
    name: "护栏校验",
    role: "Guard · 安全 / 语法 / 修复",
    color: "var(--status-warning)",
    status: "idle",
    progressPct: 0,
    tools: ["guard_check", "repair_code"],
  },
];

// 统一分组导航（路线图 A2）：核心（主屏 ≤9 项）/ 治理 / 实验室（默认折叠）。
// 玩具功能（德州扑克/鲸鱼娘/视觉取证/桌宠/关系图谱等）不物理删除，仅折叠进实验室。
export const NAV_SECTIONS: {
  id: string;
  label: string;
  defaultCollapsed?: boolean;
  items: { id: string; label: string; icon: string; badge: number | null }[];
}[] = [
  {
    id: "core",
    label: "核心",
    items: [
      { id: "new_task", label: "对话工作台", icon: "✦", badge: null },
      { id: "sessions", label: "会话管理", icon: "💬", badge: null },
      { id: "sandbox", label: "沙箱执行", icon: "▶", badge: null },
      { id: "files", label: "文件", icon: "▧", badge: null },
      { id: "kb", label: "知识库", icon: "📚", badge: null },
      { id: "automation", label: "自动化", icon: "◐", badge: null },
      { id: "computer_use", label: "电脑操控", icon: "🖥", badge: null },
      { id: "fleet", label: "模型舰队", icon: "🚀", badge: null },
      { id: "workspace", label: "工作区", icon: "▤", badge: null },
    ],
  },
  {
    id: "governance",
    label: "治理",
    items: [
      { id: "doctor", label: "系统诊断", icon: "🩺", badge: null },
      { id: "skills", label: "技能管理", icon: "⚡", badge: null },
      { id: "mcp", label: "MCP 服务器", icon: "🔌", badge: null },
      { id: "capabilities", label: "能力总览", icon: "▦", badge: null },
      { id: "templates", label: "模板库", icon: "▦", badge: null },
      { id: "routing", label: "模型路由", icon: "⇄", badge: null },
      { id: "plugins", label: "插件市场", icon: "⌬", badge: null },
      { id: "usage", label: "消费统计", icon: "▥", badge: null },
    ],
  },
  {
    id: "lab",
    label: "实验室",
    defaultCollapsed: true,
    items: [
      { id: "evolve", label: "自进化", icon: "🧬", badge: null },
      { id: "office_assistant", label: "办公助理", icon: "☺", badge: null },
      { id: "modlens", label: "视觉取证", icon: "◉", badge: null },
      { id: "poker", label: "德州扑克", icon: "◆", badge: null },
      { id: "graph", label: "关系图谱", icon: "⋈", badge: null },
      { id: "pet", label: "鲸鱼娘", icon: "◍", badge: null },
    ],
  },
];

export const INITIAL_WORKSPACES: Workspace[] = [];

