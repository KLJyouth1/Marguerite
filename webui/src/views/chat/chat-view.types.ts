// ChatView.vue 本地类型定义。
// 从 views/ChatView.vue 的 `/* ── Types ── */` 段抽出，保持零运行时行为、零响应式耦合。
import type { ChatMessage } from '@/types/chat'
import type { RunModePolicy } from '@/composables/chat/useChatRunModePreference'

/** ChatComposer 暴露给 ChatView 的命令式句柄。 */
export interface ChatComposerHandle {
  composerElement: () => HTMLElement | null
  canCollapse: () => boolean
  focusTextarea: () => void
  isTextareaFocused: () => boolean
  resizeTextarea: () => void
}

/** 视图内统一的消息类型别名。 */
export type Message = ChatMessage

/** 发送给运行模式策略的鉴权载荷。 */
export interface RpcAuthPayload {
  runModePolicy?: RunModePolicy
}

/** ChatHeaderActions 暴露给 ChatView 的命令式句柄。 */
export interface ChatHeaderActionsHandle {
  focusAction: (action: 'deliverables' | 'runs' | 'share' | 'copy-session-key') => boolean
}
