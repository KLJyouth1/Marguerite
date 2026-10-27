import { computed, ref, type Ref } from 'vue'

const ELEVATED_MODE_KEY = 'marguerite.elevatedMode'
const ELEVATED_MODE_VERSION_KEY = 'marguerite.elevatedMode.version'
const ELEVATED_MODE_STORAGE_VERSION = '2'

export interface SetElevatedModeOptions {
  persist?: boolean
  sync?: boolean
}

export interface UseChatElevatedModeOptions {
  sessionKey: Ref<string>
}

export function normalizeElevatedMode(mode: string): string {
  return mode === 'on' || mode === 'bypass' || mode === 'full' ? mode : ''
}

export function isApprovalBypassMode(mode: string): boolean {
  return mode === 'bypass' || mode === 'full'
}

export function useChatElevatedMode(options: UseChatElevatedModeOptions) {
  const elevatedMode = ref('')
  const globalElevatedMode = ref('')
  const elevatedUnavailable = ref(false)

  const effectiveElevatedMode = computed(() => {
    const mode = elevatedMode.value || globalElevatedMode.value
    return normalizeElevatedMode(mode)
  })

  function loadElevatedMode() {
    let mode = ''
    let version = ''
    try {
      mode = localStorage.getItem(ELEVATED_MODE_KEY) || ''
      version = localStorage.getItem(ELEVATED_MODE_VERSION_KEY) || ''
    } catch {}
    if (mode === 'full' && version !== ELEVATED_MODE_STORAGE_VERSION) {
      mode = 'bypass'
      try {
        localStorage.setItem(ELEVATED_MODE_KEY, mode)
        localStorage.setItem(ELEVATED_MODE_VERSION_KEY, ELEVATED_MODE_STORAGE_VERSION)
      } catch {}
    }
    setElevatedMode(mode, { persist: false, sync: true })
  }

  function setElevatedMode(mode: string, modeOptions: SetElevatedModeOptions = {}) {
    const normalized = normalizeElevatedMode(mode)
    elevatedMode.value = normalized
    if (modeOptions.persist !== false) {
      try {
        if (normalized) {
          localStorage.setItem(ELEVATED_MODE_KEY, normalized)
          localStorage.setItem(ELEVATED_MODE_VERSION_KEY, ELEVATED_MODE_STORAGE_VERSION)
        } else {
          localStorage.removeItem(ELEVATED_MODE_KEY)
          localStorage.removeItem(ELEVATED_MODE_VERSION_KEY)
        }
      } catch {}
    }
    if (modeOptions.sync) syncElevatedMode(normalized)
  }

  // Statuses that mean "this deployment does not offer the endpoint", as opposed
  // to "you are not allowed to use it" (403). All of them must land on the same
  // honest outcome: the mode is NOT active, and the user is told so.
  //
  // Why 404/405 are included: the gateway implements only `/ws`, `/api/health`
  // and the `/control/*` static routes (core/src/gateway/server.rs:149-156).
  // There is no `/api/elevated-mode` handler anywhere in the Rust tree, so the
  // request is answered by the SPA static fallback, which only serves GET/HEAD
  // and therefore replies 405. Treating that as a generic error and merely
  // `console.warn`-ing it left `elevatedMode` set in localStorage while the
  // gateway never enforced anything — the UI claimed approval-bypass was on
  // when it was not. That is a silent correctness failure, so it is now treated
  // exactly like "unsupported".
  const ELEVATED_UNSUPPORTED_STATUSES = new Set([403, 404, 405, 501])

  function markElevatedUnavailable(reason: string) {
    elevatedUnavailable.value = true
    try {
      localStorage.removeItem(ELEVATED_MODE_KEY)
      localStorage.removeItem(ELEVATED_MODE_VERSION_KEY)
    } catch {}
    elevatedMode.value = ''
    console.warn(`Approval-bypass mode is unavailable here (${reason}).`)
  }

  async function syncElevatedMode(mode: string) {
    if (!options.sessionKey.value || elevatedUnavailable.value) return
    try {
      // REST 与 WS 共用本地网关 token；不带 token 会被网关 401（Phase 0 收口）。
      const headers: Record<string, string> = { 'Content-Type': 'application/json' }
      try {
        const token = sessionStorage.getItem('marguerite.wsToken') || ''
        if (token) headers.Authorization = `Bearer ${token}`
      } catch { /* sessionStorage 不可用时按未授权处理，由后端 401 兜底 */ }
      const resp = await fetch('/api/elevated-mode', {
        method: 'POST',
        headers,
        body: JSON.stringify({ sessionKey: options.sessionKey.value, mode: mode || 'off' }),
      })
      if (ELEVATED_UNSUPPORTED_STATUSES.has(resp.status)) {
        markElevatedUnavailable(`HTTP ${resp.status}`)
        return
      }
      if (!resp.ok) throw new Error('HTTP ' + resp.status)
    } catch (err: unknown) {
      // Network failure: we cannot confirm the gateway accepted the mode, so do
      // not leave the UI claiming it did. Keep it retryable (no permanent
      // `elevatedUnavailable` latch) — only an explicit unsupported status is
      // treated as permanent.
      elevatedMode.value = ''
      console.warn('Failed to sync bypass mode:', err instanceof Error ? err.message : String(err))
    }
  }

  function setGlobalElevatedMode(mode: string) {
    globalElevatedMode.value = normalizeElevatedMode(mode)
  }

  return {
    elevatedMode,
    globalElevatedMode,
    effectiveElevatedMode,
    elevatedUnavailable,
    loadElevatedMode,
    setElevatedMode,
    syncElevatedMode,
    setGlobalElevatedMode,
    normalizeElevatedMode,
    isApprovalBypassMode,
  }
}
