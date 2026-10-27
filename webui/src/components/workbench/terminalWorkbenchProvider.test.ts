import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { WorkbenchItem } from '@/workbench/types'
import { createTerminalWorkbenchDefinition } from './terminalWorkbenchProvider'
import { createTerminalWorkbenchItem } from '@/workbench/terminalItems'

/**
 * 沙箱终端 runtime 契约（2026-10-25：真实 PTY 改版）。
 *
 * 守护的是这轮修复的四个真实缺陷：
 * 1. `spawn` 必须把会话工作区根（`root`）一起传给后端，否则 cwd 会被判越界
 *    → 面板只剩「未连接」（用户截图故障）；
 * 2. 读取必须带游标（`cursor`），否则轮询之间会丢输出；
 * 3. 不再伪造 `> cmd` 提示符（PTY 自己会回显）；
 * 4. Ctrl+C 必须走 `sandbox.terminal.key`，而不是杀整个会话。
 */

const mock = vi.hoisted(() => {
  const calls: Array<{ method: string; params?: Record<string, unknown> }> = []
  const responses: Record<string, unknown> = {}
  const rpc = {
    isConnected: true,
    supportsMethod: (_method: string) => true,
    waitForConnection: async () => true,
    call: async (method: string, params?: Record<string, unknown>) => {
      calls.push({ method, params })
      const entry = responses[method]
      return typeof entry === 'function'
        ? (entry as (p?: Record<string, unknown>) => unknown)(params)
        : entry
    },
  }
  return { calls, responses, rpc }
})

vi.mock('@/stores/rpc', () => ({ useRpcStore: () => mock.rpc }))

const notifyReadFailure = vi.hoisted(() => vi.fn())
vi.mock('@/utils/readFailureNotice', () => ({ notifyReadFailure }))

function makeItem(): WorkbenchItem {
  const item = createTerminalWorkbenchItem({ scopeId: 'session-a' })
  if (!item) throw new Error('terminal workbench item 应可创建')
  return item
}

function makeContext(state: Record<string, unknown>) {
  return {
    getRenderState: () => state,
    updateRenderState: (patch: Readonly<Record<string, unknown>>) => {
      Object.assign(state, patch)
    },
    isItemOpen: () => true,
    setExpanded: () => {},
    reportError: () => {},
  }
}

beforeEach(() => {
  mock.calls.length = 0
  notifyReadFailure.mockClear()
  for (const key of Object.keys(mock.responses)) delete mock.responses[key]
  vi.useFakeTimers()
  // vitest 环境是 node：provider 依赖 window 的定时器与 localStorage，这里给出最小桩。
  vi.stubGlobal('window', {
    setInterval: (fn: () => void, ms: number) => setInterval(fn, ms),
    clearInterval: (id: number) => clearInterval(id),
    setTimeout: (fn: () => void, ms: number) => setTimeout(fn, ms),
    clearTimeout: (id: number) => clearTimeout(id),
    localStorage: {
      getItem: () => null,
      setItem: () => {},
    },
  })

  mock.responses['sandbox.terminal.shells'] = {
    ok: true,
    defaultShell: 'pwsh',
    shells: [
      { id: 'pwsh', label: 'PowerShell 7 (pwsh)', program: 'pwsh.exe', available: true, default: true },
      { id: 'cmd', label: 'Command Prompt (cmd)', program: 'cmd.exe', available: true },
    ],
  }
  mock.responses['workspace.status'] = { root: 'C:/Users/me/AppData/Local/Marguerite' }
  mock.responses['sandbox.terminal.spawn'] = {
    ok: true,
    warning: '请求的目录不在允许的工作区内，已回落',
    terminal: {
      id: 't1',
      cwd: 'C:/Users/me/AppData/Local/Marguerite',
      running: true,
      shell: 'pwsh',
      shellLabel: 'PowerShell 7 (pwsh)',
      cursor: 4,
    },
  }
  mock.responses['sandbox.terminal.read'] = {
    ok: true,
    output: 'hello',
    cursor: 9,
    lost: 0,
    running: true,
    exitCode: null,
  }
  mock.responses['sandbox.terminal.input'] = { ok: true }
  mock.responses['sandbox.terminal.key'] = { ok: true }
  mock.responses['sandbox.terminal.resize'] = { ok: true, cols: 100, rows: 30 }
  mock.responses['sandbox.terminal.kill'] = { ok: true }
})

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('terminal Workbench provider (PTY)', () => {
  it('spawn 带上工作区根与 shell，并把非致命告警透传到面板', async () => {
    const definition = createTerminalWorkbenchDefinition()
    const state: Record<string, unknown> = {}
    const runtime = await definition.createRuntime!(makeItem(), makeContext(state))

    const spawn = mock.calls.find(call => call.method === 'sandbox.terminal.spawn')
    expect(spawn).toBeTruthy()
    expect(spawn!.params).toMatchObject({
      cwd: 'C:/Users/me/AppData/Local/Marguerite',
      root: 'C:/Users/me/AppData/Local/Marguerite',
      shell: 'pwsh',
    })
    expect(state.connected).toBe(true)
    expect(state.warning).toContain('回落')
    expect(state.shell).toBe('pwsh')

    await runtime.dispose!('closed')
  })

  it('按游标读取增量、累积输出，且不伪造提示符', async () => {
    const definition = createTerminalWorkbenchDefinition()
    const state: Record<string, unknown> = {}
    const runtime = await definition.createRuntime!(makeItem(), makeContext(state))

    await vi.advanceTimersByTimeAsync(200)
    expect(state.output).toBe('hello')
    const read = mock.calls.find(call => call.method === 'sandbox.terminal.read')
    expect(read!.params).toMatchObject({ terminalId: 't1', cursor: 4 })

    await runtime.handleComponentEvent!({ type: 'terminal-input', payload: { data: 'echo hi' } }, makeItem())
    const input = mock.calls.find(call => call.method === 'sandbox.terminal.input')
    expect(input!.params).toMatchObject({ terminalId: 't1', data: 'echo hi' })
    expect(String(state.output)).toBe('hello')

    await runtime.dispose!('closed')
  })

  it('输入被护栏拒绝时必须可见，而不是静默吞掉', async () => {
    mock.responses['sandbox.terminal.input'] = { ok: false, error: '[L1] 阻断：rm -rf /' }
    const definition = createTerminalWorkbenchDefinition()
    const state: Record<string, unknown> = {}
    const runtime = await definition.createRuntime!(makeItem(), makeContext(state))

    await runtime.handleComponentEvent!({ type: 'terminal-input', payload: { data: 'rm -rf /' } }, makeItem())
    expect(String(state.output)).toContain('[拒绝]')
    expect(String(state.output)).toContain('[L1]')

    await runtime.dispose!('closed')
  })

  it('Ctrl+C 走控制键白名单，不杀会话', async () => {
    const definition = createTerminalWorkbenchDefinition()
    const state: Record<string, unknown> = {}
    const runtime = await definition.createRuntime!(makeItem(), makeContext(state))

    await runtime.handleComponentEvent!({ type: 'terminal-interrupt' }, makeItem())
    const key = mock.calls.find(call => call.method === 'sandbox.terminal.key')
    expect(key!.params).toMatchObject({ terminalId: 't1', sequence: 'ctrl-c' })
    expect(mock.calls.some(call => call.method === 'sandbox.terminal.kill')).toBe(false)

    await runtime.dispose!('closed')
  })

  it('终止未被服务端确认时如实报错，不谎报已断开', async () => {
    mock.responses['sandbox.terminal.kill'] = {
      ok: false,
      error: '终端进程未能终止（pid 9），可能仍在运行',
    }
    const definition = createTerminalWorkbenchDefinition()
    const state: Record<string, unknown> = {}
    const runtime = await definition.createRuntime!(makeItem(), makeContext(state))

    await runtime.handleComponentEvent!({ type: 'terminal-kill' }, makeItem())
    expect(state.connected).toBe(true)
    expect(String(state.errorMessage)).toContain('未能终止')

    // 清理：改回成功后 dispose
    mock.responses['sandbox.terminal.kill'] = { ok: true }
    await runtime.dispose!('closed')
  })

  it('面板尺寸变化同步给 PTY（cols / rows）', async () => {
    const definition = createTerminalWorkbenchDefinition()
    const state: Record<string, unknown> = {}
    const runtime = await definition.createRuntime!(makeItem(), makeContext(state))

    await runtime.handleComponentEvent!({ type: 'terminal-resize', payload: { cols: 100, rows: 30 } }, makeItem())
    const resize = mock.calls.find(call => call.method === 'sandbox.terminal.resize')
    expect(resize!.params).toMatchObject({ terminalId: 't1', cols: 100, rows: 30 })

    await runtime.dispose!('closed')
  })

  it('切换 shell 会重启会话，并把选择持久化', async () => {
    const setItem = vi.fn()
    vi.stubGlobal('window', {
      setInterval: (fn: () => void, ms: number) => setInterval(fn, ms),
      clearInterval: (id: number) => clearInterval(id),
      setTimeout: (fn: () => void, ms: number) => setTimeout(fn, ms),
      clearTimeout: (id: number) => clearTimeout(id),
      localStorage: { getItem: () => null, setItem },
    })
    mock.responses['sandbox.terminal.spawn'] = (params?: Record<string, unknown>) => ({
      ok: true,
      terminal: {
        id: params?.shell === 'cmd' ? 't2' : 't1',
        cwd: 'C:/ws',
        running: true,
        shell: String(params?.shell ?? 'pwsh'),
        shellLabel: 'shell',
        cursor: 0,
      },
    })

    const definition = createTerminalWorkbenchDefinition()
    const state: Record<string, unknown> = {}
    const runtime = await definition.createRuntime!(makeItem(), makeContext(state))

    await runtime.handleComponentEvent!({ type: 'terminal-set-shell', payload: { shell: 'cmd' } }, makeItem())

    const spawns = mock.calls.filter(call => call.method === 'sandbox.terminal.spawn')
    expect(spawns).toHaveLength(2)
    expect(spawns[1].params).toMatchObject({ shell: 'cmd' })
    expect(setItem).toHaveBeenCalledWith('marguerite.terminal.shell', 'cmd')

    await runtime.dispose!('closed')
  })

  it('多根会话：spawn 传 roots（顺序即优先级）且不再传 root', async () => {
    const definition = createTerminalWorkbenchDefinition({
      resolveRoots: () => ['D:/repo/main', 'D:/repo/lib', 'D:/repo/docs'],
    })
    const state: Record<string, unknown> = {}
    const runtime = await definition.createRuntime!(makeItem(), makeContext(state))

    const spawn = mock.calls.find(call => call.method === 'sandbox.terminal.spawn')
    expect(spawn!.params).toMatchObject({
      cwd: 'D:/repo/main',
      roots: ['D:/repo/main', 'D:/repo/lib', 'D:/repo/docs'],
    })
    expect(spawn!.params).not.toHaveProperty('root')
    expect(state.roots).toEqual(['D:/repo/main', 'D:/repo/lib', 'D:/repo/docs'])

    await runtime.dispose!('closed')
  })

  it('单根会话：只传 root（不传 roots），保持旧后端兼容', async () => {
    const definition = createTerminalWorkbenchDefinition({
      resolveRoots: () => ['D:/repo/only'],
    })
    const state: Record<string, unknown> = {}
    const runtime = await definition.createRuntime!(makeItem(), makeContext(state))

    const spawn = mock.calls.find(call => call.method === 'sandbox.terminal.spawn')
    expect(spawn!.params).toMatchObject({
      cwd: 'D:/repo/only',
      root: 'D:/repo/only',
    })
    expect(spawn!.params).not.toHaveProperty('roots')

    await runtime.dispose!('closed')
  })

  it('空根、重复根与不可用项目会被过滤', async () => {
    const definition = createTerminalWorkbenchDefinition({
      resolveRoots: () => [
        '',
        '   ',
        'D:/repo/main',
        'D:\\repo\\main\\',
        { path: 'D:/repo/offline', available: false },
        { path: 'D:/repo/lib', available: true },
        { path: '' },
      ],
    })
    const state: Record<string, unknown> = {}
    const runtime = await definition.createRuntime!(makeItem(), makeContext(state))

    const spawn = mock.calls.find(call => call.method === 'sandbox.terminal.spawn')
    expect(spawn!.params).toMatchObject({ roots: ['D:/repo/main', 'D:/repo/lib'] })
    expect(spawn!.params).not.toHaveProperty('root')

    await runtime.dispose!('closed')
  })

  it('未注入 resolveRoots 时，从会话台账 projectPaths 取多根', async () => {
    mock.responses['sessions.list'] = {
      sessions: [
        { key: 'session-a', projectPaths: ['D:/ws/a', 'D:/ws/b'] },
      ],
    }
    const definition = createTerminalWorkbenchDefinition()
    const state: Record<string, unknown> = {}
    const runtime = await definition.createRuntime!(makeItem(), makeContext(state))

    const spawn = mock.calls.find(call => call.method === 'sandbox.terminal.spawn')
    expect(spawn!.params).toMatchObject({
      cwd: 'D:/ws/a',
      roots: ['D:/ws/a', 'D:/ws/b'],
    })
    expect(spawn!.params).not.toHaveProperty('root')

    await runtime.dispose!('closed')
  })

  it('注入的工作根读取失败时提示用户，且仍继续启动（不伪装成功）', async () => {
    const definition = createTerminalWorkbenchDefinition({
      resolveRoots: () => {
        throw new Error('gateway offline')
      },
    })
    const state: Record<string, unknown> = {}
    const runtime = await definition.createRuntime!(makeItem(), makeContext(state))

    expect(notifyReadFailure).toHaveBeenCalledWith(
      'terminal.status',
      'terminal.statusReadFailed',
    )
    expect(mock.calls.some(call => call.method === 'sandbox.terminal.spawn')).toBe(true)

    await runtime.dispose!('closed')
  })

  it('会话台账读取失败时提示用户，且仍继续启动', async () => {
    mock.responses['sessions.list'] = () => {
      throw new Error('gateway offline')
    }
    const definition = createTerminalWorkbenchDefinition()
    const state: Record<string, unknown> = {}
    const runtime = await definition.createRuntime!(makeItem(), makeContext(state))

    expect(notifyReadFailure).toHaveBeenCalledWith(
      'terminal.status',
      'terminal.statusReadFailed',
    )
    expect(mock.calls.some(call => call.method === 'sandbox.terminal.spawn')).toBe(true)

    await runtime.dispose!('closed')
  })

  it('默认 cwd 读取失败时提示用户，且仍按后端默认根启动', async () => {
    mock.responses['sessions.list'] = { sessions: [] }
    mock.responses['workspace.status'] = () => {
      throw new Error('gateway offline')
    }
    const definition = createTerminalWorkbenchDefinition()
    const state: Record<string, unknown> = {}
    const runtime = await definition.createRuntime!(makeItem(), makeContext(state))

    expect(notifyReadFailure).toHaveBeenCalledWith(
      'terminal.status',
      'terminal.statusReadFailed',
    )
    const spawn = mock.calls.find(call => call.method === 'sandbox.terminal.spawn')
    expect(spawn).toBeTruthy()

    await runtime.dispose!('closed')
  })
})
