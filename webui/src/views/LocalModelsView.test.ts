// @vitest-environment happy-dom
// 冷加载竞态守卫：本地模型页在 WS 仍处于 connecting 时**不得**把「未就绪」当成
// 加载失败把 `Cannot call models.local.recommend: not connected (state: connecting)`
// 裸串渲染给用户；应先等连接就绪再发起 RPC，真失败仍要可见（不吞错）。
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
    on: () => () => {},
    get isConnected() {
      return state.value === 'connected'
    },
  })
  h.state = state as unknown as { value: 'disconnected' | 'connecting' | 'connected' }
  h.call = call
  h.wait = waitForConnection
  return { useRpcStore: () => store }
})

import LocalModelsView from '@/views/LocalModelsView.vue'

let app: App<Element> | null = null

const flush = () => new Promise((r) => setTimeout(r, 0))

const entry = {
  id: 'qwen3-local',
  label: 'Qwen3 Local',
  family: 'qwen',
  intent: 'general',
  ollama_tag: 'qwen3:8b',
  hf_repo: 'Qwen/Qwen3-8B',
  msc_repo: 'qwen3',
  size_gb: 7,
  min_ram_gb: 16,
  context: '32k',
  shipped_gate: {
    weights: true,
    license: 'apache-2.0',
    eval: '良好',
    pricing: 'free',
    quantized: true,
    reproducible: true,
  },
}

function mockRpcData() {
  h.call.mockImplementation(async (method: string) => {
    switch (method) {
      case 'models.local.system':
        return { os: 'linux', ramGB: 16, cores: 8, gpu: false, gpuName: '' }
      case 'models.local.catalog':
        return { items: [entry] }
      case 'models.local.status':
        return { ollama_present: true, managed: [], local_profiles: [] }
      case 'usage.cost':
        return { estimated_cost_usd: 0, period: 'off-peak', price_input_per_mtok: 0, price_output_per_mtok: 0, advice: '' }
      case 'models.local.finetune.list':
        return { jobs: [] }
      case 'models.local.recommend':
        return { tier: 'T1', max_billions: 7, reason: '适配 16GB 内存', picks: [] }
      default:
        return {}
    }
  })
}

async function mountView() {
  const el = document.createElement('div')
  document.body.appendChild(el)
  app = createApp(LocalModelsView)
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

describe('LocalModelsView connection timing', () => {
  it('connecting 时不报错，ready 后成功加载（不把连接竞态误报为失败）', async () => {
    h.wait.mockImplementation(async () => {
      h.state.value = 'connected'
      return undefined
    })
    mockRpcData()

    const el = await mountView()
    await flush()
    await flush()

    // 未就绪阶段绝不应出现 "not connected (state: connecting)"
    expect(el.textContent).not.toContain('not connected')
    expect(el.textContent).not.toContain('加载失败')
    // ready 后真实成功
    expect(h.wait).toHaveBeenCalled()
    expect(h.call).toHaveBeenCalledWith('models.local.catalog', {})
    expect(el.textContent).toContain('Qwen3 Local')
  })

  it('真失败仍要可见（不吞错，且带友好前缀）', async () => {
    h.state.value = 'connected'
    h.call.mockRejectedValue(new Error('boom'))

    const el = await mountView()
    await flush()
    await flush()

    expect(el.textContent).toContain('加载失败')
    expect(el.textContent).toContain('boom')
    expect(el.textContent).not.toContain('not connected')
  })
})
