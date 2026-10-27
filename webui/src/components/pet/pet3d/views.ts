/**
 * 3D 桌宠多视角：预设方位角与平滑推进的纯函数。
 *
 * 为什么单独成文件：视角是「产品可验收的能力」（用户要求正面/侧面/背面切换），
 * 把角度表与插值数学从渲染器里抽出来，才能在 Node 里单测，不依赖 WebGL。
 */

export type Pet3DView = 'front' | 'threeQuarter' | 'side' | 'back' | 'free'

/** 预设视角的方位角（弧度，0 = 正对镜头；负值向角色左侧转）。 */
export const PET_3D_VIEW_ANGLES: Record<Exclude<Pet3DView, 'free'>, number> = {
  front: 0,
  threeQuarter: -Math.PI / 4,
  side: -Math.PI / 2,
  back: Math.PI,
}

export const PET_3D_VIEW_LABELS: Record<Pet3DView, string> = {
  front: '正面',
  threeQuarter: '侧面 45°',
  side: '正侧面',
  back: '背面',
  free: '自由视角',
}

/** 竖直极角基准与允许区间（1.57 = 水平；越小越俯视） */
export const PET_3D_POLAR_BASE = 1.40
export const PET_3D_POLAR_MIN = 1.05
export const PET_3D_POLAR_MAX = 1.78

export function clampPolar(value: number): number {
  if (!Number.isFinite(value)) return PET_3D_POLAR_BASE
  return Math.min(PET_3D_POLAR_MAX, Math.max(PET_3D_POLAR_MIN, value))
}

/** 预设视角 → 方位角；`free` 返回 null（保持用户当前角度）。 */
export function viewAzimuth(view: Pet3DView): number | null {
  if (view === 'free') return null
  return PET_3D_VIEW_ANGLES[view]
}

/** 两角之间的最短差值（结果 ∈ (-π, π]）——避免背面切换绕远路。 */
export function shortestAngleDelta(from: number, to: number): number {
  const twoPi = Math.PI * 2
  let d = (to - from) % twoPi
  if (d > Math.PI) d -= twoPi
  if (d <= -Math.PI) d += twoPi
  return d
}

/**
 * 指数逼近目标角（帧率无关），每帧最多逼近剩余差值的 (1 - e^(-rate·dt))。
 * 差距 < 0.0015 rad 时直接落到目标，避免「永远差一点」导致每帧都在渲染变化。
 */
export function stepAngle(current: number, target: number, dt: number, rate = 9): number {
  if (!Number.isFinite(current)) return target
  if (!Number.isFinite(target)) return current
  const k = 1 - Math.exp(-Math.max(0, dt) * Math.max(0.0001, rate))
  const delta = shortestAngleDelta(current, target)
  if (Math.abs(delta) < 0.0015) return target
  return current + delta * k
}

/** 当前方位角 → 归一化到 (-π, π]，供状态展示与测试断言。 */
export function normalizeAzimuth(value: number): number {
  const twoPi = Math.PI * 2
  let v = value % twoPi
  if (v > Math.PI) v -= twoPi
  if (v <= -Math.PI) v += twoPi
  return v
}
