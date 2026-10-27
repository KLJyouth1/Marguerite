import { describe, expect, it } from 'vitest'
import { isConnectionRecycleError, isJobFailed, normalizeCronJobs } from './useCronJobs'
import { markCronFinishNotified, wasCronFinishNotified } from '@/utils/cron/notifications'

describe('cron.list wire shape normalization', () => {
  const job = { id: 'j1', name: 'nightly', enabled: true }

  it('accepts the three legitimate shapes without dropping jobs', () => {
    expect(normalizeCronJobs([job])).toHaveLength(1)
    expect(normalizeCronJobs({ jobs: [job] })).toHaveLength(1)
    expect(normalizeCronJobs({ items: [job] })).toHaveLength(1)
    expect(normalizeCronJobs(null)).toEqual([])
    expect(normalizeCronJobs({})).toEqual([])
  })

  it('derives next_run from the Rust millisecond field only when ISO is missing', () => {
    const ms = Date.UTC(2026, 9, 26, 12, 0, 0)
    const [fromMs] = normalizeCronJobs({
      items: [{ ...job, next_fire_ms: ms }],
    })
    expect(fromMs.next_run).toBe('2026-10-26T12:00:00.000Z')

    // 已有 ISO 时优先保留，不被毫秒字段覆盖。
    const [fromIso] = normalizeCronJobs([{ ...job, next_run: '2026-01-01T00:00:00Z', next_fire_ms: ms }])
    expect(fromIso.next_run).toBe('2026-01-01T00:00:00Z')

    // 非法/缺失毫秒 → 原样返回，不编造时间。
    const [invalid] = normalizeCronJobs({ jobs: [{ ...job, next_fire_ms: -1 }] })
    expect(invalid.next_run).toBeUndefined()
  })

  it('filters non-object entries instead of throwing', () => {
    expect(normalizeCronJobs({ jobs: [job, null, 'x', 42] })).toHaveLength(1)
  })
})

describe('cron job recovery helpers', () => {
  it('recognizes transient recycled and closed connection failures', () => {
    expect(isConnectionRecycleError('Connection recycled after workspaces.list terminated')).toBe(true)
    expect(isConnectionRecycleError('Connection closed')).toBe(true)
    expect(isConnectionRecycleError('Cannot call cron.run: not connected')).toBe(true)
    expect(isConnectionRecycleError('workspaceDir does not exist')).toBe(false)
  })

  it('recognizes failed jobs for the retry action', () => {
    expect(isJobFailed({ id: 'a', lastStatus: 'error', lastResult: 'boom', error_count: 1 })).toBe(true)
    expect(isJobFailed({ id: 'b', lastStatus: 'fail', lastResult: 'boom', error_count: 1 })).toBe(true)
    expect(isJobFailed({ id: 'c', lastStatus: 'ok', lastResult: null, error_count: 0 })).toBe(false)
  })

  it('suppresses duplicate completion toasts for the same recent run', () => {
    markCronFinishNotified('job-1', 1_000)
    expect(wasCronFinishNotified('job-1', 10_000)).toBe(true)
    expect(wasCronFinishNotified('job-1', 20_000)).toBe(false)
    expect(wasCronFinishNotified('job-2', 10_000)).toBe(false)
  })
})
