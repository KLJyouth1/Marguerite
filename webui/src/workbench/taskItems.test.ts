// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest'
import type { WorkbenchItem } from './types'
import {
  TASK_WORKBENCH_OPEN_EVENT,
  createTaskWorkbenchItem,
  openTaskWorkbenchItem,
  requestTaskWorkbenchOpen,
} from './taskItems'

describe('task Workbench items', () => {
  it('builds a dom, keep-alive panel with a stable id and app scope by default', () => {
    const item = createTaskWorkbenchItem()

    expect(item.kind).toBe('task')
    expect(item.id).toBe('task:current')
    expect(item.hostKind).toBe('dom')
    expect(item.retention).toBe('keep-alive')
    expect(item.scope).toEqual({ type: 'app' })
    expect(item.payload).toEqual({})
  })

  it('scopes to the active session when one is provided', () => {
    const item = createTaskWorkbenchItem({ scopeId: 'agent:main:webchat:1' })

    expect(item.scope).toEqual({ type: 'session', id: 'agent:main:webchat:1' })
  })

  // Gap4: the panel must be openable from production code (it had no caller).
  it('openTaskWorkbenchItem opens the item through the store (production open path)', () => {
    const openItem = vi.fn<(item: WorkbenchItem) => boolean>(() => true)
    const ok = openTaskWorkbenchItem({ openItem }, 'session-1')

    expect(ok).toBe(true)
    expect(openItem).toHaveBeenCalledTimes(1)
    const opened = openItem.mock.calls[0][0]
    expect(opened.kind).toBe('task')
    expect(opened.scope).toEqual({ type: 'session', id: 'session-1' })
    // Propagates the store's refusal so the UI can surface the item-limit toast.
    expect(openTaskWorkbenchItem({ openItem: () => false })).toBe(false)
  })

  it('requestTaskWorkbenchOpen dispatches the window open event with the scope', () => {
    const received: Array<string | undefined> = []
    const handler = (event: Event) => {
      received.push((event as CustomEvent<{ scopeId?: string }>).detail?.scopeId)
    }
    window.addEventListener(TASK_WORKBENCH_OPEN_EVENT, handler)
    try {
      expect(requestTaskWorkbenchOpen('scope-9')).toBe(true)
      expect(received).toEqual(['scope-9'])
    } finally {
      window.removeEventListener(TASK_WORKBENCH_OPEN_EVENT, handler)
    }
  })
})
