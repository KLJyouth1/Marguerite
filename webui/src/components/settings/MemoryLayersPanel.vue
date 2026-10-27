<script setup lang="ts">
import { onMounted, ref, shallowRef } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import LoadingSpinner from '@/components/LoadingSpinner.vue'
import { useRpcStore } from '@/stores/rpc'

const STATUS_METHOD = 'memory.layers.status'
/// 上一轮记忆注入快照：回答「记忆到底有没有进模型上下文」。
const INJECT_METHOD = 'memory.inject.last'
/// 记忆治理（管理面）：只读列表 / 验证 / 真删。
const ENTRIES_METHOD = 'memory.entries'
const VALIDATE_METHOD = 'memory.validate'
const DELETE_METHOD = 'memory.delete'
/// 记忆管控概览（各层真实计数 + 待验证 + 已软删）。
const STATS_METHOD = 'memory.stats'
const SEARCH_METHODS: Record<string, string> = {
  sensory: 'memory.sensory.search',
  short_term: 'memory.short_term.search',
  long_term: 'memory.long_term.search',
  recall: 'memory.recall.search',
}
const SEARCHABLE = ['sensory', 'short_term', 'long_term', 'recall']

interface Layer {
  id: string
  name: string
  module: string
  reachable: boolean
  rpc: string[]
  count: number | null
}

interface SearchResult {
  title: string
  meta: string
}

const { t } = useI18n()
const rpc = useRpcStore()

const state = ref<'loading' | 'ready' | 'unsupported' | 'error'>('loading')
const layers = shallowRef<Layer[]>([])

const activeLayer = ref('short_term')
const query = ref('')
const sessionKey = ref('')
const searching = ref(false)
const results = shallowRef<SearchResult[]>([])
const searched = ref(false)

function objectValue(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' ? value as Record<string, unknown> : {}
}
function textValue(value: unknown): string {
  return typeof value === 'string' ? value : ''
}
function numberValue(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}
function stringList(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === 'string') : []
}

function normalizeLayer(value: unknown): Layer | null {
  const data = objectValue(value)
  const id = textValue(data.id)
  if (!id) return null
  return {
    id,
    name: textValue(data.name) || id,
    module: textValue(data.module),
    reachable: data.reachable === true,
    rpc: stringList(data.rpc),
    count: numberValue(data.count),
  }
}

function layerLabel(id: string): string {
  return t(`settings.memoryLayers.layers.${id}`)
}

async function load() {
  state.value = 'loading'
  try {
    await rpc.waitForConnection(8000)
    if (!rpc.supportsMethod(STATUS_METHOD)) {
      state.value = 'unsupported'
      return
    }
    const data = objectValue(await rpc.call(STATUS_METHOD))
    layers.value = (Array.isArray(data.layers) ? data.layers : [])
      .map(normalizeLayer)
      .filter((l): l is Layer => l !== null)
    state.value = 'ready'
  } catch {
    state.value = 'error'
  }
}

function normalizeResults(layer: string, value: unknown): SearchResult[] {
  const data = objectValue(value)
  const items = Array.isArray(data.items) ? data.items : []
  return items.map((item) => {
    const d = objectValue(item)
    if (layer === 'sensory') {
      return { title: textValue(d.text), meta: '' }
    }
    if (layer === 'short_term') {
      const meta = [textValue(d.session_id), textValue(d.message_count) ? `${textValue(d.message_count)} msgs` : '']
        .filter(Boolean)
        .join(' · ')
      return { title: textValue(d.summary), meta }
    }
    if (layer === 'long_term') {
      return { title: textValue(d.content), meta: textValue(d.meta) }
    }
    // recall
    const meta = [textValue(d.key), textValue(d.category)].filter(Boolean).join(' · ')
    return { title: textValue(d.snippet), meta }
  })
}

async function search() {
  const q = query.value.trim()
  if (!q || searching.value) return
  const method = SEARCH_METHODS[activeLayer.value]
  if (!method || !rpc.supportsMethod(method)) return
  searching.value = true
  searched.value = true
  try {
    const params: Record<string, unknown> = { query: q, limit: 20 }
    if (activeLayer.value === 'sensory') params.sessionKey = sessionKey.value.trim()
    if (activeLayer.value === 'recall') params.category = 'user'
    results.value = normalizeResults(activeLayer.value, await rpc.call(method, params))
  } catch {
    results.value = []
  } finally {
    searching.value = false
  }
}

function selectLayer(id: string) {
  activeLayer.value = id
  results.value = []
  searched.value = false
}

interface InjectLayer {
  layer: string
  hits: number
  kept: number
}

interface InjectSnapshot {
  available: boolean
  reason: string
  session: string
  hits: number
  kept: number
  tokens: number
  budget: number
  trimmed: number
  layers: InjectLayer[]
}

const inject = ref<InjectSnapshot | null>(null)
const injectLoading = ref(false)
const injectError = ref('')

function normalizeInject(value: unknown): InjectSnapshot {
  const d = objectValue(value)
  const rawLayers = Array.isArray(d.layers) ? d.layers : []
  return {
    available: d.available === true,
    reason: textValue(d.reason),
    session: textValue(d.session),
    hits: numberValue(d.hits) ?? 0,
    kept: numberValue(d.kept) ?? 0,
    tokens: numberValue(d.tokens) ?? 0,
    budget: numberValue(d.budget) ?? 0,
    trimmed: numberValue(d.trimmed) ?? 0,
    layers: rawLayers.map((entry) => {
      const l = objectValue(entry)
      return {
        layer: textValue(l.layer),
        hits: numberValue(l.hits) ?? 0,
        kept: numberValue(l.kept) ?? 0,
      }
    }),
  }
}

async function loadInject() {
  if (injectLoading.value) return
  if (!rpc.supportsMethod(INJECT_METHOD)) return
  injectLoading.value = true
  injectError.value = ''
  try {
    inject.value = normalizeInject(await rpc.call(INJECT_METHOD))
  } catch {
    injectError.value = t('settings.memoryLayers.inject.failed')
  } finally {
    injectLoading.value = false
  }
}

/* ── 记忆治理：条目列表 / 过滤 / 验证 / 删除 ─────────────────────────── */

interface MemoryEntry {
  id: number
  kind: string
  title: string
  body: string
  source: string
  actor: string
  confidence: number | null
  status: string
  validated: boolean
  created_at: string
  session_key?: string
}

interface MemoryStats {
  total: number
  validated: number
  pendingValidation: number
  deleted: number
  autoCaptureEnabled: boolean
  byKind: Record<string, number>
}

const KINDS = ['pitfall', 'decision', 'pattern']
const STATUSES = ['draft', 'active']

const entries = shallowRef<MemoryEntry[]>([])
const entriesLoading = ref(false)
const entriesLoaded = ref(false)
const entriesError = ref('')
const filterKind = ref('')
const filterStatus = ref('')
const actingId = ref<number | null>(null)

const stats = ref<MemoryStats | null>(null)
const statsError = ref('')

function normalizeStats(value: unknown): MemoryStats | null {
  const d = objectValue(value)
  if (d.ok === false) return null
  const byKindRaw = objectValue(d.byKind)
  const byKind: Record<string, number> = {}
  for (const [key, raw] of Object.entries(byKindRaw)) {
    byKind[key] = numberValue(raw) ?? 0
  }
  return {
    total: numberValue(d.total) ?? 0,
    validated: numberValue(d.validated) ?? 0,
    pendingValidation: numberValue(d.pendingValidation) ?? 0,
    deleted: numberValue(d.deleted) ?? 0,
    autoCaptureEnabled: d.autoCaptureEnabled === true,
    byKind,
  }
}

async function loadStats() {
  if (!rpc.supportsMethod(STATS_METHOD)) return
  statsError.value = ''
  try {
    const data = objectValue(await rpc.call(STATS_METHOD))
    if (data.ok === false) {
      statsError.value = textValue(data.message) || t('memory.gov.failed')
      stats.value = null
      return
    }
    stats.value = normalizeStats(data)
  } catch {
    statsError.value = t('memory.gov.failed')
  }
}

function normalizeEntry(value: unknown): MemoryEntry | null {
  const d = objectValue(value)
  const id = numberValue(d.id)
  if (id === null) return null
  return {
    id,
    kind: textValue(d.kind) || 'pitfall',
    title: textValue(d.title),
    body: textValue(d.body),
    source: textValue(d.source) || 'agent',
    actor: textValue(d.actor),
    confidence: numberValue(d.confidence),
    status: textValue(d.status) || 'draft',
    validated: d.validated === true,
    created_at: textValue(d.created_at),
    session_key: textValue(d.session_key),
  }
}

function kindLabel(kind: string): string {
  if (kind === 'pitfall') return t('memory.gov.kind.pitfall')
  if (kind === 'decision') return t('memory.gov.kind.decision')
  if (kind === 'pattern') return t('memory.gov.kind.pattern')
  return kind
}

function statusLabel(status: string): string {
  if (status === 'active') return t('memory.gov.status.active')
  if (status === 'draft') return t('memory.gov.status.draft')
  return status
}

async function loadEntries() {
  if (entriesLoading.value) return
  if (!rpc.supportsMethod(ENTRIES_METHOD)) {
    entriesError.value = t('memory.gov.unavailable')
    return
  }
  entriesLoading.value = true
  entriesError.value = ''
  try {
    const params: Record<string, unknown> = { limit: 50, offset: 0 }
    if (filterKind.value) params.kind = filterKind.value
    if (filterStatus.value) params.status = filterStatus.value
    const data = objectValue(await rpc.call(ENTRIES_METHOD, params))
    if (data.ok === false) {
      entriesError.value = textValue(data.message) || t('memory.gov.failed')
      entries.value = []
      return
    }
    entries.value = (Array.isArray(data.items) ? data.items : [])
      .map(normalizeEntry)
      .filter((e): e is MemoryEntry => e !== null)
    entriesLoaded.value = true
  } catch {
    entriesError.value = t('memory.gov.failed')
    entries.value = []
  } finally {
    entriesLoading.value = false
  }
}

async function validateEntry(entry: MemoryEntry) {
  if (actingId.value !== null || !rpc.supportsMethod(VALIDATE_METHOD)) return
  actingId.value = entry.id
  entriesError.value = ''
  try {
    const data = objectValue(
      await rpc.call(VALIDATE_METHOD, { id: entry.id, validated: !entry.validated }),
    )
    if (data.ok === false) {
      entriesError.value = textValue(data.message) || t('memory.gov.failed')
      return
    }
    const updated = data.item ? normalizeEntry(data.item) : null
    entries.value = entries.value.map((e) =>
      e.id === entry.id
        ? (updated ?? { ...e, validated: !entry.validated })
        : e,
    )
  } catch {
    entriesError.value = t('memory.gov.failed')
  } finally {
    actingId.value = null
  }
}

async function deleteEntry(entry: MemoryEntry) {
  if (actingId.value !== null || !rpc.supportsMethod(DELETE_METHOD)) return
  if (!window.confirm(t('memory.gov.deleteConfirm', { id: entry.id, title: entry.title }))) return
  actingId.value = entry.id
  entriesError.value = ''
  try {
    const data = objectValue(await rpc.call(DELETE_METHOD, { id: entry.id }))
    if (data.ok === false) {
      entriesError.value = textValue(data.message) || t('memory.gov.failed')
      return
    }
    // 软删成功：后端返回 {ok, id, deletedAt}；列表立即移除并刷新概览计数。
    entries.value = entries.value.filter((e) => e.id !== entry.id)
    void loadStats()
  } catch {
    entriesError.value = t('memory.gov.failed')
  } finally {
    actingId.value = null
  }
}

onMounted(() => {
  void load()
  void loadEntries()
  void loadStats()
})
</script>

<template>
  <section class="memory-layers" data-testid="memory-layers-panel">
    <header class="memory-layers__head">
      <h3 class="memory-layers__title">{{ t('settings.memoryLayers.title') }}</h3>
      <p class="memory-layers__description">{{ t('settings.memoryLayers.description') }}</p>
    </header>

    <div v-if="state === 'loading'" class="memory-layers__center">
      <LoadingSpinner />
    </div>
    <p v-else-if="state === 'unsupported'" class="memory-layers__notice" role="status">
      {{ t('settings.memoryLayers.unsupported') }}
    </p>
    <p v-else-if="state === 'error'" class="memory-layers__notice memory-layers__notice--error" role="alert">
      {{ t('settings.memoryLayers.loadFailed') }}
    </p>

    <template v-else>
      <!-- 上一轮真实注入：让「记忆有没有起作用」可自查，而不是凭感觉 -->
      <section class="memory-inject" data-testid="memory-inject-last">
        <header class="memory-inject__head">
          <h4 class="memory-inject__title">{{ t('settings.memoryLayers.inject.title') }}</h4>
          <button
            type="button"
            class="memory-inject__refresh"
            :disabled="injectLoading || !rpc.supportsMethod(INJECT_METHOD)"
            @click="loadInject"
          >
            <Icon name="refresh" :size="13" />
            <span>{{ t('settings.memoryLayers.inject.refresh') }}</span>
          </button>
        </header>
        <p v-if="injectError" class="memory-inject__notice" role="alert">{{ injectError }}</p>
        <p
          v-else-if="!inject"
          class="memory-inject__notice"
          role="status"
        >{{ injectLoading ? t('settings.memoryLayers.inject.loading') : t('settings.memoryLayers.inject.idle') }}</p>
        <p
          v-else-if="!inject.available"
          class="memory-inject__notice"
          role="status"
        >{{ inject.reason || t('settings.memoryLayers.inject.idle') }}</p>
        <template v-else>
          <p class="memory-inject__summary">
            {{ t('settings.memoryLayers.inject.summary', {
              hits: inject.hits,
              kept: inject.kept,
              tokens: inject.tokens,
              budget: inject.budget,
            }) }}
          </p>
          <p v-if="inject.trimmed > 0" class="memory-inject__notice">
            {{ t('settings.memoryLayers.inject.trimmed', { count: inject.trimmed }) }}
          </p>
          <ul class="memory-inject__layers">
            <li
              v-for="entry in inject.layers"
              :key="entry.layer"
              class="memory-inject__layer"
              :data-empty="entry.kept === 0 ? 'true' : 'false'"
            >
              <span class="memory-inject__layer-name">{{ entry.layer }}</span>
              <span class="memory-inject__layer-count">{{ entry.kept }} / {{ entry.hits }}</span>
            </li>
          </ul>
        </template>
      </section>

      <ul class="memory-layers__grid">
        <li
          v-for="layer in layers"
          :key="layer.id"
          class="memory-layers__layer"
          :data-reachable="layer.reachable ? 'true' : 'false'"
        >
          <div class="memory-layers__layer-head">
            <span class="memory-layers__layer-name">{{ layerLabel(layer.id) }}</span>
            <span class="memory-layers__layer-count" v-if="layer.count !== null">{{ layer.count }}</span>
            <span
              class="memory-layers__layer-badge"
              :class="layer.reachable ? 'is-reachable' : 'is-muted'"
            >
              {{ layer.reachable ? t('settings.memoryLayers.reachable') : t('settings.memoryLayers.notReachable') }}
            </span>
          </div>
          <code class="memory-layers__layer-module">{{ layer.module }}</code>
        </li>
      </ul>

      <!-- 记忆治理：可验证、可删除的管理面（数据缺失/失败如实显示） -->
      <section class="memory-gov" data-testid="memory-gov-entries">
        <header class="memory-gov__head">
          <div>
            <h4 class="memory-gov__title">{{ t('memory.gov.title') }}</h4>
            <p class="memory-gov__description">{{ t('memory.gov.description') }}</p>
          </div>
          <button
            type="button"
            class="memory-gov__refresh"
            :disabled="entriesLoading || !rpc.supportsMethod(ENTRIES_METHOD)"
            @click="loadEntries"
          >
            <Icon name="refresh" :size="13" />
            <span>{{ t('memory.gov.refresh') }}</span>
          </button>
        </header>

        <p v-if="statsError" class="memory-gov__notice memory-gov__notice--error" role="alert">
          {{ statsError }}
        </p>
        <div v-if="stats" class="memory-gov__stats" data-testid="memory-gov-stats">
          <div class="memory-gov__stat">
            <span class="memory-gov__stat-num">{{ stats.total }}</span>
            <span class="memory-gov__stat-label">{{ t('memory.gov.stats.total') }}</span>
          </div>
          <div class="memory-gov__stat">
            <span class="memory-gov__stat-num">{{ stats.validated }}</span>
            <span class="memory-gov__stat-label">{{ t('memory.gov.stats.validated') }}</span>
          </div>
          <div class="memory-gov__stat">
            <span class="memory-gov__stat-num">{{ stats.pendingValidation }}</span>
            <span class="memory-gov__stat-label">{{ t('memory.gov.stats.pending') }}</span>
          </div>
          <div class="memory-gov__stat">
            <span class="memory-gov__stat-num">{{ stats.deleted }}</span>
            <span class="memory-gov__stat-label">{{ t('memory.gov.stats.deleted') }}</span>
          </div>
          <div class="memory-gov__stat">
            <span class="memory-gov__stat-num">
              {{ stats.autoCaptureEnabled ? t('memory.gov.stats.on') : t('memory.gov.stats.off') }}
            </span>
            <span class="memory-gov__stat-label">{{ t('memory.gov.stats.autoCapture') }}</span>
          </div>
        </div>

        <div class="memory-gov__filters">
          <label class="memory-gov__field">
            <span>{{ t('memory.gov.filterKind') }}</span>
            <select v-model="filterKind" class="memory-gov__select" @change="loadEntries">
              <option value="">{{ t('memory.gov.all') }}</option>
              <option v-for="kind in KINDS" :key="kind" :value="kind">{{ kindLabel(kind) }}</option>
            </select>
          </label>
          <label class="memory-gov__field">
            <span>{{ t('memory.gov.filterStatus') }}</span>
            <select v-model="filterStatus" class="memory-gov__select" @change="loadEntries">
              <option value="">{{ t('memory.gov.all') }}</option>
              <option v-for="status in STATUSES" :key="status" :value="status">{{ statusLabel(status) }}</option>
            </select>
          </label>
        </div>

        <p v-if="entriesError" class="memory-gov__notice memory-gov__notice--error" role="alert">
          {{ entriesError }}
        </p>
        <p v-else-if="entriesLoading" class="memory-gov__notice" role="status">
          {{ t('memory.gov.loading') }}
        </p>
        <p v-else-if="entriesLoaded && !entries.length" class="memory-gov__notice" role="status">
          {{ t('memory.gov.empty') }}
        </p>

        <ul v-if="entries.length" class="memory-gov__list">
          <li v-for="entry in entries" :key="entry.id" class="memory-gov__item" :data-id="entry.id">
            <div class="memory-gov__item-head">
              <span class="memory-gov__item-title">{{ entry.title }}</span>
              <span class="memory-gov__badge">{{ kindLabel(entry.kind) }}</span>
              <span class="memory-gov__badge" :data-status="entry.status">{{ statusLabel(entry.status) }}</span>
              <span v-if="entry.validated" class="memory-gov__badge is-validated">
                {{ t('memory.gov.validated') }}
              </span>
            </div>
            <p class="memory-gov__item-body">{{ entry.body }}</p>
            <div class="memory-gov__item-meta">
              <span>{{ t('memory.gov.source', { source: entry.source }) }}</span>
              <span v-if="entry.actor">{{ t('memory.gov.actor', { actor: entry.actor }) }}</span>
              <span v-if="entry.session_key">
                {{ t('memory.gov.sessionKey', { key: entry.session_key }) }}
              </span>
              <span v-if="entry.confidence !== null">
                {{ t('memory.gov.confidence', { value: entry.confidence.toFixed(2) }) }}
              </span>
              <span class="memory-gov__id">#{{ entry.id }}</span>
            </div>
            <div class="memory-gov__actions">
              <button
                type="button"
                class="memory-gov__action"
                :disabled="actingId !== null || !rpc.supportsMethod(VALIDATE_METHOD)"
                @click="validateEntry(entry)"
              >
                {{ entry.validated ? t('memory.gov.unvalidate') : t('memory.gov.validate') }}
              </button>
              <button
                type="button"
                class="memory-gov__action memory-gov__action--danger"
                :disabled="actingId !== null || !rpc.supportsMethod(DELETE_METHOD)"
                @click="deleteEntry(entry)"
              >
                {{ t('memory.gov.delete') }}
              </button>
            </div>
          </li>
        </ul>
      </section>

      <div class="memory-layers__search">
        <div class="memory-layers__tabs" role="tablist">
          <button
            v-for="id in SEARCHABLE"
            :key="id"
            type="button"
            role="tab"
            class="memory-layers__tab"
            :aria-selected="activeLayer === id ? 'true' : 'false'"
            :class="{ 'is-active': activeLayer === id }"
            @click="selectLayer(id)"
          >
            {{ layerLabel(id) }}
          </button>
        </div>

        <label v-if="activeLayer === 'sensory'" class="memory-layers__field">
          <span>{{ t('settings.memoryLayers.sessionKey') }}</span>
          <input v-model="sessionKey" class="memory-layers__input" type="text" />
        </label>

        <div class="memory-layers__query">
          <input
            v-model="query"
            class="memory-layers__input"
            type="text"
            :placeholder="t('settings.memoryLayers.searchPlaceholder')"
            @keydown.enter="search"
          />
          <button type="button" class="btn btn--primary" :disabled="searching || !query.trim()" @click="search">
            <Icon name="search" :size="15" aria-hidden="true" />
            {{ t('settings.memoryLayers.search') }}
          </button>
        </div>

        <p v-if="searched && !results.length" class="memory-layers__notice">
          {{ t('settings.memoryLayers.searchEmpty') }}
        </p>
        <ul v-else-if="results.length" class="memory-layers__results">
          <li v-for="(result, index) in results" :key="index" class="memory-layers__result">
            <span class="memory-layers__result-title">{{ result.title }}</span>
            <span v-if="result.meta" class="memory-layers__result-meta">{{ result.meta }}</span>
          </li>
        </ul>
      </div>
    </template>
  </section>
</template>

<style scoped>
.memory-layers {
  display: flex;
  flex-direction: column;
  gap: var(--sp-5);
  margin: 0 auto;
  max-width: 880px;
  min-height: 100%;}
.memory-layers__head {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}
.memory-layers__title {
  color: var(--text);
  font-size: var(--fs-lg);
  margin: 0;
}
.memory-layers__description,
.memory-layers__notice {
  color: var(--text-muted);
  font-size: var(--fs-sm);
  line-height: 1.55;
  margin: 0;
}
.memory-layers__notice--error {
  color: var(--danger);
}
.memory-layers__center {
  align-items: center;
  display: flex;
  justify-content: center;
  min-height: 160px;
}
.memory-layers__grid {
  display: grid;
  gap: var(--sp-3);
  grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
  list-style: none;
  margin: 0;
  padding: 0;
}
.memory-layers__layer {
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  padding: var(--sp-3) var(--sp-4);
}
.memory-layers__layer-head {
  align-items: center;
  display: flex;
  gap: var(--sp-2);
}
.memory-layers__layer-name {
  color: var(--text);
  font-size: var(--fs-sm);
  font-weight: 600;
}
.memory-layers__layer-count {
  color: var(--text-muted);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  margin-left: auto;
}
.memory-layers__layer-badge {
  border-radius: var(--radius-pill);
  font-size: var(--fs-xs);
  padding: 2px var(--sp-2);
}
.memory-layers__layer-badge.is-reachable {
  background: color-mix(in srgb, var(--ok) 10%, transparent);
  color: var(--ok);
}
.memory-layers__layer-badge.is-muted {
  background: var(--bg-surface);
  color: var(--text-dim);
}
.memory-layers__layer-module {
  color: var(--text-dim);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.memory-layers__search {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  padding-top: var(--sp-2);
}
.memory-layers__tabs {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-1);
}
.memory-layers__tab {
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  color: var(--text-muted);
  cursor: pointer;
  font: inherit;
  font-size: var(--fs-sm);
  padding: 4px var(--sp-3);
}
.memory-layers__tab.is-active {
  background: color-mix(in srgb, var(--accent) 12%, var(--bg-surface));
  border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
  color: var(--text);
}
.memory-layers__field {
  color: var(--text-dim);
  display: flex;
  flex-direction: column;
  font-size: var(--fs-xs);
  gap: var(--sp-1);
}
.memory-layers__query {
  display: flex;
  gap: var(--sp-2);
}
.memory-layers__input {
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  color: var(--text);
  flex: 1;
  font: inherit;
  font-size: var(--fs-sm);
  padding: var(--sp-2) var(--sp-3);
}
.memory-layers__input:focus-visible {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--focus-ring);
  outline: none;
}
.memory-layers__results {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  list-style: none;
  margin: 0;
  padding: 0;
}
.memory-layers__result {
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: var(--sp-3) var(--sp-4);
}
.memory-layers__result-title {
  color: var(--text);
  font-size: var(--fs-sm);
  line-height: 1.5;
}
.memory-layers__result-meta {
  color: var(--text-dim);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 上一轮真实注入快照：低干扰卡片，读写都用主题 token。 */
.memory-inject {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  padding: var(--sp-3) var(--sp-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: color-mix(in srgb, var(--bg-surface) 92%, var(--bg-surface-2));
}
.memory-inject__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-2);
}
.memory-inject__title {
  margin: 0;
  color: var(--text);
  font-size: var(--fs-sm);
  font-weight: 600;
}
.memory-inject__refresh {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  font-size: var(--fs-xs);
  cursor: pointer;
}
.memory-inject__refresh:hover:not(:disabled) {
  color: var(--text);
  border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
}
.memory-inject__refresh:disabled {
  cursor: default;
  opacity: 0.55;
}
.memory-inject__refresh:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}
.memory-inject__notice {
  margin: 0;
  color: var(--text-dim);
  font-size: var(--fs-xs);
  line-height: 1.5;
}
.memory-inject__summary {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
  font-variant-numeric: tabular-nums;
}
.memory-inject__layers {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
  margin: 0;
  padding: 0;
  list-style: none;
}
.memory-inject__layer {
  display: inline-flex;
  align-items: baseline;
  gap: 4px;
  padding: 2px 8px;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--text) 5%, transparent);
  font-size: var(--fs-xs);
}
.memory-inject__layer[data-empty='true'] {
  opacity: 0.55;
}
.memory-inject__layer-name {
  color: var(--text-muted);
  font-family: var(--font-mono);
}
.memory-inject__layer-count {
  color: var(--text);
  font-variant-numeric: tabular-nums;
}

/* 记忆治理区块：与楼层卡片同一套主题 token，不引入新配色。 */
.memory-gov {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  padding: var(--sp-3) var(--sp-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: color-mix(in srgb, var(--bg-surface) 92%, var(--bg-surface-2));
}
.memory-gov__head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--sp-3);
}
.memory-gov__title {
  margin: 0;
  color: var(--text);
  font-size: var(--fs-sm);
  font-weight: 600;
}
.memory-gov__description,
.memory-gov__notice {
  margin: 0;
  color: var(--text-dim);
  font-size: var(--fs-xs);
  line-height: 1.5;
}
.memory-gov__notice--error {
  color: var(--danger);
}
.memory-gov__refresh {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  font-size: var(--fs-xs);
  cursor: pointer;
}
.memory-gov__refresh:disabled {
  cursor: default;
  opacity: 0.55;
}
.memory-gov__filters {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-3);
}
.memory-gov__stats {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
}
.memory-gov__stat {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  min-width: 72px;
  padding: var(--sp-1) var(--sp-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-elevated);
}
.memory-gov__stat-num {
  color: var(--text);
  font-size: var(--fs-sm);
  font-variant-numeric: tabular-nums;
  font-weight: 600;
}
.memory-gov__stat-label {
  color: var(--text-dim);
  font-size: var(--fs-xs);
}
.memory-gov__field {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
  color: var(--text-dim);
  font-size: var(--fs-xs);
}
.memory-gov__select {
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  color: var(--text);
  font: inherit;
  font-size: var(--fs-sm);
  padding: var(--sp-1) var(--sp-2);
}
.memory-gov__list {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  list-style: none;
  margin: 0;
  padding: 0;
}
.memory-gov__item {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
  padding: var(--sp-2) var(--sp-3);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-elevated);
}
.memory-gov__item-head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--sp-2);
}
.memory-gov__item-title {
  color: var(--text);
  font-size: var(--fs-sm);
  font-weight: 600;
}
.memory-gov__badge {
  padding: 1px 6px;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--text) 6%, transparent);
  color: var(--text-muted);
  font-size: var(--fs-xs);
}
.memory-gov__badge.is-validated {
  background: color-mix(in srgb, var(--ok) 12%, transparent);
  color: var(--ok);
}
.memory-gov__item-body {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}
.memory-gov__item-meta {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
  color: var(--text-dim);
  font-size: var(--fs-xs);
  font-variant-numeric: tabular-nums;
}
.memory-gov__id {
  margin-left: auto;
  font-family: var(--font-mono);
}
.memory-gov__actions {
  display: flex;
  gap: var(--sp-2);
}
.memory-gov__action {
  padding: 2px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  font-size: var(--fs-xs);
  cursor: pointer;
}
.memory-gov__action:hover:not(:disabled) {
  color: var(--text);
  border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
}
.memory-gov__action:disabled {
  cursor: default;
  opacity: 0.55;
}
.memory-gov__action--danger:hover:not(:disabled) {
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 45%, var(--border));
}
</style>
