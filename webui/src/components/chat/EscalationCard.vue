<template>
  <article
    class="escalation-card"
    data-testid="escalation-card"
    :data-escalation-id="event.id"
    :data-trigger="event.trigger"
    role="group"
    :aria-label="t('chat.escalation.requiredFor', { reason: event.reason })"
  >
    <div class="escalation-card__announce" aria-live="assertive" aria-atomic="true">
      {{ t('chat.escalation.neededFor', { reason: event.reason }) }}
    </div>
    <header class="escalation-card__head">
      <span class="escalation-card__icon" aria-hidden="true">
        <Icon name="alertTriangle" :size="18" />
      </span>
      <div class="escalation-card__heading">
        <span class="escalation-card__eyebrow">{{ t('chat.escalation.title') }}</span>
        <h3 class="escalation-card__title">{{ t(`chat.escalation.trigger.${event.trigger}`) }}</h3>
      </div>
      <span
        class="escalation-card__severity"
        :class="`escalation-card__severity--${severityClass}`"
      >severity {{ event.severity }}</span>
    </header>

    <div class="escalation-card__body">
      <p class="escalation-card__reason" data-testid="escalation-reason">{{ event.reason }}</p>
      <dl v-if="targetRow" class="escalation-card__context">
        <div class="escalation-card__context-row">
          <dt>{{ t('chat.escalation.operation') }}</dt>
          <dd><code>{{ targetRow }}</code></dd>
        </div>
      </dl>
      <p v-if="event.suggested_action" class="escalation-card__suggest">
        {{ t('chat.escalation.suggestedAction') }}：<code>{{ event.suggested_action }}</code>
      </p>
    </div>

    <footer class="escalation-card__actions">
      <button
        type="button"
        class="btn btn--danger escalation-card__action"
        :disabled="busy"
        data-testid="escalation-abort"
        @click="emit('ack', 'abort')"
      >{{ t('chat.escalation.abort') }}</button>
      <button
        type="button"
        class="btn btn--primary escalation-card__action"
        :disabled="busy"
        data-testid="escalation-continue"
        @click="emit('ack', 'continue')"
      >{{ t('chat.escalation.continue') }}</button>
    </footer>
  </article>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import type { EscalationDecision, EscalationEvent } from '@/types/escalation'

const props = defineProps<{
  event: EscalationEvent
  busy?: boolean
}>()

const emit = defineEmits<{
  (e: 'ack', decision: EscalationDecision): void
}>()

const { t } = useI18n()

const severityClass = computed(() => {
  if (props.event.severity >= 8) return 'high'
  if (props.event.severity >= 4) return 'mid'
  return 'low'
})

const targetRow = computed(() => {
  const context = props.event.context || {}
  const raw = context.operation ?? context.plan_title ?? context.step ?? context.target
  return raw && typeof raw === 'string' ? raw : ''
})
</script>

<style scoped>
.escalation-card {
  border: 1px solid var(--border);
  border-left: 3px solid var(--danger);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  padding: 14px 16px;
  max-width: 560px;
  box-shadow: var(--shadow-lg);
}
.escalation-card__announce {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
}
.escalation-card__head {
  display: flex;
  align-items: center;
  gap: 10px;
}
.escalation-card__icon {
  display: inline-flex;
  color: var(--danger);
}
.escalation-card__heading {
  flex: 1;
  min-width: 0;
}
.escalation-card__eyebrow {
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  color: var(--text-dim);
}
.escalation-card__title {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
  color: var(--text);
}
.escalation-card__severity {
  font-size: 11px;
  padding: 2px 8px;
  border-radius: var(--radius-pill);
  border: 1px solid transparent;
}
.escalation-card__severity--high {
  color: var(--danger);
  background: color-mix(in srgb, var(--danger) 18%, var(--bg-surface));
}
.escalation-card__severity--mid {
  color: var(--warn);
  background: color-mix(in srgb, var(--warn) 18%, var(--bg-surface));
}
.escalation-card__severity--low {
  color: var(--info);
  background: color-mix(in srgb, var(--info) 18%, var(--bg-surface));
}
.escalation-card__context {
  margin: 10px 0 0;
}
.escalation-card__context-row {
  display: flex;
  gap: 8px;
  font-size: 13px;
}
.escalation-card__context-row dt {
  color: var(--text-dim);
}
.escalation-card__context-row dd code {
  background: var(--bg-elevated);
  padding: 2px 6px;
  border-radius: var(--radius-xs);
}
.escalation-card__reason {
  margin: 8px 0 0;
  font-size: 13px;
  line-height: 1.5;
  color: var(--text);
}
.escalation-card__suggest {
  margin: 8px 0 0;
  font-size: 12px;
  color: var(--text-dim);
}
.escalation-card__actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  margin-top: 14px;
}
</style>
