import type { ChatRenderedMessage } from '@/types/chat'

/**
 * 「上一回合从哪开始」的时间戳：最近一条助手回复之前的用户消息时间。
 *
 * 用于把会话级台账裁成回合窗口。以下情况一律返回 null（不显示回合汇总）：
 * - 正在流式生成、首屏草稿、分享/分叉过渡中（回合尚未定稿）；
 * - 还没有助手回复；
 * - 已发出下一条用户消息但还没有助手回复（上一轮的汇总不再属于当前回合）。
 */
export function resolveLastTurnStartTs(
  rendered: ChatRenderedMessage[],
  options: { blocked?: boolean } = {},
): string | number | null {
  if (options.blocked) return null
  let lastAssistant = -1
  for (let i = rendered.length - 1; i >= 0; i--) {
    const row = rendered[i]
    if (row?.displayRole === 'assistant' && !row.stopNotice) {
      lastAssistant = i
      break
    }
  }
  if (lastAssistant < 0) return null
  for (let i = lastAssistant + 1; i < rendered.length; i++) {
    if (rendered[i]?.displayRole === 'user') return null
  }
  for (let i = lastAssistant; i >= 0; i--) {
    if (rendered[i]?.displayRole === 'user') return rendered[i].ts ?? null
  }
  return null
}

/**
 * 最近一个回合是否正常收尾。
 *
 * 「下一步建议」只在成功回合之后出现：被中断或终止性失败的回合，
 * 该做的是重试而不是继续加动作。
 */
export function isLastTurnSuccessful(rendered: ChatRenderedMessage[]): boolean {
  for (let i = rendered.length - 1; i >= 0; i--) {
    const row = rendered[i]
    if (row?.displayRole !== 'assistant' || row.stopNotice) continue
    return row.terminalFailure !== true && row.interrupted !== true
  }
  return false
}
