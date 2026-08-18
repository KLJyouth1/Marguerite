<script setup lang="ts">
import { computed, onMounted, ref, shallowRef } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import LoadingSpinner from '@/components/LoadingSpinner.vue'
import { useRpcStore } from '@/stores/rpc'

const SEARCH_METHOD = 'memory.cross_project.search'
const EXPAND_METHOD = 'memory.cross_project.expand'
const BEHAVIOR_LIST_METHOD = 'memory.behavior.list'
const BEHAVIOR_SUMMARY_METHOD = 'memory.behavior.summary'

interface Hit {
  project: string
  file: string
  kind: string
  title: string
  line: number
  excerpt: string
  score: number
  updated_at: string
}

interface ExpandedState {
  status: 'collapsed' | 'loading' | 'ready'
  hit?: string
  context?: Array<{ line: number; text: string }>
  error?: string
}

interface BehaviorItem {
  seq: number
  at_ms: number
  kind: string
  detail: string
}

interface BehaviorSummary {
  session: string | null
  events: number
  turns: number
  tool_uses: Array<{ tool: string; count: number }>
  last_kind: string | null
}

const { t } = useI18n()
const rpc = useRpcStore()

const state = ref<'loading' | 'ready' | 'unsupported' | 'error'>('loading')
const query = ref('')
const searching = ref(false)
const items = shallowRef<Hit[]>([])
const expanded = shallowRef<Record<string, ExpandedState>>({})
const errorCode = ref('')

const behaviorSummary = shallowRef<BehaviorSummary | null>(null)
const timeline = shallowRef<BehaviorItem[]>([])
const behaviorState = ref<'loading' | 'ready' | 'unsupported' | 'error'>('loading')

const behaviorCount = computed(() => behaviorSummary.value?.events ?? 0)
const toolStats = computed(() => behaviorSummary.value?.tool_uses ?? [])

function objectValue(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' ? value as Record<string, unknown> : {}
}

function textValue(value: unknown): string {
  return typeof value === 'string' ? value : ''
}

function numberValue(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

function normalizeHit(value: unknown): Hit | null {
  const data = objectValue(value)
  const project = textValue(data.project)
  const file = textValue(data.file)
  if (!project || !file) return null
  return {
    project,
    file,
    kind: textValue(data.kind) || 'project_memory',
    title: textValue(data.title),
    line: numberValue(data.line) || 1,
    excerpt: textValue(data.excerpt),
    score: numberValue(data.score),
    updated_at: textValue(data.updated_at),
  }
}

function rpcErrorCode(error: unknown): string {
  return textValue((error as { code?: string } | undefined)?.code)
}

function isMethodMissing(error: unknown): boolean {
  return rpcErrorCode(error) === 'METHOD_NOT_FOUND'
    || /method not found|unknown method|not registered/i.test(
      error instanceof Error ? error.message : String(error),
    )
}

function formatScore(score: number): string {
  return (score * 100).toFixed(0)
}

function formatTime(atMs: number): string {
  if (!atMs) return ''
  const date = new Date(atMs)
  if (Number.isNaN(date.getTime())) return ''
  return new Intl.DateTimeFormat(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  }).format(date)
}

async function search() {
  const q = query.value.trim()
  if (!q || searching.value) return
  if (!rpc.supportsMethod(SEARCH_METHOD)) {
    state.value = 'unsupported'
    return
  }
  searching.value = true
  errorCode.value = ''
  try {
    const result = objectValue(await rpc.call(SEARCH_METHOD, { query: q, limit: 8 }))
    const list = Array.isArray(result.items)
      ? result.items.map(normalizeHit).filter((hit): hit is Hit => hit !== null)
      : []
    items.value = list
    expanded.value = {}
    state.value = 'ready'
  } catch (error) {
    if (isMethodMissing(error)) {
      rpc.markMethodUnavailable(SEARCH_METHOD)
      state.value = 'unsupported'
      return
    }
    errorCode.value = rpcErrorCode(error)
    state.value = 'error'
  } finally {
    searching.value = false
  }
}

function expandedKey(hit: Hit): string {
  return `${hit.project}:${hit.file}:${hit.line}`
}

function expandedState(key: string): ExpandedState | undefined {
  return expanded.value[key]
}

async function toggleExpand(hit: Hit) {
  const key = expandedKey(hit)
  if (expanded.value[key]?.status === 'ready') {
    const next = { ...expanded.value }
    delete next[key]
    expanded.value = next
    return
  }
  if (!rpc.supportsMethod(EXPAND_METHOD)) return
  expanded.value = { ...expanded.value, [key]: { status: 'loading' } }
  try {
    const result = objectValue(await rpc.call(EXPAND_METHOD, {
      project: hit.project,
      file: hit.file,
      line: hit.line,
    }))
    expanded.value = {
      ...expanded.value,
      [key]: {
        status: 'ready',
        hit: textValue(result.hit),
        context: Array.isArray(result.context)
          ? result.context.map((entry: unknown) => {
              const row = objectValue(entry)
              return { line: numberValue(row.line), text: textValue(row.text) }
            })
          : [],
        error: textValue(result.error) || undefined,
      },
    }
  } catch (error) {
    if (isMethodMissing(error)) rpc.markMethodUnavailable(EXPAND_METHOD)
    expanded.value = { ...expanded.value, [key]: { status: 'ready', hit: '', context: [], error: rpcErrorCode(error) || undefined } }
  }
}

async function loadBehavior() {
  const listSupported = rpc.supportsMethod(BEHAVIOR_LIST_METHOD)
  const summarySupported = rpc.supportsMethod(BEHAVIOR_SUMMARY_METHOD)
  if (!listSupported && !summarySupported) {
    behaviorState.value = 'unsupported'
    return
  }
  behaviorState.value = 'loading'
  try {
    if (summarySupported) {
      const summary = objectValue(await rpc.call(BEHAVIOR_SUMMARY_METHOD, {}))
      behaviorSummary.value = {
        session: textValue(summary.session) || null,
        events: numberValue(summary.events),
        turns: numberValue(summary.turns),
        tool_uses: Array.isArray(summary.tool_uses)
          ? summary.tool_uses.map((entry: unknown) => {
              const row = objectValue(entry)
              return { tool: textValue(row.tool), count: numberValue(row.count) }
            })
          : [],
        last_kind: textValue(summary.last_kind) || null,
      }
    }
    if (listSupported) {
      const result = objectValue(await rpc.call(BEHAVIOR_LIST_METHOD, { limit: 30 }))
      timeline.value = Array.isArray(result.items)
        ? result.items.map((entry: unknown) => {
            const row = objectValue(entry)
            return {
              seq: numberValue(row.seq),
              at_ms: numberValue(row.at_ms),
              kind: textValue(row.kind),
              detail: textValue(row.detail),
            }
          })
        : []
    }
    behaviorState.value = 'ready'
  } catch (error) {
    if (isMethodMissing(error)) {
      behaviorState.value = 'unsupported'
      return
    }
    behaviorState.value = 'error'
  }
}

function kindLabel(kind: string): string {
  const key: Record<string, string> = {
    user_message: 'userMessage',
    assistant_reply: 'assistantReply',
    tool_call: 'toolCall',
    tool_result: 'toolResult',
    aborted: 'aborted',
  }
  return t(`settings.crossProjectMemory.kinds.${key[kind] || 'other'}`)
}

onMounted(() => {
  const anySupported = rpc.supportsMethod(SEARCH_METHOD)
    || rpc.supportsMethod(BEHAVIOR_LIST_METHOD)
    || rpc.supportsMethod(BEHAVIOR_SUMMARY_METHOD)
  if (!anySupported) {
    state.value = 'unsupported'
    behaviorState.value = 'unsupported'
    return
  }
  state.value = 'ready'
  void loadBehavior()
})
</script>

<template>
  <section class="cross-memory" data-testid="settings-cross-project-memory">
    <header class="cross-memory__head">
      <h3 class="cross-memory__title">{{ t('settings.crossProjectMemory.title') }}</h3>
      <p class="cross-memory__description">{{ t('settings.crossProjectMemory.description') }}</p>
    </header>

    <div v-if="state === 'unsupported'" class="cross-memory__notice" role="status">
      <Icon name="info" :size="18" aria-hidden="true" />
      <div>
        <h4>{{ t('settings.crossProjectMemory.upgradeTitle') }}</h4>
        <p>{{ t('settings.crossProjectMemory.upgradeDescription') }}</p>
      </div>
    </div>

    <template v-else>
      <!-- 检索：标题级摘要（渐进披露第 1 步） -->
      <form class="cross-memory__search" role="search" @submit.prevent="search">
        <input
          v-model="query"
          type="search"
          class="cross-memory__input"
          :placeholder="t('settings.crossProjectMemory.searchPlaceholder')"
          :aria-label="t('settings.crossProjectMemory.searchPlaceholder')"
          data-testid="cross-memory-search-input"
        />
        <button
          type="submit"
          class="btn btn--primary"
          :disabled="searching || !query.trim()"
          data-testid="cross-memory-search-button"
        >
          <span v-if="searching" class="cross-memory__spinner" aria-hidden="true"></span>
          <Icon v-else name="search" :size="15" aria-hidden="true" />
          {{ searching ? t('settings.crossProjectMemory.searching') : t('settings.crossProjectMemory.searchAction') }}
        </button>
      </form>

      <div v-if="state === 'error'" class="cross-memory__notice cross-memory__notice--error" role="alert">
        <Icon name="info" :size="18" aria-hidden="true" />
        <div>
          <h4>{{ t('settings.crossProjectMemory.searchErrorTitle') }}</h4>
          <p>{{ errorCode || t('settings.crossProjectMemory.searchError') }}</p>
        </div>
      </div>

      <div v-if="items.length" class="cross-memory__results" aria-live="polite">
        <article
          v-for="hit in items"
          :key="`${hit.project}:${hit.file}:${hit.line}`"
          class="cross-memory__hit"
        >
          <button
            type="button"
            class="cross-memory__hit-main"
            :aria-expanded="expandedState(expandedKey(hit))?.status === 'ready' ? 'true' : 'false'"
            data-testid="cross-memory-hit"
            @click="toggleExpand(hit)"
          >
            <div class="cross-memory__hit-copy">
              <strong class="cross-memory__hit-title">{{ hit.title || hit.excerpt }}</strong>
              <span class="cross-memory__hit-meta">
                {{ hit.project }} / {{ hit.file }} · L{{ hit.line }} · {{ formatScore(hit.score) }}%
              </span>
              <span v-if="hit.excerpt" class="cross-memory__hit-excerpt">{{ hit.excerpt }}</span>
            </div>
            <Icon
              :name="expandedState(expandedKey(hit))?.status === 'ready' ? 'arrowUp' : 'chevronDown'"
              :size="16"
              aria-hidden="true"
            />
          </button>
          <div
            v-if="expandedState(expandedKey(hit))?.status === 'loading'"
            class="cross-memory__context cross-memory__context--loading"
          >
            <LoadingSpinner />
          </div>
          <div
            v-else-if="expandedState(expandedKey(hit))?.status === 'ready'"
            class="cross-memory__context"
            data-testid="cross-memory-context"
          >
            <p v-if="expandedState(expandedKey(hit))?.error" class="cross-memory__context-error">
              {{ expandedState(expandedKey(hit))?.error }}
            </p>
            <pre
              v-for="row in expandedState(expandedKey(hit))?.context ?? []"
              :key="row.line"
              class="cross-memory__context-line"
              :class="{ 'is-hit': row.line === hit.line }"
            ><span class="cross-memory__context-no">{{ row.line }}</span>{{ row.text }}</pre>
          </div>
        </article>
      </div>

      <p
        v-else-if="state === 'ready' && query.trim()"
        class="cross-memory__empty"
        data-testid="cross-memory-empty"
      >{{ t('settings.crossProjectMemory.noResults') }}</p>

      <!-- 行为时间线（observation / timeline / summary 三层） -->
      <div class="cross-memory__behavior">
        <h4 class="cross-memory__behavior-title">{{ t('settings.crossProjectMemory.behaviorTitle') }}</h4>
        <p v-if="behaviorState === 'unsupported'" class="cross-memory__empty">
          {{ t('settings.crossProjectMemory.upgradeDescription') }}
        </p>
        <template v-else-if="behaviorState === 'ready'">
          <dl v-if="behaviorSummary" class="cross-memory__stats">
            <div>
              <dt>{{ t('settings.crossProjectMemory.events') }}</dt>
              <dd>{{ behaviorCount }}</dd>
            </div>
            <div>
              <dt>{{ t('settings.crossProjectMemory.turns') }}</dt>
              <dd>{{ behaviorSummary.turns }}</dd>
            </div>
          </dl>
          <ul v-if="toolStats.length" class="cross-memory__tools">
            <li v-for="stat in toolStats" :key="stat.tool">
              <code>{{ stat.tool }}</code>
              <span>× {{ stat.count }}</span>
            </li>
          </ul>
          <ol v-if="timeline.length" class="cross-memory__timeline" data-testid="cross-memory-timeline">
            <li v-for="event in timeline" :key="event.seq">
              <span class="cross-memory__timeline-kind">{{ kindLabel(event.kind) }}</span>
              <span class="cross-memory__timeline-time">{{ formatTime(event.at_ms) }}</span>
              <span class="cross-memory__timeline-detail">{{ event.detail }}</span>
            </li>
          </ol>
          <p v-else class="cross-memory__empty">{{ t('settings.crossProjectMemory.noBehavior') }}</p>
        </template>
      </div>
    </template>
  </section>
</template>

<style scoped>
.cross-memory {
  display: flex;
  flex-direction: column;
  gap: var(--sp-4);
  max-width: 880px;
}

.cross-memory__head {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}

.cross-memory__title,
.cross-memory__head h3,
.cross-memory__hit-title,
.cross-memory__behavior-title,
.cross-memory__notice h4 {
  color: var(--text);
  margin: 0;
}

.cross-memory__title {
  font-size: var(--fs-lg);
}

.cross-memory__description,
.cross-memory__hit-meta,
.cross-memory__hit-excerpt,
.cross-memory__empty,
.cross-memory__stats dd,
.cross-memory__timeline-detail,
.cross-memory__notice p {
  color: var(--text-muted);
  font-size: var(--fs-sm);
  line-height: 1.5;
}

.cross-memory__search {
  align-items: center;
  display: flex;
  gap: var(--sp-2);
}

.cross-memory__input {
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  color: var(--text);
  flex: 1 1 auto;
  font: inherit;
  font-size: var(--fs-sm);
  min-height: 36px;
  padding: 0 var(--sp-3);
}

.cross-memory__input:focus-visible {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--focus-ring);
  outline: none;
}

.cross-memory__notice {
  align-items: flex-start;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  display: flex;
  gap: var(--sp-3);
  padding: var(--sp-4);
}

.cross-memory__notice > div {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: var(--sp-1);
}

.cross-memory__notice--error {
  background: color-mix(in srgb, var(--danger) 8%, var(--bg-surface));
  border-color: color-mix(in srgb, var(--danger) 32%, var(--border));
}

.cross-memory__results {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}

.cross-memory__hit {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  overflow: clip;
}

.cross-memory__hit-main {
  align-items: center;
  background: var(--bg-elevated);
  border: 0;
  color: inherit;
  cursor: pointer;
  display: flex;
  font: inherit;
  gap: var(--sp-3);
  justify-content: space-between;
  padding: var(--sp-3) var(--sp-4);
  text-align: left;
  width: 100%;
}

.cross-memory__hit-main:hover {
  background: var(--bg-hover);
}

.cross-memory__hit-copy {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.cross-memory__hit-title {
  font-size: var(--fs-sm);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cross-memory__hit-meta {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
}

.cross-memory__hit-excerpt {
  font-size: var(--fs-xs);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cross-memory__context {
  background: var(--bg);
  display: flex;
  flex-direction: column;
  font-family: var(--font-mono);
  font-size: 12px;
  gap: 0;
  line-height: 1.6;
  padding: var(--sp-2) 0;
}

.cross-memory__context--loading {
  align-items: center;
  justify-content: center;
  min-height: 48px;
}

.cross-memory__context-line {
  color: var(--text-muted);
  display: grid;
  grid-template-columns: 48px minmax(0, 1fr);
  margin: 0;
  padding: 0 var(--sp-4) 0 0;
  white-space: pre-wrap;
}

.cross-memory__context-line.is-hit {
  background: color-mix(in srgb, var(--accent) 10%, transparent);
  color: var(--text);
  font-weight: 600;
}

.cross-memory__context-no {
  color: var(--text-dim);
  padding-right: var(--sp-2);
  text-align: right;
  user-select: none;
}

.cross-memory__context-error {
  color: var(--danger);
  font-family: var(--font-sans);
  font-size: var(--fs-xs);
  margin: 0;
  padding: 0 var(--sp-4) var(--sp-2);
}

.cross-memory__empty {
  margin: 0;
}

.cross-memory__behavior {
  border-top: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  padding-top: var(--sp-4);
}

.cross-memory__behavior-title {
  font-size: var(--fs-md);
}

.cross-memory__stats {
  display: flex;
  gap: var(--sp-5);
  margin: 0;
}

.cross-memory__stats > div {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.cross-memory__stats dt {
  color: var(--text-dim);
  font-size: var(--fs-xs);
}

.cross-memory__stats dd {
  font-size: var(--fs-md);
  font-weight: 600;
  margin: 0;
}

.cross-memory__tools {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
  list-style: none;
  margin: 0;
  padding: 0;
}

.cross-memory__tools li {
  align-items: center;
  background: color-mix(in srgb, var(--accent) 8%, var(--bg-surface));
  border: 1px solid color-mix(in srgb, var(--accent) 22%, var(--border));
  border-radius: var(--radius-pill);
  color: var(--text-muted);
  display: inline-flex;
  font-size: var(--fs-xs);
  gap: var(--sp-1);
  padding: 3px var(--sp-2);
}

.cross-memory__tools code {
  color: var(--text);
}

.cross-memory__timeline {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  list-style: none;
  margin: 0;
  padding: 0;
}

.cross-memory__timeline li {
  align-items: baseline;
  display: grid;
  gap: var(--sp-2);
  grid-template-columns: 96px 72px minmax(0, 1fr);
}

.cross-memory__timeline-kind {
  color: var(--accent);
  font-size: var(--fs-xs);
  font-weight: 600;
}

.cross-memory__timeline-time {
  color: var(--text-dim);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
}

.cross-memory__timeline-detail {
  font-size: var(--fs-xs);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cross-memory__spinner {
  animation: cross-memory-spin 0.8s linear infinite;
  border: 2px solid currentColor;
  border-radius: 50%;
  border-top-color: transparent;
  display: inline-block;
  height: 14px;
  width: 14px;
}

@keyframes cross-memory-spin {
  to { transform: rotate(360deg); }
}

@media (max-width: 640px) {
  .cross-memory__search {
    align-items: stretch;
    flex-direction: column;
  }

  .cross-memory__timeline li {
    grid-template-columns: 1fr;
  }

  .cross-memory__timeline-time {
    display: none;
  }
}
</style>
