// @vitest-environment happy-dom
// 聊天页主模型指示（P3 全域打通第 4 行）。
//
// 这组用例锁的是**"用户能不能知道消息发给谁"**这个契约，而不是渲染细节：
//   1. 读到 config.get 就显示；三者皆空 → 整块不渲染（不产生"未知模型"噪声）；
//   2. models.changed 到达 → 防抖后重新拉取并更新；
//   3. 本地/回环端点要能被识别（用户据此判断"离线可跑、不花钱"）；
//   4. 卸载后必须解绑事件（否则切页后仍在后台打 RPC）。
import { createApp, nextTick, type App } from 'vue'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { createI18n } from 'vue-i18n'
import { createRouter, createMemoryHistory } from 'vue-router'

const rpcCall = vi.fn()
const handlers: Record<string, (() => void)[]> = {}

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({
    state: 'connected',
    call: rpcCall,
    on: (evt: string, fn: () => void) => {
      ;(handlers[evt] ||= []).push(fn)
      return () => {
        handlers[evt] = (handlers[evt] || []).filter((f) => f !== fn)
      }
    },
  }),
}))

import ChatModelIndicator from '@/components/chat/ChatModelIndicator.vue'

let app: App<Element> | null = null

const i18n = createI18n({
  legacy: false,
  locale: 'zh-Hans',
  missingWarn: false,
  fallbackWarn: false,
  messages: {
    'zh-Hans': {
      chat: {
        modelIndicator: { tooltip: '当前模型：{model} · 服务商 {provider} · 端点 {endpoint}' },
      },
    },
  },
})

afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
  rpcCall.mockReset()
  for (const k of Object.keys(handlers)) delete handlers[k]
})

beforeAll(() => {
  // 组件用 RouterLink 跳设置页：给一个最小 router 即可
})

async function mount() {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/:pathMatch(.*)*', component: { template: '<div/>' } }],
  })
  app = createApp(ChatModelIndicator)
  app.use(i18n)
  app.use(router)
  app.mount(el)
  await nextTick()
  await nextTick()
  return el
}

function cfg(llm: Record<string, string>) {
  return Promise.resolve({ llm })
}

describe('ChatModelIndicator', () => {
  it('shows the active model and provider from config.get', async () => {
    rpcCall.mockReturnValue(
      cfg({ provider: 'deepseek', model: 'deepseek-v4-flash', base_url: 'https://api.deepseek.com' }),
    )
    const el = await mount()
    expect(rpcCall).toHaveBeenCalledWith('config.get')
    expect(el.querySelector('.model-indicator')).not.toBeNull()
    expect(el.querySelector('.model-indicator__name')?.textContent).toContain('deepseek-v4-flash')
    expect(el.querySelector('.model-indicator__provider')?.textContent).toContain('deepseek')
  })

  it('renders nothing when the model name is missing', async () => {
    // 老网关 / 未配置 / RPC 失败都走这条：不许显示"未知模型"这类噪声
    rpcCall.mockReturnValue(cfg({ provider: 'ollama' }))
    const el = await mount()
    expect(el.querySelector('.model-indicator')).toBeNull()
  })

  it('renders nothing when the RPC rejects', async () => {
    rpcCall.mockReturnValue(Promise.reject(new Error('gateway down')))
    const el = await mount()
    expect(el.querySelector('.model-indicator')).toBeNull()
  })

  it('marks loopback endpoints as local', async () => {
    rpcCall.mockReturnValue(
      cfg({ provider: 'custom', model: 'qwen3:8b', base_url: 'http://127.0.0.1:11434/v1' }),
    )
    const el = await mount()
    expect(el.querySelector('.model-indicator__dot--local')).not.toBeNull()
  })

  it('does not mark a cloud endpoint as local', async () => {
    rpcCall.mockReturnValue(
      cfg({ provider: 'openai', model: 'gpt-5', base_url: 'https://api.openai.com/v1' }),
    )
    const el = await mount()
    expect(el.querySelector('.model-indicator__dot--local')).toBeNull()
  })

  it('refreshes after models.changed (debounced)', async () => {
    rpcCall.mockReturnValueOnce(cfg({ provider: 'ollama', model: 'old:1b', base_url: 'http://localhost:1' }))
    const el = await mount()
    expect(el.querySelector('.model-indicator__name')?.textContent).toContain('old:1b')

    // 第二次拉取返回新模型
    rpcCall.mockReturnValue(cfg({ provider: 'deepseek', model: 'deepseek-v4-pro', base_url: 'https://api.deepseek.com' }))
    // 模拟一次部署连发三条事件：防抖后只应重新拉一次
    for (const fn of handlers['models.changed'] || []) {
      fn()
      fn()
      fn()
    }
    await new Promise((r) => setTimeout(r, 260))
    await nextTick()
    expect(el.querySelector('.model-indicator__name')?.textContent).toContain('deepseek-v4-pro')
    const callsAfterFirstLoad = rpcCall.mock.calls.filter((c) => c[0] === 'config.get').length
    expect(callsAfterFirstLoad).toBe(2)
  })

  it('unsubscribes on unmount', async () => {
    rpcCall.mockReturnValue(cfg({ provider: 'deepseek', model: 'm', base_url: 'https://x' }))
    await mount()
    expect((handlers['models.changed'] || []).length).toBe(1)
    app?.unmount()
    app = null
    expect((handlers['models.changed'] || []).length).toBe(0)
  })

  it('shows the saved profile count from llm_profiles', async () => {
    rpcCall.mockReturnValue(
      Promise.resolve({
        llm: {
          provider: 'deepseek',
          model: 'deepseek-v4-flash',
          base_url: 'https://api.deepseek.com',
        },
        llm_profiles: {
          deepseek: { model: 'deepseek-v4-flash', profileId: 'deepseek-chat', label: 'DeepSeek' },
          openrouter: { model: 'gpt-5', profileId: 'openrouter-gpt5', label: 'OR' },
        },
      }),
    )
    const el = await mount()
    expect(el.querySelector('.model-indicator__count')?.textContent).toBe('2')
  })

  it('renders no count when llm_profiles is missing or malformed', async () => {
    // 老网关没有该字段；脏数据（数组/字符串）也不许让徽标显示 "NaN"/"[object Object]"
    rpcCall.mockReturnValue(
      Promise.resolve({
        llm: { provider: 'deepseek', model: 'm', base_url: 'https://x' },
        llm_profiles: ['not-a-map'],
      }),
    )
    const el = await mount()
    expect(el.querySelector('.model-indicator')).not.toBeNull()
    expect(el.querySelector('.model-indicator__count')).toBeNull()
  })

  it('exposes a full tooltip for accessibility', async () => {
    rpcCall.mockReturnValue(
      cfg({ provider: 'deepseek', model: 'deepseek-v4-flash', base_url: 'https://api.deepseek.com' }),
    )
    const el = await mount()
    const link = el.querySelector('.model-indicator') as HTMLElement
    const tip = link.getAttribute('title') || ''
    expect(tip).toContain('deepseek-v4-flash')
    expect(tip).toContain('https://api.deepseek.com')
    expect(link.getAttribute('aria-label')).toBe(tip)
  })
})
