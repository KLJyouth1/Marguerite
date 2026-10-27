<template>
  <div class="knowledge-view">
    <header class="kb-head">
      <h1>{{ t('knowledgePages.title') }}</h1>
      <p class="kb-sub">{{ t('knowledgePages.subtitle') }}</p>
    </header>

    <section class="kb-card">
      <h2>{{ t('knowledgePages.add.title') }}</h2>
      <input v-model="form.title" :placeholder="t('knowledgePages.add.titlePlaceholder')" />
      <input v-model="form.tags" :placeholder="t('knowledgePages.add.tagsPlaceholder')" />
      <textarea v-model="form.content" rows="4" :placeholder="t('knowledgePages.add.contentPlaceholder')"></textarea>
      <button :disabled="!form.title.trim() || saving" @click="addPage">{{ t('knowledgePages.add.submit') }}</button>
      <p v-if="notice" :class="noticeOK ? 'ok' : 'err'">{{ notice }}</p>
    </section>

    <section class="kb-card">
      <h2>{{ t('knowledgePages.search.title') }}</h2>
      <div class="row">
        <input v-model="query" :placeholder="t('knowledgePages.search.placeholder')" @keyup.enter="doSearch" />
        <button :disabled="!query.trim()" @click="doSearch">{{ t('knowledgePages.search.submit') }}</button>
      </div>
      <ul v-if="searched">
        <li v-for="p in results" :key="p.id">
          <strong>{{ p.title }}</strong> <span class="score">[{{ (p.score ?? 0).toFixed(2) }}]</span>
          <pre>{{ p.content }}</pre>
        </li>
        <li v-if="results.length === 0">{{ t('knowledgePages.search.noMatch') }}</li>
      </ul>
    </section>

    <section class="kb-card">
      <h2>{{ t('knowledgePages.hybrid.title') }}</h2>
      <div class="row">
        <input v-model="hybridQuery" :placeholder="t('knowledgePages.hybrid.placeholder')" @keyup.enter="doHybridSearch" />
        <button :disabled="!hybridQuery.trim() || hybridBusy" @click="doHybridSearch">{{ t('knowledgePages.hybrid.submit') }}</button>
      </div>
      <p v-if="hybridEngine" class="kb-sub">{{ t('knowledgePages.hybrid.engine', { engine: hybridEngine }) }}</p>
      <ul v-if="hybridSearched">
        <li v-for="(p, i) in hybridResults" :key="i">
          <strong>{{ p.title }}</strong> <span class="score">{{ t('knowledgePages.hybrid.scores', { bm25: p.bm25_score.toFixed(2), vector: p.vector_score.toFixed(2), fused: p.fused_score.toFixed(2) }) }}</span>
          <pre>{{ p.chunk }}</pre>
        </li>
        <li v-if="hybridResults.length === 0">{{ t('knowledgePages.search.noMatch') }}</li>
      </ul>
    </section>

    <section class="kb-card" data-testid="kb-quality">
      <h2>{{ t('kb.quality.title') }}</h2>
      <p v-if="kbUnsupported" class="kb-sub">{{ t('kb.quality.unsupported') }}</p>
      <template v-else>
        <div class="kb-quality">
          <div class="kb-quality__stat">
            <span class="kb-quality__num">{{ kbStatus.documents }}</span>
            <span class="kb-quality__label">{{ t('kb.quality.documents') }}</span>
          </div>
          <div class="kb-quality__stat">
            <span class="kb-quality__num">{{ kbStatus.chunks }}</span>
            <span class="kb-quality__label">{{ t('kb.quality.chunks') }}</span>
          </div>
          <div class="kb-quality__stat">
            <span class="kb-quality__num">{{ kbStatus.engine || '—' }}</span>
            <span class="kb-quality__label">{{ t('kb.quality.engine') }}</span>
          </div>
          <div class="kb-quality__stat">
            <span class="kb-quality__num">{{ kbStatus.duplicatesRejected }}</span>
            <span class="kb-quality__label">{{ t('kb.quality.duplicatesRejected') }}</span>
          </div>
          <div class="kb-quality__stat">
            <span class="kb-quality__num">{{ kbStatus.denoisedDropped }}</span>
            <span class="kb-quality__label">{{ t('kb.quality.denoisedDropped') }}</span>
          </div>
          <div class="kb-quality__stat">
            <span class="kb-quality__num">{{ kbStatus.rejected }}</span>
            <span class="kb-quality__label">{{ t('kb.quality.rejected') }}</span>
          </div>
          <div class="kb-quality__stat">
            <span class="kb-quality__num">{{ formatBytes(kbStatus.sizeBytes) }}</span>
            <span class="kb-quality__label">{{ t('kb.quality.size') }}</span>
          </div>
          <div class="kb-quality__stat">
            <span class="kb-quality__num">
              {{ kbStatus.persisted ? t('kb.quality.persisted') : t('kb.quality.notPersisted') }}
            </span>
            <span class="kb-quality__label">{{ t('kb.quality.persistence') }}</span>
          </div>
        </div>
        <p class="kb-sub kb-quality__path">
          {{ t('kb.quality.storePath', { path: kbStatus.storePath || '—' }) }}
        </p>
        <p v-if="kbStatus.lastIngestAt" class="kb-sub">
          {{ t('kb.quality.lastIngestAt', { at: kbStatus.lastIngestAt }) }}
        </p>
        <p v-if="kbError" class="err" role="alert">{{ kbError }}</p>

        <div class="row kb-import">
          <input
            v-model="importPath"
            :placeholder="t('kb.quality.importPlaceholder')"
            data-testid="kb-import-path"
          />
          <button
            :disabled="!importPath.trim() || importing || !rpc.supportsMethod('kb.ingest_file')"
            @click="importFile"
          >
            {{ t('kb.quality.import') }}
          </button>
        </div>
        <p v-if="importNotice" :class="importOK ? 'ok' : 'err'" role="status">{{ importNotice }}</p>

        <ul v-if="kbDocs.length" class="kb-docs">
          <li v-for="doc in kbDocs" :key="doc.doc_id">
            <strong>{{ doc.title }}</strong>
            <span class="meta">{{ t('kb.quality.docMeta', { chunks: doc.chunks }) }}</span>
            <code class="kb-hash">{{ (doc.hash || '').slice(0, 12) }}</code>
            <button class="kb-del" :aria-label="t('kb.quality.delete')" @click="deleteDoc(doc)">
              {{ t('kb.quality.delete') }}
            </button>
          </li>
        </ul>
        <p v-else class="kb-sub">{{ t('kb.quality.empty') }}</p>
      </template>
    </section>

    <section class="kb-card">
      <h2>{{ t('knowledgePages.all.title', { count: pages.length }) }}</h2>
      <ul>
        <li v-for="p in pages" :key="p.id">
          <strong>{{ p.title }}</strong>
          <span class="tags">{{ (p.tags || []).join(', ') }}</span>
          <span class="meta">{{ t('knowledgePages.all.usage', { count: p.useCount ?? 0 }) }}</span>
        </li>
        <li v-if="pages.length === 0">{{ t('knowledgePages.all.empty') }}</li>
      </ul>
    </section>
  </div>
</template>

<script setup lang="ts">
import { onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRpcStore } from '@/stores/rpc'

const { t } = useI18n()

interface PageInfo {
  id: string
  title: string
  tags?: string[]
  useCount?: number
  updatedAt?: number
  content?: string
  score?: number
}

const rpc = useRpcStore()
const pages = ref<PageInfo[]>([])
const results = ref<PageInfo[]>([])
const searched = ref(false)
const notice = ref('')
const noticeOK = ref(true)
const saving = ref(false)
const query = ref('')
const form = reactive({ title: '', tags: '', content: '' })

async function load(): Promise<void> {
  try {
    const d = await rpc.call<{ pages: PageInfo[] }>('pages.list', {})
    pages.value = d.pages || []
  } catch {
    pages.value = []
  }
}

interface HybridHit { title: string; chunk: string; bm25_score: number; vector_score: number; fused_score: number }
const hybridQuery = ref('')
const hybridBusy = ref(false)
const hybridResults = ref<HybridHit[]>([])
const hybridSearched = ref(false)
const hybridEngine = ref('')

/* ── 知识库质量报告 / 文档列表 / 从文件导入 ─────────────────────────── */

interface KbStatus {
  documents: number
  chunks: number
  engine: string
  persisted: boolean
  storePath: string
  duplicatesRejected: number
  denoisedDropped: number
  rejected: number
  lastIngestAt: string
  sizeBytes: number
}

interface KbDoc {
  doc_id: string
  title: string
  chunks: number
  hash: string
}

function emptyKbStatus(): KbStatus {
  return {
    documents: 0,
    chunks: 0,
    engine: '',
    persisted: false,
    storePath: '',
    duplicatesRejected: 0,
    denoisedDropped: 0,
    rejected: 0,
    lastIngestAt: '',
    sizeBytes: 0,
  }
}

const kbStatus = ref<KbStatus>(emptyKbStatus())
const kbDocs = ref<KbDoc[]>([])
const kbUnsupported = ref(false)
const kbError = ref('')
const importPath = ref('')
const importing = ref(false)
const importNotice = ref('')
const importOK = ref(true)

function obj(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' ? value as Record<string, unknown> : {}
}
function str(value: unknown): string {
  return typeof value === 'string' ? value : ''
}
function num(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

function normalizeKbStatus(value: unknown): KbStatus {
  const d = obj(value)
  return {
    documents: num(d.documents),
    chunks: num(d.chunks),
    engine: str(d.engine),
    persisted: d.persisted === true,
    storePath: str(d.storePath) || str(d.path),
    duplicatesRejected: num(d.duplicatesRejected ?? d.duplicateRejected),
    denoisedDropped: num(d.denoisedDropped ?? d.denoisedRejected),
    rejected: num(d.rejected),
    lastIngestAt: str(d.lastIngestAt),
    sizeBytes: num(d.sizeBytes),
  }
}

function formatBytes(bytes: number): string {
  if (bytes <= 0) return '0 B'
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

async function loadKb(): Promise<void> {
  if (!rpc.supportsMethod('kb.status')) {
    kbUnsupported.value = true
    return
  }
  kbError.value = ''
  try {
    kbStatus.value = normalizeKbStatus(await rpc.call('kb.status', {}))
    const listed = obj(await rpc.call('kb.list', {}))
    const docs = Array.isArray(listed.documents) ? listed.documents : []
    kbDocs.value = docs.map((entry) => {
      const d = obj(entry)
      return {
        doc_id: str(d.doc_id),
        title: str(d.title) || str(d.doc_id),
        chunks: num(d.chunks),
        hash: str(d.hash),
      }
    })
  } catch {
    kbError.value = t('kb.quality.failed')
  }
}

async function deleteDoc(doc: KbDoc): Promise<void> {
  if (!window.confirm(t('kb.quality.deleteConfirm', { title: doc.title }))) return
  try {
    const res = obj(await rpc.call('kb.delete', { doc_id: doc.doc_id }))
    if (res.ok === false) {
      kbError.value = str(res.error) || t('kb.quality.failed')
      return
    }
    await loadKb()
  } catch {
    kbError.value = t('kb.quality.failed')
  }
}

async function importFile(): Promise<void> {
  const path = importPath.value.trim()
  if (!path || importing.value) return
  importing.value = true
  importNotice.value = ''
  try {
    const res = obj(await rpc.call('kb.ingest_file', { path }))
    if (res.ok === false) {
      importOK.value = false
      const code = str(res.code)
      importNotice.value = code === 'PATH_OUTSIDE_ROOTS'
        ? t('kb.quality.outsideRoots', { roots: Array.isArray(res.roots) ? res.roots.join(', ') : '' })
        : `${code ? `[${code}] ` : ''}${str(res.error) || t('kb.quality.importFailed')}`
      return
    }
    importOK.value = true
    importNotice.value = res.deduped === true
      ? t('kb.quality.deduped', { id: str(res.doc_id) })
      : t('kb.quality.imported', { id: str(res.doc_id) })
    importPath.value = ''
    await loadKb()
  } catch {
    importOK.value = false
    importNotice.value = t('kb.quality.importFailed')
  } finally {
    importing.value = false
  }
}

async function doHybridSearch() {
  if (!hybridQuery.value.trim()) return
  hybridBusy.value = true
  try {
    const r = await rpc.call<{ engine: string; items: HybridHit[] }>('kb.search.hybrid', { query: hybridQuery.value.trim(), topK: 5 })
    hybridResults.value = r.items ?? []
    hybridEngine.value = r.engine ?? ''
    hybridSearched.value = true
  } catch { hybridResults.value = [] } finally { hybridBusy.value = false }
}

onMounted(() => {
  void load()
  void loadKb()
})

async function addPage(): Promise<void> {
  saving.value = true
  notice.value = ''
  try {
    const tags = form.tags.split(',').map((s) => s.trim()).filter(Boolean)
    const d = await rpc.call<{ success: boolean; message: string }>('pages.add', {
      title: form.title.trim(),
      content: form.content,
      tags,
    })
    noticeOK.value = !!d.success
    notice.value = d.message || (d.success ? t('knowledgePages.saved') : t('knowledgePages.saveFailed'))
    if (d.success) {
      form.title = ''
      form.tags = ''
      form.content = ''
      await load()
    }
  } catch {
    noticeOK.value = false
    notice.value = t('knowledgePages.callFailed')
  } finally {
    saving.value = false
  }
}

async function doSearch(): Promise<void> {
  searched.value = true
  try {
    const d = await rpc.call<{ pages: PageInfo[] }>('pages.search', { q: query.value, topK: 10 })
    results.value = d.pages || []
  } catch {
    results.value = []
  }
}
</script>

<style scoped>
.knowledge-view { padding: 16px 20px 48px; max-width: 860px; }
.kb-head h1 { margin: 0 0 4px; font-size: 22px; font-weight: 650; }
.kb-sub { margin: 0 0 16px; opacity: 0.75; font-size: 13px; }
.kb-card { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 14px 16px; margin-bottom: 14px; }
.kb-card h2 { margin: 0 0 10px; font-size: 15px; font-weight: 600; }
input, textarea { width: 100%; box-sizing: border-box; margin: 4px 0 8px; padding: 7px 10px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--bg); color: var(--text); font: inherit; }
button { padding: 7px 14px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: transparent; color: inherit; cursor: pointer; }
button:disabled { opacity: 0.45; cursor: default; }
.row { display: flex; gap: 8px; align-items: center; }
.row input { flex: 1; }
ul { margin: 8px 0 0; padding-left: 18px; }
li { margin: 6px 0; }
pre { background: color-mix(in srgb, var(--bg) 80%, var(--card)); border: 1px solid var(--hairline); border-radius: var(--radius-sm); padding: 8px 10px; white-space: pre-wrap; margin: 4px 0 0; font-size: 12px; }
.score { opacity: 0.7; margin-left: 6px; }
.tags, .meta { opacity: 0.65; margin-left: 8px; font-size: 12px; }
.ok { color: var(--accent); }
.err { color: var(--danger); }
.kb-quality { display: flex; flex-wrap: wrap; gap: 8px; margin-bottom: 8px; }
.kb-quality__stat { display: flex; flex-direction: column; align-items: center; gap: 2px; min-width: 84px; padding: 6px 10px; border: 1px solid var(--hairline); border-radius: var(--radius-md); background: var(--bg); }
.kb-quality__num { font-family: var(--font-mono, monospace); font-size: 14px; font-weight: 650; color: var(--text); }
.kb-quality__label { font-size: 11px; color: var(--text-dim); }
.kb-quality__path { word-break: break-all; }
.kb-import { margin-top: 8px; }
.kb-docs { list-style: none; padding: 0; margin: 10px 0 0; }
.kb-docs li { display: flex; align-items: center; gap: 8px; padding: 5px 8px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); margin: 4px 0; }
.kb-hash { font-family: var(--font-mono, monospace); font-size: 11px; color: var(--text-dim); }
.kb-del { margin-left: auto; padding: 2px 8px; font-size: 11px; }
</style>
