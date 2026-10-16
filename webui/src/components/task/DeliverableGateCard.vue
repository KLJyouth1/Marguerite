<!-- DeliverableGateCard — the terminal quality-gate verdict (T05, P0-4).
     Renders the `done.completeness` verdict surfaced by `useTaskWorkspace`:
     whether the search results were valid and whether every declared deliverable
     landed. It never fabricates reasons — `missing` holds the backend's block
     reason codes verbatim. Renders nothing when there is no gate yet. -->
<script setup lang="ts">
import { computed } from 'vue'
import Icon from '@/components/Icon.vue'
import { t } from '@/i18n'
import type { IconName } from '@/utils/icons'
import type { TaskGate } from '@/types/rpc'

const props = defineProps<{ gate: TaskGate | null }>()

const title = computed(() => String(t('task.gate.title')))
const searchLabel = computed(() => String(t('task.gate.searchValid')))
const artifactsLabel = computed(() => String(t('task.gate.artifactsComplete')))
const verdictLabel = computed(() =>
  props.gate ? String(t(`task.gate.verdict.${props.gate.verdict}`)) : '',
)
const missingLabel = computed(() =>
  props.gate
    ? String(t('task.gate.missing', { items: props.gate.missing.join(', ') }))
    : '',
)

const VERDICT_ICON: Record<TaskGate['verdict'], IconName> = {
  pass: 'check',
  partial: 'clock',
  fail: 'alertTriangle',
}
</script>

<template>
  <section
    v-if="gate"
    class="gate-card"
    :class="`gate-card--${gate.verdict}`"
    role="region"
    :aria-label="title"
  >
    <header class="gate-card__head">
      <Icon name="shield" :size="15" aria-hidden="true" />
      <h3 class="gate-card__title">{{ title }}</h3>
      <span class="gate-card__verdict" :class="`gate-card__verdict--${gate.verdict}`">
        <Icon :name="VERDICT_ICON[gate.verdict]" :size="12" aria-hidden="true" />
        {{ verdictLabel }}
      </span>
    </header>
    <ul class="gate-card__checks">
      <li
        class="gate-card__check"
        :class="gate.searchValid ? 'gate-card__check--ok' : 'gate-card__check--bad'"
      >
        <Icon :name="gate.searchValid ? 'check' : 'alertTriangle'" :size="13" aria-hidden="true" />
        <span>{{ searchLabel }}</span>
      </li>
      <li
        class="gate-card__check"
        :class="gate.artifactsComplete ? 'gate-card__check--ok' : 'gate-card__check--bad'"
      >
        <Icon :name="gate.artifactsComplete ? 'check' : 'alertTriangle'" :size="13" aria-hidden="true" />
        <span>{{ artifactsLabel }}</span>
      </li>
    </ul>
    <p v-if="gate.missing.length" class="gate-card__missing">{{ missingLabel }}</p>
  </section>
</template>

<style scoped>
.gate-card {
  display: grid;
  gap: var(--sp-2);
  padding: var(--sp-3);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  color: var(--text);
}
.gate-card--pass { border-color: var(--ok); }
.gate-card--partial { border-color: var(--warn); }
.gate-card--fail { border-color: var(--danger); }
.gate-card__head { display: flex; align-items: center; gap: var(--sp-2); }
.gate-card__title { margin: 0; font-size: var(--fs-sm); font-weight: 700; flex: 1 1 auto; }
.gate-card__verdict {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 1px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-size: var(--fs-xs);
  white-space: nowrap;
}
.gate-card__verdict--pass { color: var(--ok); border-color: var(--ok); }
.gate-card__verdict--partial { color: var(--warn); border-color: var(--warn); }
.gate-card__verdict--fail { color: var(--danger); border-color: var(--danger); }
.gate-card__checks { display: grid; gap: var(--sp-1); margin: 0; padding: 0; list-style: none; }
.gate-card__check { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-sm); }
.gate-card__check--ok { color: var(--ok); }
.gate-card__check--bad { color: var(--danger); }
.gate-card__missing { margin: 0; color: var(--text-muted); font-size: var(--fs-xs); word-break: break-word; }
@media (prefers-reduced-motion: reduce) { .gate-card * { transition: none !important; animation: none !important; } }
</style>
