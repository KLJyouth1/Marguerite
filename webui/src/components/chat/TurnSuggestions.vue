<template>
  <section
    class="turn-suggestions"
    role="region"
    :aria-label="t('chat.turnSuggestions.region')"
    data-testid="turn-suggestions"
  >
    <span class="turn-suggestions__lead">{{ t('chat.turnSuggestions.lead') }}</span>
    <div class="turn-suggestions__chips">
      <button
        type="button"
        class="turn-suggestions__chip"
        data-testid="turn-suggestion-view"
        @click="emit('view')"
      >
        <Icon name="eye" :size="12" aria-hidden="true" />
        <span>{{ t('chat.turnSuggestions.view') }}</span>
      </button>
      <button
        type="button"
        class="turn-suggestions__chip"
        data-testid="turn-suggestion-tests"
        @click="emit('runTests')"
      >
        <Icon name="terminal" :size="12" aria-hidden="true" />
        <span>{{ t('chat.turnSuggestions.runTests') }}</span>
      </button>
      <button
        v-if="canRevert"
        type="button"
        class="turn-suggestions__chip turn-suggestions__chip--danger"
        data-testid="turn-suggestion-revert"
        @click="emit('revert')"
      >
        <Icon name="skipBack" :size="12" aria-hidden="true" />
        <span>{{ t('chat.turnSuggestions.revert') }}</span>
      </button>
    </div>
  </section>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'

defineProps<{
  canRevert?: boolean
}>()

const emit = defineEmits<{
  view: []
  runTests: []
  revert: []
}>()

const { t } = useI18n()
</script>

<style scoped>
/* 与消息行共用同一条阅读列：左对齐、固定起排，不居中。
   起排值必须取 --chat-gutter（不是写死 24px），否则它会比正文更靠左，
   在桌面宽度下正好落进左侧 minimap 轨道的让位区。 */
.turn-suggestions {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--sp-2);
  box-sizing: border-box;
  min-width: 0;
  width: 100%;
  max-width: var(--chat-col-start, var(--chat-col, min(calc(100% - 48px), 980px)));
  margin-inline: var(--chat-gutter, 24px) auto;
  padding: 0.125rem 0 0.5rem;
  text-align: start;
  color: var(--text-muted);
  font-size: var(--fs-xs);
}

.turn-suggestions__lead {
  flex: 0 0 auto;
  color: var(--text-dim);
  letter-spacing: 0.02em;
}

.turn-suggestions__chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  min-width: 0;
}

.turn-suggestions__chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  min-height: 26px;
  padding: 2px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: var(--bg-elevated, var(--bg-surface));
  color: var(--text);
  font-family: var(--font-sans);
  font-size: var(--fs-xs);
  line-height: 1.5;
  white-space: nowrap;
  cursor: pointer;
  transition:
    border-color var(--dur-fast) var(--ease-standard),
    background var(--dur-fast) var(--ease-standard),
    color var(--dur-fast) var(--ease-standard);
}

.turn-suggestions__chip:hover {
  border-color: var(--border-strong);
  background: var(--bg-hover);
}

.turn-suggestions__chip:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

.turn-suggestions__chip--danger {
  border-color: color-mix(in srgb, var(--danger) 30%, transparent);
  color: var(--danger);
}

.turn-suggestions__chip--danger:hover {
  background: color-mix(in srgb, var(--danger) 8%, transparent);
  border-color: var(--danger);
}
</style>
