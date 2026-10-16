<template>
  <div class="empty-state">
    <p class="empty-state__greeting">{{ greeting }}</p>
    <p class="empty-state__lead">{{ t('chat.emptyLead') }}</p>
    <ul class="empty-state__constellation" aria-hidden="true">
      <li v-for="node in orbitNodes" :key="node">{{ node }}</li>
    </ul>
    <div
      v-if="!suppressed"
      class="empty-state__chips"
      role="group"
      :aria-label="t('chat.suggestedTasks')"
      :aria-disabled="disabled || undefined"
    >
      <button
        v-for="chip in chips"
        :key="chip.label"
        type="button"
        class="empty-state__chip"
        :disabled="disabled"
        @click="emit('pick', chip.prompt)"
      >{{ chip.label }}</button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRpcCall } from '@/composables/useRpc'
import { optionalSessionRpcCallOptions } from '@/composables/chat/sessionBootstrapAdmission'

const { t } = useI18n()

interface CapabilityStatus {
  searchConfigured?: boolean
  imageGenerationConfigured?: boolean
  imageGenerationEnabled?: boolean
}

withDefaults(defineProps<{
  agentId: string
  suppressed?: boolean
  disabled?: boolean
}>(), {
  suppressed: false,
  disabled: false,
})

const emit = defineEmits<{
  pick: [text: string]
}>()

interface SuggestionChip {
  label: string
  prompt: string
}

function ordinaryChip(label: string): SuggestionChip {
  return { label, prompt: label }
}

const buildGameChip = computed<SuggestionChip>(() => ({
  label: t('chat.chips.buildGame'),
  prompt: t('chat.chips.buildGamePrompt'),
}))

const FALLBACK_CHIPS = computed(() => [
  buildGameChip.value,
  ordinaryChip(t('chat.chips.summarizeWebpage')),
  ordinaryChip(t('chat.chips.planWeek')),
])

const capabilityStatus = useRpcCall<CapabilityStatus>(
  'onboarding.status',
  undefined,
  { callOptions: optionalSessionRpcCallOptions },
)

const greeting = computed(() => {
  const hour = new Date().getHours()
  if (hour >= 5 && hour < 12) return t('chat.greetingMorning')
  if (hour >= 12 && hour < 18) return t('chat.greetingAfternoon')
  return t('chat.greetingEvening')
})

const orbitNodes = computed(() => [
  t('chat.orbit.agent'),
  t('chat.orbit.subagent'),
  t('chat.orbit.tools'),
  t('chat.orbit.memory'),
  t('chat.orbit.knowledge'),
])

const chips = computed(() => {
  const status = capabilityStatus.data.value
  if (!status) return FALLBACK_CHIPS.value
  const derived: SuggestionChip[] = []
  if (status.searchConfigured) derived.push(ordinaryChip(t('chat.chips.searchAiNews')))
  if (status.imageGenerationConfigured && status.imageGenerationEnabled !== false) {
    derived.push(ordinaryChip(t('chat.chips.generateImage')))
  }
  derived.push(
    ordinaryChip(t('chat.chips.summarizeWebpage')),
    buildGameChip.value,
  )
  if (derived.length < 3) derived.push(ordinaryChip(t('chat.chips.planWeek')))
  return derived.slice(0, 4)
})
</script>

<style scoped>
.empty-state {
  /* The landing wrapper disables pointer events; the chips need them back. */
  pointer-events: auto;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--sp-2);
  position: relative;
  text-align: center;
  max-width: 36rem;
  margin-inline: auto;
}

/* Dawn halo behind the greeting — themes without --atmosphere-dawn simply
   render no halo. Kept behind text via z-index, never intercepts input. */
.empty-state::before {
  background: var(--atmosphere-dawn, transparent);
  border-radius: var(--radius-full);
  content: '';
  filter: blur(48px);
  inset: -34% -16%;
  opacity: 0.55;
  pointer-events: none;
  position: absolute;
  z-index: -1;
}

.empty-state__greeting {
  animation: empty-state-greeting-reveal calc(var(--dur-base) * 2.5) var(--ease-out) both;
  margin: var(--sp-2) 0 0;
  font-family: var(--font-display);
  font-size: clamp(1.75rem, 1rem + 1.8vw, 2.25rem);
  font-weight: 600;
  letter-spacing: var(--track-display);
  color: var(--text);
}

.empty-state__lead {
  margin: 0;
  max-width: 38ch;
  font-size: var(--fs-sm);
  color: var(--text-dim);
  line-height: 1.5;
  animation: empty-state-greeting-reveal calc(var(--dur-base) * 2.5) var(--ease-out) both;
  animation-delay: 40ms;
}

.empty-state__constellation {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 6px;
  margin: var(--sp-2) 0 0;
  padding: 0;
  list-style: none;
}

.empty-state__constellation > li {
  padding: 3px 9px;
  border-radius: var(--radius-pill);
  border: 1px solid color-mix(in srgb, var(--accent) 22%, var(--border));
  background: color-mix(in srgb, var(--accent) 6%, transparent);
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: var(--text-muted);
  animation: empty-state-greeting-reveal calc(var(--dur-base) * 2.5) var(--ease-out) both;
}

.empty-state__constellation > li:nth-child(1) { animation-delay: 60ms; }
.empty-state__constellation > li:nth-child(2) { animation-delay: 100ms; }
.empty-state__constellation > li:nth-child(3) { animation-delay: 140ms; }
.empty-state__constellation > li:nth-child(4) { animation-delay: 180ms; }
.empty-state__constellation > li:nth-child(5) { animation-delay: 220ms; }

@keyframes empty-state-greeting-reveal {
  from {
    filter: blur(5px);
    opacity: 0;
    transform: translateY(7px);
  }
  to {
    filter: blur(0);
    opacity: 1;
    transform: translateY(0);
  }
}

.empty-state__chips {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: var(--sp-2);
  margin-top: var(--sp-4);
  /* Reserve one chip row so late capability resolution cannot shift layout. */
  min-height: 2.25rem;
}

.empty-state__chip {
  display: inline-flex;
  align-items: center;
  min-height: 2.25rem;
  padding: 0.375rem 0.875rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  background: var(--bg-elevated);
  font: inherit;
  font-size: 0.8125rem;
  color: var(--text-muted);
  cursor: pointer;
  transition: background var(--transition), border-color var(--transition), color var(--transition);
}

.empty-state__chip:hover {
  background: var(--bg-hover);
  border-color: var(--border-strong);
  color: var(--text);
}

.empty-state__chip:disabled {
  cursor: default;
  opacity: 0.72;
}

.empty-state__chip:disabled:hover {
  background: var(--bg-elevated);
  border-color: var(--border);
  color: var(--text-muted);
}

.empty-state__chip:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}

@media (max-width: 768px) {
  .empty-state__chip {
    min-height: 2.75rem;
  }

  .empty-state__chips {
    min-height: 2.75rem;
  }
}

@media (prefers-reduced-motion: reduce) {
  .empty-state__greeting,
  .empty-state__lead,
  .empty-state__constellation > li {
    animation: none;
  }

  .empty-state__chip {
    transition: none;
  }
}
</style>
