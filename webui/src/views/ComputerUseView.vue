<template>
  <div class="control-stage cu-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('computerUse.title') }}</h1>
        <p class="control-stage__subtitle">{{ t('computerUse.subtitle') }}</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" type="button" data-testid="cu-clear" @click="lastResult = ''">
          <Icon name="trash" :size="16" /><span>{{ t('computerUse.clearOutput') }}</span>
        </button>
      </div>
    </header>

    <section class="cu-grid">
      <div class="cu-panel">
        <h2 :id="actionGroupLabelId" class="cu-panel__title">{{ t('computerUse.actionGroup') }}</h2>
        <div class="cu-actions" role="radiogroup" :aria-labelledby="actionGroupLabelId">
          <button
            v-for="a in actions"
            :key="a.id"
            class="cu-btn"
            :class="{ 'is-active': action === a.id }"
            type="button"
            role="radio"
            :aria-checked="action === a.id"
            :data-testid="`cu-action-${a.id}`"
            @click="action = a.id"
          >
            <Icon class="cu-btn__icon" :name="a.icon" :size="18" />
            <span>{{ t(`computerUse.actions.${a.id}`) }}</span>
          </button>
        </div>
      </div>

      <div class="cu-panel">
        <h2 class="cu-panel__title">{{ t('computerUse.paramsGroup') }}</h2>
        <div class="cu-params">
          <div v-if="needsCoords" class="cu-param-row">
            <label class="cu-param-row__label" for="cu-x">{{ t('computerUse.coordX') }}</label>
            <input id="cu-x" v-model.number="x" type="number" class="cu-input" data-testid="cu-x" />
            <label class="cu-param-row__label" for="cu-y">{{ t('computerUse.coordY') }}</label>
            <input id="cu-y" v-model.number="y" type="number" class="cu-input" data-testid="cu-y" />
          </div>
          <div v-if="needsDelta" class="cu-param-row">
            <label class="cu-param-row__label" for="cu-dx">{{ t('computerUse.scrollDx') }}</label>
            <input id="cu-dx" v-model.number="dx" type="number" class="cu-input" data-testid="cu-dx" />
            <label class="cu-param-row__label" for="cu-dy">{{ t('computerUse.scrollDy') }}</label>
            <input id="cu-dy" v-model.number="dy" type="number" class="cu-input" data-testid="cu-dy" />
          </div>
          <div v-if="needsText" class="cu-param-row">
            <label class="cu-param-row__label" for="cu-text">{{ t('computerUse.textLabel') }}</label>
            <input
              id="cu-text"
              v-model="text"
              class="cu-input"
              data-testid="cu-text"
              :placeholder="t('computerUse.textPlaceholder')"
            />
          </div>
          <div v-if="needsKey" class="cu-param-row">
            <label class="cu-param-row__label" for="cu-key">{{ t('computerUse.keyLabel') }}</label>
            <input
              id="cu-key"
              v-model="key"
              class="cu-input"
              data-testid="cu-key"
              :placeholder="t('computerUse.keyPlaceholder')"
            />
          </div>
          <button
            class="btn btn--primary cu-execute"
            type="button"
            data-testid="cu-execute"
            :disabled="busy"
            :aria-busy="busy"
            @click="execute"
          >
            <Icon :name="busy ? 'pause' : 'play'" :size="16" />
            <span>{{ busy ? t('computerUse.executing') : t('computerUse.execute') }}</span>
          </button>
        </div>
      </div>

      <div class="cu-panel cu-panel--output">
        <h2 class="cu-panel__title">{{ t('computerUse.outputGroup') }}</h2>
        <pre class="cu-output" data-testid="cu-output" aria-live="polite">{{
          lastResult || t('computerUse.outputEmpty')
        }}</pre>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import Icon from '@/components/Icon.vue'
import type { IconName } from '@/utils/icons'
import { useRpcStore } from '@/stores/rpc'

const { t } = useI18n()
const rpc = useRpcStore()

const actionGroupLabelId = 'cu-action-group-label'

/**
 * Action ids mirror the `computer_use.action` contract on the gateway; labels
 * come from i18n so the grid is translated, and icons come from the project
 * icon registry (no glyph/emoji icons).
 */
interface ComputerUseAction {
  id: string
  icon: IconName
}

const actions: readonly ComputerUseAction[] = [
  { id: 'screenshot', icon: 'camera' },
  { id: 'perceive', icon: 'eye' },
  { id: 'click', icon: 'mouse-left' },
  { id: 'double_click', icon: 'mouse-double' },
  { id: 'right_click', icon: 'mouse-right' },
  { id: 'middle_click', icon: 'mouse-middle' },
  { id: 'move', icon: 'move' },
  { id: 'drag', icon: 'drag' },
  { id: 'scroll', icon: 'scroll-vertical' },
  { id: 'type', icon: 'keyboard' },
  { id: 'key', icon: 'key' },
  { id: 'key_combo', icon: 'command' },
  { id: 'clipboard_set', icon: 'clipboard-write' },
  { id: 'clipboard_get', icon: 'clipboard-read' },
  { id: 'window_focus', icon: 'window' },
  { id: 'cursor_pos', icon: 'crosshair' },
  { id: 'key_state', icon: 'gauge' },
  { id: 'idle_time', icon: 'timer' },
]

const COORD_ACTIONS = new Set([
  'click',
  'double_click',
  'right_click',
  'middle_click',
  'move',
  'drag',
  'scroll',
])
const DELTA_ACTIONS = new Set(['scroll', 'drag'])
const TEXT_ACTIONS = new Set(['type', 'clipboard_set'])
const KEY_ACTIONS = new Set(['key', 'key_combo', 'window_focus', 'key_state'])

const action = ref('screenshot')
const x = ref(0)
const y = ref(0)
const dx = ref(0)
const dy = ref(0)
const text = ref('')
const key = ref('')
const busy = ref(false)
const lastResult = ref('')

const needsCoords = computed(() => COORD_ACTIONS.has(action.value))
const needsDelta = computed(() => DELTA_ACTIONS.has(action.value))
const needsText = computed(() => TEXT_ACTIONS.has(action.value))
const needsKey = computed(() => KEY_ACTIONS.has(action.value))

async function execute() {
  if (busy.value) return
  busy.value = true
  lastResult.value = t('computerUse.executing')
  try {
    const params: Record<string, unknown> = { action: action.value }
    if (needsCoords.value) {
      params.x = x.value
      params.y = y.value
    }
    // `drag` carries a destination point; `scroll` carries a wheel delta. Both
    // reuse the same two inputs, so the payload key differs by action.
    if (action.value === 'drag') {
      params.x2 = dx.value
      params.y2 = dy.value
    } else if (action.value === 'scroll') {
      params.dx = dx.value
      params.dy = dy.value
    }
    if (needsText.value) params.text = text.value
    if (needsKey.value) params.key = key.value

    const res = await rpc.call<{ ok: boolean; data?: string; error?: string }>(
      'computer_use.action',
      params,
    )
    lastResult.value = res.ok
      ? res.data || t('computerUse.emptyResult')
      : t('computerUse.failed', { reason: res.error || t('computerUse.unknownError') })
  } catch (e) {
    lastResult.value = t('computerUse.failed', {
      reason: e instanceof Error ? e.message : String(e),
    })
  } finally {
    busy.value = false
  }
}
</script>

<style scoped>
.cu-stage {
  padding: 24px 28px;
  max-width: 1200px;
  margin: 0 auto;
}
.cu-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 16px;
  margin-top: 20px;
}
.cu-panel {
  background: var(--card);
  border: 1px solid var(--hairline);
  border-radius: var(--radius-md);
  padding: 16px;
}
.cu-panel--output {
  grid-column: 1 / -1;
}
.cu-panel__title {
  font-size: 13px;
  font-weight: 600;
  margin: 0 0 12px;
  color: var(--text-dim);
}
.cu-actions {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(100px, 1fr));
  gap: 6px;
}
.cu-btn {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 5px;
  padding: 10px 6px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--hairline);
  background: transparent;
  color: var(--text-dim);
  font-size: 11px;
  line-height: 1.3;
  text-align: center;
  cursor: pointer;
  transition:
    border-color var(--dur-fast),
    color var(--dur-fast),
    background-color var(--dur-fast);
}
.cu-btn:hover {
  border-color: var(--accent);
  color: var(--text);
}
.cu-btn:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}
.cu-btn.is-active {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--accent-foreground);
}
.cu-btn__icon {
  opacity: 0.85;
}
.cu-btn.is-active .cu-btn__icon {
  opacity: 1;
}
.cu-params {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.cu-param-row {
  display: flex;
  align-items: center;
  gap: 8px;
}
.cu-param-row__label {
  font-size: 11px;
  color: var(--text-dim);
  min-width: 70px;
}
.cu-input {
  flex: 1;
  min-width: 0;
  padding: 7px 10px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--hairline);
  background: var(--bg);
  color: var(--text);
  font-size: 12px;
  outline: none;
  transition: border-color var(--dur-fast);
}
.cu-input:focus {
  border-color: var(--accent);
}
.cu-execute {
  margin-top: 8px;
  justify-content: center;
}
.cu-output {
  margin: 0;
  padding: 12px;
  background: var(--bg);
  border-radius: var(--radius-sm);
  font-size: 11px;
  font-family: var(--font-mono, monospace);
  color: var(--text);
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 300px;
  overflow-y: auto;
}

@media (max-width: 860px) {
  .cu-grid {
    grid-template-columns: 1fr;
  }
}

@media (prefers-reduced-motion: reduce) {
  .cu-btn,
  .cu-input {
    transition: none;
  }
}
</style>
