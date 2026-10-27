/**
 * Harness 运行台账 —— 纯视图模型。
 *
 * 所有格式化 / 排序 / 过滤 / KPI 派生 / 状态语义映射都集中在这里，视图只做编排与渲染。
 * 数据契约（冻结，不得改名）：
 *   · harness.ledger.status / harness.ledger.query / harness.ledger.replay
 *   · skills.coverage
 *
 * 诚实原则：字段缺失、输入非法、台账不可用一律降级成「—」或空列表，不编造任何示例值，
 * 也不抛异常——上游可能给出 undefined / null / NaN。
 */

/** status 的语义角色：视图据此选择 i18n 文案与语义色 token，不直接信任后端字符串。 */
export type StatusRole = 'ok' | 'failed' | 'running' | 'cancelled' | 'other'

/** 无法展示时的统一占位符（不是 0，也不是示例数据）。 */
export const EMPTY_VALUE = '—'

export interface LedgerItem {
  seq: number
  type: string
  name: string
  status: string
  elapsedMs: number
  tokensIn: number
  tokensOut: number
  ok: boolean
  errorCode: string | null
}

export interface LedgerRun {
  runId: string
  sessionKey: string
  traceId: string
  target: string
  status: string
  /** 验收结论：passed | failed | unverified | unknown（缺省按 unverified 展示） */
  verdict?: string
  startedAtMs: number
  endedAtMs: number
  elapsedMs: number
  promptTokens: number
  outputTokens: number
  model: string
  policy: string
  itemCount: number
  items: LedgerItem[]
}

export interface LedgerStatusPayload {
  ok: boolean
  enabled: boolean
  path: string
  schemaVersion: number
  runs: number
  items: number
  bytes: number
  oldestTsMs: number | null
  newestTsMs: number | null
  successRate: number
  avgElapsedMs: number
  p95ElapsedMs: number
  tokensIn: number
  tokensOut: number
  lastError: string | null
  /** 验收结论分布（四个取值都在，后端已补 0） */
  verdicts?: Record<string, number>
  error?: string
  code?: string
}

export interface LedgerQueryPayload {
  ok: boolean
  runs: LedgerRun[]
  truncated: boolean
  error?: string
  code?: string
}

export interface LedgerReplayPayload {
  ok: boolean
  runId: string
  digest: string
  itemCount: number
  deterministic: boolean
  note: string
  error?: string
  code?: string
}

export interface SkillsCoveragePayload {
  ok: boolean
  root: string
  directories: number
  filesScanned: number
  loaded: number
  yaml: number
  skillMd: number
  skipped: Array<{ path: string; reason: string }>
  skippedTotal: number
  error?: string
  code?: string
}

export interface HarnessKpis {
  runs: string
  items: string
  successRate: string
  p95: string
  tokens: string
  dbSize: string
}

export type KpiKey = keyof HarnessKpis

export const KPI_KEYS: KpiKey[] = ['runs', 'items', 'successRate', 'p95', 'tokens', 'dbSize']

export const LIMIT_OPTIONS = [10, 25, 50, 100] as const

/** 只接受有限数值；其余（null / undefined / NaN / Infinity / 字符串）一律视作缺失。 */
function finite(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

/** 字节格式化：1024 进制，B 取整，KB 起保留一位小数（≥100 时取整）。 */
export function formatBytes(value: unknown): string {
  const n = finite(value)
  if (n === null || n < 0) return EMPTY_VALUE
  if (n < 1024) return `${Math.round(n)} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let size = n / 1024
  let unit = 0
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024
    unit += 1
  }
  return `${size >= 100 ? size.toFixed(0) : size.toFixed(1)} ${units[unit]}`
}

/** 耗时格式化：毫秒 → ms / s / m+s，负数与非数值视作缺失。 */
export function formatDuration(value: unknown): string {
  const n = finite(value)
  if (n === null || n < 0) return EMPTY_VALUE
  if (n < 1000) return `${Math.round(n)} ms`
  if (n < 60_000) return `${(n / 1000).toFixed(1)} s`
  const minutes = Math.floor(n / 60_000)
  const restMs = n - minutes * 60_000
  const seconds = Math.round(restMs / 1000)
  if (seconds === 60) return `${minutes + 1}m 0s`
  return `${minutes}m ${seconds}s`
}

/** 百分比格式化：契约里 successRate ∈ [0,1]；越界按边界截断，缺失显示「—」。 */
export function formatPercent(rate: unknown): string {
  const n = finite(rate)
  if (n === null) return EMPTY_VALUE
  const pct = Math.min(1, Math.max(0, n)) * 100
  return `${Number.isInteger(pct) ? pct.toFixed(0) : pct.toFixed(1)}%`
}

/** 计数格式化：token 量级用 k / M 缩写，缺失与负数显示「—」。 */
export function formatTokens(value: unknown): string {
  const n = finite(value)
  if (n === null || n < 0) return EMPTY_VALUE
  if (n < 1000) return String(Math.round(n))
  if (n < 1_000_000) {
    const k = n / 1000
    return `${k >= 100 ? k.toFixed(0) : k.toFixed(1)}k`
  }
  const m = n / 1_000_000
  return `${m >= 100 ? m.toFixed(0) : m.toFixed(1)}M`
}

/** 时间戳格式化（本地时区，秒级）；0 / 负值 / 非数值视作缺失。 */
export function formatTimestamp(value: unknown): string {
  const n = finite(value)
  if (n === null || n <= 0) return EMPTY_VALUE
  const date = new Date(n)
  if (!Number.isFinite(date.getTime())) return EMPTY_VALUE
  const pad = (part: number): string => String(part).padStart(2, '0')
  return (
    `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ` +
    `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`
  )
}

/** 状态字符串 → 语义角色。未知值保持 other，由视图原样透出，不猜测翻译。 */
export function statusRole(status: unknown): StatusRole {
  if (typeof status !== 'string') return 'other'
  const value = status.trim().toLowerCase()
  if (['ok', 'success', 'succeeded', 'completed', 'complete', 'done', 'passed'].includes(value)) return 'ok'
  if (['failed', 'failure', 'error', 'errored'].includes(value)) return 'failed'
  if (['running', 'pending', 'started', 'in_progress', 'inprogress'].includes(value)) return 'running'
  if (['cancelled', 'canceled', 'aborted', 'interrupted', 'stopped'].includes(value)) return 'cancelled'
  return 'other'
}

/** 按开始时间排序（默认最新在前）；无效时间戳沉底，且不修改入参。 */
export function sortRuns(
  runs: LedgerRun[] | null | undefined,
  direction: 'asc' | 'desc' = 'desc',
): LedgerRun[] {
  if (!Array.isArray(runs)) return []
  const factor = direction === 'asc' ? 1 : -1
  return [...runs].sort((a, b) => {
    const left = finite(a?.startedAtMs)
    const right = finite(b?.startedAtMs)
    if (left === null && right === null) return 0
    if (left === null) return 1
    if (right === null) return -1
    return (left - right) * factor
  })
}

/** 按 sessionKey 精确过滤；空过滤值返回全部，永远返回新数组。 */
export function filterRunsBySession(
  runs: LedgerRun[] | null | undefined,
  sessionKey: string | null | undefined,
): LedgerRun[] {
  if (!Array.isArray(runs)) return []
  const key = typeof sessionKey === 'string' ? sessionKey.trim() : ''
  if (!key) return [...runs]
  return runs.filter((run) => run?.sessionKey === key)
}

/** 会话下拉选项：去重后按字典序稳定排序（不使用本地化排序，避免顺序漂移）。 */
export function sessionOptions(runs: LedgerRun[] | null | undefined): string[] {
  if (!Array.isArray(runs)) return []
  const keys = new Set<string>()
  for (const run of runs) {
    const key = typeof run?.sessionKey === 'string' ? run.sessionKey : ''
    if (key) keys.add(key)
  }
  return [...keys].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0))
}

/** 过滤 + 排序的组合入口：视图一次调用即可拿到要渲染的行。 */
export function visibleRuns(
  runs: LedgerRun[] | null | undefined,
  sessionKey: string | null | undefined,
  direction: 'asc' | 'desc' = 'desc',
): LedgerRun[] {
  return sortRuns(filterRunsBySession(runs, sessionKey), direction)
}

/**
 * 恢复选中项：优先匹配 runId；已不存在时回退到列表首条；列表为空返回 null。
 * 这样刷新 / 换过滤条件后详情面板不会停留在已消失的 run 上。
 */
export function selectRun(
  runs: LedgerRun[] | null | undefined,
  runId: string | null | undefined,
): LedgerRun | null {
  const list = Array.isArray(runs) ? runs : []
  if (list.length === 0) return null
  if (runId) {
    const matched = list.find((run) => run?.runId === runId)
    if (matched) return matched
  }
  return list[0]
}

/** 条目数：优先契约里的 itemCount，缺失时回退到 items.length。 */
export function resolveItemCount(run: LedgerRun | null | undefined): number {
  const declared = finite(run?.itemCount)
  if (declared !== null) return Math.max(0, Math.round(declared))
  return Array.isArray(run?.items) ? run.items.length : 0
}

/** 时间线条目按 seq 升序；无效 seq 沉底，不修改入参。 */
export function sortItems(items: LedgerItem[] | null | undefined): LedgerItem[] {
  if (!Array.isArray(items)) return []
  return [...items].sort((a, b) => {
    const left = finite(a?.seq)
    const right = finite(b?.seq)
    if (left === null && right === null) return 0
    if (left === null) return 1
    if (right === null) return -1
    return left - right
  })
}

/** KPI 派生：台账不可用 / ok=false 时所有卡片显示「—」，不伪造 0 或示例值。 */
export function deriveKpis(status: LedgerStatusPayload | null | undefined): HarnessKpis {
  const unavailable: HarnessKpis = {
    runs: EMPTY_VALUE,
    items: EMPTY_VALUE,
    successRate: EMPTY_VALUE,
    p95: EMPTY_VALUE,
    tokens: EMPTY_VALUE,
    dbSize: EMPTY_VALUE,
  }
  if (!status || status.ok !== true) return unavailable
  return {
    runs: formatTokens(status.runs),
    items: formatTokens(status.items),
    successRate: formatPercent(status.successRate),
    p95: formatDuration(status.p95ElapsedMs),
    tokens: `${formatTokens(status.tokensIn)} / ${formatTokens(status.tokensOut)}`,
    dbSize: formatBytes(status.bytes),
  }
}

/** 从响应体里取可读原因；缺失时返回空串，由视图补本地化兜底文案。 */
export function payloadReason(payload: { error?: string } | null | undefined): string {
  return typeof payload?.error === 'string' ? payload.error.trim() : ''
}

/** replay 摘要文本：仅当 replay 成功时返回非空 digest。 */
export function replayDigest(replay: LedgerReplayPayload | null | undefined): string {
  if (!replay || replay.ok !== true) return ''
  return typeof replay.digest === 'string' ? replay.digest.trim() : ''
}

/** 单 run 的 token 展示（入 / 出）。 */
export function runTokens(run: LedgerRun | null | undefined): string {
  return `${formatTokens(run?.promptTokens)} / ${formatTokens(run?.outputTokens)}`
}

/** 技能覆盖的跳过清单：只保留结构完整的条目，缺失时返回空列表。 */
export function coverageSkipped(
  coverage: SkillsCoveragePayload | null | undefined,
): Array<{ path: string; reason: string }> {
  if (!coverage || !Array.isArray(coverage.skipped)) return []
  return coverage.skipped.filter(
    (entry): entry is { path: string; reason: string } =>
      typeof entry?.path === 'string' && typeof entry?.reason === 'string',
  )
}

/** 跳过总数：优先 skippedTotal，缺失时回退到清单长度。 */
export function coverageSkippedTotal(coverage: SkillsCoveragePayload | null | undefined): number {
  const declared = finite(coverage?.skippedTotal)
  if (declared !== null) return Math.max(0, Math.round(declared))
  return coverageSkipped(coverage).length
}

/** 限制值归一：只接受 LIMIT_OPTIONS 内的值，其余回落到 25。 */
export function normalizeLimit(value: unknown): number {
  const n = finite(value)
  if (n === null) return 25
  const rounded = Math.round(n)
  return (LIMIT_OPTIONS as readonly number[]).includes(rounded) ? rounded : 25
}
