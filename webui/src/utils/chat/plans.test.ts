import { describe, expect, it } from 'vitest'

import {
  normalizeCollaborationSnapshot,
  normalizePlanRevisionSnapshot,
  normalizePlanRunSnapshot,
  payloadBelongsToSession,
  planRevisionsFromToolSegments,
} from './plans'

describe('plan payload normalization', () => {
  it('normalizes the collaboration envelope without losing a fallback revision', () => {
    expect(normalizeCollaborationSnapshot({
      session_key: 'agent:main:webchat:one',
      collaboration: { mode: 'plan', revision: 4 },
    })).toEqual({ mode: 'plan', revision: 4 })
    expect(normalizeCollaborationSnapshot({
      collaborationMode: 'plan',
      collaborationRevision: 5,
    })).toEqual({ mode: 'plan', revision: 5 })

    expect(normalizeCollaborationSnapshot(
      { collaboration: { mode: 'unknown' } },
      { mode: 'default', revision: 7 },
    )).toEqual({ mode: 'default', revision: 7 })
  })

  it('accepts plan revision envelopes from events, RPC responses, and history parts', () => {
    const payload = {
      revision_id: 'revision-2',
      plan_id: 'plan-1',
      parent_revision_id: 'revision-1',
      generation: 2,
      title: 'Ship plan mode',
      markdown: 'A complete plan.',
      steps: [{ step_id: 'inspect', title: 'Inspect', details: 'Read the runtime.' }],
      created_at: 123,
    }

    expect(normalizePlanRevisionSnapshot({ plan_revision: payload })).toEqual({
      revisionId: 'revision-2',
      planId: 'plan-1',
      parentRevisionId: 'revision-1',
      generation: 2,
      title: 'Ship plan mode',
      markdown: 'A complete plan.',
      steps: [{ stepId: 'inspect', title: 'Inspect', details: 'Read the runtime.' }],
      current: false,
      createdAt: 123,
    })
    expect(normalizePlanRevisionSnapshot({ planRevision: payload })?.revisionId)
      .toBe('revision-2')

    expect(planRevisionsFromToolSegments([
      { type: 'text', text: 'not a plan' },
      { type: 'plan', snapshot: payload },
    ], 'revision-2')).toEqual([
      expect.objectContaining({ revisionId: 'revision-2', current: true }),
    ])
  })

  it('normalizes server-authoritative run state and rejects malformed snapshots', () => {
    expect(normalizePlanRunSnapshot({
      plan_run: {
        run_id: 'run-1',
        plan_revision_id: 'revision-2',
        status: 'running',
        current_step_id: 'build',
        state_revision: 8,
        driver_kind: 'goal',
        driver_id: 'goal-1',
        active_task_id: 'task-1',
        step_states: [
          { step_id: 'inspect', title: 'Inspect', status: 'completed' },
          { step_id: 'build', title: 'Build', status: 'in_progress' },
        ],
      },
    })).toEqual({
      runId: 'run-1',
      planRevisionId: 'revision-2',
      status: 'running',
      currentStepId: 'build',
      stateRevision: 8,
      driverKind: 'goal',
      driverId: 'goal-1',
      activeTaskId: 'task-1',
      steps: [
        { stepId: 'inspect', title: 'Inspect', status: 'completed' },
        { stepId: 'build', title: 'Build', status: 'in_progress' },
      ],
    })

    expect(normalizePlanRevisionSnapshot({ title: 'Missing ids' })).toBeNull()
    expect(normalizePlanRunSnapshot({
      runId: 'run-1',
      planRevisionId: 'revision-2',
      status: 'invented',
    })).toBeNull()
  })

  it('normalizes B4 run enrichments (checklist / parallelBatch / replan)', () => {
    expect(normalizePlanRunSnapshot({
      plan_run: {
        run_id: 'run-b4',
        plan_revision_id: 'revision-2',
        status: 'running',
        current_step_id: 'build',
        steps: [
          { step_id: 'inspect', title: 'Inspect', status: 'completed' },
          { step_id: 'build', title: 'Build', status: 'in_progress' },
        ],
        checklist: [
          { step_id: 'inspect', title: 'Inspect', checked: true },
          { step_id: 'build', title: 'Build', checked: false },
        ],
        parallel_batch: {
          status: 'failed',
          successful: 1,
          failed: 1,
          total: 2,
          results: [
            { step_id: 'inspect', title: 'Inspect', status: 'completed', summary: 'done' },
            { step_id: 'build', title: 'Build', status: 'failed', summary: 'failed' },
          ],
        },
        replan: {
          revision_id: 'revision-3',
          plan_id: 'plan-1',
          previous_revision_id: 'revision-2',
          similarity: 0.07,
          prompt: '遭遇网络超时',
          steps_changed: 4,
          created_at: 123,
        },
      },
    })).toEqual({
      runId: 'run-b4',
      planRevisionId: 'revision-2',
      status: 'running',
      currentStepId: 'build',
      steps: [
        { stepId: 'inspect', title: 'Inspect', status: 'completed' },
        { stepId: 'build', title: 'Build', status: 'in_progress' },
      ],
      checklist: [
        { stepId: 'inspect', title: 'Inspect', checked: true },
        { stepId: 'build', title: 'Build', checked: false },
      ],
      parallelBatch: {
        status: 'failed',
        successful: 1,
        failed: 1,
        total: 2,
        results: [
          { stepId: 'inspect', title: 'Inspect', status: 'completed', summary: 'done' },
          { stepId: 'build', title: 'Build', status: 'failed', summary: 'failed' },
        ],
      },
      replan: {
        revisionId: 'revision-3',
        planId: 'plan-1',
        previousRevisionId: 'revision-2',
        similarity: 0.07,
        prompt: '遭遇网络超时',
        stepsChanged: 4,
        createdAt: 123,
      },
    })

    // 空/畸形清单项被丢弃，缺 parallel 数值字段时整块丢弃
    expect(normalizePlanRunSnapshot({
      runId: 'run-b4',
      planRevisionId: 'revision-2',
      status: 'running',
      steps: [],
      checklist: [{ step_id: '', title: 'Bad' }, { step_id: 'ok', title: 'Good', checked: 'true' }],
      parallelBatch: { status: 'completed' },
      replan: { revisionId: 'revision-3', planId: 'plan-1' },
    })).toEqual({
      runId: 'run-b4',
      planRevisionId: 'revision-2',
      status: 'running',
      steps: [],
      checklist: [{ stepId: 'ok', title: 'Good', checked: true }],
      replan: {
        revisionId: 'revision-3',
        planId: 'plan-1',
        previousRevisionId: '',
        similarity: 0,
        prompt: '',
        stepsChanged: 0,
      },
    })
  })

  it('keeps real per-step timing on parallel results, omitting absent/invalid values', () => {
    const run = normalizePlanRunSnapshot({
      plan_run: {
        run_id: 'run-timing',
        plan_revision_id: 'rev-1',
        status: 'completed',
        steps: [],
        parallel_batch: {
          status: 'completed',
          successful: 3,
          failed: 0,
          total: 3,
          results: [
            // 真实耗时 + 重试：必须原样保留（前端据此显示单步耗时/重试徽标）
            { step_id: 'a', title: 'A', status: 'completed', summary: 'ok', elapsed_ms: 1234, attempts: 3, timed_out: false },
            // 只耗时、一次通过：attempts 如实保留（是否显示由 UI 决定，不在此处篡改）
            { stepId: 'b', title: 'B', status: 'completed', summary: 'ok', elapsedMs: 900, attempts: 1 },
            // 无有效耗时数据：耗时字段都不出现（绝不补 0 冒充），但超时标记独立保留
            { stepId: 'c', title: 'C', status: 'failed', summary: 'boom', attempts: 0, elapsedMs: -5, timed_out: true },
          ],
        },
      },
    })

    expect(run?.parallelBatch?.results).toEqual([
      { stepId: 'a', title: 'A', status: 'completed', summary: 'ok', elapsedMs: 1234, attempts: 3 },
      { stepId: 'b', title: 'B', status: 'completed', summary: 'ok', elapsedMs: 900, attempts: 1 },
      { stepId: 'c', title: 'C', status: 'failed', summary: 'boom', timedOut: true },
    ])
  })

  it('normalizes the lookahead replan (mode / skipped steps / best match)', () => {
    const run = normalizePlanRunSnapshot({
      plan_run: {
        run_id: 'run-look',
        plan_revision_id: 'rev-1',
        status: 'running',
        current_step_id: 's3',
        steps: [{ step_id: 's3', title: 'Deploy', status: 'in_progress' }],
        replan: {
          mode: 'advance',
          revision_id: 'rev-1',
          plan_id: 'plan-1',
          previous_revision_id: 'rev-1',
          similarity: 0.04,
          prompt: 'zulu yankee',
          steps_changed: 2,
          skipped_step_ids: ['s1', 's2', ''],
          best_match: { step_id: 's3', title: 'Deploy', similarity: 0.91, distance: 2 },
        },
      },
    })

    expect(run?.replan).toEqual({
      revisionId: 'rev-1',
      planId: 'plan-1',
      previousRevisionId: 'rev-1',
      similarity: 0.04,
      prompt: 'zulu yankee',
      stepsChanged: 2,
      mode: 'advance',
      skippedStepIds: ['s1', 's2'],
      bestMatch: { stepId: 's3', title: 'Deploy', similarity: 0.91, distance: 2 },
    })

    // 未知模式不冒充已知语义（缺 mode 即按旧契约渲染为「重新拆解」）
    expect(normalizePlanRunSnapshot({
      runId: 'run-x',
      planRevisionId: 'rev-1',
      status: 'running',
      steps: [],
      replan: { revisionId: 'r', planId: 'p', mode: 'invented' },
    })?.replan?.mode).toBeUndefined()
  })

  it('accepts the run cost model only when it carries real samples', () => {
    const base = {
      runId: 'run-cost',
      planRevisionId: 'rev-1',
      status: 'running',
      steps: [],
    }
    // 有实测样本 → 原样保留（前端据此在事件流缺位时兜底显示 ETA）
    expect(normalizePlanRunSnapshot({
      ...base,
      cost: { remaining_steps: 2, sample_count: 3, avg_step_ms: 1500, estimated_remaining_ms: 3000 },
    })?.cost).toEqual({
      remainingSteps: 2,
      sampleCount: 3,
      avgStepMs: 1500,
      estimatedRemainingMs: 3000,
    })

    // 无样本（引擎回传 null）→ ETA 必须归零为 null，绝不显示假进度
    expect(normalizePlanRunSnapshot({
      ...base,
      cost: { remainingSteps: 2, sampleCount: 0, avgStepMs: null, estimatedRemainingMs: null },
    })?.cost).toEqual({
      remainingSteps: 2,
      sampleCount: 0,
      avgStepMs: null,
      estimatedRemainingMs: null,
    })

    // 缺剩余步数/样本数 → 整块丢弃（不猜测）
    expect(normalizePlanRunSnapshot({ ...base, cost: { remainingSteps: 2 } })?.cost).toBeUndefined()
  })

  it('filters events to the active session while accepting payloads without a key', () => {
    expect(payloadBelongsToSession(
      { session_key: 'agent:main:webchat:one' },
      'agent:main:webchat:one',
    )).toBe(true)
    expect(payloadBelongsToSession(
      { sessionKey: 'agent:main:webchat:two' },
      'agent:main:webchat:one',
    )).toBe(false)
    expect(payloadBelongsToSession({ planRun: {} }, 'agent:main:webchat:one')).toBe(true)
  })
})
