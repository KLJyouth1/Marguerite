import type { UsageDailyPoint, UsageDailyResponse } from '@/types/usage'

const DAY_MS = 86_400_000

export type HeatGridLevel = 0 | 1 | 2 | 3 | 4

export interface HeatGridCell {
  dayMs: number
  tokens: number
  calls: number
  level: HeatGridLevel
  future: boolean
}

export interface HeatGridWeek {
  weekStartMs: number
  /** `YYYY-MM` (UTC) — the component localizes the month label. */
  monthKey: string
  cells: HeatGridCell[]
}

export interface HeatGridModel {
  weeks: HeatGridWeek[]
  startMs: number
  endMs: number
  maxTokens: number
  totalTokens: number
  totalCalls: number
  activeDays: number
}

/**
 * `usage.daily` 以 UTC 日边界分桶（`created_ms / DAY_MS`），这里保持同一口径，
 * 避免把桶边界按本地时区重算后出现「同一格两天数据」。
 */
export function utcDayIndex(value: number | string | null | undefined): number | null {
  if (value == null || value === '') return null
  const ms = Number(value)
  if (!Number.isFinite(ms)) return null
  return Math.floor(ms / DAY_MS)
}

export function normalizeUsageDaily(response: UsageDailyResponse | null | undefined): UsageDailyPoint[] {
  const rows = Array.isArray(response?.days) ? response.days : []
  const points: UsageDailyPoint[] = []
  for (const row of rows) {
    const dayMs = firstNumber(row?.day, row?.dayMs, row?.day_ms)
    if (dayMs == null) continue
    points.push({
      dayMs,
      totalTokens: Math.max(0, firstNumber(row?.total_tokens, row?.totalTokens) ?? 0),
      calls: Math.max(0, firstNumber(row?.calls, row?.count) ?? 0),
    })
  }
  return points.sort((a, b) => a.dayMs - b.dayMs)
}

function firstNumber(...values: unknown[]): number | null {
  for (const value of values) {
    if (value == null || value === '') continue
    const number = Number(value)
    if (Number.isFinite(number)) return number
  }
  return null
}

function levelFor(tokens: number, maxTokens: number): HeatGridLevel {
  if (tokens <= 0 || maxTokens <= 0) return 0
  const ratio = tokens / maxTokens
  if (ratio <= 0.25) return 1
  if (ratio <= 0.5) return 2
  if (ratio <= 0.75) return 3
  return 4
}

function monthKeyOf(dayIndex: number): string {
  return new Date(dayIndex * DAY_MS).toISOString().slice(0, 7)
}

/**
 * 生成按周对齐的网格（周一为每列第一行，最后一列是本周）。
 * 未来日期保留空位但不参与统计，避免把「尚未到来的日子」画成 0 用量。
 */
export function buildHeatGrid(
  points: UsageDailyPoint[],
  options: { weeks?: number; now?: Date } = {},
): HeatGridModel {
  const weeks = Math.max(1, Math.min(26, Math.floor(options.weeks ?? 12)))
  const nowMs = (options.now ?? new Date()).getTime()
  const todayIndex = Math.floor(nowMs / DAY_MS)
  // 1970-01-01（epoch day 0）是周四；+3 后周一为 0。
  const todayWeekday = ((todayIndex + 3) % 7 + 7) % 7
  const currentWeekStart = todayIndex - todayWeekday
  const startIndex = currentWeekStart - (weeks - 1) * 7
  const endIndex = currentWeekStart + 6

  const totals = new Map<number, { tokens: number; calls: number }>()
  let totalTokens = 0
  let totalCalls = 0
  for (const point of points) {
    const index = utcDayIndex(point.dayMs)
    if (index == null || index < startIndex || index > todayIndex) continue
    const current = totals.get(index) ?? { tokens: 0, calls: 0 }
    current.tokens += Math.max(0, Number(point.totalTokens) || 0)
    current.calls += Math.max(0, Number(point.calls) || 0)
    totals.set(index, current)
    totalTokens += Math.max(0, Number(point.totalTokens) || 0)
    totalCalls += Math.max(0, Number(point.calls) || 0)
  }

  let maxTokens = 0
  let activeDays = 0
  for (const value of totals.values()) {
    if (value.tokens > maxTokens) maxTokens = value.tokens
    if (value.tokens > 0) activeDays += 1
  }

  const gridWeeks: HeatGridWeek[] = []
  for (let week = 0; week < weeks; week += 1) {
    const weekStartIndex = startIndex + week * 7
    const cells: HeatGridCell[] = []
    for (let row = 0; row < 7; row += 1) {
      const dayIndex = weekStartIndex + row
      const value = totals.get(dayIndex)
      const tokens = value?.tokens ?? 0
      cells.push({
        dayMs: dayIndex * DAY_MS,
        tokens,
        calls: value?.calls ?? 0,
        level: levelFor(tokens, maxTokens),
        future: dayIndex > todayIndex,
      })
    }
    gridWeeks.push({
      weekStartMs: weekStartIndex * DAY_MS,
      monthKey: monthKeyOf(weekStartIndex),
      cells,
    })
  }

  return {
    weeks: gridWeeks,
    startMs: startIndex * DAY_MS,
    endMs: endIndex * DAY_MS,
    maxTokens,
    totalTokens,
    totalCalls,
    activeDays,
  }
}
