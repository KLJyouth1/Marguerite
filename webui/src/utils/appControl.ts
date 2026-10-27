/**
 * 应用自控（App Control）：让「人 / 外部智能体 / 巡检脚本」用**语义化**方式
 * 操作本应用自己的界面 —— 枚举可交互元素、点击、填写、按键、读取、等待、
 * 逐路由巡检。
 *
 * # 为什么需要它（与既有两条链路的分工）
 *
 * - `computer_use.action`：**像素级**操控整台电脑（截图 → grounding → 键鼠）。
 *   优点是不受 DOM 限制；缺点是慢、易受分辨率/缩放影响，且读不到控件语义。
 * - `browser.*`：操控**外部** Chromium（CDP），用于网页任务。
 * - 本模块：操控**本应用自己**的 DOM。有 `data-testid` / 无障碍名可直接寻址，
 *   不需要坐标，因此稳定、可断言、可批量巡检。
 *
 * # 设计约束
 *
 * 1. **不伪造**：元素清单来自真实 DOM 与真实布局盒；点击/填写构造真实事件，
 *    并回读元素状态作为证据。
 * 2. **可被外部驱动**：`installAppControl()` 把同一套能力挂到
 *    `window.MargueriteAppControl`，浏览器自动化 / 桌面壳脚本可直接调用。
 * 3. **巡检不依赖组件生命周期**：扫描状态放在模块级（`scanState`），
 *    页面切走后扫描继续，回来仍能看到结果。
 */

import { reactive, ref } from 'vue'
import type { Router } from 'vue-router'

export interface AppControlElement {
  /** 稳定 id：优先 data-testid，其次 id，再次 无障碍名 */
  id: string
  role: string
  name: string
  selector: string
  testid?: string
  disabled: boolean
  value?: string
  rect: { x: number, y: number, width: number, height: number }
}

export interface AppControlResult {
  ok: boolean
  selector?: string
  name?: string
  value?: string
  error?: string
}

export interface RouteScanEntry {
  path: string
  ok: boolean
  elements: number
  alerts: string[]
  errors: string[]
  ms: number
}

const INTERACTIVE_SELECTOR = [
  'button',
  'a[href]',
  'input',
  'select',
  'textarea',
  '[role="button"]',
  '[role="tab"]',
  '[role="switch"]',
  '[role="radio"]',
  '[role="checkbox"]',
  '[role="menuitem"]',
  '[role="link"]',
  '[contenteditable="true"]',
  '[data-testid]',
].join(',')

const MAX_ELEMENTS = 400

function cssEscape(value: string): string {
  const g = globalThis as unknown as { CSS?: { escape?: (v: string) => string } }
  if (typeof g.CSS?.escape === 'function') return g.CSS.escape(value)
  return value.replace(/["\\\]]/g, '\\$&')
}

function isVisible(el: Element): boolean {
  const rect = el.getBoundingClientRect()
  if (rect.width <= 0 && rect.height <= 0) return false
  const view = el.ownerDocument?.defaultView
  if (!view) return true
  const style = view.getComputedStyle(el)
  return style.visibility !== 'hidden' && style.display !== 'none'
}

function textOf(el: Element): string {
  return (el.textContent || '').replace(/\s+/g, ' ').trim()
}

/** 无障碍名（近似 accname：aria-label → aria-labelledby → title → 文本 → value） */
export function accessibleName(el: Element): string {
  const doc = el.ownerDocument
  const ariaLabel = el.getAttribute('aria-label')
  if (ariaLabel?.trim()) return ariaLabel.trim()
  const labelledBy = el.getAttribute('aria-labelledby')
  if (labelledBy) {
    const text = labelledBy
      .split(/\s+/)
      .map(id => doc.getElementById(id)?.textContent || '')
      .join(' ')
      .trim()
    if (text) return text
  }
  const title = el.getAttribute('title')
  if (title?.trim()) return title.trim()
  const tag = el.tagName.toLowerCase()
  if (tag === 'input' || tag === 'select' || tag === 'textarea') {
    const value = (el as HTMLInputElement).value
    const placeholder = el.getAttribute('placeholder')
    return textOf(el) || placeholder || value || ''
  }
  return textOf(el)
}

/** 生成可复用的选择器：优先 data-testid（测试/自动化契约），其次 #id，最后路径。 */
export function selectorFor(el: Element): string {
  const testid = el.getAttribute('data-testid')
  if (testid) return `[data-testid="${cssEscape(testid)}"]`
  if (el.id) return `#${cssEscape(el.id)}`
  const ariaLabel = el.getAttribute('aria-label')
  if (ariaLabel) {
    return `${el.tagName.toLowerCase()}[aria-label="${cssEscape(ariaLabel)}"]`
  }
  const parts: string[] = []
  let node: Element | null = el
  while (node && node.nodeType === 1 && parts.length < 6) {
    const tag = node.tagName.toLowerCase()
    if (tag === 'html' || tag === 'body') break
    const parent: Element | null = node.parentElement
    if (!parent) { parts.unshift(tag); break }
    const siblings = Array.from(parent.children).filter(c => c.tagName === node!.tagName)
    const index = siblings.indexOf(node) + 1
    parts.unshift(siblings.length > 1 ? `${tag}:nth-of-type(${index})` : tag)
    node = parent
  }
  return parts.join(' > ') || el.tagName.toLowerCase()
}

function roleOf(el: Element): string {
  const explicit = el.getAttribute('role')
  if (explicit) return explicit
  const tag = el.tagName.toLowerCase()
  if (tag === 'button') return 'button'
  if (tag === 'a') return 'link'
  if (tag === 'select') return 'combobox'
  if (tag === 'textarea') return 'textbox'
  if (tag === 'input') {
    const type = (el as HTMLInputElement).type || 'text'
    if (type === 'checkbox') return 'checkbox'
    if (type === 'radio') return 'radio'
    return 'textbox'
  }
  return tag
}

/** 枚举当前页面的可交互元素（真实 DOM + 真实布局盒，跳过不可见项）。 */
export function listInteractiveElements(root: ParentNode = document): AppControlElement[] {
  const found = Array.from(root.querySelectorAll(INTERACTIVE_SELECTOR)) as Element[]
  const out: AppControlElement[] = []
  for (const el of found) {
    if (out.length >= MAX_ELEMENTS) break
    if (!isVisible(el)) continue
    const rect = el.getBoundingClientRect()
    const testid = el.getAttribute('data-testid') || undefined
    const name = accessibleName(el)
    const selector = selectorFor(el)
    const ariaLabel = el.getAttribute('aria-label') || ''
    out.push({
      id: testid || el.id || ariaLabel || `${roleOf(el)}:${name}`.slice(0, 120),
      role: roleOf(el),
      name,
      selector,
      ...(testid ? { testid } : {}),
      disabled: (el as HTMLButtonElement).disabled === true
        || el.getAttribute('aria-disabled') === 'true',
      value: 'value' in el ? String((el as HTMLInputElement).value ?? '') : undefined,
      rect: {
        x: Math.round(rect.x),
        y: Math.round(rect.y),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
      },
    })
  }
  return out
}

/**
 * 解析目标。支持的写法：
 *  · `testid:foo`        → `[data-testid="foo"]`
 *  · `#id` / `.cls` / CSS 选择器
 *  · 其它字符串           → 依次尝试 data-testid、#id、CSS
 */
export function resolveTarget(target: string, root: ParentNode = document): Element | null {
  const value = target.trim()
  if (!value) return null
  if (value.startsWith('testid:')) {
    return root.querySelector(`[data-testid="${cssEscape(value.slice(7))}"]`)
  }
  try {
    const direct = root.querySelector(value)
    if (direct) return direct
  } catch {
    // 不是合法 CSS —— 继续按 id / testid 猜
  }
  try {
    return root.querySelector(`[data-testid="${cssEscape(value)}"]`)
  } catch {
    return null
  }
}

function mouseEvent(type: string, el: Element): MouseEvent {
  const view = el.ownerDocument?.defaultView || undefined
  const rect = el.getBoundingClientRect()
  return new MouseEvent(type, {
    bubbles: true,
    cancelable: true,
    composed: true,
    view,
    clientX: Math.round(rect.x + rect.width / 2),
    clientY: Math.round(rect.y + rect.height / 2),
  })
}

/** 真实点击：滚动入视 → pointer/mouse 事件 → `el.click()`。 */
export function clickTarget(target: string, root: ParentNode = document): AppControlResult {
  const el = resolveTarget(target, root)
  if (!el) return { ok: false, error: `找不到元素：${target}` }
  const selector = selectorFor(el)
  const name = accessibleName(el)
  try {
    el.scrollIntoView?.({ block: 'center' })
    el.dispatchEvent(mouseEvent('pointerdown', el))
    el.dispatchEvent(mouseEvent('mousedown', el))
    el.dispatchEvent(mouseEvent('mouseup', el))
    ;(el as HTMLElement).click?.()
    return { ok: true, selector, name }
  } catch (error) {
    return { ok: false, selector, name, error: error instanceof Error ? error.message : String(error) }
  }
}

/** 填写：走原生 value setter + input/change 事件，确保 Vue/React 受控组件都收到更新。 */
export function fillTarget(target: string, value: string, root: ParentNode = document): AppControlResult {
  const el = resolveTarget(target, root)
  if (!el) return { ok: false, error: `找不到元素：${target}` }
  const selector = selectorFor(el)
  const tag = el.tagName.toLowerCase()
  try {
    if (tag === 'input' && ['checkbox', 'radio'].includes((el as HTMLInputElement).type)) {
      ;(el as HTMLInputElement).checked = value === 'true' || value === '1'
    } else if (tag === 'input' || tag === 'textarea' || tag === 'select') {
      const proto = tag === 'textarea'
        ? el.ownerDocument.defaultView!.HTMLTextAreaElement.prototype
        : tag === 'select'
          ? el.ownerDocument.defaultView!.HTMLSelectElement.prototype
          : el.ownerDocument.defaultView!.HTMLInputElement.prototype
      const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set
      if (setter) setter.call(el, value)
      else (el as HTMLInputElement).value = value
    } else if ((el as HTMLElement).isContentEditable) {
      el.textContent = value
    } else {
      return { ok: false, selector, error: `元素不可填写：${tag}` }
    }
    el.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    el.dispatchEvent(new Event('change', { bubbles: true, composed: true }))
    return {
      ok: true,
      selector,
      name: accessibleName(el),
      value: 'value' in el ? String((el as HTMLInputElement).value ?? '') : undefined,
    }
  } catch (error) {
    return { ok: false, selector, error: error instanceof Error ? error.message : String(error) }
  }
}

/** 读文本 / 值（用于断言）。 */
export function readTarget(target: string, root: ParentNode = document): AppControlResult {
  const el = resolveTarget(target, root)
  if (!el) return { ok: false, error: `找不到元素：${target}` }
  const tag = el.tagName.toLowerCase()
  const value = (tag === 'input' || tag === 'textarea' || tag === 'select')
    ? String((el as HTMLInputElement).value ?? '')
    : (el.textContent || '').trim()
  return { ok: true, selector: selectorFor(el), name: accessibleName(el), value }
}

/** 发一次按键（默认发给当前焦点；无焦点则发给 body）。 */
export function pressKey(key: string, modifiers: string[] = []): AppControlResult {
  const doc = document
  const target = (doc.activeElement as HTMLElement | null) || doc.body
  if (!target) return { ok: false, error: '没有可接收按键的元素' }
  const init: KeyboardEventInit = {
    key,
    bubbles: true,
    cancelable: true,
    ctrlKey: modifiers.includes('ctrl'),
    shiftKey: modifiers.includes('shift'),
    altKey: modifiers.includes('alt'),
    metaKey: modifiers.includes('meta'),
  }
  try {
    target.dispatchEvent(new KeyboardEvent('keydown', init))
    target.dispatchEvent(new KeyboardEvent('keyup', init))
    return { ok: true, selector: selectorFor(target), name: accessibleName(target) }
  } catch (error) {
    return { ok: false, error: error instanceof Error ? error.message : String(error) }
  }
}

/** 轮询等待元素出现（默认 3s）。 */
export async function waitForTarget(
  target: string,
  timeoutMs = 3000,
  root: ParentNode = document,
): Promise<AppControlResult> {
  const deadline = Date.now() + Math.max(0, timeoutMs)
  let el = resolveTarget(target, root)
  while (!el && Date.now() < deadline) {
    await sleep(80)
    el = resolveTarget(target, root)
  }
  return el
    ? { ok: true, selector: selectorFor(el), name: accessibleName(el) }
    : { ok: false, error: `等待超时：${target}` }
}

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms))
}

// ───────────────────────── 逐路由巡检 ─────────────────────────

export interface AppControlScanState {
  running: boolean
  current: string
  total: number
  entries: RouteScanEntry[]
  startedAt: number | null
  finishedAt: number | null
}

/**
 * 扫描状态放模块级：巡检会 `router.push` 到别的路由，承载它的组件会被卸载；
 * 若状态在组件里，扫描会随卸载一起丢（用户看到的「点一下就没反应」）。
 */
export const scanState = reactive<AppControlScanState>({
  running: false,
  current: '',
  total: 0,
  entries: [],
  startedAt: null,
  finishedAt: null,
})

export interface ScanOptions {
  settleMs?: number
  onEntry?: (entry: RouteScanEntry) => void
}

/**
 * 操作日志放模块级：巡检会 `router.push` 走，承载它的组件会被卸载，
 * 组件内的 `ref([])` 会随之销毁 —— 实测「巡检完成回来，日志是空的」。
 */
export const actionLog = ref<string[]>([])

export function logAction(message: string): void {
  const stamp = new Date().toLocaleTimeString()
  actionLog.value.unshift(`[${stamp}] ${message}`)
  if (actionLog.value.length > 80) actionLog.value.length = 80
}

/**
 * 已知无害的浏览器噪声：`ResizeObserver loop ...` 是布局观察者在同一帧内
 * 反复触发时浏览器抛出的**规范允许**的提示，并不代表应用出错。
 * 实测来源：/device-twin（3D 舞台随容器尺寸变化）在巡检里被误判为失败。
 * 只过滤这一族固定文案，其余错误照旧上报 —— 不做宽泛的"忽略所有警告"。
 */
export function isBenignNoise(message: string): boolean {
  return /ResizeObserver loop (limit exceeded|completed with undelivered notifications)/i.test(message)
}

/**
 * 逐路由巡检：真实导航 → 等待渲染 → 采集「可交互元素数 / role=alert 文案 /
 * 控制台错误」。判据刻意保守：**有 alert 或有未捕获错误即判失败**。
 */
export async function scanRoutes(
  paths: string[],
  navigate: (path: string) => Promise<unknown> | unknown,
  options: ScanOptions = {},
): Promise<RouteScanEntry[]> {
  const settleMs = options.settleMs ?? 700
  const errors: string[] = []
  const originalError = console.error
  const onError = (event: Event) => {
    const message = (event as ErrorEvent).message
    if (message && !isBenignNoise(message)) errors.push(message)
  }
  const onRejection = (event: Event) => {
    const reason = (event as PromiseRejectionEvent).reason
    const message = reason instanceof Error ? reason.message : String(reason)
    if (!isBenignNoise(message)) errors.push(message)
  }

  scanState.running = true
  scanState.entries = []
  scanState.total = paths.length
  scanState.startedAt = Date.now()
  scanState.finishedAt = null

  console.error = (...args: unknown[]) => {
    const message = args.map(arg => (arg instanceof Error ? arg.message : String(arg))).join(' ')
    if (!isBenignNoise(message)) errors.push(message)
    originalError.apply(console, args as [])
  }
  window.addEventListener('error', onError)
  window.addEventListener('unhandledrejection', onRejection)
  logAction(`开始巡检 ${paths.length} 个路由`)

  const results: RouteScanEntry[] = []
  try {
    for (const path of paths) {
      scanState.current = path
      errors.length = 0
      const started = Date.now()
      try {
        await navigate(path)
      } catch (error) {
        errors.push(`导航失败：${error instanceof Error ? error.message : String(error)}`)
      }
      await sleep(settleMs)
      const alerts = Array.from(document.querySelectorAll('[role="alert"]'))
        .map(el => (el.textContent || '').replace(/\s+/g, ' ').trim())
        .filter(Boolean)
        .slice(0, 5)
      const elements = listInteractiveElements().length
      const entry: RouteScanEntry = {
        path,
        ok: alerts.length === 0 && errors.length === 0 && elements > 0,
        elements,
        alerts,
        errors: [...new Set(errors)].slice(0, 5),
        ms: Date.now() - started,
      }
      results.push(entry)
      scanState.entries.push(entry)
      options.onEntry?.(entry)
    }
  } finally {
    console.error = originalError
    window.removeEventListener('error', onError)
    window.removeEventListener('unhandledrejection', onRejection)
    scanState.running = false
    scanState.current = ''
    scanState.finishedAt = Date.now()
    const failed = results.filter(entry => !entry.ok)
    logAction(`巡检完成：${results.length - failed.length} 通过 / ${failed.length} 失败`)
    for (const entry of failed) {
      logAction(`失败 ${entry.path}${entry.alerts.length ? ` · alert: ${entry.alerts[0]}` : ''}${entry.errors.length ? ` · error: ${entry.errors[0]}` : ''}`)
    }
  }
  return results
}

// ───────────────────── 对外暴露（外部智能体可用） ─────────────────────

export interface AppControlApi {
  inventory: () => AppControlElement[]
  click: (target: string) => AppControlResult
  fill: (target: string, value: string) => AppControlResult
  read: (target: string) => AppControlResult
  press: (key: string, modifiers?: string[]) => AppControlResult
  waitFor: (target: string, timeoutMs?: number) => Promise<AppControlResult>
  navigate: (path: string) => Promise<void>
  scan: (paths?: string[]) => Promise<RouteScanEntry[]>
  routes: () => string[]
  state: () => AppControlScanState
}

let routerRef: Router | null = null

/** 可巡检的路由：有组件、非重定向、非动态参数、非启动页。 */
export function scannableRoutes(router: Router): string[] {
  const seen = new Set<string>()
  for (const record of router.getRoutes()) {
    if (!record.components || record.redirect) continue
    const path = record.path
    if (!path.startsWith('/') || path.includes(':') || path === '/splash') continue
    if (path === '/') continue
    seen.add(path)
  }
  return Array.from(seen).sort()
}

/**
 * 安装 `window.MargueriteAppControl` 并记住 router。
 * 由 `main.ts` 在 `app.use(router)` 之后调用一次。
 */
export function installAppControl(router: Router): void {
  routerRef = router
  if (typeof window === 'undefined') return
  const api: AppControlApi = {
    inventory: () => listInteractiveElements(),
    click: target => clickTarget(target),
    fill: (target, value) => fillTarget(target, value),
    read: target => readTarget(target),
    press: (key, modifiers) => pressKey(key, modifiers),
    waitFor: (target, timeoutMs) => waitForTarget(target, timeoutMs),
    navigate: async (path: string) => {
      if (!routerRef) throw new Error('app control 未安装 router')
      await routerRef.push(path)
    },
    routes: () => (routerRef ? scannableRoutes(routerRef) : []),
    scan: async (paths?: string[]) => {
      if (!routerRef) throw new Error('app control 未安装 router')
      const list = paths?.length ? paths : scannableRoutes(routerRef)
      return scanRoutes(list, path => routerRef!.push(path))
    },
    state: () => scanState,
  }
  ;(window as unknown as { MargueriteAppControl?: AppControlApi }).MargueriteAppControl = api
}
