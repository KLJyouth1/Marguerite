// Task workspace aggregation (T05, P0-4).
//
// The task is a first-class citizen: this composable turns the delivery-pipeline
// event stream into a single, derived view of the running task — its ordered
// steps, their statuses, and the artifacts each step produced. It **consumes**
// the contracts the backend already emits (T03/T04) and never fabricates steps:
//
//   · `session.event.router_decision`  → `decision.subtasks[]` (structure) +
//                                         `strategy` + `mode_route` (real route)
//   · `session.event.step_start`       → a step becomes in_progress
//   · `session.event.step_finish`      → a step becomes done/failed (+ elapsed/tools)
//   · `artifact.updated`               → an artifact lands (links to its step)
//   · `session.event.done`             → terminal status + quality-gate verdict
//
// Events are broadcast to every client, so the workspace is **run-scoped**: it
// adopts the session/turn of the frame it opens with (`meta.session` + task id)
// and drops frames from other sessions, other turns, and non-webchat surfaces.
// Without that guard a background session's steps/done silently overwrite the
// page — the "运行页面串台" defect.
//
// Two layers live here on purpose:
//   1. `createTaskWorkspace()` — a pure, dependency-free factory (state + pure
//      event handlers). Unit-testable with zero Pinia/RPC wiring.
//   2. `useTaskWorkspaceStore()` — the cross-chunk shared instance (Pinia, same
//      rationale as `stores/activeMode`: route-lazy bundles duplicate module
//      singletons) plus idempotent RPC binding.
import { ref } from 'vue'
import { defineStore } from 'pinia'
import type { RpcEventHandler } from '@/lib/rpc'
import type { DeliverableKind, TaskGate, TaskStep, TaskStepStatus } from '@/types/rpc'
import { frameSessionKey, payloadTaskId } from '@/utils/chat/streamEvents'

/** One artifact produced by the task (a written file registered by the backend). */
export interface TaskArtifact {
  id: string
  path: string
  kind: string
  ts: number
}

/** Coarse task lifecycle derived from the turn's terminal `done` event. */
export type TaskStatus = 'idle' | 'running' | 'completed' | 'partial' | 'failed'

const STEP_STATUSES: readonly TaskStepStatus[] = ['pending', 'in_progress', 'done', 'failed']
const DELIVERABLE_KINDS: readonly DeliverableKind[] = [
  'Research',
  'HtmlPage',
  'Presentation',
  'Document',
  'Spreadsheet',
  'Other',
]

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' ? (value as Record<string, unknown>) : {}
}

/**
 * Whether the /task page follows runs from this session key.
 *
 * The store is one global view with no session selector, so it follows the web
 * console's own conversations. Broadcast frames from CLI / TUI / MCP /
 * subagent / side-chat sessions are dropped — otherwise a background agent or
 * a side chat could take the page over mid-run. Unknown key shapes stay
 * lenient (legacy keys predate the `agent:<id>:<surface>:<rest>` contract).
 */
export function isTaskRunSession(key: string): boolean {
  if (!key) return true
  if (key.includes(':webchat:side-')) return false
  const parts = key.split(':')
  if (parts[0] !== 'agent' || parts.length < 3) return true
  return parts[2] === 'webchat'
}

function asString(value: unknown): string {
  return typeof value === 'string' ? value : ''
}

function asNumber(value: unknown): number | undefined {
  if (typeof value === 'number' && Number.isFinite(value)) return value
  if (typeof value === 'string' && value.trim() !== '' && !Number.isNaN(Number(value))) {
    return Number(value)
  }
  return undefined
}

function asStringArray(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((x): x is string => typeof x === 'string') : []
}

function normalizeStatus(value: unknown, fallback: TaskStepStatus = 'pending'): TaskStepStatus {
  return typeof value === 'string' && (STEP_STATUSES as readonly string[]).includes(value)
    ? (value as TaskStepStatus)
    : fallback
}

function normalizeKind(value: unknown): DeliverableKind {
  return typeof value === 'string' && (DELIVERABLE_KINDS as readonly string[]).includes(value)
    ? (value as DeliverableKind)
    : 'Other'
}

/**
 * Normalize a wire subtask list (T03 `decision.subtasks`) into `TaskStep[]`.
 *
 * Handles both wire shapes the router can emit: the pipeline shape
 * (`{id, role, title, kind, target_path, depends_on}`) and the degraded
 * non-pipeline shape (`{role, title, depends_on}`), synthesizing a stable id
 * when the backend omits one so steps stay addressable by later events.
 */
export function normalizeTaskSteps(raw: unknown): TaskStep[] {
  if (!Array.isArray(raw)) return []
  return raw.map((entry, index) => {
    const s = asRecord(entry)
    const role = asString(s.role)
    const title = asString(s.title) || role
    const target = s.target_path ?? s.targetPath
    return {
      id: asString(s.id) || `d${index + 1}`,
      role,
      title,
      kind: normalizeKind(s.kind),
      targetPath: typeof target === 'string' && target ? target : null,
      dependsOn: asStringArray(s.depends_on ?? s.dependsOn),
      status: 'pending' as TaskStepStatus,
    }
  })
}

/** Best-effort mapping from the terminal `done.completeness` to the gate card. */
function gateFromCompleteness(
  reason: string,
  completeness: Record<string, unknown>,
): TaskGate {
  const allowed = completeness.allowed
  const blockedBy = asStringArray(completeness.blocked_by ?? completeness.blockedBy)
  const verdictRaw = asString(completeness.verdict).toLowerCase()
  const verdict: TaskGate['verdict'] =
    verdictRaw === 'pass' || allowed === true
      ? 'pass'
      : verdictRaw === 'fail' || reason === 'failed'
        ? 'fail'
        : 'partial'
  return {
    searchValid: !blockedBy.some((b) => /search|质量|低质/i.test(b)),
    artifactsComplete: allowed === true && blockedBy.length === 0,
    // The backend reports block *reason codes* (e.g. `MissingArtifact`), not
    // paths; surface them verbatim rather than inventing paths.
    missing: blockedBy,
    verdict,
  }
}

/** The pure task-workspace state + event handlers (no Pinia / RPC dependency). */
export function createTaskWorkspace() {
  const steps = ref<TaskStep[]>([])
  const artifacts = ref<TaskArtifact[]>([])
  const gate = ref<TaskGate | null>(null)
  const status = ref<TaskStatus>('idle')
  const strategy = ref<string | null>(null)
  const modeRoute = ref<string | null>(null)
  const pipeline = ref(false)
  const rationale = ref('')
  // Identity of the run on screen: which session/turn the state belongs to.
  const sessionKey = ref('')
  const taskId = ref('')

  function clearRun(): void {
    steps.value = []
    artifacts.value = []
    gate.value = null
    status.value = 'idle'
    strategy.value = null
    modeRoute.value = null
    pipeline.value = false
    rationale.value = ''
  }

  function reset(): void {
    clearRun()
    sessionKey.value = ''
    taskId.value = ''
  }

  /**
   * Claim a frame for the run this workspace displays.
   *
   * Events are broadcast to every client, so the frame identity
   * (`meta.session` / payload key + turn task id) decides attribution.
   * `opens` marks run-opening frames (`router_decision`, `step_start`): a frame
   * from another followed session or turn is a *newer* run and the page
   * switches to it. Closing frames and artifact pings never switch — a stray
   * `done` from another session must not end the run on screen, and a late
   * `step_finish` must not resurrect a run we already left.
   */
  function claimFrame(raw: unknown, meta: unknown, opens: boolean): boolean {
    const key = frameSessionKey(raw, meta)
    if (!isTaskRunSession(key)) return false
    const frameTask = payloadTaskId(asRecord(raw))
    const foreignSession = !!key && !!sessionKey.value && key !== sessionKey.value
    const foreignTask = !!frameTask && !!taskId.value && frameTask !== taskId.value
    if (foreignSession || foreignTask) {
      if (!opens) return false
      clearRun()
      taskId.value = frameTask
      if (key) sessionKey.value = key
      return true
    }
    if (key) sessionKey.value = key
    if (frameTask) taskId.value = frameTask
    return true
  }

  function stepIndexFor(id: string, path: string, title: string): number {
    if (id) {
      const byId = steps.value.findIndex((s) => s.id === id)
      if (byId >= 0) return byId
    }
    if (path) {
      const byPath = steps.value.findIndex(
        (s) => s.targetPath === path || s.targetPath === path.split(/[\\/]/).pop(),
      )
      if (byPath >= 0) return byPath
    }
    if (title) {
      const byTitle = steps.value.findIndex((s) => s.title === title)
      if (byTitle >= 0) return byTitle
    }
    return -1
  }

  function upsertArtifact(artifact: TaskArtifact): void {
    if (!artifact.path) return
    const byPath = artifacts.value.findIndex((a) => a.path === artifact.path)
    if (byPath >= 0) {
      artifacts.value[byPath] = { ...artifacts.value[byPath], ...artifact }
    } else {
      artifacts.value.push(artifact)
    }
  }

  function applyRouterDecision(raw: unknown, meta?: unknown): void {
    if (!claimFrame(raw, meta, true)) return
    const payload = asRecord(raw)
    const decision = asRecord(payload.decision)
    const subtasks = normalizeTaskSteps(decision.subtasks)
    // The Rust gateway publishes this receipt **after** the turn's step events,
    // so statuses / elapsed / artifacts already reported must survive the
    // restructure — replacing wholesale would paint a finished run as pending.
    const known = new Map(steps.value.map((s) => [s.id, s]))
    steps.value = subtasks.map((s) => {
      const prev = known.get(s.id)
      return prev
        ? {
            ...s,
            status: prev.status,
            elapsedMs: prev.elapsedMs,
            tools: prev.tools,
            artifactPath: prev.artifactPath,
          }
        : s
    })
    strategy.value = asString(payload.strategy) || null
    modeRoute.value = asString(payload.mode_route ?? payload.modeRoute) || null
    rationale.value = asString(decision.rationale)
    pipeline.value = decision.pipeline === true || subtasks.length > 1
    if (subtasks.length || strategy.value) status.value = 'running'
  }

  function applyStepStart(raw: unknown, meta?: unknown): void {
    if (!claimFrame(raw, meta, true)) return
    const p = asRecord(raw)
    const id = asString(p.task_step_id ?? p.taskStepId)
    const path = asString(p.target_path ?? p.targetPath)
    const title = asString(p.title)
    const index = stepIndexFor(id, path, title)
    if (index >= 0) {
      steps.value[index].status = normalizeStatus(p.status, 'in_progress')
    } else if (title || id) {
      steps.value.push({
        id: id || `s${steps.value.length + 1}`,
        role: asString(p.role),
        title: title || asString(p.role),
        kind: normalizeKind(p.kind),
        targetPath: path || null,
        dependsOn: [],
        status: normalizeStatus(p.status, 'in_progress'),
      })
    }
    if (status.value === 'idle') status.value = 'running'
  }

  function applyStepFinish(raw: unknown, meta?: unknown): void {
    if (!claimFrame(raw, meta, false)) return
    const p = asRecord(raw)
    const id = asString(p.task_step_id ?? p.taskStepId)
    const path = asString(p.target_path ?? p.targetPath)
    const title = asString(p.title)
    const index = stepIndexFor(id, path, title)
    if (index < 0) return
    const step = steps.value[index]
    step.status = normalizeStatus(p.status, 'done')
    const elapsed = asNumber(p.elapsed_ms ?? p.elapsedMs)
    if (elapsed !== undefined) step.elapsedMs = elapsed
    const tools = asStringArray(p.tools)
    if (tools.length) step.tools = tools
    const artifactPath = asString(p.artifact_path ?? p.artifactPath)
    if (artifactPath) {
      step.artifactPath = artifactPath
      upsertArtifact({
        id: id || artifactPath,
        path: artifactPath,
        kind: String(step.kind),
        ts: Date.now(),
      })
    }
  }

  function applyArtifactUpdated(raw: unknown, meta?: unknown): void {
    if (!claimFrame(raw, meta, false)) return
    const p = asRecord(raw)
    const path = asString(p.path)
    if (!path) return
    upsertArtifact({
      id: asString(p.id) || path,
      path,
      kind: asString(p.kind),
      ts: asNumber(p.ts) ?? Date.now(),
    })
    const leaf = path.split(/[\\/]/).pop() ?? path
    const index = steps.value.findIndex(
      (s) => s.targetPath && (s.targetPath === path || s.targetPath === leaf),
    )
    if (index >= 0) {
      steps.value[index].artifactPath = path
      if (steps.value[index].status !== 'failed') steps.value[index].status = 'done'
    }
  }

  function applyDone(raw: unknown, meta?: unknown): void {
    if (!claimFrame(raw, meta, false)) return
    const p = asRecord(raw)
    const reason = asString(p.reason)
    status.value =
      reason === 'completed'
        ? 'completed'
        : reason === 'partial'
          ? 'partial'
          : reason === 'failed'
            ? 'failed'
            : status.value === 'idle'
              ? 'completed'
              : status.value
    const completeness = asRecord(p.completeness)
    if (Object.keys(completeness).length > 0 || reason) {
      gate.value = gateFromCompleteness(reason, completeness)
    }
  }

  return {
    steps,
    artifacts,
    gate,
    status,
    strategy,
    modeRoute,
    pipeline,
    rationale,
    sessionKey,
    // A4 审查台账需要 run 身份（sessionKey + taskId 组成 runKey）；此前 taskId
    // 只是内部状态，未随 return 暴露。
    taskId,
    reset,
    applyRouterDecision,
    applyStepStart,
    applyStepFinish,
    applyArtifactUpdated,
    applyDone,
  }
}

export type TaskWorkspace = ReturnType<typeof createTaskWorkspace>

/** Minimal RPC surface the workspace needs (matches `RpcStore.on`). */
export interface TaskRpc {
  on: (event: string, handler: RpcEventHandler) => () => void
}

/**
 * Cross-chunk shared task workspace. `bindToRpc` is idempotent so the main view
 * and the workbench panel can both call it without double-subscribing.
 */
export const useTaskWorkspaceStore = defineStore('taskWorkspace', () => {
  const ws = createTaskWorkspace()
  let unsubs: Array<() => void> = []
  let bound = false
  let lastRpc: TaskRpc | null = null

  function subscribe(rpc: TaskRpc): Array<() => void> {
    const onRouter = (payload: unknown, meta?: unknown) => ws.applyRouterDecision(payload, meta)
    const onStepStart = (payload: unknown, meta?: unknown) => ws.applyStepStart(payload, meta)
    const onStepFinish = (payload: unknown, meta?: unknown) => ws.applyStepFinish(payload, meta)
    const onArtifact = (payload: unknown, meta?: unknown) => ws.applyArtifactUpdated(payload, meta)
    const onDone = (payload: unknown, meta?: unknown) => ws.applyDone(payload, meta)
    return [
      rpc.on('session.event.router_decision', onRouter),
      rpc.on('router_decision', onRouter),
      rpc.on('session.event.step_start', onStepStart),
      rpc.on('step_start', onStepStart),
      rpc.on('session.event.step_finish', onStepFinish),
      rpc.on('step_finish', onStepFinish),
      rpc.on('artifact.updated', onArtifact),
      rpc.on('session.event.artifact_updated', onArtifact),
      rpc.on('artifact_updated', onArtifact),
      rpc.on('session.event.done', onDone),
      rpc.on('chat.done', onDone),
    ]
  }

  function bindToRpc(rpc: TaskRpc): void {
    if (bound) return
    bound = true
    lastRpc = rpc
    unsubs = subscribe(rpc)
  }

  function unbindFromRpc(): void {
    for (const unsub of unsubs) unsub()
    unsubs = []
    bound = false
  }

  /**
   * Re-establish the live subscription without discarding current state. Honest
   * "refresh": after a dropped socket the step/artifact feed can go quiet; this
   * re-subscribes (the sole recovery action available — there is no
   * `task.get` RPC to re-hydrate from, so we do not pretend to re-fetch).
   */
  function refresh(): void {
    if (!lastRpc) return
    unbindFromRpc()
    bound = true
    unsubs = subscribe(lastRpc)
  }

  return { ...ws, bindToRpc, unbindFromRpc, refresh }
})
