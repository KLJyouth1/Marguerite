import { describe, it, expect } from 'vitest'
import { validateGdd, type Gdd } from './gddSchema'

/**
 * 合法 GDD 样板：5 种道具全量覆盖（枚举收口的正例基线）。
 * 每个测试从它派生变体，单独破坏一个维度——错误码/路径可逐条断言。
 */
function validGdd(): Record<string, unknown> {
  return {
    version: '1.0.0',
    theme: '玛格丽特小岛',
    pillars: ['工作可视化', '非暴力解谜'],
    props: [
      {
        kind: 'code_crystal',
        interaction: 'analyze',
        geometry: { main: 'octahedron', accent: 'prism', detail: 2, scale: 0.8 },
        maxInstances: 6,
        labelKey: 'code_crystal',
        expReward: 12,
      },
      {
        kind: 'doc_scroll',
        interaction: 'explain',
        geometry: { main: 'cone', accent: 'scroll', detail: 1, scale: 0.7 },
        maxInstances: 6,
        labelKey: 'doc_scroll',
        expReward: 10,
      },
    ],
    quests: [
      { id: 'q1', title: '初次上岛', trigger: 'runStatus=done', rewardRegion: 'r1' },
      { id: 'q2', title: '探索副岛', trigger: 'picked=data_fruit:3', rewardRegion: '' },
    ],
    regions: [{ id: 'r1', name: '工坊湾区', locked: true }],
    balance: { propSpawnIntervalMs: 4000, expCurve: { base: 100, exponent: 1.5 } },
  }
}

describe('validateGdd — 合法契约', () => {
  it('合法 GDD 通过并规范化返回', () => {
    const r = validateGdd(validGdd())
    expect(r.ok).toBe(true)
    expect(r.errors).toEqual([])
    const gdd = r.value as Gdd
    expect(gdd.version).toBe('1.0.0')
    expect(gdd.props).toHaveLength(2)
    expect(gdd.quests).toHaveLength(2)
    expect(gdd.balance.expCurve.base).toBe(100)
  })

  it('边界 maxInstances=64 通过；65 拒绝', () => {
    const raw = validGdd()
    ;(raw.props as Record<string, unknown>[])[0].maxInstances = 64
    expect(validateGdd(raw).ok).toBe(true)

    ;(raw.props as Record<string, unknown>[])[0].maxInstances = 65
    const r = validateGdd(raw)
    expect(r.ok).toBe(false)
    expect(r.errors.some((e) => e.code === 'out_of_range' && e.path === 'props[0].maxInstances')).toBe(true)
  })
})

describe('validateGdd — 必填与枚举收口', () => {
  it('缺 version 拒绝', () => {
    const raw = validGdd()
    delete raw.version
    const r = validateGdd(raw)
    expect(r.ok).toBe(false)
    expect(r.errors.some((e) => e.path === 'version' && e.code === 'type_error')).toBe(true)
  })

  it('semver 非法拒绝（1.0 / v1.0.0 / 空串）', () => {
    for (const bad of ['1.0', 'v1.0.0', '', '1.0.0.0']) {
      const raw = validGdd()
      raw.version = bad
      const r = validateGdd(raw)
      expect(r.ok).toBe(false)
      expect(r.errors.some((e) => e.path === 'version')).toBe(true)
    }
  })

  it('非法 geometry.main 拒绝（枚举收口到 propsSpec 5 种 primitive）', () => {
    const raw = validGdd()
    ;((raw.props as Record<string, unknown>[])[0].geometry as Record<string, unknown>).main = 'torus'
    const r = validateGdd(raw)
    expect(r.ok).toBe(false)
    expect(r.errors.some((e) => e.code === 'invalid_enum' && e.path === 'props[0].geometry.main')).toBe(true)
  })

  it('非法 interaction / kind 拒绝（语义枚举不可外溢）', () => {
    const raw = validGdd()
    ;(raw.props as Record<string, unknown>[])[0].interaction = 'destroy'
    ;(raw.props as Record<string, unknown>[])[1].kind = 'magic_wand'
    const r = validateGdd(raw)
    expect(r.ok).toBe(false)
    expect(r.errors.some((e) => e.path === 'props[0].interaction')).toBe(true)
    expect(r.errors.some((e) => e.path === 'props[1].kind')).toBe(true)
  })
})

describe('validateGdd — 数值边界', () => {
  it('scale<=0 拒绝（0 与负数都拒）', () => {
    for (const bad of [0, -1]) {
      const raw = validGdd()
      ;((raw.props as Record<string, unknown>[])[0].geometry as Record<string, unknown>).scale = bad
      const r = validateGdd(raw)
      expect(r.ok).toBe(false)
      expect(r.errors.some((e) => e.code === 'out_of_range' && e.path === 'props[0].geometry.scale')).toBe(true)
    }
  })

  it('expReward<=0 与刷新间隔越界拒绝', () => {
    const raw = validGdd()
    ;(raw.props as Record<string, unknown>[])[0].expReward = 0
    ;(raw.balance as Record<string, unknown>).propSpawnIntervalMs = 100
    const r = validateGdd(raw)
    expect(r.ok).toBe(false)
    expect(r.errors.some((e) => e.path === 'props[0].expReward')).toBe(true)
    expect(r.errors.some((e) => e.path === 'balance.propSpawnIntervalMs')).toBe(true)
  })
})

describe('validateGdd — id 唯一与交叉引用', () => {
  it('id 重复拒绝（kind/quest/region 各自唯一）', () => {
    const raw = validGdd()
    ;(raw.props as Record<string, unknown>[])[1].kind = 'code_crystal'
    ;(raw.quests as Record<string, unknown>[])[1].id = 'q1'
    ;(raw.regions as Record<string, unknown>[]).push({ id: 'r1', name: '重名区', locked: false })
    const r = validateGdd(raw)
    expect(r.ok).toBe(false)
    expect(r.errors.filter((e) => e.code === 'duplicate_id')).toHaveLength(3)
  })

  it('任务奖励区域悬空引用拒绝', () => {
    const raw = validGdd()
    ;(raw.quests as Record<string, unknown>[])[0].rewardRegion = 'ghost-region'
    const r = validateGdd(raw)
    expect(r.ok).toBe(false)
    expect(r.errors.some((e) => e.code === 'dangling_ref' && e.path === 'quests[0].rewardRegion')).toBe(true)
  })
})

describe('validateGdd — 前向兼容', () => {
  it('未知字段记 warning 不阻断（旧运行时可读新 GDD）', () => {
    const raw = validGdd()
    raw.futureTopField = { experimental: true }
    ;(raw.props as Record<string, unknown>[])[0].futurePropField = 1
    const r = validateGdd(raw)
    expect(r.ok).toBe(true)
    expect(r.warnings.filter((w) => w.code === 'unknown_field')).toHaveLength(2)
    expect(r.warnings.every((w) => w.level === 'warning')).toBe(true)
    expect(r.value).not.toBeNull()
  })
})
