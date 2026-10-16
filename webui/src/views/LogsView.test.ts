// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { KeepAlive, computed, createApp, defineComponent, h, nextTick, ref } from 'vue'
import { createMemoryHistory, createRouter } from 'vue-router'

const rpcMocks = vi.hoisted(() => ({
  call: vi.fn(),
  waitForConnection: vi.fn(),
}))

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({
    client: {},
    call: rpcMocks.call,
    waitForConnection: rpcMocks.waitForConnection,
  }),
}))

const messages: Record<string, string> = {
  'usageLogs.logs.breadcrumbLabel': 'Breadcrumb',
  'usageLogs.logs.title': 'Logs',
  'usageLogs.logs.subtitle': 'Gateway logs for diagnosis.',
  'usageLogs.logs.loading': 'Loading logs…',
  'usageLogs.logs.empty': 'No logs have been recorded yet.',
  'usageLogs.logs.loadFailed': 'Could not load logs.',
  'usageLogs.logs.retry': 'Retry',
  'usageLogs.logs.noMatch': 'No lines match the current filter.',
  'usageLogs.logs.fileLogOn': 'File log on',
  'usageLogs.logs.rawOff': 'Raw capture off',
  'nav.overview': 'Overview',
  'nav.logs': 'Logs',
}

vi.mock('vue-i18n', async (importOriginal) => {
  const actual = await importOriginal<typeof import('vue-i18n')>()
  return {
    ...actual,
    useI18n: () => ({
      t: (key: string) => messages[key] ?? key,
    }),
  }
})

vi.mock('@/components/Icon.vue', () => ({
  default: defineComponent({
    name: 'IconStub',
    props: { name: { type: String, default: '' } },
    setup(props) {
      return () => h('span', { 'data-icon': props.name })
    },
  }),
}))

vi.mock('@/components/ControlSwitch.vue', () => ({
  default: defineComponent({
    name: 'ControlSwitchStub',
    setup() {
      return () => h('button', { type: 'button', 'data-testid': 'control-switch' })
    },
  }),
}))

vi.mock('@/components/SupportDiagnosticsMenu.vue', () => ({
  default: defineComponent({
    name: 'SupportDiagnosticsMenuStub',
    setup() {
      return () => h('button', { type: 'button', 'data-testid': 'support-diagnostics' }, 'Support')
    },
  }),
}))

vi.mock('@/components/run/RunTrace.vue', () => ({
  default: defineComponent({ name: 'RunTraceStub', setup: () => () => h('div') }),
}))

vi.mock('@/composables/useFixedWindow', () => ({
  useFixedWindow: <T,>(source: { value: T[] }) => ({
    visible: computed(() => source.value.map((item, index) => ({ item, index }))),
    topPad: computed(() => 0),
    bottomPad: computed(() => 0),
    onScroll: vi.fn(),
    measure: vi.fn(),
    scrollToEnd: vi.fn(),
  }),
}))

import LogsView from './LogsView.vue'

interface MountedLogs {
  el: HTMLElement
  setVisible: (visible: boolean) => Promise<void>
  unmount: () => void
}

const mounted: MountedLogs[] = []

function normalStatus() {
  return {
    gateway_file_log: { enabled: true, path: '/tmp/debug.log' },
    raw_turn_call_log: { enabled: false, source: 'off', directory: { path: '/tmp/logs' } },
  }
}

async function flush() {
  for (let index = 0; index < 8; index++) await Promise.resolve()
  await nextTick()
}

async function mountLogs(): Promise<MountedLogs> {
  const visible = ref(true)
  const Host = defineComponent({
    name: 'LogsKeepAliveHost',
    setup() {
      return () => h(KeepAlive, null, {
        default: () => visible.value ? h(LogsView) : null,
      })
    },
  })
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(Host)
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/logs', component: { render: () => null } },
      { path: '/overview', component: { render: () => null } },
    ],
  })
  await router.push('/logs')
  await router.isReady()
  app.use(router)
  app.mount(el)
  const result: MountedLogs = {
    el,
    async setVisible(nextVisible: boolean) {
      visible.value = nextVisible
      await nextTick()
      await flush()
    },
    unmount() {
      app.unmount()
      el.remove()
    },
  }
  mounted.push(result)
  await flush()
  return result
}

function tailCalls() {
  return rpcMocks.call.mock.calls.filter(([method]) => method === 'logs.tail')
}

// The ERROR chip's live count, i.e. what the operator reads as "how many
// problems do I have". Collapsed repeats must not inflate it.
function errorChipCount(el: HTMLElement): string {
  return el.querySelector('.lg-level-btn--error .lg-level-btn__count')?.textContent?.trim() ?? ''
}

function repeatBadges(el: HTMLElement): string[] {
  return Array.from(el.querySelectorAll('.lg-line__rep')).map(node => node.textContent?.trim() ?? '')
}

function lineRows(el: HTMLElement): HTMLElement[] {
  return Array.from(el.querySelectorAll('.lg-line'))
}

beforeEach(() => {
  vi.useFakeTimers()
  rpcMocks.call.mockReset()
  rpcMocks.waitForConnection.mockReset()
  rpcMocks.waitForConnection.mockResolvedValue(undefined)
  rpcMocks.call.mockImplementation(async (method: string) => {
    if (method === 'logs.status') return normalStatus()
    if (method === 'logs.tail') return { lines: [], cursor: 0 }
    throw new Error(`unexpected RPC method: ${method}`)
  })
  Object.defineProperty(document, 'hidden', { configurable: true, value: false })
  window.localStorage.clear()
  window.matchMedia = vi.fn(() => ({
    matches: true,
    media: '',
    onchange: null,
    addListener: vi.fn(),
    removeListener: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  }))
})

afterEach(() => {
  mounted.splice(0).forEach(item => item.unmount())
  document.body.innerHTML = ''
  vi.clearAllTimers()
  vi.useRealTimers()
  vi.restoreAllMocks()
})

describe('LogsView states', () => {
  it('always renders its header, status labels, and diagnostics action', async () => {
    const { el } = await mountLogs()

    expect(el.querySelector('h1')?.textContent).toBe('Logs')
    const overviewLink = el.querySelector<HTMLAnchorElement>('.lg-breadcrumb__link')
    expect(overviewLink?.textContent?.trim()).toBe('Overview')
    expect(overviewLink?.getAttribute('href')).toBe('/overview')
    expect(el.querySelector('[aria-current="page"]')?.textContent).toBe('Logs')
    expect(el.textContent).toContain('File log on')
    expect(el.textContent).toContain('Raw capture off')
    expect(el.querySelector('[data-testid="support-diagnostics"]')).not.toBeNull()
    expect(el.textContent).toContain('No logs have been recorded yet.')
  })

  it('distinguishes loading from a successful empty response', async () => {
    let resolveTail!: (value: { lines: []; cursor: number }) => void
    rpcMocks.call.mockImplementation((method: string) => {
      if (method === 'logs.status') return Promise.resolve(normalStatus())
      if (method === 'logs.tail') {
        return new Promise(resolve => { resolveTail = resolve })
      }
      return Promise.reject(new Error(`unexpected RPC method: ${method}`))
    })

    const { el } = await mountLogs()
    expect(el.textContent).toContain('Loading logs…')
    expect(el.textContent).not.toContain('No logs have been recorded yet.')

    resolveTail({ lines: [], cursor: 0 })
    await flush()

    expect(el.textContent).not.toContain('Loading logs…')
    expect(el.textContent).toContain('No logs have been recorded yet.')
  })

  it('shows a retryable failure when the initial tail request fails', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    let tailShouldFail = true
    rpcMocks.call.mockImplementation(async (method: string) => {
      if (method === 'logs.status') return normalStatus()
      if (method === 'logs.tail') {
        if (tailShouldFail) throw new Error('tail unavailable')
        return { lines: [], cursor: 0 }
      }
      throw new Error(`unexpected RPC method: ${method}`)
    })

    const { el } = await mountLogs()
    expect(el.textContent).toContain('Could not load logs.')
    expect(el.querySelector('[role="alert"]')).not.toBeNull()
    expect(warn).toHaveBeenCalledTimes(1)

    tailShouldFail = false
    const retry = Array.from(el.querySelectorAll('button'))
      .find(button => button.textContent?.trim() === 'Retry')
    retry?.click()
    await flush()

    expect(el.textContent).not.toContain('Could not load logs.')
    expect(el.textContent).toContain('No logs have been recorded yet.')
    expect(tailCalls()).toHaveLength(2)
  })

  it('keeps buffered lines visible when a later read fails', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    let tailCall = 0
    rpcMocks.call.mockImplementation(async (method: string) => {
      if (method === 'logs.status') return normalStatus()
      if (method === 'logs.tail') {
        tailCall += 1
        if (tailCall === 1) {
          return { lines: [{ level: 'INFO', message: 'gateway ready' }], cursor: 1 }
        }
        if (tailCall === 2) throw new Error('tail unavailable')
        return { lines: [], cursor: 1 }
      }
      throw new Error(`unexpected RPC method: ${method}`)
    })

    const { el } = await mountLogs()
    await vi.advanceTimersByTimeAsync(3_000)
    await flush()

    expect(el.textContent).toContain('gateway ready')
    expect(el.textContent).toContain('Could not load logs.')
    expect(el.querySelector('[role="alert"]')).not.toBeNull()

    const retry = Array.from(el.querySelectorAll('button'))
      .find(button => button.textContent?.trim() === 'Retry')
    retry?.click()
    await flush()

    expect(el.textContent).toContain('gateway ready')
    expect(el.textContent).not.toContain('Could not load logs.')
    expect(warn).toHaveBeenCalledTimes(1)
  })

  it('shows the no-match state only when buffered lines are filtered out', async () => {
    rpcMocks.call.mockImplementation(async (method: string) => {
      if (method === 'logs.status') return normalStatus()
      if (method === 'logs.tail') {
        return { lines: [{ level: 'INFO', message: 'gateway ready' }], cursor: 1 }
      }
      throw new Error(`unexpected RPC method: ${method}`)
    })

    const { el } = await mountLogs()
    expect(el.textContent).toContain('gateway ready')

    const input = el.querySelector<HTMLInputElement>('input[type="search"]')!
    input.value = 'not present'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await vi.advanceTimersByTimeAsync(150)
    await flush()

    expect(el.textContent).toContain('No lines match the current filter.')
    expect(el.textContent).not.toContain('No logs have been recorded yet.')
  })
})

describe('LogsView repeat collapsing', () => {
  const RESET_NOISE = {
    level: 'ERROR',
    message: 'ConnectionResetError: [WinError 10054] 远程主机强迫关闭了一个现有的连接。',
    timestamp: '2026-10-09 21:06:28,207',
  }

  it('collapses identical consecutive lines into one row with a ×N badge', async () => {
    rpcMocks.call.mockImplementation(async (method: string) => {
      if (method === 'logs.status') return normalStatus()
      if (method === 'logs.tail') {
        return { lines: [RESET_NOISE, RESET_NOISE, RESET_NOISE], cursor: 3 }
      }
      throw new Error(`unexpected RPC method: ${method}`)
    })

    const { el } = await mountLogs()

    expect(lineRows(el)).toHaveLength(1)
    expect(repeatBadges(el)).toEqual(['×3'])
  })

  it('reports the collapsed count as a single entry on the level chip', async () => {
    rpcMocks.call.mockImplementation(async (method: string) => {
      if (method === 'logs.status') return normalStatus()
      if (method === 'logs.tail') {
        return { lines: [RESET_NOISE, RESET_NOISE, RESET_NOISE], cursor: 3 }
      }
      throw new Error(`unexpected RPC method: ${method}`)
    })

    const { el } = await mountLogs()

    // The defect: 3 identical harmless lines used to read as "ERROR 3".
    expect(errorChipCount(el)).toBe('1')
  })

  it('keeps alternating levels as separate rows with no badge', async () => {
    rpcMocks.call.mockImplementation(async (method: string) => {
      if (method === 'logs.status') return normalStatus()
      if (method === 'logs.tail') {
        return {
          lines: [
            { level: 'ERROR', message: 'reset noise' },
            { level: 'INFO', message: 'gateway ready' },
          ],
          cursor: 2,
        }
      }
      throw new Error(`unexpected RPC method: ${method}`)
    })

    const { el } = await mountLogs()

    expect(lineRows(el)).toHaveLength(2)
    expect(repeatBadges(el)).toEqual([])
    expect(errorChipCount(el)).toBe('1')
  })

  it('does not merge the same message at a different level', async () => {
    rpcMocks.call.mockImplementation(async (method: string) => {
      if (method === 'logs.status') return normalStatus()
      if (method === 'logs.tail') {
        return {
          lines: [
            { level: 'ERROR', message: 'shared text' },
            { level: 'INFO', message: 'shared text' },
          ],
          cursor: 2,
        }
      }
      throw new Error(`unexpected RPC method: ${method}`)
    })

    const { el } = await mountLogs()

    expect(lineRows(el)).toHaveLength(2)
    expect(repeatBadges(el)).toEqual([])
  })

  it('collapses repeats that arrive across separate polls', async () => {
    let tailCall = 0
    rpcMocks.call.mockImplementation(async (method: string) => {
      if (method === 'logs.status') return normalStatus()
      if (method === 'logs.tail') {
        tailCall += 1
        if (tailCall === 1) return { lines: [RESET_NOISE], cursor: 1 }
        if (tailCall === 2) return { lines: [RESET_NOISE], cursor: 2 }
        return { lines: [], cursor: 2 }
      }
      throw new Error(`unexpected RPC method: ${method}`)
    })

    const { el } = await mountLogs()
    await vi.advanceTimersByTimeAsync(3_000)
    await flush()

    expect(lineRows(el)).toHaveLength(1)
    expect(repeatBadges(el)).toEqual(['×2'])
    expect(errorChipCount(el)).toBe('1')
  })

  it('keeps a collapsed repeat findable through the message filter', async () => {
    rpcMocks.call.mockImplementation(async (method: string) => {
      if (method === 'logs.status') return normalStatus()
      if (method === 'logs.tail') {
        return { lines: [RESET_NOISE, RESET_NOISE], cursor: 2 }
      }
      throw new Error(`unexpected RPC method: ${method}`)
    })

    const { el } = await mountLogs()
    const input = el.querySelector<HTMLInputElement>('input[type="search"]')!
    input.value = '10054'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await vi.advanceTimersByTimeAsync(150)
    await flush()

    expect(lineRows(el)).toHaveLength(1)
    expect(repeatBadges(el)).toEqual(['×2'])
  })
})

describe('LogsView KeepAlive lifecycle', () => {
  it('polls once initially and only polls or listens for visibility while active', async () => {
    const view = await mountLogs()
    expect(tailCalls()).toHaveLength(1)

    await vi.advanceTimersByTimeAsync(3_000)
    await flush()
    expect(tailCalls()).toHaveLength(2)

    await view.setVisible(false)
    const callsWhileHidden = tailCalls().length
    await vi.advanceTimersByTimeAsync(9_000)
    document.dispatchEvent(new Event('visibilitychange'))
    await flush()
    expect(tailCalls()).toHaveLength(callsWhileHidden)

    await view.setVisible(true)
    expect(tailCalls()).toHaveLength(callsWhileHidden + 1)

    document.dispatchEvent(new Event('visibilitychange'))
    await flush()
    expect(tailCalls()).toHaveLength(callsWhileHidden + 2)
  })
})
