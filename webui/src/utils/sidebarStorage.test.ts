import { afterEach, describe, expect, it, vi } from 'vitest'

const notifyReadFailure = vi.hoisted(() => vi.fn())
vi.mock('@/utils/readFailureNotice', () => ({ notifyReadFailure }))

import { readStoredSessionKeys } from './sidebarStorage'

function stubStorage(raw: string | null) {
  vi.stubGlobal('localStorage', {
    getItem: () => raw,
    setItem: () => {},
    removeItem: () => {},
  })
}

afterEach(() => {
  notifyReadFailure.mockClear()
  vi.unstubAllGlobals()
})

describe('sidebarStorage', () => {
  it('reads and de-duplicates saved session keys', () => {
    stubStorage(JSON.stringify(['a', 'a', 'b', '', 3]))

    expect(readStoredSessionKeys('order-v1')).toEqual(['a', 'b'])
    expect(notifyReadFailure).not.toHaveBeenCalled()
  })

  it('损坏的本地副本会提示用户并退回默认顺序（App.vue 启动路径）', () => {
    stubStorage('{broken json')

    expect(readStoredSessionKeys('order-v1')).toEqual([])
    expect(notifyReadFailure).toHaveBeenCalledWith(
      'sidebar-session-keys:order-v1',
      'shared.storage.sessionOrderReadFailed',
    )
  })

  it('形状不对（不是数组）同样提示，而不是静默清空', () => {
    stubStorage(JSON.stringify({ keys: ['a'] }))

    expect(readStoredSessionKeys('pinned-v1')).toEqual([])
    expect(notifyReadFailure).toHaveBeenCalledWith(
      'sidebar-session-keys:pinned-v1',
      'shared.storage.sessionOrderReadFailed',
    )
  })
})
