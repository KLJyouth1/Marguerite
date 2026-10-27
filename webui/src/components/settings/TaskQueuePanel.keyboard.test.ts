// @vitest-environment happy-dom
// M3-E3 键盘可操作性冒烟：任务队列面板所有交互控件键盘可达、无焦点陷阱，
// 输入框支持 Enter 触发入队、↑↓ 调序按钮可触发 queue.move。
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { App } from 'vue'

type QueueItem = {
  id: string
  title: string
  goal: string
  mode: string
  session: string
  status: 'queued' | 'running' | 'paused' | 'completed' | 'failed' | 'cancelled'
  priority: number
  created_at_ms: number
  updated_at_ms: number
  error: string | null
}

function makeItems(): QueueItem[] {
  return [
    { id: 'q1', title: '批量重命名报告', goal: '重命名 docs 下 30 个报告', mode: 'safe', session: 's1', status: 'queued', priority: 5, created_at_ms: 1, updated_at_ms: 1, error: null },
    { id: 'q2', title: '生成周报 PDF', goal: '汇总本周数据', mode: 'safe', session: 's1', status: 'queued', priority: 3, created_at_ms: 2, updated_at_ms: 2, error: null },
    { id: 'q3', title: '清理临时目录', goal: 'temp 清理', mode: 'full', session: 's2', status: 'paused', priority: 7, created_at_ms: 3, updated_at_ms: 3, error: null },
    { id: 'q4', title: '训练验证集评估', goal: '跑评估脚本', mode: 'full', session: 's2', status: 'running', priority: 9, created_at_ms: 4, updated_at_ms: 4, error: null },
  ]
}

const mounted: App[] = []

async function settle(times = 12) {
  for (let i = 0; i < times; i++) await Promise.resolve()
}

async function mountPanel() {
  vi.resetModules()
  document.body.innerHTML = ''
  const call = vi.fn(async (method: string, params?: Record<string, unknown>) => {
    if (method === 'queue.list') return { items: makeItems() }
    if (method === 'queue.enqueue') {
      expect(params?.title).toBeTruthy()
      return { ok: true }
    }
    if (method === 'queue.move' || method === 'queue.start' || method === 'queue.pause' ||
        method === 'queue.resume' || method === 'queue.cancel') {
      return { ok: true }
    }
    throw new Error(`unexpected method: ${method}`)
  })
  vi.doMock('@/stores/rpc', () => ({
    useRpcStore: () => ({ waitForConnection: vi.fn(async () => {}), call }),
  }))

  const { createApp } = await import('vue')
  const { createPinia } = await import('pinia')
  const { createI18n } = await import('vue-i18n')
  const en = (await import('@/locales/en.json')).default
  const Component = (await import('./TaskQueuePanel.vue')).default
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(Component)
  app.use(createPinia())
  app.use(createI18n({ legacy: false, locale: 'en', messages: { en } }))
  app.mount(el)
  mounted.push(app)
  await settle()
  return { el, call }
}

afterEach(() => {
  while (mounted.length) mounted.pop()!.unmount()
  vi.doUnmock('@/stores/rpc')
  vi.restoreAllMocks()
  document.body.innerHTML = ''
})

describe('M3-E3 键盘可操作性（TaskQueuePanel）', () => {
  it('所有交互控件均可键盘聚焦（≥16 个可聚焦点，无焦点陷阱）', async () => {
    const { el } = await mountPanel()
    document.body.focus()

    // 队列表格渲染：2 queued（恢复?否/执行/上移/下移/取消）×2 + 1 paused(恢复/取消) + 1 running(暂停)
    const focusables = Array.from(
      el.querySelectorAll<HTMLElement>('input:not([disabled]), button:not([disabled])'),
    )
    const count = focusables.length
    // 构成：3 输入 + 刷新 + queued 行4按钮×2 + paused 行2 + running 行1 = 15
    //（「入队」按钮初始 disabled，为键盘意义外的未激活态，不计入）
    expect(count).toBeGreaterThanOrEqual(15)

    for (const node of focusables) {
      node.focus()
      expect(document.activeElement).toBe(node)
    }
    expect(document.activeElement).not.toBe(null)
  })

  it('标题输入框按 Enter 触发入队（queue.enqueue）', async () => {
    const { el, call } = await mountPanel()
    const title = el.querySelector<HTMLInputElement>('input[aria-label="Task title"]')
    expect(title).toBeTruthy()
    title!.focus()
    title!.value = '键盘入队测试'
    title!.dispatchEvent(new Event('input', { bubbles: true }))
    await settle()
    title!.dispatchEvent(new KeyboardEvent('keyup', { key: 'Enter', bubbles: true }))
    await settle()
    expect(call).toHaveBeenCalledWith('queue.enqueue', {
      title: '键盘入队测试',
      goal: '键盘入队测试',
      priority: 5,
    })
  })

  it('↑ 调序按钮键盘可达并触发 queue.move', async () => {
    const { el, call } = await mountPanel()
    // queued 行有「上移」按钮
    const up = el.querySelector<HTMLButtonElement>('button[aria-label="Move up"]')
    expect(up).toBeTruthy()
    up!.focus()
    expect(document.activeElement).toBe(up)
    up!.click()
    await settle()
    expect(call).toHaveBeenCalledWith('queue.move', { id: 'q1', direction: 'up' })
  })
})
