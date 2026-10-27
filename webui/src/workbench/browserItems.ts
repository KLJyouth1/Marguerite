import type { WorkbenchItem } from './types'

export const BROWSER_WORKBENCH_OPEN_EVENT = 'marguerite:open-side-browser'

export interface BrowserWorkbenchOpenEventDetail {
  url: string
}

function urlDigest(value: string): string {
  const bytes = new TextEncoder().encode(value)
  let hash = 0xcbf29ce484222325n
  for (const byte of bytes) {
    hash ^= BigInt(byte)
    hash = BigInt.asUintN(64, hash * 0x100000001b3n)
  }
  return hash.toString(16).padStart(16, '0')
}

export function normalizeBrowserUrl(value: string): string {
  const trimmed = value.trim()
  if (!trimmed || trimmed.length > 8192 || /[\u0000-\u001f\u007f]/.test(trimmed)) return ''
  try {
    const url = new URL(trimmed)
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return ''
    url.username = ''
    url.password = ''
    return url.toString()
  } catch {
    return ''
  }
}

/**
 * 侧边浏览器工作台项。
 *
 * 2026-10-24 起 `hostKind` 从 `native-webcontents` 改为 `dom`：面板由 CDP 镜像
 * （`browser.*` RPC + 截图 + 点击转发）在 DOM 里渲染。原因写在
 * `browserWorkbenchProvider.ts` 顶部 —— 当前桌面壳未实现原生表面命令，
 * 走 native 的结果是「面板打不开」。
 */
export function createBrowserWorkbenchItem(options: {
  scopeId: string
  url: string
}): WorkbenchItem | null {
  const url = normalizeBrowserUrl(options.url)
  if (!url) return null
  const scopeId = options.scopeId.trim()
  if (!scopeId) return null
  const parsed = new URL(url)
  return {
    id: `browser:${urlDigest(scopeId)}:${urlDigest(url)}`,
    kind: 'browser',
    title: parsed.hostname,
    scope: { type: 'session', id: scopeId },
    hostKind: 'dom',
    retention: 'keep-alive',
    payload: {
      initialUrl: url,
      scopeId,
    },
  }
}

export function browserUrlFromWorkbenchItem(item: WorkbenchItem): string {
  if (item.kind !== 'browser') return ''
  return normalizeBrowserUrl(
    typeof item.payload.initialUrl === 'string' ? item.payload.initialUrl : '',
  )
}

export function requestBrowserWorkbenchOpen(url: string): boolean {
  const normalized = normalizeBrowserUrl(url)
  if (!normalized || typeof window === 'undefined') return false
  window.dispatchEvent(new CustomEvent<BrowserWorkbenchOpenEventDetail>(
    BROWSER_WORKBENCH_OPEN_EVENT,
    { detail: { url: normalized } },
  ))
  return true
}
