// @vitest-environment happy-dom
// 壁纸 × 主题可读性守卫（纯函数）回归：滑块量程、对比度下限、亮度换算。
import { beforeEach, describe, expect, it } from 'vitest'
import {
  WALLPAPER_MIN_CONTRAST,
  WALLPAPER_PANEL_ALPHA_DEFAULT,
  WALLPAPER_PANEL_ALPHA_MAX,
  WALLPAPER_PANEL_ALPHA_MIN,
  clampPanelAlpha,
  minPanelAlphaForContrast,
  readWallpaperVeil,
  relativeLuminance,
} from '@/utils/wallpaper'

beforeEach(() => {
  localStorage.clear()
})

function contrast(a: number, b: number): number {
  const hi = Math.max(a, b)
  const lo = Math.min(a, b)
  return (hi + 0.05) / (lo + 0.05)
}

describe('面板遮罩强度', () => {
  it('夹紧到允许量程，非法值回默认', () => {
    expect(clampPanelAlpha(0)).toBe(WALLPAPER_PANEL_ALPHA_MIN)
    expect(clampPanelAlpha(999)).toBe(WALLPAPER_PANEL_ALPHA_MAX)
    expect(clampPanelAlpha(Number.NaN)).toBe(WALLPAPER_PANEL_ALPHA_DEFAULT)
    expect(clampPanelAlpha(63.4)).toBe(63)
  })

  it('未持久化时读默认值，持久化后读回', () => {
    expect(readWallpaperVeil()).toBe(WALLPAPER_PANEL_ALPHA_DEFAULT)
    localStorage.setItem('marguerite.wallpaper.veil', '80')
    expect(readWallpaperVeil()).toBe(80)
    localStorage.setItem('marguerite.wallpaper.veil', 'not-a-number')
    expect(readWallpaperVeil()).toBe(WALLPAPER_PANEL_ALPHA_DEFAULT)
  })
})

describe('对比度守卫', () => {
  it('浅色面板 + 深色文字 + 深色壁纸：最小合法不透明度就已达标（不做无谓抬升）', () => {
    const panel = relativeLuminance(1, 1, 1)
    const text = relativeLuminance(0.1, 0.1, 0.1)
    const guard = minPanelAlphaForContrast(panel, text, 0.08)
    expect(guard).toBeLessThanOrEqual(WALLPAPER_PANEL_ALPHA_DEFAULT)
    expect(guard).toBeGreaterThanOrEqual(0)
  })

  it('深色主题 + 亮壁纸：给出一个能达标的中间值', () => {
    const panel = relativeLuminance(0.07, 0.07, 0.08) // 深色面板
    const text = relativeLuminance(0.92, 0.92, 0.94) // 浅色文字
    const guard = minPanelAlphaForContrast(panel, text, 0.85)
    expect(guard).toBeGreaterThan(0)
    expect(guard).toBeLessThanOrEqual(100)
    const mixed = (guard / 100) * panel + (1 - guard / 100) * 0.85
    expect(contrast(mixed, text)).toBeGreaterThanOrEqual(WALLPAPER_MIN_CONTRAST - 0.02)
  })

  it('壁纸亮度未知（渐变壁纸）时不干预', () => {
    expect(minPanelAlphaForContrast(0.9, 0.05, null)).toBe(0)
  })

  it('面板本身已不达标时返回 0（问题不在壁纸，别假装修好了）', () => {
    const panel = relativeLuminance(0.5, 0.5, 0.5)
    const text = relativeLuminance(0.45, 0.45, 0.45)
    expect(contrast(panel, text)).toBeLessThan(WALLPAPER_MIN_CONTRAST)
    expect(minPanelAlphaForContrast(panel, text, 0.6)).toBe(0)
  })
})
