// @vitest-environment happy-dom

import { afterEach, describe, expect, it } from 'vitest'
import {
  dispatchLocalSessionReset,
  dispatchLocalSessionsDeleted,
  localSessionResetDetail,
  localSessionsDeletedDetail,
  LOCAL_SESSION_RESET_EVENT,
  LOCAL_SESSIONS_DELETED_EVENT,
  normalizeSessionKeys,
} from './sessionSync'

const cleanups: Array<() => void> = []

afterEach(() => {
  cleanups.splice(0).forEach(fn => fn())
})

function listen(name: string, handler: (event: Event) => void) {
  window.addEventListener(name, handler)
  cleanups.push(() => window.removeEventListener(name, handler))
}

describe('sessionSync', () => {
  it('normalizes, trims and dedupes keys', () => {
    expect(normalizeSessionKeys([' a ', 'a', '', '   ', 'b'])).toEqual(['a', 'b'])
  })

  it('keeps reset distinct from delete so listeners never unlist a cleared session', () => {
    let reset = null as ReturnType<typeof localSessionResetDetail>
    let deleted = null as ReturnType<typeof localSessionsDeletedDetail>
    listen(LOCAL_SESSION_RESET_EVENT, event => { reset = localSessionResetDetail(event) })
    listen(LOCAL_SESSIONS_DELETED_EVENT, event => { deleted = localSessionsDeletedDetail(event) })

    dispatchLocalSessionReset([' s1 ', 's1', 's2'], 'unit')

    expect(reset).toEqual({ keys: ['s1', 's2'], source: 'unit' })
    expect(deleted).toBeNull()
  })

  it('does not emit anything for an empty key set', () => {
    let reset = null as ReturnType<typeof localSessionResetDetail>
    listen(LOCAL_SESSION_RESET_EVENT, event => { reset = localSessionResetDetail(event) })

    dispatchLocalSessionReset(['', '  '])
    dispatchLocalSessionsDeleted(['', '  '])

    expect(reset).toBeNull()
  })

  it('ignores events whose payload is not a key list', () => {
    const notAKeyList = new CustomEvent(LOCAL_SESSION_RESET_EVENT, { detail: { keys: 's1' } })
    const noDetail = new CustomEvent(LOCAL_SESSION_RESET_EVENT)
    const notCustom = new Event(LOCAL_SESSION_RESET_EVENT)

    expect(localSessionResetDetail(notAKeyList)).toBeNull()
    expect(localSessionResetDetail(noDetail)).toBeNull()
    expect(localSessionResetDetail(notCustom)).toBeNull()
  })
})
