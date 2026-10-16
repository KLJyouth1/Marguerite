// F-06 护栏：run-status 词表只存一处，两个入口（侧栏 useSessions / 聊天 useChatRunStatus）
// 对同一原始值必须输出一致——0.2.9 的 error→failed 曾只补一处，本测试封死那类分叉。
import { describe, expect, it } from 'vitest'

import {
  CANONICAL_RUN_STATUS_VALUES,
  normalizeRunStatus,
} from '@/utils/chat/runStatus'
import { normalizeRunStatus as sessionsNormalize } from '@/composables/useSessions'
import { normalizeRunStatus as chatNormalize } from '@/composables/chat/useChatRunStatus'

const RAW_INPUTS = [
  '',
  'running',
  'queued',
  'idle',
  'error',
  'ERROR',
  'abandoned',
  'killed',
  'succeeded',
  'success',
  'complete',
  'timeout',
  'cancelled',
  'interrupted',
  'failed',
  'approval_pending',
  'something-else',
]

describe('runStatus vocabulary (F-06)', () => {
  it('maps the gateway vocabulary to canonical values', () => {
    expect(normalizeRunStatus('error')).toBe('failed')
    expect(normalizeRunStatus('abandoned')).toBe('interrupted')
    expect(normalizeRunStatus('killed')).toBe('cancelled')
    expect(normalizeRunStatus('succeeded')).toBe('idle')
    expect(normalizeRunStatus('complete')).toBe('idle')
    expect(normalizeRunStatus('unknown-thing')).toBe('idle')
    expect(normalizeRunStatus('approval_pending')).toBe('approval_pending')
  })

  it('the two entries agree on every raw input (no drift)', () => {
    for (const raw of RAW_INPUTS) {
      expect(
        sessionsNormalize(raw),
        `sessions vs chat disagree on "${raw}"`,
      ).toBe(chatNormalize(raw))
    }
  })

  it('both entries always return a member of the canonical vocabulary', () => {
    for (const raw of RAW_INPUTS) {
      expect(CANONICAL_RUN_STATUS_VALUES).toContain(sessionsNormalize(raw))
      expect(CANONICAL_RUN_STATUS_VALUES).toContain(chatNormalize(raw))
    }
  })
})