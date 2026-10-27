/**
 * 「运行态」面板的纯函数视图模型。
 *
 * 口径（与后端一致，不猜）：
 * - 上下文压力只认后端 `context.status.alert`，前端不另设阈值（两处阈值必然漂移）；
 * - 预算 / 用量缺失时返回 `null`，由 UI 显示「—」，**不显示 0**（0% 会被读成「空」）；
 * - 质量结论只认 `task.receipt.verification.status`，没有回执就是 `unavailable`。
 */

export type StatTone = 'ok' | 'warn' | 'danger' | 'muted'

export type VerificationVerdict = 'passed' | 'failed' | 'unverified' | 'unknown' | 'unavailable'

/** 上下文占用百分比；预算缺失返回 null（不拿 0 冒充「什么都没用」）。 */
export function contextPercent(used?: number | null, budget?: number | null): number | null {
  if (typeof used !== 'number' || typeof budget !== 'number') return null
  if (!Number.isFinite(used) || !Number.isFinite(budget) || budget <= 0 || used < 0) return null
  return Math.min(100, Math.round((used / budget) * 100))
}

/** 压力基调：只依据后端告警级别（normal / warning / compact_required）。 */
export function contextTone(alert?: string | null): StatTone {
  switch (alert) {
    case 'compact_required':
      return 'danger'
    case 'warning':
      return 'warn'
    case 'normal':
      return 'ok'
    default:
      return 'muted'
  }
}

/** 工具面：总 / 启用 / 禁用（对应「工具不一定越多越好」——启用越少越聚焦）。 */
export function toolCounts(items?: Array<{ enabled?: boolean }> | null): {
  total: number
  enabled: number
  disabled: number
} {
  const list = Array.isArray(items) ? items : []
  let enabled = 0
  for (const item of list) {
    if (item && item.enabled !== false) enabled += 1
  }
  return { total: list.length, enabled, disabled: list.length - enabled }
}

/** 终端会话：本机看到的总数 / 运行中数量。 */
export function terminalSummary(terminals?: Array<{ running?: boolean }> | null): {
  total: number
  running: number
} {
  const list = Array.isArray(terminals) ? terminals : []
  return {
    total: list.length,
    running: list.filter(item => item?.running === true).length,
  }
}

/** 质量门结论：没有回执 / 回执失败一律如实降级。 */
export function verificationVerdict(receipt?: {
  ok?: boolean
  verification?: { status?: string }
} | null): VerificationVerdict {
  if (!receipt || receipt.ok === false) return 'unavailable'
  switch (receipt.verification?.status) {
    case 'passed':
    case 'failed':
    case 'unknown':
      return receipt.verification.status
    case 'unverified':
      return 'unverified'
    default:
      return 'unknown'
  }
}

export function verdictTone(verdict: VerificationVerdict): StatTone {
  switch (verdict) {
    case 'passed':
      return 'ok'
    case 'failed':
      return 'danger'
    case 'unverified':
      return 'warn'
    case 'unknown':
      return 'warn'
    default:
      return 'muted'
  }
}

/** 紧凑 token 数：12345 → 12.3k。缺失返回 '—'。 */
export function formatTokens(value?: number | null): string {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) return '—'
  if (value < 1000) return String(Math.round(value))
  if (value < 1_000_000) {
    const k = value / 1000
    return `${k >= 100 ? Math.round(k) : Math.round(k * 10) / 10}k`
  }
  return `${Math.round((value / 1_000_000) * 10) / 10}M`
}

/** 百分比文案：null → '—'（不把「没有数据」画成 0%）。 */
export function formatPercent(value: number | null): string {
  return value === null ? '—' : `${value}%`
}

/**
 * 模式规格里的「编排上限」：只有 Bloom 类模式才给出真实分片/验证轮次，
 * 其它模式返回 null（不把默认值当成用户可调项展示）。
 */
export function bloomLimits(spec?: {
  bloom?: {
    max_swarm_shards?: number
    max_sub_experts?: number
    verification_passes?: number
    thinking_depth?: number
    effective_max_llm_calls?: number
  } | null
} | null): {
  shards: number | null
  subExperts: number | null
  verificationPasses: number | null
  thinkingDepth: number | null
  maxLlmCalls: number | null
} {
  const bloom = spec?.bloom
  const pick = (value?: number): number | null =>
    typeof value === 'number' && Number.isFinite(value) && value > 0 ? value : null
  return {
    shards: pick(bloom?.max_swarm_shards),
    subExperts: pick(bloom?.max_sub_experts),
    verificationPasses: pick(bloom?.verification_passes),
    thinkingDepth: pick(bloom?.thinking_depth),
    maxLlmCalls: pick(bloom?.effective_max_llm_calls),
  }
}
