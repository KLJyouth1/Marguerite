<template>
  <div v-if="open" class="tool-sheet-overlay" @click.self="emit('close')">
    <aside ref="rootRef" class="tool-sheet" role="dialog" aria-modal="true" :aria-label="label">
      <header class="tool-sheet__header">
        <slot name="header" />
        <div class="tool-sheet__actions">
          <slot name="actions" />
          <button
            ref="closeBtn"
            type="button"
            class="btn btn--icon btn--ghost"
            :title="t('common.close')"
            :aria-label="t('common.close')"
            @click="emit('close')"
          >
            <Icon name="x" :size="16" />
          </button>
        </div>
      </header>
      <div class="tool-sheet__body">
        <slot />
      </div>
    </aside>
  </div>
</template>

<script setup lang="ts">
import { ref, toRef } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useDialogA11y } from '@/composables/useDialogA11y'

const props = defineProps<{
  open: boolean
  label: string
}>()

const emit = defineEmits<{
  close: []
}>()

const { t } = useI18n()
const closeBtn = ref<HTMLButtonElement | null>(null)
const rootRef = ref<HTMLElement | null>(null)

useDialogA11y(rootRef, toRef(props, 'open'), () => emit('close'), { initialFocus: closeBtn })
</script>

<style scoped>
.tool-sheet-overlay {
  position: fixed;
  inset: 0;
  z-index: 300;
  background: var(--scrim);
  display: flex;
  justify-content: flex-end;
}

.tool-sheet {
  background: var(--bg-surface);
  border-left: 1px solid var(--border);
  box-shadow: var(--shadow-lg);
  display: flex;
  flex-direction: column;
  height: 100%;
  width: min(780px, 76vw);
  animation: toolSheetIn var(--dur-base) var(--ease-out);
}

@keyframes toolSheetIn {
  from { transform: translateX(24px); opacity: 0.4; }
  to { transform: translateX(0); opacity: 1; }
}

@media (prefers-reduced-motion: reduce) {
  .tool-sheet {
    animation: none;
  }
}

.tool-sheet__header {
  align-items: center;
  border-bottom: 1px solid var(--border);
  display: flex;
  gap: var(--sp-3);
  justify-content: space-between;
  padding: var(--sp-3) var(--sp-4);
}

.tool-sheet__actions {
  align-items: center;
  display: flex;
  flex-shrink: 0;
  flex-wrap: wrap;
  gap: var(--sp-2);
  justify-content: flex-end;
}

.tool-sheet__body {
  background: var(--bg);
  display: flex;
  flex: 1;
  min-height: 0;
  overflow: hidden;
  padding: var(--sp-3);
}

@media (max-width: 768px) {
  .tool-sheet {
    width: 100%;
  }
  .tool-sheet__header {
    align-items: flex-start;
    padding: var(--sp-2) var(--sp-3);
  }
  .tool-sheet__body {
    padding: var(--sp-2);
  }
}
</style>
