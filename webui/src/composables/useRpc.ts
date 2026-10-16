import { ref, onMounted, onUnmounted, watch } from 'vue'
import { useRpcStore } from '@/stores/rpc'
import type { RpcCallOptions, RpcClientError, RpcEventHandler } from '@/lib/rpc'
import { optionalSessionRpcAllowed } from '@/composables/chat/sessionBootstrapAdmission'
import { useErrorSink } from '@/composables/useErrorSink'

export interface UseRpcCallOptions {
  callOptions?: RpcCallOptions
  /** Run once on mount (after the connection is ready). Default true. */
  immediate?: boolean
  /** Route failures through the de-duped error->toast sink. Default false. */
  toastOnError?: boolean
  /** Human label for the error toast + the dedupe key. */
  errorLabel?: string
}

const coalescedAutoCalls = new WeakMap<object, Map<string, Promise<unknown>>>()

function autoCallKey(method: string, params?: Record<string, unknown>): string {
  return `${method}:${JSON.stringify(params || {})}`
}

/**
 * Composable for subscribing to RPC events within a Vue component lifecycle.
 * Automatically unsubscribes on component unmount.
 */
export function useRpcEvent(event: string, handler: RpcEventHandler) {
  const rpc = useRpcStore()
  let unsub: (() => void) | null = null

  onMounted(() => {
    unsub = rpc.on(event, handler)
  })

  onUnmounted(() => {
    unsub?.()
    unsub = null
  })

  return {
    unsub: () => {
      unsub?.()
      unsub = null
    },
  }
}

/**
 * Unified composable that calls an RPC method on mount and exposes reactive
 * state. Subsumes the former `useRequest` composable via the `toastOnError`,
 * `errorLabel`, `immediate` and `refresh` options.
 *
 * Features:
 * - Admission gating: optional RPCs wait for chat session bootstrap.
 * - Coalescing: identical mount-time calls share a single in-flight promise.
 * - Retry: transient (retryable) errors back off once before surfacing.
 * - Auto-toast: when `toastOnError` is set, failures route through
 *   `useErrorSink` instead of throwing.
 * - Silent refresh: `refresh()` re-fetches without a loading flash.
 * - `immediate: false` suppresses auto-mount execution.
 */
export function useRpcCall<T = unknown>(
  method: string,
  params?: Record<string, unknown>,
  options: UseRpcCallOptions = {},
) {
  const rpc = useRpcStore()
  const { immediate = true, toastOnError = false, errorLabel } = options

  const data = ref<T | null>(null)
  const loading = ref(false)
  const error = ref<string | null>(null)

  let stateUnsub: (() => void) | null = null
  let admissionUnsub: (() => void) | null = null
  let autoStarted = false

  function handleError(e: unknown): null {
    const msg = e instanceof Error ? e.message : String(e)
    error.value = msg
    if (toastOnError) {
      const { reportError } = useErrorSink()
      reportError(errorLabel ? `${errorLabel}: ${msg}` : msg, errorLabel || method)
    }
    return null
  }

  async function perform(coalesce: boolean, silent = false): Promise<T | null> {
    if (!silent) loading.value = true
    error.value = null
    try {
      if (!coalesce) {
        data.value = await rpc.call<T>(method, params, options.callOptions)
        return data.value
      }
      let calls = coalescedAutoCalls.get(rpc)
      if (!calls) {
        calls = new Map()
        coalescedAutoCalls.set(rpc, calls)
      }
      const key = autoCallKey(method, params)
      let pending = calls.get(key) as Promise<T> | undefined
      if (!pending) {
        pending = rpc.call<T>(method, params, options.callOptions)
        calls.set(key, pending)
        void pending.finally(() => {
          if (calls?.get(key) === pending) calls.delete(key)
        }).catch(() => {
          // Each consumer records the shared failure through its own await.
        })
      }
      data.value = await pending
      return data.value
    } catch (e: unknown) {
      const err = e as RpcClientError
      // Transient fault (Python sidecar timeout, gateway-marked retryable):
      // back off and retry once. Permanent errors (NOT_IMPLEMENTED / perms /
      // version conflict) and side-effecting writes are not retried here.
      if (err?.retryable) {
        const backoff = err.retry_after_ms ?? 800
        await new Promise((resolve) => setTimeout(resolve, backoff))
        try {
          data.value = await rpc.call<T>(method, params, options.callOptions)
          return data.value
        } catch (e2: unknown) {
          return handleError(e2)
        }
      }
      return handleError(e)
    } finally {
      if (!silent) loading.value = false
    }
  }

  function execute(): Promise<T | null> {
    return perform(false)
  }

  /** Re-execute silently (no loading flash) — for background refresh. */
  function refresh(): Promise<T | null> {
    return perform(false, true)
  }

  function maybeAutoExecute() {
    if (
      autoStarted
      || !rpc.isConnected
      || !optionalSessionRpcAllowed.value
    ) return
    autoStarted = true
    stateUnsub?.()
    stateUnsub = null
    admissionUnsub?.()
    admissionUnsub = null
    perform(true).catch(() => {
      /* error already captured in error ref */
    })
  }

  onMounted(() => {
    if (!immediate) return
    admissionUnsub = watch(optionalSessionRpcAllowed, maybeAutoExecute, {
      immediate: true,
    })
    stateUnsub = rpc.on('_state', (s: string) => {
      if (s === 'connected') {
        maybeAutoExecute()
      }
    })
    maybeAutoExecute()
  })

  onUnmounted(() => {
    stateUnsub?.()
    stateUnsub = null
    admissionUnsub?.()
    admissionUnsub = null
  })

  return { data, loading, error, execute, refresh }
}
