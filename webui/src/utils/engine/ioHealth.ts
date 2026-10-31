// 「写入量自观测」（W7）面板的读数渲染。
//
// 为什么这段逻辑必须存在并且必须是纯函数：本仓这一轮修的是一批「少写盘」的改动
// （台账去抖、compact 序列化、WAL、目录预算、决策日志轮转）。这类收益没有功能
// 症状 —— 撤掉了照样跑得对，于是会在下一次重构里悄悄流失。唯一能守住它的办法
// 是把「现在到底写了多少 / WAL 到底开没开 / 目录占了多大」摆到界面上。
//
// 三条渲染纪律（R57）：
// ① **读不到 ≠ 不存在 ≠ 0**。`journal_mode: null` 只能显示「未知」，绝不能显示
//    成「delete」——那会把一个纯粹取不到的情况说成「WAL 没开」，用户会去改
//    一个本来就好的东西。
// ② WAL 生效与否以**回读值**为准，不看请求值：Windows 上 `-wal/-shm` 被别的
//    进程持有时，`PRAGMA journal_mode=WAL` 会静默返回旧模式且不报错。
// ③ 超预算、WAL 未生效这类「值得动手」的读数打 warn 标记，由视图上色。

import { formatBytes } from '@/views/harness/harnessViewModel'

export interface IoFileEntry {
  present?: boolean
  bytes?: number | null
  path?: string
}

export interface IoDbEntry {
  present?: boolean
  bytes?: number | null
  journal_mode?: string | null
  wal_bytes?: number | null
  shm_present?: boolean
}

export interface IoDirEntry {
  present?: boolean
  files?: number
  bytes?: number | null
  budget_bytes?: number
  max_files?: number
  partial_flush_interval_ms?: number
}

export interface IoDecisionLogEntry extends IoFileEntry {
  rotate_at_bytes?: number
  keep_generations?: number
  generations?: Array<{ generation: number; bytes: number | null }>
}

export interface IoHealth {
  process_writes?: { count?: number; bytes?: number; scope?: string }
  databases?: { sessions?: IoDbEntry; usage?: IoDbEntry; audit?: IoDbEntry }
  context_ledger?: IoDirEntry
  plans?: IoFileEntry
  decision_log?: IoDecisionLogEntry
  tool_spool?: IoDirEntry
  /** 侧车子进程的 stdout/stderr 日志目录（父进程计数够不到，只有目录读数）。 */
  sidecar_logs?: IoDirEntry
}

export type Translate = (key: string, params?: Record<string, unknown>) => string

export interface IoRow {
  /** 稳定标识，供 v-for key 与测试断言用（不本地化）。 */
  id: string
  /** 数据名（文件名/库名），本就是标识符，不本地化。 */
  name: string
  value: string
  warn?: boolean
}

/** `present: false` 与「取不到」都要显式说出来，不能让它们塌成 0。 */
function absentText(t: Translate): string {
  return t('engineHealth.io.absent')
}

/** 目录行：`{files} 个文件 · {bytes} / {budget}`，超预算转 warn。 */
function dirValue(entry: IoDirEntry | undefined, t: Translate): { value: string; warn: boolean } {
  if (!entry || entry.present !== true) {
    return { value: absentText(t), warn: false }
  }
  const files = t('engineHealth.io.files', { count: entry.files ?? 0 })
  const bytes = formatBytes(entry.bytes ?? null)
  const budget = formatBytes(entry.budget_bytes ?? null)
  const over =
    typeof entry.budget_bytes === 'number' &&
    typeof entry.bytes === 'number' &&
    entry.bytes > entry.budget_bytes
  const cap = typeof entry.max_files === 'number' ? ` ≤ ${entry.max_files}` : ''
  return { value: `${files} · ${bytes} / ${budget}${cap}`, warn: over }
}

/** 库行：以回读的 journal_mode 为准；`null` 只能说「未知」。 */
function dbValue(name: string, entry: IoDbEntry | undefined, t: Translate): IoRow {
  if (!entry || entry.present !== true) {
    return { id: `db:${name}`, name, value: absentText(t) }
  }
  const mode = entry.journal_mode
  let modeLabel: string
  let warn = false
  if (mode === null || mode === undefined) {
    modeLabel = t('engineHealth.io.journalUnknown')
  } else if (mode === 'wal') {
    modeLabel = 'wal'
  } else {
    modeLabel = `${mode} · ${t('engineHealth.io.journalNotWal')}`
    warn = true
  }
  const bytes = formatBytes(entry.bytes ?? null)
  // `-wal` 是「还没合并下去的写入量」；没有该文件时如实留空，不补 0。
  const wal = entry.wal_bytes == null ? '' : ` (+wal ${formatBytes(entry.wal_bytes)})`
  return { id: `db:${name}`, name, value: `${modeLabel} · ${bytes}${wal}`, warn }
}

/** 生成面板行。后端缺 `io` 段（旧网关）时返回空数组，整块不渲染。 */
export function ioRows(io: IoHealth | null | undefined, t: Translate): IoRow[] {
  if (!io || !io.process_writes) return []
  const w = io.process_writes
  const rows: IoRow[] = [
    {
      id: 'writes',
      name: t('engineHealth.io.writes'),
      value: `${w.count ?? 0} · ${formatBytes(w.bytes ?? null)} · ${scopeLabel(w.scope, t)}`,
    },
  ]
  const dbs = io.databases ?? {}
  for (const name of ['sessions.db', 'usage.db', 'audit.db'] as const) {
    const key = name.split('.')[0] as 'sessions' | 'usage' | 'audit'
    rows.push(dbValue(name, dbs[key], t))
  }
  const context = dirValue(io.context_ledger, t)
  rows.push({ id: 'context', name: 'context/', value: context.value, warn: context.warn })
  const plans = io.plans
  rows.push({
    id: 'plans',
    name: 'plans.json',
    value: plans?.present === true ? formatBytes(plans.bytes ?? null) : absentText(t),
  })
  const log = io.decision_log
  if (log && log.present === true) {
    // 保留代（`.1`/`.2`）也算在这份 bytes 里，所以只补一个「另有几代」的计数，
    // 不让用户以为当前代就是全貌。
    const extra = (log.generations ?? []).filter((g) => g.bytes != null).length - 1
    const gens = extra > 0 ? ` +${extra}` : ''
    rows.push({
      id: 'decision_log',
      name: 'router_decision_log.jsonl',
      value: `${formatBytes(log.bytes ?? null)} / ${formatBytes(log.rotate_at_bytes ?? null)}${gens}`,
    })
  } else {
    rows.push({ id: 'decision_log', name: 'router_decision_log.jsonl', value: absentText(t) })
  }
  const spool = dirValue(io.tool_spool, t)
  rows.push({ id: 'spool', name: 'tool_spool/', value: spool.value, warn: spool.warn })
  // 侧车日志由**子进程**写，不在 `writes` 的口径里；它也只有目录总量封顶在管
  // （轮转只发生在 spawn 时刻，长跑期间单文件不切 —— 句柄在子进程手里，
  // 父进程改不动）。所以这一行的价值就是「让它的量可见」。
  const sidecarLogs = dirValue(io.sidecar_logs, t)
  rows.push({
    id: 'sidecar_logs',
    name: 'sidecar/*.log',
    value: sidecarLogs.value,
    warn: sidecarLogs.warn,
  })
  return rows
}

/**
 * 口径机器键 → 人话。未知键一律**如实说未知**，绝不猜一个「听起来对」的解释：
 * 后端加了新口径而前端没跟上时，猜出来的句子是在骗人。
 */
function scopeLabel(scope: string | undefined, t: Translate): string {
  if (scope === 'json_append_exits') return t('engineHealth.io.scopeJsonExits')
  return `${t('engineHealth.io.unknown')} ${scope ?? ''}`.trim()
}

/** 整块是否值得渲染（旧网关没有 io 段时不要把一片「—」摆出来）。 */
export function ioAvailable(io: IoHealth | null | undefined): boolean {
  return !!io && !!io.process_writes
}
