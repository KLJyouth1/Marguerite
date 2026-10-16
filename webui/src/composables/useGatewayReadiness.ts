// useGatewayReadiness — unified gateway readiness state.
//
// Problem: the topbar conn-pill shows green "CONNECTED" the moment the
// WebSocket is up, but the gateway might still need an API key, have a
// missing env var, or be mid-config. Users see green and think "I can chat
// now" — then the first turn fails with api_key_missing.
//
// This composable merges two independent signals into one readiness state:
//
//   1. WebSocket transport state (rpcStore.state):
//      'disconnected' | 'connecting' | 'connected'
//
//   2. Gateway business readiness (onboarding.status via useReadinessSummary):
//      needsAction (true when needsOnboarding or llmSource === 'missing_env')
//
// Unified readiness:
//   'disconnected'  — WS down; pill red
//   'connecting'     — WS connecting; pill yellow (pulsing)
//   'not_ready'      — WS up but gateway needs config; pill yellow (steady)
//   'ready'          — WS up and gateway configured; pill green
//
// The composable is a thin wrapper over useRpcCall('onboarding.status') +
// useReadinessSummary, so it reuses the same RPC path and cross-component
// invalidation pattern as SidebarSetupBanner and OnboardingOverlay.
//
// ---------------------------------------------------------------------------
// KNOWN ISSUE (recorded 2026-09-22) — re-audited + fixed 2026-09-27:
//
// This composable is called from two places — `App.vue` (topbar conn-pill) and
// `components/ModeHintBar.vue` (rendered by the lazily-mounted `ChatView`).
// Each call built its own state, and on a live page the two disagreed:
//
//   .conn-pill     -> "Ready"
//   .mode-hint-bar -> "Initializing · The gateway is loading its runtime
//                     configuration..."
//
// A module-level singleton was tried and **did not work**; a Pinia store was
// tried and flipped the topbar to a permanent `Connecting` (a user-visible
// regression), so it was reverted.
//
// Re-audit evidence (2026-09-27):
//   * Original premise "Vite duplicates this module into several chunks" no
//     longer holds. `webui/dist/assets/*.js.map` shows the module sources
//     `composables/useGatewayReadiness.ts`, `composables/useRpc.ts` and
//     `composables/setup/useReadinessSummary.ts` each appear in a SINGLE chunk
//     (`index-*.js`); `sessionBootstrapAdmission.ts` is its own shared chunk.
//     The lazily-loaded `ChatView-*.js` imports them from `index-*.js`.
//   * The real, still-present defect is that `useGatewayReadiness()` builds a
//     FRESH state machine per call site (its own `useRpcCall('onboarding.status')`,
//     its own `loading`/`status` refs, its own `invalidate` watcher). So App's
//     instance and ModeHintBar's instance are independent and can legitimately
//     render different labels in the same frame — e.g. App's fetch resolves to
//     `ready` while ModeHintBar's gated fetch is still in flight (`connecting`).
//     `useRpcCall` coalesces the transport call, but each instance still keeps
//     its own `loading`/`status`, so the divergence survives the coalescing.
//
// Fix: a single runtime instance owned by the always-mounted `App.vue`,
// published through `provide`/`inject` with a STRING key (a Symbol would also
// work, but the string key is deliberate: it is immune to any future per-chunk
// module duplication because provide/inject walk the component instance tree,
// which is orthogonal to the bundler). `useGatewayReadiness()` now injects that
// instance and only falls back to a local one when no provider exists (e.g. a
// standalone panel mounted outside the app shell).
// ---------------------------------------------------------------------------

import {
  computed,
  inject,
  onUnmounted,
  provide,
  watch,
  type ComputedRef,
  type Ref,
} from 'vue'
import { useRpcStore } from '@/stores/rpc'
import { useRpcCall } from '@/composables/useRpc'
import {
  onReadinessInvalidated,
  useReadinessSummary,
  type ReadinessStatus,
} from '@/composables/setup/useReadinessSummary'
import {
  optionalSessionRpcCallOptions,
} from '@/composables/chat/sessionBootstrapAdmission'

export type GatewayReadiness = 'disconnected' | 'connecting' | 'not_ready' | 'ready'

export interface GatewayReadinessState {
  /** Unified readiness — use for conn-pill CSS class and label. */
  readiness: ComputedRef<GatewayReadiness>
  /** Whether the gateway is fully ready for chat (WS up + config done). */
  isReady: ComputedRef<boolean>
  /** Raw onboarding.status data, if loaded. */
  status: Ref<ReadinessStatus | null>
  /** True while the readiness RPC is in flight. */
  loading: Ref<boolean>
  /** Re-fetch onboarding.status now (e.g. after a save). */
  refresh: () => Promise<void>
}

/**
 * provide/inject key for the single shared readiness instance. A plain string
 * constant is used on purpose: unlike a module-scope singleton it cannot be
 * duplicated by the bundler because injection walks the component instance
 * tree.
 */
export const GATEWAY_READINESS_KEY = 'marguerite:gateway-readiness'

function createGatewayReadiness(): GatewayReadinessState {
  const rpc = useRpcStore()

  const { data: status, loading, execute } = useRpcCall<ReadinessStatus>(
    'onboarding.status',
    undefined,
    { callOptions: optionalSessionRpcCallOptions },
  )
  const { needsAction } = useReadinessSummary(status)

  const readiness = computed<GatewayReadiness>(() => {
    if (rpc.state === 'disconnected') return 'disconnected'
    if (rpc.state === 'connecting') return 'connecting'
    // WS is connected — check business readiness.
    if (loading.value && !status.value) return 'connecting' // first fetch in flight
    if (needsAction.value) return 'not_ready'
    return 'ready'
  })

  const isReady = computed(() => readiness.value === 'ready')

  // Cross-component sync: when a save hot-applies config, invalidateReadiness()
  // fires; re-fetch so the pill transitions to green without a page reload.
  let readinessPending = false

  function flushReadiness() {
    if (readinessPending || loading.value) return
    readinessPending = true
    void execute().finally(() => { readinessPending = false })
  }

  const stopReadinessSync = onReadinessInvalidated(() => {
    readinessPending = true
    flushReadiness()
  })

  watch(
    [() => rpc.isConnected, loading],
    ([connected, busy]) => {
      if (connected && !busy) flushReadiness()
    },
    { flush: 'sync' },
  )

  // The composable is used in App.vue which lives for the whole session;
  // stop the listener on unmount to be safe.
  onUnmounted(stopReadinessSync)

  async function refresh(): Promise<void> {
    readinessPending = false
    await execute()
  }

  return {
    readiness,
    isReady,
    status,
    loading,
    refresh,
  }
}

/**
 * Build THE single readiness instance and publish it to the component subtree.
 * Must be called once, in the always-mounted root component (`App.vue`).
 */
export function provideGatewayReadiness(): GatewayReadinessState {
  const state = createGatewayReadiness()
  provide(GATEWAY_READINESS_KEY, state)
  return state
}

/**
 * Consume the shared readiness instance provided by `provideGatewayReadiness()`.
 * Falls back to a local instance only when no provider is present (standalone
 * mounts outside the app shell), so no call site ever crashes.
 */
export function useGatewayReadiness(): GatewayReadinessState {
  const provided = inject<GatewayReadinessState | null>(GATEWAY_READINESS_KEY, null)
  return provided ?? createGatewayReadiness()
}
