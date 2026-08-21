<template>
  <div class="control-stage graph-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">关系图谱</h1>
        <p class="control-stage__subtitle">
          KB 2.0 知识图谱 · 实体-关系可视化（离线启发式抽取，零 NLP 依赖）
          <template v-if="graph"> · {{ graph.entity_count }} 实体 / {{ graph.edge_count }} 关系</template>
        </p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" @click="load">
          <Icon name="refresh" :size="16" /><span>刷新</span>
        </button>
      </div>
    </header>

    <section v-if="error" class="graph-error">{{ error }}</section>

    <!-- 灌入知识条目（同步进 BM25 检索 + 关系图谱） -->
    <section class="graph-ingest">
      <input v-model="seedTitle" class="graph-input" placeholder="标题（doc_id）" />
      <textarea v-model="seedText" class="graph-textarea" placeholder="正文（实体用大写开头或引号，如：Rust and Kubernetes are used for cloud native systems. Rust uses ownership.）"></textarea>
      <button class="btn btn--primary" :disabled="!seedTitle.trim() || !seedText.trim() || ingesting" @click="ingest">
        <Icon name="plus" :size="16" /><span>{{ ingesting ? '灌入中…' : '添加知识条目' }}</span>
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
      图谱为空 — 先在上方添加知识条目，或到「知识库」检索页灌入文档后刷新
    </section>

    <!-- 选中实体详情 -->
    <section v-if="selected" class="graph-detail">
      <div class="graph-detail__name">{{ selected }}</div>
      <div class="graph-detail__hint">{{ related.length > 0 ? `直接关联（${related.length}）：` : '无直接关联实体' }}</div>
      <div class="graph-detail__tags">
        <span v-for="r in related" :key="r" class="graph-detail__tag" @click="pick(r)">{{ r }}</span>
      </div>
      <div class="graph-detail__legend">图例：共现 灰 · 是 蓝 · 使用 绿 · 实现 橙 · 提供 青</div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

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
    error.value = `加载失败: ${e instanceof Error ? e.message : String(e)}`
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
    error.value = `灌入失败: ${e instanceof Error ? e.message : String(e)}`
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
</style>
