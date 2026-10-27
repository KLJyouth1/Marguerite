// @vitest-environment node
// 侧聊天会话：发送参数、严格键过滤、done/abort、一次性 contextRefs、重试与陈旧响应。
//
// 这些断言对应真机契约：WS 事件是全局广播，只有 meta.session / 载荷键**严格等于**
// 侧键的帧才允许进入侧对话；主会话帧（或无键帧）必须被丢弃。
import { nextTick, ref } from 'vue'
import { describe, expect, it } from 'vitest'
import {
  SIDE_CHAT_STORAGE_PREFIX,
  isSideChatSessionKey,
  sideChatFrameKey,
  sideChatStorageKey,
  useSideChatSession,
  type SideChatRpc,
  type SideChatStorage,
} from './useSideChatSession'

const MAIN_KEY = 'agent:main:webchat:main'

const flush = () => new Promise<void>(resolve => setTimeout(resolve, 0))

function memoryStorage(): SideChatStorage {
  const map = new Map<string, string>()
  return {
    getItem: key => map.get(key) ?? null,
    setItem: (key, value) => { map.set(key, value) },
  }
}

function harness(overrides: {
  mainKey?: string
  storage?: SideChatStorage
  createKey?: (mainKey: string) => string
} = {}) {
  const handlers = new Map<string, (payload: unknown, meta?: unknown) => void>()
  const sendCalls: Array<Record<string, unknown> | undefined> = []
  const abortCalls: Array<Record<string, unknown> | undefined> = []
  let sendResponder: () => Promise<unknown> = async () => ({ ok: true })

  const call = async <T = unknown>(method: string, params?: Record<string, unknown>): Promise<T> => {
    if (method === 'chat.send') {
      sendCalls.push(params)
      return await sendResponder() as T
    }
    if (method === 'chat.abort') {
      abortCalls.push(params)
      return { aborted: true } as T
    }
    return {} as T
  }
  const rpc: SideChatRpc = {
    call,
    on: (event, handler) => {
      handlers.set(event, handler)
      return () => { handlers.delete(event) }
    },
  }

  const mainSessionKey = ref(overrides.mainKey ?? MAIN_KEY)
  const session = useSideChatSession({
    mainSessionKey,
    rpc,
    storage: overrides.storage ?? memoryStorage(),
    createKey: overrides.createKey,
  })
  session.subscribe()

  return {
    session,
    mainSessionKey,
    handlers,
    sendCalls,
    abortCalls,
    setSendResponder: (fn: () => Promise<unknown>) => { sendResponder = fn },
  }
}

function assistantText(session: ReturnType<typeof harness>['session']): string {
  return session.messages.value
    .filter(message => message.role === 'assistant')
    .map(message => message.text)
    .join('')
}

describe('useSideChatSession · 会话键', () => {
  it('creates an agent-prefixed side key and persists it per main session', () => {
    const storage = memoryStorage()
    const first = harness({ storage })
    const key = first.session.sideSessionKey.value
    expect(key).toMatch(/^agent:main:webchat:side-[a-z0-9]+$/)
    expect(isSideChatSessionKey(key)).toBe(true)
    expect(storage.getItem(sideChatStorageKey(MAIN_KEY))).toBe(key)
    expect(sideChatStorageKey(MAIN_KEY)).toBe(`${SIDE_CHAT_STORAGE_PREFIX}${MAIN_KEY}`)

    // A remount for the same main session reuses the persisted key.
    const second = harness({ storage })
    expect(second.session.sideSessionKey.value).toBe(key)
  })

  it('aborts the running side turn when starting a new side chat', async () => {
    const runtime = harness()
    runtime.session.send('abandon me')
    const previous = runtime.session.sideSessionKey.value

    runtime.session.startNewChat()
    await flush()

    expect(runtime.abortCalls).toEqual([{ sessionKey: previous, source: 'webui_stop' }])
    expect(runtime.session.sideSessionKey.value).not.toBe(previous)
    expect(runtime.session.streaming.value).toBe(false)
  })

  it('switches to a different persisted key when the main session changes', async () => {
    const storage = memoryStorage()
    const runtime = harness({ storage })
    const mainSideKey = runtime.session.sideSessionKey.value

    runtime.mainSessionKey.value = 'agent:main:webchat:other'
    await nextTick()

    expect(runtime.session.sideSessionKey.value).not.toBe(mainSideKey)
    expect(isSideChatSessionKey(runtime.session.sideSessionKey.value)).toBe(true)
    expect(storage.getItem(sideChatStorageKey('agent:main:webchat:other')))
      .toBe(runtime.session.sideSessionKey.value)
  })

  it('new side chat rotates the key, clears the transcript, and persists the new key', () => {
    const storage = memoryStorage()
    const runtime = harness({ storage })
    runtime.session.send('before rotation')
    const previous = runtime.session.sideSessionKey.value

    runtime.session.startNewChat()

    expect(runtime.session.sideSessionKey.value).not.toBe(previous)
    expect(runtime.session.messages.value).toEqual([])
    expect(runtime.session.streaming.value).toBe(false)
    expect(storage.getItem(sideChatStorageKey(MAIN_KEY)))
      .toBe(runtime.session.sideSessionKey.value)
  })
})

describe('useSideChatSession · 发送', () => {
  it('appends the user message immediately and sends the followup contract', () => {
    const runtime = harness()
    expect(runtime.session.send('  hello side  ')).toBe(true)

    expect(runtime.session.messages.value).toHaveLength(1)
    expect(runtime.session.messages.value[0]).toMatchObject({
      role: 'user',
      text: 'hello side',
      status: 'done',
    })
    expect(runtime.session.streaming.value).toBe(true)

    const params = runtime.sendCalls[0]!
    expect(params.sessionKey).toBe(runtime.session.sideSessionKey.value)
    expect(params.message).toBe('hello side')
    expect(params.queueMode).toBe('followup')
    expect(typeof params.clientRequestId).toBe('string')
    expect(typeof params.clientMessageId).toBe('string')
    expect(params.clientMessageId).toBe(runtime.session.messages.value[0].id)
    expect(params).not.toHaveProperty('contextRefs')
  })

  it('refuses a second send while the side turn is streaming', () => {
    const runtime = harness()
    expect(runtime.session.send('one')).toBe(true)
    expect(runtime.session.send('two')).toBe(false)
    expect(runtime.sendCalls).toHaveLength(1)
  })

  it('attaches the main conversation context exactly once', async () => {
    const runtime = harness()
    runtime.session.attachContext()
    expect(runtime.session.contextAttachPending.value).toBe(true)

    runtime.session.send('first')
    expect(runtime.sendCalls[0]!.contextRefs).toEqual([{ kind: 'session', id: MAIN_KEY }])
    expect(runtime.session.contextAttachPending.value).toBe(false)
    expect(runtime.session.contextAttached.value).toBe(true)

    runtime.handlers.get('session.event.done')?.(
      {},
      { session: runtime.session.sideSessionKey.value },
    )
    await nextTick()

    runtime.session.send('second')
    expect(runtime.sendCalls[1]!).not.toHaveProperty('contextRefs')
    // The chip keeps reading "attached" so the user can see what was sent.
    expect(runtime.session.contextAttached.value).toBe(true)
  })
})

describe('useSideChatSession · 事件过滤', () => {
  it('drops main-session and keyless frames, then accepts the side key', () => {
    const runtime = harness()
    runtime.session.send('hi')
    const sideKey = runtime.session.sideSessionKey.value
    const delta = runtime.handlers.get('session.event.text_delta')!

    delta({ text: 'MAIN' }, { session: MAIN_KEY })
    delta({ text: 'KEYLESS' })
    delta({ text: 'KEYLESS-META' }, {})
    delta({ text: 'PAYLOAD-MAIN', session_key: MAIN_KEY })
    expect(assistantText(runtime.session)).toBe('')

    delta({ text: 'side ' }, { session: sideKey })
    delta({ text: 'payload' }, { session_key: sideKey })

    expect(assistantText(runtime.session)).toBe('side payload')
    const messages = runtime.session.messages.value
    expect(messages[messages.length - 1]).toMatchObject({
      role: 'assistant',
      status: 'streaming',
    })
  })

  it('ignores a done frame for the main session and settles on the side one', () => {
    const runtime = harness()
    runtime.session.send('hi')
    const sideKey = runtime.session.sideSessionKey.value
    const done = runtime.handlers.get('session.event.done')!

    done({}, { session: MAIN_KEY })
    expect(runtime.session.streaming.value).toBe(true)

    done({}, { session: sideKey })
    expect(runtime.session.streaming.value).toBe(false)
  })

  it('surfaces the tool chip only for side frames', () => {
    const runtime = harness()
    runtime.session.send('hi')
    const tool = runtime.handlers.get('session.event.tool_use_start')!

    tool({ name: 'read_file' }, { session: MAIN_KEY })
    expect(runtime.session.activeTool.value).toBe('')

    tool({ name: 'read_file' }, { session: runtime.session.sideSessionKey.value })
    expect(runtime.session.activeTool.value).toBe('read_file')
  })

  it('tracks approvals for the side key only and clears them when resolved', () => {
    const runtime = harness()
    const sideKey = runtime.session.sideSessionKey.value
    const approval = runtime.handlers.get('session.event.approval')!

    approval({ approval_id: 'ap-main', namespace: 'exec', command: 'rm -rf' }, { session: MAIN_KEY })
    expect(runtime.session.pendingApproval.value).toBeNull()

    approval({ approval_id: 'ap-1', namespace: 'exec', command: 'echo hi' }, { session: sideKey })
    expect(runtime.session.pendingApproval.value).toEqual({
      id: 'ap-1',
      namespace: 'exec',
      command: 'echo hi',
      toolName: '',
      sessionKey: sideKey,
    })

    approval({ approval_id: 'ap-1', approved: false }, { session: sideKey })
    expect(runtime.session.pendingApproval.value).toBeNull()
  })

  it('exposes the frame key precedence used by the filter', () => {
    expect(sideChatFrameKey({}, { session: 'agent:main:webchat:a' })).toBe('agent:main:webchat:a')
    expect(sideChatFrameKey({ session_key: 'payload' }, {})).toBe('payload')
    // meta wins over a conflicting payload key.
    expect(sideChatFrameKey({ session_key: 'payload' }, { session: 'meta' })).toBe('meta')
    expect(sideChatFrameKey({}, {})).toBe('')
  })
})

describe('useSideChatSession · 中止 / 失败 / 重试', () => {
  it('aborts through chat.abort with the side key only', async () => {
    const runtime = harness()
    runtime.session.send('long task')
    const sideKey = runtime.session.sideSessionKey.value

    await runtime.session.stop()

    expect(runtime.abortCalls).toEqual([{ sessionKey: sideKey, source: 'webui_stop' }])
    expect(runtime.session.streaming.value).toBe(false)
  })

  it('shows a failed state and retries with the same idempotency ids', async () => {
    const runtime = harness()
    runtime.setSendResponder(async () => { throw new Error('gateway down') })
    runtime.session.send('retry me')
    await flush()
    await nextTick()

    expect(runtime.session.failed.value).toBe('gateway down')
    expect(runtime.session.canRetry.value).toBe(true)
    expect(runtime.session.streaming.value).toBe(false)

    runtime.setSendResponder(async () => ({ ok: true }))
    expect(runtime.session.retry()).toBe(true)
    expect(runtime.sendCalls).toHaveLength(2)
    expect(runtime.sendCalls[1]!.clientRequestId).toBe(runtime.sendCalls[0]!.clientRequestId)
    expect(runtime.sendCalls[1]!.clientMessageId).toBe(runtime.sendCalls[0]!.clientMessageId)
    expect(runtime.sendCalls[1]!.message).toBe('retry me')
    // Retry must not duplicate the user turn.
    expect(runtime.session.messages.value).toHaveLength(1)
    expect(runtime.session.failed.value).toBe('')
  })

  it('turns a side-session error frame into a retryable failure', () => {
    const runtime = harness()
    runtime.session.send('explode')
    const sideKey = runtime.session.sideSessionKey.value
    const error = runtime.handlers.get('session.event.error')!

    error({ message: 'model unavailable' }, { session: MAIN_KEY })
    expect(runtime.session.streaming.value).toBe(true)
    expect(runtime.session.failed.value).toBe('')

    error({ message: 'model unavailable' }, { session: sideKey })
    expect(runtime.session.streaming.value).toBe(false)
    expect(runtime.session.failed.value).toBe('model unavailable')
    expect(runtime.session.canRetry.value).toBe(true)
  })

  it('ignores a terminal frame left over from the previous side task', async () => {
    const runtime = harness()
    runtime.setSendResponder(async () => ({ task_id: 'task-a' }))
    runtime.session.send('first')
    await flush()
    await runtime.session.stop()

    runtime.setSendResponder(async () => ({ task_id: 'task-b' }))
    runtime.session.send('second')
    await flush()
    const sideKey = runtime.session.sideSessionKey.value
    const done = runtime.handlers.get('session.event.done')!

    done({ task_id: 'task-a' }, { session: sideKey })
    expect(runtime.session.streaming.value).toBe(true)

    done({ task_id: 'task-b' }, { session: sideKey })
    expect(runtime.session.streaming.value).toBe(false)
  })

  it('ignores a chat.send resolution that lands after a new side chat', async () => {
    const runtime = harness()
    let release!: () => void
    runtime.setSendResponder(() => new Promise(resolve => {
      release = () => resolve({ ok: true })
    }))
    runtime.session.send('old turn')
    const oldKey = runtime.session.sideSessionKey.value

    runtime.session.startNewChat()
    release()
    await flush()

    expect(runtime.session.streaming.value).toBe(false)
    expect(runtime.session.messages.value).toEqual([])
    expect(runtime.session.failed.value).toBe('')

    // Late frames addressed to the retired key are also dropped.
    runtime.handlers.get('session.event.text_delta')?.(
      { text: 'late' },
      { session: oldKey },
    )
    expect(runtime.session.messages.value).toEqual([])
  })
})
