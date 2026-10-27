import type { ChatRenderedMessage } from '@/types/chat'

export type SubagentTone = 'ok' | 'err' | 'run' | 'idle'

// 与 SubagentCard 的状态口径保持一致：卡片与右栏轴不能对同一个子智能体
// 给出两种状态结论。
const STATUS_OK = new Set(['success', 'completed', 'done', 'finished', 'ok'])
const STATUS_ERR = new Set(['failed', 'error', 'timeout', 'cancelled', 'aborted', 'rejected'])
const STATUS_RUN = new Set(['running', 'queued', 'in_progress', 'pending', 'working'])

export function subagentTone(status: unknown): SubagentTone {
  const value = String(status || '').toLowerCase()
  if (STATUS_OK.has(value)) return 'ok'
  if (STATUS_ERR.has(value)) return 'err'
  if (STATUS_RUN.has(value)) return 'run'
  return 'idle'
}

export interface SubagentPayload {
  type?: string
  status?: string
  terminal_reason?: string
  child_session_key?: string
  session_key?: string
  result?: { text?: string; summary?: string; [key: string]: unknown }
  [key: string]: unknown
}

export interface SubagentPanelEntry {
  key: string
  tone: SubagentTone
  status: string
  summary: string
  ts: ChatRenderedMessage['ts']
}

export function parseSubagentPayload(text: string): SubagentPayload | null {
  try {
    const parsed = JSON.parse(text)
    return parsed && typeof parsed === 'object' ? parsed as SubagentPayload : null
  } catch {
    return null
  }
}

/**
 * 右栏「子智能体」轴的数据源：直接从已渲染的 subagent_completion 消息里取，
 * 不新增 RPC；同一 child session 的多次完成事件以最后一次为准。
 */
export function subagentPanelEntries(messages: ChatRenderedMessage[]): SubagentPanelEntry[] {
  const entries = new Map<string, SubagentPanelEntry>()
  // 从后往前扫：同一 child session 只保留最后一次完成事件，输出天然是新的在前。
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index]
    if (message?.displayRole !== 'subagent') continue
    const payload = parseSubagentPayload(message.text)
    if (!payload) continue
    const key = String(payload.child_session_key || payload.session_key || '').trim()
    if (!key || entries.has(key)) continue
    const status = String(payload.status || payload.terminal_reason || '').trim()
    entries.set(key, {
      key,
      tone: subagentTone(status),
      status,
      summary: String(payload.result?.text || payload.result?.summary || '').trim(),
      ts: message.ts,
    })
  }
  return [...entries.values()]
}
