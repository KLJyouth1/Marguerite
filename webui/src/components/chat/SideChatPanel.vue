<!--
  侧聊天（Side chat）：与主会话并排运行的第二个对话。

  为什么不是一个「临时气泡」而是真正的会话键：
  · `chat.send` 的 SESSION_BUSY 是**按会话键**判定的 —— 复用主键只会被排队，
    永远做不到「边跑边问」；所以侧聊天用独立的 `agent:<id>:webchat:side-*` 键，
    `chat.send` 自动创建它，不需要 `sessions.create`。
  · WS 事件是广播的：本组件只接受 `meta.session`（或载荷 session_key/key）
    **严格等于侧键**的帧，主会话的 delta/done/approval 一律丢弃。
  · 主会话键是**只读**输入：它只用于按主会话持久化侧键，以及一次性
    `contextRefs: [{kind:'session', id: mainKey}]` 上下文注入。

  审批：侧键有自己的 useChatApprovals 实例（不挂 onSnapshotCount，不污染全局
  审批角标），ApprovalCard 直接渲染它的 entries 并调用它的 resolveApproval。
  若 `session.event.approval` 先于快照到达，仍会显示横幅 + 一张可裁决的兜底卡，
  裁决同样走 useChatApprovals 的 API（绝不伪造成功）。

  `basePath` 预留给网关挂载前缀（`/control`）下的链接拼接；当前面板内的请求
  都是同源绝对路径，因此只作为数据属性透出，避免猜一个并不存在的路由。
-->
<template>
  <div class="sc" data-testid="side-chat" :data-base-path="basePath || ''">
    <header class="sc__head">
      <div class="sc__heading">
        <span class="sc__title" :title="sideSessionKey">{{ t('chat.sideChat.title') }}</span>
        <span class="sc__subtitle">{{ t('chat.sideChat.subtitle') }}</span>
      </div>
      <div class="sc__head-actions">
        <button
          type="button"
          class="sc__icon"
          data-testid="side-chat-new"
          :title="t('chat.sideChat.newChat')"
          :aria-label="t('chat.sideChat.newChat')"
          @click="startNewChat"
        >
          <Icon name="plus" :size="13" />
        </button>
        <button
          v-if="streaming"
          type="button"
          class="sc__icon sc__icon--stop"
          data-testid="side-chat-stop"
          :title="t('chat.stopResponse')"
          :aria-label="t('chat.stopResponse')"
          @click="stop"
        >
          <Icon name="stop" :size="12" />
        </button>
      </div>
    </header>

    <button
      type="button"
      class="sc__chip"
      :class="{ 'is-attached': contextAttached, 'is-pending': contextAttachPending }"
      data-testid="side-chat-context"
      :disabled="!mainSessionKey"
      :aria-pressed="contextAttached || contextAttachPending"
      :title="t('chat.sideChat.attachContext')"
      @click="toggleContextAttachment"
    >
      <Icon :name="contextAttached ? 'check' : 'paperclip'" :size="11" />
      <span>{{ contextAttached ? t('chat.sideChat.contextAttached') : t('chat.sideChat.attachContext') }}</span>
    </button>

    <div ref="threadRef" class="sc__thread" data-testid="side-chat-thread">
      <div v-if="messages.length === 0 && !streaming" class="sc__empty" data-testid="side-chat-empty">
        <Icon name="chat" :size="26" :opacity="0.3" />
        <p>{{ t('chat.sideChat.empty') }}</p>
      </div>

      <ol v-else class="sc__messages">
        <li
          v-for="message in messages"
          :key="message.id"
          class="sc__msg"
          :class="`sc__msg--${message.role}`"
          :data-role="message.role"
        >
          <span class="sc__avatar" aria-hidden="true">
            <Icon :name="message.role === 'user' ? 'user' : 'sparkles'" :size="11" />
          </span>
          <div class="sc__bubble">
            <p class="sc__text">{{ message.text }}</p>
            <span v-if="message.status === 'failed'" class="sc__msg-failed">{{ t('chat.retry') }}</span>
          </div>
        </li>
      </ol>

      <div v-if="streaming" class="sc__streaming" data-testid="side-chat-streaming" role="status">
        <span class="sc__dot" aria-hidden="true" />
        <span>{{ t('chat.thinking') }}</span>
        <code v-if="activeTool" class="sc__tool">{{ activeTool }}</code>
      </div>

      <div v-if="failed" class="sc__error" data-testid="side-chat-error" role="alert">
        <Icon name="alertTriangle" :size="13" />
        <span class="sc__error-text">{{ t('chat.sideChat.failed', { error: failed }) }}</span>
        <button
          type="button"
          class="sc__error-retry"
          data-testid="side-chat-retry"
          @click="retry()"
        >
          {{ t('chat.retry') }}
        </button>
      </div>

      <div v-if="approvalEntries.length > 0" class="sc__approvals">
        <ApprovalCard
          v-for="entry in approvalEntries"
          :key="entry.approval.id"
          :approval="entry.approval"
          :resolution="entry.resolution"
          :busy="approvalBusyIds.has(entry.approval.id)"
          :error="entry.error"
          timeline
          @allow-once="resolveApproval(entry, 'allow-once')"
          @allow-always="resolveApproval(entry, 'allow-always')"
          @deny="resolveApproval(entry, 'deny')"
          @extend="extendInterrupt(entry.approval.id)"
        />
      </div>
      <!-- 快照尚未到达、但 session.event.approval 已点名侧键：兜底卡同样可裁决。 -->
      <div v-else-if="fallbackApproval" class="sc__approvals">
        <ApprovalCard
          :approval="fallbackApproval"
          :resolution="fallbackResolution"
          :busy="fallbackBusy"
          :error="fallbackError"
          timeline
          @allow-once="resolveFallback('allow-once')"
          @allow-always="resolveFallback('allow-always')"
          @deny="resolveFallback('deny')"
        />
      </div>
    </div>

    <div
      v-if="needsApproval"
      class="sc__banner"
      data-testid="side-chat-approval-banner"
      role="alert"
    >
      <Icon name="shield" :size="13" />
      <span>{{ t('chat.sideChat.approvalNeeded') }}</span>
    </div>

    <form class="sc__composer" @submit.prevent="submit">
      <textarea
        v-model="draft"
        class="sc__input"
        data-testid="side-chat-input"
        rows="2"
        :placeholder="t('chat.sideChat.placeholder')"
        :aria-label="t('chat.sideChat.placeholder')"
        @keydown="onComposerKeydown"
      />
      <button
        type="submit"
        class="sc__send"
        data-testid="side-chat-send"
        :disabled="!canSend"
        :title="t('chat.send')"
        :aria-label="t('chat.send')"
      >
        <Icon name="send" :size="14" />
      </button>
    </form>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, toRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import ApprovalCard from './ApprovalCard.vue'
import { useRpcStore } from '@/stores/rpc'
import {
  useChatApprovals,
  type ChatApprovalDecision,
  type ChatApprovalEntry,
  type ChatApprovalItem,
  type ChatApprovalResolution,
} from '@/composables/chat/useChatApprovals'
import { useSideChatSession } from '@/composables/chat/useSideChatSession'
import type { ChatRunStatus } from '@/types/chat'
import type { InterruptViewState } from '@/types/parts'

const props = defineProps<{
  /** 主会话键（只读）：侧键按它持久化，并作为上下文注入的目标。 */
  mainSessionKey: string
  /** 控制台挂载前缀（如 `/control`）；当前仅透出为数据属性，供后续链接使用。 */
  basePath?: string
}>()

const { t } = useI18n()
const rpc = useRpcStore()

const mainSessionKey = toRef(props, 'mainSessionKey')
const {
  sideSessionKey,
  messages,
  streaming,
  failed,
  activeTool,
  contextAttachPending,
  contextAttached,
  pendingApproval,
  send,
  retry,
  stop,
  startNewChat,
  toggleContextAttachment,
  subscribe,
  cleanup,
} = useSideChatSession({ mainSessionKey, rpc })

// ── 审批：侧键自己的实例 ──────────────────────────────────────────────
// 只提供 composable 需要的最小「turn 面」：审批卡片由 approvalEntries 渲染，
// 不走主会话的 interrupt 折叠，也不写全局审批角标（不传 onSnapshotCount）。
const interruptState = ref<ReadonlyMap<string, InterruptViewState>>(new Map())
const sideRunStatus = ref<ChatRunStatus>({ status: 'idle', label: '', task: null })
const {
  approvalEntries,
  approvalBusyIds,
  hasUnresolvedApproval,
  resolveApproval,
  extendInterrupt,
  subscribe: subscribeApprovals,
  cleanup: cleanupApprovals,
} = useChatApprovals({
  rpc,
  sessionKey: sideSessionKey,
  runStatus: sideRunStatus,
  stream: {
    isStreaming: streaming,
    appendInterruptFrame: () => {},
    ensureInterruptBubble: () => {},
  },
  interruptState,
})

const fallbackBusy = ref(false)
const fallbackResolution = ref<ChatApprovalResolution | null>(null)
const fallbackError = ref('')

/** 横幅只在「侧键确有未决审批」时出现。 */
const needsApproval = computed(() =>
  hasUnresolvedApproval.value
  || (pendingApproval.value !== null && fallbackResolution.value === null))

/**
 * `session.event.approval` 先到时（快照尚未拉回 / 事件族未进 exec 命名空间），
 * 用帧里的身份造一张兜底卡；它不是假卡 —— 裁决仍走 useChatApprovals 的
 * resolveApproval（真实 POST /api/approvals/resolve）。
 */
const fallbackApproval = computed<ChatApprovalItem | null>(() => {
  const notice = pendingApproval.value
  if (!notice) return null
  if (approvalEntries.value.some(entry => entry.approval.id === notice.id)) return null
  return {
    id: notice.id,
    namespace: notice.namespace || 'exec',
    toolName: notice.toolName,
    command: notice.command,
    approvalKind: '',
    args: null,
    warning: '',
    displayKind: notice.command ? 'run_command' : 'sensitive_operation',
    displayTarget: notice.command,
    destructive: false,
    irreversible: false,
    backupState: 'not_applicable',
    agent: '',
    sessionKey: notice.sessionKey,
    deadline: 0,
  }
})

const fallbackEntry = computed<ChatApprovalEntry | null>(() => {
  const approval = fallbackApproval.value
  if (!approval) return null
  return {
    approval,
    resolution: fallbackResolution.value,
    error: fallbackError.value,
  }
})

async function resolveFallback(decision: ChatApprovalDecision) {
  const entry = fallbackEntry.value
  if (!entry || fallbackBusy.value || entry.resolution) return
  fallbackBusy.value = true
  try {
    await resolveApproval(entry, decision)
    fallbackResolution.value = entry.resolution
    fallbackError.value = entry.error
  } finally {
    fallbackBusy.value = false
  }
}

// A new side session starts from a clean approval surface.
watch(sideSessionKey, () => {
  fallbackBusy.value = false
  fallbackResolution.value = null
  fallbackError.value = ''
})

// ── 输入 ──────────────────────────────────────────────────────────────
const draft = ref('')
const canSend = computed(() => draft.value.trim().length > 0 && !streaming.value)

function submit() {
  if (!canSend.value) return
  if (send(draft.value)) draft.value = ''
}

/** Enter 发送，Shift+Enter 换行；输入法组合中的 Enter 不发送。 */
function onComposerKeydown(event: KeyboardEvent) {
  if (event.key !== 'Enter' || event.shiftKey || event.isComposing) return
  event.preventDefault()
  submit()
}

// ── 生命周期 ──────────────────────────────────────────────────────────
let offSession: (() => void) | null = null
let offApprovals: (() => void) | null = null

onMounted(() => {
  offSession = subscribe()
  offApprovals = subscribeApprovals()
})
onBeforeUnmount(() => {
  offSession?.()
  offApprovals?.()
  cleanupApprovals()
  cleanup()
})

// 新消息 / 流式状态 / 审批卡出现时把滚动条压到底部。
const threadRef = ref<HTMLElement | null>(null)
watch(
  [messages, streaming, () => approvalEntries.value.length, () => pendingApproval.value?.id],
  async () => {
    await nextTick()
    const el = threadRef.value
    if (el) el.scrollTop = el.scrollHeight
  },
)
</script>

<style scoped>
.sc {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  height: 100%;
  min-height: 0;
}

/* ── Header ─────────────────────────────────────────────────────── */
.sc__head {
  display: flex;
  align-items: flex-start;
  gap: var(--sp-2);
  padding: 2px 2px 0;
}
.sc__heading {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1;
}
.sc__title {
  color: var(--text);
  font-size: var(--fs-sm);
  font-weight: 600;
  line-height: 1.3;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.sc__subtitle {
  color: var(--text-dim);
  font-size: var(--fs-xs);
  line-height: 1.4;
}
.sc__head-actions {
  display: flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
}
.sc__icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  transition: background var(--dur-fast) var(--ease-standard), color var(--dur-fast) var(--ease-standard);
}
.sc__icon:hover {
  background: var(--bg-hover);
  color: var(--text);
}
.sc__icon:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}
.sc__icon--stop {
  color: var(--danger);
}
.sc__icon--stop:hover {
  color: var(--danger);
  background: color-mix(in srgb, var(--danger) 12%, transparent);
}

/* ── Context chip ───────────────────────────────────────────────── */
.sc__chip {
  display: inline-flex;
  align-items: center;
  align-self: flex-start;
  gap: 4px;
  max-width: 100%;
  padding: 3px 9px;
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: var(--bg-surface);
  color: var(--text-muted);
  font-size: var(--fs-xs);
  cursor: pointer;
  transition: border-color var(--dur-fast) var(--ease-standard), color var(--dur-fast) var(--ease-standard);
}
.sc__chip:hover:not(:disabled) {
  color: var(--text);
  border-color: var(--border-strong);
}
.sc__chip:disabled {
  opacity: 0.5;
  cursor: default;
}
.sc__chip.is-pending {
  color: var(--accent);
  border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
}
.sc__chip.is-attached {
  color: var(--ok);
  border-color: color-mix(in srgb, var(--ok) 40%, var(--border));
}
.sc__chip:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}
.sc__chip span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* ── Thread ─────────────────────────────────────────────────────── */
.sc__thread {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  padding: 2px;
}
.sc__empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 28px 14px;
  color: var(--text-dim);
  text-align: center;
}
.sc__empty p {
  margin: 0;
  max-width: 220px;
  font-size: var(--fs-xs);
  line-height: 1.55;
}
.sc__messages {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.sc__msg {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  min-width: 0;
}
.sc__avatar {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 18px;
  height: 18px;
  margin-top: 2px;
  border-radius: var(--radius-pill);
  background: var(--bg-surface);
  border: 1px solid var(--border);
  color: var(--text-muted);
}
.sc__msg--assistant .sc__avatar {
  color: var(--accent);
}
.sc__bubble {
  min-width: 0;
  max-width: 100%;
  padding: 6px 9px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
}
.sc__msg--user .sc__bubble {
  background: color-mix(in srgb, var(--accent) 14%, var(--bg-surface));
  border-color: color-mix(in srgb, var(--accent) 32%, var(--border));
}
.sc__text {
  margin: 0;
  color: var(--text);
  font-size: var(--fs-xs);
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-word;
}
.sc__msg-failed {
  display: inline-block;
  margin-top: 3px;
  color: var(--danger);
  font-size: var(--fs-xs);
}

/* ── Streaming / error ──────────────────────────────────────────── */
.sc__streaming {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 2px 4px;
  color: var(--text-muted);
  font-size: var(--fs-xs);
}
.sc__dot {
  width: 6px;
  height: 6px;
  flex-shrink: 0;
  border-radius: var(--radius-pill);
  background: var(--accent);
  animation: sc-pulse var(--dur-pulse) var(--ease-standard) infinite;
}
@keyframes sc-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.25; }
}
.sc__tool {
  padding: 0 5px;
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  color: var(--accent);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sc__error {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 8px;
  border: 1px solid color-mix(in srgb, var(--danger) 38%, transparent);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--danger) 8%, transparent);
  color: var(--text);
  font-size: var(--fs-xs);
}
.sc__error-text {
  flex: 1;
  min-width: 0;
  word-break: break-word;
}
.sc__error-retry {
  flex-shrink: 0;
  padding: 2px 8px;
  border: 1px solid color-mix(in srgb, var(--danger) 45%, transparent);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  font-size: var(--fs-xs);
  cursor: pointer;
}
.sc__error-retry:hover {
  background: color-mix(in srgb, var(--danger) 14%, transparent);
}
.sc__error-retry:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}

/* ── Approvals ──────────────────────────────────────────────────── */
.sc__approvals {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}
.sc__banner {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 9px;
  border: 1px solid color-mix(in srgb, var(--warn) 40%, transparent);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--warn) 12%, transparent);
  color: var(--text);
  font-size: var(--fs-xs);
  line-height: 1.4;
}

/* ── Composer ───────────────────────────────────────────────────── */
.sc__composer {
  display: flex;
  align-items: flex-end;
  gap: 6px;
  padding-top: 2px;
  border-top: 1px solid var(--hairline);
}
.sc__input {
  flex: 1;
  min-width: 0;
  min-height: 34px;
  max-height: 120px;
  padding: 7px 9px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  color: var(--text);
  font: inherit;
  font-size: var(--fs-xs);
  line-height: 1.5;
  resize: none;
}
.sc__input::placeholder {
  color: var(--text-dim);
}
.sc__input:focus-visible {
  outline: none;
  border-color: var(--accent);
  box-shadow: var(--focus-ring);
}
.sc__send {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 30px;
  height: 30px;
  border: 0;
  border-radius: var(--radius-md);
  background: var(--accent);
  color: var(--accent-foreground);
  cursor: pointer;
  transition: background var(--dur-fast) var(--ease-standard), opacity var(--dur-fast) var(--ease-standard);
}
.sc__send:hover:not(:disabled) {
  background: var(--accent-hover);
}
.sc__send:disabled {
  opacity: 0.45;
  cursor: default;
}
.sc__send:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}

@media (prefers-reduced-motion: reduce) {
  .sc__dot { animation: none; }
  .sc__icon,
  .sc__chip,
  .sc__send { transition: none; }
}
</style>
