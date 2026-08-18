<template>
  <section class="terminal-panel">
    <div class="terminal-panel__status" role="status">
      <span
        class="terminal-panel__dot"
        :class="{ 'is-running': running }"
        aria-hidden="true"
      />
      <span v-if="connected && running">运行中{{ cwd ? ` · ${cwd}` : '' }}</span>
      <span v-else-if="connected">会话已结束</span>
      <span v-else>未连接</span>
    </div>

    <div v-if="errorMessage" class="terminal-panel__error" role="alert">
      <Icon name="info" :size="15" />
      <span>{{ errorMessage }}</span>
      <button type="button" class="btn btn--ghost" @click="emitEvent('terminal-restart')">
        {{ t('workbench.terminal.restart') }}
      </button>
    </div>

    <div
      ref="outputRef"
      class="terminal-panel__output"
      aria-live="polite"
      data-testid="terminal-output"
    >
      <pre v-if="output">{{ output }}</pre>
      <p v-else class="terminal-panel__hint">{{ t('workbench.terminal.emptyHint') }}</p>
    </div>

    <form class="terminal-panel__input-row" @submit.prevent="submit">
      <span class="terminal-panel__prompt" aria-hidden="true">&gt;</span>
      <input
        v-model="command"
        class="terminal-panel__input"
        type="text"
        autocomplete="off"
        spellcheck="false"
        :disabled="!connected || !running"
        :placeholder="t('workbench.terminal.placeholder')"
        :aria-label="t('workbench.terminal.placeholder')"
        data-testid="terminal-input"
      >
      <button
        type="submit"
        class="btn btn--ghost"
        :disabled="!connected || !running || !command"
      >
        {{ t('workbench.terminal.send') }}
      </button>
      <button
        type="button"
        class="btn btn--icon btn--ghost"
        :aria-label="t('workbench.terminal.terminate')"
        :title="t('workbench.terminal.terminate')"
        :disabled="!connected"
        @click="emitEvent('terminal-kill')"
      >
        <Icon name="stop" :size="15" />
      </button>
    </form>
  </section>
</template>

<script setup lang="ts">
import { nextTick, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'

const props = defineProps<{
  cwd?: string
  output?: string
  running?: boolean
  connected?: boolean
  errorMessage?: string
}>()

const emit = defineEmits<{
  (e: 'workbench-event', event: { type: string; payload?: Record<string, unknown> }): void
}>()

const { t } = useI18n()
const command = ref('')
const outputRef = ref<HTMLElement | null>(null)

function emitEvent(type: string): void {
  emit('workbench-event', { type })
}

function submit(): void {
  const data = command.value.trim()
  if (!data || !props.connected || !props.running) return
  emit('workbench-event', {
    type: 'terminal-input',
    payload: { data },
  })
  command.value = ''
}

watch(
  () => props.output,
  () => {
    void nextTick(() => {
      const el = outputRef.value
      if (el) el.scrollTop = el.scrollHeight
    })
  },
)

onMounted(() => {
  // 打开面板时聚焦输入框
  const input = document.querySelector<HTMLInputElement>('[data-testid="terminal-input"]')
  input?.focus()
})
</script>

<style scoped>
.terminal-panel {
  display: grid;
  grid-template-rows: auto auto minmax(0, 1fr) auto;
  height: 100%;
  min-height: 0;
  gap: var(--sp-2);
  padding: var(--sp-3);
  background: var(--bg-surface);
  font-family: var(--font-mono, ui-monospace, monospace);
  font-size: var(--font-size-sm, 13px);
}

.terminal-panel__status {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  color: var(--text-dim);
  font-size: 12px;
}

.terminal-panel__dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-dim);
}

.terminal-panel__dot.is-running {
  background: var(--ok);
}

.terminal-panel__error {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: var(--sp-2);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--danger) 12%, transparent);
  color: var(--danger);
}

.terminal-panel__output {
  overflow: auto;
  min-height: 0;
  padding: var(--sp-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg);
  white-space: pre-wrap;
  word-break: break-all;
}

.terminal-panel__output pre {
  margin: 0;
  color: var(--text);
}

.terminal-panel__hint {
  margin: 0;
  color: var(--text-muted);
}

.terminal-panel__input-row {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}

.terminal-panel__prompt {
  color: var(--accent);
  font-weight: 700;
}

.terminal-panel__input {
  flex: 1;
  min-width: 0;
  padding: var(--sp-1) var(--sp-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg);
  color: var(--text);
  font-family: inherit;
}

.terminal-panel__input:disabled {
  opacity: 0.55;
}
</style>
