// @vitest-environment happy-dom

import { afterEach, describe, expect, it, vi } from 'vitest'
import type { ArtifactPayload } from '@/types/rpc'
import { useArtifactActions } from './useArtifactActions'

afterEach(() => {
  vi.restoreAllMocks()
})

/** `rpc.call` 是泛型签名，非泛型的 `async () => ({})` 过不了 typecheck。 */
async function rpcCall<T = unknown>(): Promise<T> {
  return {} as T
}

function artifact(): ArtifactPayload {
  return {
    id: 'artifact:agent:main:webchat:ok:notes.txt',
    name: 'notes.txt',
    path: 'notes.txt',
    mime: 'text/plain',
  } as unknown as ArtifactPayload
}

describe('useArtifactActions', () => {
  it('sends the session key header when reading artifact text', async () => {
    // 网关（Rust 与侧车同契约）按会话归属产物：缺键/错键一律 404，
    // 所以这条头是内联编辑器能否工作的前提，不是可选优化。
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockResolvedValue({
      ok: true,
      status: 200,
      headers: new Headers({ 'Content-Length': '5' }),
      blob: async () => new Blob(['hello'], { type: 'text/plain' }),
    } as unknown as Response)

    const actions = useArtifactActions({
      rpc: {
        supportsMethod: () => false,
        markMethodUnavailable: () => {},
        call: rpcCall,
      },
      authToken: () => 'tok-1',
      baseOrigin: () => 'http://127.0.0.1:18790',
      sessionKey: () => 'agent:main:webchat:ok',
    })

    const result = await actions.loadTextContent(artifact())

    expect(result.status).toBe('ok')
    expect(result.content).toBe('hello')
    expect(fetchMock).toHaveBeenCalledTimes(1)
    const [url, init] = fetchMock.mock.calls[0]
    expect(String(url)).toBe('/api/v1/artifacts/artifact%3Aagent%3Amain%3Awebchat%3Aok%3Anotes.txt')
    const headers = (init?.headers || {}) as Record<string, string>
    expect(headers['x-Marguerite-session-key']).toBe('agent:main:webchat:ok')
    expect(headers.Authorization).toBe('Bearer tok-1')
  })
})
