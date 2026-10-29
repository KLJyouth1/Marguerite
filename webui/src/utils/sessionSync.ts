export const LOCAL_SESSIONS_DELETED_EVENT = 'Marguerite:sessions-deleted'

export interface LocalSessionsDeletedDetail {
  keys: string[]
  source?: string
}

export function normalizeSessionKeys(keys: Iterable<string>): string[] {
  return [...new Set([...keys].map(key => key.trim()).filter(Boolean))]
}

export function dispatchLocalSessionsDeleted(keys: Iterable<string>, source?: string) {
  if (typeof window === 'undefined') return
  const normalized = normalizeSessionKeys(keys)
  if (normalized.length === 0) return
  window.dispatchEvent(new CustomEvent<LocalSessionsDeletedDetail>(
    LOCAL_SESSIONS_DELETED_EVENT,
    { detail: { keys: normalized, source } },
  ))
}

export function localSessionsDeletedDetail(event: Event): LocalSessionsDeletedDetail | null {
  if (!(event instanceof CustomEvent)) return null
  const detail = event.detail as Partial<LocalSessionsDeletedDetail> | null
  if (!detail || !Array.isArray(detail.keys)) return null
  const keys = normalizeSessionKeys(detail.keys)
  return keys.length > 0 ? { keys, source: detail.source } : null
}

/**
 * 「清空对话」（`sessions.reset`）与删除是两个不同的动作：会话壳还在，只是
 * 消息与模型上下文被清掉。因此它不能复用删除事件 —— 监听方要清空当前视图的
 * 消息，但绝不能把会话从列表里摘掉。
 */
export const LOCAL_SESSION_RESET_EVENT = 'Marguerite:sessions-reset'

export interface LocalSessionResetDetail {
  keys: string[]
  source?: string
}

export function dispatchLocalSessionReset(keys: Iterable<string>, source?: string) {
  if (typeof window === 'undefined') return
  const normalized = normalizeSessionKeys(keys)
  if (normalized.length === 0) return
  window.dispatchEvent(new CustomEvent<LocalSessionResetDetail>(
    LOCAL_SESSION_RESET_EVENT,
    { detail: { keys: normalized, source } },
  ))
}

export function localSessionResetDetail(event: Event): LocalSessionResetDetail | null {
  if (!(event instanceof CustomEvent)) return null
  const detail = event.detail as Partial<LocalSessionResetDetail> | null
  if (!detail || !Array.isArray(detail.keys)) return null
  const keys = normalizeSessionKeys(detail.keys)
  return keys.length > 0 ? { keys, source: detail.source } : null
}
