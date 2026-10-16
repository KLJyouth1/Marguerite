<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
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

const items = ref<QueueItem[]>([])
const loading = ref(false)
const error = ref<string | null>(null)
const message = ref<string | null>(null)

// 入队表单
const newTitle = ref('')
const newGoal = ref('')
const newPriority = ref(5)

const STATUS_LABEL: Record<string, string> = {
  queued: '排队中',
  running: '执行中',
  paused: '已暂停',
  completed: '已完成',
  failed: '失败',
  cancelled: '已取消',
}

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
    message.value = '任务已入队'
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
      title: '取消任务',
      body: `确定取消队列任务「${item.title}」？取消后不可恢复。`,
      primaryLabel: '取消任务',
      primaryClass: 'btn--danger',
    })
    if (!ok) return
  }
  error.value = null
  message.value = null
  try {
    const data = await rpc.call<{ ok: boolean; error?: string }>(`queue.${op}`, { id: item.id })
    if (data.ok) {
      message.value = `「${item.title}」已${op === 'start' ? '开始执行' : op === 'pause' ? '暂停' : op === 'resume' ? '恢复' : '取消'}`
    } else {
      error.value = data.error ?? '操作失败'
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
      message.value = `「${item.title}」已${direction === 'up' ? '上移' : '下移'}`
    } else {
      error.value = data.error ?? '操作失败'
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
      <h3 class="control-section__title">持久任务队列（Persistent Queue）</h3>
      <p class="control-section__desc">
        跨会话任务 Backlog：长任务可排队、暂停、恢复、取消，状态落盘持久化，重启后仍可继续管理。当前：{{ counts.total }} 个任务（执行中 {{ counts.running }} / 排队 {{ counts.queued }} / 暂停 {{ counts.paused }}）。
      </p>
    </div>

    <div class="control-row control-row--stack">
      <div class="control-row__label-block">
        <span class="control-row__label">入队新任务</span>
        <span class="control-row__desc">标题必填；目标可省略（默认同标题）</span>
      </div>
      <div class="control-row__control queue-enqueue">
        <input v-model="newTitle" class="control-input" placeholder="任务标题 *" aria-label="任务标题" @keyup.enter="enqueue" />
        <input v-model="newGoal" class="control-input" placeholder="目标描述" aria-label="目标描述" @keyup.enter="enqueue" />
        <input v-model.number="newPriority" class="control-input control-input--narrow" type="number" min="1" max="9" placeholder="优先级" aria-label="优先级" />
        <button type="button" class="btn" :disabled="loading || !newTitle.trim()" @click="enqueue">入队</button>
      </div>
    </div>

    <div v-if="error" class="queue-banner queue-banner--err" role="alert">{{ error }}</div>
    <div v-else-if="message" class="queue-banner queue-banner--ok" role="status">{{ message }}</div>

    <div class="queue-toolbar">
      <span class="queue-toolbar__count">{{ counts.total }} 个任务</span>
      <button type="button" class="btn btn--ghost" :disabled="loading" @click="refresh">刷新</button>
    </div>

    <div v-if="loading" class="queue-empty">加载中…</div>
    <div v-else-if="items.length === 0" class="queue-empty">队列为空。长任务可在此排队并跨会话管理。</div>
    <table v-else class="queue-table">
      <thead>
        <tr>
          <th>状态</th><th>标题</th><th>模式</th><th>优先级</th><th>创建时间</th><th>更新时间</th><th>操作</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="it in items" :key="it.id">
          <td><span class="queue-status" :class="'queue-status--' + it.status">{{ STATUS_LABEL[it.status] ?? it.status }}</span></td>
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
              <button v-if="it.status === 'paused'" type="button" class="btn btn--sm" @click="runOp(it, 'resume')">恢复</button>
              <button v-if="it.status === 'queued'" type="button" class="btn btn--sm" @click="runOp(it, 'start')">执行</button>
              <button v-if="it.status === 'queued'" type="button" class="btn btn--sm btn--ghost" title="上移（提高调度优先级）" aria-label="上移" @click="moveStep(it, 'up')">↑</button>
              <button v-if="it.status === 'queued'" type="button" class="btn btn--sm btn--ghost" title="下移（降低调度优先级）" aria-label="下移" @click="moveStep(it, 'down')">↓</button>
              <button type="button" class="btn btn--sm btn--ghost" @click="runOp(it, 'cancel')">取消</button>
            </span>
            <button v-else-if="it.status === 'running'" type="button" class="btn btn--sm btn--ghost" @click="runOp(it, 'pause')">暂停</button>
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
