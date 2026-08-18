import { describe, expect, it } from 'vitest'

import chatViewSource from './ChatView.vue?raw'
import controllerSource from '../composables/chat/useChatPromptCacheController.ts?raw'

// The keepalive orchestration moved out of ChatView.vue into
// `useChatPromptCacheController`. Template wiring is still asserted against the
// component; the stale-response and session-match guards are asserted against
// the controller that now owns them.
describe('ChatView prompt cache keepalive feedback', () => {
  it('keeps the composer/menu wiring in the component template', () => {
    expect(chatViewSource).toContain(
      ':prompt-cache-keepalive-status="promptCacheKeepaliveStatus"',
    )
    expect(chatViewSource).toContain(
      '@refresh-prompt-cache-keepalive="void refreshPromptCacheKeepaliveStatus()"',
    )
    expect(chatViewSource).toContain('@saved="onPromptCacheKeepaliveSaved"')
    expect(chatViewSource).toContain('useChatPromptCacheController(')
  })

  it('refreshes the menu state on demand and accepts the status returned by save', () => {
    expect(controllerSource).toContain("'sessions.promptCacheKeepalive.status'")
    // Stale-response guard: a status that arrives after the session switched away
    // must not overwrite the current session's state.
    expect(controllerSource).toContain(
      'if (sessionKey.value === key) promptCacheKeepaliveStatus.value = next',
    )
    expect(controllerSource).toContain(
      'if (update.sessionKey === sessionKey.value)',
    )
  })
})
