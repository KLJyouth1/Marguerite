import { ref, computed, getCurrentScope, onScopeDispose } from 'vue'
import i18n from '@/i18n'
import {
  waitForSessionRpcConnection,
} from '@/composables/chat/sessionBootstrapAdmission'
import { useRpcStore } from '@/stores/rpc'
import type { RpcCallOptions } from '@/lib/rpc'
import type {
  RawSessionItem,
  RawSessionListEntry,
  RawSessionTask,
  SessionEventPayload,
  SessionsListResponse,
  SessionsPatchResponse,
} from '@/types/rpc'
import type { ProjectWorkspaceItem } from '@/types/rpc'
import { normalizeTurnOutcome, turnOutcomePresentation } from '@/utils/chat/turnOutcome'
import { normalizeRunStatus as normalizeRunStatusValue } from '@/utils/chat/runStatus'
import { isSideChatSessionKey } from '@/utils/chat/sessionKeys'
import type { SessionTaskAttention } from '@/composables/useSessionTaskAttention'

export const SESSION_LIST_VIEW = 'session-list-v1'

/**
 * 会话列表初次读失败后的有界自愈退避（毫秒）。
 *
 * 冷启动/首次安装后的瞬时不可用（网关仍在预热、请求超时）不该把侧栏钉死在
 * 「无法加载任务」+ 手动重载上：用户截图里那一屏错误就是这么留下来的。重连路径
 * 由 `useSessionListSubscription` 的 `_state === 'connected'` 兜底，这里补齐
 * 「连接从未掉线、读请求自身失败」这一窗口。预算耗尽后仍保留手动重试按钮。
 */
export const SESSION_LIST_RETRY_DELAYS_MS = [1_000, 3_000, 8_000] as const

/**
 * How many *consecutive* successful `sessions.list` refreshes must omit a
 * locally-known session before it is dropped from the sidebar.
 *
 * A single short/degraded response (cold cache, a page that raced a refresh, a
 * reconnect that returned fewer rows) must never make rows the user is looking
 * at disappear. Only after this many consecutive misses do we treat the absence
 * as authoritative. Counters reset when the key reappears, when a read error
 * recovers (reconnect/outage boundary) and when the owning scope is disposed.
 */
export const SESSION_LIST_MISSING_TOLERANCE = 2

export type { RawSessionItem } from '@/types/rpc'

export interface SessionItem {
  key: string
  title: string
  subtitle: string
  groupLabel: string
  workspace?: string
  workspaceId?: string
  workspaceLabel?: string
  workspaceDisplayPath?: string
  /** 会话绑定的全部项目目录（P2-9 多根；[0] 与 `workspace` 一致，空/缺省 = 未绑定） */
  projectPaths?: string[]
  effectiveAgentId: string
  sessionKind: string
  surface: string
  conversationKind: string
  threadLabel: string
  channelContext: { name?: string; id?: string; accountId?: string; threadId?: string } | null
  status: string
  visualStatus: string
  runStatus: string
  runLabel: string
  messageCount: number | null
  updatedAt: number
  interactive: boolean
  /** Client-only draft that has not been materialized by its first send yet. */
  provisional?: boolean
  /** True when this session was forked from its parent's transcript. */
  forkedFromParent: boolean
  /** 会话级思考强度（'off'|'normal'|'deep'|'max'）；null/缺省 = 继承全局模型配置 */
  reasoningEffort?: string | null
  /** 会话级上下文长度；null/缺省 = 继承全局模型配置 */
  contextWindow?: number | null
  contractGaps: string[]
  raw: RawSessionItem
}

export interface SessionGroup {
  label: string
  items: SessionItem[]
  updatedAt: number
}

function hasOwn(obj: unknown, field: string): boolean {
  return !!obj && Object.prototype.hasOwnProperty.call(obj, field)
}

export function itemKey(item: unknown): string {
  if (typeof item === 'string') return item
  const row = item && typeof item === 'object' ? item as RawSessionItem : null
  return row?.key || row?.session || row?.sessionKey || ''
}

function textValue(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function numberValue(value: unknown): number | null {
  if (value == null || value === '') return null
  const n = Number(value)
  return Number.isFinite(n) ? n : null
}

export function normalizeRunStatus(status: string | undefined): string {
  // 单一事实源：utils/chat/runStatus.ts（F-06）。此前与
  // `chat/useChatRunStatus.normalizeRunStatus` 是复制粘贴关系，`error → failed`
  // / `approval_pending` 两处已出现分叉风险；现两入口共用同一词表。
  return normalizeRunStatusValue(status)
}

function stoppedTaskDurationSeconds(row: RawSessionItem | undefined): number | null {
  const lastTask = row?.last_task || row?.lastTask || null
  if (!lastTask) return null
  const startedAt = numberValue(lastTask.started_at) ?? numberValue(lastTask.startedAt)
  const finishedAt = numberValue(lastTask.finished_at) ?? numberValue(lastTask.finishedAt)
  if (startedAt == null || finishedAt == null || finishedAt < startedAt) return null
  return Math.max(1, Math.round((finishedAt - startedAt) / 1000))
}

export function runStatusLabelText(status: string, row?: RawSessionItem): string {
  const t = i18n.global.t
  if (status === 'cancelled' || status === 'interrupted') {
    const task = row?.last_task || row?.lastTask || null
    const presentation = normalizeTurnOutcome(task)
      ? turnOutcomePresentation(normalizeTurnOutcome(task))
      : status === 'cancelled' ? 'stopped' : 'interrupted'
    if (presentation === 'interrupted') return t('sessions.status.interrupted')
    const seconds = stoppedTaskDurationSeconds(row)
    if (seconds != null) return t('sessions.status.stoppedAfterSeconds', { seconds })
    return t('sessions.status.cancelled')
  }
  const keys: Record<string, string> = {
    queued: 'sessions.status.queued',
    running: 'sessions.status.running',
    interrupted: 'sessions.status.interrupted',
    failed: 'sessions.status.failed',
    timeout: 'sessions.status.timeout',
    cancelled: 'sessions.status.cancelled',
    idle: 'sessions.status.idle',
  }
  return t(keys[status] || 'sessions.status.idle')
}

function terminalRunStatus(row: RawSessionItem): string {
  const lastTask = row.last_task || row.lastTask || null
  const rawStatus = lastTask?.status || row.terminal_status || row.terminalStatus || ''
  const status = normalizeRunStatus(rawStatus)
  return ['failed', 'timeout', 'cancelled', 'interrupted'].includes(status) ? status : ''
}

export function sessionRunStatus(row: RawSessionItem): string {
  const active = row.active_task || row.activeTask || null
  const activeStatus = active ? normalizeRunStatus(active.status) : ''
  const terminal = terminalRunStatus(row)
  const rawStatus = row.runStatus || row.run_status || active?.status || terminal || ''
  const runStatus = normalizeRunStatus(rawStatus)
  if (active && (activeStatus === 'queued' || activeStatus === 'running')) return activeStatus
  if (terminal) return normalizeRunStatus(terminal)
  return runStatus
}

export function sessionVisualStatus(row: Pick<SessionItem, 'status' | 'runStatus'>): string {
  if (row.runStatus === 'failed' || row.runStatus === 'timeout') return row.runStatus
  if (row.runStatus === 'cancelled' || row.runStatus === 'interrupted') return 'killed'
  // approval_pending 是活跃态（等待用户批准），侧栏按运行中呈现；收口前它被
  // 折成 idle，与 useChatRunStatus 的词表分叉（F-06）。
  if (row.runStatus === 'approval_pending') return 'running'
  if (row.runStatus === 'queued' || row.runStatus === 'running' || row.runStatus === 'idle') return row.runStatus
  return String(row.status || 'unknown').toLowerCase()
}

function fallbackTitle(row: RawSessionItem): string {
  return (
    textValue(row.display_name) ||
    textValue(row.displayName) ||
    textValue(row.subject) ||
    textValue(row.derived_title) ||
    textValue(row.derivedTitle) ||
    ''
  )
}

function keyAgentId(key: string): string {
  const parts = key.split(':')
  return parts[0] === 'agent' && parts[1] ? parts[1] : ''
}

function sourceKind(row: RawSessionItem): string {
  return textValue(row.sourceKind) || textValue(row.source_kind)
}

function channelKind(row: RawSessionItem): string {
  return textValue(row.channelKind) || textValue(row.channel_kind) || textValue(row.lastChannel) || textValue(row.last_channel)
}

function deriveConversationKind(row: RawSessionItem, key: string): string {
  const explicit = textValue(row.conversationKind)
  if (explicit) return explicit
  const source = sourceKind(row).toLowerCase()
  const channel = channelKind(row).toLowerCase()
  const chatType = (textValue(row.chatType) || textValue(row.chat_type)).toLowerCase()
  if (key.includes(':webchat:') || chatType === 'webchat' || source === 'webui' || channel === 'webchat') return 'direct'
  if (key.startsWith('cron:') || key.includes(':cron:') || source === 'cron' || channel === 'cron') return 'unknown'
  if (source === 'channel' || (!!channel && !['cli', 'subagent', 'standalone'].includes(channel))) return chatType || 'group'
  if (key.includes(':subagent:') || source === 'subagent' || channel === 'subagent') return 'internal'
  if (key.includes(':cli:') || key.includes(':standalone:') || source === 'cli' || channel === 'cli') return 'internal'
  return 'unknown'
}

function deriveSessionKind(row: RawSessionItem, key: string): string {
  const explicit = textValue(row.sessionKind)
  if (explicit) return explicit
  const source = sourceKind(row).toLowerCase()
  const channel = channelKind(row).toLowerCase()
  if (key.includes(':webchat:') || source === 'webui' || channel === 'webchat') return 'chat'
  if (key.startsWith('cron:') || key.includes(':cron:') || source === 'cron' || channel === 'cron') return 'cron'
  if (source === 'channel' || (!!channel && !['cli', 'subagent', 'standalone'].includes(channel))) return 'channel'
  if (key.includes(':subagent:') || source === 'subagent' || channel === 'subagent') return 'task'
  if (key.includes(':cli:') || key.includes(':standalone:') || source === 'cli' || channel === 'cli') return 'chat'
  return 'unknown'
}

function deriveSurface(row: RawSessionItem, key: string, sessionKind: string): string {
  const explicit = textValue(row.surface)
  if (explicit) return explicit
  const channel = channelKind(row)
  const source = sourceKind(row)
  if (sessionKind === 'chat' && key.includes(':webchat:')) return 'webchat'
  if (sessionKind === 'cron') return channel || source || 'cron'
  if (sessionKind === 'channel') return channel || source || 'channel'
  if (key.includes(':subagent:')) return 'subagent'
  if (key.includes(':cli:') || key.includes(':standalone:')) return 'cli'
  return source || channel || 'unknown'
}

function deliveryContextValue(row: RawSessionItem, field: string): string {
  const ctx = row.deliveryContext || row.delivery_context || null
  return ctx && typeof ctx === 'object' ? textValue(ctx[field]) : ''
}

function deriveGroupLabel(row: RawSessionItem, key: string, sessionKind: string, agentId: string): string {
  const explicit = textValue(row.groupLabel)
  if (explicit) return explicit
  if (sessionKind === 'chat') return agentId || keyAgentId(key) || 'main'
  if (sessionKind === 'cron') {
    return (
      textValue(row.cron?.name) ||
      textValue(row.cron?.jobId) ||
      textValue(row.cron?.id) ||
      textValue(row.subject) ||
      i18n.global.t('sessions.kindLabel.cron')
    )
  }
  if (sessionKind === 'channel') {
    const channel = channelKind(row) || i18n.global.t('sessions.kindLabel.channel')
    const target = (
      textValue(row.lastTo) ||
      textValue(row.last_to) ||
      textValue(row.channelId) ||
      textValue(row.channel_id) ||
      deliveryContextValue(row, 'channel_id') ||
      deliveryContextValue(row, 'thread_id') ||
      textValue(row.groupId) ||
      textValue(row.group_id)
    )
    return target ? `${channel} / ${target}` : channel
  }
  return i18n.global.t('sessions.group.operational')
}

function fallbackSessionTitle(row: RawSessionItem, key: string, sessionKind: string): string {
  const t = i18n.global.t
  const semantic = fallbackTitle(row)
  if (semantic) return semantic
  if (sessionKind === 'chat') return t('sessions.fallbackTitle.chat')
  if (sessionKind === 'cron') return textValue(row.subject) || t('sessions.fallbackTitle.cron')
  if (sessionKind === 'channel') return textValue(row.subject) || t('sessions.fallbackTitle.channel')
  if (sessionKind === 'task') return t('sessions.fallbackTitle.task')
  return key || t('sessions.fallbackTitle.untitled')
}

function normalizeUpdatedAt(row: RawSessionItem, gaps: string[]): number {
  const activityValue = numberValue(row.lastActivityAt) ?? numberValue(row.last_activity_at)
  if (activityValue != null) return activityValue
  const contractValue = numberValue(row.updatedAt)
  if (contractValue != null) return contractValue
  gaps.push('updatedAt')
  return numberValue(row.updated_at) || 0
}

function normalizeMessageCount(row: RawSessionItem, gaps: string[]): number | null {
  const contractValue = numberValue(row.messageCount)
  if (contractValue != null) return contractValue
  gaps.push('messageCount')
  return numberValue(row.message_count) ?? numberValue(row.entry_count)
}

function normalizeEffectiveAgentId(row: RawSessionItem, gaps: string[], fallback = 'unknown'): string {
  const effective = textValue(row.effectiveAgentId)
  if (effective) return effective
  gaps.push('effectiveAgentId')
  return textValue(row.agentId) || textValue(row.agent_id) || fallback
}

function normalizeRequiredString(
  row: RawSessionItem,
  field: keyof RawSessionItem,
  fallback: string,
  gaps: string[],
): string {
  const value = textValue(row[field])
  if (value) return value
  gaps.push(String(field))
  return fallback
}

export function normalizeSessionItem(item: unknown): SessionItem | null {
  const raw: RawSessionItem = typeof item === 'string' ? { key: item } : (item || {}) as RawSessionItem
  const key = itemKey(item)
  if (!key || key === 'unknown') return null

  const gaps: string[] = []
  const derivedAgentId = textValue(raw.effectiveAgentId) || textValue(raw.agentId) || textValue(raw.agent_id) || keyAgentId(key) || 'unknown'
  const sessionKind = deriveSessionKind(raw, key)
  const conversationKind = deriveConversationKind(raw, key)
  const surface = deriveSurface(raw, key, sessionKind)
  const groupLabel = deriveGroupLabel(raw, key, sessionKind, derivedAgentId)
  // 会话归属项目：优先显式 workspace；否则回退到后端下发的 projectPath
  // （sessions.db 的 project_path），使带项目目录的会话能归到对应项目下，
  // 而不是全部堆在「最近」。
  const workspace = textValue(raw.workspace)
    || textValue(raw.projectPath)
    || textValue(raw.project_path)
  const workspaceId = textValue(raw.workspaceId) || textValue(raw.workspace_id)
  const workspaceLabel = textValue(raw.workspaceLabel)
  const workspaceDisplayPath = textValue(raw.workspaceDisplayPath)
  let title = normalizeRequiredString(raw, 'title', fallbackSessionTitle(raw, key, sessionKind), gaps)
  if (sessionKind === 'task' && /^you are a subagent\b/i.test(title)) title = i18n.global.t('sessions.fallbackTitle.task')
  const subtitle = hasOwn(raw, 'subtitle') ? textValue(raw.subtitle) : ''
  if (!hasOwn(raw, 'subtitle')) gaps.push('subtitle')
  const effectiveAgentId = normalizeEffectiveAgentId(raw, gaps, derivedAgentId)
  if (!hasOwn(raw, 'sessionKind')) gaps.push('sessionKind')
  if (!hasOwn(raw, 'surface')) gaps.push('surface')
  if (!hasOwn(raw, 'conversationKind')) gaps.push('conversationKind')
  const messageCount = normalizeMessageCount(raw, gaps)
  const updatedAt = normalizeUpdatedAt(raw, gaps)
  const status = textValue(raw.status) || 'unknown'
  if (!hasOwn(raw, 'runStatus')) gaps.push('runStatus')
  const runStatus = sessionRunStatus(raw)
  const thread = raw.thread && typeof raw.thread === 'object' ? raw.thread : null
  const threadLabel = thread?.kind && thread?.id ? `${thread.kind} ${thread.id}` : ''
  const channelContext = raw.channelContext && typeof raw.channelContext === 'object' ? raw.channelContext : null
  // 会话级 thinking 强度 / 上下文长度：双 key 归一（camelCase 优先），null/缺省 = 继承全局模型配置
  const reasoningEffortRaw = textValue(raw.reasoningEffort) || textValue(raw.reasoning_effort)
  const reasoningEffort = reasoningEffortRaw ? reasoningEffortRaw : null
  const contextWindow = numberValue(raw.contextWindow) ?? numberValue(raw.context_window)
  // P2-9 多根：后端 projectPaths 优先（去重保序）；缺省回退单值 workspace
  const projectPaths = Array.from(new Set(
    (Array.isArray(raw.projectPaths) ? raw.projectPaths : [])
      .map(v => textValue(v))
      .filter(Boolean),
  ))
  if (projectPaths.length === 0 && workspace) projectPaths.push(workspace)

  return {
    key,
    title,
    subtitle,
    groupLabel,
    workspace: workspace || undefined,
    workspaceId: workspaceId || undefined,
    workspaceLabel: workspaceLabel || undefined,
    workspaceDisplayPath: workspaceDisplayPath || undefined,
    projectPaths: projectPaths.length > 0 ? projectPaths : undefined,
    effectiveAgentId,
    sessionKind,
    surface,
    conversationKind,
    threadLabel,
    channelContext,
    status,
    visualStatus: sessionVisualStatus({ status, runStatus }),
    runStatus,
    runLabel: runStatusLabelText(runStatus, raw),
    messageCount,
    updatedAt,
    interactive: raw.interactive === true,
    forkedFromParent: raw.forkedFromParent === true || raw.forked_from_parent === true,
    reasoningEffort,
    contextWindow,
    contractGaps: Array.from(new Set(gaps)),
    raw,
  }
}

function parentField(item: SessionItem): Record<string, unknown> | null {
  const parent = item.raw.parent
  return parent && typeof parent === 'object' ? parent as Record<string, unknown> : null
}

/** Parent session key carried by the session contract. */
export function sessionParentKey(item: SessionItem): string {
  const key = parentField(item)?.key
  return typeof key === 'string' ? key.trim() : ''
}

/** Only spawned subagent tasks form a visual hierarchy in the sidebar. */
function isSubagentSession(item: SessionItem): boolean {
  return item.sessionKind === 'task' || item.surface === 'subagent'
}

/** Spawn depth from the session contract; 0 when the row is not a subagent. */
export function sessionSpawnDepth(item: SessionItem): number {
  const raw = parentField(item)?.spawnDepth
  const depth = Number(raw)
  return Number.isFinite(depth) && depth > 0 ? depth : 0
}

/** Parent title carried by the contract, used to label subagent lineage. */
export function sessionParentTitle(item: SessionItem): string {
  const title = parentField(item)?.title
  return typeof title === 'string' ? title.trim() : ''
}

export interface SessionLedgerEntry {
  item: SessionItem
  depth: number
  /** Resolved parent title for subagent rows; empty for root rows. */
  parentTitle: string
}

/**
 * Flatten sessions into ledger order: rows keep their recency sort, and
 * subagent rows indent directly under their parent when the parent is in
 * the same list. Orphan subagents stay at the root level.
 */
export function arrangeSessionLedger(items: SessionItem[]): SessionLedgerEntry[] {
  const byKey = new Map(items.map(item => [item.key, item]))
  const children = new Map<string, SessionItem[]>()
  const roots: SessionItem[] = []
  for (const item of items) {
    const parentKey = sessionParentKey(item)
    if (isSubagentSession(item) && parentKey && parentKey !== item.key && byKey.has(parentKey)) {
      const list = children.get(parentKey) || []
      list.push(item)
      children.set(parentKey, list)
    } else {
      roots.push(item)
    }
  }
  const entries: SessionLedgerEntry[] = []
  const visit = (item: SessionItem, depth: number, parentTitle: string) => {
    entries.push({ item, depth, parentTitle: depth > 0 ? parentTitle : '' })
    for (const child of children.get(item.key) || []) {
      visit(child, Math.min(depth + 1, 3), item.title)
    }
  }
  for (const root of roots) {
    // An orphan subagent (parent not in the visible list) still indents when
    // the contract marks it spawned; its lineage label falls back to the
    // parent title carried on the contract.
    const orphanDepth = isSubagentSession(root) && sessionSpawnDepth(root) > 0 ? 1 : 0
    visit(root, orphanDepth, orphanDepth > 0 ? sessionParentTitle(root) : '')
  }
  return entries
}

/** Family buckets rendered as collapsible sidebar sections, in display order. */
export type SidebarSectionFamily = 'chats' | 'channels' | 'automations'

/** A single rendered sidebar row, flattened with its indent depth. */
export interface SidebarSectionRow {
  rowKind: 'session' | 'workspace' | 'workspace-empty'
  key: string
  title: string
  effectiveAgentId: string
  /** Resolved display name when known; empty when the caller must resolve it. */
  agentName: string
  sessionKind: string
  /** Indent level mirroring `arrangeSessionLedger` (0 = root, subagents > 0). */
  depth: number
  runStatus: string
  runLabel: string
  taskAttention: SessionTaskAttention
  updatedAt: number
  hasContractGaps: boolean
  workspace?: string
  workspaceId?: string
  workspaceLabel?: string
  workspaceDisplayPath?: string
  workspaceTaskCount?: number
  workspacePinned?: boolean
  workspaceAvailable?: boolean
  provisional?: boolean
  /** Local sidebar preference; never changes the underlying session contract. */
  pinned?: boolean
}

/** One collapsible family section with its recency-ordered rows. */
export interface SidebarSection {
  family: SidebarSectionFamily
  label: string
  rows: SidebarSectionRow[]
}

/**
 * Decide which sidebar family a session belongs to, mirroring App.vue's
 * `sourceFamilyForSession`. Unlike the old flat list, subagent rows are no
 * longer dropped: a 'task'/'subagent' session is folded into the Chats family
 * (nested under its parent by `arrangeSessionLedger`), so this returns 'chats'
 * for it. Returns null for sessions that have no sidebar home (e.g. cli/tui/mcp
 * chat surfaces, or unknown kinds).
 */
function sidebarFamilyForSession(item: SessionItem): SidebarSectionFamily | null {
  if (item.sessionKind === 'chat') {
    if (['cli', 'tui', 'mcp', 'subagent'].includes(item.surface)) return null
    return 'chats'
  }
  if (item.sessionKind === 'task' || item.surface === 'subagent') return 'chats'
  if (item.sessionKind === 'channel') return 'channels'
  if (item.sessionKind === 'cron') return 'automations'
  return null
}

const SIDEBAR_SECTION_LABEL_KEYS: Record<SidebarSectionFamily, string> = {
  chats: 'sessions.filter.chats',
  channels: 'sessions.filter.channels',
  automations: 'sessions.filter.automations',
}

const SIDEBAR_SECTION_ORDER: SidebarSectionFamily[] = ['chats', 'channels', 'automations']

/**
 * Arrange sessions into the ordered sidebar families (Chats, Channels,
 * Automations). Each family is recency-sorted; the Chats family additionally
 * runs through `arrangeSessionLedger`, so subagent rows indent under their
 * parent chat (and orphan subagents indent at depth 1 via the contract's
 * spawn-depth fallback). The helper is pure: it returns all three families
 * (callers drop empty ones at render time).
 */
export function arrangeSidebarSections(
  items: SessionItem[],
  projects?: readonly ProjectWorkspaceItem[],
  sessionOrder: readonly string[] = [],
  pinnedSessionKeys: readonly string[] = [],
): SidebarSection[] {
  const buckets: Record<SidebarSectionFamily, SessionItem[]> = {
    chats: [],
    channels: [],
    automations: [],
  }
  for (const item of items) {
    if (!item.key || item.key === 'unknown') continue
    const family = sidebarFamilyForSession(item)
    if (!family) continue
    buckets[family].push(item)
  }

  const byRecency = (a: SessionItem, b: SessionItem) => (b.updatedAt || 0) - (a.updatedAt || 0)
  const orderIndex = new Map(sessionOrder.map((key, index) => [key, index]))
  const pinnedKeys = new Set(pinnedSessionKeys)
  const bySidebarOrder = (a: SessionItem, b: SessionItem) => {
    const pinnedDifference = Number(pinnedKeys.has(b.key)) - Number(pinnedKeys.has(a.key))
    if (pinnedDifference !== 0) return pinnedDifference
    const aIndex = orderIndex.get(a.key)
    const bIndex = orderIndex.get(b.key)
    if (aIndex !== undefined && bIndex !== undefined) return aIndex - bIndex
    // Sessions created after the last manual reorder remain at the top.
    if (aIndex !== undefined) return 1
    if (bIndex !== undefined) return -1
    return byRecency(a, b)
  }
  const toRow = (item: SessionItem, depth: number): SidebarSectionRow => ({
    rowKind: 'session',
    key: item.key,
    title: item.title,
    effectiveAgentId: item.effectiveAgentId,
    agentName: '',
    sessionKind: item.sessionKind,
    depth,
    runStatus: item.runStatus,
    runLabel: item.runLabel,
    taskAttention: ['queued', 'running'].includes(item.runStatus) ? 'running' : 'none',
    updatedAt: item.updatedAt || 0,
    hasContractGaps: item.contractGaps.length > 0,
    workspace: item.workspace,
    workspaceId: item.workspaceId,
    workspaceLabel: item.workspaceLabel,
    workspaceDisplayPath: item.workspaceDisplayPath,
    provisional: item.provisional,
    pinned: pinnedKeys.has(item.key),
  })

  type WorkspaceBucket = {
    title: string
    displayPath?: string
    rows: SidebarSectionRow[]
    updatedAt: number
  }
  type WorkspaceTopLevel =
    | { kind: 'workspace'; workspace: string; index: number }
    | { kind: 'row'; row: SidebarSectionRow; index: number }

  const arrangeWorkspaceRows = (entries: SessionLedgerEntry[]): SidebarSectionRow[] => {
    const buckets = new Map<string, WorkspaceBucket>()
    const topLevel: WorkspaceTopLevel[] = []
    let index = 0

    for (const entry of entries) {
      const row = toRow(entry.item, entry.depth)
      const workspace = entry.item.workspace
      if (!workspace) {
        topLevel.push({ kind: 'row', row, index: index++ })
        continue
      }

      let bucket = buckets.get(workspace)
      if (!bucket) {
        bucket = {
          title: entry.item.workspaceLabel || workspace,
          displayPath: entry.item.workspaceDisplayPath || workspace,
          rows: [],
          updatedAt: 0,
        }
        buckets.set(workspace, bucket)
        topLevel.push({ kind: 'workspace', workspace, index: index++ })
      }
      bucket.rows.push({ ...row, depth: Math.min(row.depth + 1, 4) })
      bucket.updatedAt = Math.max(bucket.updatedAt, row.updatedAt || 0)
    }

    return [...topLevel]
      // The ledger is already recency- or manually ordered. Preserve its first
      // occurrence for both ordinary rows and workspace groups.
      .sort((a, b) => a.index - b.index)
      .flatMap(entry => {
        if (entry.kind === 'row') return [entry.row]
        const bucket = buckets.get(entry.workspace)
        if (!bucket) return []
        const header: SidebarSectionRow = {
          rowKind: 'workspace',
          key: `workspace:${entry.workspace}`,
          title: bucket.title,
          effectiveAgentId: '',
          agentName: '',
          sessionKind: 'workspace',
          depth: 0,
          runStatus: 'idle',
          runLabel: '',
          taskAttention: 'none',
          updatedAt: bucket.updatedAt,
          hasContractGaps: false,
          workspace: entry.workspace,
          workspaceLabel: bucket.title,
          workspaceDisplayPath: bucket.displayPath || entry.workspace,
          workspaceTaskCount: bucket.rows.filter(row => row.depth === 1).length,
          workspacePinned: false,
          workspaceAvailable: true,
        }
        return [header, ...bucket.rows]
      })
  }

  /** 归一化路径用于比较（小写 + 统一分隔符）。 */
  function normalizeProjectPath(path: string): string {
    return path.replace(/\\/g, '/').toLowerCase()
  }

  const arrangePersistedProjectRows = (
    entries: SessionLedgerEntry[],
    persistedProjects: readonly ProjectWorkspaceItem[],
  ): SidebarSectionRow[] => {
    const rows: SidebarSectionRow[] = []
    // 已被某个项目收纳的会话键：不再出现在「最近」的扁平区。
    const consumed = new Set<string>()
    for (const project of persistedProjects) {
      const projectEntries = entries.filter(entry => {
        if (consumed.has(entry.item.key)) return false
        // 按 workspaceId（会话契约）或项目根目录路径（sessions.db project_path）
        // 判定归属；路径比较做大小写/分隔符归一，兼容 Windows 反斜杠差异。
        const byId = entry.item.workspaceId === project.id
        const entryWorkspace = entry.item.workspace
        const byPath = typeof entryWorkspace === 'string' && entryWorkspace.length > 0
          && normalizeProjectPath(entryWorkspace) === normalizeProjectPath(project.path)
        if (byId || byPath) consumed.add(entry.item.key)
        return byId || byPath
      })
      rows.push({
        rowKind: 'workspace',
        key: `workspace:${project.id}`,
        title: project.name,
        effectiveAgentId: '',
        agentName: '',
        sessionKind: 'workspace',
        depth: 0,
        runStatus: 'idle',
        runLabel: '',
        taskAttention: 'none',
        updatedAt: 0,
        hasContractGaps: false,
        workspace: project.path,
        workspaceId: project.id,
        workspaceLabel: project.name,
        workspaceDisplayPath: project.path,
        workspaceTaskCount: project.taskCount
          + projectEntries.filter(entry => entry.item.provisional).length,
        workspacePinned: project.pinned,
        workspaceAvailable: project.available,
      })
      if (projectEntries.length === 0) {
        rows.push({
          rowKind: 'workspace-empty',
          key: `workspace:${project.id}:empty`,
          title: i18n.global.t('workspaces.noTasks'),
          effectiveAgentId: '',
          agentName: '',
          sessionKind: 'workspace-empty',
          depth: 1,
          runStatus: 'idle',
          runLabel: '',
          taskAttention: 'none',
          updatedAt: 0,
          hasContractGaps: false,
          workspace: project.path,
          workspaceId: project.id,
          workspaceLabel: project.name,
          workspaceDisplayPath: project.path,
        })
      } else {
        rows.push(...projectEntries.map(entry => ({
          ...toRow(entry.item, Math.min(entry.depth + 1, 4)),
          workspaceId: project.id,
        })))
      }
    }
    rows.push(
      ...entries
        .filter(entry => !consumed.has(entry.item.key))
        .map(entry => toRow(entry.item, entry.depth)),
    )
    // Sessions belonging to a removed project remain durable but hidden until
    // that project is restored to the canonical project list.
    return rows
  }

  return SIDEBAR_SECTION_ORDER.map(family => {
    const bucket = buckets[family]
    let rows: SidebarSectionRow[]
    if (family === 'chats') {
      // Recency-sort first so the ledger's root ordering follows recency, then
      // flatten parent → child so subagents indent directly beneath their chat.
      const ledger = arrangeSessionLedger([...bucket].sort(bySidebarOrder))
      rows = projects === undefined
        ? arrangeWorkspaceRows(ledger)
        : arrangePersistedProjectRows(ledger, projects)
    } else {
      rows = [...bucket].sort(byRecency).map(item => toRow(item, 0))
    }
    return { family, label: i18n.global.t(SIDEBAR_SECTION_LABEL_KEYS[family]), rows }
  })
}

export function sessionMatches(item: SessionItem, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true
  return [
    item.title,
    item.subtitle,
    item.groupLabel,
    item.effectiveAgentId,
    item.sessionKind,
    item.surface,
    item.conversationKind,
    item.status,
    item.runStatus,
    item.raw.model,
    item.key,
  ].some(value => String(value || '').toLowerCase().includes(q))
}

export function groupSessions(items: SessionItem[]): SessionGroup[] {
  const groups = new Map<string, SessionGroup>()
  for (const item of items) {
    const label = item.groupLabel || i18n.global.t('sessions.group.contractGaps')
    const existing = groups.get(label)
    if (existing) {
      existing.items.push(item)
      existing.updatedAt = Math.max(existing.updatedAt, item.updatedAt || 0)
    } else {
      groups.set(label, {
        label,
        items: [item],
        updatedAt: item.updatedAt || 0,
      })
    }
  }
  return Array.from(groups.values())
    .map(group => ({
      ...group,
      items: [...group.items].sort((a, b) => (b.updatedAt || 0) - (a.updatedAt || 0)),
    }))
    .sort((a, b) => (b.updatedAt || 0) - (a.updatedAt || 0))
}

export function useSessions(readCallOptions?: RpcCallOptions) {
  const rpc = useRpcStore()
  const sessionsList = ref<RawSessionListEntry[]>([])
  const sessionListError = ref(false)
  const isLoading = ref(false)

  const allSessions = computed((): SessionItem[] =>
    sessionsList.value
      .map(normalizeSessionItem)
      .filter((item): item is SessionItem => !!item && !isSideChatSessionKey(item.key))
      .sort((a, b) => (b.updatedAt || 0) - (a.updatedAt || 0))
  )

  const groupedSessions = computed((): SessionGroup[] => groupSessions(allSessions.value))

  // 有界自愈预算（每次失败一个预算，成功后复位；耗尽则只剩手动重试按钮）。
  let sessionListRetryTimer: ReturnType<typeof setTimeout> | null = null
  let sessionListRetryAttempt = 0

  // session key → consecutive successful refreshes that omitted the row. See
  // SESSION_LIST_MISSING_TOLERANCE: a row is only dropped once this reaches the
  // tolerance. Instance-scoped; cleared on scope dispose and on the first
  // successful read after an error (the reconnect/outage boundary).
  const missingRefreshCounts = new Map<string, number>()

  function cancelPendingSessionListRetry() {
    if (!sessionListRetryTimer) return
    clearTimeout(sessionListRetryTimer)
    sessionListRetryTimer = null
  }

  function scheduleSessionListRetry() {
    if (
      sessionListRetryTimer
      || sessionListRetryAttempt >= SESSION_LIST_RETRY_DELAYS_MS.length
    ) return
    const delay = SESSION_LIST_RETRY_DELAYS_MS[sessionListRetryAttempt]
    sessionListRetryAttempt += 1
    sessionListRetryTimer = setTimeout(() => {
      sessionListRetryTimer = null
      void loadSessions()
    }, delay)
  }

  // 作用域销毁（视图卸载）即停掉待执行的重试；无作用域时（纯函数式调用）不注册。
  if (getCurrentScope()) {
    onScopeDispose(() => {
      cancelPendingSessionListRetry()
      sessionListRetryAttempt = 0
      missingRefreshCounts.clear()
    })
  }

  /**
   * Merge a `sessions.list` response into the current list instead of replacing
   * it wholesale.
   *
   * Rule (see SESSION_LIST_MISSING_TOLERANCE): rows present in the response are
   * authoritative and replace their local entry. A local row absent from this
   * response is *kept* — its miss counter increments and it is only removed once
   * it has been missing for SESSION_LIST_MISSING_TOLERANCE consecutive
   * successful refreshes. This stops a shorter/degraded response (or a reconnect
   * that returns fewer rows) from making the sidebar "lose" conversations that
   * the user can still see; the polling refresh re-adds them if they truly exist.
   */
  function mergeSessionsList(returned: RawSessionListEntry[]) {
    const seen = new Set<string>()
    const merged: RawSessionListEntry[] = []
    for (const entry of returned) {
      const key = itemKey(entry)
      if (!key) continue
      seen.add(key)
      missingRefreshCounts.delete(key)
      merged.push(entry)
    }
    for (const local of sessionsList.value) {
      const key = itemKey(local)
      if (!key || seen.has(key)) continue
      const misses = (missingRefreshCounts.get(key) || 0) + 1
      if (misses >= SESSION_LIST_MISSING_TOLERANCE) {
        missingRefreshCounts.delete(key)
        continue
      }
      missingRefreshCounts.set(key, misses)
      merged.push(local)
    }
    sessionsList.value = merged
  }

  async function loadSessions() {
    cancelPendingSessionListRetry()
    // A read that follows a failure marks the reconnect/outage boundary: reset
    // the miss counters so rows are not evicted from pre-outage counts.
    const recoveredFromError = sessionListError.value
    isLoading.value = true
    sessionListError.value = false
    try {
      await waitForSessionRpcConnection(rpc, readCallOptions)
      const params = { limit: 200, view: SESSION_LIST_VIEW }
      const data = readCallOptions
        ? await rpc.call<SessionsListResponse>(
            'sessions.list',
            params,
            readCallOptions,
          )
        : await rpc.call<SessionsListResponse>('sessions.list', params)
      const raw = (data?.sessions || data?.keys || []).filter(s => !!itemKey(s))
      if (recoveredFromError) missingRefreshCounts.clear()
      mergeSessionsList(raw)
      sessionListRetryAttempt = 0
    } catch (err: unknown) {
      console.error('[useSessions] sessions.list error:', err instanceof Error ? err.message : err)
      sessionListError.value = true
      scheduleSessionListRetry()
    } finally {
      isLoading.value = false
    }
  }

  /**
   * Apply a real-time run-status update for a single session in response to a
   * gateway `sessions.changed` / `task.*` event. Patches the underlying
   * `sessionsList` entry in place so the derived `allSessions` (and therefore
   * the sidebar) reflect running/idle immediately — no need to wait for the
   * 30s polling refresh. Returns true when a matching row was updated.
   */
  function applySessionRunStatus(payload: SessionEventPayload, event = 'sessions.changed'): boolean {
    const raw = (payload && typeof payload === 'object' ? payload : {}) as Record<string, unknown>
    const key = String(raw.session_key || raw.sessionKey || raw.key || raw.session || '')
    if (!key) return false
    const idx = sessionsList.value.findIndex(entry => itemKey(entry) === key)
    if (idx < 0) {
      // The row is not in the local list yet (short/degraded list response, a
      // brand-new session not polled in). Previously a silent `return false`,
      // which made "live pill never updated" impossible to diagnose — log it.
      console.warn('[useSessions] applySessionRunStatus: no local row for session', key, `event=${event}`)
      return false
    }

    const entry = sessionsList.value[idx]
    const prev = (typeof entry === 'string' ? { key: entry } : entry) as RawSessionItem
    const next: RawSessionItem = { ...prev }

    const explicit = normalizeRunStatus(String(
      raw.run_status
        || raw.runStatus
        || raw.status
        || (raw.active_task as Record<string, unknown> | undefined)?.status
        || (raw.last_task as Record<string, unknown> | undefined)?.status
        || '',
    ))
    const eventStatus: Record<string, string> = {
      'task.queued': 'queued',
      'task.running': 'running',
      'task.failed': 'failed',
      'task.cancelled': 'cancelled',
      'task.timeout': 'timeout',
      'task.succeeded': 'idle',
      'task.abandoned': 'interrupted',
    }
    const status = explicit || eventStatus[event] || ''
    if (!status) return false

    next.run_status = status
    if (raw.active_task) next.active_task = raw.active_task as RawSessionTask | null
    if (raw.last_task) next.last_task = raw.last_task as RawSessionTask | null
    const ts = numberValue(raw.emitted_at)
      ?? numberValue(raw.started_at)
      ?? numberValue(raw.timestamp)
      ?? Date.now()
    next.updatedAt = ts
    next.lastActivityAt = ts

    const list = sessionsList.value.slice()
    list[idx] = next
    sessionsList.value = list
    return true
  }

  return {
    sessionsList,
    sessionListError,
    isLoading,
    groupedSessions,
    allSessions,
    loadSessions,
    applySessionRunStatus,
  }
}

/**
 * 会话级 thinking 强度 / 上下文长度持久化（sessions.patch 三态语义）。
 *
 * - 字段缺省（不传）= 不改动该字段；
 * - 字段为 `null` = 清除会话级覆盖（回落到全局模型配置）；
 * - 字段为值 = 设置覆盖（contextWindow 传 0 同样按清除处理，Rust 侧 `filter(|x| *x > 0)`）。
 *
 * 独立于 `useSessions()` 组合式函数（抽屉里没有列表实例也能调），内部自取 rpc store。
 */
export async function patchReasoningSettings(
  sessionId: string,
  patch: {
    reasoningEffort?: string | null
    contextWindow?: number | null
  },
): Promise<{ reasoningEffort: string | null; contextWindow: number | null }> {
  const rpc = useRpcStore()
  // 只把显式出现的字段放进 patch 参数，缺省字段完全不传（= 不动）
  const params: Record<string, unknown> = { key: sessionId }
  if ('reasoningEffort' in patch) params.reasoningEffort = patch.reasoningEffort
  if ('contextWindow' in patch) {
    // 0 / 负数 / 非法值统一归一为清除覆盖（null）
    const n = numberValue(patch.contextWindow)
    params.contextWindow = n != null && n > 0 ? n : null
  }
  const data = await rpc.call<SessionsPatchResponse>('sessions.patch', params)
  const effort = textValue(data?.reasoningEffort)
  return {
    reasoningEffort: effort ? effort : null,
    contextWindow: numberValue(data?.contextWindow),
  }
}
