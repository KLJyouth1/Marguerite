import { beforeEach, describe, expect, it, vi } from 'vitest'

const rpc = vi.hoisted(() => ({
  call: vi.fn(),
  supportsMethod: vi.fn(),
}))

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => rpc,
}))

import { useWorkspaceStatus } from './useWorkspaceStatus'

function status(overrides: Record<string, unknown> = {}) {
  return {
    ok: true,
    root: 'D:\\proj',
    source: 'bound',
    bound: true,
    guidance: '【工作区】…',
    ...overrides,
  }
}

beforeEach(() => {
  rpc.call.mockReset()
  rpc.supportsMethod.mockReset().mockReturnValue(true)
})

describe('useWorkspaceStatus', () => {
  it('forwards the session bound root as projectPath', async () => {
    rpc.call.mockResolvedValue(status())
    const ws = useWorkspaceStatus()

    await ws.load('D:\\proj')

    expect(rpc.call).toHaveBeenCalledWith('workspace.status', { projectPath: 'D:\\proj' })
    expect(ws.status.value?.bound).toBe(true)
    expect(ws.status.value?.root).toBe('D:\\proj')
  })

  it('omits the param for an unbound session so the backend reports its own fallback', async () => {
    rpc.call.mockResolvedValue(status({ root: 'C:\\AppData\\Marguerite', source: 'app_data', bound: false }))
    const ws = useWorkspaceStatus()

    await ws.load(null)

    expect(rpc.call).toHaveBeenCalledWith('workspace.status', {})
    expect(ws.status.value?.source).toBe('app_data')
  })

  it('treats a whitespace-only root as unbound instead of sending it', async () => {
    rpc.call.mockResolvedValue(status({ source: 'app_data', bound: false }))
    const ws = useWorkspaceStatus()

    await ws.load('   ')

    expect(rpc.call).toHaveBeenCalledWith('workspace.status', {})
  })

  it('renders nothing when the gateway does not declare the method', async () => {
    rpc.supportsMethod.mockReturnValue(false)
    const ws = useWorkspaceStatus()

    await ws.load('D:\\proj')

    expect(rpc.call).not.toHaveBeenCalled()
    expect(ws.status.value).toBeNull()
  })

  it('hides the chip rather than inventing a binding state when the call fails', async () => {
    rpc.call.mockRejectedValue(new Error('offline'))
    const ws = useWorkspaceStatus()

    await ws.load('D:\\proj')

    expect(ws.status.value).toBeNull()
    expect(ws.loading.value).toBe(false)
  })

  it('reset() clears a previously loaded status', async () => {
    rpc.call.mockResolvedValue(status())
    const ws = useWorkspaceStatus()
    await ws.load('D:\\proj')
    expect(ws.status.value).not.toBeNull()

    ws.reset()

    expect(ws.status.value).toBeNull()
    expect(ws.loading.value).toBe(false)
  })
})
