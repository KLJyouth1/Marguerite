// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { installDesktopBridge } from './desktopBridge'
import { detectPlatformId } from './capabilities'

interface Bridge {
  getOsLocale: () => Promise<string | undefined>
  getGatewayStatus: () => Promise<{
    url: string
    port: number
    owned: boolean
    status: string
    logPath: string
    error?: string
  }>
  retryStartup: () => Promise<{ ok: boolean; error?: string }>
  chooseProjectDirectory: (request?: { initialPath?: string }) => Promise<{ path: string } | null>
}

function bridge(): Bridge {
  return (window as unknown as { MargueriteDesktop: Bridge }).MargueriteDesktop
}

function installTauri(invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>) {
  ;(window as unknown as { __TAURI__?: unknown }).__TAURI__ = { core: { invoke } }
}

afterEach(() => {
  delete (window as unknown as { __TAURI__?: unknown }).__TAURI__
  delete (window as unknown as { MargueriteDesktop?: unknown }).MargueriteDesktop
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('installDesktopBridge', () => {
  it('is a no-op outside the Tauri shell and keeps the platform as web', () => {
    expect(installDesktopBridge()).toBe(false)
    expect((window as unknown as { MargueriteDesktop?: unknown }).MargueriteDesktop).toBeUndefined()
    expect(detectPlatformId()).toBe('web')
  })

  it('installs the bridge inside the shell so the platform flips to desktop', () => {
    installTauri(async () => null)

    expect(installDesktopBridge()).toBe(true)
    expect(detectPlatformId()).toBe('desktop')
  })

  it('never overwrites a bridge that the shell already provided', () => {
    installTauri(async () => null)
    const existing = { marker: true }
    ;(window as unknown as { MargueriteDesktop?: unknown }).MargueriteDesktop = existing

    expect(installDesktopBridge()).toBe(true)
    expect((window as unknown as { MargueriteDesktop?: unknown }).MargueriteDesktop).toBe(existing)
  })

  it('reports the host locale from the WebView', async () => {
    installTauri(async () => null)
    installDesktopBridge()

    await expect(bridge().getOsLocale()).resolves.toBe(navigator.language || undefined)
  })

  it('maps a running sidecar into a ready gateway status', async () => {
    installTauri(async (cmd) => {
      if (cmd === 'sidecar_status') return { status: 'running', port: 54321, base_url: 'http://127.0.0.1:54321' }
      return null
    })
    installDesktopBridge()

    await expect(bridge().getGatewayStatus()).resolves.toEqual({
      url: 'http://127.0.0.1:54321',
      port: 54321,
      owned: true,
      status: 'ready',
      logPath: '',
    })
  })

  it('surfaces the sidecar log directory so the runtime panel is not stuck at 不可用', async () => {
    installTauri(async (cmd) => {
      if (cmd === 'sidecar_status') return { status: 'running', port: 18790 }
      if (cmd === 'sidecar_log_dir') return 'C:\\Temp\\marguerite-sidecar'
      return null
    })
    installDesktopBridge()

    await expect(bridge().getGatewayStatus()).resolves.toMatchObject({
      status: 'ready',
      logPath: 'C:\\Temp\\marguerite-sidecar',
    })
  })

  it('keeps the gateway status readable when the log directory command is unavailable', async () => {
    installTauri(async (cmd) => {
      if (cmd === 'sidecar_status') return { status: 'running', port: 18790 }
      throw new Error(`Command ${cmd} not allowed by ACL`)
    })
    installDesktopBridge()

    await expect(bridge().getGatewayStatus()).resolves.toMatchObject({
      status: 'ready',
      logPath: '',
    })
  })

  it('treats the gateway serving this page as the active one (Rust-owned, sidecar idle)', async () => {
    // 实机缺陷复现：Rust 引擎托管网关时 sidecar_status 回 idle，
    // 但页面正是从这个网关加载的 —— 必须报 ready，而不是「无活动网关」。
    installTauri(async (cmd) => (cmd === 'sidecar_status' ? { status: 'idle' } : null))
    vi.stubGlobal('fetch', vi.fn(async () => ({
      ok: true,
      json: async () => ({ status: 'ok', version: '0.2.16' }),
    })))
    installDesktopBridge()

    await expect(bridge().getGatewayStatus()).resolves.toMatchObject({
      status: 'ready',
      port: Number(window.location.port) || 0,
    })
    const version = await (window.MargueriteDesktop as unknown as {
      getAppVersion: () => Promise<string | undefined>
    }).getAppVersion()
    expect(version).toBe('0.2.16')
  })

  it('maps a degraded sidecar to an error status instead of pretending it is fine', async () => {
    installTauri(async (cmd) => (cmd === 'sidecar_status' ? { status: 'degraded' } : null))
    installDesktopBridge()

    const status = await bridge().getGatewayStatus()
    expect(status.status).toBe('error')
  })

  it('restarts a dead sidecar through the desktop commands', async () => {
    const calls: string[] = []
    installTauri(async (cmd) => {
      calls.push(cmd)
      if (cmd === 'sidecar_status') return { status: 'failed' }
      if (cmd === 'sidecar_start') return { status: 'running', port: 41000 }
      return null
    })
    installDesktopBridge()

    await expect(bridge().retryStartup()).resolves.toEqual({ ok: true })
    expect(calls).toEqual(['sidecar_status', 'sidecar_stop', 'sidecar_start'])
  })

  it('surfaces a restart failure instead of reporting success', async () => {
    installTauri(async (cmd) => {
      if (cmd === 'sidecar_status') return { status: 'failed' }
      if (cmd === 'sidecar_start') throw new Error('python missing')
      return null
    })
    installDesktopBridge()

    await expect(bridge().retryStartup()).resolves.toEqual({ ok: false, error: 'python missing' })
  })

  it('opens the native directory picker with the documented dialog options', async () => {
    const invoke = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'plugin:dialog|open') {
        expect(args).toEqual({
          options: { directory: true, multiple: false, defaultPath: 'D:\\work' },
        })
        return 'D:\\work\\chosen'
      }
      return null
    })
    installTauri(invoke)
    installDesktopBridge()

    await expect(bridge().chooseProjectDirectory({ initialPath: 'D:\\work' }))
      .resolves.toEqual({ path: 'D:\\work\\chosen' })
  })

  it('treats a cancelled picker as no selection', async () => {
    installTauri(async () => null)
    installDesktopBridge()

    await expect(bridge().chooseProjectDirectory()).resolves.toBeNull()
  })

  it('declares no native updater so the update banner keeps working', async () => {
    installTauri(async () => null)
    installDesktopBridge()
    const api = window.MargueriteDesktop as unknown as {
      isAutoUpdateEnabled: () => Promise<boolean>
      isDesktopUpdateManaged: () => Promise<boolean>
    }

    await expect(api.isAutoUpdateEnabled()).resolves.toBe(false)
    await expect(api.isDesktopUpdateManaged()).resolves.toBe(false)
  })
})
