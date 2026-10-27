<script setup lang="ts">
/**
 * M3-D1 行为事件：最近活动时间线面板。
 * 数据源：core events/product.rs 审计日志（audit_log 表）同源，
 * 经网关 activity.recent 拉取，含 12 类产品事件。
 */
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRpcStore } from '@/stores/rpc'

interface ActivityEntry {
  id: number
  ts: string
  session: string
  method: string
  event: string | null
  /** `product::audit` 写入的事件把业务字段嵌在 fields 里（`product::emit` 则是平铺）。 */
  fields?: Record<string, unknown> | null
  [key: string]: unknown
}

const rpc = useRpcStore()
const { t } = useI18n()
const entries = ref<ActivityEntry[]>([])
const loading = ref(false)
const error = ref<string | null>(null)

const EVENT_META: Record<string, { key: string; kind: 'task' | 'approval' | 'mode' | 'usage' | 'tool' }> = {
  'task.initiated': { key: 'eventTaskInitiated', kind: 'task' },
  'plan.confirmed': { key: 'eventPlanConfirmed', kind: 'task' },
  'tool.call': { key: 'eventToolCall', kind: 'tool' },
  'task.completed': { key: 'eventTaskCompleted', kind: 'task' },
  'task.failed': { key: 'eventTaskFailed', kind: 'task' },
  'task.cancelled': { key: 'eventTaskCancelled', kind: 'task' },
  'mode.changed': { key: 'eventModeChanged', kind: 'mode' },
  'trust.changed': { key: 'eventTrustChanged', kind: 'mode' },
  'approval.requested': { key: 'eventApprovalRequested', kind: 'approval' },
  'approval.granted': { key: 'eventApprovalGranted', kind: 'approval' },
  'approval.denied': { key: 'eventApprovalDenied', kind: 'approval' },
  'budget.warning': { key: 'eventBudgetWarning', kind: 'usage' },
}

function metaFor(event: string | null) {
  const meta = event ? EVENT_META[event] : undefined
  if (!meta) {
    return { label: event ?? t('settings.recentActivity.unknownEvent'), kind: 'tool' as const }
  }
  return { label: t(`settings.recentActivity.${meta.key}`), kind: meta.kind }
}

function fmtTime(ts: string): string {
  if (!ts) return '—'
  const d = new Date(ts)
  if (Number.isNaN(d.getTime())) return ts
  return d.toLocaleString()
}

/**
 * 读取事件字段：`product::emit` 把字段平铺在条目上，而 `product::audit`
 * （`mode.changed` 走这条）把字段包在 `fields` 里。两种形状都要支持。
 */
function fieldOf(entry: ActivityEntry, key: string): unknown {
  const fields = entry.fields
  if (fields && typeof fields === 'object' && key in fields) {
    return fields[key]
  }
  return entry[key]
}

function hasValue(value: unknown): boolean {
  return value !== undefined && value !== null && value !== ''
}

function detailFor(entry: ActivityEntry): string {
  const ev = entry.event
  if (ev === 'plan.confirmed') {
    return t('settings.recentActivity.detailRevision', { revision: entry.revision_id ?? '—', run: entry.run_id ?? '—' })
  }
  if (ev === 'tool.call') return t('settings.recentActivity.detailTool', { tool: entry.tool ?? '—' })
  if (ev === 'task.initiated') {
    return t('settings.recentActivity.detailTaskStatus', { title: entry.title ?? '', status: entry.status ?? 'queued' })
  }
  if (ev === 'task.completed' || ev === 'task.failed' || ev === 'task.cancelled') {
    const task = entry.task_id ?? '—'
    return entry.error
      ? t('settings.recentActivity.detailTaskError', { task, error: entry.error })
      : t('settings.recentActivity.detailTask', { task })
  }
  if (ev === 'mode.changed') {
    const from = fieldOf(entry, 'from')
    const to = fieldOf(entry, 'to')
    // 真正缺前后值时只保留事件摘要（「切换模式 core」），不渲染「—」→「—」
    // 这种看起来像数据丢失的空洞占位。
    if (!hasValue(from) || !hasValue(to)) return ''
    return t('settings.recentActivity.detailMode', { from, to })
  }
  if (ev === 'trust.changed') {
    const action = entry.action ?? '—'
    return entry.level
      ? t('settings.recentActivity.detailTrustLevel', { action, level: entry.level })
      : t('settings.recentActivity.detailTrust', { action })
  }
  if (ev === 'approval.requested' || ev === 'approval.granted' || ev === 'approval.denied') {
    return t('settings.recentActivity.detailApproval', { approval: entry.approval ?? '—', state: entry.state ?? '—' })
  }
  if (ev === 'budget.warning') {
    return t('settings.recentActivity.detailBudget', {
      thresholdTokens: entry.threshold_tokens ?? 0,
      thresholdUsd: entry.threshold_usd ?? 0,
      usedTokens: entry.total_tokens ?? 0,
      usedCost: entry.cost_usd ?? 0,
    })
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
      <h3 class="control-section__title">{{ t('settings.recentActivity.title') }}</h3>
      <p class="control-section__desc">
        {{ t('settings.recentActivity.description', { count: entries.length }) }}
      </p>
    </div>
    <div class="activity-toolbar">
      <span class="activity-toolbar__count">{{ t('settings.recentActivity.countLabel', { count: entries.length }) }}</span>
      <button type="button" class="btn btn--ghost" :disabled="loading" @click="refresh">{{ t('settings.recentActivity.refresh') }}</button>
    </div>
    <div v-if="error" class="activity-banner activity-banner--err" role="alert">{{ error }}</div>
    <div v-if="loading" class="activity-empty">{{ t('settings.recentActivity.loading') }}</div>
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
    <div v-else class="activity-empty">{{ t('settings.recentActivity.empty') }}</div>
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
