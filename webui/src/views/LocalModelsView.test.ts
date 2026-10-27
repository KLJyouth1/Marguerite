// @vitest-environment happy-dom
// 冷加载竞态守卫：本地模型页在 WS 仍处于 connecting 时**不得**把「未就绪」当成
// 加载失败把 `Cannot call models.local.recommend: not connected (state: connecting)`
// 裸串渲染给用户；应先等连接就绪再发起 RPC，真失败仍要可见（不吞错）。
import { createApp, nextTick, type App } from 'vue'
import { createPinia } from 'pinia'
import { createI18n } from 'vue-i18n'
import { afterEach, describe, expect, it, vi } from 'vitest'
import en from '@/locales/en.json'
import zhHans from '@/locales/zh-Hans.json'

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
        return { os: 'linux', ram_gb: 16, cores: 8, gpu: false, gpu_name: '' }
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

const LOCALES = { en, 'zh-Hans': zhHans }

async function mountView(locale: keyof typeof LOCALES = 'en') {
  const el = document.createElement('div')
  document.body.appendChild(el)
  app = createApp(LocalModelsView)
  app.use(createPinia())
  app.use(createI18n({ legacy: false, locale, messages: LOCALES }))
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
    expect(el.textContent).not.toContain('Load failed')
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

    expect(el.textContent).toContain('Load failed')
    expect(el.textContent).toContain('boom')
    expect(el.textContent).not.toContain('not connected')
  })
})

describe('LocalModelsView Ollama live models', () => {
  const omnipilot17 = {
    name: 'omnipilot17:latest',
    model: 'omnipilot17:latest',
    digest: 'sha256:421e1507f821',
    size_bytes: 986_000_000,
    size_gb: 0.92,
    family: 'qwen2',
    families: ['qwen2'],
    parameter_size: '7b',
    quantization: 'Q4_K_M',
    modified_at: '2026-10-19T10:00:00Z',
  }
  const omnipilot16c = { ...omnipilot17, name: 'omnipilot16c:latest', model: 'omnipilot16c:latest' }

  function installMocks(opts: { profiles?: unknown[]; deployResult?: unknown } = {}) {
    h.state.value = 'connected'
    h.wait.mockResolvedValue(undefined)
    h.call.mockImplementation(async (method: string) => {
      switch (method) {
        case 'models.local.system':
          return { os: 'windows', ram_gb: 32, cores: 16, gpu: true, gpu_name: 'RTX' }
        case 'models.local.catalog':
          return { items: [] }
        case 'models.local.status':
          return { ollama_present: true, managed: [], local_profiles: opts.profiles ?? [] }
        case 'usage.cost':
          return {}
        case 'models.local.finetune.list':
          return { jobs: [] }
        case 'models.local.recommend':
          return { tier: 'T1', max_billions: 7, reason: '适配 32GB 内存', picks: [] }
        case 'models.local.ollama':
          return { ok: true, online: true, base_url: 'http://localhost:11434', models: [omnipilot17, omnipilot16c] }
        case 'models.local.ollama.deploy':
          return opts.deployResult ?? {
            ok: true, deployed: true, profile_id: 'p-omnipilot17', activated: true,
            model: 'omnipilot17:latest', provider: 'ollama',
          }
        default:
          return {}
      }
    })
  }

  async function setup() {
    const el = await mountView()
    await flush()
    await flush()
    return el
  }

  it('列出本机 Ollama 在线模型（含 omnipilot 双模型）与在线徽标、大小', async () => {
    installMocks({})
    const el = await setup()
    expect(h.call).toHaveBeenCalledWith('models.local.ollama', {})
    expect(el.textContent).toContain('Installed · running')
    expect(el.textContent).toContain('omnipilot17:latest')
    expect(el.textContent).toContain('omnipilot16c:latest')
    expect(el.textContent).toContain('0.92 GB')
  })

  it('Ollama 离线时显示离线徽标与提示，不抛 Load failed', async () => {
    h.state.value = 'connected'
    h.wait.mockResolvedValue(undefined)
    h.call.mockImplementation(async (method: string) => {
      switch (method) {
        case 'models.local.system':
          return { os: 'windows', ram_gb: 32, cores: 16, gpu: true, gpu_name: 'RTX' }
        case 'models.local.catalog':
          return { items: [] }
        case 'models.local.status':
          return { ollama_present: false, managed: [], local_profiles: [] }
        case 'usage.cost':
          return {}
        case 'models.local.finetune.list':
          return { jobs: [] }
        case 'models.local.recommend':
          return { tier: 'T1', max_billions: 7, reason: '', picks: [] }
        case 'models.local.ollama':
          return { ok: true, online: false, base_url: 'http://localhost:11434', models: [], error: 'connect refused' }
        default:
          return {}
      }
    })
    const el = await setup()
    expect(el.textContent).toContain('Not installed')
    expect(el.textContent).toContain('localhost:11434')
    expect(el.textContent).not.toContain('Load failed')
    expect(el.textContent).not.toContain('omnipilot17')
  })

  it('点击「Add to routing & enable」→ 以 activate:true 调 deploy，成功后提示并刷新', async () => {
    installMocks({})
    const el = await setup()
    const btn = [...el.querySelectorAll('button')].find(b => b.textContent?.includes('Add to routing'))
    expect(btn).toBeTruthy()
    btn!.click()
    await flush()
    await flush()
    await flush()
    expect(h.call).toHaveBeenCalledWith(
      'models.local.ollama.deploy',
      { model: 'omnipilot17:latest', activate: true },
    )
    expect(el.textContent).toContain('Added to routing and activated')
  })

  it('已部署档案中的模型显示「使用中 / 已加入路由」，不再提供加入按钮', async () => {
    installMocks({
      profiles: [
        { id: 'p1', label: 'Omni17', model: 'omnipilot17:latest', base_url: 'http://localhost:11434', active: true },
        { id: 'p2', label: 'Omni16c', model: 'omnipilot16c:latest', base_url: 'http://localhost:11434', active: false },
      ],
    })
    const el = await setup()
    expect(el.textContent).toContain('In use')
    expect(el.textContent).toContain('Already in routing')
    const addBtns = [...el.querySelectorAll('button')].filter(b => b.textContent?.includes('Add to routing'))
    expect(addBtns.length).toBe(0)
  })

  it('部署失败时按后端 error 文案提示，不静默', async () => {
    installMocks({ deployResult: { ok: false, error: 'model tag not found', code: 'NOT_FOUND' } })
    const el = await setup()
    const btn = [...el.querySelectorAll('button')].find(b => b.textContent?.includes('Add to routing'))!
    btn.click()
    await flush()
    await flush()
    await flush()
    expect(el.textContent).toContain('model tag not found')
  })
})

// 真机巡检（0.2.16）发现「系统配置」卡片 4 处数据失真：
// 内存只剩单位、GPU 只写「检测到」、推荐档位裸露后端枚举、Ollama 两处状态互相矛盾。
// 这里按后端 `models.local.system` / `.status` / `.ollama` 的真实返回（snake_case）
// 逐项锁定修复结果。
describe('LocalModelsView system card data', () => {
  function installSystemMocks(opts: {
    system?: Record<string, unknown>
    status?: Record<string, unknown>
    recommend?: Record<string, unknown>
    live?: Record<string, unknown>
  } = {}) {
    h.state.value = 'connected'
    h.wait.mockResolvedValue(undefined)
    h.call.mockImplementation(async (method: string) => {
      switch (method) {
        case 'models.local.system':
          return opts.system ?? {
            os: 'windows', ram_gb: 32, cores: 16, gpu: true, gpu_name: 'NVIDIA GeForce RTX 4070',
          }
        case 'models.local.catalog':
          return { items: [] }
        case 'models.local.status':
          return opts.status ?? { ollama_present: true, managed: [], local_profiles: [] }
        case 'usage.cost':
          return {}
        case 'models.local.finetune.list':
          return { jobs: [] }
        case 'models.local.recommend':
          return opts.recommend ?? { tier: 'server', max_billions: 106, reason: '', picks: [] }
        case 'models.local.ollama':
          return opts.live ?? { ok: true, online: true, base_url: 'http://localhost:11434', models: [] }
        default:
          return {}
      }
    })
  }

  async function setup() {
    const el = await mountView()
    await flush()
    await flush()
    return el
  }

  function sysRow(el: Element, label: string): Element | undefined {
    return [...el.querySelectorAll('.lm-sys-rows > div')].find(
      row => row.querySelector('span')?.textContent?.trim() === label,
    )
  }

  it('内存读取后端 ram_gb 并格式化为数值 + 单位', async () => {
    installSystemMocks()
    const el = await setup()
    const row = sysRow(el, 'Memory')
    expect(row?.textContent).toContain('32 GB')
  })

  it('内存缺失时显示占位符，不输出孤零零的单位', async () => {
    installSystemMocks({ system: { os: 'windows', ram_gb: 0, cores: 8, gpu: false, gpu_name: '' } })
    const el = await setup()
    const row = sysRow(el, 'Memory')
    expect(row?.textContent).toContain('—')
    expect(row?.textContent).not.toContain('GB')
  })

  it('GPU 原样展示后端 gpu_name，而不是半截文案「检测到」', async () => {
    installSystemMocks()
    const el = await setup()
    const row = sysRow(el, 'GPU')
    expect(row?.textContent).toContain('NVIDIA GeForce RTX 4070')
    expect(row?.textContent).not.toContain('Detected')
  })

  it('拿不到 GPU 型号时显示「未检测到」并说明可能原因', async () => {
    installSystemMocks({ system: { os: 'windows', ram_gb: 16, cores: 8, gpu: false, gpu_name: '' } })
    const el = await setup()
    const row = sysRow(el, 'GPU')
    expect(row?.textContent).toContain('Not detected')
    expect(row?.textContent).toContain('no GPU driver or discrete GPU detected')
  })

  it('推荐档位把后端枚举本地化，并保留原始值供排查', async () => {
    installSystemMocks()
    const el = await setup()
    const tier = el.querySelector('.lm-tier')!
    expect(tier.textContent?.trim()).toBe('Server-class')
    expect(tier.getAttribute('data-tier')).toBe('server')
    expect(tier.getAttribute('title')).toBe('server')
  })

  it('未知档位枚举有兜底展示，不裸露原始 key', async () => {
    installSystemMocks({ recommend: { tier: 'quantum', max_billions: 0, reason: '', picks: [] } })
    const el = await setup()
    const tier = el.querySelector('.lm-tier')!
    expect(tier.textContent?.trim()).toBe('Unknown tier (quantum)')
    expect(tier.getAttribute('data-tier')).toBe('quantum')
  })

  it('已安装但服务未运行：系统卡片与 Ollama 徽标同源，均为「已安装 · 未运行」并给启动指引', async () => {
    installSystemMocks({
      status: { ollama_present: true, managed: [], local_profiles: [] },
      live: { ok: true, online: false, base_url: 'http://localhost:11434', models: [], error: 'connect refused' },
    })
    const el = await setup()
    expect(sysRow(el, 'Ollama')?.textContent).toContain('Installed · not running')
    const badge = el.querySelector('.lm-ollama-badge')!
    expect(badge.textContent?.trim()).toBe('Installed · not running')
    expect(badge.classList.contains('lm-ollama-badge--warn')).toBe(true)
    expect(el.textContent).toContain('ollama serve')
  })

  it('已安装且服务在线：两处都显示「已安装 · 运行中」', async () => {
    installSystemMocks()
    const el = await setup()
    expect(sysRow(el, 'Ollama')?.textContent).toContain('Installed · running')
    expect(el.querySelector('.lm-ollama-badge')?.textContent?.trim()).toBe('Installed · running')
  })
})

// GateReport 曾经把「✅ 通过 shipped gate / 阻断项 / 通过·提示·阻断」硬编码成中文，
// 英文界面下会中英混排。这里锁定：报告文案必须跟随当前 locale。
describe('LocalModelsView gate report i18n', () => {
  it('中文界面下 gate 报告使用本地化文案', async () => {
    h.state.value = 'connected'
    h.wait.mockResolvedValue(undefined)
    h.call.mockImplementation(async (method: string) => {
      switch (method) {
        case 'models.local.system':
          return { os: 'windows', ram_gb: 32, cores: 16, gpu: true, gpu_name: 'RTX 4070' }
        case 'models.local.catalog':
          return { items: [entry] }
        case 'models.local.status':
          return { ollama_present: true, managed: [], local_profiles: [] }
        case 'models.local.ollama':
          return { ok: true, online: true, base_url: 'http://localhost:11434', models: [] }
        case 'models.local.recommend':
          return { tier: 'laptop', max_billions: 7, reason: '', picks: [] }
        case 'models.gate.check':
          return {
            model: entry.id,
            passed: false,
            blocking: ['weights'],
            findings: [{ element: 'weights', verdict: 'fail', detail: 'missing' }],
          }
        default:
          return {}
      }
    })

    const el = await mountView('zh-Hans')
    await flush()
    await flush()

    const btn = [...el.querySelectorAll('button')]
      .find(b => b.textContent?.includes('校验 shipped gate'))
    expect(btn).toBeTruthy()
    btn!.click()
    await flush()
    await flush()

    const report = el.querySelector('.gate-report')
    expect(report?.textContent).toContain('未通过 shipped gate')
    expect(report?.textContent).toContain('阻断项：weights')
    expect(report?.textContent).toContain('阻断')
  })
})
