import { describe, expect, it, vi } from 'vitest'

import { useFileChanges } from './useFileChanges'
import type { UseFileChangesOptions } from './useFileChanges'
import type { FileChangeListPayload } from '@/types/rpc'

type Rpc = UseFileChangesOptions['rpc']

function makeRpc(overrides: { supportsMethod?: (method: string) => boolean } = {}) {
  return {
    call: vi.fn(),
    supportsMethod: vi.fn(overrides.supportsMethod ?? ((_method: string) => true)),
    markMethodUnavailable: vi.fn(),
  }
}

const item = (path: string) => ({
  path,
  kind: 'modify' as const,
  before: 'old',
  after: 'new',
  tool: 'edit_file',
  ts: 1,
})

describe('useFileChanges', () => {
  it('loads items from workspace.file_changes.list', async () => {
    const rpc = makeRpc()
    rpc.call.mockResolvedValue({ items: [item('a.rs'), item('b.rs')] } satisfies FileChangeListPayload)
    const store = useFileChanges({ rpc: rpc as unknown as Rpc, limit: 20 })

    const ok = await store.load()

    expect(ok).toBe(true)
    expect(store.available.value).toBe(true)
    expect(store.changes.value).toHaveLength(2)
    expect(rpc.call).toHaveBeenCalledWith('workspace.file_changes.list', { limit: 20 })
  })

  it('stays unavailable and skips the call when the method is unsupported', async () => {
    const rpc = makeRpc({ supportsMethod: () => false })
    const store = useFileChanges({ rpc: rpc as unknown as Rpc })

    const ok = await store.load()

    expect(ok).toBe(false)
    expect(store.available.value).toBe(false)
    expect(rpc.call).not.toHaveBeenCalled()
  })

  it('returns false on call failure', async () => {
    const rpc = makeRpc()
    rpc.call.mockRejectedValue(new Error('boom'))
    const store = useFileChanges({ rpc: rpc as unknown as Rpc })

    const ok = await store.load()

    expect(ok).toBe(false)
    expect(store.available.value).toBe(false)
  })

  it('resets all state', () => {
    const rpc = makeRpc()
    const store = useFileChanges({ rpc: rpc as unknown as Rpc })
    store.changes.value = [item('a.rs')]
    store.available.value = true

    store.reset()

    expect(store.changes.value).toEqual([])
    expect(store.available.value).toBe(false)
  })
})
