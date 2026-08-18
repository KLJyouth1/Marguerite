<template>
  <div class="agent-card" :class="`agent-card--${tone}`">
    <div class="agent-card__head">
      <span class="agent-card__icon" aria-hidden="true">
        <Icon :name="statusIcon" :size="14" />
      </span>
      <span class="agent-card__title">{{ t('chat.subagentCard.title') }}</span>
      <span class="agent-card__badge" :class="`agent-card__badge--${tone}`">{{ statusLabel }}</span>
      <time v-if="timeIso" class="agent-card__time" :datetime="timeIso" :title="timeFull">{{ timeAbs }}</time>
    </div>

    <p v-if="summaryText" class="agent-card__summary" :class="{ 'is-clamped': !expanded && summaryLong }">
      {{ summaryText }}
    </p>
    <button
      v-if="summaryLong"
      type="button"
      class="agent-card__more"
      @click="expanded = !expanded"
    >{{ expanded ? t('chat.subagentCard.collapse') : t('chat.subagentCard.expand') }}</button>

    <div v-if="sessionShort" class="agent-card__meta">
      <code class="agent-card__session">{{ sessionShort }}</code>
      <button
        type="button"
        class="agent-card__copy"
        :title="t('chat.subagentCard.copySession')"
        :aria-label="t('chat.subagentCard.copySession')"
        @click="copySession"
      >
        <Icon name="copy" :size="12" />
      </button>
    </div>

    <details v-if="rawJson" class="agent-card__details">
      <summary>{{ t('chat.subagentCard.viewJson') }}</summary>
      <pre>{{ rawJson }}</pre>
    </details>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import type { ChatRenderedMessage } from '@/types/chat'
import { copyTextWithFallback } from '@/utils/browser'
import { absoluteTime, fullTime, isoTime } from '@/utils/messageTime'

interface SubagentPayload {
  type?: string
  status?: string
  terminal_reason?: string
  child_session_key?: string
  parent_session_key?: string
  result?: { text?: string; summary?: string; [key: string]: unknown }
  [key: string]: unknown
}

const props = defineProps<{ message: ChatRenderedMessage }>()

const { t } = useI18n()
const expanded = ref(false)
const copied = ref(false)

const payload = computed<SubagentPayload | null>(() => {
  try {
    const parsed = JSON.parse(props.message.text)
    return parsed && typeof parsed === 'object' ? parsed as SubagentPayload : null
  } catch {
    return null
  }
})

const rawJson = computed(() => {
  try {
    return JSON.stringify(payload.value, null, 2)
  } catch {
    return props.message.text || ''
  }
})

// 状态归一：completed/success/done → ok；failed/error/timeout/cancelled → err；
// running/queued/in_progress → run；其余 → idle。
const STATUS_OK = new Set(['success', 'completed', 'done', 'finished', 'ok'])
const STATUS_ERR = new Set(['failed', 'error', 'timeout', 'cancelled', 'aborted', 'rejected'])
const STATUS_RUN = new Set(['running', 'queued', 'in_progress', 'pending', 'working'])

const tone = computed<'ok' | 'err' | 'run' | 'idle'>(() => {
  const status = String(payload.value?.status || '').toLowerCase()
  if (STATUS_OK.has(status)) return 'ok'
  if (STATUS_ERR.has(status)) return 'err'
  if (STATUS_RUN.has(status)) return 'run'
  return 'idle'
})

const statusIcon = computed(() => {
  if (tone.value === 'ok') return 'check'
  if (tone.value === 'err') return 'x'
  if (tone.value === 'run') return 'clock'
  return 'terminal'
})

const statusLabel = computed(() => {
  if (tone.value === 'ok') return t('chat.subagentCard.statusDone')
  if (tone.value === 'err') return t('chat.subagentCard.statusFailed')
  if (tone.value === 'run') return t('chat.subagentCard.statusRunning')
  return t('chat.subagentCard.statusIdle')
})

const resultText = computed(() => {
  const result = payload.value?.result
  const text = typeof result?.text === 'string' && result.text.trim()
    ? result.text.trim()
    : typeof result?.summary === 'string' && result.summary.trim()
      ? result.summary.trim()
      : ''
  return text
})

const terminalReason = computed(() => {
  const reason = payload.value?.terminal_reason
  return typeof reason === 'string' && reason.trim() ? reason.trim() : ''
})

const summaryText = computed(() => {
  const parts: string[] = []
  if (resultText.value) parts.push(resultText.value)
  if (terminalReason.value && terminalReason.value !== resultText.value) {
    parts.push(`[${terminalReason.value}]`)
  }
  return parts.join('\n')
})

const SUMMARY_CLAMP_CHARS = 220
const summaryLong = computed(() => summaryText.value.length > SUMMARY_CLAMP_CHARS)

const sessionShort = computed(() => {
  const key = String(payload.value?.child_session_key || payload.value?.session_key || '').trim()
  if (!key) return ''
  return key.length > 40 ? `${key.slice(0, 18)}…${key.slice(-14)}` : key
})

async function copySession() {
  const key = String(payload.value?.child_session_key || payload.value?.session_key || '').trim()
  if (!key) return
  try {
    await copyTextWithFallback(key)
    copied.value = true
    window.setTimeout(() => { copied.value = false }, 1600)
  } catch {
    copied.value = false
  }
}

const timeIso = computed(() => isoTime(props.message.ts))
const timeAbs = computed(() => absoluteTime(props.message.ts))
const timeFull = computed(() => fullTime(props.message.ts))
</script>

<style scoped>
.agent-card {
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-left: 3px solid var(--text-dim);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-xs);
  display: flex;
  flex-direction: column;
  gap: 0.375rem;
  max-width: 560px;
  padding: 0.625rem 0.75rem;
}

.agent-card--ok { border-left-color: var(--ok); }
.agent-card--err { border-left-color: var(--danger); }
.agent-card--run { border-left-color: var(--accent); }

.agent-card__head {
  align-items: center;
  display: flex;
  gap: 0.4375rem;
  min-width: 0;
}

.agent-card__icon {
  color: var(--text-dim);
  display: inline-flex;
  flex-shrink: 0;
}

.agent-card--ok .agent-card__icon { color: var(--ok); }
.agent-card--err .agent-card__icon { color: var(--danger); }
.agent-card--run .agent-card__icon { color: var(--accent); }

.agent-card__title {
  color: var(--text);
  font-size: 0.8125rem;
  font-weight: 650;
  line-height: 1.4;
}

.agent-card__badge {
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  color: var(--text-dim);
  flex-shrink: 0;
  font-size: 0.625rem;
  font-weight: 700;
  letter-spacing: 0.05em;
  line-height: 1.2;
  padding: 0.125rem 0.4375rem;
  text-transform: uppercase;
}

.agent-card__badge--ok {
  border-color: color-mix(in srgb, var(--ok) 45%, var(--border));
  color: var(--ok);
}

.agent-card__badge--err {
  border-color: color-mix(in srgb, var(--danger) 45%, var(--border));
  color: var(--danger);
}

.agent-card__badge--run {
  border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  color: var(--accent);
}

.agent-card__time {
  color: var(--text-dim);
  font-size: var(--fs-xs);
  font-variant-numeric: tabular-nums;
  margin-left: auto;
  opacity: 0;
  transition: opacity var(--dur-fast);
  white-space: nowrap;
}

.agent-card:hover .agent-card__time {
  opacity: 1;
}

@media (hover: none) {
  .agent-card__time { opacity: 1; }
}

.agent-card__summary {
  color: var(--text-muted);
  font-size: 0.8125rem;
  line-height: 1.55;
  margin: 0;
  overflow-wrap: anywhere;
  white-space: pre-wrap;
}

.agent-card__summary.is-clamped {
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 3;
  overflow: hidden;
}

.agent-card__more {
  align-self: flex-start;
  background: transparent;
  border: 0;
  color: var(--accent);
  cursor: pointer;
  font: inherit;
  font-size: 0.75rem;
  font-weight: 600;
  padding: 0;
}

.agent-card__more:hover { text-decoration: underline; }

.agent-card__meta {
  align-items: center;
  display: flex;
  gap: 0.3125rem;
  min-width: 0;
}

.agent-card__session {
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-dim);
  font-family: var(--font-mono);
  font-size: 0.6875rem;
  line-height: 1.4;
  min-width: 0;
  overflow: hidden;
  padding: 0.125rem 0.375rem;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.agent-card__copy {
  align-items: center;
  background: transparent;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  color: var(--text-dim);
  cursor: pointer;
  display: inline-flex;
  height: 1.375rem;
  justify-content: center;
  padding: 0;
  transition: color var(--dur-fast), border-color var(--dur-fast);
  width: 1.375rem;
}

.agent-card__copy:hover {
  border-color: var(--border);
  color: var(--text);
}

.agent-card__copy:focus-visible {
  box-shadow: var(--focus-ring);
  outline: 0;
}

.agent-card__details {
  border-top: 1px solid var(--hairline);
  font-size: 0.75rem;
  margin: 0.125rem 0 0;
  padding-top: 0.375rem;
}

.agent-card__details summary {
  color: var(--text-dim);
  cursor: pointer;
  font-weight: 600;
  user-select: none;
}

.agent-card__details pre {
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  font-family: var(--font-mono);
  font-size: 0.6875rem;
  line-height: 1.5;
  margin: 0.375rem 0 0;
  max-height: 200px;
  overflow: auto;
  padding: 0.5rem;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
</style>
