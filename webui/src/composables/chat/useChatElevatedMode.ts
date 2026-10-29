import { computed, ref, type Ref } from 'vue'

const ELEVATED_MODE_KEY = 'marguerite.elevatedMode'
const ELEVATED_MODE_VERSION_KEY = 'marguerite.elevatedMode.version'
const ELEVATED_MODE_STORAGE_VERSION = '2'
// P2-3：显式「关闭」哨兵。没有它时 `effectiveElevatedMode = local || global`，
// 用户把会话开关关掉会落回 `config.permissions.default_mode`（可能是 bypass），
// 表现为「开关关不掉」。存 'off' 表达「本会话明确关闭」，优先级高于全局默认。
const ELEVATED_OFF = 'off'

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

  const effectiveElevatedMode = computed(() => {
    if (elevatedMode.value === ELEVATED_OFF) return ''
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
    const normalized = mode === ELEVATED_OFF ? ELEVATED_OFF : normalizeElevatedMode(mode)
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
    if (modeOptions.sync) {
      syncElevatedMode(normalized === ELEVATED_OFF ? '' : normalized)
    }
  }

  // Statuses that mean "this deployment does not offer the endpoint", as opposed
  // to "you are not allowed to use it" (403). All of them must land on the same
  // honest outcome: the mode is NOT active, and the user is told so.
  //
  // 2026-10 起 Rust 网关已实现 POST /api/elevated-mode（core/src/gateway/elevated.rs）：
  // 正常部署不会再看到这些状态。保留它们只为「旧网关 / 非 Rust 宿主」的诚实降级。
  const ELEVATED_UNSUPPORTED_STATUSES = new Set([403, 404, 405, 501])

  // P2-3：不再写「永久闩锁」。
  //
  // 旧实现把 404/405/501 永久记成 `elevatedUnavailable`，此后连重试都不发 ——
  // 网关升级后用户必须刷新页面（甚至清 localStorage）才能恢复，而且界面从此
  // 不再提示任何原因。现在只回退**本次**状态：不写入本地存储、不设闩锁，
  // 下一次切换仍然会重新请求（后端已支持时自然成功）。
  function revertUnsupportedMode(reason: string) {
    try {
      localStorage.removeItem(ELEVATED_MODE_KEY)
      localStorage.removeItem(ELEVATED_MODE_VERSION_KEY)
    } catch {}
    elevatedMode.value = ''
    console.warn(`Approval-bypass mode is unavailable here (${reason}).`)
  }

  async function syncElevatedMode(mode: string) {
    if (!options.sessionKey.value) return
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
        revertUnsupportedMode(`HTTP ${resp.status}`)
        return
      }
      if (!resp.ok) throw new Error('HTTP ' + resp.status)
    } catch (err: unknown) {
      // Network failure: we cannot confirm the gateway accepted the mode, so do
      // not leave the UI claiming it did. Keep it retryable (no permanent
      // latch) — the next toggle asks the gateway again.
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
    loadElevatedMode,
    setElevatedMode,
    syncElevatedMode,
    setGlobalElevatedMode,
    normalizeElevatedMode,
    isApprovalBypassMode,
  }
}
