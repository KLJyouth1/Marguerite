import type {
  WorkbenchComponentEvent,
  WorkbenchItem,
  WorkbenchPanelDefinition,
  WorkbenchPanelRuntime,
  WorkbenchRuntimeContext,
} from '@/workbench/types'
import { useRpcStore } from '@/stores/rpc'
import { notifyReadFailure } from '@/utils/readFailureNotice'
import { terminalScopeFromWorkbenchItem } from '@/workbench/terminalItems'
import TerminalPanel from './TerminalPanel.vue'

const POLL_INTERVAL_MS = 200

/** 前端累积缓冲上限：保留最近 40 万字符（足够回看，又不会把内存吃穿）。 */
const MAX_BUFFER_CHARS = 400_000

/** 记住用户选的 shell（跨会话复用）。 */
export const TERMINAL_SHELL_STORAGE_KEY = 'marguerite.terminal.shell'

/** 工作根候选：字符串，或带可用性标志的条目（不可用项会被过滤）。 */
export type TerminalRootCandidate = string | { path?: string | null; available?: boolean }

export interface TerminalWorkbenchProviderOptions {
  /** 终端默认工作目录；缺省由网关决定（工作区根）。 */
  defaultCwd?: string
  /** 终端默认 shell id（pwsh / powershell / cmd / git-bash / bash / wsl…）。 */
  defaultShell?: string
  /**
   * 当前会话的工作根（按优先级排序，首项为主根）。
   *
   * 由调用方注入（例如 ChatView / TerminalView 用 `useSessions()` 的
   * `projectPaths` 或 `useProjectWorkspaces()` 的 `byId` 解析），provider 不直接
   * 依赖全局 store 单例，便于测试；缺省时回退到 `sessions.list` 的 `projectPaths`。
   * 允许带 `available: false` 的条目，provider 会丢弃。
   */
  resolveRoots?: () => TerminalRootCandidate[] | Promise<TerminalRootCandidate[]>
}

export interface TerminalShellOption {
  id: string
  label: string
  program: string
  available: boolean
  default?: boolean
}

interface TerminalView {
  id: string
  cwd: string
  running: boolean
  shell?: string
  shellLabel?: string
  pid?: number | null
  cols?: number
  rows?: number
  cursor?: number
  exitCode?: number | null
}

interface SpawnResult {
  ok?: boolean
  terminal?: TerminalView
  /** 后端实际接受的合法根（越界回落时与请求值不同）。 */
  roots?: string[] | null
  error?: string
  warning?: string | null
}

interface ReadResult {
  ok?: boolean
  output?: string
  cursor?: number
  lost?: number
  running?: boolean
  exitCode?: number | null
  error?: string
}

interface ShellsResult {
  ok?: boolean
  defaultShell?: string | null
  shells?: TerminalShellOption[]
}

function readStoredShell(): string {
  if (typeof window === 'undefined') return ''
  try {
    return window.localStorage.getItem(TERMINAL_SHELL_STORAGE_KEY) || ''
  } catch {
    return ''
  }
}

function persistShell(shell: string): void {
  if (typeof window === 'undefined' || !shell) return
  try {
    window.localStorage.setItem(TERMINAL_SHELL_STORAGE_KEY, shell)
  } catch {
    // 隐私模式 / 配额满：不影响当前会话
  }
}

/**
 * 归一化工作根：过滤空值、去重（忽略 `\` 与 `/`、尾斜杠差异）、丢弃
 * `available === false` 的条目。顺序即优先级，首项是主根。
 */
export function normalizeTerminalRoots(candidates: unknown): string[] {
  if (!Array.isArray(candidates)) return []
  const seen = new Set<string>()
  const roots: string[] = []
  for (const candidate of candidates) {
    if (candidate && typeof candidate === 'object') {
      if ((candidate as { available?: unknown }).available === false) continue
    }
    const raw = typeof candidate === 'string'
      ? candidate
      : candidate && typeof candidate === 'object'
        ? String((candidate as { path?: unknown }).path ?? '')
        : ''
    const path = raw.trim()
    if (!path) continue
    const key = path.replace(/\\/g, '/').replace(/\/+$/, '')
    if (seen.has(key)) continue
    seen.add(key)
    roots.push(path)
  }
  return roots
}

/**
 * 沙箱终端 runtime：负责 spawn / 游标轮询 read / input / 控制键 / resize / restart，
 * 状态经 `updateRenderState` 同步给 TerminalPanel 组件。
 *
 * 与旧实现的三点关键差异：
 * 1. **游标读取**：`sandbox.terminal.read` 返回 `cursor`，runtime 只追加增量并可补齐，
 *    不会因为「两次轮询之间有人取走输出」而丢内容；
 * 2. **不再伪造提示符**：真实 PTY 会自己回显输入，前端再插 `> cmd` 会重复一行；
 * 3. **控制键**：Ctrl+C 走 `sandbox.terminal.key`（白名单），能真正中断前台命令。
 */
class TerminalWorkbenchRuntime implements WorkbenchPanelRuntime {
  private terminalId = ''
  private pollTimer: number | null = null
  private disposed = false
  /** 已收到的输出累积。 */
  private buffer = ''
  /** 服务端输出游标（下一次增量从该位置读）。 */
  private cursor = 0
  /** 因服务端缓冲淘汰而丢失的字节数（累计）。 */
  private lostBytes = 0
  private selectedShell = ''
  private shells: TerminalShellOption[] = []
  private size = { cols: 0, rows: 0 }
  /** 本次 spawn 使用的工作根（首项为主根）。 */
  private roots: string[] = []

  constructor(
    private readonly item: WorkbenchItem,
    private readonly context: WorkbenchRuntimeContext,
    private readonly options: TerminalWorkbenchProviderOptions,
  ) {
    this.selectedShell = options.defaultShell || readStoredShell()
  }

  async activate(): Promise<void> {
    await this.loadShells()
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
    } else if (event.type === 'terminal-clear') {
      this.buffer = ''
      this.updateState({ output: '' })
    } else if (event.type === 'terminal-interrupt') {
      await this.sendKey('ctrl-c')
    } else if (event.type === 'terminal-set-shell') {
      const shell = String((event.payload as { shell?: unknown })?.shell ?? '')
      await this.changeShell(shell)
    } else if (event.type === 'terminal-resize') {
      const payload = (event.payload ?? {}) as { cols?: unknown; rows?: unknown }
      await this.resize(Number(payload.cols ?? 0), Number(payload.rows ?? 0))
    }
  }

  async dispose(): Promise<void> {
    this.disposed = true
    this.stopPolling()
    await this.kill()
  }

  /** 拉取本机 shell 清单（失败不阻塞终端：默认由网关决定）。 */
  private async loadShells(): Promise<void> {
    const rpc = useRpcStore()
    if (!rpc.supportsMethod('sandbox.terminal.shells')) return
    try {
      const res = await rpc.call<ShellsResult>('sandbox.terminal.shells')
      if (this.disposed || !res?.ok) return
      this.shells = Array.isArray(res.shells) ? res.shells : []
      if (!this.selectedShell && res.defaultShell) this.selectedShell = res.defaultShell
      this.updateState({ shells: this.shells, shell: this.selectedShell })
    } catch {
      // 清单不可用时保持空列表；spawn 仍会按默认 shell 启动
    }
  }

  private async spawn(): Promise<void> {
    const rpc = useRpcStore()
    this.buffer = ''
    this.cursor = 0
    this.lostBytes = 0
    try {
      const roots = await this.resolveSpawnRoots()
      this.roots = roots
      const cwd = await this.resolveCwd()
      const payload: Record<string, unknown> = {}
      if (cwd) payload.cwd = cwd
      // 多根时传 `roots`（顺序即优先级，首个是主根）；单根继续传 `root`，兼顾旧后端，
      // 并让后端把「界面已绑定的项目根」算作合法根，避免 cwd 被误判越界。
      if (roots.length > 1) payload.roots = roots
      else if (roots.length === 1) payload.root = roots[0]
      else if (cwd) payload.root = cwd
      if (this.selectedShell) payload.shell = this.selectedShell
      if (this.size.cols > 0 && this.size.rows > 0) {
        payload.cols = this.size.cols
        payload.rows = this.size.rows
      }
      const result = await rpc.call<SpawnResult>(
        'sandbox.terminal.spawn',
        Object.keys(payload).length ? payload : undefined,
      )
      if (this.disposed) return
      if (result.ok && result.terminal) {
        const { terminal } = result
        this.terminalId = terminal.id
        this.cursor = Number(terminal.cursor ?? 0)
        this.selectedShell = terminal.shell || this.selectedShell
        this.updateState({
          connected: true,
          running: true,
          cwd: terminal.cwd,
          shell: this.selectedShell,
          shellLabel: terminal.shellLabel || '',
          pid: terminal.pid ?? null,
          cols: terminal.cols ?? this.size.cols,
          rows: terminal.rows ?? this.size.rows,
          output: '',
          // 后端越界回落时会带 warning：不能静默吞，面板以提示样式展示。
          warning: result.warning || '',
          roots: normalizeTerminalRoots(
            Array.isArray(result.roots) && result.roots.length ? result.roots : roots,
          ),
          lostBytes: 0,
          exitCode: null,
          errorMessage: '',
        })
        this.startPolling()
      } else {
        this.updateState({
          connected: false,
          errorMessage: result.error || '终端启动失败',
          warning: result.warning || '',
          roots,
        })
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

  /**
   * 终端工作目录：显式配置 → 主工作根 → 工作区默认根。
   *
   * 只问 `workspace.status` 会永远落在应用数据目录：用户在界面上绑定了项目，
   * 终端却还在 `AppData\Local\Marguerite`，第一条 `git status` 必然报「不是仓库」。
   */
  private async resolveCwd(): Promise<string> {
    if (this.options.defaultCwd) return this.options.defaultCwd
    if (this.roots.length > 0) return this.roots[0]
    const rpc = useRpcStore()
    try {
      if (!rpc.supportsMethod('workspace.status')) return ''
      const res = await rpc.call<{ root?: string }>('workspace.status', {})
      return String(res?.root || '')
    } catch {
      // 默认工作根读失败：spawn 会退化成后端默认目录，用户看到的 cwd 与预期不符。
      // 与上面两处共用一个 source（每页只提示一次），不会刷屏。
      notifyReadFailure('terminal.status', 'terminal.statusReadFailed')
      return ''
    }
  }

  /** 注入的根优先；缺省读会话台账（`sessions.list`）的 `projectPaths`。 */
  private async resolveSpawnRoots(): Promise<string[]> {
    if (this.options.resolveRoots) {
      try {
        return normalizeTerminalRoots(await this.options.resolveRoots())
      } catch {
        // 终端工作根读失败：继续 spawn（网关会按默认根兜底），但必须让用户
        // 知道「cwd 可能不是你以为的那个」，而不是静默落进默认目录。
        notifyReadFailure('terminal.status', 'terminal.statusReadFailed')
        return []
      }
    }
    return normalizeTerminalRoots(await this.sessionProjectPaths())
  }

  /** 会话台账里的全部项目根（未绑定 / 台账不可达时返回空数组，不猜）。 */
  private async sessionProjectPaths(): Promise<string[]> {
    const scope = terminalScopeFromWorkbenchItem(this.item)
    if (!scope) return []
    const rpc = useRpcStore()
    if (!rpc.supportsMethod('sessions.list')) return []
    try {
      const res = await rpc.call<{
        sessions?: Array<{
          key?: string
          session?: string
          projectPath?: string | null
          projectPaths?: string[] | null
        }>
      }>('sessions.list', { limit: 200 })
      const hit = (res?.sessions || []).find(s => (s.key || s.session) === scope)
      const paths = Array.isArray(hit?.projectPaths) ? hit.projectPaths : []
      if (paths.length > 0) return paths
      const single = typeof hit?.projectPath === 'string' ? hit.projectPath : ''
      return single ? [single] : []
    } catch {
      // 会话台账不可达：终端仍可 spawn，但工作根已退化成默认目录 —— 提示一次。
      notifyReadFailure('terminal.status', 'terminal.statusReadFailed')
      return []
    }
  }

  private async input(data: string): Promise<void> {
    if (!this.terminalId || !data) return
    const rpc = useRpcStore()
    try {
      const res = await rpc.call<{ ok?: boolean; error?: string }>('sandbox.terminal.input', {
        terminalId: this.terminalId,
        data,
      })
      if (res?.ok === false && res.error) {
        // 护栏拒绝 / 会话失联必须可见：静默吞掉会让用户以为命令「执行了但没输出」。
        this.appendSystemLine(`[拒绝] ${res.error}`)
      }
    } catch (error) {
      this.appendSystemLine(
        `[错误] ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  private async sendKey(sequence: string): Promise<void> {
    if (!this.terminalId) return
    const rpc = useRpcStore()
    try {
      await rpc.call('sandbox.terminal.key', { terminalId: this.terminalId, sequence })
    } catch {
      // 中断失败不破坏会话
    }
  }

  private async resize(cols: number, rows: number): Promise<void> {
    if (!this.terminalId || cols <= 0 || rows <= 0) return
    if (cols === this.size.cols && rows === this.size.rows) return
    this.size = { cols, rows }
    const rpc = useRpcStore()
    try {
      await rpc.call('sandbox.terminal.resize', { terminalId: this.terminalId, cols, rows })
    } catch {
      // 尺寸同步失败不影响输入输出
    }
  }

  private async changeShell(shell: string): Promise<void> {
    if (!shell || shell === this.selectedShell) return
    const option = this.shells.find(item => item.id === shell)
    if (option && option.available === false) return
    this.selectedShell = shell
    persistShell(shell)
    this.updateState({ shell })
    await this.restart()
  }

  private appendSystemLine(text: string): void {
    this.buffer = (this.buffer + `\r\n${text}\r\n`).slice(-MAX_BUFFER_CHARS)
    this.updateState({ output: this.buffer })
  }

  /**
   * 终止会话。返回 `false` 表示**服务端未确认已停止**——此时不能把面板谎报成
   * 「已断开」并丢弃会话 id，否则用户会以为进程没了，实际还在后台跑。
   */
  private async kill(): Promise<boolean> {
    const id = this.terminalId
    if (!id) {
      this.stopPolling()
      this.updateState({ connected: false, running: false })
      return true
    }
    const rpc = useRpcStore()
    try {
      const res = await rpc.call<{ ok?: boolean; error?: string }>('sandbox.terminal.kill', {
        terminalId: id,
      })
      if (res?.ok === false) {
        this.updateState({
          connected: true,
          errorMessage: res.error || '终端未能终止，可能仍在运行；请重试',
        })
        return false
      }
    } catch (error) {
      this.updateState({
        connected: true,
        errorMessage: error instanceof Error ? error.message : String(error),
      })
      return false
    }
    this.terminalId = ''
    this.stopPolling()
    this.buffer = ''
    this.cursor = 0
    this.lostBytes = 0
    this.updateState({ connected: false, running: false, output: '' })
    return true
  }

  private async restart(): Promise<void> {
    // 终止未确认时不重启：否则旧进程留在后台、新进程又占一套资源。
    if (!(await this.kill())) return
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
        cursor: this.cursor,
      })
      if (this.disposed) return
      if (result.ok) {
        const chunk = String(result.output ?? '')
        if (chunk) {
          this.buffer = (this.buffer + chunk).slice(-MAX_BUFFER_CHARS)
        }
        if (typeof result.cursor === 'number' && result.cursor >= this.cursor) {
          this.cursor = result.cursor
        }
        if (typeof result.lost === 'number' && result.lost > 0) {
          this.lostBytes = result.lost
        }
        this.updateState({
          output: this.buffer,
          running: result.running === true,
          connected: true,
          exitCode: result.exitCode ?? null,
          lostBytes: this.lostBytes,
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
      subtitle: [
        String(state.runtimeState.cwd || ''),
        String(state.runtimeState.shellLabel || ''),
      ].filter(Boolean).join(' · '),
    }),
    getProps: (_item, state) => ({
      cwd: String(state.runtimeState.cwd || ''),
      output: String(state.runtimeState.output || ''),
      running: state.runtimeState.running === true,
      connected: state.runtimeState.connected === true,
      errorMessage: String(state.runtimeState.errorMessage || ''),
      warning: String(state.runtimeState.warning || ''),
      shell: String(state.runtimeState.shell || ''),
      shellLabel: String(state.runtimeState.shellLabel || ''),
      shells: (state.runtimeState.shells as TerminalShellOption[] | undefined) ?? [],
      roots: (state.runtimeState.roots as string[] | undefined) ?? [],
      lostBytes: Number(state.runtimeState.lostBytes || 0),
      exitCode: state.runtimeState.exitCode as number | null | undefined,
    }),
    async createRuntime(item, context) {
      const runtime = new TerminalWorkbenchRuntime(item, context, options)
      // 打开即启动会话（resume 时若已断开会重连）
      await runtime.activate()
      return runtime
    },
  }
}
