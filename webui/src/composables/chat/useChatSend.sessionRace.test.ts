import { describe, expect, it } from 'vitest'
import { decideSendResponseSession, handleSessionBusySend } from './useChatSend'

describe('handleSessionBusySend', () => {
  const busy = Object.assign(new Error('该会话正在生成回复'), { code: 'SESSION_BUSY' })

  it('queues the message instead of surfacing an error row', () => {
    let enqueued = 0
    let notified = 0
    const outcome = handleSessionBusySend(
      busy,
      () => {
        enqueued += 1
        return true
      },
      () => {
        notified += 1
      },
    )
    expect(outcome).toBe('deferred')
    expect(enqueued).toBe(1)
    expect(notified).toBe(0)
  })

  it('still reports deferred when the queue is full, but tells the user', () => {
    let notified = 0
    const outcome = handleSessionBusySend(
      busy,
      () => false,
      () => {
        notified += 1
      },
    )
    expect(outcome).toBe('deferred')
    expect(notified).toBe(1)
  })

  it('returns null for unrelated errors so the normal error path still runs', () => {
    let enqueued = 0
    const outcome = handleSessionBusySend(
      Object.assign(new Error('boom'), { code: 'SERVER' }),
      () => {
        enqueued += 1
        return true
      },
      () => {},
    )
    expect(outcome).toBeNull()
    expect(enqueued).toBe(0)
  })
})

describe('decideSendResponseSession', () => {
  it('ignores a late response after the user navigated to another session', () => {
    expect(decideSendResponseSession({
      requestSessionKey: 'agent:main:webchat:A',
      currentSessionKey: 'agent:main:webchat:B',
      responseSessionKey: 'agent:main:webchat:A',
    })).toEqual({
      action: 'ignore',
      reason: 'current_session_changed',
    })
  })

  it('persists server canonicalization while the request session is still current', () => {
    expect(decideSendResponseSession({
      requestSessionKey: 'legacy-A',
      currentSessionKey: 'legacy-A',
      responseSessionKey: 'agent:main:webchat:legacy-A',
    })).toEqual({
      action: 'persist',
      responseSessionKey: 'agent:main:webchat:legacy-A',
    })
  })

  it('ignores same-session responses', () => {
    expect(decideSendResponseSession({
      requestSessionKey: 'agent:main:webchat:A',
      currentSessionKey: 'agent:main:webchat:A',
      responseSessionKey: 'agent:main:webchat:A',
    })).toEqual({
      action: 'ignore',
      reason: 'same_session',
    })
  })

  it('ignores responses with no session key', () => {
    expect(decideSendResponseSession({
      requestSessionKey: 'agent:main:webchat:A',
      currentSessionKey: 'agent:main:webchat:A',
      responseSessionKey: undefined,
    })).toEqual({
      action: 'ignore',
      reason: 'missing_response_session',
    })
  })
})
