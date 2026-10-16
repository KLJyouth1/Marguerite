// Shared runtime-mode state (upgrade-masterplan §5).
//
// The Rust gateway broadcasts `mode.changed` on every `mode.switch`
// (core/src/events/bus.rs → envelope → gateway/events.rs maps it to
// `session.event.mode.changed`). Historically only ControlCenterView polled
// `mode.get` on a 12s timer, so a mode switch never propagated to the top bar,
// sidebar, or any other open view until the next poll tick.
//
// App.vue calls `initActiveMode()` once on mount and `disposeActiveMode()` on
// unmount; every view that needs to react to a mode change reads the same state.
//
// ---------------------------------------------------------------------------
// WHY THE STATE LIVES IN A PINIA STORE (`@/stores/activeMode`), NOT HERE
// ---------------------------------------------------------------------------
// A module-level `ref` does NOT work in this codebase. `webui/` is route-lazy
// loaded and bundles this module into multiple chunks — verified in
// `webui/dist`:
//   * `session.event.mode.changed` (this module's hydrate + subscribe) appears
//     ONLY in `index-*.js`;
//   * `MODE_KINDS`'s `omni_transcend` appears in BOTH `index-*.js` and
//     `ChatView-*.js`.
// So the module has two copies and a module-level `ref(null)` has two
// instances. `initActiveMode()` runs from App.vue (main chunk) and hydrates
// only the main chunk's copy; `ModeHintBar` (inside the lazy `ChatView` chunk)
// reads its own copy, stays `null` forever, and renders
// "Initializing · The gateway is loading its runtime configuration…"
// while the topbar — reading the hydrated copy — shows the correct state.
//
// This is the same class of bug that made gateway readiness look inconsistent.
// Pinia hangs its state off the app instance, so it is genuinely shared.
//
// `activeMode` is exposed as a **computed returning the store's ref**, which
// keeps the existing `activeMode.value` call sites working unchanged while the
// underlying state is the shared one.

import { computed } from 'vue'
import { useRpcStore } from '@/stores/rpc'
import {
  MODE_KINDS,
  normalizeModeKind,
  useActiveModeStore,
  USER_MODE_META,
  USER_MODE_INTENSITY,
  USER_MODE_WORKING,
  userModeKeyForKind,
  userModeInfoFor,
  type ModePayload,
  type UserModeInfo,
} from '@/stores/activeMode'

export { MODE_KINDS, normalizeModeKind, USER_MODE_META, USER_MODE_INTENSITY, USER_MODE_WORKING, userModeKeyForKind, userModeInfoFor }
export type { ModePayload, UserModeInfo }

/**
 * Shared, cross-chunk reactive ref of the active runtime mode.
 *
 * Writable on purpose: it delegates both directions to the Pinia store, so
 *   * readers in any chunk see the same value (the store is app-scoped), and
 *   * existing writers/tests keep using `activeMode.value = …` unchanged.
 * A plain module-level `ref` here would give each chunk its own instance — the
 * exact bug this refactor fixes (see the header comment).
 */
export const activeMode = computed({
  get: () => useActiveModeStore().mode,
  set: (value: ModePayload | null) => {
    useActiveModeStore().mode = value
  },
})

/** Who requested the last change (broadcast payload's `from`). */
export const lastChangedFrom = computed(() => useActiveModeStore().lastChangedFrom)

/** 当前 3×3 用户模式格位（来自 `mode.get.userMode`），未 hydrate 时为 null。 */
export const activeUserMode = computed<UserModeInfo | null>(
  () => useActiveModeStore().mode?.userMode ?? null,
)

/**
 * Last `mode.switch` failure, or `null`. Read by the top-bar badge to show an
 * honest error instead of silently keeping the old mode.
 */
export const switchError = computed(() => useActiveModeStore().switchError)

/** True while a `mode.switch` round-trip is in flight (drives the badge busy state). */
export const switching = computed(() => useActiveModeStore().switching)

let unsubscribe: (() => void) | null = null
/** Transport-state listener used only while waiting for the first `connected`. */
let stateUnsub: (() => void) | null = null
let initialised = false

function applyMode(to: string) {
  // The broadcast payload carries `to` = new mode kind. Reconcile the shared
  // ref so subscribers re-render without waiting for a poll. Also refresh the
  // 3×3 grid highlight via the canonical reverse map (Rust default_user_mode_for).
  const store = useActiveModeStore()
  const userMode = userModeInfoFor(userModeKeyForKind(to))
  store.mode = {
    ...(store.mode ?? {}),
    kind: to,
    mode: to,
    ...(userMode ? { userMode } : {}),
  }
}

/**
 * Water the shared mode once, then subscribe to live `mode.changed` frames.
 *
 * ⚠️ HYDRATION MUST WAIT FOR THE TRANSPORT (this was a real bug, do not "simplify" it):
 * `App.vue` calls this from `onMounted`, which runs while `rpc.state` is still
 * `'disconnected'` — the socket only starts connecting in `useRpcStore.init()`.
 * The original implementation fired `void rpc.call('mode.get')` immediately and
 * swallowed the rejection with `.catch(() => undefined)`. Measured consequence:
 * **`mode.get` never even left the browser** (0 frames sent), `activeMode` stayed
 * `null` forever, and the UI showed:
 *   - `.mode-badge` -> `mode-badge__dot--unknown` with an empty label
 *   - `.mode-hint-bar` -> "Initializing · The gateway is loading its runtime configuration…"
 * while the topbar (whose own RPC calls are readiness-gated) correctly said "Ready".
 *
 * So: hydrate once the socket reports `connected`, and if the call still fails,
 * say so instead of pretending the gateway has no mode.
 */
export function initActiveMode(): void {
  if (initialised) return
  initialised = true
  const rpc = useRpcStore()
  const store = useActiveModeStore()

  let hydrated = false

  const hydrate = () => {
    if (hydrated) return
    hydrated = true
    rpc
      .call<ModePayload>('mode.get', {})
      .then((md) => {
        if (md) store.mode = md
        else console.warn('[activeMode] mode.get returned an empty payload')
      })
      .catch((err: unknown) => {
        // No longer silent: a failed hydration leaves the badge/mode bar empty,
        // which previously had no visible cause at all.
        console.warn(
          '[activeMode] mode.get hydration failed:',
          err instanceof Error ? err.message : String(err),
        )
      })
  }

  // Already connected (this is the common case when the app is re-entered).
  if (rpc.isConnected) {
    hydrate()
  } else {
    // Otherwise wait for the transport, mirroring useRpcCall's readiness gating.
    const stop = rpc.on('_state', (s: string) => {
      if (s === 'connected') {
        stop()
        hydrate()
      }
    })
    stateUnsub = stop
  }

  // Core maps `mode.changed` → `session.event.mode.changed` (gateway/events.rs).
  unsubscribe = rpc.on('session.event.mode.changed', (frame: unknown) => {
    const to =
      (frame as { payload?: { to?: string } })?.payload?.to ??
      (frame as { to?: string })?.to
    if (typeof to === 'string' && to) {
      store.lastChangedFrom =
        (frame as { payload?: { from?: string } })?.payload?.from ?? null
      applyMode(to)
    }
  })
}

/** Tear down both the state listener and the frame subscription. */
export function disposeActiveMode(): void {
  unsubscribe?.()
  unsubscribe = null
  stateUnsub?.()
  stateUnsub = null
  initialised = false
}

/**
 * Switch the runtime mode — honestly.
 *
 * T05 (P1-3): the previous implementation was **optimistic** — it wrote the new
 * kind into the shared badge *before* awaiting `mode.switch` and swallowed every
 * failure with `catch { return null }`. So when the gateway rejected the switch
 * (the classic case being the underscore/hyphen name drift) the badge flipped to
 * a mode that was never active and no error was shown: the UI lied.
 *
 * Now the badge only changes once the gateway confirms. On failure the previous
 * mode is kept and `switchError` is exposed so the badge can say so.
 */
/**
 * Switch via the 3×3 grid: send `{ userMode }` so the gateway derives the
 * runtime execution kind through `from_user_mode`. Same honest semantics as
 * `switchActiveMode` — no optimistic write; a broadcast `mode.changed` refines it.
 */
export async function switchUserMode(key: string): Promise<ModePayload | null> {
  const rpc = useRpcStore()
  const store = useActiveModeStore()
  if (store.switching) return null
  store.switching = true
  store.switchError = null
  try {
    const r = await rpc.call<ModePayload>('mode.switch', { userMode: key })
    const userMode = userModeInfoFor(key)
    if (r) store.mode = r
    else if (userMode) store.mode = { ...(store.mode ?? {}), userMode }
    return r
  } catch (err) {
    store.switchError = err instanceof Error ? err.message : String(err)
    return null
  } finally {
    store.switching = false
  }
}

export async function switchActiveMode(kind: string): Promise<ModePayload | null> {
  const rpc = useRpcStore()
  const store = useActiveModeStore()
  // Ignore re-entrant clicks while a switch is already in flight.
  if (store.switching) return null
  store.switching = true
  store.switchError = null
  try {
    const r = await rpc.call<ModePayload>('mode.switch', { kind })
    // Only reflect the change AFTER the gateway confirms it. A broadcast
    // `mode.changed` will refine the payload; when `mode.switch` returns no
    // body we still fall back to the requested kind because the call succeeded.
    if (r) store.mode = r
    else applyMode(kind)
    return r
  } catch (err) {
    // Honest failure: keep the previous mode and surface the reason.
    store.switchError = err instanceof Error ? err.message : String(err)
    return null
  } finally {
    store.switching = false
  }
}

export function useActiveMode() {
  return {
    activeMode,
    activeUserMode,
    lastChangedFrom,
    switchError,
    switching,
    initActiveMode,
    disposeActiveMode,
    switchActiveMode,
    switchUserMode,
  }
}
