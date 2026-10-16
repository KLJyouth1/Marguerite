<template>
  <div ref="rootRef" class="run-mode-panel">
    <button
      type="button"
      class="btn btn--icon btn--ghost run-mode-panel__chip"
      :class="{ 'is-active': open }"
      :title="t('chat.composer.runModePanel')"
      :aria-label="t('chat.composer.runModePanel')"
      aria-haspopup="dialog"
      :aria-expanded="open ? 'true' : 'false'"
      :aria-describedby="runModeLocked ? 'run-mode-panel-lock-tip' : undefined"
      @click="toggleOpen"
    >
      <Icon name="shield" :size="17" />
      <span
        v-if="showNewBadge"
        class="run-mode-panel__new"
        aria-hidden="true"
      >{{ t('chat.composer.badgeNew') }}</span>
      <span class="run-mode-panel__chip-label">{{ summaryLabel }}</span>
    </button>
    <span
      v-if="runModeLocked"
      id="run-mode-panel-lock-tip"
      class="run-mode-panel__lock-tip"
      role="tooltip"
    >{{ runModeLockMessage }}</span>

      <section
        v-if="open"
        class="run-mode-panel__popover"
        role="dialog"
        :aria-label="t('chat.composer.runModePanel')"
        @keydown.esc.stop="close"
      >
      <div class="run-mode-panel__head">
        <span>{{ t('chat.composer.runModePanel') }}</span>
        <button
          type="button"
          class="run-mode-panel__close"
          :aria-label="t('chat.closeComposerSettings')"
          @click="close"
        >
          <Icon name="x" :size="14" />
        </button>
      </div>

      <div class="run-mode-panel__section">
        <div class="run-mode-panel__section-title">{{ t('chat.composer.runMode') }}</div>
        <p
          v-if="runModeLocked"
          class="run-mode-panel__lock-note"
          role="note"
        >{{ runModeLockMessage }}</p>
        <ChatComposerRunMode
          :run-mode="runMode"
          :allowed-run-modes="allowedRunModes"
          :safe-setup-available="safeSetupAvailable"
          :run-mode-locked="runModeLocked"
          :run-mode-lock-message="runModeLockMessage"
          @close="close"
          @set-run-mode="emit('setRunMode', $event)"
        />
      </div>

      <div class="run-mode-panel__section">
        <div class="run-mode-panel__section-title">{{ t('chat.planMode.label') }}</div>
        <div
          class="run-mode-panel__plan"
          role="radiogroup"
          :aria-label="t('chat.planMode.label')"
        >
          <button
            v-for="option in planOptions"
            :key="option.value"
            type="button"
            class="run-mode-panel__plan-option"
            :class="{ 'is-active': effectiveCollaborationMode === option.value }"
            :disabled="option.disabled"
            role="radio"
            :aria-checked="effectiveCollaborationMode === option.value ? 'true' : 'false'"
            @click="selectPlan(option.value)"
          >
            <span class="run-mode-panel__plan-label">{{ option.label }}</span>
            <Icon
              v-if="effectiveCollaborationMode === option.value"
              name="check"
              :size="14"
            />
          </button>
        </div>
        <ChatComposerPlanMode
          v-if="effectiveCollaborationMode === 'plan'"
          :available="planModeAvailable === true"
          :mode="effectiveCollaborationMode"
          :busy="planModeBusy === true"
          :disabled="planModeDisabled === true"
          :applies-next-turn="planModeAppliesNextTurn === true"
          @set-mode="emit('setCollaborationMode', $event)"
        />
      </div>

      <div class="run-mode-panel__section">
        <div class="run-mode-panel__section-title">{{ t('chat.composer.modelRouting') }}</div>
        <ChatComposerModelRouting
          :model-routing-mode="modelRoutingMode"
          :busy="modelRoutingSettingsBusy"
          @close="close"
          @set-model-routing-mode="emit('setModelRoutingMode', $event)"
        />
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import ChatComposerRunMode from '@/components/chat/ChatComposerRunMode.vue'
import ChatComposerModelRouting from '@/components/chat/ChatComposerModelRouting.vue'
import ChatComposerPlanMode from '@/components/chat/ChatComposerPlanMode.vue'
import type { SandboxRunMode } from '@/types/sandbox'
import type { ModelRoutingMode } from '@/types/modelRouting'
import type { CollaborationMode } from '@/types/plans'

const { t } = useI18n()

const open = defineModel<boolean>('open', { default: false })

const props = defineProps<{
  runMode: SandboxRunMode
  allowedRunModes: SandboxRunMode[]
  safeSetupAvailable?: boolean
  runModeLocked: boolean
  runModeLockMessage: string
  collaborationMode?: CollaborationMode
  planModeAvailable?: boolean
  planModeBusy?: boolean
  planModeDisabled?: boolean
  planModeAppliesNextTurn?: boolean
  modelRoutingMode: ModelRoutingMode
  modelRoutingSettingsBusy: boolean
  /** 模型路由「NEW」角标（首次打开前展示）。 */
  showNewBadge?: boolean
}>()

const emit = defineEmits<{
  setRunMode: [mode: SandboxRunMode]
  setCollaborationMode: [mode: CollaborationMode]
  setModelRoutingMode: [mode: ModelRoutingMode]
}>()

const effectiveCollaborationMode = computed<CollaborationMode>(
  () => props.collaborationMode || 'default',
)

const sandboxShort = computed(() =>
  t(`chat.composer.modeShort.${props.runMode === 'safe' ? 'safe' : 'full'}`),
)
const collaborationShort = computed(() =>
  t(`chat.composer.modeShort.${effectiveCollaborationMode.value === 'plan' ? 'plan' : 'default'}`),
)
const routingShort = computed(() => {
  const map: Record<ModelRoutingMode, string> = {
    off: 'off',
    squilla_router: 'squilla',
    llm_ensemble: 'ensemble',
    bloom_ensemble: 'bloom',
    omni_agent: 'omni',
  }
  return t(`chat.composer.modeShort.${map[props.modelRoutingMode]}`)
})
const summaryLabel = computed(
  () => `${sandboxShort.value}·${collaborationShort.value}·${routingShort.value}`,
)

const planOptions = computed(() => [
  {
    value: 'default' as CollaborationMode,
    label: t('chat.composer.modeShort.default'),
    disabled: props.planModeDisabled || props.planModeBusy,
  },
  {
    value: 'plan' as CollaborationMode,
    label: t('chat.composer.modeShort.plan'),
    disabled: props.planModeDisabled || props.planModeBusy || !props.planModeAvailable,
  },
])

function selectPlan(mode: CollaborationMode) {
  if (props.planModeDisabled || props.planModeBusy) return
  if (mode === 'plan' && !props.planModeAvailable) return
  emit('setCollaborationMode', mode)
  close()
}

function toggleOpen() {
  open.value = !open.value
}

function close() {
  open.value = false
}

const rootRef = ref<HTMLElement | null>(null)

function eventInsideRoot(event: PointerEvent): boolean {
  const root = rootRef.value
  if (!root) return false
  const path = typeof event.composedPath === 'function' ? event.composedPath() : []
  if (path.includes(root)) return true
  return event.target instanceof Node && root.contains(event.target)
}

function onOutsidePointerDown(event: PointerEvent) {
  if (open.value && !eventInsideRoot(event)) {
    open.value = false
  }
}

watch(open, (isOpen) => {
  if (isOpen) {
    document.addEventListener('pointerdown', onOutsidePointerDown, true)
  } else {
    document.removeEventListener('pointerdown', onOutsidePointerDown, true)
  }
}, { immediate: true })

onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', onOutsidePointerDown, true)
})
</script>

<style scoped>
.run-mode-panel {
  position: relative;
  display: inline-flex;
  align-items: center;
  /* Size to the chip's own content. A shrinkable flex item collapsed here to the
     chip's minimum (32px, `.btn--icon`'s square) and clipped the mode summary.
     The neighbouring composer buttons own fixed widths, so nothing else gives. */
  flex: 0 0 auto;
}

.run-mode-panel__chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-height: 34px;
  padding: 0 9px;
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  font-size: var(--fs-xs);
  font-weight: 650;
  white-space: nowrap;
}

/* Two global rules pin this chip to a 32px square: `.btn--icon` (base.css) and
   the surface-specific `#app .chat .chat-input-panel .btn--icon`
   (apple-modern.css). Its contents — shield glyph + NEW badge + the mode summary
   label — are ~190px wide, so the label used to overflow the circle and print
   across the neighbouring composer button. This is a labelled chip, not an icon
   button, so opt out of both; `!important` is the same escape hatch
   ChatComposer.vue uses against that id-scoped surface rule. */
.run-mode-panel__chip.btn--icon {
  width: auto !important;
  min-width: 0 !important;
  flex: 0 0 auto;
}

.run-mode-panel__chip-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

.run-mode-panel__chip:hover:not(:disabled),
.run-mode-panel__chip:focus-visible {
  border-color: var(--border);
  background: var(--bg-hover);
  color: var(--text);
  outline: none;
}

.run-mode-panel__chip:focus-visible {
  box-shadow: var(--focus-ring);
}

.run-mode-panel__chip.is-active {
  border-color: color-mix(in srgb, var(--accent) 30%, var(--border));
  background: color-mix(in srgb, var(--accent) 6%, var(--bg-surface));
  color: var(--text);
}

.run-mode-panel__chip:disabled {
  cursor: not-allowed;
  opacity: var(--state-disabled-opacity);
}

.run-mode-panel__chip-label {
  font-variant-numeric: tabular-nums;
}

.run-mode-panel__new {
  display: inline-flex;
  align-items: center;
  padding: 0 5px;
  height: 16px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--accent) 18%, transparent);
  color: var(--accent);
  font-size: 10px;
  font-weight: 800;
  letter-spacing: 0.02em;
  text-transform: uppercase;
}

.run-mode-panel__lock-tip {
  position: absolute;
  /* Pinned into the composer's control band, never over the box the user types
     in. The panel this is anchored to is 0-height (the chip overflows it while
     staying centred on it), so `bottom` is measured down from the chip's vertical
     centre: 17px puts the tip's bottom edge on the chip band, whose top edge sits
     ~5px below the textarea. The run-mode chip is disabled while the session is
     locked, so covering that control row costs nothing, while covering the
     message box would hide the text the user is writing. */
  bottom: 17px;
  left: 0;
  /* `width: max-content` is what makes the message readable: with only
     `max-width`, the absolutely-positioned box shrink-to-fits the 32px-wide
     chip's containing block and collapses to one word per line. The chip is
     icon-only (`.btn--icon` fixes it at 32px), so the tooltip is the only place
     this copy appears. */
  width: max-content;
  max-width: min(340px, calc(100vw - 32px));
  padding: 6px 9px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  box-shadow: var(--shadow-xl);
  color: var(--text);
  font-size: var(--fs-xs);
  line-height: 1.4;
  text-align: left;
  z-index: 50;
}

.run-mode-panel__popover {
  position: absolute;
  left: 0;
  bottom: calc(100% + 8px);
  width: min(360px, calc(100vw - 48px));
  padding: 0.75rem;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  box-shadow: var(--shadow-xl);
  z-index: 30;
  display: grid;
  gap: 0.875rem;
}

.run-mode-panel__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  font-size: 0.8125rem;
  font-weight: 700;
  color: var(--text);
}

.run-mode-panel__close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: 1px solid transparent;
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
}

.run-mode-panel__close:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.run-mode-panel__section {
  display: grid;
  gap: 0.45rem;
}

.run-mode-panel__section-title {
  font-size: 0.6875rem;
  font-weight: 700;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--text-dim);
}

.run-mode-panel__lock-note {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  line-height: 1.4;
}

.run-mode-panel__plan {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0.375rem;
}

.run-mode-panel__plan-option {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  min-height: 42px;
  padding: 0.5rem 0.625rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text);
  text-align: left;
  cursor: pointer;
}

.run-mode-panel__plan-option:hover:not(:disabled) {
  background: var(--bg-hover);
}

.run-mode-panel__plan-option:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent) 45%, transparent);
  outline-offset: 2px;
}

.run-mode-panel__plan-option.is-active {
  border-color: color-mix(in srgb, var(--ok) 55%, var(--border));
  background: color-mix(in srgb, var(--ok) 10%, var(--bg-surface));
}

.run-mode-panel__plan-option:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.run-mode-panel__plan-option:disabled:hover {
  background: transparent;
}

.run-mode-panel__plan-label {
  font-size: 0.8125rem;
  font-weight: 700;
}

@media (max-width: 520px) {
  .run-mode-panel__popover {
    left: -2.75rem;
    width: calc(100vw - 32px);
  }
}
</style>
