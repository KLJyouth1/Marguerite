// Pure state-machine tests for the task workspace (T05, P0-4).
//
// `createTaskWorkspace` is dependency-free (no Pinia/RPC), so these cover the
// event → state mapping in isolation: normalization of both wire shapes, status
// transitions, artifact upserts and the terminal quality gate.
import { describe, expect, it } from 'vitest'
import { createTaskWorkspace, normalizeTaskSteps } from '@/composables/task/useTaskWorkspace'

describe('normalizeTaskSteps', () => {
  it('normalizes the pipeline shape and fills stable ids', () => {
    const steps = normalizeTaskSteps([
      { id: 'a', role: 'r', title: 'Research', kind: 'Research', target_path: 'notes/a.md', depends_on: [] },
    ])
    expect(steps).toHaveLength(1)
    expect(steps[0]).toMatchObject({
      id: 'a',
      role: 'r',
      title: 'Research',
      kind: 'Research',
      targetPath: 'notes/a.md',
      status: 'pending',
    })
  })

  it('degrades the non-pipeline shape (title falls back to role) without inventing data', () => {
    const steps = normalizeTaskSteps([{ role: 'researcher', title: 'Research' }, { role: 'writer' }])
    expect(steps.map(s => s.id)).toEqual(['d1', 'd2'])
    expect(steps[0].title).toBe('Research')
    expect(steps[1].title).toBe('writer')
    expect(steps[0].kind).toBe('Other')
    expect(steps[0].targetPath).toBeNull()
  })

  it('returns an empty list for a non-array payload (never fabricates steps)', () => {
    expect(normalizeTaskSteps(undefined)).toEqual([])
    expect(normalizeTaskSteps({})).toEqual([])
  })
})

describe('createTaskWorkspace', () => {
  it('drives steps, artifacts and the terminal gate from the event stream', () => {
    const ws = createTaskWorkspace()

    ws.applyRouterDecision({
      strategy: 'Solo Agent',
      mode_route: 'Direct',
      decision: {
        pipeline: true,
        subtasks: [{ id: 'd1', title: 'Write report', kind: 'Document', target_path: 'report.md' }],
      },
    })
    expect(ws.steps.value).toHaveLength(1)
    expect(ws.status.value).toBe('running')
    expect(ws.modeRoute.value).toBe('Direct')
    expect(ws.strategy.value).toBe('Solo Agent')
    expect(ws.pipeline.value).toBe(true)

    ws.applyStepStart({ task_step_id: 'd1', status: 'in_progress' })
    expect(ws.steps.value[0].status).toBe('in_progress')

    ws.applyStepFinish({ task_step_id: 'd1', status: 'done', elapsed_ms: 900, artifact_path: 'report.md' })
    expect(ws.steps.value[0].status).toBe('done')
    expect(ws.steps.value[0].elapsedMs).toBe(900)
    expect(ws.artifacts.value.map(a => a.path)).toEqual(['report.md'])

    ws.applyArtifactUpdated({ path: 'chart.png', kind: 'Other' })
    expect(ws.artifacts.value).toHaveLength(2)

    ws.applyDone({
      reason: 'partial',
      completeness: { allowed: false, verdict: 'fail', blocked_by: ['MissingArtifact'] },
    })
    expect(ws.status.value).toBe('partial')
    expect(ws.gate.value?.verdict).toBe('fail')
    expect(ws.gate.value?.artifactsComplete).toBe(false)
    expect(ws.gate.value?.missing).toEqual(['MissingArtifact'])
  })

  it('maps a clean completion to a passing gate', () => {
    const ws = createTaskWorkspace()
    ws.applyDone({ reason: 'completed', completeness: { allowed: true, verdict: 'pass', blocked_by: [] } })
    expect(ws.status.value).toBe('completed')
    expect(ws.gate.value?.verdict).toBe('pass')
    expect(ws.gate.value?.searchValid).toBe(true)
    expect(ws.gate.value?.artifactsComplete).toBe(true)
  })

  it('reset() clears every field back to the idle baseline', () => {
    const ws = createTaskWorkspace()
    ws.applyRouterDecision({ decision: { subtasks: [{ id: 'd1', title: 'A' }] } })
    ws.applyArtifactUpdated({ path: 'a.md' })
    ws.reset()
    expect(ws.steps.value).toHaveLength(0)
    expect(ws.artifacts.value).toHaveLength(0)
    expect(ws.gate.value).toBeNull()
    expect(ws.status.value).toBe('idle')
    expect(ws.modeRoute.value).toBeNull()
    expect(ws.sessionKey.value).toBe('')
  })
})

// 真机顺序：Rust 网关在回合**结束后**才发 `router_decision`（chat.rs 里它在
// run_turn 返回之后），step_start/step_finish 都在它之前。若把 receipt 当成
// 「新结构」整体替换，已完成步骤会被重新画成 pending。
describe('createTaskWorkspace · router_decision 晚到不覆盖已发生状态', () => {
  it('keeps statuses/elapsed/artifacts when the receipt restates the same ids', () => {
    const ws = createTaskWorkspace()
    ws.applyStepStart({ task_step_id: 'd1', title: 'Research', status: 'in_progress' })
    ws.applyStepFinish({ task_step_id: 'd1', status: 'failed', elapsed_ms: 1200 })

    ws.applyRouterDecision({
      strategy: 'Pipeline',
      decision: {
        pipeline: true,
        subtasks: [
          { id: 'd1', role: 'researcher', title: 'Research', kind: 'Research' },
          { id: 'd2', role: 'writer', title: 'Write', kind: 'Document' },
        ],
      },
    })

    expect(ws.steps.value.map(s => s.id)).toEqual(['d1', 'd2'])
    expect(ws.steps.value[0].status).toBe('failed')
    expect(ws.steps.value[0].elapsedMs).toBe(1200)
    expect(ws.steps.value[1].status).toBe('pending')
  })
})

describe('createTaskWorkspace · 跨会话/跨回合隔离', () => {
  const A = 'agent:main:webchat:a'
  const B = 'agent:main:webchat:b'
  const meta = (session: string) => ({ session })

  it('drops finish/artifact/done from another session', () => {
    const ws = createTaskWorkspace()
    ws.applyStepStart({ task_step_id: 'd1', title: 'Mine', status: 'in_progress' }, meta(A))
    expect(ws.sessionKey.value).toBe(A)

    ws.applyStepFinish({ task_step_id: 'd1', status: 'done' }, meta(B))
    ws.applyArtifactUpdated({ path: 'foreign.md' }, meta(B))
    ws.applyDone({ reason: 'completed' }, meta(B))

    expect(ws.steps.value.map(s => s.title)).toEqual(['Mine'])
    expect(ws.steps.value[0].status).toBe('in_progress')
    expect(ws.artifacts.value).toHaveLength(0)
    expect(ws.status.value).toBe('running')
    expect(ws.sessionKey.value).toBe(A)
  })

  it('a step_start from another session follows the newer run', () => {
    const ws = createTaskWorkspace()
    ws.applyStepStart({ task_step_id: 'd1', title: 'Mine', status: 'done' }, meta(A))
    ws.applyStepStart({ task_step_id: 'x1', title: 'Foreign', status: 'in_progress' }, meta(B))

    expect(ws.sessionKey.value).toBe(B)
    expect(ws.steps.value.map(s => s.title)).toEqual(['Foreign'])
  })

  it('a router_decision from another session opens that run instead of merging', () => {
    const ws = createTaskWorkspace()
    ws.applyStepStart({ task_step_id: 'd1', title: 'Mine', status: 'done' }, meta(A))
    ws.applyArtifactUpdated({ path: 'mine.md' }, meta(A))

    ws.applyRouterDecision(
      { strategy: 'Solo Agent', decision: { subtasks: [{ id: 'n1', title: 'Next' }] } },
      meta(B),
    )

    expect(ws.sessionKey.value).toBe(B)
    expect(ws.steps.value.map(s => s.id)).toEqual(['n1'])
    expect(ws.artifacts.value).toHaveLength(0)
    expect(ws.gate.value).toBeNull()
  })

  it('a new turn (different task id) of the same session clears the previous run', () => {
    const ws = createTaskWorkspace()
    ws.applyStepStart({ task_step_id: 'd1', title: 'Old', task_id: 't1' }, meta(A))
    ws.applyArtifactUpdated({ path: 'old.md', task_id: 't1' }, meta(A))
    ws.applyDone({ reason: 'completed', task_id: 't1' }, meta(A))
    expect(ws.status.value).toBe('completed')

    ws.applyStepStart({ task_step_id: 'n1', title: 'New', task_id: 't2' }, meta(A))
    expect(ws.steps.value.map(s => s.title)).toEqual(['New'])
    expect(ws.artifacts.value).toHaveLength(0)
    expect(ws.gate.value).toBeNull()
  })

  it('ignores frames from side chats and non-webchat surfaces', () => {
    const ws = createTaskWorkspace()
    ws.applyStepStart({ task_step_id: 'd1', title: 'Main', status: 'in_progress' }, meta(A))

    ws.applyRouterDecision({ strategy: 'Solo Agent', decision: { subtasks: [{ id: 's1', title: 'Side' }] } }, meta('agent:main:webchat:side-abc'))
    ws.applyRouterDecision({ strategy: 'Solo Agent', decision: { subtasks: [{ id: 'c1', title: 'CLI' }] } }, meta('agent:main:cli:42'))
    ws.applyDone({ reason: 'completed' }, meta('agent:data-analyst:subagent:run-1'))

    expect(ws.sessionKey.value).toBe(A)
    expect(ws.steps.value.map(s => s.title)).toEqual(['Main'])
    expect(ws.status.value).toBe('running')
  })
})
