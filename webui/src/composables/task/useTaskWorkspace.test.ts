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
  })
})
