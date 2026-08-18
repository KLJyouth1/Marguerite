import i18n from '@/i18n'
import type {
  ChatMaintenanceEvent,
  ChatMessage,
  ChatRenderedMessage,
} from '@/types/chat'

/** Map a raw compaction status into the transcript maintenance event state. */
export function transcriptCompactionState(status: string): ChatMaintenanceEvent['state'] {
  if (status === 'skipped') return 'skipped'
  if (status === 'stale') return 'stale'
  if (status === 'cancelled') return 'cancelled'
  if (['failed', 'error', 'timed_out'].includes(status)) return 'failed'
  if (['completed', 'emergency_ephemeral'].includes(status)) return 'completed'
  return 'running'
}

/** Always surface the router strip for every rendered message. */
export function shouldRenderRouterStrip(_message: ChatRenderedMessage): boolean {
  return true
}

/** Escape a value for safe use inside a CSS attribute selector. */
export function cssEscapeAttr(value: string): string {
  if (typeof window !== 'undefined' && window.CSS && typeof window.CSS.escape === 'function') {
    return window.CSS.escape(value)
  }
  return String(value ?? '').replace(/[^a-zA-Z0-9_-]/g, '\\$&')
}

/** Human-friendly label for a plan action failure, mapping known causes. */
export function planActionErrorLabel(message: string): string {
  const t = i18n.global.t
  if (/Plan revision not found/i.test(message)) {
    return t('chat.plan.revisionMissing')
  }
  return t('chat.plan.actionFailed', { error: message })
}

/** True when a drag event carries file payload. */
export function dragEventHasFiles(e: DragEvent): boolean {
  const types = Array.from(e.dataTransfer?.types || [])
  return types.includes('Files')
}

/** Detect a synthetic subagent-completion system message. */
export function isSubagentCompletionMessage(
  role: string,
  text: string,
  options?: ChatMessage,
): boolean {
  if (role !== 'system' || !text) return false
  if (options?.provenanceSourceTool === 'subagent_completion') return true
  try {
    const parsed = JSON.parse(text)
    return parsed && parsed.type === 'subagent_completion'
  } catch { return false }
}