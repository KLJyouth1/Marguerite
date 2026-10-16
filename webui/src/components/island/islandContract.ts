/**
 * 岛屿工作空间契约（单一事实源）—— 「智能体运行概念 → 岛上建筑」的唯一映射。
 *
 * 设计铁律（沿用 pet 模块的成功范式）：
 * - **诚实可视化**：每类建筑都必须由一个真实运行时来源驱动（ISLAND_KIND_META.source），
 *   没有数据就不生成建筑；岛屿的装饰（地形/水/树/石）只是布景，绝不冒充运行状态。
 * - **唯一映射**：`kindForSource` 与 `ISLAND_KIND_META[kind].source` 必须互为逆映射
 *   （由 islandContract.test.ts 守卫），任何新增实体类型都必须同步两侧。
 * - **可测边界**：本文件是纯数据 + 纯函数，不 import three、不触碰 DOM，可在 node 环境单测。
 *
 * 概念对照（《我们的岛》式建筑语义）：
 *   core  中央主堡  ← 当前会话（runStatus / 编排策略）
 *   workshop 工坊群 ← 每一次工具调用（flavor 决定工位形态）
 *   outpost 哨所    ← 子代理（subagent_completion）
 *   depot  仓库码头 ← 交付产物（artifact）
 *   beacon 灯塔     ← 模型路由 / 融合编排策略（router_decision、ensemble_progress）
 *   pier   栈桥     ← 计划推进进度（plan_run / progress：一步一块铺板）
 *   athenaeum 记忆塔 ← 记忆召回（memory_recall：联想记忆网络的一次检索）
 */

/** 岛上建筑类型（= 一种智能体运行概念） */
export type IslandEntityKind =
  | 'core'
  | 'workshop'
  | 'outpost'
  | 'depot'
  | 'beacon'
  | 'pier'
  | 'athenaeum'
  | 'construction'

/** 建筑/道路的活动状态（驱动灯光、脉动与配色） */
export type IslandActivityStatus =
  | 'idle'
  | 'queued'
  | 'active'
  | 'success'
  | 'error'
  | 'waiting'

/** 实体数据来源（与 RPC 事件一一对应；construction 由施工地块系统/时间轴驱动，非 RPC） */
export type IslandSource =
  | 'session'
  | 'tool'
  | 'subagent'
  | 'artifact'
  | 'router'
  | 'plan'
  | 'memory'
  | 'construction'

/** 工坊形态：由工具名派生，让同类工位有可辨识的轮廓 */
export type IslandWorkshopFlavor = 'mill' | 'forge' | 'library' | 'observatory'

/** 取色角色（映射到 CSS 语义 token，3D 与 2D 共用一套配色口径） */
export type IslandThemeRole = 'accent' | 'ok' | 'warn' | 'danger' | 'info' | 'text'

export interface IslandKindMeta {
  /** i18n 键（约定：island.kind.<kind>） */
  labelKey: string
  /** 该类实体唯一的运行时来源 */
  source: IslandSource
  /** 状态灯取色角色 */
  role: IslandThemeRole
  /** 同屏上限（性能预算 + 布局确定性） */
  maxInstances: number
  /** 是否坐落水面（栈桥/码头类，布局时越过岸线） */
  waterborne: boolean
  /** 是否具备可点亮的灯笼（低质量档会关闭部分灯光） */
  glow: boolean
}

export const ISLAND_KINDS: readonly IslandEntityKind[] = [
  'core',
  'workshop',
  'outpost',
  'depot',
  'beacon',
  'pier',
  'athenaeum',
  'construction',
]

export const ISLAND_KIND_META: Record<IslandEntityKind, IslandKindMeta> = {
  core: {
    labelKey: 'island.kind.core',
    source: 'session',
    role: 'accent',
    maxInstances: 1,
    waterborne: false,
    glow: true,
  },
  workshop: {
    labelKey: 'island.kind.workshop',
    source: 'tool',
    role: 'info',
    maxInstances: 8,
    waterborne: false,
    glow: true,
  },
  outpost: {
    labelKey: 'island.kind.outpost',
    source: 'subagent',
    role: 'info',
    maxInstances: 6,
    waterborne: false,
    glow: false,
  },
  depot: {
    labelKey: 'island.kind.depot',
    source: 'artifact',
    role: 'ok',
    maxInstances: 5,
    // 仓库建在浅滩码头（桩柱入水），因此是 waterborne
    waterborne: true,
    glow: false,
  },
  beacon: {
    labelKey: 'island.kind.beacon',
    source: 'router',
    role: 'warn',
    maxInstances: 1,
    waterborne: false,
    glow: true,
  },
  pier: {
    labelKey: 'island.kind.pier',
    source: 'plan',
    role: 'accent',
    maxInstances: 1,
    waterborne: true,
    glow: false,
  },
  athenaeum: {
    labelKey: 'island.kind.athenaeum',
    // 记忆召回（联想记忆网络的一次检索）：知识与联想属信息性来源
    source: 'memory',
    role: 'info',
    maxInstances: 1,
    waterborne: false,
    glow: true,
  },
  construction: {
    labelKey: 'island.kind.construction',
    // 施工地块：由 buildingGrowth 的时间轴/设备进度驱动，非 RPC 事件来源
    source: 'construction',
    // 工地是「正在作业」的场所，活动时取 warn（安全橙），与脚手架/塔吊安全色同语义
    role: 'warn',
    maxInstances: 8,
    waterborne: false,
    // 工地夜间不挂灯笼（完工后 finishedDressing 也不发光）
    glow: false,
  },
}

/** 状态 → 取色角色（null = 用中性文本色，表示「无活动」） */
export const ISLAND_STATUS_ROLE: Record<IslandActivityStatus, IslandThemeRole | null> = {
  idle: null,
  queued: 'warn',
  active: 'accent',
  success: 'ok',
  error: 'danger',
  waiting: 'info',
}

/** 反向映射：来源 → 建筑类型（与 ISLAND_KIND_META[kind].source 互为逆） */
export function kindForSource(source: IslandSource): IslandEntityKind {
  const hit = ISLAND_KINDS.find((kind) => ISLAND_KIND_META[kind].source === source)
  if (!hit) {
    // 契约不变量：任何来源都必须有归属建筑。新增 IslandSource 时必须同步 META。
    throw new Error(`islandContract: 未映射的实体来源 "${source}"，请在 ISLAND_KIND_META 中登记`)
  }
  return hit
}

/**
 * 工具名 → 工坊形态。顺序即优先级（先匹配先赢）：
 * forge（执行） > observatory（联网/浏览器） > library（读取/检索） > mill（写入/其它）。
 */
export const TOOL_FLAVOR_RULES: readonly { flavor: IslandWorkshopFlavor; test: RegExp }[] = [
  { flavor: 'forge', test: /terminal|shell|exec|command|ssh|docker|npm|build|install|kill|process/i },
  { flavor: 'observatory', test: /browser|web_|web$|http|crawl|screenshot|camera|vision|fetch_url/i },
  { flavor: 'library', test: /read|search|grep|glob|list|kb|knowledge|memory|recall|doc|index/i },
]

/** 未命中任何规则时的默认工坊形态（写入/编辑类工具的工位） */
export const DEFAULT_WORKSHOP_FLAVOR: IslandWorkshopFlavor = 'mill'

export function workshopFlavor(toolName: string): IslandWorkshopFlavor {
  const name = (toolName || '').trim()
  if (!name) return DEFAULT_WORKSHOP_FLAVOR
  for (const rule of TOOL_FLAVOR_RULES) {
    if (rule.test.test(name)) return rule.flavor
  }
  return DEFAULT_WORKSHOP_FLAVOR
}

/** 工具三态 → 活动状态（运行中=active，失败=error，成功=success） */
export function statusFromTool(state: { isRunning: boolean; isError: boolean }): IslandActivityStatus {
  if (state.isRunning) return 'active'
  return state.isError ? 'error' : 'success'
}

/**
 * 会话运行态 → 岛屿状态。键集必须覆盖 ChatRunStatusState 全量取值
 * （idle/queued/running/approval_pending/interrupted/failed/timeout/cancelled）。
 */
const RUN_STATUS_MAP: Record<string, IslandActivityStatus> = {
  idle: 'idle',
  queued: 'queued',
  running: 'active',
  approval_pending: 'waiting',
  interrupted: 'waiting',
  failed: 'error',
  timeout: 'error',
  cancelled: 'idle',
}

export function statusFromRun(runStatus: string): IslandActivityStatus {
  return RUN_STATUS_MAP[runStatus] ?? 'idle'
}

/** 子代理终态 → 活动状态（完成=success，失败/中止=error，其余视为仍在途） */
export function statusFromSubagent(status: string): IslandActivityStatus {
  const s = (status || '').toLowerCase()
  if (s === 'completed' || s === 'succeeded' || s === 'success' || s === 'done') return 'success'
  if (s === 'failed' || s === 'error' || s === 'aborted' || s === 'cancelled') return 'error'
  return 'active'
}

/** 计划步骤 → 活动状态 */
export function statusFromStep(status: string): IslandActivityStatus {
  const s = (status || '').toLowerCase()
  if (s === 'completed' || s === 'done' || s === 'success') return 'success'
  if (s === 'failed' || s === 'error') return 'error'
  if (s === 'running' || s === 'active' || s === 'in_progress') return 'active'
  return 'queued'
}

/**
 * 记忆召回 → 活动状态。口径依据（诚实可视化：没有召回就没有塔）：
 * - hits = 0：本次查询未命中任何记忆条目（后端通常不发事件）→ 无召回活动（idle）。
 * - hits > 0 且 reinforced > 0：命中并强化了突触连接 → 一次完整的联想激活（success）。
 * - hits > 0 但无强化：只命中未形成新的突触强化 → 召回仍在进行（active）。
 */
export function statusFromRecall(recall: { hits: number; reinforced?: number }): IslandActivityStatus {
  const hits = Number.isFinite(recall.hits) ? Math.max(0, Math.floor(recall.hits)) : 0
  if (hits <= 0) return 'idle'
  return (recall.reinforced ?? 0) > 0 ? 'success' : 'active'
}

/** 派生工坊标题：去掉冗长的工具前缀，保留可读短名 */
export function toolDisplayName(toolName: string): string {
  const name = (toolName || '').trim() || 'tool'
  return name.length > 22 ? `${name.slice(0, 21)}…` : name
}

/**
 * 建筑详情面板用的结构化运行数据（语言中立：只放原始值，标签由 UI 侧 i18n 提供）。
 * 每类建筑只填自己那几个字段 —— 面板按「有则显示」渲染，不编造缺失项。
 */
export interface IslandEntityMeta {
  /** 工坊：工具标识/形态/起止时间（运行中实时算耗时） */
  toolId?: string
  toolFlavor?: IslandWorkshopFlavor
  startedAt?: number
  endedAt?: number
  hasError?: boolean
  /** 哨所/栈桥：原始状态字符串（直接来自事件，不做二次解释） */
  rawStatus?: string
  /** 主堡：会话运行态原始字符串（ChatRunStatusState） */
  runStatus?: string
  /** 哨所：子会话 key */
  childKey?: string
  /** 仓库：产物标识与类型 */
  artifactId?: string
  artifactKind?: string
  /** 主堡：会话与编排 */
  sessionKey?: string
  taskTitle?: string
  strategy?: string
  /** 灯塔：融合编排成员完成态 */
  ensembleMembers?: readonly { label: string; done: boolean }[]
  /** 栈桥：计划步骤全量 */
  steps?: readonly { id: string; title: string; status: string }[]
  /** 记忆塔：最近一次记忆召回（联想记忆网络的一次检索） */
  recallQuery?: string
  recallHits?: number
  recallTopScore?: number
  reinforced?: number
  memoryIds?: readonly string[]
}

/** 毫秒 → 人读耗时（>1h 显示 h，>1m 显示 m:ss，否则 s） */
export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return '—'
  const totalSeconds = Math.floor(ms / 1000)
  const hours = Math.floor(totalSeconds / 3600)
  const minutes = Math.floor((totalSeconds % 3600) / 60)
  const seconds = totalSeconds % 60
  if (hours > 0) return `${hours}h${String(minutes).padStart(2, '0')}m`
  if (minutes > 0) return `${minutes}m${String(seconds).padStart(2, '0')}s`
  return `${seconds}s`
}