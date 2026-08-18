<script setup lang="ts">
import { onMounted, ref, shallowRef } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

const { t } = useI18n()
const rpc = useRpcStore()

interface KbHit { docId: string; title: string; chunk: string; score: number }
interface KbDoc { doc_id: string; title: string; chunks: number }

const state = ref<'loading' | 'ready' | 'unsupported' | 'error'>('loading')
const query = ref('')
const searching = ref(false)
const hasSearched = ref(false)
const items = shallowRef<KbHit[]>([])
const documents = ref(0)
const chunks = ref(0)
const engine = ref('')
const docList = ref<KbDoc[]>([])
const showIngest = ref(false)
const ingestTitle = ref('')
const ingestText = ref('')
const ingesting = ref(false)
const ingestMsg = ref('')

function ov(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' ? value as Record<string, unknown> : {}
}
function tv(value: unknown): string { return typeof value === 'string' ? value : '' }
function nv(value: unknown): number { return typeof value === 'number' && Number.isFinite(value) ? value : 0 }

async function loadStatus() {
  try {
    const result = ov(await rpc.call('kb.status', {}))
    documents.value = nv(result.documents)
    chunks.value = nv(result.chunks)
    engine.value = tv(result.engine) || 'bm25'
    state.value = 'ready'
    void loadDocs()
  } catch { state.value = 'error' }
}

async function loadDocs() {
  try {
    const result = ov(await rpc.call('kb.list', {}))
    docList.value = Array.isArray(result.documents) ? result.documents as KbDoc[] : []
  } catch { /* ignore */ }
}

async function search() {
  const q = query.value.trim()
  if (!q || searching.value) return
  searching.value = true
  hasSearched.value = true
  try {
    const result = ov(await rpc.call('kb.search', { query: q, topK: 8 }))
    items.value = Array.isArray(result.items)
      ? result.items.map((v: unknown) => {
          const d = ov(v)
          return { docId: tv(d.doc_id), title: tv(d.title) || tv(d.doc_id), chunk: tv(d.chunk), score: nv(d.score) }
        }).filter((h: KbHit) => h.docId)
      : []
  } catch { items.value = [] } finally { searching.value = false }
}

async function removeDoc(docId: string) {
  try {
    await rpc.call('kb.delete', { doc_id: docId })
    await loadStatus()
  } catch { /* ignore */ }
}

async function doIngest() {
  if (!ingestTitle.value.trim() || !ingestText.value.trim()) return
  ingesting.value = true
  ingestMsg.value = ''
  try {
    const result = ov(await rpc.call('kb.ingest', { title: ingestTitle.value.trim(), text: ingestText.value }))
    if (result.ok) {
      ingestMsg.value = '已灌入'
      ingestTitle.value = ''; ingestText.value = ''
      showIngest.value = false
      await loadStatus()
    } else {
      ingestMsg.value = tv(result.error) || '失败'
    }
  } catch (e) { ingestMsg.value = String(e) } finally { ingesting.value = false }
}

function formatScore(score: number): string { return (score * 100).toFixed(0) }

onMounted(() => {
  if (!rpc.supportsMethod('kb.status')) {
    state.value = 'unsupported'
    return
  }
  void loadStatus()
})
</script>

<template>
  <section class="kb-panel" data-testid="settings-knowledge-base">
    <header class="kb-panel__head">
      <h3 class="kb-panel__title">{{ t('settings.knowledgeBase.title') }}</h3>
      <p class="kb-panel__description">{{ t('settings.knowledgeBase.description') }}</p>
    </header>

    <div v-if="state === 'unsupported'" class="kb-panel__unsupported">
      <Icon name="info" :size="18" />
      <div>
        <p class="kb-panel__unsupported-title">{{ t('settings.knowledgeBase.upgradeTitle') }}</p>
        <p class="kb-panel__hint">{{ t('settings.knowledgeBase.upgradeDescription') }}</p>
      </div>
    </div>

    <template v-else>
      <div class="kb-panel__stats">
        <div class="kb-stat">
          <span class="kb-stat__num">{{ documents }}</span>
          <span class="kb-stat__label">{{ t('settings.knowledgeBase.documents', { count: documents }) }}</span>
        </div>
        <div class="kb-stat">
          <span class="kb-stat__num">{{ chunks }}</span>
          <span class="kb-stat__label">{{ t('settings.knowledgeBase.chunks', { count: chunks }) }}</span>
        </div>
        <div class="kb-stat">
          <span class="kb-stat__num">{{ engine }}</span>
          <span class="kb-stat__label">{{ t('settings.knowledgeBase.engine') }}</span>
        </div>
        <button class="btn btn--ghost btn--sm" @click="showIngest = !showIngest">
          <Icon name="plus" :size="14" /><span>灌入文档</span>
        </button>
      </div>

      <div v-if="showIngest" class="kb-ingest">
        <input v-model="ingestTitle" class="kb-input" placeholder="文档标题" />
        <textarea v-model="ingestText" class="kb-textarea" rows="4" placeholder="粘贴文档正文…"></textarea>
        <div class="kb-ingest__actions">
          <span v-if="ingestMsg" class="kb-ingest__msg">{{ ingestMsg }}</span>
          <button class="btn btn--primary btn--sm" :disabled="ingesting" @click="doIngest">灌入</button>
        </div>
      </div>

      <div class="kb-search">
        <div class="kb-search__bar">
          <Icon name="search" :size="16" />
          <input
            v-model="query"
            class="kb-search__input"
            data-testid="kb-search-input"
            :placeholder="t('settings.knowledgeBase.searchPlaceholder')"
            @keyup.enter="search"
          />
          <button class="btn btn--primary btn--sm" data-testid="kb-search-button" :disabled="searching" @click="search">
            {{ t('settings.knowledgeBase.searchAction') }}
          </button>
        </div>
        <div v-if="items.length" class="kb-results">
          <div v-for="(hit, i) in items" :key="i" class="kb-hit">
            <div class="kb-hit__head">
              <span class="kb-hit__title">{{ hit.title }}</span>
              <span class="kb-hit__score">{{ formatScore(hit.score) }}%</span>
            </div>
            <p class="kb-hit__chunk">{{ hit.chunk }}</p>
          </div>
        </div>
        <p v-else-if="hasSearched" class="kb-results__empty">{{ t('settings.knowledgeBase.noResults') }}</p>
      </div>

      <div v-if="docList.length" class="kb-docs">
        <h4 class="kb-docs__title">已索引文档</h4>
        <div v-for="doc in docList" :key="doc.doc_id" class="kb-doc">
          <span class="kb-doc__title">{{ doc.title }}</span>
          <span class="kb-doc__meta">{{ doc.chunks }} 块</span>
          <button class="kb-doc__del" @click="removeDoc(doc.doc_id)" title="删除">
            <Icon name="trash" :size="14" />
          </button>
        </div>
      </div>
    </template>
  </section>
</template>

<style scoped>
.kb-panel { display: flex; flex-direction: column; gap: 14px; }
.kb-panel__head { display: flex; flex-direction: column; gap: 4px; }
.kb-panel__title { font-size: 15px; font-weight: 600; margin: 0; }
.kb-panel__description { font-size: 12px; color: var(--text-muted); margin: 0; }
.kb-panel__unsupported { display: flex; gap: 12px; align-items: flex-start; padding: 14px; background: var(--bg); border-radius: var(--radius-md); border: 1px solid var(--hairline); }
.kb-panel__unsupported-title { font-size: 13px; font-weight: 600; margin: 0 0 4px; }
.kb-panel__hint { font-size: 12px; color: var(--text-muted); margin: 0; line-height: 1.5; }
.kb-panel__stats { display: flex; align-items: center; gap: 16px; padding: 12px; background: var(--bg); border-radius: var(--radius-md); border: 1px solid var(--hairline); }
.kb-stat { display: flex; flex-direction: column; align-items: center; }
.kb-stat__num { font-size: 18px; font-weight: 700; font-family: var(--font-mono, monospace); }
.kb-stat__label { font-size: 10px; color: var(--text-dim); }
.kb-ingest { display: flex; flex-direction: column; gap: 8px; padding: 12px; background: var(--bg); border-radius: var(--radius-md); border: 1px solid var(--hairline); }
.kb-input { padding: 7px 10px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--card); color: var(--text); font-size: 12px; outline: none; }
.kb-input:focus { border-color: var(--accent); }
.kb-textarea { padding: 8px 10px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--card); color: var(--text); font-size: 12px; outline: none; resize: vertical; }
.kb-textarea:focus { border-color: var(--accent); }
.kb-ingest__actions { display: flex; justify-content: flex-end; align-items: center; gap: 8px; }
.kb-ingest__msg { font-size: 11px; color: var(--ok); }
.kb-search__bar { display: flex; align-items: center; gap: 8px; }
.kb-search__input { flex: 1; padding: 7px 10px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--card); color: var(--text); font-size: 12px; outline: none; }
.kb-search__input:focus { border-color: var(--accent); }
.kb-results { display: flex; flex-direction: column; gap: 8px; margin-top: 10px; }
.kb-results__empty { font-size: 12px; color: var(--text-muted); margin: 10px 0 0; }
.kb-hit { padding: 10px; background: var(--bg); border-radius: var(--radius-sm); border: 1px solid var(--hairline); }
.kb-hit__head { display: flex; justify-content: space-between; align-items: center; }
.kb-hit__title { font-size: 12px; font-weight: 600; }
.kb-hit__score { font-size: 10px; color: var(--accent); font-family: var(--font-mono, monospace); }
.kb-hit__chunk { font-size: 11px; color: var(--text-muted); margin: 4px 0 0; line-height: 1.5; }
.kb-docs { display: flex; flex-direction: column; gap: 4px; }
.kb-docs__title { font-size: 12px; font-weight: 600; margin: 0 0 4px; }
.kb-doc { display: flex; align-items: center; gap: 8px; padding: 6px 10px; background: var(--bg); border-radius: var(--radius-sm); font-size: 11px; }
.kb-doc__title { flex: 1; }
.kb-doc__meta { color: var(--text-dim); font-size: 10px; }
.kb-doc__del { background: none; border: none; color: var(--text-dim); cursor: pointer; padding: 2px; display: flex; }
.kb-doc__del:hover { color: var(--danger); }
</style>
