// 工具结果弹窗控制器（从 ChatView.vue 抽出的本地编排函数）。
// 仅依赖共享状态中的 toolResultModal 字段与 i18n。
import { useI18n } from 'vue-i18n'
import type { ChatViewState } from '../../views/chat/useChatViewState'
import type { ToolResultContext } from '@/types/chat'

export function useChatToolResultController(state: ChatViewState) {
  const { t } = useI18n()
  const { toolResultModal } = state

  function showToolResultModal(
    content: string,
    title = t('chat.toolResult'),
    context?: ToolResultContext,
  ) {
    toolResultModal.value = { open: true, title, content, context }
  }

  return { showToolResultModal }
}
