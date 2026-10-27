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
  TEAM_PROGRESS_EVENT,
  EMPTY_RUN_INFO,
  emptyMemberActivity,
  isLeadRow,
  leadActivityKey,
  normalizeTeamMember,
  normalizeTeamMembers,
  normalizeTeamRunInfo,
  reduceProgressFrame,
  resolveMemberStatus,
  useTeamBoard,
  type MemberActivityMap,
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

/**
 * 编排进度帧的线缆形状 —— 逐字段对齐 `core/src/agent/subagent.rs` 的
 * `ProgressReporter::emit`（kind/phase/runId/memberId 由它补齐）与
 * `core/src/orchestration/mod.rs::begin_orchestration`。
 */
function frame(overrides: Record<string, unknown> = {}) {
  return {
    kind: 'orchestration',
    phase: 'member_started',
    runId: 'run-1',
    memberId: 'm-1',
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
    // sessions.changed + session.event.progress —— 看板订阅两条既有事件流。
    expect(rpcMock.listeners.size).toBe(2)

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

describe('normalizeTeamRunInfo', () => {
  it('reads the run meta the backend already sends', () => {
    expect(
      normalizeTeamRunInfo({
        runId: 'run-7f3a',
        session: 'agent:main:webchat:t1',
        form: 'swarm',
        memberLimit: 512,
        dropped: 37,
        fromDisk: true,
        note: '当前没有团队编排运行记录',
      }),
    ).toEqual({
      runId: 'run-7f3a',
      session: 'agent:main:webchat:t1',
      form: 'swarm',
      memberLimit: 512,
      dropped: 37,
      fromDisk: true,
      note: '当前没有团队编排运行记录',
    })
  })

  it('leaves absent meta empty instead of inventing a limit or a drop count', () => {
    expect(normalizeTeamRunInfo({})).toEqual({ ...EMPTY_RUN_INFO })
    expect(normalizeTeamRunInfo(null)).toEqual({ ...EMPTY_RUN_INFO })
    expect(normalizeTeamRunInfo({ memberLimit: 'unbounded', dropped: -3, fromDisk: 'yes' })).toEqual({
      ...EMPTY_RUN_INFO,
    })
  })

  it('keeps an unknown form verbatim rather than guessing the shape', () => {
    expect(normalizeTeamRunInfo({ form: 'council' }).form).toBe('council')
  })
})

describe('leadActivityKey / isLeadRow', () => {
  it('mirrors the backend id scheme for the synthesized lead row', () => {
    expect(leadActivityKey('run-9')).toBe('lead:run-9')
    expect(isLeadRow(normalizeTeamMember({ id: 'lead:run-9', kind: 'lead' })!)).toBe(true)
    // 名字里带 lead 但 id 不是 `lead:<runId>` 的行是真成员，不是合成行。
    expect(isLeadRow(normalizeTeamMember({ id: 'leader-1', kind: 'lead' })!)).toBe(false)
    expect(isLeadRow(normalizeTeamMember({ id: 'sub-1', kind: 'subagent' })!)).toBe(false)
  })
})

describe('reduceProgressFrame', () => {
  const base: MemberActivityMap = {}

  it('ignores frames the board cannot attribute', () => {
    expect(reduceProgressFrame(base, null, 1_000)).toBeNull()
    expect(reduceProgressFrame(base, { kind: 'tool', phase: 'member_started', memberId: 'm-1' }, 1_000)).toBeNull()
    expect(reduceProgressFrame(base, { kind: 'orchestration' }, 1_000)).toBeNull()
    expect(reduceProgressFrame(base, { kind: 'orchestration', phase: 'member_expression', memberId: 'm-1' }, 1_000)).toBeNull()
    expect(reduceProgressFrame(base, { kind: 'orchestration', phase: 'tool_call' }, 1_000)).toBeNull()
  })

  it('captures member_thinking, which the team.members projection drops', () => {
    const next = reduceProgressFrame(
      base,
      frame({ phase: 'member_thinking', round: 2, thinking: '先读测试再定位断言' }),
      5_000,
    )
    expect(next?.['m-1']).toMatchObject({
      lastPhase: 'member_thinking',
      thinking: '先读测试再定位断言',
      thinkingRound: 2,
      liveStatus: 'running',
      startedAtMs: null,
      elapsedMs: null,
      summary: '',
    })
    // 空思考帧没有可展示的内容 —— 不写进动向表
    expect(reduceProgressFrame(base, frame({ phase: 'member_thinking', thinking: '  ' }), 5_000)).toBeNull()
  })

  it('records the start time only when member_started is really observed, once', () => {
    const first = reduceProgressFrame(base, frame({ task: '写测试' }), 1_000)
    expect(first?.['m-1'].startedAtMs).toBe(1_000)
    const replay = reduceProgressFrame(first!, frame({ task: '写测试' }), 9_000)
    expect(replay?.['m-1'].startedAtMs).toBe(1_000)
  })

  it('follows tool_call → tool_result and keeps the outcome honest', () => {
    const call = reduceProgressFrame(base, frame({ phase: 'tool_call', tool: 'shell', argsPreview: 'ls' }), 2_000)
    expect(call?.['m-1']).toMatchObject({ lastTool: 'shell', lastToolOk: null, lastToolElapsedMs: null })
    const result = reduceProgressFrame(
      call!,
      frame({ phase: 'tool_result', tool: 'shell', ok: false, elapsedMs: 4_812, outputPreview: 'boom' }),
      6_812,
    )
    expect(result?.['m-1']).toMatchObject({ lastToolOk: false, lastToolElapsedMs: 4_812 })
  })

  it('keeps every member_completed field the projection throws away', () => {
    const running = reduceProgressFrame(base, frame(), 1_000)
    const done = reduceProgressFrame(
      running!,
      frame({ phase: 'member_completed', ok: true, elapsedMs: 90_500, summary: '补了 3 个断言', timedOut: false, cancelled: false }),
      91_500,
    )
    expect(done?.['m-1']).toMatchObject({
      terminalStatus: 'finished',
      liveStatus: null,
      elapsedMs: 90_500,
      summary: '补了 3 个断言',
      timedOut: false,
      cancelled: false,
      startedAtMs: 1_000,
    })
  })

  it('treats a missing ok as success (same default as the backend) and a false ok as failure', () => {
    const bare = reduceProgressFrame(base, frame({ phase: 'member_completed' }), 2_000)
    expect(bare?.['m-1'].terminalStatus).toBe('finished')
    // 载荷里没有 elapsedMs / summary —— 落空态，不造 0 秒也不造结论文案
    expect(bare?.['m-1'].elapsedMs).toBeNull()
    expect(bare?.['m-1'].summary).toBe('')

    const failed = reduceProgressFrame(
      base,
      frame({ phase: 'member_completed', ok: false, timedOut: true }),
      2_000,
    )
    expect(failed?.['m-1']).toMatchObject({ terminalStatus: 'failed', timedOut: true })
  })

  it('folds gateway run_failed into the lead card so it can stop showing running', () => {
    const next = reduceProgressFrame(
      base,
      { kind: 'orchestration', phase: 'run_failed', runId: 'run-3', ok: false, error: '已取消' },
      4_000,
    )
    expect(next?.['lead:run-3']).toMatchObject({
      terminalStatus: 'failed',
      liveStatus: null,
      error: '已取消',
      synthesizing: false,
    })
  })

  it('marks the lead as synthesizing and clears it on run_completed', () => {
    const synth = reduceProgressFrame(base, frame({ phase: 'lead_synthesizing', runId: 'run-4', memberId: '' }), 1_000)
    expect(synth?.['lead:run-4']).toMatchObject({ synthesizing: true, liveStatus: 'running' })
    const done = reduceProgressFrame(synth!, frame({ phase: 'run_completed', runId: 'run-4', memberId: '', ok: true }), 2_000)
    expect(done?.['lead:run-4']).toMatchObject({ synthesizing: false, terminalStatus: 'finished' })
  })

  it("invalidates the previous run's activity when a new run starts", () => {
    const started = reduceProgressFrame(base, frame({ phase: 'run_started', runId: 'run-1', memberId: '' }), 1_000)!
    expect(Object.keys(started)).toEqual(['lead:run-1'])
    const withMember = reduceProgressFrame(started, frame({ runId: 'run-1', memberId: 'm-old' }), 1_500)!
    expect(Object.keys(withMember).sort()).toEqual(['lead:run-1', 'm-old'])

    const next = reduceProgressFrame(withMember, frame({ phase: 'run_started', runId: 'run-2', memberId: '' }), 2_000)!
    // m-old 属于上一次编排，留着会让用户以为它还在动
    expect(Object.keys(next)).toEqual(['lead:run-2'])
    expect(next['lead:run-2'].startedAtMs).toBe(2_000)
  })

  it('is idempotent for replayed frames of the same phase', () => {
    const once = reduceProgressFrame(base, frame({ phase: 'tool_call', tool: 'read' }), 3_000)!
    const twice = reduceProgressFrame(once, frame({ phase: 'tool_call', tool: 'read' }), 3_000)!
    expect(twice['m-1']).toEqual(once['m-1'])
  })

  it('emptyMemberActivity holds no fabricated observation', () => {
    const blank = emptyMemberActivity('m-1', 'run-1', 1_000)
    expect(blank.startedAtMs).toBeNull()
    expect(blank.thinking).toBe('')
    expect(blank.summary).toBe('')
    expect(blank.elapsedMs).toBeNull()
    expect(blank.lastTool).toBe('')
    expect(blank.terminalStatus).toBeNull()
    expect(blank.synthesizing).toBe(false)
  })
})

describe('resolveMemberStatus', () => {
  const started = emptyMemberActivity('m-1', 'run-1', 10_000)

  it('falls back to the snapshot value with no live observation', () => {
    expect(resolveMemberStatus('running', null, 9_000)).toBe('running')
    expect(resolveMemberStatus('weird-status', undefined, 9_000)).toBe('weird-status')
  })

  it('ignores frames that are older than the snapshot it would override', () => {
    expect(resolveMemberStatus('finished', { ...started, terminalStatus: 'failed' }, 20_000)).toBe('finished')
  })

  it('lets an observed terminal state beat a stale running snapshot', () => {
    expect(resolveMemberStatus('running', { ...started, terminalStatus: 'failed', lastEventAtMs: 30_000 }, 20_000)).toBe('failed')
    expect(resolveMemberStatus('running', { ...started, terminalStatus: 'finished', lastEventAtMs: 30_000 }, 20_000)).toBe('finished')
  })

  it('never downgrades a snapshot that already reached a terminal state', () => {
    expect(resolveMemberStatus('failed', { ...started, liveStatus: 'running', lastEventAtMs: 30_000 }, 20_000)).toBe('failed')
  })

  it('reports thinking only for a real member_thinking frame', () => {
    expect(
      resolveMemberStatus('running', { ...started, liveStatus: 'running', lastPhase: 'member_thinking', lastEventAtMs: 30_000 }, 20_000),
    ).toBe('thinking')
    expect(
      resolveMemberStatus('running', { ...started, liveStatus: 'running', lastPhase: 'tool_call', lastEventAtMs: 30_000 }, 20_000),
    ).toBe('running')
  })
})

describe('useTeamBoard orchestration frames', () => {
  it('subscribes to session.event.progress and folds frames into the activity map', async () => {
    const state = runComposable()
    await settle()

    expect(rpcMock.listeners.has(TEAM_PROGRESS_EVENT)).toBe(true)
    const handler = rpcMock.listeners.get(TEAM_PROGRESS_EVENT)

    handler?.(frame({ phase: 'member_thinking', memberId: 'sub-1', round: 1, thinking: '检查边界条件' }))
    expect(state.activity.value['sub-1']?.thinking).toBe('检查边界条件')

    handler?.({ kind: 'orchestration', phase: 'run_failed', runId: 'run-x', error: '已取消' })
    expect(state.activity.value['lead:run-x']?.terminalStatus).toBe('failed')
  })

  it('triggers exactly one coalesced read after a frame burst instead of starving it', async () => {
    runComposable()
    await settle()
    const handler = rpcMock.listeners.get(TEAM_PROGRESS_EVENT)
    expect(handler).toBeTypeOf('function')

    // 每 200ms 一帧、共 6 帧（< 合并窗口 400ms 的整数倍）。
    // 重置式防抖会让定时器永远不到期 —— 读数被帧风暴饿死。
    for (let index = 0; index < 6; index += 1) {
      handler?.(frame({ memberId: `m-${index}` }))
      await vi.advanceTimersByTimeAsync(200)
    }
    const calls = rpcMock.call.mock.calls.length
    expect(calls).toBeGreaterThanOrEqual(3)
    expect(calls).toBeLessThan(1 + 6)
  })

  it('ignores non-orchestration traffic on the same event', async () => {
    const state = runComposable()
    await settle()
    rpcMock.listeners.get(TEAM_PROGRESS_EVENT)?.({ kind: 'tool', phase: 'tool_call', memberId: 'm-1', tool: 'read' })
    expect(state.activity.value).toEqual({})
  })

  it('drops live activity when the gateway method disappears', async () => {
    const state = runComposable()
    await settle()
    rpcMock.listeners.get(TEAM_PROGRESS_EVENT)?.(frame({ memberId: 'sub-1', thinking: 'x' }))
    expect(state.activity.value['sub-1']).toBeTruthy()

    rpcMock.methods.value = []
    await settle()
    expect(state.activity.value).toEqual({})
    expect(state.run.value).toEqual({ ...EMPTY_RUN_INFO })
  })

  it('exposes the run meta of the snapshot it just read', async () => {
    rpcMock.call.mockResolvedValue(
      payload({ runId: 'run-9', form: 'team', memberLimit: 512, dropped: 4, note: 'note text' }),
    )
    const state = runComposable()
    await settle()

    expect(state.run.value).toMatchObject({ runId: 'run-9', form: 'team', memberLimit: 512, dropped: 4, note: 'note text' })
  })
})

