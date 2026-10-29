<!-- TaskWorkspaceView (/task) — the live task workspace (T05, P0-4).
     Reads the shared `useTaskWorkspace` store (fed by the router/step/artifact
     events) and lays out: header + mode impact + quality gate + step/deliverable
     list. The store is bound once on mount; the RPC subscription is idempotent so
     the workbench panel can bind the same store without double-subscribing.
     A4 待审查门：done 且落盘产物的步骤由人工给出 通过/打回（台账持久化）。
     诚实边界：chat 编排链后端无步间暂停点，审查台账不阻断流水线 —— hint 原文照说。 -->
<script setup lang="ts">
import { computed, onMounted } from 'vue'
import { useRpcStore } from '@/stores/rpc'
import { useTaskWorkspaceStore } from '@/composables/task/useTaskWorkspace'
import { createStepReview, stepReviewRunKey, type StepReviewRecord, type StepReviewVerdict } from '@/composables/task/useStepReview'
import { activeMode, switchError } from '@/composables/useActiveMode'
import TaskHeader from '@/components/task/TaskHeader.vue'
import StepList from '@/components/task/StepList.vue'
import DeliverableGateCard from '@/components/task/DeliverableGateCard.vue'
import ModeImpactCard from '@/components/task/ModeImpactCard.vue'
import { t } from '@/i18n'

const rpc = useRpcStore()
const task = useTaskWorkspaceStore()
const review = createStepReview()

onMounted(() => {
  // Idempotent: safe even if the workbench panel bound the same store first.
  task.bindToRpc(rpc)
})

const viewLabel = computed(() => String(t('task.title')))
const emptyLabel = computed(() => String(t('task.empty')))
const modeKind = computed(() => activeMode.value?.kind ?? activeMode.value?.mode ?? '')

// Show the empty hint only when nothing at all has happened yet — a finished
// task with zero artifacts is NOT empty (its gate/steps carry the outcome).
const hasContent = computed(
  () => task.steps.length > 0 || task.artifacts.length > 0 || task.status !== 'idle',
)

function onRefresh(): void {
  task.refresh()
}

/* ── A4 审查台账接线 ─────────────────────────────────────────── */

const reviewRunKey = computed(() => stepReviewRunKey(task.sessionKey, task.taskId))

const reviewByStepId = computed<Record<string, StepReviewRecord>>(() => {
  const bucket = review.reviews.value[reviewRunKey.value]
  return bucket ?? {}
})

function onReview(stepId: string, verdict: StepReviewVerdict): void {
  review.setReview(reviewRunKey.value, stepId, verdict)
}

function onOpenArtifact(path: string): void {
  if (!path) return
  // 台账 RPC：裸 path 只允许工作区根目录内的普通文件（core W3.2 越界拒绝）。
  if (!rpc.supportsMethod('artifacts.open_in_system')) return
  void rpc.call('artifacts.open_in_system', { path }).catch(() => {
    /* 打开失败不伪造成功；产物路径仍在清单上可复核 */
  })
}

const reviewHint = computed(() => String(t('task.review.hint')))
</script>

<template>
  <section class="task-view" role="region" :aria-label="viewLabel">
    <TaskHeader :status="task.status" @refresh="onRefresh" />
    <ModeImpactCard
      :mode-kind="modeKind"
      :mode-route="task.modeRoute"
      :strategy="task.strategy"
      :switch-error="switchError"
    />
    <DeliverableGateCard :gate="task.gate" />
    <StepList
      :steps="task.steps"
      :artifacts="task.artifacts"
      :reviews="reviewByStepId"
      @review="onReview"
      @open-artifact="onOpenArtifact"
    />
    <p v-if="task.steps.length" class="task-view__review-hint" role="note">{{ reviewHint }}</p>
    <p v-if="!hasContent" class="task-view__empty" role="status">{{ emptyLabel }}</p>
  </section>
</template>

<style scoped>
.task-view {
  display: grid;
  align-content: start;
  gap: var(--sp-4);
  height: 100%;
  overflow: auto;
  padding: var(--sp-4);
  color: var(--text);
  background: var(--bg);
}
.task-view__empty { margin: 0; color: var(--text-muted); font-size: var(--fs-sm); }
.task-view__review-hint { margin: 0; color: var(--text-muted); font-size: var(--fs-xs); }
@media (max-width: 640px) { .task-view { padding: var(--sp-3); } }
@media (prefers-reduced-motion: reduce) { .task-view * { transition: none !important; animation: none !important; } }
</style>
