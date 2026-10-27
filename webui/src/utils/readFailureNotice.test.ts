import { afterEach, describe, expect, it, vi } from 'vitest'

const pushToast = vi.hoisted(() => vi.fn())
vi.mock('@/composables/useToasts', () => ({
  useToasts: () => ({ pushToast }),
}))
vi.mock('@/i18n', () => ({
  t: (key: string) => key,
}))

import {
  _resetReadFailureNoticeForTests,
  notifyReadFailure,
} from './readFailureNotice'

afterEach(() => {
  pushToast.mockClear()
  _resetReadFailureNoticeForTests()
})

describe('readFailureNotice', () => {
  it('shows one warn toast per source and never throws', () => {
    notifyReadFailure('terminal.status', 'terminal.statusReadFailed')
    notifyReadFailure('terminal.status', 'terminal.statusReadFailed')
    expect(pushToast).toHaveBeenCalledTimes(1)
    expect(pushToast).toHaveBeenCalledWith('terminal.statusReadFailed', { tone: 'warn' })

    notifyReadFailure('pending-restart', 'shared.storage.pendingRestartReadFailed')
    expect(pushToast).toHaveBeenCalledTimes(2)
  })

  it('swallows a toast-channel failure so reading never breaks', () => {
    pushToast.mockImplementationOnce(() => {
      throw new Error('toast unavailable')
    })
    expect(() => notifyReadFailure('a', 'k')).not.toThrow()
  })

  it('reset lets the same source notify again (test-only helper)', () => {
    notifyReadFailure('a', 'k')
    _resetReadFailureNoticeForTests()
    notifyReadFailure('a', 'k')
    expect(pushToast).toHaveBeenCalledTimes(2)
  })
})
