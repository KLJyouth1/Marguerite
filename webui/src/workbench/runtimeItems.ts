import type { WorkbenchItem } from './types'

/**
 * 「运行态」工作台项：一屏看清 Harness 的三层（决策 / 执行环境 / 上下文）与质量证据。
 *
 * 对应知识图里的 `Agent = Model + Harness`：模型推理本身不可内省，但 Harness
 * 侧的**决定、环境、证据**都可以如实展示。这里只读聚合，不发起任何执行。
 */
export const RUNTIME_WORKBENCH_OPEN_EVENT = 'marguerite:open-runtime'

export interface RuntimeWorkbenchOpenEventDetail {
  scopeId?: string
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

/** 同一会话只开一个运行态面板（它是观测面，不是工作副本）。 */
export function createRuntimeWorkbenchItem(options: {
  scopeId: string
  /** 已本地化的标题（缺省中文，便于测试与命令面板以外的入口复用） */
  title?: string
}): WorkbenchItem | null {
  const scopeId = options.scopeId.trim()
  if (!scopeId) return null
  return {
    id: `runtime:${scopeDigest(scopeId)}`,
    kind: 'runtime',
    title: options.title?.trim() || '运行态',
    scope: { type: 'session', id: scopeId },
    hostKind: 'dom',
    retention: 'keep-alive',
    payload: { scopeId },
  }
}

export function runtimeScopeFromWorkbenchItem(item: WorkbenchItem): string {
  if (item.kind !== 'runtime') return ''
  return typeof item.payload.scopeId === 'string' ? item.payload.scopeId : ''
}

export function requestRuntimeWorkbenchOpen(scopeId?: string): boolean {
  if (typeof window === 'undefined') return false
  const detail: RuntimeWorkbenchOpenEventDetail = scopeId ? { scopeId } : {}
  window.dispatchEvent(new CustomEvent<RuntimeWorkbenchOpenEventDetail>(
    RUNTIME_WORKBENCH_OPEN_EVENT,
    { detail },
  ))
  return true
}
