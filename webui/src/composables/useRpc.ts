import { ref, onMounted, onUnmounted, watch } from 'vue'
import { useRpcStore } from '@/stores/rpc'
import type { RpcCallOptions, RpcClientError, RpcEventHandler } from '@/lib/rpc'
import { optionalSessionRpcAllowed } from '@/composables/chat/sessionBootstrapAdmission'

interface UseRpcCallOptions {
  callOptions?: RpcCallOptions
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
 * Composable that calls an RPC method on mount and exposes reactive state.
 */
export function useRpcCall<T = unknown>(
  method: string,
  params?: Record<string, unknown>,
  options: UseRpcCallOptions = {},
) {
  const rpc = useRpcStore()
  const data = ref<T | null>(null)
  const loading = ref(false)
  const error = ref<string | null>(null)

  let stateUnsub: (() => void) | null = null
  let admissionUnsub: (() => void) | null = null
  let autoStarted = false

  async function perform(coalesce: boolean) {
    loading.value = true
    error.value = null
    try {
      if (!coalesce) {
        data.value = await rpc.call<T>(method, params, options.callOptions)
        return
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
    } catch (e: unknown) {
      const err = e as RpcClientError
      // 瞬时故障（Python 侧车超时等，网关标记 retryable）：退避重试一次。
      // 永久错误（NOT_IMPLEMENTED/权限/版本冲突）与写型副作用不在此重试。
      if (err?.retryable) {
        const backoff = err.retry_after_ms ?? 800
        await new Promise((resolve) => setTimeout(resolve, backoff))
        try {
          data.value = await rpc.call<T>(method, params, options.callOptions)
          return
        } catch (e2: unknown) {
          error.value = e2 instanceof Error ? e2.message : String(e2)
          throw e2
        }
      }
      error.value = e instanceof Error ? e.message : String(e)
      throw e
    } finally {
      loading.value = false
    }
  }

  function execute() {
    return perform(false)
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

  return { data, loading, error, execute }
}
