import { describe, expect, it } from 'vitest'

import type { ChatRenderedMessage } from '@/types/chat'
import { chatMessageKey } from './messageIdentity'

function message(patch: Partial<ChatRenderedMessage>): ChatRenderedMessage {
  return {
    id: 'assistant-0',
    role: 'assistant',
    displayRole: 'assistant',
    text: '',
    sourceIndex: 0,
    ...patch,
  } as ChatRenderedMessage
}

describe('chatMessageKey', () => {
  it('keeps the client id stable after the server binds a durable message id', () => {
    const optimistic = message({ clientId: 'local-1', role: 'user', displayRole: 'user' })
    const settled = message({
      clientId: 'local-1',
      messageId: 'srv-42',
      role: 'user',
      displayRole: 'user',
    })

    // historyMerge preserves clientId across the first authoritative history
    // replacement specifically so live surfaces are not remounted; keying on
    // messageId first would defeat that and remount the row mid-turn.
    expect(chatMessageKey(optimistic, 3)).toBe(chatMessageKey(settled, 3))
  })

  it('falls back to the durable id for history rows that never had a client id', () => {
    expect(chatMessageKey(message({ messageId: 'srv-7', restoredFromHistory: true }), 0)).toBe('srv-7')
  })

  it('still derives a unique key when no identity exists', () => {
    expect(chatMessageKey(message({ id: '', sourceIndex: 5 }), 9)).toBe('assistant-5')
    expect(chatMessageKey(message({ id: '', sourceIndex: undefined }), 9)).toBe('assistant-9')
  })
})
