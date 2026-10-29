// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest'
import { createApp, nextTick } from 'vue'

const notifyReadFailure = vi.hoisted(() => vi.fn())
const rpcCalls = vi.hoisted(() => [] as string[])
/** 单测可替换的 RPC 应答（默认全部失败，见第一个用例）。 */
const rpcHandler = vi.hoisted(() => ({ impl: null as null | ((method: string) => unknown) }))

vi.mock('@/utils/readFailureNotice', () => ({ notifyReadFailure }))
vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({
    isConnected: true,
    waitForConnection: async () => true,
    call: async (method: string) => {
      rpcCalls.push(method)
      return rpcHandler.impl
        ? rpcHandler.impl(method)
        : { ok: false, error: 'spawn unavailable' }
    },
  }),
}))
vi.mock('@/composables/useSessions', () => ({
  useSessions: () => ({
    loadSessions: async () => {
      throw new Error('gateway offline')
    },
    sessionsList: { value: [] },
  }),
}))
vi.mock('vue-router', () => ({
  useRoute: () => ({ query: { session: 'agent:main:webchat:terminal-test' } }),
}))
vi.mock('vue-i18n', () => ({
  useI18n: () => ({ t: (key: string) => key }),
}))

import TerminalView from './TerminalView.vue'

describe('TerminalView', () => {
  it('会话工作根读取失败时提示用户，并仍尝试启动终端', async () => {
    const root = document.createElement('div')
    document.body.appendChild(root)
    const app = createApp(TerminalView)
    app.mount(root)
    try {
      await nextTick()
      await new Promise(resolve => setTimeout(resolve, 0))

      expect(notifyReadFailure).toHaveBeenCalledWith(
        'terminal.status',
        'terminal.statusReadFailed',
      )
      // 工作根读不到不应该把终端页留成空白：spawn 仍会按默认根发起。
      expect(rpcCalls).toContain('sandbox.terminal.spawn')
      expect(root.textContent).toContain('spawn unavailable')
    } finally {
      app.unmount()
      root.remove()
    }
  })

  it('回放输出时剥掉 ANSI 控制序列（含 pwsh 首屏 DSR），不把转义当正文', async () => {
    // 真机事故：`□[6n□[6n□[6n` 占满终端首屏。这里是纯文本回放（非 xterm），
    // 任何 ESC 序列都必须被剥掉，且序列被拆到两次轮询时也要续接。
    rpcHandler.impl = (method: string) => {
      if (method === 'sandbox.terminal.spawn') {
        return { ok: true, terminal: { id: 't1', cwd: 'C:/ws', cursor: 0, running: true } }
      }
      if (method === 'sandbox.terminal.read') {
        return {
          ok: true,
          output: '\u001b[6n\u001b[6nhel\u001b[6',
          cursor: 18,
          running: true,
        }
      }
      return { ok: true, roots: [] }
    }

    vi.useFakeTimers()
    const root = document.createElement('div')
    document.body.appendChild(root)
    const app = createApp(TerminalView)
    app.mount(root)
    try {
      await nextTick()
      await vi.advanceTimersByTimeAsync(300)
      await nextTick()

      const text = root.querySelector('[data-testid="terminal-output"]')?.textContent ?? ''
      expect(text).toContain('hel')
      expect(text).not.toContain('[6n')
      expect(text).not.toContain('\u001b')

      // 残片续接：下一次轮询补完 `n` 后不应出现 `[6n` 尾巴。
      rpcHandler.impl = (method: string) =>
        method === 'sandbox.terminal.read'
          ? { ok: true, output: 'nlo', cursor: 21, running: true }
          : { ok: true }
      await vi.advanceTimersByTimeAsync(300)
      await nextTick()
      const text2 = root.querySelector('[data-testid="terminal-output"]')?.textContent ?? ''
      expect(text2).not.toContain('[6n')
      expect(text2).toContain('hel')
    } finally {
      app.unmount()
      root.remove()
      rpcHandler.impl = null
      vi.useRealTimers()
    }
  })
})
