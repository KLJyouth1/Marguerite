// MetaSkill 提供商设置导航控制器（从 ChatView.vue 抽出的本地编排函数）。
// 协调 metaSetupProviderNavigationPending 共享状态与外部导航工具、setup 状态及会话持久化。
import { type Ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter, type RouteLocationNormalizedLoaded } from 'vue-router'
import { useToasts } from '@/composables/useToasts'
import { navigateMetaSetupProviderSettings } from '@/composables/chat/metaSetupProviderNavigation'
import type { ChatViewState } from '../../views/chat/useChatViewState'

export interface ChatMetaControllerDeps {
  /** useMetaSkillSetup 暴露的 setup 状态（仅取 sessionKey）。 */
  setupState: Readonly<Ref<{ sessionKey?: string | null } | null>>
  beginProviderHandoff: (providerId: string) => string
  cancelProviderHandoff: (providerId: string, clientRequestId: string) => void
  persistSession: (
    key: string,
    opts?: { updateRoute?: boolean; source?: string },
  ) => void
  route: RouteLocationNormalizedLoaded
}

export function useChatMetaController(
  state: ChatViewState,
  deps: ChatMetaControllerDeps,
) {
  const { t } = useI18n()
  const router = useRouter()
  const { pushToast } = useToasts()
  const { metaSetupProviderNavigationPending } = state

  async function openMetaSetupProviderSettings(providerId: string) {
    if (metaSetupProviderNavigationPending.value) return
    metaSetupProviderNavigationPending.value = true
    try {
      const opened = await navigateMetaSetupProviderSettings({
        providerId,
        sessionKey: deps.setupState.value?.sessionKey || '',
        currentRouteSession: deps.route.query.session,
        router,
        beginHandoff: deps.beginProviderHandoff,
        cancelHandoff: deps.cancelProviderHandoff,
        materializeSession: (handoffSessionKey) => {
          deps.persistSession(handoffSessionKey, {
            updateRoute: false,
            source: 'chatView.metaSetupProviderHandoff',
          })
        },
      })
      if (!opened) {
        pushToast(t('chat.metaSetup.providerNavigationFailed'), { tone: 'danger' })
      }
    } finally {
      metaSetupProviderNavigationPending.value = false
    }
  }

  return { openMetaSetupProviderSettings }
}
