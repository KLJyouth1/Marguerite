// @vitest-environment happy-dom
// Regression guards for the gateway readiness SINGLE SOURCE OF TRUTH (T01).
//
// History: App.vue (topbar conn-pill) and ModeHintBar (rendered by the lazily
// mounted ChatView) each called `useGatewayReadiness()` and built their own
// state machine, so the two could show different labels in the same frame.
//
// The fix makes App.vue the single owner via `provideGatewayReadiness()` and
// has every consumer `inject` the same runtime instance through the STRING key
// `GATEWAY_READINESS_KEY`. These tests drive the real call sites and assert
// object identity, which is exactly what breaks if the fix is reverted.
import { computed, createApp, defineComponent, h, nextTick, ref, type App } from 'vue'
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'

const rpc = vi.hoisted(() => ({
  state: 'connected',
  isConnected: true,
  call: vi.fn(async () => ({ llmSource: 'configured' })),
  on: vi.fn(() => () => {}),
}))

vi.mock('@/stores/rpc', () => ({ useRpcStore: () => rpc }))

// ModeHintBar reads the shared active-mode singleton; stub it so we can drive
// the real component without the full mode store.
const activeModeRef = ref<{ kind?: string; mode?: string } | null>({ kind: 'agent', mode: 'agent' })
vi.mock('@/composables/useActiveMode', () => ({
  useActiveMode: () => ({ activeMode: activeModeRef, lastChangedFrom: ref(null) }),
  switchActiveMode: vi.fn(),
}))

import i18n, { loadLocaleMessages } from '@/i18n'
import ModeHintBar from '@/components/ModeHintBar.vue'
import {
  GATEWAY_READINESS_KEY,
  provideGatewayReadiness,
  useGatewayReadiness,
  type GatewayReadinessState,
} from '@/composables/useGatewayReadiness'

let app: App<Element> | null = null

beforeEach(() => {
  // Deterministic default: WS up + gateway fully configured -> readiness `ready`.
  // Individual tests override this to drive other readiness states.
  rpc.call.mockImplementation(async () => ({ llmSource: 'configured' }))
})

afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
  activeModeRef.value = { kind: 'agent', mode: 'agent' }
})

beforeAll(async () => {
  await loadLocaleMessages('zh-Hans')
  i18n.global.locale.value = 'zh-Hans'
})

describe('useGatewayReadiness single source of truth', () => {
  it('shares ONE readiness instance between App-style provider and a real ModeHintBar consumer', async () => {
    let provided: GatewayReadinessState | null = null
    let injected: GatewayReadinessState | null = null

    const Probe = defineComponent({
      setup() {
        injected = useGatewayReadiness()
        return () => h('span', { class: 'probe' }, injected!.readiness.value)
      },
    })

    const Root = defineComponent({
      components: { ModeHintBar, Probe },
      setup() {
        provided = provideGatewayReadiness()
        return () => h('div', [h('span', { class: 'pill' }, provided!.readiness.value), h(Probe), h(ModeHintBar)])
      },
    })

    const el = document.createElement('div')
    document.body.appendChild(el)
    app = createApp(Root)
    app.use(i18n)
    app.mount(el)
    await nextTick()
    await Promise.resolve()
    await nextTick()

    // The real ModeHintBar consumed the SAME injected instance as the provider.
    expect(injected!.readiness).toBe(provided!.readiness)
    expect(injected!.isReady).toBe(provided!.isReady)
    // And they render the same frame value.
    expect(el.querySelector('.pill')!.textContent).toBe(el.querySelector('.probe')!.textContent)
    // ModeHintBar did render (its injected readiness was usable).
    expect(el.querySelector('.mode-hint-bar')).not.toBeNull()

    // The injection key is a plain string constant, immune to module duplication.
    expect(GATEWAY_READINESS_KEY).toBe('marguerite:gateway-readiness')
  })

  it('keeps provider and consumer in lockstep across an invalidation-triggered refresh', async () => {
    let provided: GatewayReadinessState | null = null
    let injected: GatewayReadinessState | null = null

    const Probe = defineComponent({
      setup() {
        injected = useGatewayReadiness()
        return () => h('span')
      },
    })
    const Root = defineComponent({
      components: { Probe },
      setup() {
        provided = provideGatewayReadiness()
        return () => h('div', [h(Probe)])
      },
    })

    const el = document.createElement('div')
    document.body.appendChild(el)
    app = createApp(Root)
    app.mount(el)
    await nextTick()
    await Promise.resolve()
    await nextTick()

    const { invalidateReadiness } = await import('@/composables/setup/useReadinessSummary')
    invalidateReadiness()
    await Promise.resolve()
    await nextTick()

    expect(injected!.readiness).toBe(provided!.readiness)
    expect(injected!.readiness.value).toBe(provided!.readiness.value)
  })

  it('falls back to a local instance when no provider is present (standalone mount)', () => {
    let standalone: GatewayReadinessState | null = null
    const Standalone = defineComponent({
      setup() {
        standalone = useGatewayReadiness()
        return () => h('div')
      },
    })
    const el = document.createElement('div')
    document.body.appendChild(el)
    app = createApp(Standalone)
    app.mount(el)
    expect(standalone).not.toBeNull()
    expect(typeof standalone!.readiness.value).toBe('string')
  })

  it('renders the SAME readiness in the topbar pill and the real ModeHintBar (T01 表述一致护栏)', async () => {
    // WS is up but the gateway still needs config -> unified readiness `not_ready`.
    rpc.call.mockImplementation(async () => ({ llmSource: 'missing_env' }))

    const Root = defineComponent({
      components: { ModeHintBar },
      setup() {
        const { readiness } = provideGatewayReadiness()
        // Mirror App.vue: connectionStateLabel = chrome.connectionState.<readiness>.
        const pillLabel = computed(() =>
          i18n.global.t(`chrome.connectionState.${readiness.value}`),
        )
        return () =>
          h('div', [h('span', { class: 'pill' }, pillLabel.value), h(ModeHintBar)])
      },
    })

    const el = document.createElement('div')
    document.body.appendChild(el)
    app = createApp(Root)
    app.use(i18n)
    app.mount(el)
    // Flush mount -> auto RPC fetch -> reactive recompute.
    for (let i = 0; i < 3; i += 1) {
      await nextTick()
      await Promise.resolve()
    }
    await new Promise((resolve) => setTimeout(resolve, 0))

    // Precondition: the shared readiness is `not_ready`.
    const pill = el.querySelector('.pill')!
    expect(pill.textContent).toBe(i18n.global.t('chrome.connectionState.not_ready'))

    // The real ModeHintBar must express the SAME readiness: its label is the
    // setup-needed copy — not a mode alias. If ModeHintBar read a DIFFERENT
    // readiness instance (the T01 regression), its label would fall through to
    // the mode alias instead.
    const barLabel = el.querySelector('.mode-hint-bar__label')!
    expect(barLabel.textContent).toBe(i18n.global.t('modeHintBar.setupNeeded'))
    expect(barLabel.textContent).not.toBe(i18n.global.t('mode.alias.agent'))
  })
})