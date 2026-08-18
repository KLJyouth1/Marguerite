// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { App } from 'vue'

const mounted: Array<{ app: App; el: HTMLElement }> = []

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) await Promise.resolve()
  await new Promise(resolve => setTimeout(resolve, 10))
}

async function waitForText(el: HTMLElement, text: string): Promise<void> {
  await vi.waitFor(() => {
    expect(el.textContent).toContain(text)
  }, { timeout: 1000, interval: 10 })
}

const ALL_METHODS = [
  'memory.cross_project.search',
  'memory.cross_project.expand',
  'memory.behavior.list',
  'memory.behavior.summary',
]

function searchResult() {
  return {
    items: [{
      project: 'demo-project',
      file: 'project_memory.md',
      kind: 'project_memory',
      title: 'Demo project notes',
      line: 12,
      excerpt: '修复登录 bug，引入 auth 模块',
      score: 0.623,
      updated_at: '1720000000',
    }],
    root: '~/.trae-cn/memory/projects',
  }
}

function expandResult() {
  return {
    expanded: true,
    project: 'demo-project',
    file: 'project_memory.md',
    line: 12,
    hit: '修复登录 bug，引入 auth 模块',
    context: [
      { line: 10, text: '背景：登录流程' },
      { line: 11, text: '进展：session 持久化' },
      { line: 12, text: '修复登录 bug，引入 auth 模块' },
      { line: 13, text: '关键决策：JWT 过期策略' },
    ],
  }
}

function behaviorSummaryResult() {
  return {
    session: 'ws.test',
    events: 5,
    turns: 1,
    user_messages: 1,
    assistant_replies: 1,
    tool_uses: [
      { tool: 'edit_file', count: 2 },
      { tool: 'cargo_test', count: 1 },
    ],
    last_kind: 'assistant_reply',
    last_at_ms: 1720000000000,
  }
}

function behaviorListResult() {
  return {
    session: 'ws.test',
    items: [
      { seq: 1, at_ms: 1720000000000, session: 'ws.test', kind: 'user_message', detail: '修复登录 bug' },
      { seq: 2, at_ms: 1720000000001, session: 'ws.test', kind: 'tool_call', detail: 'edit_file(auth.rs)' },
      { seq: 3, at_ms: 1720000000002, session: 'ws.test', kind: 'assistant_reply', detail: '已完成' },
    ],
  }
}

async function mountPanel(options: {
  methods?: string[]
  call?: ReturnType<typeof vi.fn>
} = {}) {
  vi.resetModules()
  document.body.innerHTML = ''
  const methods = options.methods ?? ALL_METHODS
  const call = options.call ?? vi.fn(async (method: string) => {
    if (method === 'memory.behavior.summary') return behaviorSummaryResult()
    if (method === 'memory.behavior.list') return behaviorListResult()
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
  const Component = (await import('./CrossProjectMemoryPanel.vue')).default
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
  const input = el.querySelector<HTMLInputElement>('[data-testid="cross-memory-search-input"]')!
  input.value = text
  input.dispatchEvent(new Event('input'))
  await nextTick()
  el.querySelector<HTMLButtonElement>('[data-testid="cross-memory-search-button"]')!.click()
  await settle()
}

afterEach(() => {
  while (mounted.length) mounted.pop()!.app.unmount()
  vi.doUnmock('@/stores/rpc')
  vi.restoreAllMocks()
  document.body.innerHTML = ''
})

describe('CrossProjectMemoryPanel', () => {
  it('loads behavior summary and timeline on mount', async () => {
    const { el } = await mountPanel()
    await waitForText(el, 'Session behavior')
    await waitForText(el, 'edit_file')
    await waitForText(el, '修复登录 bug')
  })

  it('searches across project memories and expands context progressively', async () => {
    const call = vi.fn(async (method: string) => {
      if (method === 'memory.behavior.summary') return behaviorSummaryResult()
      if (method === 'memory.behavior.list') return behaviorListResult()
      if (method === 'memory.cross_project.search') return searchResult()
      if (method === 'memory.cross_project.expand') return expandResult()
      return {}
    })
    const { el } = await mountPanel({ call })

    await typeAndSearch(el, '登录 bug')
    await waitForText(el, 'Demo project notes')
    expect(call).toHaveBeenCalledWith('memory.cross_project.search', { query: '登录 bug', limit: 8 })

    // 渐进披露：默认仅摘要级，展开后才拉取上下文
    expect(call).not.toHaveBeenCalledWith('memory.cross_project.expand', expect.anything())
    el.querySelector<HTMLButtonElement>('[data-testid="cross-memory-hit"]')!.click()
    await waitForText(el, '关键决策：JWT 过期策略')
    expect(call).toHaveBeenCalledWith('memory.cross_project.expand', {
      project: 'demo-project',
      file: 'project_memory.md',
      line: 12,
    })
  })

  it('renders empty state for searches without hits', async () => {
    const call = vi.fn(async (method: string) => {
      if (method === 'memory.behavior.summary') return behaviorSummaryResult()
      if (method === 'memory.behavior.list') return behaviorListResult()
      if (method === 'memory.cross_project.search') return { items: [], root: '' }
      return {}
    })
    const { el } = await mountPanel({ call })

    await typeAndSearch(el, '不存在')
    await waitForText(el, 'No relevant memories found')
  })

  it('feature-gates old gateways without calling unavailable methods', async () => {
    const call = vi.fn(async () => ({}))
    const { el } = await mountPanel({ methods: [], call })
    await waitForText(el, 'Cross-project memory unavailable')
    expect(call).not.toHaveBeenCalled()
  })
})
