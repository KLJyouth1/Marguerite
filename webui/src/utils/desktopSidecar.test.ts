// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { restartDesktopSidecar } from './desktopSidecar'

type InvokeMock = ReturnType<typeof vi.fn>

function installTauri(invoke: InvokeMock): void {
  ;(window as unknown as { __TAURI__?: unknown }).__TAURI__ = {
    core: { invoke },
  }
}

afterEach(() => {
  delete (window as unknown as { __TAURI__?: unknown }).__TAURI__
  vi.restoreAllMocks()
})

describe('restartDesktopSidecar', () => {
  it('reports unavailable outside the desktop shell instead of pretending', async () => {
    const result = await restartDesktopSidecar()
    expect(result).toEqual({ ok: false, reason: 'unavailable' })
  })

  it('does not interrupt a healthy sidecar', async () => {
    const invoke = vi.fn(async (cmd: string) => {
      if (cmd === 'sidecar_status') return { status: 'running', port: 1234 }
      throw new Error(`unexpected ${cmd}`)
    })
    installTauri(invoke)

    const result = await restartDesktopSidecar()

    expect(result).toEqual({ ok: true, action: 'already-running', status: 'running' })
    expect(invoke.mock.calls.map(call => call[0])).toEqual(['sidecar_status'])
  })

  it('stops then starts a dead sidecar', async () => {
    const invoke = vi.fn(async (cmd: string) => {
      if (cmd === 'sidecar_status') return { status: 'failed' }
      if (cmd === 'sidecar_stop') return null
      if (cmd === 'sidecar_start') return { status: 'running', port: 4321 }
      throw new Error(`unexpected ${cmd}`)
    })
    installTauri(invoke)

    const result = await restartDesktopSidecar()

    expect(result).toEqual({ ok: true, action: 'restarted', status: 'running' })
    expect(invoke.mock.calls.map(call => call[0]))
      .toEqual(['sidecar_status', 'sidecar_stop', 'sidecar_start'])
  })

  it('still starts when the idempotent stop call fails', async () => {
    const invoke = vi.fn(async (cmd: string) => {
      if (cmd === 'sidecar_status') return { status: 'stopped' }
      if (cmd === 'sidecar_stop') throw new Error('already stopped')
      if (cmd === 'sidecar_start') return { status: 'starting' }
      throw new Error(`unexpected ${cmd}`)
    })
    installTauri(invoke)

    await expect(restartDesktopSidecar())
      .resolves.toEqual({ ok: true, action: 'restarted', status: 'starting' })
  })

  it('surfaces a start failure with its reason', async () => {
    const invoke = vi.fn(async (cmd: string) => {
      if (cmd === 'sidecar_status') return { status: 'degraded' }
      if (cmd === 'sidecar_stop') return null
      if (cmd === 'sidecar_start') throw new Error('python not found')
      throw new Error(`unexpected ${cmd}`)
    })
    installTauri(invoke)

    await expect(restartDesktopSidecar())
      .resolves.toEqual({ ok: false, reason: 'failed', error: 'python not found' })
  })
})
