<!-- TaskHeader — title + coarse lifecycle status + refresh for the task workspace
     (T05, P0-4). Presentational: the status is handed in from the shared
     `useTaskWorkspace` store; `refresh` is emitted so the owning view decides how
     to re-establish the live feed. -->
<script setup lang="ts">
import { computed } from 'vue'
import Icon from '@/components/Icon.vue'
import { t } from '@/i18n'
import type { TaskStatus } from '@/composables/task/useTaskWorkspace'

const props = withDefaults(
  defineProps<{
    /** Overrides the default `task.title` string (the workbench supplies its own). */
    title?: string
    subtitle?: string
    status: TaskStatus
  }>(),
  { title: '', subtitle: '' },
)

const emit = defineEmits<{ (e: 'refresh'): void }>()

const heading = computed(() => props.title || String(t('task.title')))
const sub = computed(() => props.subtitle || String(t('task.subtitle')))
const statusLabel = computed(() => String(t(`task.runStatus.${props.status}`)))
const refreshLabel = computed(() => String(t('task.refresh')))

const STATUS_ICON: Record<TaskStatus, 'clock' | 'play' | 'check' | 'alertTriangle'> = {
  idle: 'clock',
  running: 'play',
  completed: 'check',
  partial: 'clock',
  failed: 'alertTriangle',
}
</script>

<template>
  <header class="task-header">
    <div class="task-header__titles">
      <h2 class="task-header__title">{{ heading }}</h2>
      <p class="task-header__subtitle">{{ sub }}</p>
    </div>
    <div class="task-header__actions">
      <span class="task-header__status" :class="`task-header__status--${status}`">
        <Icon :name="STATUS_ICON[status]" :size="13" aria-hidden="true" />
        {{ statusLabel }}
      </span>
      <button
        type="button"
        class="task-header__refresh"
        :title="refreshLabel"
        :aria-label="refreshLabel"
        @click="emit('refresh')"
      >
        <Icon name="refresh" :size="14" />
      </button>
    </div>
  </header>
</template>

<style scoped>
.task-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--sp-3);
  padding-bottom: var(--sp-3);
  border-bottom: 1px solid var(--border);
}
.task-header__titles { display: grid; gap: var(--sp-1); min-width: 0; }
.task-header__title { margin: 0; font-size: var(--fs-lg); letter-spacing: -0.01em; }
.task-header__subtitle { margin: 0; color: var(--text-muted); font-size: var(--fs-sm); }
.task-header__actions { display: flex; align-items: center; gap: var(--sp-2); flex: none; }
.task-header__status {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-size: var(--fs-xs);
  white-space: nowrap;
  color: var(--text-dim);
}
.task-header__status--running { color: var(--info); border-color: var(--info); }
.task-header__status--completed { color: var(--ok); border-color: var(--ok); }
.task-header__status--partial { color: var(--warn); border-color: var(--warn); }
.task-header__status--failed { color: var(--danger); border-color: var(--danger); }
.task-header__refresh {
  display: inline-grid;
  place-items: center;
  width: 28px;
  height: 28px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-elevated);
  color: var(--text-dim);
  cursor: pointer;
  transition: color var(--dur-fast) var(--ease-standard), border-color var(--dur-fast) var(--ease-standard);
}
.task-header__refresh:hover { color: var(--accent); border-color: var(--accent); }
@media (prefers-reduced-motion: reduce) { .task-header * { transition: none !important; animation: none !important; } }
</style>
