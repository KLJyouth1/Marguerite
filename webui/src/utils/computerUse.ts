/**
 * `computer_use.action` 请求/响应契约（纯函数）。
 *
 * 为什么抽成模块：网关侧**已具备**完整的视觉 grounding 闭环——`locate` 动作会截图 →
 * 交给 VLMM 定位 → 回传 `grounding{x,y,confidence,description}`；未配置视觉模型时诚实返回
 * `code:"VISION_UNAVAILABLE"`（绝不静默降级成普通截图）。但前端此前既不提供 `locate` 入口、
 * 也从不发 `vision`，于是这条闭环在产品里**根本走不到**，多屏/分辨率参数也无处透出。
 *
 * 把「哪个动作带哪些参数」「响应怎么解读」收敛到纯函数：既能单测锁定契约，
 * 也避免参数拼装散落在模板里（那正是这类漂移的温床）。
 */

export interface ComputerUseDraft {
  action: string
  x: number
  y: number
  dx: number
  dy: number
  text: string
  key: string
  description: string
  /** 请求视觉 grounding（`vision: true`）；关闭时网关只回「截图已就绪」的诚实降级文案 */
  vision: boolean
  /** 定位成功后自动点击（默认关闭：自动点击是不可逆操作，须显式开启） */
  autoClick: boolean
  /** ScreenSpot-Pro 分块 grounding（高 DPI 截图的小控件定位） */
  patchGrounding: boolean
  mode: CaptureMode
  /** 截图长边上限（>0 时等比缩边，控制发给视觉模型的体积） */
  maxEdge: number
}

export type CaptureMode = 'primary' | 'all' | 'window'

export const CAPTURE_MODES: readonly CaptureMode[] = ['primary', 'all', 'window']

const COORD_ACTIONS = new Set([
  'click',
  'double_click',
  'right_click',
  'middle_click',
  'move',
  'drag',
  'scroll',
])
const TEXT_ACTIONS = new Set(['type', 'clipboard_set'])
const KEY_ACTIONS = new Set(['key', 'key_combo', 'window_focus', 'key_state'])
/** 会采集屏幕的动作：只有它们认得 `mode` / `max_edge` */
const CAPTURE_ACTIONS = new Set(['screenshot', 'locate'])
/** 支持视觉 grounding 的动作（其余动作没有描述定位语义） */
const GROUNDING_ACTIONS = new Set(['locate'])

export function needsCoords(action: string): boolean {
  return COORD_ACTIONS.has(action)
}

export function needsDelta(action: string): boolean {
  return action === 'scroll' || action === 'drag'
}

export function needsText(action: string): boolean {
  return TEXT_ACTIONS.has(action)
}

export function needsKey(action: string): boolean {
  return KEY_ACTIONS.has(action)
}

export function needsCapture(action: string): boolean {
  return CAPTURE_ACTIONS.has(action)
}

export function supportsGrounding(action: string): boolean {
  return GROUNDING_ACTIONS.has(action)
}

/**
 * 组装 RPC 参数。
 *
 * 只发送当前动作真正会消费的键：多余键会让「界面显示的参数」与「实际生效的参数」脱节，
 * 而 `mode`/`max_edge` 发给 click 这类动作纯属误导。
 */
export function buildComputerUseParams(draft: ComputerUseDraft): Record<string, unknown> {
  const params: Record<string, unknown> = { action: draft.action }
  if (needsCoords(draft.action)) {
    params.x = draft.x
    params.y = draft.y
  }
  // `drag` 带终点、`scroll` 带滚轮增量，两者复用同一对输入但载荷键不同
  if (draft.action === 'drag') {
    params.x2 = draft.dx
    params.y2 = draft.dy
  } else if (draft.action === 'scroll') {
    params.dx = draft.dx
    params.dy = draft.dy
  }
  if (needsText(draft.action)) params.text = draft.text
  if (needsKey(draft.action)) params.key = draft.key
  if (needsCapture(draft.action)) {
    params.mode = draft.mode
    if (draft.maxEdge > 0) params.max_edge = Math.round(draft.maxEdge)
  }
  if (supportsGrounding(draft.action)) {
    params.description = draft.description
    if (draft.vision) params.vision = true
    if (draft.autoClick) params.auto_click = true
    if (draft.patchGrounding) params.patch_grounding = true
  }
  return params
}

/** 定位结果（网关 `grounding` 字段的可视化视图） */
export interface GroundingView {
  x: number
  y: number
  confidence?: number
  description?: string
  /** 引擎在 `data` 里以 `[低置信度]` 前缀标注（阈值由请求方决定），此处如实透传 */
  lowConfidence: boolean
}

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}

function finiteNumber(value: unknown): number | undefined {
  if (value === null || value === undefined || value === '') return undefined
  const n = Number(value)
  return Number.isFinite(n) ? n : undefined
}

/** 解析 grounding 结果；缺坐标即视为无结果（不编造占位坐标） */
export function parseGrounding(response: unknown): GroundingView | null {
  const res = record(response)
  const grounding = record(res?.grounding)
  if (!res || !grounding) return null
  const x = finiteNumber(grounding.x)
  const y = finiteNumber(grounding.y)
  if (x === undefined || y === undefined) return null
  const confidence = finiteNumber(grounding.confidence)
  const description = typeof grounding.description === 'string' && grounding.description.trim()
    ? grounding.description
    : undefined
  return {
    x,
    y,
    ...(confidence !== undefined ? { confidence } : {}),
    ...(description !== undefined ? { description } : {}),
    lowConfidence: typeof res.data === 'string' && res.data.includes('[低置信度]'),
  }
}

/** 网关在「请求了视觉但没配视觉模型」时的诚实软失败码 */
export function isVisionUnavailable(response: unknown): boolean {
  return record(response)?.code === 'VISION_UNAVAILABLE'
}
