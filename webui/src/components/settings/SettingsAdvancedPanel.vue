<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import ControlSwitch from '@/components/ControlSwitch.vue'
import MemoryLearningGroup from '@/components/settings/MemoryLearningGroup.vue'
import { usePlatform } from '@/platform'

const { t } = useI18n()
const emit = defineEmits<{
  'open-agent-configuration': []
  'open-data-maintenance': []
}>()

// Desktop-only Computer Use & FPS 助手：Web 下没有 system_control 工具
const isDesktop = usePlatform().capabilities.isDesktop

// Client-only "Labs" preferences. Each row reads/writes ONE localStorage key
// directly. The chat composables that consume these read the value once at
// construction and live behind an architecture import-fence, so writing the raw
// key here is the safe, decoupled way to surface them (no chat imports). Applies
// instantly; the reload-gated ones are labelled. Never enters the dirty bar.

// --- boolean '1'/'0' flags (absent => off) ---
const APPROVAL_KEY = 'marguerite.chat.approvalPoll'
const RUNTRACE_KEY = 'marguerite.logs.runTrace'
const BAOTA_KEY = 'marguerite.labs.baota'
const DESKTOP_CONTROL_KEY = 'marguerite.labs.desktopControl'
const FPS_ASSISTANT_KEY = 'marguerite.labs.fpsAssistant'

function readBool(key: string): boolean {
  try { return localStorage.getItem(key) === '1' } catch { return false }
}
function writeBool(key: string, on: boolean) {
  try { localStorage.setItem(key, on ? '1' : '0') } catch { /* private mode */ }
}

const approvalPoll = ref(readBool(APPROVAL_KEY))
const runTrace = ref(readBool(RUNTRACE_KEY))
const baota = ref(readBool(BAOTA_KEY))
const desktopControl = ref(readBool(DESKTOP_CONTROL_KEY))
const fpsAssistant = ref(readBool(FPS_ASSISTANT_KEY))
function setApprovalPoll(on: boolean) { approvalPoll.value = on; writeBool(APPROVAL_KEY, on) }
function setRunTrace(on: boolean) { runTrace.value = on; writeBool(RUNTRACE_KEY, on) }
function setBaota(on: boolean) { baota.value = on; writeBool(BAOTA_KEY, on) }
function setDesktopControl(on: boolean) {
  desktopControl.value = on
  writeBool(DESKTOP_CONTROL_KEY, on)
  // Computer Use 关闭时，依赖它的 FPS 助手自动跟随关闭，避免悬空
  if (!on && fpsAssistant.value) {
    fpsAssistant.value = false
    writeBool(FPS_ASSISTANT_KEY, false)
  }
}
function setFpsAssistant(on: boolean) {
  if (on && !desktopControl.value) {
    // 需先开启 Computer Use；连带打开 Computer Use 并标记联动语义
    desktopControl.value = true
    writeBool(DESKTOP_CONTROL_KEY, true)
  }
  fpsAssistant.value = on
  writeBool(FPS_ASSISTANT_KEY, on)
}

// --- foldLiveTurn: default ON; '0' is the only OFF value ---
const FOLD_KEY = 'marguerite.chat.foldLiveTurn'
const foldOn = ref(localStorageGet(FOLD_KEY) !== '0')
function setFold(on: boolean) {
  foldOn.value = on
  try { localStorage.setItem(FOLD_KEY, on ? '1' : '0') } catch { /* private mode */ }
}

// --- answerReveal: "min,max" milliseconds, min >= 0 and max >= min ---
const REVEAL_KEY = 'marguerite.chat.answerReveal'
const REVEAL_DEFAULT: [number, number] = [1800, 4000]
function readReveal(): [number, number] {
  const raw = localStorageGet(REVEAL_KEY)
  if (raw) {
    const parts = raw.split(',').map(Number)
    if (parts.length === 2 && parts.every(Number.isFinite) && parts[0] >= 0 && parts[1] >= parts[0]) {
      return [parts[0], parts[1]]
    }
  }
  return [...REVEAL_DEFAULT]
}
const initialReveal = readReveal()
const revealMin = ref(initialReveal[0])
const revealMax = ref(initialReveal[1])
const revealValid = computed(() =>
  Number.isFinite(revealMin.value) && Number.isFinite(revealMax.value) &&
  revealMin.value >= 0 && revealMax.value >= revealMin.value,
)
function commitReveal() {
  if (!revealValid.value) return
  try {
    localStorage.setItem(REVEAL_KEY, `${Math.round(revealMin.value)},${Math.round(revealMax.value)}`)
  } catch { /* private mode */ }
}

function localStorageGet(key: string): string | null {
  try { return localStorage.getItem(key) } catch { return null }
}

const agentConfigAriaLabel = computed(() =>
  `${t('setup.advanced.agentConfigAction')}: ${t('setup.advanced.agentConfigLabel')}`,
)
</script>

<template>
  <section class="control-section">
    <div class="control-section__head">
      <h3 class="control-section__title">{{ t('setup.advanced.title') }}</h3>
      <p class="control-section__desc">{{ t('setup.advanced.desc') }} <em>{{ t('setup.advanced.reload') }}</em> {{ t('setup.advanced.descReloadSuffix') }}</p>
    </div>

    <label class="control-row">
      <div class="control-row__label-block">
        <span class="control-row__label">{{ t('setup.advanced.foldLabel') }}</span>
        <span class="control-row__desc">{{ t('setup.advanced.foldDesc') }}</span>
      </div>
      <div class="control-row__control">
        <span class="labs-hint">{{ t('setup.advanced.reload') }}</span>
        <ControlSwitch name="labs_fold_live_turn" :checked="foldOn" :aria-label="t('setup.advanced.foldAria')" @change="setFold" />
      </div>
    </label>

    <div class="control-row control-row--stack">
      <div class="control-row__label-block">
        <span id="labs-reveal-label" class="control-row__label">{{ t('setup.advanced.revealLabel') }}</span>
        <span class="control-row__desc">{{ t('setup.advanced.revealDesc') }}</span>
      </div>
      <div class="control-row__control labs-range" role="group" aria-labelledby="labs-reveal-label">
        <input
          class="control-input control-input--narrow"
          name="labs_reveal_min"
          type="number" min="0" step="100" inputmode="numeric"
          v-model.number="revealMin"
          :aria-label="t('setup.advanced.revealMinAria')"
          :aria-invalid="!revealValid ? 'true' : 'false'"
          aria-describedby="labs-reveal-error"
          @change="commitReveal"
        >
        <span class="labs-range__sep" aria-hidden="true">&ndash;</span>
        <input
          class="control-input control-input--narrow"
          name="labs_reveal_max"
          type="number" min="0" step="100" inputmode="numeric"
          v-model.number="revealMax"
          :aria-label="t('setup.advanced.revealMaxAria')"
          :aria-invalid="!revealValid ? 'true' : 'false'"
          aria-describedby="labs-reveal-error"
          @change="commitReveal"
        >
        <span v-if="!revealValid" id="labs-reveal-error" class="labs-invalid" role="alert">{{ t('setup.advanced.revealInvalid') }}</span>
      </div>
    </div>

    <label class="control-row">
      <div class="control-row__label-block">
        <span class="control-row__label">{{ t('setup.advanced.approvalPollLabel') }}</span>
        <span class="control-row__desc">{{ t('setup.advanced.approvalPollDesc') }}</span>
      </div>
      <div class="control-row__control">
        <ControlSwitch name="labs_approval_poll" :checked="approvalPoll" :aria-label="t('setup.advanced.approvalPollLabel')" @change="setApprovalPoll" />
      </div>
    </label>

    <MemoryLearningGroup />

    <label class="control-row">
      <div class="control-row__label-block">
        <span class="control-row__label">{{ t('setup.advanced.runTraceLabel') }}</span>
        <span class="control-row__desc">{{ t('setup.advanced.runTraceDesc') }}</span>
      </div>
      <div class="control-row__control">
        <span class="labs-hint">{{ t('setup.advanced.reload') }}</span>
        <ControlSwitch name="labs_run_trace" :checked="runTrace" :aria-label="t('setup.advanced.runTraceAria')" @change="setRunTrace" />
      </div>
    </label>

    <div class="control-section__subhead">
      <h4 class="control-section__subhead-title">{{ t('setup.advanced.experimental') }}</h4>
      <span class="control-section__subhead-hint">仅限明确了解风险的高级用户。</span>
    </div>

    <label class="control-row control-row--experimental">
      <div class="control-row__label-block">
        <span class="control-row__label">{{ t('setup.advanced.baotaLabel') }}</span>
        <span class="control-row__desc">{{ t('setup.advanced.baotaDesc') }}</span>
      </div>
      <div class="control-row__control">
        <span class="labs-hint">{{ t('setup.advanced.reload') }}</span>
        <ControlSwitch name="labs_baota" :checked="baota" :aria-label="t('setup.advanced.baotaAria')" @change="setBaota" />
      </div>
    </label>

    <label v-if="isDesktop" class="control-row control-row--experimental">
      <div class="control-row__label-block">
        <span class="control-row__label">{{ t('setup.advanced.desktopControlLabel') }}</span>
        <span class="control-row__desc">{{ t('setup.advanced.desktopControlDesc') }}</span>
      </div>
      <div class="control-row__control">
        <span class="labs-hint">{{ t('setup.advanced.reload') }}</span>
        <ControlSwitch name="labs_desktop_control" :checked="desktopControl" :aria-label="t('setup.advanced.desktopControlAria')" @change="setDesktopControl" />
      </div>
    </label>

    <label v-if="isDesktop" class="control-row control-row--experimental">
      <div class="control-row__label-block">
        <span class="control-row__label">{{ t('setup.advanced.fpsAssistantLabel') }}</span>
        <span class="control-row__desc">{{ t('setup.advanced.fpsAssistantDesc') }}</span>
      </div>
      <div class="control-row__control">
        <span class="labs-hint">{{ t('setup.advanced.reload') }}</span>
        <ControlSwitch name="labs_fps_assistant" :checked="fpsAssistant" :aria-label="t('setup.advanced.fpsAssistantAria')" :disabled="false" @change="setFpsAssistant" />
      </div>
    </label>

    <div class="control-row">
      <div class="control-row__label-block">
        <span class="control-row__label">{{ t('setup.advanced.agentConfigLabel') }}</span>
        <span class="control-row__desc">{{ t('setup.advanced.agentConfigDesc') }}</span>
      </div>
      <div class="control-row__control">
        <button
          type="button"
          class="btn btn--ghost"
          :aria-label="agentConfigAriaLabel"
          @click="emit('open-agent-configuration')"
        >
          {{ t('setup.advanced.agentConfigAction') }}
        </button>
      </div>
    </div>

    <div class="control-row advanced-maintenance" data-testid="advanced-data-maintenance">
      <div class="control-row__label-block">
        <span class="control-row__label">{{ t('setup.advanced.dataMaintenanceLabel') }}</span>
        <span class="control-row__desc">{{ t('setup.advanced.dataMaintenanceDesc') }}</span>
      </div>
      <div class="control-row__control">
        <button
          type="button"
          class="btn btn--ghost"
          :aria-label="t('setup.advanced.dataMaintenanceAria')"
          @click="emit('open-data-maintenance')"
        >
          {{ t('setup.advanced.dataMaintenanceAction') }}
        </button>
      </div>
    </div>
  </section>
</template>

<style scoped>
.labs-hint {
  border: 1px solid color-mix(in srgb, var(--warn) 35%, var(--border));
  border-radius: var(--radius-full);
  color: var(--warn);
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.03em;
  padding: 1px 7px;
  text-transform: uppercase;
}

.labs-range {
  align-items: center;
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
}

.labs-range__sep {
  color: var(--text-dim);
}

.labs-invalid {
  color: var(--danger);
  font-size: var(--fs-xs);
  width: 100%;
}

.advanced-maintenance {
  margin-top: var(--sp-3);
  opacity: 0.82;
}

.advanced-maintenance:focus-within,
.advanced-maintenance:hover {
  opacity: 1;
}

.control-section__subhead {
  align-items: baseline;
  display: flex;
  gap: var(--sp-2);
  justify-content: space-between;
  margin: var(--sp-3) var(--sp-3) var(--sp-2) var(--sp-1);
}
.control-section__subhead-title {
  font-size: var(--fs-sm);
  font-weight: 700;
  letter-spacing: 0.02em;
  margin: 0;
  text-transform: uppercase;
}
.control-section__subhead-hint {
  color: var(--text-dim);
  font-size: var(--fs-xs);
}
.control-row--experimental .control-row__label-block::before {
  background: linear-gradient(135deg, var(--accent), color-mix(in srgb, var(--accent) 55%, transparent));
  border-radius: 999px;
  content: '';
  display: inline-block;
  height: 6px;
  margin-right: var(--sp-2);
  vertical-align: middle;
  width: 6px;
}
</style>
