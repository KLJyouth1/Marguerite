<template>
  <div class="knowledge-view">
    <header class="kb-head">
      <h1>知识页 Knowledge Pages</h1>
      <p class="kb-sub">内置本地知识库（免 Key / 离线 / 语义×关键词混合检索）；也可用终端 REPL /pages list | add | show | search | rm 管理。</p>
    </header>

    <section class="kb-card">
      <h2>新建 Add</h2>
      <input v-model="form.title" placeholder="标题 title" />
      <input v-model="form.tags" placeholder="标签 tags（逗号分隔）" />
      <textarea v-model="form.content" rows="4" placeholder="内容 content"></textarea>
      <button :disabled="!form.title.trim() || saving" @click="addPage">保存 Save</button>
      <p v-if="notice" :class="noticeOK ? 'ok' : 'err'">{{ notice }}</p>
    </section>

    <section class="kb-card">
      <h2>检索 Search（语义 × 关键词）</h2>
      <div class="row">
        <input v-model="query" placeholder="输入查询…" @keyup.enter="doSearch" />
        <button :disabled="!query.trim()" @click="doSearch">搜索</button>
      </div>
      <ul v-if="searched">
        <li v-for="p in results" :key="p.id">
          <strong>{{ p.title }}</strong> <span class="score">[{{ (p.score ?? 0).toFixed(2) }}]</span>
          <pre>{{ p.content }}</pre>
        </li>
        <li v-if="results.length === 0">无匹配。</li>
      </ul>
    </section>

    <section class="kb-card">
      <h2>向量混合检索 Vector Hybrid（kb.search.hybrid · BM25 + 向量 RRF 融合）</h2>
      <div class="row">
        <input v-model="hybridQuery" placeholder="输入查询…（默认零依赖特征哈希 256 维向量；模型后端需配置 embeddings.backend=model）" @keyup.enter="doHybridSearch" />
        <button :disabled="!hybridQuery.trim() || hybridBusy" @click="doHybridSearch">混合检索</button>
      </div>
      <p v-if="hybridEngine" class="kb-sub">引擎：{{ hybridEngine }}（语料超限或后端不可用会诚实降级纯 BM25）</p>
      <ul v-if="hybridSearched">
        <li v-for="(p, i) in hybridResults" :key="i">
          <strong>{{ p.title }}</strong> <span class="score">[BM25 {{ p.bm25_score.toFixed(2) }} · 向量 {{ p.vector_score.toFixed(2) }} · 融合 {{ p.fused_score.toFixed(2) }}]</span>
          <pre>{{ p.chunk }}</pre>
        </li>
        <li v-if="hybridResults.length === 0">无匹配。</li>
      </ul>
    </section>

    <section class="kb-card">
      <h2>全部（{{ pages.length }}）All</h2>
      <ul>
        <li v-for="p in pages" :key="p.id">
          <strong>{{ p.title }}</strong>
          <span class="tags">{{ (p.tags || []).join(', ') }}</span>
          <span class="meta">使用 {{ p.useCount ?? 0 }} 次</span>
        </li>
        <li v-if="pages.length === 0">暂无知识页。用上方表单创建。</li>
      </ul>
    </section>
  </div>
</template>

<script setup lang="ts">
import { onMounted, reactive, ref } from 'vue'
import { useRpcStore } from '@/stores/rpc'

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

onMounted(load)

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
    notice.value = d.message || (d.success ? '已保存' : '保存失败')
    if (d.success) {
      form.title = ''
      form.tags = ''
      form.content = ''
      await load()
    }
  } catch {
    noticeOK.value = false
    notice.value = '调用失败（需网关支持 pages.*）'
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
</style>
