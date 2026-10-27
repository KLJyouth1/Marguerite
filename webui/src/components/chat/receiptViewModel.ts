/**
 * 交付回执的纯函数视图模型。
 *
 * 全部逻辑与 DOM 无关，便于在 vitest 里直接断言；组件只负责渲染。
 * 口径与后端 `task.receipt` 对齐：**没有证据就是未验证**，不做「看起来应该没问题」的推断。
 */

export type ReceiptVerificationStatus = 'passed' | 'failed' | 'unverified' | 'unknown'
export type ReceiptTone = 'ok' | 'danger' | 'warn' | 'muted'

export interface ReceiptCheck {
  tool: string
  command: string
  /** null = 旧数据缺少成败标记，既不算通过也不算失败 */
  ok: boolean | null
  elapsedMs: number
}

export interface ReceiptChange {
  path: string
  kind: string
  tool: string
  ts: number
  beforeChars?: number
  afterChars?: number
}

export interface ReceiptArtifact {
  id: string
  name: string
  path: string
  kind: string
  size: number
  ts: number
  present: boolean
}

export interface ReceiptGap {
  code: string
  count?: number
  detail?: string
  paths?: string[]
}

export interface ReceiptCounts {
  userTurns: number
  assistantTurns: number
  toolCalls: number
  toolFailures: number
  toolUnknown: number
  changes: number
  artifacts: number
  artifactsPresent: number
}

export interface ReceiptBudget {
  available: boolean
  runs: number
  tokensIn: number | null
  tokensOut: number | null
  model: string | null
  policy: string | null
  runsElapsedMs: number | null
}

export interface TaskReceipt {
  ok: boolean
  error?: string
  code?: string
  session?: { key?: string; title?: string; projectPath?: string | null }
  scope?: { since?: string | null; until?: string | null; degraded?: boolean; note?: string }
  counts?: ReceiptCounts
  changes?: ReceiptChange[]
  artifacts?: ReceiptArtifact[]
  verification?: { status?: ReceiptVerificationStatus; passedChecks?: number; checks?: ReceiptCheck[] }
  budget?: ReceiptBudget
  unattested?: ReceiptGap[]
}

/** 验证状态 → 视觉基调（只用于配色，不改变语义）。 */
export function receiptTone(status: string | undefined): ReceiptTone {
  switch (status) {
    case 'passed':
      return 'ok'
    case 'failed':
      return 'danger'
    case 'unknown':
      return 'warn'
    default:
      return 'muted'
  }
}

/** 顶栏摘要：只统计真实存在的条目，缺字段按 0（不猜）。 */
export function receiptCounts(receipt: TaskReceipt | null): {
  changes: number
  artifacts: number
  tools: number
  failures: number
} {
  const c = receipt?.counts
  return {
    changes: c?.changes ?? receipt?.changes?.length ?? 0,
    artifacts: c?.artifacts ?? receipt?.artifacts?.length ?? 0,
    tools: c?.toolCalls ?? 0,
    failures: c?.toolFailures ?? 0,
  }
}

/** 是否值得展开成一张卡：纯对话（无改动/无产物/无工具）就不打扰用户。 */
export function hasReceiptContent(receipt: TaskReceipt | null): boolean {
  if (!receipt || !receipt.ok) return false
  const c = receiptCounts(receipt)
  return c.changes > 0 || c.artifacts > 0 || c.tools > 0
}

/**
 * 缺口排序：先「没验证」，再其它。信息缺失类永远排在最后，
 * 避免用户第一眼看到的是噪音而不是「这轮没验证」。
 */
const GAP_ORDER = [
  'no_verification',
  'changes_without_verification',
  'artifact_missing_on_disk',
  'tool_result_unknown',
  'usage_ledger_unavailable',
  'session_db_unavailable',
  'history_unreadable',
  'no_deliverables',
]

export function sortGaps(gaps: ReceiptGap[] | undefined): ReceiptGap[] {
  const list = (gaps || []).filter(g => g && typeof g.code === 'string')
  return [...list].sort((a, b) => {
    const ia = GAP_ORDER.indexOf(a.code)
    const ib = GAP_ORDER.indexOf(b.code)
    return (ia < 0 ? GAP_ORDER.length : ia) - (ib < 0 ? GAP_ORDER.length : ib)
  })
}

/** 验证徽标的计数文案数据（本地化交给组件）。 */
export function verificationCounts(receipt: TaskReceipt | null): {
  checks: number
  passed: number
  failed: number
  unknown: number
} {
  const checks = receipt?.verification?.checks || []
  let passed = 0
  let failed = 0
  let unknown = 0
  for (const c of checks) {
    if (c.ok === true) passed += 1
    else if (c.ok === false) failed += 1
    else unknown += 1
  }
  return { checks: checks.length, passed, failed, unknown }
}

/** 产物里「登记了但磁盘上没了」的条目（回执必须如实点出来）。 */
export function missingArtifacts(receipt: TaskReceipt | null): ReceiptArtifact[] {
  return (receipt?.artifacts || []).filter(a => a.present === false)
}

/** 把字节数格式化成可读体积（回执里的产物大小）。 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  const rounded = unit === 0 ? Math.round(value) : Math.round(value * 10) / 10
  return `${rounded} ${units[unit]}`
}
