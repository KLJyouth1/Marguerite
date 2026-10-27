// @vitest-environment happy-dom
// 侧聊天面板：空态、输入即发送、流式指示、审批横幅 / 直接裁决、失败重试。
//
// RPC 与 /api/approvals 都是 mock 的：这里验证的是**接线**（谁被调用、参数是什么、
// 哪个会话键），真实网关行为由 composable / 网关侧用例各自负责。
import { createApp, nextTick, type App } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, describe, expect, it, vi } from 'vitest'

import i18n from '@/i18n'
import SideChatPanel from './SideChatPanel.vue'
import { useRpcStore } from '@/stores/rpc'
import { sideChatStorageKey } from '@/composables/chat/useSideChatSession'

const MAIN_KEY = 'agent:main:webchat:main'
const SIDE_KEY = 'agent:main:webchat:side-panel'

const apps: App<Element>[] = []

afterEach(() => {
  apps.splice(0).forEach(app => app.unmount())
  document.body.innerHTML = ''
  localStorage.removeItem(sideChatStorageKey(MAIN_KEY))
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

const flush = async () => {
  await new Promise<void>(resolve => setTimeout(resolve, 0))
  await nextTick()
}

interface MountOptions {
  mainSessionKey?: string
  sendImpl?: (params?: Record<string, unknown>) => Promise<unknown>
  /** GET /api/approvals 的 pending 快照 */
  pending?: unknown[]
  resolvePayload?: Record<string, unknown>
}

function mount(options: MountOptions = {}) {
  i18n.global.locale.value = 'en'
  const mainSessionKey = options.mainSessionKey ?? MAIN_KEY
  // Pin the side key so the mocked approval snapshot can address this session.
  localStorage.setItem(sideChatStorageKey(mainSessionKey), SIDE_KEY)

  const el = document.createElement('div')
  document.body.appendChild(el)
  const pinia = createPinia()
  setActivePinia(pinia)

  const handlers = new Map<string, (payload: unknown, meta?: unknown) => void>()
  const calls: Array<[string, Record<string, unknown> | undefined]> = []
  const call = vi.fn(async (method: string, params?: Record<string, unknown>) => {
    calls.push([method, params])
    if (method === 'chat.send') {
      const send: (input?: Record<string, unknown>) => Promise<unknown> =
        options.sendImpl ?? (async () => ({ ok: true }))
      return send(params)
    }
    return {}
  })

  const rpc = useRpcStore()
  ;(rpc as unknown as { call: typeof call }).call = call
  ;(rpc as unknown as {
    on: (event: string, handler: (payload: unknown, meta?: unknown) => void) => () => void
  }).on = (event, handler) => {
    handlers.set(event, handler)
    return () => { handlers.delete(event) }
  }

  const fetchMock = vi.fn(async (_input: unknown, init?: RequestInit) => ({
    ok: true,
    status: 200,
    json: async () => (
      init?.method === 'POST'
        ? (options.resolvePayload ?? { resolved: true, approved: true, resolution: 'approved' })
        : { pending: options.pending ?? [] }
    ),
  }))
  vi.stubGlobal('fetch', fetchMock)

  const app = createApp(SideChatPanel, { mainSessionKey })
  apps.push(app)
  app.use(pinia)
  app.use(i18n)
  app.mount(el)

  return { el, app, call, calls, handlers, fetchMock }
}

function composer(el: HTMLElement): HTMLTextAreaElement {
  return el.querySelector<HTMLTextAreaElement>('[data-testid="side-chat-input"]')!
}

async function typeAndEnter(el: HTMLElement, text: string) {
  const input = composer(el)
  input.value = text
  input.dispatchEvent(new Event('input', { bubbles: true }))
  await nextTick()
  input.dispatchEvent(new KeyboardEvent('keydown', {
    key: 'Enter',
    bubbles: true,
    cancelable: true,
  }))
  await nextTick()
}

function sendParams(calls: Array<[string, Record<string, unknown> | undefined]>) {
  return calls.find(call => call[0] === 'chat.send')?.[1]
}

function approvalPosts(fetchMock: ReturnType<typeof vi.fn>) {
  return fetchMock.mock.calls
    .filter(([, init]) => (init as RequestInit | undefined)?.method === 'POST')
    .map(([, init]) => JSON.parse(String((init as RequestInit).body)))
}

describe('SideChatPanel · 空态与发送', () => {
  it('renders an honest empty state with a disabled composer', async () => {
    const { el } = mount()
    await flush()

    expect(el.querySelector('[data-testid="side-chat-empty"]')?.textContent)
      .toContain('Start a side conversation')
    expect(composer(el).getAttribute('placeholder')).toBe('Ask in the side chat')
    expect(el.querySelector<HTMLButtonElement>('[data-testid="side-chat-send"]')!.disabled).toBe(true)
    expect(el.querySelector('[data-testid="side-chat-approval-banner"]')).toBeNull()
  })

  it('sends on Enter with the side session key and renders the user turn immediately', async () => {
    const { el, calls } = mount()
    await flush()

    await typeAndEnter(el, '  what is this error?  ')

    const params = sendParams(calls)!
    expect(params.sessionKey).toBe(SIDE_KEY)
    expect(params.message).toBe('what is this error?')
    expect(params.queueMode).toBe('followup')
    expect(typeof params.clientRequestId).toBe('string')
    expect(typeof params.clientMessageId).toBe('string')
    expect(el.querySelector('.sc__msg--user')?.textContent).toContain('what is this error?')
    expect(composer(el).value).toBe('')
    // The side turn is streaming, so the header offers Stop.
    expect(el.querySelector('[data-testid="side-chat-stop"]')).toBeTruthy()
  })

  it('keeps Shift+Enter as a newline instead of sending', async () => {
    const { el, calls } = mount()
    await flush()

    const input = composer(el)
    input.value = 'keep typing'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    input.dispatchEvent(new KeyboardEvent('keydown', {
      key: 'Enter',
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    }))
    await nextTick()

    expect(sendParams(calls)).toBeUndefined()
    expect(input.value).toBe('keep typing')
  })

  it('shows streamed text from the side key and settles on done', async () => {
    const { el, handlers } = mount()
    await flush()
    await typeAndEnter(el, 'go')

    handlers.get('session.event.text_delta')?.({ text: 'partial' }, { session: SIDE_KEY })
    await nextTick()
    expect(el.querySelector('[data-testid="side-chat-streaming"]')).toBeTruthy()
    expect(el.querySelector('.sc__msg--assistant')?.textContent).toContain('partial')

    handlers.get('session.event.done')?.({}, { session: SIDE_KEY })
    await nextTick()
    expect(el.querySelector('[data-testid="side-chat-streaming"]')).toBeNull()
    expect(el.querySelector('[data-testid="side-chat-stop"]')).toBeNull()
    expect(el.querySelector('.sc__msg--assistant')?.textContent).toContain('partial')
  })

  it('stops the side turn through chat.abort with the side key', async () => {
    const { el, calls } = mount()
    await flush()
    await typeAndEnter(el, 'long one')

    el.querySelector<HTMLButtonElement>('[data-testid="side-chat-stop"]')!.click()
    await flush()

    const abort = calls.find(call => call[0] === 'chat.abort')?.[1]
    expect(abort).toEqual({ sessionKey: SIDE_KEY, source: 'webui_stop' })
    expect(el.querySelector('[data-testid="side-chat-stop"]')).toBeNull()
  })
})

describe('SideChatPanel · 上下文附件', () => {
  it('attaches the main conversation once and then shows it as attached', async () => {
    const { el, calls, handlers } = mount()
    await flush()

    const chip = el.querySelector<HTMLButtonElement>('[data-testid="side-chat-context"]')!
    expect(chip.textContent).toContain('Attach main conversation')
    chip.click()
    await nextTick()

    await typeAndEnter(el, 'with context')
    expect(sendParams(calls)!.contextRefs).toEqual([{ kind: 'session', id: MAIN_KEY }])
    expect(chip.textContent).toContain('Main conversation context attached')

    handlers.get('session.event.done')?.({}, { session: SIDE_KEY })
    await nextTick()
    await typeAndEnter(el, 'without context')
    const allSends = calls.filter(call => call[0] === 'chat.send')
    expect(allSends[1]?.[1] ?? {}).not.toHaveProperty('contextRefs')
  })
})

describe('SideChatPanel · 失败重试', () => {
  it('shows the failed state and retries without duplicating the user turn', async () => {
    let attempts = 0
    const { el, calls } = mount({
      sendImpl: async () => {
        attempts += 1
        if (attempts === 1) throw new Error('gateway down')
        return { ok: true }
      },
    })
    await flush()
    await typeAndEnter(el, 'retry me')
    await flush()

    const error = el.querySelector('[data-testid="side-chat-error"]')
    expect(error?.textContent).toContain('Side chat failed: gateway down')
    expect(el.querySelectorAll('.sc__msg--user')).toHaveLength(1)

    el.querySelector<HTMLButtonElement>('[data-testid="side-chat-retry"]')!.click()
    await flush()

    const sends = calls.filter(call => call[0] === 'chat.send')
    expect(sends).toHaveLength(2)
    expect(sends[1][1]!.clientRequestId).toBe(sends[0][1]!.clientRequestId)
    expect(sends[1][1]!.clientMessageId).toBe(sends[0][1]!.clientMessageId)
    expect(el.querySelector('[data-testid="side-chat-error"]')).toBeNull()
    expect(el.querySelectorAll('.sc__msg--user')).toHaveLength(1)
  })
})

describe('SideChatPanel · 审批', () => {
  it('surfaces a side-session approval card and resolves through the approvals API', async () => {
    const { el, fetchMock } = mount({
      pending: [{
        id: 'ap-side',
        namespace: 'exec',
        toolName: 'shell',
        command: 'echo hi',
        displayKind: 'run_command',
        displayTarget: 'echo hi',
        sessionKey: SIDE_KEY,
      }],
    })
    await flush()

    expect(el.querySelector('[data-testid="side-chat-approval-banner"]')?.textContent)
      .toContain('needs your approval')
    const card = el.querySelector('.approval-card[data-approval-id="ap-side"]')
    expect(card).toBeTruthy()

    card!.querySelector<HTMLButtonElement>('.btn--primary')!.click()
    await flush()

    expect(approvalPosts(fetchMock)).toContainEqual({
      id: 'ap-side',
      namespace: 'exec',
      approved: true,
      choice: 'allow_once',
    })
    expect(el.querySelector('[data-testid="side-chat-approval-banner"]')).toBeNull()
    expect(el.querySelector('[data-testid="approval-outcome"]')).toBeTruthy()
  })

  it('shows a resolvable fallback card when session.event.approval arrives before the snapshot', async () => {
    const { el, handlers, fetchMock } = mount()
    await flush()

    handlers.get('session.event.approval')?.(
      { approval_id: 'ap-frame', namespace: 'exec', command: 'ls -la' },
      { session: SIDE_KEY },
    )
    await nextTick()

    expect(el.querySelector('[data-testid="side-chat-approval-banner"]')).toBeTruthy()
    const card = el.querySelector('.approval-card[data-approval-id="ap-frame"]')
    expect(card).toBeTruthy()

    card!.querySelector<HTMLButtonElement>('.btn--primary')!.click()
    await flush()

    expect(approvalPosts(fetchMock)).toContainEqual({
      id: 'ap-frame',
      namespace: 'exec',
      approved: true,
      choice: 'allow_once',
    })
    expect(el.querySelector('[data-testid="side-chat-approval-banner"]')).toBeNull()
  })

  it('never surfaces a main-session approval frame', async () => {
    const { el, handlers } = mount()
    await flush()

    handlers.get('session.event.approval')?.(
      { approval_id: 'ap-main', namespace: 'exec', command: 'rm -rf build' },
      { session: MAIN_KEY },
    )
    await nextTick()

    expect(el.querySelector('[data-testid="side-chat-approval-banner"]')).toBeNull()
    expect(el.querySelector('.approval-card')).toBeNull()
  })
})
