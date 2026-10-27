<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRpcStore } from '@/stores/rpc'
import { useConfirm } from '@/composables/useConfirm'

/** M2-C6 持久任务队列面板：跨会话 Backlog 的排队 / 暂停 / 恢复 / 取消 / 执行。 */

interface QueueItem {
  id: string
  title: string
  goal: string
  mode: string
  session: string
  status: 'queued' | 'running' | 'paused' | 'completed' | 'failed' | 'cancelled'
  priority: number
  created_at_ms: number
  updated_at_ms: number
  error: string | null
}

const rpc = useRpcStore()
const { confirm } = useConfirm()
const { t } = useI18n()

const items = ref<QueueItem[]>([])
const loading = ref(false)
const error = ref<string | null>(null)
const message = ref<string | null>(null)

// 入队表单
const newTitle = ref('')
const newGoal = ref('')
const newPriority = ref(5)

const statusLabel = computed<Record<string, string>>(() => ({
  queued: t('settings.taskQueue.statusQueued'),
  running: t('settings.taskQueue.statusRunning'),
  paused: t('settings.taskQueue.statusPaused'),
  completed: t('settings.taskQueue.statusCompleted'),
  failed: t('settings.taskQueue.statusFailed'),
  cancelled: t('settings.taskQueue.statusCancelled'),
}))

const counts = computed(() => ({
  total: items.value.length,
  running: items.value.filter(i => i.status === 'running').length,
  paused: items.value.filter(i => i.status === 'paused').length,
  queued: items.value.filter(i => i.status === 'queued').length,
}))

async function refresh() {
  loading.value = true
  error.value = null
  try {
    await rpc.waitForConnection()
    const data = await rpc.call<{ items: QueueItem[] }>('queue.list')
    items.value = data.items ?? []
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    loading.value = false
  }
}

async function enqueue() {
  if (!newTitle.value.trim()) return
  error.value = null
  message.value = null
  try {
    await rpc.call('queue.enqueue', {
      title: newTitle.value.trim(),
      goal: newGoal.value.trim() || newTitle.value.trim(),
      priority: newPriority.value,
    })
    message.value = t('settings.taskQueue.enqueued')
    newTitle.value = ''
    newGoal.value = ''
    newPriority.value = 5
    await refresh()
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  }
}

async function runOp(item: QueueItem, op: 'start' | 'pause' | 'resume' | 'cancel') {
  if (op === 'cancel') {
    const ok = await confirm({
      title: t('settings.taskQueue.cancelTitle'),
      body: t('settings.taskQueue.cancelBody', { title: item.title }),
      primaryLabel: t('settings.taskQueue.cancelConfirm'),
      primaryClass: 'btn--danger',
    })
    if (!ok) return
  }
  error.value = null
  message.value = null
  try {
    const data = await rpc.call<{ ok: boolean; error?: string }>(`queue.${op}`, { id: item.id })
    if (data.ok) {
      const key = op === 'start' ? 'doneStart' : op === 'pause' ? 'donePause' : op === 'resume' ? 'doneResume' : 'doneCancel'
      message.value = t(`settings.taskQueue.${key}`, { title: item.title })
    } else {
      error.value = data.error ?? t('settings.taskQueue.opFailed')
    }
    await refresh()
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  }
}

function fmtTime(ms: number): string {
  if (!ms) return '—'
  return new Date(ms).toISOString().slice(0, 19).replace('T', ' ')
}

async function moveStep(item: QueueItem, direction: 'up' | 'down') {
  error.value = null
  message.value = null
  try {
    const data = await rpc.call<{ ok: boolean; error?: string }>('queue.move', {
      id: item.id,
      direction,
    })
    if (data.ok) {
      message.value = t(direction === 'up' ? 'settings.taskQueue.movedUp' : 'settings.taskQueue.movedDown', { title: item.title })
    } else {
      error.value = data.error ?? t('settings.taskQueue.opFailed')
    }
    await refresh()
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  }
}

onMounted(refresh)
</script>

<template>
  <section class="control-section">
    <div class="control-section__head">
      <h3 class="control-section__title">{{ t('settings.taskQueue.title') }}</h3>
      <p class="control-section__desc">
        {{ t('settings.taskQueue.description', counts) }}
      </p>
    </div>

    <div class="control-row control-row--stack">
      <div class="control-row__label-block">
        <span class="control-row__label">{{ t('settings.taskQueue.enqueueTitle') }}</span>
        <span class="control-row__desc">{{ t('settings.taskQueue.enqueueDesc') }}</span>
      </div>
      <div class="control-row__control queue-enqueue">
        <input v-model="newTitle" class="control-input" :placeholder="t('settings.taskQueue.titlePlaceholder')" :aria-label="t('settings.taskQueue.titleAria')" @keyup.enter="enqueue" />
        <input v-model="newGoal" class="control-input" :placeholder="t('settings.taskQueue.goalPlaceholder')" :aria-label="t('settings.taskQueue.goalAria')" @keyup.enter="enqueue" />
        <input v-model.number="newPriority" class="control-input control-input--narrow" type="number" min="1" max="9" :placeholder="t('settings.taskQueue.priorityPlaceholder')" :aria-label="t('settings.taskQueue.priorityAria')" />
        <button type="button" class="btn" :disabled="loading || !newTitle.trim()" @click="enqueue">{{ t('settings.taskQueue.enqueue') }}</button>
      </div>
    </div>

    <div v-if="error" class="queue-banner queue-banner--err" role="alert">{{ error }}</div>
    <div v-else-if="message" class="queue-banner queue-banner--ok" role="status">{{ message }}</div>

    <div class="queue-toolbar">
      <span class="queue-toolbar__count">{{ t('settings.taskQueue.countLabel', { count: counts.total }) }}</span>
      <button type="button" class="btn btn--ghost" :disabled="loading" @click="refresh">{{ t('settings.taskQueue.refresh') }}</button>
    </div>

    <div v-if="loading" class="queue-empty">{{ t('settings.taskQueue.loading') }}</div>
    <div v-else-if="items.length === 0" class="queue-empty">{{ t('settings.taskQueue.empty') }}</div>
    <table v-else class="queue-table">
      <thead>
        <tr>
          <th>{{ t('settings.taskQueue.colStatus') }}</th>
          <th>{{ t('settings.taskQueue.colTitle') }}</th>
          <th>{{ t('settings.taskQueue.colMode') }}</th>
          <th>{{ t('settings.taskQueue.colPriority') }}</th>
          <th>{{ t('settings.taskQueue.colCreated') }}</th>
          <th>{{ t('settings.taskQueue.colUpdated') }}</th>
          <th>{{ t('settings.taskQueue.colActions') }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="it in items" :key="it.id">
          <td><span class="queue-status" :class="'queue-status--' + it.status">{{ statusLabel[it.status] ?? it.status }}</span></td>
          <td>
            <div class="queue-title">{{ it.title }}</div>
            <div v-if="it.goal && it.goal !== it.title" class="queue-goal">{{ it.goal }}</div>
            <div v-if="it.error" class="queue-error">{{ it.error }}</div>
          </td>
          <td>{{ it.mode }}</td>
          <td>{{ it.priority }}</td>
          <td>{{ fmtTime(it.created_at_ms) }}</td>
          <td>{{ fmtTime(it.updated_at_ms) }}</td>
          <td>
            <span v-if="it.status === 'queued' || it.status === 'paused'" class="queue-ops">
              <button v-if="it.status === 'paused'" type="button" class="btn btn--sm" @click="runOp(it, 'resume')">{{ t('settings.taskQueue.opResume') }}</button>
              <button v-if="it.status === 'queued'" type="button" class="btn btn--sm" @click="runOp(it, 'start')">{{ t('settings.taskQueue.opStart') }}</button>
              <button v-if="it.status === 'queued'" type="button" class="btn btn--sm btn--ghost" :title="t('settings.taskQueue.upTitle')" :aria-label="t('settings.taskQueue.upAria')" @click="moveStep(it, 'up')">↑</button>
              <button v-if="it.status === 'queued'" type="button" class="btn btn--sm btn--ghost" :title="t('settings.taskQueue.downTitle')" :aria-label="t('settings.taskQueue.downAria')" @click="moveStep(it, 'down')">↓</button>
              <button type="button" class="btn btn--sm btn--ghost" @click="runOp(it, 'cancel')">{{ t('settings.taskQueue.opCancel') }}</button>
            </span>
            <button v-else-if="it.status === 'running'" type="button" class="btn btn--sm btn--ghost" @click="runOp(it, 'pause')">{{ t('settings.taskQueue.opPause') }}</button>
            <span v-else class="queue-dim">—</span>
          </td>
        </tr>
      </tbody>
    </table>
  </section>
</template>

<style scoped>
.queue-enqueue {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
}
.queue-banner {
  border-radius: var(--radius-sm);
  font-size: var(--fs-sm);
  margin: var(--sp-2) 0;
  padding: var(--sp-2) var(--sp-3);
}
.queue-banner--err { background: color-mix(in srgb, var(--danger) 15%, var(--bg-surface)); color: var(--danger); }
.queue-banner--ok { background: color-mix(in srgb, var(--ok) 15%, var(--bg-surface)); color: var(--ok); }
.queue-toolbar {
  align-items: center;
  display: flex;
  gap: var(--sp-2);
  justify-content: flex-end;
  margin: var(--sp-2) 0;
}
.queue-toolbar__count {
  color: var(--text-dim);
  flex: 1;
  font-size: var(--fs-sm);
}
.queue-empty {
  color: var(--text-muted);
  font-size: var(--fs-sm);
  padding: var(--sp-4) 0;
}
.queue-table {
  border-collapse: collapse;
  font-size: var(--fs-sm);
  width: 100%;
}
.queue-table th,
.queue-table td {
  border-bottom: 1px solid var(--hairline, var(--border));
  padding: var(--sp-2);
  text-align: left;
  vertical-align: top;
}
.queue-title { font-weight: 600; }
.queue-goal { color: var(--text-muted); font-size: var(--fs-xs); }
.queue-error { color: var(--danger); font-size: var(--fs-xs); }
.queue-status {
  border-radius: var(--radius-sm);
  display: inline-block;
  font-size: var(--fs-xs);
  font-weight: 600;
  padding: 2px var(--sp-2);
  white-space: nowrap;
}
.queue-status--queued { background: color-mix(in srgb, var(--info) 18%, var(--bg-surface)); color: var(--info); }
.queue-status--running { background: color-mix(in srgb, var(--accent) 20%, var(--bg-surface)); color: var(--accent); }
.queue-status--paused { background: color-mix(in srgb, var(--warn) 18%, var(--bg-surface)); color: var(--warn); }
.queue-status--completed { background: color-mix(in srgb, var(--ok) 18%, var(--bg-surface)); color: var(--ok); }
.queue-status--failed { background: color-mix(in srgb, var(--danger) 18%, var(--bg-surface)); color: var(--danger); }
.queue-status--cancelled { background: color-mix(in srgb, var(--text-dim) 25%, var(--bg-surface)); color: var(--text-dim); }
.queue-ops { display: inline-flex; gap: var(--sp-1); }
.queue-dim { color: var(--text-dim); }
.btn--sm { padding: 2px var(--sp-2); }
</style>
