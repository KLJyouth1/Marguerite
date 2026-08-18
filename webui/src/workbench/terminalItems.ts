import type { WorkbenchItem } from './types'

/**
 * 沙箱终端工作台项：通过自定义事件打开，scope 绑定当前会话，
 * 支持多标签复用（同一会话只打开一个终端面板）。
 */
export const TERMINAL_WORKBENCH_OPEN_EVENT = 'marguerite:open-terminal'

export interface TerminalWorkbenchOpenEventDetail {
  scopeId: string
}

function scopeDigest(value: string): string {
  const bytes = new TextEncoder().encode(value)
  let hash = 0xcbf29ce484222325n
  for (const byte of bytes) {
    hash ^= BigInt(byte)
    hash = BigInt.asUintN(64, hash * 0x100000001b3n)
  }
  return hash.toString(16).padStart(16, '0')
}

export function createTerminalWorkbenchItem(options: {
  scopeId: string
}): WorkbenchItem | null {
  const scopeId = options.scopeId.trim()
  if (!scopeId) return null
  return {
    id: `terminal:${scopeDigest(scopeId)}`,
    kind: 'terminal',
    title: '沙箱终端',
    scope: { type: 'session', id: scopeId },
    hostKind: 'dom',
    retention: 'dispose-on-suspend',
    payload: { scopeId },
  }
}

export function terminalScopeFromWorkbenchItem(item: WorkbenchItem): string {
  if (item.kind !== 'terminal') return ''
  return typeof item.payload.scopeId === 'string' ? item.payload.scopeId : ''
}

export function requestTerminalWorkbenchOpen(scopeId: string): boolean {
  if (typeof window === 'undefined') return false
  window.dispatchEvent(new CustomEvent<TerminalWorkbenchOpenEventDetail>(
    TERMINAL_WORKBENCH_OPEN_EVENT,
    { detail: { scopeId } },
  ))
  return true
}
