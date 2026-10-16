<!-- ModeImpactCard — how the current runtime mode shapes this task (T05, P0-4).
     Shows the active mode kind, the real router `mode_route` (T04) and the Bloom
     fusion `strategy`, plus an honest error when the last mode switch failed.
     Values are shown verbatim; a missing route/strategy renders an em dash
     rather than inventing one. -->
<script setup lang="ts">
import { computed } from 'vue'
import Icon from '@/components/Icon.vue'
import { t } from '@/i18n'
import { normalizeModeKind } from '@/stores/activeMode'

const props = defineProps<{
  modeKind: string
  modeRoute: string | null
  strategy: string | null
  switchError?: string | null
}>()

/** 与 ModeBadge / ModeHintBar 同源的五个模式枚举（i18n 别名键的存在前提）。 */
const KNOWN_MODE_KINDS = ['plan', 'agent', 'ensemble', 'bloom', 'omni-transcend'] as const

const title = computed(() => String(t('task.modeImpact.title')))
const modeKeyLabel = computed(() => String(t('task.mode')))
const routeLabel = computed(() => String(t('task.modeImpact.route')))
const strategyLabel = computed(() => String(t('task.modeImpact.strategy')))
const failureText = computed(() => String(t('chrome.modeSwitchFailed')))

const modeKindKey = computed(() => normalizeModeKind(props.modeKind))
const modeKindLabel = computed(() => {
  // D-03 一致性：同一个运行模式在顶栏叫「深度打磨」，任务页此前却直接显示枚举
  // `bloom`（真机实测）。现在统一走 i18n 别名，枚举值只留在 tooltip 里供高级用户核对。
  const kind = modeKindKey.value
  if (kind && (KNOWN_MODE_KINDS as readonly string[]).includes(kind)) {
    return String(t(`mode.alias.${kind}`))
  }
  return kind || '—'
})
const routeValue = computed(() => props.modeRoute || '—')
const strategyValue = computed(() => props.strategy || '—')
</script>

<template>
  <section class="mode-impact" role="region" :aria-label="title">
    <header class="mode-impact__head">
      <Icon name="mode" :size="15" aria-hidden="true" />
      <h3 class="mode-impact__title">{{ title }}</h3>
    </header>
    <dl class="mode-impact__rows">
      <div class="mode-impact__row">
        <dt class="mode-impact__key">{{ modeKeyLabel }}</dt>
        <dd class="mode-impact__val" :title="modeKindKey || undefined">{{ modeKindLabel }}</dd>
      </div>
      <div class="mode-impact__row">
        <dt class="mode-impact__key">{{ routeLabel }}</dt>
        <dd class="mode-impact__val">{{ routeValue }}</dd>
      </div>
      <div class="mode-impact__row">
        <dt class="mode-impact__key">{{ strategyLabel }}</dt>
        <dd class="mode-impact__val">{{ strategyValue }}</dd>
      </div>
    </dl>
    <p v-if="switchError" class="mode-impact__error" role="alert">
      <Icon name="alertTriangle" :size="12" aria-hidden="true" />
      <span>{{ failureText }}</span>
    </p>
  </section>
</template>

<style scoped>
.mode-impact {
  display: grid;
  gap: var(--sp-2);
  padding: var(--sp-3);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  color: var(--text);
}
.mode-impact__head { display: flex; align-items: center; gap: var(--sp-2); }
.mode-impact__title { margin: 0; font-size: var(--fs-sm); font-weight: 700; }
.mode-impact__rows { display: grid; gap: var(--sp-1); margin: 0; }
.mode-impact__row { display: flex; align-items: baseline; justify-content: space-between; gap: var(--sp-3); }
.mode-impact__key { margin: 0; color: var(--text-muted); font-size: var(--fs-xs); }
.mode-impact__val { margin: 0; font-size: var(--fs-sm); font-variant-numeric: tabular-nums; text-align: right; word-break: break-word; }
.mode-impact__error { display: inline-flex; align-items: center; gap: 5px; margin: 0; color: var(--danger); font-size: var(--fs-xs); }
@media (prefers-reduced-motion: reduce) { .mode-impact * { transition: none !important; animation: none !important; } }
</style>
