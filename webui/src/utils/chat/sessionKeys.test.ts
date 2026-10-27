import { describe, expect, it } from 'vitest'
import { isSideChatSessionKey } from './sessionKeys'

describe('isSideChatSessionKey', () => {
  it('matches only generated side-chat session keys', () => {
    expect(isSideChatSessionKey('agent:main:webchat:side-abc123')).toBe(true)
    expect(isSideChatSessionKey('agent:researcher:webchat:side-x_y-1')).toBe(true)
    expect(isSideChatSessionKey('agent:main:webchat:default')).toBe(false)
    expect(isSideChatSessionKey('agent:main:webchat:side-')).toBe(false)
    expect(isSideChatSessionKey('agent:main:webchat:side-abc:extra')).toBe(false)
    expect(isSideChatSessionKey(null)).toBe(false)
  })
})
