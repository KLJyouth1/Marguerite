import { type Ref } from 'vue'
import { useRpcCall, type UseRpcCallOptions } from '@/composables/useRpc'

export interface UseRequestOptions {
  /** Run once on mount (after the connection is ready). Default true. */
  immediate?: boolean
  /** Route failures through the de-duped error->toast sink. Default true. */
  toastOnError?: boolean
  /** Human label for the error toast + the dedupe key. */
  errorLabel?: string
}

export interface UseRequestResult<T> {
  data: Ref<T | null>
  error: Ref<string | null>
  loading: Ref<boolean>
  /** Execute with the loading flag (skeleton). Resolves to the result or null on error. */
  execute: () => Promise<T | null>
  /** Re-execute silently (no skeleton flash) — for background refresh. */
  refresh: () => Promise<T | null>
}

/**
 * DEPRECATED: Use `useRpcCall` directly with `toastOnError` / `errorLabel` /
 * `immediate` options. Kept for backward compatibility; will be removed in a
 * future release.
 *
 * Thin wrapper over `useRpcCall` that enables `toastOnError` by default and
 * accepts a function-valued `params` argument for legacy callers. All
 * connection gating, coalescing, retry and admission logic is delegated to
 * the underlying `useRpcCall`.
 */
export function useRequest<T = unknown>(
  method: string,
  params?: Record<string, unknown> | (() => Record<string, unknown> | undefined),
  options: UseRequestOptions = {},
) {
  const { immediate = true, toastOnError = true, errorLabel } = options

  // Resolve function-valued params eagerly — no production caller uses this,
  // but the type is kept for backward compatibility.
  const resolved: Record<string, unknown> | undefined =
    typeof params === 'function' ? params() : params

  const rpcOptions: UseRpcCallOptions = {
    immediate,
    toastOnError,
    errorLabel,
  }

  return useRpcCall<T>(method, resolved, rpcOptions) as UseRequestResult<T>
}
