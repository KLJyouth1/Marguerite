<template>
  <div class="control-stage fleet-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <p class="fleet-kicker">Model Fleet</p>
        <h1 class="control-stage__title">模型舰队</h1>
        <p class="control-stage__subtitle">
          多角色并行编排 — Architect / Builder / Reviewer 与工具、记忆、检索同场呼吸。
          任务一句话，自动推荐最合适的执行路径。
        </p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" @click="loadFleet">
          <Icon name="refresh" :size="16" /><span>刷新</span>
        </button>
      </div>
    </header>

    <section class="fleet-hero" aria-hidden="true">
      <div class="fleet-hero__orbit" />
      <div class="fleet-hero__orbit fleet-hero__orbit--inner" />
      <div class="fleet-hero__core">
        <span class="fleet-hero__brand">Marguerite</span>
        <span class="fleet-hero__pulse" />
      </div>
    </section>

    <section v-if="error" class="fleet-error">{{ error }}</section>

    <section v-if="loading && roles.length === 0" class="fleet-grid" aria-busy="true">
      <div
        v-for="n in 3"
        :key="n"
        class="fleet-role-card fleet-role-card--skeleton"
        aria-hidden="true"
      >
        <span class="fleet-skeleton fleet-skeleton--title" />
        <span class="fleet-skeleton fleet-skeleton--line" />
        <span class="fleet-skeleton fleet-skeleton--line" />
        <span class="fleet-skeleton fleet-skeleton--line fleet-skeleton--short" />
      </div>
    </section>

    <section v-else class="fleet-grid">
      <RunNodeCard
        v-for="(r, i) in roles"
        :key="r.role_id"
        :title="r.label || r.role_id"
        kind="fleet"
        :status="recommendation?.role_id === r.role_id ? 'running' : 'ok'"
        :subtitle="r.model"
        :detail="r.system_prompt"
        :meta="`${r.provider} · 推理 ${r.reasoning} · 并发 ${r.max_concurrency}`"
        :index="i"
        @activate="task = `用 ${r.label || r.role_id} 角色执行：`; route()"
      />
      <RunNodeCard
        v-for="(cap, i) in capabilityNodes"
        :key="cap.id"
        :title="cap.title"
        :kind="cap.kind"
        :status="cap.status"
        :subtitle="cap.subtitle"
        :detail="cap.detail"
        :meta="cap.meta"
        :index="roles.length + i"
      />
    </section>

    <section class="fleet-route">
      <h2 class="fleet-section-title">任务路由推荐</h2>
      <div class="fleet-route__form">
        <input
          v-model="task"
          class="fleet-input"
          placeholder="描述任务，AI 将推荐最合适的角色…"
          @keydown.enter="route"
        />
        <button class="btn btn--primary" :disabled="!task.trim() || routing" @click="route">
          <Icon name="sparkles" :size="16" /><span>{{ routing ? '推荐中…' : '推荐角色' }}</span>
        </button>
      </div>
      <div v-if="recommendation" class="fleet-recommendation">
        <Icon name="check" :size="16" />
        <span>推荐角色: <b>{{ recommendation.label || recommendation.role_id }}</b></span>
        <span class="fleet-recommendation__model">{{ recommendation.model }}</span>
        <span class="fleet-recommendation__source">{{ recommendation.source }}</span>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, watch } from 'vue'
import Icon from '@/components/Icon.vue'
import RunNodeCard from '@/components/fleet/RunNodeCard.vue'
import type { RunNodeKind, RunNodeStatus } from '@/components/fleet/runNodeTypes'
import { useRpcStore } from '@/stores/rpc'

const rpc = useRpcStore()

interface Role {
  role_id: string; label: string; kind: string; provider: string; model: string;
  reasoning: string; max_concurrency: number; system_prompt: string;
}

const roles = ref<Role[]>([])
const task = ref('')
const routing = ref(false)
const recommendation = ref<(Role & { source: string }) | null>(null)
const error = ref('')
const loading = ref(false)
let loadedOnce = false

/** Hardcoded capability constellation — visual shell of the memory/tool stack. */
const capabilityNodes: Array<{
  id: string
  title: string
  kind: RunNodeKind
  status: RunNodeStatus
  subtitle: string
  detail: string
  meta: string
}> = [
  {
    id: 'mem',
    title: '长期记忆',
    kind: 'memory',
    status: 'running',
    subtitle: 'Episodic + Semantic',
    detail: '跨会话记忆与偏好沉淀，比竞品更稳的召回边界。',
    meta: 'memory.graph · warm',
  },
  {
    id: 'retr',
    title: '混合检索',
    kind: 'retrieval',
    status: 'ok',
    subtitle: 'Hybrid search',
    detail: '向量 + 关键词 + 图谱路径，少 token、高命中。',
    meta: 'retrieval.hybrid · ready',
  },
  {
    id: 'kb',
    title: '知识库',
    kind: 'knowledge',
    status: 'ok',
    subtitle: 'Knowledge graph',
    detail: '结构化知识页与关系边，办公与编码共用一盘棋。',
    meta: 'kb.graph · online',
  },
  {
    id: 'tools',
    title: '工具运行时',
    kind: 'tool',
    status: 'running',
    subtitle: 'Adapters',
    detail: '办公、终端、浏览器、沙箱 — 按需扇出，不烧上下文。',
    meta: 'tools.runtime · 12 adapters',
  },
]

async function loadFleet() {
  error.value = ''
  loading.value = true
  try {
    if (!rpc.isConnected) await rpc.waitForConnection(10_000)
    const res = await rpc.call<{ roles: Role[] }>('fleet.list')
    roles.value = res.roles || []
    loadedOnce = true
  } catch (e) {
    error.value = `加载失败: ${e instanceof Error ? e.message : String(e)}`
  } finally {
    loading.value = false
  }
}

watch(
  () => rpc.state,
  (s) => {
    if (s === 'connected' && !loadedOnce && !loading.value) void loadFleet()
  },
)

async function route() {
  if (!task.value.trim()) return
  routing.value = true
  try {
    const res = await rpc.call<Role & { source: string; error?: string }>('fleet.route', { task: task.value.trim() })
    if (res.error) { error.value = res.error } else { recommendation.value = res; error.value = '' }
  } catch (e) {
    error.value = `路由失败: ${e instanceof Error ? e.message : String(e)}`
  } finally {
    routing.value = false
  }
}

onMounted(loadFleet)
</script>

<style scoped>
.fleet-stage {
  padding: 24px 28px;
  max-width: 1280px;
  margin: 0 auto;
}

.fleet-kicker {
  margin: 0 0 4px;
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.22em;
  text-transform: uppercase;
  color: var(--accent);
}

.fleet-hero {
  position: relative;
  height: 120px;
  margin-top: calc(var(--sp-2) * -1);
  border-radius: var(--radius-panel);
  overflow: hidden;
  background:
    radial-gradient(ellipse 60% 80% at 50% 50%, color-mix(in srgb, var(--accent) 14%, transparent), transparent 70%),
    var(--bg-surface);
  border: 1px solid var(--hairline);
}

.fleet-hero__orbit {
  position: absolute;
  inset: 18%;
  border-radius: var(--radius-full);
  border: 1px solid color-mix(in srgb, var(--text) 10%, transparent);
  animation: fleetSpin calc(var(--dur-pulse) * 4) linear infinite;
}

.fleet-hero__orbit--inner {
  inset: 32%;
  animation-direction: reverse;
  animation-duration: calc(var(--dur-pulse) * 3);
}

.fleet-hero__core {
  position: absolute;
  inset: 0;
  display: grid;
  place-items: center;
  gap: 8px;
}

.fleet-hero__brand {
  font-family: var(--font-display);
  font-size: var(--fs-xl);
  font-weight: 700;
  letter-spacing: 0.04em;
}

.fleet-hero__pulse {
  width: 8px;
  height: 8px;
  border-radius: var(--radius-full);
  background: var(--accent);
  box-shadow: 0 0 0 6px color-mix(in srgb, var(--accent) 16%, transparent);
  animation: fleetPulse var(--dur-pulse) linear infinite;
}

.fleet-error {
  margin: 16px 0;
  padding: 10px 14px;
  background: color-mix(in srgb, var(--danger) 10%, var(--card));
  border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent);
  border-radius: var(--radius-sm);
  color: var(--danger);
  font-size: 12px;
}

.fleet-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 12px;
  margin-top: 8px;
}

.fleet-role-card--skeleton {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-height: 148px;
  padding: var(--sp-4);
  background: var(--card);
  border: 1px solid var(--hairline);
  border-radius: var(--radius-card);
}

.fleet-skeleton {
  display: block;
  height: 12px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--text) 10%, transparent);
  animation: fleetPulse var(--dur-pulse) linear infinite;
}

.fleet-skeleton--title { width: 45%; height: 14px; }
.fleet-skeleton--line { width: 90%; }
.fleet-skeleton--short { width: 60%; }

.fleet-route {
  margin-top: 8px;
  background:
    linear-gradient(160deg, color-mix(in srgb, var(--accent) 8%, transparent), transparent 50%),
    var(--card);
  border: 1px solid var(--hairline);
  border-radius: var(--radius-panel);
  padding: 16px;
}

.fleet-section-title {
  font-size: 13px;
  font-weight: 600;
  margin: 0 0 12px;
  color: var(--text-dim);
}

.fleet-route__form { display: flex; gap: 8px; }

.fleet-input {
  flex: 1;
  padding: 9px 12px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--hairline);
  background: var(--bg);
  color: var(--text);
  font-size: 13px;
  outline: none;
}

.fleet-input:focus { border-color: var(--accent); }

.fleet-recommendation {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 12px;
  padding: 10px 14px;
  background: color-mix(in srgb, var(--ok) 8%, var(--card));
  border: 1px solid color-mix(in srgb, var(--ok) 25%, transparent);
  border-radius: var(--radius-sm);
  font-size: 12px;
  color: var(--ok);
}

.fleet-recommendation__model {
  font-family: var(--font-mono, monospace);
  color: var(--text-muted);
}

.fleet-recommendation__source {
  margin-left: auto;
  font-size: 10px;
  color: var(--text-dim);
}

@keyframes fleetSpin {
  to { transform: rotate(360deg); }
}

@keyframes fleetPulse {
  0%, 100% { opacity: 0.55; }
  50% { opacity: 1; }
}

@media (prefers-reduced-motion: reduce) {
  .fleet-hero__orbit,
  .fleet-hero__pulse,
  .fleet-skeleton {
    animation: none;
  }
}
</style>
