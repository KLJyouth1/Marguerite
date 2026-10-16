<template>
  <!-- S5 host-exit evidence card.

       Skeleton choice: `ErrorState.vue`, NOT `.msg-error-card`. The latter looks
       like a better fit but its styles live in a `<style scoped>` block inside
       `SystemMessage.vue`, so those class names are unreachable from any other
       component — reusing them would have produced an unstyled card with
       confident-looking class names. `ErrorState` is a real component with its
       own styles and a `message + retry` contract, which is exactly the shape
       needed here.

       Deliberate omission: `stderr_tail` NEVER appears in the headline. It can
       contain file paths, URLs and usernames, so it is confined to a collapsed
       <details> block. This mirrors the discipline in
       `core/src/model/error_taxonomy.rs`. -->
  <div v-if="visible" class="host-exit">
    <ErrorState :message="heading" :on-retry="onRetry" />

    <!-- Structured facts. Each row is omitted when its value is unknown rather
         than rendered as a placeholder that implies we looked and found
         nothing — "exit code unknown" and "we have no data" are different
         claims. -->
    <dl v-if="facts.length" class="host-exit__facts">
      <template v-for="fact in facts" :key="fact.label">
        <dt class="host-exit__fact-label">{{ fact.label }}</dt>
        <dd class="host-exit__fact-value">{{ fact.value }}</dd>
      </template>
    </dl>

    <!-- stderr tail: collapsed by default and capped for display. The core side
         already caps at 16KB; this cap is about not dumping a wall of text into
         the DOM when the user does expand it. -->
    <details v-if="displayTail" class="host-exit__details">
      <summary class="host-exit__summary">{{ t('chat.errors.hostExitStderrLabel') }}</summary>
      <pre class="host-exit__tail">{{ displayTail }}</pre>
    </details>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import ErrorState from '@/components/ErrorState.vue'
import { useHostExitEvidence } from '@/composables/useHostExitEvidence'
import { HOST_EXITED, localizedChatErrorMessage } from '@/utils/chat/errors'

const { t } = useI18n()

// Injects the App.vue-owned instance; falls back to a local one when mounted
// outside the app shell (same contract as `useGatewayReadiness`).
const { hasEvidence, sidecarPresent, exitCode, uptimeMs, stderrTail, errorContext, loading, refresh } =
  useHostExitEvidence()

/** Cap the rendered tail. The full 16KB stays available via the RPC. */
const DISPLAY_TAIL_LIMIT = 2000

// 卡片是「用户可见的问题指示器」，必须反映**当前**状态而非历史记录：
// `exited` 是诊断证据（"曾经退出过"），`sidecar_present` 才是"现在是否在位"。
// 侧车已在位时继续喊「已退出」是假状态（实测：瞬时退出后新侧车已健康，
// 卡片仍常驻，见 useHostExitEvidence.ts 的 sidecar_present 契约注释）。
const visible = computed(() => hasEvidence.value && !sidecarPresent.value)

const heading = computed(() =>
  localizedChatErrorMessage(HOST_EXITED, t('chat.errors.hostExitedGeneric'), errorContext.value),
)

const displayTail = computed(() => {
  const raw = stderrTail.value.trim()
  if (!raw) return ''
  return raw.length > DISPLAY_TAIL_LIMIT
    ? `${raw.slice(0, DISPLAY_TAIL_LIMIT)}\n…`
    : raw
})

const facts = computed(() => {
  const rows: Array<{ label: string; value: string }> = []
  if (exitCode.value !== null) {
    rows.push({ label: t('chat.errors.hostExitCodeLabel'), value: String(exitCode.value) })
  }
  if (uptimeMs.value !== null) {
    rows.push({
      label: t('chat.errors.hostExitUptimeLabel'),
      value: errorContext.value.uptime_ms,
    })
  }
  return rows
})

function onRetry(): void {
  if (loading.value) return
  void refresh()
}
</script>

<style scoped>
.host-exit {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}

.host-exit__facts {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: var(--sp-1) var(--sp-3);
  margin: 0;
  padding: var(--sp-3);
  font-size: var(--fs-xs);
  text-align: left;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}

.host-exit__fact-label {
  color: var(--text-muted);
  white-space: nowrap;
}

.host-exit__fact-value {
  margin: 0;
  font-family: var(--font-mono);
  color: var(--text);
}

.host-exit__details {
  font-size: var(--fs-xs);
  text-align: left;
}

.host-exit__summary {
  cursor: pointer;
  color: var(--text-muted);
}

.host-exit__tail {
  max-height: 12rem;
  margin: var(--sp-2) 0 0;
  padding: var(--sp-2);
  overflow: auto;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
}
</style>