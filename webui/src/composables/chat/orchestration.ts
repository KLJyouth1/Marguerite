// P2·E 对话编排 —— 统一编排 JSON 契约（前端侧）与常量注册表。
//
// 后端唯一契约见 `core/src/gateway/orchestrate.rs`：
//   { form: single|subagent|team|expert|swarm, mode: 3×3 档位键,
//     message, sessionKey, capabilities[], agents[], swarm_on, confirmed }
// 本文件是前端镜像：类型 + 展示层注册表（角色集 / 八大专家团 / 能力分组）。
// P3 会把 `CAPABILITY_GROUPS` 统一到单一事实源（`capabilities_groups` RPC），
// 在此之前前端以本文件常量作为展示层唯一来源。
import type { IconName } from '@/utils/icons'

/** 编排形态（与 `OrchestrateForm::as_key` 对齐）。 */
export type OrchestrationForm = 'single' | 'subagent' | 'team' | 'expert' | 'swarm'

/** 一条编排选择（ChatComposer 收集 → ChatView 提交）。`form=single` 表示未编排（原 chat.send 路径）。 */
export interface OrchestrationState {
  form: OrchestrationForm
  /** 子智能体指派（role id：orchestrator/researcher/coder/sandbox/browser/guard）。 */
  agents: string[]
  /**
   * 已选团队组合 / 专家团 id（可多选；team.compositions.list 的 name，或 EXPERT_TEAMS 的 id）。
   * 多选时后端把这些团队的名册合并、按角色去重后执行。
   */
  teams: string[]
  /** 蜂群开关与并行规模。 */
  swarmOn: boolean
  swarmSize: number
  /** 3×3 档位键（USER_MODE_META 的 key，缺省 standard_doit）。 */
  modeKey: string
  /** 能力勾选（CAPABILITY_GROUPS 的 item.id 合并结果）。 */
  capabilities: string[]
  /** 高自主形态的显式授权（后端权限门消费）。 */
  confirmed: boolean
}

/** 子智能体角色集（与 `orchestrate.rs::role_prompt_for` / gui App.tsx 同一角色集）。 */
export const ORCHESTRATION_ROLES: ReadonlyArray<{ id: string; label: string; description: string }> = [
  { id: 'orchestrator', label: '调度者', description: '任务分解、协调子代理并综合结论' },
  { id: 'researcher', label: '研究员', description: '检索、核证与综述，输出证据链' },
  { id: 'coder', label: '工程师', description: '设计与实现，输出可编译、可验证的代码' },
  { id: 'sandbox', label: '沙箱执行员', description: '隔离验证与回测，输出可复现的实验证据' },
  { id: 'browser', label: '浏览器代理', description: '打开页面、收集界面事实与截图证据' },
  { id: 'guard', label: '安全守卫', description: '审计风险、红线与权限，输出合规报告' },
]

/** 八大专家团目录（镜像 `workbench/backend/marguerite_workbench/expert_teams.py`）。 */
export const EXPERT_TEAMS: ReadonlyArray<{ id: string; label: string }> = [
  { id: 'deck_craft', label: 'PPT 智造专家团' },
  { id: 'fullstack_craft', label: '全栈应用工程专家团' },
  { id: 'strategy_council', label: '战略智囊专家团' },
  { id: 'data_insight', label: '数据智能专家团' },
  { id: 'legal_guard', label: '法务合规专家团' },
  { id: 'invest_lab', label: '投研量化专家团' },
  { id: 'game_forge', label: '游戏开发专家团' },
  { id: 'product_craft', label: '产品设计专家团' },
]

export interface CapabilityItem {
  id: string
  label: string
}
export interface CapabilityGroup {
  id: string
  label: string
  icon: IconName
  items: CapabilityItem[]
}

/**
 * 能力分组合并（展示层精简镜像）。
 *
 * 单一事实源 = 后端 `capabilities.groups` RPC（`core/domain/capabilities.rs::by_group`，
 * 12 组全量注册表，id 为 `core.chat` / `orch.swarm` 命名空间格式）——能力中心
 * `CapabilitiesHubView` 与舰队页探测直接消费全量注册表；本常量面向对话编排输入框，
 * 是**按工具域合并且面向用户高频勾选的精简投影**（5 组 25 项），避免把几百项全量
 * 注册表塞进 320px 面板。二者通过同一注册表保持同源语义：勾选即申请对应 trust tier，
 * 提交时扁平化为 `capabilities: string[]` 交给 orchestrate.run（后端权限门消费）。
 */
export const CAPABILITY_GROUPS: CapabilityGroup[] = [
  {
    id: 'planning',
    label: '规划编排',
    icon: 'listChecks',
    items: [
      { id: 'analyze', label: '分析任务' },
      { id: 'create', label: '创建方案' },
      { id: 'run', label: '执行方案' },
      { id: 'revise', label: '修订方案' },
      { id: 'replan', label: '重新规划' },
      { id: 'checklist', label: '检查清单' },
      { id: 'run.advance', label: '推进执行' },
      { id: 'parallel.run', label: '并行执行' },
    ],
  },
  {
    id: 'research',
    label: '研究检索',
    icon: 'search',
    items: [
      { id: 'web_search', label: '联网搜索' },
      { id: 'web_fetch', label: '页面抓取' },
      { id: 'knowledge', label: '知识库' },
      { id: 'memory_recall', label: '记忆召回' },
    ],
  },
  {
    id: 'execution',
    label: '执行沙箱',
    icon: 'terminal',
    items: [
      { id: 'shell', label: '命令行' },
      { id: 'code_run', label: '代码执行' },
      { id: 'file_ops', label: '文件读写' },
      { id: 'sandbox', label: '沙箱隔离' },
    ],
  },
  {
    id: 'browser',
    label: '浏览器',
    icon: 'monitor',
    items: [
      { id: 'browser_open', label: '打开页面' },
      { id: 'browser_scrape', label: '网页取证' },
      { id: 'screenshot', label: '截图验证' },
    ],
  },
  {
    id: 'guard',
    label: '安全审计',
    icon: 'shield',
    items: [
      { id: 'scan', label: '能力扫描' },
      { id: 'moderation', label: '内容审核' },
      { id: 'audit', label: '审计留存' },
    ],
  },
]

/** 默认编排状态：`single`（未编排）→ 原 chat.send 路径，零额外开销。 */
export function createDefaultOrchestration(): OrchestrationState {
  return {
    form: 'single',
    agents: [],
    teams: [],
    swarmOn: false,
    swarmSize: 3,
    modeKey: 'standard_doit',
    capabilities: [],
    confirmed: false,
  }
}

/**
 * 本轮是否需要把「调度简报」上行给后端。
 *
 * 只有**真的选了分工**（非 single 形态 / 子智能体 / 团队 / 蜂群 / 能力勾选）才需要。
 * 全默认时保持原 `chat.send` 路径：后端那段「先出调度计划」的纪律会注入每一条消息
 * （连「你好」也跑一遍），既白烧 token，又让模型误以为未勾选的能力不可用 ——
 * 与 `createDefaultOrchestration` 的「零额外开销」承诺直接冲突。
 */
export function shouldSendDispatchBrief(state: OrchestrationState): boolean {
  return state.form !== 'single'
    || state.agents.length > 0
    || state.teams.length > 0
    || state.swarmOn
    || state.capabilities.length > 0
}

/* ── 编排选择的本地持久化 ──────────────────────────────────────────────
 *
 * 缺陷（真机反馈）：编排面板的形态 / 蜂群开关 / 子智能体 / 团队 / 能力勾选
 * 全部只存在 `ChatView` 的 `orchestration` ref 里，每次进入对话页都被
 * `createDefaultOrchestration()` 重置 —— 用户口径是「很多东西每次打开都要
 * 我重新选择」。
 *
 * 这些是**用户偏好**而非会话状态：换会话、重启应用都应保留。
 * 不持久化 `modeKey`（它是服务端全局 3×3 档位的镜像，由 `mode.get` 回填）
 * 与 `confirmed`（一次性授权，不能跨回合复用）。
 */
const ORCHESTRATION_STORAGE_KEY = 'marguerite.chat.orchestration.v1'

/** 允许的蜂群并行规模（与 ChatSwarmPopover 的档位按钮一致）。 */
const SWARM_SIZES: ReadonlyArray<number> = [2, 3, 4, 6, 8]
const ORCHESTRATION_FORMS: ReadonlyArray<OrchestrationForm> = [
  'single',
  'subagent',
  'team',
  'expert',
  'swarm',
]
const ROLE_IDS = new Set(ORCHESTRATION_ROLES.map((role) => role.id))
const CAPABILITY_IDS = new Set(
  CAPABILITY_GROUPS.flatMap((group) => group.items.map((item) => item.id)),
)

function readStringArray(value: unknown, allowed: ReadonlySet<string>): string[] {
  if (!Array.isArray(value)) return []
  return [...new Set(
    value.filter((item): item is string => typeof item === 'string' && allowed.has(item)),
  )]
}

/** 读取上次的编排选择；任何脏数据都退回默认值（绝不把非法形态送进后端）。 */
export function loadStoredOrchestration(): OrchestrationState {
  const fallback = createDefaultOrchestration()
  let raw: string | null = null
  try {
    raw = globalThis.localStorage?.getItem(ORCHESTRATION_STORAGE_KEY) ?? null
  } catch {
    return fallback
  }
  if (!raw) return fallback

  // 旧版本（≤ v1）把单个团队存成 `team` 字符串；解析时用宽类型读回再迁移。
  type StoredOrchestration = Partial<OrchestrationState> & { team?: unknown }
  let saved: StoredOrchestration | null = null
  try {
    saved = JSON.parse(raw) as StoredOrchestration | null
  } catch {
    return fallback
  }
  if (!saved || typeof saved !== 'object') return fallback

  const swarmOn = saved.swarmOn === true
  const form = ORCHESTRATION_FORMS.includes(saved.form as OrchestrationForm)
    ? (saved.form as OrchestrationForm)
    : fallback.form
  const swarmSize = SWARM_SIZES.includes(Number(saved.swarmSize))
    ? Number(saved.swarmSize)
    : fallback.swarmSize
  // 旧版本只存单个 `team`：迁移为数组；新版本保存 `teams`。
  const legacyTeams = Array.isArray(saved.teams)
    ? saved.teams.filter((item): item is string => typeof item === 'string' && item.trim() !== '')
    : []
  const legacySingle = typeof saved.team === 'string' && saved.team.trim() ? [saved.team.trim()] : []
  const teams = [...new Set([...legacyTeams, ...legacySingle])].slice(0, 8)

  return {
    ...fallback,
    // 开关是蜂群形态的真相：两者必须一致，否则界面显示"已开启"但发送走单代理。
    form: swarmOn ? 'swarm' : (form === 'swarm' ? 'single' : form),
    swarmOn,
    swarmSize,
    agents: readStringArray(saved.agents, ROLE_IDS),
    capabilities: readStringArray(saved.capabilities, CAPABILITY_IDS),
    teams,
  }
}

/** 写入编排选择（失败静默：隐私模式 / 配额满不应影响发送）。 */
export function persistOrchestration(state: Readonly<OrchestrationState>): void {
  try {
    globalThis.localStorage?.setItem(ORCHESTRATION_STORAGE_KEY, JSON.stringify({
      form: state.form,
      agents: state.agents,
      teams: state.teams,
      swarmOn: state.swarmOn,
      swarmSize: state.swarmSize,
      capabilities: state.capabilities,
    }))
  } catch {
    // 存储不可用时保持默认，不阻断任何交互。
  }
}