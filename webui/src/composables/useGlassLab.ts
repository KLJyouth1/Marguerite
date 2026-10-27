/**
 * 液态玻璃的**布局参数面板**（"把所有 UI 元素的位置和大小写成一个面板，手动调，再记参数"）。
 *
 * 设计：每个可调项 = 一个 CSS 变量 + 一个数值区间；面板只负责改数、即时预览、持久化，
 * 并把当前这一组数导出成 JSON（交给设计/开发复现，不靠口头描述）。
 *
 * 为什么参数落在 `--glass-*` 而不是散在各处：
 *   · world.css 只消费这些变量 ⇒ 调参不需要改 CSS、不会出现"改了 A 忘了 B"；
 *   · 主题自带默认值（tokens.css），面板只用内联样式覆盖 <html> ⇒ 换主题自动回到该主题的基准；
 *   · 所有数值都有上下限，越界的持久化数据会被夹紧 ⇒ 不存在"上次调崩了打不开"。
 */
import { reactive } from 'vue'

export const GLASS_PARAM_STORAGE_KEY = 'marguerite.glass-lab'

export interface GlassParamDef {
  /** 参数键（也是持久化 JSON 的字段名） */
  key: string
  /** 实际写入的 CSS 变量 */
  cssVar: string
  min: number
  max: number
  step: number
  /** 数值后接的单位（写入 CSS 变量时拼接） */
  unit: 'px' | '%'
  /** i18n 标签键（`glassLab.<labelKey>`） */
  labelKey: string
  /** 主题里的基准值（与 tokens.css 一致；恢复默认时回到这里） */
  base: number
}

/** 顺序即面板里的展示顺序：先形状，再材质，最后排版。 */
// ⚠️ `base` 必须与 `themes/liquid-glass/tokens.css` 的 `--glass-*` 逐项一致：
// 面板初始化会把 base 写成 `<html>` 内联变量，**内联优先级高于主题令牌**。
// 2026-10-24 就踩过一次：tokens 改到 52%/34px，面板仍把旧的 66%/30px 顶上，
// 结果「改了 CSS 却毫无效果」。
export const GLASS_PARAM_DEFS: readonly GlassParamDef[] = [
  { key: 'radius', cssVar: '--glass-radius', min: 8, max: 40, step: 1, unit: 'px', labelKey: 'radius', base: 22 },
  { key: 'blur', cssVar: '--glass-blur', min: 0, max: 48, step: 1, unit: 'px', labelKey: 'blur', base: 34 },
  { key: 'saturate', cssVar: '--glass-saturate', min: 100, max: 220, step: 5, unit: '%', labelKey: 'saturate', base: 185 },
  { key: 'tint', cssVar: '--glass-tint', min: 4, max: 20, step: 1, unit: '%', labelKey: 'tint', base: 12 },
  { key: 'scrim', cssVar: '--glass-scrim', min: 30, max: 92, step: 2, unit: '%', labelKey: 'scrim', base: 52 },
  { key: 'edge', cssVar: '--glass-edge', min: 8, max: 90, step: 2, unit: '%', labelKey: 'edge', base: 42 },
  { key: 'sheen', cssVar: '--glass-sheen', min: 0, max: 30, step: 1, unit: '%', labelKey: 'sheen', base: 22 },
  { key: 'gap', cssVar: '--glass-gap', min: 16, max: 48, step: 1, unit: 'px', labelKey: 'gap', base: 28 },
  { key: 'stack', cssVar: '--glass-stack', min: 8, max: 40, step: 2, unit: 'px', labelKey: 'stack', base: 20 },
  { key: 'lift', cssVar: '--glass-lift', min: 0, max: 6, step: 1, unit: 'px', labelKey: 'lift', base: 2 },
  { key: 'veil', cssVar: '--glass-text-veil', min: 30, max: 90, step: 2, unit: '%', labelKey: 'veil', base: 58 },
] as const

export type GlassParams = Record<string, number>

export function defaultGlassParams(): GlassParams {
  return Object.fromEntries(GLASS_PARAM_DEFS.map((d) => [d.key, d.base]))
}

/** 夹紧到 [min,max] 并对齐 step；非有限值回落到基准值。 */
export function clampParam(def: GlassParamDef, value: unknown): number {
  const n = typeof value === 'number' ? value : Number(value)
  if (!Number.isFinite(n)) return def.base
  const clamped = Math.min(def.max, Math.max(def.min, n))
  const stepped = Math.round(clamped / def.step) * def.step
  // 对齐 step 后可能越界（例如 max 不是 step 的整数倍），再夹一次
  return Math.min(def.max, Math.max(def.min, Number(stepped.toFixed(2))))
}

/** 把任意来源（localStorage / 导入的 JSON）收敛成一组合法参数。 */
export function normalizeGlassParams(raw: unknown): GlassParams {
  const src = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {}
  const out: GlassParams = {}
  for (const def of GLASS_PARAM_DEFS) {
    out[def.key] = def.key in src ? clampParam(def, src[def.key]) : def.base
  }
  return out
}

export function readGlassParams(): GlassParams {
  try {
    const raw = localStorage.getItem(GLASS_PARAM_STORAGE_KEY)
    return raw ? normalizeGlassParams(JSON.parse(raw)) : defaultGlassParams()
  } catch {
    return defaultGlassParams()
  }
}

/** 参数 → 内联 CSS 变量（可注入任意 style 对象，便于单测）。 */
export function writeGlassParamsTo(
  style: { setProperty(name: string, value: string): void },
  params: GlassParams,
): void {
  for (const def of GLASS_PARAM_DEFS) {
    style.setProperty(def.cssVar, `${params[def.key] ?? def.base}${def.unit}`)
  }
}

export function applyGlassParams(params: GlassParams): void {
  if (typeof document === 'undefined') return
  writeGlassParamsTo(document.documentElement.style, params)
}

export function persistGlassParams(params: GlassParams): void {
  try {
    localStorage.setItem(GLASS_PARAM_STORAGE_KEY, JSON.stringify(params))
  } catch {
    // 持久化失败不阻断调参（当前会话仍然生效）
  }
}

/** 面板的单一状态源：改动即写 `<html>` 并持久化。 */
export const glassParams = reactive<GlassParams>(defaultGlassParams())

export function initGlassParams(): void {
  const stored = readGlassParams()
  Object.assign(glassParams, stored)
  applyGlassParams(glassParams)
}

export function setGlassParam(key: string, value: number): void {
  const def = GLASS_PARAM_DEFS.find((d) => d.key === key)
  if (!def) return
  glassParams[key] = clampParam(def, value)
  applyGlassParams(glassParams)
  persistGlassParams(glassParams)
}

export function resetGlassParams(): void {
  const def = defaultGlassParams()
  Object.assign(glassParams, def)
  applyGlassParams(glassParams)
  persistGlassParams(glassParams)
}

/** 导出给设计/开发复现的 JSON（含 CSS 变量映射，可直接贴进 tokens.css）。 */
export function exportGlassParams(): string {
  const cssVars: Record<string, string> = {}
  for (const def of GLASS_PARAM_DEFS) {
    cssVars[def.cssVar] = `${glassParams[def.key]}${def.unit}`
  }
  return JSON.stringify(
    {
      theme: 'liquid-glass',
      scene: glassSceneId(),
      params: { ...glassParams },
      cssVars,
    },
    null,
    2,
  )
}

// 避免循环依赖：scene 只在导出时读取（两个 composable 互不 import）
function glassSceneId(): string {
  try {
    return localStorage.getItem('marguerite.glass-scene') ?? 'clouds'
  } catch {
    return 'clouds'
  }
}
