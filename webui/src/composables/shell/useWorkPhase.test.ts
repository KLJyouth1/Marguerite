import { describe, expect, it } from 'vitest'
import { ref } from 'vue'
import { deriveWorkPhase, useWorkPhase } from './useWorkPhase'

const SESSION = [{ key: 'sess-1' }]

describe('deriveWorkPhase', () => {
  it('is idle when there is no active session', () => {
    expect(deriveWorkPhase({ sessions: [], activeToolCount: 0, pendingConfirmationCount: 0 }))
      .toEqual({ kind: 'idle', reason: 'noActiveSession' })
  })

  it('is explore when a session is open but no tool call is running', () => {
    expect(deriveWorkPhase({ sessions: SESSION, activeToolCount: 0, pendingConfirmationCount: 0 }))
      .toEqual({ kind: 'explore', reason: 'noToolCalls' })
  })

  it('is execute while tool groups are in flight', () => {
    expect(deriveWorkPhase({ sessions: SESSION, activeToolCount: 2, pendingConfirmationCount: 0 }))
      .toEqual({ kind: 'execute', reason: 'toolsRunning', count: 2 })
  })

  it('is review (and wins over execute) while something awaits confirmation', () => {
    expect(deriveWorkPhase({ sessions: SESSION, activeToolCount: 3, pendingConfirmationCount: 1 }))
      .toEqual({ kind: 'review', reason: 'awaitingConfirmation', count: 1 })
  })
})

describe('missing signals', () => {
  it('falls back to idle instead of guessing when a signal is absent', () => {
    // 没有会话列表 / 计数未上报 → 不能声称 explore 或 execute。
    expect(deriveWorkPhase({ sessions: null, activeToolCount: 1, pendingConfirmationCount: 1 }))
      .toEqual({ kind: 'idle', reason: 'noActiveSession' })
    expect(deriveWorkPhase({ sessions: undefined, activeToolCount: 1, pendingConfirmationCount: 0 }))
      .toEqual({ kind: 'idle', reason: 'noActiveSession' })
    expect(deriveWorkPhase({ sessions: SESSION, activeToolCount: null, pendingConfirmationCount: 0 }))
      .toEqual({ kind: 'idle', reason: 'signalsIncomplete' })
    expect(deriveWorkPhase({ sessions: SESSION, activeToolCount: 0, pendingConfirmationCount: undefined }))
      .toEqual({ kind: 'idle', reason: 'signalsIncomplete' })
    expect(deriveWorkPhase({ sessions: SESSION, activeToolCount: Number.NaN, pendingConfirmationCount: 0 }))
      .toEqual({ kind: 'idle', reason: 'signalsIncomplete' })
  })

  it('keeps the phase reactive to the signals passed in', async () => {
    const sessions = ref<readonly unknown[] | null | undefined>([])
    const activeToolCount = ref<number | null | undefined>(0)
    const pendingConfirmationCount = ref<number | null | undefined>(0)
    const phase = useWorkPhase({ sessions, activeToolCount, pendingConfirmationCount })

    expect(phase.value.kind).toBe('idle')
    sessions.value = SESSION
    expect(phase.value.kind).toBe('explore')
    activeToolCount.value = 1
    expect(phase.value.kind).toBe('execute')
    pendingConfirmationCount.value = 2
    expect(phase.value.kind).toBe('review')
    sessions.value = []
    expect(phase.value.kind).toBe('idle')
  })
})
