import type {
  CollaborationMode,
  CollaborationSnapshot,
  PlanParallelBatch,
  PlanReplanInfo,
  PlanReplanLookaheadHit,
  PlanReplanMode,
  PlanRevisionSnapshot,
  PlanRevisionStep,
  PlanRunChecklistItem,
  PlanRunCost,
  PlanRunSnapshot,
  PlanRunStatus,
  PlanRunStepSnapshot,
  PlanRunStepStatus,
} from '@/types/plans'

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}

function stringField(source: Record<string, unknown>, ...keys: string[]): string {
  for (const key of keys) {
    const value = source[key]
    if (typeof value === 'string' && value.trim()) return value
  }
  return ''
}

function numberField(source: Record<string, unknown>, ...keys: string[]): number | undefined {
  for (const key of keys) {
    const raw = source[key]
    if (raw === null || raw === undefined || raw === '') continue
    const value = Number(raw)
    if (Number.isFinite(value) && value >= 0) return value
  }
  return undefined
}

function booleanField(source: Record<string, unknown>, ...keys: string[]): boolean | undefined {
  for (const key of keys) {
    const value = source[key]
    if (typeof value === 'boolean') return value
  }
  return undefined
}

function nestedSnapshot(value: unknown, keys: string[]): unknown {
  const source = record(value)
  if (!source) return value
  for (const key of keys) {
    if (source[key] !== undefined) return source[key]
  }
  return value
}

function normalizePlanStep(value: unknown): PlanRevisionStep | null {
  const source = record(value)
  if (!source) return null
  const stepId = stringField(source, 'stepId', 'step_id', 'id')
  const title = stringField(source, 'title', 'label')
  if (!stepId || !title) return null
  const details = stringField(source, 'details', 'description')
  return {
    stepId,
    title,
    ...(details ? { details } : {}),
  }
}

const RUN_STATUSES = new Set<PlanRunStatus>([
  'queued',
  'running',
  'paused',
  'blocked',
  'completed',
  'cancelled',
  'superseded',
])

const STEP_STATUSES = new Set<PlanRunStepStatus>([
  'pending',
  'in_progress',
  'completed',
  'blocked',
  'skipped',
])

function normalizeRunStep(value: unknown): PlanRunStepSnapshot | null {
  const source = record(value)
  if (!source) return null
  const stepId = stringField(source, 'stepId', 'step_id', 'id')
  const title = stringField(source, 'title', 'label')
  const rawStatus = stringField(source, 'status', 'state')
  if (!stepId || !title || !STEP_STATUSES.has(rawStatus as PlanRunStepStatus)) return null
  const reason = stringField(source, 'reason')
  return {
    stepId,
    title,
    status: rawStatus as PlanRunStepStatus,
    ...(reason ? { reason } : {}),
  }
}

/** B4：归一化执行前清单项（宽松容错，坏项丢弃） */
function normalizeRunChecklist(value: unknown): PlanRunChecklistItem[] {
  if (!Array.isArray(value)) return []
  return value.flatMap(item => {
    const source = record(item)
    if (!source) return []
    const stepId = stringField(source, 'stepId', 'step_id', 'id')
    const title = stringField(source, 'title', 'label')
    if (!stepId || !title) return []
    return [{
      stepId,
      title,
      checked: source.checked === true || source.checked === 'true',
    }]
  })
}

/** B4：归一化并行子任务批 */
function normalizeParallelBatch(value: unknown): PlanParallelBatch | undefined {
  const source = record(value)
  if (!source) return undefined
  const successful = numberField(source, 'successful', 'succeeded')
  const failed = numberField(source, 'failed')
  const total = numberField(source, 'total')
  if (successful === undefined || failed === undefined || total === undefined) return undefined
  const results = Array.isArray(source.results) ? source.results : []
  return {
    status: source.status === 'failed' ? 'failed' : 'completed',
    successful,
    failed,
    total,
    ...(source.executed === true ? { executed: true } : {}),
    ...(source.simulated === true ? { simulated: true } : {}),
    results: results.flatMap(result => {
      const r = record(result)
      if (!r) return []
      const stepId = stringField(r, 'stepId', 'step_id', 'id')
      const title = stringField(r, 'title', 'label')
      if (!stepId || !title) return []
      // 真实耗时/尝试次数/超时标记：无数据即缺省（前端不显示），绝不补 0 冒充
      const elapsedMs = numberField(r, 'elapsedMs', 'elapsed_ms')
      const attempts = numberField(r, 'attempts')
      return [{
        stepId,
        title,
        status: r.status === 'failed' ? 'failed' : 'completed',
        summary: stringField(r, 'summary', 'detail'),
        ...(elapsedMs !== undefined && elapsedMs >= 0 ? { elapsedMs } : {}),
        ...(attempts !== undefined && attempts > 0 ? { attempts } : {}),
        ...(r.timedOut === true || r.timed_out === true ? { timedOut: true } : {}),
      }]
    }),
  }
}

/** B4：归一化重规划 diff */
function normalizeReplanInfo(value: unknown): PlanReplanInfo | undefined {
  const source = record(value)
  if (!source) return undefined
  const revisionId = stringField(source, 'revisionId', 'revision_id')
  const planId = stringField(source, 'planId', 'plan_id')
  if (!revisionId || !planId) return undefined
  const similarity = numberField(source, 'similarity')
  const stepsChanged = numberField(source, 'stepsChanged', 'steps_changed')
  const createdAt = numberField(source, 'createdAt', 'created_at')
  const mode = normalizeReplanMode(stringField(source, 'mode'))
  const skippedStepIds = stringList(source.skippedStepIds ?? source.skipped_step_ids)
  const bestMatch = normalizeLookaheadHit(source.bestMatch ?? source.best_match)
  return {
    revisionId,
    planId,
    previousRevisionId: stringField(source, 'previousRevisionId', 'previous_revision_id'),
    similarity: similarity ?? 0,
    prompt: stringField(source, 'prompt', 'observation'),
    stepsChanged: stepsChanged ?? 0,
    ...(createdAt !== undefined ? { createdAt } : {}),
    ...(mode ? { mode } : {}),
    ...(skippedStepIds.length ? { skippedStepIds } : {}),
    ...(bestMatch ? { bestMatch } : {}),
  }
}

const REPLAN_MODES = new Set<PlanReplanMode>(['on_plan', 'advance', 'rebuild'])

function normalizeReplanMode(value: string): PlanReplanMode | undefined {
  return REPLAN_MODES.has(value as PlanReplanMode) ? value as PlanReplanMode : undefined
}

function stringList(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === 'string' && item.trim().length > 0)
    : []
}

function normalizeLookaheadHit(value: unknown): PlanReplanLookaheadHit | undefined {
  const source = record(value)
  if (!source) return undefined
  const stepId = stringField(source, 'stepId', 'step_id')
  if (!stepId) return undefined
  return {
    stepId,
    title: stringField(source, 'title', 'label'),
    similarity: numberField(source, 'similarity') ?? 0,
    distance: numberField(source, 'distance') ?? 0,
  }
}

/**
 * 剩余代价模型：仅在带**实测样本**（`sampleCount > 0`）时才接受 ETA。
 * 无样本时引擎回传 null，前端同样不显示——缺证据就不给用户看假进度。
 */
function normalizeRunCost(value: unknown): PlanRunCost | undefined {
  const source = record(value)
  if (!source) return undefined
  const remainingSteps = numberField(source, 'remainingSteps', 'remaining_steps')
  const sampleCount = numberField(source, 'sampleCount', 'sample_count')
  if (remainingSteps === undefined || sampleCount === undefined) return undefined
  const avgStepMs = numberField(source, 'avgStepMs', 'avg_step_ms')
  const estimatedRemainingMs = numberField(source, 'estimatedRemainingMs', 'estimated_remaining_ms')
  return {
    remainingSteps,
    sampleCount,
    avgStepMs: sampleCount > 0 && avgStepMs !== undefined ? avgStepMs : null,
    estimatedRemainingMs: sampleCount > 0 && estimatedRemainingMs !== undefined
      ? estimatedRemainingMs
      : null,
  }
}

export function normalizeCollaborationSnapshot(
  value: unknown,
  fallback: CollaborationSnapshot = { mode: 'default', revision: 0 },
): CollaborationSnapshot {
  const source = record(nestedSnapshot(value, ['collaboration'])) ?? {}
  const rawMode = stringField(source, 'mode', 'collaborationMode', 'collaboration_mode')
  const mode: CollaborationMode = rawMode === 'plan'
    ? 'plan'
    : rawMode === 'default'
      ? 'default'
      : fallback.mode
  return {
    mode,
    revision: numberField(
      source,
      'revision',
      'collaborationRevision',
      'collaboration_revision',
    ) ?? fallback.revision,
  }
}

export function normalizePlanRevisionSnapshot(
  value: unknown,
  currentRevisionId = '',
): PlanRevisionSnapshot | null {
  const source = record(nestedSnapshot(value, [
    'snapshot',
    'plan',
    'planRevision',
    'plan_revision',
    'currentPlan',
    'current_plan',
  ]))
  if (!source) return null
  const revisionId = stringField(source, 'revisionId', 'revision_id')
  const planId = stringField(source, 'planId', 'plan_id')
  const title = stringField(source, 'title')
  if (!revisionId || !planId || !title) return null
  const stepsRaw = Array.isArray(source.steps) ? source.steps : []
  const parentRevisionId = stringField(source, 'parentRevisionId', 'parent_revision_id')
  const generation = numberField(source, 'generation')
  const createdAt = numberField(source, 'createdAt', 'created_at')
  const explicitCurrent = booleanField(source, 'current')
  // A5：编排形态机器键 + 决策理由（纯增、向后兼容；旧修订无此二字段）。
  const strategy = stringField(source, 'strategy')
  const rationale = stringField(source, 'rationale')
  return {
    revisionId,
    planId,
    ...(parentRevisionId ? { parentRevisionId } : {}),
    ...(generation !== undefined ? { generation } : {}),
    title,
    markdown: typeof source.markdown === 'string' ? source.markdown : '',
    steps: stepsRaw.flatMap(step => normalizePlanStep(step) ?? []),
    ...(strategy ? { strategy } : {}),
    ...(rationale ? { rationale } : {}),
    current: explicitCurrent ?? (Boolean(currentRevisionId) && revisionId === currentRevisionId),
    ...(createdAt !== undefined ? { createdAt } : {}),
  }
}

export function normalizePlanRunSnapshot(value: unknown): PlanRunSnapshot | null {
  const source = record(nestedSnapshot(value, [
    'snapshot',
    'run',
    'planRun',
    'plan_run',
    'activePlanRun',
    'active_plan_run',
  ]))
  if (!source) return null
  const runId = stringField(source, 'runId', 'run_id')
  const planRevisionId = stringField(source, 'planRevisionId', 'plan_revision_id')
  const rawStatus = stringField(source, 'status')
  if (!runId || !planRevisionId || !RUN_STATUSES.has(rawStatus as PlanRunStatus)) return null
  const stepsRaw = Array.isArray(source.steps)
    ? source.steps
    : Array.isArray(source.stepStates)
      ? source.stepStates
      : Array.isArray(source.step_states)
        ? source.step_states
        : []
  const currentStepId = stringField(source, 'currentStepId', 'current_step_id')
  const stateRevision = numberField(source, 'stateRevision', 'state_revision')
  const driverKind = stringField(source, 'driverKind', 'driver_kind')
  const driverId = stringField(source, 'driverId', 'driver_id')
  const activeTaskId = stringField(source, 'activeTaskId', 'active_task_id')
  const pauseReason = stringField(source, 'pauseReason', 'pause_reason')
  const terminalReason = stringField(source, 'terminalReason', 'terminal_reason')
  const createdAt = numberField(source, 'createdAt', 'created_at')
  const updatedAt = numberField(source, 'updatedAt', 'updated_at')
  const startedAt = numberField(source, 'startedAt', 'started_at')
  const finishedAt = numberField(source, 'finishedAt', 'finished_at')
  const checklist = normalizeRunChecklist(source.checklist)
  const parallelBatch = normalizeParallelBatch(source.parallelBatch ?? source.parallel_batch)
  const replan = normalizeReplanInfo(source.replan)
  const cost = normalizeRunCost(source.cost)
  return {
    runId,
    planRevisionId,
    status: rawStatus as PlanRunStatus,
    ...(currentStepId ? { currentStepId } : {}),
    ...(stateRevision !== undefined ? { stateRevision } : {}),
    ...(driverKind ? { driverKind } : {}),
    ...(driverId ? { driverId } : {}),
    ...(activeTaskId ? { activeTaskId } : {}),
    ...(pauseReason ? { pauseReason } : {}),
    ...(terminalReason ? { terminalReason } : {}),
    ...(createdAt !== undefined ? { createdAt } : {}),
    ...(updatedAt !== undefined ? { updatedAt } : {}),
    ...(startedAt !== undefined ? { startedAt } : {}),
    ...(finishedAt !== undefined ? { finishedAt } : {}),
    steps: stepsRaw.flatMap(step => normalizeRunStep(step) ?? []),
    ...(checklist.length ? { checklist } : {}),
    ...(parallelBatch ? { parallelBatch } : {}),
    ...(replan ? { replan } : {}),
    ...(cost ? { cost } : {}),
  }
}

export function planRevisionsFromToolSegments(
  segments: unknown,
  currentRevisionId = '',
): PlanRevisionSnapshot[] {
  if (!Array.isArray(segments)) return []
  return segments.flatMap(segment => {
    const source = record(segment)
    if (!source || source.type !== 'plan') return []
    const snapshot = normalizePlanRevisionSnapshot(source, currentRevisionId)
    return snapshot ? [snapshot] : []
  })
}

export function payloadBelongsToSession(value: unknown, sessionKey: string): boolean {
  const source = record(value)
  if (!source || !sessionKey) return false
  const key = stringField(source, 'sessionKey', 'session_key', 'key')
  return !key || key === sessionKey
}
