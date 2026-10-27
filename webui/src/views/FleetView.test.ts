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
    on: vi.fn(() => () => {}),
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

// P0-6 守卫：能力星座的状态必须**来自实测**。此前 4 格是硬编码常量
// （status=running、meta="memory.graph · warm"/"tools.runtime · 12 adapters"），
// 界面看起来是活的，实际从未测量过任何运行时状态。
describe('FleetView capability honesty', () => {
  /** 按方法名分别应答，模拟真实探测端点。 */
  function mockProbes(overrides: Record<string, unknown> = {}) {
    const table: Record<string, unknown> = {
      'fleet.list': { roles },
      'memory.layers.status': {
        layers: [
          { reachable: true },
          { reachable: true },
          { reachable: false },
        ],
      },
      'kb.status': { documents: 7, chunks: 42, engine: 'bm25' },
      'tools.catalog': { items: [{ enabled: true }, { enabled: true }, { enabled: false }] },
      ...overrides,
    }
    h.call.mockImplementation(async (method: string) => {
      if (method in table) return table[method]
      throw new Error(`unsupported: ${method}`)
    })
  }

  it('能力格的副标题是实测数值，不是装饰文案', async () => {
    h.wait.mockResolvedValue(undefined)
    mockProbes()
    h.state.value = 'connected'

    const el = await mountView()
    await flush()
    await flush()
    await flush()

    const text = el.textContent || ''
    // 记忆：3 层里 2 层可达 → 如实写出来（不全可达时不得谎报 ok）
    expect(text).toContain('3 层 · 可达 2')
    expect(text).toContain('memory.layers.status · 实测')
    // 知识库：篇数/分块来自 kb.status
    expect(text).toContain('7 篇 · 42 分块 · bm25')
    // 工具：可用数 + 已停用数
    expect(text).toContain('2 个可用 · 1 个已停用')
    // 旧硬编码文案不得再出现
    expect(text).not.toContain('12 adapters')
    expect(text).not.toContain('· warm')
  })

  it('探测失败的格子如实标注「未探测」，并给出失败原因', async () => {
    h.wait.mockResolvedValue(undefined)
    mockProbes()
    // 让记忆探测抛错
    h.call.mockImplementation(async (method: string) => {
      if (method === 'memory.layers.status') throw new Error('rpc down')
      if (method === 'fleet.list') return { roles }
      if (method === 'kb.status') return { documents: 0, chunks: 0, engine: 'bm25' }
      if (method === 'tools.catalog') return { items: [] }
      throw new Error(`unsupported: ${method}`)
    })
    h.state.value = 'connected'

    const el = await mountView()
    await flush()
    await flush()
    await flush()

    const text = el.textContent || ''
    expect(text).toContain('探测失败：rpc down')
    expect(text).toContain('memory.layers.status · 未探测')
    // 知识库为 0 篇时是 idle，不得谎报 ok(健康)
    expect(text).toContain('0 篇 · 0 分块')
  })

  it('角色卡不谎称运行中：状态恒为 idle（角色是配置，不是进程）', async () => {
    h.wait.mockResolvedValue(undefined)
    mockProbes()
    h.state.value = 'connected'

    const el = await mountView()
    await flush()
    await flush()
    await flush()

    // 推荐前后都不应出现「运行中」这类未被证实的运行态
    expect(el.textContent).not.toContain('running')
  })
})
