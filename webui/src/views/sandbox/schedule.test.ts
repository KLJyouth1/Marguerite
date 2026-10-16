import { describe, expect, it } from 'vitest'
import {
  CRUISE_SPEED,
  LOOP_LENGTH,
  LOOP_PERIOD,
  SEGMENTS,
  dwellWindows,
  distanceAt,
  nextTrainState,
  speedAt,
  trainStateAt,
  TRAIN_COUNT,
  wrapTime,
} from './schedule'

describe('schedule — 运行图速度曲线', () => {
  it('停站时速度严格为 0（开门乘降）', () => {
    for (const win of dwellWindows()) {
      // 停站窗内逐采样（含起点，不含终点）
      for (let t = win.start; t < win.end; t += 0.25) {
        expect(speedAt(t)).toBe(0)
      }
    }
  })

  it('停站窗覆盖 A 与 B 两站，且都落在周期内', () => {
    const wins = dwellWindows()
    expect(wins).toHaveLength(3) // A(头) + B + A(尾)
    for (const w of wins) {
      expect(w.start).toBeGreaterThanOrEqual(0)
      expect(w.end).toBeLessThanOrEqual(LOOP_PERIOD)
      expect(w.end - w.start).toBeGreaterThan(0)
    }
  })

  it('起步加速段速度严格递增、从 0 升到巡航', () => {
    const accel = SEGMENTS.find((s) => s.kind === 'accel')!
    let prev = -1
    for (let t = accel.t0; t < accel.t1; t += 0.5) {
      const v = speedAt(t)
      expect(v).toBeGreaterThan(prev)
      prev = v
    }
    expect(speedAt(accel.t0)).toBeCloseTo(0, 6)
    expect(speedAt(accel.t1 - 1e-6)).toBeCloseTo(CRUISE_SPEED, 1)
  })

  it('进站制动段速度严格递减、降到 0', () => {
    const brake = SEGMENTS.find((s) => s.kind === 'brake')!
    let prev = CRUISE_SPEED + 1
    for (let t = brake.t0; t < brake.t1; t += 0.5) {
      const v = speedAt(t)
      expect(v).toBeLessThan(prev)
      prev = v
    }
    expect(speedAt(brake.t0)).toBeCloseTo(CRUISE_SPEED, 1)
    expect(speedAt(brake.t1 - 1e-6)).toBeCloseTo(0, 2)
  })

  it('巡航段速度恒定为巡航速度、不匀速绕圈之外', () => {
    const cruise = SEGMENTS.find((s) => s.kind === 'cruise')!
    for (let t = cruise.t0 + 1; t < cruise.t1 - 1; t += 3) {
      expect(speedAt(t)).toBe(CRUISE_SPEED)
    }
  })

  it('不存在匀速绕圈：全程至少出现一次速度=0', () => {
    let sawZero = false
    for (let t = 0; t < LOOP_PERIOD; t += 0.5) {
      if (speedAt(t) === 0) sawZero = true
    }
    expect(sawZero).toBe(true)
  })
})

describe('schedule — 位置积分与 State From Time', () => {
  it('停站期间位置不动（s 恒定）', () => {
    const win = dwellWindows()[0]
    const s0 = distanceAt(win.start)
    for (let t = win.start; t < win.end; t += 1) {
      expect(distanceAt(t)).toBeCloseTo(s0, 6)
    }
  })

  it('位置始终落在闭环弧长内 [0, L)', () => {
    for (let t = -50; t < LOOP_PERIOD * 2 + 50; t += 1.7) {
      const s = distanceAt(t)
      expect(s).toBeGreaterThanOrEqual(0)
      expect(s).toBeLessThan(LOOP_LENGTH)
    }
  })

  it('绕一周回到起点（闭环）', () => {
    expect(distanceAt(0)).toBeCloseTo(0, 6)
    expect(distanceAt(LOOP_PERIOD - 1e-6)).toBeCloseTo(0, 3)
  })

  it('固定时刻 → 状态完全一致（可回放）', () => {
    for (const t of [0, 18, 38, 118, 133, 200, 276]) {
      const a = trainStateAt(0, t)
      const b = trainStateAt(0, t)
      expect(b.t).toBe(a.t)
      expect(b.phase).toBe(a.phase)
      expect(b.speed).toBe(a.speed)
      expect(b.distance).toBe(a.distance)
    }
  })

  it('时间轴拖回同一时刻，位置/速度逐比特一致', () => {
    const t = 123.4
    const fwd = trainStateAt(1, t)
    // 先前进再拖回
    trainStateAt(1, t + 50)
    const back = trainStateAt(1, t)
    expect(back.distance).toBe(fwd.distance)
    expect(back.speed).toBe(fwd.speed)
    expect(back.phase).toBe(fwd.phase)
  })

  it('三列车按周期 1/3 错开，互不重叠停站', () => {
    expect(TRAIN_COUNT).toBe(3)
    // 同一全局时刻，三列车本地时间相差 98s
    for (let t = 0; t < LOOP_PERIOD; t += 20) {
      const states = [0, 1, 2].map((i) => trainStateAt(i, t))
      const phases = states.map((s) => `${s.phase}@${s.t.toFixed(1)}`)
      expect(new Set(phases).size).toBe(TRAIN_COUNT)
    }
  })

  it('wrapTime 对负数与超大数稳定折叠', () => {
    expect(wrapTime(0)).toBe(0)
    expect(wrapTime(LOOP_PERIOD)).toBeCloseTo(0, 6)
    expect(wrapTime(-10)).toBeCloseTo(LOOP_PERIOD - 10, 6)
    expect(wrapTime(LOOP_PERIOD * 3 + 37)).toBeCloseTo(37, 6)
  })
})

describe('schedule — 下一步状态', () => {
  it('停站后下一步是出站加速', () => {
    const dwellT = dwellWindows()[0]
    const now = trainStateAt(0, dwellT.start + 2)
    expect(now.phase).toBe('dwell')
    const nxt = nextTrainState(0, dwellT.start + 2)
    expect(nxt.phase).toBe('departing')
  })

  it('进站制动后下一步是停站', () => {
    const brake = SEGMENTS.find((s) => s.kind === 'brake')!
    const now = trainStateAt(0, brake.t0 + 2)
    expect(now.phase).toBe('arriving')
    const nxt = nextTrainState(0, brake.t0 + 2)
    expect(nxt.phase).toBe('dwell')
  })
})
