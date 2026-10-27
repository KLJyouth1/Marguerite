// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest'
import { createApp, nextTick } from 'vue'

const notifyReadFailure = vi.hoisted(() => vi.fn())
const rpcCalls = vi.hoisted(() => [] as string[])

vi.mock('@/utils/readFailureNotice', () => ({ notifyReadFailure }))
vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({
    isConnected: true,
    waitForConnection: async () => true,
    call: async (method: string) => {
      rpcCalls.push(method)
      return { ok: false, error: 'spawn unavailable' }
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
})
