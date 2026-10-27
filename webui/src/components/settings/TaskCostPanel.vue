<script setup lang="ts">
/**
 * M3-D2 FinOps：任务成本卡片面板。
 * usage.taskCost 按 run_id 归因该任务的 token 与成本；
 * 同时从 activity.recent（plan.confirmed / task.* 事件）抽取最近 run/task id 供快速查询。
 */
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRpcStore } from '@/stores/rpc'

const { t } = useI18n()

interface TaskCost {
  ok: boolean
  run_id: string
  session: string
  total_tokens: number
  input_tokens: number
  output_tokens: number
  calls: number
  cost_usd: number
  currency: string
  models?: Array<{ model: string; calls: number; tokens: number; cost_usd: number }>
  error?: string
}

interface ActivityEntry {
  id: number
  ts: string
  session: string
  method: string
  event: string | null
  run_id?: string
  task_id?: string
  title?: string
}

const rpc = useRpcStore()
const runId = ref('')
const result = ref<TaskCost | null>(null)
const recentIds = ref<ActivityEntry[]>([])
const loading = ref(false)
const error = ref<string | null>(null)

async function loadRecent() {
  try {
    await rpc.waitForConnection()
    const data = await rpc.call<{ entries: ActivityEntry[] }>('activity.recent', { limit: 30 })
    recentIds.value = (data.entries ?? []).filter(
      entry => entry.event === 'plan.confirmed' || entry.event?.startsWith('task.'),
    )
  } catch {
    recentIds.value = []
  }
}

async function query(id?: string) {
  const target = id?.trim() || runId.value.trim()
  if (!target) return
  loading.value = true
  error.value = null
  result.value = null
  try {
    await rpc.waitForConnection()
    const data = await rpc.call<TaskCost>('usage.taskCost', { runId: target })
    if (data?.ok === false) {
      error.value = data.error ?? t('settings.taskCost.queryFailed')
    } else {
      result.value = data
    }
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    loading.value = false
  }
}

onMounted(loadRecent)
</script>

<template>
  <section class="control-section">
    <div class="control-section__head">
      <h3 class="control-section__title">{{ t('settings.taskCost.title') }}</h3>
      <p class="control-section__desc">
        {{ t('settings.taskCost.description') }}
      </p>
    </div>
    <div class="costcard-query">
      <input
        v-model="runId"
        class="control-input"
        :placeholder="t('settings.taskCost.placeholder')"
        :aria-label="t('settings.taskCost.runAria')"
        @keyup.enter="query()"
      />
      <button type="button" class="btn" :disabled="loading || !runId.trim()" @click="query()">{{ t('settings.taskCost.query') }}</button>
    </div>
    <div v-if="recentIds.length" class="costcard-recent">
      <span class="costcard-recent__label">{{ t('settings.taskCost.recent') }}</span>
      <button
        v-for="entry in recentIds.slice(0, 5)"
        :key="entry.id"
        type="button"
        class="btn btn--sm btn--ghost"
        :title="entry.title || ''"
        @click="query(entry.run_id || entry.task_id)"
      >
        {{ entry.event === 'plan.confirmed' ? (entry.run_id || '') : (entry.task_id || '') }}
      </button>
    </div>
    <div v-if="error" class="costcard-banner costcard-banner--err" role="alert">{{ error }}</div>
    <div v-if="loading" class="costcard-empty">{{ t('settings.taskCost.querying') }}</div>
    <div v-else-if="result" class="costcard" data-testid="task-cost-card">
      <div class="costcard__head">
        <span class="costcard__title">{{ t('settings.taskCost.cardTitle') }}</span>
        <code class="costcard__run">{{ result.run_id }}</code>
      </div>
      <div class="costcard__grid">
        <div class="costcard__item">
          <span class="costcard__label">{{ t('settings.taskCost.totalTokens') }}</span>
          <span class="costcard__value">{{ result.total_tokens.toLocaleString() }}</span>
        </div>
        <div class="costcard__item">
          <span class="costcard__label">{{ t('settings.taskCost.inputOutput') }}</span>
          <span class="costcard__value">{{ result.input_tokens.toLocaleString() }} / {{ result.output_tokens.toLocaleString() }}</span>
        </div>
        <div class="costcard__item">
          <span class="costcard__label">{{ t('settings.taskCost.calls') }}</span>
          <span class="costcard__value">{{ result.calls }}</span>
        </div>
        <div class="costcard__item">
          <span class="costcard__label">{{ t('settings.taskCost.cost') }}</span>
          <span class="costcard__value costcard__value--cost">${{ result.cost_usd.toFixed(4) }} {{ result.currency }}</span>
        </div>
      </div>
      <table v-if="result.models?.length" class="costcard-table">
        <thead>
          <tr>
            <th>{{ t('settings.taskCost.model') }}</th>
            <th>{{ t('settings.taskCost.callCount') }}</th>
            <th>{{ t('settings.taskCost.tokenCount') }}</th>
            <th>{{ t('settings.taskCost.costUsd') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="model in result.models" :key="model.model">
            <td>{{ model.model }}</td>
            <td>{{ model.calls }}</td>
            <td>{{ model.tokens.toLocaleString() }}</td>
            <td>{{ model.cost_usd.toFixed(4) }}</td>
          </tr>
        </tbody>
      </table>
    </div>
  </section>
</template>

<style scoped>
.costcard-query {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
  margin: var(--sp-2) 0;
}
.costcard-recent {
  align-items: center;
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-1);
  margin: var(--sp-2) 0;
}
.costcard-recent__label { color: var(--text-dim); font-size: var(--fs-xs); }
.costcard-banner {
  border-radius: var(--radius-sm);
  font-size: var(--fs-sm);
  margin: var(--sp-2) 0;
  padding: var(--sp-2) var(--sp-3);
}
.costcard-banner--err { background: color-mix(in srgb, var(--danger) 15%, var(--bg-surface)); color: var(--danger); }
.costcard-empty { color: var(--text-muted); font-size: var(--fs-sm); padding: var(--sp-3) 0; }
.costcard {
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-left: 3px solid var(--accent);
  border-radius: var(--radius-md);
  margin-top: var(--sp-2);
  padding: var(--sp-3);
}
.costcard__head {
  align-items: center;
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
  justify-content: space-between;
}
.costcard__title { font-weight: 700; }
.costcard__run {
  background: color-mix(in srgb, var(--text) 8%, transparent);
  border-radius: var(--radius-sm);
  color: var(--text-dim);
  font-size: var(--fs-xs);
  padding: 2px var(--sp-2);
}
.costcard__grid {
  display: grid;
  gap: var(--sp-2);
  grid-template-columns: repeat(auto-fit, minmax(8rem, 1fr));
  margin: var(--sp-3) 0;
}
.costcard__item { display: flex; flex-direction: column; gap: 2px; }
.costcard__label { color: var(--text-dim); font-size: var(--fs-xs); }
.costcard__value { font-size: var(--fs-sm); font-weight: 600; }
.costcard__value--cost { color: var(--accent); }
.costcard-table {
  border-collapse: collapse;
  font-size: var(--fs-sm);
  width: 100%;
}
.costcard-table th,
.costcard-table td {
  border-bottom: 1px solid var(--hairline, var(--border));
  padding: var(--sp-2);
  text-align: left;
}
.btn--sm { padding: 2px var(--sp-2); }
</style>
