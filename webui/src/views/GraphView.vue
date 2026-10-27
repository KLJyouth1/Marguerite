<template>
  <div class="control-stage graph-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('graph.title') }}</h1>
        <p class="control-stage__subtitle">
          {{ t('graph.subtitle') }}<span v-if="graph">{{ t('graph.subtitleCount', { entities: graph.entity_count, relations: graph.edge_count }) }}</span>
        </p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" @click="load">
          <Icon name="refresh" :size="16" /><span>{{ t('graph.refresh') }}</span>
        </button>
      </div>
    </header>

    <section v-if="error" class="graph-error">{{ error }}</section>

    <!-- 灌入知识条目（同步进 BM25 检索 + 关系图谱） -->
    <section class="graph-ingest">
      <input v-model="seedTitle" class="graph-input" :placeholder="t('graph.seedTitlePlaceholder')" />
      <textarea v-model="seedText" class="graph-textarea" :placeholder="t('graph.seedTextPlaceholder')"></textarea>
      <button class="btn btn--primary" :disabled="!seedTitle.trim() || !seedText.trim() || ingesting" @click="ingest">
        <Icon name="plus" :size="16" /><span>{{ ingesting ? t('graph.ingesting') : t('graph.addEntry') }}</span>
      </button>
    </section>

    <!-- 图谱 SVG -->
    <section v-if="nodes.length > 0" class="graph-canvas">
      <svg viewBox="0 0 800 460" class="graph-svg">
        <line
          v-for="(e, i) in edges"
          :key="'e' + i"
          :x1="posOf(e.from).x" :y1="posOf(e.from).y"
          :x2="posOf(e.to).x" :y2="posOf(e.to).y"
          :stroke="edgeColor(e.kind)"
          :stroke-width="Math.min(3, 0.6 + e.weight * 0.7)"
          :opacity="selected ? (e.from === selected || e.to === selected ? 0.95 : 0.12) : 0.55"
        />
        <g v-for="n in sortedNodes" :key="n.id" class="graph-node" @click="pick(n.id)">
          <circle
            :cx="posOf(n.id).x" :cy="posOf(n.id).y" :r="posOf(n.id).r"
            :fill="selected === n.id ? 'var(--accent)' : (relatedSet.has(n.id) ? 'var(--info)' : 'var(--bg-hover)')"
            :stroke="selected === n.id ? 'var(--accent)' : 'var(--text-dim)'"
            :stroke-width="selected === n.id ? 2.5 : 1.2"
          />
          <text
            :x="posOf(n.id).x" :y="posOf(n.id).y + posOf(n.id).r + 13"
            text-anchor="middle" class="graph-node-label"
            :fill="selected === n.id ? 'var(--accent)' : 'var(--text-muted)'"
            :font-weight="selected === n.id ? 700 : 400"
          >{{ n.id.length > 16 ? n.id.slice(0, 15) + '…' : n.id }}</text>
        </g>
      </svg>
    </section>
    <section v-else class="graph-empty">
      {{ t('graph.empty') }}
    </section>

    <!-- 选中实体详情 -->
    <section v-if="selected" class="graph-detail">
      <div class="graph-detail__name">{{ selected }}</div>
      <div class="graph-detail__hint">{{ related.length > 0 ? t('graph.relatedCount', { count: related.length }) : t('graph.noRelated') }}</div>
      <div class="graph-detail__tags">
        <span v-for="r in related" :key="r" class="graph-detail__tag" @click="pick(r)">{{ r }}</span>
      </div>
      <div class="graph-detail__legend">{{ t('graph.legend') }}</div>
    </section>

    <!-- 项目级关系图谱（一项目一图） -->
    <section class="graph-project">
      <div class="graph-project__title">{{ t('graph.project.title') }}</div>
      <p class="graph-project__hint">{{ t('graph.project.hint') }}</p>
      <div class="graph-project__row">
        <input v-model="projectPath" class="graph-input graph-project__input" :placeholder="t('graph.project.pathPlaceholder')" @keyup.enter="analyzeProject" />
        <button class="btn btn--primary" :disabled="busy || !projectPath.trim()" @click="analyzeProject">{{ t('graph.project.analyze') }}</button>
        <button class="btn btn--ghost" :disabled="busy || !projectGraph" @click="saveProjectGraph">{{ t('graph.project.save') }}</button>
        <button class="btn btn--ghost" :disabled="busy || !projectGraph" @click="deleteCurrentProjectGraph">{{ t('graph.project.delete') }}</button>
      </div>
      <div v-if="projectNote" class="graph-project__note">{{ projectNote }}</div>
      <div v-if="projectGraph" class="graph-project__graph">
        <div class="graph-project__cols">
          <div class="graph-project__col">
            <div class="graph-project__sub">{{ t('graph.project.nodesTitle', { count: projectGraph.nodes.length }) }}</div>
            <div class="graph-project__pills">
              <span v-for="n in projectGraph.nodes" :key="n.id" class="graph-project__pill" :data-kind="n.kind" :title="n.id">{{ n.label || n.id }}</span>
            </div>
          </div>
          <div class="graph-project__col">
            <div class="graph-project__sub">{{ t('graph.project.edgesTitle', { count: projectGraph.edges.length }) }}</div>
            <ul class="graph-project__edges">
              <li v-for="(e, i) in projectGraph.edges" :key="i"><code>{{ e.from }}</code><span class="graph-project__ekind">{{ e.kind }}</span><code>{{ e.to }}</code></li>
            </ul>
          </div>
        </div>
      </div>
      <div class="graph-project__saved">
        <div v-if="savedProjects.length" class="graph-project__saved-title">{{ t('graph.project.savedTitle', { count: savedProjects.length }) }}</div>
        <div v-else class="graph-project__saved-empty">{{ t('graph.project.savedEmpty') }}</div>
        <div v-for="s in savedProjects" :key="s.key" class="graph-project__saved-row">
          <code class="graph-project__saved-path" :title="s.project">{{ s.project }}</code>
          <span class="graph-project__saved-meta">{{ t('graph.project.savedMeta', { nodes: s.nodes, edges: s.edges }) }}</span>
          <button class="btn btn--sm btn--ghost" :disabled="busy" @click="loadProjectGraph(s.project)">{{ t('graph.project.load') }}</button>
          <button class="btn btn--sm btn--ghost" :disabled="busy" @click="removeProjectGraph(s.project)">{{ t('common.delete') }}</button>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

const { t } = useI18n()
const rpc = useRpcStore()

interface GraphNode { id: string; degree: number }
interface GraphEdge { from: string; to: string; kind: string; weight: number }
interface GraphView { nodes: GraphNode[]; edges: GraphEdge[]; entity_count: number; edge_count: number }

const graph = ref<GraphView | null>(null)
const selected = ref<string | null>(null)
const related = ref<string[]>([])
const seedTitle = ref('')
const seedText = ref('')
const ingesting = ref(false)
const error = ref('')

const nodes = computed(() => graph.value?.nodes ?? [])
const edges = computed(() => graph.value?.edges ?? [])
const relatedSet = computed(() => new Set(related.value))

const sortedNodes = computed(() => [...nodes.value].sort((a, b) => b.degree - a.degree))

function posOf(id: string): { x: number; y: number; r: number } {
  const list = sortedNodes.value
  const idx = list.findIndex((n) => n.id === id)
  const cx = 400
  const cy = 230
  const radius = Math.max(120, 175)
  const angle = (idx / Math.max(1, list.length)) * Math.PI * 2 - Math.PI / 2
  const degree = list[idx]?.degree ?? 0
  return {
    x: cx + Math.cos(angle) * radius,
    y: cy + Math.sin(angle) * radius,
    r: Math.min(16, 5 + degree * 2.2),
  }
}

function edgeColor(kind: string): string {
  switch (kind) {
    case 'is_a': return 'var(--accent)'
    case 'uses': return 'var(--ok)'
    case 'implements': return 'var(--warn)'
    case 'provides': return 'var(--info)'
    default: return 'var(--hairline)'
  }
}

async function load() {
  error.value = ''
  try {
    graph.value = await rpc.call<GraphView>('kb.graph.view')
  } catch (e) {
    error.value = t('graph.loadFailed', { error: e instanceof Error ? e.message : String(e) })
  }
}

async function ingest() {
  if (!seedTitle.value.trim() || !seedText.value.trim()) return
  ingesting.value = true
  error.value = ''
  try {
    await rpc.call('kb.ingest', { title: seedTitle.value.trim(), text: seedText.value.trim() })
    seedTitle.value = ''
    seedText.value = ''
    await load()
  } catch (e) {
    error.value = t('graph.ingestFailed', { error: e instanceof Error ? e.message : String(e) })
  } finally {
    ingesting.value = false
  }
}

async function pick(id: string) {
  selected.value = id
  related.value = []
  try {
    const res = await rpc.call<{ related: string[] }>('kb.graph.related', { entity: id })
    related.value = res.related || []
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  }
}

onMounted(load)

interface ProjectNode { id: string; kind: string; label?: string | null }
interface ProjectEdge { from: string; to: string; kind: string }
interface ProjectGraphPayload { project: string; nodes: ProjectNode[]; edges: ProjectEdge[]; summary?: string; ok?: boolean; error?: string }
interface SavedProject { key: string; project: string; nodes: number; edges: number }

const projectPath = ref('')
const projectGraph = ref<ProjectGraphPayload | null>(null)
const projectNote = ref('')
const savedProjects = ref<SavedProject[]>([])
const busy = ref(false)

async function loadSavedProjects() {
  try {
    const r = await rpc.call<{ items: SavedProject[]; total: number }>('graph.project.list', {})
    savedProjects.value = r.items ?? []
  } catch { /* 静默 */ }
}

async function analyzeProject() {
  const p = projectPath.value.trim()
  if (!p) return
  busy.value = true
  projectNote.value = ''
  try {
    const r = await rpc.call<ProjectGraphPayload>('graph.project.analyze', { project: p, maxFiles: 200 })
    if (r.ok === false) {
      projectNote.value = r.error ?? t('graph.project.analyzeFailed')
    } else {
      projectGraph.value = r
      projectNote.value = r.summary ?? t('graph.project.analyzeDone', { nodes: r.nodes.length, edges: r.edges.length })
    }
  } catch (e) {
    projectNote.value = e instanceof Error ? e.message : String(e)
  } finally { busy.value = false }
}

async function saveProjectGraph() {
  const g = projectGraph.value
  if (!g) return
  busy.value = true
  try {
    const r = await rpc.call<{ ok: boolean; error?: string }>('graph.project.save', { project: g.project, nodes: g.nodes, edges: g.edges })
    projectNote.value = r.ok ? t('graph.project.saved') : (r.error ?? t('graph.project.saveFailed'))
    if (r.ok) await loadSavedProjects()
  } catch (e) {
    projectNote.value = e instanceof Error ? e.message : String(e)
  } finally { busy.value = false }
}

async function loadProjectGraph(project: string) {
  busy.value = true
  projectPath.value = project
  try {
    const r = await rpc.call<ProjectGraphPayload>('graph.project.view', { project })
    if (r.ok === false) { projectNote.value = r.error ?? t('graph.project.loadFailed') } else { projectGraph.value = r; projectNote.value = t('graph.project.loaded') }
  } catch (e) { projectNote.value = e instanceof Error ? e.message : String(e) } finally { busy.value = false }
}

async function deleteCurrentProjectGraph() {
  if (!projectGraph.value) return
  await removeProjectGraph(projectGraph.value.project)
}

async function removeProjectGraph(project: string) {
  busy.value = true
  try {
    const r = await rpc.call<{ ok: boolean; error?: string }>('graph.project.delete', { project })
    projectNote.value = r.ok ? t('graph.project.deleted') : (r.error ?? t('graph.project.deleteFailed'))
    if (r.ok && projectGraph.value?.project === project) projectGraph.value = null
    await loadSavedProjects()
  } catch (e) { projectNote.value = e instanceof Error ? e.message : String(e) } finally { busy.value = false }
}

onMounted(() => { void loadSavedProjects() })
</script>

<style scoped>
.graph-stage { padding: 24px 28px; max-width: 1200px; margin: 0 auto; }
.graph-error { margin: 16px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
.graph-ingest { display: flex; flex-direction: column; gap: 8px; margin-top: 16px; padding: 14px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); }
.graph-input { padding: 9px 12px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--bg); color: var(--text); font-size: 13px; outline: none; }
.graph-input:focus { border-color: var(--accent); }
.graph-textarea { min-height: 64px; padding: 9px 12px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--bg); color: var(--text); font-size: 12px; resize: vertical; outline: none; }
.graph-textarea:focus { border-color: var(--accent); }
.graph-canvas { margin-top: 16px; padding: 8px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); overflow: auto; }
.graph-svg { width: 100%; height: 420px; display: block; }
.graph-node { cursor: pointer; }
.graph-node-label { font-size: 11px; }
.graph-empty { margin-top: 16px; padding: 32px; text-align: center; color: var(--text-muted); font-size: 12px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); }
.graph-detail { margin-top: 16px; padding: 16px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); }
.graph-detail__name { font-weight: 700; font-size: 14px; color: var(--accent); }
.graph-detail__hint { font-size: 11px; color: var(--text-dim); margin-top: 4px; }
.graph-detail__tags { display: flex; gap: 6px; flex-wrap: wrap; margin-top: 8px; }
.graph-detail__tag { font-size: 11px; padding: 3px 8px; border-radius: var(--radius-pill); background: var(--bg-hover); border: 1px solid var(--hairline); cursor: pointer; }
.graph-detail__legend { font-size: 10px; color: var(--text-dim); margin-top: 10px; }
.graph-project { margin-top: 16px; padding: 16px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); }
.graph-project__title { font-size: 13px; font-weight: 700; }
.graph-project__hint { margin: 6px 0 0; font-size: 11px; color: var(--text-dim); }
.graph-project__row { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; margin-top: 10px; }
.graph-project__input { flex: 1; min-width: 260px; }
.graph-project__note { margin-top: 8px; font-size: 12px; color: var(--accent); word-break: break-all; }
.graph-project__graph { margin-top: 12px; border-top: 1px solid var(--hairline); padding-top: 12px; }
.graph-project__cols { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
.graph-project__sub { font-size: 11px; font-weight: 600; color: var(--text-dim); margin-bottom: 8px; }
.graph-project__pills { display: flex; flex-wrap: wrap; gap: 6px; }
.graph-project__pill { font-size: 11px; padding: 3px 9px; border-radius: 999px; background: var(--bg-hover); border: 1px solid var(--hairline); max-width: 220px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.graph-project__pill[data-kind="project"] { color: var(--accent); border-color: color-mix(in srgb, var(--accent) 45%, transparent); }
.graph-project__pill[data-kind="module"] { color: var(--info); }
.graph-project__pill[data-kind="dependency"] { color: var(--ok); }
.graph-project__pill[data-kind="topic"] { color: var(--warn); }
.graph-project__edges { margin: 0; padding: 0; list-style: none; font-size: 11px; max-height: 260px; overflow: auto; }
.graph-project__edges li { display: flex; gap: 6px; align-items: center; padding: 3px 0; border-bottom: 1px solid var(--hairline); }
.graph-project__edges code { font-family: var(--font-mono); word-break: break-all; }
.graph-project__ekind { color: var(--text-dim); font-size: 10px; }
.graph-project__saved { margin-top: 12px; border-top: 1px solid var(--hairline); padding-top: 10px; }
.graph-project__saved-title { font-size: 11px; font-weight: 600; margin-bottom: 6px; }
.graph-project__saved-empty { font-size: 11px; color: var(--text-dim); }
.graph-project__saved-row { display: flex; gap: 10px; align-items: center; padding: 5px 0; border-bottom: 1px solid var(--hairline); font-size: 12px; }
.graph-project__saved-path { flex: 1; min-width: 0; font-family: var(--font-mono); font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.graph-project__saved-meta { color: var(--text-dim); white-space: nowrap; }
</style>
