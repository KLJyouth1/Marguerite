import { describe, expect, it } from 'vitest'
import {
  athenaeumDetail,
  DEFAULT_ISLAND_SPEC,
  getIslandSpec,
  ISLAND_QUALITY_TIERS,
  ISLAND_SPECS,
  listIslandSpecs,
  validateIslandSpec,
  type IslandModelSpec,
} from './islandSpec'

describe('islandSpec — 内置皮肤合法性', () => {
  it('全部内置 Spec 通过校验（坏 Spec 不得进入渲染器）', () => {
    for (const spec of Object.values(ISLAND_SPECS)) {
      expect(validateIslandSpec(spec), `${spec.id} 校验失败`).toEqual([])
    }
  })

  it('记忆塔模板与落点半径在各皮肤都齐备（否则渲染器会拿不到几何）', () => {
    for (const spec of Object.values(ISLAND_SPECS)) {
      expect(spec.buildings.athenaeum, `${spec.id} 缺少 athenaeum 模板`).toBeTruthy()
      expect(spec.layout.athenaeumRadius).toBeGreaterThan(0)
      // 记忆塔中心 + 半宽必须落在草地上（诚实可视化：建筑不能掉进海里）
      expect(spec.layout.athenaeumRadius + spec.buildings.athenaeum.width / 2).toBeLessThan(spec.terrain.radius)
    }
  })

  it('默认皮肤与查询回落', () => {
    expect(DEFAULT_ISLAND_SPEC.id).toBe('verdant')
    expect(getIslandSpec().id).toBe('verdant')
    expect(getIslandSpec('ember').id).toBe('ember')
    expect(getIslandSpec('does-not-exist')).toBe(DEFAULT_ISLAND_SPEC)
  })

  it('皮肤清单可用于热插拔选择器', () => {
    const ids = listIslandSpecs().map((s) => s.id)
    expect(ids).toContain('verdant')
    expect(ids).toContain('ember')
  })
})

describe('islandSpec — 校验器能抓住真实错误', () => {
  const clone = (): IslandModelSpec => structuredClone(DEFAULT_ISLAND_SPEC)

  it('草地半径非法时报错', () => {
    const spec = clone()
    spec.terrain.radius = 0
    expect(validateIslandSpec(spec).join(';')).toMatch(/terrain\.radius/)
  })

  it('建筑环超出草地半径时报错（建筑会掉进海里）', () => {
    const spec = clone()
    spec.layout.coreRadius = 9
    expect(validateIslandSpec(spec).join(';')).toMatch(/建筑环外径/)
  })

  it('灯塔越界时报错', () => {
    const spec = clone()
    spec.layout.beaconRadius = spec.terrain.radius
    expect(validateIslandSpec(spec).join(';')).toMatch(/beaconRadius/)
  })

  it('记忆塔越界时报错', () => {
    const spec = clone()
    spec.layout.athenaeumRadius = spec.terrain.radius
    expect(validateIslandSpec(spec).join(';')).toMatch(/athenaeumRadius/)
  })

  it('水面高于/等于草地平面时报错', () => {
    const spec = clone()
    spec.water.level = 0
    expect(validateIslandSpec(spec).join(';')).toMatch(/water\.level/)
  })

  it('水面半径小于岛屿外缘时报错', () => {
    const spec = clone()
    spec.water.radius = 8
    expect(validateIslandSpec(spec).join(';')).toMatch(/water\.radius/)
  })

  it('调色板非颜色值时报错', () => {
    const spec = clone()
    spec.palette.grass = -1
    expect(validateIslandSpec(spec).join(';')).toMatch(/palette\.grass/)
    const spec2 = clone()
    spec2.buildings.core.wallColor = 0xfffffff
    expect(validateIslandSpec(spec2).join(';')).toMatch(/buildings\.core\.wallColor/)
  })

  it('质量预算越界时报错', () => {
    const spec = clone()
    spec.quality.high.props = 2
    expect(validateIslandSpec(spec).join(';')).toMatch(/quality\.high\.props/)
    const spec2 = clone()
    spec2.quality.low.waterSegments = 4
    expect(validateIslandSpec(spec2).join(';')).toMatch(/waterSegments/)
  })

  it('档位缺失/倒挂被拒（三档必须齐备且单调，皮肤热插拔不得带病上线）', () => {
    const gone = clone()
    delete (gone.quality as Partial<Record<string, unknown>>).medium
    expect(validateIslandSpec(gone).join(';')).toMatch(/quality\.medium 缺失/)

    // 倒挂：中档比高档更重（档位名不副实）→ 必须被拒
    const inverted = clone()
    inverted.quality.medium.props = clone().quality.high.props + 0.1
    expect(validateIslandSpec(inverted).join(';')).toMatch(/quality\.high\.props 不得低于 quality\.medium\.props/)

    // 中档被误抄成低档（丢了阴影/灯火层次）→ 单调性不报，但「中档必须保留阴影」由下方专用用例守
    const badPref = clone()
    badPref.quality.medium.powerPreference = 'fast' as WebGLPowerPreference
    expect(validateIslandSpec(badPref).join(';')).toMatch(/powerPreference 非法/)
  })

  it('内置三档全部单调且中档保留阴影（中端机的核心收益）', () => {
    expect(ISLAND_QUALITY_TIERS).toEqual(['high', 'medium', 'low'])
    for (const spec of Object.values(ISLAND_SPECS)) {
      for (let i = 0; i + 1 < ISLAND_QUALITY_TIERS.length; i++) {
        const a = spec.quality[ISLAND_QUALITY_TIERS[i]]
        const b = spec.quality[ISLAND_QUALITY_TIERS[i + 1]]
        for (const key of ['maxPixelRatio', 'props', 'particles', 'ornaments', 'lights', 'lightGain', 'stars', 'starSize', 'waterSegments', 'targetFps', 'terrainDetail'] as const) {
          expect(a[key], `${spec.id} ${ISLAND_QUALITY_TIERS[i]}.${key} 应 ≥ ${ISLAND_QUALITY_TIERS[i + 1]}.${key}`).toBeGreaterThanOrEqual(b[key])
        }
      }
      // 三档必须真的不同：中档若等于高档/低档，等于又退回两档
      const mid = spec.quality.medium
      expect(mid.maxPixelRatio).toBeLessThan(spec.quality.high.maxPixelRatio)
      expect(mid.props).toBeGreaterThan(spec.quality.low.props)
      // 中档保留阴影（与 low 的真正分界）；抗锯齿则让给高档（省的是全屏 MSAA 大头）
      expect(mid.shadows).toBe(true)
      expect(spec.quality.low.shadows).toBe(false)
      expect(mid.antialias).toBe(false)
      expect(spec.quality.high.antialias).toBe(true)
    }
  })

  it('雾参数倒挂时报错', () => {
    const spec = clone()
    spec.lighting.fogNear = 100
    spec.lighting.fogFar = 20
    expect(validateIslandSpec(spec).join(';')).toMatch(/fogFar/)
  })
})

describe('islandSpec — 记忆塔装饰预算（质量档差异）', () => {
  it('LOW 档的记忆塔细节少于 HIGH 档，且全部由预算字段推导', () => {
    const high = athenaeumDetail(DEFAULT_ISLAND_SPEC.quality.high)
    const low = athenaeumDetail(DEFAULT_ISLAND_SPEC.quality.low)
    expect(DEFAULT_ISLAND_SPEC.quality.high.ornaments).toBeGreaterThan(DEFAULT_ISLAND_SPEC.quality.low.ornaments)
    expect(high.motes).toBeGreaterThan(low.motes)
    expect(high.pages).toBeGreaterThan(low.pages)
    expect(high.rings).toBeGreaterThanOrEqual(low.rings)
    // 低档仍保留最低可读细节，不会退化到「没有光环」
    expect(low.motes).toBeGreaterThan(0)
    expect(low.rings).toBeGreaterThanOrEqual(1)
  })
})