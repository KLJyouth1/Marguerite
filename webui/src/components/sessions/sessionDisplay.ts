import i18n from '@/i18n'
import type { IconName } from '@/utils/icons'
import type { SessionItem } from '@/composables/useSessions'
import { sessionRelativeTime } from '@/utils/messageTime'

export interface SessionStatusBadge {
  label: string
  cls: string
}

export interface SessionAgentIdentity {
  kind: 'unknown' | 'known' | 'raw' | 'deleted'
  value: string
}

/** Only call an agent deleted after one authoritative agents.list succeeded. */
export function sessionAgentIdentity(
  id: string | null | undefined,
  agentNames: ReadonlyMap<string, string>,
  agentsLoaded: boolean,
): SessionAgentIdentity {
  const normalized = String(id || '').trim()
  if (!normalized || normalized === 'unknown') return { kind: 'unknown', value: '' }
  const known = agentNames.get(normalized)
  if (known) return { kind: 'known', value: known }
  return agentsLoaded
    ? { kind: 'deleted', value: normalized }
    : { kind: 'raw', value: normalized }
}

export type SessionStatusBadgeClasses = Partial<Record<
  'needsInput' | 'running' | 'queued' | 'failed' | 'timeout' | 'interrupted' | 'cancelled',
  string
>>

const DEFAULT_STATUS_CLASSES: Required<SessionStatusBadgeClasses> = {
  needsInput: 'is-needs-input',
  running: 'is-running',
  queued: 'is-queued',
  failed: 'is-failed',
  timeout: 'is-failed',
  interrupted: 'is-queued',
  cancelled: 'is-off',
}

export function sessionSurfaceIcon(item: SessionItem): IconName {
  if (item.sessionKind === 'cron') return 'cron'
  if (item.sessionKind === 'channel') return 'channels'
  if (item.sessionKind === 'task' || item.surface === 'subagent') return 'agents'
  if (item.sessionKind === 'chat') return 'chat'
  return 'sessions'
}

export function sessionStatusBadge(
  item: SessionItem,
  needsInput = false,
  classes: SessionStatusBadgeClasses = {},
): SessionStatusBadge | null {
  const t = i18n.global.t
  if (needsInput) {
    return {
      label: t('sessions.status.needsInput'),
      cls: classes.needsInput || DEFAULT_STATUS_CLASSES.needsInput,
    }
  }
  const map: Record<string, { labelKey: string; classKey: keyof SessionStatusBadgeClasses }> = {
    running: { labelKey: 'sessions.status.running', classKey: 'running' },
    queued: { labelKey: 'sessions.status.queued', classKey: 'queued' },
    failed: { labelKey: 'sessions.status.failed', classKey: 'failed' },
    timeout: { labelKey: 'sessions.status.timeout', classKey: 'timeout' },
    interrupted: { labelKey: 'sessions.status.interrupted', classKey: 'interrupted' },
    cancelled: { labelKey: 'sessions.status.cancelled', classKey: 'cancelled' },
  }
  const entry = map[item.runStatus]
  if (!entry) return null
  const stoppedLabel =
    (item.runStatus === 'cancelled' || item.runStatus === 'interrupted') && item.runLabel
      ? item.runLabel
      : ''
  return {
    label: stoppedLabel || t(entry.labelKey),
    cls: classes[entry.classKey] || DEFAULT_STATUS_CLASSES[entry.classKey],
  }
}

/**
 * Shared relative-time formatter for every session ledger row (and any other
 * surface that lists sessions). Delegates to the single implementation in
 * `utils/messageTime.ts::sessionRelativeTime` (R65 收敛) so the ledger, the
 * sidebar and the inspect drawer cannot drift. Missing/invalid timestamps
 * render an em dash — the ledger's long-standing placeholder.
 */
export function formatRelativeTime(timestamp: number | null | undefined): string {
  if (!timestamp) return '—'
  const d = new Date(timestamp)
  if (isNaN(d.getTime())) return '—'
  return sessionRelativeTime(timestamp)
}

/** Back-compat alias; prefer {@link formatRelativeTime} for new call sites. */
export const sessionRelTime = formatRelativeTime

/**
 * Ledger title for a subagent row: "↳ Subagent · {parent title}" when the
 * parent title is known, otherwise a plain "↳ Subagent" so we never surface a
 * raw key. The arrow + label conveys lineage without rendering UUIDs.
 */
/**
 * 「fork 自」的父会话标题。父会话不在当前加载页里时返回空串 —— 调用方退回到
 * 不带名字的说明，而不是把原始会话键丢给用户看。
 */
export function forkSourceTitle(
  item: SessionItem,
  lookup: (key: string) => SessionItem | undefined,
): string {
  const key = String(item.parentSessionKey || '').trim()
  if (!key) return ''
  return String(lookup(key)?.title || '').trim()
}

export function subagentRowTitle(parentTitle: string | null | undefined): string {
  const t = i18n.global.t
  const parent = (parentTitle || '').trim()
  return parent
    ? `↳ ${t('sessions.ledger.subagentWithParent', { parent })}`
    : `↳ ${t('sessions.ledger.subagent')}`
}

export interface WorkspaceBadge {
  label: string
  cls: string
  icon: IconName
}

/** The resolved workspace root, as returned by `workspace.status`. */
export interface WorkspaceBadgeInput {
  root: string
  source: string
}

/**
 * Workspace root → status chip for the session inspect drawer.
 *
 * Only `bound` means file tools touch the user's real project; the other states
 * write into the app data dir (or a missing dir). Treating them as neutral
 * would hide the very condition that produced "why did this task fail" — hence
 * `alertTriangle` plus a distinct class for both non-bound states.
 */
export function workspaceBadge(status: WorkspaceBadgeInput): WorkspaceBadge {
  const t = i18n.global.t
  switch (status.source) {
    case 'bound':
      return { label: t('sessions.inspect.workspaceBound'), cls: 'is-bound', icon: 'folder' }
    case 'missing':
      return {
        label: t('sessions.inspect.workspaceMissing'),
        cls: 'is-missing',
        icon: 'alertTriangle',
      }
    default:
      return {
        label: t('sessions.inspect.workspaceUnbound'),
        cls: 'is-unbound',
        icon: 'alertTriangle',
      }
  }
}
