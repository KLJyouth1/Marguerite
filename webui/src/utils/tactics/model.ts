/**
 * 战术推演（`tactics.*`）的前端契约与纯函数。
 *
 * 为什么单独一层：
 * - 视图只做渲染，参数整形 / 校验 / 格式化全部在这里，可单测；
 * - 宿主契约用 camelCase（Rust 侧 `#[serde(rename_all = "camelCase")]`），
 *   这里给出可被 TS 检查的投影，字段缺失一律可选，不假设宿主一定给全。
 *
 * 诚实约束：数字输入留空 = **不覆盖后端先验**（后端会如实标 `n=0`），
 * 而不是在本地补一个 0 —— 补 0 会把「不知道」变成「零」。
 */

export const TACTICS_CATALOG_METHOD = 'tactics.catalog'
export const TACTICS_SIMULATE_METHOD = 'tactics.simulate'
export const TACTICS_REPLAY_METHOD = 'tactics.replay'

/** 推演试跑次数的边界（与 `core/src/tactics/model.rs` 的常量一致）。 */
export const MIN_TRIALS = 1
export const MAX_TRIALS = 20_000
export const DEFAULT_TRIALS = 2_000

export type TopologyKey = 'solo' | 'star' | 'panel' | 'swarm' | 'relay'
export const TOPOLOGY_KEYS: readonly TopologyKey[] = ['solo', 'star', 'panel', 'swarm', 'relay']

export type PriorSource = 'measured' | 'declared' | 'unknown'

export interface PriorView {
  value?: number
  n?: number
  source?: PriorSource
}

export interface ModelView {
  failureRate?: PriorView
  throughputKPerMin?: PriorView
  pricePerMTokUsd?: PriorView
  selfDetect?: PriorView
  retryBackoff?: PriorView
}

export interface GateView {
  detect?: number
  falsePositive?: number
  tokenOverhead?: number
  latencyFactor?: number
}

export interface TacticCatalogEntry {
  id: string
  topology: string
  width?: number
  retries?: number
  gate?: GateView | null
  bet?: string
}

export interface CatalogPayload {
  ok?: boolean
  count?: number
  tactics?: TacticCatalogEntry[]
  topologies?: string[]
  defaults?: {
    trials?: number
    maxTrials?: number
    maxFailProbability?: number
    model?: ModelView
  }
}

export interface TacticResultView {
  tactic: string
  topology?: string
  width?: number
  retries?: number
  gated?: boolean
  successRate?: number
  p50Ms?: number
  p90Ms?: number
  p50Tokens?: number
  p90Tokens?: number
  p90CostUsd?: number
  meanRetries?: number
  coordinationShare?: number
  causes?: Record<string, number>
  failureNodes?: Array<[string, number]>
  criticalPath?: string[]
  bottleneck?: string
}

export interface KillCheckView {
  id?: string
  unit?: string
  threshold?: number
  actual?: number
  violated?: boolean
}

export interface TacticVerdictView {
  tactic?: string
  viable?: boolean
  checks?: KillCheckView[]
}

export interface WhyNotView {
  tactic?: string
  reason?: string
  detail?: string
}

export interface RationaleView {
  recommended?: string | null
  noViableTactic?: boolean
  reason?: string
  verdicts?: TacticVerdictView[]
  whyNot?: WhyNotView[]
}

export interface PremortemItemView {
  id?: string
  frequency?: number
  severity?: string
  evidence?: string
  mitigation?: string
}

export interface UnknownView {
  field?: string
  reason?: string
}

export interface PlanView {
  ok?: boolean
  results?: TacticResultView[]
  rationale?: RationaleView
  premortem?: PremortemItemView[]
  unknowns?: UnknownView[]
  totalWorkK?: number
  trials?: number
  seed?: number
  error?: string
  code?: string
}

export interface MeasuredItemView {
  seq?: number
  name?: string
  elapsedMs?: number
  tokens?: number
  ok?: boolean
  errorCode?: string | null
}

export interface ReplayDeltaView {
  tactic?: string
  successRate?: number
  p50Ms?: number
  p90Ms?: number
  p90Tokens?: number
  timeDeltaPct?: number
  tokenDeltaPct?: number
  successDelta?: number
}

export interface WhyLinkView {
  question?: string
  answer?: string
  kind?: string
  evidence?: string | null
}

export interface ReplayGapView {
  field?: string
  reason?: string
}

export interface ReplayReportView {
  ok?: boolean
  observed?: {
    runId?: string
    target?: string
    status?: string
    elapsedMs?: number
    tokens?: number
    ok?: boolean
    items?: MeasuredItemView[]
  }
  scenarioSource?: string
  insufficientEvidence?: boolean
  results?: TacticResultView[]
  rationale?: RationaleView
  deltas?: ReplayDeltaView[]
  whyChain?: WhyLinkView[]
  gaps?: ReplayGapView[]
  error?: string
  code?: string
}

// ── 表单 ────────────────────────────────────────────────────────────────────

export interface TaskDraft {
  id: string
  /** 留空 = 后端默认工作量 */
  workK: string
  /** 逗号/空格分隔的依赖 id */
  dependsOn: string
}

export type PriorField =
  | 'failureRate'
  | 'throughputKPerMin'
  | 'pricePerMTokUsd'
  | 'selfDetect'
  | 'retryBackoff'

export const PRIOR_FIELDS: readonly PriorField[] = [
  'failureRate',
  'throughputKPerMin',
  'pricePerMTokUsd',
  'selfDetect',
  'retryBackoff',
]

export interface ScenarioForm {
  tasks: TaskDraft[]
  budgetTokens: string
  deadlineMs: string
  trials: string
  maxFailProbability: string
  seed: string
  priors: Record<PriorField, string>
}

export function defaultScenarioForm(): ScenarioForm {
  return {
    tasks: [
      { id: 'plan', workK: '20', dependsOn: '' },
      { id: 'impl-a', workK: '40', dependsOn: 'plan' },
      { id: 'impl-b', workK: '40', dependsOn: 'plan' },
      { id: 'verify', workK: '30', dependsOn: 'impl-a, impl-b' },
    ],
    budgetTokens: '',
    deadlineMs: '',
    trials: String(DEFAULT_TRIALS),
    maxFailProbability: '',
    seed: '42',
    priors: {
      failureRate: '',
      throughputKPerMin: '',
      pricePerMTokUsd: '',
      selfDetect: '',
      retryBackoff: '',
    },
  }
}

export type ScenarioIssueCode =
  | 'empty'
  | 'missing-id'
  | 'duplicate-id'
  | 'unknown-dependency'
  | 'bad-number'

export interface ScenarioIssue {
  code: ScenarioIssueCode
  /** 仅 `bad-number` 使用：出问题的字段名（原样展示，便于定位） */
  field?: string
}

/** `a, b  a` → ['a','b']：去空、去重、保持首次出现顺序。 */
export function parseDeps(raw: string): string[] {
  const seen = new Set<string>()
  const out: string[] = []
  for (const piece of raw.split(/[\s,]+/)) {
    const id = piece.trim()
    if (!id || seen.has(id)) continue
    seen.add(id)
    out.push(id)
  }
  return out
}

function numOrNull(raw: string): number | null {
  const text = raw.trim()
  if (!text) return null
  const n = Number(text)
  return Number.isFinite(n) ? n : null
}

/** 返回每一处问题而不是第一个：一次改完，不用反复试运行。 */
export function validateScenarioForm(form: ScenarioForm): ScenarioIssue[] {
  const issues: ScenarioIssue[] = []
  const ids: string[] = []
  const seen = new Set<string>()

  for (const task of form.tasks) {
    const id = task.id.trim()
    if (!id) {
      issues.push({ code: 'missing-id' })
      continue
    }
    if (seen.has(id)) {
      issues.push({ code: 'duplicate-id' })
      continue
    }
    seen.add(id)
    ids.push(id)
  }
  if (ids.length === 0) issues.push({ code: 'empty' })

  for (const task of form.tasks) {
    const id = task.id.trim()
    if (!id) continue
    for (const dep of parseDeps(task.dependsOn)) {
      if (!seen.has(dep)) issues.push({ code: 'unknown-dependency' })
    }
    const work = numOrNull(task.workK)
    if (task.workK.trim() && (work === null || work <= 0)) {
      issues.push({ code: 'bad-number', field: `${id}.workK` })
    }
  }

  const numberRules: Array<[string, string, (n: number) => boolean]> = [
    ['budgetTokens', form.budgetTokens, (n) => n > 0],
    ['deadlineMs', form.deadlineMs, (n) => n > 0],
    ['trials', form.trials, (n) => n >= MIN_TRIALS && n <= MAX_TRIALS],
    ['maxFailProbability', form.maxFailProbability, (n) => n >= 0 && n <= 1],
    ['seed', form.seed, (n) => n >= 0 && Number.isInteger(n)],
    ['priors.failureRate', form.priors.failureRate, (n) => n >= 0 && n <= 1],
    ['priors.throughputKPerMin', form.priors.throughputKPerMin, (n) => n > 0],
    ['priors.pricePerMTokUsd', form.priors.pricePerMTokUsd, (n) => n >= 0],
    ['priors.selfDetect', form.priors.selfDetect, (n) => n >= 0 && n <= 1],
    ['priors.retryBackoff', form.priors.retryBackoff, (n) => n >= 1],
  ]
  for (const [field, raw, ok] of numberRules) {
    if (!raw.trim()) continue
    const n = numOrNull(raw)
    if (n === null || !ok(n)) issues.push({ code: 'bad-number', field })
  }

  return issues
}

/** 表单 → `tactics.simulate` 参数。留空字段一律不发，交给后端声明先验。 */
export function buildSimulateParams(form: ScenarioForm): Record<string, unknown> {
  const tasks = form.tasks
    .filter((task) => task.id.trim() !== '')
    .map((task) => {
      const out: Record<string, unknown> = { id: task.id.trim() }
      const work = numOrNull(task.workK)
      if (work !== null && work > 0) out.workK = work
      const deps = parseDeps(task.dependsOn)
      if (deps.length) out.dependsOn = deps
      return out
    })

  const params: Record<string, unknown> = { tasks }

  const budget = numOrNull(form.budgetTokens)
  if (budget !== null && budget > 0) params.budgetTokens = budget
  const deadline = numOrNull(form.deadlineMs)
  if (deadline !== null && deadline > 0) params.deadlineMs = deadline

  const trials = numOrNull(form.trials)
  params.trials =
    trials === null
      ? DEFAULT_TRIALS
      : Math.min(MAX_TRIALS, Math.max(MIN_TRIALS, Math.round(trials)))

  const maxFail = numOrNull(form.maxFailProbability)
  if (maxFail !== null && maxFail >= 0 && maxFail <= 1) params.maxFailProbability = maxFail

  const seed = numOrNull(form.seed)
  if (seed !== null && seed >= 0 && Number.isInteger(seed)) params.seed = seed

  const priors: Record<string, number> = {}
  const rate = numOrNull(form.priors.failureRate)
  if (rate !== null && rate >= 0 && rate <= 1) priors.failureRate = rate
  const throughput = numOrNull(form.priors.throughputKPerMin)
  if (throughput !== null && throughput > 0) priors.throughputKPerMin = throughput
  const price = numOrNull(form.priors.pricePerMTokUsd)
  if (price !== null && price >= 0) priors.pricePerMTokUsd = price
  const detect = numOrNull(form.priors.selfDetect)
  if (detect !== null && detect >= 0 && detect <= 1) priors.selfDetect = detect
  const backoff = numOrNull(form.priors.retryBackoff)
  if (backoff !== null && backoff >= 1) priors.retryBackoff = backoff
  if (Object.keys(priors).length) params.priors = priors

  return params
}

// ── 展示 ────────────────────────────────────────────────────────────────────

const WHY_KINDS = ['evidence', 'inference', 'gap'] as const

export function whyKindKey(kind: string | undefined): string | null {
  if (!kind) return null
  return (WHY_KINDS as readonly string[]).includes(kind) ? `tactics.replay.kind.${kind}` : null
}

export function topologyKey(topology: string | undefined): string | null {
  if (!topology) return null
  return (TOPOLOGY_KEYS as readonly string[]).includes(topology)
    ? `tactics.topology.${topology}`
    : null
}

export function formatMs(ms: number | undefined): string {
  if (ms === undefined || !Number.isFinite(ms) || ms < 0) return '—'
  if (ms < 1000) return `${Math.round(ms)} ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`
  const minutes = Math.floor(ms / 60_000)
  const seconds = Math.round((ms % 60_000) / 1000)
  return `${minutes}m ${String(seconds).padStart(2, '0')}s`
}

export function formatTokens(tokens: number | undefined): string {
  if (tokens === undefined || !Number.isFinite(tokens) || tokens < 0) return '—'
  if (tokens < 1000) return String(Math.round(tokens))
  if (tokens < 1_000_000) return `${(tokens / 1000).toFixed(1)}k`
  return `${(tokens / 1_000_000).toFixed(2)}M`
}

export function formatPct(ratio: number | undefined, digits = 0): string {
  if (ratio === undefined || !Number.isFinite(ratio)) return '—'
  return `${(ratio * 100).toFixed(digits)}%`
}

/** 有符号百分比：-0.2 → "-20%"（相对实测的正负必须有号，否则读不出方向）。 */
export function formatDeltaPct(ratio: number | undefined, digits = 0): string {
  if (ratio === undefined || !Number.isFinite(ratio)) return '—'
  const pct = ratio * 100
  const sign = pct > 0 ? '+' : ''
  return `${sign}${pct.toFixed(digits)}%`
}

export function formatUsd(value: number | undefined): string {
  if (value === undefined || !Number.isFinite(value) || value < 0) return '—'
  if (value === 0) return '$0'
  if (value < 0.01) return '<$0.01'
  return `$${value.toFixed(2)}`
}

/** 推荐战术对应的结果行；没有推荐则返回 null。 */
export function recommendedResult(plan: PlanView | null): TacticResultView | null {
  const id = plan?.rationale?.recommended
  if (!id) return null
  return plan?.results?.find((row) => row.tactic === id) ?? null
}
