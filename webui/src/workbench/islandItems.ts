import type { WorkbenchItem } from './types'

/**
 * 岛屿工作空间面板项（workbench kind: 'island'）。
 * 与 /island 独立路由同构：都是「智能体运行的可视化岛屿」，
 * 区别是这里作为会话级面板，跟随当前会话在右侧窗格常驻（keep-alive）。
 * 稳定 id（island:<session>）保证重复打开复用面板而不是堆出多个副本。
 */
export function createIslandWorkbenchItem(options: { scopeId?: string } = {}): WorkbenchItem {
  const scopeId = options.scopeId
  return {
    id: `island:${scopeId || 'app'}`,
    kind: 'island',
    title: 'Island workspace',
    scope: scopeId ? { type: 'session', id: scopeId } : { type: 'app' },
    hostKind: 'dom',
    retention: 'keep-alive',
    payload: {},
  }
}

/**
 * Production open path for the island panel (mirrors taskItems)：
 * AppWorkbench 监听 {@link ISLAND_WORKBENCH_OPEN_EVENT} 后调用它，
 * 于是这个 provider 有真实 UI 入口，而不是只存在于测试里。
 */
export function openIslandWorkbenchItem(
  store: { openItem: (item: WorkbenchItem) => boolean },
  scopeId?: string,
): boolean {
  return store.openItem(createIslandWorkbenchItem({ scopeId }))
}

/** 请求打开岛屿面板的窗口事件（任何地方都能发起） */
export const ISLAND_WORKBENCH_OPEN_EVENT = 'marguerite:open-island'

export interface IslandWorkbenchOpenEventDetail {
  scopeId?: string
}

/** 派发打开请求；无 DOM 宿主（SSR/测试）时返回 false */
export function requestIslandWorkbenchOpen(scopeId?: string): boolean {
  if (typeof window === 'undefined') return false
  window.dispatchEvent(new CustomEvent<IslandWorkbenchOpenEventDetail>(
    ISLAND_WORKBENCH_OPEN_EVENT,
    { detail: { scopeId } },
  ))
  return true
}