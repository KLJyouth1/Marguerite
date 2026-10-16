<template>
  <section class="goal-panel" role="region" aria-labelledby="goal-panel-title" :aria-busy="Boolean(loading || actionBusy)">
    <header class="goal-panel__header">
      <div>
        <h2 id="goal-panel-title">{{ title }}</h2>
        <p v-if="goal" class="goal-panel__status">{{ statusLabel }}</p>
      </div>
      <button type="button" class="goal-panel__refresh" :disabled="loading || !connected" :aria-label="refreshLabel" @click="loadGoal">
        <Icon name="refresh" :size="15" aria-hidden="true" />
      </button>
    </header>
    <div v-if="error" class="goal-panel__error" role="alert">{{ error }}</div>
    <div v-if="loading" class="goal-panel__loading" role="status">{{ loadingLabel }}</div>
    <div v-else-if="!goal" class="goal-panel__empty" role="status">{{ emptyLabel }}</div>
    <template v-else>
      <div class="goal-panel__summary">
        <strong>{{ goal.objective }}</strong>
        <span class="goal-panel__phase">{{ statusLabel }}</span>
      </div>
      <dl class="goal-panel__stats">
        <div><dt>{{ roundsLabel }}</dt><dd>{{ goal.roundsStarted }}/{{ goal.maxGoalRounds ?? '—' }}</dd></div>
        <div><dt>{{ tokensLabel }}</dt><dd>{{ tokenCount }}</dd></div>
        <div><dt>{{ stepsLabel }}</dt><dd>{{ completedSteps }}/{{ totalSteps }}</dd></div>
      </dl>
      <section v-if="steps.length" class="goal-panel__steps" :aria-label="stepsLabel">
        <h3>{{ stepsLabel }}</h3>
        <ol>
          <li v-for="(step, index) in steps" :key="step.stepId" :class="{ 'is-current': step.stepId === currentStepId, 'is-complete': step.status === 'completed' }" :aria-current="step.stepId === currentStepId ? 'step' : undefined">
            <span class="goal-panel__step-marker" aria-hidden="true"><Icon :name="step.status === 'completed' ? 'check' : step.stepId === currentStepId ? 'play' : 'listTree'" :size="13" /></span>
            <span>{{ index + 1 }}. {{ step.title }}</span>
          </li>
        </ol>
      </section>
      <div class="goal-panel__actions" role="group" :aria-label="actionsLabel">
        <button v-if="goal.phase === 'active'" type="button" :disabled="Boolean(actionBusy || terminal)" @click="perform('goal.pause')">{{ actionBusy === 'goal.pause' ? loadingLabel : pauseLabel }}</button>
        <button v-if="goal.phase === 'paused' || goal.phase === 'blocked'" type="button" :disabled="Boolean(actionBusy || terminal)" @click="perform('goal.resume')">{{ actionBusy === 'goal.resume' ? loadingLabel : resumeLabel }}</button>
        <button type="button" :disabled="Boolean(actionBusy || terminal)" @click="perform('goal.complete')">{{ actionBusy === 'goal.complete' ? loadingLabel : completeLabel }}</button>
      </div>
    </template>
  </section>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useRpcStore } from '@/stores/rpc'
import Icon from '@/components/Icon.vue'
import { t } from '@/i18n'
import { goalIdFromWorkbenchItem } from '@/workbench/goalItems'
import type { WorkbenchComponentEvent, WorkbenchItem } from '@/workbench/types'

interface GoalSnapshot {
  id: string; objective: string; phase: 'active' | 'paused' | 'blocked' | 'completed'; roundsStarted: number; maxGoalRounds?: number; revision?: number; terminal?: boolean; tokenCount?: number; tokens?: number; steps?: Array<{ stepId: string; title: string; status?: string }>; currentStepId?: string
}
const props = defineProps<{ item: WorkbenchItem }>()
const emit = defineEmits<{ 'workbench-event': [event: WorkbenchComponentEvent] }>()
const rpc = useRpcStore()
const goal = ref<GoalSnapshot | null>(null)
const loading = ref(false)
const error = ref('')
const actionBusy = ref<string | false>(false)
const connected = computed(() => rpc.isConnected)
const title = computed(() => String(t('workbench.goal.title')))
const emptyLabel = computed(() => String(t('workbench.goal.empty')))
const loadingLabel = computed(() => String(t('workbench.goal.loading')))
const statusLabel = computed(() => goal.value ? String(t(`workbench.goal.status.${goal.value.phase}`)) : '')
const pauseLabel = computed(() => String(t('workbench.goal.pause')))
const resumeLabel = computed(() => String(t('workbench.goal.resume')))
const completeLabel = computed(() => String(t('workbench.goal.complete')))
const refreshLabel = computed(() => String(t('workbench.goal.refresh')))
const roundsLabel = computed(() => String(t('workbench.goal.rounds')))
const tokensLabel = computed(() => String(t('workbench.goal.tokens')))
const stepsLabel = computed(() => String(t('workbench.goal.steps')))
const actionsLabel = computed(() => String(t('workbench.goal.actions')))
const terminal = computed(() => goal.value?.terminal === true || goal.value?.phase === 'completed')
const steps = computed(() => goal.value?.steps ?? [])
const currentStepId = computed(() => goal.value?.currentStepId || steps.value.find(step => step.status === 'in_progress')?.stepId)
const completedSteps = computed(() => steps.value.filter(step => step.status === 'completed').length)
const totalSteps = computed(() => steps.value.length)
const tokenCount = computed(() => goal.value?.tokenCount ?? goal.value?.tokens ?? 0)

async function loadGoal() {
  if (!connected.value) return
  loading.value = true; error.value = ''
  try {
    goal.value = await rpc.call<GoalSnapshot>('goal.get', { goalId: goalIdFromWorkbenchItem(props.item) })
  } catch (cause) {
    const code = (cause as { code?: string })?.code
    if (code === 'GOAL_NOT_FOUND') goal.value = null
    else error.value = cause instanceof Error ? cause.message : String(cause)
  } finally { loading.value = false }
}
async function perform(method: string) {
  if (!goal.value || terminal.value || actionBusy.value) return
  actionBusy.value = method; error.value = ''
  try {
    goal.value = await rpc.call<GoalSnapshot>(method, { goalId: goal.value.id, expectedRevision: goal.value.revision })
    emit('workbench-event', { type: 'goal-action', payload: { method } })
  } catch (cause) { error.value = cause instanceof Error ? cause.message : String(cause) }
  finally { actionBusy.value = false }
}
const eventUnsubs: Array<() => void> = []
function onGoalEvent(payload: unknown) {
  if (!payload || typeof payload !== 'object') return
  const value = (payload as { goal?: GoalSnapshot }).goal || payload as GoalSnapshot
  if (value && typeof value.id === 'string' && value.id === goalIdFromWorkbenchItem(props.item)) goal.value = value
}
onMounted(() => {
  void loadGoal()
  for (const event of ['goal.updated', 'goal.created', 'session.event.plan_revision', 'session.event.plan_run']) {
    eventUnsubs.push(rpc.on(event, onGoalEvent))
  }
})
onUnmounted(() => { for (const unsubscribe of eventUnsubs) unsubscribe() })
</script>

<style scoped>
.goal-panel { display: grid; gap: var(--sp-4); height: 100%; overflow: auto; padding: var(--sp-4); color: var(--text); background: var(--bg-surface); }
.goal-panel__header, .goal-panel__summary, .goal-panel__actions, .goal-panel__stats div { display: flex; align-items: center; gap: var(--sp-2); }
.goal-panel__header { justify-content: space-between; border-bottom: 1px solid var(--border); padding-bottom: var(--sp-3); }
.goal-panel h2, .goal-panel h3, .goal-panel p { margin: 0; }
.goal-panel h2 { font-size: var(--fs-lg); letter-spacing: -0.01em; }
.goal-panel h3 { font-size: var(--fs-sm); }
.goal-panel__status, .goal-panel__phase { color: var(--text-muted); font-size: var(--fs-xs); }
.goal-panel__refresh, .goal-panel__actions button { min-width: 44px; min-height: 44px; border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--bg-elevated); color: inherit; cursor: pointer; }
.goal-panel__actions { flex-wrap: wrap; }
.goal-panel__actions button { padding: 0 var(--sp-3); }
.goal-panel button:hover:not(:disabled) { background: var(--bg-hover); }
.goal-panel button:focus-visible { outline: none; box-shadow: var(--focus-ring); }
.goal-panel button:disabled { opacity: var(--state-disabled-opacity); cursor: not-allowed; }
.goal-panel__stats { display: grid; grid-template-columns: repeat(3, 1fr); gap: var(--sp-2); margin: 0; }
.goal-panel__stats div { display: grid; gap: 2px; padding: var(--sp-2); border-bottom: 1px solid var(--border); }
.goal-panel dt { color: var(--text-muted); font-size: var(--fs-xs); }
.goal-panel dd { margin: 0; font-variant-numeric: tabular-nums; }
.goal-panel__steps ol { display: grid; gap: var(--sp-1); margin: var(--sp-2) 0 0; padding: 0; list-style: none; }
.goal-panel__steps li { display: flex; align-items: center; gap: var(--sp-2); min-height: 36px; color: var(--text-muted); }
.goal-panel__steps li.is-current { color: var(--text); font-weight: 600; }
.goal-panel__steps li.is-complete { color: var(--ok); }
.goal-panel__step-marker { display: inline-grid; width: 24px; height: 24px; place-items: center; }
.goal-panel__error { padding: var(--sp-2); border: 1px solid var(--danger); color: var(--danger); font-size: var(--fs-sm); }
.goal-panel__empty, .goal-panel__loading { color: var(--text-muted); font-size: var(--fs-sm); }
@media (max-width: 640px) { .goal-panel { padding: var(--sp-3); } .goal-panel__stats { grid-template-columns: 1fr; } }
@media (prefers-reduced-motion: reduce) { .goal-panel * { transition: none !important; animation: none !important; } }
</style>
