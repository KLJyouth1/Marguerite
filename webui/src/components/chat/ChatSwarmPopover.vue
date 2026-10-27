<!-- P2·E 蜂群面板（对话输入框编排入口）：
     开关 + 并行规模 + 子代理事件共享滚动（事件订阅面实时可见）。
     关闭蜂群 = `swarmOn=false`，提交参数不变且无额外开销（single 路径走 chat.send）。 -->
<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'
import { useI18n } from 'vue-i18n'

const props = defineProps<{
  open: boolean
  swarmOn: boolean
  swarmSize: number
}>()

const emit = defineEmits<{
  'update:open': [value: boolean]
  'update:swarmOn': [value: boolean]
  'update:swarmSize': [value: number]
}>()

const rpc = useRpcStore()
const { t } = useI18n()

interface ActivityItem {
  event: string
  ts: string
}

// 子代理事件共享滚动：订阅会话事件面中与步骤/任务组/编排相关的事件，
// 仅面板打开时订阅（零常驻开销）。
const ACTIVITY_EVENTS = [
  'session.event.step_start',
  'session.event.step_finish',
  'session.event.task_group.waiting',
  'session.event.task_group.synthesizing',
  'session.event.task_group.done',
  'session.event.task_group.failed',
  'session.event.subagent_completion',
  'session.event.done',
]

const activities = ref<ActivityItem[]>([])
let unsubs: Array<() => void> = []

function readableEventName(event: string): string {
  const short = event
    .replace(/^session\.event\./, '')
    .replace(/^task_group\./, t('chat.orchestration.swarmEventTaskGroupPrefix'))
  const names: Record<string, string> = {
    step_start: t('chat.orchestration.eventStepStart'),
    step_finish: t('chat.orchestration.eventStepFinish'),
    task_group_waiting: t('chat.orchestration.eventTaskGroupWaiting'),
    task_group_synthesizing: t('chat.orchestration.eventTaskGroupSynthesizing'),
    task_group_done: t('chat.orchestration.eventTaskGroupDone'),
    task_group_failed: t('chat.orchestration.eventTaskGroupFailed'),
    subagent_completion: t('chat.orchestration.eventSubagentCompletion'),
    done: t('chat.orchestration.eventTurnDone'),
  }
  return names[short] || short
}

function pushActivity(event: string) {
  activities.value = [
    { event: readableEventName(event), ts: new Date().toLocaleTimeString() },
    ...activities.value,
  ].slice(0, 8)
}

function subscribe() {
  if (unsubs.length > 0) return
  for (const evt of ACTIVITY_EVENTS) {
    unsubs.push(rpc.on(evt, () => pushActivity(evt)))
  }
  // 兜底通配：捕获编排层 agent.* 事件（orchestrate.run 发布 agent.started/stopped）。
  unsubs.push(
    rpc.on('*', (rawEvent: string) => {
      if (rawEvent.includes('agent.') || rawEvent.includes('orchestrate')) {
        pushActivity(rawEvent)
      }
    }),
  )
}

function unsubscribe() {
  for (const off of unsubs) off()
  unsubs = []
}

watch(() => props.open, (v) => {
  if (v) subscribe()
  else unsubscribe()
}, { immediate: true })

onBeforeUnmount(unsubscribe)
</script>

<template>
<div v-if="open" class="swarm-popover" role="dialog" :aria-label="t('chat.orchestration.swarmTitle')">
    <div class="swarm-popover__head">
      <Icon name="swarm" :size="14" />
      <span>{{ t('chat.orchestration.swarmTitle') }}</span>
    </div>
    <div class="swarm-popover__body">
      <div class="swarm-popover__row">
        <span class="swarm-popover__label">{{ t('chat.orchestration.swarmToggleLabel') }}</span>
        <button
          type="button"
          class="swarm-popover__toggle"
          :class="{ 'is-on': swarmOn }"
          role="switch"
          :aria-checked="swarmOn ? 'true' : 'false'"
          :aria-label="swarmOn ? t('chat.orchestration.swarmToggleOn') : t('chat.orchestration.swarmToggleOff')"
          @click="emit('update:swarmOn', !swarmOn)"
        >
          <span class="swarm-popover__knob" />
        </button>
      </div>
      <div class="swarm-popover__row">
        <span class="swarm-popover__label">{{ t('chat.orchestration.swarmSizeTitle') }}</span>
        <div class="swarm-popover__sizes" role="group" :aria-label="t('chat.orchestration.swarmSizeTitle')">
          <button
            v-for="n in [2, 3, 4, 6, 8]"
            :key="n"
            type="button"
            class="swarm-popover__size"
            :class="{ 'is-active': swarmSize === n }"
            :aria-pressed="swarmSize === n ? 'true' : 'false'"
            @click="emit('update:swarmSize', n)"
          >
            {{ n }}
          </button>
        </div>
      </div>
      <p v-if="swarmOn" class="swarm-popover__hint">
        <Icon name="zap" :size="12" />
        {{ t('chat.orchestration.swarmParallelHint', { n: swarmSize }) }}
      </p>
      <p v-else class="swarm-popover__hint">
        {{ t('chat.orchestration.swarmSingleHint') }}
      </p>
      <div v-if="swarmOn && activities.length" class="swarm-popover__feed" aria-live="polite">
        <div v-for="(a, i) in activities" :key="`${a.ts}-${i}`" class="swarm-popover__feed-item">
          <span class="swarm-popover__feed-dot" aria-hidden="true" />
          <span class="swarm-popover__feed-event">{{ a.event }}</span>
          <span class="swarm-popover__feed-ts">{{ a.ts }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.swarm-popover {
  position: absolute;
  right: 0;
  bottom: calc(100% + 6px);
  width: min(320px, calc(100vw - 32px));
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--bg-elevated, var(--bg-surface));
  box-shadow: var(--shadow-xl);
  z-index: 40;
  overflow: hidden;
}
.swarm-popover__head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border);
  color: var(--text-dim);
  font-size: var(--fs-xs);
  font-weight: 700;
  letter-spacing: 0.02em;
}
.swarm-popover__body {
  padding: 10px 12px 12px;
}
.swarm-popover__row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 5px 0;
}
.swarm-popover__label {
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.swarm-popover__toggle {
  position: relative;
  width: 38px;
  height: 22px;
  padding: 0;
  border: 0;
  border-radius: var(--radius-pill);
  background: var(--bg-hover);
  cursor: pointer;
  transition: background var(--dur-fast) var(--ease-standard);
}
.swarm-popover__toggle.is-on {
  background: var(--accent);
}
.swarm-popover__knob {
  position: absolute;
  top: 3px;
  left: 3px;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: var(--bg-surface);
  transition: transform var(--dur-fast) var(--ease-standard);
}
.swarm-popover__toggle.is-on .swarm-popover__knob {
  transform: translateX(16px);
}
.swarm-popover__sizes {
  display: flex;
  gap: 4px;
}
.swarm-popover__size {
  min-width: 30px;
  height: 26px;
  padding: 0 6px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  font-size: var(--fs-xs);
  cursor: pointer;
}
.swarm-popover__size.is-active {
  border-color: var(--accent);
  background: var(--bg-hover);
  color: var(--text);
}
.swarm-popover__hint {
  display: flex;
  align-items: flex-start;
  gap: 5px;
  margin: 8px 0 0;
  color: var(--text-dim);
  font-size: var(--fs-xs);
  line-height: 1.5;
}
.swarm-popover__hint code {
  font-family: var(--font-mono, ui-monospace, monospace);
}
.swarm-popover__feed {
  margin-top: 10px;
  border-top: 1px solid var(--border);
  padding-top: 8px;
  max-height: 220px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.swarm-popover__feed-item {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: var(--fs-xs);
  color: var(--text-muted);
}
.swarm-popover__feed-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent);
  flex-shrink: 0;
}
.swarm-popover__feed-event {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.swarm-popover__feed-ts {
  margin-left: auto;
  color: var(--text-dim);
  flex-shrink: 0;
}
</style>