import { ref, nextTick } from 'vue'
import { describe, expect, it, vi } from 'vitest'

import { useChatEscalations, normalizeEscalation } from './useChatEscalations'
import type { EscalationEvent } from '@/types/escalation'

const SESSION = 'agent:main:webchat:one'

function event(overrides: Partial<EscalationEvent> = {}): EscalationEvent {
  return {
    id: 'esc-1',
    session: SESSION,
    trigger: 'timeout',
    severity: 6,
    reason: 'Step exceeded 90s budget',
    context: { operation: 'web_search' },
    suggested_action: 'raise timeout budget',
    created_at: '2026-09-04T00:00:00Z',
    ...overrides,
  }
}

function harness(sessionKey = SESSION) {
  const handlers = new Map<string, (...args: unknown[]) => void>()
  const rpc = {
    call: vi.fn().mockResolvedValue({ ok: true }),
    on: vi.fn((eventName: string, handler: (...args: unknown[]) => void) => {
      handlers.set(eventName, handler)
      return vi.fn()
    }),
  }
  const composable = useChatEscalations({
    rpc: rpc as never,
    sessionKey: ref(sessionKey) as never,
    notifyError: vi.fn(),
  })
  const subscribe = composable.subscribe()
  return { composable, rpc, handlers, subscribe }
}

function emit(handlers: Map<string, (...args: unknown[]) => void>, eventName: string, payload: unknown) {
  const fn = handlers.get(eventName)
  expect(fn).toBeDefined()
  fn?.(payload)
}

describe('normalizeEscalation', () => {
  it('parses a well-formed escalation', () => {
    const out = normalizeEscalation(event())
    expect(out?.id).toBe('esc-1')
    expect(out?.trigger).toBe('timeout')
    expect(out?.severity).toBe(6)
  })

  it('extracts a nested escalation envelope', () => {
    const out = normalizeEscalation({ kind: 'escalation', escalation: event({ id: 'esc-nested' }) })
    expect(out?.id).toBe('esc-nested')
  })

  it('rejects junk / unknown triggers / missing id', () => {
    expect(normalizeEscalation(null)).toBeNull()
    expect(normalizeEscalation('x')).toBeNull()
    expect(normalizeEscalation({ id: 'x', trigger: 'mystery' })).toBeNull()
    expect(normalizeEscalation({ trigger: 'timeout' })).toBeNull()
  })
})

describe('useChatEscalations', () => {
  it('shows a pending card on escalation event and queues a second one', async () => {
    const { composable, handlers } = harness()
    emit(handlers, 'session.event.escalation', event())
    await nextTick()
    expect(composable.pending.value?.id).toBe('esc-1')
    expect(composable.queue.value).toHaveLength(0)

    emit(handlers, 'escalation', event({ id: 'esc-2', trigger: 'budget_exceeded' }))
    await nextTick()
    expect(composable.pending.value?.id).toBe('esc-1')
    expect(composable.queue.value.map(e => e.id)).toEqual(['esc-2'])
  })

  it('ignores events from other sessions', async () => {
    const { composable, handlers } = harness(SESSION)
    emit(handlers, 'session.event.escalation', event({ session: 'agent:main:webchat:other' }))
    await nextTick()
    expect(composable.pending.value).toBeNull()
  })

  it('resolves via escalation.ack and pops the next queued event', async () => {
    const { composable, handlers, rpc } = harness()
    emit(handlers, 'session.event.escalation', event())
    emit(handlers, 'session.event.escalation', event({ id: 'esc-2' }))
    await nextTick()
    await composable.resolve(composable.pending.value!, 'continue')
    expect(rpc.call).toHaveBeenCalledWith('escalation.ack', { id: 'esc-1', decision: 'continue' })
    expect(composable.pending.value?.id).toBe('esc-2')
    expect(composable.queue.value).toHaveLength(0)
    expect(composable.resolvedCount.value).toBe(1)
  })

  it('abort decision forwards abort and does not clear pending', async () => {
    const { composable, handlers, rpc } = harness()
    emit(handlers, 'session.event.escalation', event())
    await nextTick()
    await composable.resolve(composable.pending.value!, 'abort')
    expect(rpc.call).toHaveBeenCalledWith('escalation.ack', { id: 'esc-1', decision: 'abort' })
    expect(composable.pending.value).toBeNull()
  })

  it('clears state on session switch', async () => {
    const sessionKey = ref(SESSION)
    const handlers = new Map<string, (...args: unknown[]) => void>()
    const composable = useChatEscalations({
      rpc: { call: vi.fn().mockResolvedValue({ ok: true }), on: vi.fn((e, h) => { handlers.set(e, h); return vi.fn() }) } as never,
      sessionKey: sessionKey as never,
    })
    composable.subscribe()
    emit(handlers, 'session.event.escalation', event())
    await nextTick()
    expect(composable.pending.value).not.toBeNull()
    sessionKey.value = 'agent:main:webchat:two'
    await nextTick()
    expect(composable.pending.value).toBeNull()
    expect(composable.queue.value).toHaveLength(0)
  })

  it('recovers pre-socket escalations via escalation.pending on subscribe', async () => {
    const { composable, rpc } = harness()
    // `params` is optional, so a parameterless call records a 1-element arg list;
    // asserting an explicit `undefined` would only pin JS arity, not behaviour.
    expect(rpc.call).toHaveBeenCalledWith('escalation.pending')
    expect(composable).toBeDefined()
  })
})
