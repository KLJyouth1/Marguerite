import { describe, expect, it } from 'vitest'
import {
  EMPTY_VALUE,
  coverageSkipped,
  coverageSkippedTotal,
  deriveKpis,
  filterRunsBySession,
  formatBytes,
  formatDuration,
  formatPercent,
  formatTimestamp,
  formatTokens,
  normalizeLimit,
  payloadReason,
  replayDigest,
  resolveItemCount,
  runTokens,
  selectRun,
  sessionOptions,
  sortItems,
  sortRuns,
  statusRole,
  visibleRuns,
  type LedgerItem,
  type LedgerRun,
  type LedgerStatusPayload,
} from './harnessViewModel'

function makeRun(overrides: Partial<LedgerRun> = {}): LedgerRun {
  return {
    runId: 'run-1',
    sessionKey: 'session-a',
    traceId: 'trace-1',
    target: 'chat',
    status: 'ok',
    startedAtMs: 1_000,
    endedAtMs: 2_000,
    elapsedMs: 1_000,
    promptTokens: 100,
    outputTokens: 200,
    model: 'test-model',
    policy: 'agent',
    itemCount: 0,
    items: [],
    ...overrides,
  }
}

function makeItem(overrides: Partial<LedgerItem> = {}): LedgerItem {
  return {
    seq: 1,
    type: 'tool',
    name: 'read_file',
    status: 'ok',
    elapsedMs: 120,
    tokensIn: 10,
    tokensOut: 20,
    ok: true,
    errorCode: null,
    ...overrides,
  }
}

function makeStatus(overrides: Partial<LedgerStatusPayload> = {}): LedgerStatusPayload {
  return {
    ok: true,
    enabled: true,
    path: '/tmp/ledger.db',
    schemaVersion: 1,
    runs: 12,
    items: 340,
    bytes: 2048,
    oldestTsMs: 1_000,
    newestTsMs: 2_000,
    successRate: 0.875,
    avgElapsedMs: 1_500,
    p95ElapsedMs: 65_000,
    tokensIn: 12_345,
    tokensOut: 2_500_000,
    lastError: null,
    ...overrides,
  }
}

describe('格式化纯函数', () => {
  it('formatBytes 按 1024 进制输出且非法输入降级', () => {
    expect(formatBytes(0)).toBe('0 B')
    expect(formatBytes(512)).toBe('512 B')
    expect(formatBytes(1536)).toBe('1.5 KB')
    expect(formatBytes(5 * 1024 * 1024)).toBe('5.0 MB')
    expect(formatBytes(-1)).toBe(EMPTY_VALUE)
    expect(formatBytes(Number.NaN)).toBe(EMPTY_VALUE)
    expect(formatBytes(null)).toBe(EMPTY_VALUE)
  })

  it('formatDuration 覆盖 ms / s / 分钟并拒绝非法值', () => {
    expect(formatDuration(0)).toBe('0 ms')
    expect(formatDuration(850)).toBe('850 ms')
    expect(formatDuration(1_500)).toBe('1.5 s')
    expect(formatDuration(65_000)).toBe('1m 5s')
    expect(formatDuration(119_600)).toBe('2m 0s')
    expect(formatDuration(-5)).toBe(EMPTY_VALUE)
    expect(formatDuration(undefined)).toBe(EMPTY_VALUE)
  })

  it('formatPercent 把 0..1 映射为百分比并截断越界值', () => {
    expect(formatPercent(0)).toBe('0%')
    expect(formatPercent(0.5)).toBe('50%')
    expect(formatPercent(0.8567)).toBe('85.7%')
    expect(formatPercent(2)).toBe('100%')
    expect(formatPercent(-1)).toBe('0%')
    expect(formatPercent(null)).toBe(EMPTY_VALUE)
  })

  it('formatTokens 使用 k / M 缩写且非法输入降级', () => {
    expect(formatTokens(999)).toBe('999')
    expect(formatTokens(1_500)).toBe('1.5k')
    expect(formatTokens(2_500_000)).toBe('2.5M')
    expect(formatTokens(-3)).toBe(EMPTY_VALUE)
    expect(formatTokens('100' as unknown)).toBe(EMPTY_VALUE)
  })

  it('formatTimestamp 对缺失值与非法值返回占位符', () => {
    expect(formatTimestamp(0)).toBe(EMPTY_VALUE)
    expect(formatTimestamp(null)).toBe(EMPTY_VALUE)
    expect(formatTimestamp(Number.NaN)).toBe(EMPTY_VALUE)
    expect(formatTimestamp(1_700_000_000_000)).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/)
  })

  it('statusRole 把后端状态归到语义角色，未知值不猜测', () => {
    expect(statusRole('ok')).toBe('ok')
    expect(statusRole('SUCCESS')).toBe('ok')
    expect(statusRole('failed')).toBe('failed')
    expect(statusRole('in_progress')).toBe('running')
    expect(statusRole('canceled')).toBe('cancelled')
    expect(statusRole('timeout')).toBe('other')
    expect(statusRole(null)).toBe('other')
  })
})

describe('排序与过滤纯函数', () => {
  it('sortRuns 默认最新在前、支持升序、无效时间沉底且不修改入参', () => {
    const runs = [
      makeRun({ runId: 'old', startedAtMs: 1_000 }),
      makeRun({ runId: 'new', startedAtMs: 3_000 }),
      makeRun({ runId: 'bad', startedAtMs: Number.NaN }),
      makeRun({ runId: 'mid', startedAtMs: 2_000 }),
    ]
    expect(sortRuns(runs).map((run) => run.runId)).toEqual(['new', 'mid', 'old', 'bad'])
    expect(sortRuns(runs, 'asc').map((run) => run.runId)).toEqual(['old', 'mid', 'new', 'bad'])
    expect(runs.map((run) => run.runId)).toEqual(['old', 'new', 'bad', 'mid'])
    expect(sortRuns(null)).toEqual([])
  })

  it('filterRunsBySession 与 visibleRuns 组合过滤和排序', () => {
    const runs = [
      makeRun({ runId: 'a1', sessionKey: 'a', startedAtMs: 1_000 }),
      makeRun({ runId: 'b1', sessionKey: 'b', startedAtMs: 2_000 }),
      makeRun({ runId: 'a2', sessionKey: 'a', startedAtMs: 3_000 }),
    ]
    expect(filterRunsBySession(runs, '').length).toBe(3)
    expect(filterRunsBySession(runs, null).length).toBe(3)
    expect(filterRunsBySession(runs, 'a').map((run) => run.runId)).toEqual(['a1', 'a2'])
    expect(filterRunsBySession(null, 'a')).toEqual([])
    expect(visibleRuns(runs, 'a').map((run) => run.runId)).toEqual(['a2', 'a1'])
    expect(visibleRuns(runs, 'missing')).toEqual([])
  })

  it('sessionOptions 去重并稳定排序', () => {
    const runs = [
      makeRun({ sessionKey: 'zeta' }),
      makeRun({ sessionKey: 'alpha' }),
      makeRun({ sessionKey: 'zeta' }),
      makeRun({ sessionKey: '' }),
    ]
    expect(sessionOptions(runs)).toEqual(['alpha', 'zeta'])
    expect(sessionOptions(undefined)).toEqual([])
  })

  it('selectRun 匹配 runId、失效时回退首条、空列表返回 null', () => {
    const runs = [makeRun({ runId: 'one' }), makeRun({ runId: 'two' })]
    expect(selectRun(runs, 'two')?.runId).toBe('two')
    expect(selectRun(runs, 'gone')?.runId).toBe('one')
    expect(selectRun([], 'one')).toBeNull()
    expect(selectRun(null, 'one')).toBeNull()
  })

  it('resolveItemCount 优先 itemCount，缺失时回退 items.length', () => {
    expect(resolveItemCount(makeRun({ itemCount: 7 }))).toBe(7)
    expect(resolveItemCount(makeRun({ itemCount: Number.NaN, items: [makeItem(), makeItem()] }))).toBe(2)
    expect(resolveItemCount(null)).toBe(0)
  })

  it('sortItems 按 seq 升序且容忍异常输入', () => {
    const items = [
      makeItem({ seq: 3 }),
      makeItem({ seq: 1 }),
      makeItem({ seq: Number.NaN }),
      makeItem({ seq: 2 }),
    ]
    expect(sortItems(items).map((item) => item.seq)).toEqual([1, 2, 3, Number.NaN])
    expect(sortItems(null)).toEqual([])
  })
})

describe('KPI 与摘要派生', () => {
  it('deriveKpis 在台账不可用或 ok=false 时全部返回占位符', () => {
    expect(deriveKpis(null)).toEqual({
      runs: EMPTY_VALUE,
      items: EMPTY_VALUE,
      successRate: EMPTY_VALUE,
      p95: EMPTY_VALUE,
      tokens: EMPTY_VALUE,
      dbSize: EMPTY_VALUE,
    })
    expect(deriveKpis(makeStatus({ ok: false })).runs).toBe(EMPTY_VALUE)
  })

  it('deriveKpis 从真实 status 派生六项指标', () => {
    const kpis = deriveKpis(makeStatus())
    expect(kpis.runs).toBe('12')
    expect(kpis.items).toBe('340')
    expect(kpis.successRate).toBe('87.5%')
    expect(kpis.p95).toBe('1m 5s')
    expect(kpis.tokens).toBe('12.3k / 2.5M')
    expect(kpis.dbSize).toBe('2.0 KB')
  })

  it('payloadReason / replayDigest / runTokens 对缺失输入不崩', () => {
    expect(payloadReason({ error: ' ledger closed ' })).toBe('ledger closed')
    expect(payloadReason(null)).toBe('')
    expect(payloadReason({ error: undefined })).toBe('')
    expect(replayDigest({ ok: true, digest: ' sha256:abc ', runId: 'r', itemCount: 1, deterministic: true, note: '' })).toBe('sha256:abc')
    expect(replayDigest({ ok: false, digest: 'sha256:abc', runId: 'r', itemCount: 0, deterministic: true, note: '' })).toBe('')
    expect(replayDigest(null)).toBe('')
    expect(runTokens(makeRun())).toBe('100 / 200')
    expect(runTokens(null)).toBe(`${EMPTY_VALUE} / ${EMPTY_VALUE}`)
  })

  it('normalizeLimit 只接受既有档位', () => {
    expect(normalizeLimit(50)).toBe(50)
    expect(normalizeLimit('25')).toBe(25)
    expect(normalizeLimit(999)).toBe(25)
    expect(normalizeLimit(null)).toBe(25)
  })

  it('技能覆盖跳过清单容忍缺失字段并以 skippedTotal 为准', () => {
    const coverage = {
      ok: true,
      root: '/skills',
      directories: 3,
      filesScanned: 9,
      loaded: 4,
      yaml: 2,
      skillMd: 2,
      skipped: [
        { path: 'a.yaml', reason: 'parse' },
        { path: 'b.md', reason: 'duplicate' },
      ],
      skippedTotal: 5,
    }
    expect(coverageSkipped(coverage).map((entry) => entry.path)).toEqual(['a.yaml', 'b.md'])
    expect(coverageSkippedTotal(coverage)).toBe(5)
    expect(coverageSkipped(null)).toEqual([])
    expect(coverageSkippedTotal({ ...coverage, skippedTotal: undefined as unknown as number })).toBe(2)
  })
})
