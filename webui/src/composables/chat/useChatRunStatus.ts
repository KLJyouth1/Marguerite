import i18n from '@/i18n'
import { runStatusLabelText as sessionsRunStatusLabelText } from '@/composables/useSessions'
import { normalizeRunStatus as normalizeRunStatusValue } from '@/utils/chat/runStatus'
import type {
  ChatRunStatus,
  ChatRunStatusSource,
  ChatRunStatusState,
} from '@/types/chat'

export const CHAT_RUN_STATUS_VALUES: ChatRunStatusState[] = [
  'queued',
  'running',
  'approval_pending',
  'interrupted',
  'failed',
  'timeout',
  'cancelled',
]

/** Normalize a raw session run status string into the canonical view state. */
export function normalizeRunStatus(status: string): ChatRunStatusState {
  // 单一事实源：utils/chat/runStatus.ts（F-06）。两个入口共用同一词表，
  // 杜绝「error → failed 只补一处」那类静默分叉。
  return normalizeRunStatusValue(status) as ChatRunStatusState
}

/** Localized display label for a normalized run status. */
export function runStatusLabelText(
  status: ChatRunStatusState,
  source?: ChatRunStatusSource | null,
): string {
  const t = i18n.global.t
  if (status === 'cancelled' || status === 'interrupted') {
    return sessionsRunStatusLabelText(status, source || undefined)
  }
  const labels: Record<string, string> = {
    queued: t('chat.status.queued'),
    running: t('chat.status.running'),
    approval_pending: t('chat.status.approvalPending'),
    interrupted: t('chat.status.interrupted'),
    failed: t('chat.status.failed'),
    timeout: t('chat.status.timeout'),
    cancelled: t('chat.status.cancelled'),
    idle: t('chat.status.idle'),
  }
  return labels[status] || t('chat.status.idle')
}

/** Derive the canonical run status (with label + active task) from a raw source. */
export function sessionRunStatus(
  source: ChatRunStatusSource | null | undefined,
): ChatRunStatus {
  const stateSource = source || {}
  const active = stateSource.active_task || stateSource.activeTask || null
  const last = stateSource.last_task || stateSource.lastTask || null
  const activeStatus = active ? normalizeRunStatus(active.status || '') : ''
  let status = normalizeRunStatus(
    stateSource.run_status || stateSource.runStatus || active?.status || last?.status || '',
  )
  if (
    active
    && (activeStatus === 'queued' || activeStatus === 'running' || activeStatus === 'approval_pending')
  ) status = activeStatus
  const task = active || last || null
  return { status, label: runStatusLabelText(status, stateSource), task }
}