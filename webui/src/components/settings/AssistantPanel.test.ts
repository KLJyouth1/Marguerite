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

async function waitForCount(el: HTMLElement, selector: string, count: number): Promise<void> {
  await vi.waitFor(() => {
    expect(el.querySelectorAll(selector).length).toBe(count)
  }, { timeout: 1000, interval: 10 })
}

const ALL_METHODS = [
  'assistant.scope.get',
  'assistant.scope.set',
  'assistant.prefs.list',
  'assistant.prefs.pin',
  'assistant.prefs.unpin',
]

interface PinnedItem { text: string; source: string; at: string }
interface PreferenceItem { key: string; value: string; confidence?: number | null; evidence?: unknown }

interface GatewayState {
  scope: 'global' | 'project'
  workspace: string | null
  pinned: PinnedItem[]
  preferences: PreferenceItem[]
}

function makeState(overrides: Partial<GatewayState> = {}): GatewayState {
  return { scope: 'global', workspace: null, pinned: [], preferences: [], ...overrides }
}

// 一个内存态的假网关：pin/unpin 真的改动 pinned 列表，组件重新读 prefs.list 才能看到结果——
// 这样测试验证的是「面板相信后端的真实返回」，而不是本地猜测。
async function mountPanel(options: { state?: GatewayState; methods?: string[] } = {}) {
  vi.resetModules()
  document.body.innerHTML = ''
  const state = options.state ?? makeState()
  const methods = options.methods ?? ALL_METHODS
  const call = vi.fn(async (method: string, params?: Record<string, unknown>) => {
    if (method === 'assistant.scope.get') {
      return { scope: state.scope, workspace: state.workspace, since: '' }
    }
    if (method === 'assistant.scope.set') {
      state.scope = params?.scope === 'project' ? 'project' : 'global'
      state.workspace = state.scope === 'project' ? String(params?.workspace ?? '') : null
      return { scope: state.scope, workspace: state.workspace, since: '' }
    }
    if (method === 'assistant.prefs.list') {
      return { pinned: [...state.pinned], preferences: [...state.preferences] }
    }
    if (method === 'assistant.prefs.pin') {
      const text = String(params?.text ?? '')
      if (!state.pinned.some(item => item.text === text)) {
        state.pinned.push({ text, source: 'user_profile', at: '1720000000' })
      }
      return { ok: true }
    }
    if (method === 'assistant.prefs.unpin') {
      state.pinned = state.pinned.filter(item => item.text !== params?.text)
      return { ok: true }
    }
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
  const Component = (await import('./AssistantPanel.vue')).default
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
  return { el, state, call, nextTick }
}

afterEach(() => {
  while (mounted.length) mounted.pop()!.app.unmount()
  vi.doUnmock('@/stores/rpc')
  vi.restoreAllMocks()
  document.body.innerHTML = ''
})

describe('AssistantPanel', () => {
  it('renders empty states when nothing is pinned and nothing was learned', async () => {
    const { el } = await mountPanel()
    await waitForText(el, 'Nothing pinned yet')
    await waitForText(el, 'No preferences have been learned yet')
    expect(el.querySelectorAll('[data-testid="assistant-pinned-item"]').length).toBe(0)
    expect(el.querySelectorAll('[data-testid="assistant-preference"]').length).toBe(0)
  })

  it('shows confidence and evidence, then pins a learned preference into the pinned list', async () => {
    const { el, call, state } = await mountPanel({
      state: makeState({
        workspace: 'ws.demo',
        preferences: [{
          key: 'reply_style',
          value: 'concise',
          confidence: 0.82,
          evidence: ['turn-1', 'turn-2'],
        }],
      }),
    })

    await waitForText(el, 'reply_style')
    await waitForText(el, 'Confidence 82%')
    await waitForText(el, '2 evidence item(s)')

    el.querySelector<HTMLButtonElement>('[data-testid="assistant-pin"]')!.click()
    await waitForCount(el, '[data-testid="assistant-pinned-item"]', 1)
    expect(call).toHaveBeenCalledWith('assistant.prefs.pin', { text: 'reply_style: concise' })
    expect(state.pinned).toEqual([
      { text: 'reply_style: concise', source: 'user_profile', at: '1720000000' },
    ])
  })

  it('unpins a pinned item and removes it from the list', async () => {
    const { el, call, state } = await mountPanel({
      state: makeState({
        pinned: [{ text: 'Prefer concise replies', source: 'user_profile', at: '1720000000' }],
      }),
    })

    await waitForText(el, 'Prefer concise replies')
    await waitForCount(el, '[data-testid="assistant-pinned-item"]', 1)

    el.querySelector<HTMLButtonElement>('[data-testid="assistant-unpin"]')!.click()
    await waitForCount(el, '[data-testid="assistant-pinned-item"]', 0)
    await waitForText(el, 'Nothing pinned yet')
    expect(call).toHaveBeenCalledWith('assistant.prefs.unpin', { text: 'Prefer concise replies' })
    expect(state.pinned).toEqual([])
  })

  it('switches the memory scope only with a project workspace path', async () => {
    const { el, call, state } = await mountPanel({
      state: makeState({ workspace: 'ws.demo' }),
    })

    // 网关在 global 状态下也会回报 workspace 时，输入框预填该路径，可直接切换。
    const input = el.querySelector<HTMLInputElement>('[data-testid="assistant-workspace-input"]')!
    expect(input.value).toBe('ws.demo')
    const projectButton = el.querySelector<HTMLButtonElement>('[data-testid="assistant-scope-project"]')!
    expect(projectButton.disabled).toBe(false)
    projectButton.click()
    await vi.waitFor(() => {
      expect(call).toHaveBeenCalledWith('assistant.scope.set', { scope: 'project', workspace: 'ws.demo' })
    }, { timeout: 1000, interval: 10 })
    expect(state.scope).toBe('project')
  })

  it('disables the project scope until a workspace path is provided', async () => {
    const { el, call } = await mountPanel()

    await waitForText(el, 'Enter a project workspace path')
    const projectButton = el.querySelector<HTMLButtonElement>('[data-testid="assistant-scope-project"]')!
    expect(projectButton.disabled).toBe(true)
    projectButton.click()
    await settle()
    expect(call).not.toHaveBeenCalledWith('assistant.scope.set', expect.anything())

    const input = el.querySelector<HTMLInputElement>('[data-testid="assistant-workspace-input"]')!
    input.value = 'D:\\proj\\demo'
    input.dispatchEvent(new Event('input'))
    await vi.waitFor(() => {
      expect(projectButton.disabled).toBe(false)
    }, { timeout: 1000, interval: 10 })
  })

  it('feature-gates gateways without the assistant API instead of calling it', async () => {
    const { el, call } = await mountPanel({ methods: [] })
    await waitForText(el, 'Assistant settings unavailable')
    expect(call).not.toHaveBeenCalled()
  })
})
