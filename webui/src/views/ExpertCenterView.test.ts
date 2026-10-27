// @vitest-environment happy-dom
// 冷加载竞态守卫：Expert 页在 WS 仍处于 connecting 时**不得**把「未就绪」当成
// 加载失败把 `not connected (state: connecting)` 裸串渲染给用户；应先等连接就绪
// 再发起 RPC，真失败（服务端错误）仍要可见（不吞错）。
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

// 视图现在用 `useRouter` 提供「开始对话 / 智能体管理」两个出口。
// 不装真 router 时 `inject(routerKey)` 会 warn（Symbol(router) not found），
// 这里用最小替身把导航调用变成可断言的 spy。
const push = vi.hoisted(() => vi.fn())
vi.mock('vue-router', () => ({ useRouter: () => ({ push }) }))

import ExpertCenterView from '@/views/ExpertCenterView.vue'

let app: App<Element> | null = null

const flush = () => new Promise((r) => setTimeout(r, 0))

const expert = {
  id: 'arch',
  label: '架构专家',
  lineage: '蒸馏自资深架构师',
  mindset: '先契约后实现',
  expertise: '接口设计',
  skills: '契约 / 评审',
  playbook: ['读需求', '定契约'],
}

function mockRpcData() {
  h.call.mockImplementation(async (method: string) => {
    switch (method) {
      case 'experts.list':
        return { experts: [expert] }
      case 'agents.list':
        return { agents: [] }
      case 'mode.spec':
        return { kind: 'bloom', label: 'Bloom', risk_level: 'low', allowed_tools: 3, denied_tools: 0, bloom: null }
      case 'mode.swarm.get':
        return { effectiveShards: 11, effectiveSubExperts: 5, maxShardsCap: 200, maxSubExpertsCap: 50 }
      case 'memory.behavior.summary':
        return {}
      case 'agent.profile.auto':
        return {}
      case 'team.compositions.list':
        return { ok: true, items: [] }
      default:
        return {}
    }
  })
}

async function mountView() {
  const el = document.createElement('div')
  document.body.appendChild(el)
  app = createApp(ExpertCenterView)
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

describe('ExpertCenterView connection timing', () => {
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
    expect(h.call).toHaveBeenCalledWith('experts.list', {})
    expect(el.textContent).toContain('架构专家')
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

describe('ExpertCenterView 主智能体', () => {
  const mainAgent = {
    id: 'main',
    name: '严工 · 高级开发工程师',
    description: '先诊断根因，再修一处扫全类',
    is_builtin: true,
    enabled: true,
    model: '',
    tools: ['read_file', 'edit_file'],
    skills: ['根因诊断', '端到端验证'],
    system_prompt: '你是严工，一个拒绝打补丁的高级工程师。',
  }

  function mockWithMain() {
    h.call.mockImplementation(async (method: string) => {
      switch (method) {
        case 'agents.list':
          return { agents: [mainAgent] }
        case 'experts.list':
          return { experts: [] }
        case 'mode.spec':
          return { kind: 'standard', label: '标准', risk_level: 'low', allowed_tools: 1, denied_tools: 0, bloom: null }
        default:
          return {}
      }
    })
  }

  it('agents.list 含 main 时渲染主智能体主视觉（名称 / 能力 / 启用态 / 人设）', async () => {
    h.state.value = 'connected'
    mockWithMain()

    const el = await mountView()
    await flush()
    await flush()

    expect(el.textContent).toContain('严工 · 高级开发工程师')
    expect(el.textContent).toContain('根因诊断')
    expect(el.textContent).toContain('已启用')
    expect(el.textContent).not.toContain('未找到主智能体')

    const personaBtn = [...el.querySelectorAll('button')]
      .find(b => b.textContent?.includes('查看人设'))
    expect(personaBtn, '应有「查看人设」按钮').toBeTruthy()
    personaBtn!.dispatchEvent(new Event('click'))
    await nextTick()
    expect(el.textContent).toContain('你是严工，一个拒绝打补丁的高级工程师。')
  })

  it('加载失败时不得把「请求失败」说成「网关里没有 main」', async () => {
    h.state.value = 'connected'
    h.call.mockRejectedValue(new Error('boom'))

    const el = await mountView()
    await flush()
    await flush()

    expect(el.textContent).toContain('加载失败')
    expect(el.textContent).not.toContain('未找到主智能体')
  })
})
