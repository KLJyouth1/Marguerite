export const SESSION_NAVIGATION_DIAG_STORAGE_KEY = 'marguerite.chat.sessionNavigationDiag'
export const SESSION_NAVIGATION_DIAG_LIMIT = 200

export interface SessionNavigationDiagStorage {
  getItem: (key: string) => string | null
  setItem: (key: string, value: string) => void
  removeItem: (key: string) => void
}

export interface SessionNavigationDiagEntry {
  t: number
  iso: string
  source: string
  from?: string
  to?: string
  current?: string
  routeSession?: string
  requestSession?: string
  responseSession?: string
  reason?: string
}

export type SessionNavigationDiagData = Omit<SessionNavigationDiagEntry, 't' | 'iso' | 'source'>

let storageOverride: SessionNavigationDiagStorage | null = null
// 诊断数据的读取失败保持静默（它不承担任何用户可见功能），但必须留下一条
// 可查的线索。采样一次即可：读失败通常意味着整个存储不可用，逐次 warn 会刷屏。
let readFailureWarned = false

export function setSessionNavigationDiagStorageForTest(storage: SessionNavigationDiagStorage | null) {
  storageOverride = storage
}

/** 仅测试：让「已告警」的采样标记复位。 */
export function _resetSessionNavigationDiagWarningForTests(): void {
  readFailureWarned = false
}

function storage(): SessionNavigationDiagStorage | null {
  if (storageOverride) return storageOverride
  if (typeof window === 'undefined') return null
  try {
    return window.localStorage
  } catch {
    return null
  }
}

export function readSessionNavigationDiag(): SessionNavigationDiagEntry[] {
  const store = storage()
  if (!store) return []
  try {
    const raw = store.getItem(SESSION_NAVIGATION_DIAG_STORAGE_KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    return Array.isArray(parsed) ? parsed.filter(Boolean) : []
  } catch (err) {
    if (!readFailureWarned) {
      readFailureWarned = true
      console.warn('[sessionNavigationDiag] read failed:', err)
    }
    return []
  }
}

export function clearSessionNavigationDiag() {
  const store = storage()
  if (!store) return
  try {
    store.removeItem(SESSION_NAVIGATION_DIAG_STORAGE_KEY)
  } catch {
    // Ignore diagnostics storage failures.
  }
}

export function recordSessionNavigationDiag(
  source: string,
  data: SessionNavigationDiagData = {},
): SessionNavigationDiagEntry | null {
  const store = storage()
  if (!store) return null
  const now = Date.now()
  const entry: SessionNavigationDiagEntry = {
    t: now,
    iso: new Date(now).toISOString(),
    source,
    ...data,
  }
  try {
    const next = [entry, ...readSessionNavigationDiag()].slice(0, SESSION_NAVIGATION_DIAG_LIMIT)
    store.setItem(SESSION_NAVIGATION_DIAG_STORAGE_KEY, JSON.stringify(next))
    return entry
  } catch {
    return null
  }
}

export function installSessionNavigationDiagConsole() {
  if (typeof window === 'undefined') return
  window.MargueriteSessionDiag = {
    read: readSessionNavigationDiag,
    clear: clearSessionNavigationDiag,
  }
}

declare global {
  interface Window {
    MargueriteSessionDiag?: {
      read: () => SessionNavigationDiagEntry[]
      clear: () => void
    }
  }
}
