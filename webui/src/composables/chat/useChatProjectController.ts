// 项目 / 草稿 控制器（从 ChatView.vue 抽出的本地编排函数）。
//
// 这一批函数负责「选择项目路径、打开/关闭草稿、发送前校验活动项目、按路由
// 水合草稿、进入草稿」。它们横跨 ChatView 的两个生命周期区段（早期
// cancelActiveProjectValidation 与晚期的草稿水合块），且大量依赖已初始化的
// 组合式实例（chatSessionRoute / chatSessionSubscription / chatSessionRuntime /
// projectWorkspaces / activeProjectWorkspace / freshTaskDraft）与共享原语
// （isStreaming / isDesktopViewport / provisionalDraftUsed）。为避开声明顺序
// 耦合，本控制器在 ChatView 装配层「chatSessionRuntime 之后、validateActiveProjectBeforeSend
// 被解构进配置之前」统一接线；provisionalDraftUsed 以 setter 注入（enterDraft 复位，
// composer 发送路径读取，属跨批次共享可变 let）。draftProjectHydrationIsCurrent
// 仅被 syncDraftProjectFromRoute 调用，作为内部助手不导出。
import { type ComputedRef, type Ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useToasts } from '@/composables/useToasts'
import type { ChatViewState } from '../../views/chat/useChatViewState'
import { useRpcStore } from '@/stores/rpc'
import { useConfirm } from '@/composables/useConfirm'
import { useProjectWorkspaces } from '@/composables/useProjectWorkspaces'
import { useActiveProjectWorkspace } from '@/composables/useActiveProjectWorkspace'
import { useFreshTaskDraft } from '@/composables/useFreshTaskDraft'
import { createDraftProjectHydrationGuard } from '@/composables/useActiveProjectWorkspace'
import { useChatSessionRoute } from '@/composables/chat/useChatSessionRoute'
import { useChatSessionSubscription } from '@/composables/chat/useChatSessionSubscription'
import { useChatSessionRuntime } from '@/composables/chat/useChatSessionRuntime'
import { activeSnapshot } from '../../views/chat/active-snapshot'
import { agentIdFromSessionKey } from '@/utils/chat/sessionKeys'

/* ── 项目信任记忆 ─────────────────────────────────────────────────────
 *
 * 缺陷（真机反馈「每次打开都让我重新选择/确认」）：`chooseProjectPath` 每次都会
 * 弹 `workspaces.trustTitle` 确认框，但确认结果**从不落盘**，同一个项目下次仍要
 * 再确认一遍。信任是稳定的用户决策，应该记住。
 *
 * 纪律：
 *  · 只在**打开成功之后**记录（打开失败绝不写入）；
 *  · 上限 50 条，最近使用优先，避免 localStorage 无界增长；
 *  · Windows 路径大小写不敏感，键做小写归一（其它平台保持原样，避免误合并）。
 */
const TRUSTED_PROJECTS_KEY = 'marguerite.chat.trustedProjectPaths'
const MAX_TRUSTED_PROJECTS = 50
const IS_WINDOWS = typeof navigator !== 'undefined' && /^win/i.test(navigator.platform)

function projectTrustKey(path: string): string {
  const trimmed = path.trim().replace(/[\\/]+$/, '')
  return IS_WINDOWS ? trimmed.toLowerCase() : trimmed
}

/** 该路径是否已被用户信任过（纯读，坏数据按「未信任」处理）。 */
export function isProjectPathTrusted(path: string): boolean {
  const key = projectTrustKey(path)
  if (!key) return false
  try {
    const raw = JSON.parse(globalThis.localStorage?.getItem(TRUSTED_PROJECTS_KEY) || '[]')
    return Array.isArray(raw) && raw.includes(key)
  } catch {
    return false
  }
}

/** 记住一个成功打开的项目路径（LRU，最多 50 条）。 */
export function rememberTrustedProjectPath(path: string): void {
  const key = projectTrustKey(path)
  if (!key) return
  // 读旧值单独兜底：存储里是坏 JSON 时不能因此**丢掉这次**信任写入
  // （否则坏数据一旦写入，就永久退化成"每次都问"）。
  let rest: string[] = []
  try {
    const raw = JSON.parse(globalThis.localStorage?.getItem(TRUSTED_PROJECTS_KEY) || '[]')
    if (Array.isArray(raw)) {
      rest = raw.filter((item): item is string => typeof item === 'string' && item !== key)
    }
  } catch {
    rest = []
  }
  try {
    globalThis.localStorage?.setItem(
      TRUSTED_PROJECTS_KEY,
      JSON.stringify([key, ...rest].slice(0, MAX_TRUSTED_PROJECTS)),
    )
  } catch {
    // 写不进去就退化成「每次都确认」，绝不阻断打开。
  }
}

export interface ChatProjectControllerDeps {
  rpc: ReturnType<typeof useRpcStore>
  confirm: ReturnType<typeof useConfirm>['confirm']
  projectWorkspaces: ReturnType<typeof useProjectWorkspaces>
  activeProjectWorkspace: ReturnType<typeof useActiveProjectWorkspace>
  freshTaskDraft: ReturnType<typeof useFreshTaskDraft>
  draftProjectHydration: ReturnType<typeof createDraftProjectHydrationGuard>
  chatSessionRoute: ReturnType<typeof useChatSessionRoute>
  chatSessionSubscription: ReturnType<typeof useChatSessionSubscription>
  chatSessionRuntime: ReturnType<typeof useChatSessionRuntime>
  isStreaming: Readonly<Ref<boolean>> | Readonly<ComputedRef<boolean>>
  isDesktopViewport: Readonly<Ref<boolean>>
  /** 复位 ChatView 的 provisionalDraftUsed（enterDraft 用；composer 发送路径读取）。 */
  setProvisionalDraftUsed: (value: boolean) => void
}

export function useChatProjectController(
  state: ChatViewState,
  deps: ChatProjectControllerDeps,
) {
  const { t } = useI18n()
  const { pushToast } = useToasts()
  const {
    projectPickerOpen,
    landingPrefilled,
    pendingAutoSend,
    pendingAutoSendSessionKey,
    sessionKey,
    inputText,
    messages,
    composerRef,
    pendingSessionIntent,
  } = state
  const {
    rpc,
    confirm,
    projectWorkspaces,
    activeProjectWorkspace,
    freshTaskDraft,
    draftProjectHydration,
    chatSessionRoute,
    chatSessionSubscription,
    chatSessionRuntime,
    isStreaming,
    isDesktopViewport,
    setProvisionalDraftUsed,
  } = deps

  const {
    draftAgentId,
    goToDraft,
    isDraftRoute,
    readProjectFromUrl,
  } = chatSessionRoute
  const { retrySessionMetadata } = chatSessionSubscription
  const { startDraftSession } = chatSessionRuntime
  const { boundWorkspaceId, status: activeWorkspaceStatus, sendBlockedReason: activeWorkspaceSendBlockedReason } = activeProjectWorkspace

  // 仅被本控制器内的 validateActiveProjectBeforeSend / cancelActiveProjectValidation
  // 使用，随控制器迁移（不再留在 ChatView 顶层 let）。
  let activeProjectValidationController: AbortController | null = null

  /* ── Project picker ──────────────────────────────────────────────── */

  function cancelActiveProjectValidation() {
    activeProjectValidationController?.abort()
    activeProjectValidationController = null
  }

  function openProjectPicker() {
    if (!rpc.canChooseProject) return
    projectPickerOpen.value = true
  }

  async function chooseProjectPath(path: string) {
    projectPickerOpen.value = false
    if (!rpc.canChooseProject) return
    // 已信任的路径不再重复询问（首次仍会弹，见模块顶部的信任记忆说明）。
    const alreadyTrusted = isProjectPathTrusted(path)
    if (!alreadyTrusted) {
      const trusted = await confirm({
        title: t('workspaces.trustTitle'),
        body: t('workspaces.trustBody', { path }),
        primaryLabel: t('workspaces.trustConfirm'),
        primaryClass: 'btn--primary',
      })
      if (!trusted) return
    }
    try {
      const workspace = await projectWorkspaces.openWorkspace(path)
      if (!workspace) return
      // 打开成功才记住：失败/取消不写入任何信任记录。
      rememberTrustedProjectPath(path)
      freshTaskDraft.requestFreshTask(draftAgentId(), workspace.id)
      goToDraft({
        agentId: draftAgentId(),
        projectId: workspace.id,
        replace: true,
      })
    } catch (cause) {
      const detail = cause instanceof Error ? cause.message : String(cause)
      pushToast(t('workspaces.openFailed', { error: detail }), { tone: 'warn' })
    }
  }

  function closeProjectDraft() {
    activeProjectWorkspace.clearDraft()
    freshTaskDraft.requestFreshTask(draftAgentId())
    goToDraft({
      agentId: draftAgentId(),
      projectId: null,
      replace: true,
    })
  }

  /* ── Pre-send validation ────────────────────────────────────────── */

  async function validateActiveProjectBeforeSend(): Promise<string | null> {
    const key = sessionKey.value
    const deadlineAt = Date.now() + 7_000
    cancelActiveProjectValidation()
    const controller = new AbortController()
    activeProjectValidationController = controller
    let workspaceId = boundWorkspaceId.value
    try {
      if (
        !workspaceId
        && activeWorkspaceStatus.value === 'error'
      ) {
        const recovered = await retrySessionMetadata({
          timeoutMs: Math.max(1, deadlineAt - Date.now()),
          signal: controller.signal,
          timeoutAction: 'reconnect',
          abortAction: 'reconnect',
        })
        if (!recovered) {
          if (!controller.signal.aborted && sessionKey.value === key) {
            pushToast(t('workspaces.activeProjectBlocksSending'), { tone: 'warn' })
          }
          return activeWorkspaceSendBlockedReason.value || 'error'
        }
        workspaceId = boundWorkspaceId.value
      }
      if (!workspaceId) return activeWorkspaceSendBlockedReason.value
      if (!rpc.canManageProjectWorkspaces) {
        return activeWorkspaceSendBlockedReason.value
      }
      const workspaces = await projectWorkspaces.loadWorkspaces({
        timeoutMs: Math.max(1, deadlineAt - Date.now()),
        signal: controller.signal,
        timeoutAction: 'reconnect',
        abortAction: 'reconnect',
      })
      if (sessionKey.value !== key || boundWorkspaceId.value !== workspaceId) {
        return activeWorkspaceSendBlockedReason.value || 'resolving'
      }
      const workspace = workspaces.find(item => item.id === workspaceId) || null
      activeProjectWorkspace.applyWorkspaceRefresh(
        workspace ? activeSnapshot(workspace) : null,
      )
    } catch {
      if (
        !controller.signal.aborted
        && sessionKey.value === key
        && boundWorkspaceId.value === workspaceId
      ) {
        activeProjectWorkspace.failWorkspaceRefresh()
      }
    } finally {
      if (activeProjectValidationController === controller) {
        activeProjectValidationController = null
      }
    }
    return activeWorkspaceSendBlockedReason.value
  }

  /* ── Draft hydration ────────────────────────────────────────────── */

  // One-shot composer prefill carried in history state (the Sessions Hub task
  // input navigates here with it). Consumed on draft entry so reload or
  // back/forward does not re-apply the text.
  function consumeDraftPrefill() {
    const historyState = window.history.state as Record<string, unknown> | null
    const prefill = typeof historyState?.prefill === 'string' ? historyState.prefill : ''
    if (!prefill) return
    inputText.value = prefill
    landingPrefilled.value = true
    // A Sessions Hub "Start task" hand-off also asks the draft to send the
    // prefill in one step; the actual flush waits for the subscription in onMounted.
    if (historyState?.autosend === true) {
      pendingAutoSend.value = prefill
      pendingAutoSendSessionKey.value = sessionKey.value
    }
    try {
      window.history.replaceState({ ...window.history.state, prefill: undefined, autosend: undefined }, '')
    } catch { /* ignore */ }
  }

  function draftProjectHydrationIsCurrent(
    generation: number,
    workspaceId: string | null,
  ): boolean {
    return draftProjectHydration.isCurrent(generation)
      && isDraftRoute()
      && readProjectFromUrl() === workspaceId
  }

  async function syncDraftProjectFromRoute(generation: number): Promise<boolean> {
    const deadlineAt = Date.now() + 7_000
    const workspaceId = readProjectFromUrl()
    if (!draftProjectHydrationIsCurrent(generation, workspaceId)) return false
    if (!workspaceId) {
      activeProjectWorkspace.clearDraft()
      return true
    }
    if (!rpc.canChooseProject) {
      activeProjectWorkspace.clearDraft()
      freshTaskDraft.requestFreshTask(draftAgentId())
      goToDraft({
        agentId: draftAgentId(),
        projectId: null,
        replace: true,
      })
      return true
    }
    const cached = projectWorkspaces.byId.value.get(workspaceId)
    if (cached) {
      activeProjectWorkspace.beginProjectDraft(activeSnapshot(cached))
      return true
    }
    activeProjectWorkspace.beginUnknownProjectDraft(workspaceId)
    const controller = draftProjectHydration.createController(generation)
    if (!controller) return false
    try {
      await rpc.waitForConnection(
        Math.max(1, deadlineAt - Date.now()),
        controller.signal,
        { timeoutAction: 'reconnect', abortAction: 'reconnect' },
      )
      if (!draftProjectHydrationIsCurrent(generation, workspaceId)) return false
      await projectWorkspaces.loadWorkspaces({
        timeoutMs: Math.max(1, deadlineAt - Date.now()),
        signal: controller.signal,
        timeoutAction: 'reconnect',
        abortAction: 'reconnect',
      })
      if (!draftProjectHydrationIsCurrent(generation, workspaceId)) return false
      const workspace = projectWorkspaces.byId.value.get(workspaceId)
      if (workspace) {
        activeProjectWorkspace.beginProjectDraft(activeSnapshot(workspace))
      } else {
        activeProjectWorkspace.beginUnknownProjectDraft(workspaceId)
      }
    } catch (cause) {
      if (!draftProjectHydrationIsCurrent(generation, workspaceId)) return false
      activeProjectWorkspace.failWorkspaceRefresh()
      const detail = cause instanceof Error ? cause.message : String(cause)
      pushToast(t('workspaces.loadFailed', { error: detail }), { tone: 'warn' })
    } finally {
      draftProjectHydration.complete(generation, controller)
    }
    return true
  }

  // Reset to a clean draft for the agent requested by the draft route. The
  // provisional key stays out of the URL and storage until the first send.
  function enterDraft() {
    landingPrefilled.value = false
    setProvisionalDraftUsed(false)
    const agentId = draftAgentId()
    const isFreshDraft = pendingSessionIntent.value === 'new_chat'
      && messages.value.length === 0
      && !isStreaming.value
      && agentIdFromSessionKey(sessionKey.value) === agentId
    if (!isFreshDraft) startDraftSession(agentId)
    consumeDraftPrefill()
    if (isDesktopViewport.value) composerRef.value?.focusTextarea()
  }

  return {
    cancelActiveProjectValidation,
    openProjectPicker,
    chooseProjectPath,
    closeProjectDraft,
    validateActiveProjectBeforeSend,
    consumeDraftPrefill,
    syncDraftProjectFromRoute,
    enterDraft,
  }
}
