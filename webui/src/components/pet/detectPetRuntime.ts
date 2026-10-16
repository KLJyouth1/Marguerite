/**
 * 桌宠运行时档位探测（v3）— R1 诚实透明：档位只调特效预算，不假装能力差异。
 *
 * 渲染层已是 2D SVG（`petSvg.ts`）：**无 WebGL 也能完整渲染**。这里的 WebGL/硬件
 * 信号只用来决定特效开销（柔光模糊半径 / 粒子数），不影响形象完整度。
 *
 * 决策链（三档质量，见 `petSpec.PET_QUALITY`）：
 * - WebGL2 + 充足硬件 → 'high'（特效全开：柔光铺得开、粒子全量）
 * - WebGL2 但硬件受限 → 'medium'（特效收一档：模糊半径不变、粒子 0.75）
 * - 仅 WebGL1 → 'low'（省电：柔光半径缩到 60%、粒子 0.5）
 * - 无 WebGL（禁 GPU / jsdom / 边缘环境）→ 'fallback'：**SVG 照常完整渲染**，
 *   仅按最低档铺特效（`renderTierOf` 兜底 low）。「精简」的只是特效，不是形象。
 *
 * medium 档补的是一个真实断层：此前「WebGL2 但核心数/内存偏低」与「只有 WebGL1」
 * 被判为同一档 low，中端机只能拿「全开」或「全关」，没有中间选择。
 */

// 让调用方可直接 `import type { PetRuntimeInfo } from './detectPetRuntime'`
export type { PetRuntimeInfo } from './petContract'
import type { PetQuality, PetRenderTier, PetRuntimeInfo } from './petContract'

export interface PetRuntimeOptions {
  /** 覆盖判定（测试/诊断用） */
  override?: () => boolean
}

/** 探测 canvas 是否支持 WebGL（尝试真实创建上下文，避免误报） */
export function detectWebGL(): 'webgl2' | 'webgl1' | 'none' {
  if (typeof document === 'undefined' || typeof document.createElement !== 'function') {
    return 'none'
  }
  try {
    const canvas = document.createElement('canvas')
    const attrs: WebGLContextAttributes = { failIfMajorPerformanceCaveat: true }
    const gl2 = canvas.getContext('webgl2', attrs)
    if (gl2) return 'webgl2'
    const gl1 = canvas.getContext('webgl', attrs) || (canvas.getContext('experimental-webgl') as unknown as WebGLRenderingContext | null)
    if (gl1) return 'webgl1'
  } catch {
    /* 上下文创建抛错 → 视为无 WebGL */
  }
  return 'none'
}

/** 硬件预算：核心数 + 内存（缺省保守假设） */
function hardwareBudget(): { cores: number; memoryGB: number } {
  const nav = navigator as Navigator & { deviceMemory?: number }
  const cores = typeof navigator.hardwareConcurrency === 'number' && navigator.hardwareConcurrency > 0
    ? navigator.hardwareConcurrency
    : 4
  const memoryGB = typeof nav.deviceMemory === 'number' && nav.deviceMemory > 0
    ? nav.deviceMemory
    : 8
  return { cores, memoryGB }
}

/** 探测桌宠运行时档位（一次性调用，结果用于初始化决策） */
export function detectPetRuntime(opts: PetRuntimeOptions = {}): PetRuntimeInfo {
  if (opts.override && !opts.override()) {
    return { quality: 'fallback', reason: 'override-2d', label: '精简模式（2D）' }
  }
  const gl = detectWebGL()
  if (gl === 'none') {
    return { quality: 'fallback', reason: 'no-webgl', label: '精简模式（2D）' }
  }
  const { cores, memoryGB } = hardwareBudget()
  if (gl === 'webgl1') {
    // 只有 WebGL1：不支持部分 WebGL2 特性，且多见于老设备 → 直接省电档
    return { quality: 'low', reason: 'webgl1', label: '轻量特效模式' }
  }
  if (cores >= 4 && memoryGB >= 4) {
    return { quality: 'high', reason: 'webgl2+hw', label: '完整渲染' }
  }
  // WebGL2 可用但 CPU/内存偏低：只收一档特效预算（形象仍完整）
  return { quality: 'medium', reason: 'webgl2+hw-limited', label: '均衡模式' }
}

/**
 * 探测档位 → 渲染档位（构造 `PetSvg` 时用）。
 *
 * `fallback` 不在渲染档位表内：SVG 渲染无 WebGL 依赖、照常完整成形，
 * 这里兜底给最低档，只把特效开销压到最省，避免万一漏判时按「全开」铺特效。
 */
export function renderTierOf(quality: PetQuality): PetRenderTier {
  return quality === 'fallback' ? 'low' : quality
}