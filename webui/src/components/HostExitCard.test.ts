// @vitest-environment happy-dom
//
// Regression coverage for the "宿主进程已退出" (host exited) card staying on
// screen while a healthy Python sidecar is running. Root cause: the card keyed
// its visibility off `exited` alone — a diagnostic RECORD — and ignored
// `sidecar_present`, the LIVE fact. After a transient exit (e.g. the sidecar-
// reuse path logging `复用的 Python 侧车进程已消失`) a fresh sidecar came up
// healthy, but the on-disk record survived, so the card rendered forever.
//
// The card must therefore be driven by BOTH signals. These tests mount the real
// `HostExitCard` with the real provided state and drive the raw payload, so the
// visibility binary is exercised end-to-end (composable computed → card).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, type App } from 'vue'
import i18n from '@/i18n'

// The card reads its state through `useHostExitEvidence()`, whose only external
// dependency is the RPC store. Mock it so the composable builds without a live
// gateway; the tests drive the payload directly, so `call` is never awaited.
vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({
    isConnected: false,
    state: 'disconnected',
    call: vi.fn(async () => ({})),
  }),
}))

import HostExitCard from './HostExitCard.vue'
import {
  provideHostExitEvidence,
  type HostExitEvidencePayload,
  type HostExitEvidenceState,
} from '@/composables/useHostExitEvidence'

const mountedApps: Array<{ app: App; el: HTMLElement }> = []

interface MountedCard {
  readonly el: HTMLElement
  state(): HostExitEvidenceState
  setPayload(payload: HostExitEvidencePayload): Promise<void>
}

/** A fully-spelled payload; override only the fields a case cares about. */
function payload(over: Partial<HostExitEvidencePayload>): HostExitEvidencePayload {
  return {
    exited: false,
    source: null,
    evidence: null,
    sidecar_present: false,
    sidecar_uptime_ms: null,
    ...over,
  }
}

/**
 * Mount the real card inside a harness that owns the shared exit-evidence
 * instance exactly like `App.vue` does, and hand back a payload setter.
 */
async function mountCard(): Promise<MountedCard> {
  let state!: HostExitEvidenceState
  const Harness = defineComponent({
    setup() {
      // Mirrors `App.vue`: the always-mounted root owns the single instance and
      // publishes it through provide/inject for the card to consume.
      state = provideHostExitEvidence()
      return () => h(HostExitCard)
    },
  })

  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(Harness)
  app.use(i18n)
  app.mount(el)
  await nextTick()
  mountedApps.push({ app, el })

  return {
    el,
    state: () => state,
    setPayload: async (next) => {
      state.payload.value = next
      await nextTick()
    },
  }
}

beforeEach(() => {
  i18n.global.locale.value = 'en'
  document.body.innerHTML = ''
})

afterEach(() => {
  while (mountedApps.length) {
    const { app, el } = mountedApps.pop() as { app: App; el: HTMLElement }
    app.unmount()
    el.remove()
  }
})

describe('HostExitCard — visibility tracks the CURRENT sidecar, not a stale record', () => {
  it('stays hidden when an exit was recorded but a sidecar is present (the fix)', async () => {
    const card = await mountCard()
    await card.setPayload(
      payload({
        exited: true,
        source: 'record',
        evidence: { code: 0, uptime_ms: 3000, stderr_tail: '' },
        sidecar_present: true,
        sidecar_uptime_ms: 1200,
      }),
    )

    // Both raw signals are true — the card must still hide, because a healthy
    // sidecar is registered right now.
    expect(card.state().hasEvidence.value).toBe(true)
    expect(card.state().sidecarPresent.value).toBe(true)
    expect(card.el.querySelector('.host-exit')).toBeNull()
  })

  it('shows the card when an exit was recorded and NO sidecar is present', async () => {
    const card = await mountCard()
    await card.setPayload(
      payload({
        exited: true,
        source: 'record',
        evidence: { code: 0, uptime_ms: 3000, stderr_tail: '' },
        sidecar_present: false,
      }),
    )

    const root = card.el.querySelector('.host-exit')
    expect(root).not.toBeNull()
    // A real exit still surfaces the localized headline with structured facts.
    expect(card.state().sidecarPresent.value).toBe(false)
    expect(card.el.querySelector('.error-state__message')?.textContent).toContain('exit')
  })

  it('stays hidden when nothing has exited (normal, healthy state)', async () => {
    const card = await mountCard()
    await card.setPayload(
      payload({ exited: false, sidecar_present: true, sidecar_uptime_ms: 500 }),
    )

    expect(card.state().hasEvidence.value).toBe(false)
    expect(card.el.querySelector('.host-exit')).toBeNull()
  })
})
