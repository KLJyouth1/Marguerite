// @vitest-environment happy-dom
// P0-2 守卫：Fleet 页在 WS 仍处于 connecting 时**不得**把「未就绪」当成加载失败，
// 应显示骨架并在 ready 后成功加载；真失败（含超时 / 服务端错误）仍要可见（不吞错）。
import { createApp, nextTick, type App } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'

const h = vi.hoisted(() => ({
  state: null as unknown as { value: 'disconnected' | 'connecting' | 'connected' },
  call: null as unknown as ReturnType<typeof vi.fn>,
  wait: null as unknown as ReturnType<typeof vi.fn>,
}))

vi.mock('@/stores/rpc', async () => {
  const { ref, reactive } = await import('vue')
  const state = ref<'disconnected' | 'connecting' | 'connected'>('connecting')
  const call = vi.fn()
  const waitForConnection = vi.fn()
  const store = reactive({
    state,
    call,
    waitForConnection,
    get isConnected() {
      return state.value === 'connected'
    },
  })
  h.state = state as unknown as { value: 'disconnected' | 'connecting' | 'connected' }
  h.call = call
  h.wait = waitForConnection
  return { useRpcStore: () => store }
})

import FleetView from '@/views/FleetView.vue'

let app: App<Element> | null = null

const flush = () => new Promise((r) => setTimeout(r, 0))

const roles = [
  {
    role_id: 'architect',
    label: 'Architect',
    kind: 'Planner',
    provider: 'deepseek',
    model: 'deepseek-v4-pro',
    reasoning: 'High',
    max_concurrency: 1,
    system_prompt: 'you are architect',
  },
]

async function mountView() {
  const el = document.createElement('div')
  document.body.appendChild(el)
  app = createApp(FleetView)
  app.mount(el)
  await nextTick()
  return el
}

afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
  vi.clearAllMocks()
  h.state.value = 'connecting'
})

describe('FleetView connection timing', () => {
  it('connecting 时不报错，ready 后成功加载（不把连接竞态误报为失败）', async () => {
    // WS 未就绪：waitForConnection 等到 ready 才 resolve（模拟连接建立）
    h.wait.mockImplementation(async () => {
      h.state.value = 'connected'
      return undefined
    })
    h.call.mockResolvedValue({ roles })

    const el = await mountView()
    await flush()
    await flush()

    // 未就绪阶段绝不应出现 "not connected (state: connecting)"
    expect(el.textContent).not.toContain('not connected')
    expect(el.textContent).not.toContain('加载失败')
    // ready 后真实成功
    expect(h.wait).toHaveBeenCalled()
    expect(h.call).toHaveBeenCalledWith('fleet.list')
    expect(el.textContent).toContain('Architect')
  })

  it('真失败仍要可见（不吞错）', async () => {
    h.state.value = 'connected'
    h.call.mockRejectedValue(new Error('boom'))

    const el = await mountView()
    await flush()
    await flush()

    expect(el.textContent).toContain('加载失败')
    expect(el.textContent).toContain('boom')
  })

  it('ready 后重试一次：首次超时失败后，连接就绪自动补一次加载', async () => {
    // 首次：等待连接超时（处于 connecting）→ 失败可见
    h.wait.mockRejectedValueOnce(new Error('timeout'))
    h.call.mockResolvedValue({ roles })

    const el = await mountView()
    await flush()
    await flush()
    expect(el.textContent).toContain('加载失败')

    // 连接随后就绪 → 观察 state 变为 connected，触发一次重试
    h.state.value = 'connected'
    await nextTick()
    await flush()
    await flush()

    expect(el.textContent).toContain('Architect')
  })
})
