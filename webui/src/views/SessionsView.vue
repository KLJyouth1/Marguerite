<template>
  <div class="hub control-stage sessions-stage">
    <header class="control-stage__header sessions-topnav__head">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('sessions.title') }}</h1>
        <p class="control-stage__subtitle">
          {{ t('sessions.subtitle') }}
        </p>
      </div>
      <div class="control-stage__actions">
        <input
          ref="importInput"
          type="file"
          accept=".json,application/json"
          class="sessions-io__file"
          @change="onImportFile"
        />
        <button
          class="btn btn--sm btn--ghost"
          :disabled="ioBusy"
          :title="t('sessions.io.importHint')"
          @click="pickImport"
        >
          <Icon name="download" :size="14" style="transform: rotate(180deg)" />
          {{ t('sessions.io.import') }}
        </button>
        <button
          class="btn btn--sm btn--ghost"
          :disabled="ioBusy"
          :title="t('sessions.io.exportAllHint')"
          @click="exportAll"
        >
          <Icon name="download" :size="14" />
          {{ t('sessions.io.exportAll') }}
        </button>
        <button
          class="btn btn--sm btn--ghost"
          :disabled="ioBusy"
          :title="t('sessions.io.shareHint')"
          @click="shareMarkdown"
        >
          <Icon name="share" :size="14" />
          {{ t('sessions.io.share') }}
        </button>
        <button
          class="btn btn--icon btn--ghost sessions-refresh"
          :title="refreshing ? t('sessions.refreshing') : t('sessions.refresh')"
          :aria-label="refreshing ? t('sessions.refreshing') : t('sessions.refresh')"
          :disabled="refreshing"
          @click="refresh"
        >
          <Icon name="refresh" :size="16" />
        </button>
      </div>
    </header>

    <nav class="sessions-topnav__bar" :aria-label="t('sessions.filter.ariaLabel')">
      <div class="hub-filters control-segmented" role="group" :aria-label="t('sessions.filter.ariaLabel')">
        <button
          v-for="chip in FILTER_CHIPS"
          :key="chip.id"
          type="button"
          class="hub-filter control-segmented__btn"
          :class="{ 'is-active': filter === chip.id }"
          :aria-pressed="filter === chip.id"
          @click="filter = chip.id"
        >
          {{ t(chip.labelKey) }}
          <span class="sessions-topnav__count" aria-hidden="true">{{ filterCounts[chip.id] }}</span>
        </button>
      </div>
      <div class="hub-search">
        <span class="hub-search__icon" aria-hidden="true">
          <Icon name="search" :size="14" />
        </span>
        <input
          v-model="search"
          type="text"
          class="hub-search__input"
          :placeholder="t('sessions.search.placeholder')"
          :aria-label="t('sessions.search.ariaLabel')"
          autocomplete="off"
        />
      </div>
    </nav>

    <SessionsTaskInput @submit="startTask" />

    <SessionsAttentionStrip
      v-if="!initialLoading"
      :approvals-count="pendingApprovals.length"
      :running-count="runningCount"
      :queued-count="queuedCount"
      :cost-usd="costUsd"
      :cost-period="costPeriod"
      @open-approvals="openBlockedSession"
      @open-usage="router.push('/usage')"
    />

    <section class="hub-list">
      <ErrorState
        v-if="sessionListError"
        :message="t('sessions.error.load')"
        :on-retry="loadAll"
      />

      <div v-else-if="isLoading && allSessions.length === 0" class="hub-state control-empty">
        <LoadingSpinner />
        <p class="control-empty__hint">{{ t('sessions.loading') }}</p>
      </div>

      <EmptyState
        v-else-if="allSessions.length === 0"
        icon="sessions"
        :title="t('sessions.empty.title')"
        :description="t('sessions.empty.body')"
        test-id="sessions-empty"
      />

      <EmptyState
        v-else-if="ledgerEntries.length === 0"
        icon="search"
        :icon-size="32"
        :title="t('sessions.noMatches.title')"
        :description="t('sessions.noMatches.body')"
        test-id="sessions-no-match"
      >
        <button class="btn btn--ghost btn--sm" @click="clearFilters">{{ t('sessions.noMatches.clear') }}</button>
      </EmptyState>

      <SessionsLedger
        v-else
        :entries="ledgerEntries"
        :agent-names="agentNames"
        :agents-loaded="agentsLoaded"
        :needs-input-keys="needsInputKeys"
        @open="openSession"
        @remove="removeSession"
        @export="exportSession"
      />
    </section>

    <SessionInspectDrawer
      :open="inspectItem !== null"
      :item="inspectItem"
      :agent-name="inspectAgentName"
      :parent-item="inspectParent"
      :needs-input="inspectItem ? needsInputKeys.has(inspectItem.key) : false"
      @close="closeInspect"
      @open-chat="openInChat"
      @aborted="onInspectAborted"
    />
  </div>
</template>

<script setup lang="ts">
import { computed, onActivated, onDeactivated, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { useRpcStore } from '@/stores/rpc'
import { readAuthToken } from '@/views/chat/read-auth-token'
import Icon from '@/components/Icon.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import LoadingSpinner from '@/components/LoadingSpinner.vue'
import { useConfirm } from '@/composables/useConfirm'
import { useToasts } from '@/composables/useToasts'
import { requestUsageSnapshot } from '@/composables/usage/useUsageQuery'
import type { UsageSnapshot } from '@/types/usage'
import SessionsTaskInput from '@/components/sessions/SessionsTaskInput.vue'
import SessionsAttentionStrip from '@/components/sessions/SessionsAttentionStrip.vue'
import SessionsLedger from '@/components/sessions/SessionsLedger.vue'
import SessionInspectDrawer from '@/components/sessions/SessionInspectDrawer.vue'
import {
  arrangeSessionLedger,
  itemKey,
  sessionMatches,
  sessionParentKey,
  useSessions,
  type SessionItem,
} from '@/composables/useSessions'
import {
  dispatchLocalSessionsDeleted,
  localSessionResetDetail,
  localSessionsDeletedDetail,
  LOCAL_SESSION_RESET_EVENT,
  LOCAL_SESSIONS_DELETED_EVENT,
} from '@/utils/sessionSync'
import { sessionAgentIdentity } from '@/components/sessions/sessionDisplay'

type FilterId = 'all' | 'chats' | 'automations' | 'channels'

interface AgentsListResponse {
  agents?: Array<{ id?: string; name?: string }>
}

interface DeleteResponse {
  deleted?: string[]
  errors?: unknown[]
}

const FILTER_CHIPS: Array<{ id: FilterId; labelKey: string }> = [
  { id: 'all', labelKey: 'sessions.filter.all' },
  { id: 'chats', labelKey: 'sessions.filter.chats' },
  { id: 'automations', labelKey: 'sessions.filter.automations' },
  { id: 'channels', labelKey: 'sessions.filter.channels' },
]

const FILTER_KINDS: Record<Exclude<FilterId, 'all'>, string> = {
  chats: 'chat',
  automations: 'cron',
  channels: 'channel',
}

const REFRESH_DEBOUNCE_MS = 150
const FALLBACK_POLL_MS = 30000
const SESSIONS_VIEW_SYNC_SOURCE = 'sessions-view'

const { t } = useI18n()
const router = useRouter()
const rpc = useRpcStore()
const { confirm } = useConfirm()
const { sessionsList, allSessions, isLoading, sessionListError, loadSessions } = useSessions()

/**
 * 首屏「还没有任何会话数据、正在拉列表」的窗口期。
 *
 * 真机缺陷（08-sessions.png）：「一切正常」绿勾与「正在加载任务」转圈同时出现——
 * 状态摘要把 `runningCount=0` 当成"确实没事"，但它此刻的语义只是"还没读到"。
 * 这个窗口期不显示摘要，等列表落地后再给结论。
 */
const initialLoading = computed(() => isLoading.value && allSessions.value.length === 0)

const filter = ref<FilterId>('all')
const search = ref('')

// ── 会话可移植性（导出 / 分享 / 导入）──────────────────────────────
// 导出走 Rust `sessions.export`（行级无损）；分享生成 Markdown 并复制；
// 导入先落盘再 `loadAll()` 回读，避免前端伪造"已导入"。
const importInput = ref<HTMLInputElement | null>(null)
const ioBusy = ref(false)
const { pushToast } = useToasts()

interface SessionExportResult {
  ok: boolean
  sessions?: unknown[]
  markdown?: string
  error?: string
  code?: string
}

function fileStamp(): string {
  const d = new Date()
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`
}

function downloadText(filename: string, text: string, mime: string) {
  const url = URL.createObjectURL(new Blob([text], { type: mime }))
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  a.click()
  URL.revokeObjectURL(url)
}

function ioError(e: unknown): string {
  return e instanceof Error ? e.message : String(e)
}

async function exportAll() {
  ioBusy.value = true
  try {
    const r = await rpc.call<SessionExportResult>('sessions.export', {})
    if (!r.ok) {
      pushToast(t('sessions.io.exportFailed', { error: r.error || r.code || 'unknown' }), { tone: 'danger' })
      return
    }
    downloadText(`marguerite-sessions-${fileStamp()}.json`, JSON.stringify(r, null, 2), 'application/json')
    pushToast(t('sessions.io.exportDone', { n: r.sessions?.length ?? 0 }), { tone: 'ok' })
  } catch (e) {
    pushToast(t('sessions.io.exportFailed', { error: ioError(e) }), { tone: 'danger' })
  } finally {
    ioBusy.value = false
  }
}

async function exportSession(item: { key: string }) {
  ioBusy.value = true
  try {
    const r = await rpc.call<SessionExportResult>('sessions.export', { sessionId: item.key })
    if (!r.ok) {
      pushToast(t('sessions.io.exportFailed', { error: r.error || r.code || 'unknown' }), { tone: 'danger' })
      return
    }
    const safe = item.key.replace(/[^\w.-]+/g, '_')
    downloadText(`marguerite-session-${safe}.json`, JSON.stringify(r, null, 2), 'application/json')
    pushToast(t('sessions.io.exportDone', { n: r.sessions?.length ?? 0 }), { tone: 'ok' })
  } catch (e) {
    pushToast(t('sessions.io.exportFailed', { error: ioError(e) }), { tone: 'danger' })
  } finally {
    ioBusy.value = false
  }
}

/** 分享：生成 Markdown（含思考与工具调用）→ 复制 + 下载 .md。 */
async function shareMarkdown() {
  ioBusy.value = true
  try {
    const r = await rpc.call<SessionExportResult>('sessions.export', { format: 'markdown' })
    if (!r.ok || !r.markdown) {
      pushToast(t('sessions.io.shareFailed', { error: r.error || r.code || 'unknown' }), { tone: 'danger' })
      return
    }
    downloadText(`marguerite-share-${fileStamp()}.md`, r.markdown, 'text/markdown')
    try {
      await navigator.clipboard.writeText(r.markdown)
      pushToast(t('sessions.io.shareCopied'), { tone: 'ok' })
    } catch {
      // 剪贴板被拒（非安全上下文）时，下载已完成，如实提示。
      pushToast(t('sessions.io.shareDownloaded'), { tone: 'info' })
    }
  } catch (e) {
    pushToast(t('sessions.io.shareFailed', { error: ioError(e) }), { tone: 'danger' })
  } finally {
    ioBusy.value = false
  }
}

function pickImport() {
  importInput.value?.click()
}

async function onImportFile(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return
  ioBusy.value = true
  try {
    const payload = await file.text()
    const r = await rpc.call<{
      ok: boolean
      total?: number
      skipped?: unknown[]
      error?: string
      code?: string
    }>('sessions.import', { payload })
    if (!r.ok) {
      pushToast(t('sessions.io.importFailed', { error: r.error || r.code || 'unknown' }), { tone: 'danger' })
      return
    }
    pushToast(
      t('sessions.io.importDone', { n: r.total ?? 0, skipped: r.skipped?.length ?? 0 }),
      { tone: 'ok' },
    )
    await loadAll()
  } catch (e) {
    pushToast(t('sessions.io.importFailed', { error: ioError(e) }), { tone: 'danger' })
  } finally {
    ioBusy.value = false
    input.value = ''
  }
}
const agentNames = ref<Map<string, string>>(new Map())
const agentsLoaded = ref(false)
let agentsRequestGeneration = 0
const pendingApprovals = ref<string[]>([])
const costUsd = ref<number | null>(null)
const costPeriod = ref<'today' | 'total'>('total')

let refreshTimer: ReturnType<typeof setTimeout> | null = null
let pollTimer: ReturnType<typeof setInterval> | null = null
let unsubs: Array<() => void> = []
let lastCostSnapshot: UsageSnapshot | null = null

// ---------------------------------------------------------------------------
// Derivations
// ---------------------------------------------------------------------------

const runningCount = computed(() => allSessions.value.filter(s => s.runStatus === 'running').length)
const queuedCount = computed(() => allSessions.value.filter(s => s.runStatus === 'queued').length)
const needsInputKeys = computed(() => new Set(pendingApprovals.value))

function matchesKind(item: SessionItem, kind: string | null, byKey: Map<string, SessionItem>): boolean {
  if (!kind) return true
  // Subagent rows follow their parent through the filter.
  let current: SessionItem | undefined = item
  for (let hop = 0; current && hop < 4; hop++) {
    if (current.sessionKind === kind) return true
    const parentKey = sessionParentKey(current)
    current = parentKey ? byKey.get(parentKey) : undefined
  }
  return false
}

function matchesFilter(item: SessionItem, byKey: Map<string, SessionItem>): boolean {
  if (filter.value === 'all') return true
  return matchesKind(item, FILTER_KINDS[filter.value], byKey)
}

/** Tab badges read the same kind matcher as the ledger so counts never drift. */
const filterCounts = computed<Record<FilterId, number>>(() => {
  const byKey = new Map(allSessions.value.map(item => [item.key, item]))
  const counts: Record<FilterId, number> = {
    all: allSessions.value.length,
    chats: 0,
    automations: 0,
    channels: 0,
  }
  for (const item of allSessions.value) {
    for (const id of ['chats', 'automations', 'channels'] as const) {
      if (matchesKind(item, FILTER_KINDS[id], byKey)) counts[id] += 1
    }
  }
  return counts
})

const ledgerEntries = computed(() => {
  const query = search.value.trim().toLowerCase()
  const byKey = new Map(allSessions.value.map(item => [item.key, item]))
  const visible = allSessions.value.filter(item =>
    matchesFilter(item, byKey) && (!query || sessionMatches(item, query)))
  return arrangeSessionLedger(visible)
})

// The inspected row tracks the live session list by key so status flips keep
// rendering; the click-time snapshot covers rows that drop out of the list.
const inspectKey = ref('')
const inspectFallback = ref<SessionItem | null>(null)

const inspectItem = computed(() => {
  if (!inspectKey.value) return null
  return allSessions.value.find(item => item.key === inspectKey.value) || inspectFallback.value
})

const inspectParent = computed(() => {
  const item = inspectItem.value
  if (!item) return null
  const parentKey = sessionParentKey(item)
  return parentKey ? allSessions.value.find(candidate => candidate.key === parentKey) || null : null
})

const inspectAgentName = computed(() => {
  const identity = sessionAgentIdentity(
    inspectItem.value?.effectiveAgentId,
    agentNames.value,
    agentsLoaded.value,
  )
  if (identity.kind === 'unknown') return t('sessions.unknownAgent')
  if (identity.kind === 'deleted') return t('sessions.deletedAgent', { id: identity.value })
  return identity.value
})

// ---------------------------------------------------------------------------
// Data loading
// ---------------------------------------------------------------------------

async function loadAgents() {
  const generation = ++agentsRequestGeneration
  try {
    const data = await rpc.call<AgentsListResponse>('agents.list')
    if (generation !== agentsRequestGeneration) return
    agentNames.value = new Map(
      (data?.agents || [])
        .filter(agent => agent.id)
        .map(agent => [String(agent.id), String(agent.name || agent.id)]))
    agentsLoaded.value = true
  } catch {
    if (generation === agentsRequestGeneration) {
      // A stale directory must not produce false “Deleted agent” labels after
      // the current authoritative lookup failed.
      agentsLoaded.value = false
    }
  }
}

function approvalAuthHeaders(): Record<string, string> {
  const headers: Record<string, string> = {}
  const token = readAuthToken()
  if (token) headers['Authorization'] = `Bearer ${token}`
  return headers
}

async function refreshApprovals() {
  try {
    const res = await fetch('/api/approvals', { headers: approvalAuthHeaders() })
    if (!res.ok) return
    const data = await res.json() as { pending?: Array<{ sessionKey?: string }> }
    pendingApprovals.value = (data.pending || [])
      .map(item => String(item.sessionKey || '').trim())
      .filter(Boolean)
  } catch {
    // Strip keeps the last known count.
  }
}

async function refreshCost() {
  try {
    const snapshot = await requestUsageSnapshot(rpc, 'today', {
      days: false,
      models: false,
      sessions: false,
      // Old gateways only expose lifetime totals. Showing that value is safe
      // as long as the label says total rather than incorrectly claiming today.
      fallbackRange: 'all',
      cachedSnapshot: lastCostSnapshot,
    })
    lastCostSnapshot = snapshot
    costUsd.value = snapshot.totals.cost
    costPeriod.value = snapshot.source === 'usage_ledger' ? 'today' : 'total'
  } catch {
    costUsd.value = null
    costPeriod.value = 'total'
  }
}

function loadAll() {
  void loadSessions()
  void loadAgents()
  void refreshApprovals()
  void refreshCost()
}

const refreshing = ref(false)

// Manual refresh shows a busy state; the fallback poll keeps calling loadAll so
// the button reacts only to user clicks, not background refreshes.
async function refresh() {
  if (refreshing.value) return
  refreshing.value = true
  try {
    await Promise.all([loadSessions(), loadAgents(), refreshApprovals(), refreshCost()])
  } finally {
    refreshing.value = false
  }
}

function scheduleSessionRefresh() {
  if (refreshTimer) clearTimeout(refreshTimer)
  refreshTimer = setTimeout(() => {
    refreshTimer = null
    void loadSessions()
  }, REFRESH_DEBOUNCE_MS)
}

function applyLocalDeletedSessions(keys: Set<string>) {
  if (keys.size === 0) return
  sessionsList.value = sessionsList.value.filter(item => !keys.has(itemKey(item)))
  pendingApprovals.value = pendingApprovals.value.filter(key => !keys.has(key))
  if (inspectKey.value && keys.has(inspectKey.value)) closeInspect()
}

function handleLocalSessionsDeleted(event: Event) {
  const detail = localSessionsDeletedDetail(event)
  if (!detail || detail.source === SESSIONS_VIEW_SYNC_SOURCE) return
  applyLocalDeletedSessions(new Set(detail.keys))
  scheduleSessionRefresh()
}

// 清空不改会话存在性，但本表里的消息计数与更新时间都会变，所以只刷新数据，
// 不动列表成员（也不关 inspect 抽屉 —— 会话还在）。
function handleLocalSessionReset(event: Event) {
  if (!localSessionResetDetail(event)) return
  scheduleSessionRefresh()
}

function handleApprovalPush() {
  void refreshApprovals()
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

function startTask(text: string) {
  router.push({
    path: '/chat/new',
    query: { agent: 'main' },
    // autosend asks the draft to fire the prefill in one step, so "Start task"
    // actually starts the task instead of dropping the operator at the composer.
    state: { prefill: text, autosend: true },
  }).catch(() => {})
}

// Row click opens the inspect drawer; navigation moved to the drawer's
// explicit "Open in chat" action.
function openSession(item: SessionItem) {
  inspectKey.value = item.key
  inspectFallback.value = item
}

function closeInspect() {
  inspectKey.value = ''
  inspectFallback.value = null
}

function openInChat(item: SessionItem) {
  closeInspect()
  router.push({ path: '/chat', query: { session: item.key } })
}

function onInspectAborted() {
  void loadSessions()
}

function openBlockedSession() {
  const key = pendingApprovals.value.find(Boolean)
  if (key) {
    router.push({ path: '/chat', query: { session: key } })
  }
  // No session-attached pending approval: nothing to open. Approvals resolve
  // inline in chat, so there is no standalone queue page to fall back to.
}

function clearFilters() {
  filter.value = 'all'
  search.value = ''
}

async function removeSession(item: SessionItem) {
  const ok = await confirm({
    title: t('sessions.delete.title'),
    body: t('sessions.delete.body', { title: item.title }),
    primaryLabel: t('sessions.delete.confirm'),
  })
  if (!ok) {
    return
  }
  let result: DeleteResponse | null = null
  try {
    result = await rpc.call<DeleteResponse>('sessions.delete', { keys: [item.key] })
  } catch (err) {
    console.warn('Delete failed: ' + (err instanceof Error ? err.message : String(err)))
    return
  }
  const deleted = new Set(result?.deleted || [])
  if (!deleted.has(item.key)) {
    console.warn('Delete failed: session was not reported deleted', result?.errors)
    void loadSessions()
    return
  }
  applyLocalDeletedSessions(deleted)
  dispatchLocalSessionsDeleted(deleted, SESSIONS_VIEW_SYNC_SOURCE)
  void loadSessions()
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

// This view is kept-alive (route meta.keepAlive), so live subscriptions and the
// fallback poll are bound on activation and released on deactivation — they must
// not keep firing while the view is cached and off-screen. onActivated also runs
// on first display, covering the initial load. onUnmounted is a final safety net
// for the rare case the KeepAlive cache evicts this instance.
function teardownLive() {
  unsubs.forEach(unsub => unsub())
  unsubs = []
  if (refreshTimer) { clearTimeout(refreshTimer); refreshTimer = null }
  if (pollTimer) { clearInterval(pollTimer); pollTimer = null }
  window.removeEventListener(LOCAL_SESSIONS_DELETED_EVENT, handleLocalSessionsDeleted)
  window.removeEventListener(LOCAL_SESSION_RESET_EVENT, handleLocalSessionReset)
}

onActivated(() => {
  loadAll()
  window.removeEventListener(LOCAL_SESSIONS_DELETED_EVENT, handleLocalSessionsDeleted)
  window.addEventListener(LOCAL_SESSIONS_DELETED_EVENT, handleLocalSessionsDeleted)
  window.removeEventListener(LOCAL_SESSION_RESET_EVENT, handleLocalSessionReset)
  window.addEventListener(LOCAL_SESSION_RESET_EVENT, handleLocalSessionReset)
  unsubs = [
    rpc.on('sessions.changed', scheduleSessionRefresh),
    rpc.on('exec.approval.requested', handleApprovalPush),
    rpc.on('exec.approval.resolved', handleApprovalPush),
    rpc.on('plugin.approval.requested', handleApprovalPush),
    rpc.on('plugin.approval.resolved', handleApprovalPush),
  ]
  pollTimer = setInterval(loadAll, FALLBACK_POLL_MS)
})

onDeactivated(teardownLive)
onUnmounted(teardownLive)
</script>

<style scoped>
.sessions-stage {
  margin-inline: auto;
  max-width: 1280px;
  padding-bottom: var(--sp-8);
  width: 100%;
}

.sessions-stage .control-stage__header {
  align-items: flex-start;
}

.sessions-stage .control-stage__title {
  font-size: clamp(1.75rem, 1.6rem + 0.35vw, 2rem);
  letter-spacing: -0.035em;
  line-height: 1.1;
}

.sessions-stage .control-stage__subtitle {
  margin-top: 7px;
}

.sessions-io__file {
  display: none;
}

.sessions-refresh {
  border-radius: var(--radius-control);
  color: var(--text-muted);
  height: 36px;
  padding: 0;
  width: 36px;
}

.sessions-refresh:hover:not(:disabled) {
  background: var(--bg-surface-2);
  color: var(--text);
}

.hub-list {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}

.hub-state {
  min-height: 240px;
}

.hub-state--empty {
  align-items: flex-start;
  display: grid;
  gap: var(--sp-3);
  grid-template-columns: 24px minmax(0, 1fr);
  justify-content: stretch;
  min-height: 0;
  padding: 40px 4px 32px;
  text-align: left;
}

.hub-state--empty .control-empty__icon {
  margin: 1px 0 0;
}

.hub-state__copy {
  display: grid;
  gap: 5px;
}

.hub-state--empty .control-empty__hint {
  margin: 0;
  max-width: 52ch;
}

.sessions-topnav__bar {
  align-items: center;
  border-bottom: 1px solid var(--border);
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-3);
  justify-content: space-between;
  padding-bottom: var(--sp-3);
}

.sessions-topnav__count {
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
  font-weight: 500;
  margin-left: 6px;
}

.hub-filter.is-active .sessions-topnav__count {
  color: var(--text-muted);
}

.hub-filter:focus-visible {
  box-shadow: var(--focus-ring);
  outline: none;
}

.hub-search {
  align-items: center;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  display: flex;
  gap: var(--sp-2);
  min-width: 200px;
  padding: 0 var(--sp-3);
}

.hub-search__icon {
  color: var(--text-dim);
  display: inline-flex;
  flex-shrink: 0;
}

.hub-search__input {
  background: transparent;
  border: none;
  color: var(--text);
  font-size: var(--fs-sm);
  outline: none;
  padding: var(--sp-2) 0;
  width: 100%;
}

.hub-search__input::placeholder {
  color: var(--text-dim);
}


@media (max-width: 760px) {
  .sessions-stage {
    padding-bottom: var(--sp-6);
  }

  .sessions-topnav__bar {
    align-items: stretch;
    flex-direction: column;
  }

  .hub-state--empty {
    padding-block: var(--sp-6);
  }
}
</style>
