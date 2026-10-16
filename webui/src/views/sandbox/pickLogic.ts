/**
 * 点选纯逻辑 —— 把「是不是点击」「命中的是哪列车」从 three 场景里抽出来。
 *
 * 这层不依赖 three / DOM / vue，可在 node 单测里直接跑。
 * 场景只负责把屏幕坐标喂进来，这里给判定结果；这样「微抖算点击还是拖拽」
 * 「空白处点击要不要取消选中」都是可钉死的纯函数，不靠人肉点。
 */

/** 判定为「点击」而非「拖拽」的最大位移（CSS px）。 */
export const CLICK_MOVE_THRESHOLD_PX = 5

/** 屏幕上一个指针位置（CSS px，相对视口）。 */
export interface PointerPos {
  x: number
  y: number
}

/** 平方距离（避免开方；与阈值平方比较）。 */
export function squaredPixelDistance(a: PointerPos, b: PointerPos): number {
  const dx = a.x - b.x
  const dy = a.y - b.y
  return dx * dx + dy * dy
}

/**
 * 按下→松开的总位移是否仍在点击容差内。
 * 在容差内 = 一次点选（应做 raycast）；超出 = 一次拖拽（应留给轨道相机，不做点选）。
 */
export function isClickTap(
  down: PointerPos,
  up: PointerPos,
  threshold: number = CLICK_MOVE_THRESHOLD_PX,
): boolean {
  return squaredPixelDistance(down, up) <= threshold * threshold
}

/**
 * 按下期间，当前指针位移是否已越过拖拽阈值。
 * 一旦越过，才允许轨道相机开始转镜头；之前的微抖不转镜头 ——
 * 否则手指 1px 的自然抖动就会把列车从光标下推开，导致「点不中」。
 */
export function exceedsDragThreshold(
  down: PointerPos,
  current: PointerPos,
  threshold: number = CLICK_MOVE_THRESHOLD_PX,
): boolean {
  return !isClickTap(down, current, threshold)
}

/**
 * 从列车 rig 的 userData.entityId（形如 "train:2"）解出 0 基列车下标。
 * 空白处点击 / 未命中 / 格式不符 → 返回 null（调用方据此取消选中）。
 */
export function trainIndexFromEntityId(entityId: unknown): number | null {
  if (typeof entityId !== 'string') return null
  const m = /^train:(\d+)$/.exec(entityId)
  if (!m) return null
  const idx = Number(m[1])
  return Number.isSafeInteger(idx) ? idx : null
}
