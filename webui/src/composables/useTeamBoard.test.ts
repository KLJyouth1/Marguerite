import { effectScope, nextTick } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const rpcMock = vi.hoisted(() => ({
  call: null as unknown as ReturnType<typeof vi.fn>,
  methods: null as unknown as { value: string[] },
  listeners: new Map<string, (payload?: unknown) => void>(),
}))

vi.mock('@/stores/rpc', async () => {
  const { ref } = await import('vue')
  const methods = ref<string[]>([])
  const call = vi.fn()
  const listeners = new Map<string, (payload?: unknown) => void>()
  const on = vi.fn((event: string, handler: (payload?: unknown) => void) => {
    listeners.set(event, handler)
    return () => {
      listeners.delete(event)
    }
  })
  rpcMock.call = call as unknown as ReturnType<typeof vi.fn>
  rpcMock.methods = methods as unknown as { value: string[] }
  rpcMock.listeners = listeners
  return {
    useRpcStore: () => ({
      call,
      on,
      supportsMethod: (method: string) => methods.value.includes(method),
    }),
  }
})

import {
  TEAM_BOARD_EVENT_DEBOUNCE_MS,
  TEAM_BOARD_POLL_MS,
  TEAM_MEMBERS_METHOD,
  normalizeTeamMember,
  normalizeTeamMembers,
  useTeamBoard,
  type TeamBoardState,
} from './useTeamBoard'

function contractMembers() {
  return [
    {
      id: 'lead-1',
      label: 'Ada',
      role: '协调者',
      kind: 'lead',
      status: 'running',
      currentStep: '拆解任务',
      toolCalls: 3,
      lastActivityMs: 1_700_000_000_000,
      tokensUsed: 1200,
    },
    {
      id: 'sub-1',
      label: 'Bolt',
      role: '编码',
      kind: 'subagent',
      status: 'finished',
      currentStep: null,
      toolCalls: 9,
      lastActivityMs: 1_700_000_100_000,
      tokensUsed: 3400,
    },
  ]
}

function payload(overrides: Record<string, unknown> = {}) {
  return {
    ok: true,
    session: 'agent:main:webchat:test',
    updatedAt: 1_700_000_200_000,
    members: contractMembers(),
    ...overrides,
  }
}

const scopes: Array<{ stop: () => void }> = []

function runComposable(): TeamBoardState {
  const scope = effectScope()
  const state = scope.run(() => useTeamBoard())
  if (!state) throw new Error('useTeamBoard did not run inside the effect scope')
  scopes.push(scope)
  return state
}

async function settle(): Promise<void> {
  for (let index = 0; index < 6; index += 1) await Promise.resolve()
  await nextTick()
}

beforeEach(() => {
  vi.useFakeTimers()
  rpcMock.call.mockReset()
  rpcMock.call.mockResolvedValue(payload())
  rpcMock.methods.value = [TEAM_MEMBERS_METHOD]
  rpcMock.listeners.clear()
})

afterEach(() => {
  while (scopes.length) scopes.pop()!.stop()
  vi.clearAllTimers()
  vi.useRealTimers()
})

describe('useTeamBoard gating', () => {
  it('stays empty and never polls while team.members is not declared', async () => {
    rpcMock.methods.value = []
    const state = runComposable()
    await settle()

    expect(state.supported.value).toBe(false)
    expect(state.members.value).toEqual([])
    expect(rpcMock.call).not.toHaveBeenCalled()

    await vi.advanceTimersByTimeAsync(TEAM_BOARD_POLL_MS * 3)
    expect(rpcMock.call).not.toHaveBeenCalled()
  })

  it('starts polling as soon as the gateway declares the method', async () => {
    rpcMock.methods.value = []
    const state = runComposable()
    await settle()
    expect(rpcMock.call).not.toHaveBeenCalled()

    rpcMock.methods.value = [TEAM_MEMBERS_METHOD]
    await settle()

    expect(state.supported.value).toBe(true)
    expect(rpcMock.call).toHaveBeenCalledTimes(1)
    expect(rpcMock.call).toHaveBeenCalledWith(TEAM_MEMBERS_METHOD, {})
    expect(state.members.value.map((member) => member.id)).toEqual(['lead-1', 'sub-1'])
  })

  it('clears members and stops polling when the method disappears', async () => {
    const state = runComposable()
    await settle()
    expect(state.members.value).toHaveLength(2)

    rpcMock.methods.value = []
    await settle()
    expect(state.members.value).toEqual([])
    expect(state.error.value).toBe('')

    const calls = rpcMock.call.mock.calls.length
    await vi.advanceTimersByTimeAsync(TEAM_BOARD_POLL_MS * 2)
    expect(rpcMock.call).toHaveBeenCalledTimes(calls)
  })
})

describe('useTeamBoard polling and event refresh', () => {
  it('polls every five seconds while mounted', async () => {
    const state = runComposable()
    await settle()
    expect(rpcMock.call).toHaveBeenCalledTimes(1)
    expect(state.updatedAt.value).toBe(1_700_000_200_000)

    await vi.advanceTimersByTimeAsync(TEAM_BOARD_POLL_MS)
    expect(rpcMock.call).toHaveBeenCalledTimes(2)
    await vi.advanceTimersByTimeAsync(TEAM_BOARD_POLL_MS)
    expect(rpcMock.call).toHaveBeenCalledTimes(3)
  })

  it('refreshes on sessions.changed after a debounce window', async () => {
    runComposable()
    await settle()
    const handler = rpcMock.listeners.get('sessions.changed')
    expect(handler).toBeTypeOf('function')

    handler?.()
    expect(rpcMock.call).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(TEAM_BOARD_EVENT_DEBOUNCE_MS)
    expect(rpcMock.call).toHaveBeenCalledTimes(2)
  })

  it('coalesces an event burst into a single refresh', async () => {
    runComposable()
    await settle()
    const handler = rpcMock.listeners.get('sessions.changed')

    handler?.()
    handler?.()
    handler?.()
    await vi.advanceTimersByTimeAsync(TEAM_BOARD_EVENT_DEBOUNCE_MS)
    expect(rpcMock.call).toHaveBeenCalledTimes(2)
  })
})

describe('useTeamBoard teardown', () => {
  it('clears the poll timer and the event subscription on scope dispose', async () => {
    const state = runComposable()
    await settle()
    expect(rpcMock.listeners.size).toBe(1)

    scopes.pop()!.stop()
    expect(rpcMock.listeners.size).toBe(0)

    const calls = rpcMock.call.mock.calls.length
    await vi.advanceTimersByTimeAsync(TEAM_BOARD_POLL_MS * 3)
    expect(rpcMock.call).toHaveBeenCalledTimes(calls)

    // 卸载后残留的状态仍在，但不再有任何后台读取。
    expect(state.members.value).toHaveLength(2)
  })

  it('stops the poll timer when the method is withdrawn before unmount', async () => {
    runComposable()
    await settle()
    rpcMock.methods.value = []
    await settle()

    const calls = rpcMock.call.mock.calls.length
    await vi.advanceTimersByTimeAsync(TEAM_BOARD_POLL_MS * 2)
    expect(rpcMock.call).toHaveBeenCalledTimes(calls)
  })
})

describe('useTeamBoard failure honesty', () => {
  it('empties members and keeps a readable reason when the RPC rejects', async () => {
    rpcMock.call.mockRejectedValue(new Error('gateway offline'))
    const state = runComposable()
    await settle()

    expect(state.members.value).toEqual([])
    expect(state.updatedAt.value).toBeNull()
    expect(state.error.value).toBe('gateway offline')
  })

  it('does not keep stale members when the payload reports ok:false', async () => {
    const state = runComposable()
    await settle()
    expect(state.members.value).toHaveLength(2)

    rpcMock.call.mockResolvedValue({ ok: false, error: 'team run not found' })
    await vi.advanceTimersByTimeAsync(TEAM_BOARD_POLL_MS)

    expect(state.members.value).toEqual([])
    expect(state.error.value).toBe('team run not found')
  })
})

describe('normalizeTeamMembers', () => {
  it('drops rows without an id instead of inventing members', () => {
    expect(normalizeTeamMember({ label: 'ghost' })).toBeNull()
    expect(normalizeTeamMember(null)).toBeNull()
    expect(normalizeTeamMembers([{ id: 'a', label: 'A' }, { label: 'ghost' }])).toHaveLength(1)
    expect(normalizeTeamMembers(undefined)).toEqual([])
  })

  it('normalizes optional fields without fabricating numbers', () => {
    const member = normalizeTeamMember({
      id: 'm1',
      label: '  ',
      role: '评审',
      kind: '',
      status: '',
      currentStep: '   ',
      toolCalls: -4,
      lastActivityMs: 'yesterday',
      tokensUsed: Number.NaN,
    })

    expect(member).toMatchObject({
      id: 'm1',
      label: '评审',
      kind: 'unknown',
      status: 'unknown',
      currentStep: null,
      toolCalls: 0,
      lastActivityMs: null,
      tokensUsed: null,
    })
  })
})
