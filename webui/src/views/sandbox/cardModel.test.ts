import { describe, expect, it } from 'vitest'
import { dwellWindows, nextTrainState, trainStateAt, SEGMENTS } from './schedule'
import { describeTrainCard } from './cardModel'

describe('describeTrainCard — 状态卡文案', () => {
  it('停站时“现在”=dwell 且带车站细节，色调 ok', () => {
    const win = dwellWindows()[0] // A 站
    const now = trainStateAt(0, win.start + 1)
    const next = nextTrainState(0, win.start + 1)
    const card = describeTrainCard(now, next)
    expect(card.nowKey).toBe('sandbox.phase.dwell')
    expect(card.nowDetailKey).toBe('sandbox.station.A')
    expect(card.tone).toBe('ok')
    expect(card.kindLabelKey).toBe('sandbox.kind.train')
    // 停站之后是出站
    expect(card.nextKey).toBe('sandbox.phase.departing')
  })

  it('巡航时“现在”=cruise、无车站细节，色调 info', () => {
    const cruise = SEGMENTS.find((s) => s.kind === 'cruise')!
    const now = trainStateAt(0, cruise.t0 + 5)
    const next = nextTrainState(0, cruise.t0 + 5)
    const card = describeTrainCard(now, next)
    expect(card.nowKey).toBe('sandbox.phase.cruise')
    expect(card.nowDetailKey).toBeUndefined()
    expect(card.tone).toBe('info')
    expect(card.nextKey).toBe('sandbox.phase.arriving')
  })

  it('进站时色调 warn，下一步停站并带目标站', () => {
    const brake = SEGMENTS.find((s) => s.kind === 'brake')!
    const now = trainStateAt(0, brake.t0 + 3)
    const next = nextTrainState(0, brake.t0 + 3)
    const card = describeTrainCard(now, next)
    expect(card.nowKey).toBe('sandbox.phase.arriving')
    expect(card.tone).toBe('warn')
    expect(card.nextKey).toBe('sandbox.phase.dwell')
    expect(card.nextDetailKey).toBe('sandbox.station.B')
  })

  it('出站时色调 accent、无细节', () => {
    const accel = SEGMENTS.find((s) => s.kind === 'accel')!
    const now = trainStateAt(0, accel.t0 + 3)
    const card = describeTrainCard(now, nextTrainState(0, accel.t0 + 3))
    expect(card.nowKey).toBe('sandbox.phase.departing')
    expect(card.nowDetailKey).toBeUndefined()
    expect(card.tone).toBe('accent')
  })
})
