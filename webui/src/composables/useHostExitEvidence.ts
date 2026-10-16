// useHostExitEvidence — the gateway's answer to "why did the host process die?"
//
// S5. When the WebSocket drops, the UI used to have exactly one thing to say:
// "Disconnected". That is true but useless — the user cannot tell a transient
// network blip from the sidecar having died at startup with a profile lock
// conflict. This composable pulls the structured evidence the Rust side now
// records (`core/src/pyengine/sidecar.rs::HostExitEvidence`) so the failure
// card can show exit code / uptime / stderr tail.
//
// ---------------------------------------------------------------------------
// WHY THIS FOLLOWS THE useGatewayReadiness SINGLETON PATTERN (read this first)
//
// `useGatewayReadiness.ts:27-64` documents a real regression: two call sites each
// built their own readiness state machine and the same live page rendered two
// different labels. The fix was a single runtime instance owned by the
// always-mounted `App.vue`, published through `provide`/`inject` with a STRING
// key — immune to bundler module duplication because injection walks the
// component instance tree.
//
// The same hazard applies here with one extra edge: a disconnection fans out to
// every mounted component, so a per-call-site instance would multiply the
// `host.exit_evidence` RPC on every disconnect AND let the header card and the
// error card disagree about whether evidence was fetched. Hence: ONE owner in
// `App.vue`, everyone else injects it. Same string-key discipline, same fallback
// for standalone mounts outside the app shell.
//
// Note the shared piece is the *evidence fetch*, not a second readiness state
// machine: `useGatewayReadiness()` remains the single source of transport
// truth. We read `rpcStore.state` directly rather than re-deriving readiness,
// so the two can never disagree by construction.
// ---------------------------------------------------------------------------

import {
  computed,
  inject,
  onUnmounted,
  provide,
  ref,
  watch,
  type ComputedRef,
  type Ref,
} from 'vue'
import { useRpcStore } from '@/stores/rpc'

/**
 * provide/inject key for the single shared exit-evidence instance.
 *
 * A plain string, deliberately matching `GATEWAY_READINESS_KEY`'s rationale:
 * provide/inject walks the component instance tree, which is orthogonal to how
 * the bundler chunks modules, so this cannot be duplicated per-chunk.
 */
export const HOST_EXIT_EVIDENCE_KEY = 'marguerite:host-exit-evidence'

/** Wire shape of the `host.exit_evidence` RPC payload. */
export interface HostExitEvidencePayload {
  /** A structured exit was recorded. `false` is a NORMAL state, not an error. */
  exited: boolean
  /** Where the evidence came from: live memory slot, or the on-disk record. */
  source: 'memory' | 'record' | null
  evidence: {
    /** Exit code; `null` when the process was killed by a signal (Unix). */
    code: number | null
    /** Milliseconds from successful spawn to detected exit. */
    uptime_ms: number
    /** stderr tail, ≤16KB. May contain paths/URLs — never render as headline. */
    stderr_tail: string
  } | null
  /** Whether a sidecar instance is currently registered in this process. */
  sidecar_present: boolean
  /** Age of the currently-registered sidecar, if any. */
  sidecar_uptime_ms: number | null
}

export interface HostExitEvidenceState {
  /** Latest payload, or `null` before the first successful fetch. */
  readonly payload: Ref<HostExitEvidencePayload | null>
  /** True while a fetch is in flight. */
  readonly loading: Ref<boolean>
  /** True when a structured exit was recorded. */
  readonly hasEvidence: ComputedRef<boolean>
  /** True when a sidecar instance is currently registered — the exit card must
   *  NOT show a stale "exited" record while a healthy sidecar is running. */
  readonly sidecarPresent: ComputedRef<boolean>
  /** Exit code (null when signal-killed or unknown). */
  readonly exitCode: ComputedRef<number | null>
  /** Milliseconds the host survived before exiting. */
  readonly uptimeMs: ComputedRef<number | null>
  /** stderr tail, or `''` when none was captured. */
  readonly stderrTail: ComputedRef<string>
  /** Context object shaped for `localizedChatErrorMessage('HOST_EXITED', …)`. */
  readonly errorContext: ComputedRef<{ exit_code: string; uptime_ms: string }>
  /** Re-fetch now (used by the card's retry button). */
  readonly refresh: () => Promise<void>
}

function createHostExitEvidence(): HostExitEvidenceState {
  const rpc = useRpcStore()
  const payload = ref<HostExitEvidencePayload | null>(null)
  const loading = ref(false)

  async function fetchEvidence(): Promise<void> {
    // A disconnected transport cannot answer. Leave the previous payload in
    // place: the whole point of this composable is to survive the disconnect
    // that triggered it, so clearing on failure would defeat the purpose.
    if (!rpc.isConnected) return
    loading.value = true
    try {
      const result = (await rpc.call('host.exit_evidence')) as unknown
      // Defensive shape check. The gateway is versioned independently of the
      // webui bundle, so an older core may answer without these fields; showing
      // a half-built card is worse than showing none.
      if (result && typeof result === 'object' && 'exited' in result) {
        payload.value = result as HostExitEvidencePayload
      }
    } catch {
      // Failure to read evidence must never surface as an error of its own:
      // the user already has a disconnection to deal with, and a second
      // "could not load evidence" toast would be pure noise.
    } finally {
      loading.value = false
    }
  }

  // Fetch on every transport transition. `disconnected` is included on purpose:
  // if the socket is already down the call is a no-op, but if the transport
  // flaps back up we re-read, which is exactly when fresh evidence may exist.
  const stopWatch = watch(() => rpc.state, () => { void fetchEvidence() }, { flush: 'post' })

  const hasEvidence = computed(() => payload.value?.exited === true)
  // `exited` is a diagnostic RECORD ("it exited once"); `sidecar_present` is the
  // LIVE fact ("a sidecar is registered right now"). The card keys its
  // visibility off both so a transient exit that has since been healed — the
  // sidecar-reuse path logs `复用的 Python 侧车进程已消失` even when a fresh
  // sidecar is already healthy — cannot leave a permanent false "exited" card.
  const sidecarPresent = computed(() => payload.value?.sidecar_present === true)
  const exitCode = computed<number | null>(() => {
    const code = payload.value?.evidence?.code
    return typeof code === 'number' ? code : null
  })
  const uptimeMs = computed<number | null>(() => {
    const ms = payload.value?.evidence?.uptime_ms
    return typeof ms === 'number' ? ms : null
  })
  const stderrTail = computed<string>(() => payload.value?.evidence?.stderr_tail ?? '')

  // Interpolation context for the localized headline. Both fields are
  // pre-formatted STRINGS because `contextField()` (errors.ts) only reads string
  // values — passing a raw number would silently interpolate as empty.
  const errorContext = computed(() => ({
    exit_code: exitCode.value === null ? '—' : String(exitCode.value),
    uptime_ms: formatDuration(uptimeMs.value),
  }))

  // App.vue owns this instance for the whole session; drop the watcher on
  // unmount so a standalone mount cannot leak one.
  onUnmounted(stopWatch)

  return {
    payload,
    loading,
    hasEvidence,
    sidecarPresent,
    exitCode,
    uptimeMs,
    stderrTail,
    errorContext,
    refresh: fetchEvidence,
  }
}

/**
 * Format a millisecond duration as `2h 14m 3s`, matching the uptime rendering
 * already used by `OverviewView.vue:311-318`. Returns `''` for unknown so
 * callers can omit the clause rather than print "0s".
 */
export function formatDuration(ms: number | null): string {
  if (ms === null || !Number.isFinite(ms) || ms < 0) return ''
  const totalSeconds = Math.floor(ms / 1000)
  const h = Math.floor(totalSeconds / 3600)
  const m = Math.floor((totalSeconds % 3600) / 60)
  const s = totalSeconds % 60
  if (h > 0) return `${h}h ${m}m ${s}s`
  if (m > 0) return `${m}m ${s}s`
  return `${s}s`
}

/**
 * Build THE shared exit-evidence instance and publish it to the component
 * subtree. Must be called exactly once, in the always-mounted root (`App.vue`).
 */
export function provideHostExitEvidence(): HostExitEvidenceState {
  const state = createHostExitEvidence()
  provide(HOST_EXIT_EVIDENCE_KEY, state)
  return state
}

/**
 * Consume the shared instance provided by `provideHostExitEvidence()`, falling
 * back to a local one only for standalone mounts outside the app shell (mirrors
 * `useGatewayReadiness()` so no call site ever crashes).
 */
export function useHostExitEvidence(): HostExitEvidenceState {
  const provided = inject<HostExitEvidenceState | null>(HOST_EXIT_EVIDENCE_KEY, null)
  return provided ?? createHostExitEvidence()
}