<template>
  <section
    ref="rootElement"
    class="plan-run"
    :class="`plan-run--${run.status}`"
    :data-plan-run-id="run.runId"
    role="region"
    :aria-label="t('chat.planRun.title')"
  >
    <template v-if="hasInspectableSteps">
      <div class="plan-run__control">
        <div
          class="plan-run__disclosure"
          :class="{ 'plan-run__disclosure--open': open }"
          @pointerenter="onPointerEnter"
          @pointerleave="onPointerLeave"
          @focusin="onFocusIn"
          @focusout="onFocusOut"
          @keydown.esc.stop.prevent="onEscape"
        >
          <button
            ref="summaryButton"
            type="button"
            class="plan-run__summary"
            :aria-label="regionLabel"
            :aria-expanded="open"
            :aria-controls="stepsId"
            @pointerdown="onPointerDown"
            @pointerup="onPointerUp"
            @pointercancel="onPointerCancel"
            @keydown="onSummaryKeydown"
            @click="onSummaryClick"
          >
            <span class="plan-run__marker-slot">
              <Transition name="plan-run-marker" mode="out-in">
                <ExecutionTodoMarker
                  :key="summaryMarkerKey"
                  :status="summaryMarkerStatus"
                />
              </Transition>
            </span>
            <span class="plan-run__summary-label">
              {{ deliveryReady
                ? t('chat.planRun.finishing', {
                    completed: completedCount,
                    total: run.steps.length,
                  })
                : isRunning
                ? t('chat.planRun.progress', {
                    current: summaryOrdinal,
                    total: run.steps.length,
                  })
                : runStatusLabel }}
            </span>
            <!-- v3.3：任务类型徽章（编码/文档/检索/对话…），随 step_progress 事件更新 -->
            <span
              v-if="stepKind"
              class="plan-run__task-kind"
              role="status"
            >{{ stepKind }}</span>
            <!-- M2-C3：实时时序（真实事件流：已耗时/预计剩余，基于 core runner 实测） -->
            <span
              v-if="timingEtaText"
              class="plan-run__timing"
              role="status"
              :aria-label="timingEtaText"
            >{{ timingEtaText }}</span>
            <span
              v-if="!isRunning"
              class="plan-run__summary-count"
              aria-hidden="true"
            >
              <span aria-hidden="true">·</span>
              <span>
                {{ summaryOrdinal }}/{{ run.steps.length }}
              </span>
            </span>
            <Icon
              class="plan-run__chevron"
              :class="{ 'plan-run__chevron--open': open }"
              name="chevronRight"
              :size="14"
              aria-hidden="true"
            />
          </button>

          <Transition name="plan-run-popover">
            <div
              v-if="hasSteps && open"
              :id="stepsId"
              class="plan-run__popover"
            >
              <TransitionGroup
                tag="ol"
                name="plan-run-step"
                class="plan-run__steps"
                :aria-label="t('chat.planRun.title')"
              >
                <li
                  v-for="step in run.steps"
                  :key="step.stepId"
                  class="plan-run__step"
                  :class="`plan-run__step--${step.status}`"
                  :data-step-id="step.stepId"
                  :aria-current="isCurrent(step.stepId) ? 'step' : undefined"
                >
                  <span class="plan-run__marker-slot">
                    <Transition name="plan-run-marker" mode="out-in">
                      <ExecutionTodoMarker
                        :key="`${step.stepId}:${step.status}`"
                        :status="step.status"
                      />
                    </Transition>
                  </span>
                  <span class="plan-run__step-copy">
                    <span class="plan-run__step-title">{{ step.title }}</span>
                    <span
                      v-if="safeRunReason(step.reason)"
                      class="plan-run__step-reason"
                    >{{ safeRunReason(step.reason) }}</span>
                  </span>
                  <span class="plan-run__step-state">
                    {{ stepStatusLabel(step.status) }}
                  </span>
                </li>
              </TransitionGroup>

              <!-- B4 执行前清单：每步一项，勾选进度（展示只读） -->
              <section
                v-if="hasChecklist"
                class="plan-run__b4"
                :aria-label="t('chat.planRun.preflight')"
              >
                <header class="plan-run__b4-head">
                  <Icon name="listChecks" :size="14" aria-hidden="true" />
                  <span class="plan-run__b4-title">{{ t('chat.planRun.preflight') }}</span>
                  <span class="plan-run__b4-count">
                    {{ checkedChecklistCount }}/{{ checklistItems.length }}
                  </span>
                </header>
                <ol class="plan-run__checklist">
                  <li
                    v-for="item in checklistItems"
                    :key="item.stepId"
                    class="plan-run__checklist-item"
                    :class="{ 'plan-run__checklist-item--checked': item.checked }"
                    :data-step-id="item.stepId"
                  >
                    <span class="plan-run__checklist-box" aria-hidden="true">
                      <Icon v-if="item.checked" name="check" :size="11" />
                    </span>
                    <span class="plan-run__checklist-title">{{ item.title }}</span>
                  </li>
                </ol>
              </section>

              <!-- B4 并行子任务：真执行聚合结果（离线回退时 simulated 徽标如实声明） -->
              <section
                v-if="parallelBatch && parallelBatch.results.length"
                class="plan-run__b4"
                :aria-label="t('chat.planRun.parallel')"
              >
                <header class="plan-run__b4-head">
                  <Icon name="fork" :size="14" aria-hidden="true" />
                  <span class="plan-run__b4-title">{{ t('chat.planRun.parallel') }}</span>
                  <span class="plan-run__b4-count">
                    {{ parallelBatch.successful }}/{{ parallelBatch.total }}
                  </span>
                  <span
                    v-if="parallelBatch.simulated"
                    class="plan-run__b4-simulated"
                    :title="t('chat.planRun.parallelSimulated')"
                  >
                    {{ t('chat.planRun.parallelSimulated') }}
                  </span>
                </header>
                <ol class="plan-run__parallel">
                  <li
                    v-for="result in parallelBatch.results"
                    :key="result.stepId"
                    class="plan-run__parallel-item"
                    :class="`plan-run__parallel-item--${result.status}`"
                  >
                    <span class="plan-run__marker-slot">
                      <ExecutionTodoMarker
                        :status="result.status === 'completed' ? 'completed' : 'blocked'"
                      />
                    </span>
                    <span class="plan-run__parallel-copy">
                      <span class="plan-run__parallel-title">{{ result.title }}</span>
                      <span v-if="result.summary" class="plan-run__parallel-summary">
                        {{ result.summary }}
                      </span>
                    </span>
                    <!-- 真实耗时/重试次数（core runner 实测回传）；无数据即不显示，绝不补 0 冒充 -->
                    <span class="plan-run__parallel-meta">
                      <span
                        v-if="result.elapsedMs !== undefined"
                        class="plan-run__parallel-chip"
                        :title="t('chat.planRun.parallelStepDuration', { duration: formatDuration(result.elapsedMs) })"
                      >
                        {{ formatDuration(result.elapsedMs) }}
                      </span>
                      <span
                        v-if="result.attempts !== undefined && result.attempts > 1"
                        class="plan-run__parallel-chip plan-run__parallel-chip--retry"
                        :title="t('chat.planRun.parallelStepRetries', { count: result.attempts })"
                      >
                        {{ t('chat.planRun.parallelStepRetries', { count: result.attempts }) }}
                      </span>
                    </span>
                  </li>
                </ol>
              </section>

              <!-- B4 动态重规划：偏离观测后的新修订 diff / 前瞻跳步 -->
              <section
                v-if="hasReplan"
                class="plan-run__b4"
                :aria-label="replanTitleLabel"
              >
                <header class="plan-run__b4-head">
                  <Icon name="refresh" :size="14" aria-hidden="true" />
                  <span class="plan-run__b4-title">{{ replanTitleLabel }}</span>
                  <span v-if="replanInfo" class="plan-run__b4-count">
                    {{ replanSimilarityPercent }}
                  </span>
                </header>
                <p v-if="replanInfo?.prompt" class="plan-run__replan-prompt">
                  {{ replanInfo.prompt }}
                </p>
                <p class="plan-run__replan-detail">{{ replanDetailLabel }}</p>
              </section>
              <div v-if="statusReason" class="plan-run__popover-reason">
                {{ statusReason }}
              </div>
              <div v-if="canCancel" class="plan-run__actions">
                <button
                  type="button"
                  class="plan-run__end"
                  :disabled="cancelBusy || disabled"
                  @click.stop="onEndPlan"
                >
                  {{ cancelBusy
                    ? t('chat.planRun.endingPlan')
                    : t('chat.planRun.endPlan') }}
                </button>
              </div>
            </div>
          </Transition>
        </div>
        <span
          class="plan-run__sr-only"
          role="status"
          aria-live="polite"
          aria-atomic="true"
        >{{ liveStatusLabel }}</span>
      </div>
    </template>

    <div
      v-else-if="isRunning"
      class="plan-run__summary plan-run__summary--static"
      role="status"
    >
      <ExecutionTodoMarker status="in_progress" />
      <span class="plan-run__title">{{ t('chat.planRun.running') }}</span>
    </div>

    <div v-else class="plan-run__static" role="status">
      <ExecutionTodoMarker :status="staticTodoStatus" />
      <span class="plan-run__static-copy">
        <span class="plan-run__static-head">
          <span class="plan-run__title">{{ runStatusLabel }}</span>
          <span v-if="staticProgressText" class="plan-run__static-status">
            {{ staticProgressText }}
          </span>
        </span>
        <span v-if="statusReason" class="plan-run__reason">{{ statusReason }}</span>
      </span>
    </div>
  </section>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, useId, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import ExecutionTodoMarker from '@/components/chat/ExecutionTodoMarker.vue'
import type {
  PlanParallelBatch,
  PlanReplanInfo,
  PlanRunChecklistItem,
  PlanRunSnapshot,
  PlanRunStatus,
  PlanRunStepStatus,
  PlanRunTiming,
} from '@/types/plans'

const props = withDefaults(defineProps<{
  run: PlanRunSnapshot
  defaultOpen?: boolean
  cancelBusy?: boolean
  disabled?: boolean
  /** v3.3：当前步骤任务类型徽章（编码/文档/对话…，空串不显示） */
  stepKind?: string
  /** M2-C3：计划运行实时时序（真实事件流：elapsedMs/etaMs） */
  timing?: PlanRunTiming | null
}>(), {
  defaultOpen: false,
  cancelBusy: false,
  disabled: false,
})

const emit = defineEmits<{
  cancel: []
  focusReturn: []
}>()

const { t } = useI18n()
const rootElement = ref<HTMLElement | null>(null)
const summaryButton = ref<HTMLButtonElement | null>(null)
const pointerWithin = ref(false)
const focusWithin = ref(false)
const activationOpen = ref<boolean | null>(props.defaultOpen ? true : null)
const dismissed = ref(false)
const open = computed(() => !dismissed.value && (
  pointerWithin.value
  || activationOpen.value === true
  || (focusWithin.value && activationOpen.value !== false)
))
let lastPointerType = ''
let suppressNextFocusOpen = false
const stepsId = `plan-run-steps-${useId()}`
const isRunning = computed(() => props.run.status === 'running')

// N1：`timing.elapsedMs` 只在 step_progress 事件到达时更新，事件之间会「冻结」
// （真机截图：思考已 173s，进度条仍显示「已用 1m16s」）。这里以事件携带的
// `atMs`（本地收报时刻）为基准做本地续算，让「已用」按秒真实走动。
const nowTick = ref(Date.now())
let elapsedTimer: ReturnType<typeof setInterval> | null = null
function stopElapsedTicker() {
  if (elapsedTimer) { clearInterval(elapsedTimer); elapsedTimer = null }
}
watch(isRunning, (running) => {
  if (running && !elapsedTimer) {
    nowTick.value = Date.now()
    elapsedTimer = setInterval(() => { nowTick.value = Date.now() }, 1000)
  } else if (!running) {
    stopElapsedTicker()
  }
}, { immediate: true })
onBeforeUnmount(stopElapsedTicker)
const hasSteps = computed(() => props.run.steps.length > 0)
const inspectableStatuses = new Set<PlanRunStatus>([
  'queued',
  'running',
  'paused',
  'blocked',
])
const hasInspectableSteps = computed(() =>
  hasSteps.value && inspectableStatuses.has(props.run.status),
)
const canCancel = computed(() =>
  props.run.status === 'paused'
  || props.run.status === 'blocked',
)
const RUN_REASON_MAX_CHARS = 160

function safeRunReason(value: string | undefined): string {
  if (!value) return ''
  const compact = [...value]
    .map((character) => {
      const code = character.codePointAt(0) ?? 0
      return code < 32 || (code >= 127 && code <= 159) ? ' ' : character
    })
    .join('')
    .replace(/\s+/g, ' ')
    .trim()
  const characters = [...compact]
  if (characters.length <= RUN_REASON_MAX_CHARS) return compact
  return `${characters.slice(0, RUN_REASON_MAX_CHARS - 1).join('')}…`
}

const statusReason = computed(() => {
  if (props.run.status === 'paused' || props.run.status === 'blocked') {
    return safeRunReason(props.run.pauseReason || props.run.terminalReason)
  }
  if (props.run.status === 'cancelled' || props.run.status === 'superseded') {
    return safeRunReason(props.run.terminalReason || props.run.pauseReason)
  }
  return ''
})

watch(
  () => props.run.runId,
  () => {
    pointerWithin.value = false
    focusWithin.value = false
    activationOpen.value = props.defaultOpen ? true : null
    dismissed.value = false
    lastPointerType = ''
    suppressNextFocusOpen = false
  },
)

const terminalRunStatuses = new Set<PlanRunStatus>([
  'completed',
  'cancelled',
  'superseded',
])

watch(
  () => props.run.status,
  (status, previousStatus) => {
    if (
      !terminalRunStatuses.has(status)
      || terminalRunStatuses.has(previousStatus)
      || typeof document === 'undefined'
    ) return
    const activeElement = document.activeElement
    if (activeElement && rootElement.value?.contains(activeElement)) {
      emit('focusReturn')
    }
  },
  { flush: 'pre' },
)

function isTouchLikePointer(pointerType: string): boolean {
  return pointerType === 'touch' || pointerType === 'pen'
}

function onPointerDown(event: PointerEvent): void {
  lastPointerType = event.pointerType
  suppressNextFocusOpen = Boolean(event.pointerType)
  if (event.pointerType === 'mouse') {
    activationOpen.value = null
    dismissed.value = false
  }
}

function onPointerUp(): void {
  suppressNextFocusOpen = false
}

function onPointerCancel(): void {
  lastPointerType = ''
  suppressNextFocusOpen = false
}

function onPointerEnter(event: PointerEvent): void {
  if (event.pointerType === 'mouse' || event.pointerType === '') {
    activationOpen.value = null
    dismissed.value = false
    lastPointerType = 'mouse'
    pointerWithin.value = true
  }
}

function onPointerLeave(event: PointerEvent): void {
  if (event.pointerType === 'mouse' || event.pointerType === '') {
    pointerWithin.value = false
  }
}

function onFocusIn(): void {
  if (suppressNextFocusOpen) {
    suppressNextFocusOpen = false
    return
  }
  if (!focusWithin.value) {
    activationOpen.value = null
    dismissed.value = false
  }
  focusWithin.value = true
}

function onFocusOut(event: FocusEvent): void {
  const disclosure = event.currentTarget as HTMLElement | null
  const nextTarget = event.relatedTarget
  if (
    disclosure
    && nextTarget instanceof Node
    && disclosure.contains(nextTarget)
  ) {
    return
  }
  focusWithin.value = false
  activationOpen.value = null
  dismissed.value = false
}

function toggleActivation(): void {
  const shouldOpen = !open.value
  activationOpen.value = shouldOpen
  dismissed.value = !shouldOpen
}

function onSummaryKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    onEscape()
    return
  }
  if (event.key !== 'Enter' && event.key !== ' ') return
  pointerWithin.value = false
  lastPointerType = ''
  suppressNextFocusOpen = false
}

function onSummaryClick(event: MouseEvent): void {
  const eventPointerType = 'pointerType' in event
    ? String((event as PointerEvent).pointerType || '')
    : ''
  const pointerType = eventPointerType || lastPointerType
  const syntheticActivation = event.detail === 0
  if (!syntheticActivation && !isTouchLikePointer(pointerType)) return
  pointerWithin.value = false
  toggleActivation()
  lastPointerType = ''
  suppressNextFocusOpen = false
}

function onEscape(): void {
  pointerWithin.value = false
  activationOpen.value = false
  dismissed.value = true
  summaryButton.value?.focus({ preventScroll: true })
}

function onEndPlan(): void {
  emit('cancel')
  emit('focusReturn')
}

const currentOrdinal = computed(() => {
  if (!props.run.steps.length) return 0
  const explicit = props.run.steps.findIndex(step => step.stepId === props.run.currentStepId)
  if (explicit >= 0) return explicit + 1
  const inProgress = props.run.steps.findIndex(step => step.status === 'in_progress')
  if (inProgress >= 0) return inProgress + 1
  const completed = props.run.steps.filter(step =>
    step.status === 'completed' || step.status === 'skipped',
  ).length
  return Math.min(completed + 1, props.run.steps.length)
})
const completedCount = computed(() => props.run.steps.filter(step =>
  step.status === 'completed' || step.status === 'skipped',
).length)
const deliveryReady = computed(() =>
  isRunning.value
  && !props.run.currentStepId
  && hasSteps.value
  && completedCount.value === props.run.steps.length,
)
const currentStep = computed(() => props.run.steps[currentOrdinal.value - 1])
const currentStepStatus = computed<PlanRunStepStatus>(() =>
  currentStep.value?.status || 'in_progress',
)
const summaryMarkerStatus = computed<PlanRunStepStatus>(() => {
  if (deliveryReady.value) return 'in_progress'
  if (props.run.status === 'running') return currentStepStatus.value
  if (props.run.status === 'blocked') return 'blocked'
  return 'pending'
})
const summaryOrdinal = computed(() =>
  props.run.status === 'queued' ? completedCount.value : currentOrdinal.value,
)
const summaryTitle = computed(() =>
  deliveryReady.value
    ? String(t('chat.planRun.finishing', {
        completed: completedCount.value,
        total: props.run.steps.length,
      }))
    : currentStep.value?.title || runStatusLabel.value,
)
const summaryMarkerKey = computed(() =>
  `${props.run.status}:${props.run.currentStepId || currentOrdinal.value}:${summaryMarkerStatus.value}`,
)

const runStatusLabel = computed(() => String(t(`chat.planRun.status.${props.run.status}`)))
const staticProgressText = computed(() => hasSteps.value
  ? `${completedCount.value}/${props.run.steps.length}`
  : '')

const regionLabel = computed(() => {
  if (hasInspectableSteps.value) {
    const status = deliveryReady.value
      ? String(t('chat.planRun.regionFinishing', {
          total: props.run.steps.length,
        }))
      : isRunning.value
      ? String(t('chat.planRun.regionRunning', {
          current: summaryOrdinal.value,
          total: props.run.steps.length,
        }))
      : String(t('chat.planRun.regionStatus', { status: runStatusLabel.value }))
    return `${status}: ${summaryTitle.value}`
  }
  return String(t('chat.planRun.regionStatus', { status: runStatusLabel.value }))
})
const liveStatusLabel = computed(() =>
  isRunning.value ? summaryTitle.value : runStatusLabel.value,
)

/** N1：以事件收报时刻（atMs）续算「已用」，避免事件之间冻结。 */
const liveElapsedMs = computed<number | undefined>(() => {
  const timing = props.timing
  const live = timing?.phase === 'started' ? timing : undefined
  if (!live) return undefined
  return live.atMs
    ? live.elapsedMs + Math.max(0, nowTick.value - live.atMs)
    : live.elapsedMs
})

/** M2-C3：实时时序文案（running 时展示；事件流缺位则用运行快照的实测代价模型兜底） */
const timingEtaText = computed(() => {
  if (!isRunning.value) return ''
  const timing = props.timing
  const live = timing?.phase === 'started' ? timing : undefined
  // 页面重载 / 会话切换后 `step_progress` 事件流缺位，此时 ETA 只能来自运行快照
  // 的代价模型（同一份实测样本口径）——否则进度条会退化成「只有百分比没有剩余」。
  const snapshotEta = props.run.cost?.estimatedRemainingMs
  const etaMs = live?.etaMs !== undefined && live.etaMs > 0
    ? live.etaMs
    : snapshotEta !== null && snapshotEta !== undefined && snapshotEta > 0
      ? snapshotEta
      : undefined
  const eta = etaMs !== undefined
    ? String(t('chat.planRun.eta', { eta: formatDuration(etaMs) }))
    : ''
  const elapsed = liveElapsedMs.value !== undefined
    ? String(t('chat.planRun.elapsed', { elapsed: formatDuration(liveElapsedMs.value) }))
    : ''
  if (eta && elapsed) return `${eta} · ${elapsed}`
  return eta || elapsed
})

/** M2-C3：毫秒 → 人类可读时长（<1m 按秒，<1h 按分秒，否则时+分） */
function formatDuration(ms: number): string {
  const totalSec = Math.max(0, Math.round((ms || 0) / 1000))
  const h = Math.floor(totalSec / 3600)
  const m = Math.floor((totalSec % 3600) / 60)
  const s = totalSec % 60
  if (h > 0) return `${h}h ${m}m`
  if (m > 0) return `${m}m ${s}s`
  return `${s}s`
}

const staticTodoStatus = computed<PlanRunStepStatus>(() => {
  const statuses: Record<Exclude<PlanRunStatus, 'running'>, PlanRunStepStatus> = {
    queued: 'pending',
    paused: 'pending',
    blocked: 'blocked',
    completed: 'completed',
    cancelled: 'skipped',
    superseded: 'skipped',
  }
  return props.run.status === 'running' ? 'in_progress' : statuses[props.run.status]
})

function isCurrent(stepId: string): boolean {
  if (deliveryReady.value) return false
  if (props.run.currentStepId) return props.run.currentStepId === stepId
  return props.run.steps[currentOrdinal.value - 1]?.stepId === stepId
}

function stepStatusLabel(status: PlanRunStepStatus): string {
  return String(t(`chat.planRun.stepStatus.${status}`))
}

// ── B4：执行前清单 / 并行子任务 / 动态重规划（只读展示）──
const checklistItems = computed<PlanRunChecklistItem[]>(() => props.run.checklist ?? [])
const hasChecklist = computed(() => checklistItems.value.length > 0)
const checkedChecklistCount = computed(() =>
  checklistItems.value.filter(item => item.checked).length,
)
const parallelBatch = computed<PlanParallelBatch | null>(() => props.run.parallelBatch ?? null)
const replanInfo = computed<PlanReplanInfo | null>(() => props.run.replan ?? null)
const hasReplan = computed(() => replanInfo.value !== null)
/** 前瞻跳步（观测命中后续步骤）与「重新拆解」是两种截然不同的判定，文案必须分开。 */
const isReplanAdvance = computed(() => replanInfo.value?.mode === 'advance')
const replanTitleLabel = computed(() => String(t(
  isReplanAdvance.value ? 'chat.planRun.replannedAhead' : 'chat.planRun.replanned',
)))
const replanSimilarityPercent = computed(() => {
  if (!replanInfo.value) return ''
  const pct = Math.round(replanInfo.value.similarity * 100)
  return `${Math.max(0, Math.min(100, pct))}%`
})
const replanDetailLabel = computed(() => {
  const info = replanInfo.value
  if (!info) return ''
  if (info.mode === 'advance') {
    return String(t('chat.planRun.replanAdvanceDetail', {
      step: info.bestMatch?.title || info.bestMatch?.stepId || '',
      count: info.skippedStepIds?.length ?? info.stepsChanged,
    }))
  }
  return String(t('chat.planRun.replanDetail', {
    steps: info.stepsChanged,
    revision: info.revisionId,
  }))
})

</script>

<style scoped>
.plan-run {
  position: relative;
  width: min(440px, calc(100vw - 24px));
  max-width: 100%;
  flex-shrink: 0;
  color: var(--text);
}

.plan-run__control {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
  width: 100%;
  max-width: calc(100vw - 24px);
  align-items: center;
}

.plan-run__disclosure {
  position: relative;
  grid-column: 2;
  width: max-content;
  min-width: 0;
  max-width: 100%;
}

.plan-run__disclosure--open::after {
  position: absolute;
  bottom: 100%;
  left: 50%;
  width: min(420px, calc(100vw - 24px));
  height: 10px;
  transform: translateX(-50%);
  content: '';
}

.plan-run__summary,
.plan-run__static {
  display: flex;
  width: max-content;
  max-width: 100%;
  align-items: center;
  min-width: 0;
  min-height: 36px;
  gap: 7px;
  padding: 6px 11px;
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: var(--bg-surface);
  box-shadow: none;
  color: inherit;
  font: inherit;
  text-align: left;
}

.plan-run__summary {
  position: relative;
  width: 100%;
  flex: none;
  cursor: pointer;
  transition:
    border-color var(--dur-fast) var(--ease-standard),
    background var(--dur-fast) var(--ease-standard);
}

.plan-run__static {
  margin-inline: auto;
}

.plan-run__summary::before {
  position: absolute;
  inset: -4px 0;
  content: '';
}

.plan-run__summary--static {
  cursor: default;
}

.plan-run__summary:hover {
  border-color: var(--border);
  background: var(--bg-hover);
}

.plan-run__summary[aria-expanded="true"] {
  border-color: var(--border);
  background: var(--bg-hover);
}

.plan-run__summary--static:hover {
  border-color: var(--border);
  background: var(--bg-surface);
}

.plan-run__summary:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}

.plan-run__marker-slot {
  display: grid;
  width: 18px;
  height: 18px;
  flex: none;
  place-items: center;
}

.plan-run__sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}

.plan-run__title {
  min-width: 0;
  overflow: hidden;
  font-size: var(--fs-sm);
  font-weight: 650;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.plan-run__summary-label,
.plan-run__summary-count {
  color: var(--text-muted);
  font-size: var(--fs-sm);
  font-weight: 500;
  line-height: 1;
  white-space: nowrap;
}

.plan-run__summary-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* v3.3：任务类型徽章（编码/文档/检索/对话…）——accent 弱底 + 圆角胶囊 */
.plan-run__task-kind {
  background: color-mix(in srgb, var(--accent) 14%, transparent);
  border-radius: 999px;
  color: var(--accent);
  flex: 0 0 auto;
  font-size: var(--fs-xs);
  font-weight: 600;
  line-height: 1;
  padding: 4px 8px;
  white-space: nowrap;
}

/* M2-C3：实时时序（已耗时/预计剩余）——弱化辅助展示，随事件流刷新 */
.plan-run__timing {
  color: var(--text-muted);
  flex: 0 0 auto;
  font-size: var(--fs-xs);
  font-variant-numeric: tabular-nums;
  line-height: 1;
  white-space: nowrap;
}

.plan-run__static-status {
  color: var(--text-muted);
  font-size: var(--fs-xs);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.plan-run__summary-count {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-variant-numeric: tabular-nums;
}

.plan-run__static-copy {
  display: grid;
  min-width: 0;
  flex: 1 1 auto;
  gap: 2px;
}

.plan-run__static-head {
  display: flex;
  min-width: 0;
  align-items: baseline;
  gap: var(--sp-2);
}

.plan-run__static-head .plan-run__title {
  flex: 0 1 auto;
}

.plan-run__reason {
  max-width: 260px;
  overflow: hidden;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  line-height: 1.35;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.plan-run__chevron {
  flex: none;
  color: var(--text-muted);
  transition: transform var(--dur-fast) var(--ease-standard);
}

.plan-run__chevron--open {
  transform: rotate(90deg);
}

.plan-run__popover {
  position: absolute;
  bottom: calc(100% + 8px);
  left: 50%;
  width: min(420px, calc(100vw - 24px));
  max-height: min(50vh, 360px);
  overflow-y: auto;
  transform: translateX(-50%);
  transform-origin: bottom center;
  z-index: 1;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-elevated);
  box-shadow: var(--shadow-sm);
  scrollbar-gutter: stable;
}

.plan-run__steps {
  display: grid;
  gap: 0;
  margin: 0;
  padding: 4px 12px;
  list-style: none;
}

.plan-run__step {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  min-height: 38px;
  gap: 9px;
  padding: 8px 2px;
  border-bottom: 1px solid var(--border);
  color: var(--text-muted);
  font-size: var(--fs-sm);
  transition:
    background var(--dur-fast) var(--ease-standard),
    color var(--dur-fast) var(--ease-standard);
}

.plan-run__step:last-child {
  border-bottom: 0;
}

.plan-run__step--in_progress {
  color: var(--text);
}

.plan-run__step--completed,
.plan-run__step--skipped {
  color: var(--text-muted);
}

.plan-run__step--blocked {
  color: var(--text);
}

.plan-run__step-title {
  min-width: 0;
  display: -webkit-box;
  overflow: hidden;
  font-weight: 500;
  line-height: 1.4;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  overflow-wrap: anywhere;
}

.plan-run__step--in_progress .plan-run__step-title,
.plan-run__step--blocked .plan-run__step-title {
  font-weight: 650;
}

.plan-run__step-copy {
  display: grid;
  min-width: 0;
  gap: 2px;
}

.plan-run__step-state {
  color: var(--text-muted);
  font-size: var(--fs-xs);
  line-height: 1.25;
  white-space: nowrap;
}

.plan-run__step--in_progress .plan-run__step-state {
  color: var(--text-muted);
}

.plan-run__step--blocked .plan-run__step-state {
  color: var(--danger);
}

.plan-run__step-reason {
  color: var(--text-muted);
  font-size: var(--fs-xs);
  font-weight: 400;
  line-height: 1.35;
  overflow-wrap: anywhere;
}

.plan-run__popover-reason {
  padding: var(--sp-2) var(--sp-3);
  border-top: 1px solid var(--border);
  color: var(--text-muted);
  font-size: var(--fs-xs);
  line-height: 1.4;
}

/* ── B4：执行前清单 / 并行子任务 / 动态重规划 ── */
.plan-run__b4 {
  display: grid;
  gap: var(--sp-1);
  padding: var(--sp-2) 12px;
  border-top: 1px solid var(--border);
}

.plan-run__b4-head {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 6px;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  font-weight: 650;
  letter-spacing: 0.01em;
  text-transform: uppercase;
}

.plan-run__b4-head .icon {
  color: var(--text-muted);
}

.plan-run__b4-title {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.plan-run__b4-count {
  flex: none;
  margin-left: auto;
  font-variant-numeric: tabular-nums;
}

/* 离线回退的确定性标记：如实声明未真正执行（v19 真执行化配套） */
.plan-run__b4-simulated {
  flex: none;
  padding: 1px 6px;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--warn) 16%, transparent);
  color: var(--warn);
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.02em;
}

.plan-run__checklist,
.plan-run__parallel {
  display: grid;
  gap: 0;
  margin: 0;
  padding: 0;
  list-style: none;
}

.plan-run__checklist-item {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 9px;
  padding: 5px 2px;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}

.plan-run__checklist-box {
  display: inline-grid;
  width: 16px;
  height: 16px;
  flex: 0 0 auto;
  place-items: center;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-xs);
  background: var(--bg-surface);
  color: var(--ok);
}

.plan-run__checklist-item--checked .plan-run__checklist-box {
  border-color: color-mix(in srgb, var(--ok) 54%, var(--border));
  background: color-mix(in srgb, var(--ok) 12%, var(--bg-surface));
}

.plan-run__checklist-item--checked .plan-run__checklist-title {
  color: var(--text-muted);
}

.plan-run__checklist-title {
  min-width: 0;
  overflow: hidden;
  line-height: 1.4;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.plan-run__parallel-item {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  min-width: 0;
  align-items: center;
  gap: 9px;
  padding: 5px 2px;
  font-size: var(--fs-sm);
}

.plan-run__parallel-item--failed .plan-run__parallel-title {
  color: var(--danger);
}

/* 真实耗时 / 重试次数徽标（core runner 实测回传，无数据即整块不渲染） */
.plan-run__parallel-meta {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 4px;
}

.plan-run__parallel-chip {
  padding: 0 5px;
  border: 1px solid var(--border);
  border-radius: var(--radius-xs);
  color: var(--text-muted);
  font-size: var(--fs-xs);
  line-height: 1.6;
  white-space: nowrap;
}

.plan-run__parallel-chip--retry {
  border-color: color-mix(in srgb, var(--warn) 54%, var(--border));
  color: var(--warn);
}

.plan-run__parallel-copy {
  display: grid;
  min-width: 0;
  gap: 1px;
}

.plan-run__parallel-title {
  min-width: 0;
  overflow: hidden;
  font-weight: 500;
  line-height: 1.4;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.plan-run__parallel-summary {
  overflow: hidden;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  line-height: 1.35;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.plan-run__replan-prompt {
  margin: 0;
  overflow: hidden;
  color: var(--text);
  font-size: var(--fs-sm);
  line-height: 1.4;
  overflow-wrap: anywhere;
}

.plan-run__replan-detail {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  line-height: 1.4;
}

.plan-run__actions {
  display: flex;
  justify-content: flex-end;
  padding: 6px 10px 8px;
  border-top: 1px solid var(--border);
}

.plan-run__end {
  min-height: 30px;
  padding: 0 7px;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  font: inherit;
  font-size: var(--fs-xs);
}

.plan-run__end:hover:not(:disabled),
.plan-run__end:focus-visible {
  background: var(--bg-hover);
  color: var(--danger);
  outline: none;
}

.plan-run__end:focus-visible {
  box-shadow: var(--focus-ring);
}

.plan-run__end:disabled {
  opacity: var(--state-disabled-opacity);
  cursor: not-allowed;
}

.plan-run-popover-enter-active {
  transition:
    opacity var(--dur-base) var(--ease-out),
    transform var(--dur-base) var(--ease-out);
}

.plan-run-popover-leave-active {
  transition:
    opacity var(--dur-fast) var(--ease-in),
    transform var(--dur-fast) var(--ease-in);
}

.plan-run-popover-enter-from,
.plan-run-popover-leave-to {
  opacity: 0;
  transform: translate(-50%, 4px);
}

.plan-run-marker-enter-active,
.plan-run-marker-leave-active {
  transition:
    opacity var(--dur-fast) var(--ease-standard),
    transform var(--dur-fast) var(--ease-standard);
}

.plan-run-marker-enter-from {
  opacity: 0;
  transform: translateY(3px) scale(0.84);
}

.plan-run-marker-leave-to {
  opacity: 0;
  transform: translateY(-3px) scale(0.84);
}

.plan-run-step-enter-active,
.plan-run-step-leave-active,
.plan-run-step-move {
  transition:
    opacity var(--dur-base) var(--ease-out),
    transform var(--dur-base) var(--ease-out);
}

.plan-run-step-enter-from,
.plan-run-step-leave-to {
  opacity: 0;
  transform: translateY(6px);
}

@media (max-width: 640px) {
  .plan-run {
    width: calc(100vw - 16px);
    max-width: calc(100vw - 16px);
  }

  .plan-run__summary,
  .plan-run__static,
  .plan-run__end {
    min-height: 44px;
  }

  .plan-run__popover {
    width: calc(100vw - 16px);
  }

  .plan-run__step {
    min-height: 40px;
  }
}

@media (prefers-reduced-motion: reduce) {
  .plan-run__summary,
  .plan-run__chevron,
  .plan-run__step,
  .plan-run-popover-enter-active,
  .plan-run-popover-leave-active,
  .plan-run-marker-enter-active,
  .plan-run-marker-leave-active,
  .plan-run-step-enter-active,
  .plan-run-step-leave-active,
  .plan-run-step-move {
    transition: none;
  }
}
</style>
