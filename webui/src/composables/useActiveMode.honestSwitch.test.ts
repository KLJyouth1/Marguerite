// @vitest-environment happy-dom
//
// T05 P1-3 acceptance — the mode switch must be HONEST.
//
// Regression for the old behaviour: `switchActiveMode` wrote the new kind into
// the shared badge *before* awaiting `mode.switch` and swallowed failures, so a
// rejected switch flipped the badge to a mode that was never active and showed
// no error. Now the badge only changes after the gateway confirms; on failure it
// keeps the old mode and the top-bar badge shows an error.
import { createApp, nextTick } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import i18n from '@/i18n'

const rpcCall = vi.fn()
vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({ call: rpcCall }),
}))
vi.mock('vue-router', () => ({
  useRouter: () => ({ push: vi.fn() }),
}))

import ModeBadge from '@/components/ModeBadge.vue'
import { activeMode, switchActiveMode, switchError, switchUserMode, switching } from '@/composables/useActiveMode'
import { useActiveModeStore, userModeInfoFor, type UserModeInfo } from '@/stores/activeMode'

/** Resolve a canonical slot, failing loudly on a typo'd key (never `null!`). */
function slot(key: string): UserModeInfo {
  const info = userModeInfoFor(key)
  if (!info) throw new Error(`unknown user-mode slot: ${key}`)
  return info
}

// One pinia for the whole file: the module-level `activeMode`/`switchError`
// computed refs bind to the active pinia at first use, so swapping pinia between
// tests would leave them reading a stale store (see useActiveMode.shared.test.ts).
const pinia = createPinia()
setActivePinia(pinia)

function mountBadge() {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp(ModeBadge)
  app.use(pinia)
  app.use(i18n)
  app.mount(host)
  return { app, host }
}

beforeEach(() => {
  const store = useActiveModeStore()
  // The gateway payload carries BOTH the execution kind and the 3×3 slot; the
  // badge renders the slot, so the fixture must include it.
  store.mode = { kind: 'agent', mode: 'agent', userMode: slot('standard_plan') }
  store.switchError = null
  store.switching = false
  rpcCall.mockReset()
  i18n.global.locale.value = 'en'
})

describe('switchActiveMode — honest feedback', () => {
  it('keeps the previous badge and surfaces an error when the switch fails', async () => {
    rpcCall.mockRejectedValueOnce(new Error('INVALID_REQUEST: bad mode'))

    const result = await switchActiveMode('bad')
    expect(result).toBeNull()

    // The badge did NOT move to the rejected mode …
    expect(activeMode.value?.kind).toBe('agent')
    expect(switchError.value).toContain('bad mode')
    expect(switching.value).toBe(false)

    // … and the failure is visible in the top-bar badge.
    const { app, host } = mountBadge()
    await nextTick()
    expect(host.querySelector('.mode-badge__label')?.textContent).toContain('Standard · Plan')
    const error = host.querySelector('.mode-badge__error')
    expect(error).toBeTruthy()
    expect(error?.textContent).toContain('Could not switch')
    app.unmount()
  })

  it('keeps the previous 3x3 slot and surfaces an error when switchUserMode fails', async () => {
    rpcCall.mockRejectedValueOnce(new Error('INVALID_REQUEST: bad userMode'))

    const result = await switchUserMode('not_a_slot')
    expect(result).toBeNull()

    expect(activeMode.value?.userMode?.key).toBe('standard_plan')
    expect(switchError.value).toContain('bad userMode')
    expect(switching.value).toBe(false)

    const { app, host } = mountBadge()
    await nextTick()
    expect(host.querySelector('.mode-badge__label')?.textContent).toContain('Standard · Plan')
    expect(host.querySelector('.mode-badge__error')).toBeTruthy()
    app.unmount()
  })

  it('applies the new mode only after the gateway confirms', async () => {
    rpcCall.mockResolvedValueOnce({ kind: 'bloom', mode: 'bloom' })

    const result = await switchActiveMode('bloom')
    expect(result).toEqual({ kind: 'bloom', mode: 'bloom' })
    expect(activeMode.value?.kind).toBe('bloom')
    expect(switchError.value).toBeNull()
  })
})
