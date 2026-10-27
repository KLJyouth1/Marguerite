// 右侧面板宽度：拖拽 / 键盘微调 / clamp / 持久化。
//
// 背景（用户实测）：右侧面板此前是 `position: fixed; width: 300px` 的浮层，
// 打开时直接压在会话窗上，会话列一点不收缩，也完全不能调宽。竞品（Trae SOLO）
// 是「垂直功能轨 + 可收放面板」，面板宽度由用户决定。
//
// 这里只放**纯逻辑 + 一份共享状态**：
//   · 纯函数（clamp / 解析 / 键盘步长）可独立单测，不依赖组件；
//   · 状态放在模块作用域：RightSidebar（面板本体）与 ChatView（会话列让位）
//     必须看到同一个宽度，否则拖宽面板而会话列不动。
//
// 约束：280px .. 55vw（窄屏下 55vw 可能小于 280，此时以 280 为下限，宁可挤压
// 会话列也不让面板窄到不可用）。
import { computed, ref, type ComputedRef, type Ref } from 'vue'

const WIDTH_KEY = 'marguerite.layout.rightPanelWidth'
export const RIGHT_PANEL_WIDTH_KEY = WIDTH_KEY
export const RIGHT_PANEL_MIN_WIDTH = 280
export const RIGHT_PANEL_MAX_RATIO = 0.55
export const RIGHT_PANEL_KEYBOARD_STEP = 24
/** 首次打开 / 用户点「还原」时的宽度（与旧实现的固定 300px 一致）。 */
export const RIGHT_PANEL_DEFAULT_WIDTH = 300

/** 当前视口宽度（SSR / 测试环境无 window 时给一个保守值）。 */
function viewportWidthNow(): number {
  return typeof window === 'undefined' ? 1440 : window.innerWidth
}

/** 55vw 上限（含 280px 下限保护，返回的永远是合法区间上界）。 */
export function rightPanelMaxWidth(viewportWidth: number): number {
  const ratioMax = Math.round(Math.max(0, viewportWidth) * RIGHT_PANEL_MAX_RATIO)
  return Math.max(RIGHT_PANEL_MIN_WIDTH, ratioMax)
}

/** 把任意输入收敛到 [280px, 55vw]；NaN / 无穷大回落到默认宽度。 */
export function clampRightPanelWidth(value: number, viewportWidth: number): number {
  const max = rightPanelMaxWidth(viewportWidth)
  const fallback = Math.min(RIGHT_PANEL_DEFAULT_WIDTH, max)
  if (!Number.isFinite(value)) return fallback
  return Math.min(Math.max(Math.round(value), RIGHT_PANEL_MIN_WIDTH), max)
}

/** localStorage 原始值 → 合法宽度（缺省 / 脏数据都回落到默认宽度）。 */
export function parseRightPanelWidth(raw: string | null, viewportWidth: number): number {
  if (raw === null || raw.trim() === '') return clampRightPanelWidth(RIGHT_PANEL_DEFAULT_WIDTH, viewportWidth)
  const parsed = Number(raw)
  return clampRightPanelWidth(Number.isFinite(parsed) ? parsed : NaN, viewportWidth)
}

function safeStorage(): Storage | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage
  } catch {
    // 隐私模式 / 被禁用的存储：读写都静默跳过，宽度仍在本会话内生效。
    return null
  }
}

/** 读取持久化宽度（调用方负责视口宽度，便于单测）。 */
export function readRightPanelWidth(viewportWidth: number = viewportWidthNow()): number {
  let raw: string | null = null
  try {
    raw = safeStorage()?.getItem(WIDTH_KEY) ?? null
  } catch {
    raw = null
  }
  return parseRightPanelWidth(raw, viewportWidth)
}

/** 写入持久化宽度（失败不抛：宽度已在内存里生效，不该阻塞交互）。 */
export function persistRightPanelWidth(width: number): void {
  try {
    safeStorage()?.setItem(WIDTH_KEY, String(Math.round(width)))
  } catch {
    /* 忽略：持久化只是跨会话记忆 */
  }
}

/**
 * 键盘微调：分隔线左侧是会话列、右侧是面板。
 * ← 让分隔线左移 → 面板变宽；→ 反之。其它按键返回 null（交给浏览器）。
 */
export function widthAfterKey(
  width: number,
  key: string,
  viewportWidth: number,
): number | null {
  if (key === 'ArrowLeft') return clampRightPanelWidth(width + RIGHT_PANEL_KEYBOARD_STEP, viewportWidth)
  if (key === 'ArrowRight') return clampRightPanelWidth(width - RIGHT_PANEL_KEYBOARD_STEP, viewportWidth)
  return null
}

/* ─── 共享状态（模块作用域单例，见文件头说明） ─── */
const width = ref(readRightPanelWidth())
const viewportWidth = ref(viewportWidthNow())
const resizing = ref(false)
let dragOrigin: { startX: number; startWidth: number } | null = null
let viewportBound = false

function onViewportResize(): void {
  viewportWidth.value = viewportWidthNow()
  // 窗口变小 → 55vw 上限跟着变小：把当前宽度收回来，避免面板吃掉整块会话列。
  width.value = clampRightPanelWidth(width.value, viewportWidth.value)
}

function ensureViewportBound(): void {
  if (viewportBound || typeof window === 'undefined') return
  viewportBound = true
  window.addEventListener('resize', onViewportResize)
}

function onResizeMove(event: PointerEvent): void {
  if (!dragOrigin) return
  // 面板贴右侧：指针往左（clientX 变小）→ 面板变宽。
  const delta = dragOrigin.startX - event.clientX
  width.value = clampRightPanelWidth(dragOrigin.startWidth + delta, viewportWidthNow())
}

function endResize(): void {
  if (!dragOrigin) return
  dragOrigin = null
  resizing.value = false
  window.removeEventListener('pointermove', onResizeMove, { capture: true })
  // 松手才落盘：拖动过程中每帧写 localStorage 既慢又没必要。
  persistRightPanelWidth(width.value)
}

export interface PanelResizeApi {
  width: Ref<number>
  maxWidth: ComputedRef<number>
  resizing: Ref<boolean>
  startResize: (event: PointerEvent) => void
  onResizeKeydown: (event: KeyboardEvent) => void
  setWidth: (next: number) => void
}

/** 右侧面板宽度控制器（RightSidebar 与 ChatView 共享同一份状态）。 */
export function usePanelResize(): PanelResizeApi {
  ensureViewportBound()

  function setWidth(next: number): void {
    width.value = clampRightPanelWidth(next, viewportWidthNow())
    persistRightPanelWidth(width.value)
  }

  function startResize(event: PointerEvent): void {
    // 只认主键；右键/中键不进入拖拽（右键另有菜单语义）。
    if (event.button !== 0) return
    event.preventDefault()
    dragOrigin = { startX: event.clientX, startWidth: width.value }
    resizing.value = true
    window.addEventListener('pointermove', onResizeMove, { capture: true })
    window.addEventListener('pointerup', endResize, { once: true, capture: true })
    window.addEventListener('pointercancel', endResize, { once: true, capture: true })
  }

  function onResizeKeydown(event: KeyboardEvent): void {
    const next = widthAfterKey(width.value, event.key, viewportWidthNow())
    if (next === null) return
    event.preventDefault()
    setWidth(next)
  }

  return {
    width,
    maxWidth: computed(() => rightPanelMaxWidth(viewportWidth.value)),
    resizing,
    startResize,
    onResizeKeydown,
    setWidth,
  }
}
