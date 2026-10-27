/**
 * 任务复盘（`sessions.insights`）的纯展示派生。
 *
 * 数据来自 Rust 侧只读聚合：任务类型、工具调度、思考正文、会话粒度表现。
 * 这里只做「取前 N / 排序 / 占比」这类无副作用的变换，便于单测锁定口径；
 * 任何字段缺失都降级为 0 或空数组，不伪造数据。
 */

export interface InsightsTotals {
  sessions?: number
  messages?: number
  userTurns?: number
  assistantTurns?: number
  toolCalls?: number
  toolFailures?: number
  toolUnknown?: number
  toolMs?: number
  thinkingMessages?: number
  thinkingChars?: number
}

export interface InsightsTaskTypes {
  directAnswer?: number
  reasoning?: number
  toolAssisted?: number
  toolAndReasoning?: number
}

export interface InsightsToolRow {
  name?: string
  calls?: number
  failures?: number
  unknown?: number
  avgMs?: number | null
  p95Ms?: number | null
}

export interface InsightsSessionRow {
  key?: string
  title?: string
  messages?: number
  userTurns?: number
  assistantTurns?: number
  toolCalls?: number
  toolFailures?: number
  thinkingMessages?: number
  thinkingChars?: number
  toolMs?: number
  lastTool?: string | null
  updatedAt?: string
}

export interface InsightsCoverage {
  sessionsWithToolRows?: number
  sessionsWithThinking?: number
  sessionsWithoutAssistant?: number
  sessionsWithoutMessages?: number
}

export interface InsightsPayload {
  ok?: boolean
  error?: string
  code?: string
  generatedAt?: string
  totals?: InsightsTotals
  taskTypes?: InsightsTaskTypes
  tools?: InsightsToolRow[]
  sessions?: InsightsSessionRow[]
  coverage?: InsightsCoverage
  gaps?: string[]
}

export const TASK_TYPE_KEYS = [
  'directAnswer',
  'reasoning',
  'toolAssisted',
  'toolAndReasoning',
] as const

export type TaskTypeKey = (typeof TASK_TYPE_KEYS)[number]

export interface TaskTypeRow {
  key: TaskTypeKey
  count: number
  /** 占全部 assistant 轮次的比例（0–1）；总轮次为 0 时为 0。 */
  share: number
}

function num(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

/** assistant 轮次总数（四类之和；缺失字段按 0 计）。 */
export function assistantTurnTotal(payload: InsightsPayload | null): number {
  const types = payload?.taskTypes
  if (!types) return 0
  return TASK_TYPE_KEYS.reduce((sum, key) => sum + num(types[key]), 0)
}

/** 任务类型分布：固定四行（即使为 0 也展示，避免「看不到就是不知道」）。 */
export function taskTypeRows(payload: InsightsPayload | null): TaskTypeRow[] {
  const total = assistantTurnTotal(payload)
  return TASK_TYPE_KEYS.map((key) => {
    const count = num(payload?.taskTypes?.[key])
    return { key, count, share: total > 0 ? count / total : 0 }
  })
}

/** 工具调用排行：按次数降序、同次数按名称升序（稳定且可预期）。 */
export function topTools(payload: InsightsPayload | null, limit = 8): InsightsToolRow[] {
  const rows = Array.isArray(payload?.tools) ? [...payload.tools] : []
  rows.sort((a, b) => {
    const byCalls = num(b.calls) - num(a.calls)
    if (byCalls !== 0) return byCalls
    return String(a.name ?? '').localeCompare(String(b.name ?? ''))
  })
  return rows.slice(0, Math.max(1, limit))
}

/** 会话排行：按工具调用数降序，同数按消息数降序（复盘优先看「动过手」的会话）。 */
export function topSessions(payload: InsightsPayload | null, limit = 10): InsightsSessionRow[] {
  const rows = Array.isArray(payload?.sessions) ? [...payload.sessions] : []
  rows.sort((a, b) => {
    const byTools = num(b.toolCalls) - num(a.toolCalls)
    if (byTools !== 0) return byTools
    return num(b.messages) - num(a.messages)
  })
  return rows.slice(0, Math.max(1, limit))
}

/** 工具失败数（只认明确失败；unknown 不计入）。 */
export function toolFailureTotal(payload: InsightsPayload | null): number {
  return num(payload?.totals?.toolFailures)
}

/** 有思考正文的 assistant 轮次占比；总轮次为 0 时为 0。 */
export function thinkingShare(payload: InsightsPayload | null): number {
  const total = assistantTurnTotal(payload)
  if (total <= 0) return 0
  return num(payload?.totals?.thinkingMessages) / total
}

/** 复盘缺口提示（后端给的诚实边界），去空去重。 */
export function insightGaps(payload: InsightsPayload | null): string[] {
  if (!Array.isArray(payload?.gaps)) return []
  const seen = new Set<string>()
  const out: string[] = []
  for (const gap of payload.gaps) {
    if (typeof gap !== 'string') continue
    const text = gap.trim()
    if (!text || seen.has(text)) continue
    seen.add(text)
    out.push(text)
  }
  return out
}
