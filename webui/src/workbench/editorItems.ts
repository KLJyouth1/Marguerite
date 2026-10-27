import type { WorkbenchItem } from './types'

/**
 * 内置编辑器工作台项（IDE 式单文件标签）。
 *
 * 一个文件 = 一个标签：同一路径重复打开会复用同一 `id`（`store.openItem` 幂等），
 * 不同文件各占一个标签。路径一律**工作区相对路径**（与 `workspace.file.*` 同口径），
 * 避免「绝对路径出根」被后端 fail-closed 拒绝。
 */
export const EDITOR_WORKBENCH_OPEN_EVENT = 'marguerite:open-editor'

export interface EditorWorkbenchOpenEventDetail {
  scopeId: string
  path: string
  title?: string
  /** 会话绑定的项目根；面板据此让 `workspace.file.*` 在同一个根上读写 */
  projectPath?: string
}

export interface EditorWorkbenchRequest {
  scopeId: string
  path: string
  projectPath?: string
}

function digest(value: string): string {
  const bytes = new TextEncoder().encode(value)
  let hash = 0xcbf29ce484222325n
  for (const byte of bytes) {
    hash ^= BigInt(byte)
    hash = BigInt.asUintN(64, hash * 0x100000001b3n)
  }
  return hash.toString(16).padStart(16, '0')
}

/** 工作区相对路径归一：去掉前导 `./` 与分隔符，反斜杠统一成正斜杠。 */
export function normalizeEditorPath(raw: string): string {
  const value = raw.trim().replace(/\\/g, '/').replace(/^\.\//, '').replace(/^\/+/, '')
  if (!value || value.length > 1024) return ''
  // 只挡「词法上就出根」的写法；最终仍由后端 safe_resolve 复核（fail-closed）
  if (value.split('/').includes('..')) return ''
  return value
}

export function editorTitleForPath(path: string): string {
  const parts = path.split('/')
  return parts[parts.length - 1] || path
}

export function createEditorWorkbenchItem(options: EditorWorkbenchRequest): WorkbenchItem | null {
  const path = normalizeEditorPath(options.path)
  const scopeId = options.scopeId.trim()
  if (!path || !scopeId) return null
  const projectPath = (options.projectPath || '').trim()
  return {
    id: `file:${digest(scopeId)}:${digest(path)}`,
    kind: 'file',
    title: editorTitleForPath(path),
    scope: { type: 'session', id: scopeId },
    hostKind: 'dom',
    retention: 'keep-alive',
    payload: { path, scopeId, ...(projectPath ? { projectPath } : {}) },
  }
}

export function editorPathFromWorkbenchItem(item: WorkbenchItem): string {
  if (item.kind !== 'file') return ''
  return typeof item.payload.path === 'string' ? item.payload.path : ''
}

export function editorScopeFromWorkbenchItem(item: WorkbenchItem): string {
  if (item.kind !== 'file') return ''
  return typeof item.payload.scopeId === 'string' ? item.payload.scopeId : ''
}

export function requestEditorWorkbenchOpen(request: EditorWorkbenchRequest): boolean {
  if (typeof window === 'undefined') return false
  const path = normalizeEditorPath(request.path)
  if (!path) return false
  const projectPath = (request.projectPath || '').trim()
  window.dispatchEvent(new CustomEvent<EditorWorkbenchOpenEventDetail>(
    EDITOR_WORKBENCH_OPEN_EVENT,
    { detail: { scopeId: request.scopeId, path, ...(projectPath ? { projectPath } : {}) } },
  ))
  return true
}
