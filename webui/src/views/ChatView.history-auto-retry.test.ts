import { describe, expect, it } from 'vitest'
import chatViewSource from './ChatView.vue?raw'

// A failed first history read must not pin the user to the "暂时无法恢复会话历史"
// banner plus a manual button. The reconnect path is covered by
// `handleConnectionState`; these guards lock the remaining window — the socket
// stays connected while the read itself fails — so the wiring cannot be
// silently dropped. Behavioural coverage of the same backoff style lives in
// `useSessions.retry.test.ts`.
describe('ChatView — history recovery self-heal wiring', () => {
  it('schedules a bounded retry while the failure banner is showing', () => {
    expect(chatViewSource).toContain('HISTORY_AUTO_RETRY_DELAYS_MS')
    expect(chatViewSource).toContain('cancelHistoryAutoRetry')
    expect(chatViewSource).toMatch(
      /state === 'history-error'[\s\S]{0,900}retryHistory\(\)/,
    )
  })

  it('cancels the pending retry when the view unmounts', () => {
    expect(chatViewSource).toMatch(
      /onUnmounted\(\(\) => \{[\s\S]*cancelHistoryAutoRetry\(\)/,
    )
  })
})