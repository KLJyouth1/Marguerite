import { describe, expect, it } from 'vitest'
import {
  applyDeviceEvent,
  BUILDING_STAGES,
  partsForStage,
  snapshotAtDay,
  snapshotAtProgress,
  stageAtProgress,
  validateConstructionPlot,
  visibleFloorsAt,
  type ConstructionPlotConfig,
} from './buildingGrowth'
import {
  barrierRing,
  cranePoseAt,
  DEFAULT_CRANE_BUDGET,
  scaffoldGrid,
  vehiclePoseAt,
  workerPoseAt,
  type WorkerWaypoint,
} from './constructionSystem'

const PLOT: ConstructionPlotConfig = {
  id: 'plot:test',
  x: 0,
  z: 0,
  rotation: 0,
  building: { footprint: 2.4, floorHeight: 0.9, totalFloors: 6 },
  startDay: 0,
  endDay: 24,
  crew: 8,
  cranes: 1,
  vehicles: 2,
  deviceId: 'device:test',
}

describe('buildingGrowth — 进度→阶段映射', () => {
  it('阶段阈值边界单调落在 BUILDING_STAGES 序上', () => {
    expect(stageAtProgress(0)).toBe('foundation')
    expect(stageAtProgress(0.1)).toBe('foundation')
    expect(stageAtProgress(0.14)).toBe('structure')
    expect(stageAtProgress(0.35)).toBe('frame')
    expect(stageAtProgress(0.6)).toBe('facade')
    expect(stageAtProgress(0.8)).toBe('windows')
    expect(stageAtProgress(0.95)).toBe('interior')
    expect(stageAtProgress(1)).toBe('finished')
  })

  it('progress 被夹紧到 0..1（NaN/越界不炸）', () => {
    expect(stageAtProgress(-5)).toBe('foundation')
    expect(stageAtProgress(Number.NaN)).toBe('foundation')
    expect(stageAtProgress(99)).toBe('finished')
  })

  it('BUILDING_STAGES 覆盖七道工序', () => {
    expect(BUILDING_STAGES).toEqual([
      'foundation',
      'structure',
      'frame',
      'facade',
      'windows',
      'interior',
      'finished',
    ])
  })
})

describe('buildingGrowth — State From Time（时间轴可回放）', () => {
  it('拖回 Day3 是地基段，Day10 是结构段，Day24 完工（可逆）', () => {
    const early = snapshotAtDay(PLOT, 3) // 3/24 ≈ 0.125
    const mid = snapshotAtDay(PLOT, 10) // 10/24 ≈ 0.417
    const done = snapshotAtDay(PLOT, 24)

    expect(early.stage).toBe('foundation')
    expect(mid.stage).toBe('frame')
    expect(done.stage).toBe('finished')
    // 时间轴往前拖必须回到更早的阶段（不是只能向前播）
    expect(BUILDING_STAGES.indexOf(early.stage)).toBeLessThan(BUILDING_STAGES.indexOf(mid.stage))
    expect(BUILDING_STAGES.indexOf(mid.stage)).toBeLessThan(BUILDING_STAGES.indexOf(done.stage))
  })

  it('同一天永远得到同一快照（确定性，回放可逐帧复现）', () => {
    const a = snapshotAtDay(PLOT, 12)
    const b = snapshotAtDay(PLOT, 12)
    expect(b).toEqual(a)
  })

  it('开工前（day<startDay）不出现建筑', () => {
    const before = snapshotAtDay(PLOT, -5)
    expect(before.progress).toBe(0)
    expect(before.visibleFloors).toBe(0)
    expect(before.parts.columns).toBe(false)
  })
})

describe('buildingGrowth — 真实生长，绝不 scaleY 拉伸', () => {
  it('visibleFloors 是整数阶梯，封顶到 totalFloors 即止', () => {
    for (const p of [0, 0.15, 0.3, 0.5, 0.7, 0.9, 1]) {
      const f = visibleFloorsAt(p, 6)
      expect(Number.isInteger(f)).toBe(true)
      expect(f).toBeGreaterThanOrEqual(0)
      expect(f).toBeLessThanOrEqual(6)
    }
    // facade 之后楼层数封顶
    expect(visibleFloorsAt(0.6, 6)).toBe(6)
    expect(visibleFloorsAt(1, 6)).toBe(6)
  })

  it('脚手架只在 frame/facade/windows 出现，interior/finished 拆除', () => {
    expect(partsForStage('frame').scaffolding).toBe(true)
    expect(partsForStage('facade').scaffolding).toBe(true)
    expect(partsForStage('windows').scaffolding).toBe(true)
    expect(partsForStage('interior').scaffolding).toBe(false)
    expect(partsForStage('finished').scaffolding).toBe(false)
  })

  it('完工才出现 finishedDressing（绿化/围挡拆除）', () => {
    for (const s of BUILDING_STAGES) {
      expect(partsForStage(s).finishedDressing).toBe(s === 'finished')
    }
  })
})

describe('buildingGrowth — 设备事件 → 施工阶段', () => {
  it('设备接入 = 开工（progress 从 0 推进到地基段）', () => {
    const started = applyDeviceEvent(0, 'connected')
    expect(started).toBeGreaterThan(0)
    expect(stageAtProgress(started)).toBe('foundation')
  })

  it('任务进行中沿用外部进度，只进不退', () => {
    const running = applyDeviceEvent(0.2, 'running', 0.5)
    expect(running).toBe(0.5)
    // 外部进度回退不允许（任务在进行中，不能倒回去假装没干）
    const noBack = applyDeviceEvent(0.5, 'running', 0.3)
    expect(noBack).toBe(0.5)
  })

  it('任务完成 = 封顶（progress 1.0 → finished）', () => {
    const done = applyDeviceEvent(0.6, 'completed')
    expect(done).toBe(1)
    expect(stageAtProgress(done)).toBe('finished')
  })

  it('error 不回退、不冒进；unconnected 归零', () => {
    expect(applyDeviceEvent(0.4, 'error')).toBe(0.4)
    expect(applyDeviceEvent(0.4, 'unconnected')).toBe(0)
  })

  it('活力度随阶段变化：地基车辆多、结构塔吊忙、完工全部退场', () => {
    const found = snapshotAtProgress(PLOT, 0.05).activity
    const struct = snapshotAtProgress(PLOT, 0.2).activity
    const done = snapshotAtProgress(PLOT, 1).activity
    expect(found.vehicles).toBeGreaterThan(0)
    expect(struct.cranes).toBeGreaterThan(0)
    expect(done.workers).toBe(0)
    expect(done.cranes).toBe(0)
    expect(done.vehicles).toBe(0)
  })
})

describe('buildingGrowth — 配置校验', () => {
  it('样例配置全部合法', () => {
    expect(validateConstructionPlot(PLOT)).toEqual([])
  })
  it('坏配置在构造渲染器前被拒', () => {
    const bad: ConstructionPlotConfig = { ...PLOT, endDay: PLOT.startDay, building: { ...PLOT.building, totalFloors: 0 } }
    const errs = validateConstructionPlot(bad)
    expect(errs.length).toBeGreaterThan(0)
  })
})

describe('constructionSystem — 塔吊真实循环', () => {
  it('同一 (t, offset) 同一姿态（确定性，可回放）', () => {
    expect(cranePoseAt(12, 3)).toEqual(cranePoseAt(12, 3))
  })
  it('相位偏移让两台塔吊错峰（不机械同步）', () => {
    // 半周期偏移保证落到不同相位
    const period = Object.values(DEFAULT_CRANE_BUDGET).reduce((s, v) => s + v, 0)
    const a = cranePoseAt(12, 0)
    const b = cranePoseAt(12, period / 2)
    expect(a.armAngle).not.toBeCloseTo(b.armAngle, 5)
    expect(a.phase).not.toBe(b.phase)
  })
  it('循环闭合：t=0 与 t=周期 得到同相位', () => {
    const period = Object.values(DEFAULT_CRANE_BUDGET).reduce((s, v) => s + v, 0)
    expect(cranePoseAt(0, 0).phase).toBe(cranePoseAt(period, 0).phase)
  })
  it('载荷只在吊运相位挂着，release 后脱钩', () => {
    const releaseT = DEFAULT_CRANE_BUDGET.swingToYard + DEFAULT_CRANE_BUDGET.lowerToYard + DEFAULT_CRANE_BUDGET.hoistLoad +
      DEFAULT_CRANE_BUDGET.swingToSite + DEFAULT_CRANE_BUDGET.lowerToRoof
    expect(cranePoseAt(releaseT + 0.1, 0).loadAttached).toBe(false)
  })
})

describe('constructionSystem — 工人状态机', () => {
  const wps: WorkerWaypoint[] = [
    { x: 0, z: 0, dwell: 2 },
    { x: 4, z: 0, dwell: 2 },
  ]
  it('在 waypoint 上驻留时是 work，走在两段之间是 walk', () => {
    const working = workerPoseAt(0, 0, wps) // dwell 区间
    expect(working.state).toBe('work')
    const walking = workerPoseAt(4, 0, wps) // 走完 dwell 后在路上
    expect(walking.state).toBe('walk')
  })
  it('不同 offset 的工人位置不同（不齐步走）', () => {
    const a = workerPoseAt(3, 0, wps)
    const b = workerPoseAt(3, 1.5, wps)
    expect(a.x).not.toBeCloseTo(b.x, 5)
  })
})

describe('constructionSystem — 工程车辆', () => {
  const route = [
    { x: 0, z: 0 },
    { x: 5, z: 0 },
    { x: 5, z: 5 },
    { x: 0, z: 5 },
  ]
  it('wheelSpin 随行驶距离单调前进（车轮真在转，不是原地空转）', () => {
    const early = vehiclePoseAt(0.5, 0, route)
    const later = vehiclePoseAt(5, 0, route)
    expect(later.wheelSpin).toBeGreaterThan(early.wheelSpin)
  })
  it('环线闭合：回到停靠点速度为 0', () => {
    // 推算 stop 相位落在哪个时刻：accel + cruise + accel 之后、stopDwell 内
    const routeLen = 20
    const accelTime = 2.2 / 1.4
    const cruiseTime = routeLen / 2.2
    const stopStart = 2 * accelTime + cruiseTime
    const v = vehiclePoseAt(stopStart + 1, 0, route)
    expect(v.phase).toBe('stop')
    expect(v.speed).toBe(0)
  })
})

describe('constructionSystem — 重复件 InstancedMesh 数据', () => {
  it('脚手架/围挡产出离散 transform 数组（渲染层吃 InstancedMesh）', () => {
    const scaffold = scaffoldGrid(2.4, 6, 0.9)
    expect(scaffold.length).toBeGreaterThan(0)
    expect(scaffold[0]).toHaveProperty('x')
    expect(scaffold[0]).toHaveProperty('y')
    const ring = barrierRing(2.4)
    expect(ring.length).toBeGreaterThan(0)
  })
})
