<script setup lang="ts">
/**
 * M3-D1 行为事件：最近活动时间线面板。
 * 数据源：core events/product.rs 审计日志（audit_log 表）同源，
 * 经网关 activity.recent 拉取，含 12 类产品事件。
 */
import { onMounted, ref } from 'vue'
import { useRpcStore } from '@/stores/rpc'

interface ActivityEntry {
  id: number
  ts: string
  session: string
  method: string
  event: string | null
  [key: string]: unknown
}

const rpc = useRpcStore()
const entries = ref<ActivityEntry[]>([])
const loading = ref(false)
const error = ref<string | null>(null)

const EVENT_META: Record<string, { label: string; kind: 'task' | 'approval' | 'mode' | 'usage' | 'tool' }> = {
  'task.initiated': { label: '任务发起', kind: 'task' },
  'plan.confirmed': { label: '计划确认', kind: 'task' },
  'tool.call': { label: '工具调用', kind: 'tool' },
  'task.completed': { label: '任务完成', kind: 'task' },
  'task.failed': { label: '任务失败', kind: 'task' },
  'task.cancelled': { label: '任务取消', kind: 'task' },
  'mode.changed': { label: '切换模式', kind: 'mode' },
  'trust.changed': { label: '信任档位变更', kind: 'mode' },
  'approval.requested': { label: '确认请求', kind: 'approval' },
  'approval.granted': { label: '确认授权', kind: 'approval' },
  'approval.denied': { label: '确认拒绝', kind: 'approval' },
  'budget.warning': { label: '预算告警', kind: 'usage' },
}

function metaFor(event: string | null) {
  return (event && EVENT_META[event]) || { label: event ?? '未知事件', kind: 'tool' as const }
}

function fmtTime(ts: string): string {
  if (!ts) return '—'
  const d = new Date(ts)
  if (Number.isNaN(d.getTime())) return ts
  return d.toLocaleString()
}

function detailFor(entry: ActivityEntry): string {
  const ev = entry.event
  if (ev === 'plan.confirmed') return `修订 ${entry.revision_id ?? '—'} · run ${entry.run_id ?? '—'}`
  if (ev === 'tool.call') return `工具 ${entry.tool ?? '—'}`
  if (ev === 'task.initiated') return `${entry.title ?? ''}（${entry.status ?? 'queued'}）`
  if (ev === 'task.completed' || ev === 'task.failed' || ev === 'task.cancelled') {
    return `task ${entry.task_id ?? '—'}${entry.error ? ` · ${entry.error}` : ''}`
  }
  if (ev === 'mode.changed') return `${entry.from ?? '—'} → ${entry.to ?? '—'}`
  if (ev === 'trust.changed') return `${entry.action ?? '—'}${entry.level ? ` · ${entry.level}` : ''}`
  if (ev === 'approval.requested' || ev === 'approval.granted' || ev === 'approval.denied') {
    return `${entry.approval ?? '—'} · ${entry.state ?? '—'}`
  }
  if (ev === 'budget.warning') {
    return `阈值 ${entry.threshold_tokens ?? 0} tok / $${entry.threshold_usd ?? 0} · 已用 ${entry.total_tokens ?? 0} tok / $${entry.cost_usd ?? 0}`
  }
  return ''
}

async function refresh() {
  loading.value = true
  error.value = null
  try {
    await rpc.waitForConnection()
    const data = await rpc.call<{ entries: ActivityEntry[]; total: number }>('activity.recent', {
      limit: 50,
    })
    entries.value = data.entries ?? []
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    loading.value = false
  }
}

onMounted(refresh)
</script>

<template>
  <section class="control-section">
    <div class="control-section__head">
      <h3 class="control-section__title">最近活动（产品行为事件）</h3>
      <p class="control-section__desc">
        M3-D1 行为事件时间线：任务发起/计划确认/工具调用/完成/失败/取消、模式与信任档位切换、审批流、预算告警，均来自 core 本地审计日志，展示最近 {{ entries.length }} 条。
      </p>
    </div>
    <div class="activity-toolbar">
      <span class="activity-toolbar__count">{{ entries.length }} 条事件</span>
      <button type="button" class="btn btn--ghost" :disabled="loading" @click="refresh">刷新</button>
    </div>
    <div v-if="error" class="activity-banner activity-banner--err" role="alert">{{ error }}</div>
    <div v-if="loading" class="activity-empty">加载中…</div>
    <ol v-else-if="entries.length" class="activity-list">
      <li v-for="entry in entries" :key="entry.id" class="activity-row" :data-testid="`activity-entry-${entry.id}`">
        <span class="activity-dot" :class="`activity-dot--${metaFor(entry.event).kind}`" aria-hidden="true" />
        <div class="activity-body">
          <div class="activity-line">
            <span class="activity-event">{{ metaFor(entry.event).label }}</span>
            <span class="activity-session">{{ entry.session }}</span>
          </div>
          <div v-if="detailFor(entry)" class="activity-detail">{{ detailFor(entry) }}</div>
        </div>
        <time class="activity-time" :datetime="entry.ts">{{ fmtTime(entry.ts) }}</time>
      </li>
    </ol>
    <div v-else class="activity-empty">暂无行为事件。触发任务、切换模式或审批操作后此处将出现时间线。</div>
  </section>
</template>

<style scoped>
.activity-toolbar {
  align-items: center;
  display: flex;
  gap: var(--sp-2);
  justify-content: flex-end;
  margin: var(--sp-2) 0;
}
.activity-toolbar__count {
  color: var(--text-dim);
  flex: 1;
  font-size: var(--fs-sm);
}
.activity-banner {
  border-radius: var(--radius-sm);
  font-size: var(--fs-sm);
  margin: var(--sp-2) 0;
  padding: var(--sp-2) var(--sp-3);
}
.activity-banner--err { background: color-mix(in srgb, var(--danger) 15%, var(--bg-surface)); color: var(--danger); }
.activity-empty {
  color: var(--text-muted);
  font-size: var(--fs-sm);
  padding: var(--sp-4) 0;
}
.activity-list {
  list-style: none;
  margin: 0;
  padding: 0;
}
.activity-row {
  align-items: flex-start;
  border-bottom: 1px solid var(--hairline, var(--border));
  display: flex;
  gap: var(--sp-2);
  padding: var(--sp-2) 0;
}
.activity-dot {
  border-radius: var(--radius-full);
  flex: 0 0 auto;
  height: 0.5rem;
  margin-top: 0.3rem;
  width: 0.5rem;
}
.activity-dot--task { background: var(--accent); }
.activity-dot--tool { background: var(--info); }
.activity-dot--approval { background: var(--warn); }
.activity-dot--mode { background: var(--ok); }
.activity-dot--usage { background: var(--danger); }
.activity-body { min-width: 0; flex: 1; }
.activity-line {
  align-items: baseline;
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
}
.activity-event { font-weight: 600; }
.activity-session {
  color: var(--text-dim);
  font-size: var(--fs-xs);
}
.activity-detail {
  color: var(--text-muted);
  font-size: var(--fs-xs);
  overflow-wrap: anywhere;
}
.activity-time {
  color: var(--text-dim);
  flex: 0 0 auto;
  font-size: var(--fs-xs);
  white-space: nowrap;
}
</style>
