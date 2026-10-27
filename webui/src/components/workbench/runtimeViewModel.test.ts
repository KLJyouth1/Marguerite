import { describe, it, expect } from 'vitest'
import {
  bloomLimits,
  contextPercent,
  contextTone,
  formatPercent,
  formatTokens,
  terminalSummary,
  toolCounts,
  verdictTone,
  verificationVerdict,
} from './runtimeViewModel'

describe('contextPercent', () => {
  it('computes a rounded share of the window', () => {
    expect(contextPercent(64_000, 128_000)).toBe(50)
    expect(contextPercent(1, 3)).toBe(33)
  })

  it('returns null instead of faking 0% when data is missing', () => {
    expect(contextPercent(undefined, 128_000)).toBe(null)
    expect(contextPercent(10, undefined)).toBe(null)
    expect(contextPercent(10, 0)).toBe(null)
    expect(contextPercent(Number.NaN, 100)).toBe(null)
  })

  it('clamps to 100 rather than showing >100%', () => {
    expect(contextPercent(300, 100)).toBe(100)
  })
})

describe('contextTone', () => {
  it('maps the backend alert verbatim, no local thresholds', () => {
    expect(contextTone('compact_required')).toBe('danger')
    expect(contextTone('warning')).toBe('warn')
    expect(contextTone('normal')).toBe('ok')
    expect(contextTone(undefined)).toBe('muted')
    expect(contextTone('something_new')).toBe('muted')
  })
})

describe('toolCounts', () => {
  it('counts enabled vs disabled and tolerates a missing list', () => {
    expect(toolCounts(undefined)).toEqual({ total: 0, enabled: 0, disabled: 0 })
    expect(toolCounts([{ enabled: true }, { enabled: false }, {}])).toEqual({
      total: 3,
      // 缺失 enabled 视为启用（后端目录里未禁用即启用）
      enabled: 2,
      disabled: 1,
    })
  })
})

describe('terminalSummary', () => {
  it('counts running sessions only', () => {
    expect(terminalSummary(null)).toEqual({ total: 0, running: 0 })
    expect(terminalSummary([{ running: true }, { running: false }, {}])).toEqual({
      total: 3,
      running: 1,
    })
  })
})

describe('verificationVerdict', () => {
  it('degrades honestly when the receipt is missing or failed', () => {
    expect(verificationVerdict(null)).toBe('unavailable')
    expect(verificationVerdict({ ok: false })).toBe('unavailable')
    expect(verificationVerdict({ ok: true })).toBe('unknown')
  })

  it('passes through the backend verdict', () => {
    expect(verificationVerdict({ ok: true, verification: { status: 'passed' } })).toBe('passed')
    expect(verificationVerdict({ ok: true, verification: { status: 'failed' } })).toBe('failed')
    expect(verificationVerdict({ ok: true, verification: { status: 'unverified' } })).toBe('unverified')
  })

  it('colours failure as danger and unverified as warn', () => {
    expect(verdictTone('failed')).toBe('danger')
    expect(verdictTone('unverified')).toBe('warn')
    expect(verdictTone('passed')).toBe('ok')
    expect(verdictTone('unavailable')).toBe('muted')
  })
})

describe('formatTokens', () => {
  it('formats compactly and never prints NaN', () => {
    expect(formatTokens(999)).toBe('999')
    expect(formatTokens(12_345)).toBe('12.3k')
    expect(formatTokens(1_234_567)).toBe('1.2M')
    expect(formatTokens(null)).toBe('—')
    expect(formatTokens(-1)).toBe('—')
  })
})

describe('formatPercent', () => {
  it('uses an em dash for unknown values', () => {
    expect(formatPercent(null)).toBe('—')
    expect(formatPercent(42)).toBe('42%')
  })
})

describe('bloomLimits', () => {
  it('exposes only positive, real limits', () => {
    expect(bloomLimits(null)).toEqual({
      shards: null,
      subExperts: null,
      verificationPasses: null,
      thinkingDepth: null,
      maxLlmCalls: null,
    })
    expect(bloomLimits({ bloom: { max_swarm_shards: 4, verification_passes: 0 } })).toMatchObject({
      shards: 4,
      verificationPasses: null,
    })
  })
})
