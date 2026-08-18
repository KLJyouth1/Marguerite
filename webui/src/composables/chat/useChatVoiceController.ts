// 语音输入控制器（从 ChatView.vue 抽出的本地编排函数）。
// 仅依赖 useVoiceInput 暴露的 toggle 与 composer 文本回填，无跨控制器耦合。
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { useToasts } from '@/composables/useToasts'
import type { ChatViewState } from '../../views/chat/useChatViewState'

export interface ChatVoiceControllerDeps {
  /** useVoiceInput 暴露的麦克风开关；参数为 composer 文本回填回调。 */
  toggleVoiceInput: (append: (text: string) => void) => void | Promise<void>
  /** composer 文本追加（来自 useChatComposerController）。 */
  appendComposerText: (text: string) => void
}

export function useChatVoiceController(
  _state: ChatViewState,
  deps: ChatVoiceControllerDeps,
) {
  const { t } = useI18n()
  const router = useRouter()
  const { pushToast } = useToasts()

  function onVoiceInput() {
    void deps.toggleVoiceInput(deps.appendComposerText)
  }

  // When voice isn't configured the mic button routes here instead of recording:
  // tell the user what's missing and take them straight to the audio settings.
  function onVoiceSetup() {
    pushToast(t('chat.toast.voiceSetupNeeded'), { tone: 'info' })
    router.push('/settings/capabilities').catch(() => {})
  }

  return { onVoiceInput, onVoiceSetup }
}
