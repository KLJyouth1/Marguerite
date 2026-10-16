import { describe, expect, it } from 'vitest'
import {
  buildConstructionEntities,
  constructionDayRange,
  loadConstructionPlots,
  sampleConstructionPlots,
  type ConstructionPlotsFile,
} from './constructionWorld'
import { BUILDING_STAGES } from './buildingGrowth'

const PLOTS = sampleConstructionPlots()

describe('constructionWorld — 样例地块加载与合法性', () => {
  it('样例 JSON 全部通过校验（坏配置不进渲染器）', () => {
    expect(PLOTS.length).toBeGreaterThanOrEqual(3)
  })

  it('坏配置在入口被滤掉（不会带着炸点进 Three.js）', () => {
    const badFile: ConstructionPlotsFile = {
      version: 1,
      plots: [
        PLOTS[0],
        { ...PLOTS[1], endDay: PLOTS[1].startDay, building: { ...PLOTS[1].building, totalFloors: 0 } },
      ],
    }
    expect(loadConstructionPlots(badFile)).toHaveLength(1)
  })
})

describe('constructionWorld — State-From-Time（时间轴回放）', () => {
  it('同一 day 同一结果（确定性）；day 推进阶段单调前进', () => {
    const early = buildConstructionEntities(PLOTS, { day: 0 })
    const mid = buildConstructionEntities(PLOTS, { day: 12 })
    const late = buildConstructionEntities(PLOTS, { day: 30 })
    expect(mid).toEqual(buildConstructionEntities(PLOTS, { day: 12 }))

    const towerEarly = early.find((e) => e.id === PLOTS[0].id)!.construction!
    const towerMid = mid.find((e) => e.id === PLOTS[0].id)!.construction!
    const towerLate = late.find((e) => e.id === PLOTS[0].id)!.construction!
    expect(BUILDING_STAGES.indexOf(towerEarly.snapshot.stage)).toBeLessThan(
      BUILDING_STAGES.indexOf(towerMid.snapshot.stage),
    )
    expect(towerLate.snapshot.stage).toBe('finished')
  })

  it('day 拖回开工前：进度 0、楼层 0、不出现柱梁（承台是开工第一铲，允许在）', () => {
    const ents = buildConstructionEntities(PLOTS, { day: -50 })
    for (const e of ents) {
      const c = e.construction!
      expect(c.snapshot.progress).toBe(0)
      expect(c.snapshot.visibleFloors).toBe(0)
      expect(c.snapshot.parts.columns).toBe(false)
      expect(e.status).toBe('idle')
    }
  })

  it('封顶地块（finished）= success 状态，其余在建 = active', () => {
    const ents = buildConstructionEntities(PLOTS, { day: 30 })
    expect(ents.every((e) => e.status === 'success')).toBe(true)
    const mid = buildConstructionEntities(PLOTS, { day: 10 })
    expect(mid.some((e) => e.status === 'active')).toBe(true)
  })
})

describe('constructionWorld — 设备/任务进度驱动', () => {
  it('设备接入=开工：deviceProgress 把地块推过时间轴进度（只向前）', () => {
    const day0 = buildConstructionEntities(PLOTS, { day: -100 }) // 时间轴：还没开工
    const forced = buildConstructionEntities(PLOTS, {
      day: -100,
      deviceProgress: { [PLOTS[0].id]: 0.9 },
    })
    expect(day0[0].construction!.snapshot.visibleFloors).toBe(0)
    expect(forced[0].construction!.snapshot.visibleFloors).toBeGreaterThan(0)
    expect(forced[0].progress).toBe(0.9)
  })

  it('设备进度回退不生效（max 合并，工地不能倒回去假装没干）', () => {
    const withForward = buildConstructionEntities(PLOTS, { day: 24 })
    const regress = buildConstructionEntities(PLOTS, {
      day: 24,
      deviceProgress: { [PLOTS[0].id]: 0.1 },
    })
    // day24 已完工 progress=1；设备喂 0.1 不得把它拉回去
    expect(regress[0].progress).toBe(withForward[0].progress)
  })

  it('产物是 kind=construction 的实体，自带坐标与快照', () => {
    const [e] = buildConstructionEntities(PLOTS, { day: 12 })
    expect(e.kind).toBe('construction')
    expect(e.plot.x).toBe(PLOTS[0].x)
    expect(e.construction).toBeTruthy()
    expect(e.construction!.config).toEqual(PLOTS[0])
  })
})

describe('constructionWorld — 时间轴范围', () => {
  it('覆盖最早开工到最晚封顶；空表兜底', () => {
    const range = constructionDayRange(PLOTS)
    expect(range.min).toBeLessThan(range.max)
    expect(range.min).toBe(Math.min(...PLOTS.map((p) => p.startDay)))
    expect(range.max).toBe(Math.max(...PLOTS.map((p) => p.endDay)))
    expect(constructionDayRange([])).toEqual({ min: 0, max: 1 })
  })
})
