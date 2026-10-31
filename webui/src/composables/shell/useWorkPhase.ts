// 工作相位（work phase）：桌面端唯一真值是网关状态，这里只**读**已有信号做归类，
// 不新增任何 RPC，也不在信号缺失时给出一个好看但虚假的相位。
//
// 四个相位（由强到弱的判定顺序 = review > execute > explore > idle）：
//   - review  有改动/请求待确认 —— 用户必须先做决定，优先级最高；
//   - execute 有工具任务在跑 —— 用户此刻能做的是观察或中止；
//   - explore 会话已开但没有任何工具调用 —— 用户在读、在想；
//   - idle    没有活动会话 —— 连"在做什么"都无从谈起。
//
// `reason` 是稳定原因码（不是成品文案）：UI 按 `shell.commandDeck.reason.${reason}`
// 如实显示，免得推导层替翻译层编故事。信号缺失（null / 非数组 / 空数组 /
// 计数非有限值）一律落到 idle，不猜"大概是 explore"。
import { computed, type ComputedRef, type Ref } from 'vue'

export type WorkPhaseKind = 'idle' | 'explore' | 'execute' | 'review'

export type WorkPhaseReason =
  | 'noActiveSession'
  | 'signalsIncomplete'
  | 'noToolCalls'
  | 'toolsRunning'
  | 'awaitingConfirmation'

export interface WorkPhase {
  kind: WorkPhaseKind
  /** 稳定原因码；计数型原因（toolsRunning / awaitingConfirmation）附带读数。 */
  reason: WorkPhaseReason
  count?: number
}

export interface WorkPhaseInput {
  /** 活动会话列表；null/空数组表示没有活动会话。 */
  sessions: readonly unknown[] | null | undefined
  /** 正在跑的工具任务组数量；null/undefined 视为信号缺失。 */
  activeToolCount: number | null | undefined
  /** 待用户确认的改动/请求数量；null/undefined 视为信号缺失。 */
  pendingConfirmationCount: number | null | undefined
}

function isCount(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0
}

/** 纯函数：便于单测，也让"缺信号→idle"这条纪律只有一处实现。 */
export function deriveWorkPhase(input: WorkPhaseInput): WorkPhase {
  const sessions = input.sessions
  if (!Array.isArray(sessions) || sessions.length === 0) {
    return { kind: 'idle', reason: 'noActiveSession' }
  }
  if (!isCount(input.activeToolCount) || !isCount(input.pendingConfirmationCount)) {
    return { kind: 'idle', reason: 'signalsIncomplete' }
  }
  const pending = Math.trunc(input.pendingConfirmationCount)
  if (pending > 0) {
    return { kind: 'review', reason: 'awaitingConfirmation', count: pending }
  }
  const tools = Math.trunc(input.activeToolCount)
  if (tools > 0) {
    return { kind: 'execute', reason: 'toolsRunning', count: tools }
  }
  return { kind: 'explore', reason: 'noToolCalls' }
}

export interface WorkPhaseSignals {
  sessions: Ref<readonly unknown[] | null | undefined>
  activeToolCount: Ref<number | null | undefined>
  pendingConfirmationCount: Ref<number | null | undefined>
}

export function useWorkPhase(signals: WorkPhaseSignals): ComputedRef<WorkPhase> {
  return computed(() => deriveWorkPhase({
    sessions: signals.sessions.value,
    activeToolCount: signals.activeToolCount.value,
    pendingConfirmationCount: signals.pendingConfirmationCount.value,
  }))
}
