import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { useRpcStore } from '@/stores/rpc'
import { __resetToolSourcesForTest, useToolSources } from './useToolSources'

/** 用桩 client 把 rpc store 置为「已连接」，并返回该桩的 call。 */
function connectWith(response: unknown): ReturnType<typeof vi.fn> {
  const call = vi.fn().mockResolvedValue(response)
  setActivePinia(createPinia())
  const rpc = useRpcStore()
  rpc.client = { call } as never
  rpc.state = 'connected' as never
  return call
}

describe('useToolSources', () => {
  beforeEach(() => {
    __resetToolSourcesForTest()
  })

  it('reads mcp.tools.list with cachedOnly so rendering never probes servers', async () => {
    const call = connectWith({ tools: [{ name: 'baota_system_info', server: 'baota' }] })
    const src = useToolSources()

    await src.ensureLoaded()

    expect(call).toHaveBeenCalledWith('mcp.tools.list', { cachedOnly: true })
    expect(src.toolSource('baota_system_info')).toBe('mcp')
    expect(src.toolServer('baota_system_info')).toBe('baota')
    expect(src.toolSource('read_file')).toBeNull()
  })

  it('fetches once across consumers', async () => {
    const call = connectWith({ tools: [] })

    await useToolSources().ensureLoaded()
    await useToolSources().ensureLoaded()

    expect(call).toHaveBeenCalledTimes(1)
  })

  it('treats an empty list as a valid result and does not re-poll every render', async () => {
    const call = connectWith({ tools: [] })
    const src = useToolSources()

    await src.ensureLoaded()
    await src.ensureLoaded()

    expect(call).toHaveBeenCalledTimes(1)
    expect(src.toolSource('anything')).toBeNull()
  })

  it('stays silent on failure and retries on the next call', async () => {
    const call = vi
      .fn()
      .mockRejectedValueOnce(new Error('not connected'))
      .mockResolvedValue({ tools: [{ name: 'x', server: 'srv' }] })
    setActivePinia(createPinia())
    const rpc = useRpcStore()
    rpc.client = { call } as never
    rpc.state = 'connected' as never
    const src = useToolSources()

    await src.ensureLoaded()
    expect(src.toolSource('x')).toBeNull()

    await src.ensureLoaded()
    expect(src.toolSource('x')).toBe('mcp')
    expect(call).toHaveBeenCalledTimes(2)
  })

  it('ignores malformed entries instead of inventing a source', async () => {
    connectWith({ tools: [{ server: 'srv-no-name' }, { name: '' }, null] })
    const src = useToolSources()

    await src.ensureLoaded()

    expect(src.toolSource('')).toBeNull()
    expect(src.toolServer('')).toBe('')
  })
})
