import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import {
  GLASS_PARAM_DEFS,
  clampParam,
  defaultGlassParams,
  normalizeGlassParams,
  writeGlassParamsTo,
} from './useGlassLab'

const def = (key: string) => {
  const found = GLASS_PARAM_DEFS.find((d) => d.key === key)
  if (!found) throw new Error(`missing param def: ${key}`)
  return found
}

describe('useGlassLab 参数模型', () => {
  it('默认值等于每个参数的基准值（与 tokens.css 对齐）', () => {
    const params = defaultGlassParams()
    for (const d of GLASS_PARAM_DEFS) expect(params[d.key]).toBe(d.base)
  })

  it('白玻璃叠层基准与 tokens.css 的 --glass-tint 一致', () => {
    expect(defaultGlassParams().tint).toBe(12)
  })

  it('clampParam：越界夹紧、按 step 对齐、非数值回落基准', () => {
    expect(clampParam(def('tint'), 99)).toBe(20)
    expect(clampParam(def('tint'), -5)).toBe(4)
    expect(clampParam(def('saturate'), 163)).toBe(165)
    expect(clampParam(def('blur'), 'oops')).toBe(34)
    expect(clampParam(def('radius'), '18')).toBe(18)
  })

  it('normalizeGlassParams：忽略未知键与非法值，缺失项用基准值', () => {
    const p = normalizeGlassParams({ tint: 14, blur: 9999, unknown: 1 })
    expect(p.tint).toBe(14)
    expect(p.blur).toBe(48)
    expect(p.radius).toBe(22)
    expect('unknown' in p).toBe(false)
  })

  it('writeGlassParamsTo：把数值按单位写成 CSS 变量', () => {
    const written: Record<string, string> = {}
    const style = {
      setProperty(name: string, value: string) {
        written[name] = value
      },
    }
    writeGlassParamsTo(style, defaultGlassParams())
    expect(written['--glass-tint']).toBe('12%')
    expect(written['--glass-radius']).toBe('22px')
    expect(written['--glass-blur']).toBe('34px')
    expect(Object.keys(written)).toHaveLength(GLASS_PARAM_DEFS.length)
  })

  // 回归（2026-10-24）：面板初始化会把 `base` 以内联变量写到 <html>，
  // 内联优先级高于主题令牌 —— 只改 tokens.css 会出现"改了样式毫无变化"。
  // 这条守卫要求两处逐项相等，谁漂了就红。
  it('每个 base 都与 tokens.css 的 --glass-* 一一相等', () => {
    const here = dirname(fileURLToPath(import.meta.url))
    const tokens = readFileSync(resolve(here, '../themes/liquid-glass/tokens.css'), 'utf8')
    for (const d of GLASS_PARAM_DEFS) {
      const match = tokens.match(new RegExp(`${d.cssVar}\\s*:\\s*([\\d.]+)`))
      expect(match, `tokens.css 缺少 ${d.cssVar}`).toBeTruthy()
      expect(Number(match![1]), `${d.cssVar} 与面板 base 不一致`).toBe(d.base)
    }
  })
})
