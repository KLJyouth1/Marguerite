/**
 * 岛屿事件归约器（纯函数）—— RPC 事件流 → 运行时快照。
 *
 * - 纯函数：`applyIslandEvent(state, event, payload, { sessionKey, now })` 不读时钟、不订阅，
 *   因此载荷别名、去重、上限、会话过滤这些「最容易出错」的逻辑全部可单测。
 * - 事件载荷来自网关，字段存在多种历史别名（tool_id/toolId/tool_use_id…），
 *   这里集中做归一化，视图与渲染层只面对干净的快照。
 * - 与本会话无关的事件原样返回（引用不变 → Vue 层可以跳过无意义的重渲染）。
 */

import type { IslandRuntimeInput, IslandToolInput } from './islandWorld'
import { emptyRuntimeInput } from './islandWorld'

export interface IslandRuntimeState extends IslandRuntimeInput {
  /** 已归约的事件数（UI 用来说明数据新鲜度） */
  eventCount: number
  /** 最近一次事件名 */
  lastEventName: string
}

/** 岛屿订阅的事件名（与 useIslandRuntime 严格一致，改动必须同步） */
export const ISLAND_TRACKED_EVENTS = [
  'session.event.tool_use_start',
  'session.event.tool_result',
  'session.event.state_change',
  'session.event.subagent_completion',
  'session.event.artifact',
  'session.event.router_decision',
  'session.event.ensemble_progress',
  'session.event.plan_run',
  'session.event.plan_revision',
  'session.event.progress',
  'session.event.memory_recall',
  'task.queued',
  'task.running',
] as const

/** 快照各列表的内存上限（避免长时间运行无界增长） */
export const ISLAND_STATE_LIMITS = {
  tools: 24,
  subagents: 12,
  artifacts: 12,
  ensembleMembers: 12,
  planSteps: 24,
  memoryIds: 8,
} as const

export function createIslandRuntimeState(sessionKey = ''): IslandRuntimeState {
  return { ...emptyRuntimeInput(), sessionKey, eventCount: 0, lastEventName: '' }
}

function asRecord(payload: unknown): Record<string, unknown> {
  return payload && typeof payload === 'object' ? (payload as Record<string, unknown>) : {}
}

/** 取第一个非空字符串（载荷字段别名归一化） */
function pickString(source: Record<string, unknown>, keys: readonly string[]): string {
  for (const key of keys) {
    const value = source[key]
    if (typeof value === 'string' && value.trim()) return value
  }
  return ''
}

function pickNumber(source: Record<string, unknown>, keys: readonly string[]): number | undefined {
  for (const key of keys) {
    const value = source[key]
    if (typeof value === 'number' && Number.isFinite(value)) return value
  }
  return undefined
}

function pickBoolean(source: Record<string, unknown>, keys: readonly string[]): boolean {
  for (const key of keys) {
    if (source[key] === true) return true
  }
  return false
}

/** 会话归属：请求级事件带 session_key / sessionKey / key */
export function sessionKeyOf(payload: unknown): string {
  return pickString(asRecord(payload), ['session_key', 'sessionKey', 'key'])
}

function toolIdOf(payload: Record<string, unknown>): string {
  return pickString(payload, ['tool_id', 'toolId', 'tool_use_id', 'toolUseId', 'id'])
}

function upsertBy<T>(items: T[], match: (item: T) => boolean, next: T, limit: number): T[] {
  const index = items.findIndex(match)
  const merged = index >= 0
    ? [...items.slice(0, index), next, ...items.slice(index + 1)]
    : [...items, next]
  return merged.length > limit ? merged.slice(-limit) : merged
}

/** 从 plan_run / plan_revision / progress 载荷里找出步骤数组（多个历史位置） */
function stepsOf(payload: Record<string, unknown>): { id: string; title: string; status: string }[] | null {
  const candidates = [payload.steps, asRecord(payload.snapshot).steps, asRecord(payload.run).steps]
  for (const candidate of candidates) {
    if (!Array.isArray(candidate)) continue
    return candidate.map((raw, index) => {
      const step = asRecord(raw)
      const id = pickString(step, ['stepId', 'step_id', 'id']) || `step-${index}`
      const title = pickString(step, ['title', 'stepTitle', 'step_title']) || id
      const status = pickString(step, ['status', 'state']) || 'pending'
      return { id, title, status }
    })
  }
  return null
}

/** 单步进度（session.event.progress 的 step_progress 主题） */
function stepProgressOf(payload: Record<string, unknown>): { id: string; title: string; status: string } | null {
  if (pickString(payload, ['kind']) !== 'step_progress') return null
  const id = pickString(payload, ['stepId', 'step_id'])
  if (!id) return null
  const phase = pickString(payload, ['phase'])
  return {
    id,
    title: pickString(payload, ['stepTitle', 'step_title']) || id,
    status: phase === 'finished' ? 'completed' : 'in_progress',
  }
}

interface ApplyOptions {
  /** 只归约该会话的事件（空字符串 = 不过滤） */
  sessionKey: string
  /** 注入时间戳（保持纯函数可测） */
  now: number
}

/** 归约一条事件；与目标会话无关或无法识别时原样返回（引用不变） */
export function applyIslandEvent(
  state: IslandRuntimeState,
  event: string,
  payload: unknown,
  options: ApplyOptions,
): IslandRuntimeState {
  if (!(ISLAND_TRACKED_EVENTS as readonly string[]).includes(event)) return state

  const record = asRecord(payload)
  const eventKey = sessionKeyOf(payload)
  const expected = options.sessionKey || state.sessionKey
  if (expected && eventKey && eventKey !== expected) return state

  const base: IslandRuntimeState = {
    ...state,
    eventCount: state.eventCount + 1,
    lastEventName: event,
    lastEventAt: options.now,
  }

  switch (event) {
    case 'session.event.tool_use_start':
    case 'session.event.tool_result': {
      const id = toolIdOf(record)
      if (!id) return state
      const isResult = event === 'session.event.tool_result'
      const existing = state.tools.find((tool) => tool.id === id)
      const tool: IslandToolInput = {
        id,
        name: pickString(record, ['tool_name', 'name']) || existing?.name || 'tool',
        isRunning: !isResult,
        isError: isResult
          ? pickBoolean(record, ['is_error', 'isError']) || pickString(record, ['execution_status']) === 'error'
          : false,
        startedAt: pickNumber(record, ['started_at']) ?? existing?.startedAt,
        endedAt: isResult ? options.now : undefined,
      }
      return {
        ...base,
        tools: upsertBy(state.tools, (item) => item.id === id, tool, ISLAND_STATE_LIMITS.tools),
      }
    }

    case 'session.event.state_change': {
      const runStatus = pickString(record, ['to_state', 'toState', 'run_status', 'runStatus', 'status'])
      return runStatus ? { ...base, runStatus } : base
    }

    case 'task.queued':
      return { ...base, runStatus: 'queued' }
    case 'task.running':
      return { ...base, runStatus: 'running' }

    case 'session.event.subagent_completion': {
      const key = pickString(record, ['child_session_key', 'childSessionKey'])
      if (!key) return state
      return {
        ...base,
        subagents: upsertBy(
          state.subagents,
          (item) => item.key === key,
          { key, status: pickString(record, ['status']) || 'completed' },
          ISLAND_STATE_LIMITS.subagents,
        ),
      }
    }

    case 'session.event.artifact': {
      const id = pickString(record, ['id', 'key'])
      if (!id) return state
      return {
        ...base,
        artifacts: upsertBy(
          state.artifacts,
          (item) => item.id === id,
          { id, name: pickString(record, ['name', 'filename']) || id, kind: pickString(record, ['kind']) || undefined },
          ISLAND_STATE_LIMITS.artifacts,
        ),
      }
    }

    case 'session.event.router_decision': {
      const strategy = pickString(record, ['strategy'])
      return strategy ? { ...base, strategy } : base
    }

    case 'session.event.ensemble_progress': {
      const eventType = pickString(record, ['event_type'])
      const index = pickNumber(record, ['proposer_index'])
      const label = pickString(record, ['proposer_label']) || (index !== undefined ? `proposer-${index}` : '')
      if (!label) return state
      return {
        ...base,
        ensembleMembers: upsertBy(
          state.ensembleMembers,
          (item) => item.label === label,
          { label, done: eventType.endsWith('finish') },
          ISLAND_STATE_LIMITS.ensembleMembers,
        ),
      }
    }

    case 'session.event.plan_run':
    case 'session.event.plan_revision': {
      const steps = stepsOf(record)
      if (!steps) return state
      return { ...base, planSteps: steps.slice(0, ISLAND_STATE_LIMITS.planSteps) }
    }

    case 'session.event.progress': {
      const step = stepProgressOf(record)
      if (!step) return state
      return {
        ...base,
        planSteps: upsertBy(
          state.planSteps,
          (item) => item.id === step.id,
          step,
          ISLAND_STATE_LIMITS.planSteps,
        ),
      }
    }

    case 'session.event.memory_recall': {
      // 诚实可视化：hits=0 时后端不发事件；若仍收到 0 命中，视为「无召回」，不生成记忆塔。
      const hits = pickNumber(record, ['hits']) ?? 0
      if (hits <= 0) return state
      const rawIds = Array.isArray(record.ids) ? record.ids : []
      const ids = rawIds
        .filter((id): id is string => typeof id === 'string' && id.trim() !== '')
        .slice(0, ISLAND_STATE_LIMITS.memoryIds)
      return {
        ...base,
        recall: {
          query: pickString(record, ['query']) || '',
          hits,
          topScore: pickNumber(record, ['top_score', 'topScore']) ?? 0,
          ids,
          reinforced: pickNumber(record, ['reinforced']) ?? 0,
          at: pickNumber(record, ['ts']) ?? options.now,
        },
      }
    }

    default:
      return base
  }
}