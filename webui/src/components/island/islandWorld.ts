/**
 * 岛屿世界观构建（纯函数）—— 把「智能体运行时快照」翻译成「岛上的建筑与道路」。
 *
 * - 输入是归一化后的运行时快照（RPC 事件由 useIslandRuntime 归约而来），
 *   输出是 IslandWorld：实体 + 道路 + 计数。渲染层（Island3D / 2D 降级地图）只消费它。
 * - **确定性布局**：同一份输入永远得到同一套坐标（plotFor 只依赖 kind/index/count/spec），
 *   因此场景可以按 id 做增量 diff，而不是每帧重建。
 * - **诚实可视化**：没有对应数据就不产生建筑（无路由决策 → 无灯塔；无计划 → 无栈桥）。
 */

import {
  ISLAND_KIND_META,
  statusFromRecall,
  statusFromRun,
  statusFromStep,
  statusFromSubagent,
  statusFromTool,
  toolDisplayName,
  workshopFlavor,
  type IslandActivityStatus,
  type IslandEntityKind,
  type IslandEntityMeta,
  type IslandSource,
  type IslandWorkshopFlavor,
} from './islandContract'
import type { IslandModelSpec } from './islandSpec'
import type { ConstructionPlotConfig, StageSnapshot } from './buildingGrowth'

/**
 * 施工地块挂载数据（仅 kind==='construction' 的实体携带）。
 * 渲染层据此按工序真实增删部件（绝不对完整盒子做 scaleY 拉伸）。
 */
export interface ConstructionMount {
  config: ConstructionPlotConfig
  snapshot: StageSnapshot
}

export interface IslandToolInput {
  id: string
  name: string
  isRunning: boolean
  isError: boolean
  /** 服务端开始的 epoch ms（可选，用于详情显示耗时） */
  startedAt?: number
  endedAt?: number
}

export interface IslandPlanStepInput {
  id: string
  title: string
  status: string
}

/** 记忆召回快照（联想记忆网络的一次检索；hits=0 后端不发事件 → 通常为 null） */
export interface IslandRecallInput {
  /** 召回查询 */
  query: string
  /** 命中条数 */
  hits: number
  /** 最高相似度 0..1 */
  topScore: number
  /** 命中的记忆条目 id（可能被截断到 ≤8） */
  ids: string[]
  /** 本次强化的突触数 */
  reinforced: number
  /** 归约时间戳（epoch ms） */
  at: number
}

export interface IslandRuntimeInput {
  sessionKey: string
  /** 任务/会话标题（可空） */
  taskTitle?: string
  /** ChatRunStatusState 字符串 */
  runStatus: string
  tools: IslandToolInput[]
  planSteps: IslandPlanStepInput[]
  subagents: { key: string; status: string }[]
  artifacts: { id: string; name: string; kind?: string }[]
  /** router_decision.strategy（模型路由/融合编排形态） */
  strategy?: string
  /** ensemble_progress 归约出的成员状态（label → 是否结束） */
  ensembleMembers: { label: string; done: boolean }[]
  /** 最近一次记忆召回（无召回为 null，岛上不出记忆塔） */
  recall: IslandRecallInput | null
  /** 最近一次事件时间戳（用于「静默」判定） */
  lastEventAt: number
}

export interface IslandEntity {
  id: string
  kind: IslandEntityKind
  /** 仅工坊有：由工具名派生的工位形态 */
  flavor?: IslandWorkshopFlavor
  title: string
  status: IslandActivityStatus
  /** 语言中立的补充信息（状态名/进度/模型名，可直接展示） */
  detail: string
  source: IslandSource
  plot: { x: number; z: number; rotation: number }
  /** 0..1（栈桥铺板进度 / 仓库装载率） */
  progress?: number
  /** 详情面板用的结构化数据（按类只填自己那几个字段） */
  meta?: IslandEntityMeta
  /** 施工地块：工序快照（仅 kind==='construction'） */
  construction?: ConstructionMount
}

export interface IslandLink {
  id: string
  from: string
  to: string
  active: boolean
}

export interface IslandPlanSummary {
  total: number
  done: number
  progress: number
}

export interface IslandWorld {
  sessionKey: string
  runStatus: string
  strategy: string
  entities: IslandEntity[]
  links: IslandLink[]
  counts: Record<IslandEntityKind, number>
  plan: IslandPlanSummary | null
  /** 无任何运行数据（UI 显示空态；岛上不出现虚构建筑） */
  empty: boolean
}

export function emptyRuntimeInput(): IslandRuntimeInput {
  return {
    sessionKey: '',
    runStatus: 'idle',
    tools: [],
    planSteps: [],
    subagents: [],
    artifacts: [],
    ensembleMembers: [],
    recall: null,
    lastEventAt: 0,
  }
}

/**
 * 确定性布局：x/z 为岛平面坐标，rotation 使建筑本地 +Z 指向岛屿外侧径向。
 *
 * 分区规则（避免建筑互相穿插，同时让岛像被规划过）：
 *   core    岛心
 *   workshop 内环（半径 coreRadius，奇偶交错微调避免同环相撞）
 *   outpost  中环（coreRadius + ringGap，与内环错开半个步长）
 *   depot    浅滩码头排（越过岸线、桩柱入水，围绕 quayAngle 展开）
 *   pier     码头正上方，向海面延伸
 *   beacon   西侧岬角（固定方位，与外环留出径向间距）
 *   athenaeum 南侧岬角（固定方位 270°，与灯塔轴对称，落在外环哨所槽位正中）
 *
 * **位置只依赖 (kind, index)**：角步长取该类的 maxInstances（而非当前实体数），
 * 于是新增/移除建筑不会让已有建筑瞬移 —— 场景可以按 id 增量 diff，装饰物也能预先避让。
 */
export function plotFor(
  kind: IslandEntityKind,
  index: number,
  spec: IslandModelSpec,
): { x: number; z: number; rotation: number } {
  const { coreRadius, ringGap, plotGap, beaconRadius, quayAngle } = spec.layout
  const place = (angle: number, radius: number) => ({
    x: Math.cos(angle) * radius,
    z: Math.sin(angle) * radius,
    // 本地 +Z 指向径向：yaw = π/2 − angle（绕 Y 旋转后 (0,0,1) → (cos a, 0, sin a)）
    rotation: Math.PI / 2 - angle,
  })
  const step = (Math.PI * 2) / ISLAND_KIND_META[kind].maxInstances

  switch (kind) {
    case 'core':
      return { x: 0, z: 0, rotation: Math.PI / 2 - (quayAngle + Math.PI) }
    case 'workshop': {
      const angle = -Math.PI / 2 + index * step
      return place(angle, coreRadius * (index % 2 === 0 ? 1 : plotGap))
    }
    case 'outpost': {
      const angle = -Math.PI / 2 + step / 2 + index * step
      return place(angle, coreRadius + ringGap)
    }
    case 'depot': {
      // 码头仓库排：沿海湾等距展开，整体位于浅滩
      const spread = 1.15
      const slots = Math.max(1, ISLAND_KIND_META.depot.maxInstances - 1)
      const angle = quayAngle - spread / 2 + (index / slots) * spread
      return place(angle, spec.terrain.radius + spec.terrain.beachWidth * 0.55)
    }
    case 'beacon':
      // 固定绝对方位（210°），刻意落在两个哨所槽位（180°/240°）正中的岬角上：
      // 与 quayAngle 解耦后，灯塔永远不会随码头方位漂移而压到哨所。
      return place((Math.PI * 7) / 6, beaconRadius)
    case 'athenaeum':
      // 记忆塔：固定绝对方位（270°=正南），与灯塔（210°）左右轴对称，
      // 落在外环哨所槽位（240°/300°）正中的岬角上；半径来自 spec，与数据无关。
      return place((Math.PI * 3) / 2, spec.layout.athenaeumRadius)
    case 'pier':
      return place(quayAngle, spec.terrain.radius * 0.78)
    case 'construction':
      // 施工地块自带显式坐标（ConstructionPlotConfig.x/z），不走确定性槽位；
      // 这里只是穷尽 switch 的兜底（构造实体时直接用 config 坐标，不会调到此分支）。
      return { x: 0, z: 0, rotation: 0 }
  }
}

function shortKey(key: string): string {
  const trimmed = (key || '').trim()
  if (trimmed.length <= 12) return trimmed || 'session'
  return `…${trimmed.slice(-10)}`
}

/**
 * 运行时快照 → 岛屿世界。纯函数：不读时钟、不碰 DOM，可在 node 环境完整单测。
 */
export function buildIslandWorld(input: IslandRuntimeInput, spec: IslandModelSpec): IslandWorld {
  const entities: IslandEntity[] = []
  const runStatus = input.runStatus || 'idle'
  const strategy = (input.strategy || '').trim()

  // ── 中央主堡：会话本体（恒定存在，是唯一「无数据也成立」的建筑）──
  entities.push({
    id: 'core',
    kind: 'core',
    title: input.taskTitle?.trim() || shortKey(input.sessionKey),
    status: statusFromRun(runStatus),
    detail: strategy ? `${runStatus} · ${strategy}` : runStatus,
    source: ISLAND_KIND_META.core.source,
    plot: plotFor('core', 0, spec),
    meta: {
      sessionKey: input.sessionKey,
      taskTitle: input.taskTitle?.trim() || undefined,
      runStatus,
      strategy: strategy || undefined,
    },
  })

  // ── 工坊：最近的工具调用（保留一定历史，让岛屿有「刚刚发生过什么」）──
  const tools = input.tools.slice(-ISLAND_KIND_META.workshop.maxInstances)
  tools.forEach((tool, index) => {
    entities.push({
      id: `workshop:${tool.id}`,
      kind: 'workshop',
      flavor: workshopFlavor(tool.name),
      title: toolDisplayName(tool.name),
      status: statusFromTool(tool),
      detail: tool.isRunning ? 'running' : tool.isError ? 'error' : 'success',
      source: ISLAND_KIND_META.workshop.source,
      plot: plotFor('workshop', index, spec),
      meta: {
        toolId: tool.id,
        toolFlavor: workshopFlavor(tool.name),
        startedAt: tool.startedAt,
        endedAt: tool.endedAt,
        hasError: tool.isError,
      },
    })
  })

  // ── 哨所：子代理 ──
  const subagents = input.subagents.slice(-ISLAND_KIND_META.outpost.maxInstances)
  subagents.forEach((agent, index) => {
    entities.push({
      id: `outpost:${agent.key}`,
      kind: 'outpost',
      title: shortKey(agent.key),
      status: statusFromSubagent(agent.status),
      detail: agent.status || 'running',
      source: ISLAND_KIND_META.outpost.source,
      plot: plotFor('outpost', index, spec),
      meta: { childKey: agent.key, rawStatus: agent.status || 'running' },
    })
  })

  // ── 仓库：交付产物（装载率 = 当前数量 / 上限）──
  const artifacts = input.artifacts.slice(-ISLAND_KIND_META.depot.maxInstances)
  artifacts.forEach((artifact, index) => {
    entities.push({
      id: `depot:${artifact.id}`,
      kind: 'depot',
      title: artifact.name || artifact.id,
      status: 'success',
      detail: artifact.kind || 'artifact',
      source: ISLAND_KIND_META.depot.source,
      plot: plotFor('depot', index, spec),
      progress: Math.min(1, (index + 1) / ISLAND_KIND_META.depot.maxInstances),
      meta: { artifactId: artifact.id, artifactKind: artifact.kind },
    })
  })

  // ── 灯塔：模型路由 / 融合编排（有决策才有灯）──
  if (strategy || input.ensembleMembers.length > 0) {
    const done = input.ensembleMembers.filter((m) => m.done).length
    entities.push({
      id: 'beacon',
      kind: 'beacon',
      title: strategy || 'router',
      status: runStatus === 'running' ? 'active' : 'idle',
      detail: input.ensembleMembers.length > 0
        ? `${done}/${input.ensembleMembers.length}`
        : 'routing',
      source: ISLAND_KIND_META.beacon.source,
      plot: plotFor('beacon', 0, spec),
      meta: {
        strategy: strategy || undefined,
        ensembleMembers: input.ensembleMembers.length > 0 ? input.ensembleMembers : undefined,
      },
    })
  }

  // ── 栈桥：计划推进（一步一块铺板）──
  let plan: IslandPlanSummary | null = null
  if (input.planSteps.length > 0) {
    const total = input.planSteps.length
    const done = input.planSteps.filter((s) => statusFromStep(s.status) === 'success').length
    const progress = total > 0 ? done / total : 0
    plan = { total, done, progress }
    const running = input.planSteps.some((s) => statusFromStep(s.status) === 'active')
    entities.push({
      id: 'pier',
      kind: 'pier',
      title: input.planSteps[Math.min(done, total - 1)]?.title || 'plan',
      status: running ? 'active' : progress >= 1 ? 'success' : 'queued',
      detail: `${done}/${total}`,
      source: ISLAND_KIND_META.pier.source,
      plot: plotFor('pier', 0, spec),
      progress,
      meta: { steps: input.planSteps, rawStatus: input.planSteps.find((s) => statusFromStep(s.status) === 'active')?.status },
    })
  }

  // ── 记忆塔：记忆召回（联想记忆网络的一次检索；无召回则无塔）──
  if (input.recall) {
    const recall = input.recall
    entities.push({
      id: 'athenaeum',
      kind: 'athenaeum',
      title: recall.query || 'memory',
      status: statusFromRecall(recall),
      detail: recall.query || 'memory',
      source: ISLAND_KIND_META.athenaeum.source,
      plot: plotFor('athenaeum', 0, spec),
      meta: {
        recallQuery: recall.query || undefined,
        recallHits: recall.hits,
        recallTopScore: recall.topScore,
        reinforced: recall.reinforced,
        memoryIds: recall.ids.length > 0 ? recall.ids : undefined,
      },
    })
  }

  // ── 道路：中央主堡 → 每座建筑（数据通路）；活动中的道路会跑货物流 ──
  const links: IslandLink[] = entities
    .filter((entity) => entity.kind !== 'core')
    .map((entity) => ({
      id: `link:${entity.id}`,
      from: 'core',
      to: entity.id,
      active: entity.status === 'active',
    }))

  const counts = { core: 0, workshop: 0, outpost: 0, depot: 0, beacon: 0, pier: 0, athenaeum: 0, construction: 0 } as Record<IslandEntityKind, number>
  for (const entity of entities) counts[entity.kind] += 1

  return {
    sessionKey: input.sessionKey,
    runStatus,
    strategy,
    entities,
    links,
    counts,
    plan,
    empty: entities.length <= 1 && input.tools.length === 0 && input.planSteps.length === 0,
  }
}