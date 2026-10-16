import { describe, expect, it } from 'vitest'
import { makeRng, range, seededScatter, SANDBOX_SEED } from './rng'

describe('rng — 固定种子确定性', () => {
  it('同种子两次生成完全一致', () => {
    const a = makeRng(SANDBOX_SEED)
    const b = makeRng(SANDBOX_SEED)
    for (let i = 0; i < 100; i++) expect(b()).toBe(a())
  })

  it('不同种子产生不同序列', () => {
    const a = makeRng(1)
    const b = makeRng(2)
    let same = true
    for (let i = 0; i < 20; i++) if (a() !== b()) same = false
    expect(same).toBe(false)
  })

  it('seededScatter 同种子同结果（装饰可回放）', () => {
    const x = seededScatter(SANDBOX_SEED, 'windowLights', 30)
    const y = seededScatter(SANDBOX_SEED, 'windowLights', 30)
    expect(y).toEqual(x)
  })

  it('盐值不同 → 不同散布', () => {
    const trees = seededScatter(SANDBOX_SEED, 'trees', 20)
    const lamps = seededScatter(SANDBOX_SEED, 'lamps', 20)
    expect(trees).not.toEqual(lamps)
  })

  it('range 落在区间内', () => {
    const rng = makeRng(SANDBOX_SEED)
    for (let i = 0; i < 50; i++) {
      const v = range(rng, 2, 9)
      expect(v).toBeGreaterThanOrEqual(2)
      expect(v).toBeLessThan(9)
    }
  })
})
