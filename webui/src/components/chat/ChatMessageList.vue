<template>
  <template v-for="(message, index) in messages" :key="chatMessageKey(message, index)">
    <!-- 单一阅读列：历史水合与实时气泡共用同一行容器。子组件不再各自用
         `margin: 0 auto` 居中 —— 宽面板上 --chat-col 有上限，auto 外边距会把
         整段对话缩到屏幕中间（任务结束后尤其明显）。 -->
    <div
      class="chat-turn-row"
      :data-chat-turn-row="chatMessageKey(message, index)"
    >
      <slot
        v-if="message.isRouterStrip"
        name="router-strip"
        :message="message"
        :index="index"
      />
      <UserMessage
        v-else-if="message.displayRole === 'user'"
        :id="`chat-turn-${index}`"
        :data-chat-turn-key="chatMessageKey(message, index)"
        tabindex="-1"
        :message="message"
        :share-mode="shareMode"
        :share-selected="selectedMessageIds.has(chatMessageKey(message, index))"
        :share-message-id="chatMessageKey(message, index)"
        :strip-time-prefix="stripTimePrefix"
        :copy-message="copyMessage"
        :download-attachment="downloadAttachment"
        :show-turn-outcome="isTurnTip(index)"
        :is-streaming="isStreaming"
        @edit="$emit('editMessage', $event)"
        @toggle-share="$emit('toggleShareMessage', $event)"
      />
      <CompactionEvent
        v-else-if="message.displayRole === 'maintenance' && message.maintenance?.kind === 'context_compaction'"
        :message="message"
      />
      <AssistantMessage
        v-else-if="message.displayRole === 'assistant'"
        :message="message"
        :index="index"
        :share-mode="shareMode"
        :share-selected="selectedMessageIds.has(chatMessageKey(message, index))"
        :share-message-id="chatMessageKey(message, index)"
        :render-markdown="renderMarkdown"
        :fmt-tok="fmtTok"
        :tool-call-groups="toolCallGroups"
        :is-tool-group-open="isToolGroupOpen"
        :is-tool-item-open="isToolItemOpen"
        :tool-group-status-text="toolGroupStatusText"
        :tool-status-text="toolStatusText"
        :tool-secondary-text="toolSecondaryText"
        :session-key="sessionKey"
        :auth-token="authToken"
        :workbench-enabled="workbenchEnabled"
        :artifact-navigation-items="artifactNavigationItems"
        :copy-message="copyMessage"
        :is-tip="isForkableAssistant(index)"
        :fork-busy="forkBusy"
        :plan-action-pending="planActionPending"
        :plan-actions-disabled="planActionsDisabled"
        :show-turn-outcome="isTurnTip(index)"
        :turn-started-at="turnStartTs(index)"
        @fork="$emit('forkConversation', forkThroughTurnId(index))"
        @regenerate="$emit('regenerateMessage', $event)"
        @toggle-share="$emit('toggleShareMessage', $event)"
        @download-artifact="$emit('downloadArtifact', $event)"
        @open-artifact="$emit('openArtifact', $event)"
        @toggle-tool-group="$emit('toggleToolGroup', $event)"
        @toggle-tool-item="$emit('toggleToolItem', $event)"
        @show-tool-result="(content, title, context) => $emit('showToolResult', content, title, context)"
        @resolve-interrupt="(id, decision) => $emit('resolveInterrupt', id, decision)"
        @extend-interrupt="id => $emit('extendInterrupt', id)"
        @clarify-submit="(fields, request) => $emit('clarifySubmit', fields, request)"
        @clarify-dismiss="$emit('clarifyDismiss')"
        @plan-implement-current="$emit('planImplementCurrent', $event)"
        @plan-implement-new="$emit('planImplementNew', $event)"
        @plan-replan="$emit('planReplan', $event)"
        @plan-reuse-last="$emit('planReuseLast', $event)"
        @show-full-message="$emit('showFullMessage', $event)"
      />
      <SystemMessage
        v-else
        :message="message"
        :can-replan="canReplan"
        @resume="$emit('resumeSandbox')"
        @open-settings="$emit('openAuthSettings')"
        @retry="$emit('retryAuthFailure')"
        @replan="$emit('replanFailedTurn')"
        @view-logs="$emit('viewTurnLogs')"
        @focus-subagent="$emit('focusSubagent', $event)"
      />
    </div>
  </template>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import AssistantMessage from '@/components/chat/AssistantMessage.vue'
import CompactionEvent from '@/components/chat/CompactionEvent.vue'
import SystemMessage from '@/components/chat/SystemMessage.vue'
import UserMessage from '@/components/chat/UserMessage.vue'
import type {
  ChatRenderedMessage,
  ChatToolCall,
  ChatToolCallGroup,
  ChatToolCallRenderItem,
  ToolResultContext,
} from '@/types/chat'
import type { ArtifactPayload } from '@/types/rpc'
import type { PlanCardAction, PlanCardActionTarget } from '@/types/plans'
import { chatMessageKey } from '@/utils/chat/messageIdentity'

const props = defineProps<{
  messages: ChatRenderedMessage[]
  shareMode: boolean
  selectedMessageIds: Set<string>
  stripTimePrefix: (text: string) => string
  renderMarkdown: (text: string) => string
  fmtTok: (value: number) => string
  toolCallGroups: (calls: ChatToolCall[], baseKey: string) => ChatToolCallGroup[]
  isToolGroupOpen: (groupId: string) => boolean
  isToolItemOpen: (renderKey: string) => boolean
  toolGroupStatusText: (group: ChatToolCallGroup) => string
  toolStatusText: (call: ChatToolCallRenderItem) => string
  toolSecondaryText: (call: ChatToolCallRenderItem) => string
  copyMessage: (message: ChatRenderedMessage) => Promise<boolean>
  downloadAttachment: (attachment: import('@/types/chat').DisplayAttachment) => Promise<boolean>
  artifactNavigationItems?: ArtifactPayload[]
  sessionKey?: string
  authToken?: string
  workbenchEnabled?: boolean
  forkBusy?: boolean
  planActionPending?: PlanCardAction | null
  planActionsDisabled?: boolean
  /** B2：会话里存在可修订方案卡时，失败轮次的卡片提供「改计划」出口。 */
  canReplan?: boolean
  isStreaming?: boolean
}>()

defineEmits<{
  editMessage: [message: ChatRenderedMessage]
  regenerateMessage: [message: ChatRenderedMessage]
  toggleShareMessage: [messageId: string]
  downloadArtifact: [artifact: ArtifactPayload]
  openArtifact: [artifact: ArtifactPayload]
  toggleToolGroup: [groupId: string]
  toggleToolItem: [renderKey: string]
  showToolResult: [content: string, title: string, context?: ToolResultContext]
  showFullMessage: [message: ChatRenderedMessage]
  forkConversation: [throughTurnId?: string]
  resolveInterrupt: [id: string, decision: 'allow-once' | 'allow-always' | 'deny']
  extendInterrupt: [id: string]
  clarifySubmit: [fields: Record<string, string>, request?: NonNullable<Extract<import('@/types/parts').ChatPart, { type: 'interrupt' }>['clarify']>]
  clarifyDismiss: []
  resumeSandbox: []
  openAuthSettings: []
  retryAuthFailure: []
  /** B2：失败轮次的次级出口 —— 改计划 / 看日志。 */
  replanFailedTurn: []
  viewTurnLogs: []
  focusSubagent: [payload: { key: string }]
  planImplementCurrent: [target: PlanCardActionTarget]
  planImplementNew: [target: PlanCardActionTarget]
  planReplan: [target: PlanCardActionTarget]
  /** D-05/T08：复用上次计划。 */
  planReuseLast: [target: PlanCardActionTarget]
}>()

// Legacy transcripts can only use the whole-conversation fallback at the
// current tip. Historical branches require a durable terminal turn identity so
// the server, rather than a DOM/message index, owns the inclusive boundary.
const lastAssistantIndex = computed(() => {
  for (let i = props.messages.length - 1; i >= 0; i--) {
    if (props.messages[i].displayRole === 'assistant' && !props.messages[i].stopNotice) return i
  }
  return -1
})

function forkThroughTurnId(index: number): string | undefined {
  const turnId = props.messages[index]?.turnOutcome?.turnId?.trim()
  return turnId || undefined
}

function isForkableAssistant(index: number): boolean {
  const message = props.messages[index]
  if (
    props.isStreaming
    || message?.displayRole !== 'assistant'
    || message.stopNotice
  ) return false
  if (forkThroughTurnId(index)) return isTurnTip(index)
  if (index !== lastAssistantIndex.value) return false
  return !props.messages.slice(index + 1).some(next => (
    next.displayRole === 'user' || next.displayRole === 'assistant'
  ))
}

function isTurnTip(index: number): boolean {
  const message = props.messages[index]
  if (!message?.turnOutcome || !message.turnKey) return false
  for (let nextIndex = index + 1; nextIndex < props.messages.length; nextIndex++) {
    const next = props.messages[nextIndex]
    if (next.turnKey === message.turnKey) {
      if (next.displayRole === 'user' || next.displayRole === 'assistant') return false
      continue
    }
    if (next.displayRole === 'user') break
  }
  return true
}

/**
 * 本回合的起始时间：向前找最近一条用户消息的时间戳。
 * 交付回执用它作为证据窗口左界——没有它就退回整会话聚合（后端会标 degraded）。
 */
function turnStartTs(index: number): string | number | null {
  for (let i = index; i >= 0; i--) {
    const candidate = props.messages[i]
    if (candidate?.displayRole === 'user') return candidate.ts ?? null
  }
  return null
}
</script>

<style>
/* ── 会话阅读列（唯一定义）─────────────────────────────────────────────
   历史水合（本组件包裹的 .chat-turn-row）与实时气泡（ChatView 直接渲染的
   .msg-ai）必须共享同一个左对齐列：
   · 宽度上限来自 `--chat-col`，但绝不再用 `margin: 0 auto` 居中；
   · 左侧固定 24px 起排，宽屏多出来的空间留在右侧。
   根因行是 AssistantMessage 的 `.msg-ai { width: var(--chat-col); margin: 0 auto }`
   —— 宽面板上上限列被两侧 auto 外边距居中，整段历史看起来缩在屏幕中间。 */
.chat .chat-thread > .msg-ai:not(.msg-ai--share-mode),
.chat .chat-thread > .msg-user:not(.msg-user--share-mode),
.chat .chat-turn-row > .msg-ai:not(.msg-ai--share-mode),
.chat .chat-turn-row > .msg-user:not(.msg-user--share-mode) {
  width: min(calc(100% - 48px), var(--chat-col, 980px));
  max-width: calc(100% - 48px);
  margin-inline: 24px auto;
  text-align: start;
}

.chat-turn-row {
  display: flex;
  flex-direction: column;
  width: 100%;
  min-width: 0;
}
</style>
