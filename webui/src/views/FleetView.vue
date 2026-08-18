<template>
  <div class="control-stage fleet-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">模型舰队</h1>
        <p class="control-stage__subtitle">Fleet 多角色模型路由 — architect / builder / reviewer 并行编排，按任务自动推荐</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" @click="loadFleet">
          <Icon name="refresh" :size="16" /><span>刷新</span>
        </button>
      </div>
    </header>

    <section v-if="error" class="fleet-error">{{ error }}</section>

    <section class="fleet-roles">
      <div v-for="r in roles" :key="r.role_id" class="fleet-role-card">
        <div class="fleet-role-card__header">
          <span class="fleet-role-card__name">{{ r.label || r.role_id }}</span>
          <span class="fleet-role-card__kind">{{ r.kind }}</span>
        </div>
        <div class="fleet-role-card__model">{{ r.model }}</div>
        <div class="fleet-role-card__meta">
          <span>{{ r.provider }}</span>
          <span>推理: {{ r.reasoning }}</span>
          <span>并发: {{ r.max_concurrency }}</span>
        </div>
        <p class="fleet-role-card__prompt">{{ r.system_prompt }}</p>
      </div>
    </section>

    <section class="fleet-route">
      <h2 class="fleet-section-title">任务路由推荐</h2>
      <div class="fleet-route__form">
        <input v-model="task" class="fleet-input" placeholder="描述任务，AI 将推荐最合适的角色…" @keydown.enter="route" />
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
import { ref, onMounted } from 'vue'
import Icon from '@/components/Icon.vue'
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

async function loadFleet() {
  error.value = ''
  try {
    const res = await rpc.call<{ roles: Role[] }>('fleet.list')
    roles.value = res.roles || []
  } catch (e) {
    error.value = `加载失败: ${e instanceof Error ? e.message : String(e)}`
  }
}

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
.fleet-stage { padding: 24px 28px; max-width: 1200px; margin: 0 auto; }
.fleet-error { margin: 16px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
.fleet-roles { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 12px; margin-top: 20px; }
.fleet-role-card { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 14px; }
.fleet-role-card__header { display: flex; justify-content: space-between; align-items: center; }
.fleet-role-card__name { font-weight: 700; font-size: 14px; color: var(--accent); }
.fleet-role-card__kind { font-size: 10px; padding: 2px 8px; border-radius: var(--radius-pill); background: var(--bg-hover); color: var(--text-dim); }
.fleet-role-card__model { font-family: var(--font-mono, monospace); font-size: 12px; margin-top: 6px; }
.fleet-role-card__meta { display: flex; gap: 12px; font-size: 10px; color: var(--text-dim); margin-top: 4px; }
.fleet-role-card__prompt { font-size: 11px; color: var(--text-muted); margin: 8px 0 0; line-height: 1.5; }
.fleet-route { margin-top: 24px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 16px; }
.fleet-section-title { font-size: 13px; font-weight: 600; margin: 0 0 12px; color: var(--text-dim); }
.fleet-route__form { display: flex; gap: 8px; }
.fleet-input { flex: 1; padding: 9px 12px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--bg); color: var(--text); font-size: 13px; outline: none; }
.fleet-input:focus { border-color: var(--accent); }
.fleet-recommendation { display: flex; align-items: center; gap: 8px; margin-top: 12px; padding: 10px 14px; background: color-mix(in srgb, var(--ok) 8%, var(--card)); border: 1px solid color-mix(in srgb, var(--ok) 25%, transparent); border-radius: var(--radius-sm); font-size: 12px; color: var(--ok); }
.fleet-recommendation__model { font-family: var(--font-mono, monospace); color: var(--text-muted); }
.fleet-recommendation__source { margin-left: auto; font-size: 10px; color: var(--text-dim); }
</style>
