import { describe, expect, it } from 'vitest'
import {
  PET_3D_POLAR_MAX,
  PET_3D_POLAR_MIN,
  clampPolar,
  normalizeAzimuth,
  shortestAngleDelta,
  stepAngle,
  viewAzimuth,
} from './views'

describe('3D 视角数学', () => {
  it('四个预设视角的角度互不相同，free 保持当前角度', () => {
    const angles = ['front', 'threeQuarter', 'side', 'back'].map((v) => viewAzimuth(v as 'front'))
    expect(new Set(angles).size).toBe(4)
    expect(viewAzimuth('front')).toBe(0)
    expect(viewAzimuth('back')).toBeCloseTo(Math.PI, 6)
    expect(viewAzimuth('free')).toBeNull()
  })

  it('最短角差在 ±π 处正确回绕（背面切换不绕远路）', () => {
    // 从 -170° 到 +170°：跨过 ±180° 的短边是 -20°（而不是 +340°）
    const from = (-170 * Math.PI) / 180
    const to = (170 * Math.PI) / 180
    const d = shortestAngleDelta(from, to)
    expect(Math.abs(d)).toBeLessThan(Math.PI)
    expect(Math.abs(d)).toBeCloseTo((20 * Math.PI) / 180, 6)
  })

  it('stepAngle 单调逼近且不过冲，帧率无关', () => {
    let coarse = 0
    for (let i = 0; i < 10; i += 1) coarse = stepAngle(coarse, Math.PI, 0.1)
    let fine = 0
    for (let i = 0; i < 100; i += 1) fine = stepAngle(fine, Math.PI, 0.01)
    expect(Math.abs(coarse - fine)).toBeLessThan(0.05)
    expect(coarse).toBeLessThanOrEqual(Math.PI + 1e-9)
  })

  it('极角被夹紧在允许区间', () => {
    expect(clampPolar(0)).toBe(PET_3D_POLAR_MIN)
    expect(clampPolar(9)).toBe(PET_3D_POLAR_MAX)
    expect(clampPolar(Number.NaN)).toBeGreaterThan(0)
  })

  it('方位角归一化到 (-π, π]', () => {
    expect(normalizeAzimuth(3 * Math.PI)).toBeCloseTo(Math.PI, 6)
    expect(normalizeAzimuth(-3 * Math.PI)).toBeCloseTo(Math.PI, 6)
    expect(normalizeAzimuth(0.5)).toBeCloseTo(0.5, 6)
  })
})
