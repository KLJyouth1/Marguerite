import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const rpcMock = vi.hoisted(() => ({
  supported: true,
  call: null as unknown as ReturnType<typeof vi.fn>,
}))

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({
    supportsMethod: () => rpcMock.supported,
    call: (...args: unknown[]) =>
      (rpcMock.call as unknown as (...a: unknown[]) => unknown)(...args),
  }),
}))

import {
  EMPTY_PROVENANCE,
  FACTS_DELETE_METHOD,
  PROVENANCE_METHOD,
  groupProvenance,
  useProvenance,
} from './useProvenance'

function fullBrief() {
  return {
    scope: 'project',
    workspace: '/repo/marguerite',
    pinned: [{ text: 'Always answer in Chinese', source: 'user.pinned', at: '2026-10-30T08:00:00Z' }],
    preferences: [{ key: 'tone', value: 'concise', confidence: 0.82, evidence: 4 }],
    facts: [{ text: 'Build uses Vite', source: 'repo.docs', score: 0.64 }],
    open_loops: ['finish the drawer'],
    projection: {
      policy: 'l2_workspace_scoped',
      droppedCount: 1,
      dropped: [{ key: 'repo.layout', reason: 'workspace_derivable' }],
    },
    budget: { max_chars: 800, used: 512, notes: 'trimmed to the workspace scope' },
    truncated: true,
  }
}

beforeEach(() => {
  rpcMock.supported = true
  rpcMock.call = vi.fn()
})

afterEach(() => {
  vi.clearAllMocks()
})

describe('groupProvenance', () => {
  it('keeps every field of the complete contract shape (projection included)', () => {
    const model = groupProvenance(fullBrief())

    expect(model.scopeLabel).toBe('project')
    expect(model.workspace).toBe('/repo/marguerite')
    expect(model.pinned).toEqual([
      { text: 'Always answer in Chinese', source: 'user.pinned', at: '2026-10-30T08:00:00Z' },
    ])
    expect(model.preferences).toEqual([
      { key: 'tone', value: 'concise', confidence: 0.82, evidence: 4 },
    ])
    expect(model.facts).toEqual([
      { text: 'Build uses Vite', source: 'repo.docs', score: 0.64, id: '', deletable: false },
    ])
    expect(model.hasProjection).toBe(true)
    expect(model.policy).toBe('l2_workspace_scoped')
    expect(model.dropped).toEqual([{ key: 'repo.layout', reason: 'workspace_derivable' }])
    expect(model.budget).toEqual({
      maxChars: 800,
      used: 512,
      notes: 'trimmed to the workspace scope',
    })
  })

  it('reports a missing projection honestly instead of faking "nothing dropped"', () => {
    const payload = fullBrief()
    const noProjection: Record<string, unknown> = { ...payload }
    delete noProjection.projection

    const model = groupProvenance(noProjection)

    expect(model.hasProjection).toBe(false)
    expect(model.policy).toBe('')
    expect(model.dropped).toEqual([])
    // The rest of the brief is still real data and must survive.
    expect(model.facts).toHaveLength(1)
    expect(model.budget.maxChars).toBe(800)
  })

  it('defaults missing fields to empty strings / zero instead of leaking undefined', () => {
    const model = groupProvenance({ scope: 'global' })

    expect(model.scopeLabel).toBe('global')
    expect(model.workspace).toBe('')
    expect(model.pinned).toEqual([])
    expect(model.preferences).toEqual([])
    expect(model.facts).toEqual([])
    expect(model.dropped).toEqual([])
    expect(model.budget).toEqual({ maxChars: 0, used: 0, notes: '' })

    // No value anywhere in the model may be undefined (templates read them directly).
    const serialized = JSON.stringify(model)
    expect(serialized).not.toContain('undefined')
    for (const value of Object.values(model)) {
      expect(value).not.toBeUndefined()
    }
  })

  it('collapses non-object input (null / string / array) to the empty model', () => {
    for (const input of [null, undefined, 'nope', 42, ['not', 'an', 'object']]) {
      expect(groupProvenance(input)).toEqual(EMPTY_PROVENANCE)
    }
  })

  it('coerces malformed entries inside arrays without throwing', () => {
    const model = groupProvenance({
      pinned: [null, { text: 7, source: { nested: true } }],
      facts: ['bad', { text: 'ok', score: 'NaN' }],
      projection: { dropped: [42, { key: 'k', reason: 9 }] },
      budget: { max_chars: 'many', used: Number.POSITIVE_INFINITY },
    })

    // Non-object entries are skipped; object entries get converged defaults.
    expect(model.pinned).toEqual([{ text: '', source: '', at: '' }])
    expect(model.facts).toEqual([{ text: 'ok', source: '', score: 0, id: '', deletable: false }])
    expect(model.dropped).toEqual([{ key: 'k', reason: '' }])
    expect(model.budget.maxChars).toBe(0)
    expect(model.budget.used).toBe(0)
    expect(model.hasProjection).toBe(true)
  })
})

describe('useProvenance.loadProvenance', () => {
  it('goes unsupported (and never calls RPC) when the gateway lacks the method', async () => {
    rpcMock.supported = false
    const { state, loadProvenance } = useProvenance()

    await loadProvenance()

    expect(state.value).toBe('unsupported')
    expect(rpcMock.call).not.toHaveBeenCalled()
  })

  it('keeps the raw RPC rejection message in the error state', async () => {
    rpcMock.supported = true
    rpcMock.call.mockRejectedValue(new Error('Disconnected'))
    const { state, error, model, loadProvenance } = useProvenance()

    await loadProvenance()

    expect(state.value).toBe('error')
    expect(error.value).toBe('Disconnected')
    expect(model.value).toEqual(EMPTY_PROVENANCE)
  })

  it('lands on ready with the grouped model when the gateway answers', async () => {
    rpcMock.supported = true
    rpcMock.call.mockResolvedValue(fullBrief())
    const { state, model, loadProvenance } = useProvenance()

    await loadProvenance()

    expect(state.value).toBe('ready')
    expect(rpcMock.call).toHaveBeenCalledWith(PROVENANCE_METHOD, {})
    expect(model.value.scopeLabel).toBe('project')
    expect(model.value.hasProjection).toBe(true)
  })
})

describe('useProvenance.deleteFact (R-02 可撤销性)', () => {
  const deletableBrief = () => ({
    ...fullBrief(),
    facts: [
      {
        text: '用户禁忌：不要写 emoji',
        source: 'project:app#cross_project_facts.jsonl',
        score: 0.9,
        id: 'fact:7',
        deletable: true,
      },
      {
        text: '外部工具写的项目记忆',
        source: 'project:app#project_memory.md',
        score: 0.5,
      },
    ],
  })

  it('maps id/deletable and never invents deletability', () => {
    const model = groupProvenance(deletableBrief())
    expect(model.facts[0]).toMatchObject({ id: 'fact:7', deletable: true })
    expect(model.facts[1]).toMatchObject({ id: '', deletable: false })

    // 旧网关没有 deletable 字段：必须当不可删（宁可没按钮，也不给点了会失败的按钮）
    const legacy = groupProvenance({
      facts: [{ text: 'x', source: 's', score: 1, id: 'fact:1' }],
    })
    expect(legacy.facts[0].deletable).toBe(false)
  })

  it('calls the RPC then re-reads the brief (never patches the array locally)', async () => {
    rpcMock.supported = true
    rpcMock.call
      .mockResolvedValueOnce(deletableBrief()) // call#1：初次读简报
      .mockResolvedValueOnce({ success: true, deleted: true, id: 'fact:7' }) // call#2：删除
      .mockResolvedValueOnce({
        ...deletableBrief(),
        facts: deletableBrief().facts.slice(1), // call#3：删除后重取，只剩外部事实
      })
    const { model, deleting, deleteFact, loadProvenance } = useProvenance()
    await loadProvenance() // call#1：打开抽屉时读简报

    const ok = await deleteFact('fact:7')

    expect(ok).toBe(true)
    expect(rpcMock.call).toHaveBeenNthCalledWith(2, FACTS_DELETE_METHOD, { id: 'fact:7' })
    expect(rpcMock.call).toHaveBeenNthCalledWith(3, PROVENANCE_METHOD, {})
    expect(model.value.facts).toHaveLength(1)
    expect(model.value.facts[0].text).toBe('外部工具写的项目记忆')
    expect(deleting.value).toBe('')
  })

  it('treats {success:false} as failure and keeps the server message', async () => {
    rpcMock.supported = true
    rpcMock.call.mockResolvedValueOnce({
      success: false,
      code: 'ASSISTANT_FACT_DELETE_FAILED',
      message: '落盘失败: Permission denied',
    })
    const { error, deleteFact } = useProvenance()

    const ok = await deleteFact('fact:7')

    expect(ok).toBe(false)
    expect(error.value).toBe('落盘失败: Permission denied')
    // 失败后不得重取简报（重取会把用户刚看到的列表换掉，看起来像删成功了）
    expect(rpcMock.call).toHaveBeenCalledTimes(1)
  })

  it('does not call the gateway at all when the method is undeclared', async () => {
    rpcMock.supported = false
    const { deleteFact, error } = useProvenance()

    expect(await deleteFact('fact:7')).toBe(false)
    expect(rpcMock.call).not.toHaveBeenCalled()
    expect(error.value).toContain(FACTS_DELETE_METHOD)
  })

  it('keeps the raw rejection message on transport errors', async () => {
    rpcMock.supported = true
    rpcMock.call.mockRejectedValueOnce(new Error('Disconnected'))
    const { deleteFact, error } = useProvenance()

    expect(await deleteFact('fact:7')).toBe(false)
    expect(error.value).toBe('Disconnected')
  })
})
