import { describe, expect, it } from 'vitest'
import {
  createViewStateMachine,
  VIEW_STATES,
  type DeviceViewState,
} from './ViewStateMachine'

describe('ViewStateMachine — 六态迁移', () => {
  it('初始态为 product', () => {
    const m = createViewStateMachine()
    expect(m.state).toBe('product')
    expect(m.snapshot().progress).toBe(1)
  })

  it('合法迁移：product → open → product', () => {
    const m = createViewStateMachine()
    expect(m.requestTransition('open').ok).toBe(true)
    expect(m.state).toBe('open')
    expect(m.requestTransition('product').ok).toBe(true)
    expect(m.state).toBe('product')
  })

  it('拒绝非法迁移：explode 不能直接到 section', () => {
    const m = createViewStateMachine()
    m.requestTransition('explode')
    const r = m.requestTransition('section')
    expect(r.ok).toBe(false)
    expect(r.reason).toContain('illegal-transition')
  })

  it('inspect 是覆盖层：进入后记录来源态，退出回到来源', () => {
    const m = createViewStateMachine()
    m.requestTransition('open')
    expect(m.enterInspect('shelf-1').ok).toBe(true)
    expect(m.state).toBe('inspect')
    expect(m.inspectReturnedTo).toBe('open')
    // inspect 中禁止普通迁移
    expect(m.requestTransition('product').ok).toBe(false)
    const back = m.exitInspect()
    expect(back.ok).toBe(true)
    expect(back.returnedTo).toBe('open')
    expect(m.state).toBe('open')
  })

  it('重复 enterInspect 被拒绝', () => {
    const m = createViewStateMachine()
    m.enterInspect('a')
    expect(m.enterInspect('b').ok).toBe(false)
  })

  it('tick 指数收敛不突变：progress 单调逼近 target', () => {
    const m = createViewStateMachine()
    m.requestTransition('explode')
    const p0 = m.tick(0.016)
    const p1 = m.tick(0.016)
    expect(p1).toBeGreaterThan(p0)
    expect(p1).toBeLessThanOrEqual(1)
  })

  it('tick 拒绝非法 dt（NaN/负/0），progress 不变', () => {
    const m = createViewStateMachine()
    const before = m.snapshot().progress
    expect(m.tick(NaN)).toBe(before)
    expect(m.tick(-5)).toBe(before)
    expect(m.tick(0)).toBe(before)
  })

  it('六态齐全', () => {
    expect(VIEW_STATES).toHaveLength(6)
    const expected: DeviceViewState[] = ['product', 'open', 'explode', 'section', 'cycle', 'inspect']
    expect([...VIEW_STATES].sort()).toEqual([...expected].sort())
  })
})
