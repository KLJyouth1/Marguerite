// 运行模式 / 沙箱设置 / 计划动作 控制器（从 ChatView.vue 抽出的本地编排函数）。
//
// 这一批函数原本散落在 ChatView 的「发送按钮区」与「Helpers 区」：计划实施 /
// 修订 / 取消、协作模式切换、运行模式持久化、沙箱安全设置的开 / 关 / 确认、
// 模型路由模式与编码模式开关。它们大多是对 chatPlans / sandboxSetupStore /
// sandboxSetupRecovery 等已初始化实例的薄封装，或带本地守卫（发送阻塞、
// 附件存在、运行模式锁定）后调用副作用函数。这里统一收口为一组显式导出，
// ChatView 仅保留模板仍需的派生 ref（composerSafeSetupAvailable /
// sandboxSetupStatus / runModeLocked / sandboxSetupPending）与控制器接线。
//
// 关键约定：reportRunModePersistenceError / persistComposerRunMode 是
// setComposerRunMode 的内部助手，仅被本控制器调用，不导出回 ChatView
// （避免 noUnusedLocals）。其余 14 个函数被模板或 ChatView 其它逻辑引用，
// 由 ChatView 解构后复用。
import { computed, type ComputedRef } from 'vue'
import { storeToRefs } from 'pinia'
import { useI18n } from 'vue-i18n'
import { useToasts } from '@/composables/useToasts'
import type { ChatViewState } from '../../views/chat/useChatViewState'
import { useChatPlans } from '@/composables/chat/useChatPlans'
import { useSandboxSetupStore } from '@/stores/sandboxSetup'
import { useSandboxSetupRecovery } from '@/composables/chat/useSandboxSetupRecovery'
import { useChatAttachments } from '@/composables/chat/useChatAttachments'
import { composerRunModeSelectionAction } from '@/composables/chat/composerRunMode'
import type { SandboxRunMode } from '@/types/sandbox'
import type { ModelRoutingMode } from '@/types/modelRouting'
import type { CollaborationMode, PlanCardActionTarget } from '@/types/plans'

export interface ChatRunModeControllerDeps {
  /** 计划编排实例（implement / beginReplan / cancelRun / setMode 等）。 */
  chatPlans: ReturnType<typeof useChatPlans>
  /** 沙箱安全设置 store（noteRunModeSelection / resetOutcome / startSafeSetup / ensuring）。 */
  sandboxSetupStore: ReturnType<typeof useSandboxSetupStore>
  /** 沙箱设置恢复实例（status / canSetup / resolved / refresh）。 */
  sandboxSetupRecovery: ReturnType<typeof useSandboxSetupRecovery>
  /** ChatView 的本地发送中止函数（onComposerStop 的 fallback 分支）。 */
  onStop: () => void
  /** 待发送附件（beginPlanRevision 守卫：有附件时不允许进入 revise）。 */
  pendingAttachments: ReturnType<typeof useChatAttachments>['pendingAttachments']
  /** 发送被会话态阻塞的原因（implement* 守卫）。 */
  liveSendBlockedReason: Readonly<ComputedRef<string | null>>
  /** 运行模式锁定态（setComposerRunMode 守卫）。 */
  runModeLocked: Readonly<ComputedRef<boolean>>
  /** 持久化全局沙箱运行模式（返回被持久化后的模式，本控制器不使用该返回值）。 */
  setGlobalRunMode: (mode: SandboxRunMode) => Promise<unknown>
  /** 设置模型路由模式（写后触发历史同步）。 */
  setModelRoutingMode: (mode: ModelRoutingMode) => Promise<void>
  /** 开关编码模式（返回是否更新成功）。 */
  setCodingModeEnabled: (enabled: boolean) => Promise<boolean>
  /** 计划变更后触发历史同步。 */
  scheduleHistorySync: () => void
  /** 重新水合全局运行模式偏好（confirmComposerSandboxSetup 用）。 */
  refreshRunModePreference: () => Promise<void>
}

export function useChatRunModeController(
  state: ChatViewState,
  deps: ChatRunModeControllerDeps,
) {
  const { t } = useI18n()
  const { pushToast } = useToasts()
  const { composerRef, executionDockRun, composerSandboxSetupOpen } = state
  const {
    chatPlans,
    sandboxSetupStore,
    sandboxSetupRecovery,
    onStop,
    pendingAttachments,
    liveSendBlockedReason,
    runModeLocked,
    setGlobalRunMode,
    setModelRoutingMode,
    setCodingModeEnabled,
    scheduleHistorySync,
    refreshRunModePreference,
  } = deps

  // 这些派生量原本是 ChatView 内的 computed/ref；控制器从已传入的 store /
  // recovery 实例就地派生，保持与 ChatView 模板绑定同源。
  const composerSafeSetupAvailable = computed(() => sandboxSetupRecovery.canSetup.value)
  const sandboxSetupStatus = sandboxSetupRecovery.status
  const sandboxSetupPending = storeToRefs(sandboxSetupStore).ensuring

  /* ── Plan actions ─────────────────────────────────────────────────── */

  function implementCurrentPlan(target: PlanCardActionTarget) {
    if (liveSendBlockedReason.value) return
    void chatPlans.implement(target, false)
  }

  function implementPlanInNewTask(target: PlanCardActionTarget) {
    if (liveSendBlockedReason.value) return
    void chatPlans.implement(target, true)
  }

  /**
   * D-05/T08「继续上次计划」：`target.revisionId` 是复用候选 revision，
   * 直接复用既有实施入口（`plans.implement`）在其上启动——**复用而非新建**。
   */
  function reuseLastPlan(target: PlanCardActionTarget) {
    if (liveSendBlockedReason.value) return
    void chatPlans.implement(target, false)
  }

  function beginPlanRevision(target: PlanCardActionTarget) {
    if (pendingAttachments.value.length > 0) {
      pushToast(t('chat.plan.attachmentsUnavailable'), { tone: 'warn' })
      return
    }
    chatPlans.beginReplan(target)
  }

  function cancelPlanRevision() {
    chatPlans.cancelReplan()
  }

  function cancelActivePlanRun() {
    void chatPlans.cancelRun()
  }

  function focusComposerAfterPlanRun() {
    composerRef.value?.focusTextarea()
  }

  function onComposerStop() {
    const run = executionDockRun.value
    if (run && (run.status === 'queued' || run.status === 'running')) {
      void chatPlans.cancelRun()
      return
    }
    onStop()
  }

  function setCollaborationMode(mode: CollaborationMode) {
    void chatPlans.setMode(mode)
  }

  /* ── Run mode persistence ────────────────────────────────────────── */

  function reportRunModePersistenceError(cause: unknown): void {
    const detail = cause instanceof Error ? cause.message : String(cause)
    console.warn('Failed to persist sandbox run mode:', detail)
    pushToast(detail, { tone: 'danger' })
  }

  async function persistComposerRunMode(mode: SandboxRunMode): Promise<void> {
    await setGlobalRunMode(mode)
    void sandboxSetupRecovery.refresh()
  }

  async function setComposerRunMode(mode: SandboxRunMode): Promise<void> {
    if (runModeLocked.value) return
    sandboxSetupStore.noteRunModeSelection(mode)
    const action = composerRunModeSelectionAction(
      mode,
      sandboxSetupStatus.value,
      composerSafeSetupAvailable.value,
      sandboxSetupRecovery.resolved.value,
    )
    if (action === 'ignore') return
    if (action === 'setup') {
      sandboxSetupStore.resetOutcome()
      composerSandboxSetupOpen.value = true
      return
    }
    try {
      await persistComposerRunMode(mode)
    } catch (cause) {
      reportRunModePersistenceError(cause)
    }
  }

  function cancelComposerSandboxSetup(): void {
    if (sandboxSetupPending.value) return
    composerSandboxSetupOpen.value = false
  }

  async function confirmComposerSandboxSetup(): Promise<void> {
    if (sandboxSetupPending.value) return
    const ready = await sandboxSetupStore.startSafeSetup()
    if (ready) {
      composerSandboxSetupOpen.value = false
      await refreshRunModePreference()
      await sandboxSetupRecovery.refresh()
    }
  }

  function runComposerSandboxSetupInBackground(): void {
    composerSandboxSetupOpen.value = false
  }

  async function setComposerModelRoutingMode(mode: ModelRoutingMode) {
    await setModelRoutingMode(mode)
    scheduleHistorySync()
  }

  async function setComposerCodingModeEnabled(enabled: boolean) {
    const updated = await setCodingModeEnabled(enabled)
    pushToast(t(
      updated
        ? (enabled ? 'chat.codingMode.enabled' : 'chat.codingMode.disabled')
        : 'chat.codingMode.updateFailed',
    ))
  }

  return {
    implementCurrentPlan,
    implementPlanInNewTask,
    reuseLastPlan,
    beginPlanRevision,
    cancelPlanRevision,
    cancelActivePlanRun,
    focusComposerAfterPlanRun,
    onComposerStop,
    setCollaborationMode,
    reportRunModePersistenceError,
    persistComposerRunMode,
    setComposerRunMode,
    cancelComposerSandboxSetup,
    confirmComposerSandboxSetup,
    runComposerSandboxSetupInBackground,
    setComposerModelRoutingMode,
    setComposerCodingModeEnabled,
  }
}
