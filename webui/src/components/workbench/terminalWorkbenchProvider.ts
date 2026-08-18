import type {
  WorkbenchComponentEvent,
  WorkbenchItem,
  WorkbenchPanelDefinition,
  WorkbenchPanelRuntime,
  WorkbenchRuntimeContext,
} from '@/workbench/types'
import { useRpcStore } from '@/stores/rpc'
import { terminalScopeFromWorkbenchItem } from '@/workbench/terminalItems'
import TerminalPanel from './TerminalPanel.vue'

const POLL_INTERVAL_MS = 400

export interface TerminalWorkbenchProviderOptions {
  /** 终端默认工作目录；缺省由网关决定（当前目录）。 */
  defaultCwd?: string
}

interface SpawnResult {
  ok?: boolean
  terminal?: { id: string; cwd: string; running: boolean }
  error?: string
}

interface ReadResult {
  ok?: boolean
  output?: string
  running?: boolean
  error?: string
}

/**
 * 沙箱终端 runtime：负责 spawn / 增量轮询 read / input / kill / restart，
 * 状态经 `updateRenderState` 同步给 TerminalPanel 组件。
 */
class TerminalWorkbenchRuntime implements WorkbenchPanelRuntime {
  private terminalId = ''
  private pollTimer: number | null = null
  private disposed = false

  constructor(
    _item: WorkbenchItem,
    private readonly context: WorkbenchRuntimeContext,
    private readonly options: TerminalWorkbenchProviderOptions,
  ) {}

  async activate(): Promise<void> {
    if (!this.terminalId) await this.spawn()
  }

  async resume(): Promise<void> {
    if (!this.terminalId) await this.spawn()
    this.startPolling()
  }

  async suspend(): Promise<void> {
    this.stopPolling()
  }

  async handleComponentEvent(event: WorkbenchComponentEvent): Promise<void> {
    const data = event.payload && typeof event.payload === 'object'
      ? String((event.payload as { data?: unknown }).data ?? '')
      : ''
    if (event.type === 'terminal-input') {
      await this.input(data)
    } else if (event.type === 'terminal-kill') {
      await this.kill()
    } else if (event.type === 'terminal-restart') {
      await this.restart()
    }
  }

  async dispose(): Promise<void> {
    this.disposed = true
    this.stopPolling()
    await this.kill()
  }

  private async spawn(): Promise<void> {
    const rpc = useRpcStore()
    try {
      const result = await rpc.call<SpawnResult>(
        'sandbox.terminal.spawn',
        this.options.defaultCwd ? { cwd: this.options.defaultCwd } : undefined,
      )
      if (this.disposed) return
      if (result.ok && result.terminal) {
        this.terminalId = result.terminal.id
        this.updateState({
          connected: true,
          running: true,
          cwd: result.terminal.cwd,
          errorMessage: '',
        })
        this.startPolling()
      } else {
        this.updateState({ connected: false, errorMessage: result.error || '终端启动失败' })
      }
    } catch (error) {
      if (!this.disposed) {
        this.updateState({
          connected: false,
          errorMessage: error instanceof Error ? error.message : String(error),
        })
      }
    }
  }

  private async input(data: string): Promise<void> {
    if (!this.terminalId || !data) return
    const rpc = useRpcStore()
    try {
      await rpc.call<{ ok?: boolean; error?: string }>('sandbox.terminal.input', {
        terminalId: this.terminalId,
        data,
      })
    } catch {
      // 输入失败不中断面板；read 轮询会反映实际状态
    }
  }

  private async kill(): Promise<void> {
    const id = this.terminalId
    this.terminalId = ''
    this.stopPolling()
    if (!id) return
    const rpc = useRpcStore()
    try {
      await rpc.call('sandbox.terminal.kill', { terminalId: id })
    } catch {
      // 忽略终止失败（会话可能已由服务端回收）
    }
    this.updateState({ connected: false, running: false })
  }

  private async restart(): Promise<void> {
    await this.kill()
    await this.spawn()
  }

  private startPolling(): void {
    this.stopPolling()
    this.pollTimer = window.setInterval(() => {
      void this.poll()
    }, POLL_INTERVAL_MS)
  }

  private stopPolling(): void {
    if (this.pollTimer !== null) {
      window.clearInterval(this.pollTimer)
      this.pollTimer = null
    }
  }

  private async poll(): Promise<void> {
    if (!this.terminalId || this.disposed) return
    const rpc = useRpcStore()
    try {
      const result = await rpc.call<ReadResult>('sandbox.terminal.read', {
        terminalId: this.terminalId,
      })
      if (this.disposed) return
      if (result.ok) {
        this.updateState({
          output: String(result.output ?? ''),
          running: result.running === true,
          connected: true,
          errorMessage: '',
        })
        if (!result.running) this.stopPolling()
      } else if (result.error) {
        this.updateState({ connected: false, errorMessage: result.error })
        this.stopPolling()
      }
    } catch {
      // 轮询失败保持现状，下次重试
    }
  }

  private updateState(patch: Record<string, unknown>): void {
    this.context.updateRenderState(patch)
  }
}

export function createTerminalWorkbenchDefinition(
  options: TerminalWorkbenchProviderOptions = {},
): WorkbenchPanelDefinition {
  return {
    kind: 'terminal',
    component: TerminalPanel,
    supports: item => item.kind === 'terminal' && Boolean(terminalScopeFromWorkbenchItem(item)),
    getHeader: (_item, state) => ({
      icon: 'terminal',
      title: _item.title,
      subtitle: String(state.runtimeState.cwd || ''),
    }),
    getProps: (_item, state) => ({
      cwd: String(state.runtimeState.cwd || ''),
      output: String(state.runtimeState.output || ''),
      running: state.runtimeState.running === true,
      connected: state.runtimeState.connected === true,
      errorMessage: String(state.runtimeState.errorMessage || ''),
    }),
    async createRuntime(item, context) {
      const runtime = new TerminalWorkbenchRuntime(item, context, options)
      // 打开即启动会话（resume 时若已断开会重连）
      await runtime.activate()
      return runtime
    },
  }
}
