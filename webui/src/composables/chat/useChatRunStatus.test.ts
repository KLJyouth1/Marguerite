import { describe, expect, it } from 'vitest'
import { normalizeRunStatus, runStatusLabelText, sessionRunStatus } from './useChatRunStatus'

/**
 * 运行态词表的契约测试。
 *
 * 词表漂移过两次、分叉成两处实现（`composables/useSessions.ts` 与
 * `composables/chat/useChatRunStatus.ts`），且都漏了网关声明过的 `error`：
 * 失败轮次被静默归一成 `idle` —— 侧栏显示「空闲」、`sessionChangeIsTerminal()`
 * 也判不出终态（真机上表现为失败后又干等 630s 才报超时）。
 */
describe('normalizeRunStatus', () => {
  it('maps the gateway-declared `error` to failed, not to a silent idle', () => {
    expect(normalizeRunStatus('error')).toBe('failed')
    expect(normalizeRunStatus('ERROR')).toBe('failed')
  })

  it('keeps the canonical states untouched', () => {
    for (const state of [
      'queued',
      'running',
      'approval_pending',
      'interrupted',
      'failed',
      'timeout',
      'cancelled',
    ] as const) {
      expect(normalizeRunStatus(state)).toBe(state)
    }
  })

  it('folds the historical aliases the gateways actually emit', () => {
    expect(normalizeRunStatus('abandoned')).toBe('interrupted')
    expect(normalizeRunStatus('killed')).toBe('cancelled')
    expect(normalizeRunStatus('succeeded')).toBe('idle')
    expect(normalizeRunStatus('complete')).toBe('idle')
  })

  it('falls back to idle only for genuinely unknown values', () => {
    expect(normalizeRunStatus('')).toBe('idle')
    expect(normalizeRunStatus(undefined as unknown as string)).toBe('idle')
    expect(normalizeRunStatus('something-else')).toBe('idle')
  })
})

describe('sessionRunStatus', () => {
  it('reports a failed turn as failed (not idle) from a run_status frame', () => {
    expect(sessionRunStatus({ run_status: 'error' }).status).toBe('failed')
  })

  it('prefers an active task state over the last terminal one', () => {
    const state = sessionRunStatus({
      run_status: 'error',
      active_task: { status: 'running' },
    })
    expect(state.status).toBe('running')
  })

  it('surfaces the failure label so the UI does not claim idle', () => {
    const state = sessionRunStatus({ run_status: 'error' })
    expect(state.label).not.toBe(runStatusLabelText('idle'))
  })
})