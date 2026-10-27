import { describe, expect, it } from 'vitest'
import {
  CLOUD_SEA_Y,
  HUT_X,
  HUT_Z,
  dawnIntensity,
  hutBaseY,
  pyramidProfile,
  ridgeHeight,
  ropePathPoint,
  slopeUpness,
  sunDirection,
  sunElevation,
} from './ridgeMath'

describe('玛格丽特 · 星夜 山脊数学', () => {
  it('高度场在 0.1 m 粒度上处处有限（不会出现 NaN 把几何撑爆）', () => {
    for (let x = -110; x <= 110; x += 5) {
      for (let z = -90; z <= 40; z += 5) {
        const h = ridgeHeight(x, z)
        expect(Number.isFinite(h)).toBe(true)
        expect(Math.abs(h)).toBeLessThan(60)
      }
    }
  })

  it('金字塔主峰高于营地鞍部，鞍部高于前景肩脊（画面三层结构成立）', () => {
    const summit = ridgeHeight(0, -44)
    const knoll = ridgeHeight(HUT_X, HUT_Z)
    const shoulder = ridgeHeight(-10, 8)
    expect(summit).toBeGreaterThan(knoll)
    expect(knoll).toBeGreaterThan(shoulder)
    expect(summit).toBeGreaterThan(20)
  })

  it('山坳低于云海平面（云海才会在缺口处翻涌出来）', () => {
    // z≈-1 是前景肩脊与营地鞍部之间刻意压出的谷；远处 z≈-85 必须是低平远景，
    // 不能又冒出一座"墙"把云海和天光挡住。
    expect(ridgeHeight(6, -1)).toBeLessThan(CLOUD_SEA_Y)
    expect(ridgeHeight(0, -85)).toBeLessThan(6)
  })

  it('法线 y 分量在 (0,1] 区间：缓坡接近 1，陡壁明显更小', () => {
    const gentle = slopeUpness(0, -44)
    const steep = slopeUpness(44, -44)
    for (const v of [gentle, steep]) {
      expect(v).toBeGreaterThan(0)
      expect(v).toBeLessThanOrEqual(1)
    }
    expect(gentle).toBeGreaterThan(steep)
  })

  it('金字塔剖面在中心最高、在边界归零', () => {
    expect(pyramidProfile(0)).toBeCloseTo(1, 5)
    expect(pyramidProfile(42)).toBe(0)
    expect(pyramidProfile(-42)).toBe(0)
    expect(pyramidProfile(10)).toBeGreaterThan(pyramidProfile(30))
  })

  it('补给站落位略低于地形（不悬空）', () => {
    expect(hutBaseY()).toBeLessThan(ridgeHeight(HUT_X, HUT_Z))
    expect(ridgeHeight(HUT_X, HUT_Z) - hutBaseY()).toBeLessThan(1)
  })

  it('登山队路径贴合地形，且 x 单调向前', () => {
    const a = ropePathPoint(0.1)
    const b = ropePathPoint(0.6)
    expect(b.x).toBeGreaterThan(a.x)
    expect(a.y).toBeCloseTo(ridgeHeight(a.x, a.z), 5)
    expect(ropePathPoint(-1).x).toBe(ropePathPoint(0).x)
    expect(ropePathPoint(2).x).toBe(ropePathPoint(1).x)
  })

  it('太阳高度角：低→高→回落，且循环周期 240 s', () => {
    const deg = (t: number) => (sunElevation(t) * 180) / Math.PI
    expect(deg(0)).toBeCloseTo(-6, 4)
    expect(deg(120)).toBeCloseTo(10, 4)
    expect(deg(240)).toBeCloseTo(-6, 4)
    expect(deg(60)).toBeGreaterThan(deg(0))
  })

  it('晨曦强度：太阳未出时接近 0，跃出山脊后最暖、升高后回落', () => {
    expect(dawnIntensity(0)).toBeLessThan(0.12)
    // t=60 时太阳刚过 +2°：这一瞬最暖
    expect(dawnIntensity(60)).toBeGreaterThan(0.85)
    // 升到高点后回落，但仍保持可感知的暖调
    expect(dawnIntensity(120)).toBeGreaterThanOrEqual(0.5)
    expect(dawnIntensity(120)).toBeLessThanOrEqual(1)
  })

  it('太阳方向是单位向量，且始终在地平线附近偏右', () => {
    for (const t of [0, 60, 120, 180]) {
      const d = sunDirection(t)
      const len = Math.hypot(d.x, d.y, d.z)
      expect(len).toBeCloseTo(1, 5)
      expect(d.x).toBeGreaterThan(0)
    }
  })
})
