// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'

const mounted: Array<{ app: { unmount: () => void }; el: HTMLElement }> = []

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) await Promise.resolve()
  await new Promise(resolve => setTimeout(resolve, 10))
}

async function waitForText(el: HTMLElement, text: string): Promise<void> {
  await vi.waitFor(() => {
    expect(el.textContent).toContain(text)
  }, { timeout: 1000, interval: 10 })
}

function statusResult() {
  return { documents: 3, chunks: 9, engine: 'bm25' }
}

function searchResult() {
  return {
    items: [
      { doc_id: 'doc_1', title: 'fps-guide', chunk: '体素竞速游戏使用三缓冲渲染与固定时间步长保证帧率稳定。', score: 0.82 },
      { doc_id: 'doc_2', title: 'rust-compiler', chunk: '词法分析后进入语法分析，产出 AST。', score: 0.51 },
    ],
  }
}

async function mountPanel(options: { methods?: string[]; call?: (method: string, params: unknown) => Promise<unknown> } = {}) {
  vi.resetModules()
  document.body.innerHTML = ''
  const methods = options.methods ?? ['kb.status', 'kb.search']
  const call = options.call ?? vi.fn(async (method: string) => {
    if (method === 'kb.status') return statusResult()
    if (method === 'kb.search') return searchResult()
    return {}
  })
  const rpc = {
    waitForConnection: vi.fn(async () => {}),
    supportsMethod: vi.fn((method: string) => methods.includes(method)),
    markMethodUnavailable: vi.fn(),
    call,
  }
  vi.doMock('@/stores/rpc', () => ({ useRpcStore: () => rpc }))

  const { createApp, nextTick } = await import('vue')
  const i18n = (await import('@/i18n')).default
  i18n.global.locale.value = 'en'
  const Component = (await import('./KnowledgeBasePanel.vue')).default
  const el = document.createElement('div')
  el.style.setProperty('--dur-base', '0ms')
  el.style.setProperty('--dur-fast', '0ms')
  document.body.appendChild(el)
  const app = createApp(Component)
  app.use(i18n)
  app.mount(el)
  mounted.push({ app, el })
  await settle()
  await nextTick()
  return { el, rpc, call, nextTick }
}

async function typeAndSearch(el: HTMLElement, text: string) {
  const { nextTick } = await import('vue')
  const input = el.querySelector<HTMLInputElement>('[data-testid="kb-search-input"]')!
  input.value = text
  input.dispatchEvent(new Event('input'))
  await nextTick()
  el.querySelector<HTMLButtonElement>('[data-testid="kb-search-button"]')!.click()
  await settle()
}

afterEach(() => {
  while (mounted.length) mounted.pop()!.app.unmount()
  vi.doUnmock('@/stores/rpc')
  vi.restoreAllMocks()
  document.body.innerHTML = ''
})

describe('KnowledgeBasePanel', () => {
  it('loads kb.status statistics on mount', async () => {
    const { el } = await mountPanel()
    await waitForText(el, 'Knowledge base')
    await waitForText(el, '3 documents')
    await waitForText(el, '9 chunks')
  })

  it('searches the knowledge base and renders hits', async () => {
    const call = vi.fn(async (method: string) => {
      if (method === 'kb.status') return statusResult()
      if (method === 'kb.search') return searchResult()
      return {}
    })
    const { el } = await mountPanel({ call })
    await typeAndSearch(el, '三缓冲')
    await waitForText(el, 'fps-guide')
    expect(call).toHaveBeenCalledWith('kb.search', { query: '三缓冲', topK: 8 })
  })

  it('renders an empty state for searches without hits', async () => {
    const call = vi.fn(async (method: string) => {
      if (method === 'kb.status') return statusResult()
      if (method === 'kb.search') return { items: [] }
      return {}
    })
    const { el } = await mountPanel({ call })
    await typeAndSearch(el, 'nothing-here')
    await waitForText(el, 'No matching entries found')
  })

  it('shows an unsupported notice when the gateway lacks kb methods', async () => {
    const { el } = await mountPanel({ methods: [] })
    await waitForText(el, 'Knowledge base unavailable')
  })
})
