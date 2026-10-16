/**
 * 岛屿运行时能力探测 —— 与桌宠同一套「诚实降级」口径，但门槛更高：
 * 岛屿是整场景渲染（地形+水+多建筑+灯光），比单只桌宠重得多。
 *
 * - WebGL2 + 足够硬件 → 'high'（阴影/抗锯齿/2× 采样/全量装饰）
 * - WebGL2 但硬件吃紧 → 'medium'（**保留阴影与灯火层次**，只降抗锯齿/采样率/装饰密度）
 * - 仅 WebGL1 → 'low'（关阴影、关抗锯齿、采样率 1、降帧率，省电）
 * - 无 WebGL → 'fallback'：**不创建 3D**，由视图走 2D 示意图并诚实标注「精简模式」
 *
 * 三档补的是一个真实断层：此前「WebGL2 但核心数/内存偏低」与「只有 WebGL1」被判成
 * 同一档 low，中端机只能在「全开」与「全关」之间二选一（与桌宠修复前的缺陷同类）。
 */

import { detectWebGL } from '../pet/detectPetRuntime'
import type { IslandQualityTier } from './islandSpec'

export type IslandRuntimeQuality = IslandQualityTier | 'fallback'

export type IslandRuntimeReason =
  | 'webgl2+hw'
  | 'webgl2+hw-limited'
  | 'webgl1'
  | 'no-webgl'
  | 'override-2d'

export interface IslandRuntimeInfo {
  quality: IslandRuntimeQuality
  reason: IslandRuntimeReason
  /** i18n 键（island.quality.*） */
  labelKey: string
}

export interface IslandRuntimeOptions {
  /** 显式禁用 3D（测试/诊断/用户偏好）。返回 false 即强制降级 */
  override?: () => boolean
  /** 注入 WebGL 探测（测试缝；缺省用真实探测） */
  probe?: () => 'webgl2' | 'webgl1' | 'none'
  /** 注入硬件预算（测试缝） */
  budget?: () => { cores: number; memoryGB: number }
}

function hardwareBudget(): { cores: number; memoryGB: number } {
  const nav = navigator as Navigator & { deviceMemory?: number }
  const cores =
    typeof navigator !== 'undefined' && typeof navigator.hardwareConcurrency === 'number' && navigator.hardwareConcurrency > 0
      ? navigator.hardwareConcurrency
      : 4
  const memoryGB = typeof nav?.deviceMemory === 'number' && nav.deviceMemory > 0 ? nav.deviceMemory : 8
  return { cores, memoryGB }
}

/** 探测岛屿渲染档位（一次性调用，结果决定初始化路径） */
export function detectIslandRuntime(opts: IslandRuntimeOptions = {}): IslandRuntimeInfo {
  if (opts.override && !opts.override()) {
    return { quality: 'fallback', reason: 'override-2d', labelKey: 'island.quality.fallback' }
  }
  const gl = (opts.probe ?? detectWebGL)()
  if (gl === 'none') {
    return { quality: 'fallback', reason: 'no-webgl', labelKey: 'island.quality.fallback' }
  }
  if (gl === 'webgl1') {
    // 只有 WebGL1：不支持部分 WebGL2 特性，且多见于老设备 → 直接省电档
    return { quality: 'low', reason: 'webgl1', labelKey: 'island.quality.low' }
  }
  const { cores, memoryGB } = (opts.budget ?? hardwareBudget)()
  if (cores >= 4 && memoryGB >= 4) {
    return { quality: 'high', reason: 'webgl2+hw', labelKey: 'island.quality.high' }
  }
  // WebGL2 可用但 CPU/内存偏低：保留阴影与灯火，只降抗锯齿/采样率/装饰密度
  return { quality: 'medium', reason: 'webgl2+hw-limited', labelKey: 'island.quality.medium' }
}

/**
 * 探测档位 → 3D 渲染档位（构造 `Island3D` 时用）。
 *
 * `fallback` 不在渲染档位表内：无 WebGL 时调用方走 2D 示意图、根本不建渲染器；
 * 这里兜底给最低档，避免万一漏判时按「全开」渲染。
 * 视图必须经此函数透传 —— 此前视图里写的是 `quality === 'high' ? 'high' : 'low'`，
 * 会把 medium 静默压回两档（档位白加）。
 */
export function renderTierOf(quality: IslandRuntimeQuality): IslandQualityTier {
  return quality === 'fallback' ? 'low' : quality
}