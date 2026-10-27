import { describe, expect, it, vi } from 'vitest'

import {
  completeComposerSafeSetup,
  composerRunModeSelectionAction,
  effectiveComposerRunMode,
  resolveComposerRunMode,
} from './composerRunMode'

function status(state: 'not_setup' | 'setting_up' | 'failed' | 'unavailable' | 'ready') {
  return {
    state,
    platform: 'win32',
    message: '',
    requiresAdmin: true,
  } as const
}

describe('resolveComposerRunMode', () => {
  it.each(['not_setup', 'setting_up', 'failed', 'unavailable'] as const)(
    'keeps the Safe choice visible (with a degradation) while setup is %s',
    (state) => {
      const resolved = resolveComposerRunMode('safe', status(state), null)
      expect(resolved.mode).toBe('safe')
      expect(resolved.degraded).not.toBeNull()
    },
  )

  it('maps each non-ready state to an explicit degradation reason', () => {
    expect(resolveComposerRunMode('safe', status('not_setup'), null).degraded)
      .toBe('setup_not_installed')
    expect(resolveComposerRunMode('safe', status('setting_up'), null).degraded)
      .toBe('setup_in_progress')
    expect(resolveComposerRunMode('safe', status('failed'), null).degraded)
      .toBe('setup_failed')
    expect(resolveComposerRunMode('safe', status('unavailable'), null).degraded)
      .toBe('setup_unavailable')
  })

  it('keeps Safe when setup is ready', () => {
    expect(resolveComposerRunMode('safe', status('ready'), null)).toEqual({
      mode: 'safe',
      degraded: null,
    })
  })

  it('does not invent failure while setup status is unknown', () => {
    expect(resolveComposerRunMode('safe', null, null)).toEqual({
      mode: 'safe',
      degraded: 'setup_unresolved',
    })
  })

  it('keeps the preference (safe) until the initial setup check resolves', () => {
    expect(effectiveComposerRunMode('safe', null, null, false)).toBe('safe')
  })

  it('preserves an active task lock even if setup status changes', () => {
    expect(effectiveComposerRunMode('full', status('not_setup'), 'safe')).toBe('safe')
  })

  it('never degrades Full Access', () => {
    expect(resolveComposerRunMode('full', status('unavailable'), null)).toEqual({
      mode: 'full',
      degraded: null,
    })
  })
})

describe('composerRunModeSelectionAction', () => {
  it('routes a repairable Safe selection into setup', () => {
    expect(composerRunModeSelectionAction('safe', status('not_setup'), true)).toBe('setup')
    expect(composerRunModeSelectionAction('safe', status('failed'), true)).toBe('setup')
  })

  it('persists Safe with an actionable notice when setup cannot run', () => {
    expect(composerRunModeSelectionAction('safe', status('unavailable'), false)).toBe('notice')
    expect(composerRunModeSelectionAction('safe', status('not_setup'), false)).toBe('notice')
  })

  it('persists instead of silently ignoring when the status is unknown', () => {
    expect(composerRunModeSelectionAction('safe', null, false, false)).toBe('persist')
    expect(composerRunModeSelectionAction('full', null, false, false)).toBe('persist')
  })

  it('persists while setup is already running without claiming it is unavailable', () => {
    expect(composerRunModeSelectionAction('safe', status('setting_up'), false)).toBe('persist')
  })

  it('persists immediately once setup is ready', () => {
    expect(composerRunModeSelectionAction('full', status('not_setup'), true)).toBe('persist')
    expect(composerRunModeSelectionAction('safe', status('ready'), false)).toBe('persist')
  })
})

describe('completeComposerSafeSetup', () => {
  it('persists Safe only after setup succeeds', async () => {
    const persist = vi.fn().mockResolvedValue(undefined)

    await expect(completeComposerSafeSetup(async () => false, persist)).resolves.toBe(false)
    expect(persist).not.toHaveBeenCalled()

    await expect(completeComposerSafeSetup(async () => true, persist)).resolves.toBe(true)
    expect(persist).toHaveBeenCalledOnce()
    expect(persist).toHaveBeenCalledWith('safe')
  })
})
