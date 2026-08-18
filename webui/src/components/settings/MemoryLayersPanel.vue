<script setup lang="ts">
import { onMounted, ref, shallowRef } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import LoadingSpinner from '@/components/LoadingSpinner.vue'
import { useRpcStore } from '@/stores/rpc'

const STATUS_METHOD = 'memory.layers.status'
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

onMounted(() => void load())
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
  min-height: 100%;
}
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
</style>
