<template>
  <!-- Error role: distinct left-aligned card so a failed turn is unmissable. -->
  <div v-if="message.displayRole === 'error'" class="msg-error-wrap">
    <div class="msg-error-card" role="alert">
      <span class="msg-error-card__icon" aria-hidden="true">
        <Icon name="info" :size="16" />
      </span>
      <div class="msg-error-card__body">
        <span class="msg-error-card__heading">{{ t('chat.turnFailed') }}</span>
        <span v-if="message.text" class="msg-error-card__text">{{ message.text }}</span>
        <button
          v-if="showResume"
          type="button"
          class="msg-error-card__resume"
          :disabled="resolving"
          @click="onResume"
        >{{ t('chat.sandboxPausedResume') }}</button>
        <!-- P0-1c：鉴权失败（AUTH_FAILED）给出可操作引导——前往设置改 Key，或直接重试本轮。
             真机实测（2026-09-28）：计划子步骤失败/超时这类最常见的失败此前**没有任何出口**
             （只有 sandbox 暂停与鉴权失败有按钮），用户面对的是一条死路 —— 现在任何失败轮次
             都能直接重试（重发最后一条用户消息）。 -->
        <div v-if="showActions" class="msg-error-card__actions">
          <button
            v-if="showAuthActions"
            type="button"
            class="msg-error-card__action"
            @click="emit('open-settings')"
          >{{ t('chat.errorActions.openSettings') }}</button>
          <button
            v-if="showRetry"
            type="button"
            class="msg-error-card__action"
            @click="emit('retry')"
          >{{ t('chat.errorActions.retry') }}</button>
        </div>
        <!-- B2：失败轮次的次级出口 —— 看日志（该轮的技术细节在日志页）与
             改计划（仅当会话里存在可修订的方案卡）。刻意用独立 class，与主
             操作按钮区分：主操作是「复活本轮」，次级是「先看/先改再跑」。 -->
        <div v-if="showViewLogs || showReplan" class="msg-error-card__links">
          <button
            v-if="showReplan"
            type="button"
            class="msg-error-card__link"
            @click="emit('replan')"
          >{{ t('chat.errorActions.replan') }}</button>
          <button
            v-if="showViewLogs"
            type="button"
            class="msg-error-card__link"
            @click="emit('view-logs')"
          >{{ t('chat.errorActions.viewLogs') }}</button>
        </div>
      </div>
      <time v-if="timeIso" class="msg-error-card__time" :datetime="timeIso" :title="timeFull">{{ timeAbs }}</time>
    </div>
  </div>

  <!-- All other system roles: centered pill (unchanged). -->
  <div v-else class="msg-system-wrap">
    <div class="msg-system" :class="message.displayRole">
      <template v-if="message.displayRole === 'subagent'">
        <SubagentCard :message="message" @focus="emit('focus-subagent', $event)" />
      </template>
      <template v-else-if="message.text">
        <span class="msg-system-label">{{ message.roleLabel }}</span>
        <span class="msg-system__text">{{ message.text }}</span>
        <time v-if="timeIso" class="msg-system-time" :datetime="timeIso" :title="timeFull">{{ timeAbs }}</time>
      </template>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import SubagentCard from '@/components/chat/SubagentCard.vue'
import type { ChatRenderedMessage } from '@/types/chat'
import { absoluteTime, fullTime, isoTime } from '@/utils/messageTime'
import { AUTH_FAILED } from '@/utils/chat/errors'

const { t } = useI18n()

const props = defineProps<{ message: ChatRenderedMessage; canReplan?: boolean }>()

// Owner-driven recovery: a run paused by the sandbox denial ledger surfaces as a
// terminal error carrying this code. Offer a Resume action that the parent wires
// to the sandbox.resume RPC. Resume is idempotent, but we disable the button
// after one click to avoid duplicate confirmations.
const emit = defineEmits<{
  resume: []
  'open-settings': []
  retry: []
  replan: []
  'view-logs': []
  'focus-subagent': [payload: { key: string }]
}>()
const resolving = ref(false)
const showResume = computed(
  () =>
    props.message.displayRole === 'error' &&
    props.message.errorCode === 'sandbox_threshold_exceeded',
)
// P0-1c：鉴权失败时展示「前往设置」+「重试」。
const showAuthActions = computed(
  () =>
    props.message.displayRole === 'error' &&
    props.message.errorCode === AUTH_FAILED,
)
// 任何失败轮次都至少给一个出口：重发最后一条用户消息（ChatView.retryAuthFailure）。
// 例外是 sandbox 暂停（它有自己的 Resume，重试语义不同）。
const showRetry = computed(
  () => props.message.displayRole === 'error' && !showResume.value,
)
const showActions = computed(() => showAuthActions.value || showRetry.value)
// B2 次级出口：每次失败都能看日志；只有在会话里存在可修订方案卡时才提供改计划。
const showViewLogs = computed(() => props.message.displayRole === 'error')
const showReplan = computed(
  () => props.message.displayRole === 'error' && props.canReplan === true && !showResume.value,
)

function onResume() {
  if (resolving.value) return
  resolving.value = true
  emit('resume')
}

const timeIso = computed(() => isoTime(props.message.ts))
const timeAbs = computed(() => absoluteTime(props.message.ts))
const timeFull = computed(() => fullTime(props.message.ts))
</script>

<style scoped>
.msg-system-wrap {
  display: flex;
  justify-content: center;
  padding: 0.375rem 2rem;
}

.msg-system {
  font-size: 0.8125rem;
  color: var(--text-dim);
  padding: 0.25rem 0.625rem;
  border-radius: var(--radius-md);
  max-width: 70%;
  text-align: center;
}

.msg-system.error {
  background: color-mix(in srgb, var(--danger) 10%, var(--bg-surface));
  color: var(--danger);
}

.msg-system-label {
  font-weight: 600;
  margin-right: 0.375rem;
}

.msg-system__text {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

/* Quiet, hover-revealed timestamp on the centered status pill. */
.msg-system-time {
  margin-left: 0.375rem;
  font-size: var(--fs-xs);
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
  opacity: 0;
  transition: opacity var(--dur-fast);
}

.msg-system-wrap:hover .msg-system-time {
  opacity: 1;
}

@media (hover: none) {
  .msg-system-time {
    opacity: 1;
  }
}

/* ── Subagent card: replace the centered pill with a left-aligned card so a
    finished child agent reads as first-class activity, not system chatter. */
.msg-system-wrap:has(.agent-card) {
  justify-content: flex-start;
  padding: 0.375rem 1rem;
}

.msg-system:has(.agent-card) {
  background: transparent;
  padding: 0;
  text-align: left;
  max-width: 100%;
}

/* ── Error card (role: error) ──────────────────────────────────────────
   Left-aligned, danger-tinted card so a failed turn is unmissable.
   Distinct from the centered .msg-system pill used by all other roles. */
.msg-error-wrap {
  padding: 0.375rem 1rem;
}

.msg-error-card {
  display: flex;
  align-items: flex-start;
  gap: 0.625rem;
  padding: 0.75rem 1rem;
  border: 1px solid color-mix(in srgb, var(--danger) 40%, var(--border));
  border-left: 3px solid var(--danger);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--danger) 8%, var(--bg-surface));
  animation: errorCardIn var(--dur-base) var(--ease-out) both;
}

.msg-error-card__icon {
  flex-shrink: 0;
  color: var(--danger);
  margin-top: 0.0625rem;
}

.msg-error-card__body {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  min-width: 0;
  flex: 1;
}

.msg-error-card__heading {
  font-size: 0.8125rem;
  font-weight: 700;
  color: var(--danger);
  line-height: 1.3;
}

.msg-error-card__text {
  font-size: 0.8125rem;
  color: var(--text-muted);
  line-height: 1.5;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.msg-error-card__actions {
  display: flex;
  gap: 0.5rem;
  margin-top: 0.25rem;
}

/* B2 次级出口：文本链接式，权重低于主操作按钮。 */
.msg-error-card__links {
  display: flex;
  gap: 0.75rem;
  margin-top: 0.25rem;
}

.msg-error-card__link {
  align-self: flex-start;
  padding: 0;
  border: none;
  background: none;
  color: var(--text-dim);
  font-size: 0.75rem;
  text-decoration: underline;
  text-underline-offset: 2px;
  cursor: pointer;
  transition: color var(--dur-fast);
}

.msg-error-card__link:hover {
  color: var(--danger);
}

.msg-error-card__resume,
.msg-error-card__action {
  align-self: flex-start;
  margin-top: 0.25rem;
  padding: 0.3125rem 0.75rem;
  font-size: 0.8125rem;
  font-weight: 600;
  color: var(--danger);
  background: color-mix(in srgb, var(--danger) 12%, var(--bg-surface));
  border: 1px solid color-mix(in srgb, var(--danger) 40%, var(--border));
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background var(--dur-fast);
}

.msg-error-card__resume:hover:not(:disabled),
.msg-error-card__action:hover:not(:disabled) {
  background: color-mix(in srgb, var(--danger) 20%, var(--bg-surface));
}

.msg-error-card__resume:disabled {
  opacity: 0.55;
  cursor: default;
}

.msg-error-card__time {
  flex-shrink: 0;
  align-self: center;
  font-size: var(--fs-xs);
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
  opacity: 0;
  transition: opacity var(--dur-fast);
}

.msg-error-wrap:hover .msg-error-card__time {
  opacity: 1;
}

@media (hover: none) {
  .msg-error-card__time {
    opacity: 1;
  }
}

@keyframes errorCardIn {
  from {
    opacity: 0;
    transform: translateY(6px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@media (prefers-reduced-motion: reduce) {
  .msg-error-card {
    animation: none;
  }

  .msg-error-card__time {
    transition: none;
  }
}
</style>
