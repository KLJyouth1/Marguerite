/**
 * 施工地块 → 岛屿实体（纯函数，不 import three / 不碰 DOM，可在 node 单测）。
 *
 * 两个驱动源（任务书 §设备/状态 与 §State-From-Time）：
 * - **时间轴回放**：给定虚拟世界日 `day`，`snapshotAtDay(config, day)` 给出那天的工序形态；
 *   拖时间轴到任意一天即回放到对应施工阶段（可逆，纯函数）。
 * - **设备/任务进度**：`deviceProgress[plotId]` 由上层（设备接入=开工、任务完成=封顶）喂入；
 *   实际进度取「时间轴进度」与「设备进度」的较大值 —— 时间轴负责回放历史，设备事件只向前推。
 *
 * 产出的实体 kind='construction'，与既有 7 类建筑同走 Island3D 的实体管线
 * （增量 diff / 拾取 / 标签 / 选中），但几何由 constructionPlotBuilders 逐工序增删部件。
 */

import {
  snapshotAtDay,
  snapshotAtProgress,
  validateConstructionPlot,
  type BuildingStage,
  type ConstructionPlotConfig,
} from './buildingGrowth'
import type { IslandActivityStatus } from './islandContract'
import type { IslandEntity } from './islandWorld'
import sampleJson from './constructionPlots.sample.json'

/** 样例施工地块（数据驱动：加建筑 = 改 constructionPlots.sample.json） */
export interface ConstructionPlotsFile {
  version: number
  note?: string
  plots: ConstructionPlotConfig[]
}

/** 把 JSON 文件里的地块筛一遍：坏配置在进入渲染器前就被拒（诚实可视化） */
export function loadConstructionPlots(file: ConstructionPlotsFile): ConstructionPlotConfig[] {
  return (file.plots ?? []).filter((plot) => validateConstructionPlot(plot).length === 0)
}

/** 内置样例地块（本机会话默认演示；上层可替换为自己的配置源） */
export function sampleConstructionPlots(): ConstructionPlotConfig[] {
  return loadConstructionPlots(sampleJson as ConstructionPlotsFile)
}

export interface BuildConstructionOptions {
  /** 虚拟世界日（时间轴位置）；State-From-Time 的回放轴 */
  day: number
  /**
   * 设备/任务进度覆盖（plotId → progress 0..1）。
   * 设备接入=开工、任务完成=封顶；缺省 {} = 纯时间轴回放。
   */
  deviceProgress?: Record<string, number>
}

/** 工序阶段 → 岛屿活动状态（工地活动=active；封顶完工=success；未开工=idle） */
export function statusFromStage(stage: BuildingStage, progress: number): IslandActivityStatus {
  if (progress <= 0) return 'idle'
  if (stage === 'finished') return 'success'
  return 'active'
}

/**
 * 把地块配置列表翻译成岛屿实体。纯函数：同一 (plots, day, deviceProgress) 永远同一结果。
 */
export function buildConstructionEntities(
  plots: readonly ConstructionPlotConfig[],
  opts: BuildConstructionOptions,
): IslandEntity[] {
  const deviceProgress = opts.deviceProgress ?? {}
  const out: IslandEntity[] = []
  for (const config of plots) {
    if (validateConstructionPlot(config).length > 0) continue
    const timeSnap = snapshotAtDay(config, opts.day)
    const driven = typeof deviceProgress[config.id] === 'number' && Number.isFinite(deviceProgress[config.id])
      ? Math.min(1, Math.max(0, deviceProgress[config.id]))
      : timeSnap.progress
    // 时间轴负责回放，设备只向前推：实际进度取较大者（不允许设备进度把工地倒回去）
    const progress = Math.max(timeSnap.progress, driven)
    const snapshot = snapshotAtProgress(config, progress)
    const stageLabel = stageLabelShort(snapshot.stage)
    out.push({
      id: config.id,
      kind: 'construction',
      title: config.id.replace(/^plot:/, 'construction:'),
      status: statusFromStage(snapshot.stage, progress),
      detail: `${stageLabel} · ${snapshot.visibleFloors}/${config.building.totalFloors}F`,
      source: 'construction',
      plot: { x: config.x, z: config.z, rotation: config.rotation },
      progress,
      construction: { config, snapshot },
    })
  }
  return out
}

/** 工序阶段 → 短英文标记（写入 entity.detail，人读；中文/本地化由面板 i18n 覆盖） */
function stageLabelShort(stage: BuildingStage): string {
  switch (stage) {
    case 'foundation': return 'foundation'
    case 'structure': return 'structure'
    case 'frame': return 'frame'
    case 'facade': return 'facade'
    case 'windows': return 'windows'
    case 'interior': return 'interior'
    case 'finished': return 'finished'
  }
}

/** 时间轴滑动范围（天）：覆盖所有地块的最早开工到最晚封顶 */
export function constructionDayRange(plots: readonly ConstructionPlotConfig[]): { min: number; max: number } {
  if (plots.length === 0) return { min: 0, max: 1 }
  let min = Infinity
  let max = -Infinity
  for (const p of plots) {
    min = Math.min(min, p.startDay)
    max = Math.max(max, p.endDay)
  }
  if (!Number.isFinite(min) || !Number.isFinite(max) || max <= min) return { min: 0, max: 1 }
  return { min, max }
}
