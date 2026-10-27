import { computed, ref, watch, type Ref } from 'vue'
import { agentIdFromSessionKey, newWebchatSessionKey } from '@/utils/chat/sessionKeys'
import { createClientMessageId, createClientRequestId } from '@/utils/chat/messageIdentity'
import { payloadTaskId } from '@/utils/chat/streamEvents'

/**
 * Side chat = a second, independent conversation that runs **beside** the main
 * one. It owns its own `agent:<id>:webchat:side-*` session key, so:
 *
 *  · `chat.send` on the side key never queues behind the main turn
 *    (SESSION_BUSY is per key), and
 *  · every WS frame is filtered by `meta.session` / payload key before it can
 *    touch the side transcript — main-session deltas are dropped outright,
 *    including keyless frames (the gateway broadcasts every event to every
 *    client, so "no key" must never mean "mine").
 *
 * The main session key is only ever read: it scopes the persisted side key
 * (`<storage prefix><main key>`) and provides the optional one-shot
 * `contextRefs: [{kind:'session', id: mainKey}]` attachment. Nothing here
 * writes to the main conversation or to global stores.
 */

export type SideChatRole = 'user' | 'assistant'
export type SideChatMessageStatus = 'done' | 'streaming' | 'failed'

export interface SideChatMessage {
  id: string
  role: SideChatRole
  text: string
  status: SideChatMessageStatus
  error?: string
}

export interface SideChatContextRef {
  kind: string
  id: string
}

export interface SideChatApprovalNotice {
  id: string
  namespace: string
  command: string
  toolName: string
  sessionKey: string
}

export interface SideChatSendAttempt {
  text: string
  clientRequestId: string
  clientMessageId: string
  /** Only the first send after "Attach main conversation" carries this. */
  contextRefs?: SideChatContextRef[]
}

type SideChatEventHandler = (payload: unknown, meta?: unknown) => void

export interface SideChatRpc {
  call: <T = unknown>(method: string, params?: Record<string, unknown>) => Promise<T>
  on: (event: string, handler: SideChatEventHandler) => () => void
}

export interface SideChatStorage {
  getItem(key: string): string | null
  setItem(key: string, value: string): void
}

export interface CreateSideChatSessionOptions {
  /** The main conversation key. Only read; never sent as a write target. */
  mainSessionKey: Ref<string>
  rpc: SideChatRpc
  /** Injectable for tests; defaults to `localStorage` when available. */
  storage?: SideChatStorage
  /** Key factory; defaults to `agent:<agent>:webchat:side-<short id>`. */
  createKey?: (mainKey: string) => string
}

export const SIDE_CHAT_STORAGE_PREFIX = 'marguerite.side-chat.session:'

/** Stable localStorage slot for the side key belonging to one main session. */
export function sideChatStorageKey(mainSessionKey: string): string {
  return SIDE_CHAT_STORAGE_PREFIX + (String(mainSessionKey || '').trim() || 'default')
}

/** Guards against a corrupted/stale persisted value. */
export function isSideChatSessionKey(value: string): boolean {
  return /^agent:[^:]+:webchat:side-[A-Za-z0-9_-]+$/.test(String(value || ''))
}

/**
 * A side key must keep the `agent:` prefix so persona routing still resolves;
 * the `side-` suffix keeps it distinguishable from the primary webchat key.
 */
export function defaultSideChatKey(mainSessionKey: string): string {
  const agentId = agentIdFromSessionKey(String(mainSessionKey || '').trim())
  const suffix = `side-${Math.random().toString(36).slice(2, 8)}`
  return newWebchatSessionKey(agentId, suffix)
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' ? value as Record<string, unknown> : null
}

/**
 * Session key carried by a WS frame. `meta.session` wins because it is the
 * envelope identity the gateway stamps on every broadcast; only when the meta
 * frame is absent do we fall back to the payload's own key fields. An empty
 * result means "not addressed to any session" and callers must drop the frame.
 */
export function sideChatFrameKey(payload: unknown, meta?: unknown): string {
  const metaRecord = asRecord(meta)
  if (metaRecord) {
    const metaKey = metaRecord.session ?? metaRecord.session_key ?? metaRecord.sessionKey ?? metaRecord.key
    if (typeof metaKey === 'string' && metaKey.trim()) return metaKey.trim()
  }
  const record = asRecord(payload)
  if (!record) return ''
  for (const candidate of [record.session, record.session_key, record.sessionKey, record.key]) {
    if (typeof candidate === 'string' && candidate.trim()) return candidate.trim()
  }
  return ''
}

function defaultStorage(): SideChatStorage | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage
  } catch {
    return null
  }
}

export function useSideChatSession(options: CreateSideChatSessionOptions) {
  const { rpc } = options
  const storage = options.storage ?? defaultStorage()
  const createKey = options.createKey ?? defaultSideChatKey
  const mainSessionKey = computed(() => String(options.mainSessionKey.value || '').trim())

  function readPersistedKey(mainKey: string): string {
    try {
      return storage?.getItem(sideChatStorageKey(mainKey)) || ''
    } catch {
      return ''
    }
  }

  function persistKey(mainKey: string, sideKey: string) {
    try {
      storage?.setItem(sideChatStorageKey(mainKey), sideKey)
    } catch {
      /* private mode / quota — the in-memory key still works for this mount */
    }
  }

  function loadOrCreateKey(mainKey: string): string {
    const persisted = readPersistedKey(mainKey)
    if (isSideChatSessionKey(persisted)) return persisted
    const created = createKey(mainKey)
    persistKey(mainKey, created)
    return created
  }

  const sideSessionKey = ref(loadOrCreateKey(mainSessionKey.value))
  const messages = ref<SideChatMessage[]>([])
  const streaming = ref(false)
  const failed = ref('')
  const activeTool = ref('')
  const contextAttachPending = ref(false)
  const contextAttached = ref(false)
  const pendingApproval = ref<SideChatApprovalNotice | null>(null)

  const lastAttempt = ref<SideChatSendAttempt | null>(null)
  let activeAssistantId = ''
  // Task id bound from the accepted `chat.send`; suppresses a late terminal
  // frame from the previous side turn (e.g. done racing a stop + resend).
  let activeTaskId = ''
  // Bumped on every dispatch / abort / new chat. A `chat.send` promise that
  // settles after one of those is stale and must not touch the transcript.
  let runGeneration = 0
  let unsubscribe: (() => void) | null = null

  const canRetry = computed(() => (
    lastAttempt.value !== null && !streaming.value && failed.value !== ''
  ))
  const approvalPending = computed(() => pendingApproval.value !== null)

  function resetConversationState() {
    runGeneration += 1
    streaming.value = false
    failed.value = ''
    activeTool.value = ''
    activeAssistantId = ''
    activeTaskId = ''
    pendingApproval.value = null
    lastAttempt.value = null
    messages.value = []
  }

  // Switching the main conversation switches the side conversation with it:
  // each main key owns its own persisted side key.
  watch(mainSessionKey, (key, previous) => {
    if (key === previous) return
    // Leaving this main conversation also abandons its side turn; abort it so
    // the old side run does not keep burning tokens with no listener.
    if (streaming.value) void stop()
    resetConversationState()
    contextAttached.value = false
    contextAttachPending.value = false
    sideSessionKey.value = loadOrCreateKey(key)
  })

  function acceptFrame(payload: unknown, meta?: unknown): boolean {
    const key = sideChatFrameKey(payload, meta)
    return key !== '' && key === sideSessionKey.value
  }

  /** Lenient task guard: only a positively different task id is dropped. */
  function isCurrentTaskFrame(payload: unknown): boolean {
    if (!activeTaskId) return true
    const taskId = payloadTaskId(asRecord(payload))
    return !taskId || taskId === activeTaskId
  }

  function patchMessage(id: string, patch: Partial<SideChatMessage>) {
    messages.value = messages.value.map(message => (
      message.id === id ? { ...message, ...patch } : message
    ))
  }

  function markActiveToolFrom(payload: unknown) {
    const record = asRecord(payload)
    if (!record) return
    const name = record.name ?? record.tool_name ?? record.toolName
    if (typeof name === 'string' && name.trim()) activeTool.value = name.trim()
  }

  function handleTextDelta(payload: unknown, meta?: unknown) {
    if (!acceptFrame(payload, meta) || !streaming.value || !isCurrentTaskFrame(payload)) return
    const record = asRecord(payload)
    const text = record && typeof record.text === 'string' ? record.text : ''
    if (!text) return
    if (!activeAssistantId) {
      const assistant: SideChatMessage = {
        id: createClientMessageId(),
        role: 'assistant',
        text,
        status: 'streaming',
      }
      activeAssistantId = assistant.id
      messages.value = [...messages.value, assistant]
      return
    }
    const id = activeAssistantId
    messages.value = messages.value.map(message => (
      message.id === id
        ? { ...message, text: message.text + text, status: 'streaming' as const }
        : message
    ))
  }

  function handleDone(payload: unknown, meta?: unknown) {
    if (!acceptFrame(payload, meta) || !isCurrentTaskFrame(payload)) return
    streaming.value = false
    activeTool.value = ''
    activeTaskId = ''
    if (!activeAssistantId) return
    const id = activeAssistantId
    activeAssistantId = ''
    patchMessage(id, { status: 'done' })
  }

  function handleToolActivity(payload: unknown, meta?: unknown) {
    if (!acceptFrame(payload, meta) || !streaming.value) return
    markActiveToolFrom(payload)
  }

  function handleToolResult(payload: unknown, meta?: unknown) {
    if (!acceptFrame(payload, meta)) return
    activeTool.value = ''
  }

  /** `session.event.error` is terminal for the side turn; surface it with retry. */
  function handleError(payload: unknown, meta?: unknown) {
    if (!acceptFrame(payload, meta) || !streaming.value || !isCurrentTaskFrame(payload)) return
    const record = asRecord(payload)
    const raw = record?.message ?? record?.error
    const message = typeof raw === 'string' ? raw : ''
    streaming.value = false
    activeTool.value = ''
    activeTaskId = ''
    failed.value = message || 'unknown error'
    if (activeAssistantId) {
      const id = activeAssistantId
      activeAssistantId = ''
      patchMessage(id, { status: 'failed', error: message })
    }
  }

  function handleApproval(payload: unknown, meta?: unknown) {
    if (!acceptFrame(payload, meta)) return
    const record = asRecord(payload)
    if (!record) return
    const id = String(record.approval_id ?? record.approvalId ?? '').trim()
    if (!id) return
    // A resolved/updated frame for the same approval clears the notice; the
    // authoritative card list still comes from useChatApprovals.
    if (typeof record.approved === 'boolean' || typeof record.resolution === 'string') {
      if (pendingApproval.value?.id === id) pendingApproval.value = null
      return
    }
    pendingApproval.value = {
      id,
      namespace: String(record.namespace || 'exec'),
      command: String(record.command || record.method || ''),
      toolName: String(record.tool_name || record.toolName || record.method || ''),
      sessionKey: sideSessionKey.value,
    }
  }

  const handlers: Array<[string, SideChatEventHandler]> = [
    ['session.event.text_delta', handleTextDelta],
    ['session.event.done', handleDone],
    // Rust maps the envelope's `tool_call` type to `tool_use_start`; the
    // bare/legacy names are kept so either producer feeds the tool chip.
    ['session.event.tool_use_start', handleToolActivity],
    ['session.event.tool_call', handleToolActivity],
    ['session.event.tool_result', handleToolResult],
    ['session.event.approval', handleApproval],
    ['session.event.error', handleError],
  ]

  /** Register WS listeners. Idempotent; the returned function unsubscribes all. */
  function subscribe(): () => void {
    if (unsubscribe) return unsubscribe
    const offs = handlers.map(([event, handler]) => rpc.on(event, handler))
    const teardown = () => {
      offs.forEach(off => off())
      unsubscribe = null
    }
    unsubscribe = teardown
    return teardown
  }

  function isLive(generation: number, key: string): boolean {
    return generation === runGeneration && key === sideSessionKey.value
  }

  async function dispatch(attempt: SideChatSendAttempt) {
    const key = sideSessionKey.value
    const generation = ++runGeneration
    streaming.value = true
    failed.value = ''
    activeTool.value = ''
    activeAssistantId = ''
    activeTaskId = ''
    const params: Record<string, unknown> = {
      sessionKey: key,
      message: attempt.text,
      clientRequestId: attempt.clientRequestId,
      clientMessageId: attempt.clientMessageId,
      queueMode: 'followup',
    }
    if (attempt.contextRefs?.length) {
      params.contextRefs = attempt.contextRefs.map(ref => ({ ...ref }))
    }
    try {
      const response = await rpc.call<Record<string, unknown>>('chat.send', params)
      if (!isLive(generation, key)) return
      const taskId = response?.task_id ?? response?.taskId
      if (typeof taskId === 'string' && taskId.trim()) activeTaskId = taskId.trim()
      // Success is signalled by WS frames; nothing to append here. A late
      // resolution after abort/new-chat is stale and ignored above.
    } catch (error) {
      if (!isLive(generation, key) || !streaming.value) return
      streaming.value = false
      activeTool.value = ''
      activeTaskId = ''
      const message = error instanceof Error ? error.message : String(error ?? '')
      failed.value = message || 'unknown error'
      if (activeAssistantId) {
        const id = activeAssistantId
        activeAssistantId = ''
        patchMessage(id, { status: 'failed', error: message })
      }
    }
  }

  /**
   * Append the user turn immediately, then dispatch. The pending "attach main
   * conversation" flag is one-shot: it is consumed here, so the next send is a
   * plain message even if the first one fails (retry keeps the snapshot).
   */
  function send(rawText: string): boolean {
    const text = String(rawText || '').trim()
    if (!text || streaming.value || !sideSessionKey.value) return false
    const attempt: SideChatSendAttempt = {
      text,
      clientRequestId: createClientRequestId(),
      clientMessageId: createClientMessageId(),
    }
    if (contextAttachPending.value && mainSessionKey.value) {
      attempt.contextRefs = [{ kind: 'session', id: mainSessionKey.value }]
    }
    const userMessage: SideChatMessage = {
      id: attempt.clientMessageId,
      role: 'user',
      text,
      status: 'done',
    }
    messages.value = [...messages.value, userMessage]
    if (contextAttachPending.value) {
      contextAttachPending.value = false
      contextAttached.value = true
    }
    lastAttempt.value = attempt
    void dispatch(attempt)
    return true
  }

  /** Re-dispatch the last failed attempt with the same idempotency ids. */
  function retry(): boolean {
    const attempt = lastAttempt.value
    if (!attempt || streaming.value) return false
    failed.value = ''
    void dispatch(attempt)
    return true
  }

  /** Abort the side run only; the main conversation is never touched. */
  async function stop(): Promise<void> {
    if (!streaming.value) return
    const key = sideSessionKey.value
    // Invalidate the in-flight chat.send promise and any late delta.
    runGeneration += 1
    streaming.value = false
    activeTool.value = ''
    activeTaskId = ''
    if (activeAssistantId) {
      const id = activeAssistantId
      activeAssistantId = ''
      patchMessage(id, { status: 'done' })
    }
    try {
      await rpc.call('chat.abort', { sessionKey: key, source: 'webui_stop' })
    } catch {
      /* best effort: the side run may already have settled */
    }
  }

  /** Rotate to a brand-new side session (a new key; old frames are filtered). */
  function startNewChat() {
    // Abandoning the current side turn must also abort it, otherwise it keeps
    // running server-side with no UI listening.
    if (streaming.value) void stop()
    resetConversationState()
    contextAttached.value = false
    contextAttachPending.value = false
    const key = createKey(mainSessionKey.value)
    sideSessionKey.value = key
    persistKey(mainSessionKey.value, key)
  }

  function attachContext() {
    if (!mainSessionKey.value) return
    contextAttachPending.value = true
  }

  function detachContext() {
    contextAttachPending.value = false
    contextAttached.value = false
  }

  function toggleContextAttachment() {
    if (contextAttached.value || contextAttachPending.value) detachContext()
    else attachContext()
  }

  function clearApprovalNotice(id?: string) {
    if (!pendingApproval.value) return
    if (id && pendingApproval.value.id !== id) return
    pendingApproval.value = null
  }

  /** Drop every listener; in-flight responses become stale. */
  function cleanup() {
    unsubscribe?.()
    runGeneration += 1
    streaming.value = false
    activeTool.value = ''
    activeTaskId = ''
  }

  return {
    sideSessionKey,
    messages,
    streaming,
    failed,
    activeTool,
    contextAttachPending,
    contextAttached,
    pendingApproval,
    approvalPending,
    canRetry,
    send,
    retry,
    stop,
    startNewChat,
    attachContext,
    detachContext,
    toggleContextAttachment,
    clearApprovalNotice,
    subscribe,
    cleanup,
  }
}

export type SideChatSession = ReturnType<typeof useSideChatSession>
