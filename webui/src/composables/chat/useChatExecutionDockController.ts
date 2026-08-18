// 执行坞（plan-run dock）控制器（从 ChatView.vue 抽出的本地编排函数）。
//
// 这一批函数负责「把当前 activePlanRun 投影到 executionDockRun（模板里的
// plan-run-dock 数据源），并用一个定时器在 run 进入终态后延时收起坞」。跨 watch
// 生命周期，但耦合面窄（只依赖 executionDockRun 共享 ref、chatPlans.activePlanRun，
// 以及一个仅在两函数间共享的私有定时器），风险低，故整体迁入本控制器。
// 定时器私有变量 executionDockHideTimer 随控制器迁移（不再留在 ChatView 顶层 let）。
import { type Ref } from 'vue'
import type { ChatViewState } from '../../views/chat/useChatViewState'
import type { PlanRunSnapshot } from '@/types/plans'

export interface ChatExecutionDockControllerDeps {
  /** 来自 useChatPlans；当前执行的方案 run（模板 plan-run-dock 的数据来源）。 */
  activePlanRun: Readonly<Ref<PlanRunSnapshot | null>>
}

const PLAN_RUN_TERMINAL_HOLD_MS = 2000

export function useChatExecutionDockController(
  state: ChatViewState,
  deps: ChatExecutionDockControllerDeps,
) {
  const { executionDockRun } = state
  const { activePlanRun } = deps

  let executionDockHideTimer: ReturnType<typeof setTimeout> | null = null

  function clearExecutionDockHideTimer() {
    if (executionDockHideTimer === null) return
    clearTimeout(executionDockHideTimer)
    executionDockHideTimer = null
  }

  function syncExecutionDockRun() {
    const run = activePlanRun.value
    clearExecutionDockHideTimer()
    if (!run) {
      executionDockRun.value = null
      return
    }
    if (['queued', 'running', 'paused', 'blocked'].includes(run.status)) {
      executionDockRun.value = run
      return
    }
    if (executionDockRun.value?.runId !== run.runId) {
      executionDockRun.value = null
      return
    }
    executionDockRun.value = run
    executionDockHideTimer = setTimeout(() => {
      if (executionDockRun.value?.runId === run.runId) {
        executionDockRun.value = null
      }
      executionDockHideTimer = null
    }, PLAN_RUN_TERMINAL_HOLD_MS)
  }

  return {
    clearExecutionDockHideTimer,
    syncExecutionDockRun,
  }
}
