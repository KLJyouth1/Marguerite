import { describe, expect, it, vi } from 'vitest'
import {
  createIslandWorkbenchItem,
  ISLAND_WORKBENCH_OPEN_EVENT,
  openIslandWorkbenchItem,
  requestIslandWorkbenchOpen,
  type IslandWorkbenchOpenEventDetail,
} from './islandItems'
import { createIslandWorkbenchDefinition } from '@/components/workbench/islandWorkbenchProvider'
import { createTaskWorkbenchItem } from './taskItems'

describe('islandItems — 面板项契约', () => {
  it('kind/宿主/保留策略固定，未指定会话时落在 app 作用域', () => {
    const item = createIslandWorkbenchItem()
    expect(item).toMatchObject({
      id: 'island:app',
      kind: 'island',
      hostKind: 'dom',
      retention: 'keep-alive',
      scope: { type: 'app' },
    })
  })

  it('同一会话的 id 稳定（重复打开复用面板而不是堆副本）', () => {
    const a = createIslandWorkbenchItem({ scopeId: 'sess-1' })
    const b = createIslandWorkbenchItem({ scopeId: 'sess-1' })
    expect(a.id).toBe(b.id)
    expect(a.scope).toEqual({ type: 'session', id: 'sess-1' })
    expect(createIslandWorkbenchItem({ scopeId: 'sess-2' }).id).not.toBe(a.id)
  })

  it('openIslandWorkbenchItem 把 item 交给 store，并回传其结果', () => {
    const openItem = vi.fn().mockReturnValue(true)
    expect(openIslandWorkbenchItem({ openItem }, 'sess-9')).toBe(true)
    expect(openItem).toHaveBeenCalledTimes(1)
    expect(openItem.mock.calls[0][0]).toMatchObject({ kind: 'island', id: 'island:sess-9' })

    const rejected = vi.fn().mockReturnValue(false)
    expect(openIslandWorkbenchItem({ openItem: rejected })).toBe(false)
  })

  it('requestIslandWorkbenchOpen 派发携带 scope 的窗口事件', () => {
    const dispatched: { type: string; detail?: IslandWorkbenchOpenEventDetail }[] = []
    const stub = {
      dispatchEvent: (event: Event) => {
        const custom = event as CustomEvent<IslandWorkbenchOpenEventDetail>
        dispatched.push({ type: custom.type, detail: custom.detail })
        return true
      },
    }
    const host = globalThis as unknown as { window?: unknown }
    const previous = host.window
    host.window = stub
    try {
      expect(requestIslandWorkbenchOpen('sess-7')).toBe(true)
      expect(dispatched).toEqual([{ type: ISLAND_WORKBENCH_OPEN_EVENT, detail: { scopeId: 'sess-7' } }])
    } finally {
      host.window = previous
    }
  })

  it('无 DOM 宿主时不抛错，直接返回 false（SSR/测试环境）', () => {
    const host = globalThis as unknown as { window?: unknown }
    const previous = host.window
    delete host.window
    try {
      expect(requestIslandWorkbenchOpen()).toBe(false)
    } finally {
      host.window = previous
    }
  })
})

describe('islandWorkbenchProvider — 面板定义', () => {
  const definition = createIslandWorkbenchDefinition()

  it('只认领 island 面板，标题与图标来自 item', () => {
    const item = createIslandWorkbenchItem({ scopeId: 'sess-1' })
    expect(definition.kind).toBe('island')
    expect(definition.supports?.(item)).toBe(true)
    expect(definition.supports?.(createTaskWorkbenchItem())).toBe(false)
    expect(definition.getHeader?.(item, renderState())).toMatchObject({ title: 'Island workspace', icon: 'blocks' })
    expect(definition.getProps?.(item, renderState())).toEqual({ item })
  })

  it('面板组件已绑定（不是空定义）', () => {
    expect(definition.component).toBeTruthy()
  })
})

function renderState() {
  return { active: true, hostAvailable: true, nativeSurface: false, runtimeState: {} }
}