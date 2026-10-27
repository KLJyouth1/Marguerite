<template>
  <section class="usage-heatmap" :aria-label="t('usageLogs.heatmap.title')">
    <header class="usage-heatmap__head">
      <div class="usage-heatmap__heading">
        <h2 class="usage-heatmap__title">{{ t('usageLogs.heatmap.title') }}</h2>
        <p class="usage-heatmap__hint">{{ hint }}</p>
      </div>
      <div class="usage-heatmap__legend" aria-hidden="true">
        <span>{{ t('usageLogs.heatmap.less') }}</span>
        <span
          v-for="level in legendLevels"
          :key="level"
          class="usage-heatmap__cell"
          :class="cellClass(level, false)"
        />
        <span>{{ t('usageLogs.heatmap.more') }}</span>
      </div>
    </header>

    <p v-if="state === 'error'" class="usage-heatmap__empty" role="status">
      {{ t('usageLogs.heatmap.unavailable') }}
    </p>
    <p v-else-if="state === 'ready' && grid.activeDays === 0" class="usage-heatmap__empty" role="status">
      {{ t('usageLogs.heatmap.empty') }}
    </p>
    <div v-else class="usage-heatmap__body" :aria-busy="state === 'loading' || state === 'idle'">
      <div class="usage-heatmap__weekdays" aria-hidden="true">
        <span v-for="(label, index) in weekdayLabels" :key="index" class="usage-heatmap__weekday">{{ label }}</span>
      </div>
      <div class="usage-heatmap__scroll">
        <div class="usage-heatmap__months" aria-hidden="true">
          <span
            v-for="(week, index) in grid.weeks"
            :key="week.weekStartMs"
            class="usage-heatmap__month"
          >{{ monthLabel(week.monthKey, index) }}</span>
        </div>
        <div class="usage-heatmap__grid" role="img" :aria-label="summaryLabel">
          <template v-for="week in grid.weeks" :key="week.weekStartMs">
            <span
              v-for="cell in week.cells"
              :key="cell.dayMs"
              class="usage-heatmap__cell"
              :class="cellClass(cell.level, cell.future)"
              :title="cell.future ? undefined : cellTitle(cell)"
            />
          </template>
        </div>
      </div>
    </div>
  </section>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import type { UsageDailyPoint } from '@/types/usage'
import { buildHeatGrid } from '@/utils/usage/heatGrid'

const props = withDefaults(defineProps<{
  points: UsageDailyPoint[]
  state?: 'idle' | 'loading' | 'ready' | 'error'
  weeks?: number
  now?: Date
}>(), {
  state: 'ready',
  weeks: 12,
  now: undefined,
})

const { t, locale } = useI18n()

const legendLevels = [0, 1, 2, 3, 4] as const
const grid = computed(() => buildHeatGrid(props.points, { weeks: props.weeks, now: props.now }))

const numberFormat = computed(() => new Intl.NumberFormat(locale.value || undefined))
const dateFormat = computed(() => new Intl.DateTimeFormat(locale.value || undefined, {
  timeZone: 'UTC',
  year: 'numeric',
  month: 'short',
  day: 'numeric',
}))
const monthFormat = computed(() => new Intl.DateTimeFormat(locale.value || undefined, {
  timeZone: 'UTC',
  month: 'short',
}))

const weekdayLabels = computed(() => {
  const format = new Intl.DateTimeFormat(locale.value || undefined, {
    timeZone: 'UTC',
    weekday: 'short',
  })
  // 2024-01-01 is a Monday; only show alternating labels to keep the axis quiet.
  return Array.from({ length: 7 }, (_, index) => (
    index % 2 === 0 ? format.format(new Date(Date.UTC(2024, 0, 1 + index))) : ''
  ))
})

const hint = computed(() => {
  const nowMs = (props.now ?? new Date()).getTime()
  const lastDayMs = Math.floor(nowMs / 86_400_000) * 86_400_000
  const range = `${dateFormat.value.format(new Date(grid.value.startMs))} – ${dateFormat.value.format(new Date(lastDayMs))}`
  return `${t('usageLogs.heatmap.hint', { weeks: props.weeks })} · ${range}`
})

const summaryLabel = computed(() => t('usageLogs.heatmap.summary', {
  tokens: numberFormat.value.format(grid.value.totalTokens),
  days: numberFormat.value.format(grid.value.activeDays),
}))

function cellClass(level: number, future: boolean) {
  return {
    [`usage-heatmap__cell--l${level}`]: true,
    'usage-heatmap__cell--future': future,
  }
}

function monthLabel(monthKey: string, index: number): string {
  if (index > 0 && grid.value.weeks[index - 1]?.monthKey === monthKey) return ''
  return monthFormat.value.format(new Date(grid.value.weeks[index]?.weekStartMs ?? 0))
}

function cellTitle(cell: { dayMs: number; tokens: number; calls: number }): string {
  return t('usageLogs.heatmap.cell', {
    date: dateFormat.value.format(new Date(cell.dayMs)),
    tokens: numberFormat.value.format(cell.tokens),
    calls: numberFormat.value.format(cell.calls),
  })
}
</script>

<style scoped>
.usage-heatmap {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  padding: var(--sp-4) var(--sp-5);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-surface);
}

.usage-heatmap__head {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: var(--sp-2);
}

.usage-heatmap__heading {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.usage-heatmap__title {
  margin: 0;
  color: var(--text);
  font-size: var(--fs-sm);
  font-weight: 650;
}

.usage-heatmap__hint {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-xs);
}

.usage-heatmap__legend {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  color: var(--text-muted);
  font-size: var(--fs-xs);
}

.usage-heatmap__body {
  display: flex;
  gap: var(--sp-2);
  min-width: 0;
}

.usage-heatmap__weekdays {
  display: grid;
  grid-template-rows: repeat(7, 12px);
  gap: 3px;
  padding-top: 17px;
  color: var(--text-dim);
  font-size: var(--fs-xs);
  line-height: 12px;
}

.usage-heatmap__weekday {
  width: 18px;
}

.usage-heatmap__scroll {
  min-width: 0;
  overflow-x: auto;
}

.usage-heatmap__months {
  display: grid;
  grid-auto-flow: column;
  grid-auto-columns: 12px;
  gap: 3px;
  height: 14px;
  overflow: visible;
}

.usage-heatmap__month {
  color: var(--text-dim);
  font-size: var(--fs-xs);
  line-height: 1;
  white-space: nowrap;
}

.usage-heatmap__grid {
  display: grid;
  grid-auto-flow: column;
  grid-template-rows: repeat(7, 12px);
  grid-auto-columns: 12px;
  gap: 3px;
}

.usage-heatmap__cell {
  width: 12px;
  height: 12px;
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--text-dim) 10%, var(--bg-surface));
}

.usage-heatmap__cell--l1 {
  background: color-mix(in srgb, var(--accent) 18%, var(--bg-surface));
}

.usage-heatmap__cell--l2 {
  background: color-mix(in srgb, var(--accent) 34%, var(--bg-surface));
}

.usage-heatmap__cell--l3 {
  background: color-mix(in srgb, var(--accent) 52%, var(--bg-surface));
}

.usage-heatmap__cell--l4 {
  background: color-mix(in srgb, var(--accent) 72%, var(--bg-surface));
}

.usage-heatmap__cell--future {
  background: transparent;
}

.usage-heatmap__empty {
  margin: 0;
  padding: var(--sp-5) 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
  text-align: center;
}
</style>
