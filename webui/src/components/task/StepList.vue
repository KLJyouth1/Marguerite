<!-- StepList — the ordered step清单 + 交付物区 for the current task (T05, P0-4).
     Pure presentational: it renders the steps and artifacts handed to it and
     never invents them. Statuses come from `useTaskWorkspace` (step_start /
     step_finish); artifacts come from `artifact.updated`.
     A4 审查条：done 且落盘产物的步骤进入「待审查」，由人给 通过/打回；
     结论由父级台账（useStepReview）持有，本组件只渲染与上报。 -->
<script setup lang="ts">
import { computed } from 'vue'
import Icon from '@/components/Icon.vue'
import { t } from '@/i18n'
import type { IconName } from '@/utils/icons'
import type { DeliverableKind, TaskStep, TaskStepStatus } from '@/types/rpc'
import type { TaskArtifact } from '@/composables/task/useTaskWorkspace'
import { needsStepReview, type StepReviewRecord, type StepReviewVerdict } from '@/composables/task/useStepReview'

const props = defineProps<{
  steps: TaskStep[]
  artifacts: TaskArtifact[]
  /** 当前 run 的审查台账（stepId → 结论）；缺省 = 无台账（不做审查 UI）。 */
  reviews?: Record<string, StepReviewRecord> | null
}>()

const emit = defineEmits<{
  review: [stepId: string, verdict: StepReviewVerdict]
  'open-artifact': [path: string]
}>()

const STATUS_ICON: Record<TaskStepStatus, IconName> = {
  pending: 'clock',
  in_progress: 'play',
  done: 'check',
  failed: 'alertTriangle',
}

const KIND_ICON: Record<DeliverableKind, IconName> = {
  Research: 'search',
  HtmlPage: 'fileCode',
  Presentation: 'presentation',
  Document: 'fileText',
  Spreadsheet: 'fileSpreadsheet',
  Other: 'archive',
}

const stepsLabel = computed(() => String(t('task.steps')))
const deliverablesLabel = computed(() => String(t('task.deliverables')))
const elapsedLabel = computed(() => String(t('task.elapsed')))
const emptyArtifacts = computed(() => String(t('task.artifacts.empty')))

const stepCount = computed(() => props.steps.length)
const artifactCount = computed(() => props.artifacts.length)

function statusLabel(status: TaskStepStatus): string {
  return String(t(`task.stepStatus.${status}`))
}

function kindIcon(kind: string): IconName {
  return KIND_ICON[kind as DeliverableKind] ?? 'archive'
}

function elapsedText(ms: number | undefined): string {
  if (!ms || ms <= 0) return ''
  const seconds = ms / 1000
  return seconds >= 10 ? `${Math.round(seconds)}s` : `${seconds.toFixed(1)}s`
}

/* ── A4 审查条 ─────────────────────────────────────────────── */

const pendingReviewLabel = computed(() => String(t('task.review.pending')))
const reviewCount = computed(
  () =>
    props.steps.filter(
      (step) => needsStepReview(step) && !props.reviews?.[step.id],
    ).length,
)

type ReviewState = 'pending' | 'approved' | 'flagged'

function reviewStateOf(step: TaskStep): ReviewState | null {
  if (!needsStepReview(step)) return null
  const record = props.reviews?.[step.id]
  if (!record) return 'pending'
  return record.verdict
}

function reviewLabel(state: ReviewState): string {
  return String(t(`task.review.${state}`))
}
</script>

<template>
  <section class="step-list" role="region" :aria-label="stepsLabel">
    <header class="step-list__head">
      <Icon name="listChecks" :size="15" aria-hidden="true" />
      <h3 class="step-list__title">{{ stepsLabel }}</h3>
      <span class="step-list__count" aria-hidden="true">{{ stepCount }}</span>
      <span
        v-if="reviewCount > 0"
        class="step-list__review-chip"
        data-testid="step-review-chip"
      >{{ pendingReviewLabel }} {{ reviewCount }}</span>
    </header>

    <ol class="step-list__steps">
      <li
        v-for="(step, index) in steps"
        :key="step.id"
        class="step-list__row"
        :class="`step-list__row--${step.status}`"
        :data-step-id="step.id"
      >
        <span class="step-list__marker" aria-hidden="true">
          <Icon :name="STATUS_ICON[step.status]" :size="14" />
        </span>
        <span class="step-list__index" aria-hidden="true">{{ index + 1 }}</span>
        <span class="step-list__main">
          <span class="step-list__label">{{ step.title }}</span>
          <span v-if="step.role" class="step-list__role">{{ step.role }}</span>
        </span>
        <span v-if="step.elapsedMs" class="step-list__elapsed">
          {{ elapsedLabel }} {{ elapsedText(step.elapsedMs) }}
        </span>
        <span class="step-list__status" :class="`step-list__status--${step.status}`">
          {{ statusLabel(step.status) }}
        </span>
        <!-- A4 审查条：done + 产物落盘才有；结论来自人工台账，不由 AI 代审 -->
        <span
          v-if="reviewStateOf(step)"
          class="step-list__review"
          :class="`step-list__review--${reviewStateOf(step)}`"
          data-testid="step-review"
        >
          <span
            class="step-list__review-state"
            :data-testid="`step-review-state-${reviewStateOf(step)}`"
          >{{ reviewLabel(reviewStateOf(step)!) }}</span>
          <button
            v-if="step.artifactPath"
            class="step-list__review-btn"
            data-testid="step-review-open"
            @click="emit('open-artifact', step.artifactPath!)"
          >{{ t('task.review.openArtifact') }}</button>
          <span v-if="reviewStateOf(step) === 'pending'" class="step-list__review-actions">
            <button
              class="step-list__review-btn step-list__review-btn--approve"
              data-testid="step-review-approve"
              @click="emit('review', step.id, 'approved')"
            >{{ t('task.review.approve') }}</button>
            <button
              class="step-list__review-btn step-list__review-btn--flag"
              data-testid="step-review-flag"
              @click="emit('review', step.id, 'flagged')"
            >{{ t('task.review.flag') }}</button>
          </span>
        </span>
      </li>
    </ol>

    <section class="step-list__deliverables" :aria-label="deliverablesLabel">
      <header class="step-list__head">
        <Icon name="archive" :size="15" aria-hidden="true" />
        <h3 class="step-list__title">{{ deliverablesLabel }}</h3>
        <span class="step-list__count" aria-hidden="true">{{ artifactCount }}</span>
      </header>
      <ul v-if="artifactCount" class="step-list__artifacts">
        <li
          v-for="artifact in artifacts"
          :key="artifact.path"
          class="step-list__artifact"
          :data-artifact-path="artifact.path"
        >
          <Icon :name="kindIcon(artifact.kind)" :size="14" aria-hidden="true" />
          <span class="step-list__artifact-path">{{ artifact.path }}</span>
        </li>
      </ul>
      <p v-else class="step-list__empty" role="status">{{ emptyArtifacts }}</p>
    </section>
  </section>
</template>

<style scoped>
.step-list { display: grid; gap: var(--sp-4); }
.step-list__head { display: flex; align-items: center; gap: var(--sp-2); }
.step-list__title { margin: 0; font-size: var(--fs-sm); font-weight: 700; }
.step-list__count {
  min-width: 20px; padding: 0 6px; text-align: center;
  border: 1px solid var(--border); border-radius: var(--radius-sm);
  color: var(--text-dim); font-size: var(--fs-xs); font-variant-numeric: tabular-nums;
}
.step-list__steps, .step-list__artifacts { display: grid; gap: var(--sp-2); margin: 0; padding: 0; list-style: none; }
.step-list__row {
  display: flex; flex-wrap: wrap; align-items: center; gap: var(--sp-2);
  padding: var(--sp-2); border: 1px solid var(--border); border-radius: var(--radius-md);
  background: var(--bg-surface); color: var(--text);
}
.step-list__row--in_progress { border-color: var(--info); }
.step-list__row--done { border-color: var(--ok); }
.step-list__row--failed { border-color: var(--danger); }
.step-list__marker { display: inline-grid; place-items: center; width: 22px; height: 22px; color: var(--text-dim); }
.step-list__row--in_progress .step-list__marker { color: var(--info); }
.step-list__row--done .step-list__marker { color: var(--ok); }
.step-list__row--failed .step-list__marker { color: var(--danger); }
.step-list__index { color: var(--text-dim); font-size: var(--fs-xs); font-variant-numeric: tabular-nums; }
.step-list__main { display: flex; flex-direction: column; gap: 1px; flex: 1 1 auto; min-width: 0; }
.step-list__label { font-size: var(--fs-sm); }
.step-list__role { color: var(--text-muted); font-size: var(--fs-xs); }
.step-list__elapsed { color: var(--text-muted); font-size: var(--fs-xs); white-space: nowrap; }
.step-list__status {
  padding: 1px 8px; border: 1px solid var(--border); border-radius: var(--radius-sm);
  font-size: var(--fs-xs); white-space: nowrap;
}
.step-list__status--pending { color: var(--text-dim); }
.step-list__status--in_progress { color: var(--info); }
.step-list__status--done { color: var(--ok); }
.step-list__status--failed { color: var(--danger); }

/* ── A4 审查条 ── */
.step-list__review-chip {
  margin-left: auto;
  padding: 0 8px; border: 1px solid var(--border); border-radius: var(--radius-sm);
  color: var(--warn); font-size: var(--fs-xs); font-variant-numeric: tabular-nums;
}
.step-list__review {
  flex-basis: 100%;
  display: flex; flex-wrap: wrap; align-items: center; gap: var(--sp-2);
  padding: var(--sp-1) var(--sp-2);
  border-top: 1px solid var(--border);
  font-size: var(--fs-xs);
}
.step-list__review-state { font-weight: 600; }
.step-list__review--pending .step-list__review-state { color: var(--warn); }
.step-list__review--approved .step-list__review-state { color: var(--ok); }
.step-list__review--flagged .step-list__review-state { color: var(--danger); }
.step-list__review-actions { display: inline-flex; gap: var(--sp-2); margin-left: auto; }
.step-list__review-btn {
  padding: 1px 8px; border: 1px solid var(--border); border-radius: var(--radius-sm);
  background: var(--bg-elevated); color: var(--text); font-size: var(--fs-xs); cursor: pointer;
}
.step-list__review-btn:hover { border-color: var(--info); color: var(--info); }
.step-list__review-btn--approve:hover { border-color: var(--ok); color: var(--ok); }
.step-list__review-btn--flag:hover { border-color: var(--danger); color: var(--danger); }
.step-list__artifact {
  display: flex; align-items: center; gap: var(--sp-2);
  padding: var(--sp-2); border: 1px solid var(--border); border-radius: var(--radius-md);
  background: var(--bg-elevated); color: var(--text);
}
.step-list__artifact-path { font-size: var(--fs-sm); word-break: break-all; }
.step-list__empty { margin: 0; color: var(--text-muted); font-size: var(--fs-sm); }
@media (prefers-reduced-motion: reduce) { .step-list * { transition: none !important; animation: none !important; } }
</style>
