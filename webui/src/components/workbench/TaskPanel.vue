<!-- TaskPanel — the `task` workbench panel body (T05, P0-4).
     Shares the same `useTaskWorkspace` store as the /task view, so the panel and
     the full view never disagree. Bound to RPC on mount (idempotent). -->
<script setup lang="ts">
import { computed, onMounted } from 'vue'
import { useRpcStore } from '@/stores/rpc'
import { useTaskWorkspaceStore } from '@/composables/task/useTaskWorkspace'
import TaskHeader from '@/components/task/TaskHeader.vue'
import StepList from '@/components/task/StepList.vue'
import DeliverableGateCard from '@/components/task/DeliverableGateCard.vue'
import { t } from '@/i18n'
import type { WorkbenchItem } from '@/workbench/types'

const props = defineProps<{ item: WorkbenchItem }>()

const rpc = useRpcStore()
const task = useTaskWorkspaceStore()

onMounted(() => {
  task.bindToRpc(rpc)
})

// Stable DOM id derived from the descriptor so the node survives re-renders.
const panelId = computed(() => props.item.id)
const title = computed(() => String(t('workbench.task.title')))
const subtitle = computed(() => String(t('workbench.task.subtitle')))
const emptyLabel = computed(() => String(t('workbench.task.empty')))

const hasContent = computed(
  () => task.steps.length > 0 || task.artifacts.length > 0 || task.status !== 'idle',
)

function onRefresh(): void {
  task.refresh()
}
</script>

<template>
  <section :id="panelId" class="task-panel" role="region" :aria-label="title">
    <TaskHeader :title="title" :subtitle="subtitle" :status="task.status" @refresh="onRefresh" />
    <DeliverableGateCard :gate="task.gate" />
    <StepList :steps="task.steps" :artifacts="task.artifacts" />
    <p v-if="!hasContent" class="task-panel__empty" role="status">{{ emptyLabel }}</p>
  </section>
</template>

<style scoped>
.task-panel {
  display: grid;
  align-content: start;
  gap: var(--sp-4);
  height: 100%;
  overflow: auto;
  padding: var(--sp-4);
  color: var(--text);
  background: var(--bg-surface);
}
.task-panel__empty { margin: 0; color: var(--text-muted); font-size: var(--fs-sm); }
@media (max-width: 640px) { .task-panel { padding: var(--sp-3); } }
@media (prefers-reduced-motion: reduce) { .task-panel * { transition: none !important; animation: none !important; } }
</style>
