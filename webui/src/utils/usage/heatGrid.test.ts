import { describe, expect, it } from 'vitest'
import { buildHeatGrid, normalizeUsageDaily, utcDayIndex } from './heatGrid'

const point = (iso: string, totalTokens: number, calls = 1) => ({
  dayMs: Date.parse(iso),
  totalTokens,
  calls,
})

describe('buildHeatGrid', () => {
  it('aligns the grid to Monday weeks and leaves future days empty', () => {
    // 2024-01-03 is a Wednesday; two weeks start on 2023-12-25.
    const model = buildHeatGrid([], { weeks: 2, now: new Date('2024-01-03T12:00:00Z') })

    expect(model.weeks).toHaveLength(2)
    expect(model.weeks[0]?.cells).toHaveLength(7)
    expect(model.startMs).toBe(Date.parse('2023-12-25T00:00:00Z'))
    expect(model.endMs).toBe(Date.parse('2024-01-07T00:00:00Z'))
    expect(model.weeks[0]?.cells[0]?.dayMs).toBe(Date.parse('2023-12-25T00:00:00Z'))
    expect(model.weeks[1]?.cells[2]?.future).toBe(false)
    expect(model.weeks[1]?.cells[3]?.future).toBe(true)
    expect(model.weeks[1]?.cells[6]?.future).toBe(true)
    expect(model.activeDays).toBe(0)
    expect(model.weeks.flatMap(week => week.cells).every(cell => cell.level === 0)).toBe(true)
  })

  it('sums duplicate days, ignores out-of-window points and ranks levels against the window max', () => {
    const model = buildHeatGrid([
      point('2023-12-25T08:00:00Z', 10),
      point('2024-01-01T08:00:00Z', 100),
      point('2024-01-01T09:00:00Z', 50),
      point('2024-01-02T08:00:00Z', 50),
      point('2024-01-03T08:00:00Z', 25),
      point('2024-01-08T08:00:00Z', 999),
    ], { weeks: 2, now: new Date('2024-01-03T12:00:00Z') })

    expect(model.totalTokens).toBe(235)
    expect(model.totalCalls).toBe(5)
    expect(model.maxTokens).toBe(150)
    expect(model.activeDays).toBe(4)

    const cells = model.weeks.flatMap(week => week.cells)
    const byDay = new Map(cells.map(cell => [cell.dayMs, cell]))
    expect(byDay.get(Date.parse('2024-01-01T00:00:00Z'))?.tokens).toBe(150)
    expect(byDay.get(Date.parse('2024-01-01T00:00:00Z'))?.level).toBe(4)
    expect(byDay.get(Date.parse('2024-01-02T00:00:00Z'))?.level).toBe(2)
    expect(byDay.get(Date.parse('2024-01-03T00:00:00Z'))?.level).toBe(1)
    expect(byDay.get(Date.parse('2023-12-25T00:00:00Z'))?.level).toBe(1)
    expect(byDay.has(Date.parse('2024-01-08T00:00:00Z'))).toBe(false)
  })

  it('keeps months identifiable for the label row', () => {
    // 2024-02-07 is a Wednesday; five aligned weeks start on 2024-01-08.
    const model = buildHeatGrid([], { weeks: 5, now: new Date('2024-02-07T12:00:00Z') })
    const labels = model.weeks.map(week => week.monthKey)
    expect(labels).toEqual(['2024-01', '2024-01', '2024-01', '2024-01', '2024-02'])
  })
})

describe('normalizeUsageDaily', () => {
  it('normalizes snake_case wire rows and drops invalid days', () => {
    const points = normalizeUsageDaily({
      days: [
        { day: '1704067200000', total_tokens: '120', calls: 3 },
        { day: 1704153600000, totalTokens: 40, count: 2 },
        { day: null, total_tokens: 9 },
        { day: 1704240000000, total_tokens: -5, calls: -1 },
      ],
    })

    expect(points).toEqual([
      { dayMs: 1704067200000, totalTokens: 120, calls: 3 },
      { dayMs: 1704153600000, totalTokens: 40, calls: 2 },
      { dayMs: 1704240000000, totalTokens: 0, calls: 0 },
    ])
  })

  it('returns an empty list for missing payloads', () => {
    expect(normalizeUsageDaily(null)).toEqual([])
    expect(normalizeUsageDaily({})).toEqual([])
  })

  it('buckets timestamps by UTC day', () => {
    expect(utcDayIndex(Date.parse('2024-01-01T23:59:59Z'))).toBe(19723)
    expect(utcDayIndex(String(Date.parse('2024-01-02T00:00:00Z')))).toBe(19724)
    expect(utcDayIndex(null)).toBeNull()
  })
})
