/**
 * 施工地块 · 建筑按工序真实生长（纯逻辑，不 import three / 不碰 DOM）。
 *
 * 铁律落地：
 * - **建筑不是一个完整 GLB，也不是 scaleY 拉伸**。工序阶段随 `progress 0..1`
 *   真实出现：柱子先立、梁再架、楼板逐层出、脚手架围起、外墙封、窗装、拆架完工。
 *   可见楼层数 `visibleFloors` 是**整数阶梯**（已浇筑完成的楼层数），
 *   绝不把一个完整盒子沿 Y 拉长假装长高。
 * - **State From Time**：给定虚拟世界日期（可被时间轴任意拖动），纯函数回放
 *   到那一阶段的形态 —— 不是只能向前播的动画。
 * - **数据驱动**：阶段阈值、楼层数、工期全部来自 `ConstructionPlotConfig`；
 *   加建筑 = 加一份 JSON，不改本文件。
 * - **设备状态 → 施工阶段**：设备接入成功 = 开工（地基→首层）；
 *   任务进行中 = 对应楼层施工；任务完成 = 封顶/完工。见 `applyDeviceEvent`。
 *
 * 本模块是「模型与逻辑分离」里的逻辑侧：渲染层（后续接线者）只消费这里产出的
 * `StageSnapshot`（每一步该出现什么、该藏什么、工人数/车辆密度多少）。
 */

/** 工序阶段枚举（任务书 §3 权威枚举） */
export type BuildingStage =
  | 'foundation' // 01 开挖/地基/首柱起
  | 'structure' // 02 梁 + 03 楼板逐层
  | 'frame' // 04 脚手架搭起 + 05 工人进入作业层
  | 'facade' // 06 外墙
  | 'windows' // 07 窗
  | 'interior' // 08 拆脚手架 + 内部收尾
  | 'finished' // 完工：拆围挡、加绿化

/** 阶段次序（单一事实源：阈值表与回放都读它） */
export const BUILDING_STAGES: readonly BuildingStage[] = [
  'foundation',
  'structure',
  'frame',
  'facade',
  'windows',
  'interior',
  'finished',
]

/** 阶段 → i18n 键（6 语言文案由协调者落 locale 文件；本模块只出 key，不改现有 locale） */
export const STAGE_LABEL_KEY: Record<BuildingStage, string> = {
  foundation: 'island.construction.stage.foundation',
  structure: 'island.construction.stage.structure',
  frame: 'island.construction.stage.frame',
  facade: 'island.construction.stage.facade',
  windows: 'island.construction.stage.windows',
  interior: 'island.construction.stage.interior',
  finished: 'island.construction.stage.finished',
}

/**
 * 各阶段在 0..1 进度轴上的**进入阈值**（含端点）。
 * 例：threshold.structure = 0.18 表示 progress >= 0.18 才进入结构阶段。
 * 阈值单调递增、末段到 1.0。数据驱动：样例 JSON 可整体覆盖。
 */
export interface StageThresholds {
  foundation: number
  structure: number
  frame: number
  facade: number
  windows: number
  interior: number
  finished: number
}

export const DEFAULT_STAGE_THRESHOLDS: StageThresholds = {
  foundation: 0.0,
  structure: 0.14,
  frame: 0.32,
  facade: 0.58,
  windows: 0.78,
  interior: 0.9,
  finished: 1.0,
}

/** 设备状态（接 WP1 设备状态字段；侧车/RPC 归约后传入） */
export type DeviceLifecycle = 'unconnected' | 'connected' | 'running' | 'completed' | 'error'

/**
 * 单个施工地块配置（场景 JSON 的元素）。加建筑 = 加一份此配置。
 * 坐标沿用岛平面 x/z（与 islandWorld.plotFor 同一坐标系）。
 */
export interface ConstructionPlotConfig {
  /** 地块唯一 id（事件路由用） */
  id: string
  /** 展示名（可选；缺省由 id 的 slug 推导）。放数据里而不是代码里：
   *  不同配置源可以给自己的地块起本地化名字，渲染层不写死任何建筑名。 */
  name?: string
  /** 岛平面落点 */
  x: number
  z: number
  rotation: number
  /** 建筑本体（米） */
  building: {
    footprint: number // 边长（方盒子平面）
    floorHeight: number // 每层净高
    totalFloors: number // 设计总楼层
  }
  /** 工期（虚拟世界「天」）：startDay 开工，endDay 封顶 */
  startDay: number
  endDay: number
  /** 阶段阈值覆盖（缺省用 DEFAULT_STAGE_THRESHOLDS） */
  thresholds?: Partial<StageThresholds>
  /** 投入资源（按阶段密度由系统推导，此处只给总量） */
  crew: number
  cranes: number
  vehicles: number
  /** 关联的设备 id（WP1）；空 = 纯装饰地块（无设备事件驱动） */
  deviceId?: string
}

/** 某一阶段该出现/隐藏的模型部件集合（渲染层据此增删节点，绝不拉伸） */
export interface StageParts {
  /** 承台/地基外露 */
  foundation: boolean
  /** 已立起的柱子数（= visibleFloors 层，整数阶梯） */
  columns: boolean
  /** 已架梁 */
  beams: boolean
  /** 已浇筑楼板（= visibleFloors） */
  slabs: boolean
  /** 脚手架围合 */
  scaffolding: boolean
  /** 外墙封闭 */
  walls: boolean
  /** 窗格安装 */
  windows: boolean
  /** 内部/收尾（机电、地坪） */
  interior: boolean
  /** 完工状态：围挡拆、绿化加 */
  finishedDressing: boolean
}

/** 给定 progress，该阶段应有的「活力度」（工人/塔吊/车辆密度系数 0..1） */
export interface StageActivity {
  workers: number
  cranes: number
  vehicles: number
}

/** State From Time 的完整快照：渲染层一帧只需要这一个对象 */
export interface StageSnapshot {
  progress: number
  stage: BuildingStage
  /** 已完成楼层（整数；0 = 还在挖地基）—— 绝不对完整盒子做 scaleY */
  visibleFloors: number
  parts: StageParts
  activity: StageActivity
}

function clamp01(v: number): number {
  if (!Number.isFinite(v)) return 0
  return v < 0 ? 0 : v > 1 ? 1 : v
}

/** 合并阈值（局部覆盖 → 完整表） */
export function resolveThresholds(overrides?: Partial<StageThresholds>): StageThresholds {
  return { ...DEFAULT_STAGE_THRESHOLDS, ...(overrides ?? {}) }
}

/**
 * progress 0..1 → 阶段。纯函数、单调、可逆。
 * 末段 finished=1.0：progress>=1 才完工（避免提前「封顶」）。
 */
export function stageAtProgress(progress: number, thresholds?: Partial<StageThresholds>): BuildingStage {
  const t = resolveThresholds(thresholds)
  const p = clamp01(progress)
  // 从后往前判：p >= finished → finished；否则落到第一个进入段
  if (p >= t.finished) return 'finished'
  if (p >= t.interior) return 'interior'
  if (p >= t.windows) return 'windows'
  if (p >= t.facade) return 'facade'
  if (p >= t.frame) return 'frame'
  if (p >= t.structure) return 'structure'
  return 'foundation'
}

/**
 * 整数阶梯楼层：在 structure 阶段从 0 长到 totalFloors，之后封顶不再变化。
 * 这是「真实生长」的核心 —— 渲染层每浇筑一层就**新增一层楼板/柱子**，
 * 而不是把一个成品盒子沿 Y 拉长。
 */
export function visibleFloorsAt(progress: number, totalFloors: number, thresholds?: Partial<StageThresholds>): number {
  const t = resolveThresholds(thresholds)
  const p = clamp01(progress)
  if (p < t.structure) return 0
  if (p >= t.facade) return totalFloors
  // structure → facade 之间逐层出板
  const span = Math.max(1e-6, t.facade - t.structure)
  const k = (p - t.structure) / span
  return Math.min(totalFloors, Math.floor(k * totalFloors + 1e-6))
}

/** 阶段 → 部件可见性（数据：渲染层只查表） */
export function partsForStage(stage: BuildingStage): StageParts {
  switch (stage) {
    case 'foundation':
      return { foundation: true, columns: false, beams: false, slabs: false, scaffolding: false, walls: false, windows: false, interior: false, finishedDressing: false }
    case 'structure':
      return { foundation: true, columns: true, beams: true, slabs: true, scaffolding: false, walls: false, windows: false, interior: false, finishedDressing: false }
    case 'frame':
      return { foundation: true, columns: true, beams: true, slabs: true, scaffolding: true, walls: false, windows: false, interior: false, finishedDressing: false }
    case 'facade':
      return { foundation: true, columns: true, beams: true, slabs: true, scaffolding: true, walls: true, windows: false, interior: false, finishedDressing: false }
    case 'windows':
      return { foundation: true, columns: true, beams: true, slabs: true, scaffolding: true, walls: true, windows: true, interior: false, finishedDressing: false }
    case 'interior':
      // 拆脚手架（stage 08）：墙窗已在，脚手架退场
      return { foundation: true, columns: true, beams: true, slabs: true, scaffolding: false, walls: true, windows: true, interior: true, finishedDressing: false }
    case 'finished':
      return { foundation: true, columns: true, beams: true, slabs: true, scaffolding: false, walls: true, windows: true, interior: true, finishedDressing: true }
  }
}

/**
 * 阶段 → 活力度。地基阶段挖掘机多（车辆↑、塔吊↓）；
 * 结构阶段塔吊最忙、工人上作业层；立面阶段脚手架多、工人多；完工后全部退场。
 */
export function activityForStage(stage: BuildingStage, config: ConstructionPlotConfig): StageActivity {
  const crew = Math.max(0, Math.round(config.crew))
  const cranes = Math.max(0, Math.round(config.cranes))
  const vehicles = Math.max(0, Math.round(config.vehicles))
  switch (stage) {
    case 'foundation':
      return { workers: Math.round(crew * 0.4), cranes: 0, vehicles: vehicles }
    case 'structure':
      return { workers: Math.round(crew * 0.8), cranes, vehicles: Math.round(vehicles * 0.6) }
    case 'frame':
      return { workers: crew, cranes, vehicles: Math.round(vehicles * 0.4) }
    case 'facade':
      return { workers: Math.round(crew * 0.9), cranes: Math.round(cranes * 0.5), vehicles: Math.round(vehicles * 0.3) }
    case 'windows':
      return { workers: Math.round(crew * 0.7), cranes: 0, vehicles: Math.round(vehicles * 0.2) }
    case 'interior':
      return { workers: Math.round(crew * 0.5), cranes: 0, vehicles: 0 }
    case 'finished':
      return { workers: 0, cranes: 0, vehicles: 0 }
  }
}

/**
 * State From Time 主入口：给定虚拟世界日期，回放该地块在那天的完整形态。
 * 时间轴拖回 Day3 → 与 Day10 得到不同快照（可逆，纯函数）。
 */
export function snapshotAtDay(config: ConstructionPlotConfig, day: number): StageSnapshot {
  const span = Math.max(1e-6, config.endDay - config.startDay)
  const progress = clamp01((day - config.startDay) / span)
  return snapshotAtProgress(config, progress)
}

/** progress 0..1 → 完整阶段快照 */
export function snapshotAtProgress(config: ConstructionPlotConfig, progress: number): StageSnapshot {
  const p = clamp01(progress)
  const stage = stageAtProgress(p, config.thresholds)
  return {
    progress: p,
    stage,
    visibleFloors: visibleFloorsAt(p, config.building.totalFloors, config.thresholds),
    parts: partsForStage(stage),
    activity: activityForStage(stage, config),
  }
}

/**
 * 设备生命周期事件 → 地块进度（任务书 §4：接 WP1 设备状态字段）。
 * - unconnected：未开工 → 进度 0
 * - connected：接入成功 = 开工 → 至少推进到 foundation 段（不退回 0）
 * - running：进行中 → 沿用外部进度（由任务流给）；无进度时在基础上小幅推进
 * - completed：任务完成 = 封顶 → 1.0
 * - error：保持当前进度（不回退，也不冒进）
 *
 * 返回新进度（纯函数，不写外部状态）。
 */
export function applyDeviceEvent(
  prevProgress: number,
  lifecycle: DeviceLifecycle,
  externalProgress?: number,
  thresholds?: Partial<StageThresholds>,
): number {
  const t = resolveThresholds(thresholds)
  const prev = clamp01(prevProgress)
  switch (lifecycle) {
    case 'unconnected':
      return 0
    case 'connected':
      // 开工：至少进入地基段（foundation 段起点即 0，这里保证不低于一个最小破土量）
      return Math.max(prev, t.structure * 0.3)
    case 'running':
      if (typeof externalProgress === 'number' && Number.isFinite(externalProgress)) {
        return Math.max(prev, clamp01(externalProgress))
      }
      // 无外部进度时，作业中按半个楼层台阶推进（不跳阶段）
      return Math.min(prev + 0.02, t.facade - 1e-6)
    case 'completed':
      return 1
    case 'error':
      return prev
  }
}

/** 施工任务事件（渲染层/调试面板可读的人读阶段名） */
export const STAGE_NAME_EN: Record<BuildingStage, string> = {
  foundation: 'foundation',
  structure: 'structure',
  frame: 'frame',
  facade: 'facade',
  windows: 'windows',
  interior: 'interior',
  finished: 'finished',
}

/**
 * 纯校验：坏配置在构造渲染器前被拒（与 validateIslandSpec 同口径）。
 * 返回问题列表，空 = 合法。
 */
export function validateConstructionPlot(c: ConstructionPlotConfig): string[] {
  const errs: string[] = []
  if (!c.id.trim()) errs.push('plot.id 不能为空')
  if (!(c.building.footprint > 0)) errs.push('building.footprint 必须 > 0')
  if (!(c.building.floorHeight > 0)) errs.push('building.floorHeight 必须 > 0')
  if (!(c.building.totalFloors >= 1)) errs.push('building.totalFloors 至少 1')
  if (!(c.endDay > c.startDay)) errs.push('endDay 必须 > startDay（工期为正）')
  if (c.crew < 0 || c.cranes < 0 || c.vehicles < 0) errs.push('crew/cranes/vehicles 不能为负')
  // 阈值单调：foundation <= structure <= ... <= finished=1
  const t = resolveThresholds(c.thresholds)
  const order: readonly BuildingStage[] = BUILDING_STAGES
  for (let i = 0; i + 1 < order.length; i++) {
    if (t[order[i]] > t[order[i + 1]]) errs.push(`阶段阈值非单调：${order[i]} > ${order[i + 1]}`)
  }
  if (t.finished !== 1) errs.push('thresholds.finished 必须 = 1.0（封顶即完工）')
  for (const k of order) {
    if (t[k] < 0 || t[k] > 1) errs.push(`thresholds.${k} 必须在 0..1`)
  }
  return errs
}
