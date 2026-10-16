<template>
  <div class="control-stage cu-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('computerUse.title') }}</h1>
        <p class="control-stage__subtitle">{{ t('computerUse.subtitle') }}</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" type="button" data-testid="cu-clear" @click="clearOutput">
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
          <!-- 视觉定位：描述驱动 grounding（网关 locate 动作的真实输入） -->
          <div v-if="canGround" class="cu-param-row cu-param-row--stack">
            <label class="cu-param-row__label" for="cu-description">
              {{ t('computerUse.descriptionLabel') }}
            </label>
            <input
              id="cu-description"
              v-model="description"
              class="cu-input"
              data-testid="cu-description"
              :placeholder="t('computerUse.descriptionPlaceholder')"
            />
          </div>
          <div v-if="canGround" class="cu-param-row cu-toggles">
            <label class="cu-toggle">
              <input v-model="vision" type="checkbox" data-testid="cu-vision" />
              <span>{{ t('computerUse.visionLabel') }}</span>
            </label>
            <label class="cu-toggle">
              <input v-model="autoClick" type="checkbox" data-testid="cu-auto-click" />
              <span>{{ t('computerUse.autoClickLabel') }}</span>
            </label>
            <label class="cu-toggle">
              <input v-model="patchGrounding" type="checkbox" data-testid="cu-patch-grounding" />
              <span>{{ t('computerUse.patchLabel') }}</span>
            </label>
          </div>
          <!-- 采集参数：多屏选择 + 分辨率上限（此前完全无处透出） -->
          <div v-if="needsCapture" class="cu-param-row">
            <label class="cu-param-row__label" for="cu-mode">{{ t('computerUse.modeLabel') }}</label>
            <select id="cu-mode" v-model="mode" class="cu-input" data-testid="cu-mode">
              <option v-for="m in captureModes" :key="m" :value="m">
                {{ t(`computerUse.captureMode.${m}`) }}
              </option>
            </select>
            <label class="cu-param-row__label" for="cu-max-edge">
              {{ t('computerUse.maxEdgeLabel') }}
            </label>
            <input
              id="cu-max-edge"
              v-model.number="maxEdge"
              type="number"
              min="0"
              class="cu-input"
              data-testid="cu-max-edge"
              :placeholder="t('computerUse.maxEdgePlaceholder')"
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
        <!-- grounding 结果可视化：坐标/置信度/目标描述（此前只有一行原始文本） -->
        <div v-if="grounding" class="cu-grounding" data-testid="cu-grounding">
          <div class="cu-grounding__head">
            <span class="cu-grounding__title">{{ t('computerUse.groundingTitle') }}</span>
            <span
              v-if="grounding.lowConfidence"
              class="cu-grounding__badge"
              data-testid="cu-grounding-low"
            >{{ t('computerUse.groundingLowConfidence') }}</span>
          </div>
          <div class="cu-grounding__coords" data-testid="cu-grounding-coords">
            x {{ grounding.x.toFixed(1) }} · y {{ grounding.y.toFixed(1) }}
            <span v-if="grounding.confidence !== undefined">
              · {{ t('computerUse.groundingConfidence', { value: grounding.confidence.toFixed(2) }) }}
            </span>
          </div>
          <div v-if="grounding.description" class="cu-grounding__target">
            {{ t('computerUse.groundingDescription', { description: grounding.description }) }}
          </div>
        </div>
        <p
          v-if="visionUnavailable"
          class="cu-hint"
          data-testid="cu-vision-unavailable"
        >{{ t('computerUse.visionUnavailable') }}</p>
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
import {
  buildComputerUseParams,
  CAPTURE_MODES,
  isVisionUnavailable,
  // 别名避免与下面的 computed 同名自引用（`needsCoords = computed(() => needsCoords(...))` 会解析到自身）
  needsCapture as actionNeedsCapture,
  needsCoords as actionNeedsCoords,
  needsDelta as actionNeedsDelta,
  needsKey as actionNeedsKey,
  needsText as actionNeedsText,
  parseGrounding,
  supportsGrounding,
  type CaptureMode,
  type GroundingView,
} from '@/utils/computerUse'

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
  // 视觉 grounding 闭环入口：截图 → VLMM 定位 → 可选自动点击（此前前端无此入口，
  // 网关侧已具备的能力在产品里根本走不到）
  { id: 'locate', icon: 'search' },
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

const action = ref('screenshot')
const x = ref(0)
const y = ref(0)
const dx = ref(0)
const dy = ref(0)
const text = ref('')
const key = ref('')
const description = ref('')
/** 视觉 grounding：默认开启——关闭后 `locate` 只会拿到「截图已就绪」的降级文案 */
const vision = ref(true)
/** 自动点击默认关闭：它是不可逆操作，必须由用户显式开启 */
const autoClick = ref(false)
const patchGrounding = ref(false)
const mode = ref<CaptureMode>('primary')
const maxEdge = ref(0)
const busy = ref(false)
const lastResult = ref('')
const grounding = ref<GroundingView | null>(null)
const visionUnavailable = ref(false)

const needsCoords = computed(() => actionNeedsCoords(action.value))
const needsDelta = computed(() => actionNeedsDelta(action.value))
const needsText = computed(() => actionNeedsText(action.value))
const needsKey = computed(() => actionNeedsKey(action.value))
const needsCapture = computed(() => actionNeedsCapture(action.value))
const canGround = computed(() => supportsGrounding(action.value))
const captureModes = CAPTURE_MODES

function clearOutput() {
  lastResult.value = ''
  grounding.value = null
  visionUnavailable.value = false
}

async function execute() {
  if (busy.value) return
  busy.value = true
  lastResult.value = t('computerUse.executing')
  grounding.value = null
  visionUnavailable.value = false
  try {
    const params = buildComputerUseParams({
      action: action.value,
      x: x.value,
      y: y.value,
      dx: dx.value,
      dy: dy.value,
      text: text.value,
      key: key.value,
      description: description.value,
      vision: vision.value,
      autoClick: autoClick.value,
      patchGrounding: patchGrounding.value,
      mode: mode.value,
      maxEdge: maxEdge.value,
    })
    const res = await rpc.call<{
      ok: boolean
      data?: string
      error?: string
      code?: string
      grounding?: unknown
    }>('computer_use.action', params)
    // 诚实区分三种结局：真实定位结果 / 未配视觉模型的软失败 / 其它失败
    grounding.value = parseGrounding(res)
    visionUnavailable.value = isVisionUnavailable(res)
    if (visionUnavailable.value) {
      lastResult.value = t('computerUse.visionUnavailable')
    } else {
      lastResult.value = res.ok
        ? res.data || t('computerUse.emptyResult')
        : t('computerUse.failed', { reason: res.error || t('computerUse.unknownError') })
    }
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
/* 视觉定位结果：坐标/置信度/目标（结构化呈现，不埋在原始文本里） */
.cu-grounding {
  margin-bottom: 10px;
  padding: 10px 12px;
  border: 1px solid var(--accent);
  border-radius: var(--radius-sm);
  background: var(--bg);
}
.cu-grounding__head {
  display: flex;
  align-items: center;
  gap: 8px;
}
.cu-grounding__title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text);
}
.cu-grounding__badge {
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  font-size: 10px;
  color: var(--warn);
  border: 1px solid var(--warn);
}
.cu-grounding__coords {
  margin-top: 6px;
  font-family: var(--font-mono, monospace);
  font-size: 12px;
  color: var(--text);
}
.cu-grounding__target {
  margin-top: 4px;
  font-size: 11px;
  color: var(--text-dim);
}
.cu-hint {
  margin: 0 0 10px;
  padding: 8px 10px;
  border: 1px solid var(--warn);
  border-radius: var(--radius-sm);
  font-size: 11px;
  color: var(--warn);
}
/* 参数行可纵向堆叠（描述输入较长，横排会挤压） */
.cu-param-row--stack {
  flex-direction: column;
  align-items: stretch;
  gap: 6px;
}
.cu-toggles {
  flex-wrap: wrap;
  gap: 12px;
}
.cu-toggle {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  color: var(--text-dim);
  cursor: pointer;
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
